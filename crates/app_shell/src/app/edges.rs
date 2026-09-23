//! Edge picking: the outline segments of the body under the cursor are
//! tested on the CPU, in pixels, and the nearest within reach names its
//! kernel edge. The hovered and selected edges are drawn as line bodies
//! over the scene.

use std::sync::Arc;

use core_document::{BodyId, EdgeRef};
use glam::Vec3;
use kernel_api::TriMesh;
use uuid::Uuid;

use crate::PrintCadApp;

/// How close, in pixels, the cursor must come to an edge.
const EDGE_PICK_PX: f32 = 6.0;

/// An edge under the cursor or picked: the body, the kernel edge, the
/// point on it nearest the cursor, its direction there and its length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EdgeHit {
    pub body: Uuid,
    pub edge: u32,
    pub point: [f32; 3],
    pub direction: [f32; 3],
    pub length_mm: f32,
}

impl EdgeHit {
    pub fn as_ref(&self) -> EdgeRef {
        EdgeRef {
            point: self.point,
            direction: self.direction,
            length_mm: self.length_mm,
        }
    }
}

impl PrintCadApp {
    /// The edge of the hovered body nearest the cursor, within reach.
    pub(crate) fn edge_under_cursor(&self) -> Option<EdgeHit> {
        let body = self.session.hovered_body?;
        let (cx, cy) = self.cursor_in_viewport?;
        let geometry = self.session.document.imported_geometry(BodyId(body))?;
        let mesh = &geometry.mesh;
        if mesh.edge_ids.len() != mesh.edges.len() / 2 {
            return None;
        }
        // The cursor is viewport-local, so the endpoints project into the
        // same space: the viewport's own pixels, not the window's.
        let camera = &self.session.camera;
        let project = |p: [f32; 3]| camera.world_to_viewport(Vec3::from_array(p));
        let cursor = glam::Vec2::new(cx, cy);
        let mut best: Option<(f32, usize)> = None;
        for (segment, pair) in mesh.edges.chunks(2).enumerate() {
            let a = mesh.positions[pair[0] as usize];
            let b = mesh.positions[pair[1] as usize];
            let (Some(pa), Some(pb)) = (project(a), project(b)) else {
                continue;
            };
            let (pa, pb) = (glam::Vec2::new(pa.0, pa.1), glam::Vec2::new(pb.0, pb.1));
            let ab = pb - pa;
            let t = if ab.length_squared() < 1e-6 {
                0.0
            } else {
                ((cursor - pa).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            };
            let d = (pa + ab * t - cursor).length();
            if d <= EDGE_PICK_PX && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, segment));
            }
        }
        let (_, segment) = best?;
        let edge = mesh.edge_ids[segment];
        let pair = &mesh.edges[segment * 2..segment * 2 + 2];
        let a = Vec3::from_array(mesh.positions[pair[0] as usize]);
        let b = Vec3::from_array(mesh.positions[pair[1] as usize]);
        let direction = (b - a).normalize_or_zero();
        Some(EdgeHit {
            body,
            edge,
            point: a.lerp(b, 0.5).to_array(),
            direction: direction.to_array(),
            length_mm: edge_length(mesh, edge),
        })
    }

    /// A line body of the given edges of a body, for drawing over the scene.
    pub(crate) fn edge_outline_mesh(&self, body: Uuid, edges: &[u32]) -> Option<TriMesh> {
        let geometry = self.session.document.imported_geometry(BodyId(body))?;
        let mesh = &geometry.mesh;
        let mut out = TriMesh::default();
        for (segment, pair) in mesh.edges.chunks(2).enumerate() {
            let Some(id) = mesh.edge_ids.get(segment) else {
                break;
            };
            if !edges.contains(id) {
                continue;
            }
            let base = out.positions.len() as u32;
            out.positions.push(mesh.positions[pair[0] as usize]);
            out.positions.push(mesh.positions[pair[1] as usize]);
            out.normals.push([0.0, 0.0, 1.0]);
            out.normals.push([0.0, 0.0, 1.0]);
            out.edges.push(base);
            out.edges.push(base + 1);
        }
        (!out.positions.is_empty()).then_some(out)
    }

    /// The selected edges as the benches see them.
    pub(crate) fn selected_edge_refs(&self) -> Vec<EdgeRef> {
        self.session
            .selected_edges
            .iter()
            .map(EdgeHit::as_ref)
            .collect()
    }
}

/// The length of a kernel edge, as the sum of its outline segments.
fn edge_length(mesh: &TriMesh, edge: u32) -> f32 {
    mesh.edges
        .chunks(2)
        .zip(&mesh.edge_ids)
        .filter(|(_, id)| **id == edge)
        .map(|(pair, _)| {
            let a = Vec3::from_array(mesh.positions[pair[0] as usize]);
            let b = Vec3::from_array(mesh.positions[pair[1] as usize]);
            a.distance(b)
        })
        .sum()
}

/// A stable id and a content revision for an edge highlight line body.
pub(crate) fn highlight_revision(body: Uuid, edges: &[u32], revision: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    edges.hash(&mut h);
    revision.hash(&mut h);
    h.finish()
}

/// A highlight line body, or none when the edges are gone.
pub(crate) fn highlight_submission(
    app: &PrintCadApp,
    id: Uuid,
    body: Uuid,
    edges: &[u32],
    color: [f32; 3],
) -> Option<render_vk::BodySubmission> {
    let revision = app
        .session
        .document
        .imported_geometry(BodyId(body))
        .map(|g| g.revision)?;
    let mesh = app.edge_outline_mesh(body, edges)?;
    Some(render_vk::BodySubmission {
        id,
        revision: highlight_revision(body, edges, revision),
        mesh: Arc::new(mesh),
        color,
        opacity: 1.0,
        highlight: render_vk::HighlightState::None,
        is_wireframe: false,
        pickable: false,
    })
}
