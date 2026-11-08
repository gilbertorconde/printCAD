//! Points and vectors in the caller's 3D coordinate frame, for picking meshes, rays
//! and cameras. The host decides whether that frame is world space, model space or
//! a local floating-origin frame.

use glam::DVec3;

/// Position with finite components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3(pub DVec3);

/// Displacement or direction with finite components; not normalized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector3(pub DVec3);

impl Point3 {
    pub const ORIGIN: Self = Self(DVec3::ZERO);

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self::from_dvec3(DVec3::new(x, y, z))
    }

    /// Return from a DVec3 formula, checking finiteness as `new` does.
    pub fn from_dvec3(v: DVec3) -> Self {
        assert!(v.is_finite(), "Point3: components must be finite");
        Self(v)
    }
}

impl Vector3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self::from_dvec3(DVec3::new(x, y, z))
    }

    /// Return from a DVec3 formula, checking finiteness as `new` does.
    pub fn from_dvec3(v: DVec3) -> Self {
        assert!(v.is_finite(), "Vector3: components must be finite");
        Self(v)
    }
}
