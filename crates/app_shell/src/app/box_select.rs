//! Box selection in the view: a left drag that starts on empty space
//! draws a box, and its release picks what the box holds. Dragged left to
//! right it is a window, taking only what lies wholly inside; right to
//! left a crossing, taking whatever it touches. It works on the CPU from
//! the meshes the scene draws: a body's projected bounds first, then its
//! triangles or outline segments, grouped by the kernel face or edge each
//! was cut from (`TriMesh::faces`, `TriMesh::edge_ids`).

use glam::{Vec2, Vec3};
use kernel_api::TriMesh;
use uuid::Uuid;

use crate::PrintCadApp;
use crate::app::input::PickedFace;
use crate::log_panel as app_log;
use crate::ui::PickFilter;

/// Dash pattern of a crossing box, in pixels.
const CROSSING_DASH: (f32, f32) = (6.0, 4.0);

/// A box being dragged, its corners in viewport-local physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoxDrag {
    /// Where the button went down.
    pub from: Vec2,
    /// Where the pointer is now.
    pub to: Vec2,
}

/// How a box takes what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Catch {
    /// Only what lies wholly inside.
    Window,
    /// Whatever the box touches.
    Crossing,
}

impl BoxDrag {
    pub fn at(point: Vec2) -> Self {
        Self {
            from: point,
            to: point,
        }
    }

    /// A box dragged leftwards is a crossing.
    pub fn catch(&self) -> Catch {
        if self.to.x < self.from.x {
            Catch::Crossing
        } else {
            Catch::Window
        }
    }

    /// Whether the pointer went far enough for a box rather than a click.
    pub fn dragged(&self, threshold_px: f32) -> bool {
        (self.to - self.from).abs().max_element() >= threshold_px
    }

    pub fn rect(&self) -> Rect {
        Rect {
            min: self.from.min(self.to),
            max: self.from.max(self.to),
        }
    }

    /// The box's four sides as screen-space lines: solid for a window,
    /// dashed for a crossing.
    pub fn outline(&self, color: [f32; 3]) -> Vec<core_document::ScreenSpaceOverlay> {
        let Rect { min, max } = self.rect();
        let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        let dash = (self.catch() == Catch::Crossing).then_some(CROSSING_DASH);
        (0..4)
            .map(|i| {
                let (a, b) = (corners[i], corners[(i + 1) % 4]);
                let mut line =
                    core_document::ScreenSpaceOverlay::new(a.to_array(), b.to_array(), color, 1.0);
                line.dash = dash;
                line
            })
            .collect()
    }
}

/// An axis-aligned rectangle in viewport pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    fn overlaps(&self, other: &Rect) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }

    fn holds(&self, other: &Rect) -> bool {
        self.contains(other.min) && self.contains(other.max)
    }

    /// Whether the segment `a`–`b` passes through the rectangle: the part
    /// of its parameter range left once each side has cut it is not empty.
    fn meets_segment(&self, a: Vec2, b: Vec2) -> bool {
        let d = b - a;
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (p, q) in [
            (-d.x, a.x - self.min.x),
            (d.x, self.max.x - a.x),
            (-d.y, a.y - self.min.y),
            (d.y, self.max.y - a.y),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }

    /// Whether the triangle `abc` and the rectangle share any point.
    fn meets_triangle(&self, [a, b, c]: [Vec2; 3]) -> bool {
        self.meets_segment(a, b)
            || self.meets_segment(b, c)
            || self.meets_segment(c, a)
            || in_triangle((self.min + self.max) / 2.0, [a, b, c])
    }
}

