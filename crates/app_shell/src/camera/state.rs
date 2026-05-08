//! Core camera state for a focal-distance CAD viewport: explicit eye position,
//! orientation quaternion, focal distance along the view ray, per-mode projection,
//! and near/far clip planes.

use glam::{DVec3, Mat3, Mat4, Quat, Vec3};
use settings::{CameraSettings, ProjectionMode};

use crate::camera::math::{axis_basis, control_horizontal_vec};

pub const MAX_PITCH_RAD: f32 = 89.9_f32.to_radians();

/// Internal camera state; distances are in **world units (mm)** using `f64` for stability.
#[derive(Debug, Clone)]
pub struct CadCameraState {
    pub eye: DVec3,
    pub orientation: Quat,
    pub focal_distance: f64,
    pub projection: ProjectionMode,
    pub height_angle_rad: f64,
    pub ortho_height: f64,
    pub near_plane: f64,
    pub far_plane: f64,
    pub viewport_origin: (f32, f32),
    pub viewport_size: (u32, u32),
    /// When true, auto near/far may recompute (reduced frequency while interacting).
    pub clip_dirty: bool,
}

impl CadCameraState {
    pub fn new(settings: &CameraSettings, initial_viewport: (u32, u32)) -> Self {
        let fd = (settings.min_focal_distance as f64).max(150.0);
        let h = settings.fov_degrees as f64 * std::f64::consts::PI / 180.0;
        let preset_axes = axes::AxisSystem::from(settings.axis_preset);
        let yaw = 45.0_f32.to_radians();
        let pitch = 35.0_f32.to_radians();
        let orientation = orbit_basis_yaw_pitch(&preset_axes, yaw, pitch).normalize();
        let tmp = Self {
            eye: DVec3::ZERO,
            orientation,
            focal_distance: fd,
            projection: settings.projection,
            height_angle_rad: h,
            ortho_height: settings.ortho_height_mm as f64,
            near_plane: 0.05,
            far_plane: 10_000.0,
            viewport_origin: (0.0, 0.0),
            viewport_size: initial_viewport,
            clip_dirty: true,
        };
        let forward = tmp.forward_world(&preset_axes);
        let eye = DVec3::ZERO
            - DVec3::new(
                forward.x as f64 * fd,
                forward.y as f64 * fd,
                forward.z as f64 * fd,
            );
        Self { eye, ..tmp }
    }

    pub fn aspect(&self) -> f32 {
        let (w, h) = self.viewport_size;
        if w == 0 || h == 0 {
            1.0
        } else {
            w as f32 / h as f32
        }
    }

    /// World-space view direction (eye → focal), unit length.
    pub fn forward_world(&self, axes: &axes::AxisSystem) -> Vec3 {
        let depth = axes.depth().vector();
        (self.orientation * (-depth)).normalize_or_zero()
    }

    pub fn focal_point_dvec(&self, axes: &axes::AxisSystem) -> DVec3 {
        let f = self.forward_world(axes);
        self.eye + DVec3::new(f.x as f64, f.y as f64, f.z as f64) * self.focal_distance
    }

    pub fn focal_point_vec3(&self, axes: &axes::AxisSystem) -> Vec3 {
        let p = self.focal_point_dvec(axes);
        Vec3::new(p.x as f32, p.y as f32, p.z as f32)
    }

    pub fn up_world(&self, axes: &axes::AxisSystem) -> Vec3 {
        (self.orientation * axes.vertical().vector()).normalize_or_zero()
    }

    pub fn right_world(&self, axes: &axes::AxisSystem) -> Vec3 {
        let h = control_horizontal_vec(axes);
        (self.orientation * h).normalize_or_zero()
    }

    pub fn clamp_focal_distance(&mut self, settings: &CameraSettings) {
        let lo = settings.min_focal_distance as f64;
        let hi = settings.max_focal_distance as f64;
        self.focal_distance = self.focal_distance.clamp(lo, hi);
    }

    /// Visible world height at the focal plane (used for pan / zoom-to-cursor scaling).
    pub fn visible_height_at_focal_plane(&self) -> f64 {
        match self.projection {
            ProjectionMode::Perspective => {
                2.0 * self.focal_distance * (self.height_angle_rad * 0.5).tan()
            }
            ProjectionMode::Orthographic => self.ortho_height,
        }
    }

