//! Zoom‑to‑cursor: keep the world point under the cursor stable after zoom (`docs/CAMERA.md`, Navigation).

use glam::{Vec2, Vec3};
use settings::{CameraSettings, ProjectionMode};

use crate::camera::state::CadCameraState;

/// Where the cursor's ray, by the shared helpers the benches use, meets the
/// plane through `plane_origin` square to `plane_normal`.
pub fn viewport_to_plane(
    state: &CadCameraState,
    axes: &axes::AxisSystem,
    viewport_xy: Vec2,
    plane_origin: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> {
    let (w, h) = state.viewport_size;
    core_document::runtime::viewport_to_plane(
        state.view_projection(axes).to_cols_array_2d(),
        (0, 0, w, h),
        (viewport_xy.x, viewport_xy.y),
        plane_origin.to_array(),
        plane_normal.to_array(),
    )
    .map(Vec3::from_array)
}

#[cfg(test)]
pub fn intersect_focal_plane_world(
    state: &CadCameraState,
    axes: &axes::AxisSystem,
    viewport_xy: Vec2,
) -> Option<Vec3> {
    viewport_to_plane(
        state,
        axes,
        viewport_xy,
        state.focal_point_vec3(axes),
        state.forward_world(axes),
    )
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
    let mut camera = super::core::configured(state, axes, settings, origin);
    // At the focal plane a cursor anchor has the same axial depth as the
    // centre; the shared zoom therefore obeys the host's focal limits.
    let anchor = cursor
        .and_then(|point| camera.view().screen_ray(point))
        .and_then(|ray| {
            core_document::runtime::ray_plane(
                ray.origin.to_array(),
                ray.direction.to_array(),
                camera.pivot().to_array(),
                camera.pose().forward().to_array(),
            )
        })
        .map(Vec3::from_array)
        .unwrap_or(camera.pivot());
    camera.zoom_by(log_factor.exp(), anchor);
    super::core::apply(state, axes, origin, camera);
}
