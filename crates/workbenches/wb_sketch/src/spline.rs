//! Application conversion, sampling and updates for sketch splines.
//! Pure basis and fitting formulas live in [`math`]; this boundary alone
//! converts sampled or refitted positions to stored f32 geometry.

use crate::sketch::{BSpline, GeometryElement, Sketch, Vec2D};
use sketch_solver::spline::{Basis, solve_points};

/// The highest degree a sketch spline takes.
pub const MAX_DEGREE: u32 = 5;

/// The basis of `spline`; absent when there are too few control points.
pub fn basis_of(spline: &BSpline) -> Option<Basis> {
    Some(
        Basis::new(
            spline.degree,
            spline.control_points.len(),
            &spline.knots,
            spline.periodic,
        )?
        .with_weights(&spline.weights),
    )
}

/// Sample for the application, preserving the final f32 conversion.
pub fn sample_basis(basis: &Basis, control: &[[f64; 2]], samples: usize) -> Vec<Vec2D> {
    basis
        .sample(control, samples)
        .into_iter()
        .map(|[x, y]| Vec2D::new(x as f32, y as f32))
        .collect()
}

/// Controls of fit-point splines not themselves named as fit points:
/// these are placed by [`refit_splines`] after a solve.
pub fn derived_points(sketch: &Sketch) -> Vec<uuid::Uuid> {
    let mut ids = Vec::new();
    for element in &sketch.geometry {
        if let GeometryElement::BSpline(b) = element
            && !b.fit_points.is_empty()
        {
            ids.extend(
                b.control_points
                    .iter()
                    .filter(|id| !b.fit_points.contains(id)),
            );
        }
    }
    ids
}

/// Refit the controls at the original fit parameters after point movement.
pub fn refit_splines(sketch: &mut Sketch) {
    let mut placed: Vec<(uuid::Uuid, Vec2D)> = Vec::new();
    for element in &sketch.geometry {
        let GeometryElement::BSpline(b) = element else {
            continue;
        };
        let n = b.control_points.len();
        if b.fit_points.len() != n || b.fit_params.len() != n {
            continue;
        }
        let Some(basis) = basis_of(b) else {
            continue;
        };
        let Some(targets) = b
            .fit_points
            .iter()
            .map(|id| {
                sketch
                    .point_position(*id)
                    .map(|p| [f64::from(p.x), f64::from(p.y)])
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let rows: Vec<Vec<f64>> = b
            .fit_params
            .iter()
            .map(|t| {
                let mut row = vec![0.0; n];
                for (i, w) in basis.row(*t) {
                    row[i] += w;
                }
                row
            })
            .collect();
        let Some(control) = solve_points(rows, targets) else {
            continue;
        };
        for (id, p) in b.control_points.iter().zip(control) {
            if !b.fit_points.contains(id) {
                placed.push((*id, Vec2D::new(p[0] as f32, p[1] as f32)));
            }
        }
    }
    for (id, at) in placed {
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(id) {
            p.position = at;
        }
    }
}

impl BSpline {
    /// Stored control positions promoted to double precision.
    pub fn control_positions(&self, sketch: &Sketch) -> Option<Vec<[f64; 2]>> {
        self.control_points
            .iter()
            .map(|id| {
                sketch
                    .point_position(*id)
                    .map(|p| [f64::from(p.x), f64::from(p.y)])
            })
            .collect()
    }

    /// The curve sampled at `samples` intervals, end to end.
    pub fn points(&self, sketch: &Sketch, samples: usize) -> Option<Vec<Vec2D>> {
        Some(sample_basis(
            &basis_of(self)?,
            &self.control_positions(sketch)?,
            samples,
        ))
    }

    /// The kernel's default cubic representation needs no explicit knots
    /// or rational weights.
    pub fn is_default_cubic(&self) -> bool {
        self.degree == 3 && self.knots.is_empty() && self.weights.is_empty()
    }
}
