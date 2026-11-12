//! What the bench answers for its kinds: their info, cosmetic threads,
//! drawn and picked geometry, and the values they derive.

use crate::{DesignFeature, DesignWorkbench, borrow, build, datum_refs, viewport_out::datum_mesh};
use core_document::{Document, FeatureId, FeatureInfo, Workbench, WorkbenchFeature};

impl DesignWorkbench {
    pub(crate) fn info_of(&self, node: &core_document::FeatureNode) -> FeatureInfo {
        if node.workbench_id.as_str() == "core.datum" {
            let datum = core_document::DatumFeature::from_json(&node.data).ok();
            return FeatureInfo {
                icon: datum.as_ref().map(datum_icon).unwrap_or("datum-plane"),
                kind_label: "Datum".to_string(),
                family_label: "Datum".to_string(),
                builds_solid: false,
            };
        }
        let feature = DesignFeature::from_json(&node.data).ok();
        FeatureInfo {
            icon: feature.as_ref().map(|f| f.icon()).unwrap_or("tree-feature"),
            kind_label: feature
                .as_ref()
                .map(|f| match f.cosmetic_thread() {
                    Some(thread) => format!("{} · {thread} thread", f.kind_label()),
                    None => f.kind_label().to_string(),
                })
                .unwrap_or_else(|| "Design feature".to_string()),
            family_label: "Design feature".to_string(),
            builds_solid: !matches!(feature, Some(DesignFeature::Borrow { .. })),
        }
    }

