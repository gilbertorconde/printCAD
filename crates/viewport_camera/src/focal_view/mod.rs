//! A focal-distance view in the host's axis frame: an explicit `f64` eye,
//! an orientation relative to the frame, a focal distance along the view
//! ray, a per-mode projection and near/far clip planes, with the one
//! view-projection matrix every caller draws, picks and projects through.
//!
//! - `focal_distance` is measured along `forward` (eye to focal point).
//! - Pan translates `eye`; the focal point stays `eye + forward * focal_distance`.
//! - Perspective zoom dollies along `forward`, so the pivot stays fixed unless
//!   a cursor anchor corrects it.
//! - The orientation maps the frame's `-depth` to forward and its `vertical`
//!   to screen-up; it is never built against a fixed XYZ basis.
//!
//! Gestures, transitions and fits run through the shared camera
//! ([`crate::controller::CameraController`]) via [`hold`], converting at the
//! boundary with a model origin frozen for the whole gesture.

pub mod clip;
pub mod hold;
pub mod ops;
pub mod tween;

use glam::{DVec3, Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const MAX_PITCH_RAD: f32 = 89.9_f32.to_radians();
/// The corner a new or framed view looks from: turned 45° about the
/// vertical, and above the scene looking down 35°.
pub const CORNER_YAW_RAD: f32 = 45.0_f32.to_radians();
pub const CORNER_PITCH_RAD: f32 = -35.0_f32.to_radians();

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectionMode {
    Perspective,
    Orthographic,
}

/// The host's axis preset as three world directions: screen-right,
/// screen-up and towards the viewer of the reference view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisFrame {
    pub horizontal: Vec3,
    pub vertical: Vec3,
    pub depth: Vec3,
}

impl AxisFrame {
    pub fn basis(&self) -> Mat3 {
        Mat3::from_cols(self.horizontal, self.vertical, self.depth)
    }

    /// `-1` for a left-handed frame, else `1`.
    pub fn parity(&self) -> f32 {
        let triple = self.horizontal.cross(self.vertical).dot(self.depth);
        if triple < 0.0 { -1.0 } else { 1.0 }
    }

    /// The horizontal that makes a right-handed frame with `vertical` and
    /// `depth`: what screen-right is in every frame.
    pub fn control_horizontal(&self) -> Vec3 {
        if self.parity() < 0.0 {
            -self.horizontal
        } else {
            self.horizontal
        }
    }
}

/// The host's camera preferences the view reads, in its own units
/// (distances in model units, angles in degrees).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    pub min_focal_distance: f32,
    pub max_focal_distance: f32,
    pub projection: ProjectionMode,
    pub fov_degrees: f32,
    pub ortho_height: f32,
    /// Orbit about the frame's vertical (a level horizon) rather than
    /// the camera's own up.
    pub keep_horizon: bool,
    pub orbit_sensitivity: f32,
    pub pan_sensitivity: f32,
    /// Per wheel line multiplicative factor (`factor^lines`).
    pub wheel_zoom_factor: f32,
    pub invert_zoom: bool,
    pub zoom_to_cursor: bool,
    pub auto_near_far: bool,
    pub near_far_near_ratio: f32,
    pub near_far_depth_ratio_cap: f32,
    pub near_far_margin: f32,
    pub view_transition_ms: f32,
}

/// The view's state; distances are in model units, in `f64` for stability
/// far from the origin.
#[derive(Debug, Clone)]
pub struct ViewState {
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
    /// When true, the host re-derives the near and far planes.
    pub clip_dirty: bool,
}

