//! Unit tests for the focal-distance camera core.

use axes::AxisSystem;
use glam::Vec2;
use settings::{CameraSettings, ProjectionMode, SixDofSettings};

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

fn controller() -> (super::CameraController, CameraSettings, SixDofSettings) {
    let settings = CameraSettings::default();
    let camera = super::CameraController::new(&settings, (800, 600));
    (camera, settings, SixDofSettings::default())
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
    camera.apply_device_motion(deflect(1, device.full_scale), 0.1, &settings, &device);
    let zoomed_in = camera.state.focal_distance;
    assert!(zoomed_in < start);

    // The same deflection the other way undoes it: zoom is exponential, so
    // equal and opposite steps land back where they started.
    camera.apply_device_motion(deflect(1, -device.full_scale), 0.1, &settings, &device);
    let zoomed_out = camera.state.focal_distance;
    assert!(zoomed_out > zoomed_in);
    assert!((zoomed_out - start).abs() < start * 1e-3);
}

#[test]
fn twisting_turns_the_view_without_moving_the_focal_point() {
    let (mut camera, settings, device) = controller();
    let focal_before = camera.focal_point_world();
    let orientation_before = camera.state.orientation;
    camera.apply_device_motion(deflect(5, device.full_scale), 0.1, &settings, &device);
    assert!(camera.state.orientation.angle_between(orientation_before) > 1e-3);
    assert!((camera.focal_point_world() - focal_before).length() < 1e-2);
}

#[test]
fn rolling_keeps_the_view_pointing_where_it_was() {
    let (mut camera, settings, device) = controller();
    let axes = camera.axis_system();
    let forward_before = camera.state.forward_world(&axes).normalize();
    let up_before = camera.state.up_world(&axes).normalize();
    camera.apply_device_motion(deflect(4, device.full_scale), 0.1, &settings, &device);
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
    camera.apply_device_motion(deflect(5, device.full_scale), 0.1, &settings, &device);
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
    camera.apply_device_motion(deflect(4, device.full_scale), 0.1, &settings, &device);
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
    // A gesture that is mostly sideways, with a little forward push in it.
    let mut readings = [0.0f32; 6];
    readings[0] = SixDofSettings::default().full_scale;
    readings[2] = SixDofSettings::default().full_scale * 0.3;

    let (mut camera, settings, mut device) = controller();
    device.dominant_axis = true;
    let start = camera.position_vec();
    camera.apply_device_motion(readings, 0.1, &settings, &device);
    let with_filter = camera.position_vec() - start;

    let (mut camera, settings, device) = controller();
    let start = camera.position_vec();
    camera.apply_device_motion(readings, 0.1, &settings, &device);
    let without_filter = camera.position_vec() - start;

    let up = camera.state.up_world(&camera.axis_system()).normalize();
    assert!(with_filter.dot(up).abs() < 1e-4);
    assert!(without_filter.dot(up).abs() > 1e-3);
}

#[test]
fn a_movement_drives_whatever_it_is_assigned_to() {
    // The sideways push pans by default...
    let (mut camera, settings, device) = controller();
    let focal_before = camera.state.focal_distance;
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    assert!((camera.state.focal_distance - focal_before).abs() < 1e-6);

    // ...and zooms when that is what it is assigned to.
    let (mut camera, settings, mut device) = controller();
    device.assign[0] = settings::SixDofMotion::Zoom;
    let start = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    assert!(camera.state.focal_distance < focal_before);
    assert!((camera.position_vec() - start).length() > 1e-3);
}

