//! Faces picked on a body's mesh: the triangle under the cursor, the face
//! it belongs to, its frame and its triangles.

use core_document::mesh_query::triangle_distance_sq as point_triangle_distance_sq;
use uuid::Uuid;

/// A triangle the pick pass drew under the cursor, as an index into the
/// mesh the document holds for the picked body at `revision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HoveredTriangle {
    pub index: u32,
    pub revision: u64,
}

/// The picked triangle as one of the document's: `drawn` is the body's
/// entry in `DocumentSession::drawn_meshes`, the revision it was last
/// submitted with and its geometry's. A pick drawn with another revision,
/// or a body drawn with a mesh of its own, names no triangle of the
/// document's.
pub(crate) fn document_triangle(
    picked: render_wgpu::PickedTriangle,
    drawn: Option<(u64, u64)>,
) -> Option<HoveredTriangle> {
    let (submitted, geometry) = drawn?;
    (submitted == picked.revision).then_some(HoveredTriangle {
        index: picked.index,
        revision: geometry,
    })
}

/// The triangle `pick` names in `mesh`, the document's mesh at `revision`:
/// its index, one vertex and unit normal. `None` when the pick was taken
/// against another revision, or names no triangle of the mesh with an
/// area.
fn picked_triangle(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
) -> Option<(usize, glam::Vec3, glam::Vec3)> {
    let pick = pick.filter(|p| p.revision == revision)?;
    let index = pick.index as usize;
    let tri = mesh.indices.as_chunks::<3>().0.get(index)?;
    let corner = |i: u32| {
        mesh.positions
            .get(i as usize)
            .map(|p| glam::Vec3::from_array(*p))
    };
    let (a, b, c) = (corner(tri[0])?, corner(tri[1])?, corner(tri[2])?);
    let normal = (b - a).cross(c - a);
    (normal.length_squared() >= 1e-12).then(|| (index, a, normal.normalize()))
}

/// The face under the cursor on `mesh`, the document's mesh at `revision`:
/// on the triangle `pick` names, else on the one nearest `point`.
pub(crate) fn face_ref_at_pick(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
    point: glam::Vec3,
) -> Option<core_document::FaceRef> {
    match picked_triangle(mesh, revision, pick) {
        Some((triangle, anchor, normal)) => {
            Some(face_ref_on(mesh, triangle, anchor, normal, point))
        }
        None => face_ref_from_mesh(mesh, point),
    }
}

/// The kernel face under the cursor on `mesh`, the document's mesh at
/// `revision`, when the mesh records faces: the face of the triangle
/// `pick` names, else of the one nearest `point`.
pub(crate) fn face_id_at_pick(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
    point: [f32; 3],
) -> Option<u32> {
    if mesh.faces.len() != mesh.indices.len() / 3 || mesh.faces.is_empty() {
        return None;
    }
    match picked_triangle(mesh, revision, pick) {
        Some((triangle, _, _)) => Some(mesh.faces[triangle]),
        None => face_id_at(mesh, point),
    }
}

/// Resolve a picked world position to a face reference on `mesh`.
///
/// The GPU pick reconstructs the position from the depth buffer, whose
/// precision varies with view angle and distance: the raw point can sit a
/// millimetre or more off the surface, far enough that a naive containment
/// test misses the face and falls back to whole-body selection. So:
/// find the nearest triangle, take the face plane from ITS exact vertices,
/// and project the picked point onto that plane.
pub(crate) fn face_ref_from_mesh(
    mesh: &kernel_api::TriMesh,
    point: glam::Vec3,
) -> Option<core_document::FaceRef> {
    let (triangle, dist_sq, anchor, normal) = nearest_triangle(mesh, point)?;

    // Sanity bound relative to the model size: the pick already identified
    // this body, so the nearest triangle is the right face unless the
    // position is wildly stale.
    let (min, max) = mesh.bounds()?;
    let diag = (glam::Vec3::from_array(max) - glam::Vec3::from_array(min)).length();
    let limit = (diag * 0.05).max(1.0);
    if dist_sq > limit * limit {
        return None;
    }
    Some(face_ref_on(mesh, triangle, anchor, normal, point))
}

