use crate::compile::System;
use crate::problem::Settings;
use crate::sparse::{Block, Blocks};

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
    iterate_in(sys, None, settings)
}

/// [`iterate`], given `sys`'s [`Blocks`] where the caller keeps them: they
/// depend only on its equations and free variables, never on its values.
pub(crate) fn iterate_in(sys: &System, blocks: Option<&Blocks>, settings: Settings) -> Iteration {
    let owned = std::sync::OnceLock::new();
    let rest = Rest::of(sys);
    let piece = Piece {
        sys,
        blocks: match blocks {
            Some(blocks) => BlocksOf::Given(blocks),
            None => BlocksOf::Lazy(&owned),
        },
        rest: &rest,
    };
    let solved = iterate_pieces(&[piece], settings);
    let PiecesIteration {
        mut values,
        outcome,
        iterations,
        residual,
        threshold,
        ..
    } = solved;
    Iteration {
        values: values.pop().flatten().unwrap_or_else(|| sys.vars.clone()),
        outcome,
        iterations,
        residual,
        threshold,
    }
}

/// One of the systems a solve iterates together: the parts of a sketch
/// no equation joins, each compiled on its own. They share the solve's
/// scale, passes, continuation and outcome, so solving them together
/// gives what solving them as one system gives, to the bit.
pub(crate) struct Piece<'a> {
    pub sys: &'a System,
    pub blocks: BlocksOf<'a>,
    pub rest: &'a Rest,
}

/// A piece's [`Blocks`]: given, or worked out when first needed.
pub(crate) enum BlocksOf<'a> {
    Given(&'a Blocks),
    Lazy(&'a std::sync::OnceLock<Blocks>),
}

impl Piece<'_> {
    fn blocks(&self) -> &Blocks {
        match self.blocks {
            BlocksOf::Given(blocks) => blocks,
            BlocksOf::Lazy(cell) => cell.get_or_init(|| Blocks::of(self.sys)),
        }
    }
}

/// What a system's values say before it is solved, which depends on
/// nothing else: kept with a compiled part, so an untouched part is
/// neither evaluated nor sorted into blocks.
#[derive(Debug, Clone)]
pub(crate) struct Rest {
    /// The values the solve starts from, where parting stacked points
    /// ([`part_coincident_ends`]) changes them.
    pub start: Option<Vec<f64>>,
    /// The residuals' largest magnitude at the values, where they are the
    /// start.
    pub residual: f64,
    /// The values' largest magnitude.
    pub size: f64,
    /// The box of the movable points, for [`farthest_move`].
    bounds: std::sync::OnceLock<Bounds>,
}

/// The lower and upper corners of a box of points.
type Bounds = ([f64; 2], [f64; 2]);

impl Rest {
    pub(crate) fn of(sys: &System) -> Self {
        let start = part_coincident_ends(sys);
        let (start, residual) = if start == sys.vars {
            (None, inf_norm(&eval_residuals(sys, &sys.vars)))
        } else {
            (Some(start), f64::INFINITY)
        };
        Self {
            start,
            residual,
            size: inf_norm(&sys.vars),
            bounds: Default::default(),
        }
    }

    fn bounds(&self, sys: &System) -> Bounds {
        *self.bounds.get_or_init(|| {
            let mut bounds = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            reach(sys, &sys.vars, &sys.vars, &mut bounds);
            bounds
        })
    }
}

/// What [`iterate_pieces`] answers: each piece's values, `None` where
/// they are the piece's own.
pub(crate) struct PiecesIteration {
    pub values: Vec<Option<Vec<f64>>>,
    /// Each piece whose residuals end at or above the threshold.
    pub unmet: Vec<bool>,
    pub outcome: SolveOutcome,
    pub iterations: usize,
    pub residual: f64,
    pub threshold: f64,
}

