//! Spline arc length and its inverse, window projection, tangent turning points
//! and segment representation for solid kernels.
//!
//! An open window lies in the active knot domain. A periodic window may cross
//! the seam, with t1 at most one period beyond t0; evaluation wraps parameters.
//!
//! Split integration and search at knots to keep every piece smooth.

use glam::DVec3;

use super::{ArcLengthTable, BSplineCurve, BSplinePiece, CombTooth, SplineParam};
use crate::world::{WorldLength, WorldPoint, WorldVec};

/// Eight-point Gauss–Legendre nodes and weights on `[-1, 1]`, exact through
/// degree 15. Speed is a square root, not a polynomial, and nearly vanishing
/// speed can create a sharp bend. Adaptively halve intervals until the two
/// halves agree with the whole (a single rule can miss 0.1 mm over 127 mm).
const GAUSS_X: [f64; 4] = [
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const GAUSS_W: [f64; 4] = [
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];
/// Halving limit: `2⁻⁴⁰` of a span, below parameter storage resolution.
const GAUSS_DEPTH: u32 = 40;

/// Arc-length table steps per smooth window piece.
const TABLE_STEPS: usize = 8;

impl ArcLengthTable {
    /// Normalized window, starting in the first turn when periodic.
    pub fn window(&self) -> [SplineParam; 2] {
        [
            SplineParam(self.params[0]),
            SplineParam(*self.params.last().unwrap()),
        ]
    }

    /// Total window length.
    pub fn length(&self) -> WorldLength {
        WorldLength::new(*self.lengths.last().unwrap())
    }

    /// Table step containing window parameter `t`.
    fn step_of_param(&self, t: f64) -> usize {
        self.params
            .partition_point(|&p| p <= t)
            .clamp(1, self.params.len() - 1)
            - 1
    }
}

/// Probes per span for turning points and projection. A polynomial derivative
/// component has degree <= 4; 32 probes separate its at most four roots.
const PROBES: usize = 32;

impl BSplineCurve {
    /// Integrate `f >= 0` over `lo <= hi` by adaptive Gauss–Legendre quadrature
    /// at about 1e-13 relative precision.
    pub fn gauss(lo: f64, hi: f64, f: impl Fn(f64) -> f64) -> f64 {
        fn rule(lo: f64, hi: f64, f: &impl Fn(f64) -> f64) -> f64 {
            let (mid, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
            GAUSS_X
                .iter()
                .zip(GAUSS_W)
                .map(|(&x, w)| w * (f(mid - half * x) + f(mid + half * x)))
                .sum::<f64>()
                * half
        }
        fn adapt(
            lo: f64,
            hi: f64,
            whole: f64,
            tol: f64,
            depth: u32,
            f: &impl Fn(f64) -> f64,
        ) -> f64 {
            let mid = (lo + hi) * 0.5;
            let (left, right) = (rule(lo, mid, f), rule(mid, hi, f));
            if depth == 0 || (left + right - whole).abs() <= tol {
                return left + right;
            }
            adapt(lo, mid, left, tol * 0.5, depth - 1, f)
                + adapt(mid, hi, right, tol * 0.5, depth - 1, f)
        }
        let whole = rule(lo, hi, &f);
        adapt(lo, hi, whole, 1e-13 * (1.0 + whole.abs()), GAUSS_DEPTH, &f)
    }

    /// Normalize a periodic t0 into `[a, a + period)` and shift t1 equally.
    /// Reversed bounds or a window longer than one period are caller errors.
    fn window(&self, window: [SplineParam; 2]) -> (f64, f64) {
        let [t0, t1] = window.map(|t| t.0);
        assert!(
            t0 <= t1 && t0.is_finite() && t1.is_finite(),
            "spline window [{t0}, {t1}] is reversed or not finite"
        );
        let (a, b) = self.domain();
        match self.period() {
            Some(period) => {
                assert!(
                    t1 - t0 <= period * (1.0 + 1e-12),
                    "spline window [{t0}, {t1}] is longer than the period {period}"
                );
                let shift = (t0 - a.0).div_euclid(period) * period;
                (t0 - shift, t1 - shift)
            }
            None => {
                assert!(
                    t0 >= a.0 && t1 <= b.0,
                    "spline window [{t0}, {t1}] leaves the domain [{}, {}]",
                    a.0,
                    b.0
                );
                (t0, t1)
            }
        }
    }

    /// Increasing smooth-piece boundaries: window ends and intervening knots.
    /// Periodic knots repeat across the seam.
    pub fn breaks(&self, window: [SplineParam; 2]) -> Vec<f64> {
        let (t0, t1) = self.window(window);
        let mut knots: Vec<f64> = self.knots[self.degree..=self.poles.len()].to_vec();
        knots.dedup();
        let mut out = vec![t0];
        let shifts: &[f64] = match self.period() {
            Some(period) => &[0.0, period],
            None => &[0.0],
        };
        for &shift in shifts {
            out.extend(
                knots
                    .iter()
                    .map(|k| k + shift)
                    .filter(|&k| k > t0 && k < t1),
            );
        }
        out.push(t1);
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    /// Arc length between window parameters `from <= to`.
    pub fn arc_length(&self, from: SplineParam, to: SplineParam) -> WorldLength {
        let breaks = self.breaks([from, to]);
        let speed = |t: f64| self.derivative(SplineParam(t)).0.length();
        WorldLength::new(
            breaks
                .windows(2)
                .map(|w| Self::gauss(w[0], w[1], speed))
                .sum(),
        )
    }

    fn speed(&self, t: f64) -> f64 {
        self.derivative(SplineParam(t)).0.length()
    }

    /// Cumulative length at knots and [`TABLE_STEPS`] points per smooth piece.
    /// Queries then integrate only inside one table step.
    pub fn arc_table(&self, window: [SplineParam; 2]) -> ArcLengthTable {
        let breaks = self.breaks(window);
        let mut params = vec![breaks[0]];
        for w in breaks.windows(2) {
            params.extend(
                (1..=TABLE_STEPS).map(|i| w[0] + (w[1] - w[0]) * i as f64 / TABLE_STEPS as f64),
            );
        }
        let mut lengths = vec![0.0];
        for w in params.windows(2) {
            let last = *lengths.last().unwrap();
            lengths.push(last + Self::gauss(w[0], w[1], |t| self.speed(t)));
        }
        ArcLengthTable { params, lengths }
    }

    /// Arc length from the table window's start to its parameter `t`.
    pub fn length_at(&self, table: &ArcLengthTable, t: SplineParam) -> WorldLength {
        let k = table.step_of_param(t.0);
        WorldLength::new(table.lengths[k] + Self::gauss(table.params[k], t.0, |u| self.speed(u)))
    }

    /// Inverse of [`Self::length_at`]. A length rounded beyond the window gives
    /// its endpoint.
    pub fn param_at_length(&self, table: &ArcLengthTable, length: WorldLength) -> SplineParam {
        assert!(
            length.0.is_finite() && length.0 >= 0.0,
            "param_at_length: length {} is negative or not finite",
            length.0
        );
        let last = table.lengths.len() - 1;
        if length.0 >= table.lengths[last] {
            return SplineParam(table.params[last]);
        }
        let k = table.lengths.partition_point(|&l| l <= length.0) - 1;
        let (lo, hi) = (table.params[k], table.params[k + 1]);
        let rest = length.0 - table.lengths[k];
        let piece = table.lengths[k + 1] - table.lengths[k];
        // Bracket Newton by bisection inside a table step: length is monotone
        // in t and its derivative is speed |C'(t)|.
        let (mut a, mut b) = (lo, hi);
        let mut t = lo + (hi - lo) * if piece > 0.0 { rest / piece } else { 0.0 };
        for _ in 0..60 {
            let f = Self::gauss(lo, t, |u| self.speed(u)) - rest;
            if f.abs() <= 1e-14 * (1.0 + table.lengths[last]) {
                break;
            }
            if f > 0.0 {
                b = t;
            } else {
                a = t;
            }
            let v = self.speed(t);
            let newton = t - f / v;
            t = if v > 0.0 && newton > a && newton < b {
                newton
            } else {
                (a + b) * 0.5
            };
            if b - a <= f64::EPSILON * (1.0 + b.abs()) {
                break;
            }
        }
        SplineParam(t)
    }

    /// Nearest window parameter, point and distance. Probe [`PROBES`] points
    /// per span, then refine around the best by golden-section search.
    pub fn nearest_in(
        &self,
        target: WorldPoint,
        window: [SplineParam; 2],
    ) -> (SplineParam, WorldPoint, WorldLength) {
        let (t0, t1) = self.window(window);
        let breaks = self.breaks(window);
        let mut probes = Vec::new();
        for w in breaks.windows(2) {
            probes.extend((0..PROBES).map(|i| w[0] + (w[1] - w[0]) * i as f64 / PROBES as f64));
        }
        probes.push(t1);
        let dist = |t: f64| self.eval(self.normalize(t)).distance_squared(target.0);
        let best = (0..probes.len())
            .min_by(|&i, &j| dist(probes[i]).total_cmp(&dist(probes[j])))
            .unwrap();
        let (lo, hi) = (
            probes[best.saturating_sub(1)],
            probes[(best + 1).min(probes.len() - 1)],
        );
        let t = crate::primitives::golden_minimum(lo, hi, 80, dist).clamp(t0, t1);
        let p = WorldPoint::from_dvec2(self.eval(self.normalize(t)));
        (SplineParam(t), p, p.distance(target))
    }

    /// Increasing parameters where tangent component `axis` (0 = x, 1 = y)
    /// changes sign or is zero at a probe. That coordinate is monotone between them.
    pub fn axis_turns(&self, axis: usize, window: [SplineParam; 2]) -> Vec<SplineParam> {
        assert!(axis < 2, "axis_turns: axis {axis} of a planar curve");
        let d = |t: f64| self.derivative(SplineParam(t)).0[axis];
        let mut out = Vec::new();
        for w in self.breaks(window).windows(2) {
            let (u0, u1) = (w[0], w[1]);
            let at = |i: usize| u0 + (u1 - u0) * i as f64 / PROBES as f64;
            for i in 0..PROBES {
                let (mut lo, mut hi) = (at(i), at(i + 1));
                let (dlo, dhi) = (d(lo), d(hi));
                if dlo == 0.0 {
                    out.push(SplineParam(lo));
                    continue;
                }
                // A right-probe zero is the next interval's left probe;
                // record it there to count a span-boundary turn once.
                if dhi == 0.0 || dlo.signum() == dhi.signum() {
                    continue;
                }
                for _ in 0..60 {
                    let mid = (lo + hi) * 0.5;
                    if d(mid).signum() == dlo.signum() {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                out.push(SplineParam((lo + hi) * 0.5));
            }
        }
        out
    }

    /// Solid-kernel window piece ([`BSplinePiece`]), optionally reversed.
    ///
    /// Clamp t0/t1 to multiplicity p + 1 so the first and last poles are the
    /// endpoints. Contour stitching can then place shared vertices explicitly.
    pub fn piece(&self, window: [SplineParam; 2], reversed: bool) -> BSplinePiece {
        let (t0, t1) = self.window(window);
        assert!(
            t0 < t1,
            "a spline piece needs a non-empty window, got [{t0}, {t1}]"
        );
        let p = self.degree;
        let lifted = |i: usize| {
            let w = self.weights.as_ref().map_or(1.0, |weights| weights[i]);
            DVec3::new(self.poles[i][0] * w, self.poles[i][1] * w, w)
        };
        let (mut knots, mut poles): (Vec<f64>, Vec<DVec3>) =
            if self.periodic && self.knots[0] == self.knots[p] {
                let period = self.period().unwrap();
                let n = self.poles.len();
                let mut knots = self.knots[..n].to_vec();
                knots.extend(std::iter::repeat_n(self.knots[n], p));
                knots.extend(self.knots[p + 1..].iter().map(|u| u + period));
                let poles = (0..n).chain(1..n).map(lifted).collect();
                (knots, poles)
            } else if self.periodic {
                // Uniform periodic knots use active domain [p, p + n]. Two turns
                // plus p repeated poles extend it to [p, p + 2n]; normalization
                // keeps the window start in the first turn.
                let n = self.distinct;
                let poles: Vec<DVec3> = (0..2 * n + p).map(|i| lifted(i % n)).collect();
                let knots = (0..poles.len() + p + 1).map(|i| i as f64).collect();
                (knots, poles)
            } else {
                (
                    self.knots.clone(),
                    (0..self.poles.len()).map(lifted).collect(),
                )
            };
        for u in [t0, t1] {
            while multiplicity(&knots, u) < p {
                insert_knot(&mut knots, &mut poles, p, u);
            }
        }
        // Clamped [t0, t1]: poles from the last t0 index minus degree to the
        // first t1 index.
        let first = |u: f64| knots.iter().position(|&k| k == u).unwrap();
        let (s0, s1) = (first(t0), first(t1));
        let m0 = multiplicity(&knots, t0);
        let poles: Vec<DVec3> = poles[s0 + m0 - 1 - p..s1].to_vec();
        let mut clamped = vec![t0; p + 1];
        clamped.extend_from_slice(&knots[s0 + m0..s1]);
        clamped.extend(std::iter::repeat_n(t1, p + 1));
        let knots = clamped;
        debug_assert_eq!(knots.len(), poles.len() + p + 1);
        if !reversed {
            return BSplinePiece {
                degree: p,
                knots,
                weights: poles.iter().map(|p| p.z).collect(),
                poles: poles
                    .into_iter()
                    .map(|p| WorldPoint::from_dvec2(p.truncate() / p.z))
                    .collect(),
                range: [SplineParam(t0), SplineParam(t1)],
            };
        }
        // Reverse the same curve by u -> s − u, with s the endpoint knot sum;
        // reflect and reverse knots, and reverse poles.
        let s = knots[0] + knots[knots.len() - 1];
        BSplinePiece {
            degree: p,
            knots: knots.iter().rev().map(|k| s - k).collect(),
            weights: poles.iter().rev().map(|p| p.z).collect(),
            poles: poles
                .into_iter()
                .rev()
                .map(|p| WorldPoint::from_dvec2(p.truncate() / p.z))
                .collect(),
            range: [SplineParam(s - t1), SplineParam(s - t0)],
        }
    }
}

fn multiplicity(knots: &[f64], u: f64) -> usize {
    knots.iter().filter(|&&k| k == u).count()
}

/// Insert u once without changing the curve (Boehm; Piegl & Tiller A5.1):
/// `Qᵢ = αᵢ·Pᵢ + (1 − αᵢ)·Pᵢ₋₁`, `αᵢ = (u − uᵢ) / (uᵢ₊ₚ − uᵢ)` on
/// `k − p + 1 ..= k − s`, where `u ∈ [u_k, u_{k+1})` and s is u's multiplicity.
pub(super) fn insert_knot(knots: &mut Vec<f64>, poles: &mut Vec<DVec3>, p: usize, u: f64) {
    let n = poles.len();
    let k = knots.partition_point(|&x| x <= u) - 1;
    let k = k.clamp(p, n - 1);
    let s = multiplicity(knots, u);
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        out.push(if i + p <= k {
            poles[i]
        } else if i + s <= k {
            let a = (u - knots[i]) / (knots[i + p] - knots[i]);
            poles[i] * a + poles[i - 1] * (1.0 - a)
        } else {
            poles[i - 1]
        });
    }
    *poles = out;
    knots.insert(k + 1, u);
}

impl BSplineCurve {
    /// Signed curvature at t, or `None` for degenerate speed with no tangent.
    pub fn curvature(&self, t: SplineParam) -> Option<f64> {
        let d1 = self.derivative(t).0;
        let speed = d1.length();
        if speed < WorldLength::DEGENERATE.0 {
            return None;
        }
        Some(d1.perp_dot(self.second_derivative(t).0) / speed.powi(3))
    }

    /// `per_piece` comb teeth per smooth window piece plus its start.
    /// Skip points without tangents.
    pub fn comb(&self, window: [SplineParam; 2], per_piece: usize) -> Vec<CombTooth> {
        assert!(
            per_piece > 0,
            "BSplineCurve::comb: per_piece must be positive"
        );
        let breaks = self.breaks(window);
        let mut params = vec![breaks[0]];
        for w in breaks.windows(2) {
            params.extend(
                (1..=per_piece).map(|i| w[0] + (w[1] - w[0]) * i as f64 / per_piece as f64),
            );
        }
        params
            .into_iter()
            .filter_map(|t| {
                let t = SplineParam(t);
                let curvature = self.curvature(t)?;
                let d1 = self.derivative(t).0;
                let unit = d1 / d1.length();
                Some(CombTooth {
                    base: self.point(t),
                    normal: WorldVec::from_dvec2(glam::DVec2::new(unit.y, -unit.x)),
                    curvature,
                })
            })
            .collect()
    }
}
