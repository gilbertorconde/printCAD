use crate::orientation_cube::{CameraSnapView, RotateAxis, RotateDelta};
use glam::{Mat4, Quat, Vec3};
use settings::{CameraSettings, ProjectionMode};
use winit::dpi::PhysicalPosition;

pub(super) const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;
pub(super) const WORLD_UP: Vec3 = Vec3::Y;
pub(super) const MAX_PITCH_RAD: f32 = 1.570796; // ~90 degrees

/// Simple animation helper so camera snaps remain smooth when requested.
#[derive(Debug, Clone)]
pub(super) struct CameraAnimation {
    start_orientation: Quat,
    target_orientation: Quat,
    progress: f32,
    duration_secs: f32,
}

impl CameraAnimation {
    fn new(from: Quat, to: Quat, duration_secs: f32) -> Self {
        Self {
            start_orientation: from,
            target_orientation: to,
            progress: 0.0,
            duration_secs,
        }
    }

    fn update(&mut self, dt_secs: f32) -> Option<Quat> {
        self.progress += dt_secs / self.duration_secs.max(1e-3);
        if self.progress >= 1.0 {
            return None;
        }
        let t = 1.0 - (1.0 - self.progress).powi(3); // ease-out cubic
        Some(self.start_orientation.slerp(self.target_orientation, t))
    }

    fn target(&self) -> Quat {
        self.target_orientation
    }
}

#[derive(Debug)]
pub struct CameraController {
    pub(super) target: Vec3,
    pub(super) radius: f32,

    // Optional turntable state (used for snaps/animation rebasing if desired)
    pub(super) yaw: f32,   // around WORLD_UP
    pub(super) pitch: f32, // around camera-right

    // Actual camera orientation
    pub(super) orientation: Quat,

    pub(super) fov_y_deg: f32,
    pub(super) projection: ProjectionMode,
    pub(super) near: f32,
    pub(super) far: f32,

    pub(super) orbiting: bool,
    pub(super) panning: bool,
    pub(super) last_cursor: Option<PhysicalPosition<f64>>,

    pub(super) viewport_origin: (f32, f32),
    pub(super) viewport_size: (u32, u32),

    pub(super) animation: Option<CameraAnimation>,

    // Dynamic orbit pivot support
    /// When set, orbit will use this point instead of target during drag
    pub(super) orbit_pivot: Option<Vec3>,
    /// The pivot point we're actually using for this orbit session (captured at mouse down)
    pub(super) active_pivot: Option<Vec3>,
}

impl CameraController {
    pub fn new(settings: &CameraSettings, initial_viewport: (u32, u32)) -> Self {
        let yaw = 45.0_f32.to_radians();
        let pitch = 35.0_f32.to_radians();

        let fov_degrees = match settings.projection {
            ProjectionMode::Perspective => settings.fov_degrees,
            ProjectionMode::Orthographic => 50.0,
        };

        let mut controller = Self {
            target: Vec3::ZERO,
            radius: settings.min_distance.max(5.0),
            yaw,
            pitch,
            orientation: Quat::IDENTITY,
            fov_y_deg: fov_degrees,
            projection: settings.projection,
            near: 0.05,
            far: 10_000.0,
            orbiting: false,
            panning: false,
            last_cursor: None,
            viewport_origin: (0.0, 0.0),
            viewport_size: initial_viewport,
            animation: None,
            orbit_pivot: None,
            active_pivot: None,
        };

        controller.rebuild_orientation_from_yaw_pitch();
        controller
    }

    /// Recenter the camera on a bounding sphere.
    pub fn reset_to_fit(&mut self, center: Vec3, radius_hint: f32) {
        self.target = center;
        self.radius = radius_hint.max(1.0) * 2.5;

        self.yaw = 45.0_f32.to_radians();
        self.pitch = 30.0_f32.to_radians();
        self.animation = None;
        self.last_cursor = None;
        self.orbiting = false;
        self.panning = false;

        self.rebuild_orientation_from_yaw_pitch();
    }

    fn rebuild_orientation_from_yaw_pitch(&mut self) {
        let yaw_q = Quat::from_axis_angle(WORLD_UP, self.yaw);
        let right = yaw_q * Vec3::X;

        let pitch_q = if right.length_squared() > 0.0 {
            Quat::from_axis_angle(right.normalize(), self.pitch)
        } else {
            Quat::IDENTITY
        };

        self.orientation = (pitch_q * yaw_q).normalize();
    }

    pub fn update(&mut self, dt_secs: f32) -> bool {
        if let Some(anim) = self.animation.as_mut() {
            if let Some(orientation) = anim.update(dt_secs) {
                self.orientation = orientation;
                self.sync_yaw_pitch_from_orientation();
                true
            } else {
                self.orientation = anim.target();
                self.sync_yaw_pitch_from_orientation();
                self.animation = None;
                true
            }
        } else {
            false
        }
    }

    pub fn update_viewport(&mut self, origin: (u32, u32), size: (u32, u32)) {
        self.viewport_origin = (origin.0 as f32, origin.1 as f32);
        self.viewport_size = size;
    }

