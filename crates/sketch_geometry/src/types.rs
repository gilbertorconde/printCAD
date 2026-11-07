use serde::{Deserialize, Serialize};

use crate::world::WorldVec;

/// Angle in radians, stored and calculated in f64. Degrees belong at UI and
/// legacy serialization boundaries; rounding an angle to f32 displaces points
/// by about `radius·2⁻²⁴`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Angle(pub f64);

impl Angle {
    pub fn radians(self) -> f64 {
        self.0
    }

    /// Convert to the unit library's angle, rejecting nonfinite values.
    pub const fn to_units(self) -> sketch_units::angle::Angle {
        sketch_units::angle::Angle::radians(self.0)
    }

    pub const fn from_units(angle: sketch_units::angle::Angle) -> Self {
        Self(angle.0)
    }

    /// Vector direction: `atan2(y, x)`.
    pub fn from_vec(v: WorldVec) -> Self {
        assert!(
            v.0.is_finite(),
            "Angle::from_vec: direction must be finite: {v:?}"
        );
        Self(v.0.y.atan2(v.0.x))
    }

    pub fn normalized(self) -> Self {
        assert!(
            self.0.is_finite(),
            "Angle::normalized: angle must be finite: {self:?}"
        );
        let tau = std::f64::consts::TAU;
        let value = self.0.rem_euclid(tau);
        // Adding τ to a tiny negative remainder can round to τ itself.
        Self(if value == tau { 0.0 } else { value })
    }

    /// CCW sweep from self to `to`: always positive, in (0, TAU].
    pub fn ccw_sweep_to(self, to: Angle) -> Angle {
        let tau = std::f64::consts::TAU;
        let offset = self.ccw_offset(to).0;
        Angle(if offset == 0.0 { tau } else { offset })
    }

    /// CW sweep from self to `to`: always negative, in [-TAU, 0).
    pub fn cw_sweep_to(self, to: Angle) -> Angle {
        Angle(-to.ccw_sweep_to(self).0)
    }

    /// Signed sweep from self to `to` in the given direction.
    /// clockwise=true → negative, clockwise=false → positive.
    pub fn sweep_to(self, to: Angle, clockwise: bool) -> Angle {
        if clockwise {
            self.cw_sweep_to(to)
        } else {
            self.ccw_sweep_to(to)
        }
    }

    /// Offset of `test` from `self` in the CCW direction, in [0, TAU).
    pub fn ccw_offset(self, test: Angle) -> Angle {
        assert!(
            self.0.is_finite() && test.0.is_finite(),
            "Angle::ccw_offset: directions must be finite: {self:?}, {test:?}"
        );
        let difference = test.0 - self.0;
        // Preserve subtraction of nearby directions, including their low bits.
        // Opposite finite extremes may overflow; reduce those operands first.
        let difference = if difference.is_finite() {
            difference
        } else {
            test.normalized().0 - self.normalized().0
        };
        Angle(difference).normalized()
    }

    /// True if `test` lies within the CCW sweep from self to `end`.
    pub fn ccw_contains(self, end: Angle, test: Angle) -> bool {
        self.ccw_offset(test) <= self.ccw_sweep_to(end)
    }

    /// True if cursor is outside the CCW sweep from start to end,
    /// i.e. the arc from start to end should be clockwise to include cursor.
    /// Used by drawing tools to determine arc direction from cursor position.
    pub fn cursor_wants_clockwise(start: Angle, end: Angle, cursor: Angle) -> bool {
        start.ccw_offset(cursor) > start.ccw_sweep_to(end)
    }

    /// Make angle readable for text labels: flip by π if pointing left.
    pub fn readable_rotation(self) -> Angle {
        // Reduce multiple turns before a single π shift can make text readable.
        let mut r = self.normalize_symmetric().0;
        if r > std::f64::consts::FRAC_PI_2 {
            r -= std::f64::consts::PI;
        }
        if r < -std::f64::consts::FRAC_PI_2 {
            r += std::f64::consts::PI;
        }
        Angle(r)
    }

    /// Normalize to [-PI, PI], preserving the sign at the half-turn seam.
    /// Work is bounded even when subtracting τ would not change a large f64.
    pub fn normalize_symmetric(self) -> Angle {
        assert!(
            self.0.is_finite(),
            "Angle::normalize_symmetric: angle must be finite: {self:?}"
        );
        let mut a = self.0 % std::f64::consts::TAU;
        if a > std::f64::consts::PI {
            a -= std::f64::consts::TAU;
        }
        if a < -std::f64::consts::PI {
            a += std::f64::consts::TAU;
        }
        Angle(a)
    }
}

impl std::ops::Add for Angle {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Angle(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Angle {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Angle(self.0 - rhs.0)
    }
}

impl std::ops::Neg for Angle {
    type Output = Self;
    fn neg(self) -> Self {
        Angle(-self.0)
    }
}

impl From<f64> for Angle {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl From<Angle> for f64 {
    fn from(value: Angle) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SegmentParam(pub f64);

impl SegmentParam {
    pub fn get(self) -> f64 {
        self.0
    }

