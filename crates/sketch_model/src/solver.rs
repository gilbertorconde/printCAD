//! Application adapter for the standalone sketch constraint solver.

use crate::operations::SolveOutcome;
#[cfg(test)]
use crate::sketch::{AxisDirection, ORIGIN_ID, X_AXIS_ID};
use crate::sketch::{ConstraintKind, GeometryElement, InternalRole, Sketch, SolverSettings, Vec2D};
use sketch_solver::compile::System;
#[cfg(test)]
use sketch_solver::solve::eval_residuals;
use uuid::Uuid;

mod adapter;
#[cfg(test)]
#[path = "solver/application_tests.rs"]
mod application_tests;
#[cfg(test)]
#[path = "solver/migration.rs"]
mod migration;
pub mod report;

pub fn solve_checked(
    sketch: &mut Sketch,
) -> Result<SolveOutcome, sketch_solver::input::InputError> {
    if sketch.solver.max_iterations == 0 {
        return Err(sketch_solver::input::InputError::InvalidSettings(
            "max_iterations".into(),
        ));
    }
    if !sketch.solver.tolerance.is_finite() || sketch.solver.tolerance <= 0.0 {
        return Err(sketch_solver::input::InputError::InvalidSettings(
            "tolerance".into(),
        ));
    }
    let sys = sketch_solver::compile::compile(&adapter::prepare(sketch, &[]))?;
    Ok(solve_prepared(sketch, sys).0)
}

/// Solve the sketch's constraints by adjusting point positions and
/// circle/arc radii in place. On NotConverged the best-effort geometry is
/// still written back. Also updates `sketch.is_fully_constrained`.
pub fn solve(sketch: &mut Sketch) -> SolveOutcome {
    solve_measured(sketch).0
}

/// [`solve`], with the freedom it measured on the geometry it leaves:
/// none when it measured none, or when it laid geometry out after
/// measuring (a text's outlines, a fitted spline).
pub fn solve_measured(sketch: &mut Sketch) -> (SolveOutcome, Option<i32>) {
    let (outcome, measured) = solve_measuring(sketch);
    (outcome, measured.map(|m| m.dof))
}

/// What [`solve_measured`] measured the freedom on: the solver's problem
/// of the sketch it leaves, which that sketch's settling, locked elements
/// and diagnosis read again.
pub struct Measured {
    pub dof: i32,
    pub prepared: Prepared,
}

/// [`solve_measured`], with the problem it measured the freedom on.
pub fn solve_measuring(sketch: &mut Sketch) -> (SolveOutcome, Option<Measured>) {
    // Text outlines are made from their blocks after the solve.
    crate::text::strip(sketch);
    let (outcome, measured) = if sketch.soft.is_empty() {
        let sys = build_system(sketch);
        solve_prepared(sketch, sys)
    } else {
        solve_soft(sketch)
    };
    (outcome, measured.filter(|_| !laid_out(sketch)))
}

/// Whether the application lays geometry out after a solve: a text's
/// outlines, a refitted spline. Without either, the sketch a solve
/// leaves is the sketch it measured.
fn laid_out(sketch: &Sketch) -> bool {
    !sketch.texts.is_empty()
        || (sketch.geometry.iter()).any(|g| matches!(g, GeometryElement::BSpline(_)))
}

/// A sketch's problem as the solver is handed it with nothing held, and
/// its system compiled for rank once first asked for: one sketch's
/// freedom, settling, locked elements and diagnosis share them.
#[derive(Clone)]
pub struct Prepared {
    problem: sketch_solver::problem::Problem,
    rank: std::sync::OnceLock<std::sync::Arc<sketch_solver::compile::RankSystem>>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared").finish_non_exhaustive()
    }
}

impl Prepared {
    pub fn of(sketch: &Sketch) -> Self {
        Self {
            problem: adapter::prepare(sketch, &[]),
            rank: Default::default(),
        }
    }

    #[doc(hidden)]
    pub fn problem(&self) -> &sketch_solver::problem::Problem {
        &self.problem
    }

