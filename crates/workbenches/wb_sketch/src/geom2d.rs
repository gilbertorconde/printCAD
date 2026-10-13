//! 2D curve math shared by the editing tools: segment/arc/circle
//! intersections plus ellipse sampling.

use glam::DVec2;

use crate::sketch::{GeometryElement, Sketch, Vec2D};
use crate::snap::arc_angles;

/// Below this the two directions/points are treated as coincident.
const EPS: f64 = 1e-6;

/// Intersection of the infinite lines through `a1→a2` and `b1→b2`.
/// `None` for (near-)parallel lines.
pub fn line_line(a1: DVec2, a2: DVec2, b1: DVec2, b2: DVec2) -> Option<DVec2> {
    let da = a2 - a1;
    let db = b2 - b1;
    let denom = da.perp_dot(db);
    if denom.abs() < EPS * da.length() * db.length() {
        return None;
    }
    let t = (b1 - a1).perp_dot(db) / denom;
    Some(a1 + da * t)
}

/// Whether `p` (assumed on the carrier line) lies within the segment
/// `a→b`, with a small relative slack for intersections computed in f64.
pub fn on_segment(a: DVec2, b: DVec2, p: DVec2) -> bool {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < EPS * EPS {
        return (p - a).length_squared() < EPS * EPS;
    }
    let t = (p - a).dot(ab) / len_sq;
    (-1e-4..=1.0 + 1e-4).contains(&t)
}

/// Intersections of the infinite line through `a→b` with the circle
/// (`center`, `radius`): 0, 1 (tangent) or 2 points.
pub fn line_circle(a: DVec2, b: DVec2, center: DVec2, radius: f64) -> Vec<DVec2> {
    let d = b - a;
    let len = d.length();
    if len < EPS {
        return Vec::new();
    }
    let dir = d / len;
    // Foot of the perpendicular from the center onto the line.
    let t0 = (center - a).dot(dir);
    let foot = a + dir * t0;
    let h_sq = radius * radius - (center - foot).length_squared();
    let tangent_tol = EPS * radius.max(1.0);
    if h_sq < -tangent_tol {
        Vec::new()
    } else if h_sq < tangent_tol {
        vec![foot]
    } else {
        let h = h_sq.sqrt();
        vec![foot - dir * h, foot + dir * h]
    }
}

/// Intersections of two circles: 0, 1 (tangent) or 2 points. Coincident
/// circles report none (no isolated intersection points).
pub fn circle_circle(c1: DVec2, r1: f64, c2: DVec2, r2: f64) -> Vec<DVec2> {
    let d = (c2 - c1).length();
    let tol = EPS * (r1 + r2).max(1.0);
    if d < tol {
        return Vec::new(); // concentric
    }
    // Distance from c1 to the radical line along c1→c2.
    let a = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
    let h_sq = r1 * r1 - a * a;
    if h_sq < -tol {
        return Vec::new(); // separate or nested
    }
    let dir = (c2 - c1) / d;
    let mid = c1 + dir * a;
    if h_sq < tol {
        return vec![mid]; // tangent
    }
    let h = h_sq.sqrt();
    let n = dir.perp();
    vec![mid - n * h, mid + n * h]
}

/// Whether `p` (assumed on the circle) lies within the CCW arc `start→end`
/// around `center`.
pub fn point_on_arc(center: DVec2, start: DVec2, end: DVec2, p: DVec2) -> bool {
    let (start_angle, sweep) = arc_angles(start - center, end - center);
    let rel = wrap_positive((p - center).y.atan2((p - center).x) - start_angle);
    rel <= sweep + 1e-4
}

/// Circumcenter of three points; `None` when they are (near-)collinear.
pub fn circumcenter(a: DVec2, b: DVec2, c: DVec2) -> Option<DVec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let scale = (b - a).length() * (c - a).length();
    if d.abs() < 1e-5 * scale.max(EPS) {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    Some(DVec2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    ))
}