/// The face of `triangle` (one vertex `anchor`, unit `normal`) at `point`.
fn face_ref_on(
    mesh: &kernel_api::TriMesh,
    triangle: usize,
    anchor: glam::Vec3,
    normal: glam::Vec3,
    point: glam::Vec3,
) -> core_document::FaceRef {
    // Project the noisy picked point onto the triangle's exact plane so the
    // face plane (and any sketch placed on it) is depth-error free.
    let projected = point - normal * (point - anchor).dot(normal);
    let surface = mesh
        .faces
        .get(triangle)
        .and_then(|face| mesh.face_surfaces.get(*face as usize))
        .copied();
    let name = mesh
        .faces
        .get(triangle)
        .and_then(|face| mesh.face_names.get(*face as usize))
        .copied()
        .unwrap_or(0);
    core_document::FaceRef {
        name,
        point: projected.to_array(),
        normal: normal.to_array(),
        surface,
    }
}

/// The triangle of `mesh` nearest to `point`: its index, squared distance,
/// one vertex, and unit normal.
fn nearest_triangle(
    mesh: &kernel_api::TriMesh,
    point: glam::Vec3,
) -> Option<(usize, f32, glam::Vec3, glam::Vec3)> {
    let mut best: Option<(usize, f32, glam::Vec3, glam::Vec3)> = None;
    for (index, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let normal = (b - a).cross(c - a);
        if normal.length_squared() < 1e-12 {
            continue;
        }
        let d = point_triangle_distance_sq(point, a, b, c);
        if best.map(|(_, bd, _, _)| d < bd).unwrap_or(true) {
            best = Some((index, d, a, normal.normalize()));
        }
    }
    best
}

/// The sub-mesh of one face, for the selection highlight.
///
/// When the mesh knows which kernel face each triangle came from, the face
/// is the one under the hit, whole: a cylinder wall as much as a flat side.
/// A mesh without that (a sketch, a document saved before faces were
/// recorded) falls back to the plane through the hit, which is exact for a
/// flat face and one strip of a curved one. The face is the one `pick`
/// names on the document's mesh at `revision` ([`face_id_at_pick`]);
/// returned with the sub-mesh, when the mesh records faces.
pub(crate) fn face_submesh(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<(Option<u32>, kernel_api::TriMesh)> {
    if let Some(face) = face_id_at_pick(mesh, revision, pick, point) {
        return Some((Some(face), face_submesh_by_id(mesh, face)?));
    }
    Some((None, coplanar_face_submesh(mesh, point, normal)?))
}

/// The kernel face the triangle nearest `point` was cut from, when the mesh
/// records faces.
pub(crate) fn face_id_at(mesh: &kernel_api::TriMesh, point: [f32; 3]) -> Option<u32> {
    if mesh.faces.len() != mesh.indices.len() / 3 || mesh.faces.is_empty() {
        return None;
    }
    let (tri, _, _, _) = nearest_triangle(mesh, glam::Vec3::from_array(point))?;
    Some(mesh.faces[tri])
}

/// The face under the cursor, drawn translucent in the hover paint: the
/// body the pick found, the kernel face at the picked point, and the copy
/// of that face. Rebuilt only when the picked point moves.
pub(crate) struct FaceHover {
    pub body: Uuid,
    pub face: u32,
    pub revision: u64,
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// The picked point this hover was resolved for.
    pub probe: [f32; 3],
}

/// Where face `face` of `mesh` stands and the way it faces: the centre of
/// its area and its area-weighted outward normal, from the mesh's own
/// normals (the kernel's, outward, where the winding may not be).
pub(crate) fn face_frame(
    mesh: &kernel_api::TriMesh,
    face: u32,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut area_sum = 0.0;
    let mut centre = glam::Vec3::ZERO;
    let mut normal = glam::Vec3::ZERO;
    for (tri, id) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.faces) {
        if *id != face {
            continue;
        }
        let p = |i: u32| {
            mesh.positions
                .get(i as usize)
                .map(|p| glam::Vec3::from_array(*p))
        };
        let (a, b, c) = (p(tri[0])?, p(tri[1])?, p(tri[2])?);
        let area = (b - a).cross(c - a).length() / 2.0;
        area_sum += area;
        centre += (a + b + c) / 3.0 * area;
        for i in tri {
            if let Some(n) = mesh.normals.get(*i as usize) {
                normal += glam::Vec3::from_array(*n) * area;
            }
        }
    }
    (area_sum > 0.0 && normal.length_squared() > 1e-12)
        .then(|| (centre / area_sum, normal.normalize()))
}

