//! Typed sketch quantities in f64, with lengths in millimeters.
//!
//! `WorldPoint` is local to a sketch's 2D plane. Model-space points have
//! the separate `ModelPoint3` type and are mapped through `SketchPlane`.
//! Screen pixels remain f32 and cross an explicit scale boundary.
//!
//! # Affine algebra
//!
//! Only operations with a geometric meaning are defined:
//!
//! | Expression | Result |
//! | --- | --- |
//! | `WorldPoint − WorldPoint` | `WorldVec` |
//! | `WorldPoint ± WorldVec` | `WorldPoint` |
//! | `WorldVec ± WorldVec`, `WorldVec × f64` | `WorldVec` |
//! | `WorldVec::length()`, `WorldPoint::distance(_)` | `WorldLength` |
//! | `WorldLength ± WorldLength`, `WorldLength × f64` | `WorldLength` |
//! | `WorldLength / WorldLength` | `f64` (dimensionless ratio) |
//! | `WorldLength × WorldLength` | `WorldArea` |
//!
//! Adding two positions is undefined and must fail to compile:
//!
//! The error is E0308 because the only accepted right operand is `WorldVec`.
//!
//! ```compile_fail,E0308
//! use sketch_geometry::world::WorldPoint;
//! let a = WorldPoint::new(1.0, 2.0);
//! let _ = a + a;
//! ```
//!
//! There is no blanket `From<f64>`; lengths require explicit [`WorldLength::new`].
//!
//! # Invariants and serde
//!
//! Constructors panic on violated invariants. `.0` exposes the value at numerical
//! and serialization boundaries; there is no `Deref`. Transparent serde writes
//! points as `[x, y]` and scalar quantities as numbers. Deserialization itself
//! does not run constructor checks; callers validate external data.

use std::ops::{Add, Div, Mul, Neg, Sub};

use glam::{DMat3, DQuat, DVec2};
use serde::{Deserialize, Serialize};
use sketch_units::length::{Length, Offset};
use sketch_units::scale::PerPx;

/// Position in a sketch's local 2D coordinates. Components must be finite.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldPoint(pub DVec2);

/// Displacement or direction in a sketch's local 2D coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldVec(pub DVec2);

/// Finite nonnegative length: radius, diameter, distance or chamfer size.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldLength(pub f64);

/// Finite signed axis coordinate or label offset. The sign encodes direction,
/// so this quantity is distinct from `WorldLength`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldOffset(pub f64);

/// Finite area; the sign may encode contour orientation.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldArea(pub f64);

/// Finite volume in mm³. A negative divergence-theorem volume diagnoses
/// inward normals and is a valid signed result.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldVolume(pub f64);

/// Symmetric inertia tensor about the center of mass at unit density, in mm⁵
/// and model axes. A rotation transforms it as `R·I·Rᵀ`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldInertia(pub DMat3);

impl WorldPoint {
    /// Sketch coordinate origin.
    pub const ORIGIN: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        assert!(
            x.is_finite() && y.is_finite(),
            "WorldPoint: components must be finite"
        );
        Self(DVec2::new(x, y))
    }

    /// Return from a DVec2 formula, checking finiteness as `new` does.
    /// This explicit boundary avoids blanket conversions into world quantities.
    pub fn from_dvec2(v: DVec2) -> Self {
        Self::new(v.x, v.y)
    }

    /// Distance to `other`.
    pub fn distance(self, other: Self) -> WorldLength {
        (other - self).length()
    }

    /// `self + (other − self)·t`; values outside `[0, 1]` extrapolate.
    pub fn lerp(self, other: Self, t: f64) -> Self {
        self + (other - self) * t
    }

    pub fn midpoint(self, other: Self) -> Self {
        self.lerp(other, 0.5)
    }
}

impl WorldVec {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        assert!(
            x.is_finite() && y.is_finite(),
            "WorldVec: components must be finite"
        );
        Self(DVec2::new(x, y))
    }

    /// Return from a DVec2 formula; see [`WorldPoint::from_dvec2`].
    pub fn from_dvec2(v: DVec2) -> Self {
        Self::new(v.x, v.y)
    }

    pub fn length(self) -> WorldLength {
        WorldLength::new(self.0.length())
    }
}

