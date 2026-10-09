//! One open document and everything the app keeps about it: a tab. The
//! active session sits on the app inline; the other tabs are parked and
//! swapped in when they need a turn (a drain, a rebuild, a switch).

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use web_time::Instant;

use core_document::{BodyId, Document, FeatureId};
use uuid::Uuid;

use crate::app::doc_io::{OpenJob, SaveJob, SaveProgress};
use crate::camera::CameraController;
use crate::ui::{ActiveTool, ActiveWorkbench, Screen, TreeItemId};
use crate::{DimensionCache, RemoteImportRoute};

pub(crate) struct DocumentSession {
    /// The tab's own identity; the document's id changes when a file is
    /// opened into it, this does not.
    pub tab: Uuid,
    pub document: Document,
    pub camera: CameraController,
    /// The export dialog's draft, while it is open.
    pub export_pending: Option<crate::app::export::ExportDraft>,
    /// The exporting walkthrough is waiting on a solid to open the export
    /// dialog over.
    pub export_when_ready: bool,
    /// The view toolbar's clipping plane, when on.
    pub section: Option<crate::camera::section::SectionPlane>,
    /// What a click in the view picks.
    pub pick_filter: crate::ui::PickFilter,
    /// The last click on an edge, to tell a double click: when, the body
    /// and the edge.
    pub last_edge_click: Option<(web_time::Instant, Uuid, u32)>,
    /// The tree row under the pointer: what it stands for lights up.
    pub tree_hovered: Option<crate::ui::TreeItemId>,
    pub active_tool: ActiveTool,
    pub selected_body: Option<Uuid>,
    pub hovered_body: Option<Uuid>,
    pub hovered_world_pos: Option<[f32; 3]>,
    /// The origin plane under the cursor, while the active bench shows them
    /// to be picked from (`Workbench::shows_origin_planes`).
    pub hovered_base_plane: Option<core_document::BasePlane>,
    /// Currently active workbench (determines which tools are visible).
    pub active_workbench: ActiveWorkbench,
    /// Active document object (selected feature in tree - separate from
    /// editing mode).
    pub active_document_object: Option<FeatureId>,
    pub active_body_id: Option<BodyId>,
    pub tree_selection: Option<TreeItemId>,
    pub current_file: Option<PathBuf>,
    /// Start page or workspace: a fresh tab shows the start page.
    pub screen: Screen,
    /// The worker packing a `.prtcad` archive, and how far it has got.
    /// Packing carries every blob in the document, so it happens off the
    /// UI thread and the window keeps drawing while it runs.
    pub document_save_rx: Option<std::sync::mpsc::Receiver<SaveJob>>,
    /// The worker parsing an opened document, for the same reason.
    pub document_open_rx: Option<std::sync::mpsc::Receiver<OpenJob>>,
    pub save_progress: Option<std::sync::Arc<SaveProgress>>,
    /// The document server connection: the local daemon by default, direct
    /// files when no daemon can run. Everything that crosses it is the wire
    /// protocol; `document_load_epoch` rides
    /// Open requests as the token that invalidates late responses.
    pub server: Box<dyn core_document::server::DocumentServer>,
    /// The socket the server connection is (or should be) on: the
    /// document's own once it has a file, the tab's own for Untitled.
    /// Reconnects and connection switches aim here.
    pub server_socket: PathBuf,
    /// Last reconnect attempt, so a dead daemon is retried at a gentle pace
    /// instead of every frame.
    pub last_server_reconnect: Option<Instant>,
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
    pub document_load_epoch: u64,
    /// Picked STEP path and draft tessellation settings until the user
    /// confirms import.
    pub step_import_pending: Option<(PathBuf, kernel_api::TessellationSettings)>,
    /// Undo/redo. `note()` is called once per frame while no mouse button
    /// is held, so drags coalesce into single steps.
    pub journal: core_document::history::OpJournal,
    /// A workbench asked to create a sketch on this body; carried between
    /// hooks until the bench that edits sketches consumes it (plane picker).
    pub pending_sketch_creation: Option<core_document::SketchAttachRequest>,
    /// A bench's edit session on a plane was open last frame.
    pub plane_session_open: bool,
    /// Face under the most recent body selection click (surface point +
    /// normal derived from the picked mesh triangle).
    pub last_face_hit: Option<(Uuid, core_document::FaceRef)>,
    /// Extracted sub-mesh of the selected face, rendered as a highlight
    /// overlay. Present only while a face (not the whole body) is the
    /// selection.
    pub face_highlight: Option<crate::app::input::FaceHighlight>,
    /// Feature under the cursor (CPU hit-test, drives hover tint).
    pub hovered_feature: Option<FeatureId>,
    /// Timestamp + target of the last selection click (double-click detect).
    pub last_select_click: Option<(Instant, Uuid)>,
    /// A body double-clicked in the viewport, for the one frame it takes
    /// the tree to jump to its row.
    pub reveal_body: Option<BodyId>,
    /// The context menu a right click on a body asked for, until it is
    /// used or dismissed.
    pub viewport_menu: Option<crate::ui::ViewportMenu>,
    /// Workbench to return to when an edit session finishes, when the flow
    /// was started from another workbench.
    pub return_workbench: Option<ActiveWorkbench>,
    /// The last tool started, and its bench: what Repeat starts again.
    pub last_tool: Option<(crate::WorkbenchId, String)>,
    /// A workbench task is open in the right panel; its edits form one undo
    /// entry until it closes.
    pub task_open: bool,
    /// Bounds of the last measured body mesh, keyed by body and revision.
    pub dimension_cache: Option<DimensionCache>,
    /// The measure tool: armed with the points, edges or faces picked so
    /// far (up to two).
    pub measure: Option<Vec<crate::app::measure::MeasurePick>>,
    /// The edge under the cursor, found on the CPU against the hovered
    /// body's outline segments.
    pub hovered_edge: Option<crate::app::edges::EdgeHit>,
    /// Each measured body's volume, area and centre, with the geometry
    /// revision it was measured at.
    pub physical: std::collections::HashMap<Uuid, (u64, crate::ui::Physical)>,
    /// Mesh bodies the kernel worker is turning into solids.
    pub solids_in_flight: std::collections::HashSet<Uuid>,
    /// Mirrored copies whose snapshot the kernel is making.
    pub mirrors_in_flight: std::collections::HashSet<Uuid>,
    /// Mirrored copies the kernel could not mirror, with the source
    /// snapshot it failed on: not asked again until that changes.
    pub mirrors_failed: std::collections::HashMap<Uuid, std::sync::Arc<Vec<u8>>>,
    /// Linked parts' files being read, and when they were last looked at.
    pub links: crate::app::links::Links,
    /// The feature an open task edits, which builds carry a preview of.
    pub preview_feature: Option<core_document::FeatureId>,
    /// Bodies showing that preview: the body without the feature stands in
    /// the document, and this keeps the whole solid to put back and the
    /// feature's tool drawn over it.
    pub previews: std::collections::HashMap<core_document::BodyId, BodyPreview>,
    /// The view frames the opened document once the workspace's viewport
    /// is laid out and its bodies have geometry, which a file's meshes
    /// reach after the open; a press or a wheel turn in the view first
    /// leaves the view as the user puts it.
    pub fit_on_layout: bool,
    /// Bodies whose repair the kernel worker is running.
    pub repairs_in_flight: std::collections::HashSet<Uuid>,
    /// Bodies whose refine the kernel worker is running.
    pub refines_in_flight: std::collections::HashSet<Uuid>,
    /// Bodies whose refine failed or was cancelled this session: not asked
    /// again until the document is opened afresh.
    pub refines_failed: std::collections::HashSet<Uuid>,
    /// Bodies with a build on the kernel thread, each with the newest plan
    /// made for it since, which goes when that build lands: while a drag
    /// changes a feature every frame, only the latest shape is built.
    pub builds_in_flight:
        std::collections::HashMap<Uuid, Option<crate::app::recompute::QueuedBuild>>,
    /// Bodies whose history emptied while a build was out on the kernel
    /// thread: what lands is left unused.
    pub stale_builds: std::collections::HashSet<Uuid>,
    /// The serial of each body's build out on the kernel thread.
    pub build_serials: std::collections::HashMap<Uuid, u64>,
    /// How long each body's last build took on the kernel thread.
    pub build_times: std::collections::HashMap<Uuid, std::time::Duration>,
    /// Bodies whose build out was dropped for a newer plan: its
    /// cancellation lands quietly.
    pub dropped_builds: std::collections::HashSet<Uuid>,
    /// Bodies being changed faster than they build: when a plan last
    /// replaced one still building.
    pub moving: std::collections::HashMap<Uuid, web_time::Instant>,
    /// Bodies shown meshed coarse while moving, with the plan to build
    /// again at full detail once they settle.
    pub coarse: std::collections::HashMap<Uuid, crate::app::recompute::QueuedBuild>,
    /// Bodies whose open task's feature is built alone first, with the
    /// whole history to build once the edits settle.
    pub preview_rest: std::collections::HashMap<Uuid, crate::app::recompute::QueuedBuild>,
    /// Bodies whose build out stops at the edited feature.
    pub partial_out: std::collections::HashSet<Uuid>,
    /// Bodies whose new shape is being read, and the asset each failed to
    /// read from, which is not tried again.
    pub shapes_in_flight: std::collections::HashSet<Uuid>,
    pub shapes_failed: std::collections::HashMap<Uuid, Uuid>,
    /// The depths the pick pass drew around the cursor, and the camera it
    /// drew them with: which edges near the cursor are in view.
    pub pick_depths: Option<render_wgpu::DepthWindow>,
    /// The face under the cursor, when no edge takes the hover.
    pub hovered_face: Option<crate::app::input::FaceHover>,
    /// Each body with faces coloured on their own, as drawn: the mesh
    /// with the colours in it, and the key of what it was made from.
    pub face_colored: std::collections::HashMap<
        core_document::BodyId,
        (u64, std::sync::Arc<kernel_api::TriMesh>),
    >,
    /// The Print layout task is open: the scene shows the layout.
    pub print_layout_shown: bool,
    /// The print layout shown while its task is open.
    pub print_layout: Option<crate::app::print_layout::ShownLayout>,
    /// Each textured body as drawn: its textures pressed into its mesh,
    /// made on a thread of its own.
    pub textured: std::collections::HashMap<core_document::BodyId, crate::app::textures::Preview>,
    /// Finished texture previews, from their threads.
    pub textured_rx: Option<std::sync::mpsc::Receiver<crate::app::textures::Made>>,
    pub textured_tx: Option<std::sync::mpsc::Sender<crate::app::textures::Made>>,
    /// Pictures textures use, read once each.
    pub texture_pictures: crate::app::textures::Pictures,
    /// The edges picked in the viewport; Ctrl adds to them.
    pub selected_edges: Vec<crate::app::edges::EdgeHit>,
    /// Faces Ctrl added before the last one picked (`last_face_hit`), on
    /// its body, in the order picked. Empty whenever no face is picked.
    pub earlier_faces: Vec<crate::app::input::PickedFace>,
    /// Each bench's editing state for this tab while another tab is
    /// active, keyed by bench id; handed back to the benches on switch.
    pub bench_states: HashMap<String, Box<dyn Any + Send>>,
}