#[test]
fn an_unassigned_movement_does_nothing() {
    let (mut camera, settings, mut device) = controller();
    device.assign[0] = settings::SixDofMotion::None;
    let before = camera.position_vec();
    camera.apply_device_motion(deflect(0, device.full_scale), 0.1, &settings, &device);
    assert_eq!(camera.position_vec(), before);
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

/// The viewport-local projection is the window-absolute one minus the
/// viewport origin: what the cursor and the overlay painters use.
#[test]
fn world_to_viewport_is_the_screen_position_less_the_viewport_origin() {
    use super::CameraController;
    use glam::Vec3;

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (800, 600));
    cam.update_viewport((320, 48), (800, 600));

    let p = Vec3::new(3.0, -2.0, 1.0);
    let screen = cam.world_to_screen(p).expect("visible");
    let local = cam.world_to_viewport(p).expect("visible");
    assert!((screen.0 - 320.0 - local.0).abs() < 1e-3);
    assert!((screen.1 - 48.0 - local.1).abs() < 1e-3);
}

/// A scene box far larger than the part being looked at, with the camera
/// inside it — one stray face metres off — must not push the near plane
/// past the part: geometry can sit anywhere in the box, however close.
#[test]
fn a_camera_inside_the_scene_box_keeps_the_near_plane_close() {
    use super::CameraController;
    use glam::Vec3;

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (800, 600));
    cam.update_viewport((0, 0), (800, 600));
    // Framed on a 40 mm part at the origin.
    cam.reset_to_fit(Vec3::ZERO, 20.0, None, &settings);
    let eye = Vec3::from_array(cam.position());
    let forward = (Vec3::from_array(cam.target()) - eye).normalize();
    // The part's nearest point, seen from here.
    let part_depth = (Vec3::splat(-20.0) - eye)
        .dot(forward)
        .min((Vec3::splat(20.0) - eye).dot(forward));

    // The scene box runs metres out in every direction, so the eye is
    // inside it and its nearest corner ahead is far beyond the part.
    cam.set_scene_bounds(Some((Vec3::splat(-5000.0), Vec3::splat(12000.0))));
    cam.apply_auto_clip_planes(&settings);
    let near = cam.state.near_plane as f32;
    assert!(
        near < part_depth * 0.5,
        "near plane {near} cuts into a part {part_depth} away"
    );
}

/// Framing one small part must not narrow the clip range to it: the planes
/// enclose everything the scene draws, however the camera turns after.
#[test]
fn clipping_encloses_the_whole_scene_after_framing_one_part() {
    use super::CameraController;
    use glam::Vec3;

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (800, 600));
    cam.update_viewport((0, 0), (800, 600));
    // Fit selection frames one part and passes no box of its own.
    cam.reset_to_fit(Vec3::new(5.0, 5.0, 5.0), 5.0, None, &settings);
    let scene = (
        Vec3::new(-200.0, -150.0, -80.0),
        Vec3::new(220.0, 160.0, 90.0),
    );

    for step in 0..24 {
        cam.set_scene_bounds(Some(scene));
        cam.apply_auto_clip_planes(&settings);
        let eye = Vec3::from_array(cam.position());
        let forward = (Vec3::from_array(cam.target()) - eye).normalize();
        let (near, far) = (cam.state.near_plane as f32, cam.state.far_plane as f32);
        for corner in 0..8 {
            let p = Vec3::new(
                if corner & 1 == 0 {
                    scene.0.x
                } else {
                    scene.1.x
                },
                if corner & 2 == 0 {
                    scene.0.y
                } else {
                    scene.1.y
                },
                if corner & 4 == 0 {
                    scene.0.z
                } else {
                    scene.1.z
                },
            );
            let depth = (p - eye).dot(forward);
            if depth > 0.0 {
                assert!(
                    depth >= near,
                    "step {step}: corner {p} at {depth} is before near {near}"
                );
            }
            assert!(
                depth <= far,
                "step {step}: corner {p} at {depth} is past far {far}"
            );
        }
        // Turn a little, as a drag would.
        cam.snap_to_view(
            crate::orientation_cube::CameraSnapView::FrontTopRight,
            &settings,
        );
        for _ in 0..(step % 5 + 1) {
            cam.update(0.05, &settings);
        }
    }
}

