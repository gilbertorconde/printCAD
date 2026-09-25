//! Translation of a body's Part Design features into kernel solid ops.
//!
//! The host drives the recompute loop through the registry: each frame it
//! collects every bench's [`RebuildJob`]s and hands each plan to the kernel
//! worker. This bench's jobs are the bodies with dirty part features, each
//! with its feature history converted into a [`SolidOp`] chain.

use core_document::{BodyId, Document, FeatureId, RebuildJob, WorkbenchFeature};
pub use core_document::{BuildError, BuildPlan};
use kernel_api::{
    BooleanOp, EdgeSelection, ExtrudeTermination, Profile, ProfileSegment, ProfileWire, SolidOp,
    SweepKind,
};
use wb_sketch::SketchFeature;
use wb_sketch::profile;
use wb_sketch::sketch::{GeometryElement, Sketch};

use crate::feature::{
    ExtrudeMode, FacePick, HelixMode, HoleCut, METRIC_SIZES, PartFeature, PatternAxis, RevolveAxis,
    TransformStep,
};

/// This body's part features in creation order (the build history).
pub fn part_features_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, PartFeature)> {
    let mut features: Vec<(u64, FeatureId, PartFeature)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == "wb.part" && node.body == Some(body))
        .filter_map(|(id, node)| {
            // As it builds: with every formula's current value in.
            PartFeature::from_json(document.feature_values(*id)?)
                .ok()
                .map(|f| (node.seq, *id, f))
        })
        .collect();
    // `seq` is the document's explicit insertion order — the build history.
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

/// The bodies with dirty part features, each with its build plan. The
/// dirty flags of the body's features and of the features they read
/// (their sketches, the originals of a pattern) are settled first: the
/// rebuild is now scheduled, or has failed with an attributed error, and
/// either way the same job must not come back next frame.
pub fn rebuild_jobs(document: &mut Document) -> Vec<RebuildJob> {
    pending_body_rebuilds(document)
        .into_iter()
        .map(|body| {
            let features = part_feature_ids(document, body);
            let inputs: Vec<FeatureId> = features
                .iter()
                .flat_map(|id| document.feature_tree().dependencies(*id))
                .filter(|dep| document.get_feature_meta(*dep).is_some_and(|n| n.dirty))
                .collect();
            for id in features.iter().chain(&inputs) {
                document.clear_feature_dirty(*id);
            }
            RebuildJob {
                body,
                plan: body_build_ops(document, body),
            }
        })
        .collect()
}

/// Remove a feature and settle what depended on it: the sketches it
/// consumed show again, and its body rebuilds from the start or drops the
/// solid its history produced. `false` when nothing was removed.
pub fn delete_feature(document: &mut Document, id: FeatureId) -> bool {
    let body = document.get_feature_meta(id).and_then(|n| n.body);
    let sketches = document
        .get_feature_data(id)
        .and_then(|d| PartFeature::from_json(d).ok())
        .map(|f| f.sketches())
        .unwrap_or_default();
    if document.remove_feature(id).is_err() {
        return false;
    }
    for sketch in sketches {
        document.set_feature_visible(sketch, true);
    }
    if let Some(body) = body {
        invalidate_body(document, body);
    }
    true
}

