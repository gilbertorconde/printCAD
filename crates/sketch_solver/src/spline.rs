//! Pure double-precision B-spline basis, evaluation, interpolation and
//! least-squares fitting. Inputs and outputs contain no application types.

/// The basis functions of one spline: which control points weigh in at a
/// parameter, and how much.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Basis {
    degree: usize,
    /// The full knot vector: `count + degree + 1` knots over the control
    /// points as evaluated (a periodic ring repeats its first `degree`).
    knots: Vec<f64>,
    /// Control points as the spline names them.
    points: usize,
    periodic: bool,
    /// One per control point for a rational spline; empty when every
    /// weight is one.
    weights: Vec<f64>,
}

impl Basis {
    /// The rational basis with `weights`, one per control point; any other
    /// count, or all of them one, leaves it plain.
    pub fn with_weights(mut self, weights: &[f64]) -> Self {
        self.weights = if weights.len() == self.points && weights.iter().any(|w| *w != 1.0) {
            weights.to_vec()
        } else {
            Vec::new()
        };
        self
    }

    /// The weights of a rational basis; empty for a plain one.
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    pub fn is_periodic(&self) -> bool {
        self.periodic
    }

    /// The basis of a spline of `degree` over `points` control points:
    /// over `knots` when they fit, else clamped uniform; a periodic one is
    /// a uniform ring whatever `knots` holds.
    pub fn new(degree: u32, points: usize, knots: &[f64], periodic: bool) -> Option<Self> {
        if points < 2 || (periodic && points < 3) {
            return None;
        }
        let degree = (degree.max(1) as usize).min(points - 1);
        let knots = if periodic {
            (0..points + 2 * degree + 1).map(|i| i as f64).collect()
        } else if knots.len() == points + degree + 1 && knots.windows(2).all(|w| w[0] <= w[1]) {
            knots.to_vec()
        } else {
            clamped_uniform_knots(degree, points)
        };
        let basis = Self {
            degree,
            knots,
            points,
            periodic,
            weights: Vec::new(),
        };
        let (t0, t1) = basis.domain();
        (t1 > t0).then_some(basis)
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    /// The knot vector as the kernel takes it for an open spline.
    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    /// Control points the evaluation runs over: a periodic ring's wrapped.
    fn span_count(&self) -> usize {
        if self.periodic {
            self.points + self.degree
        } else {
            self.points
        }
    }

    /// The parameter range the curve covers.
    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.span_count()])
    }

    /// The control points that weigh in at `t` and their weights, which sum
    /// to one. A periodic ring's wrapped points fold back onto the ones
    /// they repeat.
    pub fn row(&self, t: f64) -> Vec<(usize, f64)> {
        let p = self.degree;
        let n = self.span_count();
        let (t0, t1) = self.domain();
        let t = t.clamp(t0, t1);
        let u = &self.knots;
        // The knot span holding t; the last one for the domain's end.
        let mut k = p;
        while k + 1 < n && u[k + 1] <= t {
            k += 1;
        }
        let mut values = vec![0.0; p + 1];
        let mut left = vec![0.0; p + 1];
        let mut right = vec![0.0; p + 1];
        values[0] = 1.0;
        for j in 1..=p {
            left[j] = t - u[k + 1 - j];
            right[j] = u[k + j] - t;
            let mut saved = 0.0;
            for r in 0..j {
                let denom = right[r + 1] + left[j - r];
                let temp = if denom.abs() < 1e-300 {
                    0.0
                } else {
                    values[r] / denom
                };
                values[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            values[j] = saved;
        }
        if !self.weights.is_empty() {
            // A rational spline weighs each control point's share and
            // scales them back to one in all.
            for (i, value) in values.iter_mut().enumerate() {
                *value *= self.weights[(k - p + i) % self.points];
            }
            let total: f64 = values.iter().sum();
            if total.abs() > 1e-300 {
                values.iter_mut().for_each(|v| *v /= total);
            }
        }
        let mut row: Vec<(usize, f64)> = Vec::with_capacity(p + 1);
        for (i, value) in values.into_iter().enumerate() {
            let index = (k - p + i) % self.points;
            match row.iter_mut().find(|(j, _)| *j == index) {
                Some(entry) => entry.1 += value,
                None => row.push((index, value)),
            }
        }
        row
    }

    /// The curve's point at `t` over `control`.
    pub fn eval(&self, control: &[[f64; 2]], t: f64) -> [f64; 2] {
        self.row(t).into_iter().fold([0.0, 0.0], |acc, (i, w)| {
            [acc[0] + w * control[i][0], acc[1] + w * control[i][1]]
        })
    }

    /// `samples + 1` points along the whole curve, both ends included.
    pub fn sample(&self, control: &[[f64; 2]], samples: usize) -> Vec<[f64; 2]> {
        let (t0, t1) = self.domain();
        let n = samples.max(1);
        (0..=n)
            .map(|i| {
                let t = t0 + (t1 - t0) * i as f64 / n as f64;
                self.eval(control, t)
            })
            .collect()
    }
}

/// A part of a spline: the control points and clamped knots over `[0, 1]`
/// of an open spline of the same degree that is exactly the curve from one
/// parameter to another.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub control: Vec<[f64; 2]>,
    pub knots: Vec<f64>,
    /// A rational spline's part keeps weights; empty for a plain one.
    pub weights: Vec<f64>,
}

