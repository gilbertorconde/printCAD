//! One evaluable sketch B-spline for rendering, hover, snap and preview.
//!
//! Points (poles or interpolation points), degree and periodicity define the
//! shape. [`BSplineCurve::build`] derives a basis from points; exact conics
//! carry explicit knots and weights in [`SplineBasis`]. Moving the defining
//! points moves the same curve for every consumer, including tool previews.
//!
//! Knot conventions:
//! * Open poles use uniform clamped knots with `degree + 1` endpoint repeats;
//!   the curve meets the end poles tangent to the first and last polygon arms.
//! * Open interpolation uses chord parameters and averaged knots (Piegl &
//!   Tiller, The NURBS Book, (9.5), (9.8)), retaining both endpoint points.
//! * Periodic curves use uniform knots and cyclically repeated poles, giving
//!   C^(p−1) continuity at the seam. Interpolation parameters are knots for odd
//!   degree and span midpoints for even degree. This makes the matrix strictly
//!   diagonally dominant for any point count; quadratic interpolation at knots
//!   would be singular for an even count.
//!
//! With few points the effective degree is at most `points − 1`: two give a
//! segment and three a parabola. The stored degree is retained for later points.

use std::ops::{Add, Div, Mul, Sub};

use glam::DVec2;
use serde::{Deserialize, Serialize};

use crate::world::{WorldLength, WorldPoint, WorldVec};

/// Maximum supported sketch spline degree.
pub const MAX_SPLINE_DEGREE: u32 = 5;

/// B-spline degree in `1..=MAX_SPLINE_DEGREE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SplineDegree(u32);

impl SplineDegree {
    pub const CUBIC: Self = Self(3);

    pub const fn new(degree: u32) -> Self {
        assert!(
            degree >= 1 && degree <= MAX_SPLINE_DEGREE,
            "SplineDegree: degree must be in 1..=5"
        );
        Self(degree)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Dimensionless spline parameter, bounded by [`BSplineCurve::domain`].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SplineParam(pub f64);

/// A finite, strictly positive rational pole weight (dimensionless).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct SplineWeight(f64);

impl SplineWeight {
    pub fn new(value: f64) -> Result<Self, &'static str> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err("A spline weight must be finite and greater than zero")
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for SplineWeight {
    type Error = &'static str;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SplineWeight> for f64 {
    fn from(weight: SplineWeight) -> Self {
        weight.get()
    }
}

#[cfg(test)]
/// Interior knot multiplicity; zero removes the knot. The curve additionally
/// checks that the requested count does not exceed its actual degree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SplineMultiplicity(u32);

#[cfg(test)]
impl SplineMultiplicity {
    pub const fn new(count: u32) -> Self {
        assert!(
            count <= MAX_SPLINE_DEGREE,
            "SplineMultiplicity exceeds maximum degree"
        );
        Self(count)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Meaning of the defining points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SplineDefinition {
    /// Control polygon poles; an open curve meets only its endpoint poles.
    Poles,
    /// The curve passes through every point in order.
    Through,
}

/// Spline shape stored alongside its point list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SplineShape {
    pub definition: SplineDefinition,
    pub degree: SplineDegree,
    /// Uniform periodic curve when built from points. An explicit basis may
    /// instead define a weighted ring or a clamped curve with coincident ends
    /// and periodic traversal.
    pub periodic: bool,
}

/// Explicit NURBS basis. `None` at the sketch boundary retains the
/// point-defined basis; `Some` keeps these knots and weights while poles move.
/// A closed explicit curve repeats its first pole at the end and wraps its
/// parameter, or uses a uniform periodic ring: then knots include the extended
/// period, while weights name only the original poles (like sketch point ids).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplineBasis {
    pub knots: Vec<f64>,
    pub weights: Vec<f64>,
}

impl SplineBasis {
    /// A ring has `p` extra knots for the poles repeated by evaluation. This
    /// distinguishes it from a clamped closed carrier without a file-format flag.
    pub fn is_periodic_ring(&self, shape: SplineShape, poles: usize) -> bool {
        shape.periodic && self.knots.len() == poles + 2 * shape.degree.get() as usize + 1
    }

    pub fn validate(&self, shape: SplineShape, poles: usize) -> Result<(), BSplineError> {
        let p = shape.degree.get() as usize;
        let u = &self.knots;
        let ring = self.is_periodic_ring(shape, poles);
        let valid = p >= 1
            && p <= MAX_SPLINE_DEGREE as usize
            && shape.definition == SplineDefinition::Poles
            && poles > p
            && u.len() == poles + p + 1 + if ring { p } else { 0 }
            && self.weights.len() == poles
            && self.weights.iter().all(|w| w.is_finite() && *w > 0.0)
            && u.iter().all(|t| t.is_finite())
            && u.windows(2).all(|w| w[0] <= w[1])
            && if ring {
                poles >= 3 && u.iter().enumerate().all(|(i, t)| *t == i as f64)
            } else {
                u[p] < u[poles]
                    && u[..=p].iter().all(|t| *t == u[p])
                    && u[poles..].iter().all(|t| *t == u[poles])
                    && u[p + 1..poles]
                        .chunk_by(|a, b| a == b)
                        .all(|run| run.len() <= p)
            };
        if valid {
            Ok(())
        } else {
            Err(BSplineError::InvalidBasis)
        }
    }
}

/// Why the points do not define a spline.
///
/// Dragging may legitimately collapse points; report the resulting degeneracy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BSplineError {
    InvalidBasis,
    /// Fewer than two open points or three periodic points.
    TooFewPoints {
        needed: usize,
        got: usize,
    },
    /// Adjacent interpolation points coincide, including the periodic seam pair.
    CoincidentThroughPoints {
        first: usize,
        second: usize,
    },
}

impl std::fmt::Display for BSplineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBasis => write!(f, "invalid spline knots or positive weights"),
            Self::TooFewPoints { needed, got } => {
                write!(f, "a B-spline needs at least {needed} points, it has {got}")
            }
            Self::CoincidentThroughPoints { first, second } => {
                write!(
                    f,
                    "B-spline points {} and {} coincide",
                    first + 1,
                    second + 1
                )
            }
        }
    }
}

