use glam::{Quat, Vec2};
use settings::CameraSettings;

use super::controller::{CameraController, DEG_TO_RAD};

impl CameraController {
    pub(super) fn orbit_trackball(&mut self, delta: Vec2, settings: &CameraSettings) {
        let sens = settings.orbit_sensitivity * 0.005;

        // Screen deltas (invert Y so drag up => positive pitch)
        let dx = delta.x * sens;
        let dy = -delta.y * sens;

        // Camera-local axes from AxisSystem
        let right = (self.orientation * self.control_horizontal_vec()).normalize_or_zero();
        let up = (self.orientation * self.axis_vertical_vec()).normalize_or_zero();

        if right.length_squared() == 0.0 || up.length_squared() == 0.0 {
            return;
        }

        // Horizontal drag => yaw around vertical role
        let yaw_q = Quat::from_axis_angle(up, dx);
        // Vertical drag   => pitch around horizontal role
        let pitch_q = Quat::from_axis_angle(right, dy);
        let delta_q = yaw_q * pitch_q;

        if let Some(pivot) = self.active_pivot {
            let eye_world = self.position_vec();
            let pivot_to_eye_world = eye_world - pivot;
            let new_pivot_to_eye = delta_q * pivot_to_eye_world;
            let new_eye = pivot + new_pivot_to_eye;

            self.orientation = (delta_q * self.orientation).normalize();

            let new_forward = (self.orientation * -self.axis_depth_vec()).normalize_or_zero();
            self.target = new_eye + new_forward * self.radius;
        } else {
            self.orientation = (delta_q * self.orientation).normalize();
        }
    }

    pub(super) fn pan(&mut self, delta: Vec2) {
        let height = self.viewport_size.1.max(1) as f32;

        let right = (self.orientation * -self.control_horizontal_vec()).normalize_or_zero();
        let up = (self.orientation * -self.axis_vertical_vec()).normalize_or_zero();

        let fov_rad = self.fov_y_deg * DEG_TO_RAD;
        let visible_height = 2.0 * self.radius * (fov_rad * 0.5).tan();
        let world_per_pixel = visible_height / height;

        let offset = (delta.x * world_per_pixel) * right + (delta.y * world_per_pixel) * up;
        self.target += offset;
    }

    pub(super) fn zoom(&mut self, amount: f32, settings: &CameraSettings) {
        let direction = if settings.invert_zoom { 1.0 } else { -1.0 };
        let delta = amount * direction * settings.zoom_sensitivity;
        self.radius = (self.radius + delta).clamp(settings.min_distance, settings.max_distance);
    }
}
