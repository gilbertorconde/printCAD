//! Imported-to-native spline oracles. Fixed reference points were evaluated by
//! an independent implementation; neither evaluator under test generates them.

use super::foreign::{ForeignSpline, NotNative};
use super::{BSplineCurve, SplineDefinition, SplineParam};
use crate::world::{WorldLength, WorldPoint};

const MERGE: WorldLength = WorldLength::new(1e-4);

fn pts(raw: &[(f64, f64)]) -> Vec<WorldPoint> {
    raw.iter().map(|&(x, y)| WorldPoint::new(x, y)).collect()
}

const OPEN_POLES: [(f64, f64); 5] = [
    (0.0, 0.0),
    (10.0, 20.0),
    (30.0, 20.0),
    (40.0, 0.0),
    (55.0, 5.0),
];
/// Open cubic, knots 0 ×4, 0.5, 1 ×4.
const OPEN_REFERENCE: [(f64, f64, f64); 9] = [
    (0.0, 0.0, 0.0),
    (0.125, 7.4609375, 11.484375),
    (0.25, 14.6875, 16.875),
    (0.375, 21.4453125, 17.578125),
    (0.5, 27.5, 15.0),
    (0.625, 32.8515625, 10.625),
    (0.75, 38.4375, 6.25),
    (0.875, 45.4296875, 3.75),
    (1.0, 55.0, 5.0),
];
/// The same cubic with second pole weight two.
const RATIONAL_REFERENCE: [(f64, f64, f64); 9] = [
    (0.0, 0.0, 0.0),
    (0.125, 8.302872062663186, 14.308093994778067),
    (0.25, 12.941176470588236, 18.03921568627451),
    (0.375, 17.855227882037532, 18.33780160857909),
    (0.5, 24.0, 16.0),
    (0.625, 30.671378091872793, 11.519434628975265),
    (0.75, 37.57575757575758, 6.666666666666667),
    (0.875, 45.29182879377432, 3.813229571984436),
    (1.0, 55.0, 5.0),
];
const CLOSED_POLES: [(f64, f64); 5] = [
    (100.0, 0.0),
    (110.0, 15.0),
    (125.0, 10.0),
    (130.0, -5.0),
    (115.0, -15.0),
];
/// Periodic cubic, knots 0, 0.2 … 1 of multiplicity one.
const CLOSED_REFERENCE: [(f64, f64, f64); 9] = [
    (0.0, 110.83333333333334, 11.666666666666668),
    (0.125, 119.01204427083334, 11.292317708333336),
    (0.25, 125.49479166666667, 5.559895833333335),
    (0.375, 127.138671875, -2.569986979166666),
    (0.5, 122.08333333333333, -9.375),
    (0.625, 113.13313802083333, -10.333658854166664),
    (0.75, 105.5078125, -3.684895833333334),
    (0.875, 104.81119791666669, 5.44921875),
    (1.0, 110.83333333333334, 11.666666666666668),
];

fn open(knots: Vec<f64>, weights: Vec<f64>) -> ForeignSpline {
    ForeignSpline {
        degree: 3,
        knots,
        poles: pts(&OPEN_POLES),
        weights,
    }
}

/// Unrolled periodic record: extend knots by three steps on either side and
/// repeat the first three poles at the end.
fn closed() -> ForeignSpline {
    let mut poles = pts(&CLOSED_POLES);
    poles.extend_from_within(0..3);
    ForeignSpline {
        degree: 3,
        knots: (-3..=8).map(|i| f64::from(i) * 0.2).collect(),
        poles,
        weights: vec![1.0; 8],
    }
}

/// Native curve at file parameter t in [0, 1], mapped linearly to its domain.
fn native_at(curve: &BSplineCurve, t: f64) -> WorldPoint {
    let (a, b) = curve.domain();
    curve.point(SplineParam(a.0 + t * (b.0 - a.0)))
}

fn assert_on(expected: &[(f64, f64, f64)], at: impl Fn(f64) -> WorldPoint) {
    for &(t, x, y) in expected {
        let p = at(t);
        assert!(
            p.distance(WorldPoint::new(x, y)).0 < 1e-9,
            "t = {t}: {p:?}, reference ({x}, {y})"
        );
    }
}