impl Basis {
    /// The spline over `control` from `t0` to `t1` (`t0 < t1`) as an open
    /// spline of its own. A closed spline's part may run on past the end
    /// of its domain, up to a whole turn from `t0`. `None` when the
    /// parameters fall outside what the curve covers.
    pub fn piece(&self, control: &[[f64; 2]], t0: f64, t1: f64) -> Option<Piece> {
        let p = self.degree;
        let (d0, d1) = self.domain();
        // Worked on weighted points, so a rational spline's part is exact.
        let weight = |i: usize| self.weights.get(i).copied().unwrap_or(1.0);
        let lift = |i: usize| {
            let (c, w) = (control[i], weight(i));
            [c[0] * w, c[1] * w, w]
        };
        // A closed spline laid out twice round as an open one, so a part
        // can cross where it closes.
        let (mut knots, mut ctrl) = if self.periodic {
            let n = self.points;
            let knots: Vec<f64> = (0..2 * n + 2 * p + 1).map(|i| i as f64).collect();
            let ctrl: Vec<[f64; 3]> = (0..2 * n + p).map(|i| lift(i % n)).collect();
            (knots, ctrl)
        } else {
            (self.knots.clone(), (0..control.len()).map(lift).collect())
        };
        let end = if self.periodic { d1 + (d1 - d0) } else { d1 };
        let slack = 1e-9 * (d1 - d0);
        if t0 < d0 - slack || t1 > end + slack || t1 <= t0 {
            return None;
        }
        let (t0, t1) = (t0.max(d0), t1.min(end));
        for u in [t0, t1] {
            while knots.iter().filter(|k| (**k - u).abs() < 1e-12).count() < p {
                insert_knot(&mut knots, &mut ctrl, p, u);
            }
        }
        let last_a = knots.iter().rposition(|k| (*k - t0).abs() < 1e-12)?;
        let first_b = knots.iter().position(|k| (*k - t1).abs() < 1e-12)?;
        let m = (last_a + 1).checked_sub(p)?;
        if m == 0 || first_b <= m {
            return None;
        }
        let lifted = &ctrl[m - 1..first_b];
        let control: Vec<[f64; 2]> = lifted.iter().map(|q| [q[0] / q[2], q[1] / q[2]]).collect();
        let weights: Vec<f64> = if self.weights.is_empty() {
            Vec::new()
        } else {
            lifted.iter().map(|q| q[2]).collect()
        };
        let scale = |k: f64| (k - t0) / (t1 - t0);
        let mut out = vec![0.0; p + 1];
        out.extend(knots[last_a + 1..first_b].iter().map(|k| scale(*k)));
        out.extend(std::iter::repeat_n(1.0, p + 1));
        (out.len() == control.len() + p + 1).then_some(Piece {
            control,
            knots: out,
            weights,
        })
    }
}

/// One more knot at `u`, the curve unchanged (Boehm's insertion), over
/// weighted control points.
pub fn insert_knot(knots: &mut Vec<f64>, ctrl: &mut Vec<[f64; 3]>, p: usize, u: f64) {
    let n = ctrl.len();
    let mut k = p;
    while k + 1 < n && knots[k + 1] <= u {
        k += 1;
    }
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        if i + p <= k {
            out.push(ctrl[i]);
        } else if i > k {
            out.push(ctrl[i - 1]);
        } else {
            let span = knots[i + p] - knots[i];
            let a = if span.abs() < 1e-300 {
                0.0
            } else {
                (u - knots[i]) / span
            };
            let (q, r) = (ctrl[i - 1], ctrl[i]);
            out.push([
                q[0] + (r[0] - q[0]) * a,
                q[1] + (r[1] - q[1]) * a,
                q[2] + (r[2] - q[2]) * a,
            ]);
        }
    }
    knots.insert(k + 1, u);
    *ctrl = out;
}