/// Wrap an angle into [0, TAU).
pub fn wrap_positive(a: f64) -> f64 {
    let mut a = a % std::f64::consts::TAU;
    if a < 0.0 {
        a += std::f64::consts::TAU;
    }
    a
}

/// An ellipse from one axis vector and the other radius, as the sketch
/// keeps it: the major vector and the minor-to-major ratio, at most 1.
/// When the other radius is the longer, it is the major: the axes trade
/// places, the same ellipse.
pub fn ellipse_axes(axis: glam::DVec2, other: f64) -> (glam::DVec2, f64) {
    let a = axis.length();
    let b = other.abs();
    if b > a && a > 1e-9 {
        (axis.perp() * (b / a), (a / b).clamp(1e-6, 1.0))
    } else {
        (axis, (b / a.max(1e-9)).clamp(1e-6, 1.0))
    }
}

/// Polyline sampling of an ellipse (`center`, center→major-vertex vector,
/// minor/major `ratio`), closed (first == last point).
pub fn ellipse_points(center: Vec2D, major: Vec2D, ratio: f64, segments: usize) -> Vec<Vec2D> {
    let c = center.to_glam();
    let a = major.to_glam();
    let b = a.perp() * ratio;
    (0..=segments)
        .map(|i| {
            let t = std::f64::consts::TAU * (i as f64) / (segments as f64);
            let (sin, cos) = t.sin_cos();
            Vec2D::from_glam(c + a * cos + b * sin)
        })
        .collect()
}

/// The circular arc that leaves `start` along the unit `heading` and ends at
/// `end`: its center, radius, and whether it turns counter-clockwise (to the
/// left of the heading). `None` when `end` lies on the heading's line, where
/// only a straight segment continues smoothly.
pub fn tangent_arc(start: Vec2D, heading: Vec2D, end: Vec2D) -> Option<(Vec2D, f64, bool)> {
    let t = heading.to_glam().normalize_or_zero();
    if t.length_squared() < 0.5 {
        return None;
    }
    let n = t.perp();
    let d = (end - start).to_glam();
    let across = d.dot(n);
    if across.abs() <= 1e-6 * d.length().max(1.0) {
        return None;
    }
    let signed = d.length_squared() / (2.0 * across);
    Some((
        Vec2D::from_glam(start.to_glam() + n * signed),
        signed.abs(),
        signed > 0.0,
    ))
}

/// The ellipse centred at `center` with a vertex at `major_pos` that passes
/// through `rim`: its major vector and minor-to-major ratio. When the rim
/// point makes the other axis the longer one, the axes swap so the ratio
/// stays within `(0, 1]`. `None` when the rim point lies on the axis line
/// at or beyond the vertex, where no such ellipse exists.
pub fn ellipse_through(center: Vec2D, major_pos: Vec2D, rim: Vec2D) -> Option<(Vec2D, f64)> {
    let axis = (major_pos - center).to_glam();
    let a = axis.length();
    if a <= 1e-6 {
        return None;
    }
    let u = axis / a;
    let d = (rim - center).to_glam();
    let (x, y) = (d.dot(u), d.dot(u.perp()));
    let s = 1.0 - (x / a).powi(2);
    if s <= 1e-9 || y.abs() <= 1e-6 {
        return None;
    }
    let b = y.abs() / s.sqrt();
    if b <= a {
        Some((Vec2D::from_glam(axis), b / a))
    } else {
        Some((Vec2D::from_glam(u.perp() * b), a / b))
    }
}

/// Samples of an ellipse between parameters `t0` and `t1` (see
/// [`crate::sketch::Ellipse::param_at`]), both ends included.
pub fn ellipse_arc_points(
    center: Vec2D,
    major: Vec2D,
    ratio: f64,
    t0: f64,
    t1: f64,
    segments: usize,
) -> Vec<Vec2D> {
    let c = center.to_glam();
    let a = major.to_glam();
    let b = a.perp() * ratio;
    let n = segments.max(1);
    (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * (i as f64) / (n as f64);
            let (sin, cos) = t.sin_cos();
            Vec2D::from_glam(c + a * cos + b * sin)
        })
        .collect()
}