/// Coincidence threshold for degenerate chord parameterization, at coordinate
/// storage precision (`1e-9` mm, far below a visible distance).
const COINCIDENT_EPS: f64 = 1e-9;

/// Scalar over which spline construction and evaluation operate.
///
/// Geometry uses f64; a solver can use dual numbers to obtain the exact Jacobian
/// of this same curve. Interpolation poles solve a system and open knots depend
/// on chord lengths, so derivatives propagate through construction itself.
/// Both scalar types share the algorithm.
///
/// Span selection, pivoting and period reduction inspect [`value`](Self::value),
/// independently of its derivative.
pub trait SplineReal:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Div<Output = Self>
{
    /// Constant with zero derivative.
    fn constant(value: f64) -> Self;
    fn value(self) -> f64;
    /// Shift to `value` by a constant, retaining derivatives for periodic reduction.
    fn with_value(self, value: f64) -> Self;
    fn sqrt(self) -> Self;
}

impl SplineReal for f64 {
    fn constant(value: f64) -> Self {
        value
    }

    fn value(self) -> f64 {
        self
    }

    fn with_value(self, value: f64) -> Self {
        value
    }

    fn sqrt(self) -> Self {
        f64::sqrt(self)
    }
}

/// Planar point or vector over the spline scalar.
pub type SplineVec<S> = [S; 2];

fn vec_add<S: SplineReal>(a: SplineVec<S>, b: SplineVec<S>) -> SplineVec<S> {
    [a[0] + b[0], a[1] + b[1]]
}

fn vec_sub<S: SplineReal>(a: SplineVec<S>, b: SplineVec<S>) -> SplineVec<S> {
    [a[0] - b[0], a[1] - b[1]]
}

fn vec_scale<S: SplineReal>(a: SplineVec<S>, k: S) -> SplineVec<S> {
    [a[0] * k, a[1] * k]
}

