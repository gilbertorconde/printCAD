//! Application conversion, sampling and updates for sketch splines.
//! Pure basis and fitting formulas live in [`sketch_solver::spline`]; this boundary alone
//! converts sampled or refitted positions to stored f64 geometry.
//!
//! The solver's std-only basis owns stored sketch semantics: centripetal
//! interpolation, uniform periodic rings and clamping at the domain ends.
//! Picking passes its effective knots and weights to `sketch_geometry` rather
//! than asking that library to interpolate the sketch's points again. Its
//! point-defined interpolation uses chord lengths, and its periodic evaluator
//! wraps parameters. Rational evaluation also accumulates in a different order;
//! swapping evaluators would change floating-point results even on a shared
//! valid basis. Keep this adapter free of another basis implementation.

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

/// Sample for the application, preserving the final f64 conversion.
pub fn sample_basis(basis: &Basis, control: &[[f64; 2]], samples: usize) -> Vec<Vec2D> {
    basis
        .sample(control, samples)
        .into_iter()
        .map(|[x, y]| Vec2D::new(x, y))
        .collect()
}

/// A geometric polyline with probes in every nonempty knot span. The interval
/// count is per span, so even a very short span contributes its full shape.
pub fn sample_span_basis(basis: &Basis, control: &[[f64; 2]], samples: usize) -> Vec<Vec2D> {
    let (lo, hi) = basis.domain();
    let mut breaks: Vec<_> = basis
        .knots()
        .iter()
        .copied()
        .filter(|t| *t >= lo && *t <= hi)
        .collect();
    breaks.dedup();
    let at = |t| {
        let [x, y] = basis.eval(control, t);
        Vec2D::new(x, y)
    };
    let n = samples.max(1);
    let mut points = vec![at(lo)];
    for span in breaks.windows(2) {
        points.extend((1..=n).map(|i| {
            at(if i == n {
                span[1]
            } else {
                span[0] + (span[1] - span[0]) * i as f64 / n as f64
            })
        }));
    }
    points
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
            .map(|id| sketch.point_position(*id).map(|p| [p.x, p.y]))
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
                placed.push((*id, Vec2D::new(p[0], p[1])));
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
            .map(|id| sketch.point_position(*id).map(|p| [p.x, p.y]))
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

    /// A polyline for rendering, measurement and replacement fits. Knot span
    /// boundaries are always present, even between uniform parameter samples.
    pub fn polyline(&self, sketch: &Sketch, samples_per_span: usize) -> Option<Vec<Vec2D>> {
        Some(sample_span_basis(
            &basis_of(self)?,
            &self.control_positions(sketch)?,
            samples_per_span,
        ))
    }

    /// The kernel's default cubic representation needs no explicit knots
    /// or rational weights.
    pub fn is_default_cubic(&self) -> bool {
        self.degree == 3 && self.knots.is_empty() && self.weights.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sketch_geometry::bspline::{
        BSplineCurve, SplineBasis, SplineDefinition, SplineDegree, SplineParam, SplineShape,
    };
    use sketch_geometry::world::WorldPoint;

    #[test]
    fn picking_and_stored_splines_agree_on_the_effective_basis() {
        let control = [[0., 0.], [1., 4.], [2., -3.], [5., 6.], [9., 1.], [12., 2.]];
        let poles: Vec<_> = control
            .iter()
            .map(|&[x, y]| WorldPoint::new(x, y))
            .collect();
        for degree in 1..=MAX_DEGREE {
            for periodic in [false, true] {
                for weights in [vec![1.; control.len()], vec![1., 0.5, 2., 3., 0.75, 1.]] {
                    let stored = Basis::new(degree, control.len(), &[], periodic)
                        .unwrap()
                        .with_weights(&weights);
                    let picked = BSplineCurve::build_with_basis(
                        SplineShape {
                            definition: SplineDefinition::Poles,
                            degree: SplineDegree::new(degree),
                            periodic,
                        },
                        Some(&SplineBasis {
                            knots: stored.knots().to_vec(),
                            weights,
                        }),
                        &poles,
                    )
                    .unwrap();
                    let (lo, hi) = stored.domain();
                    // Include knots, their neighbours and both domain ends.
                    for knot in stored.knots() {
                        for t in [*knot - 1e-10, *knot, *knot + 1e-10] {
                            if t < lo || t > hi {
                                continue;
                            }
                            let [x, y] = stored.eval(&control, t);
                            let p = picked.point(SplineParam(t));
                            assert!(
                                (p.0.x - x).hypot(p.0.y - y) < 1e-11,
                                "degree {degree}, periodic {periodic}, t {t}"
                            );
                        }
                    }
                }
            }
        }
    }
}
