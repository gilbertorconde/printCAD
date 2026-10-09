//! Translation of a body's Design features into kernel solid ops.
//!
//! The host drives the recompute loop through the registry: each frame it
//! collects every bench's [`RebuildJob`]s and hands each plan to the kernel
//! worker. This bench's jobs are the bodies with dirty part features, each
//! with its feature history converted into a [`SolidOp`] chain.

use core_document::{BodyId, Document, FeatureId, RebuildJob, WorkbenchFeature};
pub use core_document::{BuildError, BuildPlan};
use kernel_api::{
    BooleanOp, EdgeSelection, ExtrudeTermination, FaceProbe, Profile, ProfileSegment, ProfileWire,
    RevolveTermination, SolidOp, SweepKind,
};
use wb_sketch::SketchFeature;
use wb_sketch::profile;
use wb_sketch::sketch::{GeometryElement, Sketch};

use crate::feature::{
    BorrowSource, DesignFeature, DrillPoint, ExtrudeDirection, ExtrudeMode, FacePick, HelixMode,
    HoleCut, NutSide, NutTrap, PatternAxis, PipeCorner, PipeOrientation, RevolveAxis, RevolveMode,
    ThreadSpec, TransformStep,
};

/// This body's part features in history order (the build history).
pub fn design_features_of_body(
    document: &Document,
    body: BodyId,
) -> Vec<(FeatureId, DesignFeature)> {
    let mut features: Vec<(u64, FeatureId, DesignFeature)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.design" && node.body == Some(body))
        .filter_map(|(id, node)| {
            // As it builds: with every formula's current value in.
            DesignFeature::from_json(document.feature_values(*id)?)
                .ok()
                .map(|f| (node.seq, *id, f))
        })
        .collect();
    // `seq` is the document's explicit insertion order: the build history.
    // Ties (two replicas inserting concurrently) break on the feature id so
    // every replica agrees on the order.
    features.sort_by_key(|(seq, id, _)| (*seq, *id));
    features.into_iter().map(|(_, id, f)| (id, f)).collect()
}

/// Bodies that have at least one dirty part feature (their solids need a
/// rebuild).
pub fn pending_body_rebuilds(document: &Document) -> Vec<BodyId> {
    let mut bodies: Vec<BodyId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.design" && node.dirty)
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
                plan
            });
            RebuildJob { body, plan }
        })
        .collect()
}

/// The datums of `body` that stand on its solid, each with what it asks
/// of the solid summed up. Read from the datum as it was made: what a
/// build found is the answer, never the question.
fn following_datums(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    use std::hash::{Hash, Hasher};
    datums_asking(document, body)
        .into_iter()
        .map(|(id, _, probes)| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(&probes)
                .unwrap_or_default()
                .hash(&mut hasher);
            (id, hasher.finish())
        })
        .collect()
}

/// Tell each sketch of `body` placed on a face a borrow lends where the
/// face is now, as the borrow finds it on its source, the way a build
/// answers a sketch on the body's own solid.
fn answer_lent_faces(document: &mut Document, body: BodyId) {
    let lent: Vec<(FeatureId, wb_sketch::FaceSupport)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(body) && n.workbench_id.as_str() == "wb.sketch")
        .filter_map(|(id, n)| {
            let face = wb_sketch::SketchFeature::from_json(&n.data).ok()?.face?;
            face.lent_by.is_some().then_some((*id, face))
        })
        .collect();
    for (sketch, face) in lent {
        let Some(lent_by) = face.lent_by else {
            continue;
        };
        let answer = crate::borrow::lent_face(document, lent_by.borrow, lent_by.index)
            .map(|(point, normal)| kernel_api::ProbeAnswer::Face {
                point: point.map(f64::from),
                normal: normal.map(f64::from),
                surface: kernel_api::FaceSurface::Plane {
                    origin: point,
                    normal,
                },
            })
            .ok_or_else(|| "the borrow no longer lends a flat face there".to_string());
        document.set_probed_references(
            sketch,
            core_document::ProbedReferences {
                probes: vec![face.probe()],
                answers: vec![answer],
            },
        );
    }
}

/// The datums of `body`, and its sketches placed on its faces, with
/// references to find again on its solid: id, place in the history and
/// the probes.
fn datums_asking(
    document: &Document,
    body: BodyId,
) -> Vec<(FeatureId, u64, Vec<kernel_api::ShapeProbe>)> {
    let mut datums: Vec<(FeatureId, u64, Vec<kernel_api::ShapeProbe>)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(body) && !n.suppressed)
        .filter_map(|(id, n)| {
            let probes = match n.workbench_id.as_str() {
                core_document::DATUM_KIND => core_document::DatumFeature::from_json(&n.data)
                    .ok()?
                    .probes(),
                // A primitive attached by a mode asks what the attachment
                // asks.
                "wb.design" => match DesignFeature::from_json(&n.data).ok()? {
                    DesignFeature::Primitive {
                        attached: Some(attached),
                        ..
                    } => attached.probes(),
                    _ => return None,
                },
                "wb.sketch" => {
                    let sketch = wb_sketch::SketchFeature::from_json(&n.data).ok()?;
                    // Attached by a mode, it asks what the attachment asks.
                    match sketch.attached {
                        Some(attached) => attached.probes(),
                        // A face a borrow lends is on another body's solid:
                        // `answer_lent_faces` finds it, not this body's build.
                        None => sketch
                            .face
                            .iter()
                            .filter(|face| face.lent_by.is_none())
                            .map(|face| face.probe())
                            .collect(),
                    }
                }
                _ => return None,
            };
            (!probes.is_empty()).then_some((*id, n.seq, probes))
        })
        .collect();
    datums.sort_by_key(|(id, seq, _)| (*seq, *id));
    datums
}

/// What the datums of `body` ask of its solid, each of the solid the
/// features before it make.
pub fn datum_probes(
    document: &Document,
    body: BodyId,
    plan: &BuildPlan,
) -> Vec<core_document::PlanProbe> {
    let seq_of = |id: &FeatureId| document.get_feature_meta(*id).map(|n| n.seq);
    datums_asking(document, body)
        .into_iter()
        .flat_map(|(feature, seq, probes)| {
            let after_op = plan
                .op_features
                .iter()
                .take_while(|f| seq_of(f).is_some_and(|s| s < seq))
                .count();
            probes
                .into_iter()
                .map(move |probe| core_document::PlanProbe {
                    feature,
                    probe: kernel_api::ChainProbe { after_op, probe },
                })
        })
        .collect()
}

