use std::time::Duration;

use glam::{Quat, Vec2, Vec3};

use super::*;
use crate::fit::{BoundingSphere, fit};
use crate::view::view_orientation;

fn camera(projection: Projection) -> CameraController {
    let orientation = Quat::from_rotation_z(0.4) * Quat::from_rotation_x(0.9);
    let mut camera = CameraController::new(
        CameraPose {
            position: orientation * Vec3::Z * 40.0,
            orientation,
        },
        projection,
        Vec2::new(900.0, 500.0),
        Vec3::ZERO,
    );
    // An off-centre pivot, as a pointer-grip pan or an orbit leaves it.
    camera.set_pose(camera.pose(), orientation * Vec3::new(6.0, -2.0, -3.0));
    camera
}

fn projections() -> [Projection; 2] {
    [
        Projection::Perspective { vertical_fov: 0.8 },
        orthographic(Vec2::new(900.0, 500.0), 0.05),
    ]
}

/// The bounds the fit transitions frame.
const SPHERE: BoundingSphere = BoundingSphere {
    center: Vec3::new(-20.0, 5.0, 3.0),
    radius: 4.0,
};

fn transitions(source: CameraController) -> Vec<CameraTransition> {
    let duration = Duration::from_millis(400);
    let top = view_orientation(Vec3::Z, Vec3::Z, true, source.pose().orientation);
    vec![
        CameraTransition::view(source, top, duration),
        CameraTransition::frame(source, fit(source, SPHERE), duration),
        CameraTransition::projection(
            source,
            ProjectionKind::Perspective { vertical_fov: 1.6 },
            duration,
        ),
        CameraTransition::projection(source, ProjectionKind::Orthographic, duration),
    ]
}

/// Transitions sample the source pose using elapsed time.
///
/// Whatever way the frames split the time (30/60/144 FPS or irregular), the
/// sample at the same elapsed time is the same camera, and the end is the
/// target exactly. Mutation oracle: accumulate per-frame (sample from the
/// previous sample) — the splits disagree.
#[test]
fn samples_depend_only_on_elapsed_time() {
    let mut cases = 0;
    for projection in projections() {
        let source = camera(projection);
        for transition in transitions(source) {
            let mut reference = transition;
            reference.advance(Duration::from_millis(250));
            let reference = reference.sample();
            for frame_ms in [33.333_f64, 16.667, 6.944] {
                let mut split = transition;
                let mut elapsed = 0.0;
                while elapsed + frame_ms < 250.0 {
                    split.advance(Duration::from_secs_f64(frame_ms / 1000.0));
                    elapsed += frame_ms;
                }
                split.advance(Duration::from_secs_f64((250.0 - elapsed) / 1000.0));
                let sample = split.sample();
                assert!(sample.pose().position.distance(reference.pose().position) < 1e-3);
                assert!(
                    sample
                        .pose()
                        .orientation
                        .dot(reference.pose().orientation)
                        .abs()
                        > 1.0 - 1e-6
                );
            }
            let mut done = transition;
            done.advance(Duration::from_secs(5));
            assert!(done.is_done());
            let mut instant = transition;
            instant.advance(Duration::from_millis(400));
            assert_eq!(done.sample(), instant.sample(), "the end is the target");
            cases += 1;
        }
    }
    assert_eq!(cases, 8);
}

/// A view command turns around the pivot: at every sample the pivot keeps
/// its pixel, depth and scale.
#[test]
fn a_view_transition_keeps_the_pivot_pixel_and_scale() {
    for projection in projections() {
        let source = camera(projection);
        let pixel = source.view().world_to_screen(source.pivot()).unwrap();
        let scale = source.scale_at(source.pivot());
        let target = view_orientation(
            Vec3::new(-1.0, 0.4, 0.2),
            Vec3::Z,
            true,
            source.pose().orientation,
        );
        let mut transition = CameraTransition::view(source, target, Duration::from_millis(400));
        for _ in 0..30 {
            transition.advance(Duration::from_millis(16));
            let sample = transition.sample();
            assert_eq!(sample.pivot(), source.pivot());
            assert!(
                sample
                    .view()
                    .world_to_screen(source.pivot())
                    .unwrap()
                    .distance(pixel)
                    < 1e-2
            );
            assert!((sample.scale_at(source.pivot()) / scale - 1.0).abs() < 1e-5);
        }
        assert!(transition.is_done());
        assert_eq!(transition.sample().pose().orientation, target);
    }
}

/// Duration zero is the instant command: the very first sample is the end.
#[test]
fn zero_duration_is_the_instant_command() {
    for projection in projections() {
        let source = camera(projection);
        let top = view_orientation(Vec3::Z, Vec3::Z, true, source.pose().orientation);
        let mut turned = source;
        turned.orient_about_pivot(top);
        assert_eq!(
            CameraTransition::view(source, top, Duration::ZERO).sample(),
            turned
        );

        let mut changed = source;
        changed.set_projection(ProjectionKind::Perspective { vertical_fov: 1.2 });
        let transition = CameraTransition::projection(
            source,
            ProjectionKind::Perspective { vertical_fov: 1.2 },
            Duration::ZERO,
        );
        assert!(transition.is_done());
        assert_eq!(transition.sample(), changed);
    }
}

/// A projection animation keeps the scale at the pivot at every sample (a
/// dolly zoom), and the very first sample is the source.
#[test]
fn a_projection_transition_is_a_dolly_zoom() {
    for projection in projections() {
        let source = camera(projection);
        let pivot = source.pivot();
        let scale = source.scale_at(pivot);
        for target in [
            ProjectionKind::Perspective { vertical_fov: 2.0 },
            ProjectionKind::Orthographic,
        ] {
            let mut transition =
                CameraTransition::projection(source, target, Duration::from_millis(300));
            assert_eq!(transition.sample(), source);
            for _ in 0..25 {
                transition.advance(Duration::from_millis(16));
                let sample = transition.sample();
                assert!(
                    (sample.scale_at(pivot) / scale - 1.0).abs() < 1e-4,
                    "{target:?}: {} != {scale}",
                    sample.scale_at(pivot)
                );
            }
            assert_eq!(ProjectionKind::of(transition.sample().projection()), target);
        }
    }
}

/// Fit glides: the pivot travels in a straight line to the
/// sphere's centre and the scale changes monotonically.
#[test]
fn a_fit_transition_moves_pivot_and_scale_monotonically() {
    for projection in projections() {
        let source = camera(projection);
        let target = fit(source, SPHERE);
        let mut transition = CameraTransition::frame(source, target, Duration::from_millis(400));
        let mut last_scale = source.scale_at(source.pivot());
        let shrinking = target.scale_at(target.pivot()) < last_scale;
        for _ in 0..30 {
            transition.advance(Duration::from_millis(16));
            let sample = transition.sample();
            let along = (sample.pivot() - source.pivot()).cross(SPHERE.center - source.pivot());
            assert!(along.length() < 1e-3, "pivot left the straight line");
            let scale = sample.scale_at(sample.pivot());
            assert!(if shrinking {
                scale <= last_scale
            } else {
                scale >= last_scale
            });
            last_scale = scale;
        }
        assert_eq!(transition.sample(), target);
    }
}
