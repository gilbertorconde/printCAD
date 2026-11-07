//! Exact bounds oracles.
//!
//! Dense point sampling is independent of the analytic derivative-root search.
//! Check two containments to distinguish different errors:
//!
//! * Samples inside analytic bounds reject endpoint-only underestimation.
//! * Analytic bounds inside sample bounds reject full-carrier overestimation.
//!
//! Exact quadrant arcs can hide both errors. Random signed sweeps and explicit
//! distinguishing scenes exercise cases where those bounds must differ.

use glam::DVec2;
use proptest::prelude::*;

use crate::bbox::{
    Bbox, arc_bbox, circle_bbox, ellipse_arc_bbox, hyperbola_arc_bbox, parabola_arc_bbox,
};
use crate::curves::{ellipse_arc_sweep, ellipse_point, hyperbola_point, parabola_point};
use crate::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
use crate::world::{WorldLength, WorldPoint};

const SAMPLES: usize = 4000;

fn pt(x: f64, y: f64) -> WorldPoint {
    WorldPoint::new(x, y)
}

fn len(value: f64) -> WorldLength {
    WorldLength::new(value)
}

/// Independent bounds from dense point sampling.
fn sampled_bbox(eval: impl Fn(f64) -> WorldPoint, from: f64, to: f64) -> Bbox {
    let step = (to - from) / SAMPLES as f64;
    Bbox::from_points((0..=SAMPLES).map(|i| eval(from + step * i as f64))).unwrap()
}

/// Scale-dependent comparison tolerance for sagitta between adjacent samples.
fn tol(bbox: Bbox) -> WorldLength {
    let size = bbox.size().0;
    len(1e-3 * size.max_element().max(1.0))
}

/// Require analytic bounds to be neither smaller nor larger than the carrier.
#[track_caller]
fn assert_matches_sampling(analytic: Bbox, sampled: Bbox) {
    let eps = tol(analytic).max(tol(sampled));
    assert!(
        analytic.contains(sampled.min(), eps) && analytic.contains(sampled.max(), eps),
        "analytic bounds are smaller than the curve: analytic={analytic:?}, sampled={sampled:?}"
    );
    assert!(
        sampled.contains(analytic.min(), eps) && sampled.contains(analytic.max(), eps),
        "analytic bounds are larger than the curve: analytic={analytic:?}, sampled={sampled:?}"
    );
}

// ---------------------------------------------------------------------------
// Bounds type
// ---------------------------------------------------------------------------

/// An empty set has no bounds; a zero substitute would expand unions to the origin.
#[test]
fn an_empty_set_has_no_bbox() {
    assert_eq!(Bbox::from_points(std::iter::empty()), None);
    assert_eq!(Bbox::union_all([None, None]), None);
}

/// A union skips absent bounds without discarding its other terms.
#[test]
fn union_skips_absent_boxes() {
    let b = Bbox::from_point(pt(3.0, 4.0));
    assert_eq!(Bbox::union_all([None, Some(b), None]), Some(b));
}

