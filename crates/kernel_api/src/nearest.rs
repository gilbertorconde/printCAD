//! The point of a segment or a triangle nearest a given point: one
//! implementation over plain coordinates, in single or double precision,
//! a segment in any number of dimensions and a triangle in three.

use std::ops::{Add, Div, Mul, Sub};

/// A coordinate the queries work in: `f32` or `f64`.
pub trait Real:
    Copy
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + sealed::Sealed
{
    const ZERO: Self;
    const ONE: Self;
}

impl Real for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
}

impl Real for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

/// How far along the segment from `a` to `b` its point nearest `p` lies,
/// from 0 at `a` to 1 at `b`. A segment with no length answers 0, its
/// start.
pub fn segment_param<S: Real, const N: usize>(p: [S; N], a: [S; N], b: [S; N]) -> S {
    let ab = sub(b, a);
    let length_sq = dot(ab, ab);
    // Also false for a NaN length, which answers the start as well.
    let t = if length_sq > S::ZERO {
        dot(sub(p, a), ab) / length_sq
    } else {
        S::ZERO
    };
    if t < S::ZERO {
        S::ZERO
    } else if t > S::ONE {
        S::ONE
    } else {
        t
    }
}

/// The point of the segment from `a` to `b` nearest `p`; `a` for a
/// segment with no length.
pub fn nearest_on_segment<S: Real, const N: usize>(p: [S; N], a: [S; N], b: [S; N]) -> [S; N] {
    along(a, sub(b, a), segment_param(p, a, b))
}

/// The point of triangle `abc` nearest `p`. A triangle with no area
/// answers the nearest point of its sides.
pub fn nearest_on_triangle<S: Real>(p: [S; 3], a: [S; 3], b: [S; 3], c: [S; 3]) -> [S; 3] {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let n = cross(ab, ac);
    if dot(n, n) == S::ZERO {
        return nearest_on_sides(p, a, b, c);
    }
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= S::ZERO && d2 <= S::ZERO {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= S::ZERO && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= S::ZERO && d1 >= S::ZERO && d3 <= S::ZERO {
        return along(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= S::ZERO && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= S::ZERO && d2 >= S::ZERO && d6 <= S::ZERO {
        return along(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= S::ZERO && (d4 - d3) >= S::ZERO && (d5 - d6) >= S::ZERO {
        return along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom <= S::ZERO {
        return nearest_on_sides(p, a, b, c);
    }
    along(along(a, ab, vb / denom), ac, vc / denom)
}

fn nearest_on_sides<S: Real>(p: [S; 3], a: [S; 3], b: [S; 3], c: [S; 3]) -> [S; 3] {
    let mut best = nearest_on_segment(p, a, b);
    for (u, v) in [(b, c), (c, a)] {
        let q = nearest_on_segment(p, u, v);
        let (dq, db) = (sub(q, p), sub(best, p));
        if dot(dq, dq) < dot(db, db) {
            best = q;
        }
    }
    best
}

fn sub<S: Real, const N: usize>(a: [S; N], b: [S; N]) -> [S; N] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot<S: Real, const N: usize>(a: [S; N], b: [S; N]) -> S {
    (1..N).fold(a[0] * b[0], |s, i| s + a[i] * b[i])
}

/// `from + by * t`.
fn along<S: Real, const N: usize>(from: [S; N], by: [S; N], t: S) -> [S; N] {
    std::array::from_fn(|i| from[i] + by[i] * t)
}

fn cross<S: Real>(a: [S; 3], b: [S; 3]) -> [S; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near<const N: usize>(got: [f64; N], want: [f64; N]) -> bool {
        got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-12)
    }

    #[test]
    fn a_segment_clamps_at_both_ends_and_a_point_segment_answers_its_start() {
        let (a, b) = ([1.0, 1.0], [3.0, 1.0]);
        for (p, t, want) in [
            ([2.5, 4.0], 0.75, [2.5, 1.0]),
            ([-5.0, 0.0], 0.0, a),
            ([9.0, -2.0], 1.0, b),
        ] {
            assert_eq!(segment_param(p, a, b), t);
            assert!(near(nearest_on_segment(p, a, b), want));
        }
        let p = [0.0, 0.0, 7.0];
        assert_eq!(segment_param(p, [2.0; 3], [2.0; 3]), 0.0);
        assert_eq!(nearest_on_segment(p, [2.0; 3], [2.0; 3]), [2.0; 3]);
        assert_eq!(
            nearest_on_segment([0.5f32, 9.0], [0.0; 2], [1.0, 0.0]),
            [0.5, 0.0]
        );
    }

    #[test]
    fn a_triangle_answers_its_face_sides_and_corners_and_a_flat_one_its_sides() {
        let (a, b, c) = ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        for (p, [a, b, c], want) in [
            ([0.25, 0.25, 2.0], [a, b, c], [0.25, 0.25, 0.0]),
            ([2.0, -1.0, 0.0], [a, b, c], b),
            ([1.0, 1.0, 0.0], [a, b, c], [0.5, 0.5, 0.0]),
            ([0.5, 1.0, 0.0], [a, a, b], [0.5, 0.0, 0.0]),
            ([0.5, 1.0, 0.0], [a, b, [2.0, 0.0, 0.0]], [0.5, 0.0, 0.0]),
            (c, [a, a, a], a),
        ] {
            assert!(near(nearest_on_triangle(p, a, b, c), want), "{p:?}");
        }
    }
}
