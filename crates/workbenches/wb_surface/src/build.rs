//! A surface body's history as the kernel's surface steps: each feature in
//! history order, its sketches as chains in the body's own frame, its picks
//! as probes the kernel finds again.

use core_document::{
    BodyId, BodyPlacement, BuildError, BuildPlan, Document, FeatureId, RebuildJob, WorkbenchFeature,
};
use kernel_api::{CurveSource, EdgeProbe, FaceProbe, ProfilePlane, SolidOp, SurfaceOp};
use wb_sketch::SketchFeature;
use wb_sketch::profile;

use crate::feature::{
    Axis, CurveRef, Direction, EdgePick, FacePick, KIND, PlaneRef, SurfaceFeature,
};

/// The body's surface features in history order, as they build: with
/// every formula's value in.
pub fn surface_features_of_body(
    document: &Document,
    body: BodyId,
) -> Vec<(FeatureId, SurfaceFeature)> {
    let mut features: Vec<(u64, FeatureId, SurfaceFeature)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == KIND && node.body == Some(body))
        .filter_map(|(id, node)| {
            SurfaceFeature::from_json(document.feature_values(*id)?)
                .ok()
                .map(|f| (node.seq, *id, f))
        })
        .collect();
    features.sort_by_key(|(seq, id, _)| (*seq, *id));
    features.into_iter().map(|(_, id, f)| (id, f)).collect()
}

/// Whether the body holds a surface feature: the Surface bench builds it.
pub fn is_surface_body(document: &Document, body: BodyId) -> bool {
    document
        .feature_tree()
        .all_nodes()
        .any(|(_, n)| n.workbench_id.as_str() == KIND && n.body == Some(body))
}

/// Each surface feature of `body` that reads edges of other bodies, with
/// what those bodies are now summed up: their shapes' revisions and where
/// they sit relative to `body`.
fn edge_inputs(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    use std::hash::{Hash, Hasher};
    surface_features_of_body(document, body)
        .into_iter()
        .filter_map(|(id, feature)| {
            let others = feature.edge_bodies();
            if others.is_empty() {
                return None;
            }
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for other in others {
                other.hash(&mut hasher);
                document
                    .imported_geometry(other)
                    .map(|g| g.revision)
                    .hash(&mut hasher);
                for row in into_frame(document, body, other).rows() {
                    for value in row {
                        value.to_bits().hash(&mut hasher);
                    }
                }
            }
            Some((id, hasher.finish()))
        })
        .collect()
}

/// Where `other`'s own frame sits in `body`'s.
fn into_frame(document: &Document, body: BodyId, other: BodyId) -> BodyPlacement {
    document
        .body_placement(body)
        .inverse()
        .after(&document.body_placement(other))
}

/// Whether a surface step of `other` reads edges of `body`: then `body`
/// may not read `other`'s, or each would rebuild the other forever.
fn reads_edges_of(document: &Document, other: BodyId, body: BodyId) -> bool {
    surface_features_of_body(document, other)
        .iter()
        .any(|(_, f)| f.edge_bodies().contains(&body))
}

/// Bodies with a surface feature to build again.
fn pending(document: &Document) -> Vec<BodyId> {
    let mut bodies: Vec<BodyId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == KIND && n.dirty)
        .filter_map(|(_, n)| n.body)
        .collect();
    bodies.sort_by_key(|b| b.0);
    bodies.dedup();
    bodies
}

/// The bodies to build again, each with its plan. The dirty flags of the
/// body's surface features and of the sketches they read are settled
/// first, so a plan that fails does not come back every frame. A step
/// reading another body's edges builds again when that body's shape
/// changes or either body moves.
pub fn rebuild_jobs(document: &mut Document) -> Vec<RebuildJob> {
    let bodies: Vec<BodyId> = document.bodies().iter().map(|b| b.id).collect();
    for body in bodies {
        for (feature, inputs) in edge_inputs(document, body) {
            if document.built_against(feature) != Some(inputs) {
                document.mark_feature_stale(feature);
            }
        }
    }
    pending(document)
        .into_iter()
        .map(|body| {
            let features: Vec<FeatureId> = surface_features_of_body(document, body)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            let inputs: Vec<FeatureId> = features
                .iter()
                .flat_map(|id| document.feature_tree().dependencies(*id))
                .filter(|dep| document.get_feature_meta(*dep).is_some_and(|n| n.dirty))
                .collect();
            for id in features.iter().chain(&inputs) {
                document.clear_feature_dirty(*id);
            }
            for (feature, seen) in edge_inputs(document, body) {
                document.note_built_against(feature, seen);
            }
            RebuildJob {
                body,
                plan: plan_until_failure(document, body),
            }
        })
        .collect()
}

