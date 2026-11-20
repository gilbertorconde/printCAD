use glam::{DVec3, Vec2, Vec3, Vec4};

use super::*;

fn settings(projection: ProjectionMode) -> ViewSettings {
    ViewSettings {
        min_focal_distance: 1.0,
        max_focal_distance: 1e6,
        projection,
        fov_degrees: 45.0,
        ortho_height: 200.0,
        keep_horizon: true,
        orbit_sensitivity: 1.0,
        pan_sensitivity: 1.0,
        wheel_zoom_factor: 0.9,
        invert_zoom: false,
        zoom_to_cursor: true,
        auto_near_far: true,
        near_far_near_ratio: 0.01,
        near_far_depth_ratio_cap: 1e5,
        near_far_margin: 1.0,
        view_transition_ms: 0.0,
    }
}

/// Y-up, Z-up and a left-handed frame.
fn frames() -> [AxisFrame; 3] {
    [
        AxisFrame {
            horizontal: Vec3::X,
            vertical: Vec3::Y,
            depth: Vec3::Z,
        },
        AxisFrame {
            horizontal: Vec3::X,
            vertical: Vec3::Z,
            depth: Vec3::NEG_Y,
        },
        AxisFrame {
            horizontal: Vec3::X,
            vertical: Vec3::Z,
            depth: Vec3::Y,
        },
    ]
}

fn to_pixels(view_proj: glam::Mat4, size: (u32, u32), world: Vec3) -> (Vec2, f32) {
    let clip = view_proj * Vec4::new(world.x, world.y, world.z, 1.0);
    let ndc = clip.truncate() / clip.w;
    (
        Vec2::new(
            (ndc.x * 0.5 + 0.5) * size.0 as f32,
            (ndc.y * 0.5 + 0.5) * size.1 as f32,
        ),
        ndc.z,
    )
}

/// The matrix draws every point where the shared camera, with its top-left
/// screen origin, says it lies: the clip space is Y-down in every frame and
/// both projections.
#[test]
fn the_view_projection_puts_points_where_the_shared_camera_sees_them() {
    let mut checked = 0;
    for frame in frames() {
        for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
            for size in [(800, 600), (300, 900)] {
                let settings = settings(projection);
                let mut state = ViewState::new(&settings, &frame, size);
                ops::orbit_pixels(&mut state, &frame, Vec2::new(37.0, -12.0), &settings);
                state.eye += DVec3::new(15.0, -4.0, 9.0);
                let view_proj = state.view_projection(&frame);
                let camera = hold::camera(&state, &frame, DVec3::ZERO);
                let focal = state.focal_point_vec3(&frame);
                let up = state.up_world(&frame);
                let right = state.right_world(&frame);
                for world in [focal, focal + up * 20.0, focal + right * 30.0 - up * 5.0] {
                    let (pixels, _) = to_pixels(view_proj, size, world);
                    let expected = camera.view().world_to_screen(world).unwrap();
                    assert!(
                        pixels.distance(expected) < 0.01,
                        "{frame:?} {projection:?} {size:?}: {pixels} vs {expected}"
                    );
                    checked += 1;
                }
                let (above, _) = to_pixels(view_proj, size, focal + up * 20.0);
                assert!(above.y < size.1 as f32 * 0.5, "up is towards the top");
            }
        }
    }
    assert_eq!(checked, 36);
}

/// Depth runs from 0 at the near plane to 1 at the far one.
#[test]
fn the_clip_planes_bound_depth_from_zero_to_one() {
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        let frame = frames()[1];
        let mut state = ViewState::new(&settings(projection), &frame, (640, 480));
        state.near_plane = 2.0;
        state.far_plane = 900.0;
        let eye = state.eye_vec3();
        let forward = state.forward_world(&frame);
        let view_proj = state.view_projection(&frame);
        let (_, near) = to_pixels(view_proj, (640, 480), eye + forward * 2.0);
        let (_, far) = to_pixels(view_proj, (640, 480), eye + forward * 900.0);
        assert!(near.abs() < 1e-4, "{projection:?} near {near}");
        assert!((far - 1.0).abs() < 1e-4, "{projection:?} far {far}");
    }
}