/// Public world wrappers also occur in transparent wire data. Bbox has private
/// ordered bounds: constructors must refuse broken values before min/max can
/// silently discard NaN, or a negative radius can invert the bounds.
#[test]
fn bbox_refuses_nonfinite_points_and_invalid_radii_at_its_public_boundary() {
    let good = Bbox::from_point(pt(1.0, 2.0));
    for invalid in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
        for bad in [
            WorldPoint(DVec2::new(invalid, 0.0)),
            WorldPoint(DVec2::new(0.0, invalid)),
        ] {
            assert!(std::panic::catch_unwind(|| Bbox::from_point(bad)).is_err());
            assert!(std::panic::catch_unwind(|| good.with_point(bad)).is_err());
            assert!(std::panic::catch_unwind(|| Bbox::from_points([pt(1.0, 2.0), bad])).is_err());
        }
        // Even a full sweep still requires a finite start direction.
        for (start, sweep) in [
            (invalid, 0.0),
            (invalid, std::f64::consts::TAU),
            (0.0, invalid),
        ] {
            assert!(
                std::panic::catch_unwind(|| arc_bbox(
                    pt(0.0, 0.0),
                    len(10.0),
                    Angle(start),
                    Angle(sweep)
                ))
                .is_err()
            );
        }
    }
    let negative: WorldLength = serde_json::from_str("-10.0").unwrap();
    for radius in [
        negative,
        WorldLength(f64::NAN),
        WorldLength(f64::NEG_INFINITY),
        WorldLength(f64::INFINITY),
    ] {
        assert!(std::panic::catch_unwind(|| circle_bbox(pt(0.0, 0.0), radius)).is_err());
        for sweep in [0.0, 1.0, std::f64::consts::TAU] {
            assert!(
                std::panic::catch_unwind(|| arc_bbox(
                    pt(0.0, 0.0),
                    radius,
                    Angle(0.0),
                    Angle(sweep)
                ))
                .is_err()
            );
        }
    }
    assert_eq!(
        circle_bbox(pt(1e6, -1e6), len(0.0)),
        Bbox::from_point(pt(1e6, -1e6))
    );
}

// ---------------------------------------------------------------------------
// Circular arcs
// ---------------------------------------------------------------------------

/// Crossing the top quadrant extends above both endpoints, exposing endpoint-only bounds.
#[test]
fn an_arc_crossing_a_quadrant_sticks_out_past_its_endpoints() {
    let center = WorldPoint::ORIGIN;
    let radius = len(10.0);
    // CCW from 45° to 135°: endpoints at y = 7.07, arc top at y = 10.
    let bbox = arc_bbox(
        center,
        radius,
        Angle(std::f64::consts::FRAC_PI_4),
        Angle(std::f64::consts::FRAC_PI_2),
    );
    assert!(
        (bbox.max().0.y - 10.0).abs() < 1e-9,
        "arc top must reach its radius above the endpoints: {bbox:?}"
    );
    assert!(
        (bbox.min().0.y - 7.071067811865475).abs() < 1e-9,
        "endpoints determine the bottom: {bbox:?}"
    );
}

/// An arc reaching no quadrant has endpoint bounds, smaller than its carrier circle.
#[test]
fn an_arc_inside_one_quadrant_is_just_its_endpoints() {
    let center = WorldPoint::ORIGIN;
    let radius = len(10.0);
    let bbox = arc_bbox(center, radius, Angle(0.2), Angle(0.5));
    let start = WorldPoint::from_dvec2(center.0 + DVec2::from_angle(0.2) * radius.0);
    let end = WorldPoint::from_dvec2(center.0 + DVec2::from_angle(0.7) * radius.0);
    let by_ends = Bbox::from_point(start).with_point(end);
    assert!((bbox.min() - by_ends.min()).length().0 < 1e-9);
    assert!((bbox.max() - by_ends.max()).length().0 < 1e-9);
    assert_ne!(bbox, circle_bbox(center, radius));
}

/// A full turn retains every quadrant despite equal endpoint directions.
#[test]
fn a_full_sweep_is_the_whole_circle() {
    let center = pt(1.0, -2.0);
    let bbox = arc_bbox(center, len(5.0), Angle(0.3), Angle(std::f64::consts::TAU));
    assert_eq!(bbox, circle_bbox(center, len(5.0)));
}

/// Opposite sweep signs can join the same endpoints through different bounds.
#[test]
fn sweep_sign_picks_a_different_arc() {
    let center = WorldPoint::ORIGIN;
    let radius = len(10.0);
    let start = Angle(std::f64::consts::FRAC_PI_4);
    let ccw = arc_bbox(center, radius, start, Angle(std::f64::consts::FRAC_PI_2));
    let cw = arc_bbox(
        center,
        radius,
        start,
        Angle(-3.0 * std::f64::consts::FRAC_PI_2),
    );
    assert_ne!(ccw, cw);
    // The short upper arc misses the circle bottom; the long arc reaches it.
    assert!(ccw.min().0.y > 0.0);
    assert!((cw.min().0.y + radius.0).abs() < 1e-9);
}

