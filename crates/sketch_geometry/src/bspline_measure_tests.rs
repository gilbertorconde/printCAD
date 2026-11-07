//! Quadrature-independent oracles: linear/polyline lengths, dense-sample limits,
//! finite-difference derivatives, circle curvature and kernel-piece carrier agreement.

use glam::DVec2;

use crate::bspline::{BSplineCurve, SplineDefinition, SplineDegree, SplineParam, SplineShape};
use crate::world::{WorldLength, WorldPoint};

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

const POLES: [(f64, f64); 6] = [
    (0.0, 0.0),
    (10.0, 20.0),
    (30.0, 25.0),
    (45.0, -5.0),
    (60.0, 10.0),
    (70.0, 0.0),
];

fn every_curve() -> Vec<BSplineCurve> {
    let mut out = Vec::new();
    for definition in [SplineDefinition::Poles, SplineDefinition::Through] {
        for periodic in [false, true] {
            for degree in 1..=5 {
                out.push(
                    BSplineCurve::build(shape(definition, degree, periodic), &pts(&POLES)).unwrap(),
                );
            }
        }
    }
    out
}

fn polyline_length(points: &[WorldPoint]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1]).0).sum()
}

#[test]
fn a_degree_one_spline_is_as_long_as_its_control_polygon() {
    let poles = pts(&POLES);
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 1, false), &poles).unwrap();
    let (a, b) = c.domain();
    assert!((c.arc_length(a, b).0 - polyline_length(&poles)).abs() < 1e-9);
    let mut ring = poles.clone();
    ring.push(poles[0]);
    let closed = BSplineCurve::build(shape(SplineDefinition::Poles, 1, true), &poles).unwrap();
    let (a, b) = closed.domain();
    assert!((closed.arc_length(a, b).0 - polyline_length(&ring)).abs() < 1e-9);
}

#[test]
fn arc_length_is_the_limit_of_dense_chords_for_every_shape() {
    for c in every_curve() {
        let (a, b) = c.domain();
        let exact = c.arc_length(a, b).0;
        // Chord deficit scales as h². Richardson extrapolation with step ratio
        // four (error ratio sixteen) gives a limit without quadrature. Sharp
        // quintic bends leave about 3e-6 error in the dense sample.
        let coarse = polyline_length(&c.sample(4096));
        let dense = polyline_length(&c.sample(16384));
        let limit = dense + (dense - coarse) / 15.0;
        assert!(dense <= exact + 1e-9, "{exact} vs {dense}");
        assert!((exact - limit).abs() < 1e-8 * exact, "{exact} vs {limit}");
    }
}

#[test]
fn length_and_parameter_invert_each_other_across_the_seam() {
    for c in every_curve() {
        let (a, b) = c.domain();
        let total = c.arc_length(a, b);
        // Interior window start, crossing the seam for a periodic curve.
        let from = SplineParam(a.0 + (b.0 - a.0) * 0.37);
        let span = match c.period() {
            Some(period) => SplineParam(from.0 + period),
            None => b,
        };
        let window_length = c.arc_length(from, span);
        if c.is_periodic() {
            assert!(
                (window_length.0 - total.0).abs() < 1e-9 * total.0,
                "a full turn from anywhere"
            );
        }
        let table = c.arc_table([from, span]);
        assert!((table.length().0 - window_length.0).abs() < 1e-9 * total.0);
        for k in 0..=10 {
            let s = WorldLength::new(window_length.0 * k as f64 / 10.0);
            let t = c.param_at_length(&table, s);
            assert!(t.0 >= from.0 - 1e-12 && t.0 <= span.0 + 1e-12);
            // Check the inverse both through the table and direct window quadrature.
            let back = c.arc_length(from, t);
            assert!(
                (back.0 - s.0).abs() < 1e-9 * (1.0 + total.0),
                "{} vs {}",
                back.0,
                s.0
            );
            assert!((c.length_at(&table, t).0 - s.0).abs() < 1e-9 * (1.0 + total.0));
        }
    }
}