fn in_triangle(p: Vec2, [a, b, c]: [Vec2; 3]) -> bool {
    let side = |u: Vec2, v: Vec2| (v - u).perp_dot(p - u);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// One triangle or segment as the view shows it.
enum Shown<const N: usize> {
    /// On screen, its corners in viewport pixels.
    At([Vec2; N]),
    /// A corner lies on the side the clipping plane cuts away: the scene
    /// does not draw it, so the box neither takes nor misses it.
    Clipped,
    /// A corner lies behind the eye, so it is not wholly in any box.
    Behind,
}

/// What a box is tested against: its rectangle, how it takes, how the
/// view projects a point to viewport pixels (`None` behind the eye) and
/// the clipping plane the scene is cut at.
pub(crate) struct Probe<P> {
    pub rect: Rect,
    pub catch: Catch,
    pub project: P,
    pub clip: Option<[f32; 4]>,
}

/// What projected bounds say before the triangles are read.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Nothing of the body can be in the box.
    Missed,
    /// Every point of the body is in the box.
    Inside,
    Unsure,
}

/// Per kernel face or edge: whether any of its pieces were shown, all of
/// them inside, and any touching.
#[derive(Clone, Copy)]
struct Tally {
    shown: bool,
    all_inside: bool,
    touched: bool,
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            shown: false,
            all_inside: true,
            touched: false,
        }
    }
}

impl Tally {
    fn caught(&self, catch: Catch) -> bool {
        match catch {
            Catch::Window => self.shown && self.all_inside,
            Catch::Crossing => self.touched,
        }
    }
}

impl<P: Fn(Vec3) -> Option<Vec2>> Probe<P> {
    fn show<const N: usize>(&self, points: [Vec3; N]) -> Shown<N> {
        if let Some(plane) = self.clip
            && points
                .iter()
                .any(|p| !crate::camera::section::keeps(plane, *p))
        {
            return Shown::Clipped;
        }
        let mut out = [Vec2::ZERO; N];
        for (slot, p) in out.iter_mut().zip(points) {
            match (self.project)(p) {
                Some(at) => *slot = at,
                None => return Shown::Behind,
            }
        }
        Shown::At(out)
    }

    /// Fold one piece of a face, edge or body into its tally.
    fn count<const N: usize>(
        &self,
        tally: &mut Tally,
        piece: Shown<N>,
        meets: impl Fn(&Rect, [Vec2; N]) -> bool,
    ) {
        match piece {
            Shown::Clipped => {}
            Shown::Behind => tally.all_inside = false,
            Shown::At(points) => {
                tally.shown = true;
                let inside = points.iter().all(|p| self.rect.contains(*p));
                tally.all_inside &= inside;
                tally.touched |= inside || meets(&self.rect, points);
            }
        }
    }

    fn verdict(&self, bounds: Option<([f32; 3], [f32; 3])>) -> Verdict {
        let Some((lo, hi)) = bounds else {
            return Verdict::Unsure;
        };
        let corners = (0..8).map(|corner| {
            Vec3::new(
                if corner & 1 == 0 { lo[0] } else { hi[0] },
                if corner & 2 == 0 { lo[1] } else { hi[1] },
                if corner & 4 == 0 { lo[2] } else { hi[2] },
            )
        });
        if let Some(plane) = self.clip
            && corners
                .clone()
                .all(|p| !crate::camera::section::keeps(plane, p))
        {
            return Verdict::Missed;
        }
        let mut min = Vec2::INFINITY;
        let mut max = Vec2::NEG_INFINITY;
        for p in corners {
            let Some(at) = (self.project)(p) else {
                return Verdict::Unsure;
            };
            min = min.min(at);
            max = max.max(at);
        }
        let screen = Rect { min, max };
        if !self.rect.overlaps(&screen) {
            Verdict::Missed
        } else if self.rect.holds(&screen) && self.clip.is_none() {
            Verdict::Inside
        } else {
            Verdict::Unsure
        }
    }

    fn triangle(&self, mesh: &TriMesh, tri: &[u32; 3]) -> Option<Shown<3>> {
        let p = |i: u32| mesh.positions.get(i as usize).map(|p| Vec3::from_array(*p));
        Some(self.show([p(tri[0])?, p(tri[1])?, p(tri[2])?]))
    }

