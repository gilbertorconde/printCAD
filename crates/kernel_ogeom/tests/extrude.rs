//! Sketch-profile solid-op (pad/pocket/revolve) tests for the kernel.
//!
//! Every test builds simple analytic profiles (rectangles, circles, arcs) and
//! checks the resulting solid via its render-mesh bounds, triangle counts,
//! and BRep snapshot round-trips.

use kernel_api::{
    BooleanOp, ExtrudeTermination, Profile, ProfilePlane, ProfileSegment, ProfileWire, SolidOp,
    SweepKind, TessellationSettings, TriMesh,
};
use kernel_ogeom::OgeomKernel;

fn new_kernel() -> OgeomKernel {
    use kernel_api::Kernel;
    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize kernel");
    kernel
}

fn xy_plane() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    }
}

fn rect_wire(x0: f64, y0: f64, x1: f64, y1: f64) -> ProfileWire {
    ProfileWire {
        segments: vec![
            ProfileSegment::Line {
                start: [x0, y0],
                end: [x1, y0],
            },
            ProfileSegment::Line {
                start: [x1, y0],
                end: [x1, y1],
            },
            ProfileSegment::Line {
                start: [x1, y1],
                end: [x0, y1],
            },
            ProfileSegment::Line {
                start: [x0, y1],
                end: [x0, y0],
            },
        ],
    }
}

fn circle_wire(cx: f64, cy: f64, radius: f64) -> ProfileWire {
    ProfileWire {
        segments: vec![ProfileSegment::Circle {
            center: [cx, cy],
            radius,
        }],
    }
}

/// Blind pad on a given plane; a negative distance extrudes backwards
/// (mapped to the `reversed` flag of the new sweep contract).
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
        },
        op,
    }
}

fn assert_close(actual: f32, expected: f32, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: expected {expected} +/- {tol}, got {actual}"
    );
}

/// Re-tessellate a solid-chain BRep snapshot. Swept shapes do not expose their
/// face count through `SolidBuildResult`, so probe the all-white colour-table
/// size until the shim accepts it (it requires an exact face-count match).
fn retessellate(kernel: &OgeomKernel, blob: &[u8], detail: &TessellationSettings) -> TriMesh {
    for face_count in 1..=32 {
        let colors = vec![[1.0_f32, 1.0, 1.0]; face_count];
        if let Ok(mesh) = kernel.tessellate_step_brep(blob, &colors, detail) {
            return mesh;
        }
    }
    panic!("could not re-tessellate BRep snapshot with any face count in 1..=32");
}

#[test]
fn pads_rectangle_to_box() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [pad(
        vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        5.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("pad rectangle");

    assert!(!result.mesh.positions.is_empty(), "mesh has no vertices");
    assert!(!result.mesh.indices.is_empty(), "mesh has no triangles");
    assert_eq!(
        result.mesh.indices.len() % 3,
        0,
        "indices must be triangles"
    );
    assert_eq!(
        result.mesh.positions.len(),
        result.mesh.normals.len(),
        "positions and normals must be aligned"
    );
    assert!(!result.brep_blob.is_empty(), "missing BRep snapshot");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "min x");
    assert_close(min[1], 0.0, 1e-3, "min y");
    assert_close(min[2], 0.0, 1e-3, "min z");
    assert_close(max[0] - min[0], 10.0, 1e-3, "x extent");
    assert_close(max[1] - min[1], 20.0, 1e-3, "y extent");
    assert_close(max[2] - min[2], 5.0, 1e-3, "z extent");
}

