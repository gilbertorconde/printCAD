use crate::sketch_import::ImportedSketch;
use glam::DVec2;
use serde::Serialize;
use wb_sketch::operations::{SolveOutcome, solve};
use wb_sketch::sketch::{GeometryElement, Sketch};

pub const STATIONARY_TOLERANCE_MM: f64 = 1e-6;

#[derive(Debug, Clone, Serialize)]
pub struct Stationarity {
    pub converged: bool,
    pub points: usize,
    pub radii: usize,
    pub rounding_mm: f64,
    pub solver_shift_mm: f64,
    pub source_shift_mm: f64,
    pub tolerance_mm: f64,
}

impl Stationarity {
    pub fn holds(&self) -> bool {
        self.converged && self.source_shift_mm <= self.tolerance_mm
    }
}

pub fn check_stationary(imported: &ImportedSketch) -> Result<Stationarity, crate::error::Failure> {
    let mut solved = imported.sketch.clone();
    let outcome = solve(&mut solved).map_err(|error| {
        crate::error::Failure::new(crate::error::ErrorCategory::Stationarity, error)
    })?;
    let mut check = Stationarity {
        converged: !matches!(outcome, SolveOutcome::NotConverged { .. }),
        points: imported.source_points.len(),
        radii: imported.source_radii.len(),
        rounding_mm: 0.0,
        solver_shift_mm: 0.0,
        source_shift_mm: 0.0,
        tolerance_mm: STATIONARY_TOLERANCE_MM,
    };
    for &(id, source) in &imported.source_points {
        let p = |s: &Sketch| {
            let p = s.point_position(id).expect("imported point stays alive");
            DVec2::new(p.x, p.y)
        };
        check.rounding_mm = check.rounding_mm.max(source.distance(p(&imported.sketch)));
        check.solver_shift_mm = check
            .solver_shift_mm
            .max(p(&solved).distance(p(&imported.sketch)));
        check.source_shift_mm = check.source_shift_mm.max(source.distance(p(&solved)));
    }
    for &(id, source) in &imported.source_radii {
        let radius = |s: &Sketch| -> f64 {
            match s.get_geometry(id).expect("imported circle stays alive") {
                GeometryElement::Circle(c) => c.radius,
                GeometryElement::Arc(a) => a.radius,
                _ => unreachable!(),
            }
        };
        check.rounding_mm = check
            .rounding_mm
            .max((source - radius(&imported.sketch)).abs());
        check.solver_shift_mm = check
            .solver_shift_mm
            .max((radius(&solved) - radius(&imported.sketch)).abs());
        check.source_shift_mm = check.source_shift_mm.max((source - radius(&solved)).abs());
    }
    Ok(check)
}
