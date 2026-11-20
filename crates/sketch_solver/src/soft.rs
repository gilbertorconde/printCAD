//! Soft data: measured points that settle what the constraints leave
//! free, by least squares, without ever trading a constraint for a better
//! fit. A line fitted to a scan and dimensioned at 20 mm stays exactly 20
//! mm long; its points then decide where it lies and how it turns.
//!
//! The constraints are solved first, as [`crate::solve::solve`] does. Each
//! step then minimises the points' squared distances from their curves
//! subject to the constraints' linearisation (a damped step in their null
//! space, from the KKT system), and the constraints are solved again from
//! where it lands, so they stay met to the solver's tolerance. A step that
//! does not bring the points closer is taken back.

use crate::compile::System;
use crate::input::InputError;
use crate::problem::{
    Constraint, ConstraintId, CurveReference, Geometry, Point, PointId, PointReference, Problem,
    Relation, Vector,
};
use crate::solve::{Dense, Iteration, SolveOutcome, eval_residuals, iterate_in};
use crate::sparse::{Block, Blocks, Matrix};
use crate::trace::EquationSource;

/// A measured point that a curve should pass near.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SoftPoint {
    /// A line, a circle or an arc.
    pub curve: CurveReference,
    pub at: Vector,
}

/// A solve with soft data.
#[derive(Clone)]
pub struct SoftAnswer {
    /// The system the values are of: the problem's, with the soft points
    /// as held points after its own.
    pub system: System,
    /// The constraints' solve, its values settled by the points.
    pub iteration: Iteration,
    /// The points' root mean square distance from their curves, before
    /// and after.
    pub rms: [f64; 2],
    /// Points whose curve is not a line, circle or arc, left out.
    pub skipped: usize,
}

/// How many soft steps at most.
const STEPS: usize = 100;
/// The KKT system's regularisation of the constraint rows, so redundant
/// constraints do not make it singular; the solve after each step puts
/// back what it gives up.
const RIDGE: f64 = 1e-12;

/// `problem`'s constraints met exactly, and the freedom they leave settled
/// by `soft`'s points in the least-squares sense.
pub fn solve_soft(problem: &Problem, soft: &[SoftPoint]) -> Result<SoftAnswer, InputError> {
    let (augmented, ids, skipped) = augment(problem, soft);
    let sys = crate::compile::compile(&augmented)?;
    let (hard, fit) = split(&sys, &ids);
    let settings = problem.settings;
    // The constraints' blocks, shared by every solve of them below: only
    // their values change from one to the next.
    let hard_blocks = Blocks::of(&hard);
    let mut x = if hard.specs.is_empty() {
        sys.vars.clone()
    } else {
        let first = iterate_in(&hard, Some(&hard_blocks), settings);
        if !matches!(first.outcome, SolveOutcome::Converged { .. }) {
            let rms = rms(&fit, &first.values);
            return Ok(SoftAnswer {
                iteration: first,
                rms: [rms; 2],
                skipped,
                system: sys,
            });
        }
        first.values
    };
    let start = rms(&fit, &x);
    let mut cost = sq(&eval_residuals(&fit, &x));
    let n = sys.free.len();
    let mut lambda = settings.lambda_initial.max(1e-6);
    let mut steps = 0;
    // The step's KKT system is block diagonal over the clusters both
    // kinds of equations join: each block is solved alone, its fit rows
    // making its normal equations and its constraint rows its
    // multipliers. A variable in no block takes no step.
    let blocks = Blocks::of(&sys);
    let rows = Rows::of(&sys, &ids);
    let mut again = hard.clone();
    again.settings = settings;
    let mut dense = Dense::default();
    for _ in 0..STEPS {
        if fit.residual_len == 0 || n == 0 {
            break;
        }
        steps += 1;
        let (rs, rh) = (eval_residuals(&fit, &x), eval_residuals(&hard, &x));
        let parts: Vec<Part> = (blocks.blocks.iter())
            .zip(blocks.jacobian(&sys, &x))
            .map(|(block, jac)| Part::of(block, jac, &rows, &rs, &rh))
            .collect();
        // Summed in column order, as one matrix's diagonal would be.
        let mut diagonal = vec![0.0; n];
        for (block, part) in blocks.blocks.iter().zip(&parts) {
            for (i, &column) in block.columns.iter().enumerate() {
                diagonal[column] = part.jtj[i][i];
            }
        }
        let mean = diagonal.iter().sum::<f64>() / n as f64;
        let mut improved = false;
        for _ in 0..settings.max_inner_retries {
            let mut step = vec![0.0; n];
            let mut singular = false;
            for (block, part) in blocks.blocks.iter().zip(&parts) {
                let damping = |d: f64| lambda * (d + mean).max(settings.damping_floor);
                if !part.step(damping, &mut dense) {
                    singular = true;
                    break;
                }
                for (&column, &dx) in block.columns.iter().zip(&dense.x) {
                    step[column] = dx;
                }
            }
            if singular {
                lambda = (lambda * 10.0).min(settings.lambda_max);
                continue;
            }
            let mut trial = x.clone();
            for (column, &j) in sys.free.iter().enumerate() {
                trial[j] += step[column];
            }
            if !hard.specs.is_empty() {
                again.vars = trial;
                let back = iterate_in(&again, Some(&hard_blocks), settings);
                if !matches!(
                    back.outcome,
                    SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
                ) {
                    lambda = (lambda * 10.0).min(settings.lambda_max);
                    continue;
                }
                trial = back.values;
            }
            let trial_cost = sq(&eval_residuals(&fit, &trial));
            if trial_cost.is_finite() && trial_cost < cost {
                let gain = cost - trial_cost;
                x = trial;
                cost = trial_cost;
                lambda = (lambda / 10.0).max(settings.lambda_min);
                improved = gain > 1e-14 * cost.max(1e-30);
                break;
            }
            lambda = (lambda * 10.0).min(settings.lambda_max);
        }
        if !improved {
            break;
        }
    }
    let residual = crate::solve::inf_norm(&eval_residuals(&hard, &x));
    let end = rms(&fit, &x);
    Ok(SoftAnswer {
        iteration: Iteration {
            threshold: settings.tolerance * crate::solve::var_scale(&x),
            values: x,
            outcome: SolveOutcome::Converged { iterations: steps },
            iterations: steps,
            residual,
        },
        rms: [start, end],
        skipped,
        system: sys,
    })
}

