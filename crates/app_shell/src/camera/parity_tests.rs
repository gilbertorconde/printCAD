//! The app's camera state and the shared crate's focal view agree to the
//! bit: matrices, gestures, zoom, fits, clip planes and transitions.

use axes::{AxisPreset, AxisSystem};
use glam::{DVec3, Vec2, Vec3};
use settings::{CameraSettings, OrbitYawAxis, ProjectionMode};
use viewport_camera::focal_view::{self, AxisFrame, ViewState};

use super::state::CadCameraState;

fn frame(axes: &AxisSystem) -> AxisFrame {
    AxisFrame {
        horizontal: axes.horizontal().vector(),
        vertical: axes.vertical().vector(),
        depth: axes.depth().vector(),
    }
}

fn same(old: &CadCameraState, new: &ViewState, axes: &AxisSystem, frame: &AxisFrame, step: &str) {
    assert_eq!(old.eye, new.eye, "{step}: eye");
    assert_eq!(old.orientation, new.orientation, "{step}: orientation");
    assert_eq!(old.focal_distance, new.focal_distance, "{step}: focal");
    assert_eq!(old.projection, new.projection, "{step}: projection");
    assert_eq!(old.height_angle_rad, new.height_angle_rad, "{step}: fov");
    assert_eq!(old.ortho_height, new.ortho_height, "{step}: ortho");
    assert_eq!(old.near_plane, new.near_plane, "{step}: near");
    assert_eq!(old.far_plane, new.far_plane, "{step}: far");
    assert_eq!(
        old.view_projection(axes).to_cols_array(),
        new.view_projection(frame).to_cols_array(),
        "{step}: view_projection"
    );
}

#[test]
fn the_shared_focal_view_matches_the_app_camera_bit_for_bit() {
    let mut checked = 0;
    for preset in AxisPreset::ALL {
        for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
            for yaw_axis in [OrbitYawAxis::WorldUp, OrbitYawAxis::CameraUp] {
                for size in [(800, 600), (333, 1024), (0, 0)] {
                    let settings = CameraSettings {
                        axis_preset: preset,
                        projection,
                        orbit_yaw_axis: yaw_axis,
                        ..Default::default()
                    };
                    let view = settings.view_settings();
                    let axes = AxisSystem::from(preset);
                    let frame = frame(&axes);
                    let mut old = CadCameraState::new(&settings, size);
                    let mut new = ViewState::new(&view, &frame, size);
                    same(&old, &new, &axes, &frame, "new");

                    super::ops::orbit_pixels(&mut old, &axes, Vec2::new(41.0, -17.0), &settings);
                    focal_view::ops::orbit_pixels(&mut new, &frame, Vec2::new(41.0, -17.0), &view);
                    same(&old, &new, &axes, &frame, "orbit");

                    let pivot = DVec3::new(12.0, -30.0, 7.5);
                    super::ops::orbit_pixels_around_world_anchor(
                        &mut old,
                        &axes,
                        pivot,
                        Vec2::new(-9.0, 23.0),
                        &settings,
                    );
                    focal_view::ops::orbit_pixels_around_world_anchor(
                        &mut new,
                        &frame,
                        pivot,
                        Vec2::new(-9.0, 23.0),
                        &view,
                    );
                    same(&old, &new, &axes, &frame, "orbit about");

                    super::ops::pan_pixels(&mut old, &axes, Vec2::new(5.0, 61.0), &settings);
                    focal_view::ops::pan_pixels(&mut new, &frame, Vec2::new(5.0, 61.0), &view);
                    same(&old, &new, &axes, &frame, "pan");

                    super::ops::roll_pixels(&mut old, &axes, 13.0);
                    focal_view::ops::roll_pixels(&mut new, &frame, 13.0);
                    same(&old, &new, &axes, &frame, "roll");

                    let cursor = Some(Vec2::new(150.0, 90.0));
                    super::zoom_cursor::apply_zoom_wheels(&mut old, &axes, cursor, 2.5, &settings);
                    focal_view::ops::apply_zoom_wheels(&mut new, &frame, cursor, 2.5, &view);
                    same(&old, &new, &axes, &frame, "zoom");

                    let hit = Vec3::new(3.0, 4.0, -2.0);
                    assert_eq!(
                        super::ops::set_pivot_world_hit(&mut old, &axes, hit, &settings),
                        focal_view::ops::set_pivot_world_hit(&mut new, &frame, hit, &view)
                    );
                    same(&old, &new, &axes, &frame, "pivot");

                    let scene = Some((Vec3::new(-40.0, -10.0, 0.0), Vec3::new(60.0, 25.0, 80.0)));
                    super::auto_clip::update_auto_clip(&mut old, &axes, scene, &settings);
                    focal_view::clip::update_auto_clip(&mut new, &frame, scene, &view);
                    same(&old, &new, &axes, &frame, "clip");

                    let other = match projection {
                        ProjectionMode::Perspective => ProjectionMode::Orthographic,
                        ProjectionMode::Orthographic => ProjectionMode::Perspective,
                    };
                    old.set_projection_preserve_framing(other, &axes);
                    new.set_projection_preserve_framing(other, &frame);
                    same(&old, &new, &axes, &frame, "projection");

                    let center = Vec3::new(100.0, -50.0, 20.0);
                    super::ops::fit_sphere(&mut old, &axes, center, 75.0, None, &settings);
                    focal_view::ops::fit_sphere(&mut new, &frame, center, 75.0, &view);
                    same(&old, &new, &axes, &frame, "fit");

                    let mut target_old = old.clone();
                    let mut target_new = new.clone();
                    let turn = Vec2::new(200.0, 40.0);
                    super::ops::orbit_pixels(&mut target_old, &axes, turn, &settings);
                    focal_view::ops::orbit_pixels(&mut target_new, &frame, turn, &view);
                    let mut tween_old =
                        super::animate::CameraTween::toward(&old, &axes, &target_old, &settings);
                    let mut tween_new =
                        focal_view::tween::ViewTween::toward(&new, &frame, &target_new, &view);
                    for dt in [0.016, 0.05, 0.2, 1.0] {
                        assert_eq!(
                            tween_old.tick(dt, &mut old, &axes),
                            tween_new.tick(dt, &mut new, &frame)
                        );
                        same(&old, &new, &axes, &frame, "tween");
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, AxisPreset::ALL.len() * 12);
}