impl DocumentSession {
    /// A fresh Untitled tab: its own daemon connection, a default camera,
    /// the landing bench.
    pub fn untitled(
        camera_settings: &settings::CameraSettings,
        landing: ActiveWorkbench,
        screen: Screen,
    ) -> Self {
        let tab = Uuid::new_v4();
        // A per-tab local daemon by default, plain in-process file I/O when
        // the daemon cannot start: the same `DocumentServer` contract either
        // way.
        let server_socket = crate::app::server::socket_for_untitled(tab);
        let server: Box<dyn core_document::server::DocumentServer> =
            match crate::app::server::connect(&server_socket) {
                Ok(server) => server,
                Err(err) => {
                    tracing::warn!("document daemon unavailable ({err}); using direct file I/O");
                    crate::app::server::fallback()
                }
            };
        tracing::info!(server = server.name(), "document server connected");
        Self {
            tab,
            document: Document::new("Untitled"),
            camera: CameraController::new(camera_settings, (1, 1)),
            section: None,
            pick_filter: Default::default(),
            tree_hovered: None,
            last_edge_click: None,
            export_pending: None,
            export_when_ready: false,
            active_tool: ActiveTool::default(),
            selected_body: None,
            hovered_body: None,
            hovered_world_pos: None,
            hovered_base_plane: None,
            active_workbench: landing,
            active_document_object: None,
            active_body_id: None,
            tree_selection: Some(TreeItemId::DocumentRoot),
            current_file: None,
            screen,
            document_save_rx: None,
            document_open_rx: None,
            save_progress: None,
            server,
            server_socket,
            last_server_reconnect: None,
            remote_import_routes: HashMap::new(),
            peer_presence: HashMap::new(),
            held_remote_ops: Vec::new(),
            last_sent_presence: None,
            document_load_epoch: 0,
            step_import_pending: None,
            journal: core_document::history::OpJournal::new(64),
            pending_sketch_creation: None,
            plane_session_open: false,
            last_face_hit: None,
            face_highlight: None,
            hovered_feature: None,
            last_select_click: None,
            reveal_body: None,
            viewport_menu: None,
            return_workbench: None,
            last_tool: None,
            task_open: false,
            dimension_cache: None,
            measure: None,
            hovered_edge: None,
            hovered_face: None,
            face_colored: Default::default(),
            print_layout_shown: false,
            print_layout: None,
            textured: Default::default(),
            textured_rx: None,
            textured_tx: None,
            texture_pictures: Default::default(),
            pick_depths: None,
            repairs_in_flight: Default::default(),
            refines_in_flight: Default::default(),
            refines_failed: Default::default(),
            builds_in_flight: Default::default(),
            stale_builds: Default::default(),
            build_serials: Default::default(),
            build_times: Default::default(),
            dropped_builds: Default::default(),
            moving: Default::default(),
            coarse: Default::default(),
            preview_rest: Default::default(),
            partial_out: Default::default(),
            shapes_in_flight: Default::default(),
            shapes_failed: Default::default(),
            solids_in_flight: Default::default(),
            mirrors_in_flight: Default::default(),
            mirrors_failed: Default::default(),
            links: Default::default(),
            preview_feature: None,
            previews: Default::default(),
            fit_on_layout: false,
            physical: Default::default(),
            selected_edges: Vec::new(),
            earlier_faces: Vec::new(),
            bench_states: HashMap::new(),
        }
    }

