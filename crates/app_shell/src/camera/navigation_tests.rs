//! Unit tests for the focal-distance camera core.

use axes::AxisSystem;
use glam::Vec2;
use settings::{CameraSettings, ProjectionMode};

use super::ops;
use super::state::CadCameraState;
use super::{auto_clip, zoom_cursor};

#[test]
fn orientation_normalizes_after_orbit_steps() {
    let settings = CameraSettings::default();
    let axes = AxisSystem::from(settings.axis_preset);
    let mut cam = CadCameraState::new(&settings, (640, 480));
    for _ in 0..50 {
        ops::orbit_pixels(&mut cam, &axes, Vec2::new(2.0, -1.5), &settings);
    }
    assert!((cam.orientation.length() - 1.0).abs() < 1e-4);
}

#[test]
fn ortho_height_matches_perspective_visible_height_at_focal_plane() {
    let settings = CameraSettings::default();
    let mut cam = CadCameraState::new(&settings, (800, 600));
    cam.focal_distance = 123.45;
    cam.height_angle_rad = (47.0_f64).to_radians();
    let h_persp = cam.visible_height_at_focal_plane();
    let h_from_formula = 2.0 * cam.focal_distance * (cam.height_angle_rad * 0.5).tan();
    assert!((h_persp - h_from_formula).abs() < 1e-6);

    cam.projection = ProjectionMode::Orthographic;
    cam.ortho_height = h_from_formula;
    assert!((cam.visible_height_at_focal_plane() - h_persp).abs() < 1e-6);
}

#[test]
fn perspective_dolly_preserves_focal_point() {
    let settings = CameraSettings::default();
    let axes = AxisSystem::from(settings.axis_preset);
    let mut cam = CadCameraState::new(&settings, (512, 512));
    let focal_before = cam.focal_point_vec3(&axes);
    zoom_cursor::apply_zoom_wheels(
        &mut cam,
        &axes,
        None,
        1.0,
        &settings,
    );
    let focal_after = cam.focal_point_vec3(&axes);
    assert!((focal_before - focal_after).length() < 1e-3);
}

#[test]
fn auto_clip_empty_scene_no_panic() {
    let settings = CameraSettings::default();
    let axes = AxisSystem::from(settings.axis_preset);
    let mut cam = CadCameraState::new(&settings, (100, 100));
    auto_clip::update_auto_clip(&mut cam, &axes, None, &settings);
}

#[test]
fn zoom_to_cursor_preserves_world_point_on_focal_plane() {
    let mut settings = CameraSettings::default();
    settings.zoom_to_cursor = true;
    settings.invert_zoom = false;
    let axes = AxisSystem::from(settings.axis_preset);
    let mut cam = CadCameraState::new(&settings, (640, 480));
    cam.focal_distance = 80.0;
    let cursor = Vec2::new(123.0, 210.0);

    let w0 = zoom_cursor::intersect_focal_plane_world(&cam, &axes, cursor).expect("hit plane");
    zoom_cursor::apply_zoom_wheels(&mut cam, &axes, Some(cursor), 2.0, &settings);
    let w1 = zoom_cursor::intersect_focal_plane_world(&cam, &axes, cursor).expect("hit after zoom");
    let delta_p = (w0 - w1).length();
    assert!(
        delta_p < 5e-2,
        "persp zoom cursor delta {delta_p} (world units)"
    );

    cam.projection = ProjectionMode::Orthographic;
    cam.ortho_height = cam.visible_height_at_focal_plane();
    let w2 = zoom_cursor::intersect_focal_plane_world(&cam, &axes, cursor).expect("ortho hit");
    zoom_cursor::apply_zoom_wheels(&mut cam, &axes, Some(cursor), -1.5, &settings);
    let w3 = zoom_cursor::intersect_focal_plane_world(&cam, &axes, cursor).expect("ortho after");
    let delta_o = (w2 - w3).length();
    assert!(delta_o < 5e-2, "ortho zoom cursor delta {delta_o}");
}
