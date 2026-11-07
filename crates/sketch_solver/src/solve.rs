use crate::compile::System;
use crate::problem::Settings;

pub fn solve(problem: &crate::problem::Problem) -> Result<Iteration, crate::input::InputError> {
    let compiled = crate::compile::compile(problem)?;
    Ok(iterate(&compiled, problem.settings))
}

/// Relative convergence tolerance. Convergence is declared when the residual
/// inf-norm drops below `CONVERGENCE_TOL * max(1, |x|_inf)`: residuals carry
/// length units, so scaling the tolerance by the model's coordinate magnitude
/// makes sketches drawn in millimetres and in metres behave the same, while
/// the `max(1, ..)` floor keeps tiny sketches from demanding sub-f64 accuracy.
const CONVERGENCE_TOL: f64 = 1e-9;

/// Result of a constraint solve.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SolveOutcome {
    /// All residuals below tolerance.
    Converged { iterations: usize },
    /// Iteration limit hit with residual norm still above tolerance.
    NotConverged { residual: f64 },
    /// No constraints (or none referencing existing geometry).
    NothingToSolve,
}

/// Numerical output before storage rounding and application updates. Even
/// a stalled attempt exposes its iteration count and effective threshold.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Iteration {
    pub values: Vec<f64>,
    pub outcome: SolveOutcome,
    pub iterations: usize,
    pub residual: f64,
    pub threshold: f64,
}

/// This boundary reads only compiled equations and numerical settings. It
/// neither rounds geometry nor refits splines, follows text or computes
/// application flags; failed held attempts can therefore be inspected
/// without applying their result.
///
/// A change that moves the sketch far is solved again in small steps from
/// where it stands (`follow`): one solve straight to the targets may land
/// on another branch of the equations, the sketch's mirror image, while a
/// path of near steps stays on the branch the drawing is on.
pub fn iterate(sys: &System, settings: Settings) -> Iteration {
    let tolerance = if settings.tolerance > 0.0 {
        settings.tolerance
    } else {
        CONVERGENCE_TOL
    };
    if sys.specs.is_empty() || sys.vars.is_empty() {
        return Iteration {
            values: sys.vars.clone(),
            outcome: SolveOutcome::NothingToSolve,
            iterations: 0,
            residual: 0.0,
            threshold: tolerance * var_scale(&sys.vars),
        };
    }

    let direct = descend(sys, sys.vars.clone(), settings, tolerance, None);
    if !matches!(direct.outcome, SolveOutcome::Converged { .. }) {
        return direct;
    }
    let (moved, size) = farthest_move(sys, &sys.vars, &direct.values);
    let steps = continuation_steps(moved, size);
    if steps <= 1 {
        return direct;
    }
    let mut followed = follow(sys, settings, tolerance, steps);
    if !matches!(followed.outcome, SolveOutcome::Converged { .. }) {
        return direct;
    }
    // Both ways reaching one answer keep the direct one's numbers.
    let (apart, _) = farthest_move(sys, &direct.values, &followed.values);
    if apart <= SAME_ANSWER * size {
        return direct;
    }
    followed.iterations += direct.iterations;
    followed.outcome = SolveOutcome::Converged {
        iterations: followed.iterations,
    };
    followed
}

/// The share of the sketch's size a point may move in one step of a
/// followed solve.
const CONTINUATION_REACH: f64 = 0.05;

/// The most steps a followed solve takes.
const CONTINUATION_STEPS: usize = 16;

/// Followed and direct answers closer than this share of the sketch's size
/// are the same solution.
const SAME_ANSWER: f64 = 1e-3;

/// How many steps a move of `moved` in a sketch of `size` asks for: one
/// while it is within `CONTINUATION_REACH` of the size, more as it is
/// farther, up to `CONTINUATION_STEPS`.
fn continuation_steps(moved: f64, size: f64) -> usize {
    if moved.is_nan() || moved <= 0.0 || !size.is_finite() || size <= 0.0 {
        return 1;
    }
    ((moved / (CONTINUATION_REACH * size)).ceil() as usize).clamp(1, CONTINUATION_STEPS)
}

/// The farthest any movable point lies between `from` and `to`, and the
/// diagonal of the box holding the points in both.
fn farthest_move(sys: &System, from: &[f64], to: &[f64]) -> (f64, f64) {
    let mut free = vec![false; sys.vars.len()];
    for &j in &sys.free {
        free[j] = true;
    }
    let mut moved = 0.0_f64;
    let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for &at in sys.point_vars.values() {
        if !free[at] && !free[at + 1] {
            continue;
        }
        moved = moved.max((to[at] - from[at]).hypot(to[at + 1] - from[at + 1]));
        for (axis, value) in [from[at], from[at + 1], to[at], to[at + 1]]
            .into_iter()
            .enumerate()
        {
            low[axis % 2] = low[axis % 2].min(value);
            high[axis % 2] = high[axis % 2].max(value);
        }
    }
    let size = (high[0] - low[0]).hypot(high[1] - low[1]);
    (moved, size.max(sys.settings.minimum_length.0))
}

