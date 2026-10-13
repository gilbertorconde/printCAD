//! Probes: what a reference standing on a solid finds on it, asked part
//! way through a chain or of a finished snapshot.

use kernel_api::{
    BooleanOp, ChainProbe, FaceSurface, KernelQueries, Placement, PrimitiveKind, ProbeAnswer,
    ShapeProbe, SolidOp, TessellationSettings,
};
use kernel_ogeom::OgeomKernel;

use super::common::chain;

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < tol)
}

fn parallel(a: [f64; 3], b: [f64; 3]) -> bool {
    (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).abs() > 1.0 - 1e-6
}

fn cylinder(radius: f64, height: f64, op: BooleanOp) -> SolidOp {
    SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op,
    }
}

fn cube(size: f64, op: BooleanOp) -> SolidOp {
    SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: size,
            width: size,
            height: size,
        },
        placement: Placement::default(),
        op,
    }
}

fn probing(ops: &[SolidOp], probes: &[ChainProbe]) -> Vec<Result<ProbeAnswer, String>> {
    OgeomKernel::default()
        .execute_solid_chain_probing(ops, &TessellationSettings::default(), None, probes)
        .expect("the chain builds")
        .probes
}

#[test]
fn a_face_probe_finds_the_tangent_point_and_the_cylinder() {
    let answers = probing(
        &[cylinder(5.0, 12.0, BooleanOp::NewSolid)],
        &[ChainProbe {
            after_op: 1,
            probe: ShapeProbe::Face {
                name: 0,
                point: [7.0, 0.0, 6.0],
                normal: [1.0, 0.0, 0.0],
            },
        }],
    );
    let Ok(ProbeAnswer::Face {
        point,
        normal,
        surface,
    }) = answers[0]
    else {
        panic!("{:?}", answers[0]);
    };
    assert!(close(point, [5.0, 0.0, 6.0], 1e-6), "{point:?}");
    assert!(close(normal, [1.0, 0.0, 0.0], 1e-6), "{normal:?}");
    let FaceSurface::Cylinder { radius, axis, .. } = surface else {
        panic!("{surface:?}");
    };
    assert!((radius - 5.0).abs() < 1e-5);
    assert!(parallel(axis.map(f64::from), [0.0, 0.0, 1.0]));
}

#[test]
fn an_edge_probe_finds_a_rim_circle_and_a_straight_edge_s_ends() {
    let answers = probing(
        &[cylinder(5.0, 12.0, BooleanOp::NewSolid)],
        &[ChainProbe {
            after_op: 1,
            probe: ShapeProbe::Edge {
                along: None,
                faces: [0, 0],
                point: [5.0, 0.0, 12.0],
                direction: [0.0, 1.0, 0.0],
            },
        }],
    );
    let Ok(ProbeAnswer::Edge {
        circle: Some(circle),
        ..
    }) = answers[0]
    else {
        panic!("{:?}", answers[0]);
    };
    assert!(close(circle.centre, [0.0, 0.0, 12.0], 1e-6));
    assert!((circle.radius - 5.0).abs() < 1e-6);
    assert!(parallel(circle.normal, [0.0, 0.0, 1.0]));

    let answers = probing(
        &[cube(10.0, BooleanOp::NewSolid)],
        &[ChainProbe {
            after_op: 1,
            probe: ShapeProbe::Edge {
                along: Some(0.25),
                faces: [0, 0],
                point: [4.0, 0.0, 0.0],
                direction: [1.0, 0.0, 0.0],
            },
        }],
    );
    let Ok(ProbeAnswer::Edge {
        point,
        direction,
        start,
        end,
        middle,
        circle: None,
        along: Some((quarter, tangent)),
    }) = answers[0]
    else {
        panic!("{:?}", answers[0]);
    };
    assert!(close(point, [4.0, 0.0, 0.0], 1e-6), "{point:?}");
    assert!(parallel(direction, [1.0, 0.0, 0.0]));
    let ends = [start, end];
    assert!(ends.iter().any(|p| close(*p, [0.0; 3], 1e-6)), "{ends:?}");
    assert!(
        ends.iter().any(|p| close(*p, [10.0, 0.0, 0.0], 1e-6)),
        "{ends:?}"
    );
    assert!(close(middle, [5.0, 0.0, 0.0], 1e-6), "{middle:?}");
    // A quarter of the way from its start, whichever end that is.
    let from_start = [start[0] + (end[0] - start[0]) * 0.25, 0.0, 0.0];
    assert!(close(quarter, from_start, 1e-6), "{quarter:?}");
    assert!(parallel(tangent, [1.0, 0.0, 0.0]));
}

#[test]
fn a_probe_is_asked_of_the_solid_where_it_stands_in_the_history() {
    let answers = probing(
        &[
            cube(10.0, BooleanOp::NewSolid),
            cylinder(20.0, 3.0, BooleanOp::Fuse),
        ],
        &[
            ChainProbe {
                after_op: 0,
                probe: ShapeProbe::Mass,
            },
            ChainProbe {
                after_op: 1,
                probe: ShapeProbe::Mass,
            },
            ChainProbe {
                after_op: 2,
                probe: ShapeProbe::Mass,
            },
        ],
    );
    assert!(answers[0].is_err(), "nothing stands before the first op");
    let Ok(ProbeAnswer::Mass { centre, .. }) = answers[1] else {
        panic!("{:?}", answers[1]);
    };
    assert!(close(centre, [5.0, 5.0, 5.0], 1e-6), "{centre:?}");
    let Ok(ProbeAnswer::Mass { centre, .. }) = answers[2] else {
        panic!("{:?}", answers[2]);
    };
    assert!(!close(centre, [5.0, 5.0, 5.0], 0.1), "{centre:?}");
}

#[test]
fn the_axes_of_inertia_are_a_right_handed_frame_smallest_moment_first() {
    let answers = probing(
        &[cylinder(5.0, 12.0, BooleanOp::NewSolid)],
        &[ChainProbe {
            after_op: 1,
            probe: ShapeProbe::Mass,
        }],
    );
    let Ok(ProbeAnswer::Mass { centre, axes }) = answers[0] else {
        panic!("{:?}", answers[0]);
    };
    assert!(close(centre, [0.0, 0.0, 6.0], 1e-6), "{centre:?}");
    // This cylinder turns most easily about its own axis.
    assert!(parallel(axes[0], [0.0, 0.0, 1.0]), "{axes:?}");
    let [a, b, c] = axes;
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    assert!(close(cross, c, 1e-9), "{axes:?}");
}

#[test]
fn a_snapshot_answers_the_same_probe() {
    let result = chain(&[cylinder(5.0, 12.0, BooleanOp::NewSolid)]);
    let answer = kernel_ogeom::QUERIES
        .probe(
            &result.brep_blob,
            &ShapeProbe::Face {
                name: 0,
                point: [0.0, 6.0, 6.0],
                normal: [0.0, 1.0, 0.0],
            },
        )
        .expect("the probe is answered");
    let ProbeAnswer::Face { point, normal, .. } = answer else {
        panic!("{answer:?}");
    };
    assert!(close(point, [0.0, 5.0, 6.0], 1e-6), "{point:?}");
    assert!(close(normal, [0.0, 1.0, 0.0], 1e-6), "{normal:?}");
}
