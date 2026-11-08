use serde::{Deserialize, Serialize};

/// View scale in host space units per logical screen pixel.
/// Must be finite and strictly positive.
///
/// Camera settings persist this value,
/// so transparent serde represents it as a bare number.
/// Deserialization does not invoke the constructor; values read from
/// a file require host validation at the file boundary.
///
/// Arithmetic is deliberately left to domain quantities at the host boundary.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PerPx(pub f64);

impl PerPx {
    pub const fn new(value: f64) -> Self {
        assert!(
            value.is_finite() && value > 0.0,
            "PerPx: scale must be finite and positive"
        );
        Self(value)
    }
}
