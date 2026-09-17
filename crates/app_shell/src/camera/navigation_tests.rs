//! Unit tests for the focal-distance camera core.

use axes::AxisSystem;
use glam::Vec2;
use settings::{CameraSettings, ProjectionMode, SpaceNavSettings};

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
    zoom_cursor::apply_zoom_wheels(&mut cam, &axes, None, 1.0, &settings);
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
    let settings = CameraSettings {
        zoom_to_cursor: true,
        invert_zoom: false,
        ..CameraSettings::default()
    };
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

#[test]
fn orient_to_plane_puts_sketch_axes_screen_aligned() {
    use super::CameraController;
    use glam::Vec3;

    // Default settings use the Z-up axis preset — the case where an
    // orientation built against a hardcoded XYZ camera basis, instead of
    // the preset's, rolls the view on sketch entry.
    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (800, 600));
    cam.update_viewport((0, 0), (800, 600));

    // Default sketch plane: XY at origin, +Z normal, +Y plane-up.
    cam.orient_to_plane(Vec3::ZERO, Vec3::Z, Vec3::Y, &settings);
    // Drive the transition tween to completion.
    for _ in 0..600 {
        cam.update(0.016, &settings);
    }

    let origin = cam.world_to_screen(Vec3::ZERO).expect("origin visible");
    let plus_x = cam
        .world_to_screen(Vec3::new(10.0, 0.0, 0.0))
        .expect("+X visible");
    let plus_y = cam
        .world_to_screen(Vec3::new(0.0, 10.0, 0.0))
        .expect("+Y visible");

    // Screen space is Y-down: sketch +X must appear to the right of the
    // origin and sketch +Y above it (smaller screen y). The old code showed
    // the sketch mirrored/rolled.
    assert!(
        plus_x.0 > origin.0 + 1.0,
        "sketch +X should be screen-right: origin {origin:?}, +X {plus_x:?}"
    );
    assert!(
        (plus_x.1 - origin.1).abs() < 1.0,
        "sketch +X should stay level: origin {origin:?}, +X {plus_x:?}"
    );
    assert!(
        plus_y.1 < origin.1 - 1.0,
        "sketch +Y should be screen-up: origin {origin:?}, +Y {plus_y:?}"
    );
}

/// Full deflection on one axis of a device at rest everywhere else.
fn deflect(axis: usize, amount: f32) -> [f32; 6] {
    let mut readings = [0.0f32; 6];
    readings[axis] = amount;
    readings
}

fn controller() -> (super::CameraController, CameraSettings, SpaceNavSettings) {
    let settings = CameraSettings::default();
    let camera = super::CameraController::new(&settings, (800, 600));
    (camera, settings, SpaceNavSettings::default())
}

#[test]
fn a_resting_device_leaves_the_view_alone() {
    let (mut camera, settings, device) = controller();
    let before = camera.position();
    assert!(!camera.apply_device_motion([0.0; 6], 0.016, &settings, &device));
    assert_eq!(camera.position(), before);
}

#[test]
fn a_deflection_inside_the_dead_zone_is_rest() {
    let (mut camera, settings, device) = controller();
    let barely = device.full_scale * device.dead_zone * 0.5;
    assert!(!camera.apply_device_motion(deflect(0, barely), 0.016, &settings, &device));
}

#[test]
fn a_disabled_device_never_moves_the_view() {
    let (mut camera, settings, mut device) = controller();
    device.enabled = false;
    assert!(!camera.apply_device_motion(deflect(0, 350.0), 0.016, &settings, &device));
}

#[test]
fn pushing_sideways_pans_across_the_view() {
    let (mut camera, settings, device) = controller();
    let before = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    let moved = camera.position_vec() - before;

    // The eye slides along the screen's horizontal, not along its depth.
    let right = camera.state.right_world(&camera.axis_system()).normalize();
    let forward = camera
        .state
        .forward_world(&camera.axis_system())
        .normalize();
    assert!(moved.length() > 1e-3);
    assert!(moved.normalize().dot(right).abs() > 0.99);
    assert!(moved.normalize().dot(forward).abs() < 0.01);
}

#[test]
fn an_inverted_axis_pans_the_other_way() {
    let (mut camera, settings, device) = controller();
    let start = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    let forward_move = camera.position_vec() - start;

    let (mut camera, settings, mut device) = controller();
    device.invert[0] = true;
    let start = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    let inverted_move = camera.position_vec() - start;

    assert!((forward_move + inverted_move).length() < 1e-3);
}

