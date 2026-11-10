//! Conversions between the contract's plain arrays and the kernel's types.

use std::borrow::Borrow;

use kernel_api::ProfilePlane;
use ogeom::core::OgeomResult;
use ogeom::math::{Direction, Frame, Plane, Point, Vector};

use crate::tess;

pub(crate) fn point(p: impl Borrow<[f64; 3]>) -> Point {
    let [x, y, z] = *p.borrow();
    Point::new(x, y, z)
}

pub(crate) fn vector(v: impl Borrow<[f64; 3]>) -> Vector {
    let [x, y, z] = *v.borrow();
    Vector::new(x, y, z)
}

/// A point or vector as its three coordinates.
pub(crate) fn array(p: impl Xyz) -> [f64; 3] {
    p.xyz()
}

pub(crate) trait Xyz {
    fn xyz(&self) -> [f64; 3];
}

impl Xyz for Point {
    fn xyz(&self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

impl Xyz for Vector {
    fn xyz(&self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

impl<T: Xyz> Xyz for &T {
    fn xyz(&self) -> [f64; 3] {
        (*self).xyz()
    }
}

/// A profile plane as the kernel's: its origin, its normal as the frame's
/// z and its x axis as the frame's x. The plane is taken right-handed, its
/// y the normal crossed with x, so a profile plane whose `y_axis` points
/// the other way (a mirrored frame) is read as its right-handed twin and
/// the `y_axis` it carries is not used.
pub(crate) fn plane_of(plane: &ProfilePlane) -> OgeomResult<Plane> {
    let tol = tess::tolerances();
    let z = Direction::new(vector(plane.normal), tol)?;
    let x = Direction::new(vector(plane.x_axis), tol)?;
    Ok(Plane::new(Frame::new(point(plane.origin), z, x, tol)?))
}
