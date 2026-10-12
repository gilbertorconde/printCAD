//! Tests for the solid ops: termination modes, taper, loft, pipe, helix,
//! primitives, dress-ups, patterns, and body booleans.

use kernel_api::{
    BoolKind, BooleanOp, ChamferSpec, EdgeSelection, ExtrudeTermination, FaceProbe, Placement,
    PrimitiveKind, Profile, ProfilePlane, ProfileSegment, ProfileWire, RevolveTermination, SolidOp,
    SweepKind, TessellationSettings,
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

fn plane_at_z(z: f64) -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, z],
        ..xy_plane()
    }
}

fn rect_wire(x0: f64, y0: f64, x1: f64, y1: f64) -> ProfileWire {
    ProfileWire {
        names: Vec::new(),
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
        names: Vec::new(),
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
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // A 20x20x10 pad with a through-all circular cut from the sketch plane
    // at z=0, run forwards through everything.
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
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    let ellipse = ProfileWire {
        names: Vec::new(),
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
        names: Vec::new(),
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

/// A spline of its own degree and knots pads, and a rational one draws an
/// exact conic: a quarter disc bounded by a rational quadratic has a
/// quarter of the disc's area.
#[test]
fn nurbs_profiles_pad_exactly() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let quarter = ProfileWire {
        names: Vec::new(),
        segments: vec![
            ProfileSegment::Line {
                start: [0.0, 0.0],
                end: [10.0, 0.0],
            },
            ProfileSegment::Nurbs {
                degree: 2,
                knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                control_points: vec![[10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                weights: vec![1.0, w, 1.0],
                periodic: false,
            },
            ProfileSegment::Line {
                start: [0.0, 10.0],
                end: [0.0, 0.0],
            },
        ],
    };
    let result = kernel
        .execute_solid_chain(
            &[blind_pad(vec![quarter], 2.0, BooleanOp::NewSolid)],
            &detail,
        )
        .expect("quarter disc pad");
    let props = kernel
        .physical_properties(&result.brep_blob)
        .expect("the pad measures");
    let volume = props.volume_mm3.expect("a closed solid");
    let exact = std::f64::consts::PI * 100.0 / 4.0 * 2.0;
    assert!(
        (volume - exact).abs() < 1e-3 * exact,
        "volume {volume} vs {exact}"
    );

    // A quintic over knots of its own, closed with a line, and a
    // quadratic ring.
    let open = ProfileWire {
        names: Vec::new(),
        segments: vec![
            ProfileSegment::Nurbs {
                degree: 5,
                knots: vec![
                    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.3, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
                ],
                control_points: vec![
                    [0.0, 0.0],
                    [3.0, 6.0],
                    [7.0, 8.0],
                    [11.0, 7.0],
                    [15.0, 5.0],
                    [18.0, 2.0],
                    [20.0, 0.0],
                ],
                weights: Vec::new(),
                periodic: false,
            },
            ProfileSegment::Line {
                start: [20.0, 0.0],
                end: [0.0, 0.0],
            },
        ],
    };
    let ring = ProfileWire {
        names: Vec::new(),
        segments: vec![ProfileSegment::Nurbs {
            degree: 2,
            knots: Vec::new(),
            control_points: vec![[30.0, 0.0], [40.0, 0.0], [40.0, 10.0], [30.0, 10.0]],
            weights: Vec::new(),
            periodic: true,
        }],
    };
    let result = kernel
        .execute_solid_chain(
            &[blind_pad(vec![open, ring], 3.0, BooleanOp::NewSolid)],
            &detail,
        )
        .expect("spline pads");
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "the open spline's start");
    assert!(max[0] > 35.0 && max[0] <= 40.001, "the ring: {max:?}");
    assert_close(max[2], 3.0, 1e-3, "pad height");
}

#[test]
fn loft_skins_between_two_rectangles() {
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
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();

    // Circle profile on the YZ plane at the path start, swept along an
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
            names: Vec::new(),
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
        frame: kernel_api::PipeFrame::RotationMinimizing,
        corner: kernel_api::PipeCorner::Transformed,
        sections: Vec::new(),
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
            turns: None,
            growth: None,
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
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
            [16.0, 10.0, 6.0],
        ),
        (
            PrimitiveKind::Prism {
                sides: 6,
                circumradius: 5.0,
                height: 7.0,
                skew_x_deg: 0.0,
                skew_y_deg: 0.0,
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
                    follow_tangent: false,
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
                    follow_tangent: false,
                },
            ],
            &detail,
        )
        .expect("chamfer all edges");
    assert!(chamfered.mesh.indices.len() / 3 > plain.mesh.indices.len() / 3);
}

/// A picked point names the edge beside it; one in the middle of a face
/// names none, and the fillet says so rather than rounding whichever edge
/// happens to be nearest.
#[test]
fn an_edge_pick_far_from_every_edge_fails() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let fillet_at = |kernel: &mut OgeomKernel, point: [f64; 3]| {
        kernel.execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Fillet {
                    radius: 2.0,
                    edges: EdgeSelection::Near(vec![point]),
                    follow_tangent: false,
                },
            ],
            &detail,
        )
    };
    fillet_at(&mut kernel, [10.0, 0.5, 10.0]).expect("a pick beside the edge rounds it");
    let err =
        fillet_at(&mut kernel, [10.0, 10.0, 10.0]).expect_err("the face's middle names no edge");
    assert!(
        err.message.contains("no edge near the pick"),
        "{}",
        err.message
    );
}

/// A picked edge is the nearest one that runs the way the pick says: near
/// a corner the direction tells the edges apart, and a pick whose
/// direction no edge near it takes names none.
#[test]
fn an_edge_pick_takes_the_edge_running_its_way() {
    use kernel_api::EdgeProbe;
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let mut fillet_at = |point: [f64; 3], direction: [f64; 3]| {
        let result = kernel.execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Fillet {
                    radius: 2.0,
                    edges: EdgeSelection::Picked(vec![EdgeProbe {
                        faces: [0, 0],
                        point,
                        direction,
                    }]),
                    follow_tangent: false,
                },
            ],
            &detail,
        )?;
        let volume = kernel
            .physical_properties(&result.brep_blob)
            .unwrap()
            .volume_mm3
            .unwrap();
        Ok::<f64, kernel_api::ChainError>(volume)
    };
    // A rounded edge of length l takes (1 - pi/4) r^2 l away.
    let taken = |l: f64| 4000.0 - (1.0 - std::f64::consts::FRAC_PI_4) * 4.0 * l;
    let along_top = fillet_at([10.0, 0.5, 9.5], [1.0, 0.0, 0.0]).expect("the top edge rounds");
    assert!((along_top - taken(20.0)).abs() < 0.05, "{along_top}");
    // By the corner, nearer the top edge than the upright one.
    let upright = fillet_at([0.4, 0.6, 9.8], [0.0, 0.0, 1.0]).expect("the upright edge rounds");
    assert!((upright - taken(10.0)).abs() < 0.05, "{upright}");
    let err = fillet_at([10.0, 0.5, 9.5], [0.0, 0.0, 1.0]).expect_err("no upright edge there");
    assert!(err.message.contains("runs along"), "{}", err.message);
}

#[test]
fn fillet_of_faces_selection_uses_nearest_face() {
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
                    follow_tangent: false,
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
                    both_sides: false,
                    open_face_names: Vec::new(),
                    value: 1.5,
                    open_faces: vec![[10.0, 10.0, 10.0]],
                    inward: true,
                    join: Default::default(),
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
                    face_names: Vec::new(),
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

/// Copies without their original: the whole solid gives way to its
/// copies, and a feature named an original leaves the solid as it was.
#[test]
fn transform_copies_leaves_the_original_out() {
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
    let block = blind_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
        5.0,
        BooleanOp::NewSolid,
    );
    let moved = kernel
        .execute_solid_chain(
            &[
                block.clone(),
                SolidOp::TransformCopies {
                    transforms: vec![translate(20.0), translate(40.0)],
                    originals: vec![],
                },
            ],
            &detail,
        )
        .expect("the block's copies");
    let (min, max) = moved.bounds_mm.expect("bounds");
    assert_close(min[0], 20.0, 1e-3, "the block itself gone");
    assert_close(max[0], 50.0, 1e-3, "the last copy");

    let refused = kernel.execute_solid_chain(
        &[
            block.clone(),
            SolidOp::TransformCopies {
                transforms: vec![],
                originals: vec![],
            },
        ],
        &detail,
    );
    assert!(refused.is_err(), "nothing left");

    let boss = blind_pad(vec![circle_wire(5.0, 5.0, 2.0)], 10.0, BooleanOp::Fuse);
    let copied = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 10.0, 30.0)],
                    5.0,
                    BooleanOp::NewSolid,
                ),
                boss,
                SolidOp::TransformCopies {
                    transforms: vec![translate(20.0)],
                    originals: vec![1],
                },
            ],
            &detail,
        )
        .expect("the boss's copy");
    let (min, max) = copied.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "the plate");
    assert_close(max[0], 27.0, 1e-3, "the copy past the plate");
    let volume = kernel
        .physical_properties(&copied.brep_blob)
        .unwrap()
        .volume_mm3
        .unwrap();
    let want = 10.0 * 30.0 * 5.0 + std::f64::consts::PI * 4.0 * 10.0;
    assert!(
        (volume - want).abs() < 0.5,
        "one boss: {volume} against {want}"
    );
}

#[test]
fn pattern_of_subtractive_tool_repeats_the_cut() {
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
                    tool_transform: None,
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
                    tool_transform: None,
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
                    follow_tangent: false,
                },
            ],
            &detail,
        )
        .expect_err("oversized fillet must fail");
    assert_eq!(err.op_index, 1, "failure attributed to the fillet op");
}

#[test]
fn a_chain_can_start_from_another_chains_snapshot_and_only_start_there() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let pad = blind_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
        4.0,
        BooleanOp::NewSolid,
    );
    let built = kernel
        .execute_solid_chain(std::slice::from_ref(&pad), &detail)
        .expect("the pad builds");

    let cloned = kernel
        .execute_solid_chain(
            &[SolidOp::Shape {
                brep: built.brep_blob.clone(),
            }],
            &detail,
        )
        .expect("the snapshot builds on its own");
    assert_eq!(cloned.bounds_mm, built.bounds_mm);

    let later = kernel.execute_solid_chain(
        &[
            pad,
            SolidOp::Shape {
                brep: built.brep_blob,
            },
        ],
        &detail,
    );
    assert!(later.is_err(), "a snapshot can only begin a chain");
}

