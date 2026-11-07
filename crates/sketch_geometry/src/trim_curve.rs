//! Analytic curve spans parameterized by travelled arc length.

use glam::DVec2;

use crate::arc::ArcGeom;
use crate::types::Angle;
use crate::world::{WorldLength, WorldOffset, WorldPoint};

/// A one-dimensional point set. Closedness ([`TrimCurve::closed`]) determines
/// the topology used by the trimming algebra.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrimCurve {
    /// Open segment from a to b.
    Segment { a: WorldPoint, b: WorldPoint },
    /// Open arc: s follows the signed sweep, rather than increasing angle.
    Arc {
        center: WorldPoint,
        radius: WorldLength,
        geom: ArcGeom,
    },
    /// Closed circle, traversed counterclockwise from angle zero by convention.
    Circle {
        center: WorldPoint,
        radius: WorldLength,
    },
}

impl TrimCurve {
    /// A closed curve has no endpoints; n boundaries give n components,
    /// whereas an open curve gives n + 1.
    pub fn closed(&self) -> bool {
        matches!(self, TrimCurve::Circle { .. })
    }

    /// Total length, the upper bound of parameter s.
    pub fn length(&self) -> WorldLength {
        match self {
            TrimCurve::Segment { a, b } => a.distance(*b),
            TrimCurve::Arc { radius, geom, .. } => {
                WorldLength::new(geom.sweep_angle.radians().abs() * radius.0)
            }
            TrimCurve::Circle { radius, .. } => WorldLength::new(std::f64::consts::TAU * radius.0),
        }
    }

    /// Curve point at distance s from the start in travel direction.
    ///
    /// Requires positive length; the caller rejects degenerate curves before
    /// invoking the algebra rather than supplying a substitute point.
    pub fn pos_at(&self, s: WorldOffset) -> WorldPoint {
        match self {
            TrimCurve::Segment { a, b } => {
                let len = a.distance(*b).0;
                assert!(
                    len > 0.0,
                    "TrimCurve::pos_at requires a nondegenerate segment: {a:?} -> {b:?}"
                );
                WorldPoint::from_dvec2(a.0 + (b.0 - a.0) / len * s.0)
            }
            TrimCurve::Arc { center, radius, .. } | TrimCurve::Circle { center, radius } => {
                WorldPoint::from_dvec2(
                    center.0 + DVec2::from_angle(self.angle_at(s).radians()) * radius.0,
                )
            }
        }
    }

    /// Angle at distance s. Only circular carriers support this operation;
    /// calling it on a segment is an API error.
    pub fn angle_at(&self, s: WorldOffset) -> Angle {
        match self {
            TrimCurve::Arc { radius, geom, .. } => {
                let sign = if geom.clockwise() { -1.0 } else { 1.0 };
                Angle(geom.start_angle.radians() + sign * s.0 / radius.0)
            }
            TrimCurve::Circle { radius, .. } => Angle(s.0 / radius.0),
            TrimCurve::Segment { .. } => {
                panic!("TrimCurve::angle_at requires an angular curve, not a segment")
            }
        }
    }

    /// Arc distance corresponding to angle, inverse of [`Self::angle_at`].
    /// Converts angular intersection boundaries to the algebra's length parameter.
    pub fn s_at_angle(&self, angle: Angle) -> WorldOffset {
        match self {
            TrimCurve::Arc { radius, geom, .. } => {
                WorldOffset::new(sweep_path(geom.start_angle, geom.clockwise(), angle) * radius.0)
            }
            TrimCurve::Circle { radius, .. } => {
                WorldOffset::new(wrap_tau(angle.radians()) * radius.0)
            }
            TrimCurve::Segment { .. } => {
                panic!("TrimCurve::s_at_angle requires an angular curve, not a segment")
            }
        }
    }

    /// Nearest curve point to p, expressed as parameter s.
    ///
    /// A nonempty curve always has a nearest point. Outside an arc's angular
    /// sector choose its nearer endpoint, thereby selecting the end component.
    pub fn project(&self, p: WorldPoint) -> WorldOffset {
        match self {
            TrimCurve::Segment { a, b } => {
                let ab = b.0 - a.0;
                let len_sq = ab.length_squared();
                assert!(
                    len_sq > 0.0,
                    "TrimCurve::project requires a nondegenerate segment"
                );
                WorldOffset::new(((p.0 - a.0).dot(ab) / len_sq).clamp(0.0, 1.0) * len_sq.sqrt())
            }
            TrimCurve::Circle { center, .. } => {
                let angle = Angle::from_vec(p - *center);
                self.s_at_angle(angle)
            }
            TrimCurve::Arc { center, geom, .. } => {
                let angle = Angle::from_vec(p - *center);
                let s = self.s_at_angle(angle).0;
                let len = self.length().0;
                if s <= len {
                    return WorldOffset::new(s);
                }
                // Outside the sector, compare distances along the missing gap
                // of length full − len to choose the nearer endpoint.
                let full = std::f64::consts::TAU * self.radius_of_curvature().0;
                let past_end = s - len;
                let before_start = full - s;
                debug_assert!(!geom.sweep_angle.radians().is_nan());
                if past_end <= before_start {
                    WorldOffset::new(len)
                } else {
                    WorldOffset::ZERO
                }
            }
        }
    }

    fn radius_of_curvature(&self) -> WorldLength {
        match self {
            TrimCurve::Arc { radius, .. } | TrimCurve::Circle { radius, .. } => *radius,
            TrimCurve::Segment { .. } => panic!("a segment has no radius"),
        }
    }

