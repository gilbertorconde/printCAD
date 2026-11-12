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

/// Positive ray parameter at a triangle hit, through
/// `kernel_api::ray::ray_triangle`: edges count with its slack, so a ray
/// through an edge two triangles share hits at least one. For a unit
/// direction this is a distance; parallel rays, hits outside the triangle
/// and hits behind the origin are absent.
pub fn ray_triangle(origin: Vec3, direction: Vec3, t: [Vec3; 3]) -> Option<f32> {
    kernel_api::ray::ray_triangle(
        origin.to_array(),
        direction.to_array(),
        t[0].to_array(),
        t[1].to_array(),
        t[2].to_array(),
    )
    .filter(|&along| along > 0.0)
}

/// Squared distance to the nearest point of a triangle.
pub fn triangle_distance_sq(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    closest_on_triangle(p, a, b, c).distance_squared(p)
}