#[test]
fn a_box_outline_names_its_twelve_edges() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let pad = blind_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
        4.0,
        BooleanOp::NewSolid,
    );
    let built = kernel
        .execute_solid_chain(std::slice::from_ref(&pad), &detail)
        .expect("the pad builds");
    let mesh = &built.mesh;
    assert_eq!(
        mesh.edges.len() / 2,
        mesh.edge_ids.len(),
        "one id per segment"
    );
    let distinct: std::collections::HashSet<u32> = mesh.edge_ids.iter().copied().collect();
    assert_eq!(distinct.len(), 12, "a box has twelve edges");
    for pair in mesh.edges.chunks(2) {
        let a = mesh.positions[pair[0] as usize];
        let b = mesh.positions[pair[1] as usize];
        let len = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        assert!(len > 1e-3, "a straight edge is one segment, never a dot");
    }
}

/// The distinct kernel faces a mesh was cut from.
fn face_count(mesh: &kernel_api::TriMesh) -> usize {
    mesh.faces
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// The volume a closed, outward-wound mesh encloses.
fn mesh_volume(mesh: &kernel_api::TriMesh) -> f64 {
    let mut six_v = 0.0f64;
    for tri in mesh.indices.chunks(3) {
        let p = |i: u32| {
            let q = mesh.positions[i as usize];
            [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]
        };
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        six_v += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    six_v / 6.0
}

/// A fine triangulation, so a chord lies close to its curve.
fn fine_detail() -> TessellationSettings {
    TessellationSettings {
        linear_deflection_mode: kernel_api::LinearDeflectionMode::AbsoluteMm,
        chord_tolerance: 0.01,
        ..TessellationSettings::default()
    }
}

/// Four rounds meeting pairwise at the corners of a face mitre: the solid
/// keeps its six faces, trimmed, plus one band per edge and nothing else.
#[test]
fn fillets_of_one_face_meet_mitred_at_its_corners() {
    let mut kernel = new_kernel();
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
                    follow_tangent: false,
                },
            ],
            &TessellationSettings::default(),
        )
        .expect("fillet top face edges");
    assert_eq!(
        face_count(&result.mesh),
        10,
        "six trimmed faces and four bands, no caps at the corners"
    );
}

/// Four bevels meeting pairwise at the corners of a face mitre the same
/// way: ten faces, and the volume of the box less four wedges, each corner
/// overlap counted once.
#[test]
fn chamfers_of_one_face_meet_mitred_at_its_corners() {
    let mut kernel = new_kernel();
    let result = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Chamfer {
                    spec: ChamferSpec::EqualDistance { distance: 1.0 },
                    flip: false,
                    edges: EdgeSelection::OfFaces(vec![[10.0, 10.0, 10.0]]),
                    follow_tangent: false,
                },
            ],
            &TessellationSettings::default(),
        )
        .expect("chamfer top face edges");
    assert_eq!(
        face_count(&result.mesh),
        10,
        "six trimmed faces and four bevels"
    );
    let expected = 4000.0 - 4.0 * (0.5 * 20.0) + 4.0 / 3.0;
    let volume = mesh_volume(&result.mesh);
    assert!(
        (volume - expected).abs() < 1e-2,
        "the box less four wedges, corners counted once: {volume} vs {expected}"
    );
}

/// Three rounds meeting at a box corner close it with the octant of the
/// sphere tangent to all three bands: a tenth face, on which every point
/// of the shell inside the corner cube lies.
#[test]
fn fillets_of_three_edges_round_their_corner() {
    let mut kernel = new_kernel();
    let radius = 2.0;
    let result = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Fillet {
                    radius,
                    edges: EdgeSelection::Near(vec![
                        [10.0, 20.0, 10.0],
                        [20.0, 10.0, 10.0],
                        [20.0, 20.0, 5.0],
                    ]),
                    follow_tangent: false,
                },
            ],
            &fine_detail(),
        )
        .expect("fillet the three edges at a corner");
    assert_eq!(
        face_count(&result.mesh),
        10,
        "six trimmed faces, three bands and the corner patch"
    );
    let centre = [20.0 - radius, 20.0 - radius, 10.0 - radius];
    let mesh = &result.mesh;
    for tri in mesh.indices.chunks(3) {
        let p = |i: u32| {
            let q = mesh.positions[i as usize];
            [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]
        };
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        let at = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let inside_corner = (0..3).all(|i| at[i] > centre[i] + 1e-3);
        if !inside_corner {
            continue;
        }
        let d = (0..3)
            .map(|i| (at[i] - centre[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(
            (d - radius).abs() < 0.05,
            "a triangle at {at:?} lies {d:.3} from the corner sphere's centre, not {radius}"
        );
    }
}

/// A pad on a pad of the same footprint fuses into one block whose sides
/// are each split along the seam; refining merges every split side back
/// into one face, leaving the six a block has.
#[test]
fn refining_merges_the_faces_a_flush_fuse_splits() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let stacked = || {
        let mut upper = blind_pad(vec![rect_wire(0.0, 0.0, 20.0, 20.0)], 10.0, BooleanOp::Fuse);
        if let SolidOp::Sweep { profile, .. } = &mut upper {
            profile.plane.origin = [0.0, 0.0, 10.0];
        }
        vec![
            blind_pad(
                vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                10.0,
                BooleanOp::NewSolid,
            ),
            upper,
        ]
    };
    let split = kernel
        .execute_solid_chain(&stacked(), &detail)
        .expect("stacked pads fuse");
    assert!(
        face_count(&split.mesh) > 6,
        "the fuse leaves each side in two pieces"
    );

    let mut refined_ops = stacked();
    refined_ops.push(SolidOp::Refine);
    let refined = kernel
        .execute_solid_chain(&refined_ops, &detail)
        .expect("refine runs");
    assert_eq!(face_count(&refined.mesh), 6, "one face per side");
    let (min, max) = refined.bounds_mm.expect("bounds");
    assert_close(max[2] - min[2], 20.0, 1e-3, "the block keeps its height");
}

/// The refine after a feature merges what that feature made with its
/// neighbours and leaves the rest of the part as it stands: a split an
/// earlier feature left (its refine off) survives a later feature's refine
/// elsewhere, while the later feature's own split sides merge.
#[test]
fn a_refine_after_a_feature_looks_only_around_what_it_made() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let pad_at = |x: f64, z: f64, op| {
        let mut pad = blind_pad(vec![rect_wire(x, 0.0, x + 20.0, 20.0)], 10.0, op);
        if let SolidOp::Sweep { profile, .. } = &mut pad {
            profile.plane.origin = [0.0, 0.0, z];
        }
        pad
    };
    // Two blocks stacked flush, left unrefined: four split sides.
    let left = [
        pad_at(0.0, 0.0, BooleanOp::NewSolid),
        pad_at(0.0, 10.0, BooleanOp::Fuse),
    ];
    let split = kernel
        .execute_solid_chain(&left, &detail)
        .expect("stacked pads fuse");
    let split_faces = face_count(&split.mesh);
    assert!(split_faces > 6);
    // Far to the side, a block on a block, refined.
    let mut ops = left.to_vec();
    ops.extend([
        pad_at(100.0, 0.0, BooleanOp::Fuse),
        pad_at(100.0, 10.0, BooleanOp::Fuse),
        SolidOp::Refine,
    ]);
    let built = kernel
        .execute_solid_chain(&ops, &detail)
        .expect("the far blocks fuse and refine");
    assert_eq!(
        face_count(&built.mesh),
        split_faces + 6,
        "the left stack keeps its splits; the right one, its last block's \
         sides merged across the seam with the first block's, is one block"
    );
}

/// Each face of a mesh says what surface it was cut from: a cylinder's side
/// knows its axis and radius, its caps are planes facing out.
#[test]
fn a_mesh_names_the_exact_surface_of_each_face() {
    use kernel_api::FaceSurface;
    let mut kernel = new_kernel();
    let ops = [SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 5.0,
            height: 12.0,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .expect("a cylinder builds");
    let mesh = &result.mesh;
    let faces = mesh.faces.iter().copied().max().unwrap() as usize + 1;
    assert_eq!(mesh.face_surfaces.len(), faces, "one surface per face");
    let side = mesh
        .face_surfaces
        .iter()
        .find_map(|s| match s {
            FaceSurface::Cylinder { axis, radius, .. } => Some((*axis, *radius)),
            _ => None,
        })
        .expect("the side is a cylinder");
    assert!((side.1 - 5.0).abs() < 1e-4);
    assert!(
        (side.0[2].abs() - 1.0).abs() < 1e-5,
        "about Z: {:?}",
        side.0
    );
    let cap_normals: Vec<f32> = mesh
        .face_surfaces
        .iter()
        .filter_map(|s| match s {
            FaceSurface::Plane { normal, origin } => Some(normal[2] * (origin[2] - 6.0).signum()),
            _ => None,
        })
        .collect();
    assert_eq!(cap_normals.len(), 2);
    assert!(
        cap_normals.iter().all(|n| (*n - 1.0).abs() < 1e-5),
        "each cap faces away from the middle: {cap_normals:?}"
    );
}

/// An edge projects onto a plane as the sketch curve it is there: a
/// cylinder's rim onto its cap's plane is its circle, and a line along the
/// plane's normal is a point.
#[test]
fn an_edge_projects_onto_a_plane_as_an_exact_curve() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    let mut kernel = new_kernel();
    let ops = [SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 5.0,
            height: 12.0,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .expect("a cylinder builds");
    let raised = ProfilePlane {
        origin: [0.0, 0.0, 30.0],
        ..xy_plane()
    };
    let rim = kernel_ogeom::QUERIES
        .project_edge(&solid.brep_blob, [0.0, 5.0, 12.0], &raised)
        .expect("the rim projects");
    match rim {
        ProjectedEdge::Circle { centre, radius, .. } => {
            assert!(centre[0].abs() < 1e-9 && centre[1].abs() < 1e-9);
            assert!((radius - 5.0).abs() < 1e-9);
        }
        other => panic!("{other:?}"),
    }
    // The seam runs straight up the side, along the plane's normal.
    let seam = kernel_ogeom::QUERIES
        .project_edge(&solid.brep_blob, [5.0, 0.0, 6.0], &raised)
        .expect("the seam projects");
    assert!(matches!(seam, ProjectedEdge::Point(_)), "{seam:?}");
    // A point far from every edge names none.
    let far = kernel_ogeom::QUERIES.project_edge(&solid.brep_blob, [200.0, 200.0, 6.0], &raised);
    assert!(far.is_err(), "{far:?}");
}

/// A spline edge projects as the spline it is, not points along it: a pad
/// of a quintic's top rim comes back with the quintic's degree, knots and
/// control points.
#[test]
fn a_spline_edge_projects_as_its_spline() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    let mut kernel = new_kernel();
    let knots = vec![
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.3, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    ];
    let control = vec![
        [0.0, 0.0],
        [3.0, 6.0],
        [7.0, 8.0],
        [11.0, 7.0],
        [15.0, 5.0],
        [18.0, 2.0],
        [20.0, 0.0],
    ];
    let wire = ProfileWire {
        names: Vec::new(),
        segments: vec![
            ProfileSegment::Nurbs {
                degree: 5,
                knots: knots.clone(),
                control_points: control.clone(),
                weights: Vec::new(),
                periodic: false,
            },
            ProfileSegment::Line {
                start: [20.0, 0.0],
                end: [0.0, 0.0],
            },
        ],
    };
    let solid = kernel
        .execute_solid_chain(
            &[blind_pad(vec![wire], 3.0, BooleanOp::NewSolid)],
            &TessellationSettings::default(),
        )
        .expect("the spline pads");
    let raised = ProfilePlane {
        origin: [0.0, 0.0, 30.0],
        ..xy_plane()
    };
    let rim = kernel_ogeom::QUERIES
        .project_edge(&solid.brep_blob, [10.0, 7.0, 3.0], &raised)
        .expect("the rim projects");
    let ProjectedEdge::Spline {
        degree,
        knots: got_knots,
        control_points,
        weights,
    } = rim
    else {
        panic!("{rim:?}");
    };
    assert_eq!(degree, 5);
    assert_eq!(got_knots.len(), knots.len());
    assert!(weights.is_empty(), "not rational: {weights:?}");
    // The edge may run either way along the spline.
    let forward = control_points
        .iter()
        .zip(&control)
        .all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    let backward = control_points
        .iter()
        .zip(control.iter().rev())
        .all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    assert!(forward || backward, "{control_points:?}");
}

