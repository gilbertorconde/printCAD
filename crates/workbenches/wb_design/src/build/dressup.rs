//! The dress-ups and face edits: fillets, chamfers, drafts, offsets, moved,
//! deleted and opened faces, scaling, and sketches projected onto faces.

use super::{
    datum_direction, face_pick_plane, face_probe, mat_mul, mat_rotation, mat_translation,
    sketch_profile, target_plane,
};
use core_document::{BuildError, BuildPlan, Document, FeatureId, WorkbenchFeature};
use kernel_api::{EdgeSelection, SolidOp};

use crate::feature::{DesignFeature, FacePick};

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
pub(super) fn edge_selection(edges: &crate::feature::EdgeSel) -> Result<EdgeSelection, String> {
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

/// A picked edge as the kernel finds it again.
/// The op a fillet or chamfer builds, its size checked; `None` for any
/// other feature.
pub(crate) fn edge_op(part: &DesignFeature) -> Result<Option<SolidOp>, String> {
    Ok(Some(match part {
        DesignFeature::Fillet {
            radius,
            edges,
            follow_tangent,
        } => {
            if *radius <= 0.0 {
                return Err("fillet radius must be positive".into());
            }
            match edges {
                crate::feature::EdgeSel::BetweenFaces(picks) => match picks.as_slice() {
                    [a, b] => SolidOp::BlendFaces {
                        radius: *radius as f64,
                        faces: [face_probe(a), face_probe(b)],
                    },
                    _ => {
                        return Err(format!(
                            "a fillet between faces takes two faces, it has {}",
                            picks.len()
                        ));
                    }
                },
                edges => SolidOp::Fillet {
                    radius: *radius as f64,
                    edges: edge_selection(edges)?,
                    follow_tangent: *follow_tangent,
                },
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
                return Err("chamfer size must be positive".into());
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
            SolidOp::Chamfer {
                spec,
                flip: *flip,
                edges: edge_selection(edges)?,
                follow_tangent: *follow_tangent,
            }
        }
        _ => return Ok(None),
    }))
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

/// The ops of a dress-up or a face edit, added to the plan.
pub(super) fn plan_dressup(
    document: &Document,
    plan: &mut BuildPlan,
    feature: &DesignFeature,
    fail: &impl Fn(String) -> BuildError,
) -> Result<(), BuildError> {
    match feature {
        DesignFeature::Fillet { .. } | DesignFeature::Chamfer { .. } => {
            plan.ops.extend(edge_op(feature).map_err(fail)?);
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
        }
        | DesignFeature::Text {
            sketch,
            faces,
            mode,
            height,
            wrap,
            ..
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
        _ => {}
    }
    Ok(())
}