    /// Whether the box takes the whole of `mesh` (placed in the world),
    /// `bounds` its bounding box when known.
    pub fn takes_body(&self, mesh: &TriMesh, bounds: Option<([f32; 3], [f32; 3])>) -> bool {
        match self.verdict(bounds.or_else(|| mesh.bounds())) {
            Verdict::Missed => return false,
            Verdict::Inside => return !mesh.indices.is_empty(),
            Verdict::Unsure => {}
        }
        let mut tally = Tally::default();
        for tri in mesh.indices.as_chunks::<3>().0 {
            let Some(piece) = self.triangle(mesh, tri) else {
                continue;
            };
            self.count(&mut tally, piece, Rect::meets_triangle);
            match self.catch {
                Catch::Crossing if tally.touched => return true,
                Catch::Window if !tally.all_inside => return false,
                _ => {}
            }
        }
        tally.caught(self.catch)
    }

    /// The kernel faces of `mesh` the box takes, in order of their ids.
    /// A mesh that does not record its faces has none to take.
    pub fn faces(&self, mesh: &TriMesh, bounds: Option<([f32; 3], [f32; 3])>) -> Vec<u32> {
        if mesh.faces.is_empty() || mesh.faces.len() != mesh.indices.len() / 3 {
            return Vec::new();
        }
        let verdict = self.verdict(bounds.or_else(|| mesh.bounds()));
        if verdict == Verdict::Missed {
            return Vec::new();
        }
        let mut tallies: std::collections::BTreeMap<u32, Tally> = Default::default();
        for (tri, face) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.faces) {
            let tally = tallies.entry(*face).or_default();
            if verdict == Verdict::Inside {
                tally.shown = true;
                tally.touched = true;
                continue;
            }
            if let Some(piece) = self.triangle(mesh, tri) {
                self.count(tally, piece, Rect::meets_triangle);
            }
        }
        tallies
            .into_iter()
            .filter(|(_, tally)| tally.caught(self.catch))
            .map(|(face, _)| face)
            .collect()
    }

    /// The kernel edges of `mesh` the box takes, in order of their ids,
    /// read from its outline segments.
    pub fn edges(&self, mesh: &TriMesh, bounds: Option<([f32; 3], [f32; 3])>) -> Vec<u32> {
        if mesh.edges.is_empty() || mesh.edge_ids.len() != mesh.edges.len() / 2 {
            return Vec::new();
        }
        let verdict = self.verdict(bounds.or_else(|| mesh.bounds()));
        if verdict == Verdict::Missed {
            return Vec::new();
        }
        let mut tallies: std::collections::BTreeMap<u32, Tally> = Default::default();
        for (pair, edge) in mesh.edges.as_chunks::<2>().0.iter().zip(&mesh.edge_ids) {
            let tally = tallies.entry(*edge).or_default();
            if verdict == Verdict::Inside {
                tally.shown = true;
                tally.touched = true;
                continue;
            }
            let p = |i: u32| mesh.positions.get(i as usize).map(|p| Vec3::from_array(*p));
            let (Some(a), Some(b)) = (p(pair[0]), p(pair[1])) else {
                continue;
            };
            self.count(tally, self.show([a, b]), |rect, [a, b]| {
                rect.meets_segment(a, b)
            });
        }
        tallies
            .into_iter()
            .filter(|(_, tally)| tally.caught(self.catch))
            .map(|(edge, _)| edge)
            .collect()
    }
}

/// What a box picks under the view toolbar's filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Bodies,
    Faces,
    Edges,
}

impl PrintCadApp {
    /// A left press on empty space in the view starts a box, unless the
    /// press is the measure tool's or lands on a sketch, an edge or an
    /// origin plane.
    pub(crate) fn may_start_box(&self) -> bool {
        self.session.measure.is_none()
            && self.session.hovered_body.is_none()
            && self.session.hovered_edge.is_none()
            && self.session.hovered_base_plane.is_none()
            && !self.held_buttons[1]
            && !self.held_buttons[2]
            && self.feature_under_cursor().is_none()
    }

