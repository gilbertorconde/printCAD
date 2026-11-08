//! Independent spline properties: clamped endpoints/tangents, uniform cubic
//! knot values (P[i−1] + 4·P[i] + P[i+1])/6, degree-one polylines,
//! interpolation, convex hull containment and affine invariance.

use glam::DVec2;

use crate::bspline::{
    BSplineCurve, BSplineError, SplineDefinition, SplineDegree, SplineParam, SplineShape,
};
use crate::world::WorldPoint;

fn shape(definition: SplineDefinition, degree: u32, periodic: bool) -> SplineShape {
    SplineShape {
        definition,
        degree: SplineDegree::new(degree),
        periodic,
    }
}

fn pts(raw: &[(f64, f64)]) -> Vec<WorldPoint> {
    raw.iter().map(|&(x, y)| WorldPoint::new(x, y)).collect()
}

fn close(a: WorldPoint, b: WorldPoint, tol: f64) {
    assert!(a.distance(b).0 < tol, "{a:?} vs {b:?}");
}

const POLES: [(f64, f64); 6] = [
    (0.0, 0.0),
    (10.0, 20.0),
    (30.0, 25.0),
    (45.0, -5.0),
    (60.0, 10.0),
    (70.0, 0.0),
];

#[test]
fn clamped_spline_starts_and_ends_on_its_end_poles_tangent_to_the_end_legs() {
    for degree in 1..=5 {
        let poles = pts(&POLES);
        let c = BSplineCurve::build(shape(SplineDefinition::Poles, degree, false), &poles).unwrap();
        close(c.start(), poles[0], 1e-12);
        close(c.end(), poles[5], 1e-12);
        let (a, b) = c.domain();
        let d0 = c.derivative(a).0.normalize();
        let d1 = c.derivative(b).0.normalize();
        assert!(
            d0.perp_dot((poles[1] - poles[0]).0.normalize()).abs() < 1e-12,
            "degree {degree}"
        );
        assert!(d0.dot((poles[1] - poles[0]).0) > 0.0);
        assert!(
            d1.perp_dot((poles[5] - poles[4]).0.normalize()).abs() < 1e-12,
            "degree {degree}"
        );
    }
}

#[test]
fn degree_one_is_the_control_polygon() {
    let poles = pts(&POLES);
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 1, false), &poles).unwrap();
    // Five equal spans: knot i/5 meets pole i; a span midpoint meets its arm midpoint.
    for (i, pole) in poles.iter().enumerate() {
        close(c.point(SplineParam(i as f64 / 5.0)), *pole, 1e-12);
    }
    close(
        c.point(SplineParam(0.5)),
        poles[2].midpoint(poles[3]),
        1e-12,
    );
}

#[test]
fn few_points_lower_the_degree_to_what_they_define() {
    let two = pts(&[(0.0, 0.0), (10.0, 4.0)]);
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 3, false), &two).unwrap();
    assert_eq!(c.degree(), 1);
    close(c.point(SplineParam(0.5)), WorldPoint::new(5.0, 2.0), 1e-12);
    let three = pts(&[(0.0, 0.0), (5.0, 10.0), (10.0, 0.0)]);
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 3, false), &three).unwrap();
    assert_eq!(c.degree(), 2);
    // Quadratic Bezier: B(1/2) = (P0 + 2·P1 + P2) / 4.
    close(c.point(SplineParam(0.5)), WorldPoint::new(5.0, 5.0), 1e-12);
}

#[test]
fn uniform_periodic_cubic_sits_at_the_one_four_one_average_on_every_knot_and_closes_smoothly() {
    let poles = pts(&[
        (0.0, 0.0),
        (20.0, 0.0),
        (20.0, 10.0),
        (0.0, 10.0),
        (-5.0, 5.0),
    ]);
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 3, true), &poles).unwrap();
    let n = poles.len();
    let (a, b) = c.domain();
    assert_eq!(b.0 - a.0, n as f64);
    for i in 0..n {
        // At span start a + i, weights of P[i], P[i+1], P[i+2] are 1:4:1.
        let p = |k: usize| poles[k % n].0;
        let expected = (p(i) + p(i + 1) * 4.0 + p(i + 2)) / 6.0;
        close(
            c.point(SplineParam(a.0 + i as f64)),
            WorldPoint::from_dvec2(expected),
            1e-12,
        );
    }
    close(c.start(), c.end(), 1e-12);
    let d0 = c.derivative(a).0;
    let d1 = c.derivative(b).0;
    assert!((d0 - d1).length() < 1e-9, "seam tangent {d0} vs {d1}");
    let samples = c.sample(8);
    assert_eq!(samples.first(), samples.last());
}

#[test]
fn every_sample_stays_inside_the_hull_of_the_poles() {
    for periodic in [false, true] {
        for degree in 1..=5 {
            let poles = pts(&POLES);
            let c = BSplineCurve::build(shape(SplineDefinition::Poles, degree, periodic), &poles)
                .unwrap();
            for p in c.sample(16) {
                assert!(p.0.x >= -1e-9 && p.0.x <= 70.0 + 1e-9, "{p:?}");
                assert!(p.0.y >= -5.0 - 1e-9 && p.0.y <= 25.0 + 1e-9, "{p:?}");
            }
        }
    }
}

