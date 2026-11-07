/// Finite angle in radians. The sign follows the right-hand rule around the axis
/// supplied by the caller, such as a rotation handle's own axis.
///
/// Magnitude is unbounded: an accumulated 450° rotation is distinct from 90°.
/// Arithmetic is deliberately left to domain quantities at the host boundary.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Angle(pub f64);

impl Angle {
    pub const ZERO: Self = Self::radians(0.0);

    pub const fn radians(value: f64) -> Self {
        assert!(value.is_finite(), "Angle: must be finite");
        Self(value)
    }
}
