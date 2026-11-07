use crate::arc::angle_on_arc_cw;
use crate::roundoff::sqrt_of_proven_nonnegative;
use crate::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
use crate::world::{WorldLength, WorldPoint, WorldVec};

pub fn sample_curve_range<F>(
    p: WorldPoint,
    start: f64,
    end: f64,
    steps: usize,
    eval: F,
) -> WorldPoint
where
    F: Fn(f64) -> WorldPoint,
{
    assert!(steps > 0, "curve sampling needs at least one interval");
    let step = (end - start) / steps as f64;
    let mut best = eval(start);
    let mut best_dist = p.0.distance_squared(best.0);
    let mut best_index = 0;
    for i in 1..=steps {
        let t = start + step * i as f64;
        let pt = eval(t);
        let d = p.0.distance_squared(pt.0);
        if d < best_dist {
            best_dist = d;
            best = pt;
            best_index = i;
        }
    }
    let a = start + step * best_index.saturating_sub(1) as f64;
    let b = start + step * (best_index + 1).min(steps) as f64;
    let t = crate::primitives::golden_minimum(a.min(b), a.max(b), 60, |t| {
        p.0.distance_squared(eval(t).0)
    });
    let refined = eval(t);
    // Keep exact endpoints and exact sampled hits instead of replacing them
    // with the final interval midpoint's rounding error.
    if p.0.distance_squared(refined.0) < best_dist {
        refined
    } else {
        best
    }
}

// ---------------------------------------------------------------------------
// Ellipse
// ---------------------------------------------------------------------------

/// Semi-axes of an ellipse as VECTORS: `(a_vec, b_vec)`, so that
/// `P(t) = center + cos(t)*a_vec + sin(t)*b_vec`.
///
/// Shared coordinate frame for evaluation, projection and bounds.
///
/// `None` means the focus coincides with the center, leaving axes undefined.
pub fn ellipse_axes(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
) -> Option<(WorldVec, WorldVec)> {
    let cf = (focus1 - center).0;
    let cf_len = cf.length();
    if cf_len < 1e-8 {
        return None;
    }
    let b = semi_minor.0;
    // a² = b² + c²: an ellipse's focus lies inside its major semiaxis.
    let a = (b * b + cf_len * cf_len).sqrt();
    let major_dir = cf / cf_len;
    Some((
        WorldVec::from_dvec2(major_dir * a),
        WorldVec::from_dvec2(major_dir.perp() * b),
    ))
}

/// Evaluate a point on an ellipse at parameter t.
/// Ellipse: P(t) = center + cos(t)*a_vec + sin(t)*b_vec
/// where a_vec = major_dir * semi_major, b_vec = minor_dir * semi_minor.
pub fn ellipse_point(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    t: Angle,
) -> WorldPoint {
    let Some((a_vec, b_vec)) = ellipse_axes(center, focus1, semi_minor) else {
        return center;
    };
    WorldPoint::from_dvec2(center.0 + a_vec.0 * t.radians().cos() + b_vec.0 * t.radians().sin())
}

/// Compute semi-minor axis from ellipse definition points.
/// Given center, focus1, and a point on the ellipse (rim), returns semi_minor (b).
/// From: dist(rim, F1) + dist(rim, F2) = 2a, and a² = b² + c².
pub fn ellipse_semi_minor(center: WorldPoint, focus1: WorldPoint, rim: WorldPoint) -> WorldLength {
    let f2 = WorldPoint::from_dvec2(2.0 * center.0 - focus1.0);
    let sum_dist = rim.distance(focus1).0 + rim.distance(f2).0; // = 2a
    let a = sum_dist * 0.5;
    let c = center.distance(focus1).0;
    // Triangle inequality proves b² = a² − c² >= 0:
    // |rim−f1| + |rim−f2| >= 2c. Distance rounding depends on absolute coordinates,
    // so include their magnitude in the error scale.
    let magnitude = a + c + coordinate_magnitude(&[center, focus1, rim]);
    WorldLength::new(sqrt_of_proven_nonnegative(
        a * a - c * c,
        magnitude * magnitude,
        "ellipse_semi_minor: a² − c²",
    ))
}

