//! The one way a mesh handed to the kernel adapter becomes a kernel
//! triangulation: checked first, so a bad index or a point that is not a
//! number is an error and never a panic, then welded as the caller asks.
//! A mesh comes from files, documents and packages, so nothing about it is
//! taken on trust.

use kernel_api::{KernelError, KernelResult, TriMesh};
use ogeom::math::Point;
use ogeom::topo::Triangulation;
use std::collections::HashMap;

/// How vertices lying at one spot are treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Weld {
    /// Every vertex kept as given.
    No,
    /// Vertices at one spot made one, the triangulation holding each spot
    /// once (a drawn mesh is split at its creases for shading; the kernel
    /// follows a surface across shared edges).
    Compact,
    /// Triangles pointed at the first vertex at their spot, every vertex
    /// kept, so the triangulation's vertex numbers are the mesh's.
    InPlace,
}

/// What becomes of a triangle whose corners are not three distinct
/// vertices once welded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Degenerate {
    Keep,
    Drop,
}

/// A checked mesh as a kernel triangulation.
pub(crate) struct Intake {
    pub tri: Triangulation,
    /// Each mesh vertex's vertex in `tri`.
    pub vertex: Vec<u32>,
    /// Each vertex of `tri`'s first mesh vertex.
    pub first: Vec<u32>,
}

/// Whether `mesh` is whole triangles over its own vertices, every point a
/// number.
pub(crate) fn check(mesh: &TriMesh) -> KernelResult<()> {
    let invalid = |why: String| Err(KernelError::InvalidInput(why));
    if !mesh.indices.len().is_multiple_of(3) {
        return invalid(format!(
            "{} triangle corners are not whole triangles",
            mesh.indices.len()
        ));
    }
    if let Some(i) = mesh
        .indices
        .iter()
        .find(|i| **i as usize >= mesh.positions.len())
    {
        return invalid(format!(
            "the triangles name vertex {i} of a mesh of {}",
            mesh.positions.len()
        ));
    }
    if let Some(v) = mesh
        .positions
        .iter()
        .position(|p| p.iter().any(|c| !c.is_finite()))
    {
        return invalid(format!("vertex {v} is not a point"));
    }
    Ok(())
}

/// `mesh`, checked, as a triangulation welded as `weld` says.
pub(crate) fn intake(mesh: &TriMesh, weld: Weld, degenerate: Degenerate) -> KernelResult<Intake> {
    check(mesh)?;
    let (vertex, first) = match weld {
        Weld::No => {
            let all: Vec<u32> = (0..mesh.positions.len() as u32).collect();
            (all.clone(), all)
        }
        Weld::Compact => {
            let mut at = HashMap::new();
            let mut first = Vec::new();
            let vertex = mesh
                .positions
                .iter()
                .enumerate()
                .map(|(v, p)| {
                    *at.entry(p.map(f32::to_bits)).or_insert_with(|| {
                        first.push(v as u32);
                        first.len() as u32 - 1
                    })
                })
                .collect();
            (vertex, first)
        }
        Weld::InPlace => {
            let mut at = HashMap::new();
            let vertex = mesh
                .positions
                .iter()
                .enumerate()
                .map(|(v, p)| *at.entry(p.map(f32::to_bits)).or_insert(v as u32))
                .collect();
            (vertex, (0..mesh.positions.len() as u32).collect())
        }
    };
    let triangles = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| t.map(|v| vertex[v as usize]))
        .filter(|[a, b, c]| degenerate == Degenerate::Keep || (a != b && b != c && c != a))
        .collect();
    let tri = Triangulation {
        positions: first
            .iter()
            .map(|v| mesh.positions[*v as usize])
            .map(|p| Point::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect(),
        triangles,
        ..Triangulation::new()
    };
    Ok(Intake { tri, vertex, first })
}

/// `mesh`, checked, with the vertices at one spot made one.
pub(crate) fn welded(mesh: &TriMesh) -> KernelResult<Triangulation> {
    intake(mesh, Weld::Compact, Degenerate::Keep).map(|i| i.tri)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> TriMesh {
        TriMesh {
            positions,
            indices,
            ..TriMesh::default()
        }
    }

    #[test]
    fn a_bad_mesh_is_an_error() {
        let p = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        for (bad, said) in [
            (mesh(p.clone(), vec![0, 1]), "whole triangles"),
            (mesh(p.clone(), vec![0, 1, 3]), "vertex 3"),
            (
                mesh(
                    vec![[0.0; 3], [f32::NAN, 0.0, 0.0], [0.0; 3]],
                    vec![0, 1, 2],
                ),
                "not a point",
            ),
        ] {
            let Err(e) = intake(&bad, Weld::Compact, Degenerate::Keep) else {
                panic!("{said} accepted");
            };
            assert!(e.to_string().contains(said), "{e}");
        }
    }

    #[test]
    fn welds_compact_or_in_place() {
        // Two triangles sharing an edge, its two vertices given twice, and
        // a third triangle that collapses once welded.
        let p = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let m = mesh(p, vec![0, 1, 2, 3, 5, 4, 1, 3, 4]);
        let compact = intake(&m, Weld::Compact, Degenerate::Keep).unwrap();
        assert_eq!(compact.tri.positions.len(), 4);
        assert_eq!(compact.vertex, [0, 1, 2, 1, 2, 3]);
        assert_eq!(compact.first, [0, 1, 2, 5]);
        assert_eq!(compact.tri.triangles.len(), 3);
        let in_place = intake(&m, Weld::InPlace, Degenerate::Drop).unwrap();
        assert_eq!(in_place.tri.positions.len(), 6);
        assert_eq!(in_place.tri.triangles, [[0, 1, 2], [1, 5, 2]]);
    }
}
