//! A solid's mass properties and how two of them are held to each other.

use serde::{Deserialize, Serialize};

/// What a solid is measured by: its volume, area, bounding box and centre
/// of mass, all in mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measures {
    /// `None` when the shape encloses no volume (an open shell, a sheet).
    pub volume_mm3: Option<f64>,
    pub area_mm2: f64,
    pub centre_mm: [f64; 3],
    pub bounds_mm: Option<([f64; 3], [f64; 3])>,
    /// The kernel integrated some face over a tessellation, so the volume
    /// and area are close rather than exact.
    pub approximate: bool,
    pub faces: usize,
}

impl Measures {
    /// The kernel's mass properties of `blob` with the bounds and face
    /// count of the same solid's mesh.
    pub fn of_blob(
        kernel: &mut kernel_ogeom::OgeomKernel,
        blob: &[u8],
        bounds_mm: Option<([f32; 3], [f32; 3])>,
        faces: usize,
    ) -> Result<Self, String> {
        let properties = kernel
            .physical_properties(blob)
            .map_err(|e| format!("measuring: {e}"))?;
        Ok(Self {
            volume_mm3: properties.volume_mm3,
            area_mm2: properties.area_mm2,
            centre_mm: properties.centre_mm,
            bounds_mm: bounds_mm.map(|(lo, hi)| (lo.map(f64::from), hi.map(f64::from))),
            approximate: properties.approximate,
            faces,
        })
    }

    fn diagonal_mm(&self) -> f64 {
        self.bounds_mm.map_or(0.0, |(lo, hi)| {
            (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt()
        })
    }
}

/// How close a rebuilt solid must come to the saved one to match it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerances {
    /// Volume and area, relative to the saved figure, when both are exact.
    pub relative: f64,
    /// Volume and area, relative, when either side is approximate.
    pub relative_approximate: f64,
    /// Every bounding-box corner coordinate, in mm. Bounds come from the
    /// meshes, so they carry the tessellation's error on curved walls.
    pub bounds_mm: f64,
    /// The centre of mass, in mm, or `centre_relative` of the saved box's
    /// diagonal when that is larger.
    pub centre_mm: f64,
    pub centre_relative: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            relative: 1e-4,
            relative_approximate: 5e-3,
            bounds_mm: crate::parity::BOUNDS_TOLERANCE_MM as f64,
            centre_mm: 0.01,
            centre_relative: 1e-4,
        }
    }
}

/// One measure on which the rebuilt solid and the saved one disagree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    pub measure: String,
    pub saved: Vec<f64>,
    pub built: Vec<f64>,
    /// How far apart they are: relative for volume and area, in mm for
    /// the box and the centre; `None` when only one side has the measure.
    pub off_by: Option<f64>,
    pub tolerance: f64,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |v: &[f64]| {
            v.iter()
                .map(|x| format!("{x:.4}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let unit = match self.measure.as_str() {
            "volume" | "area" => "rel",
            _ => "mm",
        };
        let off = self
            .off_by
            .map_or("one side only".into(), |o| format!("off {o:.3e} {unit}"));
        write!(
            f,
            "{} saved [{}] built [{}] ({off}, tol {:.1e})",
            self.measure,
            list(&self.saved),
            list(&self.built),
            self.tolerance
        )
    }
}