    /// Nothing has happened in this tab: no file, no edits, no bodies,
    /// nothing on its way. New and Open reuse such a tab instead of
    /// opening another.
    pub fn is_blank(&self) -> bool {
        self.current_file.is_none()
            && !self.document.metadata().dirty()
            && !self.document.has_bodies()
            && self.document.feature_tree().all_nodes().next().is_none()
            && self.document_open_rx.is_none()
            && self.document_save_rx.is_none()
            && self.step_import_pending.is_none()
            && self.server.status().opens_in_flight == 0
    }

    /// Whether this tab still has work on its way through a worker or the
    /// server.
    pub fn busy(&self) -> bool {
        self.server.status().busy()
            || self.document_save_rx.is_some()
            || self.document_open_rx.is_some()
            || self.step_import_pending.is_some()
            || self.links.busy()
            || !self.coarse.is_empty()
            || !self.preview_rest.is_empty()
    }
}

/// A tab's place in the strip. The active tab's session lives on the app
/// itself; every other tab is parked here.
pub(crate) struct TabSlot {
    pub tab: Uuid,
    pub parked: Option<DocumentSession>,
}

/// A body showing the preview of the feature being edited.
pub struct BodyPreview {
    /// The whole solid, stored back when the preview ends.
    pub full: kernel_api::SolidBuildResult,
    /// `full` is the body's whole history; otherwise it stops at the
    /// edited feature, and the body is built again when the preview ends.
    pub complete: bool,
    /// The feature's tool, placed where the body sits.
    pub tool: std::sync::Arc<kernel_api::TriMesh>,
    /// The tool's id and revision for the renderer.
    pub id: Uuid,
    pub revision: u64,
}