    pub(crate) fn threads_of(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Vec<core_document::ThreadCallout> {
        let Some(body) = node.body else {
            return Vec::new();
        };
        // Only a threaded hole or a copy of features has bores to thread.
        let threads_or_copies = document
            .feature_as::<DesignFeature>(id)
            .is_some_and(|f| f.cosmetic_thread().is_some() || f.originals().is_some());
        if !threads_or_copies {
            return Vec::new();
        }
        let hole = |hole: FeatureId| {
            let feature = document.feature_as::<DesignFeature>(hole)?;
            Some((
                feature.cosmetic_thread()?,
                build::threaded_bores(document, &feature)?.radius,
            ))
        };
        build::threaded_bores_of(document, body, id)
            .into_iter()
            .filter_map(|bore| {
                let (designation, radius) = hole(bore.hole)?;
                Some(core_document::ThreadCallout {
                    feature: id,
                    body,
                    designation,
                    mouth: bore.mouth,
                    axis: bore.into,
                    radius,
                })
            })
            .collect()
    }

    pub(crate) fn passive_of(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Option<core_document::PassiveGeometry> {
        // A hole's cosmetic thread: a helix down its bore.
        if let Some(feature @ DesignFeature::Hole { .. }) = document.feature_as::<DesignFeature>(id)
            && feature.cosmetic_thread().is_some()
        {
            let body = node.body;
            let mesh = build::cosmetic_thread_lines(document, body, &feature);
            return (!mesh.positions.is_empty()).then(|| core_document::PassiveGeometry {
                revision: core_document::data_revision(&feature.to_json())
                    ^ body
                        .and_then(|b| document.imported_geometry(b))
                        .map_or(0, |g| g.revision),
                mesh,
                tint: core_document::PassiveTint::External,
                regions: Vec::new(),
            });
        }
        // A mirror's or a pattern's copies of threaded holes: their helices.
        if let (Some(body), Some(feature)) = (
            node.body,
            document
                .feature_as::<DesignFeature>(id)
                .filter(|f| f.originals().is_some()),
        ) {
            let bores = build::threaded_bores_of(document, body, id);
            if bores.is_empty() {
                return None;
            }
            let mesh = build::copied_thread_lines(document, body, id);
            // What the lines are drawn from: the copies' places, each hole
            // copied (its thread and depth) and the solid a through hole
            // runs to.
            let mut holes: Vec<FeatureId> = bores.iter().map(|b| b.hole).collect();
            holes.dedup();
            let revision = holes
                .iter()
                .filter_map(|h| document.feature_values(*h))
                .fold(
                    core_document::data_revision(&feature.to_json()),
                    |r, data| r.rotate_left(7) ^ core_document::data_revision(data),
                )
                ^ document.imported_geometry(body).map_or(0, |g| g.revision);
            return (!mesh.positions.is_empty()).then_some(core_document::PassiveGeometry {
                revision,
                mesh,
                tint: core_document::PassiveTint::External,
                regions: Vec::new(),
            });
        }
        let borrow = borrow::borrow_of(document, id)?;
        let mesh = borrow::lines(document, &borrow);
        if mesh.positions.is_empty() {
            return None;
        }
        Some(core_document::PassiveGeometry {
            mesh,
            revision: borrow::lines_revision(document, id, &borrow),
            tint: core_document::PassiveTint::External,
            regions: Vec::new(),
        })
    }

    pub(crate) fn pick_of(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
        pick: &core_document::ViewportPick,
    ) -> Option<f32> {
        if node.workbench_id.as_str() != core_document::DATUM_KIND
            || node.body.is_none()
            || node.body != self.tab.datums_drawn
        {
            return None;
        }
        let datum = document.feature_as::<core_document::DatumFeature>(id)?;
        let mesh = datum_mesh(&datum);
        let on_screen = |i: u32| {
            core_document::runtime::world_to_viewport(
                pick.view_proj,
                pick.viewport,
                mesh.positions[i as usize],
            )
            .map(|(x, y)| glam::Vec2::new(x, y))
        };
        let cursor = glam::Vec2::new(pick.cursor.0, pick.cursor.1);
        mesh.edges
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[a, b]| {
                let (a, b) = (on_screen(*a)?, on_screen(*b)?);
                Some(cursor.distance(core_document::mesh_query::nearest_on_segment(cursor, a, b)))
            })
            .min_by(f32::total_cmp)
    }

    pub(crate) fn follow_solid(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        probed: &core_document::rebuild::ProbedReferences,
    ) -> bool {
        use core_document::WorkbenchFeature;
        if node.workbench_id.as_str() != crate::KIND {
            return false;
        }
        let Ok(mut feature) = DesignFeature::from_json(values) else {
            return false;
        };
        // A feature starting on a face takes where the face is now.
        if let Some(asked) = DesignFeature::from_json(&node.data)
            .ok()
            .and_then(|made| made.start_face().map(|face| face.probe()))
        {
            let Some(face) = feature.start_face_mut() else {
                return false;
            };
            let before = *face;
            if probed.probes != [asked]
                || !probed.answers.first().is_some_and(|a| face.take_answer(a))
                || *face == before
            {
                return false;
            }
            *values = feature.to_json();
            return true;
        }
        let DesignFeature::Primitive {
            attached: Some(attached),
            ..
        } = &mut feature
        else {
            return false;
        };
        if probed.probes != attached.probes() {
            return false;
        }
        let mut datum = attached.datum();
        datum.take_answers(&probed.answers);
        let followed = crate::feature::Attached::from_datum(&datum);
        if followed == **attached {
            return false;
        }
        **attached = followed;
        *values = feature.to_json();
        true
    }

    pub(crate) fn derive_values(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
        _document: &core_document::Document,
    ) -> bool {
        use core_document::{LineAnchor, PointAnchor, WorkbenchFeature};
        if node.workbench_id.as_str() == crate::KIND {
            let Ok(mut feature) = DesignFeature::from_json(values) else {
                return false;
            };
            let DesignFeature::Primitive {
                attached: Some(attached),
                ..
            } = &mut feature
            else {
                return false;
            };
            let mut datum = attached.datum();
            let mut datum_values = datum.to_json();
            if !self.derive(
                &core_document::FeatureNode {
                    workbench_id: core_document::WorkbenchId::from(core_document::DATUM_KIND),
                    ..node.clone()
                },
                &mut datum_values,
                values_of,
                _document,
            ) {
                datum.follow_datums(&|id| {
                    let data = values_of(id)?;
                    core_document::DatumFeature::from_json(&data)
                        .ok()
                        .map(|d| d.frame())
                });
            } else if let Ok(followed) = core_document::DatumFeature::from_json(&datum_values) {
                datum = followed;
                datum.follow_datums(&|id| {
                    let data = values_of(id)?;
                    core_document::DatumFeature::from_json(&data)
                        .ok()
                        .map(|d| d.frame())
                });
            }
            let followed = crate::feature::Attached::from_datum(&datum);
            if followed == **attached {
                return false;
            }
            **attached = followed;
            *values = feature.to_json();
            return true;
        }
        if node.workbench_id.as_str() != core_document::DATUM_KIND {
            return false;
        }
        let Ok(mut datum) = core_document::DatumFeature::from_json(values) else {
            return false;
        };
        let before = datum;
        let at = |sketch: FeatureId, element: uuid::Uuid| {
            let feature = wb_sketch::SketchFeature::from_json(&values_of(sketch)?).ok()?;
            datum_refs::sketch_points_of(&feature, element)
        };
        for anchor in datum.sketch_points_mut() {
            if let PointAnchor::Sketch {
                sketch,
                element,
                point,
            } = anchor
                && let Some([now]) =
                    at(*sketch, *element).and_then(|p| <[[f32; 3]; 1]>::try_from(p).ok())
            {
                *point = now;
            }
        }
        for anchor in datum.sketch_lines_mut() {
            if let LineAnchor::Sketch {
                sketch,
                element,
                start,
                end,
            } = anchor
                && let Some([a, b]) =
                    at(*sketch, *element).and_then(|p| <[[f32; 3]; 2]>::try_from(p).ok())
            {
                *start = a;
                *end = b;
            }
        }
        if datum == before {
            return false;
        }
        *values = datum.to_json();
        true
    }
}

/// The design set's icon for a datum's shape.
pub(crate) fn datum_icon(datum: &core_document::DatumFeature) -> &'static str {
    match datum.shape {
        core_document::DatumShape::Plane { .. } => "datum-plane",
        core_document::DatumShape::Line { .. } => "datum-line",
        core_document::DatumShape::Point => "datum-point",
        core_document::DatumShape::CoordinateSystem { .. } => "coordinate-system",
    }
}
