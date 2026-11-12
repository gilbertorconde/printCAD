//! One open document and everything the app keeps about it: a tab. The
//! active session sits on the app inline; the other tabs are parked and
//! swapped in when they need a turn (a drain, a rebuild, a switch).

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;

use core_document::{BodyId, Document, FeatureId};
use uuid::Uuid;

pub(crate) use crate::app::session_state::{
    Builds, Connection, Drawn, FileState, HostTools, Hover, KeptSelection, KernelJobs, Selection,
    ViewState,
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

    /// `body` was built again: the picks on it are found on its new mesh
    /// (`Selection::follow_shape`), the pointer's hovers on it let go.
    pub fn shape_rebuilt(&mut self, body: BodyId) {
        match self.document.imported_geometry(body) {
            Some(geometry) => self.selection.follow_shape(body, &geometry.mesh),
            None => self.selection.drop_shape_picks(body),
        }
        self.hover.drop_shape(body);
    }

    /// Keep the selection as a task opens, for its Cancel to put back. A
    /// tool that opens one has made its feature the active object by then,
    /// but not the tree's row: the active object kept is the row's.
    pub fn keep_selection_for_task(&mut self) {
        let s = &self.selection;
        let active = match s.tree {
            Some(TreeItemId::Feature(id)) => Some(id),
            Some(_) => None,
            None => self.active_document_object,
        };
        self.selection.before_task = Some(Box::new(KeptSelection {
            body: s.body,
            last_face_hit: s.last_face_hit,
            face_highlight: s.face_highlight.clone(),
            earlier_faces: s.earlier_faces.clone(),
            edges: s.edges.clone(),
            earlier_bodies: s.earlier_bodies.clone(),
            parts: s.parts.clone(),
            tree: s.tree,
            tree_also: s.tree_also.clone(),
            active_document_object: active,
            active_body_id: self.active_body_id,
        }));
    }

    /// A cancelled task's edits are taken back: put back the selection it
    /// found, less what is gone from the document. The build the take back
    /// starts finds the picks on the shape again (`shape_rebuilt`). False
    /// when no selection was kept.
    pub fn put_back_selection(&mut self) -> bool {
        let Some(kept) = self.selection.before_task.take() else {
            return false;
        };
        let document = &self.document;
        let has_feature = |id: &FeatureId| document.get_feature_meta(*id).is_some();
        let has_body = |id: &Uuid| document.body(BodyId(*id)).is_some();
        let s = &mut self.selection;
        s.body = kept.body.filter(has_body);
        s.last_face_hit = kept.last_face_hit.filter(|(b, _)| has_body(b));
        s.face_highlight = kept.face_highlight.filter(|f| has_body(&f.body));
        s.earlier_faces = kept.earlier_faces;
        s.earlier_faces.retain(|p| has_body(&p.highlight.body));
        s.edges = kept.edges;
        s.edges.retain(|e| has_body(&e.body));
        s.earlier_bodies = kept.earlier_bodies;
        s.earlier_bodies.retain(has_body);
        s.parts = kept.parts.filter(|(f, _)| has_feature(f));
        s.tree = kept.tree.map(|row| match row {
            TreeItemId::Feature(id) if !has_feature(&id) => TreeItemId::DocumentRoot,
            TreeItemId::Body(id) if !has_body(&id.0) => TreeItemId::DocumentRoot,
            row => row,
        });
        s.tree_also = kept.tree_also.map(|(row, mut also)| {
            also.retain(has_feature);
            (row, also)
        });
        s.last_select_click = None;
        self.active_document_object = kept.active_document_object.filter(has_feature);
        self.active_body_id = kept.active_body_id.filter(|b| has_body(&b.0));
        true
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

    /// A Design tool started from the toolbar on what is picked, its
    /// task's build landing, then Cancel or OK, with real benches and
    /// kernel builds: the whole body is never what is selected, the tree's
    /// row stays, Cancel puts back what was picked (found again on the
    /// rebuilt shape) and OK lets go of the picks the task used.
    #[test]
    fn a_design_task_leaves_the_selection_it_found() {
        use crate::app::face_pick::{FaceHighlight, face_id_at, face_submesh_by_id};
        use crate::app::workbench_host::WbCtxParams;
        #[derive(Clone, Copy, PartialEq)]
        enum Pick {
            Edge,
            Face,
            Sketch,
        }
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        registry.set_kernel(&kernel_ogeom::QUERIES);
        let script = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.polyline{sketch = s, closed = true,
              points = {{0, 0}, {40, 0}, {40, 30}, {0, 30}}}
            local pad = pc.design.pad{sketch = s, length = 20}
            local top = pc.sketch.new{body = pc.doc.feature{id = pad}.body,
              plane = "XY", offset = 20, name = "Top"}
            pc.sketch.circle{sketch = top, x = 20, y = 15, radius = 5}
            assert(#pc.doc.rebuild() == 0)"#;
        let built = Document::new("t");
        let (built, out) = crate::headless::run_in_document(&mut registry, built, script, "box");
        out.unwrap();
        let body = built.bodies()[0].id;
        let land = |registry: &mut core_document::DocumentService,
                    session: &mut DocumentSession| {
            let document = std::mem::replace(&mut session.document, Document::new("t"));
            let (document, out) =
                crate::headless::run_in_document(registry, document, "pc.doc.rebuild()", "build");
            out.unwrap();
            session.document = document;
            session.shape_rebuilt(body);
        };
        // What is selected, in the view and the tree, as the user sees it.
        let seen = |s: &DocumentSession| {
            let edges: Vec<_> = s
                .selection
                .edges
                .iter()
                .map(|e| (e.edge, e.faces))
                .collect();
            let face = s.selection.face_highlight.as_ref().map(|f| f.face);
            let face = face.zip(s.selection.face().map(|f| f.name));
            (
                s.selection.body,
                edges,
                face,
                s.selection.tree,
                s.active_document_object,
            )
        };
        let cases = [
            ("design.fillet", Pick::Edge),
            ("design.chamfer", Pick::Edge),
            ("design.fillet", Pick::Face),
            ("design.datum_plane", Pick::Face),
            ("design.pad", Pick::Sketch),
        ];
        for ((tool, pick), accept) in cases.iter().flat_map(|c| [(*c, false), (*c, true)]) {
            let case = format!(
                "{tool} on {} by {}",
                pick as u8,
                if accept { "OK" } else { "Cancel" }
            );
            let mut session = session_on(crate::app::server::fallback());
            session.document = built.clone();
            session.journal.reset(&mut session.document);
            let mesh = session
                .document
                .imported_geometry(body)
                .unwrap()
                .mesh
                .clone();
            match pick {
                Pick::Edge => {
                    let front_top = (0..mesh.edge_faces.len() as u32)
                        .filter_map(|e| crate::app::edges::edge_hit(&mesh, body.0, e))
                        .find(|e| {
                            glam::Vec3::from(e.point).distance(glam::vec3(20.0, 0.0, 20.0)) < 1e-3
                        })
                        .unwrap();
                    session.selection.edges = vec![front_top];
                    session.selection.body = Some(body.0);
                }
                Pick::Face => {
                    let point = [10.0, 10.0, 20.0];
                    let id = face_id_at(&mesh, point).unwrap();
                    let face = core_document::FaceRef {
                        name: mesh.face_names[id as usize],
                        point,
                        normal: [0.0, 0.0, 1.0],
                        surface: None,
                    };
                    session.selection.last_face_hit = Some((body.0, face));
                    session.selection.face_highlight = Some(FaceHighlight {
                        body: body.0,
                        face: Some(id),
                        mesh: std::sync::Arc::new(face_submesh_by_id(&mesh, id).unwrap()),
                        revision: 0,
                    });
                    session.selection.body = Some(body.0);
                }
                Pick::Sketch => {
                    let (top, _) = session
                        .document
                        .feature_tree()
                        .all_nodes()
                        .find(|(_, n)| n.name == "Top")
                        .unwrap();
                    session.selection.tree = Some(TreeItemId::Feature(*top));
                    session.active_document_object = Some(*top);
                }
            }
            let before = seen(&session);
            // The toolbar's start: the bench reads the selection, makes the
            // feature and holds it as the active object.
            let bench = registry.ids().iter().find(|id| {
                registry
                    .tools_for(id)
                    .is_ok_and(|tools| tools.iter().any(|t| t.id == tool))
            });
            let bench = bench.cloned().unwrap();
            let s = &session.selection;
            let params = WbCtxParams {
                selected_body_id: s.body,
                active_document_object: session.active_document_object,
                selected_face: s.face(),
                selected_faces: s.face_refs(),
                selected_edges: s.edge_refs(),
                ..WbCtxParams::windowless()
            };
            let mut ctx = params.context(&mut session.document, &registry);
            let wb = registry.workbench_mut(&bench).unwrap();
            wb.on_input(
                &core_document::WorkbenchInputEvent::ToolActivated,
                Some(tool),
                &mut ctx,
            );
            let made = core_document::HookOutcome::take(&mut ctx).active_document_object;
            drop(ctx);
            assert_ne!(made, before.4, "{case}: the tool made a feature");
            session.active_document_object = made;
            session.keep_selection_for_task();
            session.journal.begin_task();
            land(&mut registry, &mut session);
            assert_eq!(session.selection.whole_body(), None, "{case}: open");
            assert_eq!(session.selection.tree, before.3, "{case}: open");
            if accept {
                session.journal.accept_task(&mut session.document, None);
                session.selection.before_task = None;
                session.selection.let_go_of_used_picks();
                land(&mut registry, &mut session);
                let after = seen(&session);
                assert_eq!(session.selection.whole_body(), None, "{case}");
                assert_eq!(
                    (after.0, after.3, after.4),
                    (None, before.3, made),
                    "{case}"
                );
            } else {
                session.journal.cancel_task(&mut session.document);
                registry.invalidate_all(&mut session.document);
                assert!(session.put_back_selection(), "{case}");
                land(&mut registry, &mut session);
                assert_eq!(seen(&session), before, "{case}");
            }
        }
    }
}
