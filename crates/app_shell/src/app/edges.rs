//! Edge picking: the outline segments of the body under the cursor are
//! tested on the CPU, in pixels, and the nearest within reach names its
//! kernel edge. The hovered and selected edges are drawn as line bodies
//! over the scene.

use std::sync::Arc;

use core_document::{BodyId, EdgeCircle, EdgeRef};
use glam::{Vec2, Vec3};
use kernel_api::TriMesh;
use uuid::Uuid;

use crate::PrintCadApp;

/// How close, in points, the cursor must come to an edge over a surface:
/// the face under the cursor keeps the hover elsewhere, so a narrow face
/// between two edges is not all edge.
const EDGE_REACH_ON_SURFACE_PT: f32 = 3.0;
/// How close over the background, where a silhouette edge has no face to
/// share the hover with.
const EDGE_REACH_PT: f32 = 6.0;
/// The reach the tests measure against, in pixels at unit scale.
#[cfg(test)]
const EDGE_PICK_PX: f32 = EDGE_REACH_PT;

/// An edge under the cursor or picked: the body, the kernel edge, the
/// point on it nearest the cursor, its direction there and its length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EdgeHit {
    pub body: Uuid,
    pub edge: u32,
    pub point: [f32; 3],
    pub direction: [f32; 3],
    pub length_mm: f32,
    pub circle: Option<EdgeCircle>,
    /// The names of the faces the edge runs between, when the mesh has
    /// them.
    pub faces: [kernel_api::TopoName; 2],
}

impl EdgeHit {
    pub fn as_ref(&self) -> EdgeRef {
        EdgeRef {
            faces: self.faces,
            point: self.point,
            direction: self.direction,
            length_mm: self.length_mm,
            body: self.body,
            circle: self.circle,
        }
    }
}

