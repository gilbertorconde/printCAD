//! Axis-aligned bounding boxes and exact primitive bounds.
//!
//! Bounds drive framing, picking and intersection broad phases. Find extrema
//! analytically from zero coordinate derivatives and retain only roots on the arc.
//!
//! Endpoint bounds miss interior extrema; carrier bounds overestimate short arcs.
//! Tests distinguish both errors using arcs beyond an exact quadrant.

use glam::DVec2;

use crate::arc::angle_on_arc_cw;
use crate::curves::{
    ellipse_axes, ellipse_point, hyperbola_axes, hyperbola_point, parabola_frame, parabola_point,
};
use crate::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
use crate::world::{WorldLength, WorldPoint, WorldVec};

/// Axis-aligned bounds with componentwise invariant `min <= max`.
///
/// Private fields preserve the invariant. Absence is `Option<Bbox>`, never an
/// infinite empty box or a zero placeholder that would expand unions to the origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bbox {
    min: WorldPoint,
    max: WorldPoint,
}

impl Bbox {
    /// A single point has valid degenerate bounds: `min == max`.
    pub fn from_point(p: WorldPoint) -> Self {
        assert!(p.0.is_finite(), "Bbox: point must be finite: {p:?}");
        Self { min: p, max: p }
    }

    /// Bounds of points, or `None` for an empty input.
    pub fn from_points(points: impl IntoIterator<Item = WorldPoint>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        Some(it.fold(Self::from_point(first), |acc, p| acc.with_point(p)))
    }

    /// Union of bounds, skipping absent bounds.
    pub fn union_all(boxes: impl IntoIterator<Item = Option<Self>>) -> Option<Self> {
        boxes
            .into_iter()
            .flatten()
            .fold(None, |acc: Option<Self>, b| {
                Some(acc.map_or(b, |a| a.union(b)))
            })
    }

    pub fn min(self) -> WorldPoint {
        self.min
    }

    pub fn max(self) -> WorldPoint {
        self.max
    }

    pub fn center(self) -> WorldPoint {
        self.min.midpoint(self.max)
    }

    /// Size, including zero components for a point or an axis-aligned segment.
    pub fn size(self) -> WorldVec {
        self.max - self.min
    }

    pub fn with_point(self, p: WorldPoint) -> Self {
        // WorldPoint is transparent wire data too. Check before IEEE min/max
        // can turn a broken point into an apparently valid existing bound.
        assert!(p.0.is_finite(), "Bbox: point must be finite: {p:?}");
        Self {
            min: WorldPoint(self.min.0.min(p.0)),
            max: WorldPoint(self.max.0.max(p.0)),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: WorldPoint(self.min.0.min(other.min.0)),
            max: WorldPoint(self.max.0.max(other.max.0)),
        }
    }

    /// Membership with an explicit tolerance, including rounded boundary points.
    pub fn contains(self, p: WorldPoint, eps: WorldLength) -> bool {
        let eps = eps.0;
        p.0.x >= self.min.0.x - eps
            && p.0.x <= self.max.0.x + eps
            && p.0.y >= self.min.0.y - eps
            && p.0.y <= self.max.0.y + eps
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.min.0.x <= other.max.0.x
            && self.max.0.x >= other.min.0.x
            && self.min.0.y <= other.max.0.y
            && self.max.0.y >= other.min.0.y
    }
}

pub fn segment_bbox(a: WorldPoint, b: WorldPoint) -> Bbox {
    Bbox::from_point(a).with_point(b)
}

pub fn circle_bbox(center: WorldPoint, radius: WorldLength) -> Bbox {
    assert!(
        radius.0.is_finite() && radius.0 >= 0.0,
        "circle_bbox: radius must be finite and nonnegative: {radius:?}"
    );
    let r = DVec2::splat(radius.0);
    Bbox {
        min: WorldPoint::from_dvec2(center.0 - r),
        max: WorldPoint::from_dvec2(center.0 + r),
    }
}

/// Exact circular arc bounds.
///
/// Direction is the sign of `sweep_angle`. Candidates are the endpoints and
/// the four quadrant directions that lie within the signed sweep.
pub fn arc_bbox(
    center: WorldPoint,
    radius: WorldLength,
    start_angle: Angle,
    sweep_angle: Angle,
) -> Bbox {
    assert!(
        radius.0.is_finite()
            && radius.0 >= 0.0
            && start_angle.0.is_finite()
            && sweep_angle.0.is_finite(),
        "arc_bbox: finite angles and nonnegative finite radius required: {radius:?}, {start_angle:?}, {sweep_angle:?}"
    );
    let tau = std::f64::consts::TAU;
    // Zero sweep denotes a point. Handle it before the endpoint predicate,
    // whose equal directions denote a full turn.
    if sweep_angle.radians() == 0.0 {
        return Bbox::from_point(WorldPoint::from_dvec2(
            center.0 + DVec2::from_angle(start_angle.radians()) * radius.0,
        ));
    }
    // A full turn includes all quadrants despite coincident endpoint directions.
    // Allow only a few ULPs of τ from `ccw_sweep_to`; a wider tolerance would
    // include quadrant extrema inside a real gap in a nearly full circle.
    if sweep_angle.radians().abs() >= tau - 1e-12 {
        return circle_bbox(center, radius);
    }

    let at =
        |a: Angle| WorldPoint::from_dvec2(center.0 + DVec2::from_angle(a.radians()) * radius.0);
    let end_angle = start_angle + sweep_angle;
    let clockwise = sweep_angle.radians() < 0.0;

    let mut bbox = segment_bbox(at(start_angle), at(end_angle));
    for quadrant in 0..4 {
        let a = Angle(quadrant as f64 * std::f64::consts::FRAC_PI_2);
        if angle_on_arc_cw(start_angle, end_angle, clockwise, a) {
            bbox = bbox.with_point(at(a));
        }
    }
    bbox
}