/// The solve taken in `steps` steps: step k asks the residuals to fall to
/// `(1 - k / steps)` of what they are at the start (a Newton homotopy), each
/// step starting where the last ended, the last asking them to vanish.
fn follow(sys: &System, settings: Settings, tolerance: f64, steps: usize) -> Iteration {
    let start = eval_residuals(sys, &sys.vars);
    let mut x = sys.vars.clone();
    let mut iterations = 0;
    let mut offset = vec![0.0; start.len()];
    for k in 1..steps {
        let left = 1.0 - k as f64 / steps as f64;
        for (o, r) in offset.iter_mut().zip(&start) {
            *o = left * r;
        }
        let step = descend(sys, x, settings, tolerance * 1e3, Some(&offset));
        iterations += step.iterations;
        x = step.values;
    }
    let mut last = descend(sys, x, settings, tolerance, None);
    last.iterations += iterations;
    last
}

/// Damped Gauss-Newton (Levenberg-Marquardt) from `x` toward residuals equal
/// to `offset` (zero when `None`).
fn descend(
    sys: &System,
    mut x: Vec<f64>,
    settings: Settings,
    tolerance: f64,
    offset: Option<&[f64]>,
) -> Iteration {
    let residuals = |x: &[f64]| {
        let mut r = eval_residuals(sys, x);
        if let Some(offset) = offset {
            for (r, o) in r.iter_mut().zip(offset) {
                *r -= o;
            }
        }
        r
    };
    let mut r = residuals(&x);
    let mut cost = sq_norm(&r);
    let mut lambda = settings.lambda_initial;
    let mut iterations = 0;
    let max_iterations = (settings.max_iterations as usize).max(1);
    let mut converged = inf_norm(&r) < tolerance * var_scale(&x);

    while !converged && iterations < max_iterations {
        iterations += 1;

        let jac = jacobian(sys, &x);
        let (jtj, jtr) = normal_equations(&jac, &r);

        // Each variable is damped by its own curvature plus the average
        // one: with its own alone, a variable the constraints barely feel
        // (a point sliding almost square to the only row that moves it)
        // costs nearly nothing to move, and the step flings it far off.
        let mean_diag =
            jtj.iter().enumerate().map(|(i, row)| row[i]).sum::<f64>() / jtj.len().max(1) as f64;
        let mut improved = false;
        for _ in 0..settings.max_inner_retries {
            // Damped normal equations: (JᵀJ + λ·D) dx = -Jᵀr, with D the
            // diagonal of JᵀJ raised by its mean.
            let mut a = jtj.clone();
            for (i, row) in a.iter_mut().enumerate() {
                row[i] += lambda * (jtj[i][i] + mean_diag).max(settings.damping_floor);
            }
            let rhs: Vec<f64> = jtr.iter().map(|v| -v).collect();
            let mut step = match solve_linear(a, rhs) {
                Some(s) => s,
                None => {
                    lambda = (lambda * 10.0).min(settings.lambda_max);
                    continue;
                }
            };
            cap_step(&mut step, var_scale(&x), settings.maximum_step_scale);

            let mut trial = x.clone();
            for (column, &j) in sys.free.iter().enumerate() {
                trial[j] += step[column];
            }
            let trial_r = residuals(&trial);
            let trial_cost = sq_norm(&trial_r);
            if trial_cost.is_finite() && trial_cost < cost {
                x = trial;
                r = trial_r;
                cost = trial_cost;
                lambda = (lambda / 10.0).max(settings.lambda_min);
                improved = true;
                break;
            }
            lambda = (lambda * 10.0).min(settings.lambda_max);
        }

        if inf_norm(&r) < tolerance * var_scale(&x) {
            converged = true;
            break;
        }
        if !improved {
            // Damping saturated without any cost reduction: the problem is
            // contradictory or the solve sits at a (possibly non-zero) local
            // minimum.
            break;
        }
    }

    let outcome = if converged {
        SolveOutcome::Converged { iterations }
    } else {
        SolveOutcome::NotConverged {
            residual: inf_norm(&r),
        }
    };
    Iteration {
        threshold: tolerance * var_scale(&x),
        values: x,
        outcome,
        iterations,
        residual: inf_norm(&r),
    }
}

pub fn eval_residuals(sys: &System, x: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(sys.residual_len);
    for spec in &sys.specs {
        spec.eval_with_minimum(x, &mut out, sys.settings.minimum_length.0);
    }
    out
}

/// Central-difference Jacobian (m residuals x n variables).
pub fn jacobian(sys: &System, x: &[f64]) -> Vec<Vec<f64>> {
    // One column per movable variable: a pinned reference contributes none,
    // so nothing can trade a constraint against tilting an axis.
    let mut jac = vec![vec![0.0; sys.free.len()]; sys.residual_len];
    let mut probe = x.to_vec();
    for (column, &j) in sys.free.iter().enumerate() {
        let eps = sys.settings.finite_difference_step * x[j].abs().max(1.0);
        let original = probe[j];
        probe[j] = original + eps;
        let r_plus = eval_residuals(sys, &probe);
        probe[j] = original - eps;
        let r_minus = eval_residuals(sys, &probe);
        probe[j] = original;
        for (row, (rp, rm)) in jac.iter_mut().zip(r_plus.iter().zip(&r_minus)) {
            row[column] = (rp - rm) / (2.0 * eps);
        }
    }
    jac
}

