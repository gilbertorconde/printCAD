//! Zoom‑to‑cursor: keep the world point under the cursor stable after zoom (`camera_system.md` §3).

use glam::{DVec3, Vec2, Vec3, Vec4};
use settings::{CameraSettings, ProjectionMode};

use crate::camera::state::CadCameraState;

pub fn viewport_ray(
    state: &CadCameraState,
    axes: &axes::AxisSystem,
    viewport_xy: Vec2,
) -> Option<(Vec3, Vec3)> {
    let (w, h) = state.viewport_size;
    if w == 0 || h == 0 {
        return None;
    }
    let vw = w as f32;
    let vh = h as f32;

    let ndc_x = (viewport_xy.x / vw) * 2.0 - 1.0;
    let ndc_y = (viewport_xy.y / vh) * 2.0 - 1.0;

    let vp = state.view_projection(axes);
    let inv = vp.inverse();

    let near_clip = inv * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    let far_clip = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
    if near_clip.w.abs() < 1e-20 || far_clip.w.abs() < 1e-20 {
        return None;
    }
    let near_w = near_clip.truncate() / near_clip.w;
    let far_w = far_clip.truncate() / far_clip.w;
    let dir = (far_w - near_w).normalize();
    let origin = state.eye_vec3();
    Some((origin, dir))
}

pub fn intersect_focal_plane_world(
    state: &CadCameraState,
    axes: &axes::AxisSystem,
    viewport_xy: Vec2,
) -> Option<Vec3> {
    let focal = state.focal_point_vec3(axes);
    let plane_n = state.forward_world(axes).normalize();
    let (origin, dir) = viewport_ray(state, axes, viewport_xy)?;
    intersect_ray_plane(origin, dir, focal, plane_n)
}

pub fn intersect_ray_plane(
    origin: Vec3,
    dir: Vec3,
    plane_origin: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> {
    let n = plane_normal.normalize();
    let denom = dir.dot(n);
    if denom.abs() < 1e-8 {
        return None;
    }
    let t = (plane_origin - origin).dot(n) / denom;
    if t < 0.0 {
        return None;
    }
    Some(origin + dir * t)
}

/// Apply exponential zoom (`factor^wheel_lines`).
pub fn apply_zoom_wheels(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    viewport_cursor: Option<Vec2>,
    wheel_lines: f32,
    settings: &CameraSettings,
) {
    if wheel_lines == 0.0 {
        return;
    }
    let mut lines = wheel_lines;
    if settings.invert_zoom {
        lines = -lines;
    }
    let factor = settings.wheel_zoom_factor.powf(lines);
    if !factor.is_finite() || factor <= 0.0 {
        return;
    }

    let cursor = if settings.zoom_to_cursor {
        viewport_cursor
    } else {
        None
    };

    let w0 = cursor.and_then(|p| intersect_focal_plane_world(state, axes, p));

    match state.projection {
        ProjectionMode::Perspective => {
            let forward = state.forward_world(axes);
            if forward.length_squared() < 1e-12 {
                return;
            }
            let forward = forward.normalize();
            let fd0 = state.focal_distance;
            let step = fd0 * (1.0 - factor as f64);

            let lo = settings.min_focal_distance as f64;
            let hi = settings.max_focal_distance as f64;
            let fd_clamped = (fd0 - step).clamp(lo, hi);
            let step_actual = fd0 - fd_clamped;

            state.eye += DVec3::new(
                forward.x as f64 * step_actual,
                forward.y as f64 * step_actual,
                forward.z as f64 * step_actual,
            );
            state.focal_distance = fd_clamped;
        }
        ProjectionMode::Orthographic => {
            state.ortho_height = (state.ortho_height * factor as f64).max(1e-9);
        }
    }

    if let (Some(p), Some(w_before)) = (cursor, w0) {
        let w_after = intersect_focal_plane_world(state, axes, p);
        if let Some(wa) = w_after {
            let corr = w_before - wa;
            state.eye += DVec3::new(corr.x as f64, corr.y as f64, corr.z as f64);
        }
    }

    state.clip_dirty = true;
}
