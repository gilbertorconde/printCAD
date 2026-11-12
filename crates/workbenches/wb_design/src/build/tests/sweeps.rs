//! The plan tests of revolutions, grooves, helices and pipes, and their
//! axes and spines.

use super::*;
use crate::build::sweeps::{sketch_spine, turn_plane};
use crate::feature::{PipeCorner, RevolveAxis, RevolveMode};
use kernel_api::RevolveTermination;

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

/// The path `lines` draw between `points`, in that order, in a body of
/// its own.
fn path(points: &[(f64, f64)], lines: &[(usize, usize)]) -> (Document, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let mut sketch = Sketch::new("path");
    let ids: Vec<_> = points
        .iter()
        .map(|&(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))))
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
                attachment: core_document::DatumAttachment::BasePlane(core_document::BasePlane::YZ),
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
    let line_axis = axis_in_sketch(&doc, &feature, &RevolveAxis::SketchLine(line), false).unwrap();
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

    let Ok(SolidOp::Pipe { frame, .. }) = op_of(pipe(PipeOrientation::Auxiliary { path: guide }))
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