/// `problem` with each soft point as a held point on its curve, and the
/// ids of the constraints that tie them there.
fn augment(problem: &Problem, soft: &[SoftPoint]) -> (Problem, Vec<ConstraintId>, usize) {
    let mut out = problem.clone();
    let mut point_id = problem
        .geometry
        .iter()
        .filter_map(|g| match g.id() {
            crate::problem::ItemReference::Point(PointReference::Point(PointId(p))) => Some(p),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut constraint_id = problem
        .constraints
        .iter()
        .map(|c| c.id.0)
        .max()
        .unwrap_or(0);
    let indexed = crate::problem::Indexed::new(problem);
    let mut ids = Vec::new();
    let mut skipped = 0;
    for s in soft {
        let relation = |point| match indexed.get_geometry(s.curve) {
            Some(Geometry::Line(_)) => Some(Relation::PointOnLine {
                point,
                line: s.curve,
            }),
            Some(Geometry::Circle(_) | Geometry::Arc(_)) => Some(Relation::PointOnCircle {
                point,
                circle: s.curve,
            }),
            _ => None,
        };
        point_id += 1;
        let point = PointReference::Point(PointId(point_id));
        let Some(kind) = relation(point) else {
            skipped += 1;
            continue;
        };
        out.geometry.push(Geometry::Point(Point {
            id: PointId(point_id),
            position: s.at,
        }));
        out.held_points.push(point);
        constraint_id += 1;
        out.constraints.push(Constraint {
            id: ConstraintId(constraint_id),
            kind,
        });
        ids.push(ConstraintId(constraint_id));
    }
    (out, ids, skipped)
}

/// The constraints' system and the soft points' system, sharing the
/// variables.
fn split(sys: &System, soft: &[ConstraintId]) -> (System, System) {
    let soft: std::collections::HashSet<_> = soft.iter().collect();
    let (mut hard, mut fit) = (
        sys.clone_with(sys.vars.clone()),
        sys.clone_with(sys.vars.clone()),
    );
    for part in [&mut hard, &mut fit] {
        part.specs.clear();
        part.origins.clear();
        part.residual_len = 0;
    }
    let mut row = 0;
    for spec in &sys.specs {
        let dim = spec.dim();
        let source = sys.origins.get(row).cloned();
        let into = match &source {
            Some(EquationSource::Constraint(id)) if soft.contains(id) => &mut fit,
            _ => &mut hard,
        };
        into.specs.push(spec.clone());
        into.origins
            .extend(sys.origins[row..row + dim].iter().cloned());
        into.residual_len += dim;
        row += dim;
    }
    (hard, fit)
}

/// Where each row of the whole system lies: in the soft points' residuals
/// or the constraints', and its place there.
struct Rows {
    fit: Vec<bool>,
    place: Vec<usize>,
}

impl Rows {
    fn of(sys: &System, soft: &[ConstraintId]) -> Self {
        let soft: std::collections::HashSet<_> = soft.iter().collect();
        let (mut fit, mut place) = (Vec::new(), Vec::new());
        let (mut fits, mut holds) = (0, 0);
        let mut row = 0;
        for spec in &sys.specs {
            let dim = spec.dim();
            let soft = matches!(
                sys.origins.get(row),
                Some(EquationSource::Constraint(id)) if soft.contains(id)
            );
            for _ in 0..dim {
                fit.push(soft);
                let count = if soft { &mut fits } else { &mut holds };
                place.push(*count);
                *count += 1;
            }
            row += dim;
        }
        Self { fit, place }
    }
}

/// One block's share of a soft step: the normal equations of its fit
/// rows and its constraint rows with their residuals.
struct Part {
    jtj: Matrix,
    g: Vec<f64>,
    jh: Matrix,
    rh: Vec<f64>,
}

impl Part {
    fn of(block: &Block, jac: Matrix, rows: &Rows, rs: &[f64], rh: &[f64]) -> Self {
        let n = block.columns.len();
        let mut jtj = vec![vec![0.0; n]; n];
        let mut g = vec![0.0; n];
        let (mut jh, mut hr) = (Vec::new(), Vec::new());
        for (&row, values) in block.rows.iter().zip(jac) {
            let place = rows.place[row];
            if rows.fit[row] {
                let r = rs[place];
                for i in 0..n {
                    g[i] += values[i] * r;
                    for k in 0..n {
                        jtj[i][k] += values[i] * values[k];
                    }
                }
            } else {
                jh.push(values);
                hr.push(rh[place]);
            }
        }
        Self { jtj, g, jh, rh: hr }
    }

    /// The block's step from its damped KKT system, each variable damped
    /// by `damping` of its curvature, solved in `dense`: its first
    /// variables' entries of `dense.x`.
    fn step(&self, damping: impl Fn(f64) -> f64, dense: &mut Dense) -> bool {
        let (n, m) = (self.g.len(), self.rh.len());
        let w = n + m;
        dense.zeroed(w);
        let k = &mut dense.a;
        for i in 0..n {
            k[i * w..i * w + n].copy_from_slice(&self.jtj[i]);
            k[i * w + i] += damping(self.jtj[i][i]);
            for (row, h) in self.jh.iter().enumerate() {
                k[i * w + n + row] = h[i];
                k[(n + row) * w + i] = h[i];
            }
        }
        for row in 0..m {
            k[(n + row) * w + n + row] = -RIDGE;
        }
        for (to, v) in dense.b.iter_mut().zip(self.g.iter().chain(&self.rh)) {
            *to = -v;
        }
        dense.solve()
    }
}

fn sq(v: &[f64]) -> f64 {
    v.iter().map(|r| r * r).sum()
}

fn rms(fit: &System, x: &[f64]) -> f64 {
    let r = eval_residuals(fit, x);
    if r.is_empty() {
        0.0
    } else {
        (sq(&r) / r.len() as f64).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{CurveId, Length, Line};
    use crate::solve::iterate;

    fn point(id: u128, x: f64, y: f64) -> Geometry {
        Geometry::Point(Point {
            id: PointId(id),
            position: Vector { x, y },
        })
    }

    /// A line held 20 long, with points along y = 1 + 0.1 x from x = 0 to
    /// 20: the length stays exactly 20, and the line turns and moves onto
    /// the points.
    #[test]
    fn soft_points_settle_what_the_constraints_leave_free() {
        let line = CurveReference::Curve(CurveId(10));
        let (a, b) = (
            PointReference::Point(PointId(1)),
            PointReference::Point(PointId(2)),
        );
        let problem = Problem {
            geometry: vec![
                point(1, 0.0, 0.0),
                point(2, 19.0, 0.0),
                Geometry::Line(Line {
                    id: CurveId(10),
                    start: a,
                    end: b,
                }),
            ],
            constraints: vec![Constraint {
                id: ConstraintId(1),
                kind: Relation::Distance {
                    point1: a,
                    point2: b,
                    distance: Length(20.0),
                },
            }],
            ..Problem::default()
        };
        let soft: Vec<SoftPoint> = (0..=20)
            .map(|k| SoftPoint {
                curve: line,
                at: Vector {
                    x: f64::from(k),
                    y: 1.0 + 0.1 * f64::from(k),
                },
            })
            .collect();
        let answer = solve_soft(&problem, &soft).unwrap();
        let sys = crate::compile::compile(&problem).unwrap();
        let at = |p| {
            let i = sys.point_variable(p).unwrap().0;
            (answer.iteration.values[i], answer.iteration.values[i + 1])
        };
        let ((x1, y1), (x2, y2)) = (at(a), at(b));
        assert!(
            ((x2 - x1).hypot(y2 - y1) - 20.0).abs() < 1e-8,
            "the length holds"
        );
        assert!(
            answer.rms[1] < 1e-6 && answer.rms[0] > 0.1,
            "{:?}",
            answer.rms
        );
        assert!(((y2 - y1) / (x2 - x1) - 0.1).abs() < 1e-6);
        assert_eq!(answer.skipped, 0);
    }

    /// The soft solve with its step from one KKT system over every
    /// movable variable, as one dense matrix.
    fn solve_soft_dense(problem: &Problem, soft: &[SoftPoint]) -> Result<SoftAnswer, InputError> {
        let (augmented, ids, skipped) = augment(problem, soft);
        let sys = crate::compile::compile(&augmented)?;
        let (hard, fit) = split(&sys, &ids);
        let settings = problem.settings;
        let mut x = if hard.specs.is_empty() {
            sys.vars.clone()
        } else {
            let first = iterate(&hard, settings);
            if !matches!(first.outcome, SolveOutcome::Converged { .. }) {
                let rms = rms(&fit, &first.values);
                return Ok(SoftAnswer {
                    iteration: first,
                    rms: [rms; 2],
                    skipped,
                    system: sys,
                });
            }
            first.values
        };
        let start = rms(&fit, &x);
        let mut cost = sq(&eval_residuals(&fit, &x));
        let n = sys.free.len();
        let mut lambda = settings.lambda_initial.max(1e-6);
        let mut steps = 0;
        for _ in 0..STEPS {
            if fit.residual_len == 0 || n == 0 {
                break;
            }
            steps += 1;
            let (js, rs) = (crate::solve::jacobian(&fit, &x), eval_residuals(&fit, &x));
            let (jh, rh) = (crate::solve::jacobian(&hard, &x), eval_residuals(&hard, &x));
            let m = rh.len();
            let mut jtj = vec![vec![0.0; n]; n];
            let mut g = vec![0.0; n];
            for (row, r) in js.iter().zip(&rs) {
                for i in 0..n {
                    g[i] += row[i] * r;
                    for k in 0..n {
                        jtj[i][k] += row[i] * row[k];
                    }
                }
            }
            let mean = (0..n).map(|i| jtj[i][i]).sum::<f64>() / n as f64;
            let mut improved = false;
            for _ in 0..settings.max_inner_retries {
                let mut k = vec![vec![0.0; n + m]; n + m];
                for i in 0..n {
                    for j in 0..n {
                        k[i][j] = jtj[i][j];
                    }
                    k[i][i] += lambda * (jtj[i][i] + mean).max(settings.damping_floor);
                    for (row, h) in jh.iter().enumerate() {
                        k[i][n + row] = h[i];
                        k[n + row][i] = h[i];
                    }
                }
                for row in 0..m {
                    k[n + row][n + row] = -RIDGE;
                }
                let rhs: Vec<f64> = g.iter().map(|v| -v).chain(rh.iter().map(|v| -v)).collect();
                let Some(step) = crate::solve::solve_linear(k, rhs) else {
                    lambda = (lambda * 10.0).min(settings.lambda_max);
                    continue;
                };
                let mut trial = x.clone();
                for (column, &j) in sys.free.iter().enumerate() {
                    trial[j] += step[column];
                }
                if !hard.specs.is_empty() {
                    let mut again = hard.clone_with(trial);
                    again.settings = settings;
                    let back = iterate(&again, settings);
                    if !matches!(
                        back.outcome,
                        SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
                    ) {
                        lambda = (lambda * 10.0).min(settings.lambda_max);
                        continue;
                    }
                    trial = back.values;
                }
                let trial_cost = sq(&eval_residuals(&fit, &trial));
                if trial_cost.is_finite() && trial_cost < cost {
                    let gain = cost - trial_cost;
                    x = trial;
                    cost = trial_cost;
                    lambda = (lambda / 10.0).max(settings.lambda_min);
                    improved = gain > 1e-14 * cost.max(1e-30);
                    break;
                }
                lambda = (lambda * 10.0).min(settings.lambda_max);
            }
            if !improved {
                break;
            }
        }
        let residual = crate::solve::inf_norm(&eval_residuals(&hard, &x));
        let end = rms(&fit, &x);
        Ok(SoftAnswer {
            iteration: Iteration {
                threshold: settings.tolerance * crate::solve::var_scale(&x),
                values: x,
                outcome: SolveOutcome::Converged { iterations: steps },
                iterations: steps,
                residual,
            },
            rms: [start, end],
            skipped,
            system: sys,
        })
    }

    /// Several lines and circles, some held to a length or a radius, some
    /// joined end to end, a few fixed, each with measured points near
    /// it and some with none: clusters of either kind of equation alone
    /// and of both.
    fn scattered(seed: u64) -> (Problem, Vec<SoftPoint>) {
        let mut state = seed;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut problem = Problem::default();
        let mut soft = Vec::new();
        let mut id = 0u128;
        let mut fresh = || {
            id += 1;
            id
        };
        let constrain = |problem: &mut Problem, kind: Relation, id: u128| {
            problem.constraints.push(Constraint {
                id: ConstraintId(id),
                kind,
            });
        };
        let mut last_end = None;
        for k in 0..6 {
            let (x, y) = (k as f64 * 30.0, 10.0 * next());
            let a = fresh();
            let b = fresh();
            let line = fresh();
            let start = match last_end {
                Some(end) if next() < 0.4 => end,
                _ => {
                    problem.geometry.push(point(a, x, y));
                    PointReference::Point(PointId(a))
                }
            };
            problem.geometry.push(point(b, x + 18.0, y + 3.0 * next()));
            let end = PointReference::Point(PointId(b));
            problem.geometry.push(Geometry::Line(Line {
                id: CurveId(line),
                start,
                end,
            }));
            last_end = Some(end);
            if next() < 0.6 {
                let c = fresh();
                constrain(
                    &mut problem,
                    Relation::Distance {
                        point1: start,
                        point2: end,
                        distance: Length(20.0),
                    },
                    c,
                );
            }
            if next() < 0.2 {
                let c = fresh();
                constrain(
                    &mut problem,
                    Relation::FixedPoint {
                        point: start,
                        position: Vector { x, y },
                    },
                    c,
                );
            }
            if next() < 0.8 {
                let slope = next() - 0.5;
                for t in 0..8 {
                    let t = f64::from(t) * 2.5;
                    soft.push(SoftPoint {
                        curve: CurveReference::Curve(CurveId(line)),
                        at: Vector {
                            x: x + t,
                            y: y + 1.0 + slope * t,
                        },
                    });
                }
            }
            let centre = fresh();
            let circle = fresh();
            problem.geometry.push(point(centre, x + 5.0, y + 25.0));
            problem
                .geometry
                .push(Geometry::Circle(crate::problem::Circle {
                    id: CurveId(circle),
                    center: PointReference::Point(PointId(centre)),
                    radius: Length(3.0),
                }));
            if next() < 0.5 {
                let c = fresh();
                constrain(
                    &mut problem,
                    Relation::Radius {
                        circle: CurveReference::Curve(CurveId(circle)),
                        radius: Length(4.0),
                    },
                    c,
                );
            }
            if next() < 0.7 {
                for t in 0..6 {
                    let angle = f64::from(t);
                    soft.push(SoftPoint {
                        curve: CurveReference::Curve(CurveId(circle)),
                        at: Vector {
                            x: x + 6.0 + 4.2 * angle.cos(),
                            y: y + 24.0 + 4.2 * angle.sin(),
                        },
                    });
                }
            }
        }
        (problem, soft)
    }

    /// The step solved block by block lands where the one dense KKT
    /// system's step does, through every step of the soft solve.
    #[test]
    fn the_soft_step_by_blocks_matches_the_dense_one() {
        let mut settled = 0;
        for seed in 1..=12 {
            let (problem, soft) = scattered(seed * 2_654_435_761);
            let blocks = solve_soft(&problem, &soft).unwrap();
            let dense = solve_soft_dense(&problem, &soft).unwrap();
            assert_eq!(blocks.iteration.outcome, dense.iteration.outcome);
            assert_eq!(blocks.rms, dense.rms, "seed {seed}");
            assert_eq!(
                blocks.iteration.values, dense.iteration.values,
                "seed {seed}"
            );
            settled +=
                usize::from(blocks.iteration.iterations > 1 && blocks.rms[1] < blocks.rms[0]);
        }
        assert!(settled > 6, "{settled}");
    }
}