/// Where a plane cuts a cone, the edge projects as the conic it is: a
/// plane parallel to the axis leaves a hyperbola's branch, one parallel to
/// a ruling a parabola, each exact rather than points along it.
#[test]
#[ignore = "kernel: a boolean of a plane and a cone stores the section edge as a fitted B-spline, not the exact hyperbola or parabola intersect_surfaces returns for the pair (ogeom-rs#211)"]
fn a_cone_cut_by_a_plane_projects_as_its_conic() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    let cone = SolidOp::Primitive {
        kind: PrimitiveKind::Cone {
            radius1: 10.0,
            radius2: 0.0,
            height: 10.0,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    };
    let near = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-6;

    // Everything past x = 3 cut away: on that plane (y, z) the rim is
    // (10 - z)² - y² = 9, the branch about (0, 10) opening down.
    let mut kernel = new_kernel();
    let past_x = SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 20.0,
            width: 40.0,
            height: 20.0,
        },
        placement: Placement {
            origin: [3.0, -20.0, -5.0],
            ..Placement::default()
        },
        op: BooleanOp::Cut,
    };
    let solid = kernel
        .execute_solid_chain(&[cone.clone(), past_x], &TessellationSettings::default())
        .expect("the cone is cut square to x");
    let side = ProfilePlane {
        origin: [30.0, 0.0, 0.0],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    };
    let rim = kernel_ogeom::QUERIES
        .project_edge(&solid.brep_blob, [3.0, 4.0, 5.0], &side)
        .expect("the rim projects");
    let ProjectedEdge::Hyperbola {
        centre,
        major,
        minor,
        range,
    } = rim
    else {
        panic!("{rim:?}");
    };
    assert!(near(centre, [0.0, 10.0]), "{centre:?}");
    assert!(near(major, [0.0, -3.0]), "{major:?}");
    assert!(
        (minor[0].abs() - 3.0).abs() < 1e-6 && minor[1].abs() < 1e-6,
        "{minor:?}"
    );
    let at = |s: f64| {
        [
            centre[0] + major[0] * s.cosh() + minor[0] * s.sinh(),
            centre[1] + major[1] * s.cosh() + minor[1] * s.sinh(),
        ]
    };
    let ends = [at(range.0), at(range.1)];
    let half = 91f64.sqrt();
    assert!(
        ends.iter()
            .all(|e| e[1].abs() < 1e-6 && (e[0].abs() - half).abs() < 1e-6),
        "{ends:?}"
    );

    // Everything past x + z = 5 cut away: the plane runs parallel to the
    // ruling on the -x side, and the rim on it is a parabola whose vertex
    // stands at (-2.5, 0, 7.5) and whose ends sit on the base circle.
    let mut kernel = new_kernel();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let x_axis = [s, 0.0, -s];
    let z_axis = [s, 0.0, s];
    let past_slant = SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 40.0,
            width: 40.0,
            height: 30.0,
        },
        placement: Placement {
            origin: [2.5 - 20.0 * x_axis[0], -20.0, 2.5 - 20.0 * x_axis[2]],
            x_axis,
            z_axis,
        },
        op: BooleanOp::Cut,
    };
    let solid = kernel
        .execute_solid_chain(&[cone, past_slant], &TessellationSettings::default())
        .expect("the cone is cut along a ruling");
    let slant = ProfilePlane {
        origin: [2.5, 0.0, 2.5],
        x_axis,
        y_axis: [0.0, 1.0, 0.0],
        normal: z_axis,
    };
    let rim = kernel_ogeom::QUERIES
        .project_edge(&solid.brep_blob, [0.0, 5.0, 5.0], &slant)
        .expect("the rim projects");
    let ProjectedEdge::Parabola {
        vertex,
        axis,
        focal,
        range,
    } = rim
    else {
        panic!("{rim:?}");
    };
    assert!(near(vertex, [-10.0 * s, 0.0]), "{vertex:?}");
    assert!(near(axis, [1.0, 0.0]), "{axis:?}");
    let at = |t: f64| {
        let along = t * t / (4.0 * focal);
        [
            vertex[0] + axis[0] * along - axis[1] * t,
            vertex[1] + axis[1] * along + axis[0] * t,
        ]
    };
    // The base circle meets x = 5 at y = ±√75, which stands 5/√2 along
    // the plane's x.
    let ends = [at(range.0), at(range.1)];
    let half = 75f64.sqrt();
    assert!(
        ends.iter()
            .all(|e| (e[0] - 5.0 * s).abs() < 1e-6 && (e[1].abs() - half).abs() < 1e-6),
        "{ends:?}"
    );
}

/// A parabola or a hyperbola edge projects as that conic, exact: every
/// point along the edge, dropped onto the plane, lies on the curve the
/// projection gives, between its ends.
#[test]
fn a_conic_edge_projects_as_its_conic() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    use ogeom::core::Tolerances;
    use ogeom::geom::{Curve, Curve3d as _, HyperbolaCurve, ParabolaCurve};
    use ogeom::math::{Direction, Frame, Hyperbola, Parabola, Point, Vector};
    let tol = Tolerances::millimetres();
    // Each conic in a plane tilted 30° about X, projected onto XY.
    let tilt = Direction::new(Vector::new(0.0, -0.5, 0.75f64.sqrt()), tol).unwrap();
    let frame = Frame::new(Point::new(1.0, 2.0, 5.0), tilt, Direction::X, tol).unwrap();
    let curves = [
        Curve::Parabola(
            ParabolaCurve::new(Parabola::new(frame, 2.0, tol).unwrap(), 100.0).unwrap(),
        ),
        Curve::Hyperbola(
            HyperbolaCurve::new(Hyperbola::new(frame, 4.0, 2.0, tol).unwrap(), 10.0).unwrap(),
        ),
    ];
    for curve in curves {
        let range = (-1.0, 1.5);
        let mut model = ogeom::topo::Model::new();
        let edge = ogeom::algo::make_edge(&mut model, curve.clone(), range, tol)
            .unwrap()
            .shape;
        let blob = ogeom::io::native::write(
            &model,
            std::slice::from_ref(&edge),
            ogeom::io::native::WriteOptions {
                triangulations: false,
            },
        )
        .unwrap()
        .into_bytes();
        let along: Vec<[f64; 2]> = (0..=20)
            .map(|i| {
                let t = range.0 + (range.1 - range.0) * f64::from(i) / 20.0;
                let p = curve.point_at(t, tol).unwrap();
                [p.x, p.y]
            })
            .collect();
        let mid = curve.point_at(0.25, tol).unwrap();
        let projected = kernel_ogeom::QUERIES
            .project_edge(&blob, [mid.x, mid.y, mid.z], &xy_plane())
            .expect("the conic projects");
        let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
        // Each point's parameter on the projected curve, and how far off
        // the curve it stands.
        let (on, ends): (Vec<(f64, f64)>, (f64, f64)) = match &projected {
            ProjectedEdge::Parabola {
                vertex,
                axis,
                focal,
                range,
            } => {
                let side = [-axis[1], axis[0]];
                let on = along
                    .iter()
                    .map(|p| {
                        let d = [p[0] - vertex[0], p[1] - vertex[1]];
                        let (u, s) = (dot(d, *axis), dot(d, side));
                        (s, (u - s * s / (4.0 * focal)).abs())
                    })
                    .collect();
                (on, *range)
            }
            ProjectedEdge::Hyperbola {
                centre,
                major,
                minor,
                range,
            } => {
                let on = along
                    .iter()
                    .map(|p| {
                        let d = [p[0] - centre[0], p[1] - centre[1]];
                        let c = dot(d, *major) / dot(*major, *major);
                        let s = (dot(d, *minor) / dot(*minor, *minor)).asinh();
                        (s, (c - s.cosh()).abs())
                    })
                    .collect();
                (on, *range)
            }
            other => panic!("{other:?}"),
        };
        for (s, off) in on {
            assert!(off < 1e-9, "{off} off {projected:?}");
            assert!(
                s >= ends.0 - 1e-9 && s <= ends.1 + 1e-9,
                "{s} past {ends:?}"
            );
        }
    }
}

