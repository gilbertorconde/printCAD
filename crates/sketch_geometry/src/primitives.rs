use crate::world::{WorldLength, WorldPoint, WorldVec};

pub fn project_onto_line(p: WorldPoint, a: WorldPoint, b: WorldPoint) -> WorldPoint {
    let ab = b - a;
    let t = (p - a).0.dot(ab.0) / ab.0.length_squared();
    a + ab * t
}

pub fn project_onto_segment(p: WorldPoint, a: WorldPoint, b: WorldPoint) -> WorldPoint {
    let ab = b - a;
    let t = ((p - a).0.dot(ab.0) / ab.0.length_squared()).clamp(0.0, 1.0);
    a + ab * t
}

/// Tangent arc center and contact points on the respective arms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilletGeometry {
    pub center: WorldPoint,
    pub tangent_a: WorldPoint,
    pub tangent_b: WorldPoint,
}

/// Fillet arc center for two rays meeting at `corner`, given a fillet radius.
///
/// `dir_a` and `dir_b` are unit vectors pointing away from the corner along each arm.
/// Returns the center and both tangencies, or `None` for parallel rays.
/// The caller checks the available arm lengths; this function only knows directions.
///
/// The center lies on the internal angle bisector, at radius / sin(half-angle):
///   bisector = normalize(dir_a + dir_b)
///   sin_half  = |cross(dir_a, bisector)|   — sine of the half-angle at the corner
///   dist      = radius / sin_half           — distance from corner to arc center along bisector
///   center    = corner + bisector * dist
///   tangent_a = project center onto line(corner, dir_a) → corner + dir_a * dot(center-corner, dir_a)
///   tangent_b = project center onto line(corner, dir_b) → corner + dir_b * dot(center-corner, dir_b)
pub fn fillet_center(
    corner: WorldPoint,
    dir_a: WorldVec,
    dir_b: WorldVec,
    radius: WorldLength,
) -> Option<FilletGeometry> {
    let dir_a = dir_a.0.normalize();
    let dir_b = dir_b.0.normalize();
    let corner = corner.0;
    let bisector = (dir_a + dir_b).normalize_or_zero();
    if bisector.length_squared() < 1e-8 {
        // dir_a and dir_b are antiparallel (180° corner) — fillet undefined
        return None;
    }
    // sin of the half-angle between the bisector and dir_a
    let sin_half = (dir_a.x * bisector.y - dir_a.y * bisector.x).abs();
    if sin_half < 1e-6 {
        // Lines are parallel (0° angle) — fillet undefined
        return None;
    }
    let dist = radius.0 / sin_half;
    let center = corner + bisector * dist;

    // Tangent point on arm A: projection of center onto the ray from corner along dir_a
    let t_a = corner + dir_a * (center - corner).dot(dir_a);
    // Tangent point on arm B: projection of center onto the ray from corner along dir_b
    let t_b = corner + dir_b * (center - corner).dot(dir_b);

    Some(FilletGeometry {
        center: WorldPoint::new(center.x, center.y),
        tangent_a: WorldPoint::new(t_a.x, t_a.y),
        tangent_b: WorldPoint::new(t_b.x, t_b.y),
    })
}

/// Compute ellipse focus1 from center, axis_pt (on major axis), and start (on ellipse).
/// a = |center - axis_pt| (semi-major), then b from start lying on ellipse,
/// c = sqrt(a² - b²), focus1 = center + c * major_dir.
/// Returns `None` unless the points determine a unique nondegenerate ellipse
/// with that major semiaxis. A start at its endpoint does not determine `b`.
pub fn compute_ellipse_focus(
    center: WorldPoint,
    axis_pt: WorldPoint,
    start: WorldPoint,
) -> Option<WorldPoint> {
    let b = ellipse_other_radius(center, axis_pt, start)?.0;
    let axis = (axis_pt - center).0;
    let a = axis.length();
    if b > a {
        return None;
    }
    let c_sq = a * a - b * b;
    Some(center + WorldVec::from_dvec2(axis / a * c_sq.sqrt()))
}

