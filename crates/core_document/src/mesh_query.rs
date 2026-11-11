//! Small queries on display triangles, in the caller's coordinate frame.
//! Mesh traversal, face identity and clipping remain caller policy.

use glam::{DVec2, DVec3, Vec2, Vec3};
use kernel_api::nearest;

/// A glam point the nearest-point queries take, answered by
/// `kernel_api::nearest` on its coordinates.
pub trait Point: Copy {
    type Scalar: nearest::Real;
    fn param_on(self, a: Self, b: Self) -> Self::Scalar;
    fn on_segment(self, a: Self, b: Self) -> Self;
}

/// A point in space, which a triangle's nearest point takes.
pub trait SpacePoint: Point {
    fn on_triangle(self, a: Self, b: Self, c: Self) -> Self;
}

macro_rules! point {
    ($($point:ty: $scalar:ty),*) => {$(
        impl Point for $point {
            type Scalar = $scalar;
            fn param_on(self, a: Self, b: Self) -> $scalar {
                nearest::segment_param(self.to_array(), a.to_array(), b.to_array())
            }
            fn on_segment(self, a: Self, b: Self) -> Self {
                Self::from_array(nearest::nearest_on_segment(
                    self.to_array(),
                    a.to_array(),
                    b.to_array(),
                ))
            }
        }
    )*};
}

point!(Vec2: f32, Vec3: f32, DVec2: f64, DVec3: f64);

macro_rules! space_point {
    ($($point:ty),*) => {$(
        impl SpacePoint for $point {
            fn on_triangle(self, a: Self, b: Self, c: Self) -> Self {
                Self::from_array(nearest::nearest_on_triangle(
                    self.to_array(),
                    a.to_array(),
                    b.to_array(),
                    c.to_array(),
                ))
            }
        }
    )*};
}

space_point!(Vec3, DVec3);

/// How far along the segment from `a` to `b` its point nearest `p` lies,
/// from 0 at `a` to 1 at `b`; 0 for a segment with no length.
pub fn segment_param<P: Point>(p: P, a: P, b: P) -> P::Scalar {
    p.param_on(a, b)
}

/// The point of the segment from `a` to `b` nearest `p`; `a` for a
/// segment with no length.
pub fn nearest_on_segment<P: Point>(p: P, a: P, b: P) -> P {
    p.on_segment(a, b)
}

/// The point of triangle `abc` nearest `p`; the nearest point of its
/// sides for a triangle with no area.
pub fn closest_on_triangle<P: SpacePoint>(p: P, a: P, b: P, c: P) -> P {
    p.on_triangle(a, b, c)
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

#[cfg(test)]
mod tests {
    use super::*;

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