/// The body's history changed shape: build it from its first surface
/// step. A body with none is the registry's to clear
/// (`DocumentService::invalidate_body`).
pub fn invalidate_body(document: &mut Document, body: BodyId) {
    if let Some((first, _)) = surface_features_of_body(document, body).first() {
        document.mark_feature_dirty(*first);
    }
}

pub fn invalidate_all(document: &mut Document) {
    let all: Vec<FeatureId> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == KIND)
        .map(|(id, _)| *id)
        .collect();
    for id in all {
        document.mark_feature_dirty(id);
    }
}

/// The body's surface history as the kernel's steps, up to its tip, the
/// suppressed left out; an error when any step cannot be planned.
pub fn body_plan(document: &Document, body: BodyId) -> Result<BuildPlan, BuildError> {
    let mut plan = plan_until_failure(document, body)?;
    match plan.failed.take() {
        Some(error) => Err(error),
        None => Ok(plan),
    }
}

/// The body's surface history as the kernel's steps, stopping at the first
/// one that cannot be planned: the plan is the history before it, which
/// the body shows, with the error and the steps left after it.
pub fn plan_until_failure(document: &Document, body: BodyId) -> Result<BuildPlan, BuildError> {
    let tip_seq = document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .and_then(|b| b.tip)
        .and_then(|tip| document.get_feature_meta(tip))
        .map(|n| n.seq);
    let mut plan = BuildPlan::default();
    let mut any = false;
    for (id, feature) in surface_features_of_body(document, body) {
        let Some(node) = document.get_feature_meta(id) else {
            continue;
        };
        if tip_seq.is_some_and(|tip| node.seq > tip) || node.suppressed {
            continue;
        }
        any = true;
        if plan.failed.is_some() {
            plan.unbuilt.push(id);
            continue;
        }
        let step = if plan.ops.is_empty() && !feature.constructs() {
            Err("works on the body's surfaces; make one first (an extrusion, a fill…)".into())
        } else {
            op_of(document, body, &feature)
        };
        match step {
            Ok(op) => {
                plan.ops.push(SolidOp::Surface(op));
                plan.op_features.push(id);
            }
            Err(message) => {
                plan.failed = Some(BuildError {
                    feature: Some(id),
                    message,
                })
            }
        }
    }
    if !any {
        return Err(BuildError {
            feature: None,
            message: "the body has no surface to build".into(),
        });
    }
    Ok(plan)
}

