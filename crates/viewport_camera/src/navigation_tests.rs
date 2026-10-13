use glam::{Quat, Vec2, Vec3};

use crate::camera::{CameraPose, Projection};
use crate::controller::{CameraController, GestureKind};
use crate::navigation::{NavigationSettings, ScreenPoint};

pub(crate) fn camera(projection: Projection, orientation: Quat) -> CameraController {
    CameraController::new(
        CameraPose {
            position: orientation * Vec3::Z * 1000.0,
            orientation,
        },
        projection,
        Vec2::new(800.0, 600.0),
        Vec3::ZERO,
    )
}

pub(crate) fn projections() -> [Projection; 2] {
    [
        Projection::Orthographic {
            left: -400.0,
            right: 400.0,
            bottom: -300.0,
            top: 300.0,
        },
        Projection::Perspective { vertical_fov: 1.0 },
    ]
}

/// `settings` saved and read back, checking the round trip, then read again
/// with `fields` taken out, as a file saved before they existed.
pub(crate) fn loaded_without(settings: NavigationSettings, fields: &[&str]) -> NavigationSettings {
    let mut json = serde_json::to_value(settings).unwrap();
    assert_eq!(
        serde_json::from_value::<NavigationSettings>(json.clone()).unwrap(),
        settings,
        "round trip"
    );
    for field in fields {
        assert!(
            json.as_object_mut().unwrap().remove(*field).is_some(),
            "{field}"
        );
    }
    serde_json::from_value(json).unwrap()
}

/// Each of `values`, put into the CAD settings by `set`, fails validation.
pub(crate) fn rejects<T: Copy + std::fmt::Debug>(
    values: &[T],
    set: impl Fn(&mut NavigationSettings, T),
) {
    for &value in values {
        let mut settings = NavigationSettings::cad();
        set(&mut settings, value);
        assert!(settings.validate().is_err(), "{value:?}");
    }
}