/// Every measure on which `built` strays from `saved` past `tolerances`;
/// empty when they match.
pub fn compare(saved: &Measures, built: &Measures, tolerances: &Tolerances) -> Vec<Difference> {
    let mut differences = Vec::new();
    let relative = if saved.approximate || built.approximate {
        tolerances.relative_approximate
    } else {
        tolerances.relative
    };
    let scalar = |name: &str, s: Option<f64>, b: Option<f64>, out: &mut Vec<Difference>| {
        let off_by = match (s, b) {
            (Some(s), Some(b)) => (s - b).abs() / s.abs().max(1e-9),
            (None, None) => return,
            _ => f64::INFINITY,
        };
        if off_by > relative {
            out.push(Difference {
                measure: name.into(),
                saved: s.into_iter().collect(),
                built: b.into_iter().collect(),
                off_by: off_by.is_finite().then_some(off_by),
                tolerance: relative,
            });
        }
    };
    scalar(
        "volume",
        saved.volume_mm3,
        built.volume_mm3,
        &mut differences,
    );
    scalar(
        "area",
        Some(saved.area_mm2),
        Some(built.area_mm2),
        &mut differences,
    );
    let corners = |b: Option<([f64; 3], [f64; 3])>| {
        b.map_or_else(Vec::new, |(lo, hi)| lo.into_iter().chain(hi).collect())
    };
    let off_by = match (saved.bounds_mm, built.bounds_mm) {
        (Some((sl, sh)), Some((bl, bh))) => (0..3)
            .map(|i| (sl[i] - bl[i]).abs().max((sh[i] - bh[i]).abs()))
            .fold(0.0, f64::max),
        (None, None) => 0.0,
        _ => f64::INFINITY,
    };
    if off_by > tolerances.bounds_mm {
        differences.push(Difference {
            measure: "bounds".into(),
            saved: corners(saved.bounds_mm),
            built: corners(built.bounds_mm),
            off_by: off_by.is_finite().then_some(off_by),
            tolerance: tolerances.bounds_mm,
        });
    }
    let centre_tolerance = tolerances
        .centre_mm
        .max(tolerances.centre_relative * saved.diagonal_mm());
    let off_by = (0..3)
        .map(|i| (saved.centre_mm[i] - built.centre_mm[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    if off_by > centre_tolerance {
        differences.push(Difference {
            measure: "centre".into(),
            saved: saved.centre_mm.to_vec(),
            built: built.centre_mm.to_vec(),
            off_by: Some(off_by),
            tolerance: centre_tolerance,
        });
    }
    differences
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(size: [f64; 3]) -> Measures {
        let [x, y, z] = size;
        Measures {
            volume_mm3: Some(x * y * z),
            area_mm2: 2.0 * (x * y + y * z + z * x),
            centre_mm: size.map(|s| s / 2.0),
            bounds_mm: Some(([0.0; 3], size)),
            approximate: false,
            faces: 6,
        }
    }

    #[test]
    fn a_solid_matches_itself_and_a_close_enough_copy() {
        let saved = block([20.0, 10.0, 5.0]);
        assert!(compare(&saved, &saved, &Tolerances::default()).is_empty());
        let mut near = saved;
        near.bounds_mm = Some(([0.0; 3], [20.04, 10.0, 5.0]));
        near.centre_mm[0] += 0.005;
        near.faces = 7;
        assert!(compare(&saved, &near, &Tolerances::default()).is_empty());
    }

    #[test]
    fn each_measure_past_its_tolerance_is_named_with_both_figures() {
        let saved = block([20.0, 10.0, 5.0]);
        let built = block([20.0, 10.0, 6.0]);
        let differences = compare(&saved, &built, &Tolerances::default());
        let names: Vec<_> = differences.iter().map(|d| d.measure.as_str()).collect();
        assert_eq!(names, ["volume", "area", "bounds", "centre"]);
        assert_eq!(differences[0].saved, [1000.0]);
        assert_eq!(differences[0].built, [1200.0]);
        let off = |i: usize| differences[i].off_by.unwrap();
        assert!((off(0) - 0.2).abs() < 1e-12);
        assert!((off(2) - 1.0).abs() < 1e-12);
        assert!((off(3) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn an_approximate_figure_widens_only_volume_and_area() {
        let saved = block([20.0, 10.0, 5.0]);
        let mut built = saved;
        built.volume_mm3 = Some(1002.0);
        built.centre_mm[2] += 0.1;
        let exact = compare(&saved, &built, &Tolerances::default());
        assert_eq!(exact.len(), 2);
        built.approximate = true;
        let approximate = compare(&saved, &built, &Tolerances::default());
        assert_eq!(approximate.len(), 1);
        assert_eq!(approximate[0].measure, "centre");
    }

    #[test]
    fn a_volume_on_one_side_only_differs() {
        let saved = block([20.0, 10.0, 5.0]);
        let mut built = saved;
        built.volume_mm3 = None;
        let differences = compare(&saved, &built, &Tolerances::default());
        assert_eq!(differences.len(), 1);
        assert_eq!(differences[0].off_by, None);
        assert!(differences[0].built.is_empty());
    }
}
