//! A sketch projected onto a cylinder's side, or wrapped onto a cylinder
//! or a cone: split along its curves, raised out of it, sunk into it.

use kernel_api::{
    BooleanOp, ExtrudeTermination, FaceProbe, Placement, PrimitiveKind, Profile, ProfilePlane,
    ProfileSegment, ProfileWire, ProjectMode, SolidBuildResult, SolidOp, SweepKind,
};
use ogeom::algo::{Containment, classify_on_face, surface_properties};
use ogeom::core::Tolerances;
use ogeom::math::Point;
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::topo::{Model, Shape, ShapeType, explore_unique};

use super::common::{chain, count_shapes, is_sound, read_shape, try_chain};

const RADIUS: f64 = 10.0;
const HEIGHT: f64 = 20.0;

/// A Ø20 × 20 cylinder standing on XY about Z.
fn cylinder() -> SolidOp {
    SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: RADIUS,
            height: HEIGHT,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }
}

/// A 6 × 4 rectangle on a plane square to Y in front of the cylinder,
/// facing away from it, centred at z = 10.
fn rectangle() -> Profile {
    let (u0, u1, v0, v1) = (-3.0, 3.0, 8.0, 12.0);
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    Profile {
        plane: ProfilePlane {
            origin: [0.0, -15.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
            normal: [0.0, -1.0, 0.0],
        },
        wires: vec![ProfileWire::new(vec![
            line([u0, v0], [u1, v0]),
            line([u1, v0], [u1, v1]),
            line([u1, v1], [u0, v1]),
            line([u0, v1], [u0, v0]),
        ])],
    }
}

fn side() -> FaceProbe {
    FaceProbe {
        point: [0.0, -RADIUS, 10.0],
        normal: [0.0, -1.0, 0.0],
        name: 0,
    }
}

fn project(mode: ProjectMode, height: f64) -> Result<SolidBuildResult, kernel_api::ChainError> {
    try_chain(&[
        cylinder(),
        SolidOp::Project {
            profile: rectangle(),
            faces: vec![side()],
            mode,
            height,
        },
    ])
}

fn volume(result: &SolidBuildResult) -> f64 {
    super::common::volume(&result.brep_blob)
}

/// The result's face count, and whether the kernel's checker finds it
/// sound.
fn faces_and_soundness(result: &SolidBuildResult) -> (usize, bool) {
    let faces = count_shapes(&result.brep_blob, ShapeType::Face);
    (faces, is_sound(&result.brep_blob))
}

/// The angle the rectangle's 6 mm width spans on the cylinder seen square
/// to it.
fn span() -> f64 {
    2.0 * (3.0 / RADIUS).asin()
}

#[test]
fn split_lines_divide_the_face_and_keep_the_volume() {
    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let result = project(ProjectMode::Split, 0.0).unwrap();
    let (faces, sound) = faces_and_soundness(&result);
    assert!(sound, "the split solid checks sound");
    assert!(faces > 3, "the side is split: {faces} faces");
    assert!((volume(&result) - plain).abs() < 1e-2 * plain);
}

#[test]
fn a_sketch_that_misses_the_face_says_so() {
    let mut far = rectangle();
    far.plane.origin = [0.0, -15.0, 100.0];
    let error = try_chain(&[
        cylinder(),
        SolidOp::Project {
            profile: far,
            faces: vec![side()],
            mode: ProjectMode::Emboss,
            height: 1.0,
        },
    ])
    .unwrap_err();
    assert_eq!(error.op_index, 1);
    assert!(error.message.contains("do not land"), "{}", error.message);
}

#[test]
fn a_sketch_running_past_the_face_embosses_what_lands() {
    // From z = 8 up past the cylinder's top: the top edge lands nowhere.
    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    let mut tall = rectangle();
    tall.wires = vec![ProfileWire::new(vec![
        line([-3.0, 8.0], [3.0, 8.0]),
        line([3.0, 8.0], [3.0, 30.0]),
        line([3.0, 30.0], [-3.0, 30.0]),
        line([-3.0, 30.0], [-3.0, 8.0]),
    ])];
    let result = try_chain(&[
        cylinder(),
        SolidOp::Project {
            profile: tall,
            faces: vec![side()],
            mode: ProjectMode::Emboss,
            height: 1.0,
        },
    ])
    .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&result);
    assert!(sound, "the embossed solid checks sound");
    let added = span() * (11.0f64.powi(2) - RADIUS.powi(2)) / 2.0 * (HEIGHT - 8.0);
    let gained = volume(&result) - plain;
    assert!((gained - added).abs() < 0.02 * added, "{gained} vs {added}");
}

