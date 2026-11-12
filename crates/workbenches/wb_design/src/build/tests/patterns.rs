//! The plan tests of mirrors, patterns and multi transforms.

use super::*;
use crate::build::patterns::{
    circular_transforms, multi_transforms, pattern_axis, point_transforms, polar_transforms,
};

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

#[test]
fn a_mirror_takes_a_datum_or_a_sketchs_plane_and_axes() {
    use crate::feature::{MirrorPlane, SketchAxis};
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
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