/// Bound distance rounding by coordinate magnitudes. A sum is at most the
/// point count times coarser than max and does not swallow NaN.
fn coordinate_magnitude(points: &[WorldPoint]) -> f64 {
    points.iter().map(|p| p.0.abs().max_element()).sum()
}

/// Angle parameter t for a point on the ellipse (inverse of ellipse_point).
pub fn ellipse_angle(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    point: WorldPoint,
) -> Angle {
    let Some((a_vec, b_vec)) = ellipse_axes(center, focus1, semi_minor) else {
        return Angle(0.0);
    };
    let d = (point - center).0;
    // Projection divided by axis length: d·â/|a| = d·a_vec/|a_vec|².
    let x = d.dot(a_vec.0) / a_vec.0.length_squared();
    let y = d.dot(b_vec.0) / b_vec.0.length_squared();
    Angle(y.atan2(x))
}

pub fn ellipse_arc_sweep(t_start: Angle, t_end: Angle, clockwise: bool) -> Angle {
    let tau = std::f64::consts::TAU;
    if clockwise {
        let mut s = (t_start.radians() - t_end.radians()) % tau;
        if s <= 0.0 {
            s += tau;
        }
        Angle(-s)
    } else {
        let mut s = (t_end.radians() - t_start.radians()) % tau;
        if s <= 0.0 {
            s += tau;
        }
        Angle(s)
    }
}

fn nearer_of(p: WorldPoint, a: WorldPoint, b: WorldPoint) -> WorldLength {
    let to_a = p.distance(a);
    let to_b = p.distance(b);
    if to_a < to_b { to_a } else { to_b }
}

pub fn dist_to_arc_of_ellipse(
    p: WorldPoint,
    center: WorldPoint,
    focus1: WorldPoint,
    start: WorldPoint,
    end: WorldPoint,
    clockwise: bool,
) -> WorldLength {
    let b = ellipse_semi_minor(center, focus1, start);
    let t_s = ellipse_angle(center, focus1, b, start);
    let t_e = ellipse_angle(center, focus1, b, end);
    let t_p = ellipse_angle(center, focus1, b, p);
    let on_arc = angle_on_arc_cw(t_s, t_e, clockwise, t_p);

    if on_arc {
        let closest = ellipse_point(center, focus1, b, t_p);
        p.distance(closest)
    } else {
        nearer_of(p, start, end)
    }
}

pub fn closest_on_ellipse_arc(
    p: WorldPoint,
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    t_start: Angle,
    t_end: Angle,
    clockwise: bool,
) -> WorldPoint {
    let Some((a_vec, b_vec)) = ellipse_axes(center, focus1, semi_minor) else {
        return center;
    };
    let sweep = ellipse_arc_sweep(t_start, t_end, clockwise);
    let eval = |t: f64| WorldPoint::from_dvec2(center.0 + a_vec.0 * t.cos() + b_vec.0 * t.sin());
    sample_curve_range(
        p,
        t_start.radians(),
        t_start.radians() + sweep.radians(),
        64,
        eval,
    )
}

// ---------------------------------------------------------------------------
// Hyperbola
// ---------------------------------------------------------------------------

/// Semi-axes of a hyperbola as VECTORS: `(a_vec, b_vec)`, so that
/// `P(t) = center + cosh(t)*a_vec + sinh(t)*b_vec`.
///
/// Shared frame as in [`ellipse_axes`], with `a² = c² - b²`.
pub fn hyperbola_axes(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
) -> Option<(WorldVec, WorldVec)> {
    let cf = (focus1 - center).0;
    let cf_len = cf.length();
    if cf_len < 1e-8 {
        return None;
    }
    let b = semi_minor.0;
    // a² = c² − b²: the focus lies outside. A semiminor computed with the
    // same center/focus has b <= c within rounding. A larger excess is an
    // inconsistent input, rather than a degenerate hyperbola with a = 0.
    let a = sqrt_of_proven_nonnegative(
        cf_len * cf_len - b * b,
        cf_len * cf_len,
        "hyperbola_axes: c² − b²",
    );
    let major_dir = cf / cf_len;
    Some((
        WorldVec::from_dvec2(major_dir * a),
        WorldVec::from_dvec2(major_dir.perp() * b),
    ))
}