/// Build JᵀJ and Jᵀr for the normal equations.
fn normal_equations(jac: &[Vec<f64>], r: &[f64]) -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = jac.first().map_or(0, Vec::len);
    let mut jtj = vec![vec![0.0; n]; n];
    let mut jtr = vec![0.0; n];
    for (row, ri) in jac.iter().zip(r) {
        for (a, ra) in row.iter().enumerate() {
            jtr[a] += ra * ri;
            for (acc, rb) in jtj[a].iter_mut().zip(row) {
                *acc += ra * rb;
            }
        }
    }
    (jtj, jtr)
}

/// Solve `a * x = b` with Gaussian elimination and partial pivoting.
/// Returns `None` when the matrix is numerically singular.
pub(crate) fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let mut pivot_row = col;
        let mut pivot_val = a[col][col].abs();
        for (row, row_vals) in a.iter().enumerate().skip(col + 1) {
            let v = row_vals[col].abs();
            if v > pivot_val {
                pivot_val = v;
                pivot_row = row;
            }
        }
        if !pivot_val.is_finite() || pivot_val < 1e-300 {
            return None;
        }
        a.swap(col, pivot_row);
        b.swap(col, pivot_row);
        let (upper, lower) = a.split_at_mut(col + 1);
        let pivot_vals = &upper[col];
        let pivot = pivot_vals[col];
        let b_pivot = b[col];
        for (offset, row_vals) in lower.iter_mut().enumerate() {
            let factor = row_vals[col] / pivot;
            if factor == 0.0 {
                continue;
            }
            for (dst, src) in row_vals[col..].iter_mut().zip(&pivot_vals[col..]) {
                *dst -= factor * src;
            }
            b[col + 1 + offset] -= factor * b_pivot;
        }
    }
    let mut x = vec![0.0; n];
    for col in (0..n).rev() {
        let mut sum = b[col];
        for k in (col + 1)..n {
            sum -= a[col][k] * x[k];
        }
        x[col] = sum / a[col][col];
    }
    if x.iter().all(|v| v.is_finite()) {
        Some(x)
    } else {
        None
    }
}

/// Numerical rank via row echelon form with partial pivoting. Pivots below
/// the relative tolerance times the largest entry are treated as zero, which is
/// loose enough to flag redundant (dependent) constraint rows near a solution
/// while keeping genuinely independent rows.
pub fn jacobian_rank(mut m: Vec<Vec<f64>>, rank_tolerance: f64) -> usize {
    let rows = m.len();
    let cols = m.first().map_or(0, Vec::len);
    if rows == 0 || cols == 0 {
        return 0;
    }
    let max_abs = m.iter().flatten().fold(0.0_f64, |acc, v| acc.max(v.abs()));
    if max_abs == 0.0 || !max_abs.is_finite() {
        return 0;
    }
    let tol = max_abs * rank_tolerance;

    let mut rank = 0;
    let mut row = 0;
    for col in 0..cols {
        if row >= rows {
            break;
        }
        let mut pivot_row = row;
        let mut pivot_val = m[row][col].abs();
        for (r, row_vals) in m.iter().enumerate().skip(row + 1) {
            let v = row_vals[col].abs();
            if v > pivot_val {
                pivot_val = v;
                pivot_row = r;
            }
        }
        if pivot_val <= tol {
            continue;
        }
        m.swap(row, pivot_row);
        let (upper, lower) = m.split_at_mut(row + 1);
        let pivot_vals = &upper[row];
        let pivot = pivot_vals[col];
        for row_vals in lower.iter_mut() {
            let factor = row_vals[col] / pivot;
            if factor == 0.0 {
                continue;
            }
            for (dst, src) in row_vals[col..].iter_mut().zip(&pivot_vals[col..]) {
                *dst -= factor * src;
            }
        }
        row += 1;
        rank += 1;
    }
    rank
}

fn sq_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

pub fn inf_norm(v: &[f64]) -> f64 {
    v.iter().fold(0.0_f64, |acc, x| acc.max(x.abs()))
}

/// Characteristic magnitude of the variable vector, floored at 1.
pub fn var_scale(x: &[f64]) -> f64 {
    inf_norm(x).max(1.0)
}

/// Cap the step inf-norm relative to the variable scale so a single
/// ill-conditioned iteration cannot fling the geometry to infinity.
fn cap_step(step: &mut [f64], scale: f64, maximum_step_scale: f64) {
    let max_step = maximum_step_scale * scale;
    let norm = inf_norm(step);
    if norm > max_step {
        let factor = max_step / norm;
        for v in step.iter_mut() {
            *v *= factor;
        }
    }
}

pub fn probe_direct(sys: &System, settings: Settings) -> Iteration {
    descend(sys, sys.vars.clone(), settings, CONVERGENCE_TOL, None)
}
