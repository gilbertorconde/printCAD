//! What the app says of a scan body: its size and density from its mesh,
//! and the units its file was read in.

use core_document::{BodyId, Document, ScanInfo};
use kernel_api::{LengthUnit, TriMesh};

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

/// Mark the mesh bodies among `bodies` as scans; how many were.
pub(crate) fn mark_scans(document: &mut Document, bodies: &[BodyId]) -> usize {
    bodies
        .iter()
        .filter(|body| document.set_body_scan(**body, Some(ScanInfo::default())))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

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