/// Orbiting about a point picked on a near surface, or zoomed right in,
/// leaves a focal distance of a few millimetres; the far plane must still
/// reach the parts across the scene, however far they sit from it.
#[test]
fn a_short_focal_distance_keeps_the_whole_scene_inside_the_far_plane() {
    use super::CameraController;
    use glam::Vec3;

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (800, 600));
    cam.update_viewport((0, 0), (800, 600));
    // Framed on a small part, then drawn in to 2 mm from a picked point.
    cam.reset_to_fit(Vec3::ZERO, 10.0, None, &settings);
    let focal = cam.state.focal_point_vec3(&cam.axes);
    cam.state.focal_distance = 2.0;
    cam.state.rederive_eye_from_focal(
        glam::DVec3::new(focal.x as f64, focal.y as f64, focal.z as f64),
        &cam.axes,
    );
    // Other imports sit up to half a metre away, around the camera.
    let scene = (Vec3::splat(-500.0), Vec3::splat(500.0));
    cam.set_scene_bounds(Some(scene));
    cam.apply_auto_clip_planes(&settings);

    let eye = Vec3::from_array(cam.position());
    let forward = (Vec3::from_array(cam.target()) - eye).normalize();
    let deepest = (0..8)
        .map(|c| {
            Vec3::new(
                if c & 1 == 0 { scene.0.x } else { scene.1.x },
                if c & 2 == 0 { scene.0.y } else { scene.1.y },
                if c & 4 == 0 { scene.0.z } else { scene.1.z },
            )
        })
        .map(|p| (p - eye).dot(forward))
        .fold(f32::MIN, f32::max);
    let far = cam.state.far_plane as f32;
    assert!(
        far >= deepest,
        "far plane {far} stops short of a part {deepest} away"
    );
    let near = cam.state.near_plane as f32;
    assert!(
        near < 1.0,
        "the near plane {near} stays clear of the picked point"
    );
}

/// Pivot picks on parts across a scene of several imports, each followed
/// by an orbit about the picked point, as a user turns a scene over: after
/// every step the far plane holds the whole scene, and the near plane is
/// never past a part the eye is not inside.
#[test]
fn picking_pivots_and_orbiting_never_clip_the_scene() {
    use super::CameraController;
    use glam::{Vec2, Vec3};

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (1300, 800));
    cam.update_viewport((0, 0), (1300, 800));
    // The box three imports made together: a pin, a 3MF shell, an assembly.
    let scene = (
        Vec3::new(-40.5, -128.9, -10.0),
        Vec3::new(154.3, 63.95, 131.9),
    );
    let (c, r) = crate::app::frame::aabb_fit_center_radius(scene.0, scene.1);
    cam.reset_to_fit(c, r, Some(scene), &settings);

    // A fixed sequence of points on the scene, spread through the box.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32
    };
    for step in 0..400 {
        cam.set_scene_bounds(Some(scene));
        if step % 3 == 0 {
            let hit = scene.0 + (scene.1 - scene.0) * Vec3::new(next(), next(), next());
            cam.on_mmb_pivot_pick(Some(hit), &settings);
        } else {
            let pivot = Vec3::from_array(cam.target());
            super::ops::orbit_pixels_around_world_anchor(
                &mut cam.state,
                &cam.axes,
                glam::DVec3::new(pivot.x as f64, pivot.y as f64, pivot.z as f64),
                Vec2::new((next() - 0.5) * 400.0, (next() - 0.5) * 300.0),
                &settings,
            );
        }
        cam.apply_auto_clip_planes(&settings);

        let eye = Vec3::from_array(cam.position());
        let forward = (Vec3::from_array(cam.target()) - eye).normalize();
        let depths: Vec<f32> = (0..8)
            .map(|k| {
                Vec3::new(
                    if k & 1 == 0 { scene.0.x } else { scene.1.x },
                    if k & 2 == 0 { scene.0.y } else { scene.1.y },
                    if k & 4 == 0 { scene.0.z } else { scene.1.z },
                )
            })
            .map(|p| (p - eye).dot(forward))
            .collect();
        let (near, far) = (cam.state.near_plane as f32, cam.state.far_plane as f32);
        let deepest = depths.iter().copied().fold(f32::MIN, f32::max);
        let nearest = depths.iter().copied().fold(f32::MAX, f32::min);
        assert!(far >= deepest, "step {step}: far {far} short of {deepest}");
        if nearest > 0.0 {
            assert!(
                near <= nearest,
                "step {step}: near {near} past the scene at {nearest}"
            );
        } else {
            // The eye is inside the scene's box: parts may be anywhere in
            // front of it, so only a hair's breadth may be cut.
            assert!(
                near < 0.5,
                "step {step}: near {near} with the eye inside the scene"
            );
        }
    }
}