#[test]
fn interpolation_passes_through_every_point_in_order() {
    let through = pts(&[
        (0.0, 0.0),
        (8.0, 15.0),
        (30.0, 18.0),
        (31.0, -4.0),
        (60.0, 2.0),
    ]);
    for periodic in [false, true] {
        for degree in 1..=5 {
            let c =
                BSplineCurve::build(shape(SplineDefinition::Through, degree, periodic), &through)
                    .unwrap();
            let mut last = f64::NEG_INFINITY;
            for (i, p) in through.iter().enumerate() {
                let (t, _, d) = c.nearest(*p);
                assert!(
                    d.0 < 1e-7,
                    "degree {degree} periodic {periodic}: point {i} off by {}",
                    d.0
                );
                if !periodic {
                    assert!(t.0 > last, "points pass in order");
                    last = t.0;
                }
            }
            if !periodic {
                close(c.start(), through[0], 1e-12);
                close(c.end(), through[4], 1e-12);
            }
        }
    }
}

#[test]
fn even_degree_closed_interpolation_with_an_even_number_of_points_does_not_degenerate() {
    // Even-count quadratic interpolation is singular at knots, but not span midpoints.
    let square = pts(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    for degree in [2, 3] {
        let c =
            BSplineCurve::build(shape(SplineDefinition::Through, degree, true), &square).unwrap();
        for p in &square {
            assert!(c.nearest(*p).2.0 < 1e-7);
        }
    }
}

#[test]
fn closed_interpolation_of_a_circle_stays_on_the_circle() {
    let n = 12;
    let ring: Vec<WorldPoint> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            WorldPoint::new(10.0 * a.cos() + 3.0, 10.0 * a.sin() - 2.0)
        })
        .collect();
    let c = BSplineCurve::build(shape(SplineDefinition::Through, 3, true), &ring).unwrap();
    let worst = c
        .sample(16)
        .iter()
        .map(|p| ((p.0 - DVec2::new(3.0, -2.0)).length() - 10.0).abs())
        .fold(0.0, f64::max);
    assert!(worst < 0.01, "radial deviation {worst}");
}

#[test]
fn collinear_points_give_a_straight_curve() {
    let line = pts(&[(0.0, 0.0), (1.0, 2.0), (4.0, 8.0), (5.0, 10.0)]);
    let c = BSplineCurve::build(shape(SplineDefinition::Through, 3, false), &line).unwrap();
    for p in c.sample(16) {
        assert!((p.0.y - 2.0 * p.0.x).abs() < 1e-9, "{p:?}");
    }
}

#[test]
fn moving_every_point_alike_moves_the_curve_alike() {
    for definition in [SplineDefinition::Poles, SplineDefinition::Through] {
        for periodic in [false, true] {
            let s = shape(definition, 3, periodic);
            let base = pts(&POLES);
            // Rotate 30° and translate: an affine map of the points.
            let (sin, cos) = 30f64.to_radians().sin_cos();
            let map = |p: WorldPoint| {
                WorldPoint::new(
                    cos * p.0.x - sin * p.0.y + 7.0,
                    sin * p.0.x + cos * p.0.y - 3.0,
                )
            };
            let moved: Vec<WorldPoint> = base.iter().copied().map(map).collect();
            let a = BSplineCurve::build(s, &base).unwrap();
            let b = BSplineCurve::build(s, &moved).unwrap();
            for (p, q) in a.sample(8).into_iter().zip(b.sample(8)) {
                close(map(p), q, 1e-9);
            }
        }
    }
}

#[test]
fn nearest_projects_onto_the_curve() {
    let c = BSplineCurve::build(
        shape(SplineDefinition::Poles, 1, false),
        &pts(&[(0.0, 0.0), (10.0, 0.0)]),
    )
    .unwrap();
    let (t, p, d) = c.nearest(WorldPoint::new(3.0, 4.0));
    // A flat squared-distance minimum determines t only to about √ε, but distance precisely.
    assert!((t.0 - 0.3).abs() < 1e-6);
    close(p, WorldPoint::new(3.0, 0.0), 1e-6);
    assert!((d.0 - 4.0).abs() < 1e-9);
    // Beyond an endpoint, the endpoint is nearest.
    let (_, p, _) = c.nearest(WorldPoint::new(-5.0, 1.0));
    close(p, WorldPoint::new(0.0, 0.0), 1e-9);
}

#[test]
fn the_points_that_cannot_make_a_spline_are_refused() {
    let one = pts(&[(0.0, 0.0)]);
    assert_eq!(
        BSplineCurve::build(shape(SplineDefinition::Poles, 3, false), &one),
        Err(BSplineError::TooFewPoints { needed: 2, got: 1 })
    );
    let two = pts(&[(0.0, 0.0), (1.0, 0.0)]);
    assert_eq!(
        BSplineCurve::build(shape(SplineDefinition::Poles, 3, true), &two),
        Err(BSplineError::TooFewPoints { needed: 3, got: 2 })
    );
    let doubled = pts(&[(0.0, 0.0), (5.0, 5.0), (5.0, 5.0), (9.0, 0.0)]);
    assert_eq!(
        BSplineCurve::build(shape(SplineDefinition::Through, 3, false), &doubled),
        Err(BSplineError::CoincidentThroughPoints {
            first: 1,
            second: 2
        })
    );
    let seam = pts(&[(0.0, 0.0), (5.0, 5.0), (9.0, 0.0), (0.0, 0.0)]);
    assert_eq!(
        BSplineCurve::build(shape(SplineDefinition::Through, 3, true), &seam),
        Err(BSplineError::CoincidentThroughPoints {
            first: 0,
            second: 3
        })
    );
    // Repeated poles are valid and pull the carrier toward the repeated point.
    assert!(BSplineCurve::build(shape(SplineDefinition::Poles, 3, false), &doubled).is_ok());
}
