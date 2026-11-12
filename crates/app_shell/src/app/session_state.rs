//! A tab's state in groups a module can borrow on their own while the
//! rest of the session is borrowed elsewhere.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use core_document::{BodyId, FeatureId};
use uuid::Uuid;

use crate::app::doc_io::{OpenJob, SaveJob, SaveProgress};
use crate::app::session::BodyPreview;
use crate::camera::CameraController;
use crate::ui::TreeItemId;
use crate::{DimensionCache, RemoteImportRoute};

/// Where the tab looks from and how the view is laid out.
pub(crate) struct ViewState {
    pub camera: CameraController,
    /// The view toolbar's clipping plane, when on.
    pub section: Option<crate::camera::section::SectionPlane>,
    /// The pick in the view the clipping plane waits for, while aiming.
    pub section_pick: Option<crate::camera::section::SectionPick>,
    /// View › Split view is on for this tab.
    pub split: bool,
    /// The side pane's own camera, while View › Link split cameras is off;
    /// `None` draws the side pane with `camera`.
    pub side_camera: Option<CameraController>,
    /// Where the side pane sat last frame, in physical window pixels
    /// (`x, y, width, height`).
    pub side_rect: Option<(u32, u32, u32, u32)>,
    pub grid_plane: super::scene_guides::GridPlaneState,
    /// The view frames the opened document once the workspace's viewport
    /// is laid out and its bodies have geometry, which a file's meshes
    /// reach after the open; a press or a wheel turn in the view first
    /// leaves the view as the user puts it.
    pub fit_on_layout: bool,
    /// What a click in the view picks.
    pub pick_filter: crate::ui::PickFilter,
}

/// What lies under the pointer, in the view and in the tree.
#[derive(Default)]
pub(crate) struct Hover {
    pub body: Option<Uuid>,
    pub world_pos: Option<[f32; 3]>,
    /// The triangle of `body`'s mesh the pick pass drew under the
    /// cursor, where the GPU numbers them and the mesh drawn was the one
    /// the document holds.
    pub triangle: Option<crate::app::input::HoveredTriangle>,
    /// The origin plane under the cursor, while the active bench shows them
    /// to be picked from (`Workbench::shows_origin_planes`).
    pub base_plane: Option<core_document::BasePlane>,
    /// Feature under the cursor (CPU hit-test, drives hover tint).
    pub feature: Option<FeatureId>,
    /// The part of a feature under the cursor (a region of a closed
    /// sketch), lit while no task is open.
    pub part: Option<(FeatureId, core_document::FeaturePart)>,
    /// The edge under the cursor, found on the CPU against the hovered
    /// body's outline segments.
    pub edge: Option<crate::app::edges::EdgeHit>,
    /// The face under the cursor, when no edge takes the hover.
    pub face: Option<crate::app::input::FaceHover>,
    /// The face under the cursor whatever takes the hover, as benches are
    /// told of it.
    pub face_ref: Option<core_document::FaceRef>,
    /// The end of the hovered edge within a few pixels of the cursor.
    pub vertex: Option<core_document::VertexRef>,
    /// The depths the pick pass drew around the cursor, and the camera it
    /// drew them with: which edges near the cursor are in view.
    pub pick_depths: Option<render_wgpu::DepthWindow>,
    /// The tree row under the pointer: what it stands for lights up.
    pub tree: Option<crate::ui::TreeItemId>,
}

