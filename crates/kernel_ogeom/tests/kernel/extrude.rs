//! Sketch-profile solid-op (pad/pocket/revolve) tests for the kernel.
//!
//! Every test builds simple analytic profiles (rectangles, circles, arcs) and
//! checks the resulting solid's bounds and volume, its render mesh, and its
//! BRep snapshot round-trips.

use super::common::{
    assert_close, chain, circle_wire, new_kernel, plane_at_z, rect_wire, try_chain, volume,
    xy_plane,
};
use kernel_api::{
    BooleanOp, ExtrudeTermination, Profile, ProfilePlane, ProfileSegment, ProfileWire, SolidOp,
    SweepKind, TessellationSettings, TriMesh,
};
use kernel_ogeom::OgeomKernel;
use std::f64::consts::PI;

/// Blind pad on a given plane; a negative distance extrudes backwards
/// (the sweep's `reversed` flag).
fn pad_on_plane(
    plane: ProfilePlane,
    wires: Vec<ProfileWire>,
    distance: f64,
    symmetric: bool,
    op: BooleanOp,
) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile { plane, wires },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind {
                distance: distance.abs(),
            },
            second_side: None,
            symmetric,
            reversed: distance < 0.0,
            taper_deg: 0.0,
            direction: None,
        },
        op,
    }
}

fn pad(wires: Vec<ProfileWire>, distance: f64, op: BooleanOp) -> SolidOp {
    pad_on_plane(xy_plane(), wires, distance, false, op)
}

fn symmetric_pad(wires: Vec<ProfileWire>, distance: f64, op: BooleanOp) -> SolidOp {
    pad_on_plane(xy_plane(), wires, distance, true, op)
}

/// An extrusion on the XY plane ending as `termination` says, its other
/// side as `second_side` says, widening by `taper_deg`.
fn extrusion(
    wires: Vec<ProfileWire>,
    termination: ExtrudeTermination,
    second_side: Option<ExtrudeTermination>,
    taper_deg: f64,
    op: BooleanOp,
) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires,
        },
        kind: SweepKind::Extrude {
            termination,
            second_side,
            symmetric: false,
            reversed: false,
            taper_deg,
            direction: None,
        },
        op,
    }
}

fn revolve(
    wires: Vec<ProfileWire>,
    axis_origin: [f64; 2],
    axis_dir: [f64; 2],
    angle_deg: f64,
    op: BooleanOp,
) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires,
        },
        kind: SweepKind::Revolve {
            axis_origin,
            axis_dir,
            angle_deg,
            second_angle_deg: None,
            midplane: false,
            reversed: false,
            termination: Default::default(),
        },
        op,
    }
}

/// Re-tessellate a solid-chain BRep snapshot with an all-white colour table,
/// trying growing sizes until one tessellates; a table that does not match
/// the face count only warns.
fn retessellate(kernel: &OgeomKernel, blob: &[u8], detail: &TessellationSettings) -> TriMesh {
    for face_count in 1..=32 {
        let colors = vec![[1.0_f32, 1.0, 1.0]; face_count];
        if let Ok(mesh) = kernel.tessellate_step_brep(blob, &colors, detail) {
            return mesh;
        }
    }
    panic!("could not re-tessellate BRep snapshot with any face count in 1..=32");
}