    /// Pick what the released box holds, by the view toolbar's filter
    /// (whole bodies unless it asks for faces or edges); Ctrl adds to what
    /// is picked, otherwise the box's catch replaces it.
    pub(crate) fn finish_box_select(&mut self, drag: BoxDrag) {
        let target = match self.session.pick_filter {
            PickFilter::Faces => Target::Faces,
            PickFilter::Edges => Target::Edges,
            PickFilter::Any | PickFilter::Bodies => Target::Bodies,
        };
        let camera = &self.session.camera;
        let probe = Probe {
            rect: drag.rect(),
            catch: drag.catch(),
            project: |p: Vec3| camera.world_to_viewport(p).map(|(x, y)| Vec2::new(x, y)),
            clip: self.frame_submission.clip_plane,
        };
        let caught = caught_in(&self.session.document, &probe, target);
        let ctrl = self.modifiers.control_key();
        if caught.is_empty() {
            if !ctrl {
                self.clear_view_selection();
                app_log::info("Box selection: nothing inside");
            }
            return;
        }
        self.session.last_select_click = None;
        match target {
            Target::Bodies => self.box_bodies(caught.into_iter().map(|(id, _)| id), ctrl),
            Target::Faces => self.box_faces(caught, ctrl),
            Target::Edges => self.box_edges(caught, ctrl),
        }
    }

    fn box_bodies(&mut self, caught: impl Iterator<Item = Uuid>, ctrl: bool) {
        let mut bodies = if ctrl {
            self.selected_bodies()
        } else {
            Vec::new()
        };
        for body in caught {
            if !bodies.contains(&body) {
                bodies.push(body);
            }
        }
        self.clear_view_selection();
        self.session.selected_body = bodies.pop();
        self.session.earlier_bodies = bodies;
        // As a double click on it does, the last body's row opens in the
        // tree, except in an edit session.
        if !self.registry.is_modal(&self.session.active_workbench.0) {
            self.session.reveal_body = self.session.selected_body.map(core_document::BodyId);
        }
        let count = self.selected_bodies().len();
        app_log::info(if count == 1 {
            "Selected 1 body".to_string()
        } else {
            format!("Selected {count} bodies")
        });
    }

    /// Faces are picked on one body: the one already picked on when Ctrl
    /// adds and the box holds some of its faces, otherwise the body the box
    /// takes the most faces of (the first of those tied).
    fn box_faces(&mut self, caught: Vec<(Uuid, Vec<u32>)>, ctrl: bool) {
        let current = self
            .session
            .last_face_hit
            .map(|(body, _)| body)
            .filter(|body| self.session.selected_body == Some(*body));
        let adding = current
            .filter(|_| ctrl)
            .and_then(|body| caught.iter().find(|(id, _)| *id == body));
        let Some((body, faces)) =
            adding.or_else(|| caught.iter().rev().max_by_key(|(_, faces)| faces.len()))
        else {
            return;
        };
        let body = *body;
        let Some(geometry) = self
            .session
            .document
            .imported_geometry(core_document::BodyId(body))
        else {
            return;
        };
        let mesh = std::sync::Arc::clone(&geometry.mesh);
        let mut picked: Vec<PickedFace> = Vec::new();
        if adding.is_some() {
            picked = std::mem::take(&mut self.session.earlier_faces);
            if let (Some((_, face)), Some(highlight)) = (
                self.session.last_face_hit,
                self.session.face_highlight.take(),
            ) {
                picked.push(PickedFace { face, highlight });
            }
        } else if !ctrl {
            self.session.selected_edges.clear();
        }
        for id in faces {
            if picked
                .iter()
                .any(|p| p.highlight.body == body && p.highlight.face == Some(*id))
            {
                continue;
            }
            let Some(point) = face_point(&mesh, *id) else {
                continue;
            };
            let Some(face) = crate::app::input::face_ref_from_mesh(&mesh, point) else {
                continue;
            };
            if let Some(highlight) = self.face_highlight_of(body, face) {
                picked.push(PickedFace { face, highlight });
            }
        }
        let last = picked.pop();
        self.session.earlier_faces = picked;
        self.session.last_face_hit = last.as_ref().map(|p| (body, p.face));
        self.session.face_highlight = last.map(|p| p.highlight);
        self.session.earlier_bodies.clear();
        self.session.selected_body = Some(body);
        let count = self.selected_face_refs().len();
        app_log::info(if count == 1 {
            "Selected 1 face".to_string()
        } else {
            format!("Selected {count} faces")
        });
    }

