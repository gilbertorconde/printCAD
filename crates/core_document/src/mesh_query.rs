//! Small queries on display triangles, in the caller's coordinate frame.
//! Mesh traversal, face identity and clipping remain caller policy.

use glam::Vec3;

/// The point of triangle `abc` nearest `p`.
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    if ab.cross(ac).length_squared() == 0.0 {
        return closest_on_rim(p, a, b, c);
    }
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom <= 0.0 {
        return closest_on_rim(p, a, b, c);
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

/// Positive ray parameter at a triangle hit. For a unit direction this
/// is a distance; parallel rays and hits outside the triangle are absent.
pub fn ray_triangle(origin: Vec3, direction: Vec3, t: [Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let along = e2.dot(q) * inv;
    (along > 0.0).then_some(along)
}

/// Squared distance to the nearest point of a triangle.
pub fn triangle_distance_sq(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    closest_on_triangle(p, a, b, c).distance_squared(p)
}

fn closest_on_rim(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    [(a, b), (b, c), (c, a)]
        .into_iter()
        .map(|(a, b)| {
            let ab = b - a;
            let t = if ab.length_squared() == 0.0 {
                0.0
            } else {
                ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            };
            a + ab * t
        })
        .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_points_cover_regions_and_collapsed_triangles() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Y);
        for (p, tri, want) in [
            (
                Vec3::new(0.25, 0.25, 2.0),
                [a, b, c],
                Vec3::new(0.25, 0.25, 0.0),
            ),
            (Vec3::new(2.0, -1.0, 0.0), [a, b, c], b),
            (
                Vec3::new(1.0, 1.0, 0.0),
                [a, b, c],
                Vec3::new(0.5, 0.5, 0.0),
            ),
            (
                Vec3::new(0.5, 1.0, 0.0),
                [a, a, b],
                Vec3::new(0.5, 0.0, 0.0),
            ),
            (
                Vec3::new(0.5, 1.0, 0.0),
                [a, b, b * 2.0],
                Vec3::new(0.5, 0.0, 0.0),
            ),
            (c, [a, a, a], a),
        ] {
            let [a, b, c] = tri;
            assert!(closest_on_triangle(p, a, b, c).distance(want) < 1e-6);
            assert!((triangle_distance_sq(p, a, b, c) - p.distance_squared(want)).abs() < 1e-6);
        }
    }

    #[test]
    fn rays_cross_only_in_front_and_inside() {
        let tri = [Vec3::ZERO, Vec3::X, Vec3::Y];
        for (from, dir, want) in [
            (Vec3::new(0.25, 0.25, 1.0), -Vec3::Z, Some(1.0)),
            (Vec3::new(0.25, 0.25, 1.0), Vec3::Z, None),
            (Vec3::new(0.25, 0.25, 1.0), Vec3::X, None),
            (Vec3::new(2.0, 2.0, 1.0), -Vec3::Z, None),
            (Vec3::Z, -Vec3::Z, Some(1.0)),
        ] {
            assert_eq!(ray_triangle(from, dir, tri), want);
        }
        assert_eq!(ray_triangle(Vec3::Z, -Vec3::Z, [Vec3::ZERO; 3]), None);
    }
}
