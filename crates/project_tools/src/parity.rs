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

/// Bounding boxes agree within this, in mm, at every corner.
pub const BOUNDS_TOLERANCE_MM: f32 = 0.05;

type Bounds = ([f32; 3], [f32; 3]);

/// A rebuilt solid measured against the shape the file saved for it: where
/// its edges reach and how many faces it has.
#[derive(Debug, Clone, Serialize)]
pub struct ShapeParity {
    pub saved_bounds_mm: Option<Bounds>,
    pub built_bounds_mm: Option<Bounds>,
    pub saved_faces: usize,
    pub built_faces: usize,
}

impl ShapeParity {
    pub fn bounds_match(&self) -> bool {
        match (self.saved_bounds_mm, self.built_bounds_mm) {
            (Some((sl, sh)), Some((bl, bh))) => (0..3).all(|i| {
                (sl[i] - bl[i]).abs() <= BOUNDS_TOLERANCE_MM
                    && (sh[i] - bh[i]).abs() <= BOUNDS_TOLERANCE_MM
            }),
            (a, b) => a.is_none() && b.is_none(),
        }
    }

    pub fn holds(&self) -> bool {
        self.bounds_match() && self.saved_faces == self.built_faces
    }
}

/// Measure a built solid (its bounds and face count) against `saved`, the
/// file's text for the same object, which `frame` (the placement of the
/// object's groups) places.
pub fn compare_shape(
    saved: &str,
    frame: project_archive::document_xml::Placement,
    built_bounds_mm: Option<Bounds>,
    built_faces: usize,
) -> Result<ShapeParity, crate::error::Failure> {
    let saved_shape = project_archive::saved_shape::SavedShape::parse(saved).map_err(|e| {
        crate::error::Failure::new(crate::error::ErrorCategory::Rebuild, e.to_string())
    })?;
    let saved_bounds = saved_shape.bounds.map(|(lo, hi)| {
        let mut placed = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for corner in 0..8 {
            let p = [0, 1, 2].map(|i| if corner >> i & 1 == 0 { lo[i] } else { hi[i] });
            let p = frame.rotation.transform_vector(p);
            for (i, (v, o)) in p.into_iter().zip(frame.position.to_array()).enumerate() {
                let v = (v + o) as f32;
                placed.0[i] = placed.0[i].min(v);
                placed.1[i] = placed.1[i].max(v);
            }
        }
        placed
    });
    Ok(ShapeParity {
        saved_bounds_mm: saved_bounds,
        built_bounds_mm,
        saved_faces: saved_shape.faces.len(),
        built_faces,
    })
}
