//! Typed f64 quantities: `space::{Point3, Vector3}`, `length::Length`,
//! `scale::PerPx` and `angle::Angle`.
//! Units belong to types; `.0` exposes the value at boundaries, without `Deref`.
//!
//! These library quantities are distinct from host domain types such as
//! model points and sketch lengths. Hosts retain those domain types and
//! convert explicitly at library boundaries. Both representations use DVec3/f64, so
//! conversion rewraps a value without recalculation.
//!
//! The quantities support geometric picking, view scales and persisted
//! camera lengths/scales. Screen pixel types belong to the host;
//! this library only carries the unit quantities needed at those boundaries.
//! `Angle` preserves accumulated rotation without wrapping.

pub mod angle;
pub mod length;
pub mod scale;
pub mod space;

#[cfg(test)]
mod units_tests;
