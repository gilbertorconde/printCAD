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
    pub fn bounds(&self, (lo, hi): ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
        let mut out_lo = [f32::INFINITY; 3];
        let mut out_hi = [f32::NEG_INFINITY; 3];
        for i in 0..8 {
            let corner = [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let p = self.point(corner);
            for k in 0..3 {
                out_lo[k] = out_lo[k].min(p[k]);
                out_hi[k] = out_hi[k].max(p[k]);
            }
        }
        (out_lo, out_hi)
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

    #[test]
    fn bounds_hold_every_moved_corner() {
        let p = turn_and_shift();
        let (lo, hi) = p.bounds(([0.0, 0.0, 0.0], [2.0, 1.0, 1.0]));
        assert!(close(lo, [9.0, 0.0, 5.0]));
        assert!(close(hi, [10.0, 2.0, 6.0]));
    }
}