#[test]
fn push_and_pull_zooms_both_ways() {
    let (mut camera, settings, device) = controller();
    let start = camera.state.focal_distance;
    camera.apply_device_motion(deflect(2, device.full_scale), 0.1, &settings, &device);
    let zoomed_in = camera.state.focal_distance;
    assert!(zoomed_in < start);

    // The same deflection the other way undoes it: zoom is exponential, so
    // equal and opposite steps land back where they started.
    camera.apply_device_motion(deflect(2, -device.full_scale), 0.1, &settings, &device);
    let zoomed_out = camera.state.focal_distance;
    assert!(zoomed_out > zoomed_in);
    assert!((zoomed_out - start).abs() < start * 1e-3);
}

#[test]
fn twisting_turns_the_view_without_moving_the_focal_point() {
    let (mut camera, settings, device) = controller();
    let focal_before = camera.focal_point_world();
    let orientation_before = camera.state.orientation;
    camera.apply_device_motion(deflect(4, device.full_scale), 0.1, &settings, &device);
    assert!(camera.state.orientation.angle_between(orientation_before) > 1e-3);
    assert!((camera.focal_point_world() - focal_before).length() < 1e-2);
}

#[test]
fn rolling_keeps_the_view_pointing_where_it_was() {
    let (mut camera, settings, device) = controller();
    let axes = camera.axis_system();
    let forward_before = camera.state.forward_world(&axes).normalize();
    let up_before = camera.state.up_world(&axes).normalize();
    camera.apply_device_motion(deflect(5, device.full_scale), 0.1, &settings, &device);
    let forward_after = camera.state.forward_world(&axes).normalize();
    let up_after = camera.state.up_world(&axes).normalize();

    assert!(forward_after.dot(forward_before) > 0.999);
    assert!(up_after.dot(up_before) < 0.999);
}

#[test]
fn a_sketch_keeps_its_plane_square_to_the_view() {
    let (mut camera, settings, device) = controller();
    camera.set_orbit_lock(true);
    let axes = camera.axis_system();
    let forward_before = camera.state.forward_world(&axes).normalize();

    // Tilt and turn are dropped under the lock...
    camera.apply_device_motion(deflect(3, device.full_scale), 0.1, &settings, &device);
    camera.apply_device_motion(deflect(4, device.full_scale), 0.1, &settings, &device);
    assert!(
        camera
            .state
            .forward_world(&axes)
            .normalize()
            .dot(forward_before)
            > 0.9999
    );

    // ...while roll, which turns about the plane's own normal, still works.
    let up_before = camera.state.up_world(&axes).normalize();
    camera.apply_device_motion(deflect(5, device.full_scale), 0.1, &settings, &device);
    assert!(camera.state.up_world(&axes).normalize().dot(up_before) < 0.999);
    assert!(
        camera
            .state
            .forward_world(&axes)
            .normalize()
            .dot(forward_before)
            > 0.9999
    );
}

#[test]
fn one_axis_at_a_time_drops_the_rest() {
    let (mut camera, settings, mut device) = controller();
    device.dominant_axis = true;
    device.rotation = false;
    device.zoom = false;

    // A gesture that is mostly sideways, with a little lift in it.
    let start = camera.position_vec();
    let mut readings = [0.0f32; 6];
    readings[0] = device.full_scale;
    readings[1] = device.full_scale * 0.3;
    camera.apply_device_motion(readings, 0.1, &settings, &device);
    let with_filter = camera.position_vec() - start;

    let (mut camera, settings, mut device) = controller();
    device.rotation = false;
    device.zoom = false;
    let start = camera.position_vec();
    camera.apply_device_motion(readings, 0.1, &settings, &device);
    let without_filter = camera.position_vec() - start;

    let up = camera.state.up_world(&camera.axis_system()).normalize();
    assert!(with_filter.dot(up).abs() < 1e-4);
    assert!(without_filter.dot(up).abs() > 1e-3);
}

#[test]
fn a_long_frame_does_not_throw_the_view_across_the_scene() {
    let (mut camera, settings, device) = controller();
    let start = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 5.0, &settings, &device);
    let long_frame = (camera.position_vec() - start).length();

    let (mut camera, settings, device) = controller();
    let start = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    let capped = (camera.position_vec() - start).length();

    assert!((long_frame - capped).abs() < 1e-3);
}
