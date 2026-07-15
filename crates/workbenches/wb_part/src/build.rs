//! Translation of a body's Part Design features into kernel extrude ops.
//!
//! The host (app shell) drives the recompute loop: it asks which bodies
//! have dirty part features, converts each body's feature history into an
//! [`ExtrudeOp`] chain, and hands the chain to the kernel worker.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature};
use kernel_api::{BooleanOp, ExtrudeOp};
use wb_sketch::profile;
use wb_sketch::SketchFeature;

use crate::feature::PartFeature;

/// This body's part features in creation order (the build history).
pub fn part_features_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, PartFeature)> {
    let mut features: Vec<(u64, FeatureId, PartFeature)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.part" && node.body == Some(body))
        .filter_map(|(id, node)| {
            PartFeature::from_json(&node.data)
                .ok()
                .map(|f| (node.seq, *id, f))
        })
        .collect();
    // `seq` is the document's explicit insertion order — the build history.
    features.sort_by_key(|(seq, _, _)| *seq);
    features.into_iter().map(|(_, id, f)| (id, f)).collect()
}

/// Bodies that have at least one dirty part feature (their solids need a
/// rebuild).
pub fn pending_body_rebuilds(document: &Document) -> Vec<BodyId> {
    let mut bodies: Vec<BodyId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.part" && node.dirty)
        .filter_map(|(_, node)| node.body)
        .collect();
    bodies.sort_by_key(|b| b.0);
    bodies.dedup();
    bodies
}

