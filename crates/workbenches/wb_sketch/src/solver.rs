//! Application adapter for the standalone sketch constraint solver.

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
/// Result of a constraint solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SolveOutcome {
    /// All residuals below tolerance.
    Converged { iterations: usize },
    /// Iteration limit hit with residual norm still above tolerance.
    NotConverged { residual: f64 },
    /// No constraints (or none referencing existing geometry).
    NothingToSolve,
}

/// Solve the sketch's constraints by adjusting point positions and
/// circle/arc radii in place. On NotConverged the best-effort geometry is
/// still written back. Also updates `sketch.is_fully_constrained`.
pub fn solve(sketch: &mut Sketch) -> SolveOutcome {
    let sys = build_system(sketch);
    let outcome = solve_system(sketch, sys);
    crate::spline::refit_splines(sketch);
    crate::text::follow(sketch);
    outcome
}

/// [`solve`], with the points `held` staying where they are (points being
/// dragged: the rest of the sketch gives way to them). When the constraints
/// cannot hold them there, the plain solve decides, from where they were
/// put.
pub fn solve_holding(sketch: &mut Sketch, held: &[Uuid]) -> SolveOutcome {
    if held.is_empty() {
        return solve(sketch);
    }
    let before = sketch.geometry.clone();
    let sys = build_system_holding(sketch, None, held);
    match solve_system(sketch, sys) {
        SolveOutcome::NotConverged { .. } => {
            sketch.geometry = before;
            solve(sketch)
        }
        outcome => {
            crate::spline::refit_splines(sketch);
            crate::text::follow(sketch);
            outcome
        }
    }
}

fn solve_system(sketch: &mut Sketch, sys: System) -> SolveOutcome {
    let iteration = iterate(&sys, sketch.solver);
    apply(sketch, &sys, &iteration);
    iteration.outcome
}

/// Numerical output before storage rounding and application updates. Even
/// a stalled attempt exposes its iteration count and effective threshold.
struct Iteration {
    values: Vec<f64>,
    outcome: SolveOutcome,
    iterations: usize,
    residual: f64,
    threshold: f64,
}

/// Store the answer and measure flags on the stored geometry.
fn apply(sketch: &mut Sketch, sys: &System, iteration: &Iteration) {
    if iteration.outcome == SolveOutcome::NothingToSolve {
        // Nothing to solve can still leave nothing free: a line drawn on
        // the ends of projected geometry, which the solver holds, cannot
        // move though no constraint names it.
        sketch.is_fully_constrained = sketch.geometry.iter().any(|g| !sketch.is_external(g.id()))
            && dof_estimate(sketch) == 0;
        sketch.unsolved = false;
        return;
    }
    debug_assert!(iteration.residual.is_finite());
    debug_assert!(iteration.threshold.is_finite());
    debug_assert!(iteration.iterations <= sketch.solver.max_iterations.max(1) as usize);
    write_back(sketch, sys, &iteration.values);
    sketch.is_fully_constrained =
        matches!(iteration.outcome, SolveOutcome::Converged { .. }) && dof_estimate(sketch) == 0;
    sketch.unsolved = matches!(iteration.outcome, SolveOutcome::NotConverged { .. });
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

pub fn dof_estimate(sketch: &Sketch) -> i32 {
    sketch_solver::freedom::estimate(&adapter::prepare(sketch, &[])).degrees
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagnosis {
    pub dof: i32,
    pub redundant: Vec<Uuid>,
    pub conflicting: Vec<Uuid>,
    pub analyzed: bool,
}

pub fn diagnose(sketch: &Sketch) -> Diagnosis {
    let problem = adapter::prepare(sketch, &[]);
    if problem.constraints.len() > problem.settings.diagnosis_limit {
        return Diagnosis {
            dof: sketch_solver::freedom::estimate(&problem).degrees,
            analyzed: false,
            ..Diagnosis::default()
        };
    }
    let mut probe = sketch.clone();
    let outcome = solve(&mut probe);
    let solved = adapter::prepare(&probe, &[]);
    let result = sketch_solver::diagnosis::diagnose_at(&problem, to_core(outcome), &solved);
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
                    p.position = Vec2D::new(x[i] as f32, x[i + 1] as f32);
                }
            }
            GeometryElement::Circle(c) => {
                if let Some(&i) = sys.radius_vars.get(&adapter::curve(c.id)) {
                    c.radius = x[i] as f32;
                }
            }
            GeometryElement::Arc(a) => {
                if let Some(&i) = sys.radius_vars.get(&adapter::curve(a.id)) {
                    a.radius = x[i] as f32;
                }
            }
            // A minor radius dragged past the major one makes it the major:
            // the axes trade places, the same ellipse.
            GeometryElement::Ellipse(e) => {
                if let Some(&k) = sys.shape_vars.get(&adapter::curve(e.id)) {
                    let major = Vec2D::new(x[k] as f32, x[k + 1] as f32);
                    let a = major.to_glam().length();
                    let b = (x[k + 2] as f32).abs();
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
                    let axis = Vec2D::new(x[k] as f32, x[k + 1] as f32);
                    if axis.to_glam().length() > 1e-9 {
                        c.axis = axis;
                        c.minor = (x[k + 2] as f32).abs();
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
/// The points the constraints leave free to move: each is nudged in turn
/// and the sketch re-solved; a point the solver leaves where it was
/// nudged has a degree of freedom in that direction. A sketch nothing
/// pins down reports every point free, which is what its three rigid
/// degrees of freedom mean.
pub fn free_points(sketch: &Sketch) -> std::collections::HashSet<Uuid> {
    const NUDGE: f32 = 0.25;
    let mut base = sketch.clone();
    solve(&mut base);
    let points: Vec<(Uuid, Vec2D)> = base
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((p.id, p.position)),
            _ => None,
        })
        .collect();
    let mut free = std::collections::HashSet::new();
    for (id, at) in points {
        for nudge in [Vec2D::new(NUDGE, 0.0), Vec2D::new(0.0, NUDGE)] {
            let mut probe = base.clone();
            for g in &mut probe.geometry {
                if let GeometryElement::Point(p) = g
                    && p.id == id
                {
                    p.position = Vec2D::new(at.x + nudge.x, at.y + nudge.y);
                }
            }
            solve(&mut probe);
            let Some(after) = probe.point_position(id) else {
                continue;
            };
            let moved = ((after.x - at.x).powi(2) + (after.y - at.y).powi(2)).sqrt();
            if moved > NUDGE * 0.5 {
                free.insert(id);
                break;
            }
        }
    }
    free
}