    /// Materialize the interval between lo and hi as a standalone curve.
    ///
    /// Trimming returns intervals in s; joining and application consume curves.
    /// A proper piece of a closed carrier is open.
    ///
    /// Requires lo < hi and a span at most the carrier length. On a closed
    /// carrier hi may exceed that length to cross the seam.
    pub fn sub_curve(&self, lo: WorldOffset, hi: WorldOffset) -> TrimCurve {
        assert!(
            hi > lo,
            "sub_curve: empty or reversed span ({}, {}); reject zero-length components before algebra",
            lo.0,
            hi.0
        );
        match self {
            TrimCurve::Segment { .. } => TrimCurve::Segment {
                a: self.pos_at(lo),
                b: self.pos_at(hi),
            },
            TrimCurve::Arc { center, radius, .. } | TrimCurve::Circle { center, radius } => {
                // angle_at follows travel, so the resulting sweep inherits its
                // sign without comparing absolute endpoint angles.
                let start = self.angle_at(lo);
                let end = self.angle_at(hi);
                TrimCurve::Arc {
                    center: *center,
                    radius: *radius,
                    geom: ArcGeom::new(start, end - start),
                }
            }
        }
    }
}

/// Relative tolerance for joining a's end to b's start.
///
/// Scale with the carrier length so the test has the same meaning at any size.
/// Also bounds collinearity as a dimensionless sine of direction deviation.
///
/// Shared model vertices differ only by f64 rounding. At length 1000, this
/// tolerance gives 1e-6 world units, matching geometric solve precision.
const JOINT_EPS_REL: f64 = 1e-9;

/// World tolerance for a pair, scaled by the total length of the joined carrier.
fn joint_eps(a: &TrimCurve, b: &TrimCurve) -> WorldLength {
    (a.length() + b.length()) * JOINT_EPS_REL
}

/// Join two adjacent pieces into one curve.
///
/// Order matters: b must continue a in travel direction. The caller swaps
/// operands explicitly when asking about the opposite order.
///
/// Retain a's parameterization, start, center and radius, rather than averaging
/// nearly matching carriers.
///
/// `None` means no join: different carriers or kinds, a gap, opposite travel,
/// or a closed operand with no endpoints. Any pair is a valid query.
///
/// # Model identity
///
/// This algebra checks geometry only. A host plan must separately require a
/// shared point identity: coincident endpoints on the same circle cannot prove
/// topological adjacency.
pub fn join(a: &TrimCurve, b: &TrimCurve) -> Option<TrimCurve> {
    let eps = joint_eps(a, b);
    assert!(
        a.length().0 > 0.0 && b.length().0 > 0.0,
        "join: degenerate operand (lengths {} and {}); reject before algebra",
        a.length().0,
        b.length().0
    );

    match (a, b) {
        (
            TrimCurve::Segment {
                a: a_start,
                b: a_end,
            },
            TrimCurve::Segment {
                a: b_start,
                b: b_end,
            },
        ) => {
            if a_end.distance(*b_start) > eps {
                return None;
            }
            let dir_a = (a_end.0 - a_start.0).normalize();
            let dir_b = (b_end.0 - b_start.0).normalize();
            // Continuation requires collinearity and positive direction dot
            // product; a reversed segment is collinear too.
            if dir_a.perp_dot(dir_b).abs() > JOINT_EPS_REL || dir_a.dot(dir_b) <= 0.0 {
                return None;
            }
            Some(TrimCurve::Segment {
                a: *a_start,
                b: *b_end,
            })
        }
        (
            TrimCurve::Arc {
                center: center_a,
                radius: radius_a,
                geom: geom_a,
            },
            TrimCurve::Arc {
                center: center_b,
                radius: radius_b,
                geom: geom_b,
            },
        ) => {
            if center_a.distance(*center_b) > eps || radius_a.abs_diff(*radius_b) > eps {
                return None;
            }
            // Opposite arc directions do not continue even at coincident ends.
            if geom_a.clockwise() != geom_b.clockwise() {
                return None;
            }
            // A circular join has equal endpoint directions modulo τ.
            let angular_eps = eps.0 / radius_a.0;
            let gap = wrap_tau(geom_b.start_angle.radians() - geom_a.end_angle().radians());
            if gap > angular_eps && gap < std::f64::consts::TAU - angular_eps {
                return None;
            }

            let sweep = geom_a.sweep_angle.radians() + geom_b.sweep_angle.radians();
            assert!(
                sweep.abs() <= std::f64::consts::TAU + angular_eps,
                "join: combined sweep {sweep} exceeds a full turn; overlapping arcs are not adjacent components"
            );
            // Joining all pieces of a circle must restore a closed circle,
            // rather than a τ arc with endpoints. Pieces of a full-turn arc
            // are geometrically indistinguishable here and also close;
            // only the host plan can retain their model identity distinction.
            if std::f64::consts::TAU - sweep.abs() < angular_eps {
                return Some(TrimCurve::Circle {
                    center: *center_a,
                    radius: *radius_a,
                });
            }
            Some(TrimCurve::Arc {
                center: *center_a,
                radius: *radius_a,
                geom: ArcGeom::new(geom_a.start_angle, Angle(sweep)),
            })
        }
        // Circles have no endpoints; a segment and arc do not share a carrier.
        _ => None,
    }
}

/// Angular distance from start to angle in travel direction, in `[0, τ)`.
fn sweep_path(start: Angle, clockwise: bool, angle: Angle) -> f64 {
    let raw = if clockwise {
        start.radians() - angle.radians()
    } else {
        angle.radians() - start.radians()
    };
    wrap_tau(raw)
}

fn wrap_tau(x: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let r = x % tau;
    if r < 0.0 { r + tau } else { r }
}
