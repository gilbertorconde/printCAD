//! Surface steps through the whole chain: sheets made from sketch curves
//! and from the body's own edges, sewn into a solid, mirrored; and the steps
//! that wait on the kernel, which fail with a message saying so.

use kernel_api::{
    Continuity, CurveSource, ProfilePlane, ProfileSegment, ProfileWire, SolidBuildResult, SolidOp,
    SurfaceOp, TessellationSettings,
};
use kernel_ogeom::OgeomKernel;
use ogeom::topo::ShapeType;

use super::common::{chain, count_shapes, try_chain, volume};

fn build(ops: Vec<SurfaceOp>) -> Result<SolidBuildResult, kernel_api::ChainError> {
    let ops: Vec<SolidOp> = ops.into_iter().map(SolidOp::Surface).collect();
    try_chain(&ops)
}

/// How many faces and solids the result's snapshot holds.
fn census(result: &SolidBuildResult) -> (usize, usize) {
    let count = |kind| count_shapes(&result.brep_blob, kind);
    (count(ShapeType::Face), count(ShapeType::Solid))
}

fn shells(result: &SolidBuildResult) -> usize {
    count_shapes(&result.brep_blob, ShapeType::Shell)
}

fn plane(origin: [f64; 3], x: [f64; 3], y: [f64; 3]) -> ProfilePlane {
    let n = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    ProfilePlane {
        origin,
        x_axis: x,
        y_axis: y,
        normal: n,
    }
}

fn xy(z: f64) -> ProfilePlane {
    plane([0.0, 0.0, z], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
}

fn sketch(plane: ProfilePlane, segments: Vec<ProfileSegment>) -> CurveSource {
    CurveSource::Sketch {
        plane,
        wire: ProfileWire::new(segments),
    }
}

fn line(a: [f64; 2], b: [f64; 2]) -> ProfileSegment {
    ProfileSegment::Line { start: a, end: b }
}

fn arc(a: [f64; 2], mid: [f64; 2], b: [f64; 2]) -> ProfileSegment {
    ProfileSegment::Arc {
        start: a,
        mid,
        end: b,
    }
}

/// An open chain on the XY plane: a line, then an arc bending on from it.
fn open_chain(z: f64) -> CurveSource {
    sketch(
        xy(z),
        vec![
            line([0.0, 0.0], [10.0, 0.0]),
            arc([10.0, 0.0], [15.0, 5.0], [10.0, 10.0]),
        ],
    )
}

fn square_loop(z: f64, size: f64) -> CurveSource {
    sketch(
        xy(z),
        vec![
            line([0.0, 0.0], [size, 0.0]),
            line([size, 0.0], [size, size]),
            line([size, size], [0.0, size]),
            line([0.0, size], [0.0, 0.0]),
        ],
    )
}

fn bounds(result: &SolidBuildResult) -> ([f32; 3], [f32; 3]) {
    result.bounds_mm.expect("bounds")
}

#[test]
fn an_open_chain_extrudes_into_a_sheet_of_one_face_per_curve() {
    let result = build(vec![SurfaceOp::Extrude {
        curves: vec![open_chain(0.0)],
        direction: [0.0, 0.0, 1.0],
        length: 8.0,
        symmetric: false,
    }])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
    let (lo, hi) = bounds(&result);
    assert!(
        (lo[2] - 0.0).abs() < 1e-3 && (hi[2] - 8.0).abs() < 1e-3,
        "{lo:?} {hi:?}"
    );
    assert!(
        (hi[0] - 15.0).abs() < 1e-2,
        "the arc bulges to x = 15: {hi:?}"
    );
}

#[test]
fn a_symmetric_extrusion_runs_half_each_way() {
    let result = build(vec![SurfaceOp::Extrude {
        curves: vec![open_chain(0.0)],
        direction: [0.0, 0.0, 1.0],
        length: 8.0,
        symmetric: true,
    }])
    .unwrap();
    let (lo, hi) = bounds(&result);
    assert!(
        (lo[2] + 4.0).abs() < 1e-3 && (hi[2] - 4.0).abs() < 1e-3,
        "{lo:?} {hi:?}"
    );
}

#[test]
fn an_open_line_revolves_into_a_cone() {
    let profile = sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![line([5.0, 0.0], [10.0, 10.0])],
    );
    let result = build(vec![SurfaceOp::Revolve {
        curves: vec![profile],
        origin: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        angle_deg: 360.0,
    }])
    .unwrap();
    assert_eq!(census(&result).1, 0, "a sheet, not a solid");
    let (lo, hi) = bounds(&result);
    assert!(
        (hi[0] - 10.0).abs() < 1e-2 && (lo[0] + 10.0).abs() < 1e-2,
        "{lo:?} {hi:?}"
    );
}

#[test]
fn a_closed_loop_with_a_hole_fills_flat() {
    let outer = square_loop(0.0, 20.0);
    let hole = sketch(
        xy(0.0),
        vec![ProfileSegment::Circle {
            center: [10.0, 10.0],
            radius: 4.0,
        }],
    );
    let result = build(vec![SurfaceOp::PlanarFill {
        curves: vec![outer, hole],
    }])
    .unwrap();
    assert_eq!(census(&result), (1, 0));
}

/// Four walls and two lids, each its own sheet, sew into a closed box: a
/// solid, which measures exactly, as every face is a rectangle on a plane.
#[test]
fn six_sheets_closing_a_box_sew_into_a_solid() {
    let base = square_loop(0.0, 10.0);
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![base.clone()],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::PlanarFill { curves: vec![base] },
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(5.0, 10.0)],
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    assert_eq!(census(&result), (6, 1));
    let measured = OgeomKernel::new()
        .physical_properties(&result.brep_blob)
        .unwrap();
    let volume = measured.volume_mm3.expect("a solid has a volume");
    assert!((volume - 500.0).abs() < 1e-3, "{volume}");
    assert!(!measured.approximate, "{measured:?}");
}

/// The same six sheets left unsewn are surfaces: they have an area and
/// no volume, though their faces close a space.
#[test]
fn unsewn_sheets_have_no_volume() {
    let base = square_loop(0.0, 10.0);
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![base.clone()],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::PlanarFill { curves: vec![base] },
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(5.0, 10.0)],
        },
    ])
    .unwrap();
    let measured = OgeomKernel::new()
        .physical_properties(&result.brep_blob)
        .unwrap();
    assert_eq!(measured.volume_mm3, None);
    assert!(
        (measured.area_mm2 - 400.0).abs() < 1e-3,
        "{}",
        measured.area_mm2
    );
}

#[test]
fn sheets_that_do_not_close_sew_into_a_shell() {
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![open_chain(0.0)],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
}