    fn box_edges(&mut self, caught: Vec<(Uuid, Vec<u32>)>, ctrl: bool) {
        if !ctrl {
            self.session.selected_edges.clear();
        }
        let mut last_body = None;
        for (body, edges) in caught {
            let Some(geometry) = self
                .session
                .document
                .imported_geometry(core_document::BodyId(body))
            else {
                continue;
            };
            let mesh = std::sync::Arc::clone(&geometry.mesh);
            for edge in edges {
                if self
                    .session
                    .selected_edges
                    .iter()
                    .any(|s| s.body == body && s.edge == edge)
                {
                    continue;
                }
                if let Some(hit) = crate::app::edges::edge_hit(&mesh, body, edge) {
                    self.session.selected_edges.push(hit);
                    last_body = Some(body);
                }
            }
        }
        let Some(body) = last_body.or(self.session.selected_body) else {
            return;
        };
        // Ctrl keeps the faces picked on the same body beside the edges.
        if !ctrl || self.session.selected_body != Some(body) {
            self.session.face_highlight = None;
            self.session.last_face_hit = None;
        }
        self.session.earlier_bodies.clear();
        self.session.selected_body = Some(body);
        app_log::info(format!(
            "Selected {} edge(s)",
            self.session.selected_edges.len()
        ));
    }

    /// Every body picked whole: the ones a box took beside the last, and
    /// the last. Empty while a face or an edge is the pick.
    pub(crate) fn selected_bodies(&self) -> Vec<Uuid> {
        if self.session.face_highlight.is_some() || !self.session.selected_edges.is_empty() {
            return Vec::new();
        }
        self.session
            .earlier_bodies
            .iter()
            .copied()
            .chain(self.session.selected_body)
            .collect()
    }
}

/// What the box takes of each body the view draws and lets be picked, in
/// the document's order of bodies: the body itself (no ids) for
/// [`Target::Bodies`], else its kernel faces or edges. Hidden and
/// unselectable bodies are left out, and so are sketches, which draw as
/// meshes too but are picked by clicks.
fn caught_in<P: Fn(Vec3) -> Option<Vec2>>(
    document: &core_document::Document,
    probe: &Probe<P>,
    target: Target,
) -> Vec<(Uuid, Vec<u32>)> {
    let order = |id: &Uuid| {
        document
            .bodies()
            .iter()
            .position(|b| b.id.0 == *id)
            .unwrap_or(usize::MAX)
    };
    let mut caught: Vec<(Uuid, Vec<u32>)> = document
        .imported_geometries()
        .filter(|(id, _)| {
            document.imported_body_effective_visible(**id)
                && document.body_selectable(**id)
                && document
                    .get_feature_meta(core_document::FeatureId(id.0))
                    .is_none()
        })
        .filter_map(|(id, geometry)| {
            let mesh = &*geometry.mesh;
            let ids = match target {
                Target::Bodies => {
                    if !probe.takes_body(mesh, geometry.bounds_mm) {
                        return None;
                    }
                    Vec::new()
                }
                Target::Faces => probe.faces(mesh, geometry.bounds_mm),
                Target::Edges => probe.edges(mesh, geometry.bounds_mm),
            };
            (target == Target::Bodies || !ids.is_empty()).then_some((id.0, ids))
        })
        .collect();
    caught.sort_by_key(|(id, _)| (order(id), *id));
    caught
}

