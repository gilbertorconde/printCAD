//! Points and tangents along a path by arc length: what is repeated along
//! a curve stands at them.

use kernel_api::{
    BooleanOp, ExtrudeTermination, KernelQueries, Profile, ProfilePlane, ProfileSegment,
    ProfileWire, SolidOp, SweepKind,
};
use kernel_ogeom::QUERIES;

use super::common::chain;

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6)
}

/// A line 10 mm along X, then a quarter turn of radius 10 up to (20, 10).
fn line_then_quarter() -> ProfileWire {
    let s = std::f64::consts::FRAC_1_SQRT_2 * 10.0;
    ProfileWire::new(vec![
        ProfileSegment::Line {
            start: [0.0, 0.0],
            end: [10.0, 0.0],
        },
        ProfileSegment::Arc {
            start: [10.0, 0.0],
            mid: [10.0 + s, 10.0 - s],
            end: [20.0, 10.0],
        },
    ])
}

#[test]
fn stations_along_a_wire_are_spaced_by_its_length() {
    let quarter = std::f64::consts::FRAC_PI_2 * 10.0;
    let path = QUERIES
        .wire_frames(
            &ProfilePlane::default(),
            &line_then_quarter(),
            &[0.0, 0.5 * 10.0 / (10.0 + quarter), 1.0],
        )
        .expect("frames along the wire");
    assert!(
        (path.length - (10.0 + quarter)).abs() < 1e-6,
        "{}",
        path.length
    );
    assert!(!path.closed);
    let [start, middle, end] = path.frames[..] else {
        panic!("three frames");
    };
    assert!(close(start.point, [0.0, 0.0, 0.0]));
    assert!(close(start.tangent, [1.0, 0.0, 0.0]));
    assert!(close(middle.point, [5.0, 0.0, 0.0]), "{:?}", middle.point);
    assert!(close(end.point, [20.0, 10.0, 0.0]), "{:?}", end.point);
    assert!(close(end.tangent, [0.0, 1.0, 0.0]), "{:?}", end.tangent);
}

#[test]
fn a_station_past_the_end_is_refused() {
    let wire = line_then_quarter();
    assert!(
        QUERIES
            .wire_frames(&ProfilePlane::default(), &wire, &[1.5])
            .is_err()
    );
}

#[test]
fn a_circle_closes_and_its_stations_go_round_it_evenly() {
    let circle = ProfileWire::new(vec![ProfileSegment::Circle {
        center: [0.0, 0.0],
        radius: 5.0,
    }]);
    let path = QUERIES
        .wire_frames(&ProfilePlane::default(), &circle, &[0.0, 0.25, 0.5])
        .expect("frames round the circle");
    assert!(path.closed);
    assert!((path.length - std::f64::consts::TAU * 5.0).abs() < 1e-6);
    let quarter = path.frames[1].point;
    let half = path.frames[2].point;
    let first = path.frames[0].point;
    let gap = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt();
    assert!((gap(first, half) - 10.0).abs() < 1e-6, "opposite ends");
    assert!((gap(first, quarter) - 5.0 * std::f64::consts::SQRT_2).abs() < 1e-6);
}

#[test]
fn stations_along_a_solid_edge_follow_it_where_the_shape_stands() {
    let square = ProfileWire::new(
        [[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]]
            .iter()
            .enumerate()
            .map(|(i, &start)| ProfileSegment::Line {
                start,
                end: [[30.0, 0.0], [30.0, 30.0], [0.0, 30.0], [0.0, 0.0]][i],
            })
            .collect(),
    );
    let blob = chain(&[SolidOp::Sweep {
        profile: Profile {
            plane: ProfilePlane::default(),
            wires: vec![square],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 10.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    }])
    .brep_blob;
    // The top edge along X at the back.
    let path = QUERIES
        .edge_frames(&blob, [12.0, 30.0, 10.0], &[0.0, 0.5, 1.0])
        .expect("frames along the edge");
    assert!((path.length - 30.0).abs() < 1e-6, "{}", path.length);
    let middle = path.frames[1];
    assert!(
        close(middle.point, [15.0, 30.0, 10.0]),
        "{:?}",
        middle.point
    );
    assert!(
        (middle.tangent[0].abs() - 1.0).abs() < 1e-9,
        "{:?}",
        middle.tangent
    );
    let ends = [path.frames[0].point[0], path.frames[2].point[0]];
    assert!(ends.contains(&0.0) && ends.contains(&30.0), "{ends:?}");
    assert!(
        QUERIES
            .edge_frames(&blob, [15.0, 15.0, 5.0], &[0.5])
            .is_err(),
        "no edge near the middle of the block"
    );
}