    fn rank(&self) -> &sketch_solver::compile::RankSystem {
        self.rank.get_or_init(|| {
            std::sync::Arc::new(sketch_solver::compile::compile_for_rank(&self.problem))
        })
    }

    /// The system compiled without rank's extra equations, where it is
    /// the one compiled for rank.
    fn plain(&self) -> Option<&System> {
        let rank = self.rank();
        rank.plain.then_some(&rank.system)
    }

    /// [`dof_estimate`] of the sketch prepared.
    pub fn dof(&self) -> i32 {
        sketch_solver::freedom::estimate_in(&self.problem, &self.rank().system).degrees
    }

    fn too_large(&self) -> bool {
        self.problem.constraints.len() > self.problem.settings.diagnosis_limit
    }
}

/// [`solve`] with the sketch's measured points settling what the
/// constraints leave free.
fn solve_soft(sketch: &mut Sketch) -> (SolveOutcome, Option<Measured>) {
    let soft: Vec<sketch_solver::soft::SoftPoint> = sketch
        .soft
        .iter()
        .filter(|s| sketch.get_geometry(s.element).is_some())
        .flat_map(|s| {
            s.points.iter().map(|p| sketch_solver::soft::SoftPoint {
                curve: adapter::curve(s.element),
                at: sketch_solver::problem::Vector { x: p[0], y: p[1] },
            })
        })
        .collect();
    let problem = adapter::prepare(sketch, &[]);
    let Ok(answer) = sketch_solver::soft::solve_soft(&problem, &soft) else {
        let sys = build_system(sketch);
        return solve_prepared(sketch, sys);
    };
    let iteration = Iteration {
        outcome: from_core(answer.iteration.outcome),
        iterations: answer.iteration.iterations,
        residual: answer.iteration.residual,
        threshold: answer.iteration.threshold,
        values: answer.iteration.values,
    };
    let dof = apply(sketch, &answer.system, &iteration);
    crate::spline::refit_splines(sketch);
    crate::text::refresh(sketch);
    (iteration.outcome, dof)
}

fn solve_prepared(sketch: &mut Sketch, sys: System) -> (SolveOutcome, Option<Measured>) {
    let solved = solve_system(sketch, sys);
    crate::spline::refit_splines(sketch);
    crate::text::refresh(sketch);
    solved
}

/// [`solve`], with the points `held` staying where they are (points being
/// dragged: the rest of the sketch gives way to them). When the constraints
/// cannot hold them there, the plain solve decides, from where they were
/// put.
pub fn solve_holding(sketch: &mut Sketch, held: &[Uuid]) -> SolveOutcome {
    if held.is_empty() {
        return solve(sketch);
    }
    crate::text::strip(sketch);
    let before = sketch.geometry.clone();
    let sys = build_system_holding(sketch, None, held);
    match solve_system(sketch, sys).0 {
        SolveOutcome::NotConverged { .. } => {
            sketch.geometry = before;
            solve(sketch)
        }
        outcome => {
            crate::spline::refit_splines(sketch);
            crate::text::refresh(sketch);
            outcome
        }
    }
}

fn solve_system(sketch: &mut Sketch, sys: System) -> (SolveOutcome, Option<Measured>) {
    let iteration = iterate(&sys, sketch.solver);
    let dof = apply(sketch, &sys, &iteration);
    (iteration.outcome, dof)
}

/// Numerical output before it is stored and the application updates. Even
/// a stalled attempt exposes its iteration count and effective threshold.
struct Iteration {
    values: Vec<f64>,
    outcome: SolveOutcome,
    iterations: usize,
    residual: f64,
    threshold: f64,
}

