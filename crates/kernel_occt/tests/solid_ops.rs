//! Tests for the extended solid-op set: termination modes, taper, loft,
//! pipe, helix, primitives, dress-ups, patterns, and body booleans.

use kernel_api::{
    BoolKind, BooleanOp, ChamferSpec, EdgeSelection, ExtrudeTermination, Placement, PrimitiveKind,
    Profile, ProfilePlane, ProfileSegment, ProfileWire, SolidOp, SweepKind, TessellationSettings,
};
use kernel_occt::OcctKernel;
use std::sync::{Mutex, MutexGuard};

/// OCCT's modelling machinery relies on process-global state and is not safe
/// across concurrent kernels in one process, so the tests in this binary
/// serialize on this mutex.
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

fn plane_at_z(z: f64) -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, z],
        ..xy_plane()
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

fn blind_pad(wires: Vec<ProfileWire>, distance: f64, op: BooleanOp) -> SolidOp {
    extrude(wires, ExtrudeTermination::Blind { distance }, false, op)
}

fn extrude(
    wires: Vec<ProfileWire>,
    termination: ExtrudeTermination,
    reversed: bool,
    op: BooleanOp,
) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires,
        },
        kind: SweepKind::Extrude {
            termination,
            second_side: None,
            symmetric: false,
            reversed,
            taper_deg: 0.0,
            direction: None,
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

#[test]
fn through_all_pocket_pierces_the_pad() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // A 20x20x10 pad with a through-all circular cut from the top plane
    // downwards (reversed = the cut runs against +z from z=10... use the
    // sketch plane at z=0 cutting forwards through everything).
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        ),
        extrude(
            vec![circle_wire(10.0, 10.0, 2.0)],
            ExtrudeTermination::ThroughAll,
            false,
            BooleanOp::Cut,
        ),
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("pad + through-all cut");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[2], 0.0, 1e-3, "min z");
    assert_close(max[2], 10.0, 1e-3, "max z");
    // The hole pierces both faces: more triangles than a plain box.
    assert!(result.mesh.indices.len() / 3 > 12);
}

#[test]
fn two_lengths_extrude_spans_both_sides() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 7.0 },
            second_side: Some(ExtrudeTermination::Blind { distance: 3.0 }),
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    }];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("two-sided pad");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[2], -3.0, 1e-3, "min z (second side)");
    assert_close(max[2], 7.0, 1e-3, "max z (first side)");
}

#[test]
fn up_to_plane_extrude_stops_on_the_plane() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [extrude(
        vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
        ExtrudeTermination::UpToPlane {
            point: [0.0, 0.0, 12.5],
            normal: [0.0, 0.0, 1.0],
            offset: 0.0,
        },
        false,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("up-to-plane pad");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[2], 0.0, 1e-3, "min z");
    assert_close(max[2], 12.5, 1e-3, "max z (trimmed at target plane)");
}

#[test]
fn to_last_pad_reaches_the_far_face() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Base box z in [0, 8]; a small pad from the sketch plane with ToLast
    // must stop exactly at z=8.
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            8.0,
            BooleanOp::NewSolid,
        ),
        extrude(
            vec![rect_wire(5.0, 5.0, 8.0, 8.0)],
            ExtrudeTermination::ToLast,
            false,
            BooleanOp::Fuse,
        ),
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("to-last pad");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[2], 0.0, 1e-3, "min z");
    assert_close(max[2], 8.0, 1e-3, "max z (stopped at the far face)");
}

#[test]
fn tapered_pad_widens_the_far_end() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![rect_wire(-5.0, -5.0, 5.0, 5.0)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 10.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 10.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    }];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("tapered pad");
    let (min, max) = result.bounds_mm.expect("bounds");
    let grow = (10.0_f32.to_radians().tan()) * 10.0;
    assert_close(max[0], 5.0 + grow, 0.05, "max x widened by the taper");
    assert_close(min[0], -5.0 - grow, 0.05, "min x widened by the taper");
    assert_close(max[2], 10.0, 1e-3, "height unchanged");
}

#[test]
fn disjoint_profile_regions_become_separate_solids() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Two disjoint circles: they must pad into two separate cylinders, not
    // one annulus (the second circle is NOT a hole of the first).
    let ops = [blind_pad(
        vec![circle_wire(0.0, 0.0, 2.0), circle_wire(10.0, 0.0, 2.0)],
        5.0,
        BooleanOp::NewSolid,
    )];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("two disjoint pads");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], -2.0, 0.05, "min x (first cylinder)");
    assert_close(max[0], 12.0, 0.05, "max x (second cylinder)");
    assert_close(max[2], 5.0, 1e-3, "height");
}