/// A trim/extend-capable curve resolved to positions.
pub(crate) enum Prim {
    Seg {
        a: DVec2,
        b: DVec2,
    },
    Arc {
        c: DVec2,
        r: f64,
        s: DVec2,
        e: DVec2,
    },
    Circle {
        c: DVec2,
        r: f64,
    },
}

pub(crate) fn prim_of(sketch: &Sketch, geom: &GeometryElement) -> Option<Prim> {
    match geom {
        GeometryElement::Line(l) => Some(Prim::Seg {
            a: sketch.point_position(l.start)?.to_glam(),
            b: sketch.point_position(l.end)?.to_glam(),
        }),
        GeometryElement::Arc(a) => {
            let c = sketch.point_position(a.center)?.to_glam();
            let s = sketch.point_position(a.start)?.to_glam();
            Some(Prim::Arc {
                c,
                r: (s - c).length(),
                s,
                e: sketch.point_position(a.end)?.to_glam(),
            })
        }
        GeometryElement::Circle(circle) => Some(Prim::Circle {
            c: sketch.point_position(circle.center)?.to_glam(),
            r: circle.radius,
        }),
        _ => None,
    }
}

/// Whether `p` lies within the prim's own extent (segments by parameter,
/// arcs by angular range; circles are unbounded).
pub(crate) fn within(prim: &Prim, p: DVec2) -> bool {
    match *prim {
        Prim::Seg { a, b } => on_segment(a, b, p),
        Prim::Arc { c, s, e, .. } => point_on_arc(c, s, e, p),
        Prim::Circle { .. } => true,
    }
}