proptest! {
    #[test]
    fn arc_bbox_agrees_with_dense_sampling(
        cx in -50.0f64..50.0,
        cy in -50.0f64..50.0,
        radius in 0.5f64..50.0,
        start in -50.0f64..50.0,
        sweep in -6.2f64..6.2,
    ) {
        prop_assume!(sweep.abs() > 1e-3);
        let center = pt(cx, cy);
        let analytic = arc_bbox(center, len(radius), Angle(start), Angle(sweep));
        let sampled = sampled_bbox(
            |t| WorldPoint::from_dvec2(center.0 + DVec2::from_angle(t) * radius),
            start,
            start + sweep,
        );
        assert_matches_sampling(analytic, sampled);
    }

    #[test]
    fn ellipse_arc_bbox_agrees_with_dense_sampling(
        cx in -30.0f64..30.0,
        cy in -30.0f64..30.0,
        fx in -20.0f64..20.0,
        fy in -20.0f64..20.0,
        semi_minor in 0.5f64..20.0,
        t_start in -50.0f64..50.0,
        t_end in -50.0f64..50.0,
        clockwise in any::<bool>(),
    ) {
        let center = pt(cx, cy);
        let focus1 = pt(cx + fx, cy + fy);
        prop_assume!(focus1.distance(center).0 > 0.5);
        let b = len(semi_minor);
        let (t_start, t_end) = (Angle(t_start), Angle(t_end));
        prop_assume!(ellipse_arc_sweep(t_start, t_end, clockwise).radians().abs() > 1e-3);

        let analytic = ellipse_arc_bbox(center, focus1, b, t_start, t_end, clockwise);
        let sweep = ellipse_arc_sweep(t_start, t_end, clockwise).radians();
        let sampled = sampled_bbox(
            |t| ellipse_point(center, focus1, b, Angle(t)),
            t_start.radians(),
            t_start.radians() + sweep,
        );
        assert_matches_sampling(analytic, sampled);
    }

    #[test]
    fn hyperbola_arc_bbox_agrees_with_dense_sampling(
        cx in -30.0f64..30.0,
        cy in -30.0f64..30.0,
        fx in -20.0f64..20.0,
        fy in -20.0f64..20.0,
        semi_minor in 0.5f64..10.0,
        t_start in -2.0f64..2.0,
        t_end in -2.0f64..2.0,
        // Exercise both disconnected hyperbola branches.
        negative_branch in any::<bool>(),
    ) {
        let center = pt(cx, cy);
        let focus1 = pt(cx + fx, cy + fy);
        // The focus lies outside: c > b is required for a real major semiaxis.
        prop_assume!(focus1.distance(center).0 > semi_minor + 0.5);
        prop_assume!((t_end - t_start).abs() > 1e-3);
        let b = len(semi_minor);
        let (t_start, t_end) = (HyperbolaParam(t_start), HyperbolaParam(t_end));
        let branch = if negative_branch {
            HyperbolaBranch::Negative
        } else {
            HyperbolaBranch::Positive
        };

        let analytic = hyperbola_arc_bbox(center, focus1, b, t_start, t_end, branch);
        let sampled = sampled_bbox(
            |t| hyperbola_point(center, focus1, b, HyperbolaParam(t), branch),
            t_start.get(),
            t_end.get(),
        );
        assert_matches_sampling(analytic, sampled);
    }

    #[test]
    fn parabola_arc_bbox_agrees_with_dense_sampling(
        vx in -30.0f64..30.0,
        vy in -30.0f64..30.0,
        fx in -20.0f64..20.0,
        fy in -20.0f64..20.0,
        u_start in -20.0f64..20.0,
        u_end in -20.0f64..20.0,
    ) {
        let vertex = pt(vx, vy);
        let focus = pt(vx + fx, vy + fy);
        prop_assume!(focus.distance(vertex).0 > 0.5);
        prop_assume!((u_end - u_start).abs() > 1e-3);
        let (u_start, u_end) = (ParabolaParam(u_start), ParabolaParam(u_end));

        let analytic = parabola_arc_bbox(vertex, focus, u_start, u_end);
        let sampled = sampled_bbox(
            |u| parabola_point(vertex, focus, ParabolaParam(u)),
            u_start.get(),
            u_end.get(),
        );
        assert_matches_sampling(analytic, sampled);
    }
}