/// Store the answer and measure flags on the stored geometry. Returns
/// the freedom measured on it, when one was.
fn apply(sketch: &mut Sketch, sys: &System, iteration: &Iteration) -> Option<Measured> {
    let measure = |sketch: &Sketch| {
        let prepared = Prepared::of(sketch);
        Measured {
            dof: prepared.dof(),
            prepared,
        }
    };
    if iteration.outcome == SolveOutcome::NothingToSolve {
        // Nothing to solve can still leave nothing free: a line drawn on
        // the ends of projected geometry, which the solver holds, cannot
        // move though no constraint names it.
        let measured = (sketch.geometry.iter())
            .any(|g| !sketch.is_external(g.id()))
            .then(|| measure(sketch));
        sketch.is_fully_constrained = measured.as_ref().is_some_and(|m| m.dof == 0);
        sketch.unsolved = false;
        return measured;
    }
    debug_assert!(iteration.residual.is_finite());
    debug_assert!(iteration.threshold.is_finite());
    debug_assert!(iteration.iterations <= sketch.solver.max_iterations.max(1) as usize);
    write_back(sketch, sys, &iteration.values);
    let measured =
        matches!(iteration.outcome, SolveOutcome::Converged { .. }).then(|| measure(sketch));
    sketch.is_fully_constrained = measured.as_ref().is_some_and(|m| m.dof == 0);
    sketch.unsolved = matches!(iteration.outcome, SolveOutcome::NotConverged { .. });
    measured
}

fn iterate(sys: &System, settings: SolverSettings) -> Iteration {
    let result = sketch_solver::solve::iterate(sys, adapter::settings(settings));
    Iteration {
        values: result.values,
        outcome: from_core(result.outcome),
        iterations: result.iterations,
        residual: result.residual,
        threshold: result.threshold,
    }
}

fn from_core(outcome: sketch_solver::solve::SolveOutcome) -> SolveOutcome {
    match outcome {
        sketch_solver::solve::SolveOutcome::Converged { iterations } => {
            SolveOutcome::Converged { iterations }
        }
        sketch_solver::solve::SolveOutcome::NotConverged { residual } => {
            SolveOutcome::NotConverged { residual }
        }
        sketch_solver::solve::SolveOutcome::NothingToSolve => SolveOutcome::NothingToSolve,
    }
}

fn to_core(outcome: SolveOutcome) -> sketch_solver::solve::SolveOutcome {
    match outcome {
        SolveOutcome::Converged { iterations } => {
            sketch_solver::solve::SolveOutcome::Converged { iterations }
        }
        SolveOutcome::NotConverged { residual } => {
            sketch_solver::solve::SolveOutcome::NotConverged { residual }
        }
        SolveOutcome::NothingToSolve => sketch_solver::solve::SolveOutcome::NothingToSolve,
    }
}

/// Whether the sketch's geometry already meets its constraints, by the
/// solver's own convergence test at the stored values, without solving.
pub fn holds(sketch: &Sketch) -> bool {
    let sys = build_system(sketch);
    if sys.specs.is_empty() || sys.vars.is_empty() {
        return true;
    }
    let tolerance = adapter::settings(sketch.solver).tolerance;
    let residual = sketch_solver::solve::eval_residuals(&sys, &sys.vars);
    sketch_solver::solve::inf_norm(&residual)
        < tolerance * sketch_solver::solve::var_scale(&sys.vars)
}

/// Whether the sketch is small enough for [`diagnose`] to look for
/// redundant and conflicting constraints.
pub fn diagnosable(sketch: &Sketch) -> bool {
    adapter::constraint_count(sketch) <= adapter::settings(sketch.solver).diagnosis_limit
}

