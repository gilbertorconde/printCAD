//! A sketch projected onto a cylinder's side: split along its curves,
//! raised out of it, sunk into it.

use kernel_api::{
    BooleanOp, FaceProbe, Placement, PrimitiveKind, Profile, ProfilePlane, ProfileSegment,
    ProfileWire, ProjectMode, SolidBuildResult, SolidOp, TessellationSettings,
};
use kernel_ogeom::OgeomKernel;
use ogeom::topo::{Model, ShapeType, explore_unique};

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
    OgeomKernel::new().execute_solid_chain(
        &[
            cylinder(),
            SolidOp::Project {
                profile: rectangle(),
                faces: vec![side()],
                mode,
                height,
            },
        ],
        &TessellationSettings::default(),
    )
}

fn volume(result: &SolidBuildResult) -> f64 {
    OgeomKernel::new()
        .physical_properties(&result.brep_blob)
        .unwrap()
        .volume_mm3
        .expect("a solid has a volume")
}

/// The result's face count, and whether the kernel's checker finds it
/// sound.
fn faces_and_soundness(result: &SolidBuildResult) -> (usize, bool) {
    let mut model = Model::new();
    let text = std::str::from_utf8(&result.brep_blob).unwrap();
    let read = ogeom::io::native::read_into(&mut model, text).unwrap();
    let shape = &read.shapes[0];
    let faces = explore_unique(&model, shape, ShapeType::Face)
        .unwrap()
        .len();
    let check = ogeom::algo::check(&model, shape, ogeom::core::Tolerances::millimetres()).unwrap();
    (faces, check.is_usable())
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
fn an_emboss_stands_out_of_a_cylinder() {
    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let result = project(ProjectMode::Emboss, 1.0).unwrap();
    let (_, sound) = faces_and_soundness(&result);
    assert!(sound, "the embossed solid checks sound");
    // A ring sector of radii 10 and 11 over the span, 4 mm tall.
    let added = span() * (11.0f64.powi(2) - RADIUS.powi(2)) / 2.0 * 4.0;
    let gained = volume(&result) - plain;
    assert!((gained - added).abs() < 0.1 * added, "{gained} vs {added}");
    let (_, max) = result.bounds_mm.expect("bounds");
    let (min, _) = result.bounds_mm.unwrap();
    assert!(
        min[1] < -RADIUS as f32 - 0.9,
        "it stands out toward the sketch"
    );
    assert!(max[1] <= RADIUS as f32 + 1e-3, "the far side is untouched");
}

#[test]
fn a_deboss_sinks_into_a_cylinder() {
    let plain = std::f64::consts::PI * RADIUS * RADIUS * HEIGHT;
    let result = project(ProjectMode::Deboss, 1.0).unwrap();
    let (_, sound) = faces_and_soundness(&result);
    assert!(sound, "the debossed solid checks sound");
    let removed = span() * (RADIUS.powi(2) - 9.0f64.powi(2)) / 2.0 * 4.0;
    let lost = plain - volume(&result);
    assert!(
        (lost - removed).abs() < 0.1 * removed,
        "{lost} vs {removed}"
    );
}

#[test]
fn a_sketch_that_misses_the_face_says_so() {
    let mut far = rectangle();
    far.plane.origin = [0.0, -15.0, 100.0];
    let error = OgeomKernel::new()
        .execute_solid_chain(
            &[
                cylinder(),
                SolidOp::Project {
                    profile: far,
                    faces: vec![side()],
                    mode: ProjectMode::Emboss,
                    height: 1.0,
                },
            ],
            &TessellationSettings::default(),
        )
        .unwrap_err();
    assert_eq!(error.op_index, 1);
    assert!(error.message.contains("do not land"), "{}", error.message);
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
    let result = OgeomKernel::new()
        .execute_solid_chain(
            &[
                cylinder(),
                SolidOp::Project {
                    profile: ring,
                    faces: vec![side()],
                    mode: ProjectMode::Emboss,
                    height: 1.0,
                },
            ],
            &TessellationSettings::default(),
        )
        .unwrap();
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
    let result = OgeomKernel::new().execute_solid_chain(
        &[
            cylinder,
            SolidOp::Project {
                profile,
                faces: vec![probe],
                mode,
                height: 1.5,
            },
        ],
        &TessellationSettings::default(),
    );
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
