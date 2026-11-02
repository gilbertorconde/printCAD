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
use crate::solve::{Iteration, SolveOutcome, eval_residuals, iterate, jacobian};
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
        let (js, rs) = (jacobian(&fit, &x), eval_residuals(&fit, &x));
        let (jh, rh) = (jacobian(&hard, &x), eval_residuals(&hard, &x));
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
    let mut ids = Vec::new();
    let mut skipped = 0;
    for s in soft {
        let relation = |point| match problem.get_geometry(s.curve) {
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
}