pub fn dof_estimate(sketch: &Sketch) -> i32 {
    Prepared::of(sketch).dof()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagnosis {
    pub dof: i32,
    pub redundant: Vec<Uuid>,
    pub conflicting: Vec<Uuid>,
    pub analyzed: bool,
}

/// A sketch solved from where it stands for an analysis of it, its
/// diagnosis and the elements nothing left free can move: its texts'
/// letters left out and not made again, its fitted splines not refitted,
/// no freedom measured. It answers the same for every sketch equal to
/// `sketch` but in where its labels sit.
#[derive(Debug, Clone)]
pub struct Settled {
    pub sketch: Sketch,
    pub outcome: SolveOutcome,
    /// The solver's problem of `sketch`, once first asked for.
    prepared: std::sync::OnceLock<Prepared>,
}

impl Settled {
    fn prepared(&self) -> &Prepared {
        self.prepared.get_or_init(|| Prepared::of(&self.sketch))
    }
}

pub fn settle(sketch: &Sketch) -> Settled {
    settle_with(sketch, None)
}

/// [`settle`], given [`Prepared::of`] `sketch` where the caller keeps it.
pub fn settle_with(sketch: &Sketch, prepared: Option<&Prepared>) -> Settled {
    let mut solved = sketch.clone();
    crate::text::strip(&mut solved);
    // With no letters to strip, the problem is the one prepared.
    let kept = prepared
        .filter(|_| sketch.derived.is_empty())
        .and_then(Prepared::plain);
    let owned;
    let sys = match kept {
        Some(sys) => sys,
        None => {
            owned = build_system(&solved);
            &owned
        }
    };
    let iteration = iterate(sys, solved.solver);
    if iteration.outcome != SolveOutcome::NothingToSolve {
        write_back(&mut solved, sys, &iteration.values);
    }
    Settled {
        sketch: solved,
        outcome: iteration.outcome,
        prepared: Default::default(),
    }
}

pub fn diagnose(sketch: &Sketch) -> Diagnosis {
    diagnose_with(sketch, None, None, None)
}

/// [`diagnose`], given [`settle`] of `sketch`, its [`dof_estimate`] and
/// [`Prepared::of`] it where the caller keeps them.
pub fn diagnose_with(
    sketch: &Sketch,
    settled: Option<&Settled>,
    dof: Option<i32>,
    prepared: Option<&Prepared>,
) -> Diagnosis {
    let owned_prepared;
    let prepared = match prepared {
        Some(prepared) => prepared,
        None => {
            owned_prepared = Prepared::of(sketch);
            &owned_prepared
        }
    };
    let dof = dof.unwrap_or_else(|| prepared.dof());
    if prepared.too_large() {
        return Diagnosis {
            dof,
            analyzed: false,
            ..Diagnosis::default()
        };
    }
    // Solved as `solve` solves it: settled, then its splines refitted and
    // its letters made again; or with its measured points settling.
    let owned_settled;
    let owned_solved;
    let (outcome, solved) = if sketch.soft.is_empty() {
        let settled = match settled {
            Some(settled) => settled,
            None => {
                owned_settled = settle_with(sketch, Some(prepared));
                &owned_settled
            }
        };
        if laid_out(&settled.sketch) {
            let mut probe = settled.sketch.clone();
            crate::spline::refit_splines(&mut probe);
            crate::text::refresh(&mut probe);
            owned_solved = Prepared::of(&probe);
            (settled.outcome, &owned_solved)
        } else {
            (settled.outcome, settled.prepared())
        }
    } else {
        let mut probe = sketch.clone();
        let outcome = solve(&mut probe);
        owned_solved = Prepared::of(&probe);
        (outcome, &owned_solved)
    };
    // The system the diagnosis compiles, where one is kept.
    let full = match outcome {
        SolveOutcome::NotConverged { .. } => prepared.plain(),
        _ => solved.plain(),
    };
    let result = sketch_solver::diagnosis::diagnose_compiled(
        &prepared.problem,
        to_core(outcome),
        &solved.problem,
        dof,
        full,
    );
    Diagnosis {
        dof: result.dof,
        analyzed: result.analyzed,
        redundant: result
            .redundant
            .into_iter()
            .map(|id| Uuid::from_u128(id.0))
            .collect(),
        conflicting: result
            .conflicting
            .into_iter()
            .map(|id| Uuid::from_u128(id.0))
            .collect(),
    }
}

fn build_system(sketch: &Sketch) -> System {
    build_system_with(sketch, None, &[], false)
}

fn build_system_holding(sketch: &Sketch, exclude: Option<Uuid>, held: &[Uuid]) -> System {
    build_system_with(sketch, exclude, held, false)
}

fn build_system_with(
    sketch: &Sketch,
    exclude: Option<Uuid>,
    held: &[Uuid],
    arcs_always: bool,
) -> System {
    sketch_solver::compile::compile_system(
        &adapter::prepare(sketch, held),
        exclude.map(|id| sketch_solver::problem::ConstraintId(id.as_u128())),
        arcs_always,
    )
}
/// The older constraints the constraints `new` (already in the sketch)
/// made redundant: those to take away so the new ones say something of
/// their own. One is taken at a time, the most recently added first, until
/// no new constraint is redundant; an older constraint that was redundant
/// before the new ones came is never one of them. Empty when the sketch is
/// too large to diagnose or the new constraints conflict instead.
pub fn superseded(sketch: &Sketch, new: &[Uuid]) -> Vec<Uuid> {
    let mut before = sketch.clone();
    before.constraints.retain(|c| !new.contains(&c.id));
    let already = diagnose(&before);
    if !already.analyzed {
        return Vec::new();
    }
    let mut work = sketch.clone();
    let mut removed = Vec::new();
    loop {
        let diagnosis = diagnose(&work);
        if !diagnosis.analyzed || !new.iter().any(|id| diagnosis.redundant.contains(id)) {
            break;
        }
        let Some(older) = work.constraints.iter().rev().map(|c| c.id).find(|id| {
            !new.contains(id) && diagnosis.redundant.contains(id) && !already.redundant.contains(id)
        }) else {
            break;
        };
        work.constraints.retain(|c| c.id != older);
        removed.push(older);
    }
    removed
}

/// Write the solved variables back into the sketch geometry.
fn write_back(sketch: &mut Sketch, sys: &System, x: &[f64]) {
    let mut turned = std::collections::HashSet::new();
    for element in &mut sketch.geometry {
        match element {
            GeometryElement::Point(p) => {
                if let Some(&i) = sys.point_vars.get(&adapter::point(p.id)) {
                    p.position = Vec2D::new(x[i], x[i + 1]);
                }
            }
            GeometryElement::Circle(c) => {
                if let Some(&i) = sys.radius_vars.get(&adapter::curve(c.id)) {
                    c.radius = x[i];
                }
            }
            GeometryElement::Arc(a) => {
                if let Some(&i) = sys.radius_vars.get(&adapter::curve(a.id)) {
                    a.radius = x[i];
                }
            }
            // A minor radius dragged past the major one makes it the major:
            // the axes trade places, the same ellipse.
            GeometryElement::Ellipse(e) => {
                if let Some(&k) = sys.shape_vars.get(&adapter::curve(e.id)) {
                    let major = Vec2D::new(x[k], x[k + 1]);
                    let a = major.to_glam().length();
                    let b = x[k + 2].abs();
                    if a > 1e-9 {
                        let (axis, ratio) = crate::geom2d::ellipse_axes(major.to_glam(), b);
                        if b > a {
                            turned.insert(e.id);
                        }
                        e.major = Vec2D::new(axis.x, axis.y);
                        e.ratio = ratio;
                    }
                }
            }
            GeometryElement::Conic(c) => {
                if let Some(&k) = sys.shape_vars.get(&adapter::curve(c.id)) {
                    let axis = Vec2D::new(x[k], x[k + 1]);
                    if axis.to_glam().length() > 1e-9 {
                        c.axis = axis;
                        c.minor = x[k + 2].abs();
                    }
                }
            }
            GeometryElement::Line(_) | GeometryElement::BSpline(_) => {}
        }
    }
    // When an ellipse's axes trade places, what names one of them is
    // turned to name the other: its radius dimensions and its shown axis
    // lines, so the next solve holds the same ellipse rather than trading
    // them back. The frame turns a quarter, so the axis that was the major
    // runs the other way as the minor: its line's ends trade too.
    let mut reversed = Vec::new();
    for c in &mut sketch.constraints {
        match &mut c.kind {
            ConstraintKind::EllipseRadius { ellipse, major, .. } if turned.contains(ellipse) => {
                *major = !*major;
            }
            ConstraintKind::InternalAlignment {
                curve,
                role,
                element,
            } if turned.contains(curve) => match role {
                InternalRole::MajorAxis => {
                    *role = InternalRole::MinorAxis;
                    reversed.push(*element);
                }
                InternalRole::MinorAxis => *role = InternalRole::MajorAxis,
                _ => {}
            },
            _ => {}
        }
    }
    for id in reversed {
        if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) {
            std::mem::swap(&mut l.start, &mut l.end);
        }
    }
    // An ellipse's first focus is the one on its major vertex's side: a
    // focus that came out on the other (through a circle, or past the
    // centre) takes the other name.
    let sides: Vec<(usize, InternalRole)> = sketch
        .constraints
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let ConstraintKind::InternalAlignment {
                curve,
                role: InternalRole::Focus1 | InternalRole::Focus2,
                element,
            } = c.kind
            else {
                return None;
            };
            if sys.derived_foci.contains(&adapter::point(element)) {
                return None;
            }
            let Some(GeometryElement::Ellipse(e)) = sketch.get_geometry(curve) else {
                return None;
            };
            let at = sketch.point_position(element)?.to_glam();
            let centre = sketch.point_position(e.center)?.to_glam();
            let axis = e.major.to_glam();
            let along = (at - centre).dot(axis) / axis.length().max(1e-9);
            if along.abs() <= 1e-6 * axis.length() {
                return None;
            }
            let role = if along > 0.0 {
                InternalRole::Focus1
            } else {
                InternalRole::Focus2
            };
            Some((i, role))
        })
        .collect();
    for (i, side) in sides {
        if let ConstraintKind::InternalAlignment { role, .. } = &mut sketch.constraints[i].kind {
            *role = side;
        }
    }
    let derived = sys
        .derived_foci
        .iter()
        .map(|id| match id {
            sketch_solver::problem::PointReference::Point(id) => Uuid::from_u128(id.0),
            sketch_solver::problem::PointReference::Origin => crate::sketch::ORIGIN_ID,
        })
        .collect();
    place_derived_foci(sketch, &derived);
}