/// Evaluate a point on a hyperbola at parameter t.
/// `P(t) = center ± cosh(t)*a_vec + sinh(t)*b_vec`, signed by [`HyperbolaBranch`].
///
/// The branch is required because cosh is positive on the whole real line.
pub fn hyperbola_point(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    t: HyperbolaParam,
    branch: HyperbolaBranch,
) -> WorldPoint {
    let Some((a_vec, b_vec)) = hyperbola_axes(center, focus1, semi_minor) else {
        return center;
    };
    WorldPoint::from_dvec2(
        center.0 + a_vec.0 * (branch.sign() * t.get().cosh()) + b_vec.0 * t.get().sinh(),
    )
}

/// Branch of a point, from the sign of its major-axis projection.
///
/// `None` if a coincident focus and center leave the axes undefined.
pub fn hyperbola_branch(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    point: WorldPoint,
) -> Option<HyperbolaBranch> {
    let (a_vec, _) = hyperbola_axes(center, focus1, semi_minor)?;
    if a_vec.0.length_squared() < 1e-16 {
        return None;
    }
    Some(if (point - center).0.dot(a_vec.0) >= 0.0 {
        HyperbolaBranch::Positive
    } else {
        HyperbolaBranch::Negative
    })
}

/// Semi-minor axis for hyperbola from definition points.
/// |dist(rim, F1) - dist(rim, F2)| = 2a, a² = c² - b².
pub fn hyperbola_semi_minor(
    center: WorldPoint,
    focus1: WorldPoint,
    rim: WorldPoint,
) -> WorldLength {
    let f2 = WorldPoint::from_dvec2(2.0 * center.0 - focus1.0);
    let diff_dist = (rim.distance(focus1).0 - rim.distance(f2).0).abs(); // = 2a
    let a = diff_dist * 0.5;
    let c = center.distance(focus1).0;
    // Triangle inequality proves b² = c² − a² >= 0:
    // ||rim−f1| − |rim−f2|| <= 2c. Use the ellipse's coordinate error scale.
    let magnitude = a + c + coordinate_magnitude(&[center, focus1, rim]);
    WorldLength::new(sqrt_of_proven_nonnegative(
        c * c - a * a,
        magnitude * magnitude,
        "hyperbola_semi_minor: c² − a²",
    ))
}

/// Inverse: find parameter t for a point on the hyperbola.
///
/// Inverse of [`hyperbola_point`]: `y = b·sinh t` gives `t = asinh(y/b)`.
///
/// An ordering surrogate such as atan2(sinh t, cosh t) is insufficient:
/// this parameter must reproduce the point when passed back to evaluation.
pub fn hyperbola_param(
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    point: WorldPoint,
) -> HyperbolaParam {
    let Some((_, b_vec)) = hyperbola_axes(center, focus1, semi_minor) else {
        return HyperbolaParam(0.0);
    };
    let b_sq = b_vec.0.length_squared();
    if b_sq < 1e-16 {
        return HyperbolaParam(0.0);
    }
    let y = (point - center).0.dot(b_vec.0) / b_sq; // = sinh(t)
    HyperbolaParam(y.asinh())
}

pub fn dist_to_arc_of_hyperbola(
    p: WorldPoint,
    center: WorldPoint,
    focus1: WorldPoint,
    start: WorldPoint,
    end: WorldPoint,
) -> WorldLength {
    let b = hyperbola_semi_minor(center, focus1, start);
    let t_s = hyperbola_param(center, focus1, b, start);
    let t_e = hyperbola_param(center, focus1, b, end);
    let t_p = hyperbola_param(center, focus1, b, p);
    let Some(branch) = hyperbola_branch(center, focus1, b, start) else {
        return nearer_of(p, start, end);
    };

    // An opposite-branch point, like one outside the interval, uses endpoint
    // distances. Its scalar parameter alone cannot establish arc membership.
    let same_branch = hyperbola_branch(center, focus1, b, p) == Some(branch);
    let (t_min, t_max) = if t_s <= t_e { (t_s, t_e) } else { (t_e, t_s) };
    if same_branch && t_p >= t_min && t_p <= t_max {
        let closest = hyperbola_point(center, focus1, b, t_p, branch);
        p.distance(closest)
    } else {
        nearer_of(p, start, end)
    }
}