/// A profile's regions mesh where they stand, holes left open: a 10 × 10
/// square with a 4 × 4 hole covers 84 mm², all of it in the plane.
#[test]
fn a_profile_meshes_to_its_regions() {
    use kernel_api::KernelQueries;
    let profile = kernel_api::Profile {
        plane: ProfilePlane {
            origin: [0.0, 0.0, 5.0],
            ..xy_plane()
        },
        wires: vec![
            rect_wire(0.0, 0.0, 10.0, 10.0),
            rect_wire(3.0, 3.0, 7.0, 7.0),
        ],
    };
    let mesh = kernel_ogeom::QUERIES
        .profile_mesh(&profile)
        .expect("the profile meshes");
    let area: f64 = mesh
        .indices
        .chunks(3)
        .map(|t| {
            let p = |i: u32| mesh.positions[i as usize].map(f64::from);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
            (u[0] * v[1] - u[1] * v[0]).abs() / 2.0
        })
        .sum();
    assert!((area - 84.0).abs() < 1e-3, "area {area}");
    assert!(mesh.positions.iter().all(|p| (p[2] - 5.0).abs() < 1e-5));
}

/// A half space bounded by a plane, the material on `inside`'s side.
fn half_space_at(
    model: &mut ogeom::topo::Model,
    origin: ogeom::math::Point,
    normal: ogeom::math::Direction,
    inside: ogeom::math::Point,
) -> ogeom::topo::Shape {
    let tol = ogeom::core::Tolerances::millimetres();
    let plane = ogeom::math::Plane::through(origin, normal);
    let surface = ogeom::geom::PlaneSurface::over(plane, (-1.0e6, 1.0e6), (-1.0e6, 1.0e6)).unwrap();
    let face = ogeom::algo::make_natural_face(model, ogeom::geom::SurfaceGeometry::Plane(surface))
        .unwrap()
        .shape;
    ogeom::algo::make_half_space(model, &face, inside, tol)
        .unwrap()
        .shape
}

/// A plane well clear of a box sections nothing from it.
#[test]
fn a_half_space_clear_of_a_solid_sections_nothing() {
    use ogeom::math::{Direction, Frame, Point};
    use ogeom::topo::{Model, ShapeType, explore_unique};
    let tol = ogeom::core::Tolerances::millimetres();
    let mut model = Model::new();
    let cube = ogeom::algo::make_box(&mut model, Frame::WORLD, (10.0, 10.0, 10.0), tol)
        .unwrap()
        .shape;
    let half = half_space_at(
        &mut model,
        Point::new(0.0, 0.0, 40.0),
        Direction::Z,
        Point::new(0.0, 0.0, 39.0),
    );
    let section = ogeom::boolean::section(&mut model, &cube, &half, tol)
        .unwrap()
        .shape;
    let edges = explore_unique(&model, &section, ShapeType::Edge).unwrap();
    assert!(edges.is_empty(), "{} edges", edges.len());
}

/// A face on its own is sectioned by a half space into the curves the
/// plane crosses it along, as its solid is.
#[test]
fn a_face_on_its_own_is_sectioned_by_a_half_space() {
    use ogeom::math::{Direction, Frame, Point};
    use ogeom::topo::{Model, ShapeType, explore_unique};
    let tol = ogeom::core::Tolerances::millimetres();
    let mut model = Model::new();
    let cube = ogeom::algo::make_box(&mut model, Frame::WORLD, (10.0, 10.0, 10.0), tol)
        .unwrap()
        .shape;
    let side = explore_unique(&model, &cube, ShapeType::Face)
        .unwrap()
        .into_iter()
        .find(|face| {
            ogeom::algo::shape_bounds(&model, face, tol)
                .unwrap()
                .high()
                .is_some_and(|p| p.x.abs() < 1e-3)
        })
        .expect("the side at x = 0");
    let half = half_space_at(
        &mut model,
        Point::new(0.0, 0.0, 4.0),
        Direction::Z,
        Point::new(0.0, 0.0, 3.0),
    );
    let section = ogeom::boolean::section(&mut model, &side, &half, tol)
        .expect("a face is sectioned")
        .shape;
    let edges = explore_unique(&model, &section, ShapeType::Edge).unwrap();
    assert_eq!(edges.len(), 1, "the line across the side at z = 4");
}

/// A plane cuts a face into the curves it crosses it along, in the plane's
/// own coordinates: a box's side into a line, a cylinder's side into its
/// circle across, or into two lines along it; a face the plane misses
/// comes to nothing.
#[test]
fn a_face_cut_by_a_plane_gives_the_curves_where_they_cross() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    let mut kernel = new_kernel();
    let build = |kernel: &mut OgeomKernel, kind: PrimitiveKind| {
        let ops = [SolidOp::Primitive {
            kind,
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        }];
        kernel
            .execute_solid_chain(&ops, &TessellationSettings::default())
            .expect("the primitive builds")
    };
    let cube = build(
        &mut kernel,
        PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
    );
    let side = kernel_ogeom::QUERIES
        .section_face(&cube.brep_blob, [0.0, 5.0, 5.0], &plane_at_z(4.0))
        .expect("the side is cut");
    let [ProjectedEdge::Line { start, end }] = side.as_slice() else {
        panic!("{side:?}");
    };
    let mut ys = [start[1], end[1]];
    ys.sort_by(f64::total_cmp);
    assert!(start[0].abs() < 1e-7 && end[0].abs() < 1e-7);
    assert!(
        ys[0].abs() < 1e-7 && (ys[1] - 10.0).abs() < 1e-7,
        "{side:?}"
    );
    let missed = kernel_ogeom::QUERIES
        .section_face(&cube.brep_blob, [0.0, 5.0, 5.0], &plane_at_z(40.0))
        .expect("a plane that misses cuts nothing");
    assert!(missed.is_empty(), "{missed:?}");

    let rod = build(
        &mut kernel,
        PrimitiveKind::Cylinder {
            radius: 5.0,
            height: 12.0,
            angle_deg: 360.0,
        },
    );
    let across = kernel_ogeom::QUERIES
        .section_face(&rod.brep_blob, [5.0, 0.0, 6.0], &plane_at_z(6.0))
        .expect("the side is cut across");
    let circle = across.iter().find_map(|e| match e {
        ProjectedEdge::Circle { centre, radius, .. } => Some((*centre, *radius)),
        _ => None,
    });
    let arcs: f64 = across
        .iter()
        .map(|e| match e {
            ProjectedEdge::Circle { range, .. } => range.1 - range.0,
            _ => 0.0,
        })
        .sum();
    assert!(
        (arcs - std::f64::consts::TAU).abs() < 1e-6,
        "a whole turn: {across:?}"
    );
    let (centre, radius) = circle.unwrap();
    assert!(centre[0].abs() < 1e-7 && centre[1].abs() < 1e-7 && (radius - 5.0).abs() < 1e-7);
    // Through its axis, the side is cut along two lines.
    let upright = ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    };
    let along = kernel_ogeom::QUERIES
        .section_face(&rod.brep_blob, [5.0, 0.0, 6.0], &upright)
        .expect("the side is cut along");
    assert_eq!(along.len(), 2, "{along:?}");
    for edge in &along {
        let ProjectedEdge::Line { start, end } = edge else {
            panic!("{along:?}");
        };
        assert!((start[0].abs() - 5.0).abs() < 1e-7 && (start[0] - end[0]).abs() < 1e-7);
        assert!(((start[1] - end[1]).abs() - 12.0).abs() < 1e-7);
    }
}

/// Two boxes where they overlap share a solid of the overlap's volume, in
/// the first box's frame; flush or apart, they share none.
#[test]
fn two_solids_overlap_by_the_volume_they_share() {
    use kernel_api::KernelQueries;
    let mut kernel = new_kernel();
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = kernel
        .execute_solid_chain(&cube, &TessellationSettings::default())
        .expect("a box builds");
    let shifted = |x: f64, y: f64, z: f64| {
        [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, y],
            [0.0, 0.0, 1.0, z],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };
    let blob = &solid.brep_blob;
    let shared = kernel_ogeom::QUERIES
        .overlap(blob, blob, &shifted(6.0, 5.0, 0.0))
        .unwrap()
        .expect("they overlap");
    assert!(
        (shared.volume_mm3 - 4.0 * 5.0 * 10.0).abs() < 1e-6,
        "{}",
        shared.volume_mm3
    );
    let (lo, hi) = shared.mesh.bounds().expect("the shared solid is drawn");
    assert!(
        (lo[0] - 6.0).abs() < 1e-3 && (hi[0] - 10.0).abs() < 1e-3 && (lo[1] - 5.0).abs() < 1e-3,
        "{lo:?} {hi:?}"
    );
    let centre = shared.centre_mm;
    assert!(
        (centre[0] - 8.0).abs() < 1e-6 && (centre[1] - 7.5).abs() < 1e-6,
        "{centre:?}"
    );
    for apart in [shifted(10.0, 0.0, 0.0), shifted(30.0, 0.0, 0.0)] {
        let none = kernel_ogeom::QUERIES.overlap(blob, blob, &apart).unwrap();
        assert!(none.is_none(), "{apart:?}");
    }
}

/// A pad run from a sketch beside a cylinder toward its side stops on the
/// curved surface itself, not on a plane through where it is first met:
/// to the first face, up to the picked face, and up to it with an offset.
#[test]
fn a_pad_stops_exactly_on_a_curved_face() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let rod = SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 10.0,
            height: 20.0,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    };
    // A 4 × 10 rectangle on the plane x = 30, facing +X, padded back
    // toward the rod (reversed).
    let beside = ProfilePlane {
        origin: [30.0, 0.0, 0.0],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    };
    let pad = |termination: ExtrudeTermination| SolidOp::Sweep {
        profile: Profile {
            plane: beside,
            wires: vec![rect_wire(-2.0, 5.0, 2.0, 15.0)],
        },
        kind: SweepKind::Extrude {
            termination,
            second_side: None,
            symmetric: false,
            reversed: true,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::Fuse,
    };
    // The pad from x = 30 in to a cylinder of radius `r`, across y in
    // [-2, 2] and 10 high: 10 × ∫ (30 − √(r² − y²)) dy.
    let pad_volume = |r: f64| {
        let chord = 2.0 * (r * r - 4.0).sqrt() + r * r * (2.0 / r).asin();
        10.0 * (120.0 - chord)
    };
    let rod_volume = std::f64::consts::PI * 100.0 * 20.0;
    let cases = [
        ("to first", ExtrudeTermination::ToFirst, pad_volume(10.0)),
        (
            "up to face",
            ExtrudeTermination::UpToFace {
                name: 0,
                point: [10.0, 0.0, 10.0],
                normal: [1.0, 0.0, 0.0],
                offset: 0.0,
            },
            pad_volume(10.0),
        ),
        (
            "up to face, 1 mm off",
            ExtrudeTermination::UpToFace {
                name: 0,
                point: [10.0, 0.0, 10.0],
                normal: [1.0, 0.0, 0.0],
                offset: 1.0,
            },
            pad_volume(11.0),
        ),
    ];
    for (name, termination, want_pad) in cases {
        let solid = kernel
            .execute_solid_chain(&[rod.clone(), pad(termination)], &detail)
            .unwrap_or_else(|e| panic!("{name}: builds: {e}"));
        let volume = kernel
            .physical_properties(&solid.brep_blob)
            .expect("measures")
            .volume_mm3
            .expect("a volume");
        let want = rod_volume + want_pad;
        assert!(
            (volume - want).abs() < 1e-3 * want,
            "{name}: volume {volume}, want {want}"
        );
    }
}