#[test]
fn a_region_with_a_hole_embosses_around_the_hole() {
    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let mut ring = rectangle();
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    ring.wires.push(ProfileWire::new(vec![
        line([-1.0, 9.0], [1.0, 9.0]),
        line([1.0, 9.0], [1.0, 11.0]),
        line([1.0, 11.0], [-1.0, 11.0]),
        line([-1.0, 11.0], [-1.0, 9.0]),
    ]));
    let result = chain(&[
        cylinder(),
        SolidOp::Project {
            profile: ring,
            faces: vec![side()],
            mode: ProjectMode::Emboss,
            height: 1.0,
        },
    ]);
    let (_, sound) = faces_and_soundness(&result);
    assert!(sound, "the embossed ring checks sound");
    let sector = |half_width: f64, tall: f64| {
        2.0 * (half_width / RADIUS).asin() * (11.0f64.powi(2) - RADIUS.powi(2)) / 2.0 * tall
    };
    let added = sector(3.0, 4.0) - sector(1.0, 2.0);
    let gained = volume(&result) - plain;
    assert!((gained - added).abs() < 0.1 * added, "{gained} vs {added}");
}

/// A Ø30 × 30 cylinder with a 10 × 12 rectangle sketched 10 mm in front of
/// the side facing `(dx, dy)`, square to it and facing away, projected onto
/// that side 1.5 mm high; the plain cylinder's volume and the result.
fn onto_the_side_facing(
    dx: f64,
    dy: f64,
    mode: ProjectMode,
) -> (f64, Result<SolidBuildResult, kernel_api::ChainError>) {
    let (radius, height) = (15.0, 30.0);
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    let (u0, u1, v0, v1) = (-5.0, 5.0, -6.0, 6.0);
    let profile = Profile {
        plane: ProfilePlane {
            origin: [25.0 * dx, 25.0 * dy, 15.0],
            x_axis: [-dy, dx, 0.0],
            y_axis: [0.0, 0.0, 1.0],
            normal: [dx, dy, 0.0],
        },
        wires: vec![ProfileWire::new(vec![
            line([u0, v0], [u1, v0]),
            line([u1, v0], [u1, v1]),
            line([u1, v1], [u0, v1]),
            line([u0, v1], [u0, v0]),
        ])],
    };
    let probe = FaceProbe {
        point: [radius * dx, radius * dy, 15.0],
        normal: [dx, dy, 0.0],
        name: 0,
    };
    let cylinder = SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    };
    let result = try_chain(&[
        cylinder,
        SolidOp::Project {
            profile,
            faces: vec![probe],
            mode,
            height: 1.5,
        },
    ]);
    (std::f64::consts::PI * radius * radius * height, result)
}

/// The volume `onto_the_side_facing` adds (an emboss) or takes away (a
/// deboss): a ring sector over the rectangle's span, 12 mm tall.
fn rectangle_slab(mode: ProjectMode) -> f64 {
    let span = 2.0 * (5.0f64 / 15.0).asin();
    let (inner, outer): (f64, f64) = match mode {
        ProjectMode::Emboss => (15.0, 16.5),
        _ => (13.5, 15.0),
    };
    span * (outer.powi(2) - inner.powi(2)) / 2.0 * 12.0
}