impl WorldLength {
    pub const ZERO: Self = Self::new(0.0);

    /// Below this length a pair of points has no stable direction. Use the same
    /// threshold for segments, radii and dimensions between coincident points.
    /// `1e-9` is about eight f64 spacings at the supported coordinate scale `1e6`;
    /// normalizing shorter vectors would amplify rounding noise.
    pub const DEGENERATE: Self = Self::new(1e-9);

    pub const fn new(value: f64) -> Self {
        assert!(
            value.is_finite() && value >= 0.0,
            "WorldLength: length must be finite and nonnegative"
        );
        Self(value)
    }

    /// Unordered difference `|self − other|`. Subtraction instead requires
    /// `self >= other` and panics otherwise.
    pub fn abs_diff(self, other: Self) -> Self {
        Self::new((self.0 - other.0).abs())
    }

    /// The same value as a positive signed coordinate.
    pub const fn as_offset(self) -> WorldOffset {
        WorldOffset(self.0)
    }

    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    /// Convert to the unit library's finite nonnegative length.
    pub const fn to_units(self) -> Length {
        Length::new(self.0)
    }

    /// Convert a library length, such as a camera depth, to sketch units.
    pub const fn from_units(length: Length) -> Self {
        Self::new(length.0)
    }
}

impl WorldOffset {
    pub const ZERO: Self = Self::new(0.0);

    pub const fn new(value: f64) -> Self {
        assert!(value.is_finite(), "WorldOffset: value must be finite");
        Self(value)
    }

    /// Absolute value as a length.
    pub fn abs(self) -> WorldLength {
        WorldLength::new(self.0.abs())
    }

    /// Convert to the unit library's finite signed displacement.
    pub const fn to_units(self) -> Offset {
        Offset::new(self.0)
    }

    pub const fn from_units(offset: Offset) -> Self {
        Self::new(offset.0)
    }
}

impl WorldArea {
    pub const ZERO: Self = Self::new(0.0);

    pub const fn new(value: f64) -> Self {
        assert!(value.is_finite(), "WorldArea: value must be finite");
        Self(value)
    }
}

impl WorldVolume {
    pub const fn new(value: f64) -> Self {
        assert!(value.is_finite(), "WorldVolume: value must be finite");
        Self(value)
    }
}

impl WorldInertia {
    pub fn new(tensor: DMat3) -> Self {
        assert!(
            tensor.is_finite(),
            "WorldInertia: components must be finite"
        );
        Self(tensor)
    }

    /// The tensor in axes rotated by `rotation`: `R·I·Rᵀ`.
    pub fn rotated(self, rotation: DQuat) -> Self {
        let r = DMat3::from_quat(rotation);
        Self::new(r * self.0 * r.transpose())
    }
}

// ---------------------------------------------------------------------------
// WorldPoint / WorldVec
// ---------------------------------------------------------------------------

impl Sub for WorldPoint {
    type Output = WorldVec;

    fn sub(self, rhs: Self) -> WorldVec {
        let diff = self.0 - rhs.0;
        WorldVec::new(diff.x, diff.y)
    }
}

impl Add<WorldVec> for WorldPoint {
    type Output = WorldPoint;

    fn add(self, rhs: WorldVec) -> WorldPoint {
        let sum = self.0 + rhs.0;
        WorldPoint::new(sum.x, sum.y)
    }
}

impl Sub<WorldVec> for WorldPoint {
    type Output = WorldPoint;

    fn sub(self, rhs: WorldVec) -> WorldPoint {
        let diff = self.0 - rhs.0;
        WorldPoint::new(diff.x, diff.y)
    }
}

impl Add for WorldVec {
    type Output = WorldVec;

    fn add(self, rhs: Self) -> WorldVec {
        let sum = self.0 + rhs.0;
        WorldVec::new(sum.x, sum.y)
    }
}

impl Sub for WorldVec {
    type Output = WorldVec;

    fn sub(self, rhs: Self) -> WorldVec {
        let diff = self.0 - rhs.0;
        WorldVec::new(diff.x, diff.y)
    }
}

impl Mul<f64> for WorldVec {
    type Output = WorldVec;