/// Feature ids of this body's part features (for dirty-flag bookkeeping).
pub fn part_feature_ids(document: &Document, body: BodyId) -> Vec<FeatureId> {
    part_features_of_body(document, body)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// Convert a body's feature history into a kernel extrude chain. Returns an
/// empty chain when the body has no part features (the caller should clear
/// its solid).
pub fn body_build_ops(document: &Document, body: BodyId) -> Result<Vec<ExtrudeOp>, String> {
    let features = part_features_of_body(document, body);
    let mut ops = Vec::with_capacity(features.len());

    for (feature_id, feature) in features {
        let sketch_id = feature.sketch();
        let sketch_data = document
            .get_feature_data(sketch_id)
            .ok_or_else(|| format!("{} references a missing sketch", feature.kind_label()))?;
        let sketch_feature = SketchFeature::from_json(sketch_data)
            .map_err(|e| format!("{}: invalid sketch data: {e}", feature.kind_label()))?;
        let wires = profile::extract_wires(&sketch_feature.sketch).map_err(|e| {
            format!(
                "{} \"{}\": {e}",
                feature.kind_label(),
                document
                    .get_feature_meta(feature_id)
                    .map(|n| n.name.as_str())
                    .unwrap_or("?")
            )
        })?;
        let plane = profile::plane_of(&sketch_feature.plane);

        let (distance, op) = match feature {
            PartFeature::Pad {
                length, reversed, ..
            } => {
                let sign = if reversed { -1.0 } else { 1.0 };
                let boolean = if ops.is_empty() {
                    BooleanOp::NewSolid
                } else {
                    BooleanOp::Fuse
                };
                (f64::from(length) * sign, boolean)
            }
            PartFeature::Pocket {
                depth, reversed, ..
            } => {
                if ops.is_empty() {
                    return Err("Pocket needs existing material; add a Pad first".to_string());
                }
                let sign = if reversed { -1.0 } else { 1.0 };
                (f64::from(depth) * sign, BooleanOp::Cut)
            }
        };
        if distance.abs() < 1e-9 {
            return Err(format!("{} has zero length", feature.kind_label()));
        }

        ops.push(ExtrudeOp {
            plane,
            wires,
            distance,
            op,
        });
    }

    Ok(ops)
}

/// Mark every part feature dirty (used after undo/redo jumps, where the
/// applied solid geometry may no longer match the restored feature state).
pub fn mark_all_part_features_dirty(document: &mut Document) {
    let ids: Vec<FeatureId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.part")
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        document.mark_feature_dirty(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};

    fn rect_sketch() -> SketchFeature {
        let mut sketch = Sketch::new("s");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 5.0))));
        let d = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 5.0))));
        for (s, e) in [(a, b), (b, c), (c, d), (d, a)] {
            sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
        }
        let plane = sketch.plane;
        SketchFeature::new(sketch, plane)
    }

    fn doc_with_body_sketch() -> (Document, BodyId, FeatureId) {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let sketch_id = doc
            .add_feature_in_body(rect_sketch(), "sketch".into(), Some(body))
            .unwrap();
        (doc, body, sketch_id)
    }

    #[test]
    fn pad_produces_new_solid_op() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            PartFeature::Pad {
                sketch: sketch_id,
                length: 7.0,
                reversed: false,
            },
            "Pad".into(),
            Some(body),
        )
        .unwrap();

        let ops = body_build_ops(&doc, body).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, BooleanOp::NewSolid);
        assert!((ops[0].distance - 7.0).abs() < 1e-9);
        assert_eq!(ops[0].wires.len(), 1);
        assert_eq!(ops[0].wires[0].segments.len(), 4);
    }

    #[test]
    fn second_pad_fuses_and_pocket_cuts() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        for feature in [
            PartFeature::Pad {
                sketch: sketch_id,
                length: 5.0,
                reversed: false,
            },
            PartFeature::Pad {
                sketch: sketch_id,
                length: 2.0,
                reversed: true,
            },
            PartFeature::Pocket {
                sketch: sketch_id,
                depth: 3.0,
                reversed: false,
            },
        ] {
            let label = feature.kind_label().to_string();
            doc.add_feature_in_body(feature, label, Some(body)).unwrap();
        }
        let ops = body_build_ops(&doc, body).unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0].op, BooleanOp::NewSolid);
        assert_eq!(ops[1].op, BooleanOp::Fuse);
        assert!(ops[1].distance < 0.0, "reversed pad extrudes backwards");
        assert_eq!(ops[2].op, BooleanOp::Cut);
    }

    #[test]
    fn pocket_first_is_an_error() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            PartFeature::Pocket {
                sketch: sketch_id,
                depth: 3.0,
                reversed: false,
            },
            "Pocket".into(),
            Some(body),
        )
        .unwrap();
        let err = body_build_ops(&doc, body).unwrap_err();
        assert!(err.contains("Pad first"), "{err}");
    }

    #[test]
    fn dirty_pad_flags_body_for_rebuild_and_sketch_edit_propagates() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = doc
            .add_feature_in_body(
                PartFeature::Pad {
                    sketch: sketch_id,
                    length: 7.0,
                    reversed: false,
                },
                "Pad".into(),
                Some(body),
            )
            .unwrap();
        assert!(pending_body_rebuilds(&doc).is_empty());

        // Editing the sketch dirties the pad through the dependency graph.
        doc.mark_feature_dirty(sketch_id);
        assert_eq!(pending_body_rebuilds(&doc), vec![body]);

        doc.clear_feature_dirty(pad_id);
        doc.clear_feature_dirty(sketch_id);
        assert!(pending_body_rebuilds(&doc).is_empty());
    }

    #[test]
    fn open_profile_reports_feature_name() {
        let (mut doc, body, _) = doc_with_body_sketch();
        // Second sketch with an open chain.
        let mut sketch = Sketch::new("open");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let plane = sketch.plane;
        let open_id = doc
            .add_feature_in_body(SketchFeature::new(sketch, plane), "open".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(
            PartFeature::Pad {
                sketch: open_id,
                length: 7.0,
                reversed: false,
            },
            "BadPad".into(),
            Some(body),
        )
        .unwrap();
        let err = body_build_ops(&doc, body).unwrap_err();
        assert!(
            err.contains("BadPad") && err.contains("not closed"),
            "{err}"
        );
    }
}
