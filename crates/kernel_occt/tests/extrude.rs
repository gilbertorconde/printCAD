//! Sketch-profile extrusion (pad/pocket) tests for the OCCT kernel.
//!
//! Every test builds simple analytic profiles (rectangles, circles, arcs) and
//! checks the resulting solid via its render-mesh bounds, triangle counts,
//! and BRep snapshot round-trips.

use kernel_api::{
    BooleanOp, ExtrudeOp, ProfilePlane, ProfileSegment, ProfileWire, TessellationSettings,
};
use kernel_occt::OcctKernel;
use std::sync::{Mutex, MutexGuard};

/// OCCT's modelling machinery relies on process-global state and is not safe
/// across concurrent kernels in one process, so the tests in this binary
/// serialize on this mutex. The production app is safe because all OCCT work
/// funnels through the single kernel-worker thread.
static OCCT_SERIAL: Mutex<()> = Mutex::new(());

fn occt_guard() -> MutexGuard<'static, ()> {
    OCCT_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn new_kernel() -> OcctKernel {
    use kernel_api::Kernel;
    let mut kernel = OcctKernel::new();
    kernel.initialize().expect("initialize OCCT kernel");
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

fn pad(wires: Vec<ProfileWire>, distance: f64, op: BooleanOp) -> ExtrudeOp {
    ExtrudeOp {
        plane: xy_plane(),
        wires,
        distance,
        op,
    }
}

fn assert_close(actual: f32, expected: f32, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: expected {expected} +/- {tol}, got {actual}"
    );
}

#[test]
fn pads_rectangle_to_box() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [pad(
        vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        5.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_extrude_chain(&ops, &detail)
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
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [pad(
        vec![circle_wire(0.0, 0.0, 5.0)],
        10.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_extrude_chain(&ops, &detail)
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
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let plain = kernel
        .execute_extrude_chain(
            &[pad(
                vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
                5.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("plain pad");
    let holed = kernel
        .execute_extrude_chain(
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
    let _serial = occt_guard();
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
        .execute_extrude_chain(&ops, &detail)
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
    let _serial = occt_guard();
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
        .execute_extrude_chain(&ops, &detail)
        .expect("pad + fuse chain");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "min x");
    assert_close(max[0], 15.0, 1e-3, "max x (grown by fuse)");
    assert_close(max[1], 20.0, 1e-3, "max y");
    assert_close(max[2], 5.0, 1e-3, "max z");
}

#[test]
fn negative_distance_extrudes_backwards() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [pad(
        vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        -5.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_extrude_chain(&ops, &detail)
        .expect("negative pad");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert!(min[2] < 0.0, "negative pad should reach below the plane");
    assert_close(min[2], -5.0, 1e-3, "min z");
    assert_close(max[2], 0.0, 1e-3, "max z");
}

#[test]
fn invalid_chains_error_instead_of_crashing() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // First op must be NewSolid.
    let err = kernel
        .execute_extrude_chain(
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
        .execute_extrude_chain(&[pad(vec![open_wire], 5.0, BooleanOp::NewSolid)], &detail)
        .expect_err("open wire must fail");
    assert!(
        err.to_string().contains("closed"),
        "error should mention the wire is not closed: {err}"
    );
}

#[test]
fn arc_profile_pads_successfully() {
    let _serial = occt_guard();
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
        .execute_extrude_chain(&[pad(vec![wire], 5.0, BooleanOp::NewSolid)], &detail)
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
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let result = kernel
        .execute_extrude_chain(
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
        .expect("re-tessellate extrude BRep snapshot");

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
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let plane = ProfilePlane {
        origin: [0.0, 0.0, 7.0],
        ..xy_plane()
    };
    let ops = [ExtrudeOp {
        plane,
        wires: vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
        distance: 5.0,
        op: BooleanOp::NewSolid,
    }];
    let result = kernel
        .execute_extrude_chain(&ops, &detail)
        .expect("pad on offset plane");

    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[2], 7.0, 1e-3, "min z (plane origin height)");
    assert_close(max[2], 12.0, 1e-3, "max z");
}
