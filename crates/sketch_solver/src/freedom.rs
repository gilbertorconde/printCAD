//! Rank and degrees of freedom, including otherwise uncompiled conic shapes.

use crate::compile::compile_system;
use crate::problem::{ConicKind, Geometry, Problem};
use crate::solve::{jacobian, jacobian_rank};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Freedom {
    pub free_variables: usize,
    pub rank: usize,
    pub unsolved_shape: i32,
    pub degrees: i32,
}

pub fn analyze(problem: &Problem) -> Result<Freedom, crate::input::InputError> {
    crate::input::validate(problem)?;
    Ok(estimate(problem))
}

pub fn estimate(problem: &Problem) -> Freedom {
    let sys = compile_system(problem, None, true);
    let unsolved_shape = problem
        .geometry
        .iter()
        .map(|g| {
            if problem.external.contains(&g.id()) {
                return 0;
            }
            match g {
                Geometry::Ellipse(e) if !sys.shape_vars.contains_key(&e.id.into()) => 3,
                Geometry::Conic(c) if !sys.shape_vars.contains_key(&c.id.into()) => {
                    if c.kind == ConicKind::Parabola { 2 } else { 3 }
                }
                _ => 0,
            }
        })
        .sum();
    let free_variables = sys.free.len();
    let rank = if sys.specs.is_empty() || sys.free.is_empty() {
        0
    } else {
        jacobian_rank(jacobian(&sys, &sys.vars), problem.settings.rank_tolerance)
    };
    Freedom {
        free_variables,
        rank,
        unsolved_shape,
        degrees: free_variables as i32 - rank as i32 + unsolved_shape,
    }
}
