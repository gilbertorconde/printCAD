//! Translation of a body's Part Design features into kernel extrude ops.
//!
//! The host (app shell) drives the recompute loop: it asks which bodies
//! have dirty part features, converts each body's feature history into an
//! [`ExtrudeOp`] chain, and hands the chain to the kernel worker.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature};
use kernel_api::{BooleanOp, SolidOp, SweepKind};
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

/// "Through all" pockets are approximated with a cut far longer than any
/// printable part; a topological through-all needs face references that the
/// feature model doesn't carry yet.
const THROUGH_ALL_MM: f64 = 1.0e5;

/// Convert a body's feature history into a kernel solid-op chain. Returns an
/// empty chain when the body has no part features (the caller should clear
/// its solid).
pub fn body_build_ops(document: &Document, body: BodyId) -> Result<Vec<SolidOp>, String> {
    let features = part_features_of_body(document, body);
    let mut ops = Vec::with_capacity(features.len());

    for (feature_id, feature) in features {
        // Suppressed features are excluded from the build (still listed in
        // the panel so they can be re-enabled).
        if document
            .get_feature_meta(feature_id)
            .map(|n| n.suppressed)
            .unwrap_or(false)
        {
            continue;
        }
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

        if feature.is_subtractive() && ops.is_empty() {
            return Err(format!(
                "{} needs existing material; add a Pad or Revolution first",
                feature.kind_label()
            ));
        }
        let additive_boolean = if ops.is_empty() {
            BooleanOp::NewSolid
        } else {
            BooleanOp::Fuse
        };

        let (kind, op) = match feature {
            PartFeature::Pad {
                length,
                reversed,
                symmetric,
                ..
            } => {
                let sign = if reversed { -1.0 } else { 1.0 };
                (
                    SweepKind::Extrude {
                        distance: f64::from(length) * sign,
                        symmetric,
                    },
                    additive_boolean,
                )
            }
            PartFeature::Pocket {
                depth,
                reversed,
                through_all,
                ..
            } => {
                // FreeCAD convention: a pocket cuts OPPOSITE the sketch
                // normal — a sketch on a solid's face has its normal
                // pointing out of the material, so the default digs in.
                // `reversed` flips to cut along +normal.
                let sign = if reversed { 1.0 } else { -1.0 };
                let distance = if through_all {
                    THROUGH_ALL_MM * sign
                } else {
                    f64::from(depth) * sign
                };
                (
                    SweepKind::Extrude {
                        distance,
                        symmetric: through_all,
                    },
                    BooleanOp::Cut,
                )
            }
            PartFeature::Revolution {
                angle_deg,
                axis,
                reversed,
                ..
            } => (revolve_kind(axis, angle_deg, reversed)?, additive_boolean),
            PartFeature::Groove {
                angle_deg,
                axis,
                reversed,
                ..
            } => (revolve_kind(axis, angle_deg, reversed)?, BooleanOp::Cut),
        };

        if let SweepKind::Extrude { distance, .. } = kind {
            if distance.abs() < 1e-9 {
                return Err(format!("{} has zero length", feature.kind_label()));
            }
        }

        ops.push(SolidOp {
            plane,
            wires,
            kind,
            op,
        });
    }

    Ok(ops)
}

/// Build the revolve sweep for a feature. `reversed` flips the axis so the
/// sweep runs the other way around.
fn revolve_kind(
    axis: crate::feature::RevolveAxis,
    angle_deg: f32,
    reversed: bool,
) -> Result<SweepKind, String> {
    if !(angle_deg > 0.0 && angle_deg <= 360.0) {
        return Err(format!(
            "Revolution angle must be in (0, 360], got {angle_deg}"
        ));
    }
    let mut dir = axis.dir_2d();
    if reversed {
        dir = [-dir[0], -dir[1]];
    }
    Ok(SweepKind::Revolve {
        axis_origin: [0.0, 0.0],
        axis_dir: dir,
        angle_deg: f64::from(angle_deg),
    })
}

