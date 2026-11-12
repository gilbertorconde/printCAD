//! Translation of a body's Design features into kernel solid ops.
//!
//! The host drives the recompute loop through the registry: each frame it
//! collects every bench's [`RebuildJob`]s and hands each plan to the kernel
//! worker. This bench's jobs are the bodies with dirty part features, each
//! with its feature history converted into a [`SolidOp`] chain.

mod bodies;
mod dressup;
mod extrude;
mod holes;
mod inputs;
mod patterns;
mod sweeps;
#[cfg(test)]
use crate::feature::{DrillPoint, HoleCut, ThreadSpec};
use bodies::plan_body;
pub(crate) use dressup::edge_op;
use dressup::plan_dressup;
pub(crate) use extrude::{distance_to_plane, start_plane};
use extrude::{plan_extrude, start_face_is_flat};
#[cfg(test)]
use holes::{Bore, bore_drawn};
pub use holes::{NutPocket, hole_diameter, nut_pocket};
pub(crate) use holes::{
    copied_thread_lines, cosmetic_thread_lines, hole_centers, hole_plane, threaded_bores,
    threaded_bores_of,
};
use holes::{cross3, hole_ops};
pub(crate) use inputs::bodies_reach;
use inputs::{
    answer_lent_faces, boolean_inputs, borrow_inputs, datum_probes, followed_bodies,
    following_datums, sketch_projections, tools_reach,
};
pub(crate) use patterns::{Mat4, occurrence_steps, occurrence_transforms, transform_point};
use patterns::{compose_steps, mat_mul, mat_rotation, mat_translation, normalize, plan_pattern};
pub(crate) use sweeps::axis_in_sketch;
use sweeps::{datum_direction, plan_sweep};

use core_document::{BodyId, Document, FeatureId, RebuildJob, WorkbenchFeature};
pub use core_document::{BuildError, BuildPlan};
use kernel_api::{BooleanOp, FaceProbe, Profile, SolidOp};
use wb_sketch::SketchFeature;
use wb_sketch::profile;
use wb_sketch::sketch::GeometryElement;

use crate::feature::{DesignFeature, FacePick};

/// This body's part features in history order (the build history).
pub fn design_features_of_body(
    document: &Document,
    body: BodyId,
) -> Vec<(FeatureId, DesignFeature)> {
    document
        .feature_tree()
        .ordered_nodes(|node| node.workbench_id.as_str() == crate::KIND && node.body == Some(body))
        .filter_map(|(id, _)| document.feature_as::<DesignFeature>(*id).map(|f| (*id, f)))
        .collect()
}

/// Bodies that have at least one dirty part feature (their solids need a
/// rebuild).
pub fn pending_body_rebuilds(document: &Document) -> Vec<BodyId> {
    let mut bodies: Vec<BodyId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == crate::KIND && node.dirty)
        .filter_map(|(_, node)| node.body)
        .collect();
    bodies.sort_by_key(|b| b.0);
    bodies.dedup();
    bodies
}

