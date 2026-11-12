//! One open document and everything the app keeps about it: a tab. The
//! active session sits on the app inline; the other tabs are parked and
//! swapped in when they need a turn (a drain, a rebuild, a switch).

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;

use core_document::{BodyId, Document, FeatureId};
use uuid::Uuid;

pub(crate) use crate::app::session_state::{
    Builds, Connection, Drawn, FileState, HostTools, Hover, KernelJobs, Selection, ViewState,
};
use crate::camera::CameraController;
use crate::ui::{ActiveTool, ActiveWorkbench, Screen, TreeItemId};

pub(crate) struct DocumentSession {
    /// The tab's own identity; the document's id changes when a file is
    /// opened into it, this does not.
    pub tab: Uuid,
    pub document: Document,
    pub view: ViewState,
    /// What the view holds between a press and its release, host side.
    pub gestures: crate::app::gesture::Gestures,
    pub hover: Hover,
    pub selection: Selection,
    pub active_tool: ActiveTool,
    /// Currently active workbench (determines which tools are visible).
    pub active_workbench: ActiveWorkbench,
    /// Active document object (selected feature in tree - separate from
    /// editing mode).
    pub active_document_object: Option<FeatureId>,
    pub active_body_id: Option<BodyId>,
    /// Start page or workspace: a fresh tab shows the start page.
    pub screen: Screen,
    pub file: FileState,
    pub connection: Connection,
    pub tools: HostTools,
    /// Undo/redo. `note()` is called once per frame while no mouse button
    /// is held, so drags coalesce into single steps.
    pub journal: core_document::history::OpJournal,
    /// A workbench asked to create a sketch on this body; carried between
    /// hooks until the bench that edits sketches consumes it (plane picker).
    pub pending_sketch_creation: Option<core_document::SketchAttachRequest>,
    /// A bench's edit session on a plane was open last frame.
    pub plane_session_open: bool,
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
    /// The active bench's floating card, while one is on screen; its
    /// edits form one undo entry until it closes.
    pub card: Option<crate::app::cards::OpenCard>,
    pub jobs: KernelJobs,
    pub builds: Builds,
    pub drawn: Drawn,
    /// Each bench's editing state for this tab while another tab is
    /// active, keyed by bench id; handed back to the benches on switch.
    pub bench_states: HashMap<String, Box<dyn Any + Send>>,
}

impl DocumentSession {
    /// Hand the document's outbox to the server: the edits recorded since
    /// the last send, in order. Every way a tab's document or connection
    /// ends calls it, so a held gesture's ops are never left behind.
    pub(crate) fn send_outbox(&mut self) {
        let ops = self.document.take_pending_ops();
        if !ops.is_empty() {
            self.connection
                .server
                .send(core_document::server::ClientMessage::Ops(ops));
        }
    }

