//! Observable filtering and compatibility-domain reports for stored relations.

use super::adapter;
use crate::sketch::Sketch;
use sketch_solver::input::{InputError, validate_constraint};
use sketch_solver::problem::{Constraint, ConstraintId, EquationIndex, Relation};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Inactive,
    ReferenceDimension,
    Compiled,
    NoEquations,
}

#[derive(Debug, Clone)]
pub struct ConstraintReport {
    pub id: Uuid,
    pub disposition: Disposition,
    pub equations: Vec<EquationIndex>,
    /// Invalid semantic input retained by the application's trusted compiler.
    /// Equations may still exist for valid members of a composite relation.
    pub input_error: Option<InputError>,
    /// Offset pair indices omitted for missing or unsupported operands.
    pub skipped_members: Vec<usize>,
}

/// Report every stored constraint without changing solve behaviour. This
/// explicit inspection path also distinguishes valid zero-row relations from
/// malformed relations the application accepts for saved-document compatibility.
pub fn compilation_reports(sketch: &Sketch) -> Vec<ConstraintReport> {
    let problem = adapter::prepare(sketch, &[]);
    let system = sketch_solver::compile::compile_system(&problem, None, false);
    let trace = sketch_solver::trace::snapshot(&problem, &system);
    sketch
        .constraints
        .iter()
        .map(|stored| {
            let id = ConstraintId(stored.id.as_u128());
            let constraint = problem.constraints.iter().find(|c| c.id == id);
            let equations = trace
                .constraints
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.equations.clone())
                .unwrap_or_default();
            let disposition = if !stored.active {
                Disposition::Inactive
            } else if !stored.is_solved() {
                Disposition::ReferenceDimension
            } else if equations.is_empty() {
                Disposition::NoEquations
            } else {
                Disposition::Compiled
            };
            let input_error = constraint.and_then(|c| validate_constraint(&problem, c).err());
            let skipped_members = match constraint.map(|c| &c.kind) {
                Some(Relation::Offset { pairs, distance }) => pairs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, pair)| {
                        let member = Constraint {
                            id,
                            kind: Relation::Offset {
                                pairs: vec![*pair],
                                distance: *distance,
                            },
                        };
                        validate_constraint(&problem, &member)
                            .is_err()
                            .then_some(index)
                    })
                    .collect(),
                _ => Vec::new(),
            };
            ConstraintReport {
                id: stored.id,
                disposition,
                equations,
                input_error,
                skipped_members,
            }
        })
        .collect()
}