/// Feature ids of this body's part features (for dirty-flag bookkeeping).
pub fn design_feature_ids(document: &Document, body: BodyId) -> Vec<FeatureId> {
    design_features_of_body(document, body)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// The bodies with dirty part features, each with its build plan. The
/// dirty flags of the body's features and of the features they read
/// (their sketches, the originals of a pattern) are settled first: the
/// rebuild is now scheduled, or has failed with an attributed error, and
/// either way the same job must not come back next frame.
///
/// A Boolean follows its tool body: when the tool's solid is rebuilt, or
/// either body moves, the Boolean is built again. A live borrow follows
/// its source the same way (the sketch as it is now, the solid as it is
/// built now, where the bodies sit), and what stands on it rebuilds with
/// it. A body waits while a body whose solid it is built against is itself
/// still to be planned, so it is built against that body's new solid
/// rather than its old one.
pub fn rebuild_jobs(document: &mut Document) -> Vec<RebuildJob> {
    let bodies: Vec<BodyId> = document.bodies().iter().map(|b| b.id).collect();
    for body in &bodies {
        for (feature, inputs) in boolean_inputs(document, *body) {
            if document.built_against(feature) != Some(inputs) {
                document.mark_feature_stale(feature);
            }
        }
        // A datum whose references changed asks them of the solid again.
        let asked_anew = following_datums(document, *body)
            .into_iter()
            .any(|(datum, asks)| document.built_against(datum) != Some(asks));
        if asked_anew && let Some(first) = design_feature_ids(document, *body).first() {
            document.mark_feature_stale(*first);
        }
        for (feature, inputs) in borrow_inputs(document, *body) {
            if document.built_against(feature) != Some(inputs) {
                document.mark_feature_stale(feature);
            }
        }
    }
    let pending = pending_body_rebuilds(document);
    let ready: Vec<BodyId> = pending
        .iter()
        .copied()
        .filter(|body| {
            !followed_bodies(document, *body)
                .iter()
                .any(|other| pending.contains(other) && !bodies_reach(document, *other, *body))
        })
        .collect();
    ready
        .into_iter()
        .map(|body| {
            let features = design_feature_ids(document, body);
            let inputs: Vec<FeatureId> = features
                .iter()
                .flat_map(|id| document.feature_tree().dependencies(*id))
                .filter(|dep| document.get_feature_meta(*dep).is_some_and(|n| n.dirty))
                .collect();
            for id in features.iter().chain(&inputs) {
                document.clear_feature_dirty(*id);
            }
            for (feature, seen) in boolean_inputs(document, body) {
                document.note_built_against(feature, seen);
            }
            for (datum, asks) in following_datums(document, body) {
                document.note_built_against(datum, asks);
            }
            answer_lent_faces(document, body);
            for (feature, seen) in borrow_inputs(document, body) {
                document.note_built_against(feature, seen);
            }
            let plan = body_plan(document, body).map(|mut plan| {
                plan.probes = datum_probes(document, body, &plan);
                plan.projections = sketch_projections(document, body, &plan);
                plan
            });
            RebuildJob { body, plan }
        })
        .collect()
}

/// Remove a feature and settle what depended on it: the sketches it
/// consumed show again, and its body rebuilds from the start. `false` when
/// nothing was removed.
pub fn delete_feature(document: &mut Document, id: FeatureId) -> bool {
    let body = document.get_feature_meta(id).and_then(|n| n.body);
    let feature = document
        .get_feature_data(id)
        .and_then(|d| DesignFeature::from_json(d).ok());
    let is_base = matches!(feature, Some(DesignFeature::Base {}));
    // The base shape goes last: the features after it build on it.
    if is_base && body.is_some_and(|body| design_feature_ids(document, body).len() > 1) {
        return false;
    }
    // Text writes in a sketch of its own, which goes with it.
    let own = match &feature {
        Some(DesignFeature::Text { sketch, .. }) => Some(*sketch),
        _ => None,
    };
    let sketches = feature.map(|f| f.sketches()).unwrap_or_default();
    if document.remove_feature(id).is_err() {
        return false;
    }
    if let Some(own) = own {
        let _ = document.remove_feature(own);
    }
    // Without its base feature the body is the imported solid again.
    if is_base && let Some(body) = body {
        document.set_body_base(body, false);
    }
    for sketch in sketches.into_iter().filter(|s| Some(*s) != own) {
        document.set_feature_visible(sketch, true);
    }
    if let Some(body) = body {
        invalidate_body(document, body);
    }
    true
}

/// The body's history changed shape: rebuild it from its first Design
/// feature. A body with none is the registry's to clear
/// (`DocumentService::invalidate_body`).
pub fn invalidate_body(document: &mut Document, body: BodyId) {
    if let Some(first) = design_feature_ids(document, body).first() {
        document.mark_feature_dirty(*first);
    }
}

pub(crate) fn face_pick_plane(pick: &FacePick) -> ([f64; 3], [f64; 3]) {
    (
        [
            pick.point[0] as f64,
            pick.point[1] as f64,
            pick.point[2] as f64,
        ],
        [
            pick.normal[0] as f64,
            pick.normal[1] as f64,
            pick.normal[2] as f64,
        ],
    )
}

/// Convert a body's feature history into a kernel solid-op chain. Returns an
/// empty plan when the body has no part features (the caller should clear
/// its solid), and an error when any feature cannot be planned.
pub fn body_build_ops(document: &Document, body: BodyId) -> Result<BuildPlan, BuildError> {
    let mut plan = body_plan(document, body)?;
    match plan.failed.take() {
        Some(error) => Err(error),
        None => Ok(plan),
    }
}

/// The body's history as a kernel chain, stopping at the first feature that
/// cannot be planned: the plan is the history before it, which the body
/// shows, with the error and the features left after it.
pub fn body_plan(document: &Document, body: BodyId) -> Result<BuildPlan, BuildError> {
    let features = design_features_of_body(document, body);
    // A body whose shape came from an import has no history to rebuild
    // from: running the features alone would replace the imported solid
    // with whatever they make on their own.
    if !features.is_empty() && document.body_solid_is_imported(body) {
        return Err(BuildError {
            feature: features.first().map(|(id, _)| *id),
            message: "this body's shape came from an import, so it has no history to rebuild; \
                      features need a body of their own"
                .into(),
        });
    }
    let mut plan = BuildPlan {
        ops: Vec::with_capacity(features.len()),
        op_features: Vec::with_capacity(features.len()),
        ..BuildPlan::default()
    };
    // Chain indices of each feature's ops, for pattern `originals` lookups.
    let mut feature_ops: std::collections::HashMap<FeatureId, Vec<usize>> =
        std::collections::HashMap::new();

    // Features after the body's tip preview an earlier history state and are
    // excluded from the build.
    let tip_seq = document
        .body(body)
        .and_then(|b| b.tip)
        .and_then(|tip| document.get_feature_meta(tip))
        .map(|n| n.seq);

    for (feature_id, feature) in features {
        if let Some(tip_seq) = tip_seq {
            let after_tip = document
                .get_feature_meta(feature_id)
                .map(|n| n.seq > tip_seq)
                .unwrap_or(false);
            if after_tip {
                continue;
            }
        }
        // Suppressed features are excluded from the build (still listed in
        // the panel so they can be re-enabled).
        if document
            .get_feature_meta(feature_id)
            .map(|n| n.suppressed)
            .unwrap_or(false)
        {
            continue;
        }
        if plan.failed.is_some() {
            plan.unbuilt.push(feature_id);
            continue;
        }
        let kept = (plan.ops.len(), plan.op_features.len(), plan.probes.len());
        if let Err(error) = plan_feature(
            document,
            body,
            &mut plan,
            &mut feature_ops,
            feature_id,
            &feature,
        ) {
            plan.ops.truncate(kept.0);
            plan.op_features.truncate(kept.1);
            plan.probes.truncate(kept.2);
            plan.failed = Some(error);
        }
    }

    Ok(plan)
}

/// One feature's ops, added to the plan; on an error the plan may hold
/// part of them, which the caller takes back out.
fn plan_feature(
    document: &Document,
    body: BodyId,
    plan: &mut BuildPlan,
    feature_ops: &mut std::collections::HashMap<FeatureId, Vec<usize>>,
    feature_id: FeatureId,
    feature: &DesignFeature,
) -> Result<(), BuildError> {
    // The error sits on its feature, which names it wherever it shows.
    let fail = |message: String| BuildError {
        feature: Some(feature_id),
        message,
    };

    if (feature.is_subtractive() || feature.is_modifier()) && plan.ops.is_empty() {
        return Err(fail(
            "needs existing material; add a Pad or another additive feature first".into(),
        ));
    }
    let additive_boolean = if plan.ops.is_empty() {
        BooleanOp::NewSolid
    } else {
        BooleanOp::Fuse
    };
    // What a cutting feature does to the body: cut its tool away, or keep
    // only what the two share.
    let cut = if feature.keeps_inside() {
        BooleanOp::Common
    } else {
        BooleanOp::Cut
    };
    let shape_boolean = |subtractive: bool| {
        if subtractive { cut } else { additive_boolean }
    };

    if feature.start_face().is_some() {
        start_face_is_flat(document, feature_id).map_err(&fail)?;
    }
    let start_index = plan.ops.len();
    match &feature {
        DesignFeature::Pad { .. } | DesignFeature::Pocket { .. } => {
            plan_extrude(document, plan, feature, &fail, cut, additive_boolean)?;
        }
        DesignFeature::Revolution { .. }
        | DesignFeature::Groove { .. }
        | DesignFeature::Loft { .. }
        | DesignFeature::Pipe { .. }
        | DesignFeature::Helix { .. } => {
            plan_sweep(
                document,
                plan,
                feature,
                &fail,
                cut,
                additive_boolean,
                shape_boolean,
            )?;
        }
        DesignFeature::Primitive {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            kind,
            placement,
            subtractive,
            attached,
        } => {
            plan.ops.push(SolidOp::Primitive {
                kind: *kind,
                placement: attached.as_ref().map_or(*placement, |a| a.placement()),
                op: shape_boolean(*subtractive),
            });
        }
        DesignFeature::Hole { .. } => {
            let hole_ops = hole_ops(document, feature).map_err(&fail)?;
            plan.ops.extend(hole_ops);
        }
        DesignFeature::Fillet { .. }
        | DesignFeature::Chamfer { .. }
        | DesignFeature::Draft { .. }
        | DesignFeature::OffsetFaces { .. }
        | DesignFeature::MoveFaces { .. }
        | DesignFeature::Project { .. }
        | DesignFeature::Text { .. }
        | DesignFeature::OffsetSolid { .. }
        | DesignFeature::Scale { .. }
        | DesignFeature::DeleteFaces { .. }
        | DesignFeature::Thickness { .. } => {
            plan_dressup(document, plan, feature, &fail)?;
        }
        DesignFeature::Mirrored { .. }
        | DesignFeature::LinearPattern { .. }
        | DesignFeature::PolarPattern { .. }
        | DesignFeature::CircularPattern { .. }
        | DesignFeature::PointPattern { .. }
        | DesignFeature::PathPattern { .. }
        | DesignFeature::MultiTransform { .. } => {
            plan_pattern(document, plan, feature_ops, feature, &fail)?;
        }
        DesignFeature::Borrow { .. }
        | DesignFeature::Base { .. }
        | DesignFeature::Clone { .. }
        | DesignFeature::Split { .. }
        | DesignFeature::Piece { .. }
        | DesignFeature::BodyBoolean { .. } => {
            plan_body(document, body, plan, feature, &fail)?;
        }
    }

    // A stated tolerance holds for every op the feature made, each keeping
    // its place, so a pattern's originals still count them.
    if let Some(tolerance) = feature.boolean_tolerance() {
        for op in &mut plan.ops[start_index..] {
            let inner = std::mem::replace(op, SolidOp::Refine);
            *op = inner.with_boolean_tolerance(Some(f64::from(tolerance)));
        }
    }
    let indices: Vec<usize> = (start_index..plan.ops.len()).collect();
    for _ in &indices {
        plan.op_features.push(feature_id);
    }
    feature_ops.insert(feature_id, indices);
    // A refine follows the feature's own ops and is not one of them: a
    // pattern re-running this feature's tool re-runs the tool alone.
    if feature.refine() && plan.ops.len() > start_index {
        plan.ops.push(SolidOp::Refine);
        plan.op_features.push(feature_id);
    }
    Ok(())
}

/// Where a mirror's plane stands in the body's frame: a point of it and its
/// normal.
pub(crate) fn mirror_plane(
    document: &Document,
    plane: &crate::feature::MirrorPlane,
) -> Result<([f64; 3], [f64; 3]), String> {
    use crate::feature::{MirrorPlane, PlaneTarget, SketchAxis};
    match plane {
        MirrorPlane::Datum { datum, plane } => target_plane(
            document,
            &PlaneTarget::Datum {
                datum: *datum,
                plane: *plane,
            },
        ),
        MirrorPlane::Sketch { sketch, axis } => {
            let feature = load_sketch(document, *sketch)?;
            let p = profile::plane_of(&feature.plane);
            let normal = match axis {
                None => p.normal,
                // The plane through the axis square to the sketch.
                Some(SketchAxis::Horizontal) => p.y_axis,
                Some(SketchAxis::Vertical) => p.x_axis,
                Some(SketchAxis::Normal) => {
                    return Err(
                        "a mirror plane holds a sketch's axis in its plane, not its normal".into(),
                    );
                }
            };
            Ok((p.origin, normal))
        }
        other => Ok(other.plane()),
    }
}

/// Where a split's plane stands in the body's frame, moved `offset` along
/// its normal: a point of it and its unit normal.
pub(crate) fn split_plane(
    document: &Document,
    plane: &crate::feature::MirrorPlane,
    offset: f32,
) -> Result<([f64; 3], [f64; 3]), String> {
    let (origin, normal) = mirror_plane(document, plane)?;
    let normal = normalize(normal)?;
    let offset = f64::from(offset);
    Ok((
        [
            origin[0] + normal[0] * offset,
            origin[1] + normal[1] * offset,
            origin[2] + normal[2] * offset,
        ],
        normal,
    ))
}

/// Where a plane target stands in the body's frame: a point of it and its
/// normal.
pub(crate) fn target_plane(
    document: &Document,
    target: &crate::feature::PlaneTarget,
) -> Result<([f64; 3], [f64; 3]), String> {
    use crate::feature::PlaneTarget;
    let frame = match target {
        PlaneTarget::Base(plane) => {
            let (origin, normal, _) = plane.frame();
            return Ok((origin.map(f64::from), normal.map(f64::from)));
        }
        PlaneTarget::Datum { datum, plane } => {
            let data = document
                .feature_values(*datum)
                .ok_or("the datum it stops on is gone")?;
            let made = core_document::DatumFeature::from_json(data)
                .map_err(|_| "what it stops on is not a datum".to_string())?;
            let frame = made.frame();
            match (made.shape, plane) {
                (core_document::DatumShape::Plane { .. }, _) => frame,
                (core_document::DatumShape::CoordinateSystem { .. }, which) => {
                    let which = which.unwrap_or(core_document::BasePlane::XY);
                    frame
                        .planes()
                        .into_iter()
                        .zip(core_document::BasePlane::ALL)
                        .find(|(_, p)| *p == which)
                        .map(|((_, f), _)| f)
                        .unwrap_or(frame)
                }
                _ => return Err("the datum it stops on is not a plane".into()),
            }
        }
    };
    Ok((frame.origin.map(f64::from), frame.normal.map(f64::from)))
}

fn edge_probe_of(pick: &crate::feature::EdgePick) -> kernel_api::EdgeProbe {
    kernel_api::EdgeProbe {
        point: pick.point.map(f64::from),
        direction: pick.direction.map(f64::from),
        faces: pick.faces,
    }
}

fn face_probe(pick: &FacePick) -> FaceProbe {
    let (point, normal) = face_pick_plane(pick);
    FaceProbe {
        name: pick.name,
        point,
        normal,
    }
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn load_sketch(
    document: &Document,
    sketch_id: FeatureId,
) -> Result<SketchFeature, String> {
    // A sketch another body lends, where this body sees it.
    if let Some(borrowed) = crate::borrow::sketch(document, sketch_id) {
        return borrowed;
    }
    // As its formulas leave it, solved.
    let data = document
        .feature_values(sketch_id)
        .ok_or("references a missing sketch")?;
    SketchFeature::from_json(data).map_err(|e| format!("invalid sketch data: {e}"))
}

/// The normal of a sketch's plane, in its body's frame.
pub(crate) fn sketch_normal(document: &Document, sketch_id: FeatureId) -> Option<[f32; 3]> {
    load_sketch(document, sketch_id)
        .ok()
        .map(|sketch| wb_sketch::boundary::narrow3(sketch.plane.normal))
}

fn sketch_profile(
    document: &Document,
    sketch_id: FeatureId,
    regions: &[String],
) -> Result<Profile, String> {
    profile_of(&load_sketch(document, sketch_id)?, regions)
}

/// The regions sketch `id`'s curves close, as a feature reads the sketch;
/// none for a sketch that is gone or closes nothing. Kept while the
/// document stays as it is, since a task panel asks every frame.
pub(crate) fn sketch_regions(
    document: &Document,
    id: FeatureId,
) -> std::rc::Rc<Vec<wb_sketch::regions::Region>> {
    type Kept = Option<(FeatureId, u64, std::rc::Rc<Vec<wb_sketch::regions::Region>>)>;
    thread_local! {
        static KEPT: std::cell::RefCell<Kept> = const { std::cell::RefCell::new(None) };
    }
    let seq = document.mutation_seq();
    KEPT.with(|kept| {
        if let Some((at, when, regions)) = &*kept.borrow()
            && (*at, *when) == (id, seq)
        {
            return regions.clone();
        }
        let regions = std::rc::Rc::new(
            load_sketch(document, id)
                .ok()
                .and_then(|s| profile::regions(&s.sketch).ok())
                .unwrap_or_default(),
        );
        *kept.borrow_mut() = Some((id, seq, regions.clone()));
        regions
    })
}

/// The sketch's profile: the outline of the regions named, or what its
/// curves close when none is.
fn profile_of(sketch_feature: &SketchFeature, regions: &[String]) -> Result<Profile, String> {
    let wires =
        profile::extract_regions(&sketch_feature.sketch, regions).map_err(|e| e.to_string())?;
    Ok(Profile {
        plane: profile::plane_of(&sketch_feature.plane),
        wires,
    })
}

/// Sketches available in a body (id + display name), for re-attachment:
/// its own and those other bodies lend it.
pub fn sketches_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, String)> {
    let mut sketches: Vec<(u64, FeatureId, String)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(id, n)| {
            n.body == Some(body)
                && (n.workbench_id.as_str() == wb_sketch::KIND
                    || crate::borrow::lends_sketch(document, **id))
        })
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, id, _)| (*seq, *id));
    sketches.into_iter().map(|(_, id, n)| (id, n)).collect()
}

