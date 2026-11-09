use crate::length::{Length, Offset};
use crate::scale::PerPx;
use crate::space::{Point3, Vector3};

#[test]
#[should_panic(expected = "Length")]
fn length_rejects_a_negative_value() {
    let _ = Length::new(-1.0);
}

#[test]
#[should_panic(expected = "Length")]
fn length_rejects_nan() {
    let _ = Length::new(f64::NAN);
}

#[test]
fn length_accepts_zero_and_negative_zero_in_const_context() {
    const ZERO: Length = Length::new(-0.0);
    assert_eq!(ZERO, Length::ZERO);
}

#[test]
#[should_panic(expected = "Offset")]
fn offset_rejects_nan() {
    let _ = Offset::new(f64::NAN);
}

#[test]
fn offset_accepts_a_negative_value() {
    assert_eq!(Offset::new(-2.5).0, -2.5);
}

#[test]
#[should_panic(expected = "Point3")]
fn point_rejects_a_non_finite_component() {
    let _ = Point3::new(0.0, f64::INFINITY, 0.0);
}

#[test]
#[should_panic(expected = "Vector3")]
fn vector_rejects_a_non_finite_component() {
    let _ = Vector3::new(f64::NAN, 0.0, 0.0);
}

#[test]
#[should_panic(expected = "PerPx")]
fn scale_rejects_zero() {
    let _ = PerPx::new(0.0);
}

/// JSON serialization contract: scale and
/// depth are bare numbers. This test covers JSON only:
/// serde_json writes a newtype as a number even without serde(transparent), so removing it
/// is not detected here; formats that expose newtype names need the attribute.
#[test]
fn scale_and_length_serialize_as_bare_numbers() {
    assert_eq!(serde_json::to_string(&PerPx(0.05)).unwrap(), "0.05");
    assert_eq!(serde_json::to_string(&Length(250.0)).unwrap(), "250.0");
    assert_eq!(serde_json::from_str::<PerPx>("3.0").unwrap(), PerPx(3.0));
    assert_eq!(
        serde_json::from_str::<Length>("10000").unwrap(),
        Length(10000.0)
    );
}
