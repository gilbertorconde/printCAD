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

/// What no remaining motion of a problem can move, at its configuration as
/// given (pass it solved): the points both of whose coordinates are
/// determined, and the curves whose own shape variables (a radius, an
/// ellipse's or conic's shape) are, every curve without such variables
/// counted in. Reference geometry and pinned external geometry are always
/// locked. A curve is held where it is when it is in `curves` and every
/// point it is drawn through is in `points`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Locked {
    pub points: std::collections::HashSet<crate::problem::PointReference>,
    pub curves: std::collections::HashSet<crate::problem::CurveReference>,
}

pub fn locked(problem: &Problem) -> Locked {
    let sys = compile_system(problem, None, true);
    let mut var_locked = vec![true; sys.vars.len()];
    for &j in &sys.free {
        var_locked[j] = false;
    }
    if !sys.free.is_empty() && !sys.specs.is_empty() {
        let columns =
            determined_columns(jacobian(&sys, &sys.vars), problem.settings.rank_tolerance);
        for (&j, held) in sys.free.iter().zip(columns) {
            var_locked[j] = held;
        }
    }
    let mut out = Locked::default();
    for (&point, &k) in &sys.point_vars {
        if var_locked[k] && var_locked[k + 1] {
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
    out
}

/// For each column of `m`, whether it lies outside the null space: no
/// direction the rows leave free moves it. Gauss-Jordan elimination with
/// partial pivoting; a pivot column is determined when its row holds
/// nothing in any non-pivot column, and a non-pivot column never is.
fn determined_columns(mut m: Vec<Vec<f64>>, rank_tolerance: f64) -> Vec<bool> {
    let rows = m.len();
    let cols = m.first().map_or(0, Vec::len);
    let max_abs = m.iter().flatten().fold(0.0_f64, |acc, v| acc.max(v.abs()));
    if rows == 0 || cols == 0 || max_abs == 0.0 || !max_abs.is_finite() {
        return vec![false; cols];
    }
    let tol = max_abs * rank_tolerance;
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
    use super::determined_columns;

    #[test]
    fn a_column_the_rows_pin_is_determined_and_a_coupled_one_is_not() {
        // x0 is pinned; x1 and x2 only move together (x1 - x2 = 0), so
        // neither is determined.
        let m = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, -1.0]];
        assert_eq!(determined_columns(m, 1e-9), vec![true, false, false]);
    }
}
