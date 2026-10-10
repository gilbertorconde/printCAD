//! Versioned semantic replay data. File I/O and revision metadata belong
//! to the runner; this module evaluates only the supplied problem.

use crate::input::InputError;
use crate::problem::{EquationIndex, Problem};
use crate::trace::{CompiledTrace, EquationSource};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub schema_version: u32,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReplayError {
    UnsupportedSchema(u32),
    Input(InputError),
}
impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "replay failed: {self:?}")
    }
}
impl std::error::Error for ReplayError {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Run {
    pub compiled: CompiledTrace,
    pub answer: crate::solve::Iteration,
    pub freedom: crate::freedom::Freedom,
    pub diagnosis: crate::diagnosis::Diagnosis,
    pub residuals: Vec<RowResidual>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RowResidual {
    pub equation: EquationIndex,
    pub source: EquationSource,
    pub value: f64,
}

pub fn run(fixture: &Fixture) -> Result<Run, ReplayError> {
    if fixture.schema_version != SCHEMA_VERSION {
        return Err(ReplayError::UnsupportedSchema(fixture.schema_version));
    }
    let system = crate::compile::compile(&fixture.problem).map_err(ReplayError::Input)?;
    let answer = crate::solve::iterate(&system, fixture.problem.settings);
    let residuals = crate::solve::eval_residuals(&system, &answer.values)
        .into_iter()
        .zip(&system.origins)
        .enumerate()
        .map(|(i, (value, source))| RowResidual {
            equation: EquationIndex(i),
            source: source.clone(),
            value,
        })
        .collect();
    Ok(Run {
        compiled: crate::trace::snapshot(&fixture.problem, &system),
        answer,
        residuals,
        freedom: crate::freedom::estimate(&fixture.problem),
        diagnosis: crate::diagnosis::diagnose(&fixture.problem),
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Regression {
    pub name: String,
    pub fixture: Fixture,
    pub expected: Result<Run, ReplayError>,
}