/// Sketches available in a body (id + display name), for re-attachment.
pub fn sketches_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, String)> {
    let mut sketches: Vec<(u64, FeatureId, String)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body == Some(body))
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, _, _)| *seq);
    sketches.into_iter().map(|(_, id, n)| (id, n)).collect()
}

/// Point a part feature at a different sketch: updates the payload, rewires
/// the dependency edge, hides the new sketch (it's consumed) and reveals
/// the old one when nothing else consumes it, then marks for rebuild.
pub fn retarget_feature_sketch(
    document: &mut Document,
    feature_id: FeatureId,
    new_sketch: FeatureId,
) -> Result<(), String> {
    let data = document
        .get_feature_data(feature_id)
        .ok_or("feature not found")?;
    let mut feature = PartFeature::from_json(data).map_err(|e| e.to_string())?;
    let old_sketch = feature.sketch();
    if old_sketch == new_sketch {
        return Ok(());
    }
    match &mut feature {
        PartFeature::Pad { sketch, .. }
        | PartFeature::Pocket { sketch, .. }
        | PartFeature::Revolution { sketch, .. }
        | PartFeature::Groove { sketch, .. } => *sketch = new_sketch,
    }
    document
        .update_feature_data(feature_id, feature.to_json())
        .map_err(|e| e.to_string())?;
    document.set_feature_dependencies(feature_id, vec![new_sketch]);
    document.set_feature_visible(new_sketch, false);
    // Reveal the old sketch only if no remaining part feature consumes it.
    let still_consumed = document
        .feature_tree()
        .all_nodes()
        .filter(|(id, n)| n.workbench_id.as_str() == "wb.part" && **id != feature_id)
        .filter_map(|(_, n)| PartFeature::from_json(&n.data).ok())
        .any(|f| f.sketch() == old_sketch);
    if !still_consumed {
        document.set_feature_visible(old_sketch, true);
    }
    Ok(())
}

