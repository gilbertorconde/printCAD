//! One open document and everything the app keeps about it: a tab. The
//! active session sits on the app inline; the other tabs are parked and
//! swapped in when they need a turn (a drain, a rebuild, a switch).

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

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
    pub active_tool: ActiveTool,
    pub selected_body: Option<Uuid>,
    pub hovered_body: Option<Uuid>,
    pub hovered_world_pos: Option<[f32; 3]>,
    /// Currently active workbench (determines which tools are visible).
    pub active_workbench: ActiveWorkbench,
    /// Active document object (selected feature in tree - separate from
    /// editing mode).
    pub active_document_object: Option<FeatureId>,
    pub active_body_id: Option<BodyId>,
    pub tree_selection: Option<TreeItemId>,
    /// Current file on disk (if any).
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
    /// The document server connection — local daemon by default, direct
    /// files when no daemon can run, a remote plugin someday. Everything
    /// that crosses it is the wire protocol; `document_load_epoch` rides
    /// Open requests as the token that invalidates late responses.
    pub server: Box<dyn core_document::server::DocumentServer>,
    /// The socket the server connection is (or should be) on — the
    /// document's own once it has a file, the tab's own for Untitled.
    /// Reconnects and connection switches aim here.
    pub server_socket: PathBuf,
    /// Last reconnect attempt, so a dead daemon is retried at a gentle pace
    /// instead of every frame.
    pub last_server_reconnect: Option<Instant>,
    /// Remote imports being re-derived: a peer's ImportModel op created the
    /// bodies; the kernel re-imports the carried bytes (written to a temp
    /// file) and the resulting meshes are routed to those pre-existing
    /// bodies by import order — deterministic at any thread count.
    pub remote_import_routes: HashMap<PathBuf, RemoteImportRoute>,
    /// What each peer has selected, keyed by actor. Bodies in here render
    /// with the peer tint; entries die with their peer.
    pub peer_presence: HashMap<Uuid, core_document::server::PresenceState>,
    /// Relayed ops that arrived while an open was in flight. The daemon's
    /// threads may interleave a relay before the Opened reply; applying it
    /// to the document the open is about to replace would lose the edit,
    /// so they wait here and apply right after the new document lands.
    pub held_remote_ops: Vec<(Uuid, Vec<core_document::op::DocumentOp>)>,
    /// Last presence we told the server, so only changes cross the wire.
    pub last_sent_presence: Option<core_document::server::PresenceState>,
    pub document_load_epoch: u64,
    /// Picked STEP path and draft tessellation settings until the user
    /// confirms import.
    pub step_import_pending: Option<(PathBuf, kernel_api::TessellationSettings)>,
    /// Undo/redo. `note()` is called once per frame while no mouse button
    /// is held, so drags coalesce into single steps.
    pub journal: core_document::history::OpJournal,
    /// A workbench asked to create a sketch on this body; carried between
    /// hooks until the sketch workbench consumes it (plane picker).
    pub pending_sketch_creation: Option<core_document::SketchAttachRequest>,
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
    /// was started from another workbench (e.g. Part Design).
    pub return_workbench: Option<ActiveWorkbench>,
    /// A workbench task is open in the right panel; its edits form one undo
    /// entry until it closes.
    pub task_open: bool,
    /// Bounds of the last measured body mesh, keyed by body and revision.
    pub dimension_cache: Option<DimensionCache>,
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
        // The document server: a per-tab local daemon by default; plain
        // in-process file I/O when the daemon cannot start. Same contract
        // either way — the trait is the seam a remote plugin replaces.
        let server_socket = doc_server::socket_path_for_untitled(tab);
        let server: Box<dyn core_document::server::DocumentServer> =
            match doc_server::DaemonClient::spawn_or_connect(&server_socket) {
                Ok(client) => Box::new(client),
                Err(err) => {
                    tracing::warn!("document daemon unavailable ({err}); using direct file I/O");
                    Box::new(doc_server::DirectFiles::new())
                }
            };
        tracing::info!(server = server.name(), "document server connected");
        Self {
            tab,
            document: Document::new("Untitled"),
            camera: CameraController::new(camera_settings, (1, 1)),
            active_tool: ActiveTool::default(),
            selected_body: None,
            hovered_body: None,
            hovered_world_pos: None,
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
            last_face_hit: None,
            face_highlight: None,
            hovered_feature: None,
            last_select_click: None,
            reveal_body: None,
            viewport_menu: None,
            return_workbench: None,
            task_open: false,
            dimension_cache: None,
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
    }
}

/// A tab's place in the strip. The active tab's session lives on the app
/// itself; every other tab is parked here.
pub(crate) struct TabSlot {
    pub tab: Uuid,
    pub parked: Option<DocumentSession>,
}