/// Every triangle cut from kernel face `face`, copied exactly: the
/// translucent pass passes equal depth, so the copy sits on its surface
/// whichever way the face is wound. A copy lifted along the winding normal
/// would sink into the solid on a face wound inward and never show.
pub(crate) fn face_submesh_by_id(
    mesh: &kernel_api::TriMesh,
    face: u32,
) -> Option<kernel_api::TriMesh> {
    let mut out = kernel_api::TriMesh::default();
    for (tri, id) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.faces) {
        if *id != face {
            continue;
        }
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-12 {
            continue;
        }
        let n = n.normalize();
        let base = out.positions.len() as u32;
        for v in [a, b, c] {
            out.positions.push(v.to_array());
            out.normals.push(n.to_array());
        }
        out.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    (!out.indices.is_empty()).then_some(out)
}

/// The faces picked, in order, after Ctrl picks `face`: taken out when it
/// is among them (by its kernel face), added at the end otherwise.
pub(crate) fn toggled(mut picked: Vec<PickedFace>, face: PickedFace) -> Vec<PickedFace> {
    let id = face.highlight.face;
    match picked.iter().position(|p| {
        id.is_some() && p.highlight.body == face.highlight.body && p.highlight.face == id
    }) {
        Some(i) => {
            picked.remove(i);
        }
        None => picked.push(face),
    }
    picked
}

/// A face picked before the last, kept with its highlight.
#[derive(Debug, Clone)]
pub(crate) struct PickedFace {
    pub face: core_document::FaceRef,
    pub highlight: FaceHighlight,
}

/// Sub-mesh rendered as a picked face's highlight.
#[derive(Debug, Clone)]
pub(crate) struct FaceHighlight {
    pub body: Uuid,
    /// The kernel face, when the mesh records faces.
    pub face: Option<u32>,
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// Bumped whenever the sub-mesh is re-extracted.
    pub revision: u64,
}