/// Human description of the plane a feature's sketch sits on.
pub fn sketch_plane_description(document: &Document, sketch: FeatureId) -> String {
    use wb_sketch::sketch::SketchPlane;
    let Some(data) = document.get_feature_data(sketch) else {
        return "missing sketch".to_string();
    };
    let Ok(feature) = SketchFeature::from_json(data) else {
        return "invalid sketch".to_string();
    };
    let p = feature.plane;
    let close = |a: [f32; 3], b: [f32; 3]| {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5 && (a[2] - b[2]).abs() < 1e-5
    };
    for (preset, label) in [
        (SketchPlane::xy(), "Top (XY)"),
        (SketchPlane::xz(), "Front (XZ)"),
        (SketchPlane::yz(), "Side (YZ)"),
    ] {
        if close(p.origin, preset.origin) && close(p.normal, preset.normal) {
            return label.to_string();
        }
    }
    format!(
        "Face @ ({:.1}, {:.1}, {:.1})  n=({:.2}, {:.2}, {:.2})",
        p.origin[0], p.origin[1], p.origin[2], p.normal[0], p.normal[1], p.normal[2]
    )
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
                symmetric: false,
            },
            "Pad".into(),
            Some(body),
        )
        .unwrap();

        let ops = body_build_ops(&doc, body).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, BooleanOp::NewSolid);
        assert!(matches!(
            ops[0].kind,
            SweepKind::Extrude { distance, symmetric: false } if (distance - 7.0).abs() < 1e-9
        ));
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
                symmetric: false,
            },
            PartFeature::Pad {
                sketch: sketch_id,
                length: 2.0,
                reversed: true,
                symmetric: false,
            },
            PartFeature::Pocket {
                sketch: sketch_id,
                depth: 3.0,
                reversed: false,
                through_all: false,
            },
        ] {
            let label = feature.kind_label().to_string();
            doc.add_feature_in_body(feature, label, Some(body)).unwrap();
        }
        let ops = body_build_ops(&doc, body).unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0].op, BooleanOp::NewSolid);
        assert_eq!(ops[1].op, BooleanOp::Fuse);
        assert!(
            matches!(
                ops[1].kind,
                SweepKind::Extrude { distance, .. } if distance < 0.0
            ),
            "reversed pad extrudes backwards"
        );
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
                through_all: false,
            },
            "Pocket".into(),
            Some(body),
        )
        .unwrap();
        let err = body_build_ops(&doc, body).unwrap_err();
        assert!(err.contains("material"), "{err}");
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
                    symmetric: false,
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
    fn suppressed_features_are_skipped_in_build() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_a = doc
            .add_feature_in_body(
                PartFeature::Pad {
                    sketch: sketch_id,
                    length: 5.0,
                    reversed: false,
                    symmetric: false,
                },
                "PadA".into(),
                Some(body),
            )
            .unwrap();
        doc.add_feature_in_body(
            PartFeature::Pad {
                sketch: sketch_id,
                length: 9.0,
                reversed: false,
                symmetric: false,
            },
            "PadB".into(),
            Some(body),
        )
        .unwrap();

        assert_eq!(body_build_ops(&doc, body).unwrap().len(), 2);
        doc.set_feature_suppressed(pad_a, true);
        let ops = body_build_ops(&doc, body).unwrap();
        assert_eq!(ops.len(), 1);
        // The remaining pad becomes the first op → NewSolid.
        assert_eq!(ops[0].op, BooleanOp::NewSolid);
        assert!(matches!(
            ops[0].kind,
            SweepKind::Extrude { distance, .. } if (distance - 9.0).abs() < 1e-9
        ));
    }

    #[test]
    fn revolution_and_groove_map_to_revolve_kind() {
        use crate::feature::RevolveAxis;
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            PartFeature::Revolution {
                sketch: sketch_id,
                angle_deg: 270.0,
                axis: RevolveAxis::SketchY,
                reversed: false,
            },
            "Revolution".into(),
            Some(body),
        )
        .unwrap();
        doc.add_feature_in_body(
            PartFeature::Groove {
                sketch: sketch_id,
                angle_deg: 90.0,
                axis: RevolveAxis::SketchX,
                reversed: true,
            },
            "Groove".into(),
            Some(body),
        )
        .unwrap();
        let ops = body_build_ops(&doc, body).unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].op, BooleanOp::NewSolid);
        assert!(matches!(
            ops[0].kind,
            SweepKind::Revolve { axis_dir: [x, y], angle_deg, .. }
                if x.abs() < 1e-9 && (y - 1.0).abs() < 1e-9 && (angle_deg - 270.0).abs() < 1e-9
        ));
        assert_eq!(ops[1].op, BooleanOp::Cut);
        assert!(
            matches!(
                ops[1].kind,
                SweepKind::Revolve { axis_dir: [x, y], .. }
                    if (x + 1.0).abs() < 1e-9 && y.abs() < 1e-9
            ),
            "reversed groove flips the axis"
        );
    }

    #[test]
    fn groove_first_is_an_error_and_bad_angle_rejected() {
        use crate::feature::RevolveAxis;
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            PartFeature::Groove {
                sketch: sketch_id,
                angle_deg: 90.0,
                axis: RevolveAxis::SketchY,
                reversed: false,
            },
            "Groove".into(),
            Some(body),
        )
        .unwrap();
        assert!(body_build_ops(&doc, body).unwrap_err().contains("material"));

        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            PartFeature::Revolution {
                sketch: sketch_id,
                angle_deg: 0.0,
                axis: RevolveAxis::SketchY,
                reversed: false,
            },
            "Revolution".into(),
            Some(body),
        )
        .unwrap();
        assert!(body_build_ops(&doc, body).unwrap_err().contains("angle"));
    }

    #[test]
    fn pocket_defaults_to_cutting_against_the_sketch_normal() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        for (reversed, name) in [(false, "Pocket"), (true, "PocketRev")] {
            doc.add_feature_in_body(
                PartFeature::Pad {
                    sketch: sketch_id,
                    length: 5.0,
                    reversed: false,
                    symmetric: false,
                },
                format!("Pad{name}"),
                Some(body),
            )
            .unwrap();
            doc.add_feature_in_body(
                PartFeature::Pocket {
                    sketch: sketch_id,
                    depth: 3.0,
                    reversed,
                    through_all: false,
                },
                name.to_string(),
                Some(body),
            )
            .unwrap();
        }
        let ops = body_build_ops(&doc, body).unwrap();
        assert!(
            matches!(
                ops[1].kind,
                SweepKind::Extrude { distance, .. } if distance < 0.0
            ),
            "default pocket cuts against the normal (into a face's material)"
        );
        assert!(
            matches!(
                ops[3].kind,
                SweepKind::Extrude { distance, .. } if distance > 0.0
            ),
            "reversed pocket cuts along the normal"
        );
    }

    #[test]
    fn through_all_pocket_uses_large_symmetric_cut() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            PartFeature::Pad {
                sketch: sketch_id,
                length: 5.0,
                reversed: false,
                symmetric: false,
            },
            "Pad".into(),
            Some(body),
        )
        .unwrap();
        doc.add_feature_in_body(
            PartFeature::Pocket {
                sketch: sketch_id,
                depth: 1.0,
                reversed: false,
                through_all: true,
            },
            "Pocket".into(),
            Some(body),
        )
        .unwrap();
        let ops = body_build_ops(&doc, body).unwrap();
        assert!(matches!(
            ops[1].kind,
            SweepKind::Extrude { distance, symmetric: true } if distance.abs() >= THROUGH_ALL_MM
        ));
    }

    #[test]
    fn retarget_moves_dependency_and_visibility() {
        let (mut doc, body, sketch_a) = doc_with_body_sketch();
        let sketch_b = doc
            .add_feature_in_body(rect_sketch(), "sketch_b".into(), Some(body))
            .unwrap();
        let pad = doc
            .add_feature_in_body(
                PartFeature::Pad {
                    sketch: sketch_a,
                    length: 5.0,
                    reversed: false,
                    symmetric: false,
                },
                "Pad".into(),
                Some(body),
            )
            .unwrap();
        doc.set_feature_visible(sketch_a, false);
        doc.clear_feature_dirty(pad);

        retarget_feature_sketch(&mut doc, pad, sketch_b).unwrap();

        assert_eq!(doc.feature_tree().dependencies(pad), vec![sketch_b]);
        assert!(
            doc.get_feature_meta(sketch_a).unwrap().visible,
            "old sketch revealed"
        );
        assert!(
            !doc.get_feature_meta(sketch_b).unwrap().visible,
            "new sketch consumed"
        );
        assert!(
            doc.get_feature_meta(pad).unwrap().dirty,
            "rebuild scheduled"
        );
        // Editing the NEW sketch dirties the pad through the rewired edge.
        doc.clear_feature_dirty(pad);
        doc.mark_feature_dirty(sketch_b);
        assert!(doc.get_feature_meta(pad).unwrap().dirty);
        // The old edge is gone.
        doc.clear_feature_dirty(pad);
        doc.clear_feature_dirty(sketch_b);
        doc.mark_feature_dirty(sketch_a);
        assert!(!doc.get_feature_meta(pad).unwrap().dirty);
    }

    #[test]
    fn sketches_of_body_lists_in_creation_order() {
        let (mut doc, body, first) = doc_with_body_sketch();
        let second = doc
            .add_feature_in_body(rect_sketch(), "second".into(), Some(body))
            .unwrap();
        let list = sketches_of_body(&doc, body);
        assert_eq!(
            list.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![first, second]
        );
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
                symmetric: false,
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