pub fn closest_on_hyperbola_arc(
    p: WorldPoint,
    center: WorldPoint,
    focus1: WorldPoint,
    semi_minor: WorldLength,
    t_start: HyperbolaParam,
    t_end: HyperbolaParam,
    branch: HyperbolaBranch,
) -> WorldPoint {
    let Some((a_vec, b_vec)) = hyperbola_axes(center, focus1, semi_minor) else {
        return center;
    };
    let sign = branch.sign();
    let eval = |t: f64| {
        WorldPoint::from_dvec2(center.0 + a_vec.0 * (sign * t.cosh()) + b_vec.0 * t.sinh())
    };
    sample_curve_range(p, t_start.get(), t_end.get(), 64, eval)
}

// ---------------------------------------------------------------------------
// Parabola
// ---------------------------------------------------------------------------

/// Orthonormal axes and focal length of one parabola. `u` is measured along
/// `transverse`; the opening axis carries the quadratic coordinate.
#[derive(Debug, Clone, Copy)]
pub struct ParabolaFrame {
    vertex: WorldPoint,
    pub axis: WorldVec,
    pub transverse: WorldVec,
    pub focal_length: WorldLength,
}

impl ParabolaFrame {
    /// P(u) = vertex + (u²/(4f))*axis + u*transverse.
    pub fn point(self, u: ParabolaParam) -> WorldPoint {
        WorldPoint::from_dvec2(
            self.vertex.0
                + self.axis.0 * (u.get() * u.get() / (4.0 * self.focal_length.0))
                + self.transverse.0 * u.get(),
        )
    }

    pub fn parameter(self, point: WorldPoint) -> ParabolaParam {
        ParabolaParam((point - self.vertex).0.dot(self.transverse.0))
    }
}

/// Frame of a parabola with its opening axis and transverse direction.
///
/// `None` if the focus coincides with the vertex, leaving no opening axis.
pub fn parabola_frame(vertex: WorldPoint, focus: WorldPoint) -> Option<ParabolaFrame> {
    let axis = (focus - vertex).0;
    let f = axis.length();
    if f < 1e-8 {
        return None;
    }
    let xdir = axis / f;
    Some(ParabolaFrame {
        vertex,
        axis: WorldVec::from_dvec2(xdir),
        transverse: WorldVec::from_dvec2(xdir.perp()),
        focal_length: WorldLength::new(f),
    })
}

/// Evaluate a point on a parabola at parameter u.
/// P(u) = vertex + (u²/(4f))*xdir + u*ydir
pub fn parabola_point(vertex: WorldPoint, focus: WorldPoint, u: ParabolaParam) -> WorldPoint {
    let Some(frame) = parabola_frame(vertex, focus) else {
        return vertex;
    };
    frame.point(u)
}

/// Inverse: find parameter u for a point on the parabola.
pub fn parabola_param(vertex: WorldPoint, focus: WorldPoint, point: WorldPoint) -> ParabolaParam {
    let Some(frame) = parabola_frame(vertex, focus) else {
        return ParabolaParam(0.0);
    };
    frame.parameter(point)
}

pub fn dist_to_arc_of_parabola(
    p: WorldPoint,
    vertex: WorldPoint,
    focus: WorldPoint,
    start: WorldPoint,
    end: WorldPoint,
) -> WorldLength {
    let u_s = parabola_param(vertex, focus, start);
    let u_e = parabola_param(vertex, focus, end);
    let u_p = parabola_param(vertex, focus, p);

    let (u_min, u_max) = if u_s <= u_e { (u_s, u_e) } else { (u_e, u_s) };
    if u_p >= u_min && u_p <= u_max {
        let closest = parabola_point(vertex, focus, u_p);
        p.distance(closest)
    } else {
        nearer_of(p, start, end)
    }
}

pub fn closest_on_parabola_arc(
    p: WorldPoint,
    vertex: WorldPoint,
    focus: WorldPoint,
    u_start: ParabolaParam,
    u_end: ParabolaParam,
) -> WorldPoint {
    let Some(frame) = parabola_frame(vertex, focus) else {
        return vertex;
    };
    let eval = |u: f64| frame.point(ParabolaParam(u));
    sample_curve_range(p, u_start.get(), u_end.get(), 64, eval)
}
