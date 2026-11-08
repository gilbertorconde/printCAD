//! Rounding oracles: tangencies and degenerate conics up to coordinates 1e6
//! must not panic. Negativity beyond rounding and inconsistent semiaxes must
//! panic with the failed proof's name.

use crate::arc::chord_step_for_sagitta;
use crate::curves::{ellipse_semi_minor, hyperbola_axes, hyperbola_semi_minor};
use crate::intersections::circle_circle_intersections;
use crate::roundoff::sqrt_of_proven_nonnegative;
use crate::world::{WorldLength, WorldPoint};

/// Origin shifts through the supported maximum coordinate scale 1e6.
const OFFSETS: [f64; 3] = [0.0, 1e3, 1e6];

fn directions() -> impl Iterator<Item = glam::DVec2> {
    (0..9).map(|k| glam::DVec2::from_angle(0.7 * f64::from(k)))
}

#[test]
fn rounding_below_zero_is_absorbed() {
    assert_eq!(sqrt_of_proven_nonnegative(-1e-17, 1.0, "test"), 0.0);
    assert_eq!(sqrt_of_proven_nonnegative(4.0, 1.0, "test"), 2.0);
}

#[test]
#[should_panic(expected = "named site: -0.000001 is negative beyond rounding")]
fn a_negative_value_beyond_rounding_is_refused_by_name() {
    sqrt_of_proven_nonnegative(-1e-6, 1.0, "named site");
}

/// Internal/external tangency at all shifts and directions, including small d
/// where height error grows as r³/d. Rounded centers may yield no points;
/// every returned point must lie on both circles, without panicking.
#[test]
fn tangent_circles_never_panic_at_far_coordinates() {
    let pairs: [(f64, f64); 5] = [
        (1.0, 1.0),
        (1e-3, 1e3),
        (1e3, 1e-3),
        (1e3, 1e3 - 1e-3),
        (5.0, 3.0),
    ];
    for offset in OFFSETS {
        for dir in directions() {
            for (r1, r2) in pairs {
                for dist in [r1 + r2, (r1 - r2).abs()] {
                    if dist == 0.0 {
                        continue;
                    }
                    let c1 = WorldPoint::new(offset, -offset);
                    let c2 = WorldPoint::from_dvec2(c1.0 + dir * dist);
                    let points = circle_circle_intersections(
                        c1,
                        WorldLength::new(r1),
                        c2,
                        WorldLength::new(r2),
                    );
                    // Point error reflects coordinate rounding and tangency
                    // conditioning: height is sqrt(error in h²).
                    let tolerance = 1e-6 * (1.0 + offset + r1 + r2);
                    for p in points {
                        assert!(
                            (p.distance(c1).0 - r1).abs() <= tolerance
                                && (p.distance(c2).0 - r2).abs() <= tolerance,
                            "offset {offset}, r1 {r1}, r2 {r2}, dist {dist}: {p:?} is off the circles"
                        );
                    }
                }
            }
        }
    }
}

/// At b = 0 an ellipse rim is between the foci (a = c); a hyperbola rim is
/// beyond a focus (|d1 − d2| = 2c). Require near-zero b without panic at every shift.
#[test]
fn degenerate_conic_semi_minor_never_panics_at_far_coordinates() {
    for offset in OFFSETS {
        for dir in directions() {
            let center = WorldPoint::new(offset, offset);
            let c = 2.5;
            let focus = WorldPoint::from_dvec2(center.0 + dir * c);
            for along in [-0.9, -0.3, 0.0, 0.4, 0.99] {
                let rim = WorldPoint::from_dvec2(center.0 + dir * (c * along));
                let b = ellipse_semi_minor(center, focus, rim);
                assert!(
                    b.0 <= 1e-4 * (1.0 + offset),
                    "ellipse, offset {offset}: b = {b:?}"
                );
            }
            for beyond in [1.0, 1.5, 7.0] {
                let rim = WorldPoint::from_dvec2(center.0 + dir * (c * beyond));
                let b = hyperbola_semi_minor(center, focus, rim);
                assert!(
                    b.0 <= 1e-4 * (1.0 + offset),
                    "hyperbola, offset {offset}: b = {b:?}"
                );
                hyperbola_axes(center, focus, b).unwrap();
            }
        }
    }
}

/// A supplied b > c cannot define a hyperbola and must not silently become a = 0.
#[test]
#[should_panic(expected = "hyperbola_axes: c² − b²")]
fn hyperbola_axes_refuses_a_semi_minor_beyond_the_focal_distance() {
    let center = WorldPoint::new(0.0, 0.0);
    let focus = WorldPoint::new(2.0, 0.0);
    hyperbola_axes(center, focus, WorldLength::new(2.001));
}

/// Independently check θ = 4·asin(√(s/2r)) through the resulting chord's sagitta.
/// Error at least the radius caps the step at a quarter turn.
#[test]
fn chord_step_meets_the_sagitta_and_caps_at_a_quarter_turn() {
    for (radius, error) in [(10.0, 0.01), (1e6, 1e-3), (1e-3, 1e-6), (5.0, 1.0)] {
        let step =
            chord_step_for_sagitta(WorldLength::new(radius), WorldLength::new(error)).radians();
        // r·(1 − cos(θ/2)) = 2r·sin²(θ/4); the second form avoids cancellation.
        let sagitta = 2.0 * radius * (step / 4.0).sin().powi(2);
        assert!(
            (sagitta - error).abs() <= 1e-12 * error,
            "r {radius}, e {error}: sagitta {sagitta}"
        );
    }
    for error in [5.0, 10.0, 1e9] {
        let step = chord_step_for_sagitta(WorldLength::new(5.0), WorldLength::new(error)).radians();
        assert_eq!(step, std::f64::consts::FRAC_PI_2);
    }
    assert_eq!(
        chord_step_for_sagitta(WorldLength::new(5.0), WorldLength::ZERO).radians(),
        0.0
    );
}

#[test]
fn chord_step_keeps_its_scale_at_finite_extremes() {
    let expected = chord_step_for_sagitta(WorldLength::new(1.0), WorldLength::new(0.01)).0;
    for radius in [f64::MIN_POSITIVE, 1e-200, 1e200, f64::MAX] {
        let actual =
            chord_step_for_sagitta(WorldLength::new(radius), WorldLength::new(radius * 0.01)).0;
        assert!(
            (actual - expected).abs() < 1e-14,
            "radius {radius}: {actual}"
        );
    }
    assert_eq!(
        chord_step_for_sagitta(WorldLength::new(f64::MAX), WorldLength::new(f64::MAX)).0,
        std::f64::consts::FRAC_PI_2
    );
    let tiny_step = chord_step_for_sagitta(
        WorldLength::new(f64::MAX),
        WorldLength::new(f64::from_bits(1)),
    )
    .0;
    assert!(
        tiny_step.is_finite() && tiny_step > 0.0,
        "representable step was lost: {tiny_step}"
    );
}

#[test]
fn chord_step_refuses_invalid_values_before_saturation() {
    for invalid in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY, -1.0] {
        assert!(
            std::panic::catch_unwind(|| chord_step_for_sagitta(
                WorldLength(invalid),
                WorldLength::new(1.0)
            ))
            .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| chord_step_for_sagitta(
                WorldLength::new(1.0),
                WorldLength(invalid)
            ))
            .is_err()
        );
    }
    assert!(
        std::panic::catch_unwind(|| chord_step_for_sagitta(WorldLength::ZERO, WorldLength::ZERO))
            .is_err()
    );
}
