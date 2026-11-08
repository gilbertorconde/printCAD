//! Geometric oracles for f64 primitive, arc and bounds tolerances.

use crate::arc::dist_to_segment;
use crate::bbox::arc_bbox;
use crate::primitives::{circumcircle, compute_ellipse_focus};
use crate::types::Angle;
use crate::world::{WorldLength, WorldPoint};

fn pt(x: f64, y: f64) -> WorldPoint {
    WorldPoint::new(x, y)
}

/// A right triangle with leg h has circumcenter at its hypotenuse midpoint
/// and radius h/√2. At h = 1e-4, d = 2e-8 is still nondegenerate in f64.
#[test]
fn circumcircle_accepts_a_small_honest_triangle() {
    let h = 1e-4;
    let (center, radius) = circumcircle(pt(0.0, 0.0), pt(h, 0.0), pt(0.0, h))
        .expect("a 1e-4 right triangle is nondegenerate: d = 2e-8 exceeds f64 epsilon");
    assert!((center.0.x - h / 2.0).abs() < 1e-18 && (center.0.y - h / 2.0).abs() < 1e-18);
    assert!((radius.0 - h / std::f64::consts::SQRT_2).abs() < 1e-18);
}

/// Squared absolute coordinates would lose about 1e-6 precision at |p| = 1e5
/// for a triangle of size 1e-3. Require accuracy near ulp(1e5), about 1.5e-11.
#[test]
fn circumcircle_far_from_origin_keeps_f64_precision() {
    let (ox, oy) = (1e5, 1e5);
    let h = 1e-3;
    let (center, radius) = circumcircle(pt(ox, oy), pt(ox + h, oy), pt(ox, oy + h)).unwrap();
    assert!(
        (center.0.x - (ox + h / 2.0)).abs() < 1e-10,
        "center x {}",
        center.0.x
    );
    assert!(
        (center.0.y - (oy + h / 2.0)).abs() < 1e-10,
        "center y {}",
        center.0.y
    );
    // Inputs ox + h are quantized at ulp(1e5), limiting radius accuracy to about 1e-11.
    assert!(
        (radius.0 - h / std::f64::consts::SQRT_2).abs() < 1e-10,
        "radius {}",
        radius.0
    );
}

/// A radius-1e6 arc with a 5e-7 rad gap around +X misses that quadrant:
/// max x = R·cos(2.5e-7) = R − 3.1e-8. Full-turn tolerance must resolve the gap.
#[test]
fn arc_bbox_of_a_nearly_full_arc_does_not_reach_the_uncovered_quadrant() {
    let radius = 1e6;
    let gap = 5e-7;
    let bbox = arc_bbox(
        pt(0.0, 0.0),
        WorldLength::new(radius),
        Angle(gap / 2.0),
        Angle(std::f64::consts::TAU - gap),
    );
    assert!(bbox.max().0.x < radius - 1e-8, "max.x = {}", bbox.max().0.x);
    assert!(bbox.max().0.x > radius - 1e-7, "max.x = {}", bbox.max().0.x);
}

/// An exact full turn still has circle bounds under the ULP tolerance.
#[test]
fn arc_bbox_of_a_full_turn_is_the_circle_bbox() {
    let bbox = arc_bbox(
        pt(0.0, 0.0),
        WorldLength::new(10.0),
        Angle(0.3),
        Angle(std::f64::consts::TAU),
    );
    assert!((bbox.max().0.x - 10.0).abs() < 1e-12 && (bbox.min().0.y + 10.0).abs() < 1e-12);
}

/// A 5e-5 segment is not a point; its endpoint has zero distance to the segment.
#[test]
fn dist_to_segment_sees_a_segment_shorter_than_the_f32_threshold() {
    let d = dist_to_segment(pt(5e-5, 0.0), pt(0.0, 0.0), pt(5e-5, 0.0));
    assert!(d.0 < 1e-15, "distance to segment endpoint {}", d.0);
}

/// A 1e-7 axis is above DEGENERATE and has a well-defined focus.
#[test]
fn ellipse_focus_with_a_short_axis_is_computed_not_defaulted() {
    let center = pt(0.0, 0.0);
    // +X semiaxis a = 1e-7, rim on its normal: b = 5e-8 < a gives an axial focus.
    let focus = compute_ellipse_focus(center, pt(1e-7, 0.0), pt(0.0, 5e-8)).unwrap();
    assert!(focus.0.x > 0.0, "focus stayed at the center: {focus:?}");
}

#[test]
fn ellipse_focus_near_major_axis_scales_with_the_geometry() {
    let center = pt(0.0, 0.0);
    let axis = pt(10.0, 0.0);
    let start = pt(9.999_999_5, 1e-4);
    let focus = compute_ellipse_focus(center, axis, start).unwrap();
    let denom = 1.0 - (start.0.x / axis.0.x).powi(2);
    let b = start.0.y / denom.sqrt();
    let expected = (axis.0.x.powi(2) - b.powi(2)).sqrt();
    assert!((focus.0.x - expected).abs() < 1e-12);

    let scale = 1e-3;
    let scaled = compute_ellipse_focus(
        center,
        pt(axis.0.x * scale, 0.0),
        pt(start.0.x * scale, start.0.y * scale),
    )
    .unwrap();
    assert!((scaled.0.x - expected * scale).abs() < 1e-10);
}

#[test]
fn ellipse_focus_rejects_an_underdetermined_major_axis_start() {
    let center = pt(0.0, 0.0);
    assert_eq!(
        compute_ellipse_focus(center, pt(10.0, 0.0), pt(10.0, 0.0)),
        None
    );
    assert_eq!(
        compute_ellipse_focus(center, pt(10.0, 0.0), pt(11.0, 1.0)),
        None
    );
}

/// Exactly collinear points must be rejected.
#[test]
fn circumcircle_rejects_collinear_points() {
    assert!(
        circumcircle(
            pt(1e5, 1e5),
            pt(1e5 + 1.0, 1e5 + 1.0),
            pt(1e5 + 2.0, 1e5 + 2.0)
        )
        .is_none()
    );
}

#[test]
fn angle_boundaries_keep_direction_sweep_and_readable_rotation() {
    for boundary in [180.0f64, 270.0, 360.0, 450.0, 720.0] {
        for sign in [-1.0, 1.0] {
            for epsilon in [-0.001f64, 0.0, 0.001] {
                let degrees = sign * (boundary + epsilon);
                let angle = Angle(degrees.to_radians());
                let normalized = angle.normalized();
                assert!((0.0..std::f64::consts::TAU).contains(&normalized.radians()));
                // Compare direction vectors independently of normalization.
                assert!((angle.radians().cos() - normalized.radians().cos()).abs() < 1e-12);
                assert!((angle.radians().sin() - normalized.radians().sin()).abs() < 1e-12);
                for direction in [-1.0, 1.0] {
                    let to = Angle((degrees + direction * 2.0).to_radians());
                    let sweep = angle.sweep_to(to, direction < 0.0).radians();
                    assert!((sweep - direction * 2_f64.to_radians()).abs() < 1e-12);
                }
                let readable = angle.readable_rotation().radians();
                assert!(
                    readable.abs() <= std::f64::consts::FRAC_PI_2 + 1e-12,
                    "{degrees}° gives unreadable text rotation {readable}"
                );
                // Readable label rotation preserves the text axis modulo π.
                assert!((2.0 * (readable - angle.radians())).sin().abs() < 1e-12);
                assert!(((2.0 * (readable - angle.radians())).cos() - 1.0).abs() < 1e-12);
            }
        }
    }
}