#[test]
fn ellipse_and_bspline_profiles_pad() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ellipse = ProfileWire {
        segments: vec![ProfileSegment::Ellipse {
            center: [0.0, 0.0],
            major: [8.0, 0.0],
            ratio: 0.5,
        }],
    };
    let result = kernel
        .execute_solid_chain(
            &[blind_pad(vec![ellipse], 4.0, BooleanOp::NewSolid)],
            &detail,
        )
        .expect("ellipse pad");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[0] - min[0], 16.0, 0.1, "major extent");
    assert_close(max[1] - min[1], 8.0, 0.1, "minor extent");

    let spline = ProfileWire {
        segments: vec![ProfileSegment::BSpline {
            control_points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            periodic: true,
        }],
    };
    let result = kernel
        .execute_solid_chain(
            &[blind_pad(vec![spline], 4.0, BooleanOp::NewSolid)],
            &detail,
        )
        .expect("periodic B-spline pad");
    assert!(!result.mesh.positions.is_empty());
    assert_close(
        result.bounds_mm.unwrap().1[2],
        4.0,
        1e-3,
        "spline pad height",
    );
}

#[test]
fn loft_skins_between_two_rectangles() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ops = [SolidOp::Loft {
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
        op: BooleanOp::NewSolid,
    }];
    let result = kernel.execute_solid_chain(&ops, &detail).expect("loft");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], -10.0, 1e-3, "min x (base section)");
    assert_close(max[2], 15.0, 1e-3, "max z (top section)");
}

#[test]
fn pipe_sweeps_profile_along_l_path() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Circle profile on the XZ-ish plane at the path start, swept along an
    // L-shaped path drawn in the XY plane starting at the origin.
    let profile_plane = ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [0.0, 0.0, 1.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    };
    let spine = Profile {
        plane: xy_plane(),
        wires: vec![ProfileWire {
            segments: vec![
                ProfileSegment::Line {
                    start: [0.0, 0.0],
                    end: [30.0, 0.0],
                },
                ProfileSegment::Line {
                    start: [30.0, 0.0],
                    end: [30.0, 20.0],
                },
            ],
        }],
    };
    let ops = [SolidOp::Pipe {
        profile: Profile {
            plane: profile_plane,
            wires: vec![circle_wire(0.0, 0.0, 2.0)],
        },
        spine,
        frenet: false,
        op: BooleanOp::NewSolid,
    }];
    let result = kernel.execute_solid_chain(&ops, &detail).expect("pipe");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[0], 32.0, 0.3, "max x (leg 1 + radius)");
    assert_close(max[1], 20.0, 0.3, "max y (leg 2 end)");
    assert!(min[2] <= -1.8 && max[2] >= 1.8, "tube cross-section in z");
}

#[test]
fn helix_sweep_builds_a_spring() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Circle at x=10 swept around the sketch Y axis: a spring of coil radius
    // 10, wire radius 1.5, pitch 6, height 24 (4 turns). The helix axis is
    // the sketch v-axis => world Y on the XY plane.
    let ops = [SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![circle_wire(10.0, 0.0, 1.5)],
        },
        kind: SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch: 6.0,
            height: 24.0,
            left_handed: false,
            cone_angle_deg: 0.0,
            reversed: false,
        },
        op: BooleanOp::NewSolid,
    }];
    let result = kernel.execute_solid_chain(&ops, &detail).expect("helix");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[0], 11.5, 0.3, "coil + wire radius in x");
    assert_close(min[0], -11.5, 0.3, "coil + wire radius in -x");
    // The spring spans the full height along the axis (plus wire radius).
    assert!(
        max[1] - min[1] >= 24.0 && max[1] - min[1] <= 27.5,
        "spring height along the axis, got {}",
        max[1] - min[1]
    );
}