/// Up to a picked flat face: the base's face nearest the pick, pushed out
/// by the offset.
#[test]
fn a_pad_stops_on_the_picked_flat_face_and_its_offset() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    // A 20 × 20 × 10 block with a 10 × 10 step, 5 high, on top.
    let block = blind_pad(
        vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
        10.0,
        BooleanOp::NewSolid,
    );
    let step = SolidOp::Sweep {
        profile: Profile {
            plane: plane_at_z(10.0),
            wires: vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 5.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::Fuse,
    };
    // A pad from z = 20 down beside the step, up to the block's top face
    // (picked at z = 10), 2 mm off it.
    let pad = SolidOp::Sweep {
        profile: Profile {
            plane: plane_at_z(20.0),
            wires: vec![rect_wire(12.0, 12.0, 18.0, 18.0)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::UpToFace {
                name: 0,
                point: [15.0, 15.0, 10.0],
                normal: [0.0, 0.0, 1.0],
                offset: 2.0,
            },
            second_side: None,
            symmetric: false,
            reversed: true,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::Fuse,
    };
    let solid = kernel
        .execute_solid_chain(&[block, step, pad], &detail)
        .expect("builds");
    let volume = kernel
        .physical_properties(&solid.brep_blob)
        .expect("measures")
        .volume_mm3
        .expect("a volume");
    // Block, step, and a 6 × 6 pad from z = 12 to z = 20.
    let want = 4000.0 + 500.0 + 36.0 * 8.0;
    assert!((volume - want).abs() < 1e-3, "volume {volume}, want {want}");
}

/// The kernel's volume of the solid `ops` build.
fn volume_of(ops: &[SolidOp]) -> Result<f64, String> {
    let mut kernel = new_kernel();
    let built = kernel
        .execute_solid_chain(ops, &TessellationSettings::default())
        .map_err(|e| e.to_string())?;
    kernel
        .physical_properties(&built.brep_blob)
        .map_err(|e| e.to_string())?
        .volume_mm3
        .ok_or_else(|| "no closed volume".to_string())
}

fn assert_volume(ops: &[SolidOp], want: f64, what: &str) {
    let got = volume_of(ops).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        (got - want).abs() <= want * 1e-4,
        "{what}: volume {got}, want {want}"
    );
}

/// `wire` walked the other way round: clockwise where it was
/// counter-clockwise.
fn walked_back(wire: &ProfileWire) -> ProfileWire {
    let mut segments = wire.segments.clone();
    segments.reverse();
    for s in &mut segments {
        if let ProfileSegment::Line { start, end } = s {
            std::mem::swap(start, end);
        }
    }
    ProfileWire::new(segments)
}

/// The XZ plane, v up along world Z: where a spine rising from the XY
/// plane is drawn.
fn xz_plane() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0; 3],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    }
}

fn pipe_along(profile: ProfileWire, spine: Vec<ProfileSegment>) -> SolidOp {
    SolidOp::Pipe {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![profile],
        },
        spine: Profile {
            plane: xz_plane(),
            wires: vec![ProfileWire {
                names: Vec::new(),
                segments: spine,
            }],
        },
        frame: kernel_api::PipeFrame::RotationMinimizing,
        corner: kernel_api::PipeCorner::Transformed,
        sections: Vec::new(),
        op: BooleanOp::NewSolid,
    }
}

fn smooth_loft(a: ProfileWire, b: ProfileWire) -> SolidOp {
    SolidOp::Loft {
        sections: vec![
            Profile {
                plane: xy_plane(),
                wires: vec![a],
            },
            Profile {
                plane: plane_at_z(15.0),
                wires: vec![b],
            },
        ],
        ruled: false,
        closed: false,
        op: BooleanOp::NewSolid,
    }
}

/// A 2 x 2 square 10 to 12 mm from the sketch's Y axis, swept 20 mm up
/// it at pitch 5: four turns of a profile whose centroid runs 11 mm out.
fn helix_of(profile: ProfileWire, cone_angle_deg: f64, pitch: f64, height: f64) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![profile],
        },
        kind: SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch,
            height,
            left_handed: false,
            cone_angle_deg,
            reversed: false,
            turns: None,
            growth: None,
        },
        op: BooleanOp::NewSolid,
    }
}

#[test]
fn a_smooth_loft_holds_its_sections_exactly() {
    let square = rect_wire(-5.0, -5.0, 5.0, 5.0);
    assert_volume(&[smooth_loft(square.clone(), square)], 1500.0, "squares");
    let circle = circle_wire(0.0, 0.0, 5.0);
    assert_volume(
        &[smooth_loft(circle.clone(), circle)],
        std::f64::consts::PI * 25.0 * 15.0,
        "circles",
    );
}

#[test]
fn a_clockwise_profile_pipes_and_coils_as_a_counter_clockwise_one_does() {
    let square = rect_wire(-2.0, -2.0, 2.0, 2.0);
    let rise = vec![ProfileSegment::Line {
        start: [0.0, 0.0],
        end: [0.0, 20.0],
    }];
    assert_volume(
        &[pipe_along(square.clone(), rise.clone())],
        320.0,
        "ccw pipe",
    );
    assert_volume(&[pipe_along(walked_back(&square), rise)], 320.0, "cw pipe");
    let coil = rect_wire(10.0, 0.0, 12.0, 2.0);
    let a = volume_of(&[helix_of(coil.clone(), 0.0, 5.0, 20.0)]).unwrap();
    let b = volume_of(&[helix_of(walked_back(&coil), 0.0, 5.0, 20.0)]).unwrap();
    assert!((a - b).abs() < 1e-6, "{a} vs {b}");
}

#[test]
fn a_profile_square_to_an_arc_spine_pipes_along_it() {
    // A quarter circle rising from the origin, square to the XY plane
    // where it starts: its centre 10 mm along X.
    let arc = vec![ProfileSegment::Arc {
        start: [0.0, 0.0],
        mid: [10.0 - 50f64.sqrt(), 50f64.sqrt()],
        end: [10.0, 10.0],
    }];
    // Pappus: the square's area times the length its centroid runs.
    let want = 16.0 * 10.0 * std::f64::consts::FRAC_PI_2;
    assert_volume(
        &[pipe_along(rect_wire(-2.0, -2.0, 2.0, 2.0), arc)],
        want,
        "arc pipe",
    );
    volume_of(&[helix_of(rect_wire(10.0, 0.0, 12.0, 2.0), 10.0, 5.0, 20.0)])
        .expect("a conical helix");
}

#[test]
fn a_circle_piped_along_a_line_is_a_cylinder() {
    let rise = vec![ProfileSegment::Line {
        start: [0.0, 0.0],
        end: [0.0, 20.0],
    }];
    assert_volume(
        &[pipe_along(circle_wire(0.0, 0.0, 2.0), rise)],
        std::f64::consts::PI * 4.0 * 20.0,
        "cylinder",
    );
}

/// A helical sweep reaches exactly as far as its profile does, and a
/// close-pitched one measures.
#[test]
fn a_helical_sweep_reaches_as_far_as_its_profile() {
    let coil = rect_wire(10.0, 0.0, 12.0, 2.0);
    let mut kernel = new_kernel();
    let built = kernel
        .execute_solid_chain(
            &[helix_of(coil, 0.0, 5.0, 20.0)],
            &TessellationSettings::default(),
        )
        .unwrap();
    let (min, max) = built.bounds_mm.unwrap();
    assert_close(max[0], 12.0, 1e-3, "outer radius");
    assert_close(min[1], 0.0, 1e-3, "starts on the profile");
    assert_close(max[1], 22.0, 1e-3, "height plus the profile");
}

/// A helical sweep is as big as Pappus says, measured at the default
/// deflection as the app measures.
#[test]
fn a_helical_sweep_is_as_big_as_pappus_says() {
    let coil = rect_wire(10.0, 0.0, 12.0, 2.0);
    let want = 4.0 * 4.0 * 2.0 * std::f64::consts::PI * 11.0;
    assert_volume(&[helix_of(coil.clone(), 0.0, 5.0, 20.0)], want, "4 turns");
    assert_volume(
        &[helix_of(coil, 0.0, 3.0, 30.0)],
        10.0 * 4.0 * 2.0 * std::f64::consts::PI * 11.0,
        "10 turns",
    );
}

#[test]
fn a_partial_revolve_of_a_profile_on_its_axis_builds() {
    use kernel_api::PrimitiveKind;
    let turn = |angle: f64| SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![rect_wire(0.0, 0.0, 5.0, 10.0)],
        },
        kind: SweepKind::Revolve {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            angle_deg: angle,
            second_angle_deg: None,
            midplane: false,
            reversed: false,
            termination: Default::default(),
        },
        op: BooleanOp::NewSolid,
    };
    let full = std::f64::consts::PI * 25.0 * 10.0;
    assert_volume(&[turn(270.0)], full * 0.75, "270 degree revolve");
    let prim = |kind| SolidOp::Primitive {
        kind,
        placement: Default::default(),
        op: BooleanOp::NewSolid,
    };
    assert_volume(
        &[prim(PrimitiveKind::Cylinder {
            radius: 5.0,
            height: 10.0,
            angle_deg: 270.0,
        })],
        full * 0.75,
        "270 degree cylinder",
    );
    let band = 462.800_306_058_165_35;
    assert_volume(
        &[prim(PrimitiveKind::Sphere {
            radius: 5.0,
            angle1_deg: -45.0,
            angle2_deg: 45.0,
            angle3_deg: 270.0,
        })],
        band * 0.75,
        "270 degree sphere band",
    );
}