#[test]
fn pads_circle_to_cylinder() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [pad(
        vec![circle_wire(0.0, 0.0, 5.0)],
        10.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("pad circle");

    assert!(!result.mesh.positions.is_empty());
    let (min, max) = result.bounds_mm.expect("bounds");
    // Tessellated circle chords sit slightly inside the true radius.
    assert_close(max[0] - min[0], 10.0, 0.1, "x extent");
    assert_close(max[1] - min[1], 10.0, 0.1, "y extent");
    assert_close(max[2] - min[2], 10.0, 1e-3, "z extent");
    assert_close(min[2], 0.0, 1e-3, "min z");
}

#[test]
fn rectangle_with_circular_hole_keeps_bounds_and_adds_triangles() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let plain = kernel
        .execute_solid_chain(
            &[pad(
                vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
                5.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("plain pad");
    let holed = kernel
        .execute_solid_chain(
            &[pad(
                vec![rect_wire(0.0, 0.0, 10.0, 20.0), circle_wire(5.0, 10.0, 2.0)],
                5.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("holed pad");

    assert_eq!(holed.mesh.indices.len() % 3, 0);
    assert!(
        holed.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3,
        "holed pad should tessellate to more triangles ({}) than a plain box ({})",
        holed.mesh.indices.len() / 3,
        plain.mesh.indices.len() / 3
    );

    let (pmin, pmax) = plain.bounds_mm.expect("plain bounds");
    let (hmin, hmax) = holed.bounds_mm.expect("holed bounds");
    for axis in 0..3 {
        assert_close(hmin[axis], pmin[axis], 1e-3, "holed min bound");
        assert_close(hmax[axis], pmax[axis], 1e-3, "holed max bound");
    }
}

#[test]
fn cut_pocket_keeps_outer_bounds() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Pad 10x20x5, then carve a centered rectangular pocket half as deep from
    // the sketch plane upwards.
    let ops = [
        pad(
            vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
            5.0,
            BooleanOp::NewSolid,
        ),
        pad(vec![rect_wire(2.0, 5.0, 8.0, 15.0)], 2.5, BooleanOp::Cut),
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("pad + pocket chain");

    assert!(!result.brep_blob.is_empty(), "missing BRep snapshot");
    assert!(!result.mesh.positions.is_empty());
    assert_eq!(result.mesh.indices.len() % 3, 0);
    // The pocket adds interior walls, so more triangles than the 12 of a box.
    assert!(result.mesh.indices.len() / 3 > 12);

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "min x");
    assert_close(min[1], 0.0, 1e-3, "min y");
    assert_close(min[2], 0.0, 1e-3, "min z");
    assert_close(max[0], 10.0, 1e-3, "max x");
    assert_close(max[1], 20.0, 1e-3, "max y");
    assert_close(max[2], 5.0, 1e-3, "max z");
}

#[test]
fn fuse_grows_bounds() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [
        pad(
            vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
            5.0,
            BooleanOp::NewSolid,
        ),
        pad(vec![rect_wire(5.0, 0.0, 15.0, 20.0)], 5.0, BooleanOp::Fuse),
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("pad + fuse chain");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "min x");
    assert_close(max[0], 15.0, 1e-3, "max x (grown by fuse)");
    assert_close(max[1], 20.0, 1e-3, "max y");
    assert_close(max[2], 5.0, 1e-3, "max z");
}

#[test]
fn negative_distance_extrudes_backwards() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [pad(
        vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        -5.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("negative pad");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert!(min[2] < 0.0, "negative pad should reach below the plane");
    assert_close(min[2], -5.0, 1e-3, "min z");
    assert_close(max[2], 0.0, 1e-3, "max z");
}

#[test]
fn invalid_chains_error_instead_of_crashing() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // First op must be NewSolid.
    let err = kernel
        .execute_solid_chain(
            &[pad(
                vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
                5.0,
                BooleanOp::Fuse,
            )],
            &detail,
        )
        .expect_err("Fuse as first op must fail");
    assert!(
        err.to_string().contains("NewSolid"),
        "unexpected error message: {err}"
    );

    // Unclosed wire (three line segments that never loop back) must fail with
    // a message mentioning "closed".
    let open_wire = ProfileWire {
        segments: vec![
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
        ],
    };
    let err = kernel
        .execute_solid_chain(&[pad(vec![open_wire], 5.0, BooleanOp::NewSolid)], &detail)
        .expect_err("open wire must fail");
    assert!(
        err.to_string().contains("closed"),
        "error should mention the wire is not closed: {err}"
    );
}

#[test]
fn arc_profile_pads_successfully() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // 10x10 rectangle whose top side is replaced by an arc bulging to y=13.
    let wire = ProfileWire {
        segments: vec![
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
        ],
    };
    let result = kernel
        .execute_solid_chain(&[pad(vec![wire], 5.0, BooleanOp::NewSolid)], &detail)
        .expect("arc profile pad");

    assert!(!result.mesh.positions.is_empty());
    assert!(!result.brep_blob.is_empty());
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[1], 0.0, 1e-3, "min y");
    assert_close(max[1], 13.0, 0.05, "max y (arc bulge apex)");
    assert_close(max[2] - min[2], 5.0, 1e-3, "z extent");
}

#[test]
fn brep_blob_round_trips_through_step_brep_tessellation() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let result = kernel
        .execute_solid_chain(
            &[pad(
                vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
                5.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("pad rectangle");
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

#[test]
fn offset_plane_places_pad_at_origin_height() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let plane = ProfilePlane {
        origin: [0.0, 0.0, 7.0],
        ..xy_plane()
    };
    let ops = [pad_on_plane(
        plane,
        vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        5.0,
        false,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("pad on offset plane");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[2], 7.0, 1e-3, "min z (plane origin height)");
    assert_close(max[2], 12.0, 1e-3, "max z");
}

#[test]
fn symmetric_extrude_straddles_sketch_plane() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [symmetric_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
        8.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("symmetric pad");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "min x");
    assert_close(max[0], 10.0, 1e-3, "max x");
    assert_close(min[1], 0.0, 1e-3, "min y");
    assert_close(max[1], 5.0, 1e-3, "max y");
    assert_close(min[2], -4.0, 1e-3, "min z (half below sketch plane)");
    assert_close(max[2], 4.0, 1e-3, "max z (half above sketch plane)");

    // A negative symmetric distance is documented to produce exactly the same
    // solid as its positive counterpart.
    let negative = kernel
        .execute_solid_chain(
            &[symmetric_pad(
                vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
                -8.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("negative symmetric pad");
    let (nmin, nmax) = negative.bounds_mm.expect("negative bounds");
    for axis in 0..3 {
        assert_close(nmin[axis], min[axis], 1e-3, "negative symmetric min bound");
        assert_close(nmax[axis], max[axis], 1e-3, "negative symmetric max bound");
    }
}

#[test]
fn full_revolve_of_offset_rectangle_makes_ring() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Rectangle x in [5, 8] revolved a full turn about the sketch v-axis
    // (world Y on the XY plane): a square-section ring of radii 5..8 that
    // spans world x/z in [-8, 8] and keeps y in [0, 4].
    let ops = [revolve(
        vec![rect_wire(5.0, 0.0, 8.0, 4.0)],
        [0.0, 0.0],
        [0.0, 1.0],
        360.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("full revolve");

    assert!(!result.mesh.positions.is_empty());
    assert!(!result.brep_blob.is_empty(), "missing BRep snapshot");
    let (min, max) = result.bounds_mm.expect("bounds");
    // Chords of the tessellated circles sit slightly inside the true radius.
    assert_close(min[0], -8.0, 0.2, "min x");
    assert_close(max[0], 8.0, 0.2, "max x");
    assert_close(min[2], -8.0, 0.2, "min z");
    assert_close(max[2], 8.0, 0.2, "max z");
    assert_close(min[1], 0.0, 1e-3, "min y");
    assert_close(max[1], 4.0, 1e-3, "max y");

    // The BRep snapshot must survive a re-tessellation round-trip.
    let mesh = retessellate(&kernel, &result.brep_blob, &detail);
    assert!(!mesh.positions.is_empty());
    assert!(!mesh.indices.is_empty());
    let (rmin, rmax) = mesh.bounds().expect("round-trip bounds");
    for axis in 0..3 {
        assert_close(rmin[axis], min[axis], 0.2, "round-trip min bound");
        assert_close(rmax[axis], max[axis], 0.2, "round-trip max bound");
    }
}

#[test]
fn partial_revolve_sweeps_half_space_only() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // 180 degrees about the sketch v-axis: the profile sweeps a half-ring, so
    // x still spans [-8, 8] but the world-z extent is only one half-space.
    let ops = [revolve(
        vec![rect_wire(5.0, 0.0, 8.0, 4.0)],
        [0.0, 0.0],
        [0.0, 1.0],
        180.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("half revolve");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], -8.0, 0.2, "min x");
    assert_close(max[0], 8.0, 0.2, "max x");
    assert_close(min[1], 0.0, 1e-3, "min y");
    assert_close(max[1], 4.0, 1e-3, "max y");
    // One z half-space stays empty (which one depends on the sweep sense).
    assert_close(max[2] - min[2], 8.0, 0.2, "z extent (half ring)");
    assert!(
        min[2].abs() <= 0.2 || max[2].abs() <= 0.2,
        "half revolve must leave one z half-space empty, got z in [{}, {}]",
        min[2],
        max[2]
    );
}

#[test]
fn groove_cut_by_revolve_keeps_box_bounds() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Pad a 20x20x6 box centered on the origin, then cut a full-turn ring
    // (radii 6..8, y in [2, 4]) around the sketch v-axis: a groove.
    let plain = kernel
        .execute_solid_chain(
            &[pad(
                vec![rect_wire(-10.0, -10.0, 10.0, 10.0)],
                6.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("plain box");
    let grooved = kernel
        .execute_solid_chain(
            &[
                pad(
                    vec![rect_wire(-10.0, -10.0, 10.0, 10.0)],
                    6.0,
                    BooleanOp::NewSolid,
                ),
                revolve(
                    vec![rect_wire(6.0, 2.0, 8.0, 4.0)],
                    [0.0, 0.0],
                    [0.0, 1.0],
                    360.0,
                    BooleanOp::Cut,
                ),
            ],
            &detail,
        )
        .expect("box + revolve groove");

    assert!(!grooved.brep_blob.is_empty(), "missing BRep snapshot");
    assert!(!grooved.mesh.positions.is_empty());
    assert_eq!(grooved.mesh.indices.len() % 3, 0);
    // The groove adds curved interior walls: more triangles than the box.
    assert!(
        grooved.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3,
        "groove should add triangles ({} vs {})",
        grooved.mesh.indices.len() / 3,
        plain.mesh.indices.len() / 3
    );

    // A cut can only keep or shrink the bounds; here the groove is interior,
    // so they stay exactly the box bounds.
    let (pmin, pmax) = plain.bounds_mm.expect("plain bounds");
    let (gmin, gmax) = grooved.bounds_mm.expect("grooved bounds");
    for axis in 0..3 {
        assert!(
            gmin[axis] >= pmin[axis] - 1e-3,
            "cut must not grow min bound"
        );
        assert!(
            gmax[axis] <= pmax[axis] + 1e-3,
            "cut must not grow max bound"
        );
        assert_close(gmin[axis], pmin[axis], 1e-3, "grooved min bound");
        assert_close(gmax[axis], pmax[axis], 1e-3, "grooved max bound");
    }
}

#[test]
fn revolve_angle_out_of_range_errors() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    for bad_angle in [0.0, -90.0, 360.5, 720.0] {
        let err = kernel
            .execute_solid_chain(
                &[revolve(
                    vec![rect_wire(5.0, 0.0, 8.0, 4.0)],
                    [0.0, 0.0],
                    [0.0, 1.0],
                    bad_angle,
                    BooleanOp::NewSolid,
                )],
                &detail,
            )
            .expect_err("out-of-range revolve angle must fail");
        assert!(
            err.to_string().contains("angle"),
            "error for angle {bad_angle} should mention the angle: {err}"
        );
    }
}

#[test]
fn revolve_zero_axis_direction_errors() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let err = kernel
        .execute_solid_chain(
            &[revolve(
                vec![rect_wire(5.0, 0.0, 8.0, 4.0)],
                [0.0, 0.0],
                [0.0, 0.0],
                180.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect_err("zero axis direction must fail");
    assert!(
        err.to_string().contains("axis"),
        "error should mention the axis: {err}"
    );
}