/// The foci the solve left out, placed from their ellipse as it stands: a
/// focus the solve placed has its partner opposite it through the centre,
/// under the other name; otherwise the first stands `√(a² − b²)` from the
/// centre toward the major vertex and the second as far the other way.
fn place_derived_foci(sketch: &mut Sketch, derived: &std::collections::HashSet<Uuid>) {
    if derived.is_empty() {
        return;
    }
    let foci: Vec<(usize, Uuid, InternalRole, Uuid)> = sketch
        .constraints
        .iter()
        .enumerate()
        .filter_map(|(i, c)| match c.kind {
            ConstraintKind::InternalAlignment {
                curve,
                role: role @ (InternalRole::Focus1 | InternalRole::Focus2),
                element,
            } => Some((i, curve, role, element)),
            _ => None,
        })
        .collect();
    for &(i, curve, role, element) in &foci {
        if !derived.contains(&element) {
            continue;
        }
        let Some(GeometryElement::Ellipse(e)) = sketch.get_geometry(curve) else {
            continue;
        };
        let Some(centre) = sketch.point_position(e.center).map(|p| p.to_glam()) else {
            continue;
        };
        let partner = foci.iter().find_map(|&(_, c, r, p)| {
            (c == curve && p != element && !derived.contains(&p))
                .then(|| sketch.point_position(p).map(|at| (r, at.to_glam())))
                .flatten()
        });
        let (role, at) = match partner {
            Some((other, at)) => {
                let role = if other == InternalRole::Focus1 {
                    InternalRole::Focus2
                } else {
                    InternalRole::Focus1
                };
                (role, centre * 2.0 - at)
            }
            None => {
                let axis = e.major.to_glam();
                let a = axis.length();
                let b = a * e.ratio;
                let f = (a * a - b * b).max(0.0).sqrt();
                let sign = if role == InternalRole::Focus1 {
                    1.0
                } else {
                    -1.0
                };
                (role, centre + axis.normalize_or_zero() * f * sign)
            }
        };
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(element) {
            p.position = Vec2D::from_glam(at);
        }
        if let ConstraintKind::InternalAlignment { role: r, .. } = &mut sketch.constraints[i].kind {
            *r = role;
        }
    }
}
/// The elements nothing left free can move: a point fixed in both
/// coordinates, a curve whose points are all fixed and whose own size
/// (a radius, an ellipse's shape) is too. Worked out on the sketch as it
/// solves, in one pass over the constraints' Jacobian. Empty for a sketch
/// too large to analyse, as its diagnosis is. `settled` is [`settle`] of
/// `sketch` where the caller keeps it.
pub fn locked_elements(
    sketch: &Sketch,
    settled: Option<&Settled>,
) -> std::collections::HashSet<Uuid> {
    // Settled: solved without its texts' letters, which the solver leaves
    // out and which would only be made again after it. Too large is known
    // before the solve, which would be wasted on it.
    let owned;
    let settled = match settled {
        Some(settled) => settled,
        None if !diagnosable(sketch) => return Default::default(),
        None => {
            owned = settle(sketch);
            &owned
        }
    };
    let prepared = settled.prepared();
    if prepared.too_large() {
        return Default::default();
    }
    locked_in(sketch, prepared)
}