fn vec_zero<S: SplineReal>() -> SplineVec<S> {
    [S::constant(0.0), S::constant(0.0)]
}

/// Evaluable degree, knots and poles. Periodic poles repeat the first `degree`
/// at the end, allowing the same evaluation for open and periodic curves.
///
/// `S` is the scalar ([`SplineReal`]); sketch geometry uses f64.
#[derive(Debug, Clone, PartialEq)]
pub struct BSplineCurve<S = f64> {
    degree: usize,
    knots: Vec<S>,
    poles: Vec<SplineVec<S>>,
    /// Number of distinct poles, excluding periodic repeats.
    distinct: usize,
    periodic: bool,
    weights: Option<Vec<S>>,
}

impl BSplineCurve {
    /// Freeze the current pole basis, preserving the original periodic poles.
    /// Interpolated curves must first explicitly change to a pole definition:
    /// their model points are interpolation targets, not these control poles.
    pub fn pole_basis(&self) -> SplineBasis {
        let count = if self.periodic && self.knots[0] < self.knots[self.degree] {
            self.distinct
        } else {
            self.poles.len()
        };
        SplineBasis {
            knots: self.knots.clone(),
            weights: self
                .weights
                .as_ref()
                .map_or_else(|| vec![1.0; count], |w| w[..count].to_vec()),
        }
    }

    pub fn build_with_basis(
        shape: SplineShape,
        basis: Option<&SplineBasis>,
        points: &[WorldPoint],
    ) -> Result<Self, BSplineError> {
        let pts: Vec<_> = points.iter().map(|p| p.0.to_array()).collect();
        Self::from_definition(shape, basis, &pts)
    }

    /// Build from sketch points and shape.
    pub fn build(shape: SplineShape, points: &[WorldPoint]) -> Result<Self, BSplineError> {
        Self::build_with_basis(shape, None, points)
    }
}

impl<S: SplineReal> BSplineCurve<S> {
    /// The same definition for numeric geometry and automatic differentiation.
    pub fn from_definition(
        shape: SplineShape,
        basis: Option<&SplineBasis>,
        points: &[SplineVec<S>],
    ) -> Result<Self, BSplineError> {
        let Some(basis) = basis else {
            return Self::from_points(shape, points);
        };
        basis.validate(shape, points.len())?;
        let ring = basis.is_periodic_ring(shape, points.len());
        let degree = shape.degree.get() as usize;
        let mut poles = points.to_vec();
        let mut weights: Vec<_> = basis.weights.iter().map(|&w| S::constant(w)).collect();
        if ring {
            poles.extend_from_within(..degree);
            weights.extend_from_within(..degree);
        }
        Ok(Self {
            degree,
            knots: basis.knots.iter().map(|&u| S::constant(u)).collect(),
            poles,
            distinct: points.len() - usize::from(shape.periodic && !ring),
            periodic: shape.periodic,
            weights: Some(weights),
        })
    }
    /// Build over any spline scalar (see [`BSplineCurve::build`]).
    pub fn from_points(shape: SplineShape, points: &[SplineVec<S>]) -> Result<Self, BSplineError> {
        let needed = if shape.periodic { 3 } else { 2 };
        if points.len() < needed {
            return Err(BSplineError::TooFewPoints {
                needed,
                got: points.len(),
            });
        }
        let degree = (shape.degree.get() as usize).min(points.len() - 1);
        match (shape.definition, shape.periodic) {
            (SplineDefinition::Poles, false) => Ok(Self::clamped(degree, points.to_vec())),
            (SplineDefinition::Poles, true) => Ok(Self::periodic(degree, points.to_vec())),
            (SplineDefinition::Through, false) => Self::interpolate_open(degree, points),
            (SplineDefinition::Through, true) => Self::interpolate_periodic(degree, points),
        }
    }