/// Radius perpendicular to the given semiaxis, inferred from a point on the
/// ellipse. Either axis may be longer; a point at/beyond its endpoint or on
/// its line does not determine a nondegenerate ellipse.
pub fn ellipse_other_radius(
    center: WorldPoint,
    axis_pt: WorldPoint,
    rim: WorldPoint,
) -> Option<WorldLength> {
    let major_dir_unnorm = (axis_pt - center).0;
    let a = major_dir_unnorm.length();
    // An axis below the geometric degeneracy threshold has no direction.
    if a < WorldLength::DEGENERATE.0 {
        return None;
    }
    let major_dir = major_dir_unnorm / a;
    let minor_dir = major_dir.perp();

    // start is on the ellipse: (sx/a)² + (sy/b)² = 1
    let d = (rim - center).0;
    let sx = d.dot(major_dir);
    let sy = d.dot(minor_dir);

    // From ellipse equation: b² = sy² / (1 - (sx/a)²)
    let denom = 1.0 - (sx / a).powi(2);
    // At denom = sy = 0 any minor radius fits, so the focus is undetermined.
    // Beyond the supplied semiaxis no ellipse can pass through the point.
    if denom <= 0.0 {
        return None;
    }
    let b = sy.abs() / denom.sqrt();
    if !b.is_finite() || b < WorldLength::DEGENERATE.0 {
        return None;
    }
    Some(WorldLength(b))
}

/// Degeneracy threshold for `|d| = 2·|cross(b−a, c−a)|`, in squared world units.
/// An f64 threshold retains triangles whose edges are as small as 1e-4.
const CIRCUMCIRCLE_EPSILON: f64 = f64::EPSILON;

/// Circumcircle center and radius from three points.
///
/// Compute relative to `a`: subtracting absolute squared coordinates loses
/// about `|p|²·2⁻⁵²` precision far from the origin. Local terms instead scale
/// with the triangle, and `d` is the cross product of its edges.
pub fn circumcircle(
    a: WorldPoint,
    b: WorldPoint,
    c: WorldPoint,
) -> Option<(WorldPoint, WorldLength)> {
    let origin = a.0;
    let b = b.0 - origin;
    let c = c.0 - origin;
    let d = 2.0 * (b.x * c.y - b.y * c.x);
    if d.abs() < CIRCUMCIRCLE_EPSILON {
        return None;
    }
    let b2 = b.x * b.x + b.y * b.y;
    let c2 = c.x * c.x + c.y * c.y;
    let ux = (c.y * b2 - b.y * c2) / d;
    let uy = (b.x * c2 - c.x * b2) / d;
    let radius = WorldLength::new((ux * ux + uy * uy).sqrt());
    Some((WorldPoint::new(origin.x + ux, origin.y + uy), radius))
}
/// Refine a sampled local minimum on a bounded parameter interval.
/// The caller brackets one minimum; `iterations` preserves its numerical budget.
pub(crate) fn golden_minimum(
    mut lo: f64,
    mut hi: f64,
    iterations: usize,
    distance: impl Fn(f64) -> f64,
) -> f64 {
    const PHI: f64 = 0.618_033_988_749_895;
    for _ in 0..iterations {
        let m1 = hi - (hi - lo) * PHI;
        let m2 = lo + (hi - lo) * PHI;
        if distance(m1) <= distance(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    (lo + hi) * 0.5
}
/// Overlap is allowed in a sketch but forbidden in a profile. Explicit world
/// tolerances keep opposite sides of a long narrow rectangle distinct.
pub fn segments_overlap_with_tolerance(
    a0: WorldPoint,
    a1: WorldPoint,
    b0: WorldPoint,
    b1: WorldPoint,
    distance_tolerance: WorldLength,
    overlap_tolerance: WorldLength,
    minimum_segment_length: WorldLength,
) -> bool {
    let (a0, a1, b0, b1) = (a0.0, a1.0, b0.0, b1.0);
    let dir_a = a1 - a0;
    let len_a = dir_a.length();
    if len_a <= minimum_segment_length.0 {
        return false;
    }
    let axis = dir_a / len_a;

    let len_b = (b1 - b0).length();
    if len_b <= minimum_segment_length.0 {
        return false;
    }

    // Both endpoints must lie on the first line; that also implies parallelism.
    // Measure deviation from a unit axis so the tolerance has length units.
    let eps = distance_tolerance.0;
    if axis.perp_dot(b0 - a0).abs() > eps || axis.perp_dot(b1 - a0).abs() > eps {
        return false;
    }

    // Project onto the common axis: endpoint contact has no positive overlap.
    let (mut b_lo, mut b_hi) = ((b0 - a0).dot(axis), (b1 - a0).dot(axis));
    if b_lo > b_hi {
        std::mem::swap(&mut b_lo, &mut b_hi);
    }
    let overlap_lo = 0.0f64.max(b_lo);
    let overlap_hi = len_a.min(b_hi);
    overlap_hi - overlap_lo > overlap_tolerance.0
}