    fn mul(self, rhs: f64) -> WorldVec {
        let scaled = self.0 * rhs;
        WorldVec::new(scaled.x, scaled.y)
    }
}

// ---------------------------------------------------------------------------
// WorldLength
// ---------------------------------------------------------------------------

impl Add for WorldLength {
    type Output = WorldLength;

    fn add(self, rhs: Self) -> WorldLength {
        WorldLength::new(self.0 + rhs.0)
    }
}

/// Length difference. Panics if `rhs > self`; use [`WorldLength::abs_diff`]
/// when operand order is unspecified.
impl Sub for WorldLength {
    type Output = WorldLength;

    fn sub(self, rhs: Self) -> WorldLength {
        WorldLength::new(self.0 - rhs.0)
    }
}

/// Scaling; panics on a negative factor.
impl Mul<f64> for WorldLength {
    type Output = WorldLength;

    fn mul(self, rhs: f64) -> WorldLength {
        WorldLength::new(self.0 * rhs)
    }
}

/// A ratio of lengths is dimensionless.
impl Div for WorldLength {
    type Output = f64;

    fn div(self, rhs: Self) -> f64 {
        self.0 / rhs.0
    }
}

impl Mul for WorldLength {
    type Output = WorldArea;

    fn mul(self, rhs: Self) -> WorldArea {
        WorldArea::new(self.0 * rhs.0)
    }
}

// ---------------------------------------------------------------------------
// WorldOffset
// ---------------------------------------------------------------------------

impl Add for WorldOffset {
    type Output = WorldOffset;

    fn add(self, rhs: Self) -> WorldOffset {
        WorldOffset::new(self.0 + rhs.0)
    }
}

impl Sub for WorldOffset {
    type Output = WorldOffset;

    fn sub(self, rhs: Self) -> WorldOffset {
        WorldOffset::new(self.0 - rhs.0)
    }
}

impl Neg for WorldOffset {
    type Output = WorldOffset;

    fn neg(self) -> WorldOffset {
        WorldOffset(-self.0)
    }
}

impl Mul<f64> for WorldOffset {
    type Output = WorldOffset;

    fn mul(self, rhs: f64) -> WorldOffset {
        WorldOffset::new(self.0 * rhs)
    }
}

// ---------------------------------------------------------------------------
// Screen-to-world boundary
// ---------------------------------------------------------------------------

/// Screen distance in pixels, stored in f32.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ScreenPx(pub f32);

impl ScreenPx {
    pub const fn new(value: f32) -> Self {
        assert!(
            value.is_finite() && value >= 0.0,
            "ScreenPx: pixel length must be finite and nonnegative"
        );
        Self(value)
    }
}

/// View scale in model millimeters per pixel; finite and strictly positive.
///
/// Pixel tolerances cross through `WorldPerPx × ScreenPx → WorldLength`,
/// rather than untyped f32 multiplication.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldPerPx(pub f64);

impl WorldPerPx {
    pub const fn new(value: f64) -> Self {
        assert!(
            value.is_finite() && value > 0.0,
            "WorldPerPx: view scale must be positive and finite"
        );
        Self(value)
    }

    /// Convert to the unit library's finite positive scale, serialized as a number.
    pub const fn to_units(self) -> PerPx {
        PerPx::new(self.0)
    }

    pub const fn from_units(scale: PerPx) -> Self {
        Self::new(scale.0)
    }
}

impl Mul<ScreenPx> for WorldPerPx {
    type Output = WorldLength;

    fn mul(self, rhs: ScreenPx) -> WorldLength {
        WorldLength::new(self.0 * f64::from(rhs.0))
    }
}

impl Mul<WorldPerPx> for ScreenPx {
    type Output = WorldLength;

    fn mul(self, rhs: WorldPerPx) -> WorldLength {
        rhs * self
    }
}

/// Convert a world length to pixels at this scale, rounding f64 to screen f32.
impl Div<WorldPerPx> for WorldLength {
    type Output = ScreenPx;

    fn div(self, rhs: WorldPerPx) -> ScreenPx {
        ScreenPx::new((self.0 / rhs.0) as f32)
    }
}

#[cfg(test)]
#[path = "world_tests.rs"]
mod world_tests;