#[test]
fn the_rational_de_boor_agrees_with_reference() {
    assert_on(&OPEN_REFERENCE, |t| {
        open(vec![0., 0., 0., 0., 0.5, 1., 1., 1., 1.], vec![1.0; 5]).point(t)
    });
    assert_on(&RATIONAL_REFERENCE, |t| {
        open(
            vec![0., 0., 0., 0., 0.5, 1., 1., 1., 1.],
            vec![1., 2., 1., 1., 1.],
        )
        .point(t)
    });
    assert_on(&CLOSED_REFERENCE, |t| closed().point(t));
}

#[test]
fn a_uniform_clamped_spline_is_native_open_whatever_its_knot_scale() {
    for (knots, weights) in [
        (vec![0., 0., 0., 0., 0.5, 1., 1., 1., 1.], vec![1.0; 5]),
        // Knots scaled to 0..6 and equal nonunit weights describe the same curve.
        (vec![0., 0., 0., 0., 3., 6., 6., 6., 6.], vec![2.5; 5]),
    ] {
        let native = open(knots, weights).native(MERGE).unwrap();
        assert_eq!(native.points, pts(&OPEN_POLES));
        assert!(!native.shape.periodic);
        assert_eq!(native.shape.definition, SplineDefinition::Poles);
        assert_eq!(native.shape.degree.get(), 3);
        let curve = BSplineCurve::build(native.shape, &native.points).unwrap();
        assert_on(&OPEN_REFERENCE, |t| native_at(&curve, t));
    }
}

#[test]
fn an_unrolled_periodic_spline_is_native_closed_on_its_distinct_poles() {
    let native = closed().native(MERGE).unwrap();
    assert!(native.shape.periodic);
    assert_eq!(native.points, pts(&CLOSED_POLES));
    let curve = BSplineCurve::build(native.shape, &native.points).unwrap();
    assert_on(&CLOSED_REFERENCE, |t| native_at(&curve, t));
}

#[test]
fn rounded_knots_of_a_file_still_name_the_uniform_spline() {
    // Six-digit DXF knots round 1/3 to 0.333333, moving the carrier about 1e-5 mm.
    let mut poles = pts(&OPEN_POLES);
    poles.push(WorldPoint::new(60.0, 20.0));
    let spline = ForeignSpline {
        degree: 3,
        knots: vec![0., 0., 0., 0., 0.333333, 0.666667, 1., 1., 1., 1.],
        poles: poles.clone(),
        weights: vec![1.0; 6],
    };
    assert_eq!(spline.native(MERGE).unwrap().points, poles);
    // A thousandfold larger knot displacement defines a different curve.
    let moved = ForeignSpline {
        knots: vec![0., 0., 0., 0., 0.3333, 0.6667, 1., 1., 1., 1.],
        ..spline
    };
    assert_eq!(moved.native(MERGE), Err(NotNative::Knots));
}

#[test]
fn what_the_sketch_cannot_hold_is_named() {
    let knots = vec![0., 0., 0., 0., 0.5, 1., 1., 1., 1.];
    assert_eq!(
        open(knots.clone(), vec![1., 2., 1., 1., 1.]).native(MERGE),
        Err(NotNative::Rational)
    );
    assert_eq!(
        open(vec![0., 0., 0., 0., 0.3, 1., 1., 1., 1.], vec![1.0; 5]).native(MERGE),
        Err(NotNative::Knots)
    );
    // Repeated poles with nonuniform periodic knots do not match the native form.
    let mut lopsided = closed();
    lopsided.knots[5] = 0.25;
    assert_eq!(lopsided.native(MERGE), Err(NotNative::Knots));
    let degree6 = ForeignSpline {
        degree: 6,
        knots: [vec![0.0; 7], vec![1.0; 7]].concat(),
        poles: (0..7).map(|i| WorldPoint::new(f64::from(i), 0.0)).collect(),
        weights: vec![1.0; 7],
    };
    assert_eq!(degree6.native(MERGE), Err(NotNative::Degree(6)));
    for broken in [
        open(knots[..8].to_vec(), vec![1.0; 5]),
        open(vec![0., 0., 0., 0., 1., 0.5, 1., 1., 1.], vec![1.0; 5]),
        open(vec![0.0; 9], vec![1.0; 5]),
        open(knots.clone(), vec![1., 0., 1., 1., 1.]),
        open(knots, vec![1., f64::NAN, 1., 1., 1.]),
    ] {
        assert_eq!(
            broken.native(MERGE),
            Err(NotNative::Malformed),
            "{broken:?}"
        );
    }
}
