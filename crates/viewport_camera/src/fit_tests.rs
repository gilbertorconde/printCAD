use glam::{Quat, Vec2, Vec3};

use super::*;
use crate::controller::{ProjectionKind, orthographic};

fn camera(viewport: Vec2, projection: Projection) -> CameraController {
    let orientation =
        Quat::from_rotation_z(0.3) * Quat::from_rotation_x(1.1) * Quat::from_rotation_z(0.2);
    CameraController::new(
        CameraPose {
            position: orientation * Vec3::Z * 50.0,
            orientation,
        },
        projection,
        viewport,
        Vec3::ZERO,
    )
}

/// Points spread over the sphere's surface (Fibonacci lattice).
fn surface(sphere: BoundingSphere) -> Vec<Vec3> {
    let n = 2000;
    (0..n)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).sqrt();
            let phi = i as f32 * 2.399_963;
            sphere.center + Vec3::new(r * phi.cos(), y, r * phi.sin()) * sphere.radius
        })
        .collect()
}

/// Fit contains the full sphere in both narrow and wide viewports.
///
/// Every surface point of the sphere projects inside the viewport, and the
/// fit is tight: on the binding axis the silhouette reaches within the margin
/// of the edge. Mutation oracle: a vertical-only distance
/// (`r·margin / tan(vfov/2)`) — in the 1:3 window the sphere sticks out
/// sideways.
#[test]
fn fit_shows_the_whole_sphere_in_narrow_and_wide_windows() {
    let sphere = BoundingSphere {
        center: Vec3::new(30.0, -12.0, 7.0),
        radius: 5.0,
    };
    let mut cases = 0;
    for viewport in [
        Vec2::new(1200.0, 400.0),
        Vec2::new(400.0, 1200.0),
        Vec2::splat(600.0),
    ] {
        for projection in [
            Projection::Perspective { vertical_fov: 1.0 },
            Projection::Perspective { vertical_fov: 0.2 },
            orthographic(viewport, 0.37),
        ] {
            let fitted = fit(camera(viewport, projection), sphere);
            let view = fitted.view();
            let mut nearest_edge = f32::INFINITY;
            for point in surface(sphere) {
                let screen = view
                    .world_to_screen(point)
                    .expect("sphere in front of the camera");
                assert!(
                    screen.cmpge(Vec2::ZERO).all() && screen.cmple(viewport).all(),
                    "{viewport:?} {projection:?}: {screen:?} outside"
                );
                nearest_edge = nearest_edge
                    .min(screen.min_element())
                    .min((viewport - screen).min_element());
            }
            // The silhouette is a centred circle. On the binding axis (half
            // extent `min(w, h)/2`) it reaches the fraction `filled` of the
            // half extent: `1/margin` in ortho; in perspective the tangent of
            // the sphere's angular radius `asin(sin α / margin)` over `tan α`.
            let filled = match projection {
                Projection::Orthographic { .. } => 1.0 / FIT_MARGIN,
                Projection::Perspective { vertical_fov } => {
                    let half_vertical = vertical_fov * 0.5;
                    let half_horizontal = (half_vertical.tan() * viewport.x / viewport.y).atan();
                    let alpha = half_vertical.min(half_horizontal);
                    (alpha.sin() / FIT_MARGIN).asin().tan() / alpha.tan()
                }
            };
            let free = viewport.min_element() * 0.5 * (1.0 - filled);
            // Lattice points fall just short of the silhouette.
            assert!(
                nearest_edge >= free - 1e-2 && nearest_edge < free + 2.0,
                "{viewport:?} {projection:?}: nearest edge {nearest_edge}, expected {free}"
            );
            cases += 1;
        }
    }
    assert_eq!(cases, 9);
}

/// A fit moves the view, not its attitude: orientation (with roll) and
/// projection kind stay, the pivot is the centre and sits mid-screen. A point
/// has no size to fit: the scale stays.
#[test]
fn fit_keeps_orientation_and_centres_the_pivot() {
    let viewport = Vec2::new(800.0, 600.0);
    let sphere = BoundingSphere::from_aabb(Vec3::new(-1.0, 2.0, 3.0), Vec3::new(5.0, 4.0, 9.0));
    assert_eq!(sphere.center, Vec3::new(2.0, 3.0, 6.0));
    let point = BoundingSphere {
        center: Vec3::new(4.0, -2.0, 1.0),
        radius: 0.0,
    };
    for projection in [
        Projection::Perspective { vertical_fov: 0.9 },
        orthographic(viewport, 2.0),
    ] {
        let source = camera(viewport, projection);
        let fitted = fit(source, sphere);
        assert_eq!(fitted.pose().orientation, source.pose().orientation);
        assert_eq!(
            ProjectionKind::of(fitted.projection()),
            ProjectionKind::of(source.projection())
        );
        assert_eq!(fitted.pivot(), sphere.center);
        assert!(
            fitted
                .view()
                .world_to_screen(sphere.center)
                .unwrap()
                .distance(viewport * 0.5)
                < 1e-3
        );
        let on_point = fit(source, point);
        let before = source.scale_at(source.pivot());
        assert!((on_point.scale_at(point.center) / before - 1.0).abs() < 1e-5);
    }
}

/// Focus keeps its viewing side and requested distance.
#[test]
fn focus_preserves_the_viewing_side_and_hits_the_requested_distance() {
    let target = Vec3::new(1.0, 2.0, 3.0);
    let current = CameraPose::looking_at(Vec3::new(11.0, 2.0, 3.0), target, Vec3::Y);
    let focused = focus_pose(current, target, 25.0, None, Vec3::Z, Vec3::Y);

    assert!((focused.position.distance(target) - 25.0).abs() < 1e-6);
    assert!(focused.position.x > target.x);
    assert!(
        focused
            .forward()
            .distance((target - focused.position).normalize())
            < 1e-6
    );
}

/// Focus on a Z-up scene keeps the horizon level.
/// Here screen-right stays horizontal and screen-up points to +Z; looking
/// straight down takes the pole's up. Mutation oracle: pass `Vec3::Y` as the
/// world up — the horizon tilts.
#[test]
fn focus_on_a_z_up_scene_keeps_the_horizon() {
    let target = Vec3::new(5.0, -3.0, 2.0);
    let current = CameraPose::looking_at(Vec3::new(20.0, 10.0, 15.0), target, Vec3::Z);
    for requested in [
        Some(Vec3::new(1.0, 0.3, 0.6)),
        None,
        Some(Vec3::new(-0.2, 1.0, -0.4)),
    ] {
        let focused = focus_pose(current, target, 12.0, requested, Vec3::NEG_Y, Vec3::Z);
        assert!(
            (focused.orientation * Vec3::X).dot(Vec3::Z).abs() < 1e-5,
            "{requested:?}"
        );
        assert!(
            (focused.orientation * Vec3::Y).dot(Vec3::Z) > 0.0,
            "{requested:?}"
        );
        assert!((focused.position.distance(target) - 12.0).abs() < 1e-4);
    }
    let top = focus_pose(current, target, 12.0, Some(Vec3::Z), Vec3::NEG_Y, Vec3::Z);
    assert!((top.orientation * Vec3::Y).distance(Vec3::Y) < 1e-6);
}