/// The body's history changed shape: rebuild it from its first feature,
/// or, with no history left, drop the solid the history produced. An
/// imported solid is not the history's to drop.
pub fn invalidate_body(document: &mut Document, body: BodyId) {
    match part_feature_ids(document, body).first() {
        Some(first) => document.mark_feature_dirty(*first),
        None if !document.body_solid_is_imported(body) => document.remove_imported_geometry(body),
        None => {}
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

fn edge_selection(edges: &crate::feature::EdgeSel) -> EdgeSelection {
    match edges {
        crate::feature::EdgeSel::All => EdgeSelection::All,
        crate::feature::EdgeSel::Faces(picks) => EdgeSelection::OfFaces(face_points(picks)),
        crate::feature::EdgeSel::Edges(picks) => EdgeSelection::Near(
            picks
                .iter()
                .map(|p| [p.point[0] as f64, p.point[1] as f64, p.point[2] as f64])
                .collect(),
        ),
    }
}

/// Convert a body's feature history into a kernel solid-op chain. Returns an
/// empty plan when the body has no part features (the caller should clear
/// its solid).
pub fn body_build_ops(document: &Document, body: BodyId) -> Result<BuildPlan, BuildError> {
    let features = part_features_of_body(document, body);
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
            PartFeature::Pad {
                refine: _,
                sketch,
                length,
                reversed,
                symmetric,
                mode,
                length2,
                taper_deg,
                up_to_face,
                up_to_offset,
            } => {
                let profile = sketch_profile(document, *sketch).map_err(&fail)?;
                let (termination, second_side) = extrude_terminations(
                    *mode,
                    *length,
                    *length2,
                    up_to_face.as_ref(),
                    *up_to_offset,
                )
                .map_err(&fail)?;
                plan.ops.push(SolidOp::Sweep {
                    profile,
                    kind: SweepKind::Extrude {
                        termination,
                        second_side,
                        symmetric: *symmetric,
                        reversed: *reversed,
                        taper_deg: *taper_deg as f64,
                        direction: None,
                    },
                    op: additive_boolean,
                });
            }
            PartFeature::Pocket {
                refine: _,
                sketch,
                depth,
                reversed,
                symmetric,
                through_all,
                mode,
                depth2,
                taper_deg,
                up_to_face,
                up_to_offset,
            } => {
                let profile = sketch_profile(document, *sketch).map_err(&fail)?;
                let effective_mode = if *through_all {
                    ExtrudeMode::ThroughAll
                } else {
                    *mode
                };
                let (termination, second_side) = extrude_terminations(
                    effective_mode,
                    *depth,
                    *depth2,
                    up_to_face.as_ref(),
                    *up_to_offset,
                )
                .map_err(&fail)?;
                plan.ops.push(SolidOp::Sweep {
                    profile,
                    kind: SweepKind::Extrude {
                        termination,
                        second_side,
                        symmetric: *symmetric,
                        // A pocket cuts OPPOSITE the sketch normal — a sketch
                        // on a solid's face has its normal pointing out of the
                        // material, so the default digs in.
                        reversed: !*reversed,
                        taper_deg: *taper_deg as f64,
                        direction: None,
                    },
                    op: BooleanOp::Cut,
                });
            }
            PartFeature::Revolution {
                refine: _,
                sketch,
                angle_deg,
                axis,
                reversed,
                midplane,
                second_angle_deg,
            }
            | PartFeature::Groove {
                refine: _,
                sketch,
                angle_deg,
                axis,
                reversed,
                midplane,
                second_angle_deg,
            } => {
                let profile = sketch_profile(document, *sketch).map_err(&fail)?;
                let kind = revolve_kind(*axis, *angle_deg, *reversed, *midplane, *second_angle_deg)
                    .map_err(&fail)?;
                let op = if matches!(feature, PartFeature::Groove { .. }) {
                    BooleanOp::Cut
                } else {
                    additive_boolean
                };
                plan.ops.push(SolidOp::Sweep { profile, kind, op });
            }
            PartFeature::Loft {
                refine: _,
                sections,
                ruled,
                closed,
                subtractive,
            } => {
                if sections.len() < 2 {
                    return Err(fail("a loft needs at least two section sketches".into()));
                }
                let mut profiles = Vec::with_capacity(sections.len());
                for section in sections {
                    profiles.push(sketch_profile(document, *section).map_err(&fail)?);
                }
                plan.ops.push(SolidOp::Loft {
                    sections: profiles,
                    ruled: *ruled,
                    closed: *closed,
                    op: shape_boolean(*subtractive),
                });
            }
            PartFeature::Pipe {
                refine: _,
                profile,
                spine,
                frenet,
                subtractive,
            } => {
                let profile = sketch_profile(document, *profile).map_err(&fail)?;
                let spine = sketch_spine(document, *spine).map_err(&fail)?;
                plan.ops.push(SolidOp::Pipe {
                    profile,
                    spine,
                    frenet: *frenet,
                    op: shape_boolean(*subtractive),
                });
            }
            PartFeature::Helix {
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
            } => {
                let profile = sketch_profile(document, *sketch).map_err(&fail)?;
                let (pitch, height) =
                    helix_extent(*mode, *pitch, *height, *turns).map_err(&fail)?;
                plan.ops.push(SolidOp::Sweep {
                    profile,
                    kind: SweepKind::Helix {
                        axis_origin: axis.origin_2d(),
                        axis_dir: axis.dir_2d(),
                        pitch,
                        height,
                        left_handed: *left_handed,
                        cone_angle_deg: *cone_angle_deg as f64,
                        reversed: *reversed,
                    },
                    op: shape_boolean(*subtractive),
                });
            }
            PartFeature::Primitive {
                refine: _,
                kind,
                placement,
                subtractive,
            } => {
                plan.ops.push(SolidOp::Primitive {
                    kind: *kind,
                    placement: *placement,
                    op: shape_boolean(*subtractive),
                });
            }
            PartFeature::Hole { .. } => {
                let hole_ops = hole_ops(document, &feature).map_err(&fail)?;
                plan.ops.extend(hole_ops);
            }
            PartFeature::Fillet { radius, edges } => {
                if *radius <= 0.0 {
                    return Err(fail("fillet radius must be positive".into()));
                }
                plan.ops.push(SolidOp::Fillet {
                    radius: *radius as f64,
                    edges: edge_selection(edges),
                });
            }
            PartFeature::Chamfer {
                size,
                mode,
                size2,
                angle_deg,
                flip,
                edges,
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
                });
            }
            PartFeature::Draft {
                angle_deg,
                neutral,
                faces,
                reversed,
            } => {
                if faces.is_empty() {
                    return Err(fail("select at least one face to draft".into()));
                }
                let (neutral_point, neutral_normal) = face_pick_plane(neutral);
                let pull = if *reversed {
                    Some([-neutral_normal[0], -neutral_normal[1], -neutral_normal[2]])
                } else {
                    None
                };
                plan.ops.push(SolidOp::Draft {
                    angle_deg: *angle_deg as f64,
                    neutral_point,
                    neutral_normal,
                    pull_dir: pull,
                    faces: face_points(faces),
                });
            }
            PartFeature::Thickness {
                value,
                faces,
                inward,
            } => {
                if faces.is_empty() {
                    return Err(fail("select at least one face to open".into()));
                }
                plan.ops.push(SolidOp::Thickness {
                    value: *value as f64,
                    open_faces: face_points(faces),
                    inward: *inward,
                });
            }
            PartFeature::Mirrored {
                originals,
                plane,
                refine: _,
            } => {
                let originals = original_ops(&feature_ops, originals).map_err(&fail)?;
                let (point, normal) = plane.plane();
                plan.ops.push(SolidOp::Transform {
                    transforms: vec![mat_mirror(point, normal)],
                    originals,
                });
            }
            PartFeature::LinearPattern {
                refine: _,
                originals,
                axis,
                length,
                occurrences,
                spacing_mode,
                reversed,
            } => {
                let originals = original_ops(&feature_ops, originals).map_err(&fail)?;
                let transforms =
                    linear_transforms(axis, *length, *occurrences, *spacing_mode, *reversed)
                        .map_err(&fail)?;
                plan.ops.push(SolidOp::Transform {
                    transforms,
                    originals,
                });
            }
            PartFeature::PolarPattern {
                refine: _,
                originals,
                axis,
                angle_deg,
                occurrences,
                reversed,
            } => {
                let originals = original_ops(&feature_ops, originals).map_err(&fail)?;
                let transforms =
                    polar_transforms(axis, *angle_deg, *occurrences, *reversed).map_err(&fail)?;
                plan.ops.push(SolidOp::Transform {
                    transforms,
                    originals,
                });
            }
            PartFeature::MultiTransform {
                originals,
                steps,
                refine: _,
            } => {
                let originals = original_ops(&feature_ops, originals).map_err(&fail)?;
                let transforms = multi_transforms(steps).map_err(&fail)?;
                plan.ops.push(SolidOp::Transform {
                    transforms,
                    originals,
                });
            }
            PartFeature::Clone { source } => {
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
            PartFeature::BodyBoolean {
                tool_body,
                kind,
                refine: _,
            } => {
                if *tool_body == body {
                    return Err(fail(
                        "a body cannot be its own tool; pick another body".into(),
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
    }

    Ok(plan)
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

fn extrude_terminations(
    mode: ExtrudeMode,
    length: f32,
    length2: f32,
    up_to_face: Option<&FacePick>,
    up_to_offset: f32,
) -> Result<(ExtrudeTermination, Option<ExtrudeTermination>), String> {
    let blind = |value: f32| -> Result<ExtrudeTermination, String> {
        if value <= 0.0 {
            return Err("length must be positive".into());
        }
        Ok(ExtrudeTermination::Blind {
            distance: value as f64,
        })
    };
    match mode {
        ExtrudeMode::Dimension => Ok((blind(length)?, None)),
        ExtrudeMode::TwoLengths => Ok((blind(length)?, Some(blind(length2)?))),
        ExtrudeMode::ThroughAll => Ok((ExtrudeTermination::ThroughAll, None)),
        ExtrudeMode::ToFirst => Ok((ExtrudeTermination::ToFirst, None)),
        ExtrudeMode::ToLast => Ok((ExtrudeTermination::ToLast, None)),
        ExtrudeMode::UpToFace => {
            let pick = up_to_face.ok_or("pick a target face for the up-to-face mode")?;
            let (point, normal) = face_pick_plane(pick);
            Ok((
                ExtrudeTermination::UpToFace {
                    point,
                    normal,
                    offset: up_to_offset as f64,
                },
                None,
            ))
        }
    }
}

/// Build the revolve sweep for a feature. `reversed` flips the axis so the
/// sweep runs the other way around.
fn revolve_kind(
    axis: RevolveAxis,
    angle_deg: f32,
    reversed: bool,
    midplane: bool,
    second_angle_deg: Option<f32>,
) -> Result<SweepKind, String> {
    if angle_deg <= 0.0 || angle_deg > 360.0 {
        return Err(format!(
            "revolution angle must be in (0, 360], got {angle_deg}"
        ));
    }
    let dir = axis.dir_2d();
    if dir[0].abs() < 1e-12 && dir[1].abs() < 1e-12 {
        return Err("revolution axis direction is zero".into());
    }
    Ok(SweepKind::Revolve {
        axis_origin: axis.origin_2d(),
        axis_dir: dir,
        angle_deg: f64::from(angle_deg),
        second_angle_deg: second_angle_deg.map(f64::from),
        midplane,
        reversed,
    })
}

fn helix_extent(
    mode: HelixMode,
    pitch: f32,
    height: f32,
    turns: f32,
) -> Result<(f64, f64), String> {
    let (pitch, height) = match mode {
        HelixMode::PitchHeight => (pitch, height),
        HelixMode::PitchTurns => (pitch, pitch * turns),
        HelixMode::HeightTurns => {
            if turns <= 0.0 {
                return Err("helix turns must be positive".into());
            }
            (height / turns, height)
        }
    };
    if pitch <= 0.0 || height <= 0.0 {
        return Err("helix pitch and height must be positive".into());
    }
    Ok((f64::from(pitch), f64::from(height)))
}

/// A hole's effective drill diameter, from the metric table when the hole is
/// standards-driven.
pub fn hole_diameter(feature: &PartFeature) -> f32 {
    if let PartFeature::Hole {
        diameter,
        metric_index,
        threaded,
        fit,
        ..
    } = feature
    {
        if let Some(index) = metric_index
            && let Some((_, _, tap_drill, clearance)) = METRIC_SIZES.get(*index)
        {
            return if *threaded {
                *tap_drill
            } else {
                clearance[*fit as usize]
            };
        }
        return *diameter;
    }
    0.0
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

/// Translate a hole feature into cut ops: main drill, plus a counterbore or
/// countersink cut per the hole-cut option.
fn hole_ops(document: &Document, feature: &PartFeature) -> Result<Vec<SolidOp>, String> {
    let PartFeature::Hole {
        sketch,
        depth,
        through_all,
        cut,
        reversed,
        metric_index,
        threaded,
        modeled_thread,
        thread_depth,
        ..
    } = feature
    else {
        return Err("not a hole feature".into());
    };
    if let Some(index) = metric_index
        && METRIC_SIZES.get(*index).is_none()
    {
        return Err(format!(
            "there is no standard size number {index}; the sizes run from 0 to {}",
            METRIC_SIZES.len() - 1
        ));
    }
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

    let circles_profile = |radius: f64| Profile {
        plane,
        wires: centers
            .iter()
            .map(|center| ProfileWire {
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

    let main_termination = if *through_all {
        ExtrudeTermination::ThroughAll
    } else {
        if *depth <= 0.0 {
            return Err("hole depth must be positive".into());
        }
        ExtrudeTermination::Blind {
            distance: f64::from(*depth),
        }
    };
    let mut ops = vec![cut_extrude(
        main_termination,
        0.0,
        f64::from(diameter) * 0.5,
    )];

    match cut {
        HoleCut::None => {}
        HoleCut::Counterbore {
            diameter: cb_diameter,
            depth: cb_depth,
        } => {
            if *cb_diameter <= diameter {
                return Err("counterbore diameter must exceed the hole diameter".into());
            }
            if *cb_depth <= 0.0 {
                return Err("counterbore depth must be positive".into());
            }
            ops.push(cut_extrude(
                ExtrudeTermination::Blind {
                    distance: f64::from(*cb_depth),
                },
                0.0,
                f64::from(*cb_diameter) * 0.5,
            ));
        }
        HoleCut::Countersink {
            diameter: cs_diameter,
            angle_deg,
        } => {
            if *cs_diameter <= diameter {
                return Err("countersink diameter must exceed the hole diameter".into());
            }
            if *angle_deg <= 0.0 || *angle_deg >= 180.0 {
                return Err("countersink angle must be in (0, 180) degrees".into());
            }
            // The cone runs from the countersink diameter at the surface down
            // to the hole diameter; its depth follows from the angle.
            let half_angle = f64::from(*angle_deg) * 0.5;
            let cs_depth = (f64::from(*cs_diameter) - f64::from(diameter)) * 0.5
                / half_angle.to_radians().tan();
            ops.push(cut_extrude(
                ExtrudeTermination::Blind { distance: cs_depth },
                -half_angle,
                f64::from(*cs_diameter) * 0.5,
            ));
        }
    }
    if *threaded && *modeled_thread {
        let index = metric_index.ok_or("a modeled thread needs a standard size")?;
        let (_, pitch, ..) = *METRIC_SIZES
            .get(index)
            .ok_or_else(|| format!("there is no standard size number {index}"))?;
        let nominal = crate::feature::metric_nominal(index).ok_or("the size names no diameter")?;
        if *thread_depth <= 0.0 {
            return Err("give the modeled thread a depth".into());
        }
        // Into the material: against the sketch normal unless reversed.
        let into = if *reversed {
            plane.normal
        } else {
            plane.normal.map(|c| -c)
        };
        for center in &centers {
            ops.push(thread_cut(
                &plane,
                *center,
                into,
                f64::from(diameter) * 0.5,
                f64::from(nominal) * 0.5,
                f64::from(pitch),
                f64::from(*thread_depth),
            ));
        }
    }
    Ok(ops)
}

/// The groove of an internal metric thread, cut into a hole's wall: a 60°
/// tooth space from inside the drilled wall (`wall` radius) out to the
/// thread's major radius, flat-topped a pitch's eighth wide there, swept
/// right-handed along a helix at `pitch` from a pitch above the surface to
/// `depth` into the material.
fn thread_cut(
    plane: &kernel_api::ProfilePlane,
    center: [f64; 2],
    into: [f64; 3],
    wall: f64,
    major: f64,
    pitch: f64,
    depth: f64,
) -> SolidOp {
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
    let inner = (wall - 0.1 * pitch).max(0.1 * wall);
    let crest = pitch / 16.0;
    let root = (crest + (major - inner) * 30f64.to_radians().tan()).min(0.45 * pitch);
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
            wires: vec![ProfileWire { segments }],
        },
        kind: SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch,
            height: depth + pitch,
            left_handed: false,
            cone_angle_deg: 0.0,
            reversed: false,
        },
        op: BooleanOp::Cut,
    }
}

fn load_sketch(document: &Document, sketch_id: FeatureId) -> Result<SketchFeature, String> {
    // As its formulas leave it, solved.
    let data = document
        .feature_values(sketch_id)
        .ok_or("references a missing sketch")?;
    SketchFeature::from_json(data).map_err(|e| format!("invalid sketch data: {e}"))
}

fn sketch_profile(document: &Document, sketch_id: FeatureId) -> Result<Profile, String> {
    let sketch_feature = load_sketch(document, sketch_id)?;
    let wires = profile::extract_wires(&sketch_feature.sketch).map_err(|e| e.to_string())?;
    Ok(Profile {
        plane: profile::plane_of(&sketch_feature.plane),
        wires,
    })
}

/// Extract a sketch's geometry as a single connected path (open or closed)
/// for use as a pipe spine.
fn sketch_spine(document: &Document, sketch_id: FeatureId) -> Result<Profile, String> {
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
                // Endpoint-bearing kinds added later (splines) expose their
                // ends as the first/last referenced points.
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
    let odd: Vec<uuid::Uuid> = degree
        .iter()
        .filter(|(_, d)| **d == 1)
        .map(|(id, _)| *id)
        .collect();
    if odd.len() != 2 && !odd.is_empty() {
        return Err("the spine must be one connected chain".into());
    }

    let start = odd.first().copied().unwrap_or(endpoints[0].0);
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
        wires: vec![ProfileWire { segments }],
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

// ---- Pattern transform math (row-major 4x4, last row 0 0 0 1) ----

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

fn linear_transforms(
    axis: &PatternAxis,
    length: f32,
    occurrences: u32,
    spacing_mode: bool,
    reversed: bool,
) -> Result<Vec<Mat4>, String> {
    if occurrences < 2 {
        return Err("a linear pattern needs at least 2 occurrences".into());
    }
    let dir = normalize(axis.dir())?;
    let sign = if reversed { -1.0 } else { 1.0 };
    let spacing = if spacing_mode {
        f64::from(length)
    } else {
        f64::from(length) / f64::from(occurrences - 1)
    };
    if spacing.abs() < 1e-9 {
        return Err("pattern spacing is zero".into());
    }
    Ok((1..occurrences)
        .map(|k| {
            let d = spacing * f64::from(k) * sign;
            mat_translation([dir[0] * d, dir[1] * d, dir[2] * d])
        })
        .collect())
}

fn polar_transforms(
    axis: &PatternAxis,
    angle_deg: f32,
    occurrences: u32,
    reversed: bool,
) -> Result<Vec<Mat4>, String> {
    if occurrences < 2 {
        return Err("a polar pattern needs at least 2 occurrences".into());
    }
    let full_circle = (f64::from(angle_deg) - 360.0).abs() < 1e-6;
    let step = if full_circle {
        // 360° spreads evenly without doubling the original position.
        f64::from(angle_deg) / f64::from(occurrences)
    } else {
        f64::from(angle_deg) / f64::from(occurrences - 1)
    };
    let sign = if reversed { -1.0 } else { 1.0 };
    (1..occurrences)
        .map(|k| mat_rotation(axis.origin(), axis.dir(), step * f64::from(k) * sign))
        .collect()
}

/// Cartesian composition: each step's occurrences (including the identity)
/// apply to every result of the previous steps; the pure identity is dropped
/// because the base solid already contains the original.
fn multi_transforms(steps: &[TransformStep]) -> Result<Vec<Mat4>, String> {
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
            } => linear_transforms(axis, *length, *occurrences, false, false)?,
            TransformStep::Polar {
                axis,
                angle_deg,
                occurrences,
            } => polar_transforms(axis, *angle_deg, *occurrences, false)?,
            TransformStep::Mirror { plane } => {
                let (point, normal) = plane.plane();
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

/// Sketches available in a body (id + display name), for re-attachment.
pub fn sketches_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, String)> {
    let mut sketches: Vec<(u64, FeatureId, String)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body == Some(body))
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, id, _)| (*seq, *id));
    sketches.into_iter().map(|(_, id, n)| (id, n)).collect()
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
    let mut feature = PartFeature::from_json(data).map_err(|e| e.to_string())?;
    let Some(old_sketch) = feature.sketch() else {
        return Err("this feature is not sketch-based".into());
    };
    if old_sketch == new_sketch {
        return Ok(());
    }
    match &mut feature {
        PartFeature::Pad { sketch, .. }
        | PartFeature::Pocket { sketch, .. }
        | PartFeature::Revolution { sketch, .. }
        | PartFeature::Groove { sketch, .. }
        | PartFeature::Helix { sketch, .. }
        | PartFeature::Hole { sketch, .. } => *sketch = new_sketch,
        PartFeature::Pipe { profile, .. } => *profile = new_sketch,
        _ => return Err("this feature is not sketch-based".into()),
    }
    document
        .update_feature_data(feature_id, feature.to_json())
        .map_err(|e| e.to_string())?;
    document.set_feature_dependencies(feature_id, feature.dependencies());
    swap_consumed_sketch(document, feature_id, old_sketch, new_sketch);
    Ok(())
}

/// `feature_id` now consumes `new_sketch` instead of `old_sketch`: the new
/// one hides, the old one shows again when no other part feature consumes
/// it. Each sketch whose visibility changed, with what it was before.
pub fn swap_consumed_sketch(
    document: &mut Document,
    feature_id: FeatureId,
    old_sketch: FeatureId,
    new_sketch: FeatureId,
) -> Vec<(FeatureId, bool)> {
    let mut changed = Vec::new();
    let mut set = |document: &mut Document, id: FeatureId, visible: bool| {
        let was = document.get_feature_meta(id).map(|n| n.visible);
        if was.is_some_and(|was| was != visible) {
            changed.push((id, !visible));
            document.set_feature_visible(id, visible);
        }
    };
    set(document, new_sketch, false);
    let still_consumed = document
        .feature_tree()
        .all_nodes()
        .filter(|(id, n)| n.workbench_id.as_str() == "wb.part" && **id != feature_id)
        .filter_map(|(_, n)| PartFeature::from_json(&n.data).ok())
        .any(|f| f.sketches().contains(&old_sketch));
    if !still_consumed {
        set(document, old_sketch, true);
    }
    changed
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
    use crate::feature::MirrorPlane;
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

    fn pad(sketch: FeatureId, length: f32) -> PartFeature {
        PartFeature::Pad {
            refine: false,
            sketch,
            length,
            reversed: false,
            symmetric: false,
            mode: ExtrudeMode::Dimension,
            length2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
        }
    }

    fn pocket(sketch: FeatureId, depth: f32, reversed: bool, through_all: bool) -> PartFeature {
        PartFeature::Pocket {
            refine: false,
            sketch,
            depth,
            reversed,
            symmetric: false,
            through_all,
            mode: ExtrudeMode::Dimension,
            depth2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
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

        let pattern = PartFeature::LinearPattern {
            refine: false,
            originals: vec![pad_id],
            axis: PatternAxis::X,
            length: 30.0,
            occurrences: 2,
            spacing_mode: false,
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
            !PartFeature::Fillet {
                radius: 1.0,
                edges: crate::feature::EdgeSel::All
            }
            .can_refine()
        );
    }

    #[test]
    fn picked_edges_reach_the_kernel_as_probe_points() {
        let picks = vec![
            crate::feature::EdgePick {
                point: [1.0, 2.0, 3.0],
                direction: [1.0, 0.0, 0.0],
            },
            crate::feature::EdgePick {
                point: [4.0, 5.0, 6.0],
                direction: [0.0, 1.0, 0.0],
            },
        ];
        match edge_selection(&crate::feature::EdgeSel::Edges(picks)) {
            EdgeSelection::Near(points) => {
                assert_eq!(points, vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
            }
            other => panic!("picked edges map to probe points, not {other:?}"),
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
    fn invalidating_a_body_restarts_its_history_or_drops_a_built_solid() {
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
                face_colors_path: None,
                health: None,
            },
        );
        invalidate_body(&mut doc, body);
        assert!(doc.imported_geometry(body).is_none(), "a built solid goes");

        let (mut doc, body, _) = doc_with_body_sketch();
        import_into(&mut doc, body);
        invalidate_body(&mut doc, body);
        assert!(doc.imported_geometry(body).is_some(), "an import stays");
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
            PartFeature::Revolution {
                refine: false,
                sketch: sketch_id,
                angle_deg: 270.0,
                axis: RevolveAxis::SketchY,
                reversed: false,
                midplane: false,
                second_angle_deg: None,
            },
            "Revolution".into(),
            Some(body),
        )
        .unwrap();
        doc.add_feature_in_body(
            PartFeature::Groove {
                refine: false,
                sketch: sketch_id,
                angle_deg: 90.0,
                axis: RevolveAxis::SketchX,
                reversed: true,
                midplane: false,
                second_angle_deg: None,
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
            PartFeature::Hole {
                refine: false,
                sketch: hole_sketch_id,
                diameter: 3.0,
                depth: 4.0,
                through_all: false,
                cut: HoleCut::Counterbore {
                    diameter: 6.0,
                    depth: 1.5,
                },
                metric_index: None,
                threaded: false,
                modeled_thread: false,
                thread_depth: 0.0,
                fit: crate::feature::HoleFit::Normal,
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
            PartFeature::BodyBoolean {
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
                PartFeature::Hole {
                    refine: false,
                    sketch: hole_sketch_id,
                    diameter: 3.0,
                    depth: 4.0,
                    through_all: false,
                    cut: HoleCut::None,
                    metric_index: Some(999),
                    threaded,
                    modeled_thread,
                    thread_depth: 3.0,
                    fit: crate::feature::HoleFit::Normal,
                    reversed: false,
                },
                "Hole".into(),
                Some(body),
            )
            .unwrap();
            let error = body_build_ops(&doc, body).unwrap_err();
            assert!(
                error.message.contains("no standard size number 999"),
                "{}",
                error.message
            );
        }
    }

    #[test]
    fn metric_hole_diameter_uses_the_table() {
        let feature = PartFeature::Hole {
            refine: false,
            sketch: FeatureId::new(),
            diameter: 99.0,
            depth: 4.0,
            through_all: false,
            cut: HoleCut::None,
            metric_index: Some(5), // M6
            threaded: true,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: crate::feature::HoleFit::Normal,
            reversed: false,
        };
        assert!((hole_diameter(&feature) - 5.0).abs() < 1e-6, "M6 tap drill");
        let clearance = PartFeature::Hole {
            refine: false,
            sketch: FeatureId::new(),
            diameter: 99.0,
            depth: 4.0,
            through_all: false,
            cut: HoleCut::None,
            metric_index: Some(5),
            threaded: false,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: crate::feature::HoleFit::Normal,
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
            PartFeature::LinearPattern {
                refine: false,
                originals: vec![pad_id],
                axis: PatternAxis::X,
                length: 30.0,
                occurrences: 4,
                spacing_mode: false,
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
        let transforms = polar_transforms(&PatternAxis::Z, 360.0, 4, false).unwrap();
        assert_eq!(transforms.len(), 3);
        // First occurrence at 90°: X axis maps to Y.
        let t = &transforms[0];
        assert!((t[0][0]).abs() < 1e-9 && (t[1][0] - 1.0).abs() < 1e-9);
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
        let transforms = multi_transforms(&steps).unwrap();
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
            PartFeature::Mirrored {
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

        let spine = sketch_spine(&doc, spine_id).unwrap();
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
            sketch_spine(&doc, spine_id)
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
                PartFeature::Mirrored {
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
                PartFeature::Pocket {
                    refine: false,
                    sketch: cut_sketch,
                    depth: 2.0,
                    reversed: false,
                    symmetric: false,
                    through_all: false,
                    mode: crate::ExtrudeMode::Dimension,
                    depth2: 0.0,
                    taper_deg: 0.0,
                    up_to_face: None,
                    up_to_offset: 0.0,
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
        assert!(
            !doc.move_feature_in_history(pocket, true),
            "the pocket stays after its sketch"
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
}