/// A whole transform of the body, row-major.
fn scaled(x: f64, s: f64) -> [[f64; 4]; 4] {
    [
        [x, 0.0, 0.0, 0.0],
        [0.0, s, 0.0, 0.0],
        [0.0, 0.0, s, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Chains of sweeps, booleans and transforms, each with the box its solid
/// fills and its volume: profiles with holes and apart, every way an
/// extrusion ends, a taper, a loft, a half turn, a pocket, a fuse, a
/// groove, and the whole body mirrored and scaled.
#[test]
fn chains_fill_the_box_and_the_volume_they_describe() {
    use BooleanOp::{Cut, Fuse, NewSolid};
    let rect = || vec![rect_wire(0.0, 0.0, 10.0, 20.0)];
    let square = || vec![rect_wire(0.0, 0.0, 10.0, 10.0)];
    // A 10 × 10 square whose top side bulges to y = 13: a segment of a
    // circle 3 high on a 10 chord.
    let bulging = ProfileWire::new(vec![
        ProfileSegment::Line {
            start: [0.0, 0.0],
            end: [10.0, 0.0],
        },
        ProfileSegment::Line {
            start: [10.0, 0.0],
            end: [10.0, 10.0],
        },
        ProfileSegment::Arc {
            start: [10.0, 10.0],
            mid: [5.0, 13.0],
            end: [0.0, 10.0],
        },
        ProfileSegment::Line {
            start: [0.0, 10.0],
            end: [0.0, 0.0],
        },
    ]);
    let r: f64 = 34.0 / 6.0;
    let segment = r * r * ((r - 3.0) / r).acos() - (r - 3.0) * (6.0 * r - 9.0).sqrt();
    let grow = 10f64.to_radians().tan() * 10.0;
    let far = 10.0 + 2.0 * grow;
    // The ring of radii 6 to 8 the groove turns about the box's Y axis,
    // kept to the box's 0 ≤ z ≤ 6: half the annulus less the disc's
    // segment past z = 6.
    let groove = 14.0 * PI - (64.0 * 0.75f64.acos() - 6.0 * 28f64.sqrt());
    let box_20 = || pad(vec![rect_wire(-10.0, -10.0, 10.0, 10.0)], 6.0, NewSolid);
    let whole = |transform| SolidOp::Transform {
        transforms: vec![transform],
        originals: vec![],
    };
    type Case = (&'static str, Vec<SolidOp>, [f64; 3], [f64; 3], f64);
    let cases: Vec<Case> = vec![
        (
            "a rectangle",
            vec![pad(rect(), 5.0, NewSolid)],
            [0.0, 0.0, 0.0],
            [10.0, 20.0, 5.0],
            1000.0,
        ),
        (
            "a circle",
            vec![pad(vec![circle_wire(0.0, 0.0, 5.0)], 10.0, NewSolid)],
            [-5.0, -5.0, 0.0],
            [5.0, 5.0, 10.0],
            250.0 * PI,
        ),
        (
            "a circle nested in a rectangle, a hole",
            vec![pad(
                vec![rect_wire(0.0, 0.0, 10.0, 20.0), circle_wire(5.0, 10.0, 2.0)],
                5.0,
                NewSolid,
            )],
            [0.0, 0.0, 0.0],
            [10.0, 20.0, 5.0],
            5.0 * (200.0 - 4.0 * PI),
        ),
        (
            "two circles apart, two solids",
            vec![pad(
                vec![circle_wire(0.0, 0.0, 2.0), circle_wire(10.0, 0.0, 2.0)],
                5.0,
                NewSolid,
            )],
            [-2.0, -2.0, 0.0],
            [12.0, 2.0, 5.0],
            40.0 * PI,
        ),
        (
            "an arc's bulge",
            vec![pad(vec![bulging], 5.0, NewSolid)],
            [0.0, 0.0, 0.0],
            [10.0, 13.0, 5.0],
            5.0 * (100.0 + segment),
        ),
        (
            "a negative distance",
            vec![pad(rect(), -5.0, NewSolid)],
            [0.0, 0.0, -5.0],
            [10.0, 20.0, 0.0],
            1000.0,
        ),
        (
            "on a raised plane",
            vec![pad_on_plane(plane_at_z(7.0), rect(), 5.0, false, NewSolid)],
            [0.0, 0.0, 7.0],
            [10.0, 20.0, 12.0],
            1000.0,
        ),
        (
            "symmetric",
            vec![symmetric_pad(
                vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
                8.0,
                NewSolid,
            )],
            [0.0, 0.0, -4.0],
            [10.0, 5.0, 4.0],
            400.0,
        ),
        (
            "symmetric, a negative distance",
            vec![symmetric_pad(
                vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
                -8.0,
                NewSolid,
            )],
            [0.0, 0.0, -4.0],
            [10.0, 5.0, 4.0],
            400.0,
        ),
        (
            "two lengths",
            vec![extrusion(
                square(),
                ExtrudeTermination::Blind { distance: 7.0 },
                Some(ExtrudeTermination::Blind { distance: 3.0 }),
                0.0,
                NewSolid,
            )],
            [0.0, 0.0, -3.0],
            [10.0, 10.0, 7.0],
            1000.0,
        ),
        (
            "up to a plane",
            vec![extrusion(
                square(),
                ExtrudeTermination::UpToPlane {
                    point: [0.0, 0.0, 12.5],
                    normal: [0.0, 0.0, 1.0],
                    offset: 0.0,
                },
                None,
                0.0,
                NewSolid,
            )],
            [0.0, 0.0, 0.0],
            [10.0, 10.0, 12.5],
            1250.0,
        ),
        (
            "to the last face, inside the block",
            vec![
                pad(vec![rect_wire(0.0, 0.0, 20.0, 20.0)], 8.0, NewSolid),
                extrusion(
                    vec![rect_wire(5.0, 5.0, 8.0, 8.0)],
                    ExtrudeTermination::ToLast,
                    None,
                    0.0,
                    Fuse,
                ),
            ],
            [0.0, 0.0, 0.0],
            [20.0, 20.0, 8.0],
            3200.0,
        ),
        (
            "tapered 10° outward",
            vec![extrusion(
                vec![rect_wire(-5.0, -5.0, 5.0, 5.0)],
                ExtrudeTermination::Blind { distance: 10.0 },
                None,
                10.0,
                NewSolid,
            )],
            [-5.0 - grow, -5.0 - grow, 0.0],
            [5.0 + grow, 5.0 + grow, 10.0],
            10.0 / 3.0 * (100.0 + far * far + 10.0 * far),
        ),
        (
            "a ruled loft",
            vec![SolidOp::Loft {
                sections: vec![
                    Profile {
                        plane: xy_plane(),
                        wires: vec![rect_wire(-10.0, -10.0, 10.0, 10.0)],
                    },
                    Profile {
                        plane: plane_at_z(15.0),
                        wires: vec![rect_wire(-4.0, -4.0, 4.0, 4.0)],
                    },
                ],
                ruled: true,
                closed: false,
                op: NewSolid,
            }],
            [-10.0, -10.0, 0.0],
            [10.0, 10.0, 15.0],
            15.0 / 3.0 * (400.0 + 64.0 + 160.0),
        ),
        (
            "half a turn about the sketch's v axis",
            vec![revolve(
                vec![rect_wire(5.0, 0.0, 8.0, 4.0)],
                [0.0, 0.0],
                [0.0, 1.0],
                180.0,
                NewSolid,
            )],
            [-8.0, 0.0, -8.0],
            [8.0, 4.0, 0.0],
            78.0 * PI,
        ),
        (
            "a pocket half as deep",
            vec![
                pad(rect(), 5.0, NewSolid),
                pad(vec![rect_wire(2.0, 5.0, 8.0, 15.0)], 2.5, Cut),
            ],
            [0.0, 0.0, 0.0],
            [10.0, 20.0, 5.0],
            1000.0 - 60.0 * 2.5,
        ),
        (
            "a fuse past the side",
            vec![
                pad(rect(), 5.0, NewSolid),
                pad(vec![rect_wire(5.0, 0.0, 15.0, 20.0)], 5.0, Fuse),
            ],
            [0.0, 0.0, 0.0],
            [15.0, 20.0, 5.0],
            1500.0,
        ),
        (
            "a groove turned into the box",
            vec![
                box_20(),
                revolve(
                    vec![rect_wire(6.0, 2.0, 8.0, 4.0)],
                    [0.0, 0.0],
                    [0.0, 1.0],
                    360.0,
                    Cut,
                ),
            ],
            [-10.0, -10.0, 0.0],
            [10.0, 10.0, 6.0],
            2400.0 - 2.0 * groove,
        ),
        (
            "the whole body mirrored across YZ",
            vec![pad(square(), 5.0, NewSolid), whole(scaled(-1.0, 1.0))],
            [-10.0, 0.0, 0.0],
            [10.0, 10.0, 5.0],
            1000.0,
        ),
        (
            "the whole body scaled twice about the origin",
            vec![pad(square(), 5.0, NewSolid), whole(scaled(2.0, 2.0))],
            [0.0, 0.0, 0.0],
            [20.0, 20.0, 10.0],
            4000.0,
        ),
    ];
    let mut failures = Vec::new();
    for (what, ops, lo, hi, want) in cases {
        let result = match try_chain(&ops) {
            Ok(result) => result,
            Err(e) => {
                failures.push(format!("{what}: {e}"));
                continue;
            }
        };
        let mesh = &result.mesh;
        if mesh.indices.is_empty()
            || mesh.indices.len() % 3 != 0
            || mesh.positions.len() != mesh.normals.len()
        {
            failures.push(format!("{what}: not a mesh of triangles"));
        }
        let (min, max) = result.bounds_mm.expect("bounds");
        let near = |got: [f32; 3], want: [f64; 3]| {
            (0..3).all(|i| (f64::from(got[i]) - want[i]).abs() < 1e-3)
        };
        if !near(min, lo) || !near(max, hi) {
            failures.push(format!("{what}: {min:?}..{max:?}, want {lo:?}..{hi:?}"));
        }
        let got = volume(&result.brep_blob);
        if (got - want).abs() > 1e-4 * want {
            failures.push(format!("{what}: volume {got}, want {want}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A chain the kernel cannot build is refused with why: a first op that is
/// not a new solid, a wire that never closes, a turn of no angle or more
/// than a whole one, and a turn about no axis.
#[test]
fn chains_that_cannot_build_are_refused_with_why() {
    use BooleanOp::NewSolid;
    // Three sides of a square that never loop back.
    let open_wire = ProfileWire::new(vec![
        ProfileSegment::Line {
            start: [0.0, 0.0],
            end: [10.0, 0.0],
        },
        ProfileSegment::Line {
            start: [10.0, 0.0],
            end: [10.0, 10.0],
        },
        ProfileSegment::Line {
            start: [10.0, 10.0],
            end: [0.0, 10.0],
        },
    ]);
    let ring = || vec![rect_wire(5.0, 0.0, 8.0, 4.0)];
    let mut cases = vec![
        (
            "a fuse first".to_string(),
            pad(vec![rect_wire(0.0, 0.0, 10.0, 20.0)], 5.0, BooleanOp::Fuse),
            "NewSolid",
        ),
        (
            "an open wire".into(),
            pad(vec![open_wire], 5.0, NewSolid),
            "closed",
        ),
        (
            "no axis".into(),
            revolve(ring(), [0.0, 0.0], [0.0, 0.0], 180.0, NewSolid),
            "axis",
        ),
    ];
    for angle in [0.0, -90.0, 360.5, 720.0] {
        cases.push((
            format!("a turn of {angle}°"),
            revolve(ring(), [0.0, 0.0], [0.0, 1.0], angle, NewSolid),
            "angle",
        ));
    }
    for (what, op, says) in cases {
        let err = try_chain(&[op]).expect_err(&what);
        assert!(err.to_string().contains(says), "{what}: {err}");
    }
}

#[test]
fn brep_blob_round_trips_through_step_brep_tessellation() {
    let kernel = new_kernel();
    let detail = TessellationSettings::default();

    let result = chain(&[pad(
        vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        5.0,
        BooleanOp::NewSolid,
    )]);
    assert!(!result.brep_blob.is_empty());

    // A rectangular pad is a box: 6 faces, all white.
    let face_colors = vec![[1.0_f32, 1.0, 1.0]; 6];
    let mesh = kernel
        .tessellate_step_brep(&result.brep_blob, &face_colors, &detail)
        .expect("re-tessellate solid-chain BRep snapshot");

    let tri_extrude = result.mesh.indices.len() / 3;
    let tri_roundtrip = mesh.indices.len() / 3;
    assert!(tri_extrude > 0 && tri_roundtrip > 0);
    let rel =
        (tri_extrude as f64 - tri_roundtrip as f64).abs() / (tri_extrude.max(tri_roundtrip) as f64);
    assert!(
        rel <= 0.12,
        "triangle count drift too large: extrude {tri_extrude} vs round-trip {tri_roundtrip}"
    );
}

/// A full turn's snapshot, curved faces and all, tessellates again to the
/// solid it was taken of.
#[test]
fn a_revolved_ring_round_trips_through_its_snapshot() {
    let kernel = new_kernel();
    let detail = TessellationSettings::default();
    let ops = [revolve(
        vec![rect_wire(5.0, 0.0, 8.0, 4.0)],
        [0.0, 0.0],
        [0.0, 1.0],
        360.0,
        BooleanOp::NewSolid,
    )];
    let result = chain(&ops);
    let (min, max) = result.bounds_mm.expect("bounds");
    let mesh = retessellate(&kernel, &result.brep_blob, &detail);
    assert!(!mesh.indices.is_empty());
    let (rmin, rmax) = mesh.bounds().expect("round-trip bounds");
    // Chords of the tessellated circles sit slightly inside the true radius.
    for axis in 0..3 {
        assert_close(rmin[axis], min[axis], 0.2, "round-trip min bound");
        assert_close(rmax[axis], max[axis], 0.2, "round-trip max bound");
    }
}