#[test]
fn derivatives_match_finite_differences() {
    for c in every_curve() {
        let (a, b) = c.domain();
        for k in 1..20 {
            // Probe between knots, where low-degree derivatives are continuous.
            let t = a.0 + (b.0 - a.0) * (k as f64 + 0.31) / 20.0;
            let h = 1e-6 * (b.0 - a.0);
            let fd1 = (c.point(SplineParam(t + h)).0 - c.point(SplineParam(t - h)).0) / (2.0 * h);
            let d1 = c.derivative(SplineParam(t)).0;
            assert!(
                (fd1 - d1).length() < 1e-4 * (1.0 + d1.length()),
                "{fd1:?} vs {d1:?}"
            );
            if c.degree() >= 3 {
                let fd2 = (c.derivative(SplineParam(t + h)).0 - c.derivative(SplineParam(t - h)).0)
                    / (2.0 * h);
                let d2 = c.second_derivative(SplineParam(t)).0;
                assert!(
                    (fd2 - d2).length() < 1e-3 * (1.0 + d2.length()),
                    "{fd2:?} vs {d2:?}"
                );
            }
        }
        if c.degree() == 1 {
            assert_eq!(c.second_derivative(a).0, DVec2::ZERO);
        }
    }
}

#[test]
fn curvature_of_a_closed_interpolated_circle_is_its_inverse_radius() {
    let n = 24;
    let ring: Vec<WorldPoint> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            WorldPoint::new(10.0 * a.cos(), 10.0 * a.sin())
        })
        .collect();
    let c = BSplineCurve::build(shape(SplineDefinition::Through, 3, true), &ring).unwrap();
    let (a, b) = c.domain();
    for k in 0..48 {
        let t = SplineParam(a.0 + (b.0 - a.0) * k as f64 / 48.0);
        let d1 = c.derivative(t).0;
        let d2 = c.second_derivative(t).0;
        let kappa = d1.perp_dot(d2) / d1.length().powi(3);
        // Counterclockwise travel has positive curvature.
        assert!((kappa - 0.1).abs() < 2e-3, "curvature {kappa}");
    }
    assert!((c.arc_length(a, b).0 - std::f64::consts::TAU * 10.0).abs() < 1e-3);
}

#[test]
fn nearest_in_a_window_stays_in_the_window() {
    let line = BSplineCurve::build(
        shape(SplineDefinition::Poles, 1, false),
        &pts(&[(0.0, 0.0), (10.0, 0.0)]),
    )
    .unwrap();
    let window = [SplineParam(0.5), SplineParam(0.8)];
    let (t, p, _) = line.nearest_in(WorldPoint::new(2.0, 3.0), window);
    assert!((t.0 - 0.5).abs() < 1e-9);
    assert!(p.distance(WorldPoint::new(5.0, 0.0)).0 < 1e-8);
    let (t, p, d) = line.nearest_in(WorldPoint::new(6.0, -1.0), window);
    assert!((t.0 - 0.6).abs() < 1e-6 && p.distance(WorldPoint::new(6.0, 0.0)).0 < 1e-6);
    assert!((d.0 - 1.0).abs() < 1e-9);
}

#[test]
fn a_closed_round_curve_turns_twice_in_each_axis() {
    let square = pts(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 3, true), &square).unwrap();
    let (a, b) = c.domain();
    for axis in 0..2 {
        let turns = c.axis_turns(axis, [a, b]);
        assert_eq!(turns.len(), 2, "axis {axis}: {turns:?}");
        for t in turns {
            assert!(c.derivative(t).0[axis].abs() < 1e-9);
        }
    }
}