/// What is picked in the view and the tree.
#[derive(Default)]
pub(crate) struct Selection {
    pub body: Option<Uuid>,
    /// Face under the most recent body selection click (surface point +
    /// normal derived from the picked mesh triangle).
    pub last_face_hit: Option<(Uuid, core_document::FaceRef)>,
    /// Extracted sub-mesh of the selected face, rendered as a highlight
    /// overlay. Present only while a face (not the whole body) is the
    /// selection.
    pub face_highlight: Option<crate::app::input::FaceHighlight>,
    /// The selected faces' ids, found once per selection and mesh.
    pub face_ids: crate::app::face_pick::PickedFaceIds,
    /// Parts of a feature picked in the view on their own, in the order
    /// picked, with the feature they belong to: they stand for the
    /// selection while that feature is `active_document_object`.
    pub parts: Option<(FeatureId, Vec<core_document::FeaturePart>)>,
    /// The edges picked in the viewport; Ctrl adds to them.
    pub edges: Vec<crate::app::edges::EdgeHit>,
    /// Faces Ctrl added before the last one picked (`last_face_hit`), on
    /// its body, in the order picked. Empty whenever no face is picked.
    pub earlier_faces: Vec<crate::app::input::PickedFace>,
    /// Bodies a box picked whole beside `selected_body`, the last. Empty
    /// whenever a face or an edge is the pick.
    pub earlier_bodies: Vec<Uuid>,
    /// Timestamp + target of the last selection click (double-click detect).
    pub last_select_click: Option<(Instant, Uuid)>,
    /// The last click on an edge, to tell a double click: when, the body
    /// and the edge.
    pub last_edge_click: Option<(std::time::Instant, Uuid, u32)>,
    pub tree: Option<TreeItemId>,
    /// Features Ctrl-clicked into the tree's selection beside the row
    /// they were added with; read through `tree_also`, which drops them
    /// once that row is no longer the selection.
    pub tree_also: Option<(TreeItemId, Vec<FeatureId>)>,
    /// A body double-clicked in the viewport, for the one frame it takes
    /// the tree to jump to its row.
    pub reveal_body: Option<BodyId>,
    /// A face was just picked: the tree opens and scrolls to the row of
    /// the feature that made it.
    pub reveal_face_maker: bool,
    /// The selection as the open task found it, which its Cancel puts back.
    pub before_task: Option<Box<KeptSelection>>,
}

/// What was selected, in the view and the tree, as a task opened.
pub(crate) struct KeptSelection {
    pub body: Option<Uuid>,
    pub last_face_hit: Option<(Uuid, core_document::FaceRef)>,
    pub face_highlight: Option<crate::app::input::FaceHighlight>,
    pub earlier_faces: Vec<crate::app::input::PickedFace>,
    pub edges: Vec<crate::app::edges::EdgeHit>,
    pub earlier_bodies: Vec<Uuid>,
    pub parts: Option<(FeatureId, Vec<core_document::FeaturePart>)>,
    pub tree: Option<TreeItemId>,
    pub tree_also: Option<(TreeItemId, Vec<FeatureId>)>,
    pub active_document_object: Option<FeatureId>,
    pub active_body_id: Option<BodyId>,
}

/// The tab's file and the work on its way to or from disk.
#[derive(Default)]
pub(crate) struct FileState {
    pub path: Option<PathBuf>,
    /// What wrote the tab's file and whether it can be written in place.
    pub safety: crate::app::file_safety::FileSafety,
    /// The worker packing a `.prtcad` archive, and how far it has got.
    /// Packing carries every blob in the document, so it happens off the
    /// UI thread and the window keeps drawing while it runs.
    pub save_rx: Option<std::sync::mpsc::Receiver<SaveJob>>,
    /// The worker parsing an opened document, for the same reason.
    pub open_rx: Option<std::sync::mpsc::Receiver<OpenJob>>,
    pub save_progress: Option<std::sync::Arc<SaveProgress>>,
    /// The export dialog's draft, while it is open.
    pub export_pending: Option<crate::app::export::ExportDraft>,
    /// The exporting walkthrough is waiting on a solid to open the export
    /// dialog over.
    pub export_when_ready: bool,
    /// Picked STEP path and draft tessellation settings until the user
    /// confirms import.
    pub step_import_pending: Option<(PathBuf, kernel_api::TessellationSettings)>,
    /// An STL picked or dropped, waiting for the answer to what units it is in.
    pub mesh_units_pending: Option<PathBuf>,
    /// Files picked or dropped together, waiting in order while the
    /// import dialog or the units question is open for the one before.
    pub imports_waiting: std::collections::VecDeque<PathBuf>,
}

