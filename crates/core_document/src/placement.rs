//! Where a body sits in the document: a rotation and then a translation
//! applied to its own geometry. A body's features, sketches and kernel shape
//! stay in the body's frame; what the scene draws and picks is placed.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::TriMesh;

/// A rigid motion: points go through `rotation`, then `translation`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BodyPlacement {
    /// Millimetres.
    pub translation: [f32; 3],
    /// A unit quaternion, `[x, y, z, w]`.
    pub rotation: [f32; 4],
}

impl Default for BodyPlacement {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl BodyPlacement {
    pub const IDENTITY: BodyPlacement = BodyPlacement {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };

    pub fn new(rotation: Quat, translation: Vec3) -> Self {
        Self {
            translation: translation.to_array(),
            rotation: rotation.normalize().to_array(),
        }
    }

    pub fn quat(&self) -> Quat {
        Quat::from_array(self.rotation).normalize()
    }

    pub fn offset(&self) -> Vec3 {
        Vec3::from_array(self.translation)
    }

    /// Leaves everything where it is, to within a micron and a microradian.
    pub fn is_identity(&self) -> bool {
        self.offset().length() < 1e-3 && self.quat().angle_between(Quat::IDENTITY) < 1e-6
    }

    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        (self.quat() * Vec3::from_array(p) + self.offset()).to_array()
    }

    pub fn direction(&self, d: [f32; 3]) -> [f32; 3] {
        (self.quat() * Vec3::from_array(d)).to_array()
    }

    /// The motion that undoes this one.
    pub fn inverse(&self) -> Self {
        let q = self.quat().inverse();
        Self::new(q, -(q * self.offset()))
    }

    /// `self` after `first`: a point goes through `first`, then `self`.
    pub fn after(&self, first: &BodyPlacement) -> Self {
        Self::new(
            self.quat() * first.quat(),
            self.quat() * first.offset() + self.offset(),
        )
    }

    /// As a row-major 4×4 matrix in double precision, the kernel's form.
    pub fn rows(&self) -> [[f64; 4]; 4] {
        glam::DMat4::from_rotation_translation(self.quat().as_dquat(), self.offset().as_dvec3())
            .transpose()
            .to_cols_array_2d()
    }

    /// The rigid motion a row-major 4×4 matrix holds, the inverse of
    /// [`rows`](Self::rows); the bottom row is not read. `None` when a
    /// number is not finite or the matrix scales or mirrors by more than
    /// a millionth.
    pub fn from_rows(rows: &[[f64; 4]; 4]) -> Option<Self> {
        if rows.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        let m = glam::DMat4::from_cols_array_2d(rows).transpose();
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        if (scale - glam::DVec3::ONE).abs().max_element() > 1e-6 {
            return None;
        }
        Some(Self::new(rotation.as_quat(), translation.as_vec3()))
    }

    /// `self` after the row-major transform `first`, in double precision
    /// and as rows: a point goes through `first`, then `self`.
    pub fn after_rows(&self, first: &[[f64; 4]; 4]) -> [[f64; 4]; 4] {
        let a = glam::DMat4::from_cols_array_2d(&self.rows()).transpose();
        let b = glam::DMat4::from_cols_array_2d(first).transpose();
        (a * b).transpose().to_cols_array_2d()
    }

    /// As a column-major 4×4 matrix.
    pub fn matrix(&self) -> [[f32; 4]; 4] {
        glam::Mat4::from_rotation_translation(self.quat(), self.offset()).to_cols_array_2d()
    }

    /// A copy of `mesh` moved by this placement: positions, normals and
    /// the faces' surfaces. Indices, outlines and colours are unchanged.
    pub fn mesh(&self, mesh: &TriMesh) -> TriMesh {
        let mut out = mesh.clone();
        for p in &mut out.positions {
            *p = self.point(*p);
        }
        for n in &mut out.normals {
            *n = self.direction(*n);
        }
        for surface in &mut out.face_surfaces {
            *surface = surface.moved(|p| self.point(p), |d| self.direction(d));
        }
        out
    }

    /// The box around `bounds` once moved.
    pub fn bounds(&self, bounds: ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
        kernel_api::Aabb::from(bounds)
            .moved(|p| self.point(p))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    fn turn_and_shift() -> BodyPlacement {
        BodyPlacement::new(
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 0.0, 5.0),
        )
    }

    #[test]
    fn a_point_turns_then_moves() {
        let p = turn_and_shift();
        assert!(close(p.point([1.0, 0.0, 0.0]), [10.0, 1.0, 5.0]));
        assert!(close(p.direction([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
    }

    #[test]
    fn the_inverse_undoes_and_composition_chains() {
        let p = turn_and_shift();
        let q = [3.0, -2.0, 7.0];
        assert!(close(p.inverse().point(p.point(q)), q));
        assert!(p.inverse().after(&p).is_identity());
        let twice = p.after(&p);
        assert!(close(twice.point(q), p.point(p.point(q))));
    }

    /// Rows read back as the motion written, a composition in rows agrees
    /// with `after`, and a scaled or broken matrix is no placement.
    #[test]
    fn rows_round_trip_and_compose() {
        let p = turn_and_shift();
        let q = BodyPlacement::new(Quat::from_rotation_x(0.3), Vec3::new(-1.0, 2.0, 0.5));
        let back = BodyPlacement::from_rows(&p.rows()).unwrap();
        assert!(back.inverse().after(&p).is_identity());
        let composed = BodyPlacement::from_rows(&p.after_rows(&q.rows())).unwrap();
        assert!(close(
            composed.point([3.0, -2.0, 7.0]),
            p.after(&q).point([3.0, -2.0, 7.0])
        ));
        let mut scaled = p.rows();
        scaled[2][2] *= 2.0;
        assert!(BodyPlacement::from_rows(&scaled).is_none());
        let mut broken = p.rows();
        broken[1][3] = f64::NAN;
        assert!(BodyPlacement::from_rows(&broken).is_none());
    }

    #[test]
    fn bounds_hold_every_moved_corner() {
        let p = turn_and_shift();
        let (lo, hi) = p.bounds(([0.0, 0.0, 0.0], [2.0, 1.0, 1.0]));
        assert!(close(lo, [9.0, 0.0, 5.0]));
        assert!(close(hi, [10.0, 2.0, 6.0]));
    }
}