/// A saddle: four lines meeting end to end, on two heights, filled.
fn saddle() -> Vec<CurveSource> {
    let side = |a: [f64; 3], b: [f64; 3]| {
        // A line in its own vertical plane, through both points.
        let x = [b[0] - a[0], b[1] - a[1], 0.0];
        let len = (x[0] * x[0] + x[1] * x[1]).sqrt();
        let x = [x[0] / len, x[1] / len, 0.0];
        sketch(
            plane([a[0], a[1], 0.0], x, [0.0, 0.0, 1.0]),
            vec![line([0.0, a[2]], [len, b[2]])],
        )
    };
    vec![
        side([0.0, 0.0, 0.0], [10.0, 0.0, 4.0]),
        side([10.0, 0.0, 4.0], [10.0, 10.0, 0.0]),
        side([10.0, 10.0, 0.0], [0.0, 10.0, 4.0]),
        side([0.0, 10.0, 4.0], [0.0, 0.0, 0.0]),
    ]
}

#[test]
fn four_curves_meeting_end_to_end_fill() {
    let result = build(vec![SurfaceOp::Fill {
        boundary: saddle(),
        continuity: Continuity::G0,
    }])
    .unwrap();
    assert_eq!(census(&result), (1, 0));
    let (lo, hi) = bounds(&result);
    assert!(lo[2] > -0.1 && hi[2] < 4.1, "{lo:?} {hi:?}");
}

#[test]
fn a_ruled_surface_spans_two_curves() {
    let result = build(vec![SurfaceOp::Ruled {
        first: open_chain(0.0),
        second: sketch(
            xy(6.0),
            vec![
                line([0.0, 0.0], [12.0, 6.0]),
                line([12.0, 6.0], [8.0, 12.0]),
            ],
        ),
    }])
    .unwrap();
    assert_eq!(census(&result), (2, 0), "one face per pair of curves");
    let (lo, hi) = bounds(&result);
    assert!(
        (lo[2]).abs() < 1e-2 && (hi[2] - 6.0).abs() < 1e-2,
        "{lo:?} {hi:?}"
    );
}

/// A ruled surface between a chain of two curves and a single line.
#[test]
fn a_ruled_surface_spans_curves_of_different_counts() {
    let result = build(vec![SurfaceOp::Ruled {
        first: open_chain(0.0),
        second: sketch(xy(6.0), vec![line([0.0, 0.0], [12.0, 6.0])]),
    }])
    .unwrap();
    assert_eq!(census(&result).1, 0);
}

#[test]
fn a_loft_runs_through_open_sections_in_order() {
    let section =
        |z: f64, bulge: f64| sketch(xy(z), vec![arc([0.0, 0.0], [5.0, bulge], [10.0, 0.0])]);
    let result = build(vec![SurfaceOp::Loft {
        sections: vec![section(0.0, 2.0), section(5.0, 5.0), section(10.0, 1.0)],
        closed: false,
    }])
    .unwrap();
    let (lo, hi) = bounds(&result);
    assert!((hi[2] - 10.0).abs() < 1e-2 && hi[1] > 4.5, "{lo:?} {hi:?}");
}

#[test]
fn a_straight_path_sweeps_exactly() {
    let path = sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![line([0.0, 0.0], [0.0, 7.0])],
    );
    let result = build(vec![SurfaceOp::Sweep {
        profile: vec![open_chain(0.0)],
        path: vec![path],
        frame: Default::default(),
    }])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
    assert!((bounds(&result).1[2] - 7.0).abs() < 1e-3);
}