impl ViewState {
    pub fn new(settings: &ViewSettings, frame: &AxisFrame, initial_viewport: (u32, u32)) -> Self {
        let fd = (settings.min_focal_distance as f64).max(150.0);
        let h = settings.fov_degrees as f64 * std::f64::consts::PI / 180.0;
        let orientation =
            orbit_basis_yaw_pitch(frame, CORNER_YAW_RAD, CORNER_PITCH_RAD).normalize();
        let tmp = Self {
            eye: DVec3::ZERO,
            orientation,
            focal_distance: fd,
            projection: settings.projection,
            height_angle_rad: h,
            ortho_height: settings.ortho_height as f64,
            near_plane: 0.05,
            far_plane: 10_000.0,
            viewport_origin: (0.0, 0.0),
            viewport_size: initial_viewport,
            clip_dirty: true,
        };
        let forward = tmp.forward_world(frame);
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

    /// World-space view direction (eye to focal point), unit length.
    pub fn forward_world(&self, frame: &AxisFrame) -> Vec3 {
        (self.orientation * (-frame.depth)).normalize_or_zero()
    }

    pub fn focal_point_dvec(&self, frame: &AxisFrame) -> DVec3 {
        let f = self.forward_world(frame);
        self.eye + DVec3::new(f.x as f64, f.y as f64, f.z as f64) * self.focal_distance
    }

    pub fn focal_point_vec3(&self, frame: &AxisFrame) -> Vec3 {
        let p = self.focal_point_dvec(frame);
        Vec3::new(p.x as f32, p.y as f32, p.z as f32)
    }

    pub fn up_world(&self, frame: &AxisFrame) -> Vec3 {
        (self.orientation * frame.vertical).normalize_or_zero()
    }

    pub fn right_world(&self, frame: &AxisFrame) -> Vec3 {
        (self.orientation * frame.control_horizontal()).normalize_or_zero()
    }

    pub fn clamp_focal_distance(&mut self, settings: &ViewSettings) {
        let lo = settings.min_focal_distance as f64;
        let hi = settings.max_focal_distance as f64;
        self.focal_distance = self.focal_distance.clamp(lo, hi);
    }

    /// Visible world height at the focal plane (pan and zoom-to-cursor scale).
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

    /// Switch projection keeping what stands at the focal point its size.
    pub fn set_projection_preserve_framing(&mut self, new: ProjectionMode, frame: &AxisFrame) {
        if new == self.projection {
            return;
        }
        let origin = self.eye;
        let mut camera = hold::camera(self, frame, origin);
        let kind = match new {
            ProjectionMode::Perspective => crate::controller::ProjectionKind::Perspective {
                vertical_fov: self.height_angle_rad as f32,
            },
            ProjectionMode::Orthographic => crate::controller::ProjectionKind::Orthographic,
        };
        camera.set_projection(kind);
        hold::apply(self, frame, origin, camera);
    }

    pub fn view_matrix(&self, frame: &AxisFrame) -> Mat4 {
        let eye = self.eye_vec3();
        let at = self.focal_point_vec3(frame);
        let up = self.up_world(frame);
        glam::camera::rh::view::look_at_mat4(eye, at, up)
    }

    /// The view-projection matrix, with depth in `0..1` and a Y-down clip
    /// space: the projection's Y is flipped here, and the renderer's vertex
    /// shaders flip it back.
    pub fn view_projection(&self, frame: &AxisFrame) -> Mat4 {
        let view = self.view_matrix(frame);
        let aspect = self.aspect().max(0.001);
        let mut proj = match self.projection {
            ProjectionMode::Perspective => glam::camera::rh::proj::directx::perspective(
                self.height_angle_rad as f32,
                aspect,
                self.near_plane as f32,
                self.far_plane as f32,
            ),
            ProjectionMode::Orthographic => {
                let half_h = (self.ortho_height * 0.5) as f32;
                let half_w = half_h * aspect;
                glam::camera::rh::proj::directx::orthographic(
                    -half_w,
                    half_w,
                    -half_h,
                    half_h,
                    self.near_plane as f32,
                    self.far_plane as f32,
                )
            }
        };
        proj.y_axis.y *= -1.0;
        proj * view
    }

    pub fn eye_vec3(&self) -> Vec3 {
        Vec3::new(self.eye.x as f32, self.eye.y as f32, self.eye.z as f32)
    }

    pub fn rederive_eye_from_focal(&mut self, focal: DVec3, frame: &AxisFrame) {
        let f = self.forward_world(frame);
        let fd = self.focal_distance;
        self.eye = focal - DVec3::new(f.x as f64, f.y as f64, f.z as f64) * fd;
    }

    /// The eye that looks at `focal` from the focal distance with
    /// `orientation`: where a turn about the focal point ends.
    pub fn eye_for(&self, frame: &AxisFrame, orientation: Quat, focal: DVec3) -> DVec3 {
        let forward = orientation * (frame.depth * -1.0);
        focal
            - DVec3::new(forward.x as f64, forward.y as f64, forward.z as f64) * self.focal_distance
    }

    /// Change the perspective's field of view keeping the framing: the eye
    /// moves along the view so what stands at the focal point keeps its size.
    pub fn set_field_of_view(&mut self, frame: &AxisFrame, radians: f64, settings: &ViewSettings) {
        let old = self.height_angle_rad;
        let focal = self.focal_point_dvec(frame);
        self.focal_distance *= (old * 0.5).tan() / (radians * 0.5).tan();
        self.clamp_focal_distance(settings);
        self.height_angle_rad = radians;
        self.rederive_eye_from_focal(focal, frame);
        self.clip_dirty = true;
    }
}

/// Yaw around the frame's vertical, then pitch around the turned horizontal.
fn orbit_basis_yaw_pitch(frame: &AxisFrame, yaw: f32, pitch: f32) -> Quat {
    let up_axis = frame.vertical.normalize();
    let yaw_q = Quat::from_axis_angle(up_axis, yaw);
    let right_axis = frame.horizontal.normalize();
    let right = (yaw_q * right_axis).normalize();
    let pitch_q = if right.length_squared() > 0.0 {
        Quat::from_axis_angle(right, pitch)
    } else {
        Quat::IDENTITY
    };
    (pitch_q * yaw_q).normalize()
}

/// The orientation of a yaw and pitch in the frame, pitch held short of the poles.
pub fn orientation_from_yaw_pitch(frame: &AxisFrame, yaw: f32, pitch: f32) -> Quat {
    orbit_basis_yaw_pitch(frame, yaw, pitch.clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD)).normalize()
}