/// The Booleans of `body`'s history that are not suppressed, each with
/// the body it takes as its tool.
fn boolean_tools(document: &Document, body: BodyId) -> Vec<(FeatureId, BodyId)> {
    design_features_of_body(document, body)
        .into_iter()
        .filter(|(id, _)| !document.get_feature_meta(*id).is_some_and(|n| n.suppressed))
        .flat_map(|(id, feature)| match feature {
            DesignFeature::BodyBoolean {
                tool_body,
                more_tools,
                ..
            } => std::iter::once(tool_body)
                .chain(more_tools)
                .map(|tool| (id, tool))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// Each Boolean of `body` with what its tools are now, all of them in one
/// sum: a Boolean with several tools is out of date when any of them is.
/// A tool that takes this body as a tool in turn follows nothing: the
/// Boolean is refused, and the two would rebuild each other forever.
fn boolean_inputs(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    use std::hash::{Hash, Hasher};
    let mut out: Vec<(FeatureId, std::collections::hash_map::DefaultHasher)> = Vec::new();
    for (feature, tool) in boolean_tools(document, body) {
        if tools_reach(document, tool, body) {
            continue;
        }
        let inputs = tool_inputs(document, body, tool);
        match out.iter_mut().find(|(f, _)| *f == feature) {
            Some((_, hasher)) => inputs.hash(hasher),
            None => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                inputs.hash(&mut hasher);
                out.push((feature, hasher));
            }
        }
    }
    out.into_iter().map(|(f, h)| (f, h.finish())).collect()
}

/// Each live borrow of `body` with what it follows now. A borrow of a
/// body built from this one in turn follows nothing: the two would rebuild
/// each other forever, and what stands on it says so instead.
fn borrow_inputs(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    crate::borrow::borrows_of_body(document, body)
        .into_iter()
        .filter(|(_, borrow)| borrow.frozen.is_none())
        .filter(|(_, borrow)| match &borrow.source {
            BorrowSource::Solid { body: source, .. } => !bodies_reach(document, *source, body),
            BorrowSource::Sketch(_) => true,
        })
        .map(|(id, borrow)| (id, crate::borrow::inputs(document, body, &borrow.source)))
        .collect()
}

/// The bodies whose solids `body` is built against: the tool bodies of its
/// Booleans and the sources of the faces and edges it borrows live.
fn followed_bodies(document: &Document, body: BodyId) -> Vec<BodyId> {
    let mut followed: Vec<BodyId> = boolean_tools(document, body)
        .into_iter()
        .map(|(_, tool)| tool)
        .collect();
    for (_, borrow) in crate::borrow::borrows_of_body(document, body) {
        if let (None, BorrowSource::Solid { body: source, .. }) = (&borrow.frozen, &borrow.source) {
            followed.push(*source);
        }
    }
    followed
}

/// Whether `to` is among the bodies `from` is built against, or theirs,
/// and so on: a body that reaches itself this way can never be built.
pub(crate) fn bodies_reach(document: &Document, from: BodyId, to: BodyId) -> bool {
    let mut stack = vec![from];
    let mut seen: Vec<BodyId> = Vec::new();
    while let Some(body) = stack.pop() {
        if body == to {
            return true;
        }
        if seen.contains(&body) {
            continue;
        }
        seen.push(body);
        stack.extend(followed_bodies(document, body));
    }
    false
}

/// Whether `to` is among the tool bodies `from` takes, or theirs, and so
/// on: a body that reaches itself this way can never be built.
fn tools_reach(document: &Document, from: BodyId, to: BodyId) -> bool {
    let mut stack = vec![from];
    let mut seen: Vec<BodyId> = Vec::new();
    while let Some(body) = stack.pop() {
        if body == to {
            return true;
        }
        if seen.contains(&body) {
            continue;
        }
        seen.push(body);
        stack.extend(
            boolean_tools(document, body)
                .into_iter()
                .map(|(_, tool)| tool),
        );
    }
    false
}

/// What a Boolean of `body` is built against, summed up: the tool body's
/// solid (its geometry revision, none before it has one) and where the
/// tool sits relative to the body.
fn tool_inputs(document: &Document, body: BodyId, tool: BodyId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    document
        .imported_geometry(tool)
        .map(|g| g.revision)
        .hash(&mut hasher);
    let relative = document
        .body_placement(body)
        .inverse()
        .after(&document.body_placement(tool));
    for row in relative.rows() {
        for value in row {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
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

fn face_pick_plane(pick: &FacePick) -> ([f64; 3], [f64; 3]) {
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

fn edge_selection(edges: &crate::feature::EdgeSel) -> EdgeSelection {
    match edges {
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
    }
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
        .bodies()
        .iter()
        .find(|b| b.id == body)
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
    let shape_boolean = |subtractive: bool| {
        if subtractive {
            BooleanOp::Cut
        } else {
            additive_boolean
        }
    };

    let start_index = plan.ops.len();
    match &feature {
        DesignFeature::Pad { profile_face, .. } | DesignFeature::Pocket { profile_face, .. } => {
            if profile_face.is_some() && plan.ops.is_empty() {
                return Err(fail(
                    "extruding a face needs a solid to take it from; add a feature first".into(),
                ));
            }
            let boolean = if matches!(feature, DesignFeature::Pocket { .. }) {
                BooleanOp::Cut
            } else {
                additive_boolean
            };
            plan.ops
                .extend(extrude_op(document, feature, boolean).map_err(&fail)?);
        }
        DesignFeature::Revolution {
            sketch,
            angle_deg,
            axis,
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
            axis,
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
            let profile = profile_of(&sketch_feature).map_err(&fail)?;
            let axis_2d = axis_in_sketch(document, &sketch_feature, axis).map_err(&fail)?;
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
                BooleanOp::Cut
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
            refine: _,
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
            refine: _,
            profile,
            spine,
            orientation,
            corner,
            sections,
            subtractive,
            profile_face,
            path_edges,
            path_borrowed,
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
                let profile = sketch_profile(document, *profile).map_err(&fail)?;
                let spine =
                    sketch_spine(document, *spine, profile_anchor(&profile)).map_err(&fail)?;
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
                        sketch_profile(document, *profile).map_err(&fail)?,
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
                        sketch_spine(document, *spine, anchor).map_err(&fail)?,
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
            sketch,
            axis,
            mode,
            pitch,
            height,
            turns,
            left_handed,
            cone_angle_deg,
            reversed,
            subtractive,
            growth,
            keep_inside,
        } => {
            let sketch_feature = load_sketch(document, *sketch).map_err(&fail)?;
            let profile = profile_of(&sketch_feature).map_err(&fail)?;
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
                    axis_in_sketch(document, &sketch_feature, axis).map_err(&fail)?;
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
                op: if *subtractive && *keep_inside {
                    BooleanOp::Common
                } else {
                    shape_boolean(*subtractive)
                },
            });
        }
        DesignFeature::Primitive {
            refine: _,
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
            plan.ops.push(SolidOp::Fillet {
                radius: *radius as f64,
                edges: edge_selection(edges),
                follow_tangent: *follow_tangent,
            });
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
                edges: edge_selection(edges),
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
            originals,
            plane,
            refine: _,
        } => {
            let originals = original_ops(feature_ops, originals).map_err(&fail)?;
            let (point, normal) = mirror_plane(document, plane).map_err(&fail)?;
            plan.ops.push(SolidOp::Transform {
                transforms: vec![mat_mirror(point, normal)],
                originals,
            });
        }
        DesignFeature::LinearPattern {
            refine: _,
            originals,
            axis,
            length,
            occurrences,
            spacing_mode,
            reversed,
            spacings,
        } => {
            let originals = original_ops(feature_ops, originals).map_err(&fail)?;
            let axis = pattern_axis(document, axis).map_err(&fail)?;
            let transforms = linear_transforms(
                axis,
                *length,
                *occurrences,
                *spacing_mode,
                *reversed,
                spacings,
            )
            .map_err(&fail)?;
            if !transforms.is_empty() {
                plan.ops.push(SolidOp::Transform {
                    transforms,
                    originals,
                });
            }
        }
        DesignFeature::PolarPattern {
            refine: _,
            originals,
            axis,
            angle_deg,
            occurrences,
            reversed,
            step_mode,
            angles,
        } => {
            let originals = original_ops(feature_ops, originals).map_err(&fail)?;
            let axis = pattern_axis(document, axis).map_err(&fail)?;
            let transforms = polar_transforms(
                axis,
                *angle_deg,
                *occurrences,
                *reversed,
                *step_mode,
                angles,
            )
            .map_err(&fail)?;
            if !transforms.is_empty() {
                plan.ops.push(SolidOp::Transform {
                    transforms,
                    originals,
                });
            }
        }
        DesignFeature::MultiTransform {
            originals,
            steps,
            refine: _,
        } => {
            let originals = original_ops(feature_ops, originals).map_err(&fail)?;
            let transforms = multi_transforms(document, steps).map_err(&fail)?;
            if !transforms.is_empty() {
                plan.ops.push(SolidOp::Transform {
                    transforms,
                    originals,
                });
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
        DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools,
            refine: _,
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
                if !document.bodies().iter().any(|b| b.id == *tool_body) {
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
fn original_ops(
    feature_ops: &std::collections::HashMap<FeatureId, Vec<usize>>,
    originals: &[FeatureId],
) -> Result<Vec<usize>, String> {
    let mut indices = Vec::new();
    for original in originals {
        let ops = feature_ops
            .get(original)
            .ok_or("a pattern original is missing or comes after the pattern (or is suppressed)")?;
        indices.extend_from_slice(ops);
    }
    Ok(indices)
}

/// One side of an extrusion as a feature gives it: its length and the
/// faces it may stop on.
struct Side<'a> {
    length: f32,
    face: Option<&'a FacePick>,
    offset: f32,
    shape: &'a [FacePick],
    /// Faces other bodies lend that an up-to-shape side stops on too.
    borrowed: &'a [crate::feature::BorrowedRef],
}

/// Where one side of an extrusion ends, in `mode`.
fn side_termination(
    document: &Document,
    mode: ExtrudeMode,
    side: &Side,
) -> Result<ExtrudeTermination, String> {
    match mode {
        ExtrudeMode::Dimension | ExtrudeMode::TwoLengths => {
            if side.length <= 0.0 {
                return Err("length must be positive".into());
            }
            Ok(ExtrudeTermination::Blind {
                distance: side.length as f64,
            })
        }
        ExtrudeMode::ThroughAll => Ok(ExtrudeTermination::ThroughAll),
        ExtrudeMode::ToFirst => Ok(ExtrudeTermination::ToFirst),
        ExtrudeMode::ToLast => Ok(ExtrudeTermination::ToLast),
        ExtrudeMode::UpToFace => {
            let pick = side
                .face
                .ok_or("pick a target face for the up-to-face mode")?;
            let (point, normal) = face_pick_plane(pick);
            Ok(ExtrudeTermination::UpToFace {
                name: pick.name,
                point,
                normal,
                offset: side.offset as f64,
            })
        }
        ExtrudeMode::UpToBorrowed(borrowed) => {
            let face = crate::borrow::kernel_face(document, &borrowed)?;
            Ok(ExtrudeTermination::UpToFaceOf {
                shape: face.shape,
                transform: face.transform.map(Box::new),
                point: face.point,
                offset: side.offset as f64,
            })
        }
        ExtrudeMode::UpToShape => {
            if side.shape.is_empty() && side.borrowed.is_empty() {
                return Err("pick the faces the up-to-shape mode stops on".into());
            }
            let faces = side.shape.iter().map(face_probe).collect();
            if side.borrowed.is_empty() {
                return Ok(ExtrudeTermination::UpToShape {
                    faces,
                    offset: side.offset as f64,
                });
            }
            Ok(ExtrudeTermination::UpToShapeOf {
                faces,
                others: crate::borrow::kernel_faces(document, side.borrowed)?,
                offset: side.offset as f64,
            })
        }
        ExtrudeMode::UpToPlane(target) => {
            let (point, normal) = target_plane(document, &target)?;
            Ok(ExtrudeTermination::UpToPlane {
                point,
                normal,
                offset: side.offset as f64,
            })
        }
    }
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

/// The kernel ops of a pad or a pocket: its profile (a sketch's, or a face
/// of the solid), each side's end and the way it runs. One op, or two when
/// the second side takes its own taper: each side swept on its own.
fn extrude_op(
    document: &Document,
    feature: &DesignFeature,
    boolean: BooleanOp,
) -> Result<Vec<SolidOp>, String> {
    let (sketch, profile_face, reversed, symmetric, taper_deg, direction, mode, mode2) =
        match feature {
            DesignFeature::Pad {
                sketch,
                profile_face,
                reversed,
                symmetric,
                taper_deg,
                direction,
                mode,
                mode2,
                ..
            } => (
                *sketch,
                profile_face.as_ref(),
                *reversed,
                *symmetric,
                *taper_deg,
                *direction,
                *mode,
                *mode2,
            ),
            DesignFeature::Pocket {
                sketch,
                profile_face,
                reversed,
                symmetric,
                through_all,
                taper_deg,
                direction,
                mode,
                mode2,
                ..
            } => (
                *sketch,
                profile_face.as_ref(),
                *reversed,
                *symmetric,
                *taper_deg,
                *direction,
                if *through_all {
                    ExtrudeMode::ThroughAll
                } else {
                    *mode
                },
                *mode2,
            ),
            _ => return Err("not a pad or a pocket".into()),
        };
    let borrowed = match feature {
        DesignFeature::Pad {
            profile_borrowed, ..
        }
        | DesignFeature::Pocket {
            profile_borrowed, ..
        } => *profile_borrowed,
        _ => None,
    };
    let (first, second) = match feature {
        DesignFeature::Pad {
            length,
            length2,
            up_to_face,
            up_to_offset,
            up_to_shape,
            up_to_face2,
            up_to_offset2,
            up_to_shape2,
            extras,
            ..
        }
        | DesignFeature::Pocket {
            depth: length,
            depth2: length2,
            up_to_face,
            up_to_offset,
            up_to_shape,
            up_to_face2,
            up_to_offset2,
            up_to_shape2,
            extras,
            ..
        } => (
            Side {
                length: *length,
                face: up_to_face.as_ref(),
                offset: *up_to_offset,
                shape: up_to_shape,
                borrowed: &extras.up_to_shape_borrowed,
            },
            Side {
                length: *length2,
                face: up_to_face2.as_ref(),
                offset: *up_to_offset2,
                shape: up_to_shape2,
                borrowed: &extras.up_to_shape_borrowed2,
            },
        ),
        _ => return Err("not a pad or a pocket".into()),
    };
    let extras = match feature {
        DesignFeature::Pad { extras, .. } | DesignFeature::Pocket { extras, .. } => extras.clone(),
        _ => Default::default(),
    };
    let (first_mode, second_mode) = mode.sides(mode2);
    let mut termination = side_termination(document, first_mode, &first)?;
    let mut second_side = second_mode
        .map(|mode| side_termination(document, mode, &second))
        .transpose()?;
    // Through all, centred: through all both ways.
    if symmetric && first_mode == ExtrudeMode::ThroughAll && second_side.is_none() {
        second_side = Some(ExtrudeTermination::ThroughAll);
    }

    let normal = match (profile_face, sketch) {
        _ if let Some(r) = borrowed => crate::borrow::lent_face(document, r.borrow, r.index)
            .map(|(_, n)| n.map(f64::from))
            .ok_or("the borrowed face is not flat, or the borrow no longer lends it")?,
        (Some(face), _) => face_pick_plane(face).1,
        (None, Some(sketch)) => profile::plane_of(&load_sketch(document, sketch)?.plane).normal,
        (None, None) => return Err("pick a sketch or a flat face for the profile".into()),
    };
    let custom = match &direction {
        ExtrudeDirection::Borrowed(r) => {
            Some(crate::borrow::edge(document, r)?.direction.map(f64::from))
        }
        ExtrudeDirection::Datum(datum) => Some(datum_direction(document, *datum)?),
        ExtrudeDirection::SketchLine { sketch, element } => {
            Some(sketch_line_direction(document, *sketch, *element)?)
        }
        other => other.vector(),
    };
    if let Some(d) = custom {
        let d = normalize(d).map_err(|_| "the extrusion direction is zero".to_string())?;
        let n = normalize(normal).map_err(|_| "the profile has no normal".to_string())?;
        let slant = dot3(d, n).abs();
        if slant < 1e-3 {
            return Err("the extrusion direction lies in the profile's plane".into());
        }
        // A length along the normal goes further along a slant.
        if extras.along_normal {
            for end in [Some(&mut termination), second_side.as_mut()]
                .into_iter()
                .flatten()
            {
                if let ExtrudeTermination::Blind { distance } = end {
                    *distance /= slant;
                }
            }
        }
    }
    let reversed = if boolean == BooleanOp::Cut && custom.is_none() {
        !reversed
    } else {
        reversed
    };
    let kind = SweepKind::Extrude {
        termination,
        second_side,
        // Centred only in the one mode that offers it.
        symmetric: symmetric && first_mode == ExtrudeMode::Dimension && second_mode.is_none(),
        // A pocket along the normal cuts against it: a sketch on a solid's
        // face has its normal pointing out of the material, so the default
        // digs in. A direction set is the way the cut runs.
        reversed,
        taper_deg: taper_deg as f64,
        direction: custom,
    };
    // The second side on its own, with its own taper.
    let split = match (&kind, extras.taper2_deg) {
        (
            SweepKind::Extrude {
                second_side: Some(_),
                ..
            },
            Some(taper2),
        ) if f64::from(taper2) != taper_deg as f64 => Some(taper2),
        _ => None,
    };
    let (kind, second) = match (split, kind) {
        (
            Some(taper2),
            SweepKind::Extrude {
                termination,
                second_side: Some(back),
                reversed,
                taper_deg,
                direction,
                ..
            },
        ) => (
            SweepKind::Extrude {
                termination,
                second_side: None,
                symmetric: false,
                reversed,
                taper_deg,
                direction,
            },
            Some(SweepKind::Extrude {
                termination: back,
                second_side: None,
                symmetric: false,
                reversed: !reversed,
                taper_deg: f64::from(taper2),
                direction,
            }),
        ),
        (_, kind) => (kind, None),
    };
    let then = if boolean == BooleanOp::NewSolid {
        BooleanOp::Fuse
    } else {
        boolean
    };
    if let Some(r) = borrowed {
        if extras.start_offset != 0.0 || second.is_some() {
            return Err(
                "a start offset or a second side's own taper takes a sketch's profile, not a face"
                    .into(),
            );
        }
        let face = crate::borrow::kernel_face(document, &r)?;
        return Ok(vec![SolidOp::SweepFaceOf {
            shape: face.shape,
            transform: face.transform.map(Box::new),
            point: face.point,
            kind,
            op: boolean,
        }]);
    }
    Ok(match (profile_face, sketch) {
        (Some(_), _) if extras.start_offset != 0.0 => {
            return Err("a start offset takes a sketch's profile, not a face".into());
        }
        (Some(_), _) if second.is_some() => {
            return Err("a second side's own taper takes a sketch's profile, not a face".into());
        }
        (Some(face), _) => vec![SolidOp::SweepFace {
            face: face_probe(face),
            kind,
            op: boolean,
        }],
        (None, Some(sketch)) => {
            let mut profile = sketch_profile(document, sketch)?;
            if extras.start_offset != 0.0 {
                // Along the way the first side runs.
                let way = normalize(custom.unwrap_or(profile.plane.normal))
                    .map_err(|_| "the extrusion direction is zero".to_string())?;
                let s = f64::from(extras.start_offset) * if reversed { -1.0 } else { 1.0 };
                for (o, w) in profile.plane.origin.iter_mut().zip(way) {
                    *o += w * s;
                }
            }
            let mut ops = vec![SolidOp::Sweep {
                profile: profile.clone(),
                kind,
                op: boolean,
            }];
            if let Some(kind) = second {
                ops.push(SolidOp::Sweep {
                    profile,
                    kind,
                    op: then,
                });
            }
            ops
        }
        (None, None) => unreachable!("a profile was required above"),
    })
}

/// The way a datum runs: a line along itself, a plane or coordinate
/// system square to it.
fn datum_direction(document: &Document, datum: FeatureId) -> Result<[f64; 3], String> {
    let data = document
        .feature_values(datum)
        .ok_or("the datum it runs along is gone")?;
    let made = core_document::DatumFeature::from_json(data)
        .map_err(|_| "what it runs along is not a datum".to_string())?;
    let frame = made.frame();
    Ok(match made.shape {
        core_document::DatumShape::Line { .. } => frame.x_axis,
        _ => frame.normal,
    }
    .map(f64::from))
}

/// The way a sketch's line runs, in the body's frame.
fn sketch_line_direction(
    document: &Document,
    sketch: FeatureId,
    element: uuid::Uuid,
) -> Result<[f64; 3], String> {
    let points = crate::datum_refs::sketch_points(document, sketch, element)
        .ok_or("the sketch line it runs along is gone")?;
    let [a, b] = <[[f32; 3]; 2]>::try_from(points)
        .map_err(|_| "what it runs along is not a line".to_string())?;
    Ok([b[0] - a[0], b[1] - a[1], b[2] - a[2]].map(f64::from))
}

/// Where a revolution's (or helix's) axis lies in its sketch: a point and a
/// direction in the sketch's own coordinates. A picked edge or a datum line
/// must run along the sketch plane, and is taken where it falls on it.
pub(crate) fn axis_in_sketch(
    document: &Document,
    sketch: &SketchFeature,
    axis: &RevolveAxis,
) -> Result<([f64; 2], [f64; 2]), String> {
    let plane = profile::plane_of(&sketch.plane);
    let onto_plane = |point: [f64; 3], dir: [f64; 3], what: &str| {
        let d = normalize(dir).map_err(|_| format!("the {what} has no direction"))?;
        let n = normalize(plane.normal).map_err(|_| "the sketch has no normal".to_string())?;
        if dot3(d, n).abs() > 1e-3 {
            return Err(format!(
                "the {what} does not run along the sketch plane, so it cannot be the axis"
            ));
        }
        let rel = [
            point[0] - plane.origin[0],
            point[1] - plane.origin[1],
            point[2] - plane.origin[2],
        ];
        Ok((
            [dot3(rel, plane.x_axis), dot3(rel, plane.y_axis)],
            [dot3(d, plane.x_axis), dot3(d, plane.y_axis)],
        ))
    };
    let (origin, dir) = match axis {
        RevolveAxis::SketchY => ([0.0, 0.0], [0.0, 1.0]),
        RevolveAxis::SketchX => ([0.0, 0.0], [1.0, 0.0]),
        RevolveAxis::Base(base) => onto_plane([0.0; 3], base.vector(), "body's axis")?,
        RevolveAxis::SketchNormal => {
            return Err(
                "the sketch's normal stands square to its plane; only a helix turns about it"
                    .into(),
            );
        }
        RevolveAxis::Custom { origin, dir } => (origin.map(f64::from), dir.map(f64::from)),
        RevolveAxis::Edge(edge) => onto_plane(
            edge.point.map(f64::from),
            edge.direction.map(f64::from),
            "picked edge",
        )?,
        RevolveAxis::Datum(id) => {
            let data = document
                .feature_values(*id)
                .ok_or("the axis's datum line is missing")?;
            let datum = core_document::DatumFeature::from_json(data)
                .map_err(|_| "the axis is not a datum".to_string())?;
            if !matches!(datum.shape, core_document::DatumShape::Line { .. }) {
                return Err("the axis's datum is not a line".into());
            }
            let frame = datum.frame();
            onto_plane(
                frame.origin.map(f64::from),
                frame.x_axis.map(f64::from),
                "datum line",
            )?
        }
        RevolveAxis::Borrowed(r) => {
            let edge = crate::borrow::edge(document, r)?;
            onto_plane(
                edge.point.map(f64::from),
                edge.direction.map(f64::from),
                "borrowed edge",
            )?
        }
        RevolveAxis::SketchLine(id) => {
            let Some(GeometryElement::Line(line)) = sketch.sketch.get_geometry(*id) else {
                return Err("the axis line is not a line of the sketch".into());
            };
            let at = |point| {
                sketch
                    .sketch
                    .point_position(point)
                    .map(|p| [f64::from(p.x), f64::from(p.y)])
                    .ok_or_else(|| "the axis line has lost an end".to_string())
            };
            let (a, b) = (at(line.start)?, at(line.end)?);
            (a, [b[0] - a[0], b[1] - a[1]])
        }
    };
    if dir[0].hypot(dir[1]) < 1e-12 {
        return Err("revolution axis direction is zero".into());
    }
    Ok((origin, dir))
}

/// Build the revolve sweep for a feature. `reversed` flips the axis so the
/// sweep runs the other way around.
fn revolve_kind(
    (axis_origin, axis_dir): ([f64; 2], [f64; 2]),
    angle_deg: f32,
    reversed: bool,
    midplane: bool,
    second_angle_deg: Option<f32>,
    termination: RevolveTermination,
) -> Result<SweepKind, String> {
    if termination == RevolveTermination::Angle && (angle_deg <= 0.0 || angle_deg > 360.0) {
        return Err(format!(
            "revolution angle must be in (0, 360], got {angle_deg}"
        ));
    }
    Ok(SweepKind::Revolve {
        axis_origin,
        axis_dir,
        angle_deg: f64::from(angle_deg),
        second_angle_deg: second_angle_deg.map(f64::from),
        midplane,
        reversed,
        termination,
    })
}

/// A helix's extent as the kernel reads it: pitch and height, and in the
/// growth mode the turn count and growth per turn as given.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HelixExtent {
    pitch: f64,
    height: f64,
    turns: Option<f64>,
    growth: Option<f64>,
}

fn helix_extent(
    mode: HelixMode,
    pitch: f32,
    height: f32,
    turns: f32,
    growth: f32,
) -> Result<HelixExtent, String> {
    let (pitch, height) = match mode {
        HelixMode::PitchHeight => (pitch, height),
        HelixMode::PitchTurns => (pitch, pitch * turns),
        HelixMode::HeightTurns => {
            if turns <= 0.0 {
                return Err("helix turns must be positive".into());
            }
            (height / turns, height)
        }
        HelixMode::HeightTurnsGrowth => {
            if turns <= 0.0 {
                return Err("helix turns must be positive".into());
            }
            if height < 0.0 {
                return Err("helix height must not be negative".into());
            }
            if height == 0.0 && growth == 0.0 {
                return Err("a flat spiral needs a growth per turn".into());
            }
            return Ok(HelixExtent {
                pitch: f64::from(height) / f64::from(turns),
                height: f64::from(height),
                turns: Some(f64::from(turns)),
                growth: Some(f64::from(growth)),
            });
        }
    };
    if pitch <= 0.0 || height <= 0.0 {
        return Err("helix pitch and height must be positive".into());
    }
    Ok(HelixExtent {
        pitch: f64::from(pitch),
        height: f64::from(height),
        turns: None,
        growth: None,
    })
}

/// A hole's drill diameter where it opens, from its thread when it is
/// standards-driven: the tap drill, moved out by the class's allowance (a
/// tapered thread's minor diameter at the face, which its taper narrows
/// from), or else the clearance of its fit (the major diameter where the
/// standard names no clearance).
pub fn hole_diameter(feature: &DesignFeature) -> f32 {
    let DesignFeature::Hole {
        diameter,
        thread,
        threaded,
        fit,
        clearance,
        ..
    } = feature
    else {
        return 0.0;
    };
    let Some((spec, Ok(size))) = thread.as_ref().map(|t| (t, t.resolve())) else {
        return *diameter;
    };
    if let (false, Some(own)) = (*threaded, clearance) {
        return *own;
    }
    if *threaded {
        let drill = if spec.standard.is_tapered() {
            size.minor
        } else {
            size.tap_drill
        };
        (drill + spec.allowance(&size)) as f32
    } else {
        size.clearance.map_or(size.major, |c| c[*fit as usize]) as f32
    }
}

/// The angle a hole's wall leans in by toward its bottom, degrees: a
/// tapped tapered thread's own, else the hole's.
pub fn hole_taper_deg(feature: &DesignFeature) -> f64 {
    let DesignFeature::Hole {
        thread,
        threaded,
        taper_deg,
        ..
    } = feature
    else {
        return 0.0;
    };
    match thread {
        Some(spec) if *threaded && spec.standard.is_tapered() => {
            crate::hole_tables::pipe_taper_deg()
        }
        _ => f64::from(*taper_deg),
    }
}

/// Circle centers + standalone points of a sketch (hole positions).
fn hole_centers(sketch: &Sketch) -> Vec<[f64; 2]> {
    let mut centers = Vec::new();
    let referenced: std::collections::HashSet<_> = sketch
        .geometry
        .iter()
        .flat_map(Sketch::curve_point_ids)
        .collect();
    for element in &sketch.geometry {
        if sketch.is_construction(element.id()) {
            continue;
        }
        match element {
            GeometryElement::Circle(circle) => {
                if let Some(pos) = sketch.point_position(circle.center) {
                    centers.push([pos.x as f64, pos.y as f64]);
                }
            }
            GeometryElement::Point(point) if !referenced.contains(&point.id) => {
                centers.push([point.position.x as f64, point.position.y as f64]);
            }
            _ => {}
        }
    }
    centers
}

/// Translate a hole feature into cut ops: the drill (and its point), then
/// what the hole cut makes of its mouth, then a modeled thread's groove.
fn hole_ops(document: &Document, feature: &DesignFeature) -> Result<Vec<SolidOp>, String> {
    let DesignFeature::Hole {
        sketch,
        depth,
        through_all,
        cut,
        reversed,
        thread,
        threaded,
        modeled_thread,
        thread_depth,
        thread_length,
        drill_point,
        point_in_depth,
        nut_trap,
        ..
    } = feature
    else {
        return Err("not a hole feature".into());
    };
    let size = thread.as_ref().map(ThreadSpec::resolve).transpose()?;
    let sketch_feature = load_sketch(document, *sketch)?;
    let plane = profile::plane_of(&sketch_feature.plane);
    let centers = hole_centers(&sketch_feature.sketch);
    if centers.is_empty() {
        return Err("the hole sketch has no circles or points to place holes at".into());
    }
    let diameter = hole_diameter(feature);
    if diameter <= 0.0 {
        return Err("hole diameter must be positive".into());
    }
    let radius = f64::from(diameter) * 0.5;
    let taper = hole_taper_deg(feature);
    if taper.is_nan() || taper.abs() >= 45.0 {
        return Err("hole taper must lie within ±45°".into());
    }
    let tan_taper = taper.to_radians().tan();
    // The drill's radius `down` into the material.
    let radius_at = |down: f64| radius - down * tan_taper;
    // Into the material: against the sketch normal unless reversed.
    let into = if *reversed {
        plane.normal
    } else {
        plane.normal.map(|c| -c)
    };

    let circles_profile = |radius: f64| Profile {
        plane,
        wires: centers
            .iter()
            .map(|center| ProfileWire {
                names: Vec::new(),
                segments: vec![ProfileSegment::Circle {
                    center: *center,
                    radius,
                }],
            })
            .collect(),
    };
    let cut_extrude = |termination: ExtrudeTermination, taper_deg: f64, radius: f64| {
        SolidOp::Sweep {
            profile: circles_profile(radius),
            kind: SweepKind::Extrude {
                termination,
                second_side: None,
                symmetric: false,
                // Holes drill against the sketch normal by default, like
                // pockets.
                reversed: !*reversed,
                taper_deg,
                direction: None,
            },
            op: BooleanOp::Cut,
        }
    };
    // A cone at every center, `down` into the material, from `wide` there
    // to `narrow` `height` further in.
    let cones = |down: f64, wide: f64, narrow: f64, height: f64| {
        centers
            .iter()
            .map(|center| cone_cut(&plane, *center, into, down, wide, narrow, height))
            .collect::<Vec<_>>()
    };

    let mut ops = Vec::new();
    if *through_all {
        ops.push(cut_extrude(ExtrudeTermination::ThroughAll, -taper, radius));
    } else {
        if *depth <= 0.0 {
            return Err("hole depth must be positive".into());
        }
        let depth = f64::from(*depth);
        match drill_point {
            DrillPoint::Flat => {
                if radius_at(depth) <= 0.0 {
                    return Err("the taper closes the hole before its bottom".into());
                }
                ops.push(cut_extrude(
                    ExtrudeTermination::Blind { distance: depth },
                    -taper,
                    radius,
                ));
            }
            DrillPoint::Angled { angle_deg } => {
                if *angle_deg <= 0.0 || *angle_deg >= 180.0 {
                    return Err("the drill point angle must be in (0, 180) degrees".into());
                }
                let tan_half = (f64::from(*angle_deg) * 0.5).to_radians().tan();
                // The point below a wall ending at radius ρ is ρ / tan(half)
                // tall; within the depth, wall and point share it.
                let wall = if *point_in_depth {
                    (depth - radius / tan_half) / (1.0 - tan_taper / tan_half)
                } else {
                    depth
                };
                if wall <= 0.0 {
                    return Err(
                        "the drill point is deeper than the hole; deepen it or put the point \
                         below the depth"
                            .into(),
                    );
                }
                let bottom = radius_at(wall);
                if bottom <= 0.0 {
                    return Err("the taper closes the hole before its bottom".into());
                }
                ops.push(cut_extrude(
                    ExtrudeTermination::Blind { distance: wall },
                    -taper,
                    radius,
                ));
                ops.extend(cones(wall, bottom, 0.0, bottom / tan_half));
            }
        }
    }

    let cut = match cut {
        HoleCut::Seat { seat } => {
            let (spec, size) = thread
                .as_ref()
                .zip(size.as_ref())
                .filter(|(spec, _)| spec.standard.is_metric())
                .ok_or("a screw seat needs the hole sized from an ISO metric thread")?;
            seat.cut(size.major)
                .ok_or_else(|| format!("there is no {} for {}", seat.label(), spec.size))?
        }
        other => *other,
    };
    match cut {
        HoleCut::None | HoleCut::Seat { .. } => {}
        HoleCut::Counterbore {
            diameter: cb_diameter,
            depth: cb_depth,
        }
        | HoleCut::Spotface {
            diameter: cb_diameter,
            depth: cb_depth,
        } => {
            let name = if matches!(cut, HoleCut::Spotface { .. }) {
                "spotface"
            } else {
                "counterbore"
            };
            if cb_diameter <= diameter {
                return Err(format!("{name} diameter must exceed the hole diameter"));
            }
            if cb_depth <= 0.0 {
                return Err(format!("{name} depth must be positive"));
            }
            ops.push(cut_extrude(
                ExtrudeTermination::Blind {
                    distance: f64::from(cb_depth),
                },
                0.0,
                f64::from(cb_diameter) * 0.5,
            ));
        }
        HoleCut::Countersink {
            diameter: cs_diameter,
            angle_deg,
        } => {
            if cs_diameter <= diameter {
                return Err("countersink diameter must exceed the hole diameter".into());
            }
            if angle_deg <= 0.0 || angle_deg >= 180.0 {
                return Err("countersink angle must be in (0, 180) degrees".into());
            }
            // The cone runs from the countersink diameter at the surface down
            // to the hole diameter; its depth follows from the angle.
            let half_angle = f64::from(angle_deg) * 0.5;
            let cs_depth = (f64::from(cs_diameter) - f64::from(diameter)) * 0.5
                / half_angle.to_radians().tan();
            ops.push(cut_extrude(
                ExtrudeTermination::Blind { distance: cs_depth },
                -half_angle,
                f64::from(cs_diameter) * 0.5,
            ));
        }
        HoleCut::Counterdrill {
            diameter: cd_diameter,
            depth: cd_depth,
            angle_deg,
        } => {
            if cd_depth <= 0.0 {
                return Err("counterdrill depth must be positive".into());
            }
            if angle_deg <= 0.0 || angle_deg >= 180.0 {
                return Err("counterdrill angle must be in (0, 180) degrees".into());
            }
            let wide = f64::from(cd_diameter) * 0.5;
            let cd_depth = f64::from(cd_depth);
            let narrow = radius_at(cd_depth);
            if wide <= narrow || cd_diameter <= diameter {
                return Err("counterdrill diameter must exceed the hole diameter".into());
            }
            let tan_half = (f64::from(angle_deg) * 0.5).to_radians().tan();
            ops.push(cut_extrude(
                ExtrudeTermination::Blind { distance: cd_depth },
                0.0,
                wide,
            ));
            ops.extend(cones(cd_depth, wide, narrow, (wide - narrow) / tan_half));
        }
    }
    if let Some(trap) = nut_trap {
        let metric = thread.as_ref().is_some_and(|t| t.standard.is_metric());
        let nominal = size.as_ref().filter(|_| metric).map(|s| s.major);
        let pocket = nut_pocket(trap, nominal)?;
        if pocket.across_flats <= f64::from(diameter) {
            return Err(format!(
                "the nut trap, {:.2} mm across its flats, must be wider than the hole",
                pocket.across_flats
            ));
        }
        let down = match trap.side {
            NutSide::Top => 0.0,
            NutSide::Bottom if *through_all => {
                return Err(
                    "a nut trap at the bottom needs the hole's depth: give the hole \
                     a depth (where the nut sits) rather than through all"
                        .into(),
                );
            }
            NutSide::Bottom => {
                let depth = f64::from(*depth);
                if pocket.depth >= depth {
                    return Err(format!(
                        "the nut trap, {:.2} mm deep, must be shallower than the hole",
                        pocket.depth
                    ));
                }
                depth - pocket.depth
            }
        };
        let mut at = plane;
        for (origin, into) in at.origin.iter_mut().zip(into) {
            *origin += into * down;
        }
        ops.push(SolidOp::Sweep {
            profile: Profile {
                plane: at,
                wires: centers
                    .iter()
                    .map(|center| ProfileWire {
                        names: Vec::new(),
                        segments: hexagon(*center, pocket.across_flats, trap.turn_deg),
                    })
                    .collect(),
            },
            kind: SweepKind::Extrude {
                termination: ExtrudeTermination::Blind {
                    distance: pocket.depth,
                },
                second_side: None,
                symmetric: false,
                reversed: !*reversed,
                taper_deg: 0.0,
                direction: None,
            },
            op: BooleanOp::Cut,
        });
    }
    if *threaded && *modeled_thread {
        let (spec, size) = thread
            .as_ref()
            .zip(size)
            .ok_or("a modeled thread needs a standard size")?;
        // A through hole's whole depth: as far as the solid reaches.
        let hole_depth = if *through_all {
            document
                .get_feature_meta(*sketch)
                .and_then(|n| n.body)
                .and_then(|b| document.imported_geometry(b))
                .and_then(|g| g.bounds_mm)
                .map(|(lo, hi)| {
                    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
                    f64::from((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
                })
                .ok_or("a through hole's thread needs the solid built first, or a depth given")?
        } else {
            f64::from(*depth)
        };
        let length = match thread_length {
            crate::feature::ThreadLength::Given => f64::from(*thread_depth),
            crate::feature::ThreadLength::HoleDepth => hole_depth,
            crate::feature::ThreadLength::RunOut => hole_depth - 3.0 * size.pitch,
        };
        if length <= 0.0 {
            return Err("give the modeled thread a depth".into());
        }
        let form = ThreadForm {
            wall: radius,
            major: (size.major + spec.allowance(&size)) * 0.5,
            pitch: size.pitch,
            depth: length,
            flank_deg: spec.standard.flank_angle_deg(),
            taper_deg: taper,
            left_handed: spec.left_handed,
        };
        for center in &centers {
            ops.push(thread_cut(&plane, *center, into, &form));
        }
    }
    Ok(ops)
}

/// A cone cut `down` along `into` below a hole center: `wide` in radius
/// there, `narrow` `height` further in.
/// A nut trap's pocket: across its flats and how deep, mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NutPocket {
    pub across_flats: f64,
    pub depth: f64,
}

/// The pocket `trap` cuts for a screw of `nominal` diameter (the hole's
/// metric thread's major diameter, when it has one): the nut's across
/// flats and thickness, each with the clearance, unless the trap gives
/// its own.
pub fn nut_pocket(trap: &NutTrap, nominal: Option<f64>) -> Result<NutPocket, String> {
    let clearance = f64::from(trap.clearance);
    if clearance < 0.0 {
        return Err("the nut trap's clearance must not be negative".into());
    }
    let nut = nominal.and_then(|d| trap.standard.nut(d));
    let across_flats = match (trap.across_flats, nut) {
        (Some(own), _) => f64::from(own),
        (None, Some(nut)) => nut.across_flats + clearance,
        (None, None) => {
            return Err(format!(
                "a nut trap needs the hole sized from an ISO metric thread with a {} nut, or \
                 an across-flats of its own",
                trap.standard.label()
            ));
        }
    };
    let depth = match (trap.depth, nut) {
        (Some(own), _) => f64::from(own),
        (None, Some(nut)) => nut.thickness + clearance,
        (None, None) => {
            return Err("a nut trap of its own size needs its depth".into());
        }
    };
    if across_flats <= 0.0 || depth <= 0.0 {
        return Err("the nut trap's size and depth must be positive".into());
    }
    Ok(NutPocket {
        across_flats,
        depth,
    })
}

/// A regular hexagon about `center`, `across_flats` wide between
/// opposite sides, a corner turned `turn_deg` from the X axis.
fn hexagon(center: [f64; 2], across_flats: f64, turn_deg: f32) -> Vec<ProfileSegment> {
    let radius = across_flats / 3f64.sqrt();
    let corner = |i: usize| {
        let a = f64::from(turn_deg).to_radians() + i as f64 * std::f64::consts::FRAC_PI_3;
        [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
    };
    (0..6)
        .map(|i| ProfileSegment::Line {
            start: corner(i),
            end: corner((i + 1) % 6),
        })
        .collect()
}

fn cone_cut(
    plane: &kernel_api::ProfilePlane,
    center: [f64; 2],
    into: [f64; 3],
    down: f64,
    wide: f64,
    narrow: f64,
    height: f64,
) -> SolidOp {
    let at = |k: usize| {
        plane.origin[k] + plane.x_axis[k] * center[0] + plane.y_axis[k] * center[1] + into[k] * down
    };
    SolidOp::Primitive {
        kind: kernel_api::PrimitiveKind::Cone {
            radius1: wide,
            radius2: narrow,
            height,
            angle_deg: 360.0,
        },
        placement: kernel_api::Placement {
            origin: [at(0), at(1), at(2)],
            x_axis: plane.x_axis,
            z_axis: into,
        },
        op: BooleanOp::Cut,
    }
}

/// The internal thread a hole's wall takes.
struct ThreadForm {
    /// The drilled wall's radius at the face.
    wall: f64,
    /// The thread's major radius at the face.
    major: f64,
    pitch: f64,
    /// How far into the material it runs.
    depth: f64,
    /// The included angle between its flanks, degrees.
    flank_deg: f64,
    /// How far its diameters lean in toward the bottom, degrees.
    taper_deg: f64,
    left_handed: bool,
}

/// The groove of an internal thread, cut into a hole's wall: a tooth
/// space of the form's flank angle from inside the drilled wall out to the
/// thread's major radius, flat-topped a pitch's eighth wide there, swept
/// along a helix at the pitch (narrowing along a cone as the wall tapers)
/// from a pitch above the surface to the form's depth into the material.
fn thread_cut(
    plane: &kernel_api::ProfilePlane,
    center: [f64; 2],
    into: [f64; 3],
    form: &ThreadForm,
) -> SolidOp {
    let pitch = form.pitch;
    let at = |k: usize| plane.origin[k] + plane.x_axis[k] * center[0] + plane.y_axis[k] * center[1];
    let x = plane.x_axis;
    let normal = [
        x[1] * into[2] - x[2] * into[1],
        x[2] * into[0] - x[0] * into[2],
        x[0] * into[1] - x[1] * into[0],
    ];
    // The section, in a plane through the hole's axis: x out from the
    // axis, y down it.
    let section = kernel_api::ProfilePlane {
        origin: [at(0), at(1), at(2)],
        x_axis: x,
        y_axis: into,
        normal,
    };
    // The sweep starts a pitch above the face, where a taper leaves every
    // radius wider by a pitch's worth of it.
    let widen = pitch * form.taper_deg.to_radians().tan();
    let wall = form.wall + widen;
    let major = form.major + widen;
    let inner = (wall - 0.1 * pitch).max(0.1 * wall);
    let crest = pitch / 16.0;
    let root =
        (crest + (major - inner) * (form.flank_deg * 0.5).to_radians().tan()).min(0.45 * pitch);
    let start = -pitch;
    let corners = [
        [inner, start - root],
        [major, start - crest],
        [major, start + crest],
        [inner, start + root],
    ];
    let segments = (0..4)
        .map(|i| ProfileSegment::Line {
            start: corners[i],
            end: corners[(i + 1) % 4],
        })
        .collect();
    SolidOp::Sweep {
        profile: Profile {
            plane: section,
            wires: vec![ProfileWire::new(segments)],
        },
        kind: SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch,
            height: form.depth + pitch,
            left_handed: form.left_handed,
            cone_angle_deg: -form.taper_deg,
            reversed: false,
            turns: None,
            growth: None,
        },
        op: BooleanOp::Cut,
    }
}

/// A loft's section as the kernel takes it: a sketch's profile, a point
/// (a datum point, or a sketch holding only one point), or a face.
fn loft_section(
    document: &Document,
    section: &crate::feature::LoftSection,
) -> Result<kernel_api::LoftSection, String> {
    use crate::feature::LoftSection;
    let id = match section {
        LoftSection::Face(pick) => return Ok(kernel_api::LoftSection::Face(face_probe(pick))),
        LoftSection::Feature(id) => *id,
    };
    let node = document
        .get_feature_meta(id)
        .ok_or("a loft section is missing")?;
    if node.workbench_id.as_str() == core_document::DATUM_KIND {
        let data = document
            .feature_values(id)
            .ok_or("a loft section is missing")?;
        let datum = core_document::DatumFeature::from_json(data)
            .map_err(|_| "a loft section is not a datum".to_string())?;
        if !matches!(datum.shape, core_document::DatumShape::Point) {
            return Err("a datum section of a loft must be a point".into());
        }
        return Ok(kernel_api::LoftSection::Point(
            datum.frame().origin.map(f64::from),
        ));
    }
    let sketch = load_sketch(document, id)?;
    let curves = sketch
        .sketch
        .geometry
        .iter()
        .filter(|g| !matches!(g, wb_sketch::sketch::GeometryElement::Point(_)))
        .count();
    if curves == 0 {
        let points: Vec<_> = sketch
            .sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                wb_sketch::sketch::GeometryElement::Point(p) => Some(p.position),
                _ => None,
            })
            .collect();
        if let [p] = points.as_slice() {
            let plane = profile::plane_of(&sketch.plane);
            let at = |i: usize| {
                plane.origin[i]
                    + plane.x_axis[i] * f64::from(p.x)
                    + plane.y_axis[i] * f64::from(p.y)
            };
            return Ok(kernel_api::LoftSection::Point([at(0), at(1), at(2)]));
        }
    }
    Ok(kernel_api::LoftSection::Profile(profile_of(&sketch)?))
}

fn load_sketch(document: &Document, sketch_id: FeatureId) -> Result<SketchFeature, String> {
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
        .map(|sketch| sketch.plane.normal)
}

fn sketch_profile(document: &Document, sketch_id: FeatureId) -> Result<Profile, String> {
    profile_of(&load_sketch(document, sketch_id)?)
}

fn profile_of(sketch_feature: &SketchFeature) -> Result<Profile, String> {
    let wires = profile::extract_wires(&sketch_feature.sketch).map_err(|e| e.to_string())?;
    Ok(Profile {
        plane: profile::plane_of(&sketch_feature.plane),
        wires,
    })
}

/// The kernel's frame for a pipe's orientation: an auxiliary path read as a
/// spine of its own, a binormal as the direction it names in the body's
/// frame, where every profile of the build sits.
fn pipe_frame(
    document: &Document,
    orientation: &PipeOrientation,
) -> Result<kernel_api::PipeFrame, String> {
    use kernel_api::PipeFrame;
    Ok(match orientation {
        PipeOrientation::Standard => PipeFrame::RotationMinimizing,
        PipeOrientation::Frenet => PipeFrame::Frenet,
        PipeOrientation::Auxiliary { path } => PipeFrame::Auxiliary {
            path: sketch_spine(document, *path, None)
                .map_err(|e| format!("auxiliary path: {e}"))?,
        },
        PipeOrientation::Binormal { x, y, z } => {
            let direction = [f64::from(*x), f64::from(*y), f64::from(*z)];
            if direction.iter().all(|c| *c == 0.0) {
                return Err("the binormal direction is zero".into());
            }
            PipeFrame::Binormal { direction }
        }
        PipeOrientation::Fixed => PipeFrame::Fixed,
    })
}

/// Where a profile sits, near enough to tell which end of a path it is at:
/// its first curve's centre, or where that curve starts when it has none.
fn profile_anchor(profile: &Profile) -> Option<[f64; 3]> {
    let [u, v] = match profile.wires.first()?.segments.first()? {
        ProfileSegment::Line { start, .. } | ProfileSegment::Arc { start, .. } => *start,
        ProfileSegment::Circle { center, .. }
        | ProfileSegment::Ellipse { center, .. }
        | ProfileSegment::EllipseArc { center, .. } => *center,
        ProfileSegment::BSpline { control_points, .. }
        | ProfileSegment::Nurbs { control_points, .. } => *control_points.first()?,
    };
    let plane = &profile.plane;
    Some([0, 1, 2].map(|k| plane.origin[k] + plane.x_axis[k] * u + plane.y_axis[k] * v))
}

/// A sketch's curves as one path, in order. An open path runs from the end
/// nearer `near` (where the profile swept along it sits).
fn sketch_spine(
    document: &Document,
    sketch_id: FeatureId,
    near: Option<[f64; 3]>,
) -> Result<Profile, String> {
    let sketch_feature = load_sketch(document, sketch_id)?;
    let sketch = &sketch_feature.sketch;
    let plane = profile::plane_of(&sketch_feature.plane);

    let curves: Vec<&GeometryElement> = sketch
        .geometry
        .iter()
        .filter(|e| !sketch.is_construction(e.id()))
        .filter(|e| !matches!(e, GeometryElement::Point(_)))
        .collect();
    if curves.is_empty() {
        return Err("the spine sketch has no curves".into());
    }

    // A single circle is a closed spine by itself.
    if curves.len() == 1
        && let GeometryElement::Circle(circle) = curves[0]
    {
        let center = sketch
            .point_position(circle.center)
            .ok_or("spine circle has no center point")?;
        return Ok(Profile {
            plane,
            wires: vec![ProfileWire {
                names: Vec::new(),
                segments: vec![ProfileSegment::Circle {
                    center: [center.x as f64, center.y as f64],
                    radius: circle.radius as f64,
                }],
            }],
        });
    }

    // Order curves into one chain by shared endpoints.
    let mut endpoints: Vec<(uuid::Uuid, uuid::Uuid, usize)> = Vec::new();
    for (i, element) in curves.iter().enumerate() {
        let ids = Sketch::curve_point_ids(element);
        match element {
            GeometryElement::Line(line) => endpoints.push((line.start, line.end, i)),
            GeometryElement::Arc(arc) => endpoints.push((arc.start, arc.end, i)),
            _ => {
                // Other curves with ends (splines) give them as the first
                // and last points they reference.
                if ids.len() >= 2 {
                    endpoints.push((ids[0], *ids.last().unwrap(), i));
                } else {
                    return Err("the spine may only contain connected lines and arcs".into());
                }
            }
        }
    }
    let mut degree: std::collections::HashMap<uuid::Uuid, u32> = std::collections::HashMap::new();
    for (a, b, _) in &endpoints {
        *degree.entry(*a).or_default() += 1;
        *degree.entry(*b).or_default() += 1;
    }
    if degree.values().any(|d| *d > 2) {
        return Err("the spine path branches; it must be a single chain".into());
    }
    // The open ends, in the order the sketch holds its curves.
    let mut odd: Vec<uuid::Uuid> = Vec::new();
    for (a, b, _) in &endpoints {
        for end in [*a, *b] {
            if degree[&end] == 1 && !odd.contains(&end) {
                odd.push(end);
            }
        }
    }
    if odd.len() != 2 && !odd.is_empty() {
        return Err("the spine must be one connected chain".into());
    }

    // An open path starts at the end nearer `near`, else at its first.
    let world = |id: uuid::Uuid| {
        sketch.point_position(id).map(|p| {
            let (u, v) = (f64::from(p.x), f64::from(p.y));
            [0, 1, 2].map(|k| plane.origin[k] + plane.x_axis[k] * u + plane.y_axis[k] * v)
        })
    };
    let distance = |id: uuid::Uuid, to: [f64; 3]| {
        world(id).map_or(f64::INFINITY, |p| {
            (0..3).map(|k| (p[k] - to[k]).powi(2)).sum::<f64>()
        })
    };
    let start = match (odd.as_slice(), near) {
        ([first, second], Some(to)) if distance(*second, to) < distance(*first, to) => *second,
        ([first, ..], _) => *first,
        _ => endpoints[0].0,
    };
    let mut remaining: Vec<(uuid::Uuid, uuid::Uuid, usize)> = endpoints.clone();
    let mut segments = Vec::with_capacity(curves.len());
    let mut cursor = start;
    while !remaining.is_empty() {
        let position = remaining
            .iter()
            .position(|(a, b, _)| *a == cursor || *b == cursor)
            .ok_or("the spine is disconnected")?;
        let (a, b, index) = remaining.swap_remove(position);
        let forward = a == cursor;
        segments.push(spine_segment(sketch, curves[index], forward)?);
        cursor = if forward { b } else { a };
    }

    Ok(Profile {
        plane,
        wires: vec![ProfileWire::new(segments)],
    })
}

fn spine_segment(
    sketch: &Sketch,
    element: &GeometryElement,
    forward: bool,
) -> Result<ProfileSegment, String> {
    let pos = |id: uuid::Uuid| -> Result<[f64; 2], String> {
        sketch
            .point_position(id)
            .map(|p| [p.x as f64, p.y as f64])
            .ok_or_else(|| "spine references a missing point".into())
    };
    match element {
        GeometryElement::Line(line) => {
            let (s, e) = if forward {
                (line.start, line.end)
            } else {
                (line.end, line.start)
            };
            Ok(ProfileSegment::Line {
                start: pos(s)?,
                end: pos(e)?,
            })
        }
        GeometryElement::Arc(arc) => {
            let center = pos(arc.center)?;
            let start = pos(arc.start)?;
            let end = pos(arc.end)?;
            let start_vec = wb_sketch::sketch::Vec2D::new(
                (start[0] - center[0]) as f32,
                (start[1] - center[1]) as f32,
            );
            let end_vec = wb_sketch::sketch::Vec2D::new(
                (end[0] - center[0]) as f32,
                (end[1] - center[1]) as f32,
            );
            let (start_angle, sweep) =
                wb_sketch::snap::arc_angles(start_vec.to_glam(), end_vec.to_glam());
            let mid_angle = (start_angle + sweep * 0.5) as f64;
            let radius = ((start[0] - center[0]).powi(2) + (start[1] - center[1]).powi(2)).sqrt();
            let mid = [
                center[0] + radius * mid_angle.cos(),
                center[1] + radius * mid_angle.sin(),
            ];
            let (s, e) = if forward { (start, end) } else { (end, start) };
            Ok(ProfileSegment::Arc {
                start: s,
                mid,
                end: e,
            })
        }
        _ => Err("the spine may only contain connected lines and arcs".into()),
    }
}

// Pattern transforms: row-major 4x4, last row 0 0 0 1.

type Mat4 = [[f64; 4]; 4];

fn mat_identity() -> Mat4 {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            out[r][c] = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

fn mat_translation(v: [f64; 3]) -> Mat4 {
    let mut m = mat_identity();
    m[0][3] = v[0];
    m[1][3] = v[1];
    m[2][3] = v[2];
    m
}

fn normalize(v: [f64; 3]) -> Result<[f64; 3], String> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-12 {
        return Err("axis direction is zero".into());
    }
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

fn mat_rotation(origin: [f64; 3], dir: [f64; 3], angle_deg: f64) -> Result<Mat4, String> {
    let [x, y, z] = normalize(dir)?;
    let a = angle_deg.to_radians();
    let (s, c) = a.sin_cos();
    let t = 1.0 - c;
    let r = [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ];
    let mut m = mat_identity();
    for i in 0..3 {
        m[i][..3].copy_from_slice(&r[i]);
        // Affine part: rotate about `origin`, not the world origin.
        m[i][3] = origin[i] - r[i][0] * origin[0] - r[i][1] * origin[1] - r[i][2] * origin[2];
    }
    Ok(m)
}

fn mat_mirror(point: [f64; 3], normal: [f64; 3]) -> Mat4 {
    let n = normalize(normal).unwrap_or([0.0, 0.0, 1.0]);
    let d = point[0] * n[0] + point[1] * n[1] + point[2] * n[2];
    let mut m = mat_identity();
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (if i == j { 1.0 } else { 0.0 }) - 2.0 * n[i] * n[j];
        }
        m[i][3] = 2.0 * d * n[i];
    }
    m
}

fn mat_scale(center: [f64; 3], factor: f64) -> Mat4 {
    let mut m = mat_identity();
    for i in 0..3 {
        m[i][i] = factor;
        m[i][3] = center[i] * (1.0 - factor);
    }
    m
}

/// A pattern's axis as a point and a unit direction, in the body's own
/// frame: a picked edge as it was picked, a datum line where the datum
/// sits now, a sketch's axis where the sketch sits now.
pub(crate) fn pattern_axis(
    document: &Document,
    axis: &PatternAxis,
) -> Result<([f64; 3], [f64; 3]), String> {
    let (origin, dir) = match axis {
        PatternAxis::X => ([0.0; 3], [1.0, 0.0, 0.0]),
        PatternAxis::Y => ([0.0; 3], [0.0, 1.0, 0.0]),
        PatternAxis::Z => ([0.0; 3], [0.0, 0.0, 1.0]),
        PatternAxis::Custom { origin, dir } => (origin.map(f64::from), dir.map(f64::from)),
        PatternAxis::Edge(edge) => (edge.point.map(f64::from), edge.direction.map(f64::from)),
        PatternAxis::Datum(id) => {
            let data = document
                .feature_values(*id)
                .ok_or("the pattern's datum line is missing")?;
            let datum = core_document::DatumFeature::from_json(data)
                .map_err(|_| "the pattern's axis is not a datum".to_string())?;
            if !matches!(datum.shape, core_document::DatumShape::Line { .. }) {
                return Err("the pattern's datum is not a line".into());
            }
            let frame = datum.frame();
            (frame.origin.map(f64::from), frame.x_axis.map(f64::from))
        }
        PatternAxis::Sketch { sketch, axis } => {
            let data = document
                .feature_values(*sketch)
                .ok_or("the pattern axis's sketch is missing")?;
            let sketch = SketchFeature::from_json(data)
                .map_err(|_| "the pattern's axis is not a sketch's".to_string())?;
            let plane = profile::plane_of(&sketch.plane);
            let dir = match axis {
                crate::feature::SketchAxis::Horizontal => plane.x_axis,
                crate::feature::SketchAxis::Vertical => plane.y_axis,
                crate::feature::SketchAxis::Normal => plane.normal,
            };
            (plane.origin, dir)
        }
    };
    let dir = normalize(dir).map_err(|_| "the pattern axis has no direction".to_string())?;
    Ok((origin, dir))
}

/// The offsets from the original, first copy to last, of `count` copies
/// spaced `even` apart, save where `gaps` gives the gap before a copy.
fn uneven_offsets(count: u32, even: f64, gaps: &[f32]) -> Vec<f64> {
    let mut at = 0.0;
    (0..count as usize)
        .map(|i| {
            at += gaps.get(i).map_or(even, |gap| f64::from(*gap));
            at
        })
        .collect()
}

fn linear_transforms(
    (_, dir): ([f64; 3], [f64; 3]),
    length: f32,
    occurrences: u32,
    spacing_mode: bool,
    reversed: bool,
    spacings: &[f32],
) -> Result<Vec<Mat4>, String> {
    // One occurrence is the original alone: nothing to copy.
    if occurrences == 0 {
        return Err("a linear pattern needs at least 1 occurrence".into());
    }
    if occurrences == 1 {
        return Ok(Vec::new());
    }
    let sign = if reversed { -1.0 } else { 1.0 };
    let spacing = if spacing_mode {
        f64::from(length)
    } else {
        f64::from(length) / f64::from(occurrences - 1)
    };
    let gaps = spacings.len().min(occurrences as usize - 1);
    if spacing.abs() < 1e-9 && gaps < occurrences as usize - 1 {
        return Err("pattern spacing is zero".into());
    }
    if let Some(i) = spacings[..gaps].iter().position(|gap| gap.abs() < 1e-6) {
        return Err(format!("spacing {} of the pattern is zero", i + 1));
    }
    Ok(uneven_offsets(occurrences - 1, spacing, spacings)
        .into_iter()
        .map(|offset| {
            let d = offset * sign;
            mat_translation([dir[0] * d, dir[1] * d, dir[2] * d])
        })
        .collect())
}

fn polar_transforms(
    (origin, dir): ([f64; 3], [f64; 3]),
    angle_deg: f32,
    occurrences: u32,
    reversed: bool,
    step_mode: bool,
    angles: &[f32],
) -> Result<Vec<Mat4>, String> {
    if occurrences == 0 {
        return Err("a polar pattern needs at least 1 occurrence".into());
    }
    if occurrences == 1 {
        return Ok(Vec::new());
    }
    let full_circle = (f64::from(angle_deg) - 360.0).abs() < 1e-6;
    let step = if step_mode {
        f64::from(angle_deg)
    } else if full_circle {
        // 360° spreads evenly without doubling the original position.
        f64::from(angle_deg) / f64::from(occurrences)
    } else {
        f64::from(angle_deg) / f64::from(occurrences - 1)
    };
    let gaps = angles.len().min(occurrences as usize - 1);
    if step.abs() < 1e-9 && gaps < occurrences as usize - 1 {
        return Err("the pattern's angle between occurrences is zero".into());
    }
    if let Some(i) = angles[..gaps].iter().position(|gap| gap.abs() < 1e-6) {
        return Err(format!("step angle {} of the pattern is zero", i + 1));
    }
    let sign = if reversed { -1.0 } else { 1.0 };
    uneven_offsets(occurrences - 1, step, angles)
        .into_iter()
        .map(|angle| mat_rotation(origin, dir, angle * sign))
        .collect()
}

/// Cartesian composition: each step's occurrences (including the identity)
/// apply to every result of the previous steps; the pure identity is dropped
/// because the base solid already contains the original.
fn multi_transforms(document: &Document, steps: &[TransformStep]) -> Result<Vec<Mat4>, String> {
    if steps.is_empty() {
        return Err("a multi-transform needs at least one step".into());
    }
    let mut accumulated = vec![mat_identity()];
    for step in steps {
        let step_transforms: Vec<Mat4> = match step {
            TransformStep::Linear {
                axis,
                length,
                occurrences,
            } => linear_transforms(
                pattern_axis(document, axis)?,
                *length,
                *occurrences,
                false,
                false,
                &[],
            )?,
            TransformStep::Polar {
                axis,
                angle_deg,
                occurrences,
            } => polar_transforms(
                pattern_axis(document, axis)?,
                *angle_deg,
                *occurrences,
                false,
                false,
                &[],
            )?,
            TransformStep::Mirror { plane } => {
                let (point, normal) = mirror_plane(document, plane)?;
                vec![mat_mirror(point, normal)]
            }
            TransformStep::Scale {
                factor,
                center,
                occurrences,
            } => {
                if *factor <= 0.0 {
                    return Err("scale factor must be positive".into());
                }
                let occ = (*occurrences).max(2);
                // The factor applies to the LAST occurrence; the rest
                // interpolate evenly.
                (1..occ)
                    .map(|k| {
                        let f =
                            1.0 + (f64::from(*factor) - 1.0) * f64::from(k) / f64::from(occ - 1);
                        mat_scale([center[0] as f64, center[1] as f64, center[2] as f64], f)
                    })
                    .collect()
            }
        };
        let mut next = Vec::with_capacity(accumulated.len() * (step_transforms.len() + 1));
        for base in &accumulated {
            next.push(*base);
            for t in &step_transforms {
                next.push(mat_mul(t, base));
            }
        }
        accumulated = next;
    }
    // Drop the identity (index 0 by construction).
    Ok(accumulated.into_iter().skip(1).collect())
}

/// Sketches available in a body (id + display name), for re-attachment:
/// its own and those other bodies lend it.
pub fn sketches_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, String)> {
    let mut sketches: Vec<(u64, FeatureId, String)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(id, n)| {
            n.body == Some(body)
                && (n.workbench_id.as_str() == "wb.sketch"
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

/// Point a part feature at a different sketch: updates the payload, rewires
/// the dependency edges, hides the new sketch (it's consumed) and reveals
/// the old one when nothing else consumes it, then marks for rebuild.
pub fn retarget_feature_sketch(
    document: &mut Document,
    feature_id: FeatureId,
    new_sketch: FeatureId,
) -> Result<(), String> {
    let data = document
        .get_feature_data(feature_id)
        .ok_or("feature not found")?;
    let mut feature = DesignFeature::from_json(data).map_err(|e| e.to_string())?;
    let Some(old_sketch) = feature.sketch() else {
        return Err("this feature is not sketch-based".into());
    };
    if old_sketch == new_sketch {
        return Ok(());
    }
    match &mut feature {
        DesignFeature::Pad { sketch, .. } | DesignFeature::Pocket { sketch, .. } => {
            *sketch = Some(new_sketch)
        }
        DesignFeature::Revolution { sketch, .. }
        | DesignFeature::Groove { sketch, .. }
        | DesignFeature::Helix { sketch, .. }
        | DesignFeature::Hole { sketch, .. } => *sketch = new_sketch,
        DesignFeature::Pipe { profile, .. } => *profile = new_sketch,
        _ => return Err("this feature is not sketch-based".into()),
    }
    document
        .update_feature_data(feature_id, feature.to_json())
        .map_err(|e| e.to_string())?;
    document.set_feature_dependencies(feature_id, feature.dependencies());
    swap_consumed_sketch(document, feature_id, Some(old_sketch), Some(new_sketch));
    Ok(())
}

/// `feature_id` now consumes `new_sketch` instead of `old_sketch` (either
/// none when the profile is a face of the solid): the new one hides, the
/// old one shows again when no other part feature consumes it. Each sketch
/// whose visibility changed, with what it was before.
pub fn swap_consumed_sketch(
    document: &mut Document,
    feature_id: FeatureId,
    old_sketch: Option<FeatureId>,
    new_sketch: Option<FeatureId>,
) -> Vec<(FeatureId, bool)> {
    let mut changed = Vec::new();
    let mut set = |document: &mut Document, id: FeatureId, visible: bool| {
        let was = document.get_feature_meta(id).map(|n| n.visible);
        if was.is_some_and(|was| was != visible) {
            changed.push((id, !visible));
            document.set_feature_visible(id, visible);
        }
    };
    if let Some(new_sketch) = new_sketch {
        set(document, new_sketch, false);
    }
    let Some(old_sketch) = old_sketch else {
        return changed;
    };
    let still_consumed = document
        .feature_tree()
        .all_nodes()
        .filter(|(id, n)| n.workbench_id.as_str() == "wb.design" && **id != feature_id)
        .filter_map(|(_, n)| DesignFeature::from_json(&n.data).ok())
        .any(|f| f.sketches().contains(&old_sketch));
    if !still_consumed {
        set(document, old_sketch, true);
    }
    changed
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

/// Mark every part feature dirty (`invalidate_all`: a history jump or
/// Recompute All, where the solids shown may not match the features).
pub fn mark_all_design_features_dirty(document: &mut Document) {
    let ids: Vec<FeatureId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.design")
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        document.mark_feature_dirty(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::MirrorPlane;
    use crate::hole_tables::{ScrewSeat, ThreadStandard};
    use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, Vec2D};

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

    fn pad(sketch: FeatureId, length: f32) -> DesignFeature {
        DesignFeature::Pad {
            profile_borrowed: None,
            extras: Default::default(),
            refine: false,
            sketch: Some(sketch),
            length,
            reversed: false,
            symmetric: false,
            mode: ExtrudeMode::Dimension,
            length2: 0.0,
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

    fn pocket(sketch: FeatureId, depth: f32, reversed: bool, through_all: bool) -> DesignFeature {
        DesignFeature::Pocket {
            profile_borrowed: None,
            extras: Default::default(),
            refine: false,
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
        let sketch_id = doc
            .add_feature_in_body(rect_sketch(), "sketch".into(), Some(body))
            .unwrap();
        (doc, body, sketch_id)
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

    /// Mark `body` as carrying an imported solid, the way a STEP import does.
    fn import_into(doc: &mut Document, body: BodyId) {
        doc.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
                source_asset: Some(uuid::Uuid::new_v4()),
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
    }

    #[test]
    fn an_imported_body_is_not_rebuilt_from_features() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        import_into(&mut doc, body);
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();

        let err = body_build_ops(&doc, body).expect_err("the import has no history to rebuild");
        assert_eq!(err.feature, Some(pad_id));
        assert!(err.message.contains("import"), "{}", err.message);
    }

    #[test]
    fn a_body_built_from_features_is_not_taken_for_an_import() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        // A rebuild's own result carries no source asset.
        doc.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        assert!(!doc.body_solid_is_imported(body));
        assert!(
            body_build_ops(&doc, body).is_ok(),
            "still rebuilds normally"
        );
    }

    #[test]
    fn pad_produces_new_solid_op() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();

        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
        assert!((extrude_distance(&plan.ops[0]) - 7.0).abs() < 1e-9);
        assert_eq!(plan.op_features.len(), 1);
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
        assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
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
        doc.add_feature_in_body(
            pocket(sketch_id, 3.0, false, false),
            "Pocket".into(),
            Some(body),
        )
        .unwrap();
        let err = body_build_ops(&doc, body).unwrap_err();
        assert!(err.message.contains("material"), "{}", err.message);
    }

    #[test]
    fn rebuild_jobs_settle_the_flags_of_the_plan_and_its_inputs_only() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        // A sketch of another body, dirty on its own: not this plan's input.
        let other = doc.create_body(None);
        let other_sketch = doc
            .add_feature_in_body(rect_sketch(), "Other".into(), Some(other))
            .unwrap();
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
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        let before = body_build_ops(&doc, body).unwrap().ops.len();
        let pattern = |occurrences| DesignFeature::LinearPattern {
            refine: false,
            originals: vec![pad_id],
            axis: PatternAxis::X,
            length: 30.0,
            occurrences,
            spacing_mode: false,
            spacings: Vec::new(),
            reversed: false,
        };
        let id = doc
            .add_feature_in_body(pattern(1), "Pattern".into(), Some(body))
            .unwrap();
        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), before);
        doc.update_feature_data(id, pattern(0).to_json()).unwrap();
        let error = body_build_ops(&doc, body).unwrap_err();
        assert!(error.message.contains("at least 1"), "{}", error.message);

        // Rebuilding, the plan stops at the pattern: the pad before it
        // builds, and a pad after it is left out.
        let later = doc
            .add_feature_in_body(pad(sketch_id, 9.0), "Pad 2".into(), Some(body))
            .unwrap();
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
        let pad_id = doc
            .add_feature_in_body(padded, "Pad".into(), Some(body))
            .unwrap();
        let plain = body_build_ops(&doc, body).unwrap();
        assert!(matches!(plain.ops.last(), Some(SolidOp::Refine)));
        assert_eq!(plain.op_features.last(), Some(&pad_id));
        assert_eq!(plain.ops.len(), plain.op_features.len());

        let pattern = DesignFeature::LinearPattern {
            refine: false,
            originals: vec![pad_id],
            axis: PatternAxis::X,
            length: 30.0,
            occurrences: 2,
            spacing_mode: false,
            spacings: Vec::new(),
            reversed: false,
        };
        doc.add_feature_in_body(pattern, "Pattern".into(), Some(body))
            .unwrap();
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
        match edge_selection(&crate::feature::EdgeSel::Edges(picks)) {
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
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        let pocket_id = doc
            .add_feature_in_body(
                pocket(sketch_id, 2.0, false, false),
                "Pocket".into(),
                Some(body),
            )
            .unwrap();
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
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        doc.clear_feature_dirty(pad_id);
        invalidate_body(&mut doc, body);
        assert!(doc.get_feature_meta(pad_id).unwrap().dirty);

        doc.remove_feature(pad_id).unwrap();
        doc.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        invalidate_body(&mut doc, body);
        assert!(
            doc.imported_geometry(body).is_some(),
            "another bench's features may build it: the registry clears it"
        );
    }

    #[test]
    fn dirty_pad_flags_body_for_rebuild_and_sketch_edit_propagates() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        assert!(pending_body_rebuilds(&doc).is_empty());

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
            .add_feature_in_body(pad(sketch_id, 5.0), "PadA".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(pad(sketch_id, 9.0), "PadB".into(), Some(body))
            .unwrap();

        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
        doc.set_feature_suppressed(pad_a, true);
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 1);
        // The remaining pad becomes the first op → NewSolid.
        assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
        assert!((extrude_distance(&plan.ops[0]) - 9.0).abs() < 1e-9);
    }

    #[test]
    fn revolution_and_groove_map_to_revolve_kind() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            DesignFeature::Revolution {
                refine: false,
                sketch: sketch_id,
                angle_deg: 270.0,
                axis: RevolveAxis::SketchY,
                reversed: false,
                midplane: false,
                second_angle_deg: None,
                mode: Default::default(),
                up_to_face: None,
            },
            "Revolution".into(),
            Some(body),
        )
        .unwrap();
        doc.add_feature_in_body(
            DesignFeature::Groove {
                refine: false,
                sketch: sketch_id,
                angle_deg: 90.0,
                axis: RevolveAxis::SketchX,
                reversed: true,
                midplane: false,
                second_angle_deg: None,
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

    #[test]
    fn through_all_pocket_maps_to_through_all_termination() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(sketch_id, 5.0), "Pad".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(
            pocket(sketch_id, 1.0, false, true),
            "Pocket".into(),
            Some(body),
        )
        .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        assert!(matches!(
            &plan.ops[1],
            SolidOp::Sweep {
                kind: SweepKind::Extrude {
                    termination: ExtrudeTermination::ThroughAll,
                    ..
                },
                ..
            }
        ));
    }

    /// Through all, centred on the sketch: through all both ways.
    #[test]
    fn a_symmetric_through_all_pocket_cuts_both_ways() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(sketch_id, 5.0), "Pad".into(), Some(body))
            .unwrap();
        let mut centred = pocket(sketch_id, 1.0, false, true);
        if let DesignFeature::Pocket { symmetric, .. } = &mut centred {
            *symmetric = true;
        }
        doc.add_feature_in_body(centred, "Pocket".into(), Some(body))
            .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        assert!(matches!(
            &plan.ops[1],
            SolidOp::Sweep {
                kind: SweepKind::Extrude {
                    termination: ExtrudeTermination::ThroughAll,
                    second_side: Some(ExtrudeTermination::ThroughAll),
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn hole_feature_emits_cut_circles() {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(base_sketch, 5.0), "Pad".into(), Some(body))
            .unwrap();

        let mut hole_sketch = Sketch::new("holes");
        for x in [2.0f32, 8.0] {
            let center =
                hole_sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 2.5))));
            hole_sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
        }
        let plane = hole_sketch.plane;
        let hole_sketch_id = doc
            .add_feature_in_body(
                SketchFeature::new(hole_sketch, plane),
                "holes".into(),
                Some(body),
            )
            .unwrap();
        doc.add_feature_in_body(
            DesignFeature::Hole {
                clearance: None,
                thread_length: Default::default(),
                refine: false,
                sketch: hole_sketch_id,
                diameter: 3.0,
                depth: 4.0,
                through_all: false,
                cut: HoleCut::Counterbore {
                    diameter: 6.0,
                    depth: 1.5,
                },
                thread: None,
                threaded: false,
                modeled_thread: false,
                thread_depth: 0.0,
                fit: crate::feature::HoleFit::Normal,
                drill_point: DrillPoint::Flat,
                point_in_depth: false,
                taper_deg: 0.0,
                nut_trap: None,
                reversed: false,
            },
            "Hole".into(),
            Some(body),
        )
        .unwrap();

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

    /// A size the table does not have is an error on the hole, whatever
    /// its thread settings, never a panic.
    #[test]
    fn a_body_is_never_its_own_boolean_tool() {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(base_sketch, 5.0), "Pad".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(
            DesignFeature::BodyBoolean {
                more_tools: Vec::new(),
                refine: false,
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
        doc.add_feature_in_body(pad(base_sketch, 5.0), "Pad".into(), Some(body))
            .unwrap();
        let tools: Vec<BodyId> = (0..2)
            .map(|_| {
                let tool = doc.create_body(None);
                doc.set_imported_geometry(
                    tool,
                    core_document::ImportedGeometry {
                        mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
                        source_asset: None,
                        revision: 0,
                        bounds_mm: None,
                        brep_blob_path: None,
                        mesh_path: None,
                        face_colors_path: None,
                        health: None,
                    },
                );
                tool
            })
            .collect();
        doc.add_feature_in_body(
            DesignFeature::BodyBoolean {
                more_tools: vec![tools[1]],
                refine: false,
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
        doc.add_feature_in_body(pad(base_sketch, 20.0), "Pad".into(), Some(body))
            .unwrap();
        let mut hole_sketch = Sketch::new("holes");
        hole_sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 2.5))));
        let plane = hole_sketch.plane;
        let sketch = doc
            .add_feature_in_body(
                SketchFeature::new(hole_sketch, plane),
                "holes".into(),
                Some(body),
            )
            .unwrap();
        let hole = DesignFeature::Hole {
            clearance,
            thread_length: length,
            refine: false,
            sketch,
            diameter: 3.0,
            depth: 10.0,
            through_all: false,
            cut: HoleCut::None,
            thread: Some(crate::feature::ThreadSpec::new(
                crate::hole_tables::ThreadStandard::IsoMetricCoarse,
                "M6",
            )),
            threaded,
            modeled_thread: threaded,
            thread_depth: 4.0,
            fit: crate::feature::HoleFit::Normal,
            drill_point: DrillPoint::Flat,
            point_in_depth: false,
            taper_deg: 0.0,
            nut_trap: None,
            reversed: false,
        };
        let ops = hole_ops(&doc, &hole).unwrap();
        (hole, ops)
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
        doc.add_feature_in_body(pad(sketch, 5.0), "Pad".into(), Some(body))
            .unwrap();
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

    #[test]
    fn a_clearance_of_ones_own_takes_the_fits_place() {
        use crate::feature::ThreadLength;
        let (normal, _) = m6_hole(false, ThreadLength::Given, None);
        assert!(
            (hole_diameter(&normal) - 6.6).abs() < 1e-4,
            "ISO 273 normal"
        );
        let (own, _) = m6_hole(false, ThreadLength::Given, Some(6.3));
        assert!((hole_diameter(&own) - 6.3).abs() < 1e-6);
    }

    #[test]
    fn a_hole_of_a_size_the_table_lacks_fails_cleanly() {
        for (threaded, modeled_thread) in [(false, false), (true, false), (true, true)] {
            let (mut doc, body, base_sketch) = doc_with_body_sketch();
            doc.add_feature_in_body(pad(base_sketch, 5.0), "Pad".into(), Some(body))
                .unwrap();
            let mut hole_sketch = Sketch::new("holes");
            let center =
                hole_sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 2.5))));
            hole_sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
            let plane = hole_sketch.plane;
            let hole_sketch_id = doc
                .add_feature_in_body(
                    SketchFeature::new(hole_sketch, plane),
                    "holes".into(),
                    Some(body),
                )
                .unwrap();
            doc.add_feature_in_body(
                DesignFeature::Hole {
                    clearance: None,
                    thread_length: Default::default(),
                    refine: false,
                    sketch: hole_sketch_id,
                    diameter: 3.0,
                    depth: 4.0,
                    through_all: false,
                    cut: HoleCut::None,
                    thread: Some(crate::feature::ThreadSpec::new(
                        crate::hole_tables::ThreadStandard::IsoMetricCoarse,
                        "M999",
                    )),
                    threaded,
                    modeled_thread,
                    thread_depth: 3.0,
                    fit: crate::feature::HoleFit::Normal,
                    drill_point: DrillPoint::Flat,
                    point_in_depth: false,
                    taper_deg: 0.0,
                    nut_trap: None,
                    reversed: false,
                },
                "Hole".into(),
                Some(body),
            )
            .unwrap();
            let error = body_build_ops(&doc, body).unwrap_err();
            assert!(
                error.message.contains("no ISO metric coarse size \"M999\""),
                "{}",
                error.message
            );
        }
    }

    /// A padded body with one hole position and `hole` drilled there: the
    /// hole's ops, or its build error.
    fn hole_plan(edit: impl FnOnce(&mut DesignFeature)) -> Result<Vec<SolidOp>, String> {
        let (mut doc, body, base_sketch) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(base_sketch, 10.0), "Pad".into(), Some(body))
            .unwrap();
        let mut hole_sketch = Sketch::new("holes");
        hole_sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 2.5))));
        let plane = hole_sketch.plane;
        let hole_sketch_id = doc
            .add_feature_in_body(
                SketchFeature::new(hole_sketch, plane),
                "holes".into(),
                Some(body),
            )
            .unwrap();
        let mut hole = DesignFeature::Hole {
            clearance: None,
            thread_length: Default::default(),
            refine: false,
            sketch: hole_sketch_id,
            diameter: 3.0,
            depth: 6.0,
            through_all: false,
            cut: HoleCut::None,
            thread: None,
            threaded: false,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: crate::feature::HoleFit::Normal,
            drill_point: DrillPoint::Flat,
            point_in_depth: false,
            taper_deg: 0.0,
            nut_trap: None,
            reversed: false,
        };
        edit(&mut hole);
        doc.add_feature_in_body(hole, "Hole".into(), Some(body))
            .unwrap();
        body_build_ops(&doc, body)
            .map(|plan| plan.ops[1..].to_vec())
            .map_err(|e| e.message)
    }

    fn set_hole(f: &mut DesignFeature, edit: impl FnOnce(&mut HoleFields)) {
        let DesignFeature::Hole {
            thread,
            threaded,
            modeled_thread,
            thread_depth,
            cut,
            drill_point,
            point_in_depth,
            taper_deg,
            through_all,
            ..
        } = f
        else {
            unreachable!()
        };
        edit(&mut HoleFields {
            thread,
            threaded,
            modeled_thread,
            thread_depth,
            cut,
            drill_point,
            point_in_depth,
            taper_deg,
            through_all,
        });
    }

    struct HoleFields<'a> {
        thread: &'a mut Option<ThreadSpec>,
        threaded: &'a mut bool,
        modeled_thread: &'a mut bool,
        thread_depth: &'a mut f32,
        cut: &'a mut HoleCut,
        drill_point: &'a mut DrillPoint,
        point_in_depth: &'a mut bool,
        taper_deg: &'a mut f32,
        through_all: &'a mut bool,
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
            hole_plan(|f| {
                set_hole(f, |h| {
                    *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
                    *h.point_in_depth = in_depth;
                })
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
        let through = hole_plan(|f| {
            set_hole(f, |h| {
                *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
                *h.through_all = true;
            })
        })
        .unwrap();
        assert_eq!(through.len(), 1);
    }

    #[test]
    fn a_point_deeper_than_the_hole_is_an_error() {
        let error = hole_plan(|f| {
            if let DesignFeature::Hole { depth, .. } = f {
                *depth = 0.5;
            }
            set_hole(f, |h| {
                *h.drill_point = DrillPoint::Angled { angle_deg: 135.0 };
                *h.point_in_depth = true;
            })
        })
        .unwrap_err();
        assert!(error.contains("drill point is deeper"), "{error}");
    }

    #[test]
    fn a_tapered_hole_narrows_its_extrude_and_its_point() {
        let ops = hole_plan(|f| {
            set_hole(f, |h| {
                *h.taper_deg = 3.0;
                *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
            })
        })
        .unwrap();
        let (distance, taper) = blind_distance(&ops[0]);
        assert!((distance - 6.0).abs() < 1e-9 && (taper + 3.0).abs() < 1e-9);
        let (r1, ..) = cone_of(&ops[1]);
        assert!((r1 - (1.5 - 6.0 * 3f64.to_radians().tan())).abs() < 1e-9);
        let closed = hole_plan(|f| set_hole(f, |h| *h.taper_deg = 40.0)).unwrap_err();
        assert!(closed.contains("taper closes"), "{closed}");
    }

    #[test]
    fn spotface_and_counterdrill_cut_their_mouths() {
        let spot = hole_plan(|f| {
            set_hole(f, |h| {
                *h.cut = HoleCut::Spotface {
                    diameter: 6.0,
                    depth: 0.5,
                }
            })
        })
        .unwrap();
        assert_eq!(spot.len(), 2);
        assert!((blind_distance(&spot[1]).0 - 0.5).abs() < 1e-9);
        let drill = hole_plan(|f| {
            set_hole(f, |h| {
                *h.cut = HoleCut::Counterdrill {
                    diameter: 5.0,
                    depth: 2.0,
                    angle_deg: 90.0,
                }
            })
        })
        .unwrap();
        assert_eq!(drill.len(), 3, "drill, bore, cone");
        assert!((blind_distance(&drill[1]).0 - 2.0).abs() < 1e-9);
        let (r1, r2, h, origin) = cone_of(&drill[2]);
        assert!((r1 - 2.5).abs() < 1e-9 && (r2 - 1.5).abs() < 1e-9 && (h - 1.0).abs() < 1e-9);
        assert!((origin[2] + 2.0).abs() < 1e-9);
        let narrow = hole_plan(|f| {
            set_hole(f, |h| {
                *h.cut = HoleCut::Spotface {
                    diameter: 2.0,
                    depth: 0.5,
                }
            })
        })
        .unwrap_err();
        assert!(narrow.contains("spotface diameter"), "{narrow}");
    }

    #[test]
    fn a_screw_seat_takes_its_size_from_the_metric_thread() {
        let seat = |standard: ThreadStandard, size: &str, seat: ScrewSeat| {
            hole_plan(|f| {
                set_hole(f, |h| {
                    *h.thread = Some(ThreadSpec::new(standard, size));
                    *h.cut = HoleCut::Seat { seat };
                })
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
            let ops = hole_plan(|f| {
                set_hole(f, |h| {
                    *h.thread = Some(spec);
                    *h.threaded = true;
                    *h.modeled_thread = true;
                    *h.thread_depth = 4.0;
                })
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
        let error = hole_plan(|f| set_hole(f, |h| *h.thread = Some(spec))).unwrap_err();
        assert!(error.contains("UNC has no class 6H"), "{error}");
    }

    #[test]
    fn a_tapered_standard_tapers_the_tapped_drill_only() {
        let drilled = |threaded: bool| {
            hole_plan(|f| {
                set_hole(f, |h| {
                    *h.thread = Some(ThreadSpec::new(ThreadStandard::BspTaper, "1/4"));
                    *h.threaded = threaded;
                    *h.taper_deg = 7.0;
                })
            })
            .unwrap()
        };
        let (_, taper) = blind_distance(&drilled(true)[0]);
        assert!((taper + crate::hole_tables::pipe_taper_deg()).abs() < 1e-9);
        let (_, taper) = blind_distance(&drilled(false)[0]);
        assert!((taper + 7.0).abs() < 1e-9, "a clearance hole keeps its own");
    }

    #[test]
    fn metric_hole_diameter_uses_the_table() {
        let feature = DesignFeature::Hole {
            clearance: None,
            thread_length: Default::default(),
            refine: false,
            sketch: FeatureId::new(),
            diameter: 99.0,
            depth: 4.0,
            through_all: false,
            cut: HoleCut::None,
            thread: Some(crate::feature::ThreadSpec::new(
                crate::hole_tables::ThreadStandard::IsoMetricCoarse,
                "M6",
            )),
            threaded: true,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: crate::feature::HoleFit::Normal,
            drill_point: DrillPoint::Flat,
            point_in_depth: false,
            taper_deg: 0.0,
            nut_trap: None,
            reversed: false,
        };
        assert!((hole_diameter(&feature) - 5.0).abs() < 1e-6, "M6 tap drill");
        let clearance = DesignFeature::Hole {
            clearance: None,
            thread_length: Default::default(),
            refine: false,
            sketch: FeatureId::new(),
            diameter: 99.0,
            depth: 4.0,
            through_all: false,
            cut: HoleCut::None,
            thread: Some(crate::feature::ThreadSpec::new(
                crate::hole_tables::ThreadStandard::IsoMetricCoarse,
                "M6",
            )),
            threaded: false,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: crate::feature::HoleFit::Normal,
            drill_point: DrillPoint::Flat,
            point_in_depth: false,
            taper_deg: 0.0,
            nut_trap: None,
            reversed: false,
        };
        assert!(
            (hole_diameter(&clearance) - 6.6).abs() < 1e-6,
            "M6 normal fit"
        );
    }

    #[test]
    fn linear_pattern_emits_transform_op_with_original_indices() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_id = doc
            .add_feature_in_body(pad(sketch_id, 5.0), "Pad".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(
            DesignFeature::LinearPattern {
                refine: false,
                originals: vec![pad_id],
                axis: PatternAxis::X,
                length: 30.0,
                occurrences: 4,
                spacing_mode: false,
                spacings: Vec::new(),
                reversed: false,
            },
            "Pattern".into(),
            Some(body),
        )
        .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 2);
        let SolidOp::Transform {
            transforms,
            originals,
        } = &plan.ops[1]
        else {
            panic!("expected transform op");
        };
        assert_eq!(originals, &vec![0]);
        assert_eq!(transforms.len(), 3, "occurrences minus the original");
        // Overall length 30 over 4 occurrences → 10 mm spacing.
        assert!((transforms[0][0][3] - 10.0).abs() < 1e-9);
        assert!((transforms[2][0][3] - 30.0).abs() < 1e-9);
    }

    #[test]
    fn polar_full_circle_spacing_avoids_overlap() {
        let z = ([0.0; 3], [0.0, 0.0, 1.0]);
        let transforms = polar_transforms(z, 360.0, 4, false, false, &[]).unwrap();
        assert_eq!(transforms.len(), 3);
        // First occurrence at 90°: X axis maps to Y.
        let t = &transforms[0];
        assert!((t[0][0]).abs() < 1e-9 && (t[1][0] - 1.0).abs() < 1e-9);
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
    fn uneven_spacing_puts_each_copy_its_own_gap_past_the_last() {
        let x = ([0.0; 3], [1.0, 0.0, 0.0]);
        let transforms = linear_transforms(x, 30.0, 4, false, false, &[5.0, 15.0]).unwrap();
        let at: Vec<f64> = transforms.iter().map(|t| t[0][3]).collect();
        // 5, then 15 more, then the even 10 more.
        assert_eq!(at, vec![5.0, 20.0, 30.0]);
        let reversed = linear_transforms(x, 30.0, 3, false, true, &[5.0, 15.0]).unwrap();
        assert_eq!(reversed[1][0][3], -20.0);
        let err = linear_transforms(x, 30.0, 3, false, false, &[5.0, 0.0]).unwrap_err();
        assert!(err.contains("spacing 2"), "{err}");
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
    fn mirror_transform_reflects_across_plane() {
        let m = mat_mirror([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]);
        // Point at z=2 reflects to z=8.
        let z = m[2][0] * 0.0 + m[2][1] * 0.0 + m[2][2] * 2.0 + m[2][3];
        assert!((z - 8.0).abs() < 1e-9);
    }

    #[test]
    fn multi_transform_composes_cartesian_product() {
        let steps = vec![
            TransformStep::Linear {
                axis: PatternAxis::X,
                length: 10.0,
                occurrences: 2,
            },
            TransformStep::Linear {
                axis: PatternAxis::Y,
                length: 10.0,
                occurrences: 2,
            },
        ];
        let transforms = multi_transforms(&Document::new("t"), &steps).unwrap();
        // 2x2 grid minus the original.
        assert_eq!(transforms.len(), 3);
    }

    #[test]
    fn pattern_referencing_later_feature_fails() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let ghost = FeatureId::new();
        doc.add_feature_in_body(pad(sketch_id, 5.0), "Pad".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(
            DesignFeature::Mirrored {
                refine: false,
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

    #[test]
    fn spine_orders_segments_into_one_chain() {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        // An L-shaped open path: two lines sharing a corner point.
        let mut sketch = Sketch::new("path");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(20.0, 0.0))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(20.0, 15.0))));
        // Insert out of order to exercise the chain walk.
        sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let plane = sketch.plane;
        let spine_id = doc
            .add_feature_in_body(SketchFeature::new(sketch, plane), "path".into(), Some(body))
            .unwrap();

        let spine = sketch_spine(&doc, spine_id, None).unwrap();
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
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let mut sketch = Sketch::new("path");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 10.0))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        let plane = sketch.plane;
        let spine_id = doc
            .add_feature_in_body(SketchFeature::new(sketch, plane), "path".into(), Some(body))
            .unwrap();
        let first_start = |near| {
            let spine = sketch_spine(&doc, spine_id, near).unwrap();
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
    fn branching_spine_is_rejected() {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let mut sketch = Sketch::new("branch");
        let hub = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        for (x, y) in [(10.0, 0.0), (0.0, 10.0), (-10.0, 0.0)] {
            let tip = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
            sketch.add_geometry(GeometryElement::Line(Line::new(hub, tip)));
        }
        let plane = sketch.plane;
        let spine_id = doc
            .add_feature_in_body(
                SketchFeature::new(sketch, plane),
                "branch".into(),
                Some(body),
            )
            .unwrap();
        assert!(
            sketch_spine(&doc, spine_id, None)
                .unwrap_err()
                .contains("branches")
        );
    }

    #[test]
    fn tip_excludes_later_features_from_the_build() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_a = doc
            .add_feature_in_body(pad(sketch_id, 5.0), "PadA".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(pad(sketch_id, 9.0), "PadB".into(), Some(body))
            .unwrap();

        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
        doc.set_body_tip(body, Some(pad_a));
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(plan.ops.len(), 1, "features after the tip are excluded");
        assert!((extrude_distance(&plan.ops[0]) - 5.0).abs() < 1e-9);

        doc.set_body_tip(body, None);
        assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
    }

    #[test]
    fn history_reorder_swaps_build_order_and_respects_dependencies() {
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        let pad_a = doc
            .add_feature_in_body(pad(sketch_id, 5.0), "PadA".into(), Some(body))
            .unwrap();
        let pad_b = doc
            .add_feature_in_body(pad(sketch_id, 9.0), "PadB".into(), Some(body))
            .unwrap();

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
                    refine: false,
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
        let pad_id = doc
            .add_feature_in_body(pad(base, 5.0), "Pad".into(), Some(body))
            .unwrap();
        let cut_sketch = doc
            .add_feature_in_body(rect_sketch(), "cut".into(), Some(body))
            .unwrap();
        let pocket = doc
            .add_feature_in_body(
                DesignFeature::Pocket {
                    profile_borrowed: None,
                    extras: Default::default(),
                    refine: false,
                    sketch: Some(cut_sketch),
                    depth: 2.0,
                    reversed: false,
                    symmetric: false,
                    through_all: false,
                    mode: crate::ExtrudeMode::Dimension,
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
                },
                "Pocket".into(),
                Some(body),
            )
            .unwrap();
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
        let sketch_b = doc
            .add_feature_in_body(rect_sketch(), "sketch_b".into(), Some(body))
            .unwrap();
        let pad_id = doc
            .add_feature_in_body(pad(sketch_a, 5.0), "Pad".into(), Some(body))
            .unwrap();
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
            doc.add_feature_in_body(pad(sketch, 5.0), "Base".into(), Some(body))
                .unwrap();
            let mut feature = feature;
            match &mut feature {
                DesignFeature::Pad { sketch: s, .. } | DesignFeature::Pocket { sketch: s, .. } => {
                    *s = Some(sketch)
                }
                _ => unreachable!(),
            }
            let id = doc
                .add_feature_in_body(feature, "F".into(), Some(body))
                .unwrap();
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
        let pad_id = doc
            .add_feature_in_body(pad(before, 5.0), "Pad".into(), Some(body))
            .unwrap();
        let after = doc
            .add_feature_in_body(rect_sketch(), "after".into(), Some(body))
            .unwrap();
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
    fn an_open_profile_fails_on_its_feature() {
        let (mut doc, body, _) = doc_with_body_sketch();
        let mut sketch = Sketch::new("open");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let plane = sketch.plane;
        let open_id = doc
            .add_feature_in_body(SketchFeature::new(sketch, plane), "open".into(), Some(body))
            .unwrap();
        let bad = doc
            .add_feature_in_body(pad(open_id, 7.0), "BadPad".into(), Some(body))
            .unwrap();
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
        doc.add_feature_in_body(pad(sketch_id, 5.0), "Base".into(), Some(body))
            .unwrap();
        let mut feature = feature;
        if let DesignFeature::Pad { sketch, .. } | DesignFeature::Pocket { sketch, .. } =
            &mut feature
        {
            *sketch = Some(sketch_id);
        }
        edit(&mut feature);
        doc.add_feature_in_body(feature, "Feature".into(), Some(body))
            .unwrap();
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
        doc.add_feature_in_body(first, "Pad".into(), Some(body))
            .unwrap();
        assert!(body_build_ops(&doc, body).is_err());
    }

    /// A revolution's end mode reaches the kernel as its termination, and
    /// a stop on a face needs material before it.
    #[test]
    fn a_revolution_takes_its_end_mode() {
        let revolution =
            |mode: RevolveMode, up_to_face: Option<FacePick>, sketch| DesignFeature::Revolution {
                refine: false,
                sketch,
                angle_deg: 360.0,
                axis: RevolveAxis::SketchY,
                reversed: false,
                midplane: false,
                second_angle_deg: None,
                mode,
                up_to_face,
            };
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(
            revolution(RevolveMode::ToLast, None, sketch_id),
            "Rev".into(),
            Some(body),
        )
        .unwrap();
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
            doc.add_feature_in_body(pad(sketch_id, 1.0), "Base".into(), Some(body))
                .unwrap();
            doc.add_feature_in_body(
                revolution(mode, Some(face), sketch_id),
                "Rev".into(),
                Some(body),
            )
            .unwrap();
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
        let line_axis = axis_in_sketch(&doc, &feature, &RevolveAxis::SketchLine(line)).unwrap();
        assert!(near(line_axis, ([-1.0, 0.0], [0.0, 3.0])), "{line_axis:?}");
        // The YZ plane pushed back 4 along its normal (world X): a line
        // along world Y through x = -4.
        let datum_axis = axis_in_sketch(&doc, &feature, &RevolveAxis::Datum(datum)).unwrap();
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
        let edge_axis = axis_in_sketch(&doc, &feature, &edge([1.0, 0.0, 0.0])).unwrap();
        assert!(near(edge_axis, ([2.0, 7.0], [1.0, 0.0])), "{edge_axis:?}");
        let across = axis_in_sketch(&doc, &feature, &edge([0.0, 0.0, 1.0]));
        assert!(across.unwrap_err().contains("sketch plane"));
        let missing = axis_in_sketch(
            &doc,
            &feature,
            &RevolveAxis::SketchLine(uuid::Uuid::new_v4()),
        );
        assert!(missing.is_err());
    }

    fn helix(sketch: FeatureId, mode: HelixMode, height: f32, growth: f32) -> DesignFeature {
        DesignFeature::Helix {
            refine: false,
            sketch,
            axis: RevolveAxis::SketchY,
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
        doc.add_feature_in_body(feature, "Helix".into(), Some(body))
            .unwrap();
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
        doc.add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
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
        doc.add_feature_in_body(cut.clone(), "Helix".into(), Some(body))
            .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Common);

        // Kept inside means nothing on an adding helix.
        let (mut doc, body, sketch_id) = doc_with_body_sketch();
        doc.add_feature_in_body(pad(sketch_id, 7.0), "Pad".into(), Some(body))
            .unwrap();
        if let DesignFeature::Helix {
            sketch,
            subtractive,
            ..
        } = &mut cut
        {
            *sketch = sketch_id;
            *subtractive = false;
        }
        doc.add_feature_in_body(cut, "Helix".into(), Some(body))
            .unwrap();
        let plan = body_build_ops(&doc, body).unwrap();
        assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Fuse);
    }

    #[test]
    fn a_pipe_hands_its_orientation_corners_and_sections_to_the_kernel() {
        let (mut doc, body, profile) = doc_with_body_sketch();
        let spine = doc
            .add_feature_in_body(rect_sketch(), "spine".into(), Some(body))
            .unwrap();
        let guide = doc
            .add_feature_in_body(rect_sketch(), "guide".into(), Some(body))
            .unwrap();
        let section = doc
            .add_feature_in_body(rect_sketch(), "section".into(), Some(body))
            .unwrap();
        let pipe = |orientation: PipeOrientation| DesignFeature::Pipe {
            path_borrowed: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            profile,
            spine,
            orientation,
            corner: PipeCorner::Round,
            sections: vec![section],
            subtractive: false,
        };
        let op_of = |feature: DesignFeature| {
            let mut doc = doc.clone();
            doc.add_feature_in_body(feature, "Pipe".into(), Some(body))
                .unwrap();
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