/// The tab's document server connection and what crosses it.
pub(crate) struct Connection {
    /// The document server connection: the local daemon by default, direct
    /// files when no daemon can run. Everything that crosses it is the wire
    /// protocol; `document_load_epoch` rides
    /// Open requests as the token that invalidates late responses.
    pub server: Box<dyn core_document::server::DocumentServer>,
    /// The socket the server connection is (or should be) on: the
    /// document's own once it has a file, the tab's own for Untitled.
    /// Reconnects and connection switches aim here.
    pub socket: PathBuf,
    /// Last reconnect attempt, so a dead daemon is retried at a gentle pace
    /// instead of every frame.
    pub last_reconnect: Option<Instant>,
    /// Remote imports being re-derived: a peer's ImportModel op created the
    /// bodies; the kernel re-imports the carried bytes (written to a temp
    /// file) and the resulting meshes are routed to those pre-existing
    /// bodies by import order, deterministic at any thread count.
    pub remote_import_routes: HashMap<PathBuf, RemoteImportRoute>,
    /// What each peer has selected, keyed by actor. Bodies in here render
    /// with the peer tint; entries die with their peer.
    pub peer_presence: HashMap<Uuid, core_document::server::PresenceState>,
    /// Relayed ops that arrived while an open was in flight. The daemon's
    /// threads may interleave a relay before the Opened reply; applying it
    /// to the document the open is about to replace would lose the edit,
    /// so they wait here and apply right after the new document lands.
    pub held_remote_ops: Vec<(Uuid, Vec<core_document::op::DocumentOp>)>,
    /// The last presence told to the server, so only changes cross the wire.
    pub last_sent_presence: Option<core_document::server::PresenceState>,
    pub load_epoch: u64,
}

/// The host's own tools in the view and what they hold.
#[derive(Default)]
pub(crate) struct HostTools {
    /// The measure tool: armed with the points, edges, faces and datums
    /// picked so far, in the order picked.
    pub measure: Option<Vec<crate::app::measure::MeasurePick>>,
    /// The least distance between the measure tool's last two faces or
    /// edges, asked of the kernel.
    pub measure_gap: Option<crate::app::measure::MeasureGap>,
    /// The status bar's quick measure: the selection it read, and the
    /// least distance asked of the kernel between its last two picks.
    pub quick_measure: Option<crate::app::measure::QuickMeasure>,
    /// The last token a least distance was asked under, by the measure
    /// tool or the quick measure.
    pub gap_token: u64,
    /// The note tool is armed: a click on a part puts a note there.
    pub note_tool: bool,
    /// The note card, open on a new note or one in the document.
    pub note_card: Option<crate::app::note_tool::NoteCard>,
    /// The marked surface of a scan, and the marking tool in hand.
    pub marking: Option<crate::app::marking::Marking>,
    pub mark_arm: Option<crate::app::marking::MarkArm>,
    pub print_layout_shown: bool,
    /// The print layout shown while its task is open.
    pub print_layout: Option<crate::app::print_layout::ShownLayout>,
}

/// Bodies the kernel worker and the tab's threads are working on.
#[derive(Default)]
pub(crate) struct KernelJobs {
    /// Mesh bodies the kernel worker is turning into solids.
    pub solids: std::collections::HashSet<Uuid>,
    /// Mirrored copies whose snapshot the kernel is making.
    pub mirrors: std::collections::HashSet<Uuid>,
    /// Mirrored copies the kernel could not mirror, with the source
    /// snapshot it failed on: not asked again until that changes.
    pub mirrors_failed: std::collections::HashMap<Uuid, std::sync::Arc<Vec<u8>>>,
    /// Linked parts' files being read, and when they were last looked at.
    pub links: crate::app::links::Links,
    /// Bodies whose repair the kernel worker is running.
    pub repairs: std::collections::HashSet<Uuid>,
    /// Bodies whose refine the kernel worker is running.
    pub refines: std::collections::HashSet<Uuid>,
    /// Bodies whose refine failed or was cancelled this session: not asked
    /// again until the document is opened afresh.
    pub refines_failed: std::collections::HashSet<Uuid>,
    /// Bodies whose new shape is being read, and the asset each failed to
    /// read from, which is not tried again.
    pub shapes: std::collections::HashSet<Uuid>,
    pub shapes_failed: std::collections::HashMap<Uuid, Uuid>,
    /// Mesh bodies' histories, built on threads.
    pub mesh_histories: crate::app::mesh_histories::MeshHistories,
}

