//! Square roots of quantities proven nonnegative by the caller.
//!
//! Differences such as `r² − a²` can round below zero at tangencies or
//! degenerate conics. Clamping is allowed only within a rounding bound;
//! a larger negative value breaks the proof and panics with the call site's name.

/// Rounding allowance in epsilon units. The caller bounds conditioning in
/// `error_scale`; this factor accounts only for the number of operations.
const ROUNDOFF_ULPS: f64 = 16.0;

/// `√value` when the caller proves `value >= 0` apart from rounding.
///
/// `error_scale` bounds absolute error in epsilon units, before the operation
/// count factor. It depends on the formula (internal circle tangency grows
/// as `r³/d`) and coordinate magnitude, including origins near `1e6`.
///
/// Panics below `-ROUNDOFF_ULPS · ε · error_scale`; `what` names the failed proof.
pub fn sqrt_of_proven_nonnegative(value: f64, error_scale: f64, what: &str) -> f64 {
    assert!(
        value.is_finite() && error_scale.is_finite() && error_scale >= 0.0,
        "{what}: non-finite input (value {value}, error scale {error_scale})"
    );
    let roundoff = ROUNDOFF_ULPS * f64::EPSILON * error_scale;
    assert!(
        value >= -roundoff,
        "{what}: {value} is negative beyond rounding ({roundoff}); the non-negativity proof is broken"
    );
    value.max(0.0).sqrt()
}
