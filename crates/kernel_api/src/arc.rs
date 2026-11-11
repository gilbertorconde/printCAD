//! What a [`ProfileSegment::Arc`](crate::ProfileSegment::Arc)'s three
//! points mean, so the sketcher's regions, its drawing tools and the kernel
//! adapter agree on which triples make an arc and where it runs.

use std::f64::consts::TAU;

/// Below this sine of the angle at the first point, between the chords to
/// the other two, three points are collinear. A ratio, so it judges three
/// points a micron apart as it judges three a metre apart; the kernel's
/// three-point circle refuses at the same ratio.
pub const COLLINEAR_SINE: f64 = 1e-12;

/// The largest radius, in millimetres, a three-point circle may have. The
/// kernel holds a circle as a centre and a radius, and from about 1e8 mm
/// on (sooner the further the arc lies from the origin) the circle it
/// builds through three points misses them by more than its tolerance;
/// this leaves a tenth of that.
pub const MAX_RADIUS: f64 = 1e7;

/// The circular arc from `start` through a middle point to `end`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreePointArc {
    pub center: [f64; 2],
    pub radius: f64,
    /// The angle of `start` about the centre.
    pub start_angle: f64,
    /// Radians from `start` to `end`, positive counter-clockwise, through
    /// the middle point.
    pub sweep: f64,
}

impl ThreePointArc {
    /// The arc through the three points; `None` when [`circumcircle`] has
    /// no circle through them.
    pub fn through(start: [f64; 2], mid: [f64; 2], end: [f64; 2]) -> Option<ThreePointArc> {
        let (center, radius) = circumcircle(start, mid, end)?;
        let angle = |p: [f64; 2]| (p[1] - center[1]).atan2(p[0] - center[0]);
        let (a0, am, a1) = (angle(start), angle(mid), angle(end));
        let ccw = (a1 - a0).rem_euclid(TAU);
        let sweep = if (am - a0).rem_euclid(TAU) <= ccw {
            ccw
        } else {
            ccw - TAU
        };
        Some(ThreePointArc {
            center,
            radius,
            start_angle: a0,
            sweep,
        })
    }

    /// The point a fraction `t` of the sweep from the start.
    pub fn at(&self, t: f64) -> [f64; 2] {
        let angle = self.start_angle + self.sweep * t;
        [
            self.center[0] + self.radius * angle.cos(),
            self.center[1] + self.radius * angle.sin(),
        ]
    }
}

/// The centre and radius of the circle through three points; `None` when
/// they are collinear within [`COLLINEAR_SINE`], two of them coincide, or
/// the circle is wider than [`MAX_RADIUS`].
pub fn circumcircle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let len = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() <= 2.0 * COLLINEAR_SINE * len(a, b) * len(a, c) {
        return None;
    }
    let sq = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let center = [
        (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d,
        (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d,
    ];
    let radius = len(center, a);
    (radius <= MAX_RADIUS).then_some((center, radius))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arcs either way round, short and long, from a few degrees to nearly
    /// a whole turn, run from their start through their middle point to
    /// their end.
    #[test]
    fn an_arc_runs_from_its_start_through_its_middle_to_its_end() {
        let c = [3.0, -2.0];
        let r = 7.0;
        let on = |a: f64| [c[0] + r * a.cos(), c[1] + r * a.sin()];
        let dist = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        for (a0, sweep) in [(0.3, 0.1), (1.0, 2.5), (-2.0, 3.5), (0.5, 6.2)] {
            for sweep in [sweep, -sweep] {
                let (start, mid, end) = (on(a0), on(a0 + sweep / 2.0), on(a0 + sweep));
                let arc = ThreePointArc::through(start, mid, end).unwrap();
                assert!((arc.sweep - sweep).abs() < 1e-12, "{sweep}: {}", arc.sweep);
                for i in 0..=10 {
                    let t = i as f64 / 10.0;
                    assert!(
                        dist(arc.at(t), on(a0 + sweep * t)) < 1e-12,
                        "{sweep} at {t}"
                    );
                }
            }
        }
    }

    /// Collinearity is a ratio, the same shape an arc or not whatever its
    /// size; the radius is a length; coincident points are never an arc.
    #[test]
    fn three_points_make_a_circle_by_shape_up_to_a_radius() {
        for size in [1e-7, 1e-6, 1e-5] {
            let bulged =
                |sine: f64| circumcircle([0.0, 0.0], [size / 2.0, sine * size / 2.0], [size, 0.0]);
            assert!(bulged(2e-12).is_some(), "{size}");
            assert!(bulged(5e-13).is_none(), "{size}");
        }
        let flat = |radius: f64| {
            let (chord, half) = (10.0, (5.0 / radius).asin());
            circumcircle(
                [0.0, 0.0],
                [chord / 2.0, radius * (1.0 - half.cos())],
                [chord, 0.0],
            )
        };
        assert!(flat(0.5 * MAX_RADIUS).is_some());
        assert!(flat(2.0 * MAX_RADIUS).is_none());
        assert!(circumcircle([1.0, 1.0], [1.0, 1.0], [2.0, 0.0]).is_none());
    }
}