/// The sketches `feature` may take in `body`: those earlier in history,
/// since one made after it builds after it, and `current`, whatever it is,
/// so the choice already made stays on the list.
#[cfg_attr(not(feature = "egui"), allow(dead_code))]
/// How many points a point pattern on `sketch` stands at: its points that
/// are not construction.
pub(crate) fn sketch_point_count(document: &Document, sketch: FeatureId) -> usize {
    load_sketch(document, sketch).map_or(0, |feature| {
        feature
            .sketch
            .geometry
            .iter()
            .filter(
                |g| matches!(g, GeometryElement::Point(p) if !feature.sketch.is_construction(p.id)),
            )
            .count()
    })
}

/// How many chains of curves `sketch` lays out for a path to follow: none
/// when it has no curves or they branch.
pub(crate) fn sketch_chain_count(document: &Document, sketch: FeatureId) -> usize {
    load_sketch(document, sketch)
        .ok()
        .and_then(|feature| profile::extract_chains(&feature.sketch).ok())
        .map_or(0, |chains| chains.len())
}

pub(crate) fn sketch_choices(
    document: &Document,
    body: BodyId,
    feature: FeatureId,
    current: Option<FeatureId>,
) -> Vec<(FeatureId, String)> {
    let tree = document.feature_tree();
    let seq_of = |id: FeatureId| tree.get_node(id).map(|n| n.seq);
    let limit = seq_of(feature).unwrap_or(u64::MAX);
    sketches_of_body(document, body)
        .into_iter()
        .filter(|(id, _)| Some(*id) == current || seq_of(*id).is_some_and(|seq| seq < limit))
        .collect()
}

