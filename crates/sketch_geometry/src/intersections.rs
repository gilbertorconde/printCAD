use crate::roundoff::sqrt_of_proven_nonnegative;
use crate::world::{WorldLength, WorldPoint};

/// Segment-segment intersection within both finite extents.
pub fn seg_seg_intersection(
    a1: WorldPoint,
    a2: WorldPoint,
    b1: WorldPoint,
    b2: WorldPoint,
) -> Option<WorldPoint> {
    let r = a2.0 - a1.0;
    let s = b2.0 - b1.0;
    let denom = r.x * s.y - r.y * s.x;
    if denom == 0.0 {
        return None;
    }
    let diff = b1.0 - a1.0;
    let t = (diff.x * s.y - diff.y * s.x) / denom;
    let u = (diff.x * r.y - diff.y * r.x) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(WorldPoint::from_dvec2(a1.0 + r * t))
    } else {
        None
    }
}

/// Segment-circle intersections (0, 1, or 2 points on the segment).
pub fn seg_circle_intersections(
    a: WorldPoint,
    b: WorldPoint,
    center: WorldPoint,
    radius: WorldLength,
) -> Vec<WorldPoint> {
    let d = b.0 - a.0;
    let f = a.0 - center.0;
    let aa = d.dot(d);
    if aa == 0.0 {
        return vec![];
    }
    // Project first: b² - 4ac cancels large terms when a long segment crosses a
    // small circle. This form subtracts distances in the circle's local frame.
    let middle = -f.dot(d) / aa;
    let closest = f + d * middle;
    let height_squared = radius.0 * radius.0 - closest.length_squared();
    if height_squared < 0.0 {
        return vec![];
    }
    let half_span = (height_squared / aa).sqrt();
    let mut pts = Vec::new();
    for sign in [1.0_f64, -1.0_f64] {
        let t = middle + sign * half_span;
        if (0.0..=1.0).contains(&t) {
            pts.push(WorldPoint::from_dvec2(a.0 + d * t));
        }
    }
    pts
}

/// Circle-circle intersections (0, 1, or 2 points).
pub fn circle_circle_intersections(
    c1: WorldPoint,
    r1: WorldLength,
    c2: WorldPoint,
    r2: WorldLength,
) -> Vec<WorldPoint> {
    let (r1, r2) = (r1.0, r2.0);
    let d = c2.0 - c1.0;
    let dist = d.length();
    if dist == 0.0 || dist > r1 + r2 || dist < (r1 - r2).abs() {
        return vec![];
    }
    let a = (r1 * r1 - r2 * r2 + dist * dist) / (2.0 * dist);
    // The distance tests prove existence, so `r1² − a² ≥ 0` up to roundoff at a
    // tangent, where the exact height is zero. Error of `a`: the numerator carries
    // ε·(r1² + r2² + d²) plus ε·|c|·d from the centres, divided by 2d; `a²` doubles it
    // by `a ≤ r1`. With M bounding every length and coordinate this is ≤ ε·M³/d — at
    // internal tangency with a small `d` it is genuinely that large.
    let magnitude = r1 + r2 + dist + c1.0.length() + c2.0.length();
    let h = sqrt_of_proven_nonnegative(
        r1 * r1 - a * a,
        magnitude * magnitude * magnitude / dist,
        "circle_circle_intersections: tangent height",
    );
    let mid = c1.0 + d * (a / dist);
    let prp = d.perp() * (h / dist);
    if h == 0.0 {
        vec![WorldPoint::from_dvec2(mid)]
    } else {
        vec![
            WorldPoint::from_dvec2(mid + prp),
            WorldPoint::from_dvec2(mid - prp),
        ]
    }
}
