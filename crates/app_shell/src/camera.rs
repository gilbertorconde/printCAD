use crate::orientation_cube::{CameraSnapView, RotateAxis, RotateDelta};
use glam::{Mat4, Quat, Vec2, Vec3};
use settings::{CameraSettings, MouseButtonSetting, ProjectionMode};
use winit::{
    dpi::PhysicalPosition,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
};

const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;
const WORLD_UP: Vec3 = Vec3::Y;
const MAX_PITCH_RAD: f32 = 1.570796; // ~90 degrees

/// Simple animation helper so camera snaps remain smooth when requested.
#[derive(Debug, Clone)]
struct CameraAnimation {
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
    target: Vec3,
    radius: f32,

    // Optional turntable state (used for snaps/animation rebasing if desired)
    yaw: f32,   // around WORLD_UP
    pitch: f32, // around camera-right

    // Actual camera orientation
    orientation: Quat,

    fov_y_deg: f32,
    projection: ProjectionMode,
    near: f32,
    far: f32,

    orbiting: bool,
    panning: bool,
    last_cursor: Option<PhysicalPosition<f64>>,

    viewport_origin: (f32, f32),
    viewport_size: (u32, u32),

    animation: Option<CameraAnimation>,
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
        // You can clamp pitch here if you only use yaw/pitch mode
        // self.pitch = self.pitch.clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);

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

    pub fn handle_event(&mut self, event: &WindowEvent, settings: &CameraSettings) -> bool {
        match event {
            WindowEvent::MouseInput { state, button, .. } => {
                let orbit_button = mouse_button_from_setting(settings.orbit_button);
                let pan_button = mouse_button_from_setting(settings.pan_button);
                let pressed = matches!(state, ElementState::Pressed);
                match (button, pressed) {
                    (b, true) if *b == orbit_button => {
                        self.orbiting = true;
                        self.animation = None; // user input overrides animation
                        true
                    }
                    (b, false) if *b == orbit_button => {
                        self.orbiting = false;
                        // If you want to "rebase" to no-roll here, we can add it:
                        // self.rebase_orientation_remove_roll();
                        self.last_cursor = None;
                        true
                    }
                    (b, true) if *b == pan_button => {
                        self.panning = true;
                        true
                    }
                    (b, false) if *b == pan_button => {
                        self.panning = false;
                        self.last_cursor = None;
                        true
                    }
                    _ => false,
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let changed = self.handle_cursor_moved(*position, settings);
                if self.orbiting || self.panning {
                    self.last_cursor = Some(*position);
                } else {
                    self.last_cursor = None;
                }
                changed
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta, settings);
                true
            }
            WindowEvent::Resized(size) => {
                self.viewport_size = (size.width, size.height);
                false
            }
            _ => false,
        }
    }

    fn handle_cursor_moved(
        &mut self,
        position: PhysicalPosition<f64>,
        settings: &CameraSettings,
    ) -> bool {
        let last = match self.last_cursor {
            Some(pos) => pos,
            None => return false,
        };

        let delta = Vec2::new((position.x - last.x) as f32, (position.y - last.y) as f32);

        if self.orbiting {
            self.orbit_trackball(delta, settings);
            true
        } else if self.panning {
            self.pan(delta);
            true
        } else {
            false
        }
    }

    fn handle_scroll(&mut self, delta: &MouseScrollDelta, settings: &CameraSettings) {
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, y) => *y,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 120.0,
        };
        self.zoom(amount, settings);
    }

    /// Camera-space trackball orbit:
    /// - Horizontal drag: yaw around camera up.
    /// - Vertical drag: pitch around camera right.
    /// Full freedom (can roll, spin over 180°).
    fn orbit_trackball(&mut self, delta: Vec2, settings: &CameraSettings) {
        let sens = settings.orbit_sensitivity * 0.005;
        let dx = -delta.x * sens;
        let dy = delta.y * sens;

        // Camera-local axes
        let right = (self.orientation * Vec3::X).normalize_or_zero();
        let up = (self.orientation * Vec3::Y).normalize_or_zero();

        if right.length_squared() == 0.0 || up.length_squared() == 0.0 {
            return;
        }

        // Drag right => rotate around up (yaw)
        let yaw_q = Quat::from_axis_angle(up, -dx);
        // Drag up   => rotate around right (pitch)
        let pitch_q = Quat::from_axis_angle(right, -dy);

        // Apply pitch then yaw (you can swap order if you prefer)
        let delta_q = yaw_q * pitch_q;
        self.orientation = (delta_q * self.orientation).normalize();
    }

    fn pan(&mut self, delta: Vec2) {
        let height = self.viewport_size.1.max(1) as f32;

        // Camera-local axes
        let right = (self.orientation * Vec3::X).normalize_or_zero();
        let up_cam = (self.orientation * Vec3::Y).normalize_or_zero();

        let fov_rad = self.fov_y_deg * DEG_TO_RAD;
        let visible_height = 2.0 * self.radius * (fov_rad * 0.5).tan();
        let world_per_pixel = visible_height / height;

        // Drag right => move scene right on screen
        let offset = (-delta.x * world_per_pixel) * right + (-delta.y * world_per_pixel) * up_cam;

        self.target += offset;
    }

    fn zoom(&mut self, amount: f32, settings: &CameraSettings) {
        let direction = if settings.invert_zoom { 1.0 } else { -1.0 };
        let delta = amount * direction * settings.zoom_sensitivity;
        self.radius = (self.radius + delta).clamp(settings.min_distance, settings.max_distance);
    }

    /// Sync yaw/pitch from the current orientation (used after animations).
    fn sync_yaw_pitch_from_orientation(&mut self) {
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

    fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = self.view_matrix();
        let fov_persp_rad = self.fov_y_deg * DEG_TO_RAD;
        let fov_ortho_rad = 50.0_f32.to_radians();
        let proj = match self.projection {
            ProjectionMode::Perspective => {
                Mat4::perspective_rh_gl(fov_persp_rad, aspect.max(0.001), self.near, self.far)
            }
            ProjectionMode::Orthographic => {
                let half_height = self.radius * (fov_ortho_rad * 0.5).tan();
                let half_width = half_height * aspect;
                Mat4::orthographic_rh_gl(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    -self.far,
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

    fn position_vec(&self) -> Vec3 {
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

    /// Optional: call this on orbit end if you want to remove roll but keep view.
    #[allow(dead_code)]
    fn rebase_orientation_remove_roll(&mut self) {
        let forward = (self.orientation * Vec3::NEG_Z).normalize();
        let world_up = WORLD_UP;

        // Compute canonical right/up with no roll
        let mut right = forward.cross(world_up);
        if right.length_squared() < 1e-6 {
            right = if forward.y.abs() < 0.9 {
                Vec3::X
            } else {
                Vec3::Z
            };
        }
        right = right.normalize();
        let up = right.cross(forward).normalize();

        let basis = glam::Mat3::from_cols(right, up, -forward);
        self.orientation = Quat::from_mat3(&basis).normalize();
    }
}

fn mouse_button_from_setting(setting: MouseButtonSetting) -> MouseButton {
    match setting {
        MouseButtonSetting::Left => MouseButton::Left,
        MouseButtonSetting::Middle => MouseButton::Middle,
        MouseButtonSetting::Right => MouseButton::Right,
    }
}