/// [`locked_elements`] of `sketch` from `prepared`, the solver's problem
/// of it settled, at any size. What the application lays out after a solve
/// follows what it is laid out from: a text's letters are locked when its
/// point and its frame's corners or the curves it runs along are, and the
/// controls of a spline refitted through its fit points when those
/// points are.
fn locked_in(sketch: &Sketch, prepared: &Prepared) -> std::collections::HashSet<Uuid> {
    use sketch_solver::problem::{CurveReference, PointReference};
    let locked = sketch_solver::freedom::locked_in(&prepared.problem, &prepared.rank().system);
    let point = |id: Uuid| {
        locked
            .points
            .contains(&PointReference::from(sketch_solver::problem::PointId(
                id.as_u128(),
            )))
    };
    let curve = |id: Uuid| {
        locked
            .curves
            .contains(&CurveReference::from(sketch_solver::problem::CurveId(
                id.as_u128(),
            )))
    };
    let letters: std::collections::HashSet<Uuid> = (sketch.texts.iter())
        .flat_map(|b| b.elements.iter().copied())
        .collect();
    let refitted: std::collections::HashSet<Uuid> =
        crate::spline::derived_points(sketch, &crate::spline::solved_fits(sketch))
            .into_iter()
            .collect();
    let mut locked: std::collections::HashSet<Uuid> = sketch
        .geometry
        .iter()
        .filter(|g| !sketch.derived.contains(&g.id()) && !letters.contains(&g.id()))
        .filter(|g| !refitted.contains(&g.id()))
        .filter(|g| match g {
            GeometryElement::Point(p) => point(p.id),
            other => curve(other.id()) && Sketch::curve_point_ids(other).into_iter().all(point),
        })
        .map(GeometryElement::id)
        .collect();
    let controls: Vec<Uuid> = (sketch.geometry.iter())
        .filter_map(|g| match g {
            GeometryElement::BSpline(b) if b.fit_points.iter().all(|f| locked.contains(f)) => {
                Some(b.control_points.iter().filter(|c| refitted.contains(c)))
            }
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    locked.extend(controls);
    let still: Vec<Uuid> = sketch
        .texts
        .iter()
        .filter(|b| {
            std::iter::once(b.anchor)
                .chain(b.supports())
                .all(|id| locked.contains(&id))
        })
        .flat_map(|b| b.elements.iter().copied())
        .collect();
    locked.extend(still);
    locked
}

/// The points the constraints leave free to move: every point of the
/// sketch but those [`locked_elements`] finds, at any size. A sketch
/// nothing pins down reports every point free, which is what its three
/// rigid degrees of freedom mean; pinned external geometry is never free.
pub fn free_points(sketch: &Sketch) -> std::collections::HashSet<Uuid> {
    let locked = locked_in(sketch, settle(sketch).prepared());
    (sketch.geometry.iter())
        .filter_map(|g| match g {
            GeometryElement::Point(p) if !locked.contains(&p.id) => Some(p.id),
            _ => None,
        })
        .collect()
}
