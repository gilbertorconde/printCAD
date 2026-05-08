//! Near/far adjustment from scene AABB (depth along view forward).
//!
//! See `camera_system.md` §5 — handles empty/degenerate bounds and caps `far/near` ratio.

use glam::Vec3;
use settings::CameraSettings;

use crate::camera::state::CadCameraState;

pub fn update_auto_clip(
    state: &mut CadCameraState,
    axes: &axes::AxisSystem,
    scene_aabb: Option<(Vec3, Vec3)>,
    settings: &CameraSettings,
) {
    if !settings.auto_near_far {
        return;
    }
    let Some((mn, mx)) = scene_aabb else {
        return;
    };
    if mn.x > mx.x || mn.y > mx.y || mn.z > mx.z {
        return;
    }

    let eye = state.eye_vec3();
    let forward = state.forward_world(axes);
    if forward.length_squared() < 1e-12 {
        return;
    }
    let forward = forward.normalize();

    let corners = [
        Vec3::new(mn.x, mn.y, mn.z),
        Vec3::new(mx.x, mn.y, mn.z),
        Vec3::new(mn.x, mx.y, mn.z),
        Vec3::new(mx.x, mx.y, mn.z),
        Vec3::new(mn.x, mn.y, mx.z),
        Vec3::new(mx.x, mn.y, mx.z),
        Vec3::new(mn.x, mx.y, mx.z),
        Vec3::new(mx.x, mx.y, mx.z),
    ];

    let mut min_positive = f32::INFINITY;
    let mut max_depth = f32::NEG_INFINITY;
    for p in corners {
        let d = (p - eye).dot(forward);
        max_depth = max_depth.max(d);
        if d > 0.0 {
            min_positive = min_positive.min(d);
        }
    }

    let diag = (mx - mn).length();
    let min_range = (diag * 1e-6).max(1e-3);

    let fd = state.focal_distance as f32;
    let near_from_focal = (fd * settings.near_far_near_ratio).max(1e-4);

    let near = if min_positive.is_finite() {
        // Stay in front of the closest visible point; still respect focal-based floor.
        near_from_focal
            .max(min_positive * 0.02)
            .min((min_positive * 0.9).max(near_from_focal))
    } else {
        near_from_focal
    };

    let mut far = max_depth + settings.near_far_margin;
    if !far.is_finite() || far <= near {
        far = near + min_range;
    }
    far = far.max(near + min_range);

    let ratio_cap = settings.near_far_depth_ratio_cap;
    if far / near > ratio_cap {
        far = near * ratio_cap;
    }

    state.near_plane = near as f64;
    state.far_plane = far as f64;
}
