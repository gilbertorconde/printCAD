//! Translation of a body's Design features into kernel solid ops.
//!
//! The host drives the recompute loop through the registry: each frame it
//! collects every bench's [`RebuildJob`]s and hands each plan to the kernel
//! worker. This bench's jobs are the bodies with dirty part features, each
//! with its feature history converted into a [`SolidOp`] chain.

mod extrude;
mod holes;
mod inputs;
mod patterns;
mod sweeps;
#[cfg(test)]
use crate::feature::{DrillPoint, HoleCut, ThreadSpec};
pub(crate) use extrude::{distance_to_plane, start_plane};
use extrude::{extrude_op, start_face_is_flat};
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
use patterns::{compose_steps, mat_mul, mat_rotation, mat_translation, normalize};
pub(crate) use sweeps::axis_in_sketch;
use sweeps::{
    datum_direction, helix_extent, loft_section, pipe_frame, profile_anchor, revolve_kind,
    sketch_spine, turn_plane,
};

use core_document::{BodyId, Document, FeatureId, RebuildJob, WorkbenchFeature};
pub use core_document::{BuildError, BuildPlan};
use kernel_api::{
    BooleanOp, EdgeSelection, ExtrudeTermination, FaceProbe, Profile, RevolveTermination, SolidOp,
    SweepKind,
};
use wb_sketch::SketchFeature;
use wb_sketch::profile;
use wb_sketch::sketch::GeometryElement;

