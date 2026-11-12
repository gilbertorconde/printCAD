//! Where a ray crosses a triangle, in single or double precision, one
//! test for every caller so that neighbouring triangles agree on the edge
//! they share.

use crate::nearest::{Real, cross, dot, sub};

/// A coordinate [`ray_triangle`] works in, with its tolerances.
pub trait RayReal: Real {
    /// How far past a triangle's edges, in its barycentric coordinates,
    /// a crossing still counts: a few rounding errors of this precision.
    const EDGE_SLACK: Self;
    /// The determinant below which a ray is taken as parallel to the
    /// triangle, or the triangle as having no area.
    const FLAT: Self;
}

impl RayReal for f32 {
    const EDGE_SLACK: Self = 1e-5;
    const FLAT: Self = 1e-12;
}

impl RayReal for f64 {
    const EDGE_SLACK: Self = 1e-9;
    const FLAT: Self = 1e-12;
}

/// The ray parameter where the ray from `origin` along `direction`
/// crosses triangle `abc`, negative for a crossing behind the origin;
/// whether one behind counts is the caller's to say. For a unit
/// direction it is a distance. A ray parallel to the triangle's plane,
/// or a triangle with no area, is absent.
///
/// Edges and corners count, with [`RayReal::EDGE_SLACK`] of the triangle's
/// barycentric coordinates past them, so a ray through an edge two
/// triangles share hits at least one of them and may hit both; a caller
/// counting surfaces crossed keeps one crossing per distance.
pub fn ray_triangle<S: RayReal>(
    origin: [S; 3],
    direction: [S; 3],
    a: [S; 3],
    b: [S; 3],
    c: [S; 3],
) -> Option<S> {
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(direction, e2);
    let det = dot(e1, p);
    if det < S::FLAT && det > S::ZERO - S::FLAT {
        return None;
    }
    let inv = S::ONE / det;
    let s = sub(origin, a);
    let u = dot(s, p) * inv;
    let (low, high) = (S::ZERO - S::EDGE_SLACK, S::ONE + S::EDGE_SLACK);
    if u < low || u > high {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(direction, q) * inv;
    if v < low || u + v > high {
        return None;
    }
    Some(dot(e2, q) * inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crosses_inside_on_either_side_and_not_beside_or_along() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let down = [0.0, 0.0, -1.0];
        for (from, dir, want) in [
            ([0.25, 0.25, 1.0], down, Some(1.0)),
            ([0.25, 0.25, -2.0], down, Some(-2.0)),
            ([0.0, 0.0, 1.0], down, Some(1.0)),
            ([2.0, 2.0, 1.0], down, None),
            ([0.25, 0.25, 1.0], [1.0, 0.0, 0.0], None),
        ] {
            assert_eq!(ray_triangle::<f64>(from, dir, a, b, c), want, "{from:?}");
        }
        assert_eq!(ray_triangle([0.0, 0.0, 1.0], down, a, a, a), None);
    }

    /// Rays from scattered origins through points of an edge two
    /// triangles share, slanted to the axes: every one hits at least one
    /// of them, in either precision.
    #[test]
    fn a_ray_through_a_shared_edge_hits_a_side_of_it() {
        fn check<S: RayReal>(of: fn(f64) -> S) {
            let point = |p: [f64; 3]| p.map(of);
            let (a, b) = ([0.137, 2.31, 0.71], [3.91, -1.07, 1.93]);
            let (c, d) = ([4.2, 3.3, 0.4], [-0.6, -1.9, 1.1]);
            let mut seed = 12345_u64;
            let mut next = || {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 11) as f64 / (1_u64 << 53) as f64
            };
            for _ in 0..20_000 {
                let s = next();
                let on: [f64; 3] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * s);
                let from = [
                    next() * 20.0 - 10.0,
                    next() * 20.0 - 10.0,
                    15.0 + next() * 10.0,
                ];
                let dir: [f64; 3] = std::array::from_fn(|i| on[i] - from[i]);
                let (from, dir) = (point(from), point(dir));
                let hits = [
                    ray_triangle(from, dir, point(a), point(b), point(c)),
                    ray_triangle(from, dir, point(b), point(a), point(d)),
                ];
                assert!(hits.iter().any(Option::is_some), "through {on:?}");
            }
        }
        check(|x| x as f32);
        check(|x| x);
    }
}