    fn clamped(degree: usize, poles: Vec<SplineVec<S>>) -> Self {
        let n = poles.len();
        let spans = n - degree;
        let mut knots = vec![S::constant(0.0); degree + 1];
        knots.extend((1..spans).map(|i| S::constant(i as f64 / spans as f64)));
        knots.extend(std::iter::repeat_n(S::constant(1.0), degree + 1));
        Self {
            degree,
            knots,
            distinct: n,
            poles,
            periodic: false,
            weights: None,
        }
    }

    fn periodic(degree: usize, mut poles: Vec<SplineVec<S>>) -> Self {
        let n = poles.len();
        poles.extend_from_within(0..degree);
        // n + degree poles and n + 2·degree + 1 uniform knots give exactly n
        // active spans in [degree, degree + n], one per original pole.
        let knots = (0..=(n + 2 * degree))
            .map(|i| S::constant(i as f64))
            .collect();
        Self {
            degree,
            knots,
            poles,
            distinct: n,
            periodic: true,
            weights: None,
        }
    }

    fn chord(a: SplineVec<S>, b: SplineVec<S>) -> S {
        let d = vec_sub(b, a);
        (d[0] * d[0] + d[1] * d[1]).sqrt()
    }

    fn interpolate_open(degree: usize, points: &[SplineVec<S>]) -> Result<Self, BSplineError> {
        let n = points.len();
        let mut params = Vec::with_capacity(n);
        params.push(S::constant(0.0));
        for k in 1..n {
            let chord = Self::chord(points[k - 1], points[k]);
            if chord.value() <= COINCIDENT_EPS {
                return Err(BSplineError::CoincidentThroughPoints {
                    first: k - 1,
                    second: k,
                });
            }
            params.push(params[k - 1] + chord);
        }
        let total = params[n - 1];
        for u in &mut params {
            *u = *u / total;
        }
        params[n - 1] = S::constant(1.0);
        let mut knots = vec![S::constant(0.0); degree + 1];
        for j in 1..(n - degree) {
            let sum = params[j..j + degree]
                .iter()
                .fold(S::constant(0.0), |acc, &u| acc + u);
            knots.push(sum / S::constant(degree as f64));
        }
        knots.extend(std::iter::repeat_n(S::constant(1.0), degree + 1));
        let shell = Self {
            degree,
            knots,
            poles: vec![vec_zero(); n],
            distinct: n,
            periodic: false,
            weights: None,
        };
        let rows = params.iter().map(|&u| shell.basis_row(u, n)).collect();
        let poles = solve_dense(rows, points.to_vec());
        Ok(Self { poles, ..shell })
    }

    fn interpolate_periodic(degree: usize, points: &[SplineVec<S>]) -> Result<Self, BSplineError> {
        let n = points.len();
        for k in 0..n {
            let next = (k + 1) % n;
            if Self::chord(points[k], points[next]).value() <= COINCIDENT_EPS {
                return Err(BSplineError::CoincidentThroughPoints {
                    first: k.min(next),
                    second: k.max(next),
                });
            }
        }
        let shell = Self::periodic(degree, vec![vec_zero(); n]);
        let offset = if degree.is_multiple_of(2) { 0.5 } else { 0.0 };
        let rows = (0..n)
            .map(|k| shell.basis_row(S::constant((degree + k) as f64 + offset), n))
            .collect();
        let distinct = solve_dense(rows, points.to_vec());
        Ok(Self::periodic(degree, distinct))
    }

    /// Interpolation row: weights of the n distinct poles at `t`.
    /// Accumulate a repeated periodic pole's weight at its original index.
    fn basis_row(&self, t: S, n: usize) -> Vec<S> {
        let span = self.span(t.value());
        let basis = self.basis_of_degree(span, t, self.degree);
        let mut row = vec![S::constant(0.0); n];
        for (j, &w) in basis.iter().take(self.degree + 1).enumerate() {
            let k = (span - self.degree + j) % n;
            row[k] = row[k] + w;
        }
        row
    }

    /// Effective degree, reduced when there are too few points.
    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn is_periodic(&self) -> bool {
        self.periodic
    }