/// Map an orientation given in the canonical Y-up right-handed frame into
/// the world through the frame's basis.
pub fn canonical_quat_to_world(frame: &AxisFrame, quat: Quat) -> Quat {
    let b = frame.basis();
    let m = b * Mat3::from_quat(quat) * b.transpose();
    Quat::from_mat3(&m).normalize()
}

/// The orientation that looks along `forward` with `up` as screen-up:
/// the rotation taking the frame's `-depth` and `vertical` onto them.
/// `up` must be square to `forward`.
pub fn orientation_facing(frame: &AxisFrame, forward: Vec3, up: Vec3) -> Quat {
    let src_forward = -frame.depth;
    let src = Mat3::from_cols(
        src_forward,
        frame.vertical,
        src_forward.cross(frame.vertical),
    );
    let dst = Mat3::from_cols(forward, up, forward.cross(up));
    Quat::from_mat3(&(dst * src.transpose())).normalize()
}

/// The view direction nearest `forward` among the 26 of a cube of views
/// (its faces, edges and corners), and the up to keep with it: `vertical`
/// made square to it, or for a view along `vertical` the axis nearest `up`.
pub fn nearest_view(forward: Vec3, up: Vec3, vertical: Vec3) -> (Vec3, Vec3) {
    let mut best = (f32::MIN, Vec3::NEG_Z);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let d = Vec3::new(x as f32, y as f32, z as f32);
                if d == Vec3::ZERO {
                    continue;
                }
                let d = d.normalize();
                let along = d.dot(forward);
                if along > best.0 {
                    best = (along, d);
                }
            }
        }
    }
    let direction = best.1;
    let square = vertical - direction * vertical.dot(direction);
    let up = if square.length_squared() > 1e-6 {
        square.normalize()
    } else {
        [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ]
        .into_iter()
        .filter(|a| a.dot(direction).abs() < 1e-6)
        .max_by(|a, b| a.dot(up).total_cmp(&b.dot(up)))
        .unwrap_or(Vec3::Y)
    };
    (direction, up)
}

#[cfg(test)]
#[path = "../focal_view_tests.rs"]
mod tests;