/// A parabolic span across the vertex must include that interior extremum.
#[test]
fn a_parabola_arc_spanning_the_vertex_reaches_it() {
    let vertex = WorldPoint::ORIGIN;
    let focus = pt(1.0, 0.0);
    let bbox = parabola_arc_bbox(vertex, focus, ParabolaParam(-4.0), ParabolaParam(4.0));
    assert!(
        bbox.min().0.x.abs() < 1e-9,
        "the vertex is the leftmost point: {bbox:?}"
    );
    assert!(
        (bbox.max().0.x - 4.0).abs() < 1e-9,
        "endpoints: u²/4f = 4: {bbox:?}"
    );
}

/// Inverse hyperbola parameterization must reproduce the input point, rather
/// than merely preserve parameter ordering.
#[test]
fn hyperbola_param_is_a_real_inverse() {
    let center = pt(2.0, -1.0);
    let focus1 = pt(9.0, 3.0);
    let b = len(3.0);
    for branch in [HyperbolaBranch::Positive, HyperbolaBranch::Negative] {
        for t in [-1.7f64, -0.5, 0.0, 0.9, 2.1] {
            let point = hyperbola_point(center, focus1, b, HyperbolaParam(t), branch);
            let back = crate::curves::hyperbola_param(center, focus1, b, point);
            assert!(
                (back.get() - t).abs() < 1e-9,
                "{branch:?}: t={t} -> point -> t={} (expected the same t)",
                back.get()
            );
            // Recover the branch independently from geometry; t does not encode it.
            assert_eq!(
                crate::curves::hyperbola_branch(center, focus1, b, point),
                Some(branch),
                "point at t={t} was assigned to the wrong branch"
            );
        }
    }
}

/// Deterministic angular boundaries, with starts beyond 360° and signed sweeps
/// bounded to one turn for these circular arc cases.
#[test]
fn angular_boundaries_of_circle_and_ellipse_bboxes_match_sampling() {
    let center = pt(3.0, -4.0);
    let focus = pt(9.0, -4.0);
    for boundary in [180.0f64, 270.0, 360.0, 450.0, 720.0] {
        for epsilon in [-0.001f64, 0.0, 0.001] {
            for sign in [-1.0, 1.0] {
                let start = sign * (boundary + epsilon).to_radians();
                let zero = arc_bbox(center, len(10.0), Angle(start), Angle(0.0));
                assert_eq!(zero.min(), zero.max(), "zero sweep is not a full turn");
                assert!(
                    zero.min()
                        .distance(pt(3.0 + 10.0 * start.cos(), -4.0 + 10.0 * start.sin()))
                        .0
                        < 1e-12
                );
                for span in [180.0f64, 270.0, 360.0] {
                    let sweep = sign * span.to_radians();
                    assert_matches_sampling(
                        arc_bbox(center, len(10.0), Angle(start), Angle(sweep)),
                        sampled_bbox(
                            |t| pt(3.0 + 10.0 * t.cos(), -4.0 + 10.0 * t.sin()),
                            start,
                            start + sweep,
                        ),
                    );
                    // Equal elliptic parameters denote a full turn. Pass exactly
                    // equal values: rounded start + τ could leave a near-zero remainder.
                    let end = if span == 360.0 { start } else { start + sweep };
                    assert_matches_sampling(
                        ellipse_arc_bbox(
                            center,
                            focus,
                            len(4.0),
                            Angle(start),
                            Angle(end),
                            sign < 0.0,
                        ),
                        sampled_bbox(
                            |t| ellipse_point(center, focus, len(4.0), Angle(t)),
                            start,
                            start + sweep,
                        ),
                    );
                }
            }
        }
    }
}