    pub fn view_projection(&self) -> [[f32; 4]; 4] {
        let (w, h) = self.viewport_size;
        let aspect = if w == 0 || h == 0 {
            1.0
        } else {
            w as f32 / h as f32
        };
        self.view_proj(aspect).to_cols_array_2d()
    }

    /// Get viewport info: (origin_x, origin_y, width, height)
    pub fn viewport_info(&self) -> (f32, f32, u32, u32) {
        (
            self.viewport_origin.0,
            self.viewport_origin.1,
            self.viewport_size.0,
            self.viewport_size.1,
        )
    }

    /// Get the active orbit pivot point (only set while orbiting with a pivot)
    pub fn active_pivot(&self) -> Option<Vec3> {
        self.active_pivot
    }

    /// Project a world position to screen coordinates
    /// Returns (x, y) in pixels relative to viewport, or None if behind camera
    pub fn world_to_screen(&self, world_pos: Vec3) -> Option<(f32, f32)> {
        let (w, h) = self.viewport_size;
        let aspect = if w == 0 || h == 0 {
            1.0
        } else {
            w as f32 / h as f32
        };
        let view_proj = self.view_proj(aspect);

        // Transform to clip space
        let clip = view_proj * world_pos.extend(1.0);

        // Check if behind camera
        if clip.w <= 0.0 {
            return None;
        }

        // Perspective divide to NDC
        let ndc = clip.truncate() / clip.w;

        // Convert NDC to screen coordinates (Vulkan-style, Y grows downward)
        let screen_x = (ndc.x + 1.0) * 0.5 * w as f32 + self.viewport_origin.0;
        let screen_y = (ndc.y + 1.0) * 0.5 * h as f32 + self.viewport_origin.1;

        Some((screen_x, screen_y))
    }

    fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = self.view_matrix();
        let fov_persp_rad = self.fov_y_deg * DEG_TO_RAD;
        let fov_ortho_rad = 50.0_f32.to_radians();
        let proj = match self.projection {
            ProjectionMode::Perspective => {
                Mat4::perspective_rh(fov_persp_rad, aspect.max(0.001), self.near, self.far)
            }
            ProjectionMode::Orthographic => {
                let half_height = self.radius * (fov_ortho_rad * 0.5).tan();
                let half_width = half_height * aspect;
                Mat4::orthographic_rh(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    self.near,
                    self.far,
                )
            }
        };
        proj * view
    }

    fn view_matrix(&self) -> Mat4 {
        let eye = self.position_vec();
        let up = self.orientation * Vec3::Y;
        Mat4::look_at_rh(eye, self.target, up)
    }

    pub(super) fn position_vec(&self) -> Vec3 {
        let forward = self.orientation * Vec3::NEG_Z;
        self.target - forward * self.radius
    }

    pub fn position(&self) -> [f32; 3] {
        self.position_vec().to_array()
    }

    pub fn orientation(&self) -> [f32; 4] {
        self.orientation.to_array()
    }

    pub fn sync_with_settings(&mut self, settings: &CameraSettings) {
        self.radius = self
            .radius
            .clamp(settings.min_distance, settings.max_distance);
        self.projection = settings.projection;
        self.fov_y_deg = settings.fov_degrees;
        self.last_cursor = None;
        self.orbiting = false;
        self.panning = false;
    }

    /// Set a dynamic orbit pivot point.
    /// When orbiting starts, the camera will orbit around this point instead of target.
    /// Call with None to clear the pivot (orbit around target).
    pub fn set_orbit_pivot(&mut self, pivot: Option<Vec3>) {
        if !self.orbiting {
            self.orbit_pivot = pivot;
        }
    }

    pub fn snap_to_view(&mut self, view: CameraSnapView) {
        let target = view.orientation();
        self.animation = Some(CameraAnimation::new(self.orientation, target, 0.25));
    }

    pub fn apply_rotate_delta(&mut self, delta: &RotateDelta, _settings: &CameraSettings) {
        let angle_rad = delta.degrees * DEG_TO_RAD;
        let current = self.orientation;
        let axis = match delta.axis {
            RotateAxis::ScreenX => current * Vec3::NEG_X,
            RotateAxis::ScreenY => current * Vec3::NEG_Y,
            RotateAxis::ScreenZ => current * Vec3::Z,
        };
        if axis.length_squared() <= 0.0 {
            return;
        }
        let rotation = Quat::from_axis_angle(axis.normalize(), angle_rad);
        let target = (rotation * current).normalize();
        self.animation = Some(CameraAnimation::new(current, target, 0.2));
    }

    pub(super) fn sync_yaw_pitch_from_orientation(&mut self) {
        let forward = (self.orientation * Vec3::NEG_Z).normalize();
        let horiz = Vec3::new(forward.x, 0.0, forward.z);
        let horiz_len = horiz.length();

        let yaw = if horiz_len > 1e-5 {
            horiz.x.atan2(horiz.z)
        } else {
            self.yaw
        };

        let pitch = forward
            .y
            .atan2(horiz_len)
            .clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);

        self.yaw = yaw;
        self.pitch = pitch;
    }
}