/// Every triangle of `mesh` lying on the plane (point, normal), copied
/// exactly. The face as geometry sees it: exact for a flat face, one strip
/// of a curved one, and two flat faces on one plane together.
pub(crate) fn coplanar_face_submesh(
    mesh: &kernel_api::TriMesh,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<kernel_api::TriMesh> {
    const NORMAL_ALIGN: f32 = 0.999;
    const PLANE_TOL: f32 = 0.05;
    let n = glam::Vec3::from_array(normal).normalize();
    let p0 = glam::Vec3::from_array(point);

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for tri in mesh.indices.as_chunks::<3>().0 {
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let tri_n_raw = (b - a).cross(c - a);
        if tri_n_raw.length_squared() < 1e-12 {
            continue;
        }
        let tri_n = tri_n_raw.normalize();
        if tri_n.dot(n) < NORMAL_ALIGN {
            continue;
        }
        if (a - p0).dot(n).abs() > PLANE_TOL {
            continue;
        }
        let base = positions.len() as u32;
        for v in [a, b, c] {
            positions.push(v.to_array());
            normals.push(n.to_array());
        }
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    if indices.is_empty() {
        return None;
    }
    Some(kernel_api::TriMesh {
        positions,
        normals,
        indices,
        ..kernel_api::TriMesh::default()
    })
}

#[cfg(test)]
mod tests {

    /// A face's frame is the centre of its area and the way it faces out,
    /// read from the mesh's normals whatever the triangles' winding.
    #[test]
    fn a_face_frame_is_its_centre_and_outward_normal() {
        // A 2 × 2 square at z = 3 facing +z, one triangle wound backwards,
        // beside a second face.
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [0.0, 0.0, 3.0],
                [2.0, 0.0, 3.0],
                [2.0, 2.0, 3.0],
                [0.0, 2.0, 3.0],
                [9.0, 9.0, 9.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 5],
            indices: vec![0, 1, 2, 0, 3, 2, 1, 2, 4],
            faces: vec![7, 7, 8],
            ..Default::default()
        };
        let (centre, normal) = super::face_frame(&mesh, 7).expect("face 7 is there");
        assert!(
            (centre - glam::Vec3::new(1.0, 1.0, 3.0)).length() < 1e-5,
            "{centre}"
        );
        assert!((normal - glam::Vec3::Z).length() < 1e-5, "{normal}");
        assert!(super::face_frame(&mesh, 3).is_none());
    }

    use super::coplanar_face_submesh;
    use kernel_api::TriMesh;

    /// Two quads: top face at z=1 (normal +Z), bottom at z=0 (normal -Z).
    fn two_face_mesh() -> TriMesh {
        let positions = vec![
            // top (CCW seen from +Z)
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
            // bottom (CCW seen from -Z)
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        TriMesh {
            positions,
            normals: Vec::new(),
            indices,
            ..TriMesh::default()
        }
    }

    /// The kept face ids are the ones searched afresh after any of what
    /// they read moves: the points, the mesh (another at the same
    /// revision, or the same at another) and the body.
    #[test]
    fn kept_face_ids_follow_the_points_and_the_mesh() {
        use std::sync::Arc;
        let mesh = Arc::new(TriMesh {
            faces: vec![3, 3, 5, 5],
            ..two_face_mesh()
        });
        let renumbered = Arc::new(TriMesh {
            faces: vec![9, 9, 2, 2],
            ..two_face_mesh()
        });
        let (a, b) = (
            core_document::BodyId(uuid::Uuid::new_v4()),
            core_document::BodyId(uuid::Uuid::new_v4()),
        );
        let (top, bottom) = ([0.5, 0.5, 1.0], [0.5, 0.5, 0.0]);
        let kept = super::PickedFaceIds::default();
        for (body, mesh, revision, points) in [
            (a, &mesh, 1, vec![top]),
            (a, &mesh, 1, vec![top]),
            (a, &mesh, 1, vec![top, bottom]),
            (a, &renumbered, 1, vec![top, bottom]),
            (a, &renumbered, 2, vec![bottom]),
            (a, &mesh, 2, vec![bottom]),
            (b, &mesh, 2, vec![bottom]),
        ] {
            let searched: Vec<_> = points
                .iter()
                .filter_map(|p| Some((body, super::face_id_at(mesh, *p)?)))
                .collect();
            assert_eq!(kept.of(body, mesh, revision, &points), searched);
        }
        assert_eq!(kept.of(b, &renumbered, 2, &[top]), [(b, 9)]);
    }

    #[test]
    fn extracts_only_the_hit_plane_exactly_where_it_lies() {
        let mesh = two_face_mesh();
        let sub = coplanar_face_submesh(&mesh, [0.5, 0.5, 1.0], [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(sub.indices.len(), 6, "only the top quad's two triangles");
        assert!(
            sub.positions.iter().all(|p| (p[2] - 1.0).abs() < 1e-6),
            "the copy sits on the face, not above it"
        );
        // No triangle lies in a side plane.
        assert!(coplanar_face_submesh(&mesh, [1.0, 0.5, 0.5], [1.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn noisy_pick_point_still_resolves_the_face() {
        use super::face_ref_from_mesh;
        let mesh = two_face_mesh();
        // Simulate depth-buffer error: 0.4 above the top face.
        let face = face_ref_from_mesh(&mesh, glam::Vec3::new(0.5, 0.5, 1.4)).unwrap();
        assert!((glam::Vec3::from_array(face.normal) - glam::Vec3::Z).length() < 1e-4);
        assert!(
            (face.point[2] - 1.0).abs() < 1e-4,
            "point projected onto the exact face plane: {:?}",
            face.point
        );
        let sub = coplanar_face_submesh(&mesh, face.point, face.normal).unwrap();
        assert_eq!(sub.indices.len(), 6);
        assert!(
            face_ref_from_mesh(&mesh, glam::Vec3::new(50.0, 50.0, 50.0)).is_none(),
            "a point far off every face is no pick"
        );
    }

    /// A point between the two faces of [`two_face_mesh`], nearer the
    /// bottom: where depth noise could put a pick on the top face of a thin
    /// wall.
    const BETWEEN: glam::Vec3 = glam::Vec3::new(0.5, 0.5, 0.3);

    fn top_and_bottom() -> TriMesh {
        TriMesh {
            faces: vec![1, 1, 2, 2],
            ..two_face_mesh()
        }
    }

    #[test]
    fn a_picked_triangle_names_its_face_wherever_the_point_lies() {
        use super::{HoveredTriangle, face_id_at_pick, face_ref_at_pick};
        let mesh = top_and_bottom();
        let top = Some(HoveredTriangle {
            index: 1,
            revision: 5,
        });
        let face = face_ref_at_pick(&mesh, 5, top, BETWEEN).unwrap();
        assert!((glam::Vec3::from_array(face.normal) - glam::Vec3::Z).length() < 1e-4);
        assert!(
            (face.point[2] - 1.0).abs() < 1e-6,
            "projected onto the picked triangle's plane: {:?}",
            face.point
        );
        assert_eq!(face_id_at_pick(&mesh, 5, top, BETWEEN.to_array()), Some(1));
        let (id, sub) =
            super::face_submesh(&mesh, 5, top, BETWEEN.to_array(), face.normal).unwrap();
        assert_eq!(id, Some(1));
        assert!(
            sub.positions.iter().all(|p| p[2] == 1.0),
            "the highlight is the top face"
        );
    }

    #[test]
    fn a_stale_or_foreign_triangle_falls_back_to_the_nearest() {
        use super::{HoveredTriangle, face_id_at_pick, face_ref_at_pick};
        let mesh = top_and_bottom();
        let picks = [
            // Taken against another revision of the mesh.
            Some(HoveredTriangle {
                index: 1,
                revision: 4,
            }),
            // Past the mesh's triangles.
            Some(HoveredTriangle {
                index: 4,
                revision: 5,
            }),
            None,
        ];
        for pick in picks {
            let face = face_ref_at_pick(&mesh, 5, pick, BETWEEN).unwrap();
            assert!(face.point[2].abs() < 1e-6, "{pick:?}: the bottom, nearest");
            assert_eq!(
                face_id_at_pick(&mesh, 5, pick, BETWEEN.to_array()),
                Some(2),
                "{pick:?}"
            );
        }
    }

    #[test]
    fn a_pick_counts_in_the_document_mesh_only_as_it_was_drawn() {
        use super::{HoveredTriangle, document_triangle};
        let picked = render_wgpu::PickedTriangle {
            index: 3,
            revision: 9,
        };
        assert_eq!(
            document_triangle(picked, Some((9, 2))),
            Some(HoveredTriangle {
                index: 3,
                revision: 2
            }),
            "drawn as submitted: the geometry's revision"
        );
        assert_eq!(
            document_triangle(picked, Some((10, 2))),
            None,
            "re-meshed since"
        );
        assert_eq!(
            document_triangle(picked, None),
            None,
            "drawn with its own mesh"
        );
    }

    /// A quarter-cylinder wall as three strips that turn 30° each, all cut
    /// from one kernel face, next to one flat strip from another.
    fn curved_face_mesh() -> TriMesh {
        let mut mesh = TriMesh::default();
        let mut strip = |angle_a: f32, angle_b: f32, face: u32| {
            let (sa, ca) = angle_a.to_radians().sin_cos();
            let (sb, cb) = angle_b.to_radians().sin_cos();
            let base = mesh.positions.len() as u32;
            mesh.positions
                .extend([[ca, sa, 0.0], [cb, sb, 0.0], [cb, sb, 1.0], [ca, sa, 1.0]]);
            mesh.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            mesh.faces.extend([face, face]);
        };
        strip(0.0, 30.0, 7);
        strip(30.0, 60.0, 7);
        strip(60.0, 90.0, 7);
        strip(90.0, 120.0, 8);
        mesh
    }

    /// A hit on the middle strip selects all of its kernel face; with no
    /// face ids, only the plane through the hit.
    #[test]
    fn a_curved_face_selects_whole_when_the_mesh_knows_its_faces() {
        use super::face_submesh;
        let mut mesh = curved_face_mesh();
        let hit = [45f32.to_radians().cos(), 45f32.to_radians().sin(), 0.5];
        let normal = [hit[0], hit[1], 0.0];
        let (face, sub) = face_submesh(&mesh, 0, None, hit, normal).unwrap();
        assert_eq!(face, Some(7));
        assert_eq!(
            sub.indices.len(),
            3 * 6,
            "all three strips of face 7, not the fourth"
        );
        mesh.faces.clear();
        let (face, sub) = face_submesh(&mesh, 0, None, hit, normal).unwrap();
        assert_eq!(face, None);
        assert_eq!(
            sub.indices.len(),
            6,
            "the plane through the hit is one strip"
        );
    }
}

/// Which mesh an answer was worked out on: the body's mesh as an
/// allocation and its geometry revision. The weak handle keeps the
/// allocation's address from going to another mesh while the stamp is
/// kept, and lets the mesh's data go when the body drops it.
#[derive(Debug, Clone)]
pub(crate) struct MeshStamp {
    mesh: std::sync::Weak<kernel_api::TriMesh>,
    revision: u64,
}

impl MeshStamp {
    pub fn of(mesh: &std::sync::Arc<kernel_api::TriMesh>, revision: u64) -> Self {
        Self {
            mesh: std::sync::Arc::downgrade(mesh),
            revision,
        }
    }
}

impl PartialEq for MeshStamp {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.mesh.ptr_eq(&other.mesh)
    }
}

/// The selected faces' ids on their body's mesh ([`face_id_at`] of each
/// picked point), kept with what they were found from so a frame that
/// changes none of it reads them rather than searching the mesh again.
#[derive(Default)]
pub(crate) struct PickedFaceIds(std::cell::RefCell<Option<(PickedFacesKey, FaceIds)>>);

/// Faces as their body's mesh numbers them.
type FaceIds = Vec<(core_document::BodyId, u32)>;

#[derive(PartialEq)]
struct PickedFacesKey {
    body: core_document::BodyId,
    mesh: MeshStamp,
    points: Vec<[u32; 3]>,
}

impl PickedFaceIds {
    /// The id of the face at each of `points` on `mesh`, the mesh of
    /// `body` at `revision`, in order; a point on no face is left out.
    pub fn of(
        &self,
        body: core_document::BodyId,
        mesh: &std::sync::Arc<kernel_api::TriMesh>,
        revision: u64,
        points: &[[f32; 3]],
    ) -> FaceIds {
        let key = PickedFacesKey {
            body,
            mesh: MeshStamp::of(mesh, revision),
            points: points.iter().map(|p| p.map(f32::to_bits)).collect(),
        };
        let mut kept = self.0.borrow_mut();
        if let Some((_, ids)) = kept.as_ref().filter(|(k, _)| *k == key) {
            return ids.clone();
        }
        let ids: FaceIds = points
            .iter()
            .filter_map(|p| Some((body, face_id_at(mesh, *p)?)))
            .collect();
        *kept = Some((key, ids.clone()));
        ids
    }
}
