//! 2D curve math shared by the editing tools: segment/arc/circle
//! intersections plus ellipse and B-spline sampling.

use glam::Vec2;

use crate::sketch::Vec2D;
use crate::snap::arc_angles;

/// Below this the two directions/points are treated as coincident.
const EPS: f32 = 1e-6;

/// Intersection of the infinite lines through `a1→a2` and `b1→b2`.
/// `None` for (near-)parallel lines.
pub fn line_line(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> Option<Vec2> {
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
/// `a→b`, with a small relative slack for intersections computed in f32.
pub fn on_segment(a: Vec2, b: Vec2, p: Vec2) -> bool {
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
pub fn line_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Vec<Vec2> {
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
pub fn circle_circle(c1: Vec2, r1: f32, c2: Vec2, r2: f32) -> Vec<Vec2> {
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
pub fn point_on_arc(center: Vec2, start: Vec2, end: Vec2, p: Vec2) -> bool {
    let (start_angle, sweep) = arc_angles(start - center, end - center);
    let rel = wrap_positive((p - center).y.atan2((p - center).x) - start_angle);
    rel <= sweep + 1e-4
}

/// Circumcenter of three points; `None` when they are (near-)collinear.
pub fn circumcenter(a: Vec2, b: Vec2, c: Vec2) -> Option<Vec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let scale = (b - a).length() * (c - a).length();
    if d.abs() < 1e-5 * scale.max(EPS) {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    Some(Vec2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    ))
}

/// Wrap an angle into [0, TAU).
pub fn wrap_positive(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a < 0.0 {
        a += std::f32::consts::TAU;
    }
    a
}

/// Polyline sampling of an ellipse (`center`, center→major-vertex vector,
/// minor/major `ratio`), closed (first == last point).
pub fn ellipse_points(center: Vec2D, major: Vec2D, ratio: f32, segments: usize) -> Vec<Vec2D> {
    let c = center.to_glam();
    let a = major.to_glam();
    let b = a.perp() * ratio;
    (0..=segments)
        .map(|i| {
            let t = std::f32::consts::TAU * (i as f32) / (segments as f32);
            let (sin, cos) = t.sin_cos();
            Vec2D::from_glam(c + a * cos + b * sin)
        })
        .collect()
}

/// Polyline sampling of a cubic B-spline over `ctrl`. Open splines are
/// clamped (they pass through the first and last control point); periodic
/// splines close smoothly (first == last sample). Fewer than 2 control
/// points yield an empty polyline.
pub fn bspline_points(ctrl: &[Vec2D], periodic: bool, samples: usize) -> Vec<Vec2D> {
    let n = ctrl.len();
    if n < 2 {
        return Vec::new();
    }
    let degree = 3.min(n - 1);
    // Periodic splines wrap `degree` extra control points; the knot vector
    // is uniform. Open splines use a clamped uniform knot vector.
    let (points, knots): (Vec<Vec2>, Vec<f32>) = if periodic {
        let pts: Vec<Vec2> = ctrl
            .iter()
            .chain(ctrl.iter().take(degree))
            .map(|p| p.to_glam())
            .collect();
        let knots = (0..pts.len() + degree + 1).map(|i| i as f32).collect();
        (pts, knots)
    } else {
        let pts: Vec<Vec2> = ctrl.iter().map(|p| p.to_glam()).collect();
        let m = pts.len() + degree + 1;
        let inner = m - 2 * (degree + 1);
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..=inner).map(|i| i as f32 / (inner + 1) as f32));
        knots.extend(std::iter::repeat_n(1.0, degree + 1));
        (pts, knots)
    };
    let (t0, t1) = (knots[degree], knots[points.len()]);
    (0..=samples)
        .map(|i| {
            let t = t0 + (t1 - t0) * (i as f32) / (samples as f32);
            Vec2D::from_glam(de_boor(&points, &knots, degree, t))
        })
        .collect()
}

/// De Boor evaluation of a degree-`p` B-spline at parameter `t`.
fn de_boor(points: &[Vec2], knots: &[f32], p: usize, t: f32) -> Vec2 {
    // Knot span index k with knots[k] <= t < knots[k+1], clamped to the
    // valid range so t == t1 evaluates the last span.
    let k = knots[p..points.len()]
        .iter()
        .rposition(|&u| u <= t)
        .map_or(p, |i| i + p)
        .min(points.len() - 1);
    let mut d: Vec<Vec2> = (0..=p).map(|j| points[k - p + j]).collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let (lo, hi) = (knots[j + k - p], knots[j + 1 + k - r]);
            let alpha = if (hi - lo).abs() < f32::EPSILON {
                0.0
            } else {
                (t - lo) / (hi - lo)
            };
            d[j] = d[j - 1] * (1.0 - alpha) + d[j] * alpha;
        }
    }
    d[p]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
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
    fn line_circle_hits_clip_to_segment_via_on_segment() {
        // Segment ends inside the circle: only the entry hit remains.
        let (a, b) = (v(-10.0, 0.0), v(0.0, 0.0));
        let hits: Vec<Vec2> = line_circle(a, b, v(0.0, 0.0), 5.0)
            .into_iter()
            .filter(|p| on_segment(a, b, *p))
            .collect();
        assert_eq!(hits.len(), 1);
        assert!((hits[0] - v(-5.0, 0.0)).length() < 1e-4);
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
            5.0 * std::f32::consts::FRAC_1_SQRT_2,
            5.0 * std::f32::consts::FRAC_1_SQRT_2,
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
    fn open_bspline_hits_first_and_last_control_point() {
        let ctrl = [
            Vec2D::new(0.0, 0.0),
            Vec2D::new(5.0, 8.0),
            Vec2D::new(10.0, -3.0),
            Vec2D::new(15.0, 2.0),
        ];
        let pts = bspline_points(&ctrl, false, 24);
        let first = pts.first().unwrap();
        let last = pts.last().unwrap();
        assert!((*first - ctrl[0]).to_glam().length() < 1e-4);
        assert!((*last - ctrl[3]).to_glam().length() < 1e-4);
    }

    #[test]
    fn periodic_bspline_closes() {
        let ctrl = [
            Vec2D::new(0.0, 0.0),
            Vec2D::new(10.0, 0.0),
            Vec2D::new(10.0, 10.0),
            Vec2D::new(0.0, 10.0),
        ];
        let pts = bspline_points(&ctrl, true, 32);
        let first = pts.first().unwrap();
        let last = pts.last().unwrap();
        assert!(
            (*first - *last).to_glam().length() < 1e-3,
            "periodic spline sample loop closes: {first:?} vs {last:?}"
        );
    }
}
