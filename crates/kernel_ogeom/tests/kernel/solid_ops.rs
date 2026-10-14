//! Tests for the solid ops: termination modes, taper, loft, pipe, helix,
//! primitives, dress-ups, patterns, and body booleans.

use kernel_api::{
    BoolKind, BooleanOp, ChamferSpec, EdgeSelection, ExtrudeTermination, FaceProbe, Placement,
    PrimitiveKind, Profile, ProfilePlane, ProfileSegment, ProfileWire, RevolveTermination, SolidOp,
    SweepKind, TessellationSettings,
};

use super::common::{
    assert_close, chain, circle_wire, new_kernel, plane_at_z, primitive, rect_wire, translate,
    try_chain, volume, xy_plane,
};

/// A `length` × `width` × `height` box primitive at the origin.
fn box_of(length: f64, width: f64, height: f64) -> SolidOp {
    primitive(PrimitiveKind::Box {
        length,
        width,
        height,
    })
}

/// The 20 × 20 × 10 block most chains start from.
fn block() -> SolidOp {
    blind_pad(
        vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
        10.0,
        BooleanOp::NewSolid,
    )
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
    extrude_on(xy_plane(), wires, termination, reversed, op)
}

#[test]
fn ellipse_and_bspline_profiles_pad() {
    let ellipse = ProfileWire {
        names: Vec::new(),
        segments: vec![ProfileSegment::Ellipse {
            center: [0.0, 0.0],
            major: [8.0, 0.0],
            ratio: 0.5,
        }],
    };
    let result = chain(&[blind_pad(vec![ellipse], 4.0, BooleanOp::NewSolid)]);
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
    let result = chain(&[blind_pad(vec![spline], 4.0, BooleanOp::NewSolid)]);
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
    let result = chain(&[blind_pad(vec![quarter], 2.0, BooleanOp::NewSolid)]);
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
    let result = chain(&[blind_pad(vec![open, ring], 3.0, BooleanOp::NewSolid)]);
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "the open spline's start");
    assert!(max[0] > 35.0 && max[0] <= 40.001, "the ring: {max:?}");
    assert_close(max[2], 3.0, 1e-3, "pad height");
}

