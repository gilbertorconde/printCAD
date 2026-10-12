//! Host commands and device-rate steps through the shared camera.

use glam::{DVec3, Vec2, Vec3};
use settings::{CameraSettings, ProjectionMode};
use viewport_camera::controller::GestureKind;
use viewport_camera::fit::{BoundingSphere, fit};

use super::core;
use super::state::CadCameraState;
use super::zoom_cursor::{intersect_ray_plane, viewport_ray};

pub fn pan_pixels(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    delta_px: Vec2,
    settings: &CameraSettings,
) {
    if delta_px == Vec2::ZERO {
        return;
    }
    let hold = core::Hold::begin(
        state,
        axes,
        settings,
        Vec2::ZERO,
        state.focal_point_dvec(axes),
        GestureKind::Pan,
        1.0,
    );
    hold.sample(state, axes, delta_px);
}

pub fn orbit_pixels(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    delta_px: Vec2,
    settings: &CameraSettings,
) {
    orbit_pixels_around_world_anchor(
        state,
        axes,
        state.focal_point_dvec(axes),
        delta_px,
        settings,
    );
}

pub fn orbit_pixels_around_world_anchor(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    pivot_world: DVec3,
    delta_px: Vec2,
    settings: &CameraSettings,
) {
    if delta_px == Vec2::ZERO {
        return;
    }
    let hold = core::Hold::begin(
        state,
        axes,
        settings,
        Vec2::ZERO,
        pivot_world,
        GestureKind::Orbit,
        1.0,
    );
    hold.sample(state, axes, delta_px);
}

pub fn roll_pixels(state: &mut CadCameraState, axes: &axes::AxisSystem, delta_px_x: f32) {
    if delta_px_x == 0.0 {
        return;
    }
    let forward = state.forward_world(axes);
    state.orientation =
        (glam::Quat::from_axis_angle(forward, delta_px_x * 0.01) * state.orientation).normalize();
    state.clip_dirty = true;
}

pub fn set_pivot_world_hit(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    hit: Vec3,
    settings: &CameraSettings,
) -> bool {
    let depth = (hit - state.eye_vec3()).dot(state.forward_world(axes));
    // Orthographic clipping includes geometry behind the eye. Recentring a
    // picked point there keeps the eye in front without changing the picture's scale.
    if depth > 1e-4 {
        state.focal_distance = f64::from(depth);
    } else if state.projection != ProjectionMode::Orthographic {
        return false;
    }
    state.clamp_focal_distance(settings);
    state.rederive_eye_from_focal(hit.as_dvec3(), axes);
    state.clip_dirty = true;
    true
}

pub fn set_pivot_focal_plane_cursor(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    viewport_xy: Vec2,
    settings: &CameraSettings,
) -> bool {
    let Some((origin, dir)) = viewport_ray(state, axes, viewport_xy) else {
        return false;
    };
    intersect_ray_plane(
        origin,
        dir,
        state.focal_point_vec3(axes),
        state.forward_world(axes),
    )
    .is_some_and(|point| set_pivot_world_hit(state, axes, point, settings))
}

pub fn fit_sphere(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    center: Vec3,
    radius: f32,
    _scene_aabb: Option<(Vec3, Vec3)>,
    settings: &CameraSettings,
) {
    state.orientation = super::state::orientation_from_yaw_pitch(
        axes,
        super::state::CORNER_YAW_RAD,
        super::state::CORNER_PITCH_RAD,
    );
    fit_sphere_as_oriented(state, axes, center, radius, settings);
}

/// [`fit_sphere`] looking the way the view already does.
pub fn fit_sphere_as_oriented(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    center: Vec3,
    radius: f32,
    settings: &CameraSettings,
) {
    state.height_angle_rad = f64::from(settings.fov_degrees).to_radians();
    let origin = center.as_dvec3();
    let camera = core::camera(state, axes, origin);
    let fitted = fit(
        camera,
        BoundingSphere {
            center: Vec3::ZERO,
            radius: radius.max(1.0),
        },
    );
    core::apply(state, axes, origin, fitted);
    // Framing a large model is a view command, so wheel limits do not crop it.
    state.focal_distance = state
        .focal_distance
        .max(f64::from(settings.min_focal_distance));
    state.rederive_eye_from_focal(origin, axes);
}