/// A clamped knot vector with evenly spaced interior knots over `[0, 1]`.
pub fn clamped_uniform_knots(degree: usize, points: usize) -> Vec<f64> {
    let interior = points.saturating_sub(degree + 1);
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..=interior).map(|i| i as f64 / (interior + 1) as f64));
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

/// A clamped knot vector from interpolation parameters by averaging, which
/// keeps the interpolation system well conditioned.
fn averaged_knots(degree: usize, params: &[f64]) -> Vec<f64> {
    let n = params.len();
    let mut knots = vec![params[0]; degree + 1];
    for j in 1..n - degree {
        knots.push(params[j..j + degree].iter().sum::<f64>() / degree as f64);
    }
    knots.extend(std::iter::repeat_n(params[n - 1], degree + 1));
    knots
}

/// Parameters for points along a path: centripetal (the square root of
/// each step's length), over `[0, 1]`. `None` when two neighbours coincide.
fn centripetal_params(points: &[[f64; 2]]) -> Option<Vec<f64>> {
    let steps: Vec<f64> = points
        .windows(2)
        .map(|w| ((w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sqrt())
        .collect();
    if steps.iter().any(|d| *d < 1e-9) {
        return None;
    }
    let total: f64 = steps.iter().sum();
    let mut params = Vec::with_capacity(points.len());
    let mut at = 0.0;
    params.push(0.0);
    for d in steps {
        at += d;
        params.push(at / total);
    }
    *params.last_mut()? = 1.0;
    Some(params)
}

/// A spline through given points: its control points and knots, and the
/// parameter each point sits at.
#[derive(Debug, Clone, PartialEq)]
pub struct Interpolation {
    pub degree: u32,
    pub control: Vec<[f64; 2]>,
    /// Empty for a periodic spline, which is a uniform ring.
    pub knots: Vec<f64>,
    pub params: Vec<f64>,
}

/// The spline of `degree` through `points`, one control point per point:
/// open and clamped, so it starts and ends on the first and last, or a
/// periodic ring closing smoothly through all of them. `None` when there
/// are too few points or two in a row coincide.
pub fn interpolate(points: &[[f64; 2]], degree: u32, periodic: bool) -> Option<Interpolation> {
    let n = points.len();
    if n < 2 || (periodic && n < 3) {
        return None;
    }
    let (basis, params) = if periodic {
        let basis = Basis::new(degree, n, &[], true)?;
        let p = basis.degree();
        // At the knots an even degree's ring gives a singular system; half
        // way between them it does not.
        let shift = if p % 2 == 0 { 0.5 } else { 0.0 };
        let params: Vec<f64> = (0..n).map(|i| (p + i) as f64 + shift).collect();
        (basis, params)
    } else {
        let params = centripetal_params(points)?;
        let p = (degree.max(1) as usize).min(n - 1);
        let basis = Basis::new(p as u32, n, &averaged_knots(p, &params), false)?;
        (basis, params)
    };
    let rows: Vec<Vec<f64>> = params
        .iter()
        .map(|t| {
            let mut row = vec![0.0; n];
            for (i, w) in basis.row(*t) {
                row[i] += w;
            }
            row
        })
        .collect();
    let control = solve_points(rows, points.to_vec())?;
    Some(Interpolation {
        degree: basis.degree() as u32,
        control,
        knots: if periodic {
            Vec::new()
        } else {
            basis.knots().to_vec()
        },
        params,
    })
}

/// Solve `a · x = b` for a square `a` and a right-hand side of points, by
/// Gaussian elimination with partial pivoting.
pub fn solve_points(mut a: Vec<Vec<f64>>, mut b: Vec<[f64; 2]>) -> Option<Vec<[f64; 2]>> {
    let n = a.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            let pivot_row = a[col].clone();
            for (value, above) in a[row][col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= f * above;
            }
            b[row][0] -= f * b[col][0];
            b[row][1] -= f * b[col][1];
        }
    }
    let mut x = vec![[0.0; 2]; n];
    for row in (0..n).rev() {
        let mut acc = b[row];
        for k in row + 1..n {
            acc[0] -= a[row][k] * x[k][0];
            acc[1] -= a[row][k] * x[k][1];
        }
        x[row] = [acc[0] / a[row][row], acc[1] / a[row][row]];
    }
    x.iter()
        .all(|p| p[0].is_finite() && p[1].is_finite())
        .then_some(x)
}