    /// The end of a frame's send. While a pointer gesture is under way on
    /// this tab the outbox stays, where consecutive whole-payload writes
    /// collapse, so a drag reaches the server and its peers as one op when
    /// the button is let go, the boundary that closes its undo step.
    pub(crate) fn send_outbox_after_frame(&mut self, gesture_held: bool) {
        if !gesture_held {
            self.send_outbox();
        }
    }

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
        Self::with_server(camera_settings, landing, screen, tab, server_socket, server)
    }

    /// A fresh Untitled tab `tab` talking to `server`.
    fn with_server(
        camera_settings: &settings::CameraSettings,
        landing: ActiveWorkbench,
        screen: Screen,
        tab: Uuid,
        server_socket: PathBuf,
        server: Box<dyn core_document::server::DocumentServer>,
    ) -> Self {
        Self {
            tab,
            document: Document::new("Untitled"),
            view: ViewState::new(CameraController::new(camera_settings, (1, 1))),
            gestures: Default::default(),
            hover: Default::default(),
            selection: Selection {
                tree: Some(TreeItemId::DocumentRoot),
                ..Default::default()
            },
            active_tool: ActiveTool::default(),
            active_workbench: landing,
            active_document_object: None,
            active_body_id: None,
            screen,
            file: Default::default(),
            connection: Connection::new(server, server_socket),
            tools: Default::default(),
            journal: core_document::history::OpJournal::new(64),
            pending_sketch_creation: None,
            plane_session_open: false,
            viewport_menu: None,
            return_workbench: None,
            last_tool: None,
            task_open: false,
            card: None,
            jobs: Default::default(),
            builds: Default::default(),
            drawn: Default::default(),
            bench_states: HashMap::new(),
        }
    }

    /// Nothing has happened in this tab: no file, no edits, no bodies,
    /// nothing on its way. New and Open reuse such a tab instead of
    /// opening another.
    pub fn is_blank(&self) -> bool {
        self.file.path.is_none()
            && !self.document.metadata().dirty()
            && !self.document.has_bodies()
            && self.document.feature_tree().all_nodes().next().is_none()
            && self.file.idle()
            && self.connection.server.status().opens_in_flight == 0
    }

    /// Let go of the picks and hovers on `body`'s faces and edges: its
    /// shape was replaced, so what they name is gone.
    pub fn shape_replaced(&mut self, body: BodyId) {
        self.selection.drop_shape_picks(body);
        self.hover.drop_shape(body);
    }

    /// Take `body` out of the selection, with every pick on it: it is
    /// hidden or no longer selectable, so not what the next click acts on.
    pub fn deselect_body(&mut self, body: BodyId) {
        self.hover.drop_shape(body);
        self.selection.deselect_body(body);
    }

    /// Let go of everything the tab holds on a body that went.
    pub fn forget_body(&mut self, body: BodyId) {
        self.deselect_body(body);
        if self.active_body_id == Some(body) {
            self.active_body_id = None;
        }
        self.hover.forget_body(body);
    }

    /// The parts of the selected feature picked on their own; empty when
    /// the whole feature is the selection.
    pub fn selected_parts(&self) -> Vec<core_document::FeaturePart> {
        self.selection.parts_of(self.active_document_object)
    }

    /// Whether this tab still has work on its way through a worker or the
    /// server.
    pub fn busy(&self) -> bool {
        self.connection.server.status().busy()
            || !self.file.idle()
            || self.jobs.links.busy()
            || self.builds.settling()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session_on(server: Box<dyn core_document::server::DocumentServer>) -> DocumentSession {
        DocumentSession::with_server(
            &settings::CameraSettings::default(),
            ActiveWorkbench(core_document::WorkbenchId::new("wb.any")),
            Screen::Start,
            Uuid::new_v4(),
            PathBuf::new(),
            server,
        )
    }

    /// A server that keeps every batch of ops it is sent.
    struct Recording(std::sync::Arc<std::sync::Mutex<Vec<Vec<core_document::op::DocumentOp>>>>);

    impl core_document::server::DocumentServer for Recording {
        fn name(&self) -> &str {
            "recording"
        }
        fn send(&mut self, msg: core_document::server::ClientMessage) {
            if let core_document::server::ClientMessage::Ops(ops) = msg {
                self.0.lock().unwrap().push(ops);
            }
        }
        fn poll(&mut self) -> Vec<core_document::server::ServerMessage> {
            Vec::new()
        }
        fn status(&self) -> core_document::server::ServerStatus {
            core_document::server::ServerStatus {
                connected: true,
                ..Default::default()
            }
        }
        fn flush(&mut self) {}
    }

    /// A drag's frames reach the server once, as the last payload, when the
    /// button is let go; and a gesture cut off before that (a tab closing,
    /// a save, the app quitting) still sends what it held.
    #[test]
    fn a_held_gesture_reaches_the_server_once_coalesced() {
        use core_document::op::DocumentOp;
        let sent = std::sync::Arc::default();
        let mut session = session_on(Box::new(Recording(std::sync::Arc::clone(&sent))));
        let kind = core_document::WorkbenchId::new("wb.any");
        let id = session.document.add_feature_of_kind(
            kind,
            "sketch".into(),
            None,
            Vec::new(),
            serde_json::json!({ "x": 0 }),
            Default::default(),
        );
        session.journal.note(&mut session.document);
        session.send_outbox_after_frame(false);
        sent.lock().unwrap().clear();
        let drag = |session: &mut DocumentSession, frames: std::ops::RangeInclusive<i32>| {
            for x in frames {
                let data = serde_json::json!({ "x": x });
                session.document.update_feature_data(id, data).unwrap();
                session.send_outbox_after_frame(true);
            }
        };
        let only_payload = |sent: &[Vec<DocumentOp>]| match sent {
            [batch] => match batch.as_slice() {
                [DocumentOp::UpdateFeatureData { data, .. }] => data["x"].clone(),
                other => panic!("one update expected, got {other:?}"),
            },
            other => panic!("one batch expected, got {other:?}"),
        };

        drag(&mut session, 1..=120);
        assert!(sent.lock().unwrap().is_empty(), "nothing leaves mid-drag");
        session.send_outbox_after_frame(false);
        assert_eq!(only_payload(&sent.lock().unwrap()), 120);
        session.send_outbox_after_frame(false);
        assert_eq!(sent.lock().unwrap().len(), 1, "sent exactly once");

        // Its undo is one step back to where it started, sent as an edit.
        session.journal.note(&mut session.document);
        sent.lock().unwrap().clear();
        session.journal.undo(&mut session.document);
        session.send_outbox_after_frame(false);
        assert_eq!(only_payload(&sent.lock().unwrap()), 0);

        sent.lock().unwrap().clear();
        drag(&mut session, 1..=5);
        session.send_outbox();
        assert_eq!(only_payload(&sent.lock().unwrap()), 5);
    }

    /// A body that went takes every pick on it along, faces and edges
    /// alike, and leaves the picks on other bodies.
    #[test]
    fn forgetting_a_body_drops_its_face_and_edge_picks() {
        let mut session = session_on(crate::app::server::fallback());
        let (gone, kept) = (BodyId::new(), BodyId::new());
        let edge = |body: BodyId| crate::app::edges::EdgeHit {
            body: body.0,
            edge: 0,
            point: [0.0; 3],
            direction: [1.0, 0.0, 0.0],
            length_mm: 1.0,
            circle: None,
            faces: Default::default(),
        };
        let face = core_document::FaceRef {
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            surface: None,
            name: Default::default(),
        };
        session.selection.edges = vec![edge(gone), edge(kept)];
        session.hover.edge = Some(edge(gone));
        session.selection.body = Some(gone.0);
        session.selection.last_face_hit = Some((gone.0, face));
        session.forget_body(gone);
        assert_eq!(session.selection.edges, vec![edge(kept)]);
        assert_eq!(session.hover.edge, None);
        assert_eq!(session.selection.body, None);
        assert!(session.selection.last_face_hit.is_none());
    }
}