#[test]
fn a_scaled_copy_standing_apart_is_whole() {
    let apart: [[f64; 4]; 4] = [
        [1.5, 0.0, 0.0, 10.0],
        [0.0, 1.5, 0.0, 0.0],
        [0.0, 0.0, 1.5, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    assert_volume(
        &[
            blind_pad(
                vec![rect_wire(0.0, 0.0, 4.0, 4.0)],
                4.0,
                BooleanOp::NewSolid,
            ),
            SolidOp::Transform {
                transforms: vec![apart],
                originals: vec![0],
            },
        ],
        64.0 + 216.0,
        "cube and its copy",
    );
}

/// A sphere's latitude limits cut it flat: a band between two parallels,
/// and a hemisphere from the equator to either pole.
#[test]
fn a_sphere_between_two_latitudes_is_a_flat_capped_band() {
    use kernel_api::PrimitiveKind;
    let sphere = |a1: f64, a2: f64| {
        [SolidOp::Primitive {
            kind: PrimitiveKind::Sphere {
                radius: 5.0,
                angle1_deg: a1,
                angle2_deg: a2,
                angle3_deg: 360.0,
            },
            placement: Default::default(),
            op: BooleanOp::NewSolid,
        }]
    };
    let pi = std::f64::consts::PI;
    let hemisphere = 2.0 / 3.0 * pi * 125.0;
    assert_volume(&sphere(0.0, 90.0), hemisphere, "upper hemisphere");
    assert_volume(&sphere(-90.0, 0.0), hemisphere, "lower hemisphere");
    // A zone of height 2h about the equator: pi (2h r^2 - (2/3) h^3).
    let h = 5.0 * 45f64.to_radians().sin();
    assert_volume(
        &sphere(-45.0, 45.0),
        pi * (2.0 * h * 25.0 - 2.0 / 3.0 * h.powi(3)),
        "band",
    );
}

#[test]
fn a_refined_solid_measures_exactly() {
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        ),
        SolidOp::Sweep {
            profile: Profile {
                plane: plane_at_z(10.0),
                wires: vec![rect_wire(0.0, 0.0, 5.0, 5.0)],
            },
            kind: SweepKind::Extrude {
                termination: ExtrudeTermination::Blind { distance: 5.0 },
                second_side: None,
                symmetric: false,
                reversed: false,
                taper_deg: 0.0,
                direction: None,
            },
            op: BooleanOp::Fuse,
        },
        SolidOp::Refine,
    ];
    let mut kernel = new_kernel();
    let built = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let props = kernel.physical_properties(&built.brep_blob).unwrap();
    assert!(
        !props.approximate,
        "a box and a box on it have closed forms"
    );
    assert!((props.volume_mm3.unwrap() - 4125.0).abs() < 1e-6);
}

/// A pocket two lengths deep from a sketch lying on the solid's top face:
/// the tool reaching both ways from the face cuts like any other.
#[test]
fn a_two_sided_pocket_from_the_top_face_cuts() {
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        ),
        SolidOp::Sweep {
            profile: Profile {
                plane: plane_at_z(10.0),
                wires: vec![rect_wire(5.0, 5.0, 15.0, 15.0)],
            },
            kind: SweepKind::Extrude {
                termination: ExtrudeTermination::Blind { distance: 4.0 },
                second_side: Some(ExtrudeTermination::Blind { distance: 2.0 }),
                symmetric: false,
                reversed: true,
                taper_deg: 0.0,
                direction: None,
            },
            op: BooleanOp::Cut,
        },
    ];
    assert_volume(&ops, 4000.0 - 100.0 * 4.0, "pocketed box");
}

/// A turned body's bounds reach its full radius on every side, not the
/// corners of the mesh that draws it.
#[test]
fn a_revolved_body_measures_its_full_radius() {
    let mut kernel = new_kernel();
    let result = kernel
        .execute_solid_chain(
            &[SolidOp::Sweep {
                profile: Profile {
                    plane: xy_plane(),
                    wires: vec![rect_wire(4.0, 0.0, 6.0, 10.0)],
                },
                kind: SweepKind::Revolve {
                    axis_origin: [0.0, 0.0],
                    axis_dir: [0.0, 1.0],
                    angle_deg: 360.0,
                    second_angle_deg: None,
                    midplane: false,
                    reversed: false,
                    termination: Default::default(),
                },
                op: BooleanOp::NewSolid,
            }],
            &TessellationSettings::default(),
        )
        .expect("a tube");
    let (lo, hi) = result.bounds_mm.expect("bounds");
    for axis in [0, 2] {
        assert!((hi[axis] - 6.0).abs() < 1e-5, "{hi:?}");
        assert!((lo[axis] + 6.0).abs() < 1e-5, "{lo:?}");
    }
    assert!(
        (hi[1] - 10.0).abs() < 1e-5 && lo[1].abs() < 1e-5,
        "{lo:?} {hi:?}"
    );
}

/// A drafted solid measures: its tilted face is a plane like any other.
#[test]
fn a_drafted_solid_measures() {
    let mut kernel = new_kernel();
    let result = kernel
        .execute_solid_chain(
            &[
                blind_pad(
                    vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                SolidOp::Draft {
                    face_names: Vec::new(),
                    angle_deg: 1.5,
                    neutral_point: [10.0, 10.0, 0.0],
                    neutral_normal: [0.0, 0.0, -1.0],
                    pull_dir: None,
                    faces: vec![[0.0, 10.0, 5.0]],
                },
            ],
            &TessellationSettings::default(),
        )
        .expect("a drafted box");
    let props = kernel
        .physical_properties(&result.brep_blob)
        .expect("the drafted box measures");
    // One wall leans in by tan(1.5°) over its 10 mm: a wedge of that
    // section comes off (or goes on) along the 20 mm wall.
    let wedge = 0.5 * 10.0 * 10.0 * 1.5f64.to_radians().tan() * 20.0;
    let volume = props.volume_mm3.expect("a volume");
    assert!(
        (volume - (4000.0 - wedge)).abs() < 1e-3 || (volume - (4000.0 + wedge)).abs() < 1e-3,
        "{volume}"
    );
}

/// A chamfer wider than a face it runs along would take the whole face
/// and more: it is refused, not cut.
#[test]
fn a_chamfer_wider_than_its_face_is_refused() {
    let mut kernel = new_kernel();
    let result = kernel.execute_solid_chain(
        &[
            blind_pad(
                vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                10.0,
                BooleanOp::NewSolid,
            ),
            SolidOp::Chamfer {
                spec: ChamferSpec::EqualDistance { distance: 12.0 },
                flip: false,
                edges: EdgeSelection::Near(vec![[10.0, 0.0, 10.0]]),
                follow_tangent: false,
            },
        ],
        &TessellationSettings::default(),
    );
    assert!(
        result.is_err(),
        "a 12 mm chamfer on a 10 mm face is refused"
    );
}

/// A square swept along straight legs has flat sides, which measure
/// exactly.
#[test]
fn a_square_piped_along_straight_legs_has_flat_sides() {
    let mut kernel = new_kernel();
    let result = kernel
        .execute_solid_chain(
            &[pipe_along(
                rect_wire(-2.0, -2.0, 2.0, 2.0),
                vec![
                    ProfileSegment::Line {
                        start: [0.0, 0.0],
                        end: [0.0, 20.0],
                    },
                    ProfileSegment::Line {
                        start: [0.0, 20.0],
                        end: [20.0, 20.0],
                    },
                ],
            )],
            &TessellationSettings::default(),
        )
        .expect("an L of square section");
    assert!(
        result
            .mesh
            .face_surfaces
            .iter()
            .all(|s| matches!(s, kernel_api::FaceSurface::Plane { .. })),
        "{:?}",
        result.mesh.face_surfaces
    );
    let props = kernel.physical_properties(&result.brep_blob).unwrap();
    assert!(!props.approximate);
    assert!((props.volume_mm3.unwrap() - 640.0).abs() < 1e-6);
}

/// Asked for, a build carries what the edited feature does: an adding
/// feature beside the body before it, a cutting one beside the body after
/// it, each with its own tool.
#[test]
fn a_build_previews_the_feature_it_is_asked_for() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let base = blind_pad(
        vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
        10.0,
        BooleanOp::NewSolid,
    );
    let on_top = |op| SolidOp::Sweep {
        profile: Profile {
            plane: plane_at_z(10.0),
            wires: vec![rect_wire(5.0, 5.0, 15.0, 15.0)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 4.0 },
            second_side: None,
            symmetric: false,
            reversed: op == BooleanOp::Cut,
            taper_deg: 0.0,
            direction: None,
        },
        op,
    };
    let z_range = |mesh: &kernel_api::TriMesh| {
        let (lo, hi) = mesh.bounds().unwrap();
        (lo[2], hi[2])
    };

    let pad = kernel
        .execute_solid_chain_previewing(
            &[base.clone(), on_top(BooleanOp::Fuse)],
            &detail,
            Some(1..2),
        )
        .expect("a boss");
    let preview = pad.preview.expect("a preview");
    assert!(!preview.cuts);
    assert_eq!(z_range(&preview.tool), (10.0, 14.0), "the boss alone");
    let shown = preview.shown.expect("the body before it");
    assert_eq!(z_range(&shown.mesh), (0.0, 10.0));
    assert_eq!(z_range(&pad.mesh), (0.0, 14.0), "the build itself is whole");

    let pocket = kernel
        .execute_solid_chain_previewing(
            &[base.clone(), on_top(BooleanOp::Cut)],
            &detail,
            Some(1..2),
        )
        .expect("a pocket");
    let preview = pocket.preview.expect("a preview");
    assert!(preview.cuts);
    assert_eq!(z_range(&preview.tool), (6.0, 10.0), "the cut alone");
    let shown = preview.shown.expect("the body after it");
    let volume = kernel
        .physical_properties(&shown.brep_blob)
        .unwrap()
        .volume_mm3
        .unwrap();
    assert!((volume - (4000.0 - 400.0)).abs() < 1e-6, "{volume}");

    let first = kernel
        .execute_solid_chain_previewing(&[base], &detail, Some(0..1))
        .expect("a box");
    assert!(
        first.preview.expect("a preview").shown.is_none(),
        "nothing before it"
    );
}

/// A fillet wider than a face it runs along would take the whole face and
/// more: it is refused, not cut.
#[test]
fn a_fillet_wider_than_its_face_is_refused() {
    let mut kernel = new_kernel();
    let result = kernel.execute_solid_chain(
        &[
            blind_pad(
                vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
                10.0,
                BooleanOp::NewSolid,
            ),
            SolidOp::Fillet {
                radius: 12.0,
                edges: EdgeSelection::Near(vec![[10.0, 0.0, 10.0]]),
                follow_tangent: false,
            },
        ],
        &TessellationSettings::default(),
    );
    assert!(result.is_err(), "a 12 mm fillet on a 10 mm face is refused");
}

