//! Instant view commands and device-rate steps through the shared camera:
//! pan, orbit, roll, zoom, pivot moves and fits.

use glam::{DVec3, Vec2, Vec3};

use super::hold::{self, Hold};
use super::{AxisFrame, CORNER_PITCH_RAD, CORNER_YAW_RAD, ProjectionMode, ViewSettings, ViewState};
use crate::controller::GestureKind;
use crate::fit::{BoundingSphere, fit};

pub fn pan_pixels(
    state: &mut ViewState,
    frame: &AxisFrame,
    delta_px: Vec2,
    settings: &ViewSettings,
) {
    if delta_px == Vec2::ZERO {
        return;
    }
    let hold = Hold::begin(
        state,
        frame,
        settings,
        Vec2::ZERO,
        state.focal_point_dvec(frame),
        GestureKind::Pan,
        1.0,
    );
    hold.sample(state, frame, delta_px);
}

pub fn orbit_pixels(
    state: &mut ViewState,
    frame: &AxisFrame,
    delta_px: Vec2,
    settings: &ViewSettings,
) {
    orbit_pixels_around_world_anchor(
        state,
        frame,
        state.focal_point_dvec(frame),
        delta_px,
        settings,
    );
}

pub fn orbit_pixels_around_world_anchor(
    state: &mut ViewState,
    frame: &AxisFrame,
    pivot_world: DVec3,
    delta_px: Vec2,
    settings: &ViewSettings,
) {
    if delta_px == Vec2::ZERO {
        return;
    }
    let hold = Hold::begin(
        state,
        frame,
        settings,
        Vec2::ZERO,
        pivot_world,
        GestureKind::Orbit,
        1.0,
    );
    hold.sample(state, frame, delta_px);
}

/// Turn about the view direction, 0.01 radians per pixel.
pub fn roll_pixels(state: &mut ViewState, frame: &AxisFrame, delta_px_x: f32) {
    if delta_px_x == 0.0 {
        return;
    }
    let forward = state.forward_world(frame);
    state.orientation =
        (glam::Quat::from_axis_angle(forward, delta_px_x * 0.01) * state.orientation).normalize();
    state.clip_dirty = true;
}

/// Move the focal point onto `hit` without turning the view. Refused
/// (`false`) for a point behind the eye in perspective.
pub fn set_pivot_world_hit(
    state: &mut ViewState,
    frame: &AxisFrame,
    hit: Vec3,
    settings: &ViewSettings,
) -> bool {
    let depth = (hit - state.eye_vec3()).dot(state.forward_world(frame));
    // Orthographic clipping includes geometry behind the eye. Recentring a
    // picked point there keeps the eye in front without changing the picture's scale.
    if depth > 1e-4 {
        state.focal_distance = f64::from(depth);
    } else if state.projection != ProjectionMode::Orthographic {
        return false;
    }
    state.clamp_focal_distance(settings);
    state.rederive_eye_from_focal(hit.as_dvec3(), frame);
    state.clip_dirty = true;
    true
}

/// Exponential zoom (`factor^wheel_lines`), about the point under
/// `viewport_cursor` on the focal plane when zooming to the cursor.
pub fn apply_zoom_wheels(
    state: &mut ViewState,
    frame: &AxisFrame,
    viewport_cursor: Option<Vec2>,
    wheel_lines: f32,
    settings: &ViewSettings,
) {
    if wheel_lines == 0.0 {
        return;
    }
    let mut lines = wheel_lines;
    if settings.invert_zoom {
        lines = -lines;
    }
    let cursor = if settings.zoom_to_cursor {
        viewport_cursor
    } else {
        None
    };

    let log_factor = lines * settings.wheel_zoom_factor.ln();
    let log_factor = match state.projection {
        ProjectionMode::Perspective => log_factor.clamp(
            (f64::from(settings.min_focal_distance) / state.focal_distance).ln() as f32,
            (f64::from(settings.max_focal_distance) / state.focal_distance).ln() as f32,
        ),
        ProjectionMode::Orthographic => log_factor.clamp(
            (1e-9 / state.ortho_height).ln() as f32,
            (1e12 / state.ortho_height).ln() as f32,
        ),
    };
    let origin = state.eye;
    let mut camera = hold::configured(state, frame, settings, origin);
    // At the focal plane a cursor anchor has the same axial depth as the
    // centre; the shared zoom therefore obeys the host's focal limits.
    let anchor = cursor
        .and_then(|point| camera.view().screen_ray(point))
        .and_then(|ray| ray.plane_hit(camera.pivot(), camera.pose().forward()))
        .unwrap_or(camera.pivot());
    camera.zoom_by(log_factor.exp(), anchor);
    hold::apply(state, frame, origin, camera);
}

/// Frame the sphere from the corner view.
pub fn fit_sphere(
    state: &mut ViewState,
    frame: &AxisFrame,
    center: Vec3,
    radius: f32,
    settings: &ViewSettings,
) {
    state.orientation = super::orientation_from_yaw_pitch(frame, CORNER_YAW_RAD, CORNER_PITCH_RAD);
    fit_sphere_as_oriented(state, frame, center, radius, settings);
}

/// [`fit_sphere`] looking the way the view already does.
pub fn fit_sphere_as_oriented(
    state: &mut ViewState,
    frame: &AxisFrame,
    center: Vec3,
    radius: f32,
    settings: &ViewSettings,
) {
    state.height_angle_rad = f64::from(settings.fov_degrees).to_radians();
    let origin = center.as_dvec3();
    let camera = hold::camera(state, frame, origin);
    let fitted = fit(
        camera,
        BoundingSphere {
            center: Vec3::ZERO,
            radius: radius.max(1.0),
        },
    );
    hold::apply(state, frame, origin, fitted);
    // Framing a large model is a view command, so wheel limits do not crop it.
    state.focal_distance = state
        .focal_distance
        .max(f64::from(settings.min_focal_distance));
    state.rederive_eye_from_focal(origin, frame);
}