#[test]
fn primitives_build_with_expected_bounds() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let cases: Vec<(PrimitiveKind, [f32; 3])> = vec![
        (
            PrimitiveKind::Box {
                length: 10.0,
                width: 20.0,
                height: 5.0,
            },
            [10.0, 20.0, 5.0],
        ),
        (
            PrimitiveKind::Cylinder {
                radius: 5.0,
                height: 12.0,
                angle_deg: 360.0,
            },
            [10.0, 10.0, 12.0],
        ),
        (
            PrimitiveKind::Sphere {
                radius: 6.0,
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
            [12.0, 12.0, 12.0],
        ),
        (
            PrimitiveKind::Cone {
                radius1: 6.0,
                radius2: 2.0,
                height: 9.0,
                angle_deg: 360.0,
            },
            [12.0, 12.0, 9.0],
        ),
        (
            PrimitiveKind::Torus {
                radius1: 10.0,
                radius2: 2.0,
                angle1_deg: -180.0,
                angle2_deg: 180.0,
                angle3_deg: 360.0,
            },
            [24.0, 24.0, 4.0],
        ),
        (
            PrimitiveKind::Ellipsoid {
                radius1: 8.0,
                radius2: 5.0,
                radius3: 3.0,
            },
            [16.0, 10.0, 6.0],
        ),
        (
            PrimitiveKind::Prism {
                sides: 6,
                circumradius: 5.0,
                height: 7.0,
            },
            [10.0, 8.66, 7.0],
        ),
        (
            PrimitiveKind::Wedge {
                xmin: 0.0,
                xmax: 10.0,
                ymin: 0.0,
                ymax: 8.0,
                zmin: 0.0,
                zmax: 10.0,
                x2min: 3.0,
                x2max: 7.0,
                z2min: 3.0,
                z2max: 7.0,
            },
            [10.0, 8.0, 10.0],
        ),
    ];

    for (kind, expected_extent) in cases {
        let label = format!("{kind:?}");
        let ops = [SolidOp::Primitive {
            kind,
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        }];
        let result = kernel
            .execute_solid_chain(&ops, &detail)
            .unwrap_or_else(|e| panic!("primitive {label}: {e}"));
        let (min, max) = result.bounds_mm.expect("bounds");
        for axis in 0..3 {
            assert_close(
                max[axis] - min[axis],
                expected_extent[axis],
                0.25,
                &format!("{label} extent[{axis}]"),
            );
        }
    }
}

#[test]
fn fillet_and_chamfer_modify_all_edges() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let base = || {
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        )
    };
    let plain = kernel
        .execute_solid_chain(&[base()], &detail)
        .expect("plain box");

    let filleted = kernel
        .execute_solid_chain(
            &[
                base(),
                SolidOp::Fillet {
                    radius: 2.0,
                    edges: EdgeSelection::All,
                },
            ],
            &detail,
        )
        .expect("fillet all edges");
    assert!(
        filleted.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3,
        "fillets add curved faces"
    );
    let (fmin, fmax) = filleted.bounds_mm.expect("bounds");
    assert_close(fmax[0] - fmin[0], 20.0, 0.05, "fillet keeps extents");

    let chamfered = kernel
        .execute_solid_chain(
            &[
                base(),
                SolidOp::Chamfer {
                    spec: ChamferSpec::EqualDistance { distance: 1.5 },
                    flip: false,
                    edges: EdgeSelection::All,
                },
            ],
            &detail,
        )
        .expect("chamfer all edges");
    assert!(chamfered.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3);
}

#[test]
fn fillet_of_faces_selection_uses_nearest_face() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Fillet only the edges of the top face (sample point on top center).
    let result = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Fillet {
                    radius: 2.0,
                    edges: EdgeSelection::OfFaces(vec![[10.0, 10.0, 10.0]]),
                },
            ],
            &detail,
        )
        .expect("fillet top face edges");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[2], 10.0, 1e-3, "height preserved");
    assert_close(min[2], 0.0, 1e-3, "bottom untouched");
}

#[test]
fn thickness_hollows_the_box() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let plain = kernel
        .execute_solid_chain(
            &[blind_pad(
                vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                10.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("plain box");
    let hollowed = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Thickness {
                    value: 1.5,
                    open_faces: vec![[10.0, 10.0, 10.0]],
                    inward: true,
                },
            ],
            &detail,
        )
        .expect("hollowed box");
    let (min, max) = hollowed.bounds_mm.expect("bounds");
    assert_close(
        max[0] - min[0],
        20.0,
        1e-3,
        "outer x preserved (inward walls)",
    );
    assert!(
        hollowed.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3,
        "inner walls add triangles"
    );
}

#[test]
fn draft_tilts_side_faces() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Draft one side face about the bottom neutral plane. A positive angle
    // leans the face inward (mold-release style), which leaves the bounds
    // unchanged (the base edge stays put), so lean it OUTWARD: the top edge
    // then extends past x=20 by tan(angle) * height.
    let result = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Draft {
                    angle_deg: -10.0,
                    neutral_point: [0.0, 0.0, 0.0],
                    neutral_normal: [0.0, 0.0, 1.0],
                    pull_dir: None,
                    faces: vec![[20.0, 10.0, 5.0]],
                },
            ],
            &detail,
        )
        .expect("draft side face");
    let (min, max) = result.bounds_mm.expect("bounds");
    let lean = (10.0_f32.to_radians().tan()) * 10.0;
    assert_close(max[0], 20.0 + lean, 0.1, "drafted top edge leans outward");
    assert_close(min[0], 0.0, 1e-3, "opposite face untouched");
}