/// A sweep of `wires` on `plane`, extruded as `termination` says.
fn extrude_on(
    plane: ProfilePlane,
    wires: Vec<ProfileWire>,
    termination: ExtrudeTermination,
    reversed: bool,
    op: BooleanOp,
) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile { plane, wires },
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

fn probe(point: [f64; 3], normal: [f64; 3]) -> FaceProbe {
    FaceProbe {
        name: 0,
        point,
        normal,
    }
}

/// A 20 × 20 × 10 block with a 10 × 20 step, 5 high, over its x < 10 half.
fn stepped_block() -> Vec<SolidOp> {
    vec![
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        ),
        extrude_on(
            plane_at_z(10.0),
            vec![rect_wire(0.0, 0.0, 10.0, 20.0)],
            ExtrudeTermination::Blind { distance: 5.0 },
            false,
            BooleanOp::Fuse,
        ),
    ]
}

/// Up to shape: every line of a pad run down over the step's edge ends on
/// the first picked face it meets, the step's top over the step and the
/// block's top beside it.
#[test]
fn an_up_to_shape_pad_stops_on_the_first_picked_face_each_line_meets() {
    let mut ops = stepped_block();
    ops.push(extrude_on(
        plane_at_z(20.0),
        vec![rect_wire(4.0, 5.0, 16.0, 15.0)],
        ExtrudeTermination::UpToShape {
            faces: vec![
                probe([5.0, 10.0, 15.0], [0.0, 0.0, 1.0]),
                probe([15.0, 10.0, 10.0], [0.0, 0.0, 1.0]),
            ],
            offset: 0.0,
        },
        true,
        BooleanOp::Fuse,
    ));
    // Block and step, then 6 × 10 down 5 onto the step and 6 × 10 down 10
    // onto the block.
    assert_volume(&ops, 4000.0 + 1000.0 + 300.0 + 600.0, "up to shape");
}

/// An up-to-shape offset moves the stop along the sweep: a pocket down to
/// the block's floor, 2 short of it.
#[test]
fn an_up_to_shape_offset_moves_the_stop_along_the_sweep() {
    let ops = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        ),
        extrude_on(
            plane_at_z(10.0),
            vec![rect_wire(5.0, 5.0, 15.0, 15.0)],
            ExtrudeTermination::UpToShape {
                faces: vec![probe([10.0, 10.0, 0.0], [0.0, 0.0, -1.0])],
                offset: -2.0,
            },
            true,
            BooleanOp::Cut,
        ),
    ];
    assert_volume(&ops, 4000.0 - 800.0, "a pocket 8 deep");
}

/// A flat face of the solid, hole and all, extruded as a profile: out of
/// the material as a pad, into it as a pocket.
#[test]
fn a_flat_face_of_the_solid_is_extruded_as_a_profile() {
    let base = [
        blind_pad(
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        ),
        extrude(
            vec![circle_wire(10.0, 10.0, 3.0)],
            ExtrudeTermination::ThroughAll,
            false,
            BooleanOp::Cut,
        ),
    ];
    let face = |distance: f64, reversed: bool, op: BooleanOp| SolidOp::SweepFace {
        face: probe([2.0, 2.0, 10.0], [0.0, 0.0, 1.0]),
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance },
            second_side: None,
            symmetric: false,
            reversed,
            taper_deg: 0.0,
            direction: None,
        },
        op,
    };
    let area = 400.0 - 9.0 * std::f64::consts::PI;
    let mut padded = base.to_vec();
    padded.push(face(5.0, false, BooleanOp::Fuse));
    assert_volume(&padded, area * 15.0, "the top face padded 5");
    let mut pocketed = base.to_vec();
    pocketed.push(face(3.0, true, BooleanOp::Cut));
    assert_volume(&pocketed, area * 7.0, "the top face pocketed 3");
}

/// The XZ plane, v up along world Z: a profile on it turns about world Z
/// taking the sketch's v axis.
fn xz_turn(termination: RevolveTermination, op: BooleanOp) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: xz_plane(),
            wires: vec![rect_wire(5.0, 0.0, 10.0, 10.0)],
        },
        kind: SweepKind::Revolve {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            angle_deg: 360.0,
            second_angle_deg: None,
            midplane: false,
            reversed: false,
            termination,
        },
        op,
    }
}

/// A revolution about world Z from the +X side stops on a face whose plane
/// holds the axis: the wall's side at x = 0 a quarter turn on, adding
/// material up to it; its face at y = 0 half a turn on, a groove that far
/// cutting the quarter through the wall.
#[test]
fn a_revolution_stops_on_a_face_whose_plane_holds_its_axis() {
    // The wall: x in [-20, 0], y in [0, 20], 10 high.
    let wall = blind_pad(
        vec![rect_wire(-20.0, 0.0, 0.0, 20.0)],
        10.0,
        BooleanOp::NewSolid,
    );
    let ring = 75.0 * std::f64::consts::PI * 10.0;
    let cases = [
        ("to first", RevolveTermination::ToFirst, ring / 4.0),
        (
            "up to face",
            RevolveTermination::UpToFace(probe([0.0, 7.0, 5.0], [1.0, 0.0, 0.0])),
            ring / 4.0,
        ),
        ("to last, cut", RevolveTermination::ToLast, -ring / 4.0),
        (
            "up to the far face, cut",
            RevolveTermination::UpToFace(probe([-10.0, 0.0, 5.0], [0.0, -1.0, 0.0])),
            -ring / 4.0,
        ),
    ];
    for (name, termination, change) in cases {
        let op = if change < 0.0 {
            BooleanOp::Cut
        } else {
            BooleanOp::Fuse
        };
        assert_volume(
            &[wall.clone(), xz_turn(termination, op)],
            4000.0 + change,
            name,
        );
    }
}

