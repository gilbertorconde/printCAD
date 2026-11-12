//! What a path or a cam joint keeps of the body it runs on: an edge as a
//! polyline, a face as triangles, read from the body's mesh in its own
//! frame, and the nearest point of either to a point.

use core_document::mesh_query::{closest_on_triangle, nearest_on_segment};
use glam::{DVec3, Vec3};
use kernel_api::TriMesh;

/// The kernel edge whose outline passes nearest `near`, as a polyline in
/// order; the nearest outline segment alone where the mesh names no edges.
pub fn edge_polyline(mesh: &TriMesh, near: [f32; 3]) -> Option<Vec<[f32; 3]>> {
    let near = Vec3::from_array(near);
    let segment = |k: usize| {
        let (a, b) = (mesh.edges[2 * k] as usize, mesh.edges[2 * k + 1] as usize);
        (
            Vec3::from_array(mesh.positions[a]),
            Vec3::from_array(mesh.positions[b]),
        )
    };
    let count = mesh.edges.len() / 2;
    let nearest = (0..count).min_by(|&i, &j| {
        let (a, b) = segment(i);
        let (c, d) = segment(j);
        distance_to_segment(near, a, b).total_cmp(&distance_to_segment(near, c, d))
    })?;
    let Some(id) = mesh.edge_ids.get(nearest).copied() else {
        let (a, b) = segment(nearest);
        return Some(vec![a.to_array(), b.to_array()]);
    };
    let mut pieces: Vec<(Vec3, Vec3)> = (0..count)
        .filter(|k| mesh.edge_ids.get(*k) == Some(&id))
        .map(segment)
        .collect();
    // Chain the pieces end to end, from the first, both ways.
    let (a, b) = pieces.remove(0);
    let mut line = std::collections::VecDeque::from([a, b]);
    let close = |p: Vec3, q: Vec3| p.distance(q) < 1e-4;
    while !pieces.is_empty() {
        let (back, front) = (*line.back()?, *line.front()?);
        let Some(k) = pieces.iter().position(|(p, q)| {
            close(*p, back) || close(*q, back) || close(*p, front) || close(*q, front)
        }) else {
            break;
        };
        let (p, q) = pieces.remove(k);
        if close(p, back) {
            line.push_back(q);
        } else if close(q, back) {
            line.push_back(p);
        } else if close(q, front) {
            line.push_front(p);
        } else {
            line.push_front(q);
        }
    }
    Some(line.into_iter().map(|p| p.to_array()).collect())
}

/// The triangles of the kernel face nearest `near`, wound as the mesh
/// winds them (outward), flattened three corners at a time; the nearest
/// triangle alone where the mesh names no faces.
pub fn face_triangles(mesh: &TriMesh, near: [f32; 3]) -> Option<Vec<[f32; 3]>> {
    let near = Vec3::from_array(near);
    let corners = |t: usize| {
        [0, 1, 2].map(|i| Vec3::from_array(mesh.positions[mesh.indices[3 * t + i] as usize]))
    };
    let count = mesh.indices.len() / 3;
    let near = near.as_dvec3();
    let wide = |t: usize| corners(t).map(|c| c.as_dvec3());
    let nearest = (0..count).min_by(|&i, &j| {
        let on = |[a, b, c]: [DVec3; 3]| closest_on_triangle(near, a, b, c).distance(near);
        on(wide(i)).total_cmp(&on(wide(j)))
    })?;
    let face = mesh.faces.get(nearest).copied();
    Some(
        (0..count)
            .filter(|t| match face {
                Some(f) => mesh.faces.get(*t) == Some(&f),
                None => *t == nearest,
            })
            .flat_map(|t| corners(t).map(|p| p.to_array()))
            .collect(),
    )
}

/// Keep, for a path or a cam, the edge or face of the other body its
/// fixed end was picked on, from that body's mesh in its own frame.
pub fn take_shape(document: &core_document::Document, joint: &mut crate::JointFeature) {
    let near = joint.fixed.parts().0.as_vec3().to_array();
    let Some((mesh, _)) = document.local_geometry(joint.other_body) else {
        return;
    };
    joint.shape = match joint.kind {
        crate::JointKind::Path => edge_polyline(&mesh, near).unwrap_or_default(),
        crate::JointKind::Cam { .. } => face_triangles(&mesh, near).unwrap_or_default(),
        _ => return,
    };
}

fn distance_to_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    nearest_on_segment(p.as_dvec3(), a.as_dvec3(), b.as_dvec3()).distance(p.as_dvec3()) as f32
}

/// The point of the polyline nearest `p`.
pub fn nearest_on_polyline(p: DVec3, line: &[DVec3]) -> Option<DVec3> {
    line.windows(2)
        .map(|w| nearest_on_segment(p, w[0], w[1]))
        .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
        .or_else(|| line.first().copied())
}

/// The nearest point of the triangles (three corners each) to `p`, and
/// the outward normal of the triangle it lies on.
pub fn nearest_on_triangles(p: DVec3, corners: &[DVec3]) -> Option<(DVec3, DVec3)> {
    corners
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let q = closest_on_triangle(p, t[0], t[1], t[2]);
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            (q, n)
        })
        .min_by(|a, b| a.0.distance(p).total_cmp(&b.0.distance(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_is_chained_and_a_face_gathered() {
        // A square's outline as two edges: the bottom and right side (edge
        // 0, given out of order) and the top and left (edge 1).
        let mesh = TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 10.0, 0.0],
                [0.0, 10.0, 0.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            faces: vec![5, 5],
            edges: vec![1, 2, 0, 1, 2, 3, 3, 0],
            edge_ids: vec![0, 0, 1, 1],
            ..TriMesh::default()
        };
        let line = edge_polyline(&mesh, [4.0, 0.2, 0.0]).unwrap();
        assert_eq!(line.len(), 3);
        assert!(line.contains(&[10.0, 10.0, 0.0]) && line[1] == [10.0, 0.0, 0.0]);
        let face = face_triangles(&mesh, [2.0, 8.0, 0.0]).unwrap();
        assert_eq!(face.len(), 6, "both triangles of face 5");
        let wide = |p: &[f32; 3]| DVec3::from_array(p.map(f64::from));
        let corners: Vec<DVec3> = face.iter().map(wide).collect();
        let (q, n) = nearest_on_triangles(DVec3::new(3.0, 4.0, 7.0), &corners).unwrap();
        assert!(q.distance(DVec3::new(3.0, 4.0, 0.0)) < 1e-9);
        assert!((n.z - 1.0).abs() < 1e-9);
        let line: Vec<DVec3> = line.iter().map(wide).collect();
        let on = nearest_on_polyline(DVec3::new(12.0, 5.0, 0.0), &line).unwrap();
        assert!(on.distance(DVec3::new(10.0, 5.0, 0.0)) < 1e-9);
    }
}