#[test]
fn primitives_build_with_expected_bounds() {
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
        let ops = [primitive(kind)];
        let result = try_chain(&ops).unwrap_or_else(|e| panic!("primitive {label}: {e}"));
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

/// A picked point names the edge beside it; one in the middle of a face
/// names none, and the fillet says so rather than rounding whichever edge
/// happens to be nearest.
#[test]
fn an_edge_pick_far_from_every_edge_fails() {
    let fillet_at = |point: [f64; 3]| {
        try_chain(&[
            block(),
            SolidOp::Fillet {
                radius: 2.0,
                edges: EdgeSelection::Near(vec![point]),
                follow_tangent: false,
            },
        ])
    };
    fillet_at([10.0, 0.5, 10.0]).expect("a pick beside the edge rounds it");
    let err = fillet_at([10.0, 10.0, 10.0]).expect_err("the face's middle names no edge");
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
    let fillet_at = |point: [f64; 3], direction: [f64; 3]| {
        let result = try_chain(&[
            block(),
            SolidOp::Fillet {
                radius: 2.0,
                edges: EdgeSelection::Picked(vec![EdgeProbe {
                    faces: [0, 0],
                    point,
                    direction,
                }]),
                follow_tangent: false,
            },
        ])?;
        let volume = volume(&result.brep_blob);
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
fn draft_tilts_side_faces() {
    // Draft one side face about the bottom neutral plane. A positive angle
    // leans the face inward (mold-release style), which leaves the bounds
    // unchanged (the base edge stays put), so lean it OUTWARD: the top edge
    // then extends past x=20 by tan(angle) * height.
    let result = chain(&[
        block(),
        SolidOp::Draft {
            face_names: Vec::new(),
            angle_deg: -10.0,
            neutral_point: [0.0, 0.0, 0.0],
            neutral_normal: [0.0, 0.0, 1.0],
            pull_dir: None,
            faces: vec![[20.0, 10.0, 5.0]],
        },
    ]);
    let (min, max) = result.bounds_mm.expect("bounds");
    let lean = (10.0_f32.to_radians().tan()) * 10.0;
    assert_close(max[0], 20.0 + lean, 0.1, "drafted top edge leans outward");
    assert_close(min[0], 0.0, 1e-3, "opposite face untouched");
}

/// Copies without their original: the whole solid gives way to its
/// copies, and a feature named an original leaves the solid as it was.
#[test]
fn transform_copies_leaves_the_original_out() {
    let block = blind_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
        5.0,
        BooleanOp::NewSolid,
    );
    let moved = chain(&[
        block.clone(),
        SolidOp::TransformCopies {
            transforms: vec![translate(20.0, 0.0, 0.0), translate(40.0, 0.0, 0.0)],
            originals: vec![],
        },
    ]);
    let (min, max) = moved.bounds_mm.expect("bounds");
    assert_close(min[0], 20.0, 1e-3, "the block itself gone");
    assert_close(max[0], 50.0, 1e-3, "the last copy");

    let refused = try_chain(&[
        block.clone(),
        SolidOp::TransformCopies {
            transforms: vec![],
            originals: vec![],
        },
    ]);
    assert!(refused.is_err(), "nothing left");

    let boss = blind_pad(vec![circle_wire(5.0, 5.0, 2.0)], 10.0, BooleanOp::Fuse);
    let copied = chain(&[
        blind_pad(
            vec![rect_wire(0.0, 0.0, 10.0, 30.0)],
            5.0,
            BooleanOp::NewSolid,
        ),
        boss,
        SolidOp::TransformCopies {
            transforms: vec![translate(20.0, 0.0, 0.0)],
            originals: vec![1],
        },
    ]);
    let (min, max) = copied.bounds_mm.expect("bounds");
    assert_close(min[0], 0.0, 1e-3, "the plate");
    assert_close(max[0], 27.0, 1e-3, "the copy past the plate");
    let volume = volume(&copied.brep_blob);
    let want = 10.0 * 30.0 * 5.0 + std::f64::consts::PI * 4.0 * 10.0;
    assert!(
        (volume - want).abs() < 0.5,
        "one boss: {volume} against {want}"
    );
}

/// A body boolean's common keeps only what the tool body shares with the
/// block: the cylinder's footprint, the block's height.
#[test]
fn a_body_boolean_keeps_what_the_tool_shares() {
    let tool = chain(&[blind_pad(
        vec![circle_wire(10.0, 10.0, 4.0)],
        20.0,
        BooleanOp::NewSolid,
    )]);
    let common = chain(&[
        block(),
        SolidOp::Boolean {
            tool_brep: tool.brep_blob,
            kind: BoolKind::Common,
            tool_transform: None,
        },
    ]);
    let want = std::f64::consts::PI * 16.0 * 10.0;
    let got = volume(&common.brep_blob);
    assert!((got - want).abs() < 1e-3, "{got} against {want}");
}

/// A dress-up the box cannot take fails, and the failure names the op: a
/// fillet far larger than the box, and a fillet or a chamfer wider than a
/// face it runs along, which would take the whole face and more.
#[test]
fn a_dress_up_too_big_for_the_box_is_refused_at_its_op() {
    let near_top_edge = || EdgeSelection::Near(vec![[10.0, 0.0, 10.0]]);
    for (what, dress_up) in [
        (
            "a 500 mm fillet",
            SolidOp::Fillet {
                radius: 500.0,
                edges: EdgeSelection::All,
                follow_tangent: false,
            },
        ),
        (
            "a 12 mm fillet on a 10 mm face",
            SolidOp::Fillet {
                radius: 12.0,
                edges: near_top_edge(),
                follow_tangent: false,
            },
        ),
        (
            "a 12 mm chamfer on a 10 mm face",
            SolidOp::Chamfer {
                spec: ChamferSpec::EqualDistance { distance: 12.0 },
                flip: false,
                edges: near_top_edge(),
                follow_tangent: false,
            },
        ),
    ] {
        let err = try_chain(&[block(), dress_up]).expect_err(what);
        assert_eq!(
            err.op_index, 1,
            "{what}: failure attributed to the dress-up"
        );
    }
}

#[test]
fn a_chain_can_start_from_another_chains_snapshot_and_only_start_there() {
    let pad = blind_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
        4.0,
        BooleanOp::NewSolid,
    );
    let built = chain(std::slice::from_ref(&pad));

    let cloned = chain(&[SolidOp::Shape {
        brep: built.brep_blob.clone(),
    }]);
    assert_eq!(cloned.bounds_mm, built.bounds_mm);

    let later = try_chain(&[
        pad,
        SolidOp::Shape {
            brep: built.brep_blob,
        },
    ]);
    assert!(later.is_err(), "a snapshot can only begin a chain");
}

#[test]
fn a_box_outline_names_its_twelve_edges() {
    let pad = blind_pad(
        vec![rect_wire(0.0, 0.0, 10.0, 5.0)],
        4.0,
        BooleanOp::NewSolid,
    );
    let built = chain(std::slice::from_ref(&pad));
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

/// The edges of the face nearest a point rounded: four rounds meeting
/// pairwise at the face's corners mitre, the solid keeping its six faces,
/// trimmed, plus one band per edge and nothing else, and its height.
#[test]
fn fillets_of_one_face_meet_mitred_at_its_corners() {
    let result = chain(&[
        block(),
        SolidOp::Fillet {
            radius: 2.0,
            edges: EdgeSelection::OfFaces(vec![[10.0, 10.0, 10.0]]),
            follow_tangent: false,
        },
    ]);
    assert_eq!(
        face_count(&result.mesh),
        10,
        "six trimmed faces and four bands, no caps at the corners"
    );
    let (min, max) = result.bounds_mm.expect("bounds");
    assert_close(max[2], 10.0, 1e-3, "height preserved");
    assert_close(min[2], 0.0, 1e-3, "bottom untouched");
}

/// Four bevels meeting pairwise at the corners of a face mitre the same
/// way: ten faces, and the volume of the box less four wedges, each corner
/// overlap counted once.
#[test]
fn chamfers_of_one_face_meet_mitred_at_its_corners() {
    let result = chain(&[
        block(),
        SolidOp::Chamfer {
            spec: ChamferSpec::EqualDistance { distance: 1.0 },
            flip: false,
            edges: EdgeSelection::OfFaces(vec![[10.0, 10.0, 10.0]]),
            follow_tangent: false,
        },
    ]);
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
                block(),
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
        vec![block(), upper]
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
    let refined = chain(&refined_ops);
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
    let split = chain(&left);
    let split_faces = face_count(&split.mesh);
    assert!(split_faces > 6);
    // Far to the side, a block on a block, refined.
    let mut ops = left.to_vec();
    ops.extend([
        pad_at(100.0, 0.0, BooleanOp::Fuse),
        pad_at(100.0, 10.0, BooleanOp::Fuse),
        SolidOp::Refine,
    ]);
    let built = chain(&ops);
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
    let ops = [primitive(PrimitiveKind::Cylinder {
        radius: 5.0,
        height: 12.0,
        angle_deg: 360.0,
    })];
    let result = chain(&ops);
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
    let ops = [primitive(PrimitiveKind::Cylinder {
        radius: 5.0,
        height: 12.0,
        angle_deg: 360.0,
    })];
    let solid = chain(&ops);
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
    let solid = chain(&[blind_pad(vec![wire], 3.0, BooleanOp::NewSolid)]);
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
fn a_cone_cut_by_a_plane_projects_as_its_conic() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    let cone = primitive(PrimitiveKind::Cone {
        radius1: 10.0,
        radius2: 0.0,
        height: 10.0,
        angle_deg: 360.0,
    });
    let near = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-6;

    // Everything past x = 3 cut away: on that plane (y, z) the rim is
    // (10 - z)² - y² = 9, the branch about (0, 10) opening down.
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
    let solid = chain(&[cone.clone(), past_x]);
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
    // The branch runs from base to base, or stops at its vertex where the
    // cone's seam splits it in two.
    let ends = [at(range.0), at(range.1)];
    let half = 91f64.sqrt();
    let on_base = |e: &[f64; 2]| e[1].abs() < 1e-6 && (e[0].abs() - half).abs() < 1e-6;
    let at_vertex = |e: &[f64; 2]| near(*e, [0.0, 7.0]);
    assert!(
        ends.iter().all(|e| on_base(e) || at_vertex(e)) && ends.iter().any(on_base),
        "{ends:?}"
    );

    // Everything past x + z = 5 cut away: the plane runs parallel to the
    // ruling on the -x side, and the rim on it is a parabola whose vertex
    // stands at (-2.5, 0, 7.5) and whose ends sit on the base circle.
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
    let solid = chain(&[cone, past_slant]);
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

/// An edge on an offset of a line or a circle, as a file may state one,
/// projects as the segment or arc it is: a sketch takes it as its own line
/// or arc, ends where the edge's are.
#[test]
fn an_offset_edge_projects_as_a_line_or_an_arc() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    use ogeom::core::Tolerances;
    use ogeom::geom::{CircleCurve, Curve, Curve3d as _, LineCurve, OffsetCurve};
    use ogeom::math::{Axis, Circle, Direction, Frame, Point, Vector};
    let tol = Tolerances::millimetres();
    let centre = Frame::new(Point::new(1.0, 2.0, 5.0), Direction::Z, Direction::X, tol).unwrap();
    let line = Axis {
        location: Point::new(1.0, 2.0, 3.0),
        direction: Direction::new(Vector::new(1.0, 1.0, 0.0), tol).unwrap(),
    };
    let offset = |basis: Curve| {
        Curve::Offset(Box::new(
            OffsetCurve::new(basis, 2.0, Direction::Z).unwrap(),
        ))
    };
    let rim = offset(Curve::Circle(CircleCurve::new(
        Circle::new(centre, 4.0, tol).unwrap(),
    )));
    let wall = offset(Curve::Line(LineCurve::new(line)));
    for (curve, range) in [(rim, (0.5, 2.0)), (wall, (-4.0, 6.0))] {
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
        let at = |t: f64| {
            let p = curve.point_at(t, tol).unwrap();
            [p.x, p.y]
        };
        let mid = curve.point_at((range.0 + range.1) / 2.0, tol).unwrap();
        let projected = kernel_ogeom::QUERIES
            .project_edge(&blob, [mid.x, mid.y, mid.z], &xy_plane())
            .expect("the offset edge projects");
        let ends = match projected {
            ProjectedEdge::Line { start, end } => [start, end],
            ProjectedEdge::Circle {
                centre,
                radius,
                range: (a, b),
            } => {
                assert!((centre[0] - 1.0).abs() < 1e-9 && (centre[1] - 2.0).abs() < 1e-9);
                assert!((radius - 6.0).abs() < 1e-9, "{radius}");
                [a, b].map(|a| [centre[0] + radius * a.cos(), centre[1] + radius * a.sin()])
            }
            other => panic!("{other:?}"),
        };
        for (got, want) in ends.iter().zip([at(range.0), at(range.1)]) {
            assert!(
                (got[0] - want[0]).hypot(got[1] - want[1]) < 1e-9,
                "{got:?} vs {want:?}"
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
    let cube = chain(&[box_of(10.0, 10.0, 10.0)]);
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

    let rod = chain(&[primitive(PrimitiveKind::Cylinder {
        radius: 5.0,
        height: 12.0,
        angle_deg: 360.0,
    })]);
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

/// A 10 mm cube with a 4 mm bore down its middle, along Z.
fn bored_cube() -> kernel_api::SolidBuildResult {
    let ops = [
        box_of(10.0, 10.0, 10.0),
        SolidOp::Primitive {
            kind: PrimitiveKind::Cylinder {
                radius: 2.0,
                height: 12.0,
                angle_deg: 360.0,
            },
            placement: Placement {
                origin: [5.0, 5.0, -1.0],
                ..Placement::default()
            },
            op: BooleanOp::Cut,
        },
    ];
    chain(&ops)
}

/// The length of a polyline.
fn run_length(points: &[[f64; 3]]) -> f64 {
    points
        .windows(2)
        .map(|w| {
            (0..3)
                .map(|i| (w[1][i] - w[0][i]).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .sum()
}

fn same_point(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6)
}

/// A plane across a bored cube cuts it into two closed loops, its square
/// outside and the bore's circle, each curve running on from the last;
/// asked of one face, only that face's line comes back, open; a plane
/// that misses cuts nothing.
#[test]
fn a_shape_cut_by_a_plane_gives_its_section_as_loops() {
    use kernel_api::{KernelQueries, ProjectedEdge};
    let cube = bored_cube();
    let runs = kernel_ogeom::QUERIES
        .section(&cube.brep_blob, &plane_at_z(4.0), None)
        .expect("the cube is cut");
    assert_eq!(runs.len(), 2, "{runs:?}");
    let mut lengths: Vec<f64> = runs
        .iter()
        .map(|run| {
            assert!(run.closed, "{run:?}");
            for pair in run.curves.windows(2) {
                assert!(same_point(
                    *pair[0].points.last().unwrap(),
                    pair[1].points[0]
                ));
            }
            for curve in &run.curves {
                assert!(curve.points.iter().all(|p| (p[2] - 4.0).abs() < 1e-7));
            }
            run.curves.iter().map(|c| run_length(&c.points)).sum()
        })
        .collect();
    lengths.sort_by(f64::total_cmp);
    assert!(
        (lengths[0] - 4.0 * std::f64::consts::PI).abs() < 0.1,
        "{lengths:?}"
    );
    assert!((lengths[1] - 40.0).abs() < 1e-6, "{lengths:?}");
    let bore = runs
        .iter()
        .find(|run| {
            run.curves
                .iter()
                .all(|c| matches!(c.curve, ProjectedEdge::Circle { .. }))
        })
        .expect("the bore's run is circles");
    let ProjectedEdge::Circle { centre, radius, .. } = bore.curves[0].curve else {
        unreachable!()
    };
    assert!((centre[0] - 5.0).abs() < 1e-7 && (centre[1] - 5.0).abs() < 1e-7);
    assert!((radius - 2.0).abs() < 1e-7);

    let side = kernel_ogeom::QUERIES
        .section(&cube.brep_blob, &plane_at_z(4.0), Some([0.0, 5.0, 5.0]))
        .expect("the side is cut");
    let [run] = side.as_slice() else {
        panic!("{side:?}");
    };
    assert!(!run.closed && run.curves.len() == 1, "{run:?}");
    assert!(matches!(run.curves[0].curve, ProjectedEdge::Line { .. }));

    let missed = kernel_ogeom::QUERIES
        .section(&cube.brep_blob, &plane_at_z(40.0), None)
        .expect("a plane that misses cuts nothing");
    assert!(missed.is_empty(), "{missed:?}");
}

/// A bored cube's top has two loops, its square first and then the bore's
/// rim, each edge running on from the last; the bore's own face has its
/// seam twice, once each way.
#[test]
fn a_face_s_loops_come_outer_first_each_edge_in_turn() {
    use kernel_api::KernelQueries;
    let cube = bored_cube();
    let top = kernel_ogeom::QUERIES
        .face_loops(&cube.brep_blob, [1.0, 1.0, 10.0])
        .expect("the top's loops");
    assert_eq!(top.len(), 2, "{top:?}");
    let length = |edges: &[kernel_api::LoopEdge]| -> f64 {
        edges.iter().map(|e| run_length(&e.points)).sum()
    };
    assert_eq!(top[0].len(), 4);
    assert!((length(&top[0]) - 40.0).abs() < 1e-6);
    assert!((length(&top[1]) - 4.0 * std::f64::consts::PI).abs() < 0.1);
    for edges in &top {
        assert!(edges.iter().all(|e| !e.seam));
        for (a, b) in edges.iter().zip(edges.iter().cycle().skip(1)) {
            assert!(
                same_point(*a.points.last().unwrap(), b.points[0]),
                "{edges:?}"
            );
            assert!(a.points.iter().all(|p| (p[2] - 10.0).abs() < 1e-7));
        }
    }

    let bore = kernel_ogeom::QUERIES
        .face_loops(&cube.brep_blob, [7.0, 5.0, 5.0])
        .expect("the bore's loops");
    let seams = bore.iter().flatten().filter(|e| e.seam).count();
    assert_eq!(seams, 2, "{bore:?}");
}

/// Two boxes where they overlap share a solid of the overlap's volume, in
/// the first box's frame; flush or apart, they share none.
#[test]
fn two_solids_overlap_by_the_volume_they_share() {
    use kernel_api::KernelQueries;
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = chain(&cube);
    let blob = &solid.brep_blob;
    let shared = kernel_ogeom::QUERIES
        .overlap(blob, blob, &translate(6.0, 5.0, 0.0))
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
    for apart in [translate(10.0, 0.0, 0.0), translate(30.0, 0.0, 0.0)] {
        let none = kernel_ogeom::QUERIES.overlap(blob, blob, &apart).unwrap();
        assert!(none.is_none(), "{apart:?}");
    }
}

/// A pad run from a sketch beside a cylinder toward its side stops on the
/// curved surface itself, not on a plane through where it is first met:
/// to the first face, up to the picked face, and up to it with an offset.
#[test]
fn a_pad_stops_exactly_on_a_curved_face() {
    let rod = primitive(PrimitiveKind::Cylinder {
        radius: 10.0,
        height: 20.0,
        angle_deg: 360.0,
    });
    // A 4 × 10 rectangle on the plane x = 30, facing +X, padded back
    // toward the rod (reversed).
    let beside = ProfilePlane {
        origin: [30.0, 0.0, 0.0],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    };
    let pad = |termination: ExtrudeTermination| {
        extrude_on(
            beside,
            vec![rect_wire(-2.0, 5.0, 2.0, 15.0)],
            termination,
            true,
            BooleanOp::Fuse,
        )
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
        let solid = try_chain(&[rod.clone(), pad(termination)])
            .unwrap_or_else(|e| panic!("{name}: builds: {e}"));
        let volume = volume(&solid.brep_blob);
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
    // A 20 × 20 × 10 block with a 10 × 10 step, 5 high, on top.
    let block = block();
    let step = extrude_on(
        plane_at_z(10.0),
        vec![rect_wire(0.0, 0.0, 10.0, 10.0)],
        ExtrudeTermination::Blind { distance: 5.0 },
        false,
        BooleanOp::Fuse,
    );
    // A pad from z = 20 down beside the step, up to the block's top face
    // (picked at z = 10), 2 mm off it.
    let pad = extrude_on(
        plane_at_z(20.0),
        vec![rect_wire(12.0, 12.0, 18.0, 18.0)],
        ExtrudeTermination::UpToFace {
            name: 0,
            point: [15.0, 15.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            offset: 2.0,
        },
        true,
        BooleanOp::Fuse,
    );
    let solid = chain(&[block, step, pad]);
    let volume = volume(&solid.brep_blob);
    // Block, step, and a 6 × 6 pad from z = 12 to z = 20.
    let want = 4000.0 + 500.0 + 36.0 * 8.0;
    assert!((volume - want).abs() < 1e-3, "volume {volume}, want {want}");
}

/// The kernel's volume of the solid `ops` build.
fn volume_of(ops: &[SolidOp]) -> Result<f64, String> {
    try_chain(ops)
        .map(|built| volume(&built.brep_blob))
        .map_err(|e| e.to_string())
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

/// Smooth lofts between equal sections and pipes measure what Pappus
/// says: a square or a circle lofted, a square piped up a line either way
/// round, a circle piped into a cylinder, and a square along a quarter arc
/// square to its start (its area times the length its centroid runs). A
/// conical helix builds.
#[test]
fn lofts_and_pipes_measure_what_their_sections_sweep() {
    use std::f64::consts::{FRAC_PI_2, PI};
    let square = rect_wire(-2.0, -2.0, 2.0, 2.0);
    let rise = || {
        vec![ProfileSegment::Line {
            start: [0.0, 0.0],
            end: [0.0, 20.0],
        }]
    };
    // A quarter circle rising from the origin, its centre 10 mm along X.
    let arc = vec![ProfileSegment::Arc {
        start: [0.0, 0.0],
        mid: [10.0 - 50f64.sqrt(), 50f64.sqrt()],
        end: [10.0, 10.0],
    }];
    let (wide, disc) = (rect_wire(-5.0, -5.0, 5.0, 5.0), circle_wire(0.0, 0.0, 5.0));
    let cases = [
        ("lofted squares", smooth_loft(wide.clone(), wide), 1500.0),
        (
            "lofted circles",
            smooth_loft(disc.clone(), disc),
            PI * 25.0 * 15.0,
        ),
        ("ccw pipe", pipe_along(square.clone(), rise()), 320.0),
        ("cw pipe", pipe_along(walked_back(&square), rise()), 320.0),
        (
            "cylinder",
            pipe_along(circle_wire(0.0, 0.0, 2.0), rise()),
            PI * 4.0 * 20.0,
        ),
        ("arc pipe", pipe_along(square, arc), 16.0 * 10.0 * FRAC_PI_2),
    ];
    for (what, op, want) in cases {
        assert_volume(&[op], want, what);
    }
    volume_of(&[helix_of(rect_wire(10.0, 0.0, 12.0, 2.0), 10.0, 5.0, 20.0)])
        .expect("a conical helix");
}

/// A helical sweep reaches exactly as far as its profile does and is as
/// big as Pappus says, measured at the default deflection as the app
/// measures, close-pitched or not; a clockwise profile coils as a
/// counter-clockwise one does.
#[test]
fn a_helical_sweep_reaches_its_profile_and_is_as_big_as_pappus_says() {
    let coil = rect_wire(10.0, 0.0, 12.0, 2.0);
    let turn = 4.0 * 2.0 * std::f64::consts::PI * 11.0;
    let built = chain(&[helix_of(coil.clone(), 0.0, 5.0, 10.0)]);
    let (min, max) = built.bounds_mm.unwrap();
    assert_close(max[0], 12.0, 1e-3, "outer radius");
    assert_close(min[1], 0.0, 1e-3, "starts on the profile");
    assert_close(max[1], 12.0, 1e-3, "height plus the profile");
    let ccw = volume(&built.brep_blob);
    assert!(
        (ccw - 2.0 * turn).abs() <= 2.0 * turn * 1e-4,
        "2 turns: {ccw}"
    );
    let cw = volume_of(&[helix_of(walked_back(&coil), 0.0, 5.0, 10.0)]).unwrap();
    assert!((ccw - cw).abs() < 1e-6, "{ccw} vs {cw}");
    assert_volume(&[helix_of(coil, 0.0, 3.0, 9.0)], 3.0 * turn, "3 turns");
}

/// Part turns and cut primitives measure what their angles keep: a
/// profile on its axis and a cylinder turned 270°, a sphere band turned
/// 270°; a sphere cut flat at its latitudes, a hemisphere from the equator
/// to either pole and a band between two parallels; an ellipsoid cut as a
/// sphere is, its upper half and a quarter turn of that, or whole; and a
/// hexagonal prism, upright or leaning without losing volume.
#[test]
fn part_turns_and_cut_primitives_measure_what_they_keep() {
    use kernel_api::PrimitiveKind::{Cylinder, Ellipsoid, Prism, Sphere};
    use std::f64::consts::PI;
    let prim = |kind| vec![primitive(kind)];
    let turn = |angle_deg: f64| {
        vec![SolidOp::Sweep {
            profile: Profile {
                plane: xy_plane(),
                wires: vec![rect_wire(0.0, 0.0, 5.0, 10.0)],
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
            op: BooleanOp::NewSolid,
        }]
    };
    let sphere = |angle1_deg, angle2_deg, angle3_deg| {
        prim(Sphere {
            radius: 5.0,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        })
    };
    let ellipsoid = |angle1_deg, angle3_deg| {
        prim(Ellipsoid {
            radius1: 8.0,
            radius2: 5.0,
            radius3: 3.0,
            angle1_deg,
            angle2_deg: 90.0,
            angle3_deg,
        })
    };
    let prism = |skew_x_deg| {
        prim(Prism {
            sides: 6,
            circumradius: 5.0,
            height: 7.0,
            skew_x_deg,
            skew_y_deg: 0.0,
        })
    };
    let cylinder = PI * 25.0 * 10.0;
    let hemisphere = 2.0 / 3.0 * PI * 125.0;
    // A zone of height 2h about the equator: pi (2h r^2 - (2/3) h^3).
    let h = 5.0 * 45f64.to_radians().sin();
    let band = PI * (2.0 * h * 25.0 - 2.0 / 3.0 * h.powi(3));
    let ellipsoid_whole = 4.0 / 3.0 * PI * 8.0 * 5.0 * 3.0;
    let hexagonal = 1.5 * 3f64.sqrt() * 25.0 * 7.0;
    let cases = [
        (
            "a profile on its axis turned 270°",
            turn(270.0),
            cylinder * 0.75,
            1e-4,
        ),
        (
            "a cylinder turned 270°",
            prim(Cylinder {
                radius: 5.0,
                height: 10.0,
                angle_deg: 270.0,
            }),
            cylinder * 0.75,
            1e-4,
        ),
        (
            "a sphere band turned 270°",
            sphere(-45.0, 45.0, 270.0),
            band * 0.75,
            1e-4,
        ),
        (
            "the upper hemisphere",
            sphere(0.0, 90.0, 360.0),
            hemisphere,
            1e-4,
        ),
        (
            "the lower hemisphere",
            sphere(-90.0, 0.0, 360.0),
            hemisphere,
            1e-4,
        ),
        ("the band", sphere(-45.0, 45.0, 360.0), band, 1e-4),
        (
            "an ellipsoid's upper half",
            ellipsoid(0.0, 360.0),
            ellipsoid_whole / 2.0,
            2e-3,
        ),
        (
            "a quarter turn of it",
            ellipsoid(0.0, 90.0),
            ellipsoid_whole / 8.0,
            8e-3,
        ),
        (
            "a whole ellipsoid",
            ellipsoid(-90.0, 360.0),
            ellipsoid_whole,
            1e-3,
        ),
        ("an upright prism", prism(0.0), hexagonal, 1e-4),
        ("a prism leaning 30°", prism(30.0), hexagonal, 1e-4),
    ];
    let mut measured = Vec::new();
    for (what, ops, want, tolerance) in cases {
        let built = try_chain(&ops).unwrap_or_else(|e| panic!("{what}: {e}"));
        let got = volume(&built.brep_blob);
        assert!(
            (got - want).abs() <= tolerance * want,
            "{what}: volume {got}, want {want}"
        );
        measured.push((got, built.bounds_mm.expect("bounds")));
    }
    // The ellipsoid's upper half stands on z = 0, as tall as its radius.
    let (lo, hi) = measured[6].1;
    assert!(
        lo[2].abs() < 1e-3 && (hi[2] - 3.0).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
    // Leaning, the prism keeps its volume and widens by its shift.
    let (upright, (leaning, (lo, hi))) = (measured[9].0, measured[10]);
    assert!((leaning - upright).abs() < 1e-6 * upright);
    let shift = 7.0 * 30f32.to_radians().tan();
    assert!(
        (hi[0] - lo[0] - (10.0 + shift)).abs() < 1e-3,
        "{lo:?}..{hi:?}"
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

#[test]
fn a_refined_solid_measures_exactly() {
    let ops = [
        block(),
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
    let built = chain(&ops);
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
        block(),
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
    let result = chain(&[SolidOp::Sweep {
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
    }]);
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
    let result = chain(&[
        block(),
        SolidOp::Draft {
            face_names: Vec::new(),
            angle_deg: 1.5,
            neutral_point: [10.0, 10.0, 0.0],
            neutral_normal: [0.0, 0.0, -1.0],
            pull_dir: None,
            faces: vec![[0.0, 10.0, 5.0]],
        },
    ]);
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

/// A square swept along straight legs has flat sides, which measure
/// exactly.
#[test]
fn a_square_piped_along_straight_legs_has_flat_sides() {
    let mut kernel = new_kernel();
    let result = chain(&[pipe_along(
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
    )]);
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
    let base = block();
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
    let volume = volume(&shown.brep_blob);
    assert!((volume - (4000.0 - 400.0)).abs() < 1e-6, "{volume}");

    let first = kernel
        .execute_solid_chain_previewing(&[base], &detail, Some(0..1))
        .expect("a box");
    assert!(
        first.preview.expect("a preview").shown.is_none(),
        "nothing before it"
    );
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
        block(),
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
        block(),
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
        block(),
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
            transforms: vec![translate(15.0, 0.0, 0.0), translate(30.0, 0.0, 0.0)],
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
    let block = box_of(20.0, 20.0, 10.0);
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

/// A workbench measuring a solid gets its volume, centre and inertia, in
/// its own frame: a 10 x 20 x 30 box turns most easily about its long side.
#[test]
fn a_bench_measures_a_box() {
    use kernel_api::KernelQueries;
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 20.0,
            height: 30.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = chain(&cube);
    let measured = kernel_ogeom::QUERIES.measure(&solid.brep_blob).unwrap();
    let volume = measured.volume_mm3.expect("a closed solid");
    assert!((volume - 6000.0).abs() < 1e-6, "{volume}");
    let c = measured.centre_mm;
    assert!(
        (c[0] - 5.0).abs() < 1e-6 && (c[1] - 10.0).abs() < 1e-6 && (c[2] - 15.0).abs() < 1e-6,
        "{c:?}"
    );
    let inertia = measured.inertia.expect("a solid has an inertia");
    // m (b² + c²) / 12 about each axis through the centre.
    let expected = [(250_000.0, 2), (500_000.0, 1), (650_000.0, 0)];
    for ((moment, axis), (want, along)) in inertia.principal.iter().zip(expected) {
        assert!((moment - want).abs() < 1e-3, "{inertia:?}");
        assert!((axis[along].abs() - 1.0).abs() < 1e-9, "{inertia:?}");
    }
    assert!((inertia.about_centre[0][0] - 650_000.0).abs() < 1e-3);
    assert!(inertia.about_centre[0][1].abs() < 1e-6, "{inertia:?}");
}

/// Two boxes side by side, measured between whole solids alike, have a
/// combined inertia about their shared centre.
#[test]
fn two_solids_weigh_as_one() {
    use kernel_api::KernelQueries;
    let block = |x: f64| SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        placement: Placement {
            origin: [x, 0.0, 0.0],
            ..Placement::default()
        },
        op: BooleanOp::Fuse,
    };
    let mut first = block(0.0);
    if let SolidOp::Primitive { op, .. } = &mut first {
        *op = BooleanOp::NewSolid;
    }
    let solid = chain(&[first, block(30.0)]);
    let measured = kernel_ogeom::QUERIES.measure(&solid.brep_blob).unwrap();
    assert!((measured.centre_mm[0] - 20.0).abs() < 1e-6, "{measured:?}");
    let inertia = measured.inertia.unwrap();
    // Each cube: 1000 * 200 / 12 about its own centre, plus 1000 * 15² off
    // the shared one about Y and Z.
    let own = 1000.0 * 200.0 / 12.0;
    assert!((inertia.about_centre[0][0] - 2.0 * own).abs() < 1e-3);
    assert!((inertia.about_centre[1][1] - 2.0 * (own + 225_000.0)).abs() < 1e-3);
    assert!(
        (inertia.principal[0].1[0].abs() - 1.0).abs() < 1e-9,
        "{inertia:?}"
    );
}

/// Two boxes 7 mm apart come 7 mm near, between their facing walls; two
/// that overlap come to nothing.
#[test]
fn two_solids_are_as_near_as_their_facing_walls() {
    use kernel_api::KernelQueries;
    let cube = [SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }];
    let solid = chain(&cube);
    let blob = &solid.brep_blob;
    let gap = kernel_ogeom::QUERIES
        .gap(blob, blob, &translate(17.0, 0.0, 0.0))
        .unwrap();
    assert!((gap.distance_mm - 7.0).abs() < 1e-6, "{gap:?}");
    assert!((gap.on_a[0] - 10.0).abs() < 1e-6 && (gap.on_b[0] - 17.0).abs() < 1e-6);
    let crossing = kernel_ogeom::QUERIES
        .gap(blob, blob, &translate(5.0, 0.0, 0.0))
        .unwrap();
    assert!(crossing.distance_mm.abs() < 1e-6, "{crossing:?}");
}

/// An edge taken out of a solid measures against another edge or a face
/// as a face does: the boxes' nearest edges, and an edge to the facing wall.
#[test]
fn edges_are_as_near_as_their_closest_points() {
    use kernel_api::KernelQueries;
    let cube = [box_of(10.0, 10.0, 10.0)];
    let solid = chain(&cube);
    let blob = &solid.brep_blob;
    let shifted = translate(17.0, 0.0, 0.0);
    let queries = &kernel_ogeom::QUERIES;
    // The upright edge at x = 10, y = 0 and the top edge along y at x = 0
    // of the box moved 17 along x: nearest at their shared corner height.
    let upright = queries.edge_of(blob, [10.0, 0.0, 5.0]).unwrap();
    let top = queries.edge_of(blob, [0.0, 5.0, 10.0]).unwrap();
    let gap = queries.gap(&upright, &top, &shifted).unwrap();
    assert!((gap.distance_mm - 7.0).abs() < 1e-6, "{gap:?}");
    assert!(
        (gap.on_a[2] - 10.0).abs() < 1e-6 && gap.on_a[1].abs() < 1e-6,
        "{gap:?}"
    );
    let wall = queries.face_of(blob, [0.0, 5.0, 5.0]).unwrap();
    let to_wall = queries.gap(&upright, &wall, &shifted).unwrap();
    assert!((to_wall.distance_mm - 7.0).abs() < 1e-6, "{to_wall:?}");
    assert!(
        queries.edge_of(blob, [5.0, 5.0, 60.0]).is_err(),
        "no edge there"
    );
}

/// A cone's wall knows its half angle and a torus its two radii.
#[test]
fn a_cone_and_a_torus_carry_their_sizes() {
    use kernel_api::FaceSurface;
    let built = |kind| {
        let ops = [primitive(kind)];
        chain(&ops).mesh
    };
    let cone = built(PrimitiveKind::Cone {
        radius1: 10.0,
        radius2: 5.0,
        height: 10.0,
        angle_deg: 360.0,
    });
    let half_angle = cone
        .face_surfaces
        .iter()
        .find_map(|s| match s {
            FaceSurface::Cone { half_angle, .. } => Some(*half_angle),
            _ => None,
        })
        .expect("the side is a cone");
    assert!(
        (half_angle.to_degrees() - 0.5f32.atan().to_degrees()).abs() < 1e-3,
        "{half_angle}"
    );
    let torus = built(PrimitiveKind::Torus {
        radius1: 10.0,
        radius2: 2.0,
        angle1_deg: -180.0,
        angle2_deg: 180.0,
        angle3_deg: 360.0,
    });
    let radii = torus
        .face_surfaces
        .iter()
        .find_map(|s| match s {
            FaceSurface::Torus {
                major_radius,
                minor_radius,
                ..
            } => Some((*major_radius, *minor_radius)),
            _ => None,
        })
        .expect("a torus");
    assert_eq!(radii, (10.0, 2.0));
}

/// A solid mirrored across a plane keeps its volume and lands on the other
/// side of it.
#[test]
fn a_solid_mirrors_across_a_plane() {
    use kernel_api::KernelQueries;
    let cube = [box_of(10.0, 20.0, 30.0)];
    let solid = chain(&cube);
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
/// fuses and cuts: on the base planes, and on one turned a radian about
/// the axis, where the seams lie together off every base plane.
#[test]
fn a_whole_collar_fuses_with_and_cuts_a_turned_cylinder() {
    let (sin, cos) = 1f64.sin_cos();
    let turned = ProfilePlane {
        origin: [0.0; 3],
        x_axis: [-sin, cos, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [cos, sin, 0.0],
    };
    for plane in [yz_plane(), xz_plane(), turned] {
        for op in [BooleanOp::Fuse, BooleanOp::Cut] {
            for angle in [360.0, 359.0] {
                let result = try_chain(&[
                    revolve_on(
                        plane,
                        rect_wire(0.0, 0.0, 10.0, 20.0),
                        360.0,
                        BooleanOp::NewSolid,
                    ),
                    revolve_on(plane, rect_wire(8.0, 18.0, 14.0, 24.0), angle, op),
                ]);
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
        let result = try_chain(&[SolidOp::Loft {
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
        }]);
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
/// ball bearing, as the hardware package makes them, the worm seven turns
/// of its groove shorter than the package's 60 mm: its first 16 mm, where
/// the bearing sits, are the package's own.
fn worm_and_bearing() -> (Vec<u8>, Vec<u8>) {
    let text = include_str!("../data/worm_and_608.json");
    let parts: std::collections::HashMap<String, Vec<SolidOp>> =
        serde_json::from_str(text).expect("the fixture reads");
    let build = |name: &str| {
        try_chain(&parts[name])
            .unwrap_or_else(|e| panic!("{name} builds: {e}"))
            .brep_blob
    };
    (build("worm"), build("bearing_608"))
}

/// A worm passing through a bearing's bore shares the ring between the
/// bore and the worm's outside with it: the kernel answers with that
/// solid rather than failing, with both standing on one plane, where the
/// groove runs out through the end faces they share.
#[test]
fn a_worm_through_a_bearing_shares_material_with_it() {
    use kernel_api::KernelQueries;
    let (worm, bearing) = worm_and_bearing();
    let shared = kernel_ogeom::QUERIES
        .overlap(&worm, &bearing, &translate(0.0, 0.0, 0.0))
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("they share material");
    // At most the ring between the bore (4 mm) and the worm's outside
    // (8 mm), 7 mm tall.
    let ring = std::f64::consts::PI * (64.0 - 16.0) * 7.0;
    assert!(
        shared.volume_mm3 > 0.0 && shared.volume_mm3 < ring,
        "{}",
        shared.volume_mm3
    );
}
