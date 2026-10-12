//! Real host pointer events, projection conventions and interruption boundaries.

use axes::AxisPreset;
use glam::{DVec3, Vec2, Vec3};
use settings::{CameraSettings, OrbitYawAxis, ProjectionMode};
use winit::event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};

use super::{CameraController, CameraPointerResult, core};

fn button(
    camera: &mut CameraController,
    settings: &CameraSettings,
    button: MouseButton,
    pressed: bool,
    hit: Option<Vec3>,
) -> CameraPointerResult {
    camera.on_viewport_pointer(
        &WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            button,
            state: if pressed {
                ElementState::Pressed
            } else {
                ElementState::Released
            },
        },
        settings,
        hit,
    )
}

fn moved(camera: &mut CameraController, settings: &CameraSettings, point: Vec2, hit: Option<Vec3>) {
    camera.set_cursor_viewport(Some(point));
    camera.on_viewport_pointer(
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(0.0, 0.0),
        },
        settings,
        hit,
    );
}

fn wheel(camera: &mut CameraController, settings: &CameraSettings, lines: f32) {
    camera.on_viewport_pointer(
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, lines),
            phase: winit::event::TouchPhase::Moved,
        },
        settings,
        None,
    );
    camera.flush_pending_wheel(settings);
}

fn start(settings: &CameraSettings, scale: f32) -> CameraController {
    let mut camera =
        CameraController::new(settings, ((800.0 * scale) as u32, (600.0 * scale) as u32));
    camera.set_pixels_per_point(scale);
    camera.set_cursor_viewport(Some(Vec2::new(300.0, 220.0) * scale));
    camera
}

