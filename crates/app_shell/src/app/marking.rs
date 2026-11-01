//! Marking a scan's surface by hand: a window, a brush or a lasso dragged
//! over the view marks the triangles it covers, a right drag (or a drag
//! with Alt) rubs them out. What is marked is a set of the scan mesh's
//! vertices; a triangle is marked when its three corners are. Only
//! triangles facing the view are taken unless "faces pointing away" is on,
//! so a window never marks the far wall. The marking is the tab's until it
//! is cleared or its scan changes; the tools that read it (fits, mesh
//! edits) take it from here.

use std::sync::Arc;

use core_document::BodyId;
use glam::{Vec2, Vec3};
use kernel_api::TriMesh;

use crate::PrintCadApp;

/// How a gesture covers the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MarkTool {
    #[default]
    Window,
    Brush,
    Lasso,
}

impl MarkTool {
    pub const ALL: [MarkTool; 3] = [MarkTool::Window, MarkTool::Brush, MarkTool::Lasso];

    pub fn label(self) -> &'static str {
        match self {
            MarkTool::Window => "Window",
            MarkTool::Brush => "Brush",
            MarkTool::Lasso => "Lasso",
        }
    }
}

/// The marking tool in hand, while the Mark surface task is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MarkArm {
    pub body: BodyId,
    pub tool: MarkTool,
    /// The brush's diameter, in millimetres.
    pub brush_mm: f32,
    /// Take faces pointing away from the view too.
    pub through: bool,
}

/// A change to the whole marking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkEdit {
    Invert,
    Clear,
}

/// A drag in progress: whether it rubs out, and the pointer's way.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MarkGesture {
    pub erase: bool,
    pub points: Vec<Vec2>,
}

/// The marked part of a scan.
#[derive(Debug, Clone)]
pub(crate) struct Marking {
    pub body: BodyId,
    /// The scan mesh's revision the vertex numbers belong to.
    pub revision: u64,
    pub marked: Vec<bool>,
    /// What the view draws: the marked triangles, rebuilt on change.
    pub drawn: Arc<TriMesh>,
    /// Bumped on every change, so the drawn mesh's buffers are replaced.
    pub version: u64,
}

impl Marking {
    pub fn new(body: BodyId, revision: u64, vertices: usize) -> Self {
        Self {
            body,
            revision,
            marked: vec![false; vertices],
            drawn: Arc::new(TriMesh::default()),
            version: 0,
        }
    }

    /// How many vertices and whole triangles are marked.
    pub fn counts(&self, mesh: &TriMesh) -> (usize, usize) {
        let vertices = self.marked.iter().filter(|m| **m).count();
        (vertices, marked_triangles(mesh, &self.marked).count())
    }

    /// Mark (or, `erase`, unmark) the corners of `triangles`.
    pub fn apply(&mut self, mesh: &TriMesh, triangles: &[u32], erase: bool) {
        for &t in triangles {
            for k in 0..3 {
                if let Some(&v) = mesh.indices.get(t as usize * 3 + k)
                    && let Some(slot) = self.marked.get_mut(v as usize)
                {
                    *slot = !erase;
                }
            }
        }
        self.refresh(mesh);
    }

    /// Swap marked and unmarked. A vertex on the border between them stays
    /// bare both ways, so a marking and its inverse never share a triangle.
    pub fn invert(&mut self, mesh: &TriMesh) {
        let mut border = vec![false; self.marked.len()];
        for t in mesh.indices.as_chunks::<3>().0 {
            let states = [t[0], t[1], t[2]].map(|v| self.marked[v as usize]);
            if states.iter().any(|s| *s) && !states.iter().all(|s| *s) {
                for v in t {
                    border[*v as usize] = true;
                }
            }
        }
        for (m, edge) in self.marked.iter_mut().zip(border) {
            *m = !*m && !edge;
        }
        self.refresh(mesh);
    }

