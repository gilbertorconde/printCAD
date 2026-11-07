use std::f64::consts::TAU;

use super::{angle_on_signed_arc, closest_on_arc, closest_on_circle, dist_to_arc};
use crate::types::Angle;
use crate::world::{WorldLength, WorldPoint};

#[test]
fn signed_membership_checks_exact_boundaries_and_their_neighbors() {
    let eps = 1e-8;
    for turns in [-2.0, 0.0, 2.0] {
        for sign in [-1.0, 1.0] {
            for degrees in [0.0_f64, 180.0, 270.0, 360.0, 450.0, 720.0] {
                let start = turns * TAU;
                let sweep = sign * degrees.to_radians();
                for (offset, inside) in [
                    (-eps, degrees >= 360.0),
                    (0.0, true),
                    (eps, degrees > 0.0),
                    (degrees.to_radians() - eps, degrees > 0.0),
                    (degrees.to_radians() + eps, degrees >= 360.0),
                ] {
                    assert_eq!(
                        angle_on_signed_arc(
                            Angle(start),
                            Angle(sweep),
                            Angle(start + sign * offset)
                        ),
                        inside,
                        "start={start}, sweep={sweep}, offset={offset}"
                    );
                }
                // Exact endpoints use the same representation as stored arcs;
                // half and quarter turns remain exact under these full-turn shifts.
                assert!(angle_on_signed_arc(
                    Angle(start),
                    Angle(sweep),
                    Angle(start + sweep)
                ));
            }
        }
    }
}

#[test]
fn projection_from_the_center_returns_a_point_on_the_curve() {
    for center in [WorldPoint::ORIGIN, WorldPoint::new(1e6, -1e6)] {
        let radius = WorldLength::new(10.0);
        assert_eq!(
            closest_on_circle(center, center, radius),
            WorldPoint::new(center.0.x + 10.0, center.0.y)
        );
        for sign in [-1.0, 1.0] {
            for degrees in [0.0_f64, 180.0, 270.0, 360.0, 450.0, 720.0] {
                let start = Angle(std::f64::consts::PI);
                let result = closest_on_arc(
                    center,
                    center,
                    radius,
                    start,
                    Angle(sign * degrees.to_radians()),
                );
                assert!(
                    result
                        .distance(WorldPoint::new(center.0.x - 10.0, center.0.y))
                        .0
                        < 1e-9
                );
            }
        }
    }
}

#[test]
fn projection_keeps_representable_directions_when_their_square_overflows_or_underflows() {
    for scale in [1e-200, 1.0, 1e200] {
        let result = closest_on_circle(
            WorldPoint::new(3.0 * scale, 4.0 * scale),
            WorldPoint::ORIGIN,
            WorldLength::new(10.0),
        );
        assert!(
            result.distance(WorldPoint::new(6.0, 8.0)).0 < 1e-12,
            "{scale}: {result:?}"
        );
    }
}

#[test]
fn zero_sweep_projects_and_measures_only_its_start() {
    let center = WorldPoint::new(1e6, -1e6);
    let start = WorldPoint::new(center.0.x + 10.0, center.0.y);
    let p = WorldPoint::new(center.0.x - 10.0, center.0.y);
    for sweep in [0.0, -0.0] {
        assert_eq!(
            closest_on_arc(p, center, WorldLength::new(10.0), Angle(0.0), Angle(sweep)),
            start
        );
        assert_eq!(
            dist_to_arc(p, center, start, start, Angle(0.0), Angle(sweep)),
            WorldLength::new(20.0)
        );
    }
    for sweep in [-2.0 * TAU, -TAU, TAU, 2.0 * TAU] {
        assert_eq!(
            closest_on_arc(p, center, WorldLength::new(10.0), Angle(0.0), Angle(sweep)),
            p
        );
        assert_eq!(
            dist_to_arc(p, center, start, start, Angle(0.0), Angle(sweep)),
            WorldLength::ZERO
        );
    }
}

#[test]
fn invalid_curve_inputs_are_refused_before_they_can_become_a_projection() {
    let center = WorldPoint::ORIGIN;
    let p = WorldPoint::new(1.0, 0.0);
    let radius = WorldLength::new(10.0);
    for invalid in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
        for (start, sweep) in [(invalid, 0.0), (invalid, TAU), (0.0, invalid)] {
            assert!(
                std::panic::catch_unwind(|| closest_on_arc(
                    p,
                    center,
                    radius,
                    Angle(start),
                    Angle(sweep)
                ))
                .is_err()
            );
            assert!(
                std::panic::catch_unwind(|| {
                    closest_on_arc(center, center, radius, Angle(start), Angle(sweep))
                })
                .is_err()
            );
            assert!(
                std::panic::catch_unwind(|| dist_to_arc(
                    p,
                    center,
                    p,
                    p,
                    Angle(start),
                    Angle(sweep)
                ))
                .is_err()
            );
        }
        let broken = WorldPoint(glam::DVec2::new(invalid, 0.0));
        assert!(std::panic::catch_unwind(|| closest_on_circle(broken, center, radius)).is_err());
        assert!(std::panic::catch_unwind(|| closest_on_circle(p, broken, radius)).is_err());
    }
    // Transparent world wrappers can arrive unchecked from a wire/old file.
    for invalid in [-1.0, f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
        assert!(
            std::panic::catch_unwind(|| closest_on_circle(p, center, WorldLength(invalid)))
                .is_err()
        );
    }
}
