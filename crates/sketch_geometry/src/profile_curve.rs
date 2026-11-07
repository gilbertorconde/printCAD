//! Analytic profile curves and exact B-spline windows, walked by arc length.

use glam::DVec2;

use crate::bspline::SplineParam;
use crate::bspline_window::SplineWindow;
use crate::trim_curve::TrimCurve;
use crate::world::{WorldLength, WorldOffset, WorldPoint};

/// Profile edge curve in its own travel direction.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileCurve {
    Analytic(TrimCurve),
    Spline(SplineWindow),
}

impl ProfileCurve {
    /// Whether this is a circle or a full turn of a periodic spline.
    pub fn closed(&self) -> bool {
        match self {
            Self::Analytic(curve) => curve.closed(),
            Self::Spline(spline) => spline.closed(),
        }
    }

    pub fn length(&self) -> WorldLength {
        match self {
            Self::Analytic(curve) => curve.length(),
            Self::Spline(spline) => spline.length(),
        }
    }

    pub fn pos_at(&self, s: WorldOffset) -> WorldPoint {
        match self {
            Self::Analytic(curve) => curve.pos_at(s),
            Self::Spline(spline) => spline.point(spline.param_at(s)),
        }
    }

    /// Arc length from the start to the curve point nearest `p`.
    pub fn project(&self, p: WorldPoint) -> WorldOffset {
        match self {
            Self::Analytic(curve) => curve.project(p),
            Self::Spline(spline) => spline.length_to(spline.nearest(p).0),
        }
    }

    /// Arc lengths of all passes through cut point `p` within the tolerance.
    pub fn cut_offsets(&self, p: WorldPoint, tolerance: WorldLength) -> Vec<WorldOffset> {
        super::profile_spline_contacts::passes_through(self, p, tolerance)
    }

    pub fn distance(&self, p: WorldPoint) -> WorldLength {
        match self {
            Self::Analytic(curve) => curve.pos_at(curve.project(p)).distance(p),
            Self::Spline(spline) => spline.nearest(p).1,
        }
    }

    /// On a closed curve `hi` may cross the seam.
    pub fn sub_curve(&self, lo: WorldOffset, hi: WorldOffset) -> Self {
        match self {
            Self::Analytic(curve) => Self::Analytic(curve.sub_curve(lo, hi)),
            Self::Spline(spline) => {
                assert!(
                    hi > lo,
                    "sub_curve: empty spline interval ({}, {})",
                    lo.0,
                    hi.0
                );
                // Unwrapped parameters preserve a window crossing the seam.
                let window = [spline.param_at(lo), spline.param_at(hi)];
                Self::Spline(SplineWindow::on_window(spline.curve().clone(), window))
            }
        }
    }

    /// Unit tangent in travel direction and signed curvature.
    pub fn direction(&self, at: WorldOffset) -> (DVec2, f64) {
        match self {
            Self::Analytic(curve) => analytic_direction(*curve, at),
            Self::Spline(spline) => {
                let t = spline.param_at(at);
                let d1 = spline.curve().derivative(t).0;
                let d2 = spline.curve().second_derivative(t).0;
                let window = spline.window();
                let span = window[1].0 - window[0].0;
                let speed = d1.length();
                if speed > 1e-9 * spline.length().0 / span {
                    return (d1 / speed, d1.perp_dot(d2) / (speed * speed * speed));
                }
                // A zero derivative uses the local chord in travel direction.
                let step = 1e-6 * span;
                let (from, to) = if t.0 + step <= window[1].0 {
                    (t.0, t.0 + step)
                } else {
                    (t.0 - step, t.0)
                };
                let chord = spline.point(SplineParam(to)).0 - spline.point(SplineParam(from)).0;
                (chord.normalize(), 0.0)
            }
        }
    }
}

fn analytic_direction(curve: TrimCurve, at: WorldOffset) -> (DVec2, f64) {
    match curve {
        TrimCurve::Segment { a, b } => ((b.0 - a.0).normalize(), 0.0),
        TrimCurve::Arc { radius, .. } | TrimCurve::Circle { radius, .. } => {
            let sign = match curve {
                TrimCurve::Arc { geom, .. } => geom.sweep_angle.radians().signum(),
                _ => 1.0,
            };
            let angle = curve.angle_at(at).normalized();
            (
                DVec2::from_angle(angle.radians()).perp() * sign,
                sign / radius.0,
            )
        }
    }
}
