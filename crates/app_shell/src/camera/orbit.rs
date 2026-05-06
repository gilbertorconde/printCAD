use glam::{Quat, Vec2};
use settings::CameraSettings;

use super::controller::{CameraController, DEG_TO_RAD};

impl CameraController {
    pub(super) fn orbit_trackball(&mut self, delta: Vec2, settings: &CameraSettings) {
        let sens = settings.orbit_sensitivity * 0.005;

        // egui delivers `delta.y > 0` when the cursor moves down on screen.
        // After the Vulkan Y-flip in `view_proj`, world-up renders at the top of
        // the framebuffer, so a "drag the part" trackball needs *positive* dy
        // when the cursor drops — pulling the bottom of the part forward, which
        // makes the scene track the cursor.
        let dx = delta.x * sens;
        let dy = delta.y * sens;

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

        // `right` and `up` are the world-space directions the camera target
        // should slide so the scene tracks the cursor (drag-right ⇒ part right,
        // drag-down ⇒ part down). Both are negated relative to the camera's
        // canonical "right" / "up" axes because moving the target is equivalent
        // to moving the camera, so to make the part appear to follow the
        // cursor the camera has to slide opposite. Note this used to use
        // `-axis_vertical_vec`, but with the Vulkan Y-flip in `view_proj`
        // world-up now renders at the top of the framebuffer; the sign matched
        // the old upside-down render and is flipped accordingly.
        let right = (self.orientation * -self.control_horizontal_vec()).normalize_or_zero();
        let up = (self.orientation * self.axis_vertical_vec()).normalize_or_zero();

        let fov_rad = self.fov_y_deg * DEG_TO_RAD;
        let visible_height = 2.0 * self.radius * (fov_rad * 0.5).tan();
        let world_per_pixel = visible_height / height;

        let offset = (delta.x * world_per_pixel) * right + (delta.y * world_per_pixel) * up;
        self.target += offset;
    }

    pub(super) fn zoom(&mut self, amount: f32, settings: &CameraSettings) {
        // Multiplicative (exponential) zoom: each scroll tick scales the
        // camera radius by a fixed fraction so the apparent speed stays
        // constant regardless of distance from the pivot. `exp(-step)` is
        // symmetric around zero, which keeps zoom-in and zoom-out parity
        // exact (`exp(-x) * exp(x) == 1`).
        let direction = if settings.invert_zoom { 1.0 } else { -1.0 };
        let step = amount * direction * settings.zoom_sensitivity;
        let scale = (-step).exp();
        self.radius =
            (self.radius * scale).clamp(settings.min_distance, settings.max_distance);
    }
}