use crate::feature::{DesignFeature, FacePick, PipeCorner, RevolveAxis, RevolveMode};

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
    let sketches = feature.map(|f| f.sketches()).unwrap_or_default();
    if document.remove_feature(id).is_err() {
        return false;
    }
    // Without its base feature the body is the imported solid again.
    if is_base && let Some(body) = body {
        document.set_body_base(body, false);
    }
    for sketch in sketches {
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

fn face_points(picks: &[FacePick]) -> Vec<[f64; 3]> {
    picks
        .iter()
        .map(|p| [p.point[0] as f64, p.point[1] as f64, p.point[2] as f64])
        .collect()
}

/// The name of each pick, in order.
fn face_names(picks: &[FacePick]) -> Vec<kernel_api::TopoName> {
    picks.iter().map(|p| p.name).collect()
}

/// What the kernel rounds or bevels for `edges`; a pair of faces is no
/// selection of edges, and only a fillet takes one, as a blend.
fn edge_selection(edges: &crate::feature::EdgeSel) -> Result<EdgeSelection, String> {
    Ok(match edges {
        crate::feature::EdgeSel::BetweenFaces(_) => {
            return Err("a chamfer takes edges, not a pair of faces".into());
        }
        crate::feature::EdgeSel::All => EdgeSelection::All,
        crate::feature::EdgeSel::Faces(picks) => {
            EdgeSelection::OfPickedFaces(picks.iter().map(face_probe).collect())
        }
        crate::feature::EdgeSel::Edges(picks) => EdgeSelection::Picked(
            picks
                .iter()
                .map(|p| kernel_api::EdgeProbe {
                    faces: p.faces,
                    point: p.point.map(f64::from),
                    direction: p.direction.map(f64::from),
                })
                .collect(),
        ),
    })
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
        DesignFeature::Pad { profile_face, .. } | DesignFeature::Pocket { profile_face, .. } => {
            if profile_face.is_some() && plan.ops.is_empty() {
                return Err(fail(
                    "extruding a face needs a solid to take it from; add a feature first".into(),
                ));
            }
            let boolean = if matches!(feature, DesignFeature::Pocket { .. }) {
                cut
            } else {
                additive_boolean
            };
            plan.ops
                .extend(extrude_op(document, feature, boolean).map_err(&fail)?);
        }
        DesignFeature::Revolution {
            sketch,
            angle_deg,
            start_angle_deg,
            axis,
            project_axis,
            reversed,
            midplane,
            second_angle_deg,
            mode,
            up_to_face,
            ..
        }
        | DesignFeature::Groove {
            sketch,
            angle_deg,
            start_angle_deg,
            axis,
            project_axis,
            reversed,
            midplane,
            second_angle_deg,
            mode,
            up_to_face,
            ..
        } => {
            if !matches!(mode, RevolveMode::Angle | RevolveMode::UpToBorrowed(_))
                && plan.ops.is_empty()
            {
                return Err(fail(
                    "stopping on a face needs existing material; add a feature first".into(),
                ));
            }
            let sketch_feature = load_sketch(document, *sketch).map_err(&fail)?;
            let mut profile = profile_of(&sketch_feature, feature.regions()).map_err(&fail)?;
            let axis_2d =
                axis_in_sketch(document, &sketch_feature, axis, *project_axis).map_err(&fail)?;
            if *start_angle_deg != 0.0 {
                let turn = if *reversed {
                    -*start_angle_deg
                } else {
                    *start_angle_deg
                };
                turn_plane(&mut profile.plane, axis_2d, f64::from(turn));
            }
            let termination = match (mode, up_to_face) {
                (RevolveMode::Angle, _) => RevolveTermination::Angle,
                (RevolveMode::ToFirst, _) => RevolveTermination::ToFirst,
                (RevolveMode::ToLast, _) => RevolveTermination::ToLast,
                (RevolveMode::UpToFace, Some(pick)) => {
                    RevolveTermination::UpToFace(face_probe(pick))
                }
                (RevolveMode::UpToFace, None) => {
                    return Err(fail("pick a target face for the up-to-face mode".into()));
                }
                // Not an angle: the turn ends where it meets the borrowed
                // face the op carries, whatever the angle field holds.
                (RevolveMode::UpToBorrowed(_), _) => RevolveTermination::ToFirst,
            };
            let kind = revolve_kind(
                axis_2d,
                *angle_deg,
                *reversed,
                *midplane,
                *second_angle_deg,
                termination,
            )
            .map_err(&fail)?;
            let op = if matches!(feature, DesignFeature::Groove { .. }) {
                cut
            } else {
                additive_boolean
            };
            if let RevolveMode::UpToBorrowed(borrowed) = mode {
                let face = crate::borrow::kernel_face(document, borrowed).map_err(&fail)?;
                plan.ops.push(SolidOp::RevolveToFaceOf {
                    profile,
                    kind,
                    shape: face.shape,
                    transform: face.transform.map(Box::new),
                    point: face.point,
                    op,
                });
            } else {
                plan.ops.push(SolidOp::Sweep { profile, kind, op });
            }
        }
        DesignFeature::Loft {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            sections,
            ruled,
            closed,
            subtractive,
        } => {
            if sections.len() < 2 {
                return Err(fail("a loft needs at least two sections".into()));
            }
            let mut through = Vec::with_capacity(sections.len());
            for section in sections {
                through.push(loft_section(document, section).map_err(&fail)?);
            }
            // Profiles alone go as the plain loft, which names its faces
            // after the first section's curves.
            let profiles: Option<Vec<Profile>> = through
                .iter()
                .map(|s| match s {
                    kernel_api::LoftSection::Profile(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            plan.ops.push(match profiles {
                Some(sections) => SolidOp::Loft {
                    sections,
                    ruled: *ruled,
                    closed: *closed,
                    op: shape_boolean(*subtractive),
                },
                None => SolidOp::LoftThrough {
                    sections: through,
                    ruled: *ruled,
                    closed: *closed,
                    op: shape_boolean(*subtractive),
                },
            });
        }
        DesignFeature::Pipe {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            profile,
            spine,
            orientation,
            corner,
            sections,
            subtractive,
            profile_face,
            path_edges,
            path_borrowed,
            path_curves,
        } => {
            let frame = pipe_frame(document, orientation).map_err(&fail)?;
            let corner = match corner {
                PipeCorner::Transformed => kernel_api::PipeCorner::Transformed,
                PipeCorner::Right => kernel_api::PipeCorner::Right,
                PipeCorner::Round => kernel_api::PipeCorner::Round,
            };
            let mut through = Vec::with_capacity(sections.len());
            for section in sections {
                through.push(
                    loft_section(document, &crate::feature::LoftSection::Feature(*section))
                        .map_err(&fail)?,
                );
            }
            let plain = profile_face.is_none()
                && path_edges.is_empty()
                && path_borrowed.is_empty()
                && through
                    .iter()
                    .all(|s| matches!(s, kernel_api::LoftSection::Profile(_)));
            if plain {
                let profile = sketch_profile(document, *profile, &[]).map_err(&fail)?;
                let spine = sketch_spine(document, *spine, profile_anchor(&profile), path_curves)
                    .map_err(&fail)?;
                plan.ops.push(SolidOp::Pipe {
                    profile,
                    spine,
                    frame,
                    corner,
                    sections: through
                        .into_iter()
                        .filter_map(|s| match s {
                            kernel_api::LoftSection::Profile(p) => Some(p),
                            _ => None,
                        })
                        .collect(),
                    op: shape_boolean(*subtractive),
                });
            } else {
                let section = match profile_face {
                    Some(pick) => kernel_api::LoftSection::Face(face_probe(pick)),
                    None => kernel_api::LoftSection::Profile(
                        sketch_profile(document, *profile, &[]).map_err(&fail)?,
                    ),
                };
                let path = if !path_edges.is_empty() {
                    kernel_api::PipePath::Edges(path_edges.iter().map(edge_probe_of).collect())
                } else if !path_borrowed.is_empty() {
                    crate::borrow::kernel_edges(document, path_borrowed).map_err(&fail)?
                } else {
                    let anchor = match &section {
                        kernel_api::LoftSection::Profile(p) => profile_anchor(p),
                        _ => None,
                    };
                    kernel_api::PipePath::Profile(
                        sketch_spine(document, *spine, anchor, path_curves).map_err(&fail)?,
                    )
                };
                plan.ops.push(SolidOp::PipeThrough {
                    profile: section,
                    path,
                    frame,
                    corner,
                    sections: through,
                    op: shape_boolean(*subtractive),
                });
            }
        }
        DesignFeature::Helix {
            refine: _,
            boolean_tolerance: _,
            sketch,
            axis,
            project_axis,
            mode,
            pitch,
            height,
            turns,
            left_handed,
            cone_angle_deg,
            reversed,
            subtractive,
            growth,
            keep_inside: _,
        } => {
            let sketch_feature = load_sketch(document, *sketch).map_err(&fail)?;
            let profile = profile_of(&sketch_feature, &[]).map_err(&fail)?;
            let extent = helix_extent(*mode, *pitch, *height, *turns, *growth).map_err(&fail)?;
            let kind = if *axis == RevolveAxis::SketchNormal {
                SweepKind::HelixNormal {
                    axis_origin: [0.0, 0.0],
                    pitch: extent.pitch,
                    height: extent.height,
                    left_handed: *left_handed,
                    cone_angle_deg: *cone_angle_deg as f64,
                    reversed: *reversed,
                    turns: extent.turns,
                    growth: extent.growth,
                }
            } else {
                let (axis_origin, axis_dir) =
                    axis_in_sketch(document, &sketch_feature, axis, *project_axis)
                        .map_err(&fail)?;
                SweepKind::Helix {
                    axis_origin,
                    axis_dir,
                    pitch: extent.pitch,
                    height: extent.height,
                    left_handed: *left_handed,
                    cone_angle_deg: *cone_angle_deg as f64,
                    reversed: *reversed,
                    turns: extent.turns,
                    growth: extent.growth,
                }
            };
            plan.ops.push(SolidOp::Sweep {
                profile,
                kind,
                op: shape_boolean(*subtractive),
            });
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
        DesignFeature::Fillet {
            radius,
            edges,
            follow_tangent,
        } => {
            if *radius <= 0.0 {
                return Err(fail("fillet radius must be positive".into()));
            }
            match edges {
                crate::feature::EdgeSel::BetweenFaces(picks) => match picks.as_slice() {
                    [a, b] => plan.ops.push(SolidOp::BlendFaces {
                        radius: *radius as f64,
                        faces: [face_probe(a), face_probe(b)],
                    }),
                    _ => {
                        return Err(fail(format!(
                            "a fillet between faces takes two faces, it has {}",
                            picks.len()
                        )));
                    }
                },
                edges => plan.ops.push(SolidOp::Fillet {
                    radius: *radius as f64,
                    edges: edge_selection(edges).map_err(fail)?,
                    follow_tangent: *follow_tangent,
                }),
            }
        }
        DesignFeature::Chamfer {
            size,
            mode,
            size2,
            angle_deg,
            flip,
            edges,
            follow_tangent,
        } => {
            if *size <= 0.0 {
                return Err(fail("chamfer size must be positive".into()));
            }
            let spec = match mode {
                crate::feature::ChamferMode::EqualDistance => {
                    kernel_api::ChamferSpec::EqualDistance {
                        distance: *size as f64,
                    }
                }
                crate::feature::ChamferMode::TwoDistances => {
                    kernel_api::ChamferSpec::TwoDistances {
                        distance1: *size as f64,
                        distance2: *size2 as f64,
                    }
                }
                crate::feature::ChamferMode::DistanceAngle => {
                    kernel_api::ChamferSpec::DistanceAngle {
                        distance: *size as f64,
                        angle_deg: *angle_deg as f64,
                    }
                }
            };
            plan.ops.push(SolidOp::Chamfer {
                spec,
                flip: *flip,
                edges: edge_selection(edges).map_err(fail)?,
                follow_tangent: *follow_tangent,
            });
        }
        DesignFeature::Draft {
            angle_deg,
            neutral,
            faces,
            reversed,
            neutral_plane,
            pull,
        } => {
            if faces.is_empty() {
                return Err(fail(
                    "no faces to tilt: pick them in the Draft's panel, or set its faces, \
                     each {point, normal} in the body's own frame"
                        .into(),
                ));
            }
            let (neutral_point, neutral_normal) = match neutral_plane {
                Some(target) => target_plane(document, target).map_err(&fail)?,
                None => face_pick_plane(neutral),
            };
            // The pull: along an edge or datum line when one is given,
            // else along the neutral plane's normal; reversed turns it.
            let along = match pull {
                Some(crate::feature::PullRef::Edge(edge)) => Some(edge.direction.map(f64::from)),
                Some(crate::feature::PullRef::Datum(datum)) => {
                    Some(datum_direction(document, *datum).map_err(&fail)?)
                }
                None => None,
            };
            let pull = match (along, *reversed) {
                (Some(d), false) => Some(d),
                (Some(d), true) => Some([-d[0], -d[1], -d[2]]),
                (None, true) => Some([-neutral_normal[0], -neutral_normal[1], -neutral_normal[2]]),
                (None, false) => None,
            };
            plan.ops.push(SolidOp::Draft {
                angle_deg: *angle_deg as f64,
                neutral_point,
                neutral_normal,
                pull_dir: pull,
                faces: face_points(faces),
                face_names: face_names(faces),
            });
        }
        DesignFeature::OffsetFaces { faces, distance } => {
            if faces.is_empty() {
                return Err(fail("select at least one face to offset".into()));
            }
            plan.ops.push(SolidOp::OffsetFaces {
                faces: face_points(faces),
                face_names: face_names(faces),
                distance: f64::from(*distance),
            });
        }
        DesignFeature::MoveFaces {
            faces,
            translation,
            angle_deg,
            axis_point,
            axis_dir,
        } => {
            if faces.is_empty() {
                return Err(fail("select at least one face to move".into()));
            }
            let mut transform = mat_translation(translation.map(f64::from));
            if angle_deg.abs() > 1e-6 {
                let turn = mat_rotation(
                    axis_point.map(f64::from),
                    axis_dir.map(f64::from),
                    f64::from(*angle_deg),
                )
                .map_err(&fail)?;
                transform = mat_mul(&transform, &turn);
            }
            plan.ops.push(SolidOp::MoveFaces {
                faces: face_points(faces),
                face_names: face_names(faces),
                transform,
            });
        }
        DesignFeature::Project {
            sketch,
            faces,
            mode,
            height,
            wrap,
        } => {
            if faces.is_empty() {
                return Err(fail(
                    "select at least one face to project the sketch onto".into(),
                ));
            }
            let profile = sketch_profile(document, *sketch, &[]).map_err(&fail)?;
            let faces = faces.iter().map(face_probe).collect();
            let (mode, height) = (*mode, f64::from(*height));
            plan.ops.push(if *wrap {
                SolidOp::Wrap {
                    profile,
                    faces,
                    mode,
                    height,
                }
            } else {
                SolidOp::Project {
                    profile,
                    faces,
                    mode,
                    height,
                }
            });
        }
        DesignFeature::OffsetSolid { distance } => {
            if *distance == 0.0 || !distance.is_finite() {
                return Err(fail("give a distance other than zero".into()));
            }
            plan.ops.push(SolidOp::OffsetSolid {
                distance: f64::from(*distance),
            });
        }
        DesignFeature::Scale {
            factor,
            per_axis,
            factors,
            about,
        } => {
            let factors = if *per_axis { *factors } else { [*factor; 3] };
            if factors.iter().any(|f| !f.is_finite() || *f <= 0.0) {
                return Err(fail("every scale factor must be above zero".into()));
            }
            let centre = match about {
                crate::feature::ScaleCentre::Origin => Some([0.0; 3]),
                crate::feature::ScaleCentre::BodyCentre => None,
                crate::feature::ScaleCentre::Datum { datum } => {
                    Some(datum_origin(document, *datum).map_err(&fail)?)
                }
            };
            plan.ops.push(SolidOp::Scale {
                factors: factors.map(f64::from),
                centre,
            });
        }
        DesignFeature::DeleteFaces { faces } => {
            if faces.is_empty() {
                return Err(fail("select at least one face to delete".into()));
            }
            plan.ops.push(SolidOp::RemoveFaces {
                faces: face_points(faces),
                face_names: face_names(faces),
            });
        }
        DesignFeature::Thickness {
            value,
            faces,
            inward,
            join,
            both_sides,
        } => {
            if faces.is_empty() {
                return Err(fail("select at least one face to open".into()));
            }
            plan.ops.push(SolidOp::Thickness {
                value: *value as f64,
                open_faces: face_points(faces),
                open_face_names: face_names(faces),
                inward: *inward,
                join: *join,
                both_sides: *both_sides,
            });
        }
        DesignFeature::Mirrored {
            originals, skipped, ..
        }
        | DesignFeature::LinearPattern {
            originals, skipped, ..
        }
        | DesignFeature::PolarPattern {
            originals, skipped, ..
        }
        | DesignFeature::CircularPattern {
            originals, skipped, ..
        }
        | DesignFeature::PointPattern {
            originals, skipped, ..
        }
        | DesignFeature::PathPattern {
            originals, skipped, ..
        }
        | DesignFeature::MultiTransform {
            originals, skipped, ..
        } => {
            let originals = original_ops(document, feature_ops, originals).map_err(&fail)?;
            let steps = occurrence_steps(document, feature).map_err(&fail)?;
            if skipped.contains(&0) {
                let starts_body = originals.iter().any(|&op| {
                    plan.ops.get(op).and_then(SolidOp::boolean_op) == Some(BooleanOp::NewSolid)
                });
                if starts_body {
                    return Err(fail(
                        "the body's first solid cannot be left out of a pattern: skip \
                         another occurrence"
                            .into(),
                    ));
                }
            }
            match compose_steps(&steps) {
                // A step follows a path: the chain finds its places on the
                // solid as it stands where the pattern does.
                None => plan.ops.push(SolidOp::Pattern {
                    steps,
                    originals,
                    skipped: skipped.clone(),
                }),
                Some(composed) => {
                    let transforms: Vec<Mat4> = composed
                        .into_iter()
                        .zip(0u32..)
                        .skip(1)
                        .filter(|(_, index)| !skipped.contains(index))
                        .map(|(transform, _)| transform)
                        .collect();
                    if skipped.contains(&0) {
                        plan.ops.push(SolidOp::TransformCopies {
                            transforms,
                            originals,
                        });
                    } else if !transforms.is_empty() {
                        plan.ops.push(SolidOp::Transform {
                            transforms,
                            originals,
                        });
                    }
                }
            }
        }
        // Lent geometry builds nothing; the features that take it do.
        DesignFeature::Borrow { .. } => {}
        DesignFeature::Base {} => {
            if !plan.ops.is_empty() {
                return Err(fail(
                    "the base shape can only be a body's first feature".into(),
                ));
            }
            let brep = document
                .base_brep_blob(body)
                .ok_or_else(|| fail("this body has no base shape".into()))?
                .to_vec();
            plan.ops.push(SolidOp::Shape { brep });
        }
        DesignFeature::Clone { source } => {
            if !plan.ops.is_empty() {
                return Err(fail("a clone can only be a body's first feature".into()));
            }
            let brep = document
                .imported_brep_blob(*source)
                .ok_or_else(|| {
                    fail("the source body has no built solid yet (build it first)".into())
                })?
                .to_vec();
            plan.ops.push(SolidOp::Shape { brep });
        }
        DesignFeature::Split {
            plane,
            offset,
            sketch,
            other_side,
            split: _,
        } => match sketch {
            Some(sketch) => {
                let profile = sketch_profile(document, *sketch, &[]).map_err(&fail)?;
                let through = ExtrudeTermination::ThroughAll;
                plan.ops.push(SolidOp::Sweep {
                    profile,
                    kind: SweepKind::Extrude {
                        termination: through.clone(),
                        second_side: Some(through),
                        symmetric: false,
                        reversed: false,
                        taper_deg: 0.0,
                        direction: None,
                    },
                    op: if *other_side {
                        BooleanOp::Cut
                    } else {
                        BooleanOp::Common
                    },
                });
            }
            None => {
                let (origin, normal) = split_plane(document, plane, *offset).map_err(&fail)?;
                let normal = if *other_side {
                    normal.map(|v| -v)
                } else {
                    normal
                };
                plan.ops.push(SolidOp::HalfSpace { origin, normal });
            }
        },
        DesignFeature::Piece { near } => {
            plan.ops.push(SolidOp::KeepPiece {
                near: near.map(f64::from),
            });
        }
        DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools,
            refine: _,
            boolean_tolerance: _,
        } => {
            for tool_body in std::iter::once(tool_body).chain(more_tools) {
                if *tool_body == body {
                    return Err(fail(
                        "a body cannot be its own tool; pick another body".into(),
                    ));
                }
                if tools_reach(document, *tool_body, body) {
                    return Err(fail(
                        "the tool body takes this body as a tool in turn; one of the two \
                     has to go"
                            .into(),
                    ));
                }
                if document.body(*tool_body).is_none() {
                    return Err(fail(
                        "the tool body is not in this document; pick another body".into(),
                    ));
                }
                let tool_brep = document
                    .imported_brep_blob(*tool_body)
                    .ok_or_else(|| {
                        fail("the tool body has no built solid yet (build it first)".into())
                    })?
                    .to_vec();
                // The tool's shape is in its own body's frame; it meets this
                // body's where the two bodies sit.
                let relative = document
                    .body_placement(body)
                    .inverse()
                    .after(&document.body_placement(*tool_body));
                plan.ops.push(SolidOp::Boolean {
                    tool_brep,
                    kind: *kind,
                    tool_transform: (!relative.is_identity()).then(|| relative.rows()),
                });
            }
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

/// Resolve pattern `originals` (feature ids) into chain op indices. An empty
/// list means "transform the whole body".
/// A feature that keeps only what it shares with the body cannot be
/// repeated: each copy would keep only what it shares with the last.
fn original_ops(
    document: &Document,
    feature_ops: &std::collections::HashMap<FeatureId, Vec<usize>>,
    originals: &[FeatureId],
) -> Result<Vec<usize>, String> {
    let mut indices = Vec::new();
    for original in originals {
        let keeps_inside = document
            .feature_as::<DesignFeature>(*original)
            .is_some_and(|feature| feature.keeps_inside());
        if keeps_inside {
            let name = document
                .get_feature_meta(*original)
                .map_or("an original", |n| n.name.as_str());
            return Err(format!(
                "{name} keeps only what it shares with the body, which a pattern cannot \
                 repeat; cut it away instead to pattern it"
            ));
        }
        let ops = feature_ops
            .get(original)
            .ok_or("a pattern original is missing or comes after the pattern (or is suppressed)")?;
        indices.extend_from_slice(ops);
    }
    Ok(indices)
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
/// Where a datum's origin stands in the body's frame: a datum point, or
/// the origin of any other datum.
fn datum_origin(document: &Document, datum: FeatureId) -> Result<[f64; 3], String> {
    let data = document
        .feature_values(datum)
        .ok_or("the datum it scales about is gone")?;
    let made = core_document::DatumFeature::from_json(data)
        .map_err(|_| "what it scales about is not a datum".to_string())?;
    Ok(made.frame().origin.map(f64::from))
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

/// A picked edge as the kernel finds it again.
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
mod tests {
    use super::patterns::{
        circular_transforms, multi_transforms, pattern_axis, point_transforms, polar_transforms,
    };
    use super::*;
    use crate::feature::MirrorPlane;
    use crate::feature::{ExtrudeMode, HelixMode, PatternAxis, PipeOrientation, TransformStep};
    use crate::hole_tables::{ScrewSeat, ThreadStandard};
    use crate::references::retarget_feature_sketch;
    use crate::test_support::{HoleFields, bare_geometry, hole, pad, set_hole};
    use kernel_api::ProfileSegment;
    use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, Vec2D};

    fn rect_sketch() -> SketchFeature {
        crate::test_support::rect_sketch(10.0, 5.0)
    }

    fn pocket(sketch: FeatureId, depth: f32, reversed: bool, through_all: bool) -> DesignFeature {
        DesignFeature::Pocket {
            regions: Vec::new(),
            keep_inside: false,
            profile_borrowed: None,
            extras: Default::default(),
            refine: false,
            boolean_tolerance: None,
            sketch: Some(sketch),
            depth,
            reversed,
            symmetric: false,
            through_all,
            mode: ExtrudeMode::Dimension,
            depth2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
            profile_face: None,
            direction: Default::default(),
            up_to_shape: Vec::new(),
            mode2: None,
            up_to_face2: None,
            up_to_offset2: 0.0,
            up_to_shape2: Vec::new(),
        }
    }

    fn doc_with_body_sketch() -> (Document, BodyId, FeatureId) {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let sketch_id = add(&mut doc, body, rect_sketch(), "sketch");
        (doc, body, sketch_id)
    }

    /// `feature` added to `body` as `name`.
    fn add(
        doc: &mut Document,
        body: BodyId,
        feature: impl core_document::WorkbenchFeature,
        name: &str,
    ) -> FeatureId {
        doc.add_feature_in_body(feature, name.into(), Some(body))
            .unwrap()
    }

    fn extrude_distance(op: &SolidOp) -> f64 {
        match op {
            SolidOp::Sweep {
                kind:
                    SweepKind::Extrude {
                        termination: ExtrudeTermination::Blind { distance },
                        ..
                    },
                ..
            } => *distance,
            _ => panic!("not a blind extrude: {op:?}"),
        }
    }

    fn boolean_of(op: &SolidOp) -> BooleanOp {
        op.boolean_op().expect("shape-producing op")
    }

    /// `originals` repeated along X over 30 mm, the `skipped` occurrences
    /// left out.
    fn linear_pattern(
        originals: Vec<FeatureId>,
        occurrences: u32,
        skipped: Vec<u32>,
    ) -> DesignFeature {
        DesignFeature::LinearPattern {
            skipped,
            refine: false,
            boolean_tolerance: None,
            originals,
            axis: PatternAxis::X,
            length: 30.0,
            occurrences,
            spacing_mode: false,
            spacings: Vec::new(),
            reversed: false,
        }
    }

    /// A sketch of `points` in `body`.
    fn points_sketch(doc: &mut Document, body: BodyId, points: &[(f64, f64)]) -> FeatureId {
        let mut sketch = Sketch::new("points");
        for &(x, y) in points {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
        }
        add_sketch(doc, body, sketch)
    }

    fn add_sketch(doc: &mut Document, body: BodyId, sketch: Sketch) -> FeatureId {
        let plane = sketch.plane;
        let name = sketch.name.clone();
        doc.add_feature_in_body(SketchFeature::new(sketch, plane), name, Some(body))
            .unwrap()
    }

    /// A body whose geometry names a source file is an import, with no
    /// history to rebuild; a rebuild's own result names none, and the body
    /// rebuilds from its features.
    #[test]
    fn only_an_imported_solid_stops_a_rebuild_from_features() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        doc.set_imported_geometry(body, bare_geometry(None));
        assert!(!doc.body_solid_is_imported(body));
        assert!(body_build_ops(&doc, body).is_ok(), "rebuilds normally");

        doc.set_imported_geometry(body, bare_geometry(Some(uuid::Uuid::new_v4())));
        let err = body_build_ops(&doc, body).expect_err("the import has no history to rebuild");
        assert_eq!(err.feature, Some(pad_id));
        assert!(err.message.contains("import"), "{}", err.message);
    }

    #[test]
    fn second_pad_fuses_and_pocket_cuts() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        for feature in [
            pad(sketch_id, 5.0),
            pad(sketch_id, 2.0),
            pocket(sketch_id, 3.0, false, false),
        ] {
            let label = feature.kind_label().to_string();
            doc.add_feature_in_body(feature, label, Some(body)).unwrap();
        }
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 3);
        assert_eq!(plan.op_features.len(), 3);
        assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
        assert!((extrude_distance(&plan.ops[0]) - 5.0).abs() < 1e-9);
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Fuse);
        assert_eq!(boolean_of(&plan.ops[2]), BooleanOp::Cut);
        // The default pocket cuts against the sketch normal via `reversed`.
        assert!(matches!(
            &plan.ops[2],
            SolidOp::Sweep {
                kind: SweepKind::Extrude { reversed: true, .. },
                ..
            }
        ));
    }

    #[test]
    fn pocket_first_is_an_error() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(
            &mut doc,
            body,
            pocket(sketch_id, 3.0, false, false),
            "Pocket",
        );
        let err = body_build_ops(&doc, body).unwrap_err();
        assert!(err.message.contains("material"), "{}", err.message);
    }

    #[test]
    fn rebuild_jobs_settle_the_flags_of_the_plan_and_its_inputs_only() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        // A sketch of another body, dirty on its own: not this plan's input.
        let other = doc.create_body(None);
        let other_sketch = add(&mut doc, other, rect_sketch(), "Other");
        doc.mark_feature_dirty(other_sketch);
        doc.mark_feature_dirty(sketch_id);

        let jobs = rebuild_jobs(&mut doc);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].body, body);
        assert!(jobs[0].plan.as_ref().is_ok_and(|p| !p.ops.is_empty()));
        assert!(!doc.get_feature_meta(pad_id).unwrap().dirty);
        assert!(!doc.get_feature_meta(sketch_id).unwrap().dirty);
        assert!(doc.get_feature_meta(other_sketch).unwrap().dirty);
        assert!(rebuild_jobs(&mut doc).is_empty(), "nothing comes back");
    }

    /// A pattern of one occurrence is the original alone: it builds, and
    /// adds nothing. None at all is refused.
    #[test]
    fn a_pattern_of_one_occurrence_adds_nothing() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        let before = body_build_ops(&doc, body).unwrap().ops.len();
        let pattern = |occurrences| linear_pattern(vec![pad_id], occurrences, Vec::new());
        let id = add(&mut doc, body, pattern(1), "Pattern");
        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), before);
        doc.update_feature_data(id, pattern(0).to_json()).unwrap();
        let error = body_build_ops(&doc, body).unwrap_err();
        assert!(error.message.contains("at least 1"), "{}", error.message);

        // Rebuilding, the plan stops at the pattern: the pad before it
        // builds, and a pad after it is left out.
        let later = add(&mut doc, body, pad(sketch_id, 9.0), "Pad 2");
        let plan = body_plan(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), before, "the pad before the pattern");
        assert_eq!(plan.failed.map(|e| e.feature), Some(Some(id)));
        assert_eq!(plan.unbuilt, vec![later]);
    }

    /// A refined feature is followed by a Refine op that answers to it,
    /// and a pattern of that feature re-runs its tool, not the refine.
    #[test]
    fn a_refined_feature_is_followed_by_a_refine_its_pattern_skips() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let mut padded = pad(sketch_id, 7.0);
        padded.set_refine(true);
        let pad_id = add(&mut doc, body, padded, "Pad");
        let plain = body_build_ops(&doc, body).unwrap();
        assert!(matches!(plain.ops.last(), Some(SolidOp::Refine)));
        assert_eq!(plain.op_features.last(), Some(&pad_id));
        assert_eq!(plain.ops.len(), plain.op_features.len());

        let pattern = linear_pattern(vec![pad_id], 2, Vec::new());
        add(&mut doc, body, pattern, "Pattern");
        let patterned = body_build_ops(&doc, body).unwrap();
        match patterned.ops.last() {
            Some(SolidOp::Transform { originals, .. }) => {
                assert_eq!(originals, &vec![0], "the pad's own op, not its refine");
            }
            other => panic!("the pattern ends the chain, not {other:?}"),
        }

        let mut unrefined = pad(sketch_id, 7.0);
        unrefined.set_refine(false);
        assert!(!unrefined.refine());
        assert!(unrefined.can_refine());
        assert!(
            !DesignFeature::Fillet {
                radius: 1.0,
                edges: crate::feature::EdgeSel::All,
                follow_tangent: false,
            }
            .can_refine()
        );
    }

    /// A stated boolean tolerance wraps each op the feature makes, in its
    /// place, so a pattern still counts the original's op and the refine
    /// after it runs as it is.
    #[test]
    fn a_stated_tolerance_wraps_the_feature_s_ops_in_place() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch_id, 7.0), "Base");
        let mut padded = pad(sketch_id, 3.0);
        padded.set_refine(true);
        padded.set_boolean_tolerance(Some(0.05));
        let pad_id = add(&mut doc, body, padded, "Pad");
        let mut pattern = linear_pattern(vec![pad_id], 2, Vec::new());
        pattern.set_boolean_tolerance(Some(0.02));
        add(&mut doc, body, pattern, "Pattern");
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), plan.op_features.len());
        assert_eq!(
            plan.ops[0].boolean_tolerance(),
            None,
            "the base states none"
        );
        assert_eq!(plan.ops[1].boolean_tolerance(), Some(0.05_f32.into()));
        assert!(matches!(plan.ops[1].inner(), SolidOp::Sweep { .. }));
        assert!(matches!(plan.ops[2], SolidOp::Refine));
        assert_eq!(plan.ops[3].boolean_tolerance(), Some(0.02_f32.into()));
        match plan.ops[3].inner() {
            SolidOp::Transform { originals, .. } => assert_eq!(originals, &vec![1]),
            other => panic!("the pattern ends the chain, not {other:?}"),
        }
    }

    #[test]
    fn picked_edges_reach_the_kernel_as_probe_points() {
        let picks = vec![
            crate::feature::EdgePick {
                faces: [0, 0],
                point: [1.0, 2.0, 3.0],
                direction: [1.0, 0.0, 0.0],
            },
            crate::feature::EdgePick {
                faces: [0, 0],
                point: [4.0, 5.0, 6.0],
                direction: [0.0, 1.0, 0.0],
            },
        ];
        match edge_selection(&crate::feature::EdgeSel::Edges(picks)).unwrap() {
            EdgeSelection::Picked(probes) => {
                let points: Vec<[f64; 3]> = probes.iter().map(|p| p.point).collect();
                assert_eq!(points, vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
                assert_eq!(probes[1].direction, [0.0, 1.0, 0.0]);
            }
            other => panic!("picked edges map to probes, not {other:?}"),
        }
    }

    #[test]
    fn deleting_a_feature_reveals_its_sketch_and_restarts_the_body() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        let pocket_id = add(
            &mut doc,
            body,
            pocket(sketch_id, 2.0, false, false),
            "Pocket",
        );
        doc.set_feature_visible(sketch_id, false);
        doc.clear_feature_dirty(pad_id);
        doc.clear_feature_dirty(pocket_id);

        assert!(delete_feature(&mut doc, pocket_id));
        assert!(doc.get_feature_meta(pocket_id).is_none());
        assert!(doc.get_feature_meta(sketch_id).unwrap().visible);
        assert!(
            doc.get_feature_meta(pad_id).unwrap().dirty,
            "the body restarts"
        );
        assert!(!delete_feature(&mut doc, pocket_id), "gone already");
    }

    #[test]
    fn invalidating_a_body_restarts_its_history_and_leaves_clearing_to_the_registry() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        doc.clear_feature_dirty(pad_id);
        invalidate_body(&mut doc, body);
        assert!(doc.get_feature_meta(pad_id).unwrap().dirty);

        doc.remove_feature(pad_id).unwrap();
        doc.set_imported_geometry(body, bare_geometry(None));
        invalidate_body(&mut doc, body);
        assert!(
            doc.imported_geometry(body).is_some(),
            "another bench's features may build it: the registry clears it"
        );
    }

    #[test]
    fn dirty_pad_flags_body_for_rebuild_and_sketch_edit_propagates() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        assert!(pending_body_rebuilds(&doc).is_empty());

        doc.mark_feature_dirty(sketch_id);
        assert_eq!(pending_body_rebuilds(&doc), vec![body]);

        doc.clear_feature_dirty(pad_id);
        doc.clear_feature_dirty(sketch_id);
        assert!(pending_body_rebuilds(&doc).is_empty());
    }

    /// A suppressed feature and the features after the body's tip are left
    /// out of the build.
    #[test]
    fn suppressed_features_and_those_after_the_tip_are_skipped_in_build() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_a = add(&mut doc, body, pad(sketch_id, 5.0), "PadA");
        add(&mut doc, body, pad(sketch_id, 9.0), "PadB");

        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
        doc.set_feature_suppressed(pad_a, true);
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 1);
        // The remaining pad becomes the first op → NewSolid.
        assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
        assert!((extrude_distance(&plan.ops[0]) - 9.0).abs() < 1e-9);

        doc.set_feature_suppressed(pad_a, false);
        doc.set_body_tip(body, Some(pad_a));
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 1, "features after the tip are excluded");
        assert!((extrude_distance(&plan.ops[0]) - 5.0).abs() < 1e-9);
        doc.set_body_tip(body, None);
        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
    }

    #[test]
    fn revolution_and_groove_map_to_revolve_kind() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            DesignFeature::Revolution {
                regions: Vec::new(),
                refine: false,
                boolean_tolerance: None,
                sketch: sketch_id,
                angle_deg: 270.0,
                axis: RevolveAxis::SketchY,
                project_axis: false,
                reversed: false,
                midplane: false,
                second_angle_deg: None,
                start_angle_deg: 0.0,
                mode: Default::default(),
                up_to_face: None,
            },
            "Revolution".into(),
            Some(body),
        )
        .unwrap();
        doc.add_feature_in_body(
            DesignFeature::Groove {
                regions: Vec::new(),
                keep_inside: false,
                refine: false,
                boolean_tolerance: None,
                sketch: sketch_id,
                angle_deg: 90.0,
                axis: RevolveAxis::SketchX,
                project_axis: false,
                reversed: true,
                midplane: false,
                second_angle_deg: None,
                start_angle_deg: 0.0,
                mode: Default::default(),
                up_to_face: None,
            },
            "Groove".into(),
            Some(body),
        )
        .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 2);
        assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
        assert!(matches!(
            &plan.ops[0],
            SolidOp::Sweep {
                kind: SweepKind::Revolve { axis_dir: [x, y], angle_deg, .. },
                ..
            } if x.abs() < 1e-9 && (y - 1.0).abs() < 1e-9 && (angle_deg - 270.0).abs() < 1e-9
        ));
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Cut);
        assert!(matches!(
            &plan.ops[1],
            SolidOp::Sweep {
                kind: SweepKind::Revolve { reversed: true, .. },
                ..
            }
        ));
    }

    /// A through-all pocket cuts through all; centred on the sketch, it
    /// cuts through all both ways.
    #[test]
    fn a_through_all_pocket_cuts_through_all_and_a_symmetric_one_both_ways() {
        for symmetric in [false, true] {
            let (mut doc, body, sketch_id) = doc_with_body_sketch();
            add(&mut doc, body, pad(sketch_id, 5.0), "Pad");
            let mut through = pocket(sketch_id, 1.0, false, true);
            if let DesignFeature::Pocket { symmetric: s, .. } = &mut through {
                *s = symmetric;
            }
            add(&mut doc, body, through, "Pocket");
            let plan = body_build_ops(&doc, body).unwrap();
            let SolidOp::Sweep {
                kind:
                    SweepKind::Extrude {
                        termination,
                        second_side,
                        ..
                    },
                ..
            } = &plan.ops[1]
            else {
                panic!("not an extrude: {:?}", plan.ops[1]);
            };
            assert_eq!(
                *termination,
                ExtrudeTermination::ThroughAll,
                "symmetric {symmetric}"
            );
            if symmetric {
                assert_eq!(*second_side, Some(ExtrudeTermination::ThroughAll));
            }
        }
    }

    #[test]
    fn hole_feature_emits_cut_circles() {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        add(&mut doc, body, pad(base_sketch, 5.0), "Pad");

        let mut hole_sketch = Sketch::new("holes");
        for x in [2.0f64, 8.0] {
            let center =
                hole_sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 2.5))));
            hole_sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
        }
        let hole_sketch_id = add_sketch(&mut doc, body, hole_sketch);
        let counterbored = hole(hole_sketch_id, |h| {
            *h.depth = 4.0;
            *h.cut = HoleCut::Counterbore {
                diameter: 6.0,
                depth: 1.5,
            };
        });
        add(&mut doc, body, counterbored, "Hole");

        let plan = body_build_ops(&doc, body).unwrap();
        // Pad + hole drill + counterbore.
        assert_eq!(plan.ops.len(), 3);
        assert_eq!(plan.op_features[1], plan.op_features[2]);
        for op in &plan.ops[1..] {
            assert_eq!(boolean_of(op), BooleanOp::Cut);
            let SolidOp::Sweep { profile, .. } = op else {
                panic!("hole ops are sweeps");
            };
            assert_eq!(profile.wires.len(), 2, "one wire per hole center");
        }
    }

    #[test]
    fn a_body_is_never_its_own_boolean_tool() {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        add(&mut doc, body, pad(base_sketch, 5.0), "Pad");
        doc.add_feature_in_body(
            DesignFeature::BodyBoolean {
                more_tools: Vec::new(),
                refine: false,
                boolean_tolerance: None,
                tool_body: body,
                kind: kernel_api::BoolKind::Cut,
            },
            "Boolean".into(),
            Some(body),
        )
        .unwrap();
        let error = body_build_ops(&doc, body).unwrap_err();
        assert!(error.message.contains("its own tool"), "{}", error.message);
    }

    /// A Boolean with two tool bodies is planned once and then left alone
    /// until one of its tools changes.
    #[test]
    fn a_boolean_with_two_tools_rebuilds_once() {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        add(&mut doc, body, pad(base_sketch, 5.0), "Pad");
        let tools: Vec<BodyId> = (0..2)
            .map(|_| {
                let tool = doc.create_body(None);
                doc.set_imported_geometry(tool, bare_geometry(None));
                tool
            })
            .collect();
        doc.add_feature_in_body(
            DesignFeature::BodyBoolean {
                more_tools: vec![tools[1]],
                refine: false,
                boolean_tolerance: None,
                tool_body: tools[0],
                kind: kernel_api::BoolKind::Cut,
            },
            "Boolean".into(),
            Some(body),
        )
        .unwrap();
        let first = rebuild_jobs(&mut doc);
        assert!(first.iter().any(|j| j.body == body));
        let again = rebuild_jobs(&mut doc);
        assert!(
            again.iter().all(|j| j.body != body),
            "nothing changed, nothing to rebuild"
        );
    }

    /// An M6 hole 10 deep, its thread modeled `length` long or cleared
    /// with `clearance`; the ops it makes.
    fn m6_hole(
        threaded: bool,
        length: crate::feature::ThreadLength,
        clearance: Option<f32>,
    ) -> (DesignFeature, Vec<SolidOp>) {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        add(&mut doc, body, pad(base_sketch, 20.0), "Pad");
        let sketch = points_sketch(&mut doc, body, &[(2.0, 2.5)]);
        let m6 = hole(sketch, |h| {
            *h.clearance = clearance;
            *h.thread_length = length;
            *h.depth = 10.0;
            *h.thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6"));
            *h.threaded = threaded;
            *h.modeled_thread = threaded;
            *h.thread_depth = 4.0;
        });
        let ops = hole_ops(&doc, &m6).unwrap();
        (m6, ops)
    }

    /// A 5 mm hole at `sketch`'s points showing an M6 thread, as `edit`
    /// changes it.
    fn cosmetic_m6(sketch: FeatureId, edit: impl FnOnce(&mut HoleFields)) -> DesignFeature {
        hole(sketch, |h| {
            *h.diameter = 5.0;
            *h.thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6"));
            *h.threaded = true;
            *h.cosmetic_thread = true;
            edit(h);
        })
    }

    /// The height of the thread's helix: its length and one pitch.
    fn thread_run(ops: &[SolidOp]) -> f64 {
        ops.iter()
            .find_map(|op| match op {
                SolidOp::Sweep {
                    kind: SweepKind::Helix { height, .. },
                    ..
                } => Some(*height),
                _ => None,
            })
            .expect("a thread")
    }

    #[test]
    fn a_mirror_takes_a_datum_or_a_sketchs_plane_and_axes() {
        use crate::feature::{MirrorPlane, SketchAxis};
        use core_document::{
            AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape,
        };
        let (mut doc, body, sketch) = doc_with_body_sketch();
        let system = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::CoordinateSystem { size: 10.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::XY),
                    offset: AttachmentOffset {
                        translation: [3.0, 0.0, 0.0],
                        ..Default::default()
                    },
                },
                "CS".into(),
                Some(body),
            )
            .unwrap();
        let (p, n) = mirror_plane(
            &doc,
            &MirrorPlane::Datum {
                datum: system,
                plane: Some(BasePlane::YZ),
            },
        )
        .unwrap();
        assert_eq!((p, n.map(f64::abs)), ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        // The rectangle's sketch lies on XY: its plane, and the plane
        // through its horizontal axis square to it.
        let (_, n) = mirror_plane(&doc, &MirrorPlane::Sketch { sketch, axis: None }).unwrap();
        assert_eq!(n.map(f64::abs), [0.0, 0.0, 1.0]);
        let (_, n) = mirror_plane(
            &doc,
            &MirrorPlane::Sketch {
                sketch,
                axis: Some(SketchAxis::Horizontal),
            },
        )
        .unwrap();
        assert_eq!(n.map(f64::abs), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_draft_stands_on_a_plane_and_pulls_along_a_datum_line() {
        use crate::feature::{PlaneTarget, PullRef};
        use core_document::{BasePlane, DatumAttachment, DatumFeature, DatumShape};
        let (mut doc, body, sketch) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch, 5.0), "Pad");
        let line = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::Line { length: 10.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::YZ),
                    offset: Default::default(),
                },
                "Line".into(),
                Some(body),
            )
            .unwrap();
        doc.add_feature_in_body(
            DesignFeature::Draft {
                angle_deg: 5.0,
                neutral: FacePick {
                    point: [0.0; 3],
                    normal: [1.0, 0.0, 0.0],
                    name: 0,
                },
                faces: vec![FacePick {
                    point: [0.0, 2.5, 2.5],
                    normal: [-1.0, 0.0, 0.0],
                    name: 0,
                }],
                reversed: false,
                neutral_plane: Some(PlaneTarget::Base(BasePlane::XY)),
                pull: Some(PullRef::Datum(line)),
            },
            "Draft".into(),
            Some(body),
        )
        .unwrap();
        let ops = body_build_ops(&doc, body).unwrap().ops;
        let Some(SolidOp::Draft {
            neutral_normal,
            pull_dir,
            ..
        }) = ops.iter().find(|op| matches!(op, SolidOp::Draft { .. }))
        else {
            panic!("{ops:?}");
        };
        assert_eq!(*neutral_normal, [0.0, 0.0, 1.0]);
        // The YZ plane's line runs along its x-axis, the world's y.
        assert_eq!(pull_dir.map(|d| d.map(f64::abs)), Some([0.0, 1.0, 0.0]));
    }

    #[test]
    fn a_modeled_thread_runs_as_long_as_asked() {
        use crate::feature::ThreadLength;
        // M6 coarse: pitch 1.
        let given = thread_run(&m6_hole(true, ThreadLength::Given, None).1);
        let whole = thread_run(&m6_hole(true, ThreadLength::HoleDepth, None).1);
        let run_out = thread_run(&m6_hole(true, ThreadLength::RunOut, None).1);
        assert!((given - 5.0).abs() < 1e-9, "{given}");
        assert!((whole - 11.0).abs() < 1e-9, "{whole}");
        assert!((run_out - 8.0).abs() < 1e-9, "{run_out}");
    }

    /// A threaded metric hole drills the table's tap size, a clearance hole
    /// the fit's size, or a clearance of its own in the fit's place.
    #[test]
    fn a_metric_hole_takes_its_size_from_the_table_or_a_clearance_of_its_own() {
        use crate::feature::ThreadLength;
        let (tapped, _) = m6_hole(true, ThreadLength::Given, None);
        assert!((hole_diameter(&tapped) - 5.0).abs() < 1e-6, "M6 tap drill");
        let (normal, _) = m6_hole(false, ThreadLength::Given, None);
        assert!(
            (hole_diameter(&normal) - 6.6).abs() < 1e-4,
            "ISO 273 normal"
        );
        let (own, _) = m6_hole(false, ThreadLength::Given, Some(6.3));
        assert!((hole_diameter(&own) - 6.3).abs() < 1e-6);
    }

    /// A cosmetic M6 thread down a 10 mm hole: a helix per hole of the
    /// thread's 1 mm pitch, ten turns, just inside the 5 mm tap bore, and
    /// its designation in the hole's name; none without the switch.
    #[test]
    fn a_cosmetic_thread_draws_a_helix_down_the_bore() {
        let (mut doc, body, _) = doc_with_body_sketch();
        let holes = points_sketch(&mut doc, body, &[(3.0, 4.0)]);
        let mut hole = cosmetic_m6(holes, |h| *h.depth = 10.0);
        assert_eq!(hole.cosmetic_thread().as_deref(), Some("M6"));
        let lines = cosmetic_thread_lines(&doc, Some(body), &hole);
        assert_eq!(lines.edges.len() / 2, 10 * holes::THREAD_SEGMENTS_PER_TURN);
        let bore = f64::from(hole_diameter(&hole)) * 0.5;
        for p in &lines.positions {
            let r = (f64::from(p[0]) - 3.0).hypot(f64::from(p[1]) - 4.0);
            assert!(
                r < bore && r > bore * 0.95,
                "just inside the bore: {r} of {bore}"
            );
            assert!((-10.0..=0.0).contains(&p[2]), "down the hole: {}", p[2]);
        }
        set_hole(&mut hole, |h| *h.cosmetic_thread = false);
        assert!(
            cosmetic_thread_lines(&doc, Some(body), &hole)
                .positions
                .is_empty()
        );

        // The document keeps the thread once evaluated: its designation,
        // where its bore opens and which way it runs, until the hole
        // leaves the build.
        set_hole(&mut hole, |h| *h.cosmetic_thread = true);
        let id = add(&mut doc, body, hole.clone(), "Hole");
        let mut registry = core_document::DocumentService::default();
        registry
            .register_workbench(Box::new(crate::DesignWorkbench::default()))
            .unwrap();
        registry.evaluate(&mut doc);
        let threads = doc.cosmetic_threads();
        assert_eq!(threads.len(), 1, "{threads:?}");
        let thread = &threads[0];
        assert_eq!((thread.feature, thread.body), (id, body));
        assert_eq!(thread.label(), "M6 thread");
        assert_eq!(thread.mouth, [3.0, 4.0, 0.0]);
        assert_eq!(thread.axis, [0.0, 0.0, -1.0]);
        assert!((thread.radius - f64::from(hole_diameter(&hole)) * 0.5).abs() < 1e-9);
        doc.set_feature_suppressed(id, true);
        registry.evaluate(&mut doc);
        assert!(doc.cosmetic_threads().is_empty(), "a suppressed hole");
    }

    /// A mirror or a pattern threads its copies of a threaded hole, across
    /// a mirror cut to the same hand; a skipped copy has none, and a mirror
    /// of the whole body takes the hole too.
    #[test]
    fn copies_of_a_threaded_hole_are_threaded() {
        use crate::feature::MirrorPlane;
        let (mut doc, body, _) = doc_with_body_sketch();
        let holes = points_sketch(&mut doc, body, &[(3.0, 4.0)]);
        let hole = add(
            &mut doc,
            body,
            cosmetic_m6(holes, |h| *h.depth = 10.0),
            "Hole",
        );
        let mirror = |originals: Vec<FeatureId>, skipped: Vec<u32>| DesignFeature::Mirrored {
            originals,
            plane: MirrorPlane::YZ,
            refine: false,
            boolean_tolerance: None,
            skipped,
        };
        let copy = add(&mut doc, body, mirror(vec![hole], vec![]), "Mirrored");
        let bores = threaded_bores_of(&doc, body, copy);
        assert_eq!(bores.len(), 1, "{bores:?}");
        assert_eq!((bores[0].hole, bores[0].mouth), (hole, [-3.0, 4.0, 0.0]));
        assert_eq!(bores[0].into, [0.0, 0.0, -1.0]);
        // Which way each helix turns about its bore, looking down it.
        let hand = |lines: &kernel_api::TriMesh, centre: [f64; 2]| {
            let at = |i: usize| {
                let p = lines.positions[i];
                (f64::from(p[0]) - centre[0], f64::from(p[1]) - centre[1])
            };
            let ((ax, ay), (bx, by)) = (at(0), at(1));
            (ax * by - ay * bx).signum()
        };
        let own = cosmetic_thread_lines(
            &doc,
            Some(body),
            &DesignFeature::from_json(doc.feature_values(hole).unwrap()).unwrap(),
        );
        let copied = copied_thread_lines(&doc, body, copy);
        assert_eq!(copied.edges.len(), own.edges.len());
        assert_eq!(
            hand(&copied, [-3.0, 4.0]),
            hand(&own, [3.0, 4.0]),
            "the same hand"
        );

        let skipped = add(&mut doc, body, mirror(vec![hole], vec![1]), "Skipped");
        assert!(threaded_bores_of(&doc, body, skipped).is_empty());
        let whole = add(&mut doc, body, mirror(vec![], vec![]), "Whole body");
        let bores = threaded_bores_of(&doc, body, whole);
        assert!(
            bores.iter().any(|b| b.mouth == [-3.0, 4.0, 0.0])
                && bores.iter().any(|b| b.mouth == [3.0, 4.0, 0.0]),
            "the hole and the first mirror's copy, each mirrored: {bores:?}"
        );
    }

    /// A copy is threaded where the solid shows its bore: a rim of the
    /// bore's radius round its mouth, not merely a solid face there.
    #[test]
    fn a_copy_is_threaded_where_the_solid_shows_its_bore() {
        let bore = Bore {
            hole: FeatureId(uuid::Uuid::nil()),
            mouth: [10.0, 0.0, 5.0],
            into: [0.0, 0.0, -1.0],
            across: [1.0, 0.0, 0.0],
        };
        let rim = |r: f32| kernel_api::TriMesh {
            positions: (0..12)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 12.0;
                    [10.0 + r * a.cos(), r * a.sin(), 5.0]
                })
                .collect(),
            ..Default::default()
        };
        assert!(bore_drawn(&rim(1.25), &bore, 1.25));
        assert!(!bore_drawn(&rim(4.0), &bore, 1.25), "another round there");
        assert!(!bore_drawn(&kernel_api::TriMesh::default(), &bore, 1.25));
    }

    /// A cosmetic thread through all of an 8 mm plate stops at the plate's
    /// far side, though the plate is far wider than it is thick: at the
    /// bore's exit once the bore is cut, at the solid's far face along the
    /// hole before, and a thread depth longer than the hole stops there too.
    #[test]
    fn a_cosmetic_thread_through_all_stops_where_the_hole_leaves_the_solid() {
        let (mut doc, body, _) = doc_with_body_sketch();
        let holes = points_sketch(&mut doc, body, &[(3.0, 4.0)]);
        let hole = |thread_depth: f32| {
            cosmetic_m6(holes, |h| {
                *h.depth = 0.0;
                *h.through_all = true;
                *h.thread_depth = thread_depth;
            })
        };
        let plate = |bored: bool| {
            let mut positions = Vec::new();
            for corner in 0..8 {
                positions.push([
                    if corner & 1 == 0 { -50.0 } else { 50.0 },
                    if corner & 2 == 0 { -50.0 } else { 50.0 },
                    if corner & 4 == 0 { -8.0 } else { 0.0 },
                ]);
            }
            if bored {
                for k in 0..16 {
                    let (sin, cos) = (std::f32::consts::TAU * k as f32 / 16.0).sin_cos();
                    for z in [0.0, -8.0] {
                        positions.push([3.0 + 2.5 * cos, 4.0 + 2.5 * sin, z]);
                    }
                }
            }
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh {
                    positions,
                    ..Default::default()
                }),
                revision: 1,
                bounds_mm: Some(([-50.0, -50.0, -8.0], [50.0, 50.0, 0.0])),
                ..bare_geometry(None)
            }
        };
        for bored in [true, false] {
            doc.set_imported_geometry(body, plate(bored));
            for thread_depth in [0.0, 30.0] {
                let lines = cosmetic_thread_lines(&doc, Some(body), &hole(thread_depth));
                assert!(!lines.positions.is_empty());
                let deepest = lines
                    .positions
                    .iter()
                    .map(|p| p[2])
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    (deepest + 8.0).abs() < 1e-3,
                    "bored {bored}, thread depth {thread_depth}: ends at {deepest}, not -8"
                );
                assert!(lines.positions.iter().all(|p| p[2] <= 1e-4));
            }
        }
    }

    /// A size the table does not have is an error on the hole, whatever
    /// its thread settings, never a panic.
    #[test]
    fn a_hole_of_a_size_the_table_lacks_fails_cleanly() {
        for (threaded, modeled_thread) in [(false, false), (true, false), (true, true)] {
            let error = hole_plan(|h| {
                *h.thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M999"));
                *h.threaded = threaded;
                *h.modeled_thread = modeled_thread;
                *h.thread_depth = 3.0;
            })
            .unwrap_err();
            assert!(
                error.contains("no ISO metric coarse size \"M999\""),
                "threaded {threaded}, modeled {modeled_thread}: {error}"
            );
        }
    }

    /// A padded body with one hole position and a hole, as `edit` changes
    /// it, drilled there: the hole's ops, or its build error.
    fn hole_plan(edit: impl FnOnce(&mut HoleFields)) -> Result<Vec<SolidOp>, String> {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        add(&mut doc, body, pad(base_sketch, 10.0), "Pad");
        let holes = points_sketch(&mut doc, body, &[(2.0, 2.5)]);
        add(&mut doc, body, hole(holes, edit), "Hole");
        body_build_ops(&doc, body)
            .map(|plan| plan.ops[1..].to_vec())
            .map_err(|e| e.message)
    }

    fn blind_distance(op: &SolidOp) -> (f64, f64) {
        let SolidOp::Sweep {
            kind:
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance },
                    taper_deg,
                    ..
                },
            ..
        } = op
        else {
            panic!("a blind extrude: {op:?}");
        };
        (*distance, *taper_deg)
    }

    fn cone_of(op: &SolidOp) -> (f64, f64, f64, [f64; 3]) {
        let SolidOp::Primitive {
            kind:
                kernel_api::PrimitiveKind::Cone {
                    radius1,
                    radius2,
                    height,
                    ..
                },
            placement,
            op,
        } = op
        else {
            panic!("a cone: {op:?}");
        };
        assert_eq!(*op, BooleanOp::Cut);
        (*radius1, *radius2, *height, placement.origin)
    }

    #[test]
    fn an_angled_point_adds_a_cone_below_the_wall_or_within_the_depth() {
        let point = |in_depth: bool| {
            hole_plan(|h| {
                *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
                *h.point_in_depth = in_depth;
            })
            .unwrap()
        };
        let tip = 1.5 / 59f64.to_radians().tan();
        let below = point(false);
        assert_eq!(below.len(), 2);
        assert!((blind_distance(&below[0]).0 - 6.0).abs() < 1e-9);
        let (r1, r2, h, origin) = cone_of(&below[1]);
        assert!((r1 - 1.5).abs() < 1e-9 && r2 == 0.0 && (h - tip).abs() < 1e-9);
        assert!(
            (origin[2] + 6.0).abs() < 1e-9,
            "at the wall's end: {origin:?}"
        );
        let within = point(true);
        assert!((blind_distance(&within[0]).0 - (6.0 - tip)).abs() < 1e-9);
        let (.., origin) = cone_of(&within[1]);
        assert!((origin[2] + 6.0 - tip).abs() < 1e-9);
        // Through all has no bottom to point.
        let through = hole_plan(|h| {
            *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
            *h.through_all = true;
        })
        .unwrap();
        assert_eq!(through.len(), 1);
    }

    #[test]
    fn a_point_deeper_than_the_hole_is_an_error() {
        let error = hole_plan(|h| {
            *h.depth = 0.5;
            *h.drill_point = DrillPoint::Angled { angle_deg: 135.0 };
            *h.point_in_depth = true;
        })
        .unwrap_err();
        assert!(error.contains("drill point is deeper"), "{error}");
    }

    #[test]
    fn a_tapered_hole_narrows_its_extrude_and_its_point() {
        let ops = hole_plan(|h| {
            *h.taper_deg = 3.0;
            *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
        })
        .unwrap();
        let (distance, taper) = blind_distance(&ops[0]);
        assert!((distance - 6.0).abs() < 1e-9 && (taper + 3.0).abs() < 1e-9);
        let (r1, ..) = cone_of(&ops[1]);
        assert!((r1 - (1.5 - 6.0 * 3f64.to_radians().tan())).abs() < 1e-9);
        let closed = hole_plan(|h| *h.taper_deg = 40.0).unwrap_err();
        assert!(closed.contains("taper closes"), "{closed}");
    }

    #[test]
    fn spotface_and_counterdrill_cut_their_mouths() {
        let spot = hole_plan(|h| {
            *h.cut = HoleCut::Spotface {
                diameter: 6.0,
                depth: 0.5,
            }
        })
        .unwrap();
        assert_eq!(spot.len(), 2);
        assert!((blind_distance(&spot[1]).0 - 0.5).abs() < 1e-9);
        let drill = hole_plan(|h| {
            *h.cut = HoleCut::Counterdrill {
                diameter: 5.0,
                depth: 2.0,
                angle_deg: 90.0,
            }
        })
        .unwrap();
        assert_eq!(drill.len(), 3, "drill, bore, cone");
        assert!((blind_distance(&drill[1]).0 - 2.0).abs() < 1e-9);
        let (r1, r2, h, origin) = cone_of(&drill[2]);
        assert!((r1 - 2.5).abs() < 1e-9 && (r2 - 1.5).abs() < 1e-9 && (h - 1.0).abs() < 1e-9);
        assert!((origin[2] + 2.0).abs() < 1e-9);
        let narrow = hole_plan(|h| {
            *h.cut = HoleCut::Spotface {
                diameter: 2.0,
                depth: 0.5,
            }
        })
        .unwrap_err();
        assert!(narrow.contains("spotface diameter"), "{narrow}");
    }

    #[test]
    fn a_screw_seat_takes_its_size_from_the_metric_thread() {
        let seat = |standard: ThreadStandard, size: &str, seat: ScrewSeat| {
            hole_plan(|h| {
                *h.thread = Some(ThreadSpec::new(standard, size));
                *h.cut = HoleCut::Seat { seat };
            })
        };
        let bore = seat(ThreadStandard::IsoMetricCoarse, "M6", ScrewSeat::SocketHead).unwrap();
        assert!((blind_distance(&bore[1]).0 - 6.4).abs() < 1e-6);
        let SolidOp::Sweep { profile, .. } = &bore[1] else {
            panic!()
        };
        let ProfileSegment::Circle { radius, .. } = profile.wires[0].segments[0] else {
            panic!()
        };
        assert!((radius - 5.5).abs() < 1e-6, "DIN 974-1 Ø11");
        let sink = seat(
            ThreadStandard::IsoMetricFine,
            "M8x1",
            ScrewSeat::Countersunk,
        )
        .unwrap();
        assert_eq!(sink.len(), 2);
        let inch = seat(ThreadStandard::Unc, "1/4-20", ScrewSeat::SocketHead).unwrap_err();
        assert!(inch.contains("ISO metric"), "{inch}");
        let small = seat(
            ThreadStandard::IsoMetricCoarse,
            "M2",
            ScrewSeat::Countersunk,
        )
        .unwrap_err();
        assert!(small.contains("no ISO 10642 seat for M2"), "{small}");
    }

    #[test]
    fn a_modeled_thread_takes_its_standards_form_hand_and_taper() {
        let thread = |spec: ThreadSpec| {
            let ops = hole_plan(|h| {
                *h.thread = Some(spec);
                *h.threaded = true;
                *h.modeled_thread = true;
                *h.thread_depth = 4.0;
            })
            .unwrap();
            let Some(SolidOp::Sweep {
                profile,
                kind:
                    SweepKind::Helix {
                        pitch,
                        left_handed,
                        cone_angle_deg,
                        ..
                    },
                ..
            }) = ops.last()
            else {
                panic!("a helix last: {ops:?}");
            };
            let corners: Vec<[f64; 2]> = profile.wires[0]
                .segments
                .iter()
                .map(|s| match s {
                    ProfileSegment::Line { start, .. } => *start,
                    _ => panic!(),
                })
                .collect();
            // The flank's slope: out from the wall to the major radius, and
            // along the axis from root to crest.
            let flank = ((corners[1][1] - corners[0][1]) / (corners[1][0] - corners[0][0]))
                .atan()
                .to_degrees();
            (*pitch, *left_handed, *cone_angle_deg, corners[1][0], flank)
        };
        let (pitch, left, cone, major, flank) =
            thread(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6"));
        assert!((pitch - 1.0).abs() < 1e-9 && !left && cone == 0.0);
        assert!((major - 3.0).abs() < 1e-9 && (flank - 30.0).abs() < 1e-6);

        let mut lh = ThreadSpec::new(ThreadStandard::Bsw, "1/4");
        lh.left_handed = true;
        let (pitch, left, _, _, flank) = thread(lh);
        assert!((pitch - 25.4 / 20.0).abs() < 1e-9 && left);
        assert!((flank - 27.5).abs() < 1e-6, "55° Whitworth form: {flank}");

        let (_, _, cone, major, _) = thread(ThreadSpec::new(ThreadStandard::Npt, "1/2"));
        let taper = crate::hole_tables::pipe_taper_deg();
        assert!((cone + taper).abs() < 1e-9, "narrows 1:16: {cone}");
        let npt = ThreadStandard::Npt.size("1/2").unwrap();
        let pitch = 25.4 / 14.0;
        assert!(
            (major - (npt.major * 0.5 + pitch / 32.0)).abs() < 1e-9,
            "a pitch above the face, a pitch's taper wider"
        );

        let mut g = ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6");
        g.class = "6G".into();
        let (.., major, _) = thread(g);
        assert!((major - (3.0 + 0.026 * 0.5)).abs() < 1e-9, "6G sits EI out");
    }

    #[test]
    fn a_thread_the_standard_lacks_fails_cleanly() {
        let mut spec = ThreadSpec::new(ThreadStandard::Unc, "1/4-20");
        spec.class = "6H".into();
        let error = hole_plan(|h| *h.thread = Some(spec)).unwrap_err();
        assert!(error.contains("UNC has no class 6H"), "{error}");
    }

    #[test]
    fn a_tapered_standard_tapers_the_tapped_drill_only() {
        let drilled = |threaded: bool| {
            hole_plan(|h| {
                *h.thread = Some(ThreadSpec::new(ThreadStandard::BspTaper, "1/4"));
                *h.threaded = threaded;
                *h.taper_deg = 7.0;
            })
            .unwrap()
        };
        let (_, taper) = blind_distance(&drilled(true)[0]);
        assert!((taper + crate::hole_tables::pipe_taper_deg()).abs() < 1e-9);
        let (_, taper) = blind_distance(&drilled(false)[0]);
        assert!((taper + 7.0).abs() < 1e-9, "a clearance hole keeps its own");
    }

    /// The standalone circular and point patterns place their copies as
    /// their multi-transform steps do, the original left out.
    #[test]
    fn circular_and_point_patterns_build_as_their_steps() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 5.0), "Pad");
        let points = points_sketch(&mut doc, body, &[(1.0, 1.0), (6.0, 1.0), (1.0, 9.0)]);
        let circular = DesignFeature::CircularPattern {
            originals: vec![pad_id],
            axis: PatternAxis::Z,
            outward: PatternAxis::X,
            spacing: 10.0,
            counts: vec![4, 8],
            refine: false,
            boolean_tolerance: None,
            skipped: vec![2],
        };
        let at_points = DesignFeature::PointPattern {
            originals: vec![pad_id],
            sketch: points,
            refine: false,
            boolean_tolerance: None,
            skipped: Vec::new(),
        };
        for (feature, step, copies) in [
            (
                circular,
                TransformStep::Circular {
                    axis: PatternAxis::Z,
                    outward: PatternAxis::X,
                    spacing: 10.0,
                    counts: vec![4, 8],
                },
                3 + 8 - 1,
            ),
            (at_points, TransformStep::Points { sketch: points }, 2),
        ] {
            let mut doc = doc.clone();
            let skipped: Vec<u32> = feature.skipped().unwrap().to_vec();
            add(&mut doc, body, feature, "Pattern");
            let plan = body_build_ops(&doc, body).unwrap();
            let SolidOp::Transform {
                transforms,
                originals,
            } = &plan.ops[1]
            else {
                panic!("expected a transform op, got {:?}", plan.ops[1]);
            };
            assert_eq!(originals, &vec![0]);
            assert_eq!(transforms.len(), copies);
            let stepped: Vec<Mat4> = multi_transforms(&doc, &[step])
                .unwrap()
                .into_iter()
                .zip(1u32..)
                .filter(|(_, i)| !skipped.contains(i))
                .map(|(t, _)| t)
                .collect();
            assert_eq!(transforms, &stepped);
        }
    }

    /// A path pattern leaves its places to the chain: its op carries the
    /// sketch's chain on the sketch's plane, or the picked edge as a
    /// probe, with the spacing as the feature gives it.
    #[test]
    fn a_path_pattern_hands_its_path_to_the_chain() {
        use crate::feature::{PathSpacing, PatternPath};
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 5.0), "Pad");
        let layout = PathSpacing {
            occurrences: 5,
            spacing_mode: true,
            spacing: 4.0,
            reversed: true,
            align: true,
        };
        let along = |path| DesignFeature::PathPattern {
            originals: vec![pad_id],
            path,
            layout,
            refine: false,
            boolean_tolerance: None,
            skipped: vec![2],
        };
        let mut on_sketch = doc.clone();
        on_sketch
            .add_feature_in_body(
                along(PatternPath::Sketch {
                    sketch: sketch_id,
                    chain: 0,
                }),
                "Path".into(),
                Some(body),
            )
            .unwrap();
        let plan = body_build_ops(&on_sketch, body).unwrap();
        let SolidOp::Pattern {
            steps,
            originals,
            skipped,
        } = &plan.ops[1]
        else {
            panic!("expected a pattern op, got {:?}", plan.ops[1]);
        };
        assert_eq!((originals, skipped), (&vec![0], &vec![2]));
        let [kernel_api::PatternStep::Path(path)] = steps.as_slice() else {
            panic!("one path step, not {steps:?}");
        };
        assert_eq!(
            (path.count, path.spacing, path.reversed, path.align),
            (5, Some(4.0), true, true)
        );
        let kernel_api::PatternPath::Wire { wire, .. } = &path.path else {
            panic!("a sketch's chain goes as a wire");
        };
        assert_eq!(wire.segments.len(), 4, "the rectangle's four sides");

        let pick = crate::feature::EdgePick {
            point: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 1.0],
            faces: [0, 0],
        };
        add(&mut doc, body, along(PatternPath::Edge(pick)), "Path");
        let plan = body_build_ops(&doc, body).unwrap();
        let SolidOp::Pattern { steps, .. } = &plan.ops[1] else {
            panic!("expected a pattern op");
        };
        let [kernel_api::PatternStep::Path(path)] = steps.as_slice() else {
            panic!("one path step");
        };
        assert_eq!(
            path.path,
            kernel_api::PatternPath::Edge(kernel_api::EdgeProbe {
                point: [1.0, 2.0, 3.0],
                direction: [0.0, 0.0, 1.0],
                faces: [0, 0],
            })
        );
    }

    /// A multi-transform with a path step goes to the chain as steps, the
    /// others' copies worked out here; without one it composes here.
    #[test]
    fn a_multi_transform_with_a_path_step_goes_to_the_chain_as_steps() {
        use crate::feature::PatternPath;
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(sketch_id, 5.0), "Pad");
        let linear = TransformStep::Linear {
            axis: PatternAxis::Z,
            length: 20.0,
            occurrences: 3,
        };
        doc.add_feature_in_body(
            DesignFeature::MultiTransform {
                originals: vec![pad_id],
                steps: vec![
                    linear.clone(),
                    TransformStep::Path {
                        path: PatternPath::Sketch {
                            sketch: sketch_id,
                            chain: 0,
                        },
                        layout: Default::default(),
                    },
                ],
                refine: false,
                boolean_tolerance: None,
                skipped: Vec::new(),
            },
            "Multi".into(),
            Some(body),
        )
        .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        let SolidOp::Pattern { steps, .. } = &plan.ops[1] else {
            panic!("expected a pattern op, got {:?}", plan.ops[1]);
        };
        let [
            kernel_api::PatternStep::Transforms(rows),
            kernel_api::PatternStep::Path(_),
        ] = steps.as_slice()
        else {
            panic!("a row's copies, then the path: {steps:?}");
        };
        assert_eq!(rows, &multi_transforms(&doc, &[linear]).unwrap());
        assert!(
            multi_transforms(
                &doc,
                &[TransformStep::Path {
                    path: PatternPath::Sketch {
                        sketch: sketch_id,
                        chain: 0,
                    },
                    layout: Default::default(),
                }]
            )
            .is_err(),
            "a path's places are not worked out here"
        );
    }

    /// A pocket kept inside keeps what it shares with the body: its op is
    /// the common, and a pattern refuses to repeat it.
    #[test]
    fn a_pocket_kept_inside_keeps_the_common_and_does_not_pattern() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch_id, 10.0), "Pad");
        let mut kept = pocket(sketch_id, 5.0, false, false);
        *kept.keep_inside_mut().expect("a pocket offers it") = true;
        let pocket_id = add(&mut doc, body, kept, "Pocket");
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Common);
        // It digs into the material as the cutting pocket does.
        let digs = |op: &SolidOp| match op {
            SolidOp::Sweep {
                kind: SweepKind::Extrude { reversed, .. },
                ..
            } => *reversed,
            other => panic!("expected an extrusion, got {other:?}"),
        };
        let mut cutting = doc.clone();
        cutting
            .update_feature_data(pocket_id, pocket(sketch_id, 5.0, false, false).to_json())
            .unwrap();
        let cut_plan = body_build_ops(&cutting, body).unwrap();
        assert_eq!(boolean_of(&cut_plan.ops[1]), BooleanOp::Cut);
        assert_eq!(digs(&plan.ops[1]), digs(&cut_plan.ops[1]));

        let pattern = linear_pattern(vec![pocket_id], 3, Vec::new());
        add(&mut doc, body, pattern, "Pattern");
        let refused = body_build_ops(&doc, body).unwrap_err().to_string();
        assert!(refused.contains("cannot repeat"), "{refused}");
        // An adding feature offers no such switch.
        assert!(pad(sketch_id, 1.0).keep_inside_mut().is_none());
    }

    /// A linear pattern's op repeats its original's op, the copies spread
    /// over its length; a skipped copy's transform is left out; a skipped
    /// original makes the copies alone, and the body's first solid cannot
    /// be skipped.
    #[test]
    fn a_pattern_leaves_its_skipped_occurrences_out_of_the_plan() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let base = add(&mut doc, body, pad(sketch_id, 5.0), "Pad");
        let boss = add(&mut doc, body, pad(sketch_id, 8.0), "Boss");
        let pattern = |originals, skipped| linear_pattern(originals, 4, skipped);
        let id = add(&mut doc, body, pattern(vec![boss], vec![]), "Pattern");
        let copies_at = |doc: &Document| {
            let plan = body_build_ops(doc, body).unwrap();
            assert_eq!(plan.ops.len(), 3);
            let SolidOp::Transform {
                transforms,
                originals,
            } = &plan.ops[2]
            else {
                panic!("expected a transform: {:?}", plan.ops[2]);
            };
            assert_eq!(originals, &vec![1], "the boss's op");
            transforms.iter().map(|t| t[0][3]).collect::<Vec<f64>>()
        };
        // Overall length 30 over 4 occurrences: 10 mm apart.
        assert_eq!(copies_at(&doc), vec![10.0, 20.0, 30.0]);
        doc.update_feature_data(id, pattern(vec![boss], vec![2]).to_json())
            .unwrap();
        assert_eq!(copies_at(&doc), vec![10.0, 30.0], "the copy at 20 left out");

        doc.update_feature_data(id, pattern(vec![boss], vec![0, 3]).to_json())
            .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        let SolidOp::TransformCopies {
            transforms,
            originals,
        } = &plan.ops[2]
        else {
            panic!("expected the copies alone: {:?}", plan.ops[2]);
        };
        assert_eq!(originals, &vec![1]);
        let at: Vec<f64> = transforms.iter().map(|t| t[0][3]).collect();
        assert_eq!(at, vec![10.0, 20.0]);

        doc.update_feature_data(id, pattern(vec![base], vec![0]).to_json())
            .unwrap();
        let refused = body_build_ops(&doc, body).unwrap_err();
        assert!(
            refused.message.contains("first solid"),
            "{}",
            refused.message
        );
    }

    /// Two rings about Z, 4 and 8 round, the second 10 mm out along X:
    /// eleven copies (the original left out), the outer ring's at radius
    /// 10 from where the original stands, a quarter and an eighth apart.
    #[test]
    fn circular_rings_spread_their_counts_round_the_axis() {
        let z = ([0.0; 3], [0.0, 0.0, 1.0]);
        let ts = circular_transforms(z, [1.0, 0.0, 0.0], 10.0, &[4, 8]).unwrap();
        assert_eq!(ts.len(), 3 + 8);
        let moved = |t: &Mat4| transform_point(t, [0.0; 3]);
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
        // The first ring turns the original in place.
        assert!(close(moved(&ts[0]), [0.0; 3]));
        assert!((ts[0][1][0] - 1.0).abs() < 1e-9, "a quarter turn: x to y");
        // The outer ring: out along X, then round by eighths.
        assert!(close(moved(&ts[3]), [10.0, 0.0, 0.0]));
        let r = 10.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!(close(moved(&ts[4]), [r, r, 0.0]), "{:?}", moved(&ts[4]));
        assert!(
            circular_transforms(z, [0.0, 0.0, 1.0], 10.0, &[4, 8]).is_err(),
            "outward along the axis"
        );
        assert!(circular_transforms(z, [1.0, 0.0, 0.0], 10.0, &[4, 0]).is_err());
    }

    /// A copy at each point of the sketch after the first, moved by how far
    /// it is from the first; construction points left out.
    #[test]
    fn a_points_step_moves_copies_to_each_point() {
        let (mut doc, body, _) = doc_with_body_sketch();
        let mut points = Sketch::new("points");
        for (x, y) in [(1.0, 1.0), (6.0, 1.0), (1.0, 9.0)] {
            points.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
        }
        let guide = points.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(50.0, 50.0))));
        points.set_construction(guide, true);
        let sketch = add_sketch(&mut doc, body, points);
        let ts = point_transforms(&doc, sketch).unwrap();
        assert_eq!(ts.len(), 2);
        assert_eq!(transform_point(&ts[0], [0.0; 3]), [5.0, 0.0, 0.0]);
        assert_eq!(transform_point(&ts[1], [0.0; 3]), [0.0, 8.0, 0.0]);
    }

    /// A pad that starts on another sketch's plane, 7 mm above its own,
    /// sweeps from there (and a start offset adds to it); one that starts
    /// on a plane the extrusion runs along is refused.
    #[test]
    fn a_pad_starts_on_the_plane_it_names() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let mut lifted = Sketch::new("lifted");
        lifted.plane.origin = [0.0, 0.0, 7.0];
        let lifted = add_sketch(&mut doc, body, lifted);
        let mut started = pad(sketch_id, 5.0);
        if let DesignFeature::Pad { extras, .. } = &mut started {
            extras.start_at = Some(lifted);
            extras.start_offset = 1.0;
        }
        assert!(started.dependencies().contains(&lifted));
        add(&mut doc, body, started.clone(), "Pad");
        let plan = body_build_ops(&doc, body).unwrap();
        let SolidOp::Sweep { profile, .. } = &plan.ops[0] else {
            panic!("a sweep");
        };
        assert!(
            (profile.plane.origin[2] - 8.0).abs() < 1e-9,
            "{:?}",
            profile.plane.origin
        );

        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let side = {
            let s = Sketch::new("side");
            SketchFeature::new(s, wb_sketch::sketch::SketchPlane::xz())
        };
        let side = add(&mut doc, body, side, "side");
        if let DesignFeature::Pad { extras, sketch, .. } = &mut started {
            extras.start_at = Some(side);
            *sketch = Some(sketch_id);
        }
        add(&mut doc, body, started, "Pad");
        let refused = body_build_ops(&doc, body).unwrap_err().to_string();
        assert!(refused.contains("runs along the plane"), "{refused}");
    }

    /// A pad starting on a face of the solid sweeps from the face's
    /// plane, asks the solid before it where the face is, and is refused
    /// once a build finds the face curved.
    #[test]
    fn a_pad_starts_on_the_face_it_picks() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch_id, 10.0), "Pad");
        let face = FacePick {
            point: [1.0, 1.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            name: 0,
        };
        let mut boss = pad(sketch_id, 2.0);
        if let DesignFeature::Pad { extras, .. } = &mut boss {
            extras.start_face = Some(face);
        }
        let boss = add(&mut doc, body, boss, "Boss");
        let mut plan = body_build_ops(&doc, body).unwrap();
        plan.probes = datum_probes(&doc, body, &plan);
        let SolidOp::Sweep { profile, .. } = &plan.ops[1] else {
            panic!("a sweep");
        };
        assert!((profile.plane.origin[2] - 10.0).abs() < 1e-9);
        assert_eq!(plan.probes.len(), 1);
        assert_eq!(plan.probes[0].feature, boss);
        assert_eq!(plan.probes[0].probe.after_op, 1, "the solid before it");
        assert_eq!(plan.probes[0].probe.probe, face.probe());

        doc.store_probe_answers(
            &plan.probes,
            &[Ok(kernel_api::ProbeAnswer::Face {
                point: [1.0, 1.0, 10.0],
                normal: [1.0, 0.0, 0.0],
                surface: kernel_api::FaceSurface::Cylinder {
                    origin: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 1.0,
                },
            })],
        );
        let refused = body_build_ops(&doc, body).unwrap_err().to_string();
        assert!(refused.contains("not flat"), "{refused}");
    }

    /// A hole starts on its sketch's plane, carried along the drill onto
    /// another sketch's plane, then its offset further in; the plane it
    /// starts on is a dependency.
    #[test]
    fn a_hole_starts_where_its_start_says() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let mut lifted = Sketch::new("lifted");
        lifted.plane.origin = [0.0, 0.0, 7.0];
        let lifted = add_sketch(&mut doc, body, lifted);
        let mut hole = match DesignFeature::from_json(&serde_json::json!({"Hole": {
            "sketch": sketch_id, "diameter": 2.0, "depth": 3.0, "through_all": false,
        }})) {
            Ok(hole) => hole,
            Err(e) => panic!("{e}"),
        };
        let start_z = |doc: &Document, hole: &DesignFeature| {
            let sketch = load_sketch(doc, sketch_id).unwrap();
            hole_plane(doc, hole, &sketch).unwrap().0.origin[2]
        };
        assert!(start_z(&doc, &hole).abs() < 1e-9);
        if let DesignFeature::Hole { start, .. } = &mut hole {
            start.offset = 2.0;
        }
        assert!(
            (start_z(&doc, &hole) + 2.0).abs() < 1e-9,
            "into the material"
        );
        if let DesignFeature::Hole { start, .. } = &mut hole {
            start.at = Some(lifted);
        }
        assert!((start_z(&doc, &hole) - 5.0).abs() < 1e-9);
        assert!(hole.dependencies().contains(&lifted));
    }

    /// A plane turned about an axis it holds keeps the axis where it was
    /// and turns right-handed about the axis's direction.
    #[test]
    fn a_plane_turns_about_the_axis_it_holds() {
        let mut plane = kernel_api::ProfilePlane {
            origin: [0.0, 0.0, 5.0],
            ..Default::default()
        };
        // The line x = 2 of the plane, along its y.
        turn_plane(&mut plane, ([2.0, 0.0], [0.0, 1.0]), 90.0);
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
        assert!(close(plane.normal, [1.0, 0.0, 0.0]), "{:?}", plane.normal);
        assert!(close(plane.x_axis, [0.0, 0.0, -1.0]), "{:?}", plane.x_axis);
        assert!(close(plane.y_axis, [0.0, 1.0, 0.0]));
        // The axis's point stays put: origin + 2 x.
        let on_axis: [f64; 3] = std::array::from_fn(|k| plane.origin[k] + 2.0 * plane.x_axis[k]);
        assert!(close(on_axis, [2.0, 0.0, 5.0]), "{on_axis:?}");
    }

    /// Where a transform takes the point `p`.
    fn apply(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
    }

    fn near(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn a_polar_pattern_by_step_turns_each_occurrence_by_the_step() {
        let z = ([0.0; 3], [0.0, 0.0, 1.0]);
        let transforms = polar_transforms(z, 30.0, 4, false, true, &[]).unwrap();
        let x = [1.0, 0.0, 0.0];
        let turned = |deg: f64| {
            let r = deg.to_radians();
            [r.cos(), r.sin(), 0.0]
        };
        for (k, t) in transforms.iter().enumerate() {
            assert!(near(apply(t, x), turned(30.0 * (k + 1) as f64)), "copy {k}");
        }
        // Uneven: 10° then 50°, the third step the even 30°.
        let transforms = polar_transforms(z, 30.0, 4, false, true, &[10.0, 50.0]).unwrap();
        for (t, deg) in transforms.iter().zip([10.0, 60.0, 90.0]) {
            assert!(near(apply(t, x), turned(deg)), "{deg}");
        }
        assert!(polar_transforms(z, 30.0, 3, false, true, &[0.0]).is_err());
    }

    #[test]
    fn a_pattern_axis_resolves_from_an_edge_a_datum_line_or_a_sketch() {
        use core_document::{BasePlane, DatumAttachment, DatumFeature, DatumShape};
        let (mut doc, body, sketch) = doc_with_body_sketch();
        let edge = PatternAxis::Edge(crate::EdgePick {
            faces: [0, 0],
            point: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 2.0],
        });
        assert_eq!(
            pattern_axis(&doc, &edge).unwrap(),
            ([1.0, 2.0, 3.0], [0.0, 0.0, 1.0])
        );
        // The YZ plane's line runs along world Y.
        let datum = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::Line { length: 20.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::YZ),
                    offset: Default::default(),
                },
                "Line".into(),
                Some(body),
            )
            .unwrap();
        let (_, dir) = pattern_axis(&doc, &PatternAxis::Datum(datum)).unwrap();
        assert!(near(dir, [0.0, 1.0, 0.0]), "{dir:?}");
        assert!(pattern_axis(&doc, &PatternAxis::Datum(sketch)).is_err());
        let sketch_axis = |axis| PatternAxis::Sketch { sketch, axis };
        for (axis, want) in [
            (crate::SketchAxis::Horizontal, [1.0, 0.0, 0.0]),
            (crate::SketchAxis::Vertical, [0.0, 1.0, 0.0]),
            (crate::SketchAxis::Normal, [0.0, 0.0, 1.0]),
        ] {
            let (origin, dir) = pattern_axis(&doc, &sketch_axis(axis)).unwrap();
            assert!(
                near(origin, [0.0; 3]) && near(dir, want),
                "{axis:?}: {dir:?}"
            );
        }
    }

    #[test]
    fn pattern_referencing_later_feature_fails() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let ghost = FeatureId::new();
        add(&mut doc, body, pad(sketch_id, 5.0), "Pad");
        doc.add_feature_in_body(
            DesignFeature::Mirrored {
                skipped: Vec::new(),
                refine: false,
                boolean_tolerance: None,
                originals: vec![ghost],
                plane: MirrorPlane::YZ,
            },
            "Mirror".into(),
            Some(body),
        )
        .unwrap();
        let err = body_build_ops(&doc, body).unwrap_err();
        assert!(err.message.contains("original"), "{}", err.message);
    }

    /// The path `lines` draw between `points`, in that order, in a body of
    /// its own.
    fn path(points: &[(f64, f64)], lines: &[(usize, usize)]) -> (Document, FeatureId) {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let mut sketch = Sketch::new("path");
        let ids: Vec<_> = points
            .iter()
            .map(|&(x, y)| {
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
            })
            .collect();
        for &(a, b) in lines {
            sketch.add_geometry(GeometryElement::Line(Line::new(ids[a], ids[b])));
        }
        let id = add_sketch(&mut doc, body, sketch);
        (doc, id)
    }

    #[test]
    fn spine_orders_segments_into_one_chain() {
        // An L-shaped open path, its lines drawn out of order.
        let (doc, spine_id) = path(&[(0.0, 0.0), (20.0, 0.0), (20.0, 15.0)], &[(1, 2), (0, 1)]);
        let spine = sketch_spine(&doc, spine_id, None, &[]).unwrap();
        assert_eq!(spine.wires.len(), 1);
        assert_eq!(spine.wires[0].segments.len(), 2);
        // Consecutive segments share an endpoint.
        let ProfileSegment::Line { end, .. } = spine.wires[0].segments[0] else {
            panic!("line expected");
        };
        let ProfileSegment::Line { start, .. } = spine.wires[0].segments[1] else {
            panic!("line expected");
        };
        assert_eq!(end, start);
    }

    #[test]
    fn an_open_spine_starts_at_the_end_by_its_profile() {
        let (doc, spine_id) = path(&[(0.0, 0.0), (0.0, 10.0), (10.0, 10.0)], &[(0, 1), (1, 2)]);
        let first_start = |near| {
            let spine = sketch_spine(&doc, spine_id, near, &[]).unwrap();
            match spine.wires[0].segments[0] {
                ProfileSegment::Line { start, .. } => start,
                _ => panic!("line expected"),
            }
        };
        // The same end every time without a profile to go by.
        for _ in 0..8 {
            assert_eq!(first_start(None), [0.0, 0.0]);
        }
        assert_eq!(first_start(Some([10.0, 10.0, 0.0])), [10.0, 10.0]);
        assert_eq!(first_start(Some([0.0, -1.0, 0.0])), [0.0, 0.0]);
    }

    #[test]
    fn a_branching_spine_is_refused_unless_a_chain_of_it_is_chosen() {
        let (doc, spine_id) = path(
            &[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (-10.0, 0.0)],
            &[(0, 1), (0, 2), (0, 3)],
        );
        assert!(
            sketch_spine(&doc, spine_id, None, &[])
                .unwrap_err()
                .contains("branches")
        );
        // Two of its three lines are one chain through the corner.
        let drawn = load_sketch(&doc, spine_id).unwrap();
        let lines: Vec<_> = drawn
            .sketch
            .geometry
            .iter()
            .filter(|e| matches!(e, GeometryElement::Line(_)))
            .map(|e| e.id())
            .collect();
        let spine = sketch_spine(&doc, spine_id, None, &lines[..2]).unwrap();
        assert_eq!(spine.wires.len(), 1);
        assert_eq!(spine.wires[0].segments.len(), 2);
        assert!(
            sketch_spine(&doc, spine_id, None, &[uuid::Uuid::new_v4()])
                .unwrap_err()
                .contains("chosen curves")
        );
    }

    #[test]
    fn history_reorder_swaps_build_order_and_respects_dependencies() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_a = add(&mut doc, body, pad(sketch_id, 5.0), "PadA");
        let pad_b = add(&mut doc, body, pad(sketch_id, 9.0), "PadB");

        assert!(doc.move_feature_in_history(pad_b, true));
        let plan = body_build_ops(&doc, body).unwrap();
        assert!(
            (extrude_distance(&plan.ops[0]) - 9.0).abs() < 1e-9,
            "B first"
        );
        assert!((extrude_distance(&plan.ops[1]) - 5.0).abs() < 1e-9);

        // A pattern must not move before its original.
        let pattern = doc
            .add_feature_in_body(
                DesignFeature::Mirrored {
                    skipped: Vec::new(),
                    refine: false,
                    boolean_tolerance: None,
                    originals: vec![pad_a],
                    plane: MirrorPlane::YZ,
                },
                "Mirror".into(),
                Some(body),
            )
            .unwrap();
        assert!(
            !doc.move_feature_in_history(pattern, true),
            "moving the mirror above its original is blocked"
        );
        // But the original may not move below its dependent either.
        assert!(!doc.move_feature_in_history(pad_a, false));
    }

    /// A move is one place in the body's whole history: the Pocket cannot
    /// pass its own sketch, and the Pad never jumps past a sketch it does
    /// not use.
    #[test]
    fn a_move_is_one_place_in_the_whole_history() {
        let (mut doc, body, base) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(base, 5.0), "Pad");
        let cut_sketch = add(&mut doc, body, rect_sketch(), "cut");
        let pocket = add(
            &mut doc,
            body,
            pocket(cut_sketch, 2.0, false, false),
            "Pocket",
        );
        let order = |doc: &Document| {
            let mut nodes: Vec<(u64, FeatureId)> = doc
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.body == Some(body))
                .map(|(id, n)| (n.seq, *id))
                .collect();
            nodes.sort();
            nodes.into_iter().map(|(_, id)| id).collect::<Vec<_>>()
        };
        assert_eq!(
            doc.try_move_feature_in_history(pocket, true),
            Err(core_document::MoveRefused::Dependency {
                neighbour: cut_sketch
            }),
            "the pocket stays after its sketch, and names it"
        );
        assert_eq!(order(&doc), vec![base, pad_id, cut_sketch, pocket]);
        assert!(doc.move_feature_in_history(cut_sketch, true));
        assert_eq!(order(&doc), vec![base, cut_sketch, pad_id, pocket]);
        // The pocket does not use the pad, so the pad may move past it.
        assert!(doc.move_feature_in_history(pad_id, false));
        assert_eq!(order(&doc), vec![base, cut_sketch, pocket, pad_id]);
    }

    #[test]
    fn retarget_moves_dependency_and_visibility() {
        let (mut doc, body, sketch_a) = doc_with_body_sketch();
        let sketch_b = add(&mut doc, body, rect_sketch(), "sketch_b");
        let pad_id = add(&mut doc, body, pad(sketch_a, 5.0), "Pad");
        doc.set_feature_visible(sketch_a, false);
        doc.clear_feature_dirty(pad_id);

        retarget_feature_sketch(&mut doc, pad_id, sketch_b).unwrap();

        assert_eq!(doc.feature_tree().dependencies(pad_id), vec![sketch_b]);
        assert!(doc.get_feature_meta(sketch_a).unwrap().visible);
        assert!(!doc.get_feature_meta(sketch_b).unwrap().visible);
        assert!(doc.get_feature_meta(pad_id).unwrap().dirty);
    }

    /// Symmetric is a choice of the Dimension mode alone: a pad or pocket
    /// left with it on in another mode builds as that mode does.
    #[test]
    fn symmetric_centres_only_a_dimension_extrusion() {
        let symmetric_of = |feature: DesignFeature| {
            let (mut doc, body, sketch) = doc_with_body_sketch();
            // Material first, for the modes that cut through it.
            add(&mut doc, body, pad(sketch, 5.0), "Base");
            let mut feature = feature;
            match &mut feature {
                DesignFeature::Pad { sketch: s, .. } | DesignFeature::Pocket { sketch: s, .. } => {
                    *s = Some(sketch)
                }
                _ => unreachable!(),
            }
            let id = add(&mut doc, body, feature, "F");
            let plan = body_build_ops(&doc, body).unwrap();
            let index = plan.op_features.iter().position(|f| *f == id).unwrap();
            match &plan.ops[index] {
                SolidOp::Sweep {
                    kind: SweepKind::Extrude { symmetric, .. },
                    ..
                } => *symmetric,
                other => panic!("not an extrude: {other:?}"),
            }
        };
        let placeholder = FeatureId::new();
        for mode in [
            ExtrudeMode::Dimension,
            ExtrudeMode::TwoLengths,
            ExtrudeMode::ThroughAll,
        ] {
            let mut padded = pad(placeholder, 4.0);
            let mut pocketed = pocket(placeholder, 2.0, false, false);
            for feature in [&mut padded, &mut pocketed] {
                match feature {
                    DesignFeature::Pad {
                        symmetric,
                        mode: m,
                        length2,
                        ..
                    } => {
                        (*symmetric, *m, *length2) = (true, mode, 3.0);
                    }
                    DesignFeature::Pocket {
                        symmetric,
                        mode: m,
                        depth2,
                        ..
                    } => {
                        (*symmetric, *m, *depth2) = (true, mode, 1.0);
                    }
                    _ => unreachable!(),
                }
            }
            let expected = mode == ExtrudeMode::Dimension;
            assert_eq!(symmetric_of(padded), expected, "pad, {mode:?}");
            assert_eq!(symmetric_of(pocketed), expected, "pocket, {mode:?}");
        }
        // A pocket's legacy through-all flag is the ThroughAll mode.
        let mut legacy = pocket(placeholder, 2.0, false, true);
        if let DesignFeature::Pocket { symmetric, .. } = &mut legacy {
            *symmetric = true;
        }
        assert!(!symmetric_of(legacy));
    }

    /// A feature is offered the sketches before it in history, and the
    /// one it already has even when that one comes later.
    #[test]
    fn a_feature_is_offered_the_sketches_before_it() {
        let (mut doc, body, before) = doc_with_body_sketch();
        let pad_id = add(&mut doc, body, pad(before, 5.0), "Pad");
        let after = add(&mut doc, body, rect_sketch(), "after");
        let ids = |current| {
            sketch_choices(&doc, body, pad_id, current)
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Some(before)), vec![before]);
        assert_eq!(ids(None), vec![before]);
        assert_eq!(ids(Some(after)), vec![before, after]);
    }

    #[test]
    fn sketches_of_body_lists_in_creation_order() {
        let (mut doc, body, first) = doc_with_body_sketch();
        let second = add(&mut doc, body, rect_sketch(), "second");
        let list = sketches_of_body(&doc, body);
        assert_eq!(
            list.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn an_open_profile_fails_on_its_feature() {
        let (mut doc, body, _) = doc_with_body_sketch();
        let mut sketch = Sketch::new("open");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let open_id = add_sketch(&mut doc, body, sketch);
        let bad = add(&mut doc, body, pad(open_id, 7.0), "BadPad");
        let err = body_build_ops(&doc, body).unwrap_err();
        assert_eq!(err.feature, Some(bad));
        // The feature names itself wherever the error shows.
        assert!(!err.message.contains("BadPad"), "{}", err.message);
    }

    /// The one extrude op a pad or pocket, `edit`ed from its defaults,
    /// builds on a pad of the rectangle.
    fn extrude_of(
        feature: DesignFeature,
        edit: impl FnOnce(&mut DesignFeature),
    ) -> Result<SolidOp, String> {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch_id, 5.0), "Base");
        let mut feature = feature;
        if let DesignFeature::Pad { sketch, .. } | DesignFeature::Pocket { sketch, .. } =
            &mut feature
        {
            *sketch = Some(sketch_id);
        }
        edit(&mut feature);
        add(&mut doc, body, feature, "Feature");
        body_build_ops(&doc, body)
            .map(|plan| plan.ops[1].clone())
            .map_err(|e| e.message)
    }

    fn extrude_kind(op: &SolidOp) -> &SweepKind {
        match op {
            SolidOp::Sweep { kind, .. } | SolidOp::SweepFace { kind, .. } => kind,
            _ => panic!("not a sweep: {op:?}"),
        }
    }

    /// A direction set runs the way it points, a pocket's included; one in
    /// the sketch plane is refused.
    #[test]
    fn a_direction_set_is_the_way_a_pad_or_a_pocket_runs() {
        let sketch = FeatureId::new();
        for feature in [pad(sketch, 5.0), pocket(sketch, 5.0, false, false)] {
            let op = extrude_of(feature.clone(), |f| {
                if let DesignFeature::Pad { direction, .. }
                | DesignFeature::Pocket { direction, .. } = f
                {
                    *direction = crate::ExtrudeDirection::Custom([0.0, 1.0, 1.0]);
                }
            })
            .unwrap();
            assert!(matches!(
                extrude_kind(&op),
                SweepKind::Extrude {
                    direction: Some([x, y, z]),
                    reversed: false,
                    ..
                } if *x == 0.0 && *y == 1.0 && *z == 1.0
            ));
            let edge = extrude_of(feature.clone(), |f| {
                if let DesignFeature::Pad { direction, .. }
                | DesignFeature::Pocket { direction, .. } = f
                {
                    *direction = crate::ExtrudeDirection::Edge(crate::EdgePick {
                        faces: [0, 0],
                        point: [0.0; 3],
                        direction: [1.0, 0.0, 1.0],
                    });
                }
            })
            .unwrap();
            assert!(matches!(
                extrude_kind(&edge),
                SweepKind::Extrude { direction: Some([x, _, z]), .. } if *x == 1.0 && *z == 1.0
            ));
            let flat = extrude_of(feature, |f| {
                if let DesignFeature::Pad { direction, .. }
                | DesignFeature::Pocket { direction, .. } = f
                {
                    *direction = crate::ExtrudeDirection::Custom([1.0, 1.0, 0.0]);
                }
            });
            assert!(flat.unwrap_err().contains("plane"));
        }
    }

    /// The second side ends as `mode2` says, with its own face, faces and
    /// offset; two lengths without it is a dimension each way.
    #[test]
    fn each_side_takes_its_own_end() {
        let sketch = FeatureId::new();
        let face = FacePick {
            name: 0,
            point: [1.0, 2.0, 3.0],
            normal: [0.0, 0.0, 1.0],
        };
        let op = extrude_of(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad {
                mode2,
                up_to_face2,
                up_to_offset2,
                ..
            } = f
            {
                *mode2 = Some(ExtrudeMode::UpToFace);
                *up_to_face2 = Some(face);
                *up_to_offset2 = 1.5;
            }
        })
        .unwrap();
        assert!(matches!(
            extrude_kind(&op),
            SweepKind::Extrude {
                termination: ExtrudeTermination::Blind { distance },
                second_side: Some(ExtrudeTermination::UpToFace { point, offset, .. }),
                ..
            } if *distance == 5.0 && point[2] == 3.0 && *offset == 1.5
        ));
        let op = extrude_of(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad {
                mode,
                length2,
                up_to_shape,
                ..
            } = f
            {
                *mode = ExtrudeMode::TwoLengths;
                *length2 = 2.0;
                up_to_shape.push(face);
            }
        })
        .unwrap();
        assert!(matches!(
            extrude_kind(&op),
            SweepKind::Extrude {
                second_side: Some(ExtrudeTermination::Blind { distance }),
                ..
            } if *distance == 2.0
        ));
        let op = extrude_of(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad {
                mode, up_to_shape, ..
            } = f
            {
                *mode = ExtrudeMode::UpToShape;
                up_to_shape.push(face);
                up_to_shape.push(face);
            }
        })
        .unwrap();
        assert!(matches!(
            extrude_kind(&op),
            SweepKind::Extrude {
                termination: ExtrudeTermination::UpToShape { faces, .. },
                second_side: None,
                ..
            } if faces.len() == 2
        ));
        let none = extrude_of(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad { mode, .. } = f {
                *mode = ExtrudeMode::UpToShape;
            }
        });
        assert!(none.unwrap_err().contains("faces"));
    }

    /// A face profile extrudes that face of the solid, a pocket's into it;
    /// with neither a face nor a sketch there is nothing to extrude.
    #[test]
    fn a_face_profile_sweeps_the_face() {
        let face = FacePick {
            name: 0,
            point: [5.0, 2.5, 5.0],
            normal: [0.0, 0.0, 1.0],
        };
        let sketch = FeatureId::new();
        let op = extrude_of(pocket(sketch, 2.0, false, false), |f| {
            if let DesignFeature::Pocket {
                sketch,
                profile_face,
                ..
            } = f
            {
                *sketch = None;
                *profile_face = Some(face);
            }
        })
        .unwrap();
        assert!(matches!(
            &op,
            SolidOp::SweepFace {
                face: FaceProbe { point, .. },
                kind: SweepKind::Extrude { reversed: true, .. },
                op: BooleanOp::Cut,
            } if point[2] == 5.0
        ));
        let nothing = extrude_of(pad(sketch, 2.0), |f| {
            if let DesignFeature::Pad { sketch, .. } = f {
                *sketch = None;
            }
        });
        assert!(nothing.unwrap_err().contains("profile"));

        // A face needs a solid to come from.
        let (mut doc, body, _) = doc_with_body_sketch();
        let mut first = pad(sketch, 2.0);
        if let DesignFeature::Pad {
            sketch,
            profile_face,
            ..
        } = &mut first
        {
            *sketch = None;
            *profile_face = Some(face);
        }
        add(&mut doc, body, first, "Pad");
        assert!(body_build_ops(&doc, body).is_err());
    }

    /// A revolution's end mode reaches the kernel as its termination, and
    /// a stop on a face needs material before it.
    #[test]
    fn a_revolution_takes_its_end_mode() {
        let revolution =
            |mode: RevolveMode, up_to_face: Option<FacePick>, sketch| DesignFeature::Revolution {
                regions: Vec::new(),
                refine: false,
                boolean_tolerance: None,
                sketch,
                angle_deg: 360.0,
                axis: RevolveAxis::SketchY,
                project_axis: false,
                reversed: false,
                midplane: false,
                second_angle_deg: None,
                start_angle_deg: 0.0,
                mode,
                up_to_face,
            };
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(
            &mut doc,
            body,
            revolution(RevolveMode::ToLast, None, sketch_id),
            "Rev",
        );
        assert!(body_build_ops(&doc, body).is_err(), "nothing to stop on");

        let face = FacePick {
            name: 0,
            point: [0.0, 1.0, 2.0],
            normal: [1.0, 0.0, 0.0],
        };
        for (mode, want) in [
            (RevolveMode::ToFirst, RevolveTermination::ToFirst),
            (RevolveMode::ToLast, RevolveTermination::ToLast),
            (
                RevolveMode::UpToFace,
                RevolveTermination::UpToFace(FaceProbe {
                    name: 0,
                    point: [0.0, 1.0, 2.0],
                    normal: [1.0, 0.0, 0.0],
                }),
            ),
        ] {
            let (mut doc, body, sketch_id) = doc_with_body_sketch();
            add(&mut doc, body, pad(sketch_id, 1.0), "Base");
            add(
                &mut doc,
                body,
                revolution(mode, Some(face), sketch_id),
                "Rev",
            );
            let plan = body_build_ops(&doc, body).unwrap();
            assert!(matches!(
                &plan.ops[1],
                SolidOp::Sweep { kind: SweepKind::Revolve { termination, .. }, .. }
                    if *termination == want
            ));
        }
    }

    /// Each kind of axis lands in the sketch's own coordinates: a line of
    /// the sketch as drawn, a datum line or an edge where it falls on the
    /// plane; one across the plane is refused.
    #[test]
    fn every_axis_lands_in_the_sketch() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut feature = rect_sketch();
        let a = feature
            .sketch
            .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-1.0, 0.0))));
        let b = feature
            .sketch
            .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-1.0, 3.0))));
        let line = feature
            .sketch
            .add_geometry(GeometryElement::Line(Line::new(a, b)));
        let datum = doc
            .add_feature_in_body(
                core_document::DatumFeature {
                    shape: core_document::DatumShape::Line { length: 10.0 },
                    attachment: core_document::DatumAttachment::BasePlane(
                        core_document::BasePlane::YZ,
                    ),
                    offset: core_document::AttachmentOffset {
                        translation: [0.0, 0.0, -4.0],
                        ..Default::default()
                    },
                },
                "Line".into(),
                Some(body),
            )
            .unwrap();
        let near = |got: ([f64; 2], [f64; 2]), want: ([f64; 2], [f64; 2])| {
            let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-5;
            let unit = |v: [f64; 2]| {
                let l = v[0].hypot(v[1]);
                [v[0] / l, v[1] / l]
            };
            close(got.0, want.0) && close(unit(got.1), unit(want.1))
        };
        let line_axis =
            axis_in_sketch(&doc, &feature, &RevolveAxis::SketchLine(line), false).unwrap();
        assert!(near(line_axis, ([-1.0, 0.0], [0.0, 3.0])), "{line_axis:?}");
        // The YZ plane pushed back 4 along its normal (world X): a line
        // along world Y through x = -4.
        let datum_axis = axis_in_sketch(&doc, &feature, &RevolveAxis::Datum(datum), false).unwrap();
        assert!(
            near(datum_axis, ([-4.0, 0.0], [0.0, 1.0])),
            "{datum_axis:?}"
        );
        let edge = |direction| {
            RevolveAxis::Edge(crate::EdgePick {
                faces: [0, 0],
                point: [2.0, 7.0, 5.0],
                direction,
            })
        };
        let edge_axis = axis_in_sketch(&doc, &feature, &edge([1.0, 0.0, 0.0]), false).unwrap();
        assert!(near(edge_axis, ([2.0, 7.0], [1.0, 0.0])), "{edge_axis:?}");
        let across = axis_in_sketch(&doc, &feature, &edge([0.0, 0.0, 1.0]), false);
        assert!(across.unwrap_err().contains("sketch plane"));
        // Tilted to the plane: refused, unless projected, when it is its
        // shadow on the plane. Square to it, even projected, it is refused.
        let tilted = edge([1.0, 0.0, 1.0]);
        assert!(axis_in_sketch(&doc, &feature, &tilted, false).is_err());
        let shadow = axis_in_sketch(&doc, &feature, &tilted, true).unwrap();
        assert!(near(shadow, ([2.0, 7.0], [1.0, 0.0])), "{shadow:?}");
        assert!(
            (shadow.1[0].hypot(shadow.1[1]) - 1.0).abs() < 1e-9,
            "a unit direction"
        );
        let square = axis_in_sketch(&doc, &feature, &edge([0.0, 0.0, 1.0]), true);
        assert!(square.unwrap_err().contains("square"));
        let missing = axis_in_sketch(
            &doc,
            &feature,
            &RevolveAxis::SketchLine(uuid::Uuid::new_v4()),
            false,
        );
        assert!(missing.is_err());
    }

    fn helix(sketch: FeatureId, mode: HelixMode, height: f32, growth: f32) -> DesignFeature {
        DesignFeature::Helix {
            refine: false,
            boolean_tolerance: None,
            sketch,
            axis: RevolveAxis::SketchY,
            project_axis: false,
            mode,
            pitch: 2.0,
            height,
            turns: 4.0,
            left_handed: false,
            cone_angle_deg: 10.0,
            reversed: false,
            subtractive: false,
            growth,
            keep_inside: false,
        }
    }

    /// The helix sweep of a body that is one helix.
    fn helix_kind(mode: HelixMode, height: f32, growth: f32) -> Result<SweepKind, BuildError> {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let feature = helix(sketch_id, mode, height, growth);
        add(&mut doc, body, feature, "Helix");
        let plan = body_build_ops(&doc, body)?;
        match plan.ops.into_iter().next() {
            Some(SolidOp::Sweep { kind, .. }) => Ok(kind),
            other => panic!("not a sweep: {other:?}"),
        }
    }

    #[test]
    fn a_helix_in_the_growth_mode_gives_its_turns_and_growth() {
        let SweepKind::Helix {
            pitch,
            height,
            turns,
            growth,
            ..
        } = helix_kind(HelixMode::HeightTurnsGrowth, 12.0, 1.5).unwrap()
        else {
            panic!("not a helix")
        };
        assert_eq!((pitch, height), (3.0, 12.0));
        assert_eq!((turns, growth), (Some(4.0), Some(1.5)));

        // A height of 0 is a flat spiral, handed on to the kernel whole.
        let SweepKind::Helix {
            pitch,
            height,
            turns,
            growth,
            ..
        } = helix_kind(HelixMode::HeightTurnsGrowth, 0.0, 2.0).unwrap()
        else {
            panic!("not a helix")
        };
        assert_eq!((pitch, height), (0.0, 0.0));
        assert_eq!((turns, growth), (Some(4.0), Some(2.0)));

        // One that neither climbs nor grows is nothing.
        let flat = helix_kind(HelixMode::HeightTurnsGrowth, 0.0, 0.0).unwrap_err();
        assert!(flat.message.contains("growth"), "{}", flat.message);

        // The other modes leave the cone angle to say how it grows.
        let SweepKind::Helix {
            turns,
            growth,
            cone_angle_deg,
            ..
        } = helix_kind(HelixMode::PitchHeight, 12.0, 1.5).unwrap()
        else {
            panic!("not a helix")
        };
        assert_eq!((turns, growth, cone_angle_deg), (None, None, 10.0));
    }

    #[test]
    fn a_subtractive_helix_kept_inside_keeps_the_common() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        let mut cut = helix(sketch_id, HelixMode::PitchHeight, 8.0, 0.0);
        if let DesignFeature::Helix {
            subtractive,
            keep_inside,
            ..
        } = &mut cut
        {
            *subtractive = true;
            *keep_inside = true;
        }
        add(&mut doc, body, cut.clone(), "Helix");
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Common);

        // Kept inside means nothing on an adding helix.
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
        if let DesignFeature::Helix {
            sketch,
            subtractive,
            ..
        } = &mut cut
        {
            *sketch = sketch_id;
            *subtractive = false;
        }
        add(&mut doc, body, cut, "Helix");
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Fuse);
    }

    #[test]
    fn a_pipe_hands_its_orientation_corners_and_sections_to_the_kernel() {
        let (mut doc, body, profile) = doc_with_body_sketch();
        let spine = add(&mut doc, body, rect_sketch(), "spine");
        let guide = add(&mut doc, body, rect_sketch(), "guide");
        let section = add(&mut doc, body, rect_sketch(), "section");
        let pipe = |orientation: PipeOrientation| DesignFeature::Pipe {
            keep_inside: false,
            path_borrowed: Vec::new(),
            path_curves: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            boolean_tolerance: None,
            profile,
            spine,
            orientation,
            corner: PipeCorner::Round,
            sections: vec![section],
            subtractive: false,
        };
        let op_of = |feature: DesignFeature| {
            let mut doc = doc.clone();
            add(&mut doc, body, feature, "Pipe");
            body_build_ops(&doc, body).map(|plan| plan.ops.into_iter().next().unwrap())
        };
        let Ok(SolidOp::Pipe {
            frame,
            corner,
            sections,
            ..
        }) = op_of(pipe(PipeOrientation::Frenet))
        else {
            panic!("not a pipe")
        };
        assert_eq!(frame, kernel_api::PipeFrame::Frenet);
        assert_eq!(corner, kernel_api::PipeCorner::Round);
        assert_eq!(sections.len(), 1);

        let Ok(SolidOp::Pipe { frame, .. }) = op_of(pipe(PipeOrientation::Binormal {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        })) else {
            panic!("not a pipe")
        };
        assert_eq!(
            frame,
            kernel_api::PipeFrame::Binormal {
                direction: [0.0, 1.0, 0.0]
            }
        );

        let Ok(SolidOp::Pipe { frame, .. }) =
            op_of(pipe(PipeOrientation::Auxiliary { path: guide }))
        else {
            panic!("not a pipe")
        };
        assert!(
            matches!(frame, kernel_api::PipeFrame::Auxiliary { ref path } if path.wires.len() == 1),
            "{frame:?}"
        );

        let zero = op_of(pipe(PipeOrientation::Binormal {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }))
        .unwrap_err();
        assert!(zero.message.contains("binormal"), "{}", zero.message);
    }
}