/// Bodies' builds out on the kernel thread and the previews they carry.
#[derive(Default)]
pub(crate) struct Builds {
    /// The feature an open task edits, which builds carry a preview of.
    pub preview_feature: Option<core_document::FeatureId>,
    /// Bodies showing that preview: the body without the feature stands in
    /// the document, and this keeps the whole solid to put back and the
    /// feature's tool drawn over it.
    pub previews: std::collections::HashMap<core_document::BodyId, BodyPreview>,
    /// Bodies with a build on the kernel thread, and what waits for each.
    pub out: crate::app::recompute::BuildsOut<crate::app::recompute::QueuedBuild>,
    /// Bodies whose history emptied while a build was out on the kernel
    /// thread: what lands is left unused.
    pub stale: std::collections::HashSet<Uuid>,
    /// The serial of each body's build out on the kernel thread.
    pub serials: std::collections::HashMap<Uuid, u64>,
    /// How long each body's last build took on the kernel thread.
    pub times: std::collections::HashMap<Uuid, std::time::Duration>,
    /// Bodies whose build out was dropped for a newer plan: its
    /// cancellation lands quietly.
    pub dropped: std::collections::HashSet<Uuid>,
    /// Bodies being changed faster than they build: when a plan last
    /// replaced one still building.
    pub moving: std::collections::HashMap<Uuid, std::time::Instant>,
    /// Bodies shown meshed coarse while moving, with the plan to build
    /// again at full detail once they settle.
    pub coarse: std::collections::HashMap<Uuid, crate::app::recompute::QueuedBuild>,
    /// Bodies whose open task's feature is built alone first, with the
    /// whole history to build once the edits settle.
    pub preview_rest: std::collections::HashMap<Uuid, crate::app::recompute::QueuedBuild>,
    /// Bodies whose build out stops at the edited feature.
    pub partial_out: std::collections::HashSet<Uuid>,
}

