//! A mesh body's history: its start mesh welded once, each step applied
//! to the same editable mesh in turn, so a marking made on the start mesh
//! still names its faces after earlier steps (a vertex keeps the number of
//! the start mesh's vertex it came from).

use kernel_api::{
    KernelError, KernelResult, MeshChainAnswer, MeshOp, MeshRegion, MeshStep, TriMesh,
};
use ogeom::math::{Direction, Plane, Point, Vector};
use ogeom::mesh::{Cap, Editable, Side};
use ogeom::topo::Triangulation;

/// `start` with `ops` applied in turn; a step that fails leaves the mesh
/// as it stood before it, with the failure.
pub(crate) fn build(start: &TriMesh, ops: &[MeshOp]) -> KernelResult<MeshChainAnswer> {
    if !start.indices.len().is_multiple_of(3)
        || start
            .indices
            .iter()
            .any(|i| *i as usize >= start.positions.len())
    {
        return Err(KernelError::InvalidInput("invalid mesh triangles".into()));
    }
    let (tri, welded) = welded(start);
    let mut mesh =
        Editable::from(&tri, 0.0).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
    let mut answer = MeshChainAnswer::default();
    for (k, op) in ops.iter().enumerate() {
        match step(&mut mesh, op, &welded) {
            Ok(note) => answer.steps.push(MeshStep {
                faces: live_faces(&mesh),
                note,
            }),
            Err(why) => {
                answer.failed = Some((k, why));
                break;
            }
        }
    }
    answer.mesh = crate::mesh::display_mesh(&mesh.to_triangulation(), &crate::mesh::Paint::None);
    Ok(answer)
}

/// The start mesh with the vertices at one spot made one, each numbered
/// as the first of them; and each start vertex's number in it.
fn welded(mesh: &TriMesh) -> (Triangulation, Vec<u32>) {
    let mut first: std::collections::HashMap<[u32; 3], u32> = Default::default();
    let welded: Vec<u32> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| *first.entry(p.map(f32::to_bits)).or_insert(i as u32))
        .collect();
    let tri = Triangulation {
        positions: mesh
            .positions
            .iter()
            .map(|p| Point::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect(),
        triangles: mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|v| welded[v as usize]))
            .filter(|[a, b, c]| a != b && b != c && c != a)
            .collect(),
        ..Triangulation::new()
    };
    (tri, welded)
}

fn live_faces(mesh: &Editable) -> usize {
    (0..mesh.face_count() as u32)
        .filter(|f| mesh.face(*f).is_some())
        .count()
}

/// The faces of `mesh` all three of whose corners lie in `region`.
fn faces_in(mesh: &Editable, region: &MeshRegion, welded: &[u32]) -> Vec<u32> {
    let marked: std::collections::HashSet<u32> = match region {
        MeshRegion::Vertices(list) => list
            .iter()
            .filter_map(|v| welded.get(*v as usize).copied())
            .collect(),
        _ => Default::default(),
    };
    let inside = |v: u32| -> bool {
        match region {
            MeshRegion::All => true,
            MeshRegion::Vertices(_) => mesh.source_vertex(v).is_some_and(|s| marked.contains(&s)),
            MeshRegion::Box { min, max } => mesh.position(v).is_some_and(|p| {
                let p = [p.x, p.y, p.z];
                (0..3).all(|k| p[k] >= min[k] && p[k] <= max[k])
            }),
            MeshRegion::Ball { centre, radius } => mesh.position(v).is_some_and(|p| {
                let d = [p.x - centre[0], p.y - centre[1], p.z - centre[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= *radius
            }),
        }
    };
    (0..mesh.face_count() as u32)
        .filter(|f| mesh.face(*f).is_some_and(|t| t.iter().all(|v| inside(*v))))
        .collect()
}

/// One step on `mesh`, and what it did in words.
fn step(mesh: &mut Editable, op: &MeshOp, welded: &[u32]) -> Result<String, String> {
    match op {
        MeshOp::RemoveIslands { smaller_than } => {
            let pieces = mesh.components();
            let total = pieces.len();
            let small: Vec<_> = pieces
                .into_iter()
                .filter(|c| c.bounds.diagonal() < *smaller_than)
                .collect();
            if small.len() == total && total > 0 {
                return Err("every piece is smaller than that: nothing would be left".into());
            }
            let faces: Vec<u32> = small.iter().flat_map(|c| c.faces.iter().copied()).collect();
            mesh.remove_faces(&faces);
            Ok(format!(
                "{} of {total} pieces removed, {} faces",
                small.len(),
                faces.len()
            ))
        }
        MeshOp::Delete { region } => {
            let faces = faces_in(mesh, region, welded);
            if faces.is_empty() {
                return Err("no face lies in the region".into());
            }
            mesh.remove_faces(&faces);
            Ok(format!("{} faces removed", faces.len()))
        }
        MeshOp::Flip { region } => {
            let faces = faces_in(mesh, region, welded);
            mesh.flip_faces(&faces);
            Ok(format!("{} faces turned over", faces.len()))
        }
        MeshOp::Cut {
            origin,
            normal,
            keep_above,
            close,
            bridge,
        } => {
            let tol = crate::tess::tolerances();
            let n = Direction::new(Vector::new(normal[0], normal[1], normal[2]), tol)
                .map_err(|e| format!("the plane has no direction: {e}"))?;
            let plane = Plane::through(Point::new(origin[0], origin[1], origin[2]), n);
            let report = mesh
                .cut(
                    &plane,
                    if *keep_above {
                        Side::Above
                    } else {
                        Side::Below
                    },
                    close.then_some(Cap {
                        bridge: bridge.max(0.0),
                    }),
                )
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "{} loops capped ({} bridged), {} left open",
                report.capped_loops, report.bridged, report.open
            ))
        }
    }
}
