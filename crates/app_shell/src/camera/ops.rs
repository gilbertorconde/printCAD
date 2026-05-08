//! Pan / orbit / roll / pivot / fit operations on [`CadCameraState`].

use glam::{DVec3, Vec2, Vec3};
use settings::{CameraSettings, OrbitYawAxis, ProjectionMode};

use crate::camera::math::control_horizontal_vec;
use crate::camera::state::CadCameraState;
use crate::camera::zoom_cursor::{intersect_ray_plane, viewport_ray};

pub fn pan_pixels(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    delta_px: Vec2,
    settings: &CameraSettings,
) {
    if delta_px.length_squared() < 1e-12 {
        return;
    }
    let wpp = state.world_per_pixel() * settings.pan_sensitivity as f64;
    let right = (state.orientation * control_horizontal_vec(axes)).normalize_or_zero();
    let up = (state.orientation * axes.vertical().vector()).normalize_or_zero();
    if right.length_squared() < 1e-12 || up.length_squared() < 1e-12 {
        return;
    }
    // Negate viewport X so drags behave like grabbing the scene (matches vertical feel).
    let world_delta = (-delta_px.x as f64 * wpp) * DVec3::new(right.x as f64, right.y as f64, right.z as f64)
        + (delta_px.y as f64 * wpp) * DVec3::new(up.x as f64, up.y as f64, up.z as f64);

    state.eye += world_delta;
    state.clip_dirty = true;
}

pub fn orbit_pixels(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    delta_px: Vec2,
    settings: &CameraSettings,
) {
    if delta_px.length_squared() < 1e-12 {
        return;
    }
    let sens = settings.orbit_sensitivity * 0.005;
    // Screen-space Δ with negation so orbit matches CAD/turntable expect: drag follows scene motion.
    let dx = -delta_px.x * sens;
    let dy = -delta_px.y * sens;

    let focal = state.focal_point_dvec(axes);

    let up_axis = match settings.orbit_yaw_axis {
        OrbitYawAxis::WorldUp => axes.vertical().vector().normalize(),
        OrbitYawAxis::CameraUp => state.up_world(axes).normalize_or_zero(),
    };
    let right = state.right_world(axes);
    if up_axis.length_squared() < 1e-12 || right.length_squared() < 1e-12 {
        return;
    }
    let up_axis = up_axis.normalize();
    let right = right.normalize();

    let yaw_q = glam::Quat::from_axis_angle(up_axis, dx);
    // Pitch magnitude clamped loosely to reduce pole flipping (CAD-style).
    let pitch_q = glam::Quat::from_axis_angle(right, dy.clamp(-1.45, 1.45));
    let delta_q = (yaw_q * pitch_q).normalize();

    state.orientation = (delta_q * state.orientation).normalize();
    state.rederive_eye_from_focal(focal, axes);
    state.clip_dirty = true;
}

pub fn roll_pixels(state: &mut CadCameraState, axes: &axes::AxisSystem, delta_px_x: f32) {
    if delta_px_x.abs() < 1e-6 {
        return;
    }
    let fwd = state.forward_world(axes).normalize_or_zero();
    if fwd.length_squared() < 1e-12 {
        return;
    }
    let angle = delta_px_x * 0.01;
    let roll_q = glam::Quat::from_axis_angle(fwd, angle);
    state.orientation = (roll_q * state.orientation).normalize();
    state.clip_dirty = true;
}

pub fn set_pivot_world_hit(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    hit: Vec3,
    settings: &CameraSettings,
) -> bool {
    let eye = state.eye_vec3();
    let forward = state.forward_world(axes);
    if forward.length_squared() < 1e-12 {
        return false;
    }
    let forward = forward.normalize();
    let to_hit = hit - eye;
    if to_hit.dot(forward) <= 1e-4 {
        return false;
    }
    let focal = hit;
    state.focal_distance = to_hit.dot(forward) as f64;
    state.clamp_focal_distance(settings);
    state.rederive_eye_from_focal(
        DVec3::new(focal.x as f64, focal.y as f64, focal.z as f64),
        axes,
    );
    state.clip_dirty = true;
    true
}

pub fn set_pivot_focal_plane_cursor(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    viewport_xy: Vec2,
    settings: &CameraSettings,
) -> bool {
    let focal_now = state.focal_point_vec3(axes);
    let n = state.forward_world(axes).normalize();
    let Some((origin, dir)) = viewport_ray(state, axes, viewport_xy) else {
        return false;
    };
    match intersect_ray_plane(origin, dir, focal_now, n) {
        Some(p) => set_pivot_world_hit(state, axes, p, settings),
        None => false,
    }
}

pub fn fit_sphere(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    center: Vec3,
    radius: f32,
    scene_aabb: Option<(Vec3, Vec3)>,
    settings: &CameraSettings,
) {
    let margin = 1.08_f32;
    let r_ext = radius.max(1.0) * margin;
    let focal = center;
    state.height_angle_rad = (settings.fov_degrees as f64).to_radians();

    state.focal_distance = (r_ext as f64
        / ((state.height_angle_rad * 0.5).tan()).max(1e-9))
    .max(settings.min_focal_distance as f64);

    if let Some((a, b)) = scene_aabb {
        let diag = (b - a).length().max(r_ext);
        let fd_min = (diag as f64 * 1e-4).max(settings.min_focal_distance as f64);
        state.focal_distance = state.focal_distance.max(fd_min);
    }
    state.clamp_focal_distance(settings);

    state.orientation =
        crate::camera::state::orientation_from_yaw_pitch(axes, 45.0_f32.to_radians(), 35.0_f32.to_radians());
    state.rederive_eye_from_focal(
        DVec3::new(focal.x as f64, focal.y as f64, focal.z as f64),
        axes,
    );

    if state.projection == ProjectionMode::Orthographic {
        state.ortho_height =
            (2.0 * state.focal_distance * (state.height_angle_rad * 0.5).tan()).max(1e-3);
    }

    state.clip_dirty = true;
}