/// What the view drew last and the meshes made for drawing.
#[derive(Default)]
pub(crate) struct Drawn {
    /// The bodies last drawn with their document mesh's triangles, in its
    /// order: by body, the revision it was submitted with and its
    /// geometry's revision. A textured body is drawn with its triangles
    /// split, and is not here.
    pub meshes: std::collections::HashMap<Uuid, (u64, u64)>,
    /// The overlay submissions drawn for a body (selection and hover
    /// paints), by submission id and body: each draws in its body's pane.
    pub overlay_bodies: Vec<(Uuid, Uuid)>,
    /// The scalar maps the view drew last: pictures (the `view` tool,
    /// `doc.picture`, reports) paint them too.
    pub shown_maps: Vec<core_document::ScalarMap>,
    /// Whether each body's drawn mesh closes, at the geometry revision it
    /// was read at: an open one (a scan) shows its inside as a shell under
    /// the clipping plane, a closed one a cap.
    pub closed: std::collections::HashMap<core_document::BodyId, (u64, bool)>,
    /// Each body with faces coloured on their own, as drawn: the mesh
    /// with the colours in it, and the key of what it was made from.
    pub face_colored: std::collections::HashMap<
        core_document::BodyId,
        (u64, std::sync::Arc<kernel_api::TriMesh>),
    >,
    /// Mesh bodies drawn reshaded (`Body::shading`), by what they were made
    /// from.
    pub shaded: std::collections::HashMap<
        core_document::BodyId,
        (u64, std::sync::Arc<kernel_api::TriMesh>),
    >,
    /// The meshes linked copies share with their source, each under the
    /// key the renderer holds it by: by the mesh's address, kept from
    /// reuse by the weak reference while the entry lasts.
    pub shared_meshes:
        std::collections::HashMap<usize, (std::sync::Weak<kernel_api::TriMesh>, uuid::Uuid)>,
    /// The Print layout task is open: the scene shows the layout.
    /// Each textured body as drawn: its textures pressed into its own
    /// mesh, made on a thread of its own.
    pub textured: std::collections::HashMap<core_document::BodyId, crate::app::textures::Preview>,
    /// The pressed meshes those previews share, made and being made.
    pub texture_meshes: crate::app::textures::PressedMeshes,
    /// Pictures textures use, read once each.
    pub texture_pictures: crate::app::textures::Pictures,
    /// Whether each shown note's point is hidden by its own body, kept
    /// with what it was worked out from.
    pub note_occlusions: crate::app::note_tool::Occlusions,
    /// Bounds of the last measured body mesh, keyed by body and revision.
    pub dimension_cache: Option<DimensionCache>,
    /// Each measured body's volume, area and centre, with the geometry
    /// revision it was measured at.
    pub physical: std::collections::HashMap<Uuid, (u64, crate::ui::Physical)>,
}
impl ViewState {
    /// The view from `camera`, with no clipping plane, split or grid.
    pub fn new(camera: CameraController) -> Self {
        Self {
            camera,
            section: None,
            section_pick: None,
            split: false,
            side_camera: None,
            side_rect: None,
            grid_plane: Default::default(),
            fit_on_layout: false,
            pick_filter: Default::default(),
        }
    }
}

impl Hover {
    /// Let go of the hovers on `body`'s faces and edges: its shape was
    /// replaced, so what they name is gone.
    pub fn drop_shape(&mut self, body: core_document::BodyId) {
        if self.edge.as_ref().is_some_and(|e| e.body == body.0) {
            self.edge = None;
        }
        self.face = None;
        self.face_ref = None;
    }

    /// Let go of the pointer's hold on a body that went.
    pub fn forget_body(&mut self, body: core_document::BodyId) {
        if self.body == Some(body.0) {
            self.body = None;
            self.world_pos = None;
            self.triangle = None;
        }
    }
}

impl Selection {
    /// The features Ctrl-clicked into the tree's selection beside the
    /// selected row.
    pub fn tree_also(&self) -> &[FeatureId] {
        match &self.tree_also {
            Some((with, also)) if self.tree == Some(*with) => also,
            _ => &[],
        }
    }