/// A revolution up to a wall beside its axis: every point of the profile
/// turns until its circle meets the wall's plane, x = -2, at
/// acos(-2 / r).
#[test]
fn a_revolution_stops_on_a_wall_beside_its_axis() {
    let wall = blind_pad(
        vec![rect_wire(-20.0, 0.0, -2.0, 20.0)],
        10.0,
        BooleanOp::NewSolid,
    );
    // 10 × ∫ acos(-2 / r) r dr over r in [5, 10], by Simpson's rule.
    let n = 1000;
    let f = |r: f64| (-2.0 / r).acos() * r;
    let h = 5.0 / n as f64;
    let mut sum = f(5.0) + f(10.0);
    for i in 1..n {
        sum += f(5.0 + i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    let turned = 10.0 * sum * h / 3.0;
    assert_volume(
        &[
            wall,
            xz_turn(
                RevolveTermination::UpToFace(probe([-2.0, 5.0, 5.0], [1.0, 0.0, 0.0])),
                BooleanOp::Fuse,
            ),
        ],
        3600.0 + turned,
        "up to the wall",
    );
}

/// A pattern repeats a face swept off the solid as the tool it built there:
/// a boss, lengthened by padding its top face, repeated twice along x.
#[test]
fn a_pattern_repeats_a_face_swept_off_the_solid() {
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
        extrude_on(
            plane_at_z(3.0),
            vec![rect_wire(7.0, 7.0, 13.0, 13.0)],
            ExtrudeTermination::Blind { distance: 5.0 },
            false,
            BooleanOp::Fuse,
        ),
        SolidOp::SweepFace {
            face: probe([10.0, 10.0, 8.0], [0.0, 0.0, 1.0]),
            kind: SweepKind::Extrude {
                termination: ExtrudeTermination::Blind { distance: 2.0 },
                second_side: None,
                symmetric: false,
                reversed: false,
                taper_deg: 0.0,
                direction: None,
            },
            op: BooleanOp::Fuse,
        },
        SolidOp::Transform {
            transforms: vec![translate(15.0), translate(30.0)],
            originals: vec![1, 2],
        },
    ];
    // The plate, and three bosses 6 × 6 × 7.
    assert_volume(&ops, 3000.0 + 3.0 * 36.0 * 7.0, "three whole bosses");
}

/// A thread groove cuts into a blind bore made by the cylinder primitive as
/// it does into the same bore extruded.
#[test]
fn a_thread_cuts_into_a_primitive_bore_as_into_an_extruded_one() {
    use kernel_api::{Placement, PrimitiveKind};
    let pitch = 1.0;
    let (inner, major, crest, root, start) = (2.4, 3.0, pitch / 16.0, 0.4, -pitch);
    let corners = [
        [inner, start - root],
        [major, start - crest],
        [major, start + crest],
        [inner, start + root],
    ];
    let groove = SolidOp::Sweep {
        profile: Profile {
            plane: ProfilePlane {
                origin: [10.0, 10.0, 10.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.0, -1.0],
                normal: [0.0, 1.0, 0.0],
            },
            wires: vec![ProfileWire {
                names: Vec::new(),
                segments: (0..4)
                    .map(|i| ProfileSegment::Line {
                        start: corners[i],
                        end: corners[(i + 1) % 4],
                    })
                    .collect(),
            }],
        },
        kind: SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch,
            height: 2.0 * pitch,
            left_handed: false,
            cone_angle_deg: 0.0,
            reversed: false,
            turns: None,
            growth: None,
        },
        op: BooleanOp::Cut,
    };
    let block = SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 20.0,
            width: 20.0,
            height: 10.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    };
    let primitive_bore = SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 2.5,
            height: 8.0,
            angle_deg: 360.0,
        },
        placement: Placement {
            origin: [10.0, 10.0, 2.0],
            ..Placement::default()
        },
        op: BooleanOp::Cut,
    };
    let extruded_bore = SolidOp::Sweep {
        profile: Profile {
            plane: plane_at_z(2.0),
            wires: vec![circle_wire(10.0, 10.0, 2.5)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 8.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::Cut,
    };
    let want = volume_of(&[block.clone(), extruded_bore, groove.clone()])
        .expect("the extruded bore threads");
    let got = volume_of(&[block, primitive_bore, groove]).expect("the primitive's bore threads");
    assert!((got - want).abs() < want * 1e-4, "{got} against {want}");
}

/// A solid's volume and bounds.
type Measured = (f64, ([f32; 3], [f32; 3]));

/// Build `kind` at the origin: its volume and bounds.
fn primitive_volume(kind: PrimitiveKind) -> Result<Measured, String> {
    let ops = [SolidOp::Primitive {
        kind,
        placement: Placement {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            z_axis: [0.0, 0.0, 1.0],
        },
        op: BooleanOp::NewSolid,
    }];
    let mut kernel = OgeomKernel::new();
    let built = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .map_err(|e| e.to_string())?;
    let volume = kernel
        .physical_properties(&built.brep_blob)
        .unwrap()
        .volume_mm3
        .unwrap();
    Ok((volume, built.bounds_mm.unwrap()))
}

/// An ellipsoid cut as a sphere is keeps the upper half of itself, and a
/// quarter turn of that a quarter of it.
#[test]
fn an_ellipsoid_takes_a_spheres_cut() {
    let whole = 4.0 / 3.0 * std::f64::consts::PI * 8.0 * 5.0 * 3.0;
    let cut = |sweep: f64| {
        primitive_volume(PrimitiveKind::Ellipsoid {
            radius1: 8.0,
            radius2: 5.0,
            radius3: 3.0,
            angle1_deg: 0.0,
            angle2_deg: 90.0,
            angle3_deg: sweep,
        })
        .unwrap()
    };
    let (half, (lo, hi)) = cut(360.0);
    assert!(
        (half - whole / 2.0).abs() < 1e-3 * whole,
        "{half} vs {whole}"
    );
    assert!(
        lo[2].abs() < 1e-3 && (hi[2] - 3.0).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
    let (quarter, _) = cut(90.0);
    assert!((quarter - whole / 8.0).abs() < 1e-3 * whole, "{quarter}");
}

/// A whole ellipsoid is the sphere scaled to its three radii.
#[test]
fn a_whole_ellipsoid_builds() {
    let (whole, _) = primitive_volume(PrimitiveKind::Ellipsoid {
        radius1: 2.0,
        radius2: 1.0,
        radius3: 1.0,
        angle1_deg: -90.0,
        angle2_deg: 90.0,
        angle3_deg: 360.0,
    })
    .unwrap();
    assert!((whole - 4.0 / 3.0 * std::f64::consts::PI * 2.0).abs() < 1e-2);
}

/// A skewed prism leans without losing volume.
#[test]
fn a_skewed_prism_leans() {
    let prism = |skew: f64| {
        primitive_volume(PrimitiveKind::Prism {
            sides: 6,
            circumradius: 5.0,
            height: 7.0,
            skew_x_deg: skew,
            skew_y_deg: 0.0,
        })
        .unwrap()
    };
    let (upright, _) = prism(0.0);
    let (leaning, (lo, hi)) = prism(30.0);
    assert!((leaning - upright).abs() < 1e-6 * upright);
    let shift = 7.0 * 30f32.to_radians().tan();
    assert!(
        (hi[0] - lo[0] - (10.0 + shift)).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
}

/// A workbench measuring a solid gets its volume and centre, in its own
/// frame.
#[test]
fn a_bench_measures_a_box() {
    use kernel_api::KernelQueries;
    let mut kernel = new_kernel();
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 20.0,
            height: 30.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = kernel
        .execute_solid_chain(&cube, &TessellationSettings::default())
        .expect("a box builds");
    let measured = kernel_ogeom::QUERIES.measure(&solid.brep_blob).unwrap();
    let volume = measured.volume_mm3.expect("a closed solid");
    assert!((volume - 6000.0).abs() < 1e-6, "{volume}");
    let c = measured.centre_mm;
    assert!(
        (c[0] - 5.0).abs() < 1e-6 && (c[1] - 10.0).abs() < 1e-6 && (c[2] - 15.0).abs() < 1e-6,
        "{c:?}"
    );
}

/// Two boxes 7 mm apart come 7 mm near, between their facing walls; two
/// that overlap come to nothing.
#[test]
fn two_solids_are_as_near_as_their_facing_walls() {
    use kernel_api::KernelQueries;
    let mut kernel = new_kernel();
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = kernel
        .execute_solid_chain(&cube, &TessellationSettings::default())
        .expect("a box builds");
    let shifted = |x: f64| {
        [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };
    let blob = &solid.brep_blob;
    let gap = kernel_ogeom::QUERIES
        .gap(blob, blob, &shifted(17.0))
        .unwrap();
    assert!((gap.distance_mm - 7.0).abs() < 1e-6, "{gap:?}");
    assert!((gap.on_a[0] - 10.0).abs() < 1e-6 && (gap.on_b[0] - 17.0).abs() < 1e-6);
    let crossing = kernel_ogeom::QUERIES
        .gap(blob, blob, &shifted(5.0))
        .unwrap();
    assert!(crossing.distance_mm.abs() < 1e-6, "{crossing:?}");
}

/// A solid mirrored across a plane keeps its volume and lands on the other
/// side of it.
#[test]
fn a_solid_mirrors_across_a_plane() {
    use kernel_api::KernelQueries;
    let mut kernel = new_kernel();
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 20.0,
            height: 30.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = kernel
        .execute_solid_chain(&cube, &TessellationSettings::default())
        .expect("a box builds");
    let mirrored = kernel_ogeom::QUERIES
        .mirror(&solid.brep_blob, [20.0, 0.0, 0.0], [1.0, 0.0, 0.0])
        .unwrap();
    let measured = kernel_ogeom::QUERIES.measure(&mirrored).unwrap();
    assert!(
        (measured.volume_mm3.unwrap() - 6000.0).abs() < 1e-6,
        "{measured:?}"
    );
    assert!((measured.centre_mm[0] - 35.0).abs() < 1e-6, "{measured:?}");
}

fn yz_plane() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0; 3],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    }
}

fn revolve_on(plane: ProfilePlane, wire: ProfileWire, angle_deg: f64, op: BooleanOp) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane,
            wires: vec![wire],
        },
        kind: SweepKind::Revolve {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            angle_deg,
            second_angle_deg: None,
            midplane: false,
            reversed: false,
            termination: Default::default(),
        },
        op,
    }
}

/// A collar turned a whole turn, or just short of one, over the rim of a
/// whole turned cylinder, both sketched on the plane holding their axis,
/// fuses and cuts.
#[test]
fn a_whole_collar_fuses_with_and_cuts_a_turned_cylinder() {
    for plane in [yz_plane(), xz_plane()] {
        for op in [BooleanOp::Fuse, BooleanOp::Cut] {
            for angle in [360.0, 359.0] {
                let mut kernel = new_kernel();
                let result = kernel.execute_solid_chain(
                    &[
                        revolve_on(
                            plane,
                            rect_wire(0.0, 0.0, 10.0, 20.0),
                            360.0,
                            BooleanOp::NewSolid,
                        ),
                        revolve_on(plane, rect_wire(8.0, 18.0, 14.0, 24.0), angle, op),
                    ],
                    &TessellationSettings::default(),
                );
                assert!(
                    result.is_ok(),
                    "{:?} {op:?} {angle}: {:?}",
                    plane.normal,
                    result.err()
                );
            }
        }
    }
}

/// A loft from a square to a circle, smooth or ruled, builds: the ruled
/// one through the skinned loft, which through two sections is the ruled
/// surface.
#[test]
fn a_loft_from_a_square_to_a_circle_builds() {
    for ruled in [false, true] {
        let mut kernel = new_kernel();
        let result = kernel.execute_solid_chain(
            &[SolidOp::Loft {
                sections: vec![
                    Profile {
                        plane: xy_plane(),
                        wires: vec![rect_wire(-10.0, -10.0, 10.0, 10.0)],
                    },
                    Profile {
                        plane: plane_at_z(20.0),
                        wires: vec![circle_wire(0.0, 0.0, 8.0)],
                    },
                ],
                ruled,
                closed: false,
                op: BooleanOp::NewSolid,
            }],
            &TessellationSettings::default(),
        );
        assert!(result.is_ok(), "ruled {ruled}: {:?}", result.err());
    }
}

fn cylinder(radius: f64, height: f64) -> SolidOp {
    blind_pad(
        vec![circle_wire(0.0, 0.0, radius)],
        height,
        BooleanOp::NewSolid,
    )
}

/// Every edge of a cylinder, or the edges of its top face, bevel: the
/// seam, where the side meets itself, is no edge to bevel and is left out.
#[test]
fn a_cylinder_bevels_every_edge_and_its_top_face_s_leaving_the_seam() {
    use kernel_api::{ChamferSpec, EdgeSelection, FaceProbe};
    let whole = std::f64::consts::PI * 100.0 * 20.0;
    for edges in [
        EdgeSelection::All,
        EdgeSelection::OfPickedFaces(vec![FaceProbe {
            point: [0.0, 0.0, 20.0],
            normal: [0.0, 0.0, 1.0],
            name: 0,
        }]),
    ] {
        let ops = [
            cylinder(10.0, 20.0),
            SolidOp::Chamfer {
                spec: ChamferSpec::EqualDistance { distance: 1.0 },
                flip: false,
                edges: edges.clone(),
                follow_tangent: false,
            },
        ];
        let got = volume_of(&ops).unwrap_or_else(|e| panic!("{edges:?}: {e}"));
        assert!(got < whole - 1.0, "{edges:?}: {got}");
    }
}

/// A worm (a revolved core with a helical groove and a bore) and a 608
/// ball bearing, as the hardware package makes them.
fn worm_and_bearing() -> (Vec<u8>, Vec<u8>) {
    let text = include_str!("../data/worm_and_608.json");
    let parts: std::collections::HashMap<String, Vec<SolidOp>> =
        serde_json::from_str(text).expect("the fixture reads");
    let build = |name: &str| {
        new_kernel()
            .execute_solid_chain(&parts[name], &TessellationSettings::default())
            .unwrap_or_else(|e| panic!("{name} builds: {e}"))
            .brep_blob
    };
    (build("worm"), build("bearing_608"))
}

/// A worm passing through a bearing's bore shares the ring between the
/// bore and the worm's outside with it: the kernel answers with that
/// solid rather than failing.
#[test]
fn a_worm_through_a_bearing_shares_material_with_it() {
    use kernel_api::KernelQueries;
    let (worm, bearing) = worm_and_bearing();
    let at = |z: f64| {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, z],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };
    for z in [0.0, 20.0] {
        let shared = kernel_ogeom::QUERIES
            .overlap(&worm, &bearing, &at(z))
            .unwrap_or_else(|e| panic!("at z = {z}: {e}"))
            .expect("they share material");
        // At most the ring between the bore (4 mm) and the worm's
        // outside (8 mm), 7 mm tall.
        let ring = std::f64::consts::PI * (64.0 - 16.0) * 7.0;
        assert!(
            shared.volume_mm3 > 0.0 && shared.volume_mm3 < ring,
            "at z = {z}: {}",
            shared.volume_mm3
        );
    }
}
