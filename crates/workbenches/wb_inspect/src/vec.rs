//! The bench's vector arithmetic: glam's `DVec3`, converted from and to
//! the `[f64; 3]` the documents and the kernel contract keep, and the one
//! way a direction is made unit. A vector too short to have a direction
//! has none: every caller says what that means for it.

use glam::DVec3;

/// Shorter than this, a vector has no direction.
const SHORTEST: f64 = 1e-12;

/// `v` scaled to length one; `None` when it is too short to point
/// anywhere or not finite.
pub(crate) fn unit(v: DVec3) -> Option<DVec3> {
    let len = v.length();
    (len > SHORTEST && len.is_finite()).then(|| v * (1.0 / len))
}

/// Two unit directions square to `n` and to each other, the first across
/// X unless `n` lies near X; `None` when `n` has no direction.
pub(crate) fn basis(n: DVec3) -> Option<(DVec3, DVec3)> {
    let n = unit(n)?;
    let pick = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    let u = unit(n.cross(pick))?;
    Some((u, n.cross(u)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vector with length has a unit direction and a square basis; a
    /// zero, a vanishing or a non-finite one has neither.
    #[test]
    fn a_direction_needs_a_length() {
        let n = DVec3::new(0.0, 3.0, 4.0);
        let u = unit(n).unwrap();
        assert!((u.length() - 1.0).abs() < 1e-15 && u.dot(n) > 0.0);
        let (a, b) = basis(n).unwrap();
        for (p, q) in [(a, b), (a, u), (b, u)] {
            assert!(p.dot(q).abs() < 1e-15);
        }
        assert!((a.length() - 1.0).abs() < 1e-15 && (b.length() - 1.0).abs() < 1e-15);
        assert_eq!(basis(DVec3::X), Some((DVec3::Z, -DVec3::Y)));
        for none in [
            DVec3::ZERO,
            DVec3::splat(1e-13),
            DVec3::new(f64::NAN, 0.0, 1.0),
            DVec3::new(f64::INFINITY, 0.0, 0.0),
        ] {
            assert_eq!(unit(none), None);
            assert_eq!(basis(none), None);
        }
    }
}
