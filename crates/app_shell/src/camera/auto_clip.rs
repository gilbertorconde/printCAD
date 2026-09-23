//! Near/far adjustment from scene AABB (depth along view forward).
//!
//! See `camera_system.md` §5 — handles empty/degenerate bounds and caps `far/near` ratio.

use glam::Vec3;
use settings::{CameraSettings, ProjectionMode};

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

    let mut min_depth = f32::INFINITY;
    let mut max_depth = f32::NEG_INFINITY;
    for p in corners {
        let d = (p - eye).dot(forward);
        max_depth = max_depth.max(d);
        min_depth = min_depth.min(d);
    }

    let diag = (mx - mn).length();
    let min_range = (diag * 1e-6).max(1e-3);

    // An orthographic view shows its whole box, what lies behind the eye
    // as much as what lies ahead, and its depth is linear: the planes take
    // in the scene from end to end, the near one behind the eye where the
    // scene reaches back there.
    if state.projection == ProjectionMode::Orthographic {
        let margin = settings.near_far_margin;
        let near = min_depth - margin;
        let far = (max_depth + margin).max(near + min_range);
        state.near_plane = near as f64;
        state.far_plane = far as f64;
        return;
    }

    let fd = state.focal_distance as f32;
    let near_from_focal = (fd * settings.near_far_near_ratio).max(1e-4);

    // With the whole box ahead, nothing is nearer than its nearest corner and
    // the near plane may move up towards it. A box that reaches the eye's
    // own plane holds geometry at any depth down to zero — the eye is inside
    // it or beside it — so only the focal floor is safe; the nearest corner
    // ahead can be metres beyond what is in front of the camera.
    let near = if min_depth > 0.0 {
        near_from_focal
            .max(min_depth * 0.02)
            .min((min_depth * 0.9).max(near_from_focal))
    } else {
        near_from_focal
    };

    let mut far = max_depth + settings.near_far_margin;
    if !far.is_finite() || far <= near {
        far = near + min_range;
    }
    far = far.max(near + min_range);

    // Depth precision allows only so wide a far/near ratio. The far plane
    // encloses the whole scene, always; past the cap, the near plane moves
    // out instead. Pulling the far plane in would cut away parts across
    // the scene whenever the focal distance is short (zoomed right in, or
    // orbiting a point picked on a near surface), where moving the near
    // plane out costs only what lies within a hair of the eye.
    let ratio_cap = settings.near_far_depth_ratio_cap;
    let mut near = near;
    if far / near > ratio_cap {
        near = far / ratio_cap;
    }

    state.near_plane = near as f64;
    state.far_plane = far as f64;
}
