//! Equation provenance and deterministic compilation snapshots.

use crate::compile::System;
use crate::problem::{
    ConstraintId, CurveReference, EquationIndex, PointReference, Problem, VariableIndex,
};
use crate::residual::ResidualSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EquationSource {
    Constraint(ConstraintId),
    ArcEndpoint {
        curve: CurveReference,
        point: PointReference,
    },
    ConicEndpoint {
        curve: CurveReference,
        point: PointReference,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CompiledTrace {
    pub initial_values: Vec<f64>,
    pub free_variables: Vec<VariableIndex>,
    pub points: Vec<(PointReference, VariableIndex)>,
    pub radii: Vec<(CurveReference, VariableIndex)>,
    pub shapes: Vec<(CurveReference, VariableIndex)>,
    pub derived_foci: Vec<PointReference>,
    /// Includes chosen contact branches and ray order; auxiliary initial
    /// parameters are the corresponding entries in initial_values.
    pub specifications: Vec<ResidualSpec>,
    pub equations: Vec<(EquationIndex, EquationSource)>,
    /// Every input constraint, including those contributing zero rows.
    pub constraints: Vec<ConstraintRows>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConstraintRows {
    pub id: ConstraintId,
    pub equations: Vec<EquationIndex>,
}

pub fn snapshot(problem: &Problem, system: &System) -> CompiledTrace {
    let points = sorted(&system.point_vars);
    let radii = sorted(&system.radius_vars);
    let shapes = sorted(&system.shape_vars);
    let mut derived_foci: Vec<_> = system.derived_foci.iter().copied().collect();
    derived_foci.sort();
    let equations: Vec<_> = system
        .origins
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, s)| (EquationIndex(i), s))
        .collect();
    let constraints = problem
        .constraints
        .iter()
        .map(|c| ConstraintRows {
            id: c.id,
            equations: equations
                .iter()
                .filter_map(|(i, s)| (*s == EquationSource::Constraint(c.id)).then_some(*i))
                .collect(),
        })
        .collect();
    CompiledTrace {
        initial_values: system.vars.clone(),
        free_variables: system.variable_indices().collect(),
        points,
        radii,
        shapes,
        derived_foci,
        specifications: system.specs.clone(),
        equations,
        constraints,
    }
}

fn sorted<Id: Ord + Copy>(
    values: &std::collections::HashMap<Id, usize>,
) -> Vec<(Id, VariableIndex)> {
    let mut result: Vec<_> = values
        .iter()
        .map(|(id, index)| (*id, VariableIndex(*index)))
        .collect();
    result.sort_by_key(|(id, _)| *id);
    result
}