/// Exact elliptic arc bounds.
///
/// `P(t) = C + cos t · A + sin t · B` has `dx/dt = 0` when `tan t = B.x / A.x`,
/// at `atan2(B.x, A.x)` and that angle plus π. Use the second components for y.
/// Include a candidate only when it lies on the arc.
pub fn ellipse_arc_bbox(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    t_start: Angle,
    t_end: Angle,
    clockwise: bool,
) -> Bbox {
    let start = ellipse_point(center, focus1, semi_minor, t_start);
    let end = ellipse_point(center, focus1, semi_minor, t_end);
    let mut bbox = segment_bbox(start, end);

    let Some((a_vec, b_vec)) = ellipse_axes(center, focus1, semi_minor) else {
        // A degenerate definition has only its two endpoints to bound.
        return bbox;
    };

    let pi = std::f64::consts::PI;
    let base_x = b_vec.0.x.atan2(a_vec.0.x);
    let base_y = b_vec.0.y.atan2(a_vec.0.y);
    for t in [base_x, base_x + pi, base_y, base_y + pi] {
        let t = Angle(t);
        if angle_on_arc_cw(t_start, t_end, clockwise, t) {
            bbox = bbox.with_point(ellipse_point(center, focus1, semi_minor, t));
        }
    }
    bbox
}

/// Exact hyperbolic arc bounds.
///
/// `P(t) = C ± cosh t · A + sinh t · B` has `dx/dt = ±A.x sinh t + B.x cosh t = 0`
/// at `tanh t = ∓B.x / A.x`. A root exists only for `|B.x / A.x| < 1`,
/// when the branch turns along that axis. The parameter interval is not cyclic.
///
/// The branch supplies the sign; opposite branches have extrema on opposite
/// sides of `t = 0`.
pub fn hyperbola_arc_bbox(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    t_start: HyperbolaParam,
    t_end: HyperbolaParam,
    branch: HyperbolaBranch,
) -> Bbox {
    let start = hyperbola_point(center, focus1, semi_minor, t_start, branch);
    let end = hyperbola_point(center, focus1, semi_minor, t_end, branch);
    let mut bbox = segment_bbox(start, end);

    let Some((a_vec, b_vec)) = hyperbola_axes(center, focus1, semi_minor) else {
        return bbox;
    };

    let (lo, hi) = if t_start.get() <= t_end.get() {
        (t_start.get(), t_end.get())
    } else {
        (t_end.get(), t_start.get())
    };
    for (a_comp, b_comp) in [(a_vec.0.x, b_vec.0.x), (a_vec.0.y, b_vec.0.y)] {
        if a_comp.abs() < 1e-12 {
            continue;
        }
        let ratio = -b_comp / (branch.sign() * a_comp);
        if ratio.abs() >= 1.0 {
            // A monotone coordinate has extrema only at the included endpoints.
            continue;
        }
        let t = ratio.atanh();
        if t >= lo && t <= hi {
            bbox = bbox.with_point(hyperbola_point(
                center,
                focus1,
                semi_minor,
                HyperbolaParam(t),
                branch,
            ));
        }
    }
    bbox
}

/// Exact parabolic arc bounds.
///
/// `P(u) = V + (u²/4f)·xdir + u·ydir` has `dx/du = xdir.x·u/(2f) + ydir.x = 0`
/// at `u = -2f·ydir.x / xdir.x`. The vertex is a special case of this same root.
pub fn parabola_arc_bbox(
    vertex: WorldPoint,
    focus: WorldPoint,
    u_start: ParabolaParam,
    u_end: ParabolaParam,
) -> Bbox {
    let start = parabola_point(vertex, focus, u_start);
    let end = parabola_point(vertex, focus, u_end);
    let mut bbox = segment_bbox(start, end);

    let Some(frame) = parabola_frame(vertex, focus) else {
        return bbox;
    };

    let (lo, hi) = if u_start.get() <= u_end.get() {
        (u_start.get(), u_end.get())
    } else {
        (u_end.get(), u_start.get())
    };
    for (x_comp, y_comp) in [
        (frame.axis.0.x, frame.transverse.0.x),
        (frame.axis.0.y, frame.transverse.0.y),
    ] {
        if x_comp.abs() < 1e-12 {
            continue;
        }
        let u = -2.0 * frame.focal_length.0 * y_comp / x_comp;
        if u >= lo && u <= hi {
            bbox = bbox.with_point(parabola_point(vertex, focus, ParabolaParam(u)));
        }
    }
    bbox
}