/// One feature as the kernel's step.
pub fn op_of(
    document: &Document,
    body: BodyId,
    feature: &SurfaceFeature,
) -> Result<SurfaceOp, String> {
    let curves = |refs: &[CurveRef]| sources(document, body, refs);
    Ok(match feature {
        SurfaceFeature::Extrude {
            curves: refs,
            direction,
            length,
            symmetric,
            reversed,
        } => {
            let mut along = direction_of(document, body, refs, *direction)?;
            if *reversed {
                along = along.map(|c| -c);
            }
            SurfaceOp::Extrude {
                curves: curves(refs)?,
                direction: along,
                length: f64::from(*length),
                symmetric: *symmetric,
            }
        }
        SurfaceFeature::Revolve {
            curves: refs,
            axis,
            angle_deg,
        } => {
            let (origin, axis) = axis_of(document, body, refs, *axis)?;
            SurfaceOp::Revolve {
                curves: curves(refs)?,
                origin,
                axis,
                angle_deg: f64::from(*angle_deg),
            }
        }
        SurfaceFeature::PlanarFill { curves: refs } => SurfaceOp::PlanarFill {
            curves: curves(refs)?,
        },
        SurfaceFeature::Fill {
            boundary,
            continuity,
        } => SurfaceOp::Fill {
            boundary: curves(boundary)?,
            continuity: *continuity,
        },
        SurfaceFeature::Ruled { first, second } => {
            let one = |side: &Option<CurveRef>, which: &str| -> Result<CurveSource, String> {
                let side = side.ok_or_else(|| format!("pick the {which} curve"))?;
                let mut got = sources(document, body, &[side])?;
                match got.len() {
                    1 => Ok(got.remove(0)),
                    n => Err(format!(
                        "the {which} curve must be one chain; its sketch has {n}"
                    )),
                }
            };
            SurfaceOp::Ruled {
                first: one(first, "first")?,
                second: one(second, "second")?,
            }
        }
        SurfaceFeature::Loft {
            sections,
            closed,
            guides,
        } => {
            let mut chains = Vec::with_capacity(sections.len());
            for section in sections {
                let mut got = sources(document, body, std::slice::from_ref(section))?;
                if got.len() != 1 {
                    return Err(format!(
                        "each section must be one chain; one has {}",
                        got.len()
                    ));
                }
                chains.push(got.remove(0));
            }
            if guides.is_empty() {
                SurfaceOp::Loft {
                    sections: chains,
                    closed: *closed,
                }
            } else if *closed {
                return Err("a loft that follows guides does not close; turn Closed off".into());
            } else {
                SurfaceOp::GuidedLoft {
                    sections: chains,
                    guides: curves(guides)?,
                }
            }
        }
        SurfaceFeature::Sweep {
            profile,
            path,
            second_rail,
        } => {
            if second_rail.is_empty() {
                SurfaceOp::Sweep {
                    profile: curves(profile)?,
                    path: curves(path)?,
                    frame: Default::default(),
                }
            } else {
                SurfaceOp::SweepTwoRails {
                    profile: curves(profile)?,
                    first_rail: curves(path)?,
                    second_rail: curves(second_rail)?,
                }
            }
        }
        SurfaceFeature::Offset { faces, distance } => SurfaceOp::Offset {
            faces: faces.iter().map(face_probe).collect(),
            distance: f64::from(*distance),
        },
        SurfaceFeature::Extend {
            edges,
            length,
            continuity,
        } => SurfaceOp::Extend {
            edges: edges.iter().map(edge_probe).collect(),
            length: f64::from(*length),
            continuity: *continuity,
        },
        SurfaceFeature::Blend {
            first,
            second,
            continuity,
        } => SurfaceOp::Blend {
            first: edge_probe(&first.ok_or("pick the first edge")?),
            second: edge_probe(&second.ok_or("pick the second edge")?),
            continuity: *continuity,
        },
        SurfaceFeature::Split {
            faces,
            curves: refs,
        } => SurfaceOp::Split {
            faces: faces.iter().map(face_probe).collect(),
            curves: curves(refs)?,
        },
        SurfaceFeature::Sew { gap } => SurfaceOp::Sew {
            gap: f64::from(*gap),
        },
        SurfaceFeature::Fillet { edges, radius } => SurfaceOp::Fillet {
            edges: edges.iter().map(edge_probe).collect(),
            radius: f64::from(*radius),
        },
        SurfaceFeature::FilletFaces {
            first,
            second,
            radius,
            flip_first,
            flip_second,
        } => SurfaceOp::FilletFaces {
            first: face_probe(&first.ok_or("pick the first face")?),
            second: face_probe(&second.ok_or("pick the second face")?),
            radius: f64::from(*radius),
            flip: [*flip_first, *flip_second],
        },
        SurfaceFeature::Thicken {
            thickness,
            both_sides,
        } => SurfaceOp::Thicken {
            thickness: f64::from(*thickness),
            both_sides: *both_sides,
        },
        SurfaceFeature::Trim {
            plane,
            offset,
            flip,
        } => {
            let (origin, normal) = plane_of(*plane, *offset);
            SurfaceOp::TrimByPlane {
                origin,
                normal: if *flip { normal.map(|c| -c) } else { normal },
            }
        }
        SurfaceFeature::Mirror { plane, offset } => {
            let (origin, normal) = plane_of(*plane, *offset);
            SurfaceOp::Mirror { origin, normal }
        }
    })
}

fn edge_probe(pick: &EdgePick) -> EdgeProbe {
    EdgeProbe {
        point: pick.point.map(f64::from),
        direction: pick.direction.map(f64::from),
        faces: pick.faces,
    }
}

fn face_probe(pick: &FacePick) -> FaceProbe {
    FaceProbe {
        point: pick.point.map(f64::from),
        normal: pick.normal.map(f64::from),
        name: pick.name,
    }
}

/// A sketch as its body holds it, with every formula's value in, solved.
pub fn load_sketch(document: &Document, id: FeatureId) -> Result<SketchFeature, String> {
    let data = document
        .feature_values(id)
        .ok_or("reads a sketch that is gone")?;
    SketchFeature::from_json(data).map_err(|e| format!("reads a sketch it cannot: {e}"))
}

/// Where a sketch of another body sits in `body`'s frame: from the
/// sketch's body to the world, then into `body`.
fn into_body(document: &Document, body: BodyId, sketch: FeatureId) -> BodyPlacement {
    let from = document
        .get_feature_meta(sketch)
        .and_then(|n| n.body)
        .unwrap_or(body);
    if from == body {
        return BodyPlacement::default();
    }
    document
        .body_placement(body)
        .inverse()
        .after(&document.body_placement(from))
}

/// A sketch's plane in `body`'s frame.
fn sketch_plane(document: &Document, body: BodyId, id: FeatureId) -> Result<ProfilePlane, String> {
    let sketch = load_sketch(document, id)?;
    let moved = into_body(document, body, id);
    let p = &sketch.plane;
    use wb_sketch::boundary::{place_direction, place_point};
    Ok(profile::plane_of(&wb_sketch::sketch::SketchPlane {
        origin: place_point(&moved, p.origin),
        normal: place_direction(&moved, p.normal),
        x_axis: place_direction(&moved, p.x_axis),
        y_axis: place_direction(&moved, p.y_axis),
    }))
}