    pub fn world_per_pixel(&self) -> f64 {
        let h = self.viewport_size.1.max(1) as f64;
        self.visible_height_at_focal_plane() / h
    }

    pub fn set_projection_preserve_framing(&mut self, new: ProjectionMode, _axes: &axes::AxisSystem) {
        if new == self.projection {
            return;
        }
        let prev = self.projection;
        match (prev, new) {
            (ProjectionMode::Perspective, ProjectionMode::Orthographic) => {
                self.ortho_height =
                    2.0 * self.focal_distance * (self.height_angle_rad * 0.5).tan();
            }
            (ProjectionMode::Orthographic, ProjectionMode::Perspective) => {
                let tan_half = self.ortho_height / (2.0 * self.focal_distance.max(1e-9));
                self.height_angle_rad = 2.0 * tan_half.clamp(1e-6, 1e6).atan();
            }
            _ => {}
        }
        self.projection = new;
        self.clip_dirty = true;
        tracing::debug!(
            target: "printcad.camera",
            ?prev,
            ?new,
            focal_mm = self.focal_distance,
            ortho_height_mm = self.ortho_height,
            height_angle_deg = self.height_angle_rad.to_degrees(),
            near = self.near_plane,
            far = self.far_plane,
            "projection transition"
        );
    }

    pub fn view_matrix(&self, axes: &axes::AxisSystem) -> Mat4 {
        let eye = self.eye_vec3();
        let at = self.focal_point_vec3(axes);
        let up = self.up_world(axes);
        Mat4::look_at_rh(eye, at, up)
    }

    pub fn view_projection(&self, axes: &axes::AxisSystem) -> Mat4 {
        let view = self.view_matrix(axes);
        let aspect = self.aspect().max(0.001);
        let mut proj = match self.projection {
            ProjectionMode::Perspective => Mat4::perspective_rh(
                self.height_angle_rad as f32,
                aspect,
                self.near_plane as f32,
                self.far_plane as f32,
            ),
            ProjectionMode::Orthographic => {
                let half_h = (self.ortho_height * 0.5) as f32;
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(
                    -half_w,
                    half_w,
                    -half_h,
                    half_h,
                    self.near_plane as f32,
                    self.far_plane as f32,
                )
            }
        };
        // Vulkan clip-space Y flip (same as previous implementation).
        proj.y_axis.y *= -1.0;
        proj * view
    }

    pub fn eye_vec3(&self) -> Vec3 {
        Vec3::new(self.eye.x as f32, self.eye.y as f32, self.eye.z as f32)
    }

    pub fn rederive_eye_from_focal(&mut self, focal: DVec3, axes: &axes::AxisSystem) {
        let f = self.forward_world(axes);
        let fd = self.focal_distance;
        self.eye = focal - DVec3::new(f.x as f64, f.y as f64, f.z as f64) * fd;
    }
}

/// Yaw around world vertical (axis-local horizontal plane), pitch around camera right.
fn orbit_basis_yaw_pitch(axes: &axes::AxisSystem, yaw: f32, pitch: f32) -> Quat {
    let up_axis = axes.vertical().vector().normalize();
    let yaw_q = Quat::from_axis_angle(up_axis, yaw);
    let right_axis = axes.horizontal().vector().normalize();
    let right = (yaw_q * right_axis).normalize();
    let pitch_q = if right.length_squared() > 0.0 {
        Quat::from_axis_angle(right, pitch)
    } else {
        Quat::IDENTITY
    };
    (pitch_q * yaw_q).normalize()
}

/// Rebuild orientation from yaw/pitch in axis-local frame (for reference views).
pub fn orientation_from_yaw_pitch(axes: &axes::AxisSystem, yaw: f32, pitch: f32) -> Quat {
    orbit_basis_yaw_pitch(axes, yaw, pitch.clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD)).normalize()
}

/// Map a canonical yaw/pitch orientation (cube template, Y‑up RH) into world using the axis preset basis.
pub fn canonical_quat_to_world(axes: &axes::AxisSystem, quat: Quat) -> Quat {
    let b = axis_basis(axes);
    let m = b * Mat3::from_quat(quat) * b.transpose();
    Quat::from_mat3(&m).normalize()
}