#[test]
fn off_center_orbit_anchor_stays_fixed_for_both_projections_and_orbit_policies() {
    let mut checked = 0;
    for projection in projections() {
        for orientation in [
            Quat::IDENTITY,
            Quat::from_rotation_x(1.2),
            Quat::from_rotation_z(0.7),
        ] {
            for keep_horizon in [false, true] {
                let settings = NavigationSettings {
                    keep_horizon,
                    ..NavigationSettings::cad()
                };
                let camera = settings.configure(camera(projection, orientation));
                let anchor = orientation * Vec3::new(100.0, -60.0, 0.0);
                let start = camera.view().world_to_screen(anchor).unwrap();
                let drag =
                    settings.begin_drag(camera, ScreenPoint(start), anchor, GestureKind::Orbit);
                assert_eq!(
                    drag.sample(ScreenPoint(start)).pose(),
                    camera.pose(),
                    "press must not jump"
                );
                let moved = drag.sample(ScreenPoint(start + Vec2::new(80.0, -40.0)));
                assert!(
                    moved
                        .view()
                        .world_to_screen(anchor)
                        .unwrap()
                        .distance(start)
                        < 0.002
                );
                assert_ne!(moved.pose().orientation, camera.pose().orientation);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 12);
}

#[test]
fn pan_is_pixel_exact_and_total_displacement_is_independent_of_samples() {
    for projection in projections() {
        let orientation = Quat::from_rotation_x(0.8) * Quat::from_rotation_z(0.4);
        let camera = NavigationSettings::cad().configure(camera(projection, orientation));
        let anchor = orientation * Vec3::new(60.0, 40.0, 0.0);
        let start = camera.view().world_to_screen(anchor).unwrap();
        let delta = Vec2::new(55.0, -35.0);
        let drag = NavigationSettings::cad().begin_drag(
            camera,
            ScreenPoint(start),
            anchor,
            GestureKind::Pan,
        );
        let direct = drag.sample(ScreenPoint(start + delta));
        for step in 0..12 {
            let _ = drag.sample(ScreenPoint(start + delta * step as f32 / 12.0));
        }
        assert_eq!(drag.sample(ScreenPoint(start + delta)), direct);
        assert!(
            direct
                .view()
                .world_to_screen(anchor)
                .unwrap()
                .distance(start + delta)
                < 0.002
        );
    }
}

#[test]
fn ground_orbit_has_no_accumulated_roll_and_attitude_is_independent_of_anchor() {
    assert!(NavigationSettings::default().keep_horizon);
    assert!(NavigationSettings::middle_orbit().keep_horizon);
    let settings = NavigationSettings::cad();
    let mut checked = 0;
    for projection in projections() {
        for up in [Vec3::Z, Vec3::Y] {
            let rotation = Quat::from_rotation_arc(Vec3::Z, up);
            for roll in [0.0, 0.7] {
                let orientation =
                    rotation * Quat::from_rotation_x(1.0) * Quat::from_rotation_z(roll);
                let original = camera(projection, orientation);
                let mut source = original;
                // Repeated cornered paths must not accumulate roll.
                for delta in [
                    Vec2::new(60.0, 0.0),
                    Vec2::new(0.0, -30.0),
                    Vec2::new(-60.0, 0.0),
                    Vec2::new(0.0, 30.0),
                ]
                .into_iter()
                .cycle()
                .take(80)
                {
                    let press = source;
                    let mut attitudes = Vec::new();
                    for local_anchor in [Vec3::ZERO, Vec3::new(300.0, -200.0, 400.0)] {
                        let anchor = orientation * local_anchor;
                        let start = press.view().world_to_screen(anchor).unwrap();
                        let drag = settings.begin_drag_with_up(
                            press,
                            ScreenPoint(start),
                            anchor,
                            GestureKind::Orbit,
                            up,
                        );
                        assert_eq!(drag.sample(ScreenPoint(start)).pose(), press.pose());
                        let moved = drag.sample(ScreenPoint(start + delta));
                        assert!(
                            moved
                                .view()
                                .world_to_screen(anchor)
                                .unwrap()
                                .distance(start)
                                < 0.01
                        );
                        assert!(
                            (moved.pose().world_to_view(anchor).z
                                - press.pose().world_to_view(anchor).z)
                                .abs()
                                < 0.002
                        );
                        let right = moved.pose().forward().cross(up).normalize();
                        let screen_right = moved.pose().orientation * Vec3::X;
                        assert!(
                            (right.dot(screen_right) - roll.cos()).abs() < 1e-5,
                            "roll accumulated"
                        );
                        attitudes.push(moved.pose().orientation);
                        if local_anchor == Vec3::ZERO {
                            source = moved;
                        }
                        checked += 1;
                    }
                    assert_eq!(
                        attitudes[0], attitudes[1],
                        "off-centre anchor changed attitude"
                    );
                }
                assert!(
                    source
                        .pose()
                        .orientation
                        .dot(original.pose().orientation)
                        .abs()
                        > 1.0 - 1e-5
                );
            }
        }
    }
    assert_eq!(checked, 1280);
}

#[test]
fn ground_orbit_pitch_clamps_at_poles_and_small_reverse_motion_leaves_them() {
    let settings = NavigationSettings::cad();
    let start = ScreenPoint(Vec2::ZERO);
    let mut checked = 0;
    for projection in projections() {
        for up in [Vec3::Z, Vec3::Y] {
            let basis = Quat::from_rotation_arc(Vec3::Z, up);
            for sign in [-1.0, 1.0] {
                for pole_offset in [0.0, 0.0001, 0.03, std::f32::consts::FRAC_PI_2] {
                    let tilt = if sign > 0.0 {
                        pole_offset
                    } else {
                        std::f32::consts::PI - pole_offset
                    };
                    let orientation = basis * Quat::from_rotation_x(tilt);
                    let source = camera(projection, orientation);
                    let mut attitudes = Vec::new();
                    for anchor in [Vec3::ZERO, orientation * Vec3::new(300.0, -200.0, 400.0)] {
                        let drag = settings.begin_drag_with_up(
                            source,
                            start,
                            anchor,
                            GestureKind::Orbit,
                            up,
                        );
                        let pole = drag.sample(ScreenPoint(Vec2::new(0.0, sign * 10000.0)));
                        assert!(((-pole.pose().forward()).dot(up) - sign).abs() < 1e-6);
                        let away = settings
                            .begin_drag_with_up(pole, start, anchor, GestureKind::Orbit, up)
                            .sample(ScreenPoint(Vec2::new(0.0, -sign)));
                        assert!(
                            (pole
                                .pose()
                                .orientation
                                .angle_between(away.pose().orientation)
                                - 0.005)
                                .abs()
                                < 0.0002
                        );
                        assert!((away.pose().orientation * Vec3::X).dot(up).abs() < 1e-6);
                        // Oversized input must clamp instead of rejecting the whole
                        // pitch step, and a 1 px reverse must never stick.
                        attitudes.push(pole.pose().orientation);
                        checked += 1;
                    }
                    assert_eq!(attitudes[0], attitudes[1], "off-centre anchor changed tilt");
                }
            }
        }
    }
    assert_eq!(checked, 64);
}

#[test]
fn invalid_navigation_settings_fail_at_the_file_boundary() {
    let speeds = [0.0, -1.0, 4.0, f32::NAN, f32::INFINITY];
    rejects(&speeds, |s, speed| s.orbit_speed = speed);
    rejects(&speeds, |s, speed| s.zoom_speed = speed);
}

#[test]
fn limits_and_depth_reject_invalid_and_unrepresentable_file_values() {
    use crate::length::Length;
    use crate::scale::PerPx;

    use crate::navigation::FallbackDepth;
    // Raw tuple constructors on purpose: these are the values a file can carry past
    // `PerPx::new`/`Length::new`, and `validate` is what must reject them.
    let values = [0.0, -1.0, 1e-10, 1e30, f64::NAN, f64::INFINITY];
    rejects(&values, |s, v| s.limits.min_scale = PerPx(v));
    rejects(&values, |s, v| s.limits.max_scale = PerPx(v));
    rejects(&values, |s, v| s.limits.initial_depth = Length(v));
    rejects(&values, |s, v| {
        s.limits.fallback_depth = FallbackDepth::Fixed(Length(v))
    });
    for max in [0.04, 0.05] {
        let mut settings = NavigationSettings::cad();
        settings.limits.max_scale = PerPx(max);
        assert!(settings.validate().unwrap_err().contains("less than"));
    }
}

/// 🎯 **Pan holds the point under the pointer, not the pivot's depth.**
///
/// Panning at the view-centre depth lets a nearer point slide away from the
/// pointer. `PanGrip::Pointer` keeps it under the pointer and makes it the
/// pivot; `PanGrip::PivotDepth` keeps the old motion as an explicit choice.
///
/// Mutation oracle: drop the `PanGrip::Pointer` branch in `NavigationDrag::sample`
/// — the pivot stays at `anchor + translation` and the pivot assertion fails.
#[test]
fn pan_grip_pointer_holds_the_grabbed_point_and_pivot_depth_slides_it() {
    use crate::navigation::PanGrip;
    let camera = |projection| camera(projection, Quat::IDENTITY);
    // Pivot at depth 1000; the grabbed point is off-centre at depth 400.
    let near = Vec3::new(100.0, -60.0, 600.0);
    let delta = Vec2::new(80.0, -40.0);
    let perspective = Projection::Perspective { vertical_fov: 1.0 };

    let pointer = NavigationSettings::cad();
    assert_eq!(pointer.pan_grip, PanGrip::Pointer, "CAD default");
    let held = pointer.configure(camera(perspective));
    let start = held.view().world_to_screen(near).unwrap();
    let drag = pointer.begin_drag(held, ScreenPoint(start), near, GestureKind::Pan);
    assert_eq!(
        drag.sample(ScreenPoint(start)).pose(),
        held.pose(),
        "press must not jump"
    );
    // Intermediate samples do not matter: the press snapshot owns the gesture.
    drag.sample(ScreenPoint(start + delta * 0.3));
    let moved = drag.sample(ScreenPoint(start + delta));
    let under = moved.view().world_to_screen(near).unwrap();
    assert!(
        under.distance(start + delta) < 0.01,
        "{under:?} slid from {:?}",
        start + delta
    );
    assert_eq!(moved.pivot(), near, "the grabbed point becomes the pivot");

    let pivot_depth = NavigationSettings {
        pan_grip: PanGrip::PivotDepth,
        ..NavigationSettings::cad()
    };
    let centred = pivot_depth.configure(camera(perspective));
    let drag = pivot_depth.begin_drag(
        centred,
        ScreenPoint(start),
        centred.pivot(),
        GestureKind::Pan,
    );
    let moved = drag.sample(ScreenPoint(start + delta));
    let slid = moved.view().world_to_screen(near).unwrap();
    assert!(
        slid.distance(start + delta) > 10.0,
        "a nearer point must slide at the pivot's pan speed: {slid:?}"
    );
    let pivot = moved.view().world_to_screen(moved.pivot()).unwrap();
    assert!(
        pivot.distance(Vec2::new(400.0, 300.0)) < 0.01,
        "pivot stays at view centre"
    );

    // Orthographic has no depth-dependent speed: both grips move the camera alike.
    let ortho = projections()[0];
    let poses = [(pointer, near), (pivot_depth, Vec3::ZERO)].map(|(settings, anchor)| {
        let camera = settings.configure(camera(ortho));
        let start = camera.view().world_to_screen(anchor).unwrap();
        settings
            .begin_drag(camera, ScreenPoint(start), anchor, GestureKind::Pan)
            .sample(ScreenPoint(start + delta))
            .pose()
    });
    assert!(poses[0].position.distance(poses[1].position) < 1e-3);
    assert_eq!(poses[0].orientation, poses[1].orientation);
}

/// Saved host documents carry `NavigationSettings`; files written before the
/// grip existed must load, with the default grip.
#[test]
fn pan_grip_is_saved_and_absent_field_loads_as_pointer() {
    use crate::navigation::PanGrip;
    let settings = NavigationSettings {
        pan_grip: PanGrip::PivotDepth,
        ..NavigationSettings::cad()
    };
    assert_eq!(
        serde_json::to_value(settings).unwrap()["pan_grip"],
        "PivotDepth"
    );
    let legacy = loaded_without(settings, &["pan_grip"]);
    assert_eq!(legacy.pan_grip, PanGrip::Pointer);
}

#[test]
fn fov_wheel_is_saved_and_legacy_documents_enable_it() {
    for enabled in [false, true] {
        let settings = NavigationSettings {
            fov_wheel: enabled,
            ..NavigationSettings::cad()
        };
        assert!(loaded_without(settings, &["fov_wheel"]).fov_wheel);
    }
}
