//! Edge picking: the outline segments of the body under the cursor are
//! tested on the CPU, in pixels, and the nearest within reach names its
//! kernel edge. The hovered and selected edges are drawn as line bodies
//! over the scene.

use std::sync::Arc;

use core_document::{BodyId, EdgeRef};
use glam::{Vec2, Vec3};
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
    /// The edge nearest the cursor, within reach, over every visible body.
    /// The outline is tested in viewport pixels, so a silhouette edge is
    /// found from either side of it; the surface the GPU pick found under
    /// the cursor, when there is one, hides the edges behind it.
    pub(crate) fn edge_under_cursor(&self) -> Option<EdgeHit> {
        let (cx, cy) = self.cursor_in_viewport?;
        let cursor = Vec2::new(cx, cy);
        let camera = &self.session.camera;
        let project = |p: [f32; 3]| camera.world_to_viewport(Vec3::from_array(p));
        let eye = Vec3::from_array(camera.position());
        let forward = (Vec3::from_array(camera.target()) - eye).normalize_or_zero();
        let depth_of = |p: Vec3| (p - eye).dot(forward);
        let surface_depth = self
            .session
            .hovered_world_pos
            .map(|p| depth_of(Vec3::from_array(p)));
        let document = &self.session.document;
        let mut best: Option<(SegmentHit, Uuid, &TriMesh)> = None;
        for (body_id, geometry) in document.imported_geometries() {
            if !document.imported_body_effective_visible(*body_id) {
                continue;
            }
            let mesh = &*geometry.mesh;
            if mesh.edges.is_empty() || mesh.edge_ids.len() != mesh.edges.len() / 2 {
                continue;
            }
            // A body whose projected bounds miss the cursor has no edge
            // near it; this keeps an assembly's outlines off the CPU.
            if let Some((lo, hi)) = geometry.bounds_mm.or_else(|| mesh.bounds())
                && !bounds_reach(project, lo, hi, cursor, EDGE_PICK_PX)
            {
                continue;
            }
            let Some(hit) = nearest_segment(mesh, project, depth_of, cursor, surface_depth) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(current, _, _)| hit.beats(current))
            {
                best = Some((hit, body_id.0, mesh));
            }
        }
        let (hit, body, mesh) = best?;
        let edge = mesh.edge_ids[hit.segment];
        let pair = &mesh.edges[hit.segment * 2..hit.segment * 2 + 2];
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

/// An outline segment within reach of the cursor: how far, in pixels, and
/// how deep into the scene its nearest point lies.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SegmentHit {
    segment: usize,
    distance_px: f32,
    depth: f32,
}

impl SegmentHit {
    /// Two segments about equally far from the cursor are told apart by
    /// depth, so the front edge wins where a back edge crosses it.
    fn beats(&self, other: &SegmentHit) -> bool {
        const SAME_PX: f32 = 1.5;
        if (self.distance_px - other.distance_px).abs() < SAME_PX {
            self.depth < other.depth
        } else {
            self.distance_px < other.distance_px
        }
    }
}

/// How far behind the picked surface an edge may lie and still count as
/// visible: an edge bounding an oblique face is a little deeper than the
/// surface a few pixels away from it, a back edge is a body's thickness
/// deeper.
fn occlusion_slack(surface_depth: f32) -> f32 {
    1.0 + 0.05 * surface_depth.abs()
}

/// Whether the projection of a box comes within `reach` pixels of the
/// cursor. A box with a corner behind the camera is never ruled out.
fn bounds_reach(
    project: impl Fn([f32; 3]) -> Option<(f32, f32)>,
    lo: [f32; 3],
    hi: [f32; 3],
    cursor: Vec2,
    reach: f32,
) -> bool {
    let mut min = Vec2::INFINITY;
    let mut max = Vec2::NEG_INFINITY;
    for corner in 0..8 {
        let p = [
            if corner & 1 == 0 { lo[0] } else { hi[0] },
            if corner & 2 == 0 { lo[1] } else { hi[1] },
            if corner & 4 == 0 { lo[2] } else { hi[2] },
        ];
        let Some((x, y)) = project(p) else {
            return true;
        };
        min = min.min(Vec2::new(x, y));
        max = max.max(Vec2::new(x, y));
    }
    cursor.x >= min.x - reach
        && cursor.x <= max.x + reach
        && cursor.y >= min.y - reach
        && cursor.y <= max.y + reach
}

