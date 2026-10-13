//! Orbit and gesture options, with serialization and default coverage.

#[test]
fn host_zoom_range_clamps_totals_without_changing_the_press() {
    for projection in projections() {
        let source = camera(projection, Quat::IDENTITY);
        let start = ScreenPoint(Vec2::new(400.0, 300.0));
        let drag = NavigationSettings::cad()
            .begin_drag(source, start, Vec3::ZERO, GestureKind::Orbit)
            .with_zoom_range([-2.0, 1.0]);
        assert_eq!(drag.sample(start).pose(), source.pose());
        let scale = source.scale_at(Vec3::ZERO);
        for (step, expected) in [(-10.0, -2.0_f32), (0.0, 0.0), (10.0, 1.0)] {
            let moved = drag.zoomed(step).sample(start);
            assert!((moved.scale_at(Vec3::ZERO) / scale - expected.exp()).abs() < 1e-5);
        }
        let bounded = drag.zoomed(-10.0).with_zoom_range([-0.5, 0.5]);
        assert_eq!(bounded.zoom(), -0.5);
    }
}

#[test]
fn host_zoom_range_rejects_nonfinite_or_nonzero_press_ranges() {
    let source = camera(projections()[0], Quat::IDENTITY);
    let drag = NavigationSettings::cad().begin_drag(
        source,
        ScreenPoint(Vec2::ZERO),
        Vec3::ZERO,
        GestureKind::Pan,
    );
    for range in [
        [f32::NAN, 1.0],
        [-1.0, f32::INFINITY],
        [0.1, 1.0],
        [-2.0, -1.0],
    ] {
        assert!(std::panic::catch_unwind(|| drag.with_zoom_range(range)).is_err());
    }
}

use glam::{Quat, Vec2, Vec3};

use crate::camera::{CameraPose, Projection};
use crate::controller::{CameraController, GestureKind};
use crate::navigation::{NavigationSettings, ScreenPoint, ViewCubeSize};
use crate::navigation_tests::{camera, loaded_without, projections, rejects};

fn close(a: CameraPose, b: CameraPose, what: &str) {
    // `angle_between` is acos of a dot near 1: one f32 ulp reads as ~7e-4 rad.
    // Compare the frame's axes instead.
    let angle = [Vec3::X, Vec3::Y, Vec3::Z]
        .map(|axis| (a.orientation * axis).distance(b.orientation * axis))
        .into_iter()
        .fold(0.0, f32::max);
    let distance = a.position.distance(b.position);
    assert!(
        angle < 1e-4 && distance < 1e-2,
        "{what}: {angle} rad, {distance} apart"
    );
}