#[test]
fn linear_pattern_repeats_the_tool_solid() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // A base plate with one boss, boss patterned 3x along +x at 15 mm.
    let translate = |d: f64| -> [[f64; 4]; 4] {
        [
            [1.0, 0.0, 0.0, d],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 50.0, 20.0)],
            3.0,
            BooleanOp::NewSolid,
        ),
        blind_pad(vec![circle_wire(10.0, 10.0, 3.0)], 10.0, BooleanOp::Fuse),
        SolidOp::Transform {
            transforms: vec![translate(15.0), translate(30.0)],
            originals: vec![1],
        },
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("linear pattern");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[2], 10.0, 1e-3, "boss height everywhere");
    assert_close(min[0], 0.0, 1e-3, "plate min x");
    assert_close(max[0], 50.0, 1e-3, "plate max x");
    // Patterned bosses at x=10, 25, 40 all exist: the mesh grows.
    assert!(result.mesh.indices.len() / 3 > 24);
}

#[test]
fn pattern_of_subtractive_tool_repeats_the_cut() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let translate = |d: f64| -> [[f64; 4]; 4] {
        [
            [1.0, 0.0, 0.0, d],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };
    let single_hole = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 50.0, 20.0)],
                    5.0,
                    BooleanOp::NewSolid,
                ),
                extrude(
                    vec![circle_wire(10.0, 10.0, 2.0)],
                    ExtrudeTermination::ThroughAll,
                    false,
                    BooleanOp::Cut,
                ),
            ],
            &detail,
        )
        .expect("single hole");
    let patterned = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 50.0, 20.0)],
                    5.0,
                    BooleanOp::NewSolid,
                ),
                extrude(
                    vec![circle_wire(10.0, 10.0, 2.0)],
                    ExtrudeTermination::ThroughAll,
                    false,
                    BooleanOp::Cut,
                ),
                SolidOp::Transform {
                    transforms: vec![translate(15.0), translate(30.0)],
                    originals: vec![1],
                },
            ],
            &detail,
        )
        .expect("patterned holes");
    assert!(
        patterned.mesh.indices.len() / 3 > single_hole.mesh.indices.len() / 3,
        "two more holes add wall triangles"
    );
    let (min, max) = patterned.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "plate min x");
    assert_close(max[0], 50.0, 1e-3, "plate max x");
}

#[test]
fn mirror_transform_of_whole_body_doubles_the_extent() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Mirror the whole body (no originals) across the YZ plane at x=0.
    let mirror_yz: [[f64; 4]; 4] = [
        [-1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
            5.0,
            BooleanOp::NewSolid,
        ),
        SolidOp::Transform {
            transforms: vec![mirror_yz],
            originals: vec![],
        },
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("mirror whole body");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], -10.0, 1e-3, "mirrored min x");
    assert_close(max[0], 10.0, 1e-3, "original max x");
}

#[test]
fn scaling_transform_uses_general_transform_path() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Fuse a 2x scaled copy of the body about the origin.
    let scale2: [[f64; 4]; 4] = [
        [2.0, 0.0, 0.0, 0.0],
        [0.0, 2.0, 0.0, 0.0],
        [0.0, 0.0, 2.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
            5.0,
            BooleanOp::NewSolid,
        ),
        SolidOp::Transform {
            transforms: vec![scale2],
            originals: vec![],
        },
    ];
    let result = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("scaled copy");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[0], 20.0, 1e-3, "scaled max x");
    assert_close(max[2], 10.0, 1e-3, "scaled max z");
    assert_close(min[0], 0.0, 1e-3, "origin fixed point");
}

#[test]
fn body_boolean_combines_external_solid() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Build a tool body separately, then cut it from a base body.
    let tool = kernel
        .execute_solid_chain(
            &[blind_pad(
                vec![circle_wire(10.0, 10.0, 4.0)],
                20.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("tool body");
    let plain = kernel
        .execute_solid_chain(
            &[blind_pad(
                vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                10.0,
                BooleanOp::NewSolid,
            )],
            &detail,
        )
        .expect("base body");
    let cut = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Boolean {
                    tool_brep: tool.brep_blob.clone(),
                    kind: BoolKind::Cut,
                },
            ],
            &detail,
        )
        .expect("base minus tool");
    assert!(
        cut.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3,
        "the cylindrical cut adds wall triangles"
    );
    let common = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Boolean {
                    tool_brep: tool.brep_blob,
                    kind: BoolKind::Common,
                },
            ],
            &detail,
        )
        .expect("base intersect tool");
    let (min, max) = common.bounds_mm.expect("bounds");
    assert_close(
        max[0] - min[0],
        8.0,
        0.1,
        "common is the cylinder footprint",
    );
    assert_close(max[2] - min[2], 10.0, 1e-3, "common keeps base height");
}

#[test]
fn chain_error_reports_failing_op_index() {
    let _serial = occt_guard();
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let err = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
                    5.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Fillet {
                    radius: 500.0, // impossibly large for a 10 mm box
                    edges: EdgeSelection::All,
                },
            ],
            &detail,
        )
        .expect_err("oversized fillet must fail");
    assert_eq!(err.op_index, 1, "failure attributed to the fillet op");
}
