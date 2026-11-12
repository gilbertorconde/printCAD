//! The kernel ops of a pad or a pocket: each side's end and the plane it
//! starts on.

use super::sweeps::sketch_line_direction;
use super::{
    datum_direction, dot3, face_pick_plane, face_probe, load_sketch, normalize, sketch_profile,
    target_plane,
};
use core_document::{BuildError, BuildPlan, Document, FeatureId, WorkbenchFeature};
use kernel_api::{BooleanOp, ExtrudeTermination, SolidOp, SweepKind};
use wb_sketch::profile;

use crate::feature::{DesignFeature, ExtrudeDirection, ExtrudeMode, FacePick};

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

/// The kernel ops of a pad or a pocket: its profile (a sketch's, or a face
/// of the solid), each side's end and the way it runs. One op, or two when
/// the second side takes its own taper: each side swept on its own.
pub(super) fn extrude_op(
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
    // A pocket's tool digs into the material whether it cuts or keeps.
    let pocket = matches!(feature, DesignFeature::Pocket { .. });
    let reversed = if pocket && custom.is_none() {
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
        if extras.moves_start() || second.is_some() {
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
        (Some(_), _) if extras.moves_start() => {
            return Err("a start offset or plane takes a sketch's profile, not a face".into());
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
            let mut profile = sketch_profile(document, sketch, feature.regions())?;
            if extras.moves_start() {
                // Along the way the first side runs.
                let way = normalize(custom.unwrap_or(profile.plane.normal))
                    .map_err(|_| "the extrusion direction is zero".to_string())?;
                let to_start = match (&extras.start_face, extras.start_at) {
                    (Some(face), _) => {
                        distance_to_plane(profile.plane.origin, way, face_pick_plane(face))?
                    }
                    (None, Some(start)) => {
                        distance_to_plane(profile.plane.origin, way, start_plane(document, start)?)?
                    }
                    (None, None) => 0.0,
                };
                let s =
                    to_start + f64::from(extras.start_offset) * if reversed { -1.0 } else { 1.0 };
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

/// Refused when the last build found the face `feature` starts on curved:
/// a start is a plane.
pub(super) fn start_face_is_flat(document: &Document, feature: FeatureId) -> Result<(), String> {
    let answer = document
        .probed_references(feature)
        .and_then(|probed| probed.answers.first());
    match answer {
        Some(Ok(kernel_api::ProbeAnswer::Face { surface, .. }))
            if !matches!(surface, kernel_api::FaceSurface::Plane { .. }) =>
        {
            Err("the face it starts on is not flat".into())
        }
        _ => Ok(()),
    }
}

/// The plane an extrusion starts on, as a point and its normal in the
/// body's frame: a datum plane's (or coordinate system's), or a sketch's.
pub(crate) fn start_plane(
    document: &Document,
    start: FeatureId,
) -> Result<([f64; 3], [f64; 3]), String> {
    let data = document
        .feature_values(start)
        .ok_or("the plane the extrusion starts on is gone")?;
    if let Ok(datum) = core_document::DatumFeature::from_json(data) {
        if matches!(
            datum.shape,
            core_document::DatumShape::Line { .. } | core_document::DatumShape::Point
        ) {
            return Err("the extrusion starts on a plane, not a datum line or point".into());
        }
        let frame = datum.frame();
        return Ok((frame.origin.map(f64::from), frame.normal.map(f64::from)));
    }
    let sketch = load_sketch(document, start)
        .map_err(|_| "the extrusion starts on a datum plane or a sketch".to_string())?;
    let plane = profile::plane_of(&sketch.plane);
    Ok((plane.origin, plane.normal))
}

/// How far from `from` along `way` the plane through `point` square to
/// `normal` lies; refused for a plane the way runs along.
pub(crate) fn distance_to_plane(
    from: [f64; 3],
    way: [f64; 3],
    (point, normal): ([f64; 3], [f64; 3]),
) -> Result<f64, String> {
    let across = dot3(way, normal);
    if across.abs() < 1e-6 {
        return Err("the extrusion runs along the plane it should start on".into());
    }
    let gap = [point[0] - from[0], point[1] - from[1], point[2] - from[2]];
    Ok(dot3(gap, normal) / across)
}

/// The ops of a pad or a pocket, added to the plan.
#[allow(clippy::single_match)]
pub(super) fn plan_extrude(
    document: &Document,
    plan: &mut BuildPlan,
    feature: &DesignFeature,
    fail: &impl Fn(String) -> BuildError,
    cut: BooleanOp,
    additive_boolean: BooleanOp,
) -> Result<(), BuildError> {
    match feature {
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
        _ => {}
    }
    Ok(())
}