/// An orthographic view shows everything in its box, what lies behind the
/// eye as much as what lies ahead: with the eye in the middle of the scene
/// (a focal distance shortened before switching to orthographic, or an
/// orbit about a picked point that swings it in), the clip planes still
/// take in every part, in front of the eye and behind it.
#[test]
fn an_orthographic_view_keeps_what_lies_behind_the_eye() {
    use super::CameraController;
    use glam::Vec3;

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (1300, 800));
    cam.update_viewport((0, 0), (1300, 800));
    let scene = (
        Vec3::new(-40.5, -128.9, -10.0),
        Vec3::new(154.3, 63.95, 131.9),
    );
    let (c, r) = crate::app::frame::aabb_fit_center_radius(scene.0, scene.1);
    cam.reset_to_fit(c, r, Some(scene), &settings);
    cam.state.projection = ProjectionMode::Orthographic;
    // The eye 20 mm off a point in the middle of the scene.
    cam.state.focal_distance = 20.0;
    cam.state.rederive_eye_from_focal(
        glam::DVec3::new(c.x as f64, c.y as f64, c.z as f64),
        &cam.axes,
    );
    cam.set_scene_bounds(Some(scene));
    cam.apply_auto_clip_planes(&settings);

    let eye = Vec3::from_array(cam.position());
    let forward = (Vec3::from_array(cam.target()) - eye).normalize();
    let depths: Vec<f32> = (0..8)
        .map(|k| {
            Vec3::new(
                if k & 1 == 0 { scene.0.x } else { scene.1.x },
                if k & 2 == 0 { scene.0.y } else { scene.1.y },
                if k & 4 == 0 { scene.0.z } else { scene.1.z },
            )
        })
        .map(|p| (p - eye).dot(forward))
        .collect();
    let nearest = depths.iter().copied().fold(f32::MAX, f32::min);
    let deepest = depths.iter().copied().fold(f32::MIN, f32::max);
    let (near, far) = (cam.state.near_plane as f32, cam.state.far_plane as f32);
    assert!(
        nearest < 0.0,
        "the pick puts the eye inside the scene: {nearest}"
    );
    assert!(near <= nearest, "near {near} cuts parts from {nearest} on");
    assert!(far >= deepest, "far {far} short of {deepest}");
}

/// In an orthographic view a part behind the eye is on screen, so a pivot
/// picked on it is taken, and the eye comes round in front of it.
#[test]
fn an_orthographic_pivot_pick_behind_the_eye_is_taken() {
    use super::CameraController;
    use glam::Vec3;

    let settings = CameraSettings::default();
    let mut cam = CameraController::new(&settings, (800, 600));
    cam.update_viewport((0, 0), (800, 600));
    cam.reset_to_fit(Vec3::ZERO, 50.0, None, &settings);
    cam.state.projection = ProjectionMode::Orthographic;
    let eye = Vec3::from_array(cam.position());
    let forward = (Vec3::from_array(cam.target()) - eye).normalize();
    let behind = eye - forward * 30.0;

    cam.on_mmb_pivot_pick(Some(behind), &settings);
    assert!(
        (Vec3::from_array(cam.target()) - behind).length() < 1e-3,
        "the picked point is the pivot"
    );
    let eye = Vec3::from_array(cam.position());
    assert!(
        (behind - eye).dot(forward) > 0.0,
        "and it lies ahead of the eye"
    );
}
