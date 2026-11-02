//! A mesh body drawn shaded as its look asks: every triangle flat (sharp),
//! or its normals averaged across the triangles meeting at a point that
//! turn less than a crease angle from each other (smooth). The triangles
//! keep their order and faces, so picks read as on the mesh it came from.

use glam::Vec3;
use kernel_api::TriMesh;

/// `mesh` with a vertex per triangle corner, each corner's normal its
/// triangle's (`crease_deg` `None`) or the area-weighted average of the
/// triangles at its point within `crease_deg` of its own.
pub fn reshade(mesh: &TriMesh, crease_deg: Option<f32>) -> TriMesh {
    let tris = mesh.indices.as_chunks::<3>().0;
    let at = |v: u32| Vec3::from(mesh.positions[v as usize]);
    // Each triangle's normal, its length twice the area.
    let normals: Vec<Vec3> = tris
        .iter()
        .map(|[a, b, c]| (at(*b) - at(*a)).cross(at(*c) - at(*a)))
        .collect();
    let mut by_point: std::collections::HashMap<[u32; 3], Vec<usize>> = Default::default();
    if crease_deg.is_some() {
        for (t, tri) in tris.iter().enumerate() {
            for v in tri {
                by_point
                    .entry(mesh.positions[*v as usize].map(f32::to_bits))
                    .or_default()
                    .push(t);
            }
        }
    }
    let cos = crease_deg.map(|d| d.to_radians().cos());
    let mut out = TriMesh {
        positions: Vec::with_capacity(tris.len() * 3),
        normals: Vec::with_capacity(tris.len() * 3),
        indices: (0..(tris.len() * 3) as u32).collect(),
        ..mesh.clone()
    };
    out.colors.clear();
    let coloured = mesh.colors.len() == mesh.positions.len();
    // Where each of the mesh's vertices went, for its outline.
    let mut moved = vec![u32::MAX; mesh.positions.len()];
    for (t, tri) in tris.iter().enumerate() {
        let own = normals[t].normalize_or_zero();
        for v in tri {
            let normal = match cos {
                None => own,
                Some(cos) => by_point[&mesh.positions[*v as usize].map(f32::to_bits)]
                    .iter()
                    .map(|u| normals[*u])
                    .filter(|n| n.normalize_or_zero().dot(own) >= cos)
                    .sum::<Vec3>()
                    .normalize_or(own),
            };
            if moved[*v as usize] == u32::MAX {
                moved[*v as usize] = out.positions.len() as u32;
            }
            out.positions.push(mesh.positions[*v as usize]);
            out.normals.push(normal.to_array());
            if coloured {
                out.colors.push(mesh.colors[*v as usize]);
            }
        }
    }
    out.edges = mesh
        .edges
        .iter()
        .map(|v| moved.get(*v as usize).copied().unwrap_or(u32::MAX))
        .collect();
    if out.edges.contains(&u32::MAX) {
        out.edges.clear();
        out.edge_ids.clear();
        out.edge_faces.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles folded 90° along a shared edge: sharp keeps each
    /// flat, smooth at 30° keeps the fold, smooth at 100° rounds it.
    #[test]
    fn a_fold_stays_sharp_below_its_crease_and_rounds_above() {
        let mesh = TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            indices: vec![0, 1, 2, 0, 3, 1],
            edges: vec![0, 1],
            faces: vec![0, 1],
            ..TriMesh::default()
        };
        let sharp = reshade(&mesh, None);
        assert_eq!((sharp.positions.len(), sharp.faces.len()), (6, 2));
        assert_eq!(sharp.normals[0], [0.0, 0.0, 1.0]);
        let kept = reshade(&mesh, Some(30.0));
        assert_eq!(kept.normals[0], [0.0, 0.0, 1.0]);
        let round = reshade(&mesh, Some(100.0));
        let n = Vec3::from(round.normals[0]);
        assert!(n.z > 0.5 && n.y > 0.5, "{n}");
        assert_eq!(round.edges, [0, 1], "the outline follows its vertices");
    }
}