impl PrintCadApp {
    /// The edge nearest the cursor, within reach, over every visible body.
    /// The outline is tested in viewport pixels, so a silhouette edge is
    /// found from either side of it; an edge counts only where the pick pass
    /// drew it in front, at its own pixel.
    pub(crate) fn edge_under_cursor(&self) -> Option<EdgeHit> {
        let (cx, cy) = self.cursor_in_viewport?;
        let scale = self
            .gfx
            .as_ref()
            .map_or(1.0, |g| g.window.scale_factor() as f32);
        let reach = scale
            * if self.session.hovered_world_pos.is_some() {
                EDGE_REACH_ON_SURFACE_PT
            } else {
                EDGE_REACH_PT
            };
        let cursor = Vec2::new(cx, cy);
        let camera = &self.session.camera;
        let project = |p: [f32; 3]| camera.world_to_viewport(Vec3::from_array(p));
        let eye = Vec3::from_array(camera.position());
        let forward = (Vec3::from_array(camera.target()) - eye).normalize_or_zero();
        let depth_of = |p: Vec3| (p - eye).dot(forward);
        // The surface under the cursor hides every edge deeper than it by
        // more than the surface itself can fall away across the pick
        // radius; the allowance scales with what a pixel spans there.
        let hidden_beyond = self.session.hovered_world_pos.map(|p| {
            let p = Vec3::from_array(p);
            let mm_per_px = mm_per_px_at(project, p, forward);
            depth_of(p) + occlusion_slack(mm_per_px, reach)
        });
        // The depths the pick pass drew around the cursor, while they are
        // of the view on screen.
        let view_proj = camera.view_projection();
        let vp = camera.viewport_info();
        let window = self
            .session
            .pick_depths
            .as_ref()
            .filter(|w| w.view_proj == view_proj);
        let mm_per_px = self
            .session
            .hovered_world_pos
            .map(|p| mm_per_px_at(project, Vec3::from_array(p), forward))
            .unwrap_or(1.0);
        let shows = |at: Vec2, depth: f32| {
            let window = window?;
            let (x, y) = ((vp.0 + at.x).floor() as i64, (vp.1 + at.y).floor() as i64);
            let neighbours: Vec<Option<f32>> = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (x + dx, y + dy)))
                .filter(|(nx, ny)| window.covers(*nx, *ny))
                .map(|(nx, ny)| {
                    window
                        .world_at(nx, ny)
                        .map(|p| depth_of(Vec3::from_array(p)))
                })
                .collect();
            shows_among(&neighbours, depth, mm_per_px)
        };
        // The clipping plane hides an edge as the renderer does: a segment
        // with an end on its hidden side is not offered.
        let clip = self.frame_submission.clip_plane;
        let project_kept = |p: [f32; 3]| {
            clip.is_none_or(|plane| crate::camera::section::keeps(plane, Vec3::from_array(p)))
                .then(|| project(p))
                .flatten()
        };
        let document = &self.session.document;
        let mut best: Option<(SegmentHit, Uuid, &TriMesh)> = None;
        for (body_id, geometry) in document.imported_geometries() {
            if !document.imported_body_effective_visible(*body_id)
                || !document.body_selectable(*body_id)
            {
                continue;
            }
            let mesh = &*geometry.mesh;
            if mesh.edges.is_empty() || mesh.edge_ids.len() != mesh.edges.len() / 2 {
                continue;
            }
            // A body whose projected bounds miss the cursor has no edge
            // near it; this keeps an assembly's outlines off the CPU.
            if let Some((lo, hi)) = geometry.bounds_mm.or_else(|| mesh.bounds())
                && !bounds_reach(project, lo, hi, cursor, reach)
            {
                continue;
            }
            let Some(hit) = nearest_segment(
                mesh,
                project_kept,
                depth_of,
                cursor,
                reach,
                hidden_beyond,
                shows,
            ) else {
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
            circle: EdgeCircle::fit(&edge_points(mesh, edge)),
            faces: mesh
                .edge_faces
                .get(edge as usize)
                .copied()
                .unwrap_or_default(),
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

    /// Every picked face as the benches see them, in the order picked: the
    /// ones Ctrl added, then the last.
    pub(crate) fn selected_face_refs(&self) -> Vec<core_document::FaceRef> {
        let Some((body, last)) = self
            .session
            .last_face_hit
            .filter(|(body, _)| self.session.selected_body == Some(*body))
        else {
            return Vec::new();
        };
        self.session
            .earlier_faces
            .iter()
            .filter(|p| p.highlight.body == body)
            .map(|p| p.face)
            .chain(std::iter::once(last))
            .collect()
    }

    /// The view's picks in words when there are several ("3 faces, 2
    /// edges"); `None` for one pick or none.
    pub(crate) fn picks_summary(&self) -> Option<String> {
        let faces = self.selected_face_refs().len();
        let edges = self.session.selected_edges.len();
        if faces + edges < 2 {
            return None;
        }
        let count = |n: usize, one: &str, many: &str| match n {
            0 => None,
            1 => Some(format!("1 {one}")),
            n => Some(format!("{n} {many}")),
        };
        Some(
            [count(faces, "face", "faces"), count(edges, "edge", "edges")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", "),
        )
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

/// How far an edge may turn from the one before it and still continue
/// it: the outline's chords stand off a curve's tangent by up to half the
/// display's angular step.
const CHAIN_TURN_DEG: f32 = 20.0;

/// Where a kernel edge's outline ends, and the way it runs out of each
/// end. A closed edge (a full circle) has none.
fn edge_ends(mesh: &TriMesh, edge: u32) -> Vec<(Vec3, Vec3)> {
    let key = |p: Vec3| (p * 1000.0).round().as_ivec3();
    let segments: Vec<(Vec3, Vec3)> = mesh
        .edges
        .chunks(2)
        .zip(&mesh.edge_ids)
        .filter(|(_, id)| **id == edge)
        .map(|(pair, _)| {
            (
                Vec3::from_array(mesh.positions[pair[0] as usize]),
                Vec3::from_array(mesh.positions[pair[1] as usize]),
            )
        })
        .collect();
    let mut seen: std::collections::HashMap<glam::IVec3, usize> = Default::default();
    for (a, b) in &segments {
        *seen.entry(key(*a)).or_default() += 1;
        *seen.entry(key(*b)).or_default() += 1;
    }
    let mut ends = Vec::new();
    for (a, b) in &segments {
        if seen[&key(*a)] == 1 {
            ends.push((*a, (*a - *b).normalize_or_zero()));
        }
        if seen[&key(*b)] == 1 {
            ends.push((*b, (*b - *a).normalize_or_zero()));
        }
    }
    ends
}

/// The edges of `mesh` that continue `start` smoothly, `start` first: at
/// each end, an edge meeting it there and running on within
/// [`CHAIN_TURN_DEG`] of its direction, and so on from that one.
pub(crate) fn tangent_chain(mesh: &TriMesh, start: u32) -> Vec<u32> {
    let key = |p: Vec3| (p * 1000.0).round().as_ivec3();
    let mut ids: Vec<u32> = mesh.edge_ids.clone();
    ids.sort_unstable();
    ids.dedup();
    let ends: std::collections::HashMap<u32, Vec<(Vec3, Vec3)>> =
        ids.iter().map(|id| (*id, edge_ends(mesh, *id))).collect();
    let mut at: std::collections::HashMap<glam::IVec3, Vec<(u32, Vec3)>> = Default::default();
    for (id, list) in &ends {
        for (p, out) in list {
            at.entry(key(*p)).or_default().push((*id, *out));
        }
    }
    let smooth = CHAIN_TURN_DEG.to_radians().cos();
    let mut chain = vec![start];
    let mut next = vec![start];
    while let Some(edge) = next.pop() {
        for (p, out) in ends.get(&edge).into_iter().flatten() {
            for (other, other_out) in at.get(&key(*p)).into_iter().flatten() {
                // Running on means leaving the point the way this one
                // arrived: against the other's own way out.
                if !chain.contains(other) && out.dot(-*other_out) >= smooth {
                    chain.push(*other);
                    next.push(*other);
                }
            }
        }
    }
    chain
}

/// The pick of edge `edge` of a body, as a click at its middle makes it.
pub(crate) fn edge_hit(mesh: &TriMesh, body: Uuid, edge: u32) -> Option<EdgeHit> {
    let segments: Vec<&[u32]> = mesh
        .edges
        .chunks(2)
        .zip(&mesh.edge_ids)
        .filter(|(_, id)| **id == edge)
        .map(|(pair, _)| pair)
        .collect();
    let pair = segments.get(segments.len() / 2)?;
    let a = Vec3::from_array(mesh.positions[pair[0] as usize]);
    let b = Vec3::from_array(mesh.positions[pair[1] as usize]);
    Some(EdgeHit {
        body,
        edge,
        point: a.lerp(b, 0.5).to_array(),
        direction: (b - a).normalize_or_zero().to_array(),
        length_mm: edge_length(mesh, edge),
        circle: EdgeCircle::fit(&edge_points(mesh, edge)),
        faces: mesh
            .edge_faces
            .get(edge as usize)
            .copied()
            .unwrap_or_default(),
    })
}

fn edge_points(mesh: &TriMesh, edge: u32) -> Vec<[f32; 3]> {
    mesh.edges
        .chunks(2)
        .zip(&mesh.edge_ids)
        .filter(|(_, id)| **id == edge)
        .flat_map(|(pair, _)| pair.iter().map(|&i| mesh.positions[i as usize]))
        .collect()
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

/// `doc.edges`: each kernel edge of `mesh`, in the mesh's frame: its index,
/// whether it is a line, a circle (or an arc of one) or another curve, a
/// point halfway along it and its direction there (what an edge pick
/// takes), its length, the indices of the faces it runs between as
/// `doc.faces` numbers them, their names as strings, and a circle's
/// centre, normal and radius.
pub(crate) fn edges_of(mesh: &TriMesh) -> serde_json::Value {
    use serde_json::json;
    let key = |i: u32| {
        let p = Vec3::from_array(mesh.positions[i as usize]);
        (p * 1000.0).round().as_ivec3().to_array()
    };
    let side = |a: u32, b: u32| {
        let (a, b) = (key(a), key(b));
        if a <= b { (a, b) } else { (b, a) }
    };
    // The faces each triangle side belongs to, to find an edge's faces
    // where the mesh names none.
    let mut sides: std::collections::HashMap<_, Vec<u32>> = Default::default();
    for (t, face) in mesh.faces.iter().enumerate() {
        let corner = |i: usize| mesh.indices[3 * t + i];
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let list = sides.entry(side(corner(a), corner(b))).or_default();
            if !list.contains(face) {
                list.push(*face);
            }
        }
    }
    let named = |name: kernel_api::TopoName| {
        let mut found = mesh
            .face_names
            .iter()
            .enumerate()
            .filter(|(_, n)| name != 0 && **n == name);
        match (found.next(), found.next()) {
            (Some((f, _)), None) => Some(f as u32),
            _ => None,
        }
    };
    let mut ids = mesh.edge_ids.clone();
    ids.sort_unstable();
    ids.dedup();
    let list = ids
        .into_iter()
        .filter_map(|edge| {
            let hit = edge_hit(mesh, Uuid::nil(), edge)?;
            let points = edge_points(mesh, edge);
            let names = mesh
                .edge_faces
                .get(edge as usize)
                .copied()
                .unwrap_or_default();
            let faces: Vec<u32> = match (named(names[0]), named(names[1])) {
                (Some(a), Some(b)) => vec![a, b],
                _ => {
                    let mut faces = Vec::new();
                    for (pair, _) in mesh
                        .edges
                        .chunks(2)
                        .zip(&mesh.edge_ids)
                        .filter(|(_, id)| **id == edge)
                    {
                        for f in sides.get(&side(pair[0], pair[1])).into_iter().flatten() {
                            if !faces.contains(f) {
                                faces.push(*f);
                            }
                        }
                    }
                    faces.truncate(2);
                    faces
                }
            };
            let a = Vec3::from_array(points[0]);
            let far = points
                .iter()
                .map(|p| Vec3::from_array(*p))
                .max_by(|p, q| p.distance_squared(a).total_cmp(&q.distance_squared(a)))
                .unwrap_or(a);
            let chord = (far - a).normalize_or_zero();
            let straight = chord != Vec3::ZERO
                && points.iter().all(|p| {
                    (Vec3::from_array(*p) - a)
                        .reject_from_normalized(chord)
                        .length()
                        <= 1e-3 * a.distance(far)
                });
            let circle = if straight { None } else { hit.circle };
            let mut point = Vec3::from_array(hit.point);
            if let Some(c) = circle {
                // The chord's middle lies inside the curve: put it on it.
                let centre = Vec3::from_array(c.center);
                let normal = Vec3::from_array(c.normal).normalize_or_zero();
                let out = (point - centre).reject_from(normal).normalize_or_zero();
                if out != Vec3::ZERO {
                    point = centre + out * c.radius;
                }
            }
            let kind = match (straight, circle) {
                (true, _) => "line",
                (false, Some(_)) => "circle",
                (false, None) => "other",
            };
            let mut out = json!({
                "index": edge,
                "kind": kind,
                "point": point.to_array(),
                "direction": hit.direction,
                "length": hit.length_mm,
                "faces": faces,
            });
            if names.iter().any(|n| *n != 0) {
                // Strings, since a script's numbers cannot hold every name.
                out["names"] = json!(names.map(|n| n.to_string()));
            }
            if let Some(c) = circle {
                out["centre"] = json!(c.center);
                out["normal"] = json!(c.normal);
                out["radius"] = json!(c.radius);
            }
            Some(out)
        })
        .collect();
    serde_json::Value::Array(list)
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
) -> Option<render_wgpu::BodySubmission> {
    let revision = app
        .session
        .document
        .imported_geometry(BodyId(body))
        .map(|g| g.revision)?;
    let mesh = app.edge_outline_mesh(body, edges)?;
    Some(render_wgpu::BodySubmission {
        id,
        revision: highlight_revision(body, edges, revision),
        mesh: Arc::new(mesh),
        color,
        opacity: 1.0,
        highlight: render_wgpu::HighlightState::None,
        is_wireframe: false,
        pickable: false,
        on_top: false,
        edge_color: None,
        front_only: false,
        instance: None,
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
/// visible, given what one pixel spans there. An edge bounding the face
/// under the cursor is at most the reach away on screen, and the face
/// falls away across that distance by at most its slope: twice the reach,
/// for a face tilted up to about 63° from the screen. A back edge, a wall's
/// thickness deeper, is past it.
fn occlusion_slack(mm_per_px: f32, reach_px: f32) -> f32 {
    const FLOOR_MM: f32 = 0.05;
    FLOOR_MM + 2.0 * reach_px * mm_per_px
}

/// Whether an edge point at `edge_depth` shows at its pixel, given the view
/// depth drawn at each known pixel of its 3 × 3 neighbourhood (`None` where
/// nothing was drawn); `None` when no pixel of it is known.
///
/// Background beside the edge puts it on a silhouette, in view. Otherwise
/// it shows unless the farthest surface around it is nearer than it by more
/// than a surface falls away across a pixel and a half at a steep slope:
/// the faces an edge bounds meet it there, whatever hides it covers the
/// whole neighbourhood.
fn shows_among(neighbours: &[Option<f32>], edge_depth: f32, mm_per_px: f32) -> Option<bool> {
    if neighbours.is_empty() {
        return None;
    }
    if neighbours.iter().any(Option::is_none) {
        return Some(true);
    }
    let farthest = neighbours
        .iter()
        .flatten()
        .fold(f32::NEG_INFINITY, |a, b| a.max(*b));
    Some(edge_depth <= farthest + 0.05 + 3.0 * mm_per_px)
}

/// Millimetres one viewport pixel spans at `point`, across the view.
fn mm_per_px_at(
    project: impl Fn([f32; 3]) -> Option<(f32, f32)>,
    point: Vec3,
    forward: Vec3,
) -> f32 {
    let across = forward.any_orthonormal_vector();
    match (
        project(point.to_array()),
        project((point + across).to_array()),
    ) {
        (Some(a), Some(b)) => {
            let px = Vec2::new(a.0, a.1).distance(Vec2::new(b.0, b.1));
            if px > 1e-6 { 1.0 / px } else { 1.0 }
        }
        _ => 1.0,
    }
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
/// those hidden: by what the pick pass drew at the edge's own pixel, as
/// `shows` answers, or where it cannot, by lying deeper than
/// `hidden_beyond`, the surface under the cursor.
fn nearest_segment(
    mesh: &TriMesh,
    project: impl Fn([f32; 3]) -> Option<(f32, f32)>,
    depth_of: impl Fn(Vec3) -> f32,
    cursor: Vec2,
    reach_px: f32,
    hidden_beyond: Option<f32>,
    shows: impl Fn(Vec2, f32) -> Option<bool>,
) -> Option<SegmentHit> {
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
        if distance_px > reach_px {
            continue;
        }
        let depth = depth_of(Vec3::from_array(a).lerp(Vec3::from_array(b), t));
        // What the pick pass drew at the edge's own pixel decides; where
        // that is unknown, the surface under the cursor does.
        let hidden = match shows(pa + ab * t, depth) {
            Some(shown) => !shown,
            None => hidden_beyond.is_some_and(|limit| depth > limit),
        };
        if hidden {
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

    /// An outline of kernel edges, each a run of points.
    fn outline(edges: &[&[[f32; 3]]]) -> TriMesh {
        let mut mesh = TriMesh::default();
        for (id, points) in edges.iter().enumerate() {
            let base = mesh.positions.len() as u32;
            mesh.positions.extend_from_slice(points);
            for i in 1..points.len() as u32 {
                mesh.edges.extend_from_slice(&[base + i - 1, base + i]);
                mesh.edge_ids.push(id as u32);
            }
        }
        mesh
    }

    /// A slot's outline: two lines joined by a half circle run on into one
    /// another; the square corner at the far end stops the chain.
    #[test]
    fn a_tangent_chain_runs_on_until_a_corner() {
        let arc: Vec<[f32; 3]> = (0..=8)
            .map(|i| {
                let t = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 8.0;
                [10.0 + 5.0 * t.cos(), 5.0 + 5.0 * t.sin(), 0.0]
            })
            .collect();
        let mesh = outline(&[
            &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            &arc,
            &[[10.0, 10.0, 0.0], [0.0, 10.0, 0.0]],
            // Square to the bottom line at its start.
            &[[0.0, 10.0, 0.0], [0.0, 0.0, 0.0]],
        ]);
        let mut chain = super::tangent_chain(&mesh, 0);
        chain.sort();
        assert_eq!(chain, [0, 1, 2]);
        assert_eq!(super::tangent_chain(&mesh, 3), [3], "a corner at both ends");
        let hit = super::edge_hit(&mesh, Uuid::nil(), 1).expect("the arc");
        assert!((hit.length_mm - 5.0 * std::f32::consts::PI).abs() < 0.2);
    }

    /// On a real solid, a padded rounded rectangle, the top outline's four
    /// lines and four arcs are one chain, and nothing that runs down the
    /// sides joins it.
    #[test]
    fn a_rounded_outline_s_edges_are_one_chain() {
        use kernel_api::{
            BooleanOp, ExtrudeTermination, Profile, ProfilePlane, ProfileSegment, ProfileWire,
            SolidOp, SweepKind, TessellationSettings,
        };
        let r = 2.0;
        let (w, h) = (20.0, 10.0);
        let d = r * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
        let arc =
            |start: [f64; 2], mid: [f64; 2], end: [f64; 2]| ProfileSegment::Arc { start, mid, end };
        let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
        let wire = ProfileWire {
            names: Vec::new(),
            segments: vec![
                line([r, 0.0], [w - r, 0.0]),
                arc([w - r, 0.0], [w - d, d], [w, r]),
                line([w, r], [w, h - r]),
                arc([w, h - r], [w - d, h - d], [w - r, h]),
                line([w - r, h], [r, h]),
                arc([r, h], [d, h - d], [0.0, h - r]),
                line([0.0, h - r], [0.0, r]),
                arc([0.0, r], [d, d], [r, 0.0]),
            ],
        };
        let pad = SolidOp::Sweep {
            profile: Profile {
                plane: ProfilePlane {
                    origin: [0.0; 3],
                    x_axis: [1.0, 0.0, 0.0],
                    y_axis: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                wires: vec![wire],
            },
            kind: SweepKind::Extrude {
                termination: ExtrudeTermination::Blind { distance: 5.0 },
                second_side: None,
                symmetric: false,
                reversed: false,
                taper_deg: 0.0,
                direction: None,
            },
            op: BooleanOp::NewSolid,
        };
        let built = kernel_ogeom::OgeomKernel::new()
            .execute_solid_chain(&[pad], &TessellationSettings::default())
            .expect("the rounded box pads");
        let mesh = &built.mesh;
        // A top edge: every point of its outline at the top.
        let top = |edge: u32| {
            edge_points(mesh, edge)
                .iter()
                .all(|p| (p[2] - 5.0).abs() < 1e-3)
        };
        let start = *mesh.edge_ids.iter().find(|e| top(**e)).expect("a top edge");
        let chain = super::tangent_chain(mesh, start);
        assert_eq!(chain.len(), 8, "{chain:?}");
        assert!(chain.iter().all(|e| top(*e)), "only the top loop");
    }

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

    /// With no face names, an edge's faces are the ones whose triangles
    /// share its segments; an arc is a circle with its point on the curve.
    #[test]
    fn edges_are_listed_with_their_faces_and_kind() {
        let mut mesh = cube();
        // The bottom (z = 0) and front (y = 0) faces.
        mesh.indices = vec![0, 1, 3, 0, 3, 2, 0, 1, 5, 0, 5, 4];
        mesh.faces = vec![0, 0, 1, 1];
        let arc: Vec<[f32; 3]> = (0..=8)
            .map(|i| {
                let a = std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
                [30.0 + 5.0 * a.cos(), 5.0 * a.sin(), 0.0]
            })
            .collect();
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(&arc);
        for i in 1..arc.len() as u32 {
            mesh.edges.extend([base + i - 1, base + i]);
            mesh.edge_ids.push(12);
        }
        let listed = edges_of(&mesh);
        let listed = listed.as_array().unwrap();
        assert_eq!(listed.len(), 13);
        assert_eq!(listed[0]["kind"], "line");
        assert_eq!(listed[0]["faces"], serde_json::json!([0, 1]));
        assert_eq!(listed[0]["point"], serde_json::json!([5.0, 0.0, 0.0]));
        assert!((listed[0]["length"].as_f64().unwrap() - 10.0).abs() < 1e-4);
        let arc = &listed[12];
        assert_eq!(arc["kind"], "circle");
        assert!((arc["radius"].as_f64().unwrap() - 5.0).abs() < 1e-3);
        let p: Vec<f64> = serde_json::from_value(arc["point"].clone()).unwrap();
        assert!(((p[0] - 30.0).hypot(p[1]) - 5.0).abs() < 1e-3, "on the arc");
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
        let hit = nearest_segment(
            &mesh,
            project,
            depth_of,
            Vec2::new(-4.0, 50.0),
            EDGE_PICK_PX,
            None,
            |_, _| None,
        )
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
        // Ten pixels a millimetre: the allowance is 1.25 mm.
        let limit = |surface: f32| Some(surface + occlusion_slack(0.1, EDGE_PICK_PX));
        let hit = nearest_segment(
            &mesh,
            project,
            depth_of,
            Vec2::new(3.0, 50.0),
            EDGE_PICK_PX,
            limit(10.0),
            |_, _| None,
        )
        .expect("edge within reach");
        assert_eq!(mesh.edge_ids[hit.segment], 6);
        // Seen from below, the top face's edges are the hidden ones.
        let from_below = |p: Vec3| p.z;
        let hit = nearest_segment(
            &mesh,
            project,
            from_below,
            Vec2::new(3.0, 50.0),
            EDGE_PICK_PX,
            limit(0.0),
            |_, _| None,
        )
        .expect("edge within reach");
        assert_eq!(mesh.edge_ids[hit.segment], 4);
    }

    /// A 3 mm plate seen face-on from 200 mm, a pixel spanning 0.15 mm:
    /// the edge on its back, a few pixels from the cursor over the middle
    /// of the front face, stays hidden behind it.
    #[test]
    fn a_back_edge_near_the_cursor_stays_behind_a_thin_plate() {
        // One back edge, 3 mm behind the front face at depth 200.
        let mut mesh = TriMesh {
            positions: vec![[0.0, 0.0, 203.0], [10.0, 0.0, 203.0]],
            edges: vec![0, 1],
            edge_ids: vec![0],
            ..TriMesh::default()
        };
        let project = |p: [f32; 3]| Some((p[0] / 0.15, p[1] / 0.15));
        let depth_of = |p: Vec3| p.z;
        // The cursor sits 3 px from the back edge's image.
        let cursor = Vec2::new(30.0, 3.0);
        let limit = Some(200.0 + occlusion_slack(0.15, EDGE_PICK_PX));
        assert!(
            nearest_segment(
                &mesh,
                project,
                depth_of,
                cursor,
                EDGE_PICK_PX,
                limit,
                |_, _| None
            )
            .is_none(),
            "the back edge takes the hover from the plate's face"
        );
        // Over the plate's own rim, where the edge is at the surface's
        // depth, it is found.
        mesh.positions = vec![[0.0, 0.0, 200.4], [10.0, 0.0, 200.4]];
        assert!(
            nearest_segment(
                &mesh,
                project,
                depth_of,
                cursor,
                EDGE_PICK_PX,
                limit,
                |_, _| None
            )
            .is_some()
        );
    }

    #[test]
    fn nothing_out_of_reach_is_picked() {
        let mesh = cube();
        assert!(
            nearest_segment(
                &mesh,
                project,
                depth_of,
                Vec2::new(50.0, 50.0),
                EDGE_PICK_PX,
                None,
                |_, _| None
            )
            .is_none()
        );
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
