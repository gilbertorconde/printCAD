//! Represent an imported NURBS in the sketch's point-defined spline form.
//!
//! Files supply degree, flat knots, poles and weights. Conversion succeeds only
//! if the same poles in [`super::BSplineCurve::build`] reproduce the carrier:
//! uniform clamped open knots or uniform periodic knots and repeated poles.
//! Unequal weights, nonuniform knots and excessive degrees are explicit
//! conversion losses, rather than substitutions by a similar curve.
//!
//! Compare curve points, not knot encodings: file rounding such as DXF's
//! 0.333333 needs a geometric tolerance. An independent homogeneous rational
//! de Boor evaluator compares corresponding parameters after a linear domain
//! mapping; knots scaled from 0..1 to 0..4 can describe the same curve.

use glam::{DVec2, DVec3};

use super::{BSplineCurve, MAX_SPLINE_DEGREE, SplineDefinition, SplineDegree, SplineShape};
use crate::world::{WorldLength, WorldPoint};

/// General NURBS with `poles.len() + degree + 1` knots and active domain
/// `[knots[degree], knots[poles.len()]]`. Periodic curves are unrolled with the
/// first degree poles repeated at the end.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignSpline {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub poles: Vec<WorldPoint>,
    /// Pole weights, all one for a polynomial spline.
    pub weights: Vec<f64>,
}

/// Native pole-defined shape and sketch points.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeSpline {
    pub shape: SplineShape,
    pub points: Vec<WorldPoint>,
}

/// Why the imported spline cannot use the native point-defined form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotNative {
    /// Invalid knot count/order, empty domain, nonpositive weight or nonfinite value.
    Malformed,
    /// Degree exceeds the supported range.
    Degree(usize),
    /// Unequal pole weights require an explicit rational basis.
    Rational,
    /// Knots match neither open nor periodic uniform native form.
    Knots,
}

impl std::fmt::Display for NotNative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The adjective qualifies "spline(s)" in the diagnostic.
            Self::Malformed => write!(f, "with knots and poles that describe no curve"),
            Self::Degree(degree) => {
                write!(f, "of degree {degree}, above {MAX_SPLINE_DEGREE}")
            }
            Self::Rational => write!(f, "with poles of different weights"),
            Self::Knots => write!(f, "with knots not evenly spaced"),
        }
    }
}

/// Probes between consecutive knots from either curve. Both carriers are
/// smooth with degree <= 5 there. Four times p + 1 probes leave room beyond
/// polynomial interpolation for rational carriers and conditioning.
const PROBES_PER_PIECE: usize = 4 * (MAX_SPLINE_DEGREE as usize + 1);

impl ForeignSpline {
    /// Active parameter domain.
    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.poles.len()])
    }

    /// Active-domain point by rational de Boor evaluation.
    pub fn point(&self, t: f64) -> WorldPoint {
        let p = self.degree;
        let n = self.poles.len();
        // Span k: knots[k] <= t < knots[k + 1]; the right endpoint uses the last nonzero span.
        let mut k = p;
        while k + 1 < n && self.knots[k + 1] <= t {
            k += 1;
        }
        let mut d: Vec<DVec3> = (0..=p)
            .map(|j| {
                let i = k - p + j;
                let w = self.weights[i];
                DVec3::new(self.poles[i].0.x * w, self.poles[i].0.y * w, w)
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = k - p + j;
                let span = self.knots[i + p + 1 - r] - self.knots[i];
                let alpha = if span > 0.0 {
                    (t - self.knots[i]) / span
                } else {
                    0.0
                };
                d[j] = d[j - 1] * (1.0 - alpha) + d[j] * alpha;
            }
        }
        WorldPoint::from_dvec2(DVec2::new(d[p].x / d[p].z, d[p].y / d[p].z))
    }

    /// Whether the record describes a valid curve.
    fn well_formed(&self) -> bool {
        let n = self.poles.len();
        let p = self.degree;
        p >= 1
            && n > p
            && self.knots.len() == n + p + 1
            && self.weights.len() == n
            && self.knots.iter().all(|k| k.is_finite())
            && self.knots.windows(2).all(|w| w[0] <= w[1])
            && self.knots[p] < self.knots[n]
            && self.weights.iter().all(|w| w.is_finite() && *w > 0.0)
            && self.poles.iter().all(|q| q.0.is_finite())
    }

    /// Native poles and shape whose curve lies within `tolerance` of this
    /// carrier at corresponding parameters.
    pub fn native(&self, tolerance: WorldLength) -> Result<NativeSpline, NotNative> {
        if !self.well_formed() {
            return Err(NotNative::Malformed);
        }
        if self.degree > MAX_SPLINE_DEGREE as usize {
            return Err(NotNative::Degree(self.degree));
        }
        let degree = SplineDegree::new(self.degree as u32);
        let open = NativeSpline {
            shape: SplineShape {
                definition: SplineDefinition::Poles,
                degree,
                periodic: false,
            },
            points: self.poles.clone(),
        };
        if self.matches(&open, tolerance) {
            return Ok(open);
        }
        // A periodic record repeats the first degree poles at its end;
        // native periodic shapes need at least three distinct points.
        let n = self.poles.len();
        let distinct = n - self.degree;
        let wraps = distinct >= 3
            && (0..self.degree)
                .all(|i| self.poles[i].distance(self.poles[distinct + i]) <= tolerance);
        if wraps {
            let closed = NativeSpline {
                shape: SplineShape {
                    definition: SplineDefinition::Poles,
                    degree,
                    periodic: true,
                },
                points: self.poles[..distinct].to_vec(),
            };
            if self.matches(&closed, tolerance) {
                return Ok(closed);
            }
        }
        let first = self.weights[0];
        if self.weights.iter().any(|&w| w != first) {
            Err(NotNative::Rational)
        } else {
            Err(NotNative::Knots)
        }
    }

    /// Whether `native` follows this carrier under a linear parameter mapping.
    fn matches(&self, native: &NativeSpline, tolerance: WorldLength) -> bool {
        let curve = BSplineCurve::build(native.shape, &native.points).unwrap();
        // The record has exactly poles + degree + 1 knots; native degree must not reduce.
        if curve.degree() != self.degree {
            return false;
        }
        let (a, b) = self.domain();
        let (na, nb) = curve.domain_values();
        let to_native = |t: f64| na + (t - a) / (b - a) * (nb - na);
        let from_native = |s: f64| a + (s - na) / (nb - na) * (b - a);
        let mut breaks: Vec<f64> = self
            .knots
            .iter()
            .copied()
            .chain(curve.knots.iter().map(|k| from_native(*k)))
            .filter(|t| (a..=b).contains(t))
            .collect();
        breaks.push(a);
        breaks.push(b);
        breaks.sort_by(f64::total_cmp);
        breaks.dedup();
        breaks.windows(2).all(|piece| {
            (0..=PROBES_PER_PIECE).all(|i| {
                let t = piece[0] + (piece[1] - piece[0]) * i as f64 / PROBES_PER_PIECE as f64;
                let native = WorldPoint::from_dvec2(curve.eval(to_native(t)));
                native.distance(self.point(t)) <= tolerance
            })
        })
    }
}
