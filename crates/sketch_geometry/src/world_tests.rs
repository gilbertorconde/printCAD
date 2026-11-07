// Format oracles compare typed quantities against f32 vector/scalar encodings.
#![allow(
    clippy::disallowed_types,
    reason = "serde-format oracle compares against the former f32 Vec2 layout"
)]

use glam::{DVec2, Vec2};

use super::{WorldArea, WorldLength, WorldOffset, WorldPoint, WorldVec};

// ---------------------------------------------------------------------------
// Constructor invariants
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "WorldLength")]
fn length_rejects_a_negative_value() {
    let _ = WorldLength::new(-1.0);
}

#[test]
#[should_panic(expected = "WorldLength")]
fn length_rejects_nan() {
    let _ = WorldLength::new(f64::NAN);
}

#[test]
#[should_panic(expected = "WorldLength")]
fn length_rejects_infinity() {
    let _ = WorldLength::new(f64::INFINITY);
}

#[test]
fn length_accepts_zero_and_negative_zero() {
    assert_eq!(WorldLength::new(0.0).0, 0.0);
    assert_eq!(WorldLength::new(-0.0).0, 0.0);
}

#[test]
#[should_panic(expected = "WorldPoint")]
fn point_rejects_a_non_finite_component() {
    let _ = WorldPoint::new(1.0, f64::NAN);
}

#[test]
#[should_panic(expected = "WorldVec")]
fn vec_rejects_a_non_finite_component() {
    let _ = WorldVec::new(f64::INFINITY, 0.0);
}

#[test]
#[should_panic(expected = "WorldOffset")]
fn offset_rejects_nan() {
    let _ = WorldOffset::new(f64::NAN);
}

#[test]
#[should_panic(expected = "WorldArea")]
fn area_rejects_infinity() {
    let _ = WorldArea::new(f64::INFINITY);
}

/// Constructors support compile-time tolerance and dimension constants.
#[test]
fn constructors_work_in_const_context() {
    const RADIUS: WorldLength = WorldLength::new(2.5);
    const SHIFT: WorldOffset = WorldOffset::new(-4.0);
    const CORNER: WorldPoint = WorldPoint::new(1.0, -2.0);
    const STEP: WorldVec = WorldVec::new(0.5, 0.25);
    const FLOOR: WorldArea = WorldArea::new(-3.0);
    assert_eq!(RADIUS.0, 2.5);
    assert_eq!(SHIFT.0, -4.0);
    assert_eq!(CORNER.0, DVec2::new(1.0, -2.0));
    assert_eq!(STEP.0, DVec2::new(0.5, 0.25));
    assert_eq!(FLOOR.0, -3.0);
}

// ---------------------------------------------------------------------------
// Algebra
// ---------------------------------------------------------------------------

#[test]
fn point_minus_point_is_a_vec_and_plus_vec_returns_the_point() {
    let a = WorldPoint::new(1.0, 2.0);
    let b = WorldPoint::new(4.0, 6.0);
    let delta = b - a;
    assert_eq!(delta, WorldVec::new(3.0, 4.0));
    assert_eq!(a + delta, b);
    assert_eq!(b - delta, a);
}

#[test]
fn vec_length_and_point_distance_are_lengths() {
    let a = WorldPoint::new(1.0, 2.0);
    let b = WorldPoint::new(4.0, 6.0);
    assert_eq!(WorldVec::new(3.0, 4.0).length(), WorldLength::new(5.0));
    assert_eq!(a.distance(b), WorldLength::new(5.0));
    assert_eq!(b.distance(a), a.distance(b));
}

#[test]
fn lerp_and_midpoint() {
    let a = WorldPoint::new(0.0, 10.0);
    let b = WorldPoint::new(4.0, 20.0);
    assert_eq!(a.lerp(b, 0.0), a);
    assert_eq!(a.lerp(b, 1.0), b);
    assert_eq!(a.lerp(b, 0.25), WorldPoint::new(1.0, 12.5));
    assert_eq!(a.midpoint(b), WorldPoint::new(2.0, 15.0));
}

#[test]
fn vec_arithmetic() {
    let u = WorldVec::new(1.0, 2.0);
    let v = WorldVec::new(3.0, -1.0);
    assert_eq!(u + v, WorldVec::new(4.0, 1.0));
    assert_eq!(u - v, WorldVec::new(-2.0, 3.0));
    assert_eq!(u * 2.0, WorldVec::new(2.0, 4.0));
    assert_eq!(u * -1.0, WorldVec::new(-1.0, -2.0));
}