/// The outline segment of `mesh` nearest the cursor within reach, skipping
/// those hidden behind the surface at `surface_depth`.
fn nearest_segment(
    mesh: &TriMesh,
    project: impl Fn([f32; 3]) -> Option<(f32, f32)>,
    depth_of: impl Fn(Vec3) -> f32,
    cursor: Vec2,
    surface_depth: Option<f32>,
) -> Option<SegmentHit> {
    let hidden_beyond = surface_depth.map(|s| s + occlusion_slack(s));
    let mut best: Option<SegmentHit> = None;
    for (segment, pair) in mesh.edges.chunks(2).enumerate() {
        let a = mesh.positions[pair[0] as usize];
        let b = mesh.positions[pair[1] as usize];
        let (Some(pa), Some(pb)) = (project(a), project(b)) else {
            continue;
        };
        let (pa, pb) = (Vec2::new(pa.0, pa.1), Vec2::new(pb.0, pb.1));
        let ab = pb - pa;
        let t = if ab.length_squared() < 1e-6 {
            0.0
        } else {
            ((cursor - pa).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        };
        let distance_px = (pa + ab * t - cursor).length();
        if distance_px > EDGE_PICK_PX {
            continue;
        }
        let depth = depth_of(Vec3::from_array(a).lerp(Vec3::from_array(b), t));
        if hidden_beyond.is_some_and(|limit| depth > limit) {
            continue;
        }
        let hit = SegmentHit {
            segment,
            distance_px,
            depth,
        };
        if best.is_none_or(|current| hit.beats(&current)) {
            best = Some(hit);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube outline seen straight down -Z with 10 px per mm: x and y
    /// map to the viewport, z is depth (nearer at larger z).
    fn cube() -> TriMesh {
        let mut mesh = TriMesh::default();
        for corner in 0..8u32 {
            let p = |bit: u32| if corner & bit == 0 { 0.0 } else { 10.0 };
            mesh.positions.push([p(1), p(2), p(4)]);
        }
        let edges: [(u32, u32); 12] = [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];
        for (i, (a, b)) in edges.iter().enumerate() {
            mesh.edges.extend([*a, *b]);
            mesh.edge_ids.push(i as u32);
        }
        mesh
    }

    fn project(p: [f32; 3]) -> Option<(f32, f32)> {
        Some((p[0] * 10.0, p[1] * 10.0))
    }

    fn depth_of(p: Vec3) -> f32 {
        20.0 - p.z
    }

    #[test]
    fn a_silhouette_edge_is_found_from_outside_the_body() {
        let mesh = cube();
        // 4 px left of the x = 0 edge, over background: no surface.
        let hit = nearest_segment(&mesh, project, depth_of, Vec2::new(-4.0, 50.0), None)
            .expect("edge within reach");
        // The top (z = 10) and bottom (z = 0) x = 0 edges coincide on
        // screen; the nearer one wins.
        assert_eq!(mesh.edge_ids[hit.segment], 6);
        assert!((hit.depth - 10.0).abs() < 1e-4);
    }

    #[test]
    fn an_edge_behind_the_picked_surface_is_skipped() {
        let mesh = cube();
        // The cursor is over the top face (depth 10) near the x = 0 edge;
        // the bottom face's edge lies 10 mm deeper and is hidden.
        let hit = nearest_segment(&mesh, project, depth_of, Vec2::new(3.0, 50.0), Some(10.0))
            .expect("edge within reach");
        assert_eq!(mesh.edge_ids[hit.segment], 6);
        // Seen from below, the top face's edges are the hidden ones.
        let from_below = |p: Vec3| p.z;
        let hit = nearest_segment(&mesh, project, from_below, Vec2::new(3.0, 50.0), Some(0.0))
            .expect("edge within reach");
        assert_eq!(mesh.edge_ids[hit.segment], 4);
    }

    #[test]
    fn nothing_out_of_reach_is_picked() {
        let mesh = cube();
        assert!(nearest_segment(&mesh, project, depth_of, Vec2::new(50.0, 50.0), None).is_none());
        assert!(!bounds_reach(
            project,
            [0.0; 3],
            [10.0; 3],
            Vec2::new(120.0, 50.0),
            EDGE_PICK_PX
        ));
        assert!(bounds_reach(
            project,
            [0.0; 3],
            [10.0; 3],
            Vec2::new(104.0, 50.0),
            EDGE_PICK_PX
        ));
    }
}
