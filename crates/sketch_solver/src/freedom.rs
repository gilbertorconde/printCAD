//! Rank and degrees of freedom, including otherwise uncompiled conic shapes.

use crate::compile::compile_system;
use crate::problem::{ConicKind, Geometry, InternalRole, ItemReference, Problem, Relation};

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
    estimate_in(problem, &compile_system(problem, None, true))
}

/// [`estimate`] from `sys`, `problem` compiled for rank
/// ([`crate::compile::compile_for_rank`]).
pub fn estimate_in(problem: &Problem, sys: &crate::compile::System) -> Freedom {
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
        crate::sparse::rank(sys, problem.settings.rank_tolerance)
    };
    Freedom {
        free_variables,
        rank,
        unsolved_shape,
        degrees: free_variables as i32 - rank as i32 + unsolved_shape,
    }
}

/// What no remaining motion of a problem can move, at its configuration as
/// given (pass it solved): the points both of whose coordinates are
/// determined, and the curves whose own shape variables (a radius, an
/// ellipse's or conic's shape) are, every curve without such variables
/// counted in. Reference geometry and pinned external geometry are always
/// locked. A curve is held where it is when it is in `curves` and every
/// point it is drawn through is in `points`. An ellipse's focus placed
/// from its curve after the solve is locked when the curve and its centre
/// are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Locked {
    pub points: crate::problem::IdSet<crate::problem::PointReference>,
    pub curves: crate::problem::IdSet<crate::problem::CurveReference>,
}

pub fn locked(problem: &Problem) -> Locked {
    locked_in(problem, &compile_system(problem, None, true))
}

/// [`locked`] from `sys`, `problem` compiled for rank
/// ([`crate::compile::compile_for_rank`]).
pub fn locked_in(problem: &Problem, sys: &crate::compile::System) -> Locked {
    let mut var_locked = vec![true; sys.vars.len()];
    for &j in &sys.free {
        var_locked[j] = false;
    }
    if !sys.free.is_empty() && !sys.specs.is_empty() {
        let columns = crate::sparse::determined(sys, problem.settings.rank_tolerance);
        for (&j, held) in sys.free.iter().zip(columns) {
            var_locked[j] = held;
        }
    }
    let mut out = Locked::default();
    for (&point, &k) in &sys.point_vars {
        if var_locked[k] && var_locked[k + 1] && !sys.derived_foci.contains(&point) {
            out.points.insert(point);
        }
    }
    for g in &problem.geometry {
        let crate::problem::ItemReference::Curve(curve) = g.id() else {
            continue;
        };
        let own = match g {
            Geometry::Circle(_) | Geometry::Arc(_) => {
                sys.radius_vars.get(&curve).map(|&k| var_locked[k])
            }
            Geometry::Ellipse(_) | Geometry::Conic(_) => sys
                .shape_vars
                .get(&curve)
                .map(|&k| (k..k + 3).all(|i| var_locked[i])),
            _ => Some(true),
        };
        // An ellipse or conic whose shape is not solved for is free unless
        // it is pinned external geometry.
        let held = own.unwrap_or_else(|| problem.external.contains(&g.id()));
        if held {
            out.curves.insert(curve);
        }
    }
    // A focus left out of the solve is placed from its ellipse after it:
    // it moves exactly when the ellipse's centre or shape does.
    for c in &problem.constraints {
        let Relation::InternalAlignment {
            element: ItemReference::Point(focus),
            curve,
            role: InternalRole::Focus1 | InternalRole::Focus2,
        } = c.kind
        else {
            continue;
        };
        if !sys.derived_foci.contains(&focus) {
            continue;
        }
        let Some(Geometry::Ellipse(e)) = problem.get_geometry(curve) else {
            continue;
        };
        let centre = sys
            .point_vars
            .get(&e.center)
            .is_none_or(|&k| var_locked[k] && var_locked[k + 1]);
        if centre && out.curves.contains(&curve) {
            out.points.insert(focus);
        }
    }
    out
}

/// For each column of `m`, whether it lies outside the null space: no
/// direction the rows leave free moves it. Gauss-Jordan elimination with
/// partial pivoting, a pivot at or below `tol` taken for zero; a pivot
/// column is determined when its row holds nothing in any non-pivot
/// column, and a non-pivot column never is.
pub(crate) fn determined_within(mut m: Vec<Vec<f64>>, tol: f64, rank_tolerance: f64) -> Vec<bool> {
    let rows = m.len();
    let cols = m.first().map_or(0, Vec::len);
    let mut pivots: Vec<(usize, usize)> = Vec::new();
    let mut row = 0;
    for col in 0..cols {
        if row >= rows {
            break;
        }
        let (pivot_row, pivot_val) =
            (row..rows)
                .map(|r| (r, m[r][col].abs()))
                .fold(
                    (row, -1.0),
                    |best, cur| if cur.1 > best.1 { cur } else { best },
                );
        if pivot_val <= tol {
            continue;
        }
        m.swap(row, pivot_row);
        let pivot = m[row][col];
        for v in &mut m[row] {
            *v /= pivot;
        }
        let pivot_vals = m[row].clone();
        for (r, row_vals) in m.iter_mut().enumerate() {
            if r == row {
                continue;
            }
            let factor = row_vals[col];
            if factor == 0.0 {
                continue;
            }
            for (dst, src) in row_vals.iter_mut().zip(&pivot_vals) {
                *dst -= factor * src;
            }
        }
        pivots.push((row, col));
        row += 1;
    }
    let pivot_cols: std::collections::HashSet<usize> = pivots.iter().map(|&(_, c)| c).collect();
    let mut out = vec![false; cols];
    // Entries left in a reduced row are relative to its unit pivot.
    let reduced_tol = rank_tolerance.sqrt().max(1e-9);
    for &(r, c) in &pivots {
        out[c] = (0..cols)
            .filter(|f| !pivot_cols.contains(f))
            .all(|f| m[r][f].abs() <= reduced_tol);
    }
    out
}

#[cfg(test)]
mod locked_tests {
    use super::determined_within;

    #[test]
    fn a_column_the_rows_pin_is_determined_and_a_coupled_one_is_not() {
        // x0 is pinned; x1 and x2 only move together (x1 - x2 = 0), so
        // neither is determined.
        let m = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, -1.0]];
        assert_eq!(determined_within(m, 1e-9, 1e-9), vec![true, false, false]);
    }
}
