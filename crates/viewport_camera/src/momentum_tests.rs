use std::time::Duration;

use glam::{Quat, Vec2, Vec3};

use super::{
    DampingRate, Friction, Momentum, MomentumLaw, MomentumSettings, PointerTrack, ScreenVelocity,
    WHEEL_DEBOUNCE,
};
use crate::camera::Projection;
use crate::controller::{CameraController, GestureKind};
use crate::navigation::{NavigationDrag, NavigationSettings, ScreenPoint};
use crate::navigation_tests::{camera, loaded_without, rejects};
use crate::smoothing::SmoothingMillis;

fn perspective() -> Projection {
    Projection::Perspective { vertical_fov: 1.0 }
}

fn law() -> MomentumLaw {
    MomentumSettings::default().pan
}

/// Elevation of the camera's back vector above world XY (Z-up), radians.
fn elevation(camera: CameraController) -> f32 {
    let back = -camera.pose().forward();
    back.z.atan2(Vec2::new(back.x, back.y).length())
}

fn drag(
    kind: GestureKind,
    keep_horizon: bool,
    orientation: Quat,
) -> (NavigationDrag, ScreenPoint, Vec3) {
    let settings = NavigationSettings {
        keep_horizon,
        ..NavigationSettings::cad()
    };
    let camera = settings.configure(camera(perspective(), orientation));
    let anchor = orientation * Vec3::new(120.0, -80.0, 0.0);
    let press = ScreenPoint(camera.view().world_to_screen(anchor).unwrap());
    (
        settings.begin_drag(camera, press, anchor, kind),
        press,
        anchor,
    )
}

/// 🎯 The closed form solves `dv/dt = −(a·v + b)`: a fine RK4 integration of
/// the equation itself (not of the formula) lands on the same path and stop.
#[test]
fn closed_form_solves_the_brake_equation() {
    for (a, b, v0) in [(6.25, 8.0, 1500.0), (2.0, 300.0, 400.0), (40.0, 0.5, 20.0)] {
        let law = MomentumLaw {
            damping: DampingRate(a),
            friction: Friction(b),
            window: SmoothingMillis(40),
        };
        let (a, b) = (f64::from(a), f64::from(b));
        let accel = |v: f64| -(a * v + b);
        let (mut t, mut v, mut s) = (0.0_f64, f64::from(v0), 0.0_f64);
        let h = 1e-5;
        let mut checked = 0;
        while v > 0.0 {
            // RK4 on (s, v); stop integrating at the zero crossing.
            let k1 = (v, accel(v));
            let k2 = (v + 0.5 * h * k1.1, accel(v + 0.5 * h * k1.1));
            let k3 = (v + 0.5 * h * k2.1, accel(v + 0.5 * h * k2.1));
            let k4 = (v + h * k3.1, accel(v + h * k3.1));
            s += h / 6.0 * (k1.0 + 2.0 * k2.0 + 2.0 * k3.0 + k4.0);
            v += h / 6.0 * (k1.1 + 2.0 * k2.1 + 2.0 * k3.1 + k4.1);
            t += h;
            if ((t / h).round() as u64).is_multiple_of(5000) && v > 0.0 {
                let closed = f64::from(law.distance(v0, Duration::from_secs_f64(t)));
                assert!(
                    (closed - s).abs() <= 1e-3 * s.max(1.0),
                    "path at {t}: {closed} vs integrated {s}"
                );
                checked += 1;
            }
        }
        let stop = law.stop_time(v0).as_secs_f64();
        assert!((stop - t).abs() <= 2.0 * h, "stop {stop} vs integrated {t}");
        let end = f64::from(law.distance(v0, Duration::from_secs(60)));
        assert!((end - s).abs() <= 1e-3 * s, "total {end} vs integrated {s}");
        assert!(checked >= 3);
    }
}