/// Intersections of the *unbounded* carriers of two prims (infinite line /
/// full circle), before any extent filtering.
pub(crate) fn raw_hits(a: &Prim, b: &Prim) -> Vec<DVec2> {
    match (a, b) {
        (Prim::Seg { a: a1, b: a2 }, Prim::Seg { a: b1, b: b2 }) => {
            line_line(*a1, *a2, *b1, *b2).into_iter().collect()
        }
        (Prim::Seg { a: a1, b: a2 }, Prim::Arc { c, r, .. })
        | (Prim::Seg { a: a1, b: a2 }, Prim::Circle { c, r })
        | (Prim::Arc { c, r, .. }, Prim::Seg { a: a1, b: a2 })
        | (Prim::Circle { c, r }, Prim::Seg { a: a1, b: a2 }) => line_circle(*a1, *a2, *c, *r),
        (
            Prim::Arc { c: c1, r: r1, .. } | Prim::Circle { c: c1, r: r1 },
            Prim::Arc { c: c2, r: r2, .. } | Prim::Circle { c: c2, r: r2 },
        ) => circle_circle(*c1, *r1, *c2, *r2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }

    #[test]
    fn line_line_crossing_and_parallel() {
        let p = line_line(v(0.0, 0.0), v(10.0, 0.0), v(5.0, -5.0), v(5.0, 5.0)).unwrap();
        assert!((p - v(5.0, 0.0)).length() < 1e-5);
        assert!(line_line(v(0.0, 0.0), v(10.0, 0.0), v(0.0, 1.0), v(10.0, 1.0)).is_none());
    }

    #[test]
    fn on_segment_respects_extents() {
        // The infinite lines cross at (5, 0) but segment b stops short.
        let p = line_line(v(0.0, 0.0), v(10.0, 0.0), v(5.0, 2.0), v(5.0, 1.0)).unwrap();
        assert!(!on_segment(v(5.0, 2.0), v(5.0, 1.0), p));
        assert!(on_segment(v(5.0, 2.0), v(5.0, -1.0), p));
        // Touching at an endpoint counts.
        assert!(on_segment(v(10.0, 0.0), v(10.0, 5.0), v(10.0, 0.0)));
    }

    #[test]
    fn line_circle_secant_tangent_miss() {
        let hits = line_circle(v(-10.0, 0.0), v(10.0, 0.0), v(0.0, 0.0), 5.0);
        assert_eq!(hits.len(), 2);
        assert!((hits[0] - v(-5.0, 0.0)).length() < 1e-4);
        assert!((hits[1] - v(5.0, 0.0)).length() < 1e-4);

        let hits = line_circle(v(-10.0, 5.0), v(10.0, 5.0), v(0.0, 0.0), 5.0);
        assert_eq!(hits.len(), 1, "tangent line yields one point");
        assert!((hits[0] - v(0.0, 5.0)).length() < 1e-3);

        assert!(line_circle(v(-10.0, 8.0), v(10.0, 8.0), v(0.0, 0.0), 5.0).is_empty());
    }

    #[test]
    fn circle_circle_secant_tangent_miss() {
        let hits = circle_circle(v(0.0, 0.0), 5.0, v(6.0, 0.0), 5.0);
        assert_eq!(hits.len(), 2);
        for p in &hits {
            assert!((p.length() - 5.0).abs() < 1e-4);
            assert!(((*p - v(6.0, 0.0)).length() - 5.0).abs() < 1e-4);
        }
        // Externally tangent.
        let hits = circle_circle(v(0.0, 0.0), 3.0, v(7.0, 0.0), 4.0);
        assert_eq!(hits.len(), 1);
        assert!((hits[0] - v(3.0, 0.0)).length() < 1e-3);
        // Separate, nested, and concentric: no hits.
        assert!(circle_circle(v(0.0, 0.0), 2.0, v(10.0, 0.0), 3.0).is_empty());
        assert!(circle_circle(v(0.0, 0.0), 10.0, v(1.0, 0.0), 2.0).is_empty());
        assert!(circle_circle(v(0.0, 0.0), 5.0, v(0.0, 0.0), 5.0).is_empty());
    }

    #[test]
    fn point_on_arc_checks_angular_range() {
        // CCW quarter arc from (5,0) to (0,5).
        let (c, s, e) = (v(0.0, 0.0), v(5.0, 0.0), v(0.0, 5.0));
        let on = v(
            5.0 * std::f64::consts::FRAC_1_SQRT_2,
            5.0 * std::f64::consts::FRAC_1_SQRT_2,
        );
        assert!(point_on_arc(c, s, e, on));
        assert!(!point_on_arc(c, s, e, v(0.0, -5.0)));
    }

    #[test]
    fn circumcenter_of_right_triangle_is_hypotenuse_midpoint() {
        let c = circumcenter(v(0.0, 0.0), v(6.0, 0.0), v(0.0, 8.0)).unwrap();
        assert!((c - v(3.0, 4.0)).length() < 1e-4);
        assert!(circumcenter(v(0.0, 0.0), v(5.0, 0.0), v(10.0, 0.0)).is_none());
    }

    #[test]
    fn ellipse_points_lie_on_the_ellipse() {
        let pts = ellipse_points(Vec2D::new(1.0, 2.0), Vec2D::new(4.0, 0.0), 0.5, 32);
        assert_eq!(pts.len(), 33);
        assert_eq!(pts[0].x, pts[32].x, "closed polyline");
        for p in &pts {
            let (dx, dy) = (p.x - 1.0, p.y - 2.0);
            let r = (dx / 4.0).powi(2) + (dy / 2.0).powi(2);
            assert!((r - 1.0).abs() < 1e-4, "off-ellipse point {p:?}");
        }
    }

    #[test]
    fn the_longer_radius_is_the_major() {
        let (major, ratio) = ellipse_axes(glam::DVec2::new(4.0, 0.0), 2.0);
        assert_eq!((major, ratio), (glam::DVec2::new(4.0, 0.0), 0.5));
        let (major, ratio) = ellipse_axes(glam::DVec2::new(4.0, 0.0), 8.0);
        assert!(
            (major - glam::DVec2::new(0.0, 8.0)).length() < 1e-5,
            "{major:?}"
        );
        assert!((ratio - 0.5).abs() < 1e-6);
    }
}
