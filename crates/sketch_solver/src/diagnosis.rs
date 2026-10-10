//! Constraint health from recompilation and rank at a solved configuration.

use crate::compile::compile_system;
use crate::freedom::estimate;
use crate::problem::{ConstraintId, Geometry, Length, Problem, Vector};
use crate::solve::{SolveOutcome, iterate, jacobian, jacobian_rank};

#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnosis {
    pub dof: i32,
    pub redundant: Vec<ConstraintId>,
    pub conflicting: Vec<ConstraintId>,
    pub analyzed: bool,
}

pub fn analyze(problem: &Problem) -> Result<Diagnosis, crate::input::InputError> {
    crate::input::validate(problem)?;
    Ok(diagnose(problem))
}

/// The standalone path evaluates redundancy on double-precision geometry.
pub fn diagnose(problem: &Problem) -> Diagnosis {
    let system = compile_system(problem, None, false);
    let answer = iterate(&system, problem.settings);
    let mut solved = problem.clone();
    for g in &mut solved.geometry {
        match g {
            Geometry::Point(p) => {
                let k = system.point_vars[&p.id.into()];
                p.position = Vector {
                    x: answer.values[k],
                    y: answer.values[k + 1],
                };
            }
            Geometry::Circle(c) => {
                c.radius = Length(answer.values[system.radius_vars[&c.id.into()]])
            }
            Geometry::Arc(a) => a.radius = Length(answer.values[system.radius_vars[&a.id.into()]]),
            Geometry::Ellipse(e) => {
                if let Some(&k) = system.shape_vars.get(&e.id.into()) {
                    e.major = Vector {
                        x: answer.values[k],
                        y: answer.values[k + 1],
                    };
                    e.minor = Length(answer.values[k + 2]);
                }
            }
            Geometry::Conic(c) => {
                if let Some(&k) = system.shape_vars.get(&c.id.into()) {
                    c.axis = Vector {
                        x: answer.values[k],
                        y: answer.values[k + 1],
                    };
                    c.minor = Length(answer.values[k + 2]);
                }
            }
            Geometry::Line(_) | Geometry::BSpline(_) => {}
        }
    }
    diagnose_at(problem, answer.outcome, &solved)
}

/// Callers with storage rounding or derived geometry may provide the
/// configuration after their application step. Exclusion probes remain
/// mathematical recompilations; there are no application callbacks.
pub fn diagnose_at(problem: &Problem, outcome: SolveOutcome, solved: &Problem) -> Diagnosis {
    let dof = estimate(problem).degrees;
    if problem.constraints.len() > problem.settings.diagnosis_limit {
        return Diagnosis {
            dof,
            analyzed: false,
            ..Diagnosis::default()
        };
    }
    let mut diagnosis = Diagnosis {
        dof,
        analyzed: true,
        ..Diagnosis::default()
    };
    match outcome {
        SolveOutcome::NotConverged { .. } => {
            for c in &problem.constraints {
                let mut without = problem.clone();
                without.constraints.retain(|other| other.id != c.id);
                let system = compile_system(&without, None, false);
                if matches!(
                    iterate(&system, problem.settings).outcome,
                    SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
                ) {
                    diagnosis.conflicting.push(c.id);
                }
            }
        }
        SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve => {
            let full = compile_system(solved, None, false);
            if !full.specs.is_empty() && !full.vars.is_empty() {
                let full_rank =
                    jacobian_rank(jacobian(&full, &full.vars), problem.settings.rank_tolerance);
                for c in &problem.constraints {
                    let system = compile_system(solved, Some(c.id), false);
                    if system.residual_len == full.residual_len {
                        continue;
                    }
                    if jacobian_rank(
                        jacobian(&system, &system.vars),
                        problem.settings.rank_tolerance,
                    ) == full_rank
                    {
                        diagnosis.redundant.push(c.id);
                    }
                }
            }
        }
    }
    diagnosis
}
