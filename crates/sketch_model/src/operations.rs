//! Checked sketch operations for consumers outside the editing workbench.

use std::collections::HashMap;
use uuid::Uuid;

use crate::sketch::{GeometryElement, ORIGIN_ID, Sketch};

/// Result of solving the stored sketch, including storage write-back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SolveOutcome {
    /// All residuals below tolerance.
    Converged { iterations: usize },
    /// The best-effort geometry is stored, but the constraints did not converge.
    NotConverged { residual: f64 },
    /// No equations require solving.
    NothingToSolve,
}

/// Solve active driving constraints with the workbench's storage precision,
/// spline refit and text following. Invalid semantic input returns an error
/// before mutation; nonconvergence is a numerical outcome, not an input error.
pub fn solve(sketch: &mut Sketch) -> Result<SolveOutcome, sketch_solver::input::InputError> {
    crate::solver::solve_checked(sketch)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointJoinError {
    MissingPoint(Uuid),
    ImmutableOrigin,
    NonCanonicalAlias { from: Uuid, to: Uuid },
}

impl std::fmt::Display for PointJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPoint(id) => write!(f, "point alias names a missing point: {id}"),
            Self::ImmutableOrigin => write!(f, "the origin cannot be replaced by a point alias"),
            Self::NonCanonicalAlias { from, to } => {
                write!(
                    f,
                    "point alias {from} -> {to} must name a kept point directly"
                )
            }
        }
    }
}

impl std::error::Error for PointJoinError {}

/// Merge explicitly chosen point aliases without proximity repair or solving.
/// Each source is an existing point; each target is a kept point or the origin.
/// Self aliases, chains and cycles are rejected before any mutation. References
/// and construction membership follow the kept points; coordinates stay theirs.
pub fn join_points(
    sketch: &mut Sketch,
    aliases: &HashMap<Uuid, Uuid>,
) -> Result<(), PointJoinError> {
    let point_exists = |id| matches!(sketch.get_geometry(id), Some(GeometryElement::Point(_)));
    for (&from, &to) in aliases {
        if from == ORIGIN_ID {
            return Err(PointJoinError::ImmutableOrigin);
        }
        if !point_exists(from) {
            return Err(PointJoinError::MissingPoint(from));
        }
        if to != ORIGIN_ID && !point_exists(to) {
            return Err(PointJoinError::MissingPoint(to));
        }
        if aliases.contains_key(&to) {
            return Err(PointJoinError::NonCanonicalAlias { from, to });
        }
    }
    crate::repair::join_points(sketch, aliases);
    Ok(())
}
