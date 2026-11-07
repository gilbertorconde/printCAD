//! Placement of a 2D sketch in model space.
//!
//! Geometry and solving stay two-dimensional. The model/view boundary maps
//! to 3D as `origin + x_axis * x + y_axis * y`.

use glam::DVec3;
use serde::{Deserialize, Serialize};
use sketch_units::space::{Point3, Vector3};

use crate::world::{WorldOffset, WorldPoint, WorldVec};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelPoint3(pub DVec3);

impl ModelPoint3 {
    pub const ORIGIN: Self = Self(DVec3::ZERO);

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        assert!(
            x.is_finite() && y.is_finite() && z.is_finite(),
            "3D point must be finite"
        );
        Self(DVec3::new(x, y, z))
    }

    /// Rewrap the same DVec3 in the unit library, checking finiteness.
    pub fn to_units(self) -> Point3 {
        Point3::from_dvec3(self.0)
    }

    pub fn from_units(point: Point3) -> Self {
        Self::new(point.0.x, point.0.y, point.0.z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelVec3(pub DVec3);

impl ModelVec3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        assert!(
            x.is_finite() && y.is_finite() && z.is_finite(),
            "3D vector must be finite"
        );
        Self(DVec3::new(x, y, z))
    }

    /// Rewrap the same DVec3 in the unit library, checking finiteness.
    pub fn to_units(self) -> Vector3 {
        Vector3::from_dvec3(self.0)
    }
}

/// Orthonormal right-handed sketch basis in model space. Persist axes
/// explicitly so loading does not depend on an auxiliary global axis choice.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SketchPlane {
    origin: ModelPoint3,
    x_axis: ModelVec3,
    y_axis: ModelVec3,
}

impl Default for SketchPlane {
    fn default() -> Self {
        Self {
            origin: ModelPoint3::ORIGIN,
            x_axis: ModelVec3(DVec3::X),
            y_axis: ModelVec3(DVec3::Y),
        }
    }
}

impl SketchPlane {
    pub fn new(origin: ModelPoint3, x_axis: ModelVec3, y_axis: ModelVec3) -> Result<Self, String> {
        let plane = Self {
            origin,
            x_axis,
            y_axis,
        };
        plane.validate()?;
        Ok(plane)
    }

    /// Project the supplied X direction onto the plane, preserving the normal.
    /// Reject near-zero and parallel directions.
    pub fn from_normal_x(
        origin: ModelPoint3,
        normal: ModelVec3,
        x_hint: ModelVec3,
    ) -> Result<Self, String> {
        if !normal.0.is_finite() || !x_hint.0.is_finite() || !origin.0.is_finite() {
            return Err("sketch plane must be finite".into());
        }
        if normal.0.length() <= 1e-12 {
            return Err("sketch plane normal has no direction".into());
        }
        let normal = normal.0.normalize();
        let x = x_hint.0 - normal * x_hint.0.dot(normal);
        if x.length() <= 1e-12 {
            return Err("sketch plane X direction is parallel to its normal".into());
        }
        let x = x.normalize();
        Self::new(origin, ModelVec3(x), ModelVec3(normal.cross(x)))
    }

    pub fn validate(&self) -> Result<(), String> {
        let x = self.x_axis.0;
        let y = self.y_axis.0;
        if !self.origin.0.is_finite() || !x.is_finite() || !y.is_finite() {
            return Err("sketch plane must be finite".into());
        }
        if (x.length_squared() - 1.0).abs() > 1e-10
            || (y.length_squared() - 1.0).abs() > 1e-10
            || x.dot(y).abs() > 1e-10
        {
            return Err("sketch plane axes must be unit length and perpendicular".into());
        }
        Ok(())
    }

    pub fn origin(self) -> ModelPoint3 {
        self.origin
    }
    pub fn x_axis(self) -> ModelVec3 {
        self.x_axis
    }
    pub fn y_axis(self) -> ModelVec3 {
        self.y_axis
    }
    pub fn normal(self) -> ModelVec3 {
        ModelVec3(self.x_axis.0.cross(self.y_axis.0))
    }

    pub fn to_model(self, point: WorldPoint) -> ModelPoint3 {
        self.try_to_model(point)
            .expect("sketch-to-model transform must stay finite")
    }