    pub fn clear(&mut self, mesh: &TriMesh) {
        self.marked.iter_mut().for_each(|m| *m = false);
        self.refresh(mesh);
    }

    pub(crate) fn refresh(&mut self, mesh: &TriMesh) {
        let mut drawn = TriMesh {
            positions: mesh.positions.clone(),
            normals: mesh.normals.clone(),
            ..TriMesh::default()
        };
        for t in marked_triangles(mesh, &self.marked) {
            drawn
                .indices
                .extend_from_slice(&mesh.indices[t * 3..t * 3 + 3]);
        }
        self.drawn = Arc::new(drawn);
        self.version += 1;
    }
}

fn marked_triangles<'a>(mesh: &'a TriMesh, marked: &'a [bool]) -> impl Iterator<Item = usize> + 'a {
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, t)| t.iter().all(|v| marked.get(*v as usize) == Some(&true)))
        .map(|(i, _)| i)
}

/// What a gesture covers on screen, in viewport pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cover {
    /// The rectangle between two corners.
    Window(Vec2, Vec2),
    /// The polygon a lasso drew.
    Lasso(Vec<Vec2>),
    /// Discs of `radius` along a brush's stroke.
    Brush { centres: Vec<Vec2>, radius: f32 },
}

impl Cover {
    /// Whether a point on screen is covered.
    pub fn holds(&self, p: Vec2) -> bool {
        match self {
            Cover::Window(a, b) => {
                let (lo, hi) = (a.min(*b), a.max(*b));
                p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
            }
            Cover::Lasso(path) => in_polygon(p, path),
            Cover::Brush { centres, radius } => centres.iter().any(|c| c.distance(p) <= *radius),
        }
    }
}

/// The triangles of `mesh` whose three corners `cover` holds as the view
/// shows them: `project` gives a point's place in viewport pixels (`None`
/// behind the eye), `eye` where the view looks from, `None` for a view
/// along `forward` alone (orthographic). With `through` off only triangles
/// facing the view are taken.
pub(crate) fn covered(
    mesh: &TriMesh,
    cover: &Cover,
    project: impl Fn(Vec3) -> Option<Vec2>,
    eye: Option<Vec3>,
    forward: Vec3,
    through: bool,
) -> Vec<u32> {
    let shown: Vec<Option<Vec2>> = mesh
        .positions
        .iter()
        .map(|p| project(Vec3::from_array(*p)).filter(|s| cover.holds(*s)))
        .collect();
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, t)| t.iter().all(|v| shown[*v as usize].is_some()))
        .filter(|(_, t)| {
            through || {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|v| Vec3::from_array(mesh.positions[v as usize]));
                let normal = (b - a).cross(c - a);
                let toward_eye = eye.map_or(-forward, |e| e - (a + b + c) / 3.0);
                normal.dot(toward_eye) > 0.0
            }
        })
        .map(|(i, _)| i as u32)
        .collect()
}