/// A clamped cubic through `points` (samples along a chain of curves, in
/// order) within `tolerance` of every one: the fewest control points that
/// get there, the first and last on the chain's ends. The knots, and the
/// control points. `None` when no spline of up to `max_points` control
/// points gets that close.
pub fn fit_chain(
    points: &[[f64; 2]],
    tolerance: f64,
    max_points: usize,
) -> Option<(Vec<f64>, Vec<[f64; 2]>)> {
    const DEGREE: usize = 3;
    let m = points.len();
    if m < 2 {
        return None;
    }
    // Chord-length parameters: a chain of curves is sampled evenly along
    // its length, and a uniform step fits it evenly.
    let steps: Vec<f64> = points
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .collect();
    let total: f64 = steps.iter().sum();
    if total < 1e-9 {
        return None;
    }
    let mut params = vec![0.0];
    let mut at = 0.0;
    for d in &steps {
        at += d;
        params.push(at / total);
    }
    let fit = |count: usize| -> Option<(Vec<f64>, Vec<[f64; 2]>)> {
        let knots = clamped_uniform_knots(DEGREE, count);
        let basis = Basis::new(DEGREE as u32, count, &knots, false)?;
        let control = least_squares(&basis, points, &params, count)?;
        let worst = points
            .iter()
            .zip(&params)
            .map(|(q, t)| {
                let c = basis.eval(&control, *t);
                (c[0] - q[0]).hypot(c[1] - q[1])
            })
            .fold(0.0, f64::max);
        (worst <= tolerance).then_some((knots, control))
    };
    // More control points follow the chain more closely: grow until one
    // count is close enough, then find the fewest between the last that
    // was not and it.
    // Several samples to each control point, so the fit follows the chain
    // between its samples too rather than threading them.
    let cap = max_points.min(m / 3).max(DEGREE + 1);
    let mut below = DEGREE;
    let mut count = DEGREE + 1;
    let mut best = loop {
        if let Some(found) = fit(count) {
            break (count, found);
        }
        if count >= cap {
            return None;
        }
        below = count;
        count = (count * 3 / 2 + 1).min(cap);
    };
    while best.0 - below > 1 {
        let middle = (below + best.0) / 2;
        match fit(middle) {
            Some(found) => best = (middle, found),
            None => below = middle,
        }
    }
    Some(best.1)
}