    /// External coordinates may be finite separately yet overflow their sum on
    /// an oblique/translated plane. Validate before constructing a model point;
    /// internal geometry continues to use the fail-fast `to_model` contract.
    pub fn try_to_model(self, point: WorldPoint) -> Result<ModelPoint3, String> {
        let p = self.origin.0 + self.x_axis.0 * point.0.x + self.y_axis.0 * point.0.y;
        if !p.is_finite() {
            return Err(format!(
                "sketch-to-model transform is nonfinite at ({}, {})",
                point.0.x, point.0.y
            ));
        }
        Ok(ModelPoint3::new(p.x, p.y, p.z))
    }

    /// Map a displacement without the plane origin. Subtracting two absolute
    /// `to_model` positions near 1e7 loses about 2e-9 precision and can move
    /// the direction outside the plane validation tolerance.
    pub fn to_model_vec(self, delta: WorldVec) -> ModelVec3 {
        ModelVec3(self.x_axis.0 * delta.0.x + self.y_axis.0 * delta.0.y)
    }

    /// Orthogonal projection onto local sketch axes. The caller checks distance
    /// to the plane when membership is required.
    pub fn coordinates(self, point: ModelPoint3) -> WorldPoint {
        let delta = point.0 - self.origin.0;
        WorldPoint::new(delta.dot(self.x_axis.0), delta.dot(self.y_axis.0))
    }

    pub fn signed_distance(self, point: ModelPoint3) -> WorldOffset {
        WorldOffset::new((point.0 - self.origin.0).dot(self.normal().0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_plane_roundtrips_local_points() {
        let plane = SketchPlane::from_normal_x(
            ModelPoint3::new(1e6, -2e6, 3e6),
            ModelVec3::new(1.0, 2.0, 3.0),
            ModelVec3::new(4.0, -2.0, 1.0),
        )
        .unwrap();
        let local = WorldPoint::new(123.25, -78.5);
        let model = plane.to_model(local);
        assert_eq!(plane.try_to_model(local).unwrap(), model);
        assert!(plane.coordinates(model).distance(local).0 < 1e-9);
        assert!(plane.signed_distance(model).abs().0 < 1e-9);
    }

    /// A direction far from the origin must remain within the plane validation
    /// tolerance; subtracting absolute model positions exceeds it on these inputs.
    #[test]
    fn far_segment_direction_stays_in_the_plane() {
        let plane = SketchPlane::from_normal_x(
            ModelPoint3::new(1e7 - 9.6, -1e7 - 5.4, 1e7 + 1.6),
            ModelVec3::new(0.3, -0.4, 1.0),
            ModelVec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        let (from, to) = (WorldPoint::new(5.8, 0.0), WorldPoint::new(1.6, 8.6));
        let n = plane.normal().0;
        let local = plane.to_model_vec(to - from).0.normalize();
        assert!(local.dot(n).abs() <= 1e-15, "{}", local.dot(n));
        let absolute = (plane.to_model(to).0 - plane.to_model(from).0).normalize();
        assert!(absolute.dot(n).abs() > 1e-10, "{}", absolute.dot(n));
    }

    #[test]
    fn checked_transform_reports_overflow_but_internal_transform_stays_fail_fast() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let plane = SketchPlane::new(
            ModelPoint3::ORIGIN,
            ModelVec3::new(s, 0.0, s),
            ModelVec3::new(s, 0.0, -s),
        )
        .unwrap();
        let point = WorldPoint::new(f64::MAX, f64::MAX);
        let error = plane.try_to_model(point).unwrap_err();
        assert!(
            error.contains("sketch-to-model transform is nonfinite"),
            "{error}"
        );
        assert!(std::panic::catch_unwind(|| plane.to_model(point)).is_err());
    }

    #[test]
    fn rejects_parallel_axes_and_invalid_file_data() {
        assert!(
            SketchPlane::from_normal_x(
                ModelPoint3::ORIGIN,
                ModelVec3::new(0.0, 0.0, 1.0),
                ModelVec3::new(0.0, 0.0, 2.0),
            )
            .is_err()
        );
        let invalid: SketchPlane =
            serde_json::from_str(r#"{"origin":[0,0,0],"x_axis":[2,0,0],"y_axis":[0,1,0]}"#)
                .unwrap();
        assert!(invalid.validate().is_err());
    }
}