#[test]
fn length_arithmetic() {
    let a = WorldLength::new(6.0);
    let b = WorldLength::new(4.0);
    assert_eq!(a + b, WorldLength::new(10.0));
    assert_eq!(a - b, WorldLength::new(2.0));
    assert_eq!(a * 0.5, WorldLength::new(3.0));
    assert_eq!(a / b, 1.5);
    assert_eq!(a * b, WorldArea::new(24.0));
    assert_eq!(b.abs_diff(a), WorldLength::new(2.0));
    assert_eq!(a.abs_diff(b), WorldLength::new(2.0));
}

/// A negative length difference violates the subtraction contract.
#[test]
#[should_panic(expected = "WorldLength")]
fn length_subtraction_below_zero_panics() {
    let _ = WorldLength::new(1.0) - WorldLength::new(2.0);
}

#[test]
#[should_panic(expected = "WorldLength")]
fn length_scaled_by_a_negative_factor_panics() {
    let _ = WorldLength::new(1.0) * -2.0;
}

#[test]
fn offset_is_signed_and_converts_to_length_by_abs() {
    let up = WorldOffset::new(3.0);
    let down = -up;
    assert_eq!(down, WorldOffset::new(-3.0));
    assert_eq!(up + down, WorldOffset::ZERO);
    assert_eq!(up - down, WorldOffset::new(6.0));
    assert_eq!(down * 2.0, WorldOffset::new(-6.0));
    assert_eq!(down.abs(), WorldLength::new(3.0));
    assert_eq!(WorldLength::new(3.0).as_offset(), up);
}

// ---------------------------------------------------------------------------
// File format
// ---------------------------------------------------------------------------

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

/// WorldPoint uses the same [x, y] array encoding as f32 vectors and tuples,
/// allowing old files to load without migration.
#[test]
fn point_serializes_like_vec2_and_like_the_fix_tuple() {
    let point = WorldPoint::new(1.5, -2.25);
    assert_eq!(json(&point), json(&Vec2::new(1.5, -2.25)));
    assert_eq!(json(&point), json(&(1.5_f32, -2.25_f32)));
    assert_eq!(json(&point), "[1.5,-2.25]");

    let from_tuple: WorldPoint = serde_json::from_str(&json(&(1.5_f32, -2.25_f32))).unwrap();
    assert_eq!(from_tuple, point);
    let from_vec2: WorldPoint = serde_json::from_str(&json(&Vec2::new(1.5, -2.25))).unwrap();
    assert_eq!(from_vec2, point);
}

#[test]
fn scalars_serialize_as_bare_numbers() {
    assert_eq!(json(&WorldLength::new(2.5)), json(&2.5_f32));
    assert_eq!(json(&WorldOffset::new(-2.5)), json(&-2.5_f32));
    assert_eq!(json(&WorldArea::new(6.25)), json(&6.25_f32));
    assert_eq!(json(&WorldVec::new(1.0, 2.0)), json(&Vec2::new(1.0, 2.0)));
    assert_eq!(
        serde_json::from_str::<WorldLength>("2.5").unwrap(),
        WorldLength::new(2.5)
    );
}

/// Read decimal values as their nearest f64: 4173.9 must not become
/// 4173.89990234375. Bit-exact JSON roundtrips require float_roundtrip.
#[test]
fn an_f32_era_decimal_is_read_as_the_nearest_f64() {
    let point: WorldPoint = serde_json::from_str("[-4756.27,4173.9]").unwrap();
    assert_eq!(point.0.x.to_bits(), (-4756.27_f64).to_bits());
    assert_eq!(point.0.y.to_bits(), 4173.9_f64.to_bits());
    assert_ne!(point.0.y, f64::from(4173.9_f32));
}

#[test]
fn f64_roundtrips_bitwise_through_json() {
    for value in [
        1_000_000.000_1_f64,
        -1_000_000.0,
        4173.9,
        f64::MIN_POSITIVE,
        0.1 + 0.2,
    ] {
        let text = json(&WorldOffset::new(value));
        let back: WorldOffset = serde_json::from_str(&text).unwrap();
        assert_eq!(back.0.to_bits(), value.to_bits(), "{text}");
    }
}