/// A point on kernel face `face` of `mesh`: the centre of its largest
/// triangle, which lies on the face however the face curves.
fn face_point(mesh: &TriMesh, face: u32) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for (tri, id) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.faces) {
        if *id != face {
            continue;
        }
        let p = |i: u32| mesh.positions.get(i as usize).map(|p| Vec3::from_array(*p));
        let (Some(a), Some(b), Some(c)) = (p(tri[0]), p(tri[1]), p(tri[2])) else {
            continue;
        };
        let area = (b - a).cross(c - a).length();
        if best.is_none_or(|(most, _)| area > most) {
            best = Some((area, (a + b + c) / 3.0));
        }
    }
    best.map(|(_, centre)| centre)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view straight down the z axis: a point's x and y are its pixels,
    /// and nothing at z above 10 is in front of the eye.
    fn probe(from: Vec2, to: Vec2, clip: Option<[f32; 4]>) -> Probe<impl Fn(Vec3) -> Option<Vec2>> {
        let drag = BoxDrag { from, to };
        Probe {
            rect: drag.rect(),
            catch: drag.catch(),
            project: |p: Vec3| (p.z < 10.0).then_some(Vec2::new(p.x, p.y)),
            clip,
        }
    }

    /// Two unit squares side by side at z = 0, each its own kernel face
    /// (two triangles), outlined by four edges each.
    fn two_squares() -> TriMesh {
        let mut mesh = TriMesh::default();
        for (face, x) in [(0u32, 0.0f32), (1, 10.0)] {
            let base = mesh.positions.len() as u32;
            for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                mesh.positions.push([x + dx, dy, 0.0]);
                mesh.normals.push([0.0, 0.0, 1.0]);
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            mesh.faces.extend_from_slice(&[face, face]);
            for side in 0..4 {
                mesh.edges.push(base + side);
                mesh.edges.push(base + (side + 1) % 4);
                mesh.edge_ids.push(face * 4 + side);
            }
        }
        mesh
    }

    #[test]
    fn dragging_left_is_a_crossing_and_a_short_drag_is_a_click() {
        let right = BoxDrag {
            from: Vec2::new(5.0, 5.0),
            to: Vec2::new(50.0, 1.0),
        };
        assert_eq!(right.catch(), Catch::Window);
        assert_eq!(right.rect().min, Vec2::new(5.0, 1.0));
        let left = BoxDrag {
            from: Vec2::new(50.0, 5.0),
            to: Vec2::new(5.0, 9.0),
        };
        assert_eq!(left.catch(), Catch::Crossing);
        assert!(left.outline([1.0; 3]).iter().all(|l| l.dash.is_some()));
        assert!(right.outline([1.0; 3]).iter().all(|l| l.dash.is_none()));
        assert!(!BoxDrag::at(Vec2::ONE).dragged(4.0));
        assert!(right.dragged(4.0));
    }

    #[test]
    fn a_window_takes_only_faces_wholly_inside() {
        let mesh = two_squares();
        // Holds the first square and half the second.
        let window = probe(Vec2::new(-1.0, -1.0), Vec2::new(10.5, 2.0), None);
        assert_eq!(window.faces(&mesh, None), vec![0]);
        assert!(!window.takes_body(&mesh, None));
        let all = probe(Vec2::new(-1.0, -1.0), Vec2::new(12.0, 2.0), None);
        assert_eq!(all.faces(&mesh, None), vec![0, 1]);
        assert!(all.takes_body(&mesh, None));
    }

    #[test]
    fn a_crossing_takes_whatever_it_touches() {
        let mesh = two_squares();
        let crossing = probe(Vec2::new(10.5, 2.0), Vec2::new(5.0, 0.5), None);
        assert_eq!(crossing.faces(&mesh, None), vec![1]);
        assert!(crossing.takes_body(&mesh, None));
        // The second square's top and left sides; its bottom and right
        // pass outside the box.
        assert_eq!(crossing.edges(&mesh, None), vec![6, 7]);
        // A box wholly inside a face touches it though it holds no corner.
        let within = probe(Vec2::new(10.6, 0.6), Vec2::new(10.4, 0.4), None);
        assert_eq!(within.faces(&mesh, None), vec![1]);
        assert!(within.edges(&mesh, None).is_empty());
        let empty = probe(Vec2::new(8.0, 2.0), Vec2::new(3.0, -1.0), None);
        assert!(empty.faces(&mesh, None).is_empty());
        assert!(!empty.takes_body(&mesh, None));
    }

    #[test]
    fn a_window_takes_edges_wholly_inside() {
        let mesh = two_squares();
        // The first square's left side, x = 0, and nothing else whole.
        let window = probe(Vec2::new(-1.0, -1.0), Vec2::new(0.5, 2.0), None);
        assert_eq!(window.edges(&mesh, None), vec![3]);
    }

    #[test]
    fn what_the_clipping_plane_cuts_away_is_left_out() {
        let mesh = two_squares();
        // The scene keeps x <= 5: the second square is not drawn.
        let keep_left = Some([-1.0, 0.0, 0.0, 5.0]);
        let crossing = probe(Vec2::new(12.0, 2.0), Vec2::new(-1.0, -1.0), keep_left);
        assert_eq!(crossing.faces(&mesh, None), vec![0]);
        assert_eq!(crossing.edges(&mesh, None), vec![0, 1, 2, 3]);
        // A window round what is drawn takes the body, the cut part aside.
        let window = probe(Vec2::new(-1.0, -1.0), Vec2::new(2.0, 2.0), keep_left);
        assert!(window.takes_body(&mesh, None));
        // A body wholly cut away is not taken, whatever box holds it.
        let keep_far = Some([1.0, 0.0, 0.0, -100.0]);
        let all = probe(Vec2::new(12.0, 2.0), Vec2::new(-1.0, -1.0), keep_far);
        assert!(!all.takes_body(&mesh, None));
        assert!(all.faces(&mesh, None).is_empty());
    }

    #[test]
    fn what_reaches_behind_the_eye_is_never_wholly_inside() {
        let mut mesh = two_squares();
        // Lift a corner of the second square's first triangle behind the
        // eye; its other triangle stays in view.
        mesh.positions[5][2] = 20.0;
        let window = probe(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0), None);
        assert_eq!(window.faces(&mesh, None), vec![0]);
        assert!(!window.takes_body(&mesh, None));
        let crossing = probe(Vec2::new(100.0, 100.0), Vec2::new(-100.0, -100.0), None);
        assert_eq!(crossing.faces(&mesh, None), vec![0, 1]);
    }

    #[test]
    fn hidden_bodies_are_never_taken() {
        let mut document = core_document::Document::new("t");
        let mut bodies = Vec::new();
        for (name, dx) in [("left", 0.0f32), ("right", 100.0), ("hidden", 200.0)] {
            let mut mesh = two_squares();
            for p in &mut mesh.positions {
                p[0] += dx;
            }
            let body = document.create_body(Some(name.into()));
            document.set_imported_geometry(
                body,
                core_document::ImportedGeometry {
                    bounds_mm: mesh.bounds(),
                    mesh: std::sync::Arc::new(mesh),
                    source_asset: None,
                    revision: 1,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
            bodies.push(body.0);
        }
        document.set_body_visible(core_document::BodyId(bodies[2]), false);
        let everything = probe(Vec2::new(500.0, 50.0), Vec2::new(-50.0, -50.0), None);
        let taken = caught_in(&document, &everything, Target::Bodies);
        let ids: Vec<Uuid> = taken.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![bodies[0], bodies[1]], "in the document's order");
        let faces = caught_in(&document, &everything, Target::Faces);
        assert!(
            faces
                .iter()
                .all(|(id, faces)| *id != bodies[2] && faces == &[0, 1])
        );
        let edges = caught_in(&document, &everything, Target::Edges);
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn bounds_wholly_inside_settle_without_the_triangles() {
        let mesh = two_squares();
        let all = probe(Vec2::new(-1.0, -1.0), Vec2::new(12.0, 2.0), None);
        let bounds = Some(([0.0, 0.0, 0.0], [11.0, 1.0, 0.0]));
        assert_eq!(all.faces(&mesh, bounds), vec![0, 1]);
        assert_eq!(all.edges(&mesh, bounds).len(), 8);
        // Bounds off the box rule the body out.
        let far = probe(Vec2::new(50.0, 50.0), Vec2::new(60.0, 60.0), None);
        assert!(!far.takes_body(&mesh, bounds));
    }
}