    pub fn clamped(raw: f64, eps: f64) -> Self {
        Self(raw.clamp(eps, 1.0 - eps))
    }

    pub fn is_within_unit(self) -> bool {
        (0.0..=1.0).contains(&self.0)
    }
}

impl From<f64> for SegmentParam {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl From<SegmentParam> for f64 {
    fn from(value: SegmentParam) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct HyperbolaParam(pub f64);

impl HyperbolaParam {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl From<f64> for HyperbolaParam {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl From<HyperbolaParam> for f64 {
    fn from(value: HyperbolaParam) -> Self {
        value.0
    }
}

/// Hyperbola branch: `P(t) = C ± cosh(t)·A + sinh(t)·B`.
///
/// A scalar parameter cannot distinguish the two disconnected branches:
/// cosh is always positive, so the branch must be supplied separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HyperbolaBranch {
    /// Along the major semiaxis toward `focus1`.
    Positive,
    /// Opposite side, toward the second focus.
    Negative,
}

impl HyperbolaBranch {
    /// Multiplier of `cosh(t)·A`.
    pub fn sign(self) -> f64 {
        match self {
            HyperbolaBranch::Positive => 1.0,
            HyperbolaBranch::Negative => -1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ParabolaParam(pub f64);

impl ParabolaParam {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl From<f64> for ParabolaParam {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl From<ParabolaParam> for f64 {
    fn from(value: ParabolaParam) -> Self {
        value.0
    }
}

#[cfg(test)]
mod angle_tests {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};

    use super::Angle;

    #[test]
    fn quarter_turn_sweeps_are_exact_across_the_angle_seam() {
        let axes = [0.0, FRAC_PI_2, PI, -FRAC_PI_2];
        for i in 0..axes.len() {
            let start = Angle(axes[i]);
            let end = Angle(axes[(i + 1) % axes.len()]);
            assert_eq!(start.ccw_sweep_to(end), Angle(FRAC_PI_2));
            assert_eq!(end.cw_sweep_to(start), Angle(-FRAC_PI_2));
            assert_eq!(start.ccw_sweep_to(start), Angle(TAU));
            assert_eq!(start.cw_sweep_to(start), Angle(-TAU));
        }
    }

    #[test]
    fn direction_normalization_preserves_geometry_at_every_required_seam() {
        for degrees in [
            -720.0_f64, -450.0, -360.0, -270.0, -180.0, 0.0, 180.0, 270.0, 360.0, 450.0, 720.0,
        ] {
            for epsilon in [-1e-9, 0.0, 1e-9] {
                let angle = Angle(degrees.to_radians() + epsilon);
                let normalized = angle.normalized().0;
                let symmetric = angle.normalize_symmetric().0;
                assert!((0.0..TAU).contains(&normalized));
                assert!((-PI..=PI).contains(&symmetric));
                for reduced in [normalized, symmetric] {
                    assert!((reduced.sin() - angle.0.sin()).abs() < 2e-14);
                    assert!((reduced.cos() - angle.0.cos()).abs() < 2e-14);
                }
                let readable = angle.readable_rotation().0;
                assert!((-FRAC_PI_2..=FRAC_PI_2).contains(&readable));
                // Text may flip half a turn, but its baseline remains parallel.
                assert!((readable - angle.0).sin().abs() < 2e-14);
            }
        }
        assert_eq!(Angle(PI).normalize_symmetric(), Angle(PI));
        assert_eq!(Angle(-PI).normalize_symmetric(), Angle(-PI));
        assert_eq!(Angle(-f64::MIN_POSITIVE).normalized(), Angle(0.0));
    }

    #[test]
    fn extreme_finite_directions_terminate_and_do_not_overflow_their_difference() {
        for value in [-f64::MAX, -1e100, 1e100, f64::MAX] {
            let angle = Angle(value);
            assert!((-PI..=PI).contains(&angle.normalize_symmetric().0));
            let other = Angle(-value);
            assert!((0.0..TAU).contains(&angle.ccw_offset(other).0));
            assert!((0.0..=TAU).contains(&angle.ccw_sweep_to(other).0));
            assert!(angle.ccw_sweep_to(other).0 > 0.0);
            assert!(angle.cw_sweep_to(other).0 < 0.0);
        }
    }

    #[test]
    fn nonfinite_directions_are_refused_before_they_can_become_arc_membership() {
        use crate::arc::{angle_on_arc, angle_on_arc_cw};

        for invalid in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
            assert!(std::panic::catch_unwind(|| Angle(invalid).normalized()).is_err());
            assert!(std::panic::catch_unwind(|| Angle(invalid).normalize_symmetric()).is_err());
            for (start, end, test) in [
                (Angle(invalid), Angle(1.0), Angle(0.5)),
                (Angle(0.0), Angle(invalid), Angle(0.5)),
                (Angle(0.0), Angle(1.0), Angle(invalid)),
            ] {
                assert!(std::panic::catch_unwind(|| angle_on_arc(start, end, test)).is_err());
                for clockwise in [false, true] {
                    assert!(
                        std::panic::catch_unwind(|| angle_on_arc_cw(start, end, clockwise, test))
                            .is_err()
                    );
                }
            }
        }
    }
}