#[test]
fn a_kernel_piece_traces_the_same_points_either_way() {
    for c in every_curve() {
        let (a, b) = c.domain();
        let windows = match c.period() {
            // Seam crossing and a full turn starting inside the domain.
            Some(period) => {
                vec![
                    [
                        SplineParam(a.0 + 0.7 * period),
                        SplineParam(a.0 + 1.3 * period),
                    ],
                    [
                        SplineParam(a.0 + 0.5 * period),
                        SplineParam(a.0 + 1.5 * period),
                    ],
                ]
            }
            None => {
                vec![
                    [a, b],
                    [
                        SplineParam(a.0 + 0.2 * (b.0 - a.0)),
                        SplineParam(a.0 + 0.9 * (b.0 - a.0)),
                    ],
                ]
            }
        };
        for window in windows {
            for reversed in [false, true] {
                let piece = c.piece(window, reversed);
                let [r0, r1] = piece.range;
                // A clamped piece has its endpoints at the first and last poles.
                assert!(piece.point(r0).distance(piece.poles[0]).0 < 1e-12);
                assert!(piece.point(r1).distance(*piece.poles.last().unwrap()).0 < 1e-12);
                assert_eq!(piece.knots.len(), piece.poles.len() + piece.degree + 1);
                for k in 0..=8 {
                    let u = k as f64 / 8.0;
                    let source =
                        c.point(SplineParam(window[0].0 + (window[1].0 - window[0].0) * u));
                    let at = if reversed {
                        piece.point(SplineParam(r1.0 - (r1.0 - r0.0) * u))
                    } else {
                        piece.point(SplineParam(r0.0 + (r1.0 - r0.0) * u))
                    };
                    assert!(at.distance(source).0 < 1e-9, "{at:?} vs {source:?}");
                }
            }
        }
    }
}

fn ring(n: usize, radius: f64, ccw: bool) -> Vec<WorldPoint> {
    (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64 * if ccw { 1.0 } else { -1.0 };
            WorldPoint::new(radius * a.cos(), radius * a.sin())
        })
        .collect()
}

/// Circle comb teeth extend outward by κ·scale in both travel directions:
/// curvature and normal signs cancel, leaving the tip farther from the center.
#[test]
fn comb_teeth_of_a_circle_point_away_from_the_centre_in_both_windings() {
    for ccw in [true, false] {
        let c = BSplineCurve::build(
            shape(SplineDefinition::Through, 3, true),
            &ring(24, 10.0, ccw),
        )
        .unwrap();
        let teeth = c.comb(<[_; 2]>::from(c.domain()), 6);
        assert!(teeth.len() > 24, "{} teeth", teeth.len());
        for tooth in &teeth {
            let tip = tooth.base.0 + tooth.normal.0 * tooth.curvature * 50.0;
            let from_centre = tip.length();
            let base = tooth.base.0.length();
            assert!(
                (from_centre - base - 0.1 * 50.0).abs() < 0.2,
                "ccw {ccw}: base {base}, tip {from_centre}"
            );
        }
    }
}

/// Independent finite-difference curvature: chord turning angle per arc length,
/// without curve derivatives, on an open pole-defined cubic.
#[test]
fn curvature_agrees_with_the_turning_of_a_fine_polyline() {
    let c = BSplineCurve::build(shape(SplineDefinition::Poles, 3, false), &pts(&POLES)).unwrap();
    let (a, b) = c.domain();
    let h = 1e-4;
    for k in 1..20 {
        let t = a.0 + (b.0 - a.0) * k as f64 / 20.0;
        let p = |t: f64| c.point(SplineParam(t)).0;
        let (p0, p1, p2) = (p(t - h), p(t), p(t + h));
        let (e1, e2) = (p1 - p0, p2 - p1);
        let turn = e1.perp_dot(e2).atan2(e1.dot(e2));
        let expected = turn / (0.5 * (e1.length() + e2.length()));
        let got = c.curvature(SplineParam(t)).unwrap();
        assert!(
            (got - expected).abs() < 1e-4 * (1.0 + got.abs()),
            "t {t}: {got} vs {expected}"
        );
    }
}

#[test]
fn a_straight_spline_has_a_flat_comb() {
    let line = BSplineCurve::build(
        shape(SplineDefinition::Poles, 1, false),
        &pts(&[(0.0, 0.0), (10.0, 5.0)]),
    )
    .unwrap();
    let teeth = line.comb(<[_; 2]>::from(line.domain()), 4);
    assert_eq!(teeth.len(), 5);
    assert!(teeth.iter().all(|t| t.curvature == 0.0));
}