/// Human description of the plane a feature's sketch sits on.
pub fn sketch_plane_description(document: &Document, sketch: FeatureId) -> String {
    use wb_sketch::sketch::SketchPlane;
    if let Some(borrowed) = crate::borrow::sketch(document, sketch) {
        return match borrowed {
            Ok(feature) => {
                let p = feature.plane;
                format!(
                    "Borrowed @ ({:.1}, {:.1}, {:.1})  n=({:.2}, {:.2}, {:.2})",
                    p.origin[0], p.origin[1], p.origin[2], p.normal[0], p.normal[1], p.normal[2]
                )
            }
            Err(e) => e,
        };
    }
    let Some(data) = document.get_feature_data(sketch) else {
        return "missing sketch".to_string();
    };
    let Ok(feature) = SketchFeature::from_json(data) else {
        return "invalid sketch".to_string();
    };
    let p = feature.plane;
    let close = |a: [f64; 3], b: [f64; 3]| {
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

/// Mark every part feature dirty (`invalidate_all`: a history jump or
/// Recompute All, where the solids shown may not match the features).
pub fn mark_all_design_features_dirty(document: &mut Document) {
    let ids: Vec<FeatureId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == crate::KIND)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        document.mark_feature_dirty(id);
    }
}

#[cfg(test)]
mod tests;