/// The control points of `basis` nearest `points` at `params` in the least
/// squares, the first and last pinned to the first and last point.
fn least_squares(
    basis: &Basis,
    points: &[[f64; 2]],
    params: &[f64],
    count: usize,
) -> Option<Vec<[f64; 2]>> {
    let (first, last) = (points[0], points[points.len() - 1]);
    if count == 2 {
        return Some(vec![first, last]);
    }
    let inner = count - 2;
    let mut normal = vec![vec![0.0; inner]; inner];
    let mut rhs = vec![[0.0; 2]; inner];
    for (q, t) in points.iter().zip(params) {
        let row = basis.row(*t);
        // What the pinned ends already put there.
        let mut target = *q;
        for &(i, w) in &row {
            if i == 0 {
                target = [target[0] - w * first[0], target[1] - w * first[1]];
            } else if i == count - 1 {
                target = [target[0] - w * last[0], target[1] - w * last[1]];
            }
        }
        for &(i, wi) in row.iter().filter(|(i, _)| *i > 0 && *i < count - 1) {
            rhs[i - 1][0] += wi * target[0];
            rhs[i - 1][1] += wi * target[1];
            for &(j, wj) in row.iter().filter(|(j, _)| *j > 0 && *j < count - 1) {
                normal[i - 1][j - 1] += wi * wj;
            }
        }
    }
    let middle = solve_points(normal, rhs)?;
    let mut control = Vec::with_capacity(count);
    control.push(first);
    control.extend(middle);
    control.push(last);
    Some(control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
        (a[0] - b[0]).hypot(a[1] - b[1])
    }

    #[test]
    fn weights_sum_to_one_and_the_clamped_ends_are_the_end_points() {
        for degree in 1..=5 {
            let basis = Basis::new(degree, 7, &[], false).unwrap();
            let (t0, t1) = basis.domain();
            for i in 0..=20 {
                let t = t0 + (t1 - t0) * i as f64 / 20.0;
                let sum: f64 = basis.row(t).iter().map(|(_, w)| w).sum();
                assert!((sum - 1.0).abs() < 1e-12, "degree {degree} at {t}: {sum}");
            }
            let control: Vec<[f64; 2]> = (0..7).map(|i| [i as f64, (i * i) as f64]).collect();
            assert!(dist(basis.eval(&control, t0), control[0]) < 1e-12);
            assert!(dist(basis.eval(&control, t1), control[6]) < 1e-12);
        }
    }

    #[test]
    fn a_periodic_ring_closes_on_itself() {
        let control = [[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]];
        for degree in 2..=3 {
            let basis = Basis::new(degree, 4, &[], true).unwrap();
            let pts = basis.sample(&control, 40);
            let (a, b) = (pts[0], pts[40]);
            assert!(dist(a, b) < 1e-5, "degree {degree}");
        }
    }

    #[test]
    fn interpolation_passes_through_every_point_at_every_degree() {
        let points = [
            [0.0, 0.0],
            [3.0, 4.0],
            [7.0, 5.0],
            [9.0, 1.0],
            [12.0, 2.0],
            [15.0, 6.0],
        ];
        for degree in 2..=5 {
            for periodic in [false, true] {
                let fit = interpolate(&points, degree, periodic).unwrap();
                let basis = Basis::new(fit.degree, points.len(), &fit.knots, periodic).unwrap();
                for (q, t) in points.iter().zip(&fit.params) {
                    let on = basis.eval(&fit.control, *t);
                    assert!(
                        dist(on, *q) < 1e-9,
                        "degree {degree} periodic {periodic}: {on:?} vs {q:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn two_points_make_a_straight_spline_and_repeats_make_none() {
        let fit = interpolate(&[[0.0, 0.0], [4.0, 2.0]], 3, false).unwrap();
        assert_eq!(fit.degree, 1);
        assert!(interpolate(&[[0.0, 0.0], [0.0, 0.0], [1.0, 1.0]], 3, false).is_none());
    }

    #[test]
    fn a_chain_is_fitted_within_the_tolerance_with_its_ends_kept() {
        // A quarter circle and the line after it.
        let mut points: Vec<[f64; 2]> = (0..=30)
            .map(|i| {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / 30.0;
                [10.0 * a.cos(), 10.0 * a.sin()]
            })
            .collect();
        points.extend((1..=20).map(|i| [-(i as f64) * 0.5, 10.0]));
        let (knots, control) = fit_chain(&points, 0.01, 64).expect("a fit");
        assert_eq!(control[0], points[0]);
        assert_eq!(*control.last().unwrap(), *points.last().unwrap());
        let basis = Basis::new(3, control.len(), &knots, false).unwrap();
        let sampled = basis.sample(&control, 400);
        for q in &points {
            let nearest = sampled
                .iter()
                .map(|p| dist(*p, *q))
                .fold(f64::MAX, f64::min);
            assert!(nearest < 0.05, "{q:?} is {nearest} away");
        }
    }

    #[test]
    fn a_piece_of_a_spline_is_that_part_of_the_curve() {
        let control = [[0.0, 0.0], [2.0, 5.0], [5.0, -1.0], [8.0, 4.0], [10.0, 0.0]];
        let open = Basis::new(3, 5, &[], false).unwrap();
        let closed = Basis::new(3, 5, &[], true).unwrap();
        for (basis, t0, t1) in [(&open, 0.2, 0.7), (&open, 0.0, 0.4), (&open, 0.5, 1.0)] {
            let piece = basis.piece(&control, t0, t1).unwrap();
            let part = Basis::new(3, piece.control.len(), &piece.knots, false).unwrap();
            for i in 0..=10 {
                let s = i as f64 / 10.0;
                let [x, y] = part.eval(&piece.control, s);
                let [ex, ey] = basis.eval(&control, t0 + (t1 - t0) * s);
                assert!((x - ex).hypot(y - ey) < 1e-9, "{t0}..{t1} at {s}");
            }
        }
        // Across where a closed spline closes.
        let (d0, d1) = closed.domain();
        let (t0, t1) = (d1 - 1.5, d1 + 1.0);
        let piece = closed.piece(&control, t0, t1).unwrap();
        let part = Basis::new(3, piece.control.len(), &piece.knots, false).unwrap();
        for i in 0..=10 {
            let s = i as f64 / 10.0;
            let t = t0 + (t1 - t0) * s;
            let t = if t > d1 { t - (d1 - d0) } else { t };
            let [x, y] = part.eval(&piece.control, s);
            let [ex, ey] = closed.eval(&control, t);
            assert!((x - ex).hypot(y - ey) < 1e-9, "at {s}");
        }
    }
}
