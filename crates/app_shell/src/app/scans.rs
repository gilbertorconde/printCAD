//! What the app says of a scan body: its size and density from its mesh,
//! and the units its file was read in.

use core_document::{BodyId, Document, ScanInfo};
use kernel_api::{ImportedModel, LengthUnit, TriMesh};

/// A scan mesh's size and density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeshFacts {
    pub triangles: usize,
    pub points: usize,
    /// The median length of its triangles' edges, in millimetres: how far
    /// apart the scanner's points lie.
    pub spacing: f64,
}

/// At most this many triangles are read for the spacing: a median over a
/// sample spread through the mesh reads the same as over all of it.
const SPACING_SAMPLE: usize = 20_000;

pub(crate) fn mesh_facts(mesh: &TriMesh) -> MeshFacts {
    let triangles = mesh.indices.len() / 3;
    let step = triangles.div_ceil(SPACING_SAMPLE).max(1);
    let mut lengths: Vec<f64> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .step_by(step)
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
        .filter_map(|(a, b)| {
            let (a, b) = (
                mesh.positions.get(a as usize)?,
                mesh.positions.get(b as usize)?,
            );
            let d: f64 = (0..3).map(|i| f64::from(a[i] - b[i]).powi(2)).sum();
            Some(d.sqrt())
        })
        .collect();
    let spacing = if lengths.is_empty() {
        0.0
    } else {
        let middle = lengths.len() / 2;
        *lengths.select_nth_unstable_by(middle, f64::total_cmp).1
    };
    MeshFacts {
        triangles,
        points: mesh.positions.len(),
        spacing,
    }
}

/// The units an STL can be read in, as the question and Preferences offer
/// them.
pub(crate) const MESH_UNITS: [(LengthUnit, &str); 4] = [
    (LengthUnit::Millimetre, "Millimetres"),
    (LengthUnit::Centimetre, "Centimetres"),
    (LengthUnit::Metre, "Metres"),
    (LengthUnit::Inch, "Inches"),
];

/// How many millimetres one of `unit` is.
pub(crate) fn millimetres_per(unit: LengthUnit) -> f32 {
    match unit {
        LengthUnit::Millimetre => 1.0,
        LengthUnit::Centimetre => 10.0,
        LengthUnit::Metre => 1000.0,
        LengthUnit::Inch => 25.4,
        LengthUnit::Foot => 304.8,
    }
}

/// The name a command takes a unit by, and back.
pub(crate) fn unit_key(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Millimetre => "mm",
        LengthUnit::Centimetre => "cm",
        LengthUnit::Metre => "m",
        LengthUnit::Inch => "inch",
        LengthUnit::Foot => "ft",
    }
}

pub(crate) fn unit_of_key(key: &str) -> Option<LengthUnit> {
    MESH_UNITS
        .iter()
        .map(|(u, _)| *u)
        .chain([LengthUnit::Foot])
        .find(|u| unit_key(*u).eq_ignore_ascii_case(key))
}

/// Whether a file is one whose units the reader cannot know, so the app
/// asks: an STL.
pub(crate) fn has_no_units(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("stl"))
}

/// The meshes of a model read in `unit`, brought to millimetres.
pub(crate) fn scale_to_millimetres(model: &mut ImportedModel, unit: LengthUnit) {
    let k = millimetres_per(unit);
    if k == 1.0 {
        return;
    }
    for body in &mut model.bodies {
        for p in &mut body.mesh.positions {
            *p = p.map(|c| c * k);
        }
        if let Some((lo, hi)) = &mut body.bounds_mm {
            *lo = lo.map(|c| c * k);
            *hi = hi.map(|c| c * k);
        }
    }
}

/// A unit's name as a sentence says it.
pub(crate) fn unit_name(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Millimetre => "millimetres",
        LengthUnit::Centimetre => "centimetres",
        LengthUnit::Metre => "metres",
        LengthUnit::Inch => "inches",
        LengthUnit::Foot => "feet",
    }
}

/// Mark the mesh bodies among `bodies` as scans, read in `unit`; how
/// many were.
pub(crate) fn mark_scans(document: &mut Document, bodies: &[BodyId], unit: LengthUnit) -> usize {
    let info = ScanInfo {
        read_in: (unit != LengthUnit::Millimetre).then_some(unit),
    };
    bodies
        .iter()
        .filter(|body| document.set_body_scan(**body, Some(info)))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An STL read in inches comes in 25.4 times larger, its bounds too;
    /// units are named as commands take them.
    #[test]
    fn a_mesh_read_in_inches_is_scaled_to_millimetres() {
        let mut model = ImportedModel {
            bodies: vec![kernel_api::ImportedBody {
                name: None,
                mesh: TriMesh {
                    positions: vec![[1.0, 2.0, 0.0]],
                    ..Default::default()
                },
                brep_blob: Vec::new(),
                face_colors: Vec::new(),
                bounds_mm: Some(([0.0; 3], [1.0, 2.0, 0.0])),
                health: None,
                layers: Vec::new(),
            }],
            ..Default::default()
        };
        scale_to_millimetres(&mut model, unit_of_key("INCH").unwrap());
        assert_eq!(model.bodies[0].mesh.positions, vec![[25.4, 50.8, 0.0]]);
        assert_eq!(
            model.bodies[0].bounds_mm,
            Some(([0.0; 3], [25.4, 50.8, 0.0]))
        );
        assert_eq!(unit_of_key("furlong"), None);
        assert!(has_no_units(std::path::Path::new("part.STL")));
        assert!(!has_no_units(std::path::Path::new("part.3mf")));
    }

    /// A grid of unit squares, two triangles each: edges of 1 and √2,
    /// two of every three 1, so the median spacing is 1.
    #[test]
    fn a_grid_spaces_its_points_by_its_pitch() {
        let n = 30;
        let positions: Vec<[f32; 3]> = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f32, y as f32, 0.0]))
            .collect();
        let at = |x: u32, y: u32| y * (n + 1) + x;
        let indices = (0..n)
            .flat_map(|y| (0..n).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                [
                    at(x, y),
                    at(x + 1, y),
                    at(x + 1, y + 1),
                    at(x, y),
                    at(x + 1, y + 1),
                    at(x, y + 1),
                ]
            })
            .collect();
        let facts = mesh_facts(&TriMesh {
            positions,
            indices,
            ..Default::default()
        });
        assert_eq!((facts.triangles, facts.points), (1800, 961));
        assert!((facts.spacing - 1.0).abs() < 1e-9, "{facts:?}");
    }
}