/// [`iterate`] of the pieces together. A piece whose values meet its
/// equations where they stand takes no step and keeps them.
pub(crate) fn iterate_pieces(pieces: &[Piece], settings: Settings) -> PiecesIteration {
    let tolerance = if settings.tolerance > 0.0 {
        settings.tolerance
    } else {
        CONVERGENCE_TOL
    };
    let size = pieces.iter().fold(0.0_f64, |m, p| m.max(p.rest.size));
    let specs = pieces.iter().any(|p| !p.sys.specs.is_empty());
    let vars = pieces.iter().any(|p| !p.sys.vars.is_empty());
    let unchanged = || pieces.iter().map(|_| None).collect();
    let met = || vec![false; pieces.len()];
    if !specs || !vars {
        return PiecesIteration {
            values: unchanged(),
            unmet: met(),
            outcome: SolveOutcome::NothingToSolve,
            iterations: 0,
            residual: 0.0,
            threshold: tolerance * size.max(1.0),
        };
    }

    // Already within the tolerance where it stands: no block takes a
    // step, so nothing is worth sorting into blocks.
    if pieces.iter().all(|p| p.rest.start.is_none()) {
        let residual = pieces.iter().fold(0.0_f64, |m, p| m.max(p.rest.residual));
        let threshold = tolerance * size.max(1.0);
        if residual < threshold {
            return PiecesIteration {
                values: unchanged(),
                unmet: met(),
                outcome: SolveOutcome::Converged { iterations: 0 },
                iterations: 0,
                residual,
                threshold,
            };
        }
    }
    let starts =
        || -> Vec<Option<Vec<f64>>> { pieces.iter().map(|p| p.rest.start.clone()).collect() };
    let direct = descend(pieces, starts(), settings, tolerance, None);
    if !matches!(direct.outcome, SolveOutcome::Converged { .. }) {
        return direct;
    }
    let (moved, size) = farthest_move(pieces, None, &direct.values);
    let steps = continuation_steps(moved, size);
    if steps <= 1 {
        return direct;
    }
    let mut followed = follow(pieces, starts(), settings, tolerance, steps);
    if !matches!(followed.outcome, SolveOutcome::Converged { .. }) {
        return direct;
    }
    // Both ways reaching one answer keep the direct one's numbers.
    let (apart, _) = farthest_move(pieces, Some(&direct.values), &followed.values);
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

/// The farthest any movable point lies between `from` and `to` (each
/// piece's own values where `None`; every piece's where `from` is), and
/// the diagonal of the box holding the points in both.
fn farthest_move(
    pieces: &[Piece],
    from: Option<&[Option<Vec<f64>>]>,
    to: &[Option<Vec<f64>>],
) -> (f64, f64) {
    let mut moved = 0.0_f64;
    let mut bounds = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (i, piece) in pieces.iter().enumerate() {
        let vars = &piece.sys.vars;
        let from = from.and_then(|from| from[i].as_deref());
        let to = to[i].as_deref();
        if from.is_none() && to.is_none() {
            // Neither moved: its points stay in the box they stand in.
            let (low, high) = piece.rest.bounds(piece.sys);
            for axis in 0..2 {
                bounds.0[axis] = bounds.0[axis].min(low[axis]);
                bounds.1[axis] = bounds.1[axis].max(high[axis]);
            }
            continue;
        }
        let (from, to) = (from.unwrap_or(vars), to.unwrap_or(vars));
        moved = moved.max(reach(piece.sys, from, to, &mut bounds));
    }
    let size = (bounds.1[0] - bounds.0[0]).hypot(bounds.1[1] - bounds.0[1]);
    let minimum = pieces
        .first()
        .map_or(0.0, |p| p.sys.settings.minimum_length.0);
    (moved, size.max(minimum))
}

/// The farthest a movable point of `sys` lies between `from` and `to`,
/// the points in both taken into `bounds`.
fn reach(sys: &System, from: &[f64], to: &[f64], bounds: &mut Bounds) -> f64 {
    let mut free = vec![false; sys.vars.len()];
    for &j in &sys.free {
        free[j] = true;
    }
    let mut moved = 0.0_f64;
    let (low, high) = bounds;
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
    moved
}

/// The solve taken in `steps` steps: step k asks the residuals to fall to
/// `(1 - k / steps)` of what they are at the start (a Newton homotopy), each
/// step starting where the last ended, the last asking them to vanish.
fn follow(
    pieces: &[Piece],
    starts: Vec<Option<Vec<f64>>>,
    settings: Settings,
    tolerance: f64,
    steps: usize,
) -> PiecesIteration {
    let mut x: Vec<Option<Vec<f64>>> = (pieces.iter().zip(starts))
        .map(|(p, start)| Some(start.unwrap_or_else(|| p.sys.vars.clone())))
        .collect();
    let start: Vec<Vec<f64>> = (pieces.iter().zip(&x))
        .map(|(p, x)| eval_residuals(p.sys, x.as_deref().unwrap_or_default()))
        .collect();
    let mut iterations = 0;
    let mut offset: Vec<Vec<f64>> = start.iter().map(|r| vec![0.0; r.len()]).collect();
    for k in 1..steps {
        let left = 1.0 - k as f64 / steps as f64;
        for (offset, start) in offset.iter_mut().zip(&start) {
            for (o, r) in offset.iter_mut().zip(start) {
                *o = left * r;
            }
        }
        let step = descend(pieces, x, settings, tolerance * 1e3, Some(&offset));
        iterations += step.iterations;
        x = step.values;
    }
    let mut last = descend(pieces, x, settings, tolerance, None);
    last.iterations += iterations;
    last
}

/// The starting values with every pair of points a distance or length
/// holds apart, and every point held on a circle's rim with that circle's
/// centre, but which sit on one spot, moved a little apart: there
/// `|p2 - p1|` changes the same way whichever way a point moves, so the
/// solve would see no direction to pull them and a rank would not count
/// what holds them. The second point (the one on the rim) moves along x
/// (its y, then the first point, when that is held), always the same way,
/// so one input always starts from one place.
pub(crate) fn part_coincident_ends(sys: &System) -> Vec<f64> {
    use crate::residual::{ResidualSpec, segment_length};
    let mut x = sys.vars.clone();
    let mut free = vec![false; x.len()];
    for &j in &sys.free {
        free[j] = true;
    }
    for spec in &sys.specs {
        let (a, b, d) = match *spec {
            ResidualSpec::Distance { p1, p2, d } => (p1, p2, d),
            ResidualSpec::Length { s, e, len } => (s, e, len),
            ResidualSpec::PointOnCircle { p, c, r } | ResidualSpec::ArcEndpoint { p, c, r } => {
                (c, p, x[r])
            }
            _ => continue,
        };
        // The finite-difference step, so the parted pair is seen apart by
        // every probe of the Jacobian.
        let reach = [x[a], x[a + 1], x[b], x[b + 1]]
            .iter()
            .fold(1.0_f64, |m, v| m.max(v.abs()))
            * sys.settings.finite_difference_step;
        if d <= reach || segment_length(&x, a, b) > reach {
            continue;
        }
        if let Some(j) = [b, b + 1, a, a + 1].into_iter().find(|&j| free[j]) {
            x[j] += if (b..=b + 1).contains(&j) { 2.0 } else { -2.0 } * reach;
        }
    }
    x
}

/// The damping after a rejected step, or `None` when it stays where it
/// is (already the largest): the next retry would take the very step just
/// rejected, from the same point, and be rejected again.
pub(crate) fn raised(lambda: f64, settings: &Settings) -> Option<f64> {
    let next = (lambda * 10.0).min(settings.lambda_max);
    #[cfg(test)]
    if RETRY_SATURATED.get() {
        return Some(next);
    }
    (next != lambda).then_some(next)
}

#[cfg(test)]
thread_local! {
    /// Retry at the largest damping until the retries run out, as a test's
    /// reference for what stopping there gives.
    pub(crate) static RETRY_SATURATED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The most passes [`descend`] makes over the blocks: a pass solves each
/// block to the tolerance at the scale the pass began with, and another
/// follows only when the geometry's scale shrank under a block's answer.
const SCALE_PASSES: usize = 4;

/// One piece's state through [`descend`], once it is worked on.
struct Worked {
    x: Vec<f64>,
    probe: Vec<f64>,
    spent: Vec<usize>,
    stalled: Vec<bool>,
}

/// Damped Gauss-Newton (Levenberg-Marquardt) from `x` toward residuals equal
/// to `offset` (zero when `None`), each Jacobian block on its own: its own
/// damping, its own acceptance of a step by its own cost, and its own
/// convergence, so a block already within the tolerance takes no step and
/// keeps its values exactly, and how one block converges never changes
/// another's numbers. The whole solve converges when every residual is
/// within the tolerance at the scale of the values it ends on, as each
/// block's own test is. A piece starting from its own values (`None`) is
/// left there while its residuals are within each pass's tolerance, as
/// each of its blocks would be; `offset` is `None` whenever one does.
fn descend(
    pieces: &[Piece],
    x: Vec<Option<Vec<f64>>>,
    settings: Settings,
    tolerance: f64,
    offset: Option<&[Vec<f64>]>,
) -> PiecesIteration {
    debug_assert!(offset.is_none() || x.iter().all(Option::is_some));
    let norm = |i: usize, x: &[f64]| {
        let mut r = eval_residuals(pieces[i].sys, x);
        if let Some(offset) = offset {
            for (r, o) in r.iter_mut().zip(&offset[i]) {
                *r -= o;
            }
        }
        inf_norm(&r)
    };
    let max_iterations = (settings.max_iterations as usize).max(1);
    let mut worked: Vec<Option<Worked>> = x
        .into_iter()
        .map(|x| {
            x.map(|x| Worked {
                probe: x.clone(),
                x,
                spent: Vec::new(),
                stalled: Vec::new(),
            })
        })
        .collect();
    let mut norms: Vec<f64> = (pieces.iter().zip(&worked).enumerate())
        .map(|(i, (p, w))| match w {
            Some(w) => norm(i, &w.x),
            None => p.rest.residual,
        })
        .collect();
    let scale = |worked: &[Option<Worked>]| {
        (pieces.iter().zip(worked))
            .fold(0.0_f64, |m, (p, w)| {
                m.max(w.as_ref().map_or(p.rest.size, |w| inf_norm(&w.x)))
            })
            .max(1.0)
    };
    let mut work = BlockWork::default();
    for _ in 0..SCALE_PASSES {
        let values = scale(&worked);
        let threshold = tolerance * values;
        if norms.iter().fold(0.0_f64, |m, &r| m.max(r)) < threshold {
            break;
        }
        let mut stepped = false;
        for (i, piece) in pieces.iter().enumerate() {
            if worked[i].is_none() && norms[i] < threshold {
                continue;
            }
            let blocks = piece.blocks();
            let w = worked[i].get_or_insert_with(|| Worked {
                x: piece.sys.vars.clone(),
                probe: piece.sys.vars.clone(),
                spent: Vec::new(),
                stalled: Vec::new(),
            });
            w.spent.resize(blocks.blocks.len(), 0);
            w.stalled.resize(blocks.blocks.len(), false);
            std::mem::swap(&mut work.probe, &mut w.probe);
            for (b, block) in blocks.blocks.iter().enumerate() {
                if w.stalled[b] || w.spent[b] >= max_iterations {
                    continue;
                }
                let outcome = descend_block(
                    piece.sys,
                    block,
                    &mut w.x,
                    &settings,
                    Scale { values, threshold },
                    offset.map(|o| o[i].as_slice()),
                    max_iterations - w.spent[b],
                    &mut work,
                );
                w.spent[b] += outcome.iterations;
                w.stalled[b] = !outcome.converged;
                stepped |= outcome.iterations > 0;
            }
            std::mem::swap(&mut work.probe, &mut w.probe);
            norms[i] = norm(i, &w.x);
        }
        if !stepped {
            break;
        }
    }

    let residual = norms.iter().fold(0.0_f64, |m, &r| m.max(r));
    let threshold = tolerance * scale(&worked);
    let iterations = (worked.iter().flatten())
        .flat_map(|w| w.spent.iter().copied())
        .max()
        .unwrap_or(0);
    let outcome = if residual < threshold {
        SolveOutcome::Converged { iterations }
    } else {
        SolveOutcome::NotConverged { residual }
    };
    PiecesIteration {
        threshold,
        unmet: (norms.iter())
            .map(|r| r.partial_cmp(&threshold) != Some(std::cmp::Ordering::Less))
            .collect(),
        values: worked.into_iter().map(|w| w.map(|w| w.x)).collect(),
        outcome,
        iterations,
        residual,
    }
}

/// Buffers one block's iteration reuses from the block before. `probe`
/// holds the values being solved between iterations, so a block's
/// differences and trial steps are taken in place.
#[derive(Default)]
struct BlockWork {
    dense: Dense,
    r: Vec<f64>,
    trial_r: Vec<f64>,
    probe: Vec<f64>,
}

/// A pass's scale of the values and the residual threshold at it.
#[derive(Clone, Copy)]
struct Scale {
    values: f64,
    threshold: f64,
}

/// How one block's iteration ended.
struct BlockOutcome {
    iterations: usize,
    converged: bool,
}

/// One block of [`descend`]: Levenberg-Marquardt over the block's
/// columns and rows alone, until its residuals are all below the
/// threshold, it stalls, or it has taken `budget` iterations. A step is
/// capped at the pass's scale of the values.
#[allow(clippy::too_many_arguments)]
fn descend_block(
    sys: &System,
    block: &Block,
    x: &mut [f64],
    settings: &Settings,
    scale: Scale,
    offset: Option<&[f64]>,
    budget: usize,
    work: &mut BlockWork,
) -> BlockOutcome {
    let BlockWork {
        dense,
        r,
        trial_r,
        probe,
    } = work;
    let less_offset = |r: &mut Vec<f64>| {
        if let Some(offset) = offset {
            for (r, &row) in r.iter_mut().zip(&block.rows) {
                *r -= offset[row];
            }
        }
    };
    block.residuals(sys, x, r);
    less_offset(r);
    if inf_norm(r) < scale.threshold {
        return BlockOutcome {
            iterations: 0,
            converged: true,
        };
    }
    let mut cost = sq_norm(r);
    let mut lambda = settings.lambda_initial;
    let mut iterations = 0;
    // The block's values and the step taken on them, by column.
    let vars: Vec<usize> = block.columns.iter().map(|&c| sys.free[c]).collect();
    let mut step = vec![0.0; vars.len()];
    while iterations < budget {
        iterations += 1;
        let jac = block.jacobian(sys, x, probe);
        let (jtj, jtr) = normal_equations(&jac, r);

        // Each variable is damped by its own curvature plus the block's
        // average one: with its own alone, a variable the constraints
        // barely feel (a point sliding almost square to the only row that
        // moves it) costs nearly nothing to move, and the step flings it
        // far off.
        let mean_diag = (0..jtj.len()).map(|i| jtj[i][i]).sum::<f64>() / jtj.len().max(1) as f64;
        let mut improved = false;
        for _ in 0..settings.max_inner_retries {
            // Damped normal equations: (JᵀJ + λ·D) dx = -Jᵀr, with D the
            // diagonal of JᵀJ raised by its mean.
            dense.a.clear();
            for (i, row) in jtj.iter().enumerate() {
                dense.a.extend_from_slice(row);
                let at = dense.a.len() - row.len() + i;
                dense.a[at] += lambda * (jtj[i][i] + mean_diag).max(settings.damping_floor);
            }
            dense.b.clear();
            dense.b.extend(jtr.iter().map(|v| -v));
            if !dense.solve() {
                match raised(lambda, settings) {
                    Some(raised) => lambda = raised,
                    None => break,
                }
                continue;
            }
            step.copy_from_slice(&dense.x);
            cap_step(&mut step, scale.values, settings.maximum_step_scale);

            for (&j, dx) in vars.iter().zip(&step) {
                probe[j] = x[j] + dx;
            }
            block.residuals(sys, probe, trial_r);
            less_offset(trial_r);
            let trial_cost = sq_norm(trial_r);
            if trial_cost.is_finite() && trial_cost < cost {
                for &j in &vars {
                    x[j] = probe[j];
                }
                std::mem::swap(r, trial_r);
                cost = trial_cost;
                lambda = (lambda / 10.0).max(settings.lambda_min);
                improved = true;
                break;
            }
            for &j in &vars {
                probe[j] = x[j];
            }
            match raised(lambda, settings) {
                Some(raised) => lambda = raised,
                None => break,
            }
        }

        if inf_norm(r) < scale.threshold {
            return BlockOutcome {
                iterations,
                converged: true,
            };
        }
        if !improved {
            // Damping saturated without any cost reduction: the block's
            // equations contradict each other or the solve sits at a
            // (possibly non-zero) local minimum.
            break;
        }
    }
    BlockOutcome {
        iterations,
        converged: false,
    }
}

pub fn eval_residuals(sys: &System, x: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(sys.residual_len);
    for spec in &sys.specs {
        spec.eval_with_minimum(x, &mut out, sys.settings.minimum_length.0);
    }
    out
}

/// Central-difference Jacobian (m residuals x n variables), worked out
/// block by block ([`Blocks`]). One column per movable variable: a pinned
/// reference contributes none, so nothing can trade a constraint against
/// tilting an axis.
pub fn jacobian(sys: &System, x: &[f64]) -> Vec<Vec<f64>> {
    let blocks = Blocks::of(sys);
    blocks.dense(&blocks.jacobian(sys, x), sys.residual_len, sys.free.len())
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
#[cfg(test)]
pub(crate) fn solve_linear(a: Vec<Vec<f64>>, b: Vec<f64>) -> Option<Vec<f64>> {
    let mut dense = Dense::default();
    dense.a.extend(a.iter().flatten());
    dense.b = b;
    dense.solve().then_some(dense.x)
}

/// A square system `a x = b` solved by Gaussian elimination with partial
/// pivoting, its buffers kept from one system to the next: `a` row-major,
/// `x` the answer after [`Dense::solve`].
#[derive(Default)]
pub(crate) struct Dense {
    pub a: Vec<f64>,
    pub b: Vec<f64>,
    pub x: Vec<f64>,
}

impl Dense {
    /// Empty `n`×`n` matrix and right-hand side, every entry zero.
    pub(crate) fn zeroed(&mut self, n: usize) {
        self.a.clear();
        self.a.resize(n * n, 0.0);
        self.b.clear();
        self.b.resize(n, 0.0);
    }

    /// Solve in place, leaving `a` and `b` reduced. False when a pivot is
    /// zero or not finite, or the answer is not finite.
    pub(crate) fn solve(&mut self) -> bool {
        let n = self.b.len();
        let (a, b) = (&mut self.a, &mut self.b);
        for col in 0..n {
            let mut pivot_row = col;
            let mut pivot_val = a[col * n + col].abs();
            for row in col + 1..n {
                let v = a[row * n + col].abs();
                if v > pivot_val {
                    pivot_val = v;
                    pivot_row = row;
                }
            }
            if !pivot_val.is_finite() || pivot_val < 1e-300 {
                return false;
            }
            if pivot_row != col {
                let (upper, lower) = a.split_at_mut(pivot_row * n);
                upper[col * n..(col + 1) * n].swap_with_slice(&mut lower[..n]);
                b.swap(col, pivot_row);
            }
            let (upper, lower) = a.split_at_mut((col + 1) * n);
            let pivot_vals = &upper[col * n..];
            let pivot = pivot_vals[col];
            let b_pivot = b[col];
            for (offset, row_vals) in lower.chunks_exact_mut(n).enumerate() {
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
        self.x.clear();
        self.x.resize(n, 0.0);
        let x = &mut self.x;
        for col in (0..n).rev() {
            let mut sum = b[col];
            for k in (col + 1)..n {
                sum -= a[col * n + k] * x[k];
            }
            x[col] = sum / a[col * n + col];
        }
        x.iter().all(|v| v.is_finite())
    }
}

/// Numerical rank via row echelon form with partial pivoting. Pivots below
/// the relative tolerance times the largest entry are treated as zero, which is
/// loose enough to flag redundant (dependent) constraint rows near a solution
/// while keeping genuinely independent rows.
pub fn jacobian_rank(m: Vec<Vec<f64>>, rank_tolerance: f64) -> usize {
    if m.first().is_none_or(Vec::is_empty) {
        return 0;
    }
    let max_abs = m.iter().flatten().fold(0.0_f64, |acc, v| acc.max(v.abs()));
    if max_abs == 0.0 || !max_abs.is_finite() {
        return 0;
    }
    rank_within(m, max_abs * rank_tolerance)
}

/// [`jacobian_rank`] with pivots at or below `tol` treated as zero.
pub(crate) fn rank_within(mut m: Vec<Vec<f64>>, tol: f64) -> usize {
    let rows = m.len();
    let cols = m.first().map_or(0, Vec::len);
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
    let blocks = Blocks::of(sys);
    let rest = Rest::of(sys);
    let piece = Piece {
        sys,
        blocks: BlocksOf::Given(&blocks),
        rest: &rest,
    };
    let mut direct = descend(
        &[piece],
        vec![Some(sys.vars.clone())],
        settings,
        CONVERGENCE_TOL,
        None,
    );
    Iteration {
        values: direct.values.pop().flatten().unwrap_or_default(),
        outcome: direct.outcome,
        iterations: direct.iterations,
        residual: direct.residual,
        threshold: direct.threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::Dense;

    /// Elimination over a matrix of rows, row swaps as row moves: the
    /// reference the flat buffers are checked against.
    fn by_rows(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
        let n = b.len();
        for col in 0..n {
            let mut pivot_row = col;
            let mut pivot_val = a[col][col].abs();
            for (row, vals) in a.iter().enumerate().skip(col + 1) {
                if vals[col].abs() > pivot_val {
                    pivot_val = vals[col].abs();
                    pivot_row = row;
                }
            }
            if !pivot_val.is_finite() || pivot_val < 1e-300 {
                return None;
            }
            a.swap(col, pivot_row);
            b.swap(col, pivot_row);
            let pivot = a[col].clone();
            for row in col + 1..n {
                let factor = a[row][col] / pivot[col];
                if factor == 0.0 {
                    continue;
                }
                for (dst, src) in a[row][col..].iter_mut().zip(&pivot[col..]) {
                    *dst -= factor * src;
                }
                b[row] -= factor * b[col];
            }
        }
        let mut x = vec![0.0; n];
        for col in (0..n).rev() {
            let mut sum = b[col];
            for k in col + 1..n {
                sum -= a[col][k] * x[k];
            }
            x[col] = sum / a[col][col];
        }
        x.iter().all(|v| v.is_finite()).then_some(x)
    }

    #[test]
    fn flat_elimination_matches_elimination_by_rows_to_the_bit() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut unit = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut dense = Dense::default();
        let mut solved = 0;
        for case in 0..400 {
            let n = 1 + case % 9;
            let mut rows: Vec<Vec<f64>> = (0..n)
                .map(|_| (0..n).map(|_| unit() * 4.0 - 2.0).collect())
                .collect();
            // Some with a zero entry, a repeated row or a zero column, so
            // eliminations are skipped and singular systems are met.
            match case % 5 {
                1 => rows[0][n - 1] = 0.0,
                2 if n > 1 => rows[n - 1] = rows[0].clone(),
                3 => rows.iter_mut().for_each(|r| r[0] = 0.0),
                _ => {}
            }
            let b: Vec<f64> = (0..n).map(|_| unit() * 10.0 - 5.0).collect();
            let expected = by_rows(rows.clone(), b.clone());
            dense.a.clear();
            dense.a.extend(rows.iter().flatten());
            dense.b.clone_from(&b);
            let got = dense.solve().then(|| dense.x.clone());
            let bits = |v: &Option<Vec<f64>>| {
                v.as_ref()
                    .map(|v| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>())
            };
            assert_eq!(bits(&got), bits(&expected), "case {case}");
            solved += usize::from(got.is_some());
        }
        assert!(solved > 200, "{solved}");
    }
}