/// Vertical orbit speed is its own setting. The ratio scales the
/// tilt only: a vertical drag at ratio 2 equals twice that drag at ratio 1,
/// and a horizontal drag is bit-identical. Both orbit policies.
///
/// Mutation oracle: build `orbit_sensitivity` without the ratio — the first
/// assertion fails.
#[test]
fn vertical_orbit_ratio_scales_the_tilt_and_leaves_the_turn() {
    let mut checked = 0;
    for projection in projections() {
        for keep_horizon in [false, true] {
            let orientation = Quat::from_rotation_x(1.0) * Quat::from_rotation_z(0.4);
            let base = NavigationSettings {
                keep_horizon,
                ..NavigationSettings::cad()
            };
            let steep = NavigationSettings {
                orbit_vertical_ratio: 2.0,
                ..base
            };
            let source = camera(projection, orientation);
            let anchor = orientation * Vec3::new(100.0, -60.0, 0.0);
            let start = source.view().world_to_screen(anchor).unwrap();
            let sample = |settings: NavigationSettings, delta: Vec2| {
                settings
                    .begin_drag(source, ScreenPoint(start), anchor, GestureKind::Orbit)
                    .sample(ScreenPoint(start + delta))
                    .pose()
            };
            close(
                sample(steep, Vec2::new(0.0, 30.0)),
                sample(base, Vec2::new(0.0, 60.0)),
                "ratio 2 tilts twice as far",
            );
            assert_eq!(
                sample(steep, Vec2::new(40.0, 0.0)),
                sample(base, Vec2::new(40.0, 0.0)),
                "the turn does not depend on the ratio"
            );
            assert!(
                sample(steep, Vec2::new(40.0, 30.0))
                    .orientation
                    .angle_between(sample(base, Vec2::new(40.0, 30.0)).orientation)
                    > 0.1
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 4);
}

/// World-up orbit passes over the pole. Without the option the
/// elevation stops at the pole; with it the same drag carries the camera
/// 0.3 rad over the top: upside down, with a level horizon (no roll), the
/// anchor's pixel and depth kept, no jump at the pole. A new gesture from
/// there continues the tilt the same way on screen: the second press
/// sampled 10 px further equals the first gesture sampled 170 px.
///
/// Mutation oracles: ignore `pass_poles` in `sample` — the pole clamp fails
/// the «over the pole» assertion; drop the side-axis sign flip in
/// `begin_drag_with_up` — the next gesture tilts back and the continuation
/// fails.
#[test]
fn pass_poles_tilts_over_the_pole_and_the_next_gesture_continues() {
    let clamped = NavigationSettings::cad();
    let passing = NavigationSettings {
        pass_poles: true,
        ..NavigationSettings::cad()
    };
    let mut checked = 0;
    for projection in projections() {
        for up in [Vec3::Z, Vec3::Y] {
            let basis = Quat::from_rotation_arc(Vec3::Z, up);
            // 0.5 rad below the top pole; a screen-down drag raises the camera
            // by 0.005 rad/px, so 160 px end 0.3 rad past the pole.
            let orientation = basis * Quat::from_rotation_x(0.5);
            let source = camera(projection, orientation);
            for local_anchor in [Vec3::ZERO, Vec3::new(300.0, -200.0, 400.0)] {
                let anchor = orientation * local_anchor;
                let start = source.view().world_to_screen(anchor).unwrap();
                let press = |settings: NavigationSettings, camera: CameraController| {
                    settings.begin_drag_with_up(
                        camera,
                        ScreenPoint(start),
                        anchor,
                        GestureKind::Orbit,
                        up,
                    )
                };
                let at = |dy: f32| ScreenPoint(start + Vec2::new(0.0, dy));
                let pole = press(clamped, source).sample(at(160.0));
                assert!(
                    ((-pole.pose().forward()).dot(up) - 1.0).abs() < 1e-6,
                    "stops at the pole"
                );

                let drag = press(passing, source);
                let over = drag.sample(at(160.0));
                let back = -over.pose().forward();
                assert!((back.dot(up) - 0.3f32.cos()).abs() < 1e-4, "over the pole");
                // Screen up of a camera at elevation e is cos e above the
                // horizon; upside down at 90° − 0.3 rad it is −sin 0.3.
                assert!(
                    ((over.pose().orientation * Vec3::Y).dot(up) + 0.3f32.sin()).abs() < 1e-4,
                    "upside down"
                );
                assert!(
                    (over.pose().orientation * Vec3::X).dot(up).abs() < 1e-5,
                    "no roll"
                );
                assert!(over.view().world_to_screen(anchor).unwrap().distance(start) < 0.01);
                let depth = |camera: CameraController| camera.pose().world_to_view(anchor).z;
                assert!((depth(over) - depth(source)).abs() < 0.002);
                let mut previous = source.pose().orientation;
                for dy in 1..=160 {
                    let now = drag.sample(at(dy as f32)).pose().orientation;
                    assert!(now.angle_between(previous) < 0.0051, "jump at {dy} px");
                    previous = now;
                }
                // The second press starts upside down.
                let next = press(passing, over).sample(at(10.0));
                close(next.pose(), drag.sample(at(170.0)).pose(), "continuation");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 8);
}

/// Zoom in a hold is a separate total. Steps in any order with the
/// same sum give the same camera; the zoomed gesture keeps its anchor
/// relation (orbit: the pixel; pan: the pixel moved by the pointer), the
/// anchor's scale is the press scale × e^zoom, and the total stops at the
/// limits. Zero zoom is the plain gesture, bit for bit.
///
/// Mutation oracle: skip the zoom in `sample` — the scale assertion fails.
#[test]
fn hold_zoom_is_a_separate_total_clamped_to_the_limits() {
    let mut checked = 0;
    for projection in projections() {
        for kind in [GestureKind::Pan, GestureKind::Orbit] {
            for keep_horizon in [false, true] {
                let settings = NavigationSettings {
                    keep_horizon,
                    ..NavigationSettings::cad()
                };
                assert!(
                    settings.zoom_in_hold,
                    "wheel remains available while holding orbit"
                );
                let orientation = Quat::from_rotation_x(0.8) * Quat::from_rotation_z(0.4);
                let camera = settings.configure(camera(projection, orientation));
                let anchor = orientation * Vec3::new(60.0, 40.0, 200.0);
                let start = camera.view().world_to_screen(anchor).unwrap();
                let drag = settings.begin_drag(camera, ScreenPoint(start), anchor, kind);
                assert_eq!(drag.zoom(), 0.0);
                let delta = Vec2::new(55.0, -35.0);
                let point = ScreenPoint(start + delta);
                assert_eq!(drag.with_zoom(0.0).sample(point), drag.sample(point));

                let once = drag.zoomed(-0.6);
                for steps in [[-0.3, 0.1, -0.4], [-0.4, -0.3, 0.1]] {
                    let split = steps.into_iter().fold(drag, |d, s| d.zoomed(s));
                    assert!((split.zoom() - once.zoom()).abs() < 1e-6);
                    close(
                        split.sample(point).pose(),
                        once.sample(point).pose(),
                        "order",
                    );
                }
                let zoomed = once.sample(point);
                let expected = if kind == GestureKind::Pan {
                    start + delta
                } else {
                    start
                };
                assert!(
                    zoomed
                        .view()
                        .world_to_screen(anchor)
                        .unwrap()
                        .distance(expected)
                        < 0.01
                );
                let ratio = zoomed.scale_at(anchor) / camera.scale_at(anchor);
                assert!(
                    (ratio / (-0.6f32).exp() - 1.0).abs() < 1e-4,
                    "ratio {ratio}"
                );
                for (step, limit) in [(-100.0, 0.05), (100.0, 50.0)] {
                    let scale = drag.zoomed(step).sample(point).scale_at(anchor);
                    assert!(
                        (scale / limit - 1.0).abs() < 1e-4,
                        "{scale} at limit {limit}"
                    );
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 8);
}

/// Perspective zoom passes through the anchor. Wheel notches at a
/// surface point: without the option the camera stops at `min_scale` in
/// front of it; with it the camera travels on, the pivot staying under the
/// pointer at the limit's scale, and passes the surface. Past the limit each
/// notch travels the same `0.5 · reach` (linear, additive). Orthographic is
/// unaffected.
///
/// Mutation oracle: ignore `zoom_through` in `zoom_to` — the surface is
/// never passed.
#[test]
fn zoom_through_passes_the_anchor_in_perspective_only() {
    let perspective = Projection::Perspective { vertical_fov: 1.0 };
    let min: f32 = 0.05;
    for zoom_through in [false, true] {
        let settings = NavigationSettings {
            zoom_through,
            ..NavigationSettings::cad()
        };
        let mut camera = settings.configure(camera(perspective, Quat::IDENTITY));
        // Off-centre, 100 units in front: scale ≈ 0.18 > min.
        let surface = Vec3::new(20.0, -10.0, 900.0);
        let pixel = camera.view().world_to_screen(surface).unwrap();
        let mut anchor = surface;
        let mut passed = false;
        let mut linear = 0;
        for notch in 0..40 {
            let before = camera.pose().position;
            let at_limit = passed && camera.scale_at(anchor) <= min * (1.0 + 1e-4);
            let target = camera.scale_at(anchor).ln() - 0.5;
            settings.zoom_to(&mut camera, anchor, target);
            let pivot = camera.pivot();
            assert!(
                camera
                    .view()
                    .world_to_screen(pivot)
                    .unwrap()
                    .distance(pixel)
                    < 0.01,
                "notch {notch}: the pivot left the pointer"
            );
            assert!(camera.scale_at(pivot) >= min * (1.0 - 1e-4));
            if at_limit {
                let reach = (pivot - camera.pose().position).length();
                let travel = camera.pose().position.distance(before);
                assert!((travel / (0.5 * reach) - 1.0).abs() < 1e-3, "notch {notch}");
                linear += 1;
            }
            // The host's policy: the surface while it is in front, else
            // the pivot (the empty-space fallback at the last depth).
            passed |= camera.pose().world_to_view(surface).z >= 0.0;
            anchor = if passed { camera.pivot() } else { surface };
        }
        assert_eq!(passed, zoom_through, "zoom_through={zoom_through}");
        assert_eq!(linear > 0, zoom_through);
    }

    // Additive past the limit, and one step travels at most ln(max/min) reaches.
    let settings = NavigationSettings {
        zoom_through: true,
        ..NavigationSettings::cad()
    };
    let mut at_limit = settings.configure(camera(perspective, Quat::IDENTITY));
    let surface = Vec3::new(20.0, -10.0, 900.0);
    settings.zoom_to(&mut at_limit, surface, min.ln());
    let reach = (at_limit.pivot() - at_limit.pose().position).length();
    let pivot = at_limit.pivot();
    let mut once = at_limit;
    settings.zoom_to(&mut once, pivot, min.ln() - 1.0);
    let mut twice = at_limit;
    settings.zoom_to(&mut twice, pivot, min.ln() - 0.5);
    let pivot = twice.pivot();
    settings.zoom_to(&mut twice, pivot, min.ln() - 0.5);
    close(once.pose(), twice.pose(), "additive");
    let mut far = at_limit;
    settings.zoom_to(&mut far, at_limit.pivot(), -1000.0);
    let travel = far.pose().position.distance(at_limit.pose().position);
    assert!(
        (travel / (reach * 1000f32.ln()) - 1.0).abs() < 1e-3,
        "{travel} vs {reach}"
    );

    // Orthographic has no near limit to pass: the option changes nothing.
    let ortho = projections()[0];
    let views = [false, true].map(|zoom_through| {
        let settings = NavigationSettings {
            zoom_through,
            ..NavigationSettings::cad()
        };
        let mut camera = settings.configure(camera(ortho, Quat::IDENTITY));
        settings.zoom_to(&mut camera, Vec3::new(20.0, -10.0, 0.0), -100.0);
        camera
    });
    assert_eq!(views[0], views[1]);
}

/// Host documents carry `NavigationSettings`:
/// the options round-trip, files saved before them load with all four off
/// (ratio 1), and a malformed ratio is rejected at the file boundary.
#[test]
fn gesture_options_are_saved_and_old_files_load_without_them() {
    let settings = NavigationSettings {
        orbit_vertical_ratio: 1.7,
        pass_poles: true,
        zoom_in_hold: true,
        zoom_through: true,
        ..NavigationSettings::cad()
    };
    let legacy = loaded_without(
        settings,
        &[
            "orbit_vertical_ratio",
            "pass_poles",
            "zoom_in_hold",
            "zoom_through",
        ],
    );
    assert_eq!(legacy, NavigationSettings::cad());
    assert_eq!(legacy.orbit_vertical_ratio, 1.0);
    rejects(
        &[0.0, -1.0, 0.2, 5.0, f32::NAN, f32::INFINITY],
        |s, ratio| s.orbit_vertical_ratio = ratio,
    );
}

/// The view cube size is saved with the other overlay setting; files from
/// before it load with Medium.
#[test]
fn view_cube_size_is_saved_and_old_files_load_medium() {
    for size in ViewCubeSize::ALL {
        let settings = NavigationSettings {
            view_cube_size: size,
            ..NavigationSettings::cad()
        };
        let legacy = loaded_without(settings, &["view_cube_size"]);
        assert_eq!(legacy.view_cube_size, ViewCubeSize::Medium);
        assert_eq!(legacy, NavigationSettings::cad());
    }
}

/// Pointer grab is saved; files from before it load with the grab on at
/// speed 1. The speed is validated like the other speeds.
#[test]
fn pointer_lock_is_saved_and_old_files_load_it_on() {
    let settings = NavigationSettings {
        pointer_lock: false,
        pointer_lock_speed: 2.0,
        pointer_lock_pan_speed: 3.0,
        pointer_lock_orbit_speed: 0.75,
        ..NavigationSettings::cad()
    };
    let legacy = loaded_without(
        settings,
        &[
            "pointer_lock",
            "pointer_lock_speed",
            "pointer_lock_pan_speed",
            "pointer_lock_orbit_speed",
        ],
    );
    assert!(legacy.pointer_lock);
    assert_eq!(legacy.pointer_lock_speed, 1.0);
    assert_eq!(legacy.pointer_lock_pan_speed, 1.0);
    assert_eq!(legacy.pointer_lock_orbit_speed, 0.25);
    assert_eq!(legacy, NavigationSettings::cad());
    let speeds = [0.0, -1.0, 0.2, 5.0, f32::NAN, f32::INFINITY];
    rejects(&speeds, |s, speed| s.pointer_lock_pan_speed = speed);
    rejects(&speeds, |s, speed| s.pointer_lock_orbit_speed = speed);
    rejects(&speeds, |s, speed| s.pointer_lock_speed = speed);
}