/// The curves as the kernel takes them: each sketch's chains, open or
/// closed, on its plane; each edge as a probe.
pub fn sources(
    document: &Document,
    body: BodyId,
    refs: &[CurveRef],
) -> Result<Vec<CurveSource>, String> {
    if refs.is_empty() {
        return Err("pick the curves it is built from".into());
    }
    let mut out = Vec::new();
    for curve in refs {
        match curve {
            CurveRef::Sketch(id) => {
                let sketch = load_sketch(document, *id)?;
                let plane = sketch_plane(document, body, *id)?;
                let chains = profile::extract_chains(&sketch.sketch).map_err(|e| e.to_string())?;
                out.extend(
                    chains
                        .into_iter()
                        .map(|wire| CurveSource::Sketch { plane, wire }),
                );
            }
            CurveRef::Edge(pick) => out.push(CurveSource::Edge(edge_probe(pick))),
            CurveRef::BodyEdge { body: other, edge } if *other == body => {
                out.push(CurveSource::Edge(edge_probe(edge)));
            }
            CurveRef::BodyEdge { body: other, edge } => {
                if reads_edges_of(document, *other, body) {
                    return Err(
                        "it reads edges of a body that reads this one's; one of the two has \
                         to go"
                            .into(),
                    );
                }
                let brep = document
                    .imported_brep_blob(*other)
                    .ok_or("the body its edge is on has no shape yet")?
                    .to_vec();
                let moved = into_frame(document, body, *other);
                out.push(CurveSource::BodyEdge {
                    brep,
                    edge: edge_probe(edge),
                    transform: (!moved.is_identity()).then(|| Box::new(moved.rows())),
                });
            }
        }
    }
    Ok(out)
}

/// The first sketch's plane among the curves, when there is one.
fn first_plane(document: &Document, body: BodyId, refs: &[CurveRef]) -> Option<ProfilePlane> {
    refs.iter().find_map(|r| match r {
        CurveRef::Sketch(id) => sketch_plane(document, body, *id).ok(),
        CurveRef::Edge(_) | CurveRef::BodyEdge { .. } => None,
    })
}

fn direction_of(
    document: &Document,
    body: BodyId,
    refs: &[CurveRef],
    direction: Direction,
) -> Result<[f64; 3], String> {
    Ok(match direction {
        Direction::SketchNormal => first_plane(document, body, refs)
            .map(|p| p.normal)
            .ok_or("the direction follows a sketch's plane; pick a sketch, or another direction")?,
        Direction::X => [1.0, 0.0, 0.0],
        Direction::Y => [0.0, 1.0, 0.0],
        Direction::Z => [0.0, 0.0, 1.0],
        Direction::Custom(d) => d.map(f64::from),
    })
}

fn axis_of(
    document: &Document,
    body: BodyId,
    refs: &[CurveRef],
    axis: Axis,
) -> Result<([f64; 3], [f64; 3]), String> {
    let plane = || {
        first_plane(document, body, refs)
            .ok_or("the axis is a sketch's; pick a sketch, or another axis")
    };
    Ok(match axis {
        Axis::SketchVertical => {
            let p = plane()?;
            (p.origin, p.y_axis)
        }
        Axis::SketchHorizontal => {
            let p = plane()?;
            (p.origin, p.x_axis)
        }
        Axis::X => ([0.0; 3], [1.0, 0.0, 0.0]),
        Axis::Y => ([0.0; 3], [0.0, 1.0, 0.0]),
        Axis::Z => ([0.0; 3], [0.0, 0.0, 1.0]),
        Axis::Custom { origin, direction } => (origin.map(f64::from), direction.map(f64::from)),
    })
}

/// A plane's point and normal, moved `offset` along the normal.
fn plane_of(plane: PlaneRef, offset: f32) -> ([f64; 3], [f64; 3]) {
    let (origin, normal) = match plane {
        PlaneRef::YZ => ([0.0; 3], [1.0, 0.0, 0.0]),
        PlaneRef::XZ => ([0.0; 3], [0.0, 1.0, 0.0]),
        PlaneRef::XY => ([0.0; 3], [0.0, 0.0, 1.0]),
        PlaneRef::Custom { origin, normal } => (origin.map(f64::from), normal.map(f64::from)),
    };
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let unit = if length > 0.0 {
        normal.map(|c| c / length)
    } else {
        normal
    };
    let shift = f64::from(offset);
    ([0, 1, 2].map(|k| origin[k] + unit[k] * shift), normal)
}
