//! The plan tests of pads and pockets: their ends, directions, sides and
//! the planes they start on.

use super::*;

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

/// The one extrude op a pad or pocket, `edit`ed from its defaults,
/// builds on a pad of the rectangle.
fn extrude_of(
    feature: DesignFeature,
    edit: impl FnOnce(&mut DesignFeature),
) -> Result<SolidOp, String> {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch_id, 5.0), "Base");
    let mut feature = feature;
    if let DesignFeature::Pad { sketch, .. } | DesignFeature::Pocket { sketch, .. } = &mut feature {
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
            if let DesignFeature::Pad { direction, .. } | DesignFeature::Pocket { direction, .. } =
                f
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
            if let DesignFeature::Pad { direction, .. } | DesignFeature::Pocket { direction, .. } =
                f
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
            if let DesignFeature::Pad { direction, .. } | DesignFeature::Pocket { direction, .. } =
                f
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