    /// Every feature of the tree's selection, the selected row first, when
    /// there are several (Ctrl-clicked, or selected together by a bench);
    /// empty while one row or none is.
    pub fn features(&self) -> Vec<FeatureId> {
        match self.tree {
            Some(TreeItemId::Feature(primary)) if !self.tree_also().is_empty() => {
                std::iter::once(primary)
                    .chain(self.tree_also().iter().copied())
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The last face picked, while it is on the selected body.
    pub fn face(&self) -> Option<core_document::FaceRef> {
        self.last_face_hit
            .filter(|(body, _)| self.body == Some(*body))
            .map(|(_, face)| face)
    }

    /// The selected edges as the benches see them.
    pub fn edge_refs(&self) -> Vec<core_document::EdgeRef> {
        self.edges
            .iter()
            .map(crate::app::edges::EdgeHit::as_ref)
            .collect()
    }

    /// The view's picks in words when there are several ("3 faces, 2
    /// edges"); `None` for one pick or none.
    pub fn summary(&self) -> Option<String> {
        let faces = self.face_refs().len();
        let edges = self.edges.len();
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

    /// The faces picked on the selected body, the last picked last.
    pub fn face_refs(&self) -> Vec<core_document::FaceRef> {
        let Some((body, last)) = self
            .last_face_hit
            .filter(|(body, _)| self.body == Some(*body))
        else {
            return Vec::new();
        };
        self.earlier_faces
            .iter()
            .filter(|p| p.highlight.body == body)
            .map(|p| p.face)
            .chain(std::iter::once(last))
            .collect()
    }

    /// The parts picked on their own of `feature`, the selected feature;
    /// empty when the whole feature is the selection.
    pub fn parts_of(&self, feature: Option<FeatureId>) -> Vec<core_document::FeaturePart> {
        match &self.parts {
            Some((of, parts)) if feature == Some(*of) => parts.clone(),
            _ => Vec::new(),
        }
    }

    /// Let go of the picks on `body`'s faces and edges: its shape was
    /// replaced, so what they name is gone. A body that held them is no
    /// longer selected: it was not picked whole.
    pub fn drop_shape_picks(&mut self, body: core_document::BodyId) {
        let held = self.holds_picks_on(body);
        if self
            .face_highlight
            .as_ref()
            .is_some_and(|f| f.body == body.0)
        {
            self.face_highlight = None;
        }
        if self.last_face_hit.is_some_and(|(b, _)| b == body.0) {
            self.last_face_hit = None;
            self.earlier_faces.clear();
        }
        self.earlier_faces.retain(|p| p.highlight.body != body.0);
        self.edges.retain(|e| e.body != body.0);
        if held && self.body == Some(body.0) {
            self.body = None;
        }
    }

    /// Find the picks on `body` again on `mesh`, its rebuilt mesh: a face
    /// by the name it was picked under, an edge by the two faces it parts
    /// ([`edge_again`](crate::app::edges::edge_again)). A pick the new
    /// shape lacks goes, and once none is left on a body that held picks,
    /// nothing is selected there. A body picked whole stays so.
    pub fn follow_shape(&mut self, body: core_document::BodyId, mesh: &kernel_api::TriMesh) {
        use crate::app::face_pick::{FaceHighlight, face_id_at, face_submesh_by_id};
        let held = self.holds_picks_on(body);
        let again = |face: &core_document::FaceRef, was: &FaceHighlight| {
            let named =
                |id: &u32| face.name != 0 && mesh.face_names.get(*id as usize) == Some(&face.name);
            let id = face_id_at(mesh, face.point)
                .filter(named)
                .or_else(|| (0..mesh.face_names.len() as u32).find(named))?;
            Some(FaceHighlight {
                body: was.body,
                face: Some(id),
                mesh: std::sync::Arc::new(face_submesh_by_id(mesh, id)?),
                revision: was.revision.wrapping_add(1),
            })
        };
        let last = self.last_face_hit.filter(|(b, _)| *b == body.0);
        match (
            last,
            self.face_highlight.as_ref().filter(|f| f.body == body.0),
        ) {
            (Some((_, face)), Some(was)) => match again(&face, was) {
                Some(found) => {
                    self.face_highlight = Some(found);
                    self.earlier_faces.retain_mut(|picked| {
                        if picked.highlight.body != body.0 {
                            return true;
                        }
                        again(&picked.face, &picked.highlight)
                            .map(|found| picked.highlight = found)
                            .is_some()
                    });
                }
                None => self.drop_face_picks(body),
            },
            // A body picked whole keeps no face to find.
            _ => self.drop_face_picks(body),
        }
        self.edges = std::mem::take(&mut self.edges)
            .into_iter()
            .filter_map(|hit| match hit.body == body.0 {
                true => crate::app::edges::edge_again(mesh, &hit),
                false => Some(hit),
            })
            .collect();
        if held && !self.holds_picks_on(body) && self.body == Some(body.0) {
            self.body = None;
        }
    }

    fn drop_face_picks(&mut self, body: core_document::BodyId) {
        if self.last_face_hit.is_some_and(|(b, _)| b == body.0) {
            self.last_face_hit = None;
        }
        if self
            .face_highlight
            .as_ref()
            .is_some_and(|f| f.body == body.0)
        {
            self.face_highlight = None;
        }
        self.earlier_faces.retain(|p| p.highlight.body != body.0);
    }

    /// Whether a face or an edge of `body` is picked: the body is then
    /// selected for them, not whole.
    fn holds_picks_on(&self, body: core_document::BodyId) -> bool {
        self.face_highlight
            .as_ref()
            .is_some_and(|f| f.body == body.0)
            || self
                .earlier_faces
                .iter()
                .any(|p| p.highlight.body == body.0)
            || self.edges.iter().any(|e| e.body == body.0)
    }

    /// The body picked whole, which the selection paint covers: selected
    /// with no face or edge of it picked.
    pub fn whole_body(&self) -> Option<Uuid> {
        self.body
            .filter(|_| self.face_highlight.is_none() && self.edges.is_empty())
    }

    /// An accepted task used the faces, edges and parts picked for it:
    /// they go, and the body with them unless it was picked whole.
    pub fn let_go_of_used_picks(&mut self) {
        if self.body.is_some_and(|b| self.holds_picks_on(BodyId(b))) {
            self.body = None;
        }
        self.last_face_hit = None;
        self.face_highlight = None;
        self.earlier_faces.clear();
        self.edges.clear();
        self.parts = None;
        self.last_select_click = None;
    }

    /// Take `body` out of the selection, with every pick on it.
    pub fn deselect_body(&mut self, body: core_document::BodyId) {
        self.drop_shape_picks(body);
        if self.body == Some(body.0) {
            self.body = None;
        }
        self.earlier_bodies.retain(|b| *b != body.0);
    }
}

impl FileState {
    /// No open, save or import of this tab is on its way or waiting for
    /// an answer.
    pub fn idle(&self) -> bool {
        self.open_rx.is_none()
            && self.save_rx.is_none()
            && !self.asking()
            && self.imports_waiting.is_empty()
    }

    /// The import dialog or the units question is open.
    pub fn asking(&self) -> bool {
        self.step_import_pending.is_some() || self.mesh_units_pending.is_some()
    }

    /// `path` to import now, or `None` when it joins the files waiting
    /// behind an open question, so files picked together import in order.
    pub fn import_in_turn(&mut self, path: PathBuf) -> Option<PathBuf> {
        if self.asking() || !self.imports_waiting.is_empty() {
            self.imports_waiting.push_back(path);
            return None;
        }
        Some(path)
    }

    /// The next waiting file, once no question is open.
    pub fn next_waiting_import(&mut self) -> Option<PathBuf> {
        if self.asking() {
            return None;
        }
        self.imports_waiting.pop_front()
    }
}

impl Connection {
    /// The connection `server` on `socket`, with nothing relayed yet.
    pub fn new(server: Box<dyn core_document::server::DocumentServer>, socket: PathBuf) -> Self {
        Self {
            server,
            socket,
            last_reconnect: None,
            remote_import_routes: HashMap::new(),
            peer_presence: HashMap::new(),
            held_remote_ops: Vec::new(),
            last_sent_presence: None,
            load_epoch: 0,
        }
    }
}

impl Builds {
    /// Bodies still wait for a build at full detail or of the whole
    /// history.
    pub fn settling(&self) -> bool {
        !self.coarse.is_empty() || !self.preview_rest.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files dropped together while each asks a question import one by
    /// one, in the order given, none of them lost behind the next.
    #[test]
    fn files_picked_together_wait_their_turn() {
        let mut file = FileState::default();
        let [a, b, c] = ["a.step", "b.stl", "c.obj"].map(PathBuf::from);
        assert_eq!(file.import_in_turn(a.clone()), Some(a.clone()));
        file.step_import_pending = Some((a, Default::default()));
        assert_eq!(file.import_in_turn(b.clone()), None);
        assert_eq!(file.import_in_turn(c.clone()), None);
        assert_eq!(file.next_waiting_import(), None);
        file.step_import_pending = None;
        assert!(!file.idle());
        assert_eq!(file.next_waiting_import(), Some(b));
        assert_eq!(file.next_waiting_import(), Some(c));
        assert!(file.idle());
    }
}