/// Upstream integrates per frame (Euler), so its path depends on the FPS. The
/// same frame histories through the closed form stay put — and the per-frame
/// mutant run through the same oracle is caught.
#[test]
fn path_is_independent_of_frame_rate_and_frame_partition_and_euler_is_caught() {
    let law = law();
    let v0 = 1500.0_f32;
    let stop = law.stop_time(v0);
    // Frame histories: three fixed rates and an irregular one, all summing past the stop.
    let histories: Vec<Vec<Duration>> = [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0]
        .into_iter()
        .map(|dt| vec![Duration::from_secs_f64(dt); (stop.as_secs_f64() / dt) as usize + 2])
        .chain(std::iter::once(
            (0..400)
                .map(|i| Duration::from_micros([3_000, 11_000, 29_000, 7_000][i % 4]))
                .collect(),
        ))
        .collect();
    let closed = |frames: &[Duration]| {
        let elapsed: Duration = frames.iter().sum();
        law.distance(v0, elapsed)
    };
    // Upstream's loop: v -= (a·v + b)·dt, s += v·dt per frame.
    let euler = |frames: &[Duration]| {
        let (a, b) = (law.damping.0, law.friction.0);
        let (mut v, mut s) = (v0, 0.0_f32);
        for dt in frames {
            let dt = dt.as_secs_f32();
            v = (v - (a * v + b) * dt).max(0.0);
            s += v * dt;
        }
        s
    };
    let judge = |path: &dyn Fn(&[Duration]) -> f32| -> Result<(), String> {
        let ends: Vec<f32> = histories.iter().map(|h| path(h)).collect();
        for end in &ends {
            if (end - ends[0]).abs() > 1e-3 {
                return Err(format!("path depends on frame rate: {ends:?}"));
            }
        }
        Ok(())
    };
    judge(&closed).unwrap();
    // Intermediate times are also partition-free: same elapsed, same point.
    let elapsed = Duration::from_millis(150);
    let one = law.distance(v0, elapsed);
    let split = law.distance(v0, Duration::from_millis(50) + Duration::from_millis(100));
    assert_eq!(one, split);
    let error = judge(&euler).unwrap_err();
    assert!(error.contains("frame rate"), "{error}");
}

/// The camera keeps moving after release, slows down, and stops at `t*`.
#[test]
fn inertia_continues_then_converges_in_finite_time() {
    for kind in [GestureKind::Pan, GestureKind::Orbit] {
        let (drag, press, _) = drag(kind, true, Quat::from_rotation_x(0.9));
        let release = ScreenPoint(press.0 + Vec2::new(60.0, -25.0));
        let released = drag.sample(release);
        let mut momentum = Momentum::release(
            drag,
            release,
            ScreenVelocity(Vec2::new(900.0, -300.0)),
            law(),
        )
        .unwrap();
        assert_eq!(
            momentum.sample().pose(),
            released.pose(),
            "first sample is the released camera"
        );
        let mut previous = momentum.pointer();
        let mut steps = Vec::new();
        while !momentum.is_done() {
            momentum.advance(Duration::from_millis(16));
            let pointer = momentum.pointer();
            steps.push(pointer.0.distance(previous.0));
            previous = pointer;
        }
        assert!(
            steps[0] > 0.0,
            "{kind:?}: inertia must continue after release"
        );
        assert!(
            steps.windows(2).all(|w| w[1] <= w[0] + 1e-3),
            "{kind:?}: inertia must slow down: {steps:?}"
        );
        let stopped = momentum.sample();
        momentum.advance(Duration::from_secs(10));
        assert_eq!(
            momentum.sample(),
            stopped,
            "{kind:?}: nothing moves after the stop"
        );
        assert!(momentum.is_done());
        assert!(stopped.pose() != released.pose());
    }
}

/// The anchor keeps its screen relation through inertia: orbit holds it in
/// place (pixel and depth), pan carries it along the virtual pointer. No
/// second camera mathematics — the relations of the hold continue.
#[test]
fn inertia_keeps_the_anchor_relation_of_the_hold() {
    for kind in [GestureKind::Pan, GestureKind::Orbit] {
        for keep_horizon in [false, true] {
            let (drag, press, anchor) = drag(kind, keep_horizon, Quat::from_rotation_x(0.9));
            let depth = drag.sample(press).pose().world_to_view(anchor).z;
            let mut momentum =
                Momentum::release(drag, press, ScreenVelocity(Vec2::new(-700.0, 400.0)), law())
                    .unwrap();
            for _ in 0..40 {
                momentum.advance(Duration::from_millis(10));
                let camera = momentum.sample();
                let pixel = camera.view().world_to_screen(anchor).unwrap();
                let expected = match kind {
                    GestureKind::Orbit => press.0,
                    GestureKind::Pan => momentum.pointer().0,
                };
                assert!(
                    pixel.distance(expected) < 0.05,
                    "{kind:?} horizon={keep_horizon}: {pixel} vs {expected}"
                );
                if kind == GestureKind::Orbit {
                    let now = camera.pose().world_to_view(anchor).z;
                    // f32 depth ~1e3: 0.01 is ~100 ULP.
                    assert!(
                        (now - depth).abs() < 0.01,
                        "{kind:?}: depth {now} vs {depth}"
                    );
                }
            }
        }
    }
}