/// Even-odd point in polygon.
fn in_polygon(p: Vec2, path: &[Vec2]) -> bool {
    let mut inside = false;
    for (i, a) in path.iter().enumerate() {
        let b = path[(i + 1) % path.len()];
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

impl PrintCadApp {
    /// The marking of the armed scan, made fresh when there is none or its
    /// scan changed; `None` when the scan has no mesh.
    fn marking_for(&mut self, body: BodyId) -> Option<(&mut Marking, Arc<TriMesh>)> {
        let geometry = self.session.document.imported_geometry(body)?;
        let (mesh, revision) = (Arc::clone(&geometry.mesh), geometry.revision);
        let fresh = self
            .session
            .marking
            .as_ref()
            .is_none_or(|m| m.body != body || m.revision != revision);
        if fresh {
            self.session.marking = Some(Marking::new(body, revision, mesh.positions.len()));
        }
        Some((self.session.marking.as_mut()?, mesh))
    }

    /// A press over the view while a tool is armed starts a gesture.
    pub(crate) fn mark_press(&mut self, at: Vec2, erase: bool) {
        self.session.mark_gesture = Some(MarkGesture {
            erase,
            points: vec![at],
        });
        self.redraw_needed = true;
    }

    pub(crate) fn mark_move(&mut self, at: Vec2) {
        if let Some(gesture) = self.session.mark_gesture.as_mut() {
            gesture.points.push(at);
            self.redraw_needed = true;
        }
    }

    /// The gesture's release marks (or rubs out) what it covered.
    pub(crate) fn mark_release(&mut self) {
        let (Some(gesture), Some(arm)) = (self.session.mark_gesture.take(), self.session.mark_arm)
        else {
            return;
        };
        let Some(cover) = self.mark_cover(&gesture, &arm) else {
            return;
        };
        let camera = &self.session.camera;
        let project = |p: Vec3| camera.world_to_viewport(p).map(|(x, y)| Vec2::new(x, y));
        let view = crate::app::scene_guides::GuideView::of(camera);
        let eye = matches!(
            camera.state.projection,
            settings::ProjectionMode::Perspective
        )
        .then_some(view.eye);
        let Some(geometry) = self.session.document.imported_geometry(arm.body) else {
            return;
        };
        let triangles = covered(
            &geometry.mesh,
            &cover,
            project,
            eye,
            view.forward,
            arm.through,
        );
        if let Some((marking, mesh)) = self.marking_for(arm.body) {
            marking.apply(&mesh, &triangles, gesture.erase);
        }
        self.redraw_needed = true;
    }

    /// What a gesture covers: a brush's discs are its diameter across, at
    /// the depth of the scan's middle.
    fn mark_cover(&self, gesture: &MarkGesture, arm: &MarkArm) -> Option<Cover> {
        let first = *gesture.points.first()?;
        let last = *gesture.points.last()?;
        Some(match arm.tool {
            MarkTool::Window => Cover::Window(first, last),
            MarkTool::Lasso if gesture.points.len() >= 3 => Cover::Lasso(gesture.points.clone()),
            MarkTool::Lasso => return None,
            MarkTool::Brush => {
                let (lo, hi) = self
                    .session
                    .document
                    .imported_geometry(arm.body)?
                    .mesh
                    .bounds()?;
                let middle = (Vec3::from_array(lo) + Vec3::from_array(hi)) / 2.0;
                let view = crate::app::scene_guides::GuideView::of(&self.session.camera);
                Cover::Brush {
                    centres: gesture.points.clone(),
                    radius: arm.brush_mm / 2.0 / view.world_per_px_at(middle).max(1e-6),
                }
            }
        })
    }

    pub(crate) fn mark_edit(&mut self, edit: MarkEdit) {
        let Some(arm) = self.session.mark_arm else {
            return;
        };
        if let Some((marking, mesh)) = self.marking_for(arm.body) {
            match edit {
                MarkEdit::Invert => marking.invert(&mesh),
                MarkEdit::Clear => marking.clear(&mesh),
            }
        }
        self.redraw_needed = true;
    }

    /// The marked and total counts of the armed scan, for its task.
    pub(crate) fn marking_counts(&self) -> Option<(usize, usize)> {
        let arm = self.session.mark_arm?;
        let marking = self
            .session
            .marking
            .as_ref()
            .filter(|m| m.body == arm.body)?;
        let mesh = &self.session.document.imported_geometry(arm.body)?.mesh;
        Some(marking.counts(mesh))
    }

    /// The gesture's outline over the view: the window, the lasso's path,
    /// the brush's ring at the pointer.
    pub(crate) fn mark_outline(&self, color: [f32; 3]) -> Vec<core_document::ScreenSpaceOverlay> {
        let (Some(gesture), Some(arm)) = (&self.session.mark_gesture, self.session.mark_arm) else {
            return Vec::new();
        };
        let line = |a: Vec2, b: Vec2| {
            core_document::ScreenSpaceOverlay::new(a.to_array(), b.to_array(), color, 1.0)
        };
        let ring = |c: Vec2, r: f32| -> Vec<_> {
            (0..32)
                .map(|i| {
                    let at = |k: f32| c + r * Vec2::from_angle(k * std::f32::consts::TAU / 32.0);
                    line(at(i as f32), at(i as f32 + 1.0))
                })
                .collect()
        };
        match self.mark_cover(gesture, &arm) {
            Some(Cover::Window(a, b)) => {
                let corners = [a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)];
                (0..4)
                    .map(|i| line(corners[i], corners[(i + 1) % 4]))
                    .collect()
            }
            Some(Cover::Brush { centres, radius }) => {
                centres.last().map_or_else(Vec::new, |c| ring(*c, radius))
            }
            _ => gesture
                .points
                .windows(2)
                .map(|w| line(w[0], w[1]))
                .collect(),
        }
    }
}

/// Which vertices a script's region takes: inside its box and ball, where
/// given, and facing within `limit` (radians) of `facing`, where given.
pub(crate) struct Region {
    pub bounds: Option<(Vec3, Vec3)>,
    pub ball: Option<(Vec3, f32)>,
    pub facing: Option<(Vec3, f32)>,
}

impl Region {
    fn takes(&self, mesh: &TriMesh, v: usize) -> bool {
        let p = Vec3::from_array(mesh.positions[v]);
        let inside = self
            .bounds
            .is_none_or(|(lo, hi)| p.cmpge(lo).all() && p.cmple(hi).all());
        let near = self.ball.is_none_or(|(c, r)| p.distance(c) <= r);
        let faces = self.facing.is_none_or(|(dir, limit)| {
            mesh.normals
                .get(v)
                .is_some_and(|n| Vec3::from_array(*n).angle_between(dir) <= limit)
        });
        inside && near && faces
    }
}

impl PrintCadApp {
    /// `scan.mark`: change the marking of a scan as a script asks, and say
    /// how much is marked.
    pub(crate) fn run_scan_mark(
        &mut self,
        a: &core_document::Args,
    ) -> core_document::CommandResult {
        use core_document::CommandError;
        let body = BodyId(a.id("body")?);
        if self.session.document.body_scan(body).is_none() {
            return Err(CommandError::bad("body", "is not a scan"));
        }
        let point = |v: &serde_json::Value, name: &str| -> Result<Vec3, CommandError> {
            let get = |k: &str, i: usize| v.get(k).or_else(|| v.get(i)).and_then(|n| n.as_f64());
            match (get("x", 0), get("y", 1), get("z", 2)) {
                (Some(x), Some(y), Some(z)) => Ok(Vec3::new(x as f32, y as f32, z as f32)),
                _ => Err(CommandError::bad(name, "needs {x, y, z}")),
            }
        };
        let field = |name: &str| a.0.get(name).filter(|v| !v.is_null());
        let bounds = match field("box") {
            Some(v) => Some((
                point(v.get("min").unwrap_or(&serde_json::Value::Null), "box")?,
                point(v.get("max").unwrap_or(&serde_json::Value::Null), "box")?,
            )),
            None => None,
        };
        let ball = match field("ball") {
            Some(v) => {
                let radius = v
                    .get("radius")
                    .and_then(|r| r.as_f64())
                    .filter(|r| *r > 0.0)
                    .ok_or_else(|| CommandError::bad("ball", "needs a radius above 0"))?;
                Some((
                    point(v.get("centre").unwrap_or(&serde_json::Value::Null), "ball")?,
                    radius as f32,
                ))
            }
            None => None,
        };
        let limit = a.opt_number("limit")?.unwrap_or(60.0).to_radians() as f32;
        let facing = match field("facing") {
            Some(v) => Some((point(v, "facing")?.normalize_or_zero(), limit)),
            None => None,
        };
        let erase = a.opt_bool("erase")? == Some(true);
        let (invert, clear) = (
            a.opt_bool("invert")? == Some(true),
            a.opt_bool("clear")? == Some(true),
        );
        let Some((marking, mesh)) = self.marking_for(body) else {
            return Err(CommandError::failed("the scan has no mesh"));
        };
        if clear {
            marking.clear(&mesh);
        }
        if invert {
            marking.invert(&mesh);
        }
        if bounds.is_some() || ball.is_some() || facing.is_some() {
            let region = Region {
                bounds,
                ball,
                facing,
            };
            for v in 0..mesh.positions.len() {
                if region.takes(&mesh, v) {
                    marking.marked[v] = !erase;
                }
            }
            marking.refresh(&mesh);
        }
        let (points, triangles) = marking.counts(&mesh);
        self.redraw_needed = true;
        Ok(serde_json::json!({"points": points, "triangles": triangles}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two unit squares facing the eye on +Z, and a square under them
    /// facing away, viewed straight down: a window over the left square
    /// takes its two triangles and not the far one below; with faces
    /// pointing away it takes that one too. Inverting leaves the border
    /// bare, and clearing leaves nothing.
    #[test]
    fn a_window_takes_what_faces_it_and_inverts_with_a_bare_border() {
        let positions = vec![
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
            [2.0, 0.0, 1.0],
            [2.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let indices = vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2, 6, 8, 7];
        let mesh = TriMesh {
            positions,
            indices,
            ..TriMesh::default()
        };
        let project = |p: Vec3| Some(Vec2::new(p.x * 100.0, p.y * 100.0));
        let down = Vec3::NEG_Z;
        let window = Cover::Window(Vec2::new(-5.0, -5.0), Vec2::new(105.0, 105.0));
        assert_eq!(
            covered(&mesh, &window, project, None, down, false),
            vec![0, 1]
        );
        assert_eq!(
            covered(&mesh, &window, project, None, down, true),
            vec![0, 1, 4]
        );
        // A lasso round the lower right half of the left square.
        let lasso = Cover::Lasso(vec![
            Vec2::new(-10.0, -5.0),
            Vec2::new(110.0, -5.0),
            Vec2::new(110.0, 110.0),
        ]);
        assert_eq!(covered(&mesh, &lasso, project, None, down, false), vec![0]);
        let brush = Cover::Brush {
            centres: vec![Vec2::new(200.0, 50.0)],
            radius: 60.0,
        };
        assert!(covered(&mesh, &brush, project, None, down, false).is_empty());

        let mut marking = Marking::new(BodyId(uuid::Uuid::nil()), 0, mesh.positions.len());
        marking.apply(&mesh, &[0, 1], false);
        assert_eq!(marking.counts(&mesh), (4, 2));
        marking.invert(&mesh);
        // Vertices 1, 2, 4 and 5 border the marking: bare both ways, so
        // only the far square is marked.
        assert_eq!(marking.counts(&mesh), (3, 1));
        marking.clear(&mesh);
        assert_eq!(marking.counts(&mesh), (0, 0));

        // A script's region: points in a ball, and only those facing down.
        let mut mesh = mesh;
        mesh.normals = (0..9)
            .map(|v| {
                if v < 6 {
                    [0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, -1.0]
                }
            })
            .collect();
        let ball = Region {
            bounds: None,
            ball: Some((Vec3::ZERO, 1.5)),
            facing: None,
        };
        let down = Region {
            facing: Some((Vec3::NEG_Z, 0.1)),
            ..ball
        };
        let taken = |r: &Region| (0..9).filter(|v| r.takes(&mesh, *v)).collect::<Vec<_>>();
        assert_eq!(taken(&ball), vec![0, 1, 3, 6, 7, 8]);
        assert_eq!(taken(&down), vec![6, 7, 8]);
    }
}