#[test]
fn shared_projection_and_rays_match_vulkan_for_every_axis_preset() {
    let mut checked = 0;
    for preset in AxisPreset::ALL {
        for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
            let settings = CameraSettings {
                axis_preset: preset,
                projection,
                ..Default::default()
            };
            let camera = start(&settings, 1.0);
            let shared = core::camera(&camera.state, &camera.axes, DVec3::ZERO);
            for pointer in [
                Vec2::new(123.0, 97.0),
                Vec2::new(400.0, 300.0),
                Vec2::new(680.0, 520.0),
            ] {
                let ray = shared.view().screen_ray(pointer).unwrap();
                let point = ray.point_at(180.0);
                let (x, y) = camera.world_to_viewport(point).unwrap();
                assert!(
                    Vec2::new(x, y).distance(pointer) < 0.005,
                    "{preset:?} {projection:?}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 18);
}

#[test]
fn pointer_holds_are_partition_independent_and_freeze_the_picked_anchor() {
    let mut checked = 0;
    for preset in AxisPreset::ALL {
        for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
            for yaw in [OrbitYawAxis::WorldUp, OrbitYawAxis::CameraUp] {
                for mouse in [MouseButton::Right, MouseButton::Middle] {
                    let settings = CameraSettings {
                        axis_preset: preset,
                        projection,
                        orbit_yaw_axis: yaw,
                        orbit_pivot_pick: true,
                        ..Default::default()
                    };
                    let mut direct = start(&settings, 1.0);
                    let mut split = start(&settings, 1.0);
                    let anchor = super::zoom_cursor::intersect_focal_plane_world(
                        &direct.state,
                        &direct.axes,
                        Vec2::new(300.0, 220.0),
                    )
                    .unwrap();
                    button(&mut direct, &settings, mouse, true, Some(anchor));
                    button(&mut split, &settings, mouse, true, Some(anchor));
                    let from = Vec2::new(300.0, 220.0);
                    let delta = Vec2::new(180.0, 75.0);
                    moved(
                        &mut direct,
                        &settings,
                        from + delta,
                        Some(Vec3::splat(20.0)),
                    );
                    for step in 1..=47 {
                        moved(
                            &mut split,
                            &settings,
                            from + delta * step as f32 / 47.0,
                            Some(Vec3::splat(-20.0)),
                        );
                    }
                    assert_eq!(direct.state.eye, split.state.eye);
                    assert_eq!(direct.state.orientation, split.state.orientation);
                    if mouse == MouseButton::Middle {
                        let before = start(&settings, 1.0).world_to_viewport(anchor).unwrap();
                        let after = split.world_to_viewport(anchor).unwrap();
                        assert!(
                            Vec2::new(after.0, after.1).distance(Vec2::new(before.0, before.1))
                                < 0.005
                        );
                    }
                    button(&mut split, &settings, mouse, false, None);
                    let eye = split.state.eye;
                    moved(&mut split, &settings, from, None);
                    assert_eq!(split.state.eye, eye);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 24);
}

#[test]
fn navigation_size_and_speed_are_stable_at_ui_scales() {
    let mut checked = 0;
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        for mouse in [MouseButton::Right, MouseButton::Middle] {
            let settings = CameraSettings {
                projection,
                ..Default::default()
            };
            let mut reference = start(&settings, 1.0);
            button(&mut reference, &settings, mouse, true, None);
            moved(&mut reference, &settings, Vec2::new(410.0, 270.0), None);
            for scale in [1.0, 1.25, 2.0, 3.0] {
                let mut camera = start(&settings, scale);
                button(&mut camera, &settings, mouse, true, None);
                moved(
                    &mut camera,
                    &settings,
                    Vec2::new(410.0, 270.0) * scale,
                    None,
                );
                assert!(camera.state.eye.distance(reference.state.eye) < 0.001);
                assert!(
                    camera
                        .state
                        .orientation
                        .dot(reference.state.orientation)
                        .abs()
                        > 0.999999
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 16);
}

#[test]
fn navigation_keeps_small_model_deltas_at_large_origins() {
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        for mouse in [MouseButton::Right, MouseButton::Middle] {
            let settings = CameraSettings {
                projection,
                ..Default::default()
            };
            let mut local = start(&settings, 1.0);
            let mut remote = start(&settings, 1.0);
            let origin = DVec3::new(1e10, -1e10, 1e10);
            remote.state.eye += origin;
            button(&mut local, &settings, mouse, true, None);
            button(&mut remote, &settings, mouse, true, None);
            moved(&mut local, &settings, Vec2::new(310.0, 223.0), None);
            moved(&mut remote, &settings, Vec2::new(310.0, 223.0), None);
            assert!((remote.state.eye - origin).distance(local.state.eye) < 0.0001);
        }
    }
}

#[test]
fn world_up_orbit_stops_at_poles_without_flipping_or_accumulating_roll() {
    let settings = CameraSettings {
        orbit_sensitivity: 1.0,
        ..Default::default()
    };
    let mut checked = 0;
    for preset in AxisPreset::ALL {
        let settings = CameraSettings {
            axis_preset: preset,
            ..settings.clone()
        };
        for degrees in [
            -720.0_f32, -450.0, -360.0, -270.0, -180.0, 180.0, 270.0, 360.0, 450.0, 720.0,
        ] {
            let mut camera = start(&settings, 1.0);
            button(&mut camera, &settings, MouseButton::Middle, true, None);
            moved(
                &mut camera,
                &settings,
                Vec2::new(300.0 + degrees.to_radians() / 0.005, 1e5 * degrees.signum()),
                None,
            );
            let (forward, _) = camera.view_basis();
            assert!((forward.dot(camera.axes.vertical().vector()).abs() - 1.0).abs() < 0.00001);
            assert!(camera.state.orientation.is_normalized());
            // Returning to the press restores the exact frame even after a full turn.
            moved(&mut camera, &settings, Vec2::new(300.0, 220.0), None);
            assert_eq!(
                camera.state.orientation,
                start(&settings, 1.0).state.orientation
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 30);
}

#[test]
fn zoom_in_hold_uses_totals_and_survives_the_next_pointer_event() {
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        for mouse in [MouseButton::Right, MouseButton::Middle] {
            let settings = CameraSettings {
                projection,
                ..Default::default()
            };
            let mut first = start(&settings, 1.0);
            let mut last = start(&settings, 1.0);
            button(&mut first, &settings, mouse, true, None);
            button(&mut last, &settings, mouse, true, None);
            wheel(&mut first, &settings, 4.0);
            moved(&mut first, &settings, Vec2::new(420.0, 260.0), None);
            moved(&mut last, &settings, Vec2::new(420.0, 260.0), None);
            wheel(&mut last, &settings, 4.0);
            assert_eq!(first.state.eye, last.state.eye);
            assert_eq!(first.state.focal_distance, last.state.focal_distance);
            assert_eq!(first.state.ortho_height, last.state.ortho_height);
            let height = first.state.visible_height_at_focal_plane();
            moved(&mut first, &settings, Vec2::new(421.0, 260.0), None);
            assert_eq!(first.state.visible_height_at_focal_plane(), height);
            assert!(height < start(&settings, 1.0).state.visible_height_at_focal_plane());
        }
    }
}

#[test]
fn fit_keeps_the_whole_sphere_in_portrait_landscape_and_large_models() {
    let mut checked = 0;
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        let settings = CameraSettings {
            projection,
            ..Default::default()
        };
        for size in [(400, 1200), (1200, 400), (800, 600)] {
            for radius in [5.0, 10_000.0] {
                let mut camera = CameraController::new(&settings, size);
                camera.reset_to_fit(Vec3::new(5.0, -20.0, 12.0), radius, None, &settings);
                for index in 0..512 {
                    let y = 1.0 - 2.0 * (index as f32 + 0.5) / 512.0;
                    let r = (1.0 - y * y).sqrt();
                    let phi = index as f32 * 2.399963;
                    let point = Vec3::new(5.0, -20.0, 12.0)
                        + Vec3::new(r * phi.cos(), y, r * phi.sin()) * radius;
                    let (x, y) = camera.world_to_viewport(point).unwrap();
                    assert!(x >= 0.0 && x <= size.0 as f32 && y >= 0.0 && y <= size.1 as f32);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 6144);
}

#[test]
fn interruption_and_planar_lock_release_held_snapshots() {
    let settings = CameraSettings::default();
    for interruption in 0..4 {
        let mut camera = start(&settings, 1.0);
        button(&mut camera, &settings, MouseButton::Middle, true, None);
        moved(&mut camera, &settings, Vec2::new(380.0, 260.0), None);
        match interruption {
            0 => camera.forget_released([false; 3]),
            1 => camera.update_viewport((0, 0), (900, 700)),
            2 => camera.set_orbit_lock(true),
            _ => camera.snap_to_view(crate::orientation_cube::CameraSnapView::Front, &settings),
        }
        assert!(camera.navigation_hold.is_none());
        let eye = camera.state.eye;
        moved(&mut camera, &settings, Vec2::new(420.0, 300.0), None);
        assert_eq!(camera.state.eye, eye);
    }
    let mut camera = start(&settings, 1.0);
    camera.set_orbit_lock(true);
    let orientation = camera.state.orientation;
    button(&mut camera, &settings, MouseButton::Middle, true, None);
    moved(&mut camera, &settings, Vec2::new(450.0, 300.0), None);
    assert_eq!(camera.state.orientation, orientation);
    button(&mut camera, &settings, MouseButton::Middle, false, None);
    button(&mut camera, &settings, MouseButton::Right, true, None);
    moved(&mut camera, &settings, Vec2::new(490.0, 330.0), None);
    assert_eq!(camera.state.orientation, orientation);
    wheel(&mut camera, &settings, 2.0);
    assert_eq!(camera.state.orientation, orientation);
}

#[test]
fn roll_is_press_relative_in_both_projections_and_at_full_turn_boundaries() {
    let settings = CameraSettings::default();
    let mut checked = 0;
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        let settings = CameraSettings {
            projection,
            ..settings.clone()
        };
        for degrees in [
            -720.0_f32, -450.0, -360.0, -270.0, -180.0, 180.0, 270.0, 360.0, 450.0, 720.0,
        ] {
            for epsilon in [-0.0001_f32, 0.0, 0.0001] {
                let mut camera = start(&settings, 1.0);
                let before = camera.state.orientation;
                let forward = camera.view_basis().0;
                button(&mut camera, &settings, MouseButton::Left, true, None);
                button(&mut camera, &settings, MouseButton::Right, true, None);
                let angle = degrees.to_radians() + epsilon;
                moved(
                    &mut camera,
                    &settings,
                    Vec2::new(300.0 + angle / 0.01, 220.0),
                    None,
                );
                let expected_up = glam::Quat::from_axis_angle(forward, angle)
                    * (before * camera.axes.vertical().vector());
                assert!(camera.view_basis().1.distance(expected_up) < 0.00001);
                moved(&mut camera, &settings, Vec2::new(300.0, 220.0), None);
                assert_eq!(camera.state.orientation, before);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 60);
}

#[test]
fn pole_boundary_samples_are_continuous_in_both_directions() {
    let settings = CameraSettings {
        orbit_sensitivity: 1.0,
        ..Default::default()
    };
    let mut checked = 0;
    for preset in AxisPreset::ALL {
        let settings = CameraSettings {
            axis_preset: preset,
            ..settings.clone()
        };
        for sign in [-1.0, 1.0] {
            let mut camera = start(&settings, 1.0);
            let up = camera.axes.vertical().vector();
            let elevation = (-camera.view_basis().0).dot(up).asin();
            let boundary = (sign * std::f32::consts::FRAC_PI_2 - elevation) / 0.005;
            button(&mut camera, &settings, MouseButton::Middle, true, None);
            let at = Vec2::new(320.0, 220.0 + boundary);
            moved(&mut camera, &settings, at, None);
            let pole = camera.state.orientation;
            for offset in [-0.001, 0.001, 0.0, 0.001, -0.001] {
                moved(&mut camera, &settings, at + Vec2::Y * offset, None);
                assert!(camera.state.orientation.dot(pole).abs() > 0.999999);
                assert!((camera.view_basis().0.dot(up) + sign).abs() < 0.00001);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 30);
}

#[test]
fn saved_camera_settings_keep_explicit_free_rotation() {
    let old = serde_json::json!({ "orbit_yaw_axis": "CameraUp", "wheel_zoom_factor": 0.9, "axis_preset": "RightHandedZBackward" });
    let settings: CameraSettings = serde_json::from_value(old).unwrap();
    assert_eq!(settings.orbit_yaw_axis, OrbitYawAxis::CameraUp);
    assert_eq!(settings.axis_preset, AxisPreset::RightHandedZBackward);
    assert_eq!(
        serde_json::from_value::<CameraSettings>(serde_json::to_value(&settings).unwrap()).unwrap(),
        settings
    );
    assert_eq!(
        CameraSettings::default().orbit_yaw_axis,
        OrbitYawAxis::WorldUp
    );
}

#[test]
fn a_cube_drag_orbits_as_the_middle_button_does_and_respects_the_plane_lock() {
    let mut checked = 0;
    for preset in AxisPreset::ALL {
        for scale in [1.0, 2.0] {
            let settings = CameraSettings {
                axis_preset: preset,
                ..Default::default()
            };
            let from = Vec2::new(300.0, 220.0) * scale;
            let offset = Vec2::new(180.0, 75.0);
            let mut middle = start(&settings, scale);
            button(&mut middle, &settings, MouseButton::Middle, true, None);
            moved(&mut middle, &settings, from + offset * scale, None);

            let mut cube = start(&settings, scale);
            assert!(cube.orbit_from_cube(offset * 0.5, &settings));
            assert!(cube.orbit_from_cube(offset, &settings));
            assert!(cube.state.orientation.dot(middle.state.orientation).abs() > 0.99999);
            assert!(cube.state.eye.distance(middle.state.eye) < 1e-3);

            // Released, the next drag starts from where this one left off.
            cube.release_cube_orbit();
            let held = cube.state.orientation;
            assert!(cube.orbit_from_cube(Vec2::ZERO, &settings));
            assert!(cube.state.orientation.dot(held).abs() > 0.999999);

            cube.set_orbit_lock(true);
            let locked = cube.state.orientation;
            assert!(!cube.orbit_from_cube(offset, &settings));
            assert_eq!(cube.state.orientation, locked);
            checked += 1;
        }
    }
    assert_eq!(checked, 2 * AxisPreset::ALL.len());
}

#[test]
fn fly_to_fit_turns_to_the_view_and_frames_the_sphere() {
    use crate::orientation_cube::CameraSnapView;
    let center = Vec3::new(40.0, -12.0, 7.0);
    let radius = 25.0;
    for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
        let settings = CameraSettings {
            projection,
            ..Default::default()
        };
        let mut home = start(&settings, 1.0);
        home.orbit_from_cube(Vec2::new(90.0, -40.0), &settings);
        home.release_cube_orbit();
        home.fly_to_fit(None, center, radius, &settings);
        assert!(home.is_animating());
        home.update(60.0, &settings);
        let mut fitted = start(&settings, 1.0);
        fitted.reset_to_fit(center, radius, None, &settings);
        assert!(home.state.orientation.dot(fitted.state.orientation).abs() > 0.99999);
        assert!(home.state.eye.distance(fitted.state.eye) < 1e-3);

        for view in [
            CameraSnapView::Front,
            CameraSnapView::Top,
            CameraSnapView::RearLeft,
        ] {
            let mut camera = start(&settings, 1.0);
            camera.fly_to_fit(Some(view), center, radius, &settings);
            camera.update(60.0, &settings);
            let mut snapped = start(&settings, 1.0);
            snapped.snap_to_view(view, &settings);
            snapped.update(60.0, &settings);
            assert!(
                camera
                    .state
                    .orientation
                    .dot(snapped.state.orientation)
                    .abs()
                    > 0.99999
            );
            let focal = camera.focal_point_world();
            assert!(focal.distance(center) < 1e-3, "{focal:?}");
            for dir in [
                Vec3::X,
                Vec3::NEG_X,
                Vec3::Y,
                Vec3::NEG_Y,
                Vec3::Z,
                Vec3::NEG_Z,
            ] {
                let (x, y) = camera.world_to_viewport(center + dir * radius).unwrap();
                assert!((0.0..=800.0).contains(&x) && (0.0..=600.0).contains(&y));
            }
        }
    }
}