/// World-up orbit inertia toward the pole stops tilting at the pole (the
/// ground clamp of the gesture) instead of flipping over it.
#[test]
fn world_up_orbit_inertia_respects_the_pole_clamp() {
    let (drag, press, _) = drag(GestureKind::Orbit, true, Quat::from_rotation_x(1.2));
    // Screen-down drag raises the camera (elevation grows with +y).
    let mut momentum =
        Momentum::release(drag, press, ScreenVelocity(Vec2::new(0.0, 4000.0)), law()).unwrap();
    let mut highest = elevation(momentum.sample());
    while !momentum.is_done() {
        momentum.advance(Duration::from_millis(16));
        let now = elevation(momentum.sample());
        assert!(
            now >= highest - 1e-4,
            "elevation went back: {now} < {highest}"
        );
        assert!(
            now <= std::f32::consts::FRAC_PI_2 + 1e-4,
            "crossed the pole: {now}"
        );
        highest = now;
    }
    assert!(
        (highest - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
        "reached the pole and held: {highest}"
    );
}

/// Release speed is measured over a window of explicit time: a pointer that stood
/// still for the whole window releases with zero speed and no inertia,
/// whatever the frame rate.
#[test]
fn a_pointer_still_for_the_window_releases_without_inertia() {
    let window = Duration::from_millis(40);
    for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0] {
        let dt = Duration::from_secs_f64(dt);
        let mut track = PointerTrack::new(ScreenPoint(Vec2::ZERO), window);
        // Fast move for 200 ms, 1000 px/s along x.
        let mut time = Duration::ZERO;
        while time < Duration::from_millis(200) {
            track.advance(dt);
            time += dt;
            track.input(ScreenPoint(Vec2::new(time.as_secs_f32() * 1000.0, 0.0)));
        }
        let moving = track.velocity().0;
        assert!(
            (moving.x - 1000.0).abs() < 1.0 && moving.y == 0.0,
            "dt={dt:?}: velocity of a steady move: {moving}"
        );
        let held = time.as_secs_f32() * 1000.0;
        // Still, but less than the window: some speed remains.
        track.advance(window / 2);
        track.input(ScreenPoint(Vec2::new(held, 0.0)));
        assert!(
            track.velocity().0.x > 0.0,
            "dt={dt:?}: still for half the window"
        );
        // Still for the whole window: none.
        track.advance(window / 2);
        track.input(ScreenPoint(Vec2::new(held, 0.0)));
        assert_eq!(track.velocity(), ScreenVelocity(Vec2::ZERO), "dt={dt:?}");
        let (drag, press, _) = drag(GestureKind::Pan, true, Quat::IDENTITY);
        assert!(Momentum::release(drag, press, track.velocity(), law()).is_none());
    }
}

/// A hold shorter than the window measures from the press (the pointer was
/// there before it), so a quick flick gets a finite, direction-true speed.
#[test]
fn a_hold_shorter_than_the_window_measures_from_the_press() {
    let mut track = PointerTrack::new(
        ScreenPoint(Vec2::new(10.0, 10.0)),
        Duration::from_millis(60),
    );
    track.advance(Duration::from_millis(20));
    track.input(ScreenPoint(Vec2::new(10.0, 40.0)));
    assert_eq!(track.velocity(), ScreenVelocity(Vec2::new(0.0, 500.0)));
}

#[test]
fn the_wheel_is_debounced_only_right_after_release() {
    let (drag, press, _) = drag(GestureKind::Pan, true, Quat::IDENTITY);
    let mut momentum =
        Momentum::release(drag, press, ScreenVelocity(Vec2::new(5000.0, 0.0)), law()).unwrap();
    assert!(momentum.wheel_debounced());
    momentum.advance(WHEEL_DEBOUNCE - Duration::from_millis(1));
    assert!(momentum.wheel_debounced());
    momentum.advance(Duration::from_millis(1));
    assert!(!momentum.wheel_debounced());
    assert!(!momentum.is_done(), "a fast fling outlives the debounce");
}

/// Settings without `momentum` load with inertia off; the
/// field round-trips.
#[test]
fn settings_without_momentum_load_with_inertia_off_and_round_trip() {
    let mut on = NavigationSettings::cad();
    on.momentum.enabled = true;
    on.momentum.orbit.damping = DampingRate(3.0);
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("\"damping\":3.0"), "{json}");
    let loaded = loaded_without(on, &["momentum"]);
    assert_eq!(loaded, NavigationSettings::cad());
    assert!(!loaded.momentum.enabled);
}

#[test]
fn invalid_inertia_laws_are_rejected() {
    let bad = [
        MomentumLaw {
            friction: Friction(0.0),
            ..law()
        },
        MomentumLaw {
            damping: DampingRate(f32::NAN),
            ..law()
        },
        MomentumLaw {
            window: SmoothingMillis(0),
            ..law()
        },
    ];
    rejects(&bad, |s, law| s.momentum.pan = law);
}