    /// Parameter domain in knot values.
    fn domain_values(&self) -> (f64, f64) {
        (
            self.knots[self.degree].value(),
            self.knots[self.poles.len()].value(),
        )
    }

    /// Active span k: `knots[k] <= t < knots[k + 1]`; the right domain endpoint
    /// belongs to the last nonzero span.
    fn span(&self, t: f64) -> usize {
        let knot = |i: usize| self.knots[i].value();
        let last = self.poles.len() - 1;
        if t >= knot(last + 1) {
            let mut k = last;
            while knot(k) >= knot(k + 1) {
                k -= 1;
            }
            return k;
        }
        let mut k = self.degree;
        while k < last && t >= knot(k + 1) {
            k += 1;
        }
        k
    }

    /// Nonzero degree-p basis in `span` (Piegl & Tiller A2.2).
    /// At full degree this is `N[span − degree ..= span]`; at lower degree it
    /// is the derivative basis on trimmed knots ((3.8)). Knot and span index
    /// shifts cancel, so left/right formulas use the original knot vector.
    fn basis_of_degree(&self, span: usize, t: S, p: usize) -> [S; MAX_SPLINE_DEGREE as usize + 1] {
        let zero = S::constant(0.0);
        let mut n = [zero; MAX_SPLINE_DEGREE as usize + 1];
        let mut left = [zero; MAX_SPLINE_DEGREE as usize + 1];
        let mut right = [zero; MAX_SPLINE_DEGREE as usize + 1];
        n[0] = S::constant(1.0);
        for j in 1..=p {
            left[j] = t - self.knots[span + 1 - j];
            right[j] = self.knots[span + j] - t;
            let mut saved = zero;
            for r in 0..j {
                let temp = n[r] / (right[r + 1] + left[j - r]);
                n[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            n[j] = saved;
        }
        n
    }

    /// Point at an active-domain `t`, without normalization.
    fn eval_in_domain(&self, t: S) -> SplineVec<S> {
        let span = self.span(t.value());
        let basis = self.basis_of_degree(span, t, self.degree);
        if self.weights.is_some() {
            return self.rational_derivative(t, 0);
        }
        (0..=self.degree).fold(vec_zero(), |acc, j| {
            vec_add(acc, vec_scale(self.poles[span - self.degree + j], basis[j]))
        })
    }

    /// Order-k derivative at an active-domain `t`, without building another curve.
    /// Derivative poles in span s (Piegl & Tiller (3.8)):
    /// `P⁽ᵏ⁾ᵢ = (p − k + 1)·(P⁽ᵏ⁻¹⁾ᵢ₊₁ − P⁽ᵏ⁻¹⁾ᵢ) / (u[i+p+1] − u[i+k])`, `i = s − p ..= s − k`,
    /// and `C⁽ᵏ⁾(t) = Σ N[i, p−k](t)·P⁽ᵏ⁾ᵢ`. Repeated knots yield zero poles
    /// for zero denominators, since their basis functions vanish on this span.
    fn derivative_in_domain(&self, t: S, k: usize) -> SplineVec<S> {
        if self.weights.is_some() {
            return self.rational_derivative(t, k);
        }
        let p = self.degree;
        if k > p {
            return vec_zero();
        }
        let s = self.span(t.value());
        let mut q = [vec_zero(); MAX_SPLINE_DEGREE as usize + 1];
        q[..=p].copy_from_slice(&self.poles[s - p..=s]);
        for order in 1..=k {
            for j in 0..=(p - order) {
                let i = s - p + j;
                let du = self.knots[i + p + 1] - self.knots[i + order];
                q[j] = if du.value() > 0.0 {
                    vec_scale(
                        vec_sub(q[j + 1], q[j]),
                        S::constant((p - order + 1) as f64) / du,
                    )
                } else {
                    vec_zero()
                };
            }
        }
        let basis = self.basis_of_degree(s, t, p - k);
        (0..=(p - k)).fold(vec_zero(), |acc, j| vec_add(acc, vec_scale(q[j], basis[j])))
    }

    /// Homogeneous derivatives followed by the quotient recurrence
    /// C^(k) = (A^(k) − Σ binom(k,i) w^(i) C^(k−i)) / w.
    /// It also handles orders above the polynomial degree (a rational quadratic
    /// has nonzero higher derivatives), and keeps pole/parameter dual derivatives.
    fn rational_derivative(&self, t: S, k: usize) -> SplineVec<S> {
        let weights = self.weights.as_ref().unwrap();
        let p = self.degree;
        let s = self.span(t.value());
        let zero = S::constant(0.0);
        // Work relative to a nearby pole: homogeneous quotient derivatives
        // otherwise subtract the sketch's absolute translation at every order.
        let origin = self.poles[s - p];
        let mut q: Vec<[S; 3]> = (s - p..=s)
            .map(|i| {
                [
                    (self.poles[i][0] - origin[0]) * weights[i],
                    (self.poles[i][1] - origin[1]) * weights[i],
                    weights[i],
                ]
            })
            .collect();
        let mut homogeneous = vec![[zero; 3]; k + 1];
        for (order, h) in homogeneous.iter_mut().enumerate().take(p.min(k) + 1) {
            if order > 0 {
                for j in 0..=(p - order) {
                    let i = s - p + j;
                    let du = self.knots[i + p + 1] - self.knots[i + order];
                    let factor = if du.value() > 0.0 {
                        S::constant((p - order + 1) as f64) / du
                    } else {
                        zero
                    };
                    let next = q[j + 1];
                    for (value, next) in q[j].iter_mut().zip(next) {
                        *value = (next - *value) * factor;
                    }
                }
            }
            let basis = self.basis_of_degree(s, t, p - order);
            for j in 0..=(p - order) {
                for axis in 0..3 {
                    h[axis] = h[axis] + q[j][axis] * basis[j];
                }
            }
        }
        let w = homogeneous[0][2];
        let mut derivatives = Vec::with_capacity(k + 1);
        for order in 0..=k {
            let mut v = [homogeneous[order][0], homogeneous[order][1]];
            let mut binomial = 1.0;
            for i in 1..=order {
                binomial *= (order + 1 - i) as f64 / i as f64;
                v = vec_sub(
                    v,
                    vec_scale(
                        derivatives[order - i],
                        S::constant(binomial) * homogeneous[i][2],
                    ),
                );
            }
            derivatives.push(vec_scale(v, S::constant(1.0) / w));
        }
        if k == 0 {
            vec_add(derivatives[0], origin)
        } else {
            derivatives[k]
        }
    }

    /// Clamp an open parameter to a constant endpoint; reduce a periodic
    /// parameter by a constant shift, preserving its derivative.
    fn normalized(&self, t: S) -> S {
        let (a, b) = self.domain_values();
        let v = t.value();
        if self.periodic {
            t.with_value(a + (v - a).rem_euclid(b - a))
        } else if v < a {
            S::constant(a)
        } else if v > b {
            S::constant(b)
        } else {
            t
        }
    }

    /// Point at `t`, normalized as in [`BSplineCurve::point`].
    pub fn point_at(&self, t: S) -> SplineVec<S> {
        self.eval_in_domain(self.normalized(t))
    }

    /// Order-k parameter derivative at `t`, normalized as for a point.
    pub fn derivative_at(&self, t: S, k: usize) -> SplineVec<S> {
        self.derivative_in_domain(self.normalized(t), k)
    }
}

impl BSplineCurve {
    /// Parameter domain.
    pub fn domain(&self) -> (SplineParam, SplineParam) {
        let (a, b) = self.domain_values();
        (SplineParam(a), SplineParam(b))
    }

    /// Distinct poles in polygon order; a periodic polygon's closing arm is implicit.
    pub fn control_polygon(&self) -> Vec<WorldPoint> {
        self.poles[..self.distinct]
            .iter()
            .map(|p| WorldPoint::from_dvec2(DVec2::from_array(*p)))
            .collect()
    }

    fn eval(&self, t: f64) -> DVec2 {
        DVec2::from_array(self.eval_in_domain(t))
    }

    /// Curve point: clamp an open parameter to [`domain`](Self::domain), or
    /// wrap periodically so `t` and `t + period` denote the same point.
    pub fn point(&self, t: SplineParam) -> WorldPoint {
        WorldPoint::from_dvec2(DVec2::from_array(self.point_at(t.0)))
    }

    /// First parameter derivative: travel direction and speed.
    pub fn derivative(&self, t: SplineParam) -> WorldVec {
        WorldVec::from_dvec2(DVec2::from_array(self.derivative_at(t.0, 1)))
    }

    /// Second parameter derivative, zero inside spans of a degree-one polyline.
    pub fn second_derivative(&self, t: SplineParam) -> WorldVec {
        WorldVec::from_dvec2(DVec2::from_array(self.derivative_at(t.0, 2)))
    }

    /// Period of a periodic curve; an open curve has none.
    pub fn period(&self) -> Option<f64> {
        let (a, b) = self.domain();
        self.periodic.then_some(b.0 - a.0)
    }

    /// Normalize a parameter by clamping or periodic wrapping.
    fn normalize(&self, t: f64) -> f64 {
        self.normalized(t)
    }

    /// Curve start, coincident with its end when periodic.
    pub fn start(&self) -> WorldPoint {
        self.point(self.domain().0)
    }

    pub fn end(&self) -> WorldPoint {
        self.point(self.domain().1)
    }

    /// Polyline with `per_span` segments per nonzero span, including both ends.
    /// A periodic polyline ends at its first point.
    pub fn sample(&self, per_span: usize) -> Vec<WorldPoint> {
        assert!(
            per_span > 0,
            "BSplineCurve::sample: per_span must be positive"
        );
        let (a, b) = self.domain();
        let mut out = vec![self.point(a)];
        for k in self.degree..self.poles.len() {
            let (u0, u1) = (self.knots[k], self.knots[k + 1]);
            if u1 <= u0 || u0 >= b.0 {
                continue;
            }
            for i in 1..=per_span {
                let t = u0 + (u1 - u0) * i as f64 / per_span as f64;
                out.push(WorldPoint::from_dvec2(self.eval(t)));
            }
        }
        if self.periodic {
            let first = out[0];
            *out.last_mut().unwrap() = first;
        }
        out
    }

    /// Bounds candidates: endpoints and axis-parallel tangencies (`x' = 0` or
    /// `y' = 0`). A polynomial span derivative has degree `p − 1 <= 4`;
    /// 32 probes separate roots and bisection refines sign changes to storage precision.
    pub fn extremes(&self) -> Vec<WorldPoint> {
        let (a, b) = self.domain();
        let mut out = vec![self.point(a), self.point(b)];
        for axis in 0..2 {
            out.extend(
                self.axis_turns(axis, [a, b])
                    .into_iter()
                    .map(|t| self.point(t)),
            );
        }
        out
    }

    /// Nearest point to `target`: parameter, point and distance.
    ///
    /// Probe 32 points per span, then use golden sections around the best probe.
    /// A distance function may have multiple minima, but 1/32 of a span is
    /// smaller than a visible sketch bend.
    pub fn nearest(&self, target: WorldPoint) -> (SplineParam, WorldPoint, WorldLength) {
        const COARSE: usize = 32;
        let (a, b) = self.domain();
        let spans = self.poles.len() - self.degree;
        let steps = COARSE * spans;
        let at = |i: usize| a.0 + (b.0 - a.0) * i as f64 / steps as f64;
        let dist = |t: f64| self.eval(t).distance_squared(target.0);
        let best = (0..=steps)
            .min_by(|&i, &j| dist(at(i)).total_cmp(&dist(at(j))))
            .unwrap();
        let (lo, hi) = (at(best.saturating_sub(1)), at((best + 1).min(steps)));
        let t = crate::primitives::golden_minimum(lo, hi, 60, dist);
        let p = WorldPoint::from_dvec2(self.eval(t));
        (SplineParam(t), p, p.distance(target))
    }
}

/// Solid-kernel segment: nonperiodic knots, poles and a parameter interval.
/// Unroll periodic curves over two turns so a seam-crossing piece is one
/// interval. Reversal traverses the same carrier from its end to its start.
#[derive(Debug, Clone, PartialEq)]
pub struct BSplinePiece {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub poles: Vec<WorldPoint>,
    pub weights: Vec<f64>,
    /// `range[0] < range[1]`, within the active knot domain.
    pub range: [SplineParam; 2],
}

impl BSplinePiece {
    /// Segment point at its own knot parameter `t`.
    pub fn point(&self, t: SplineParam) -> WorldPoint {
        let curve = BSplineCurve {
            degree: self.degree,
            knots: self.knots.clone(),
            poles: self.poles.iter().map(|p| p.0.to_array()).collect(),
            distinct: self.poles.len(),
            periodic: false,
            weights: Some(self.weights.clone()),
        };
        curve.point(t)
    }
}

/// Cumulative window arc length ([`BSplineCurve::arc_table`]). The table does
/// not own its curve and must be evaluated against the curve that built it.
#[derive(Debug, Clone, PartialEq)]
pub struct ArcLengthTable {
    /// Increasing table parameters from the normalized window start to its end.
    params: Vec<f64>,
    /// Arc length from the window start to each parameter.
    lengths: Vec<f64>,
}

/// Curvature comb tooth: curve point, unit normal and signed curvature.
///
/// Tip = `base + normal · curvature · scale`. The right travel normal `(ty, −tx)`
/// makes teeth point away from the curvature center. The renderer owns the scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombTooth {
    pub base: WorldPoint,
    pub normal: WorldVec,
    /// `(x′ × x″) / |x′|³`, in 1/mm; positive on a left turn.
    pub curvature: f64,
}

#[path = "bspline_measure.rs"]
mod measure;

#[cfg(test)]
#[path = "bspline_degree.rs"]
mod degree_edit;
#[cfg(test)]
#[path = "bspline_join.rs"]
mod join;
#[cfg(test)]
#[path = "bspline_join_tests.rs"]
mod join_tests;

#[cfg(test)]
#[path = "bspline_knots.rs"]
mod knot_edit;

#[cfg(test)]
#[path = "bspline_degree_tests.rs"]
mod degree_tests;
#[cfg(test)]
#[path = "bspline_knots_tests.rs"]
mod knot_tests;

#[path = "bspline_foreign.rs"]
pub mod foreign;

#[cfg(test)]
#[path = "bspline_foreign_tests.rs"]
mod foreign_tests;

#[cfg(test)]
#[path = "bspline_measure_tests.rs"]
mod measure_tests;

/// Solve square `A·X = B` with two coordinate columns by pivoted elimination.
/// Averaged open knots meet the Schoenberg–Whitney condition; the periodic
/// matrix is strictly diagonally dominant. Singularity violates the invariant.
///
/// Pivot by scalar value. Do not skip a zero-valued row multiplier: a dual
/// number with zero value can still have a nonzero derivative.
fn solve_dense<S: SplineReal>(mut a: Vec<Vec<S>>, mut b: Vec<SplineVec<S>>) -> Vec<SplineVec<S>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| a[i][col].value().abs().total_cmp(&a[j][col].value().abs()))
            .unwrap();
        assert!(
            a[pivot][col].value().abs() > 1e-12,
            "B-spline interpolation matrix is singular at column {col}"
        );
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            let (previous, current) = a.split_at_mut(row);
            for (value, &pivot) in current[0][col..].iter_mut().zip(&previous[col][col..]) {
                *value = *value - f * pivot;
            }
            b[row] = vec_sub(b[row], vec_scale(b[col], f));
        }
    }
    let mut x = vec![vec_zero(); n];
    for row in (0..n).rev() {
        let mut acc = b[row];
        for k in row + 1..n {
            acc = vec_sub(acc, vec_scale(x[k], a[row][k]));
        }
        x[row] = [acc[0] / a[row][row], acc[1] / a[row][row]];
    }
    x
}