#[test]
fn a_projection_marks_only_the_side_facing_the_sketch() {
    // Facing -X the side's far half carries the rectangle too, carried
    // back along the plane's normal, and must stay untouched.
    for (dx, dy) in [(-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        for (mode, sign) in [(ProjectMode::Emboss, 1.0), (ProjectMode::Deboss, -1.0)] {
            let (plain, result) = onto_the_side_facing(dx, dy, mode);
            let result = result.unwrap_or_else(|e| panic!("({dx}, {dy}) {mode:?}: {}", e.message));
            let (_, sound) = faces_and_soundness(&result);
            assert!(sound, "({dx}, {dy}) {mode:?} checks sound");
            let moved = volume(&result) - plain;
            let expected = sign * rectangle_slab(mode);
            assert!(
                (moved - expected).abs() < 0.02 * expected.abs(),
                "({dx}, {dy}) {mode:?}: {moved} vs {expected}"
            );
        }
    }
}

#[test]
#[ignore = "kernel: split_face refuses a closed curve projected across a periodic face's seam, \"a curve to split along runs along the face's boundary\" (ogeom-rs#208)"]
fn a_projection_across_the_cylinders_seam() {
    // The cylinder's seam runs up its +X side, through the rectangle.
    for mode in [ProjectMode::Emboss, ProjectMode::Deboss] {
        let (plain, result) = onto_the_side_facing(1.0, 0.0, mode);
        let result = result.unwrap_or_else(|e| panic!("{mode:?}: {}", e.message));
        let (_, sound) = faces_and_soundness(&result);
        assert!(sound, "{mode:?} checks sound");
        let moved = (volume(&result) - plain).abs();
        let expected = rectangle_slab(mode);
        assert!(
            (moved - expected).abs() < 0.02 * expected,
            "{mode:?}: {moved} vs {expected}"
        );
    }
}

/// A `width` × `tall` rectangle centred at `(0, middle)` on a plane square
/// to Y at `y = -distance`, its X across and its Y up Z, facing away.
fn rectangle_facing_minus_y(width: f64, tall: f64, middle: f64, distance: f64) -> Profile {
    let (u0, u1) = (-width / 2.0, width / 2.0);
    let (v0, v1) = (middle - tall / 2.0, middle + tall / 2.0);
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    Profile {
        plane: ProfilePlane {
            origin: [0.0, -distance, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
            normal: [0.0, -1.0, 0.0],
        },
        wires: vec![ProfileWire::new(vec![
            line([u0, v0], [u1, v0]),
            line([u1, v0], [u1, v1]),
            line([u1, v1], [u0, v1]),
            line([u0, v1], [u0, v0]),
        ])],
    }
}

fn wrap_onto(
    solid: SolidOp,
    profile: Profile,
    faces: Vec<FaceProbe>,
    mode: ProjectMode,
    height: f64,
) -> Result<SolidBuildResult, kernel_api::ChainError> {
    try_chain(&[
        solid,
        SolidOp::Wrap {
            profile,
            faces,
            mode,
            height,
        },
    ])
}

/// The result's solid in a model of its own.
fn read(result: &SolidBuildResult) -> (Model, Shape) {
    read_shape(&result.brep_blob)
}

/// The face of `result` holding `point`: its area, and the angle its drawn
/// points span about Z.
fn face_at(result: &SolidBuildResult, point: [f64; 3]) -> (f64, f64) {
    let (model, shape) = read(result);
    let tol = Tolerances::millimetres();
    let fine = Deflection {
        chord: 1e-3,
        ..Deflection::default()
    };
    let at = Point::new(point[0], point[1], point[2]);
    let face = explore_unique(&model, &shape, ShapeType::Face)
        .unwrap()
        .into_iter()
        .find(|f| {
            classify_on_face(&model, f, at, Deflection::default(), tol)
                .is_ok_and(|c| c == Containment::In)
        })
        .expect("a face holds the point");
    let area = surface_properties(&model, &face, fine, tol).unwrap().mass;
    let drawn = triangulate_face(&model, &face, fine, tol).unwrap();
    let angles: Vec<f64> = drawn.positions.iter().map(|p| p.y.atan2(p.x)).collect();
    let low = angles.iter().copied().fold(f64::INFINITY, f64::min);
    let high = angles.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (area, high - low)
}

#[test]
fn a_wrapped_rectangle_keeps_its_width_round_a_cylinder() {
    // 20 mm across a Ø20 cylinder: wider than a straight projection can
    // reach, two radians round it.
    let (width, tall) = (20.0, 4.0);
    let split = wrap_onto(
        cylinder(),
        rectangle_facing_minus_y(width, tall, 10.0, 15.0),
        vec![side()],
        ProjectMode::Split,
        0.0,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&split);
    assert!(sound, "the wrapped split checks sound");
    let (area, span) = face_at(&split, [0.0, -RADIUS, 10.0]);
    assert!((span - width / RADIUS).abs() < 1e-3, "{span} radians");
    assert!(
        (area - width * tall).abs() < 1e-3 * width * tall,
        "{area} mm²"
    );

    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let thick = 1.0;
    for (mode, sign) in [(ProjectMode::Emboss, 1.0), (ProjectMode::Deboss, -1.0)] {
        let result = wrap_onto(
            cylinder(),
            rectangle_facing_minus_y(width, tall, 10.0, 15.0),
            vec![side()],
            mode,
            thick,
        )
        .unwrap_or_else(|e| panic!("{mode:?}: {}", e.message));
        let (_, sound) = faces_and_soundness(&result);
        assert!(sound, "{mode:?} checks sound");
        // An emboss's ring sector runs from the radius out, a deboss's in.
        let expected = width * tall * thick * (1.0 + sign * thick / (2.0 * RADIUS));
        let moved = sign * (volume(&result) - plain);
        assert!(
            (moved - expected).abs() < 5e-3 * expected,
            "{mode:?}: {moved} vs {expected}"
        );
    }
}

#[test]
fn a_wrapped_circle_lies_on_the_cylinder() {
    let mut profile = rectangle_facing_minus_y(1.0, 1.0, 0.0, 15.0);
    profile.wires = vec![ProfileWire::new(vec![ProfileSegment::Circle {
        center: [2.0, 10.0],
        radius: 4.0,
    }])];
    let result = wrap_onto(cylinder(), profile, vec![side()], ProjectMode::Split, 0.0)
        .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&result);
    assert!(sound, "the wrapped circle checks sound");
    // The middle of the sketch is its circle's centre, which lands
    // straight in front of it.
    let (area, span) = face_at(&result, [2.0, -(RADIUS * RADIUS - 4.0).sqrt(), 10.0]);
    let disc = std::f64::consts::PI * 16.0;
    assert!((area - disc).abs() < 2e-3 * disc, "{area} mm²");
    assert!((span - 8.0 / RADIUS).abs() < 1e-2, "{span} radians");
}

/// A cone standing on XY about Z, Ø24 at its foot and Ø12 at its top,
/// 20 tall.
fn cone() -> SolidOp {
    SolidOp::Primitive {
        kind: PrimitiveKind::Cone {
            radius1: 12.0,
            radius2: 6.0,
            height: 20.0,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }
}

#[test]
fn a_wrapped_rectangle_keeps_its_area_on_a_cone() {
    let (width, tall) = (16.0, 6.0);
    let side = FaceProbe {
        point: [0.0, -9.0, 10.0],
        normal: [0.0, -1.0, 0.3],
        name: 0,
    };
    let split = wrap_onto(
        cone(),
        rectangle_facing_minus_y(width, tall, 10.0, 20.0),
        vec![side],
        ProjectMode::Split,
        0.0,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&split);
    assert!(sound, "the wrapped split checks sound");
    // A cone unrolls without stretching, so the wrapped region covers as
    // much as the rectangle does, and its middle arc, 9 mm from the axis,
    // runs the rectangle's width.
    let (area, span) = face_at(&split, [0.0, -9.0, 10.0]);
    assert!(
        (area - width * tall).abs() < 2e-3 * width * tall,
        "{area} mm²"
    );
    assert!(span > width / 9.0 * 0.99, "{span} radians");

    let plain = chain(&[cone()]);
    let embossed = wrap_onto(
        cone(),
        rectangle_facing_minus_y(width, tall, 10.0, 20.0),
        vec![side],
        ProjectMode::Emboss,
        1.0,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&embossed);
    assert!(sound, "the wrapped emboss checks sound");
    let gained = volume(&embossed) - volume(&plain);
    let expected = width * tall;
    assert!(
        (gained - expected).abs() < 0.06 * expected,
        "{gained} vs {expected}"
    );
}

#[test]
#[ignore = "kernel: split_face refuses a closed curve projected across a periodic face's seam, \"a curve to split along runs along the face's boundary\" (ogeom-rs#208)"]
fn a_wrap_across_the_cylinders_seam() {
    // The cylinder's seam runs up its +X side, through the rectangle.
    let mut profile = rectangle_facing_minus_y(6.0, 4.0, 10.0, 15.0);
    profile.plane = ProfilePlane {
        origin: [15.0, 0.0, 0.0],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    };
    let face = FaceProbe {
        point: [RADIUS, 0.0, 10.0],
        normal: [1.0, 0.0, 0.0],
        name: 0,
    };
    let result = wrap_onto(cylinder(), profile, vec![face], ProjectMode::Emboss, 1.0)
        .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&result);
    assert!(sound, "the wrap checks sound");
    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let expected = 6.0 * 4.0 * (1.0 + 1.0 / (2.0 * RADIUS));
    let gained = volume(&result) - plain;
    assert!(
        (gained - expected).abs() < 5e-3 * expected,
        "{gained} vs {expected}"
    );
}

#[test]
fn wrapping_refuses_a_flat_face() {
    let error = wrap_onto(
        SolidOp::Primitive {
            kind: PrimitiveKind::Box {
                length: 20.0,
                width: 20.0,
                height: 20.0,
            },
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        },
        rectangle_facing_minus_y(6.0, 4.0, 10.0, 15.0),
        vec![FaceProbe {
            point: [10.0, 0.0, 10.0],
            normal: [0.0, -1.0, 0.0],
            name: 0,
        }],
        ProjectMode::Emboss,
        1.0,
    )
    .unwrap_err();
    assert!(
        error.message.contains("cylinder or a cone"),
        "{}",
        error.message
    );
}

/// A Ø20 × 20 cylinder padded from two half circles, so its side is two
/// faces meeting up its ±X sides.
fn halved_cylinder() -> SolidOp {
    let arc =
        |start: [f64; 2], mid: [f64; 2], end: [f64; 2]| ProfileSegment::Arc { start, mid, end };
    SolidOp::Sweep {
        profile: Profile {
            plane: ProfilePlane::default(),
            wires: vec![ProfileWire::new(vec![
                arc([RADIUS, 0.0], [0.0, RADIUS], [-RADIUS, 0.0]),
                arc([-RADIUS, 0.0], [0.0, -RADIUS], [RADIUS, 0.0]),
            ])],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: HEIGHT },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    }
}

/// A `width` × 4 rectangle in front of the halved cylinder's -X side,
/// across the edge its two side faces share there.
fn across_the_halves(width: f64) -> Profile {
    let mut profile = rectangle_facing_minus_y(width, 4.0, 10.0, 15.0);
    profile.plane = ProfilePlane {
        origin: [-15.0, 0.0, 0.0],
        x_axis: [0.0, -1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
    };
    profile
}

fn halves() -> Vec<FaceProbe> {
    let probe = |y: f64| FaceProbe {
        point: [-(RADIUS * RADIUS - y * y).sqrt(), y, 10.0],
        normal: [-1.0, 0.0, 0.0],
        name: 0,
    };
    vec![probe(5.0), probe(-5.0)]
}

/// The halved cylinder's volume.
fn halved_volume() -> f64 {
    let (faces, _) = faces_and_soundness(&chain(&[halved_cylinder()]));
    assert_eq!(faces, 4, "the side is two faces");
    std::f64::consts::PI * RADIUS * RADIUS * HEIGHT
}

#[test]
#[ignore = "kernel: split_face refuses a closed curve that enters a face across the low end of its periodic chart's angle window (a half cylinder's side, where the other half meets it): \"a curve to split along does not cross the face\" (ogeom-rs#229)"]
fn a_wrapped_sketch_spans_two_faces() {
    let plain = halved_volume();
    let width: f64 = 6.0;
    let wrapped = wrap_onto(
        halved_cylinder(),
        across_the_halves(width),
        halves(),
        ProjectMode::Emboss,
        1.0,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&wrapped);
    assert!(sound, "the wrap checks sound");
    let expected = width * 4.0 * (1.0 + 1.0 / (2.0 * RADIUS));
    let gained = volume(&wrapped) - plain;
    assert!(
        (gained - expected).abs() < 5e-3 * expected,
        "{gained} vs {expected}"
    );
}

#[test]
#[ignore = "kernel: split_face refuses a closed curve that enters a face across the low end of its periodic chart's angle window (a half cylinder's side, where the other half meets it): \"a curve to split along does not cross the face\" (ogeom-rs#229)"]
fn a_projected_sketch_spans_two_faces() {
    let plain = halved_volume();
    let width: f64 = 6.0;
    let projected = try_chain(&[
        halved_cylinder(),
        SolidOp::Project {
            profile: across_the_halves(width),
            faces: halves(),
            mode: ProjectMode::Emboss,
            height: 1.0,
        },
    ])
    .unwrap_or_else(|e| panic!("{}", e.message));
    let (_, sound) = faces_and_soundness(&projected);
    assert!(sound, "the projection checks sound");
    let span = 2.0 * (width / 2.0 / RADIUS).asin();
    let expected = span * (11.0f64.powi(2) - RADIUS.powi(2)) / 2.0 * 4.0;
    let gained = volume(&projected) - plain;
    assert!(
        (gained - expected).abs() < 0.01 * expected,
        "{gained} vs {expected}"
    );
}
