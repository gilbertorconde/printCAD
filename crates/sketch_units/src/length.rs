use serde::{Deserialize, Serialize};

/// Length in host space units: ray distance, clipping interval or
/// camera depth. Must be finite and nonnegative.
///
/// Persisted camera depths use transparent serde, represented as
/// bare numbers. Deserialization does not invoke the constructor;
/// the host must validate lengths read from external data.
///
/// Arithmetic is deliberately left to domain quantities at the host boundary.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Length(pub f64);

impl Length {
    pub const ZERO: Self = Self::new(0.0);

    pub const fn new(value: f64) -> Self {
        assert!(
            value.is_finite() && value >= 0.0,
            "Length: must be finite and non-negative"
        );
        Self(value)
    }
}

/// Signed displacement along an axis, including a placement joint's translation.
/// Finite; the sign encodes direction. [`Length`]'s nonnegative invariant does
/// not apply to signed displacements.
///
/// Arithmetic is left to host domain quantities, as for [`Length`].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Offset(pub f64);

impl Offset {
    pub const ZERO: Self = Self::new(0.0);

    pub const fn new(value: f64) -> Self {
        assert!(value.is_finite(), "Offset: must be finite");
        Self(value)
    }
}