/// An edge of the body's own sheet, picked, is extruded on from it.
#[test]
fn a_picked_edge_of_the_body_extrudes() {
    let first = SurfaceOp::Extrude {
        curves: vec![sketch(xy(0.0), vec![line([0.0, 0.0], [10.0, 0.0])])],
        direction: [0.0, 0.0, 1.0],
        length: 5.0,
        symmetric: false,
    };
    // The sheet's top edge, at z = 5.
    let top = CurveSource::Edge(kernel_api::EdgeProbe {
        point: [5.0, 0.0, 5.0],
        direction: [1.0, 0.0, 0.0],
        faces: [0, 0],
    });
    let result = build(vec![
        first,
        SurfaceOp::Extrude {
            curves: vec![top],
            direction: [0.0, 1.0, 0.0],
            length: 3.0,
            symmetric: false,
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
    let (_, hi) = bounds(&result);
    assert!(
        (hi[1] - 3.0).abs() < 1e-3 && (hi[2] - 5.0).abs() < 1e-3,
        "{hi:?}"
    );
}

#[test]
fn a_mirror_keeps_the_sheet_and_adds_its_image() {
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![open_chain(0.0)],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Mirror {
            origin: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (4, 0));
    let (lo, hi) = bounds(&result);
    assert!(
        (lo[0] + 15.0).abs() < 1e-2 && (hi[0] - 15.0).abs() < 1e-2,
        "{lo:?} {hi:?}"
    );
}

/// A step that cannot build says why, at its own index: a sew with no
/// sheets, a trim whose plane keeps nothing of the sheet, a fillet between
/// two sheets meeting along an edge they do not share (they need sewing
/// first), and a guide that misses the loft's sections.
#[test]
fn surface_steps_that_cannot_build_say_why() {
    let wall = |a: [f64; 2], b: [f64; 2]| SurfaceOp::Extrude {
        curves: vec![sketch(xy(0.0), vec![line(a, b)])],
        direction: [0.0, 0.0, 1.0],
        length: 5.0,
        symmetric: false,
    };
    let section = |z: f64| sketch(xy(z), vec![line([0.0, 0.0], [10.0, 0.0])]);
    let stray = sketch(
        plane([0.0, 5.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        vec![line([0.0, 0.0], [10.0, 0.0])],
    );
    let cases = [
        ("a sew on nothing", vec![SurfaceOp::Sew { gap: 0.0 }], 0, ""),
        (
            "a trim keeping nothing",
            vec![
                SurfaceOp::PlanarFill {
                    curves: vec![square_loop(0.0, 10.0)],
                },
                SurfaceOp::TrimByPlane {
                    origin: [20.0, 0.0, 0.0],
                    normal: [1.0, 0.0, 0.0],
                },
            ],
            1,
            "leaves nothing",
        ),
        (
            "a fillet between unsewn sheets",
            vec![
                wall([0.0, 0.0], [10.0, 0.0]),
                wall([10.0, 0.0], [10.0, 10.0]),
                SurfaceOp::Fillet {
                    edges: vec![kernel_api::EdgeProbe {
                        point: [10.0, 0.0, 2.5],
                        direction: [0.0, 0.0, 1.0],
                        faces: [0, 0],
                    }],
                    radius: 2.0,
                },
            ],
            2,
            "Sew them first",
        ),
        (
            "a stray guide",
            vec![SurfaceOp::GuidedLoft {
                sections: vec![section(0.0), section(10.0)],
                guides: vec![stray],
            }],
            0,
            "guided loft",
        ),
    ];
    for (what, ops, at, says) in cases {
        let error = build(ops).expect_err(what);
        assert_eq!(error.op_index, at, "{what}: {error:?}");
        assert!(error.message.contains(says), "{what}: {}", error.message);
    }
}

/// An open square tray: a floor and four walls, each its own sheet.
fn tray() -> Vec<SurfaceOp> {
    vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Extrude {
            curves: vec![square_loop(0.0, 10.0)],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
    ]
}

/// A corner patch: three quarter circles meeting end to end.
fn three_sided() -> Vec<CurveSource> {
    let quarter = |plane: ProfilePlane| {
        sketch(
            plane,
            vec![ProfileSegment::Arc {
                start: [10.0, 0.0],
                mid: [7.071_067_811_865_475, 7.071_067_811_865_475],
                end: [0.0, 10.0],
            }],
        )
    };
    vec![
        quarter(plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])),
        quarter(plane([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])),
        quarter(plane([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0])),
    ]
}

#[test]
fn three_curves_meeting_end_to_end_fill() {
    let result = build(vec![SurfaceOp::Fill {
        boundary: three_sided(),
        continuity: Continuity::G0,
    }])
    .unwrap();
    assert_eq!(census(&result), (1, 0));
}

/// A tray's top edges, picked, filled and sewn: a closed box.
#[test]
fn a_fill_closes_a_tray_into_a_solid() {
    let mut ops = tray();
    let top = |a: [f64; 3], b: [f64; 3]| {
        CurveSource::Edge(kernel_api::EdgeProbe {
            point: [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, 5.0],
            direction: [b[0] - a[0], b[1] - a[1], 0.0],
            faces: [0, 0],
        })
    };
    ops.push(SurfaceOp::Fill {
        boundary: vec![
            top([0.0, 0.0, 5.0], [10.0, 0.0, 5.0]),
            top([10.0, 0.0, 5.0], [10.0, 10.0, 5.0]),
            top([10.0, 10.0, 5.0], [0.0, 10.0, 5.0]),
            top([0.0, 10.0, 5.0], [0.0, 0.0, 5.0]),
        ],
        continuity: Continuity::G0,
    });
    ops.push(SurfaceOp::Sew { gap: 0.0 });
    let result = build(ops).unwrap();
    assert_eq!(census(&result), (6, 1), "the tray and its lid close");
}

/// A cone sloping out and down from a circle, in four quarters, filled
/// tangent to them: the fill crowns over the circle, meeting every quarter
/// without a crease.
#[test]
fn a_tangent_fill_meets_its_neighbours_without_a_crease() {
    use kernel_api::KernelQueries;
    let turns = [0.0f64, 90.0, 180.0, 270.0];
    let mut ops: Vec<SurfaceOp> = turns
        .iter()
        .map(|deg| {
            let (s, c) = deg.to_radians().sin_cos();
            SurfaceOp::Revolve {
                curves: vec![sketch(
                    plane([0.0, 0.0, 0.0], [c, s, 0.0], [0.0, 0.0, 1.0]),
                    vec![line([5.0, 5.0], [9.0, 1.0])],
                )],
                origin: [0.0, 0.0, 0.0],
                axis: [0.0, 0.0, 1.0],
                angle_deg: 90.0,
            }
        })
        .collect();
    let rim = |deg: &f64| {
        let (s, c) = (deg + 45.0).to_radians().sin_cos();
        CurveSource::Edge(kernel_api::EdgeProbe {
            point: [5.0 * c, 5.0 * s, 5.0],
            direction: [-s, c, 0.0],
            faces: [0, 0],
        })
    };
    ops.push(SurfaceOp::Fill {
        boundary: turns.iter().map(rim).collect(),
        continuity: Continuity::G1,
    });
    ops.push(SurfaceOp::Sew { gap: 0.0 });
    let result = build(ops).unwrap();
    let (_, hi) = bounds(&result);
    assert!(hi[2] > 5.5, "the fill crowns above the square: {hi:?}");
    let joins = kernel_ogeom::QUERIES.continuity(&result.brep_blob).unwrap();
    let crowned: Vec<_> = joins
        .iter()
        .filter(|j| (j.point[2] - 5.0).abs() < 1e-3)
        .collect();
    assert_eq!(crowned.len(), 4, "{joins:?}");
    assert!(crowned.iter().all(|j| j.angle_deg < 0.5), "{crowned:?}");
}

/// Two walls of an L meeting at a right angle, rounded where they meet.
#[test]
fn an_edge_where_two_sheets_meet_rounds() {
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![sketch(
                xy(0.0),
                vec![
                    line([0.0, 0.0], [10.0, 0.0]),
                    line([10.0, 0.0], [10.0, 10.0]),
                ],
            )],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Fillet {
            edges: vec![kernel_api::EdgeProbe {
                point: [10.0, 0.0, 2.5],
                direction: [0.0, 0.0, 1.0],
                faces: [0, 0],
            }],
            radius: 2.0,
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (3, 0), "two walls and the round between");
}

#[test]
fn sheets_a_hair_apart_sew() {
    let near = sketch(xy(0.0), vec![line([10.05, 0.0], [20.0, 0.0])]);
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![sketch(xy(0.0), vec![line([0.0, 0.0], [10.0, 0.0])])],
            direction: [0.0, 1.0, 0.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Extrude {
            curves: vec![near],
            direction: [0.0, 1.0, 0.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Sew { gap: 0.1 },
    ])
    .unwrap();
    assert_eq!(shells(&result), 1, "the strips join across the gap");
}

#[test]
fn a_sheet_thickens_into_a_solid() {
    let result = build(vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Thicken {
            thickness: 2.0,
            both_sides: false,
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (6, 1));
}

#[test]
fn a_sheet_offsets() {
    let result = build(vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Offset {
            faces: vec![kernel_api::FaceProbe {
                point: [5.0, 5.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                name: 0,
            }],
            distance: 2.0,
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
    assert!((bounds(&result).1[2].abs() - 2.0).abs() < 1e-3);
}

#[test]
fn a_free_form_sheet_offsets() {
    let result = build(vec![
        SurfaceOp::Fill {
            boundary: saddle(),
            continuity: Continuity::G0,
        },
        SurfaceOp::Offset {
            faces: vec![kernel_api::FaceProbe {
                point: [5.0, 5.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                name: 0,
            }],
            distance: 1.0,
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
}

#[test]
fn a_sheet_trims_by_a_plane() {
    let result = build(vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::TrimByPlane {
            origin: [5.0, 0.0, 0.0],
            normal: [-1.0, 0.0, 0.0],
        },
    ])
    .unwrap();
    assert!((bounds(&result).1[0] - 5.0).abs() < 1e-3);
}

#[test]
fn a_face_splits_along_a_curve() {
    let result = build(vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Split {
            faces: vec![kernel_api::FaceProbe {
                point: [5.0, 5.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                name: 0,
            }],
            curves: vec![sketch(xy(0.0), vec![line([0.0, 5.0], [10.0, 5.0])])],
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
}

/// A closed loop inside a face cuts it into the patch it rings and the
/// face around it, holed.
#[test]
fn a_face_splits_along_a_closed_loop_inside_it() {
    let result = build(vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Split {
            faces: vec![kernel_api::FaceProbe {
                point: [1.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                name: 0,
            }],
            curves: vec![sketch(
                xy(0.0),
                vec![
                    line([3.0, 3.0], [7.0, 3.0]),
                    line([7.0, 3.0], [7.0, 7.0]),
                    line([7.0, 7.0], [3.0, 7.0]),
                    line([3.0, 7.0], [3.0, 3.0]),
                ],
            )],
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
    let (min, max) = bounds(&result);
    assert!((min[0]).abs() < 1e-3 && (max[0] - 10.0).abs() < 1e-3);
}

#[test]
fn a_face_extends_past_its_edge() {
    let result = build(vec![
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Extend {
            edges: vec![kernel_api::EdgeProbe {
                point: [10.0, 5.0, 0.0],
                direction: [0.0, 1.0, 0.0],
                faces: [0, 0],
            }],
            length: 5.0,
            continuity: Continuity::G0,
        },
    ])
    .unwrap();
    assert!((bounds(&result).1[0] - 15.0).abs() < 1e-3);
}

/// An extruded line and arc, a sheet of two faces, extended past both top
/// edges: each face grows and the two stay one sheet.
#[test]
fn an_extruded_sheet_extends_past_its_top_edges() {
    let top = |point: [f64; 3], direction: [f64; 3]| kernel_api::EdgeProbe {
        point,
        direction,
        faces: [0, 0],
    };
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![open_chain(0.0)],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Extend {
            edges: vec![
                top([5.0, 0.0, 5.0], [1.0, 0.0, 0.0]),
                top([15.0, 5.0, 5.0], [0.0, 1.0, 0.0]),
            ],
            length: 3.0,
            continuity: Continuity::G1,
        },
    ])
    .unwrap();
    assert!(
        (bounds(&result).1[2] - 8.0).abs() < 1e-3,
        "{:?}",
        bounds(&result)
    );
    assert_eq!(census(&result), (2, 0), "still two faces");
    assert_eq!(shells(&result), 1, "the faces still share their edge");
}

#[test]
fn two_strips_blend() {
    // Each strip runs away from the gap, so the edge facing it is the
    // profile's own line.
    let strip = |y: f64, z: f64, way: f64| SurfaceOp::Extrude {
        curves: vec![sketch(xy(z), vec![line([0.0, y], [10.0, y])])],
        direction: [0.0, way, 0.0],
        length: 5.0,
        symmetric: false,
    };
    let edge = |y: f64, z: f64| kernel_api::EdgeProbe {
        point: [5.0, y, z],
        direction: [1.0, 0.0, 0.0],
        faces: [0, 0],
    };
    let result = build(vec![
        strip(5.0, 0.0, -1.0),
        strip(15.0, 6.0, 1.0),
        SurfaceOp::Blend {
            first: edge(5.0, 0.0),
            second: edge(15.0, 6.0),
            continuity: Continuity::G1,
        },
    ])
    .unwrap();
    assert_eq!(census(&result), (3, 0));
}

#[test]
fn an_open_profile_sweeps_along_a_curve() {
    let path = sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![arc([0.0, 0.0], [5.0, 2.0], [10.0, 0.0])],
    );
    let profile = sketch(
        plane([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        vec![line([0.0, 0.0], [3.0, 0.0]), line([3.0, 0.0], [3.0, 3.0])],
    );
    let result = build(vec![SurfaceOp::Sweep {
        profile: vec![profile],
        path: vec![path],
        frame: Default::default(),
    }])
    .unwrap();
    assert_eq!(census(&result), (2, 0));
}

#[test]
fn a_closed_loft_comes_back_to_its_first_section() {
    let ring = |z: f64, r: f64| {
        sketch(
            xy(z),
            vec![ProfileSegment::Circle {
                center: [0.0, 0.0],
                radius: r,
            }],
        )
    };
    let result = build(vec![SurfaceOp::Loft {
        sections: vec![ring(0.0, 5.0), ring(5.0, 8.0), ring(10.0, 5.0)],
        closed: true,
    }])
    .unwrap();
    assert_eq!(census(&result).1, 0);
}

#[test]
fn a_sheet_exports_as_step() {
    let result = build(vec![SurfaceOp::Extrude {
        curves: vec![open_chain(0.0)],
        direction: [0.0, 0.0, 1.0],
        length: 5.0,
        symmetric: false,
    }])
    .unwrap();
    let body = kernel_ogeom::export::ExportBody {
        name: "sheet".into(),
        brep: Some(&result.brep_blob),
        transform: None,
        mesh: &result.mesh,
        finish: None,
    };
    let written = kernel_ogeom::export::export(
        &[body],
        kernel_ogeom::export::ExportFormat::Step,
        &TessellationSettings::default(),
    )
    .unwrap();
    assert_eq!(written.written, 1, "skipped: {:?}", written.skipped);
}

/// Where faces meet, measured: an L's two walls meet at a right angle, a
/// line running on into a tangent arc of radius 5 meets it smoothly but
/// goes from flat to bending 1/5.
#[test]
fn continuity_tells_a_crease_from_a_tangent_join() {
    use kernel_api::KernelQueries;
    let walls = |segments| {
        build(vec![SurfaceOp::Extrude {
            curves: vec![sketch(xy(0.0), segments)],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        }])
        .unwrap()
    };
    let l = walls(vec![
        line([0.0, 0.0], [10.0, 0.0]),
        line([10.0, 0.0], [10.0, 10.0]),
    ]);
    let joins = kernel_ogeom::QUERIES.continuity(&l.brep_blob).unwrap();
    assert_eq!(joins.len(), 1, "{joins:?}");
    assert!((joins[0].angle_deg - 90.0).abs() < 0.5, "{joins:?}");
    assert!(joins[0].gap < 1e-6);
    assert!((joins[0].point[0] - 10.0).abs() < 1e-6 && (joins[0].point[2] - 2.5).abs() < 1e-6);

    // The arc leaves (10, 0) heading along +x, as the line does.
    let smooth = walls(vec![
        line([0.0, 0.0], [10.0, 0.0]),
        arc(
            [10.0, 0.0],
            [13.535_533_905_932_737, 1.464_466_094_067_262_4],
            [15.0, 5.0],
        ),
    ]);
    let joins = kernel_ogeom::QUERIES.continuity(&smooth.brep_blob).unwrap();
    assert_eq!(joins.len(), 1, "{joins:?}");
    assert!(joins[0].angle_deg < 0.5, "{joins:?}");
    let jump = joins[0].curvature.expect("read");
    assert!((jump - 0.2).abs() < 1e-3, "{joins:?}");
}

/// Walls with a floor sewn on: every edge two faces share meets with no
/// gap.
#[test]
fn a_sewn_floor_meets_its_walls_with_no_gap() {
    use kernel_api::KernelQueries;
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![square_loop(0.0, 10.0)],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::PlanarFill {
            curves: vec![square_loop(0.0, 10.0)],
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    let joins = kernel_ogeom::QUERIES.continuity(&result.brep_blob).unwrap();
    assert_eq!(
        joins.len(),
        8,
        "four corners and four floor edges: {joins:?}"
    );
    assert!(joins.iter().all(|j| j.gap < 1e-6), "{joins:?}");
}

/// A line in its own vertical plane, from `a` to `b`.
fn line_3d(a: [f64; 3], b: [f64; 3]) -> CurveSource {
    let x = [b[0] - a[0], b[1] - a[1], 0.0];
    let len = (x[0] * x[0] + x[1] * x[1]).sqrt();
    let x = [x[0] / len, x[1] / len, 0.0];
    sketch(
        plane([a[0], a[1], 0.0], x, [0.0, 0.0, 1.0]),
        vec![line([0.0, a[2]], [len, b[2]])],
    )
}

/// A path of a line and an arc tangent to it sweeps a profile along both.
#[test]
fn a_sweep_follows_a_line_into_a_tangent_arc() {
    let half = std::f64::consts::FRAC_1_SQRT_2 * 5.0;
    let path = sketch(
        xy(0.0),
        vec![
            line([0.0, 0.0], [10.0, 0.0]),
            arc([10.0, 0.0], [10.0 + half, 5.0 - half], [15.0, 5.0]),
        ],
    );
    let profile = sketch(
        plane([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        vec![line([-1.0, 0.0], [1.0, 0.0])],
    );
    build(vec![SurfaceOp::Sweep {
        profile: vec![profile],
        path: vec![path],
        frame: Default::default(),
    }])
    .unwrap();
}

/// Two semicircles in crossing planes, sharing their ends, close a hole.
#[test]
fn two_semicircles_in_crossing_planes_fill() {
    let up = sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![arc([-10.0, 0.0], [0.0, 10.0], [10.0, 0.0])],
    );
    let down = sketch(xy(0.0), vec![arc([10.0, 0.0], [0.0, -10.0], [-10.0, 0.0])]);
    build(vec![SurfaceOp::Fill {
        boundary: vec![up, down],
        continuity: Continuity::G0,
    }])
    .unwrap();
}

/// Four lines rising and falling around a diamond fill: seen along
/// (0, 1, -1) the loop is simple.
#[test]
fn a_four_line_saddle_fills() {
    let p = [
        [-10.0, 0.0, 0.0],
        [0.0, 0.0, 10.0],
        [10.0, 0.0, 0.0],
        [0.0, -10.0, 0.0],
    ];
    build(vec![SurfaceOp::Fill {
        boundary: (0..4).map(|i| line_3d(p[i], p[(i + 1) % 4])).collect(),
        continuity: Continuity::G0,
    }])
    .unwrap();
}

/// A tube's rim filled tangent to its wall: a dome rising off the rim.
#[test]
fn a_tangent_cap_closes_a_tube() {
    let circle = sketch(
        xy(0.0),
        vec![
            arc([5.0, 0.0], [0.0, 5.0], [-5.0, 0.0]),
            arc([-5.0, 0.0], [0.0, -5.0], [5.0, 0.0]),
        ],
    );
    let rim = |x: f64, y: f64, dx: f64, dy: f64| {
        CurveSource::Edge(kernel_api::EdgeProbe {
            point: [x, y, 10.0],
            direction: [dx, dy, 0.0],
            faces: [0, 0],
        })
    };
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![circle],
            direction: [0.0, 0.0, 1.0],
            length: 10.0,
            symmetric: false,
        },
        SurfaceOp::Fill {
            boundary: vec![rim(0.0, 5.0, -1.0, 0.0), rim(0.0, -5.0, 1.0, 0.0)],
            continuity: Continuity::G1,
        },
    ])
    .unwrap();
    assert!(bounds(&result).1[2] > 10.5);
}

/// Two walls meeting at a right angle thicken into one solid.
#[test]
fn a_folded_sheet_thickens() {
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![sketch(
                xy(0.0),
                vec![
                    line([0.0, 0.0], [10.0, 0.0]),
                    line([10.0, 0.0], [10.0, 10.0]),
                ],
            )],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Thicken {
            thickness: 1.0,
            both_sides: false,
        },
    ])
    .unwrap();
    assert_eq!(census(&result).1, 1);
}

/// A loft between two arcs of different radii thickens into a solid, one
/// side or both, thin or thick.
#[test]
fn a_loft_between_two_arcs_thickens() {
    let section =
        |z: f64, bulge: f64| sketch(xy(z), vec![arc([0.0, 0.0], [10.0, bulge], [20.0, 0.0])]);
    for thickness in [0.5, 3.0] {
        for both_sides in [false, true] {
            let result = build(vec![
                SurfaceOp::Loft {
                    sections: vec![section(0.0, 4.0), section(20.0, 8.0)],
                    closed: false,
                },
                SurfaceOp::Thicken {
                    thickness,
                    both_sides,
                },
            ]);
            match result {
                Ok(r) => assert_eq!(census(&r).1, 1, "{thickness} {both_sides}"),
                Err(e) => panic!("{thickness} {both_sides}: {e}"),
            }
        }
    }
}

/// A sketch line seen square to its plane crosses a half cylinder from
/// its bottom edge to its top: the face splits along the curve it lands
/// on, and the two halves bend alike either side of the join.
#[test]
fn a_curved_face_splits_along_a_sketch_seen_square_to_it() {
    use kernel_api::KernelQueries;
    let split = build(vec![
        SurfaceOp::Extrude {
            curves: vec![sketch(
                xy(0.0),
                vec![arc([5.0, 0.0], [0.0, 5.0], [-5.0, 0.0])],
            )],
            direction: [0.0, 0.0, 1.0],
            length: 10.0,
            symmetric: false,
        },
        SurfaceOp::Split {
            faces: vec![kernel_api::FaceProbe {
                point: [0.0, 5.0, 5.0],
                normal: [0.0, 1.0, 0.0],
                name: 0,
            }],
            curves: vec![sketch(
                plane([0.0, 10.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                vec![line([-4.0, -1.0], [4.0, 11.0])],
            )],
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    assert_eq!(census(&split), (2, 0));
    let joins = kernel_ogeom::QUERIES.continuity(&split.brep_blob).unwrap();
    assert_eq!(joins.len(), 1, "{joins:?}");
    assert!(joins[0].angle_deg < 0.01, "{joins:?}");
    assert!(joins[0].curvature.expect("read") < 1e-6, "{joins:?}");
}

/// The curvature at every vertex of a mesh, read from the exact surfaces
/// and signed against the mesh's normal there, asked of a sheet and of a
/// solid: a tube of radius 5 bends
/// 1/5 round and not at all along, a ball of radius 4 bends 1/4 every way,
/// negative where the normal points out of the bulge.
#[test]
fn curvature_is_read_at_points_on_the_faces() {
    use kernel_api::KernelQueries;
    let tube = build(vec![SurfaceOp::Revolve {
        curves: vec![sketch(
            plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            vec![line([5.0, 0.0], [5.0, 10.0])],
        )],
        origin: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        angle_deg: 360.0,
    }])
    .unwrap();
    let ball = build(vec![
        SurfaceOp::Revolve {
            curves: vec![sketch(
                plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                vec![arc([0.0, -4.0], [4.0, 0.0], [0.0, 4.0])],
            )],
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle_deg: 360.0,
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    // How far out from the axis or the centre a normal points, and the
    // curvatures, largest first, for a normal pointing out.
    let round = |p: [f32; 3], n: [f32; 3]| {
        let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
        (p[0] * n[0] + p[1] * n[1]) / r
    };
    let centre = |p: [f32; 3], n: [f32; 3]| {
        let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        (p[0] * n[0] + p[1] * n[1] + p[2] * n[2]) / r
    };
    type Out = fn([f32; 3], [f32; 3]) -> f32;
    for (result, out, bends) in [
        (&tube, round as Out, [0.0, -0.2]),
        (&ball, centre as Out, [-0.25, -0.25]),
    ] {
        let mesh = &result.mesh;
        let inside = {
            let t = &mesh.indices[0..3];
            [0, 1, 2].map(|k| {
                t.iter()
                    .map(|&i| f64::from(mesh.positions[i as usize][k]))
                    .sum::<f64>()
                    / 3.0
            })
        };
        // The triangles' corners: the outline has vertices of its own.
        let mut used: Vec<usize> = mesh.indices.iter().map(|&i| i as usize).collect();
        used.sort_unstable();
        used.dedup();
        let points: Vec<([f64; 3], [f64; 3])> = used
            .iter()
            .map(|&i| {
                (
                    mesh.positions[i].map(f64::from),
                    mesh.normals[i].map(f64::from),
                )
            })
            .collect();
        let read = kernel_ogeom::QUERIES
            .curvature(&result.brep_blob, &[(inside, points.clone())])
            .unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].len(), points.len());
        let mut seen = 0;
        for (&i, k) in used.iter().zip(&read[0]) {
            let Some(k) = k else { continue };
            seen += 1;
            let outward = out(mesh.positions[i], mesh.normals[i]);
            assert!(outward.abs() > 0.9, "a normal square to the surface");
            let expect = if outward > 0.0 {
                bends
            } else {
                [-bends[1], -bends[0]]
            };
            assert!(
                (k[0] - expect[0]).abs() < 1e-6 && (k[1] - expect[1]).abs() < 1e-6,
                "{k:?} for {expect:?}"
            );
        }
        assert!(
            seen * 10 >= points.len() * 9,
            "most points read: {seen} of {}",
            points.len()
        );
    }
}

/// A loft between two straight sections that follows two arched guides:
/// the sheet passes through the guides' crowns.
#[test]
fn a_loft_follows_its_guides() {
    let section = |z: f64| sketch(xy(z), vec![line([0.0, 0.0], [10.0, 0.0])]);
    // In a plane square to X at `x`: along Z, bulging toward +Y.
    let guide = |x: f64| {
        sketch(
            plane([x, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            vec![arc([0.0, 0.0], [5.0, 3.0], [10.0, 0.0])],
        )
    };
    let result = build(vec![SurfaceOp::GuidedLoft {
        sections: vec![section(0.0), section(10.0)],
        guides: vec![guide(0.0), guide(10.0)],
    }])
    .unwrap();
    assert_eq!(census(&result), (1, 0), "one face");
    let (lo, hi) = bounds(&result);
    assert!((hi[1] - 3.0).abs() < 1e-2, "through the crowns: {hi:?}");
    assert!(
        lo[1].abs() < 1e-2 && (hi[2] - 10.0).abs() < 1e-3,
        "{lo:?} {hi:?}"
    );
}

/// An arch swept between two rails that spread apart: it widens with
/// them, and stands taller as it does.
#[test]
fn a_profile_sweeps_between_two_rails() {
    let arch = sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![arc([0.0, 0.0], [5.0, 3.0], [10.0, 0.0])],
    );
    let result = build(vec![SurfaceOp::SweepTwoRails {
        profile: vec![arch],
        first_rail: vec![line_3d([0.0, 0.0, 0.0], [0.0, 20.0, 0.0])],
        second_rail: vec![line_3d([10.0, 0.0, 0.0], [14.0, 20.0, 0.0])],
    }])
    .unwrap();
    assert_eq!(census(&result), (1, 0));
    let (lo, hi) = bounds(&result);
    assert!(
        (hi[0] - 14.0).abs() < 1e-2 && (hi[1] - 20.0).abs() < 1e-3,
        "{hi:?}"
    );
    assert!(
        (hi[2] - 3.0 * 1.4).abs() < 2e-2,
        "scaled with the width: {hi:?}"
    );
    assert!(lo[2].abs() < 1e-3);
}

/// Two walls of an L that stop short of its corner, separate sheets:
/// the round between them meets each, cutting both back to where it
/// touches, and the three are one sheet. Rolled on the faces' other sides
/// (their normals face out of the L) the ball cannot reach both.
#[test]
fn a_round_joins_two_separate_walls() {
    let ops = |flip| {
        vec![
            SurfaceOp::Extrude {
                curves: vec![sketch(xy(0.0), vec![line([2.0, 0.0], [10.0, 0.0])])],
                direction: [0.0, 0.0, 1.0],
                length: 5.0,
                symmetric: false,
            },
            SurfaceOp::Extrude {
                curves: vec![sketch(xy(0.0), vec![line([0.0, 10.0], [0.0, 2.0])])],
                direction: [0.0, 0.0, 1.0],
                length: 5.0,
                symmetric: false,
            },
            SurfaceOp::FilletFaces {
                first: kernel_api::FaceProbe {
                    point: [6.0, 0.0, 2.5],
                    normal: [0.0; 3],
                    name: 0,
                },
                second: kernel_api::FaceProbe {
                    point: [0.0, 6.0, 2.5],
                    normal: [0.0; 3],
                    name: 0,
                },
                radius: 3.0,
                flip,
            },
        ]
    };
    let result = build(ops([true, true])).unwrap();
    assert_eq!(census(&result), (3, 0), "two walls and the round");
    assert_eq!(shells(&result), 1, "one sheet");
    let (lo, hi) = bounds(&result);
    assert!(lo[0].abs() < 1e-3 && lo[1].abs() < 1e-3, "{lo:?}");
    assert!((hi[0] - 10.0).abs() < 1e-3 && (hi[1] - 10.0).abs() < 1e-3);
    let joins = {
        use kernel_api::KernelQueries;
        kernel_ogeom::QUERIES.continuity(&result.brep_blob).unwrap()
    };
    assert_eq!(joins.len(), 2, "{joins:?}");
    assert!(
        joins.iter().all(|j| j.angle_deg < 0.1),
        "tangent: {joins:?}"
    );
    let error = build(ops([false, false])).unwrap_err();
    assert!(
        error.message.contains("rounding between the faces"),
        "{error:?}"
    );
}

/// An edge of another body's sheet, where that body sits 5 above this
/// one, extrudes into a wall standing on it there; one it does not hold
/// is refused.
#[test]
fn an_edge_of_another_body_extrudes_where_that_body_sits() {
    let other = build(vec![SurfaceOp::PlanarFill {
        curves: vec![square_loop(0.0, 10.0)],
    }])
    .unwrap();
    let raised = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 5.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let wall = |point: [f64; 3]| {
        build(vec![SurfaceOp::Extrude {
            curves: vec![CurveSource::BodyEdge {
                brep: other.brep_blob.clone(),
                edge: kernel_api::EdgeProbe {
                    point,
                    direction: [1.0, 0.0, 0.0],
                    faces: [0, 0],
                },
                transform: Some(Box::new(raised)),
            }],
            direction: [0.0, 0.0, 1.0],
            length: 3.0,
            symmetric: false,
        }])
    };
    let result = wall([5.0, 0.0, 0.0]).unwrap();
    assert_eq!(census(&result), (1, 0));
    let (lo, hi) = bounds(&result);
    assert!(
        (lo[2] - 5.0).abs() < 1e-6 && (hi[2] - 8.0).abs() < 1e-6,
        "{lo:?} {hi:?}"
    );
    assert!(
        lo[1].abs() < 1e-6 && hi[1].abs() < 1e-6,
        "along the edge at y = 0"
    );
    assert!((hi[0] - 10.0).abs() < 1e-6);
    assert!(wall([5.0, 40.0, 0.0]).is_err(), "no edge there");
}

/// A profile sitting at the far end of its path sweeps back along it: the
/// path runs from the end the profile is at, whichever way it was drawn.
#[test]
fn a_sweep_starts_at_the_end_of_the_path_its_profile_sits_by() {
    let side = plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let up = sketch(side, vec![line([0.0, 0.0], [0.0, 20.0])]);
    let down = sketch(side, vec![line([0.0, 20.0], [0.0, 0.0])]);
    let bend = sketch(side, vec![arc([0.0, 0.0], [5.0, 10.0], [0.0, 20.0])]);
    let bend_back = sketch(side, vec![arc([0.0, 20.0], [5.0, 10.0], [0.0, 0.0])]);
    let profile = |z: f64| {
        sketch(
            plane([0.0, 0.0, z], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            vec![line([-2.0, 0.0], [2.0, 0.0])],
        )
    };
    for (name, path) in [
        ("up", &up),
        ("down", &down),
        ("bend", &bend),
        ("bend back", &bend_back),
    ] {
        for z in [0.0, 20.0] {
            let result = build(vec![SurfaceOp::Sweep {
                profile: vec![profile(z)],
                path: vec![path.clone()],
                frame: Default::default(),
            }])
            .unwrap();
            let (lo, hi) = bounds(&result);
            assert!(
                lo[2].abs() < 1e-3 && (hi[2] - 20.0).abs() < 1e-3,
                "{name}, profile at z = {z}: {lo:?} {hi:?}"
            );
        }
    }
}

/// A padded block's twelve edges meet with no gap: the pad's far end is
/// its sketch's face moved along, and is measured where it stands.
#[test]
fn a_padded_block_has_no_gaps() {
    use kernel_api::{BooleanOp, ExtrudeTermination, KernelQueries, Profile, SweepKind};
    let CurveSource::Sketch { plane, wire } = square_loop(0.0, 10.0) else {
        unreachable!()
    };
    let pad = SolidOp::Sweep {
        profile: Profile {
            plane,
            wires: vec![wire],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 5.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    };
    let block = chain(&[pad]);
    let joins = kernel_ogeom::QUERIES.continuity(&block.brep_blob).unwrap();
    assert_eq!(joins.len(), 12, "{joins:?}");
    for join in &joins {
        assert!(join.gap < 1e-6, "{join:?}");
        assert!((join.angle_deg - 90.0).abs() < 1e-6, "{join:?}");
    }
}

/// A line extruded across itself is a rectangle; extruded along itself it
/// sweeps nothing, and the step says so rather than leave the body empty.
#[test]
fn a_line_extruded_along_itself_is_refused() {
    let line_along_x = sketch(xy(0.0), vec![line([0.0, 0.0], [10.0, 0.0])]);
    let extrude = |direction: [f64; 3]| {
        build(vec![SurfaceOp::Extrude {
            curves: vec![line_along_x.clone()],
            direction,
            length: 5.0,
            symmetric: false,
        }])
    };
    for across in [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
        let result = extrude(across).unwrap();
        let area = OgeomKernel::new()
            .physical_properties(&result.brep_blob)
            .unwrap()
            .area_mm2;
        assert!((area - 50.0).abs() < 1e-6, "{across:?}: {area}");
    }
    let along = extrude([1.0, 0.0, 0.0]).unwrap_err();
    assert_eq!(along.op_index, 0);
    assert!(along.message.contains("sweeps out no surface"), "{along}");
}

/// A quarter arc through a point given to eight places, as a sketch holds
/// one, from the dome's rim at (10, 0, 0) up to its pole.
fn dome_quarter() -> CurveSource {
    sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![arc([10.0, 0.0], [7.071_067_8, 7.071_067_8], [0.0, 10.0])],
    )
}

/// Half a dome sewn to its mirror image, or to a second half turned the
/// other way: one open sheet, the two rims left apart, its shape as it was.
#[test]
fn half_domes_sew_into_an_open_dome() {
    let half = |axis: [f64; 3]| SurfaceOp::Revolve {
        curves: vec![dome_quarter()],
        origin: [0.0; 3],
        axis,
        angle_deg: 180.0,
    };
    let mirrored = vec![
        half([0.0, 0.0, 1.0]),
        SurfaceOp::Mirror {
            origin: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
        },
        SurfaceOp::Sew { gap: 0.0 },
    ];
    let turned = vec![
        half([0.0, 0.0, 1.0]),
        half([0.0, 0.0, -1.0]),
        SurfaceOp::Sew { gap: 0.0 },
    ];
    for (name, ops) in [("mirrored", mirrored), ("turned", turned)] {
        let result = build(ops).unwrap();
        assert_eq!(census(&result), (2, 0), "{name}: an open sheet");
        assert_eq!(shells(&result), 1, "{name}");
        let (lo, hi) = bounds(&result);
        assert!(
            (lo[1] + 10.0).abs() < 1e-3 && (hi[2] - 10.0).abs() < 1e-3,
            "{name}: {lo:?} {hi:?}"
        );
    }
}

/// Triangles whose vertex normals point against the side their winding
/// faces: drawn lit from behind.
fn shaded_from_behind(mesh: &kernel_api::TriMesh) -> usize {
    mesh.indices
        .chunks(3)
        .filter(|t| {
            let p = [0, 1, 2].map(|k| mesh.positions[t[k] as usize].map(f64::from));
            let (a, b) = (
                [0, 1, 2].map(|k| p[1][k] - p[0][k]),
                [0, 1, 2].map(|k| p[2][k] - p[0][k]),
            );
            let facing = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let normal = [0, 1, 2].map(|k| {
                t.iter()
                    .map(|&i| f64::from(mesh.normals[i as usize][k]))
                    .sum::<f64>()
            });
            (0..3).map(|k| facing[k] * normal[k]).sum::<f64>() < 0.0
        })
        .count()
}

/// A half cylinder and its mirror image: the image's mesh is shaded the
/// way its triangles face, as the original's is.
#[test]
fn a_mirrored_sheet_is_shaded_the_way_it_faces() {
    let result = build(vec![
        SurfaceOp::Extrude {
            curves: vec![sketch(
                xy(0.0),
                vec![arc([10.0, 0.0], [0.0, 10.0], [-10.0, 0.0])],
            )],
            direction: [0.0, 0.0, 1.0],
            length: 5.0,
            symmetric: false,
        },
        SurfaceOp::Mirror {
            origin: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
        },
    ])
    .unwrap();
    assert_eq!(shaded_from_behind(&result.mesh), 0);
}

/// Free-form sheets thicken into a solid, one side or both: a loft
/// through three bowed sections that bow the other way between, and a
/// dome turned from a quarter arc.
#[test]
fn free_form_sheets_thicken() {
    let section =
        |z: f64, bulge: f64| sketch(xy(z), vec![arc([0.0, 0.0], [10.0, bulge], [20.0, 0.0])]);
    let loft = SurfaceOp::Loft {
        sections: vec![section(0.0, 3.0), section(10.0, -3.0), section(20.0, 5.0)],
        closed: false,
    };
    let dome = SurfaceOp::Revolve {
        curves: vec![dome_quarter()],
        origin: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        angle_deg: 360.0,
    };
    for (name, sheet) in [("loft", loft), ("dome", dome)] {
        for both_sides in [false, true] {
            let result = build(vec![
                sheet.clone(),
                SurfaceOp::Thicken {
                    thickness: 1.0,
                    both_sides,
                },
            ]);
            match result {
                Ok(r) => assert_eq!(census(&r).1, 1, "{name} {both_sides}"),
                Err(e) => panic!("{name} {both_sides}: {e}"),
            }
        }
    }
}

/// A half circle turned a whole turn is a closed sheet; sewn, it is a
/// ball's solid.
#[test]
fn a_revolved_half_circle_sews_into_a_ball() {
    let half_circle = sketch(
        plane([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        vec![arc([0.0, -10.0], [10.0, 0.0], [0.0, 10.0])],
    );
    let result = build(vec![
        SurfaceOp::Revolve {
            curves: vec![half_circle],
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle_deg: 360.0,
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    assert_eq!(census(&result), (1, 1));
    let volume = volume(&result.brep_blob);
    let ball = 4.0 / 3.0 * std::f64::consts::PI * 1000.0;
    assert!((volume - ball).abs() < 1e-6 * ball, "{volume}");
}

/// Mesh edges, welded where their ends meet, that two triangles walk the
/// same way: where neighbouring faces disagree about which side is out.
fn walked_one_way(mesh: &kernel_api::TriMesh) -> usize {
    let key = |i: u32| mesh.positions[i as usize].map(|c| (c * 1000.0).round() as i64);
    let mut walks = std::collections::HashMap::new();
    for t in mesh.indices.chunks(3) {
        for k in 0..3 {
            let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
            if a != b {
                *walks.entry((a, b)).or_insert(0usize) += 1;
            }
        }
    }
    walks.values().filter(|&&n| n > 1).count()
}

/// Sheets made facing every which way sew into one that faces one way: a
/// box of walls run down from the top and lids drawn upside down is a
/// solid with its volume outside in, and an open tray's floor turns to
/// face out with its walls.
#[test]
fn sewn_sheets_face_one_way() {
    let upside_down = |z: f64| {
        sketch(
            plane([0.0, 0.0, z], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            vec![
                line([0.0, 0.0], [10.0, 0.0]),
                line([10.0, 0.0], [10.0, 10.0]),
                line([10.0, 10.0], [0.0, 10.0]),
                line([0.0, 10.0], [0.0, 0.0]),
            ],
        )
    };
    let walls_down = SurfaceOp::Extrude {
        curves: vec![square_loop(5.0, 10.0)],
        direction: [0.0, 0.0, -1.0],
        length: 5.0,
        symmetric: false,
    };
    let result = build(vec![
        walls_down,
        SurfaceOp::PlanarFill {
            curves: vec![upside_down(0.0)],
        },
        SurfaceOp::PlanarFill {
            curves: vec![upside_down(5.0)],
        },
        SurfaceOp::Sew { gap: 0.0 },
    ])
    .unwrap();
    assert_eq!(census(&result), (6, 1));
    assert_eq!(walked_one_way(&result.mesh), 0);
    let volume = volume(&result.brep_blob);
    assert!((volume - 500.0).abs() < 1e-6, "{volume}");

    let mut tray = tray();
    assert!(walked_one_way(&build(tray.clone()).unwrap().mesh) > 0);
    tray.push(SurfaceOp::Sew { gap: 0.0 });
    let sewn = build(tray).unwrap();
    assert_eq!(walked_one_way(&sewn.mesh), 0);
}
