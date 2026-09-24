//! Shared plumbing for handing a [`WorkbenchRuntimeContext`] to workbench
//! hooks. Context construction and outcome extraction have one shape,
//! and this module is the single place it lives; hook call sites never
//! build a context by hand, and no hook's requests are dropped.

use core_document::{
    FeatureId, HookOutcome, HostRequest, LogEntry, LogLevel, Workbench, WorkbenchId,
    WorkbenchRuntimeContext,
};
use uuid::Uuid;

use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::ui::TreeItemId;

/// Snapshot of host state a hook's context is built from. Constructed via
/// [`PrintCadApp::interaction_ctx_params`] / [`PrintCadApp::overlay_ctx_params`]
/// (`&self`-only — a `&mut self` builder could not coexist with the split
/// field borrows inside [`PrintCadApp::with_workbench_ctx`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct WbCtxParams {
    pub cam_pos: [f32; 3],
    pub cam_target: [f32; 3],
    pub viewport: (u32, u32, u32, u32),
    pub view_proj: Option<[[f32; 4]; 4]>,
    pub hovered_world_pos: Option<[f32; 3]>,
    pub hovered_body_id: Option<Uuid>,
    pub selected_body_id: Option<Uuid>,
    pub cursor_viewport_pos: Option<(f32, f32)>,
    pub active_document_object: Option<FeatureId>,
    pub attach_request: Option<core_document::SketchAttachRequest>,
    pub selected_face: Option<core_document::FaceRef>,
    pub selected_edges: Vec<core_document::EdgeRef>,
    pub ctrl_down: bool,
}

/// Where a hook ran from, which decides what its requests may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookSite {
    /// Input, per-frame and edit-session hooks: every request applies.
    Interaction,
    /// Activate, deactivate and the per-frame overlay getters: a bench
    /// switch from inside a switch, or mid-scene, would re-enter, so
    /// switch and start requests are dropped here.
    Lifecycle,
}

impl PrintCadApp {
    /// Context params for activate/deactivate/input hooks: camera-derived
    /// viewport plus the live hover/selection state.
    /// Close the current undo step, after working out the formulas and
    /// letting every bench follow values they moved, so an edit and what
    /// it moves are one step.
    pub(crate) fn close_gesture(&mut self) {
        self.settle_formulas();
        self.session.journal.note(&mut self.session.document);
    }

    /// Work out the document's formulas and hand the features whose values
    /// moved to every bench (`Workbench::values_moved`).
    pub(crate) fn settle_formulas(&mut self) {
        self.registry.evaluate(&mut self.session.document);
        let moved = self.session.document.take_moved_values();
        if moved.is_empty() {
            return;
        }
        for id in self.registry.ids().to_vec() {
            let params = self.interaction_ctx_params();
            if let Some(((), outcome)) =
                self.with_workbench_ctx(&id, params, |wb, ctx| wb.values_moved(ctx, &moved))
            {
                self.apply_hook_outcome(outcome, HookSite::Interaction);
            }
        }
    }

    pub(crate) fn interaction_ctx_params(&self) -> WbCtxParams {
        let vp = self.session.camera.viewport_info();
        WbCtxParams {
            cam_pos: self.session.camera.position(),
            cam_target: self.session.camera.target(),
            viewport: (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            view_proj: Some(self.session.camera.view_projection()),
            hovered_world_pos: self.session.hovered_world_pos,
            hovered_body_id: self.session.hovered_body,
            selected_body_id: self.session.selected_body,
            cursor_viewport_pos: self.cursor_in_viewport,
            active_document_object: self.session.active_document_object,
            attach_request: self.session.pending_sketch_creation,
            selected_face: self
                .session
                .last_face_hit
                .filter(|(body, _)| self.session.selected_body == Some(*body))
                .map(|(_, face)| face),
            selected_edges: self.selected_edge_refs(),
            ctrl_down: self.modifiers.control_key(),
        }
    }

    /// Context params for the per-frame overlay hooks. Uses the UI-derived
    /// viewport rect (falling back to a nominal size before the first UI
    /// frame) and the *active body* as selection, matching the tree-driven
    /// editing state rather than the click-driven `selected_body`.
    pub(crate) fn overlay_ctx_params(&self) -> WbCtxParams {
        let viewport = if let Some(rect) = self.frame_submission.viewport_rect {
            (rect.x, rect.y, rect.width, rect.height)
        } else {
            (0, 0, 1920, 1080)
        };
        WbCtxParams {
            cam_pos: self.session.camera.position(),
            cam_target: self.session.camera.target(),
            viewport,
            view_proj: Some(self.session.camera.view_projection()),
            hovered_world_pos: None,
            hovered_body_id: None,
            selected_body_id: self.session.active_body_id.map(|id| id.0),
            cursor_viewport_pos: None,
            active_document_object: self.session.active_document_object,
            attach_request: None,
            selected_face: None,
            selected_edges: self.selected_edge_refs(),
            ctrl_down: false,
        }
    }

    /// Run `f` with the workbench and a fully populated runtime context.
    ///
    /// Logs are flushed unconditionally; everything else the hook left is
    /// returned as a [`HookOutcome`] for [`Self::apply_hook_outcome`].
    /// Returns `None` when the workbench id is not registered.
    pub(crate) fn with_workbench_ctx<R>(
        &mut self,
        wb_id: &WorkbenchId,
        params: WbCtxParams,
        f: impl FnOnce(&mut dyn Workbench, &mut WorkbenchRuntimeContext<'_>) -> R,
    ) -> Option<(R, HookOutcome)> {
        let Ok(wb) = self.registry.workbench_mut(wb_id) else {
            return None;
        };
        let mut ctx = WorkbenchRuntimeContext::new(
            &mut self.session.document,
            params.cam_pos,
            params.cam_target,
            params.viewport,
        );
        ctx.view_proj = params.view_proj;
        ctx.hovered_world_pos = params.hovered_world_pos;
        ctx.hovered_body_id = params.hovered_body_id;
        ctx.selected_body_id = params.selected_body_id;
        ctx.cursor_viewport_pos = params.cursor_viewport_pos;
        ctx.active_document_object = params.active_document_object;
        ctx.attach_request = params.attach_request;
        ctx.selected_face = params.selected_face;
        ctx.selected_edges = params.selected_edges;
        ctx.kernel = Some(&kernel_ogeom::QUERIES);
        ctx.ctrl_down = params.ctrl_down;

        let result = f(wb.as_mut(), &mut ctx);

        let outcome = HookOutcome::take(&mut ctx);
        Self::flush_logs(ctx.drain_logs());
        Some((result, outcome))
    }

    /// The active workbench's per-frame hook. Tool enablement, overlays
    /// and the HUD all read state it refreshes, so it runs before the
    /// scene and the UI are built.
    pub(crate) fn call_workbench_on_frame(&mut self, dt: f32) {
        let wb_id = self.active_workbench_id();
        let params = self.interaction_ctx_params();
        if let Some((_, outcome)) =
            self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.on_frame(dt, ctx))
        {
            self.apply_hook_outcome(outcome, HookSite::Interaction);
        }
    }

    /// Apply what a hook left: the active document object it may have
    /// changed, the attach inbox as it left it, then its requests in the
    /// host's order.
    pub(crate) fn apply_hook_outcome(&mut self, outcome: HookOutcome, site: HookSite) {
        self.record_calls(outcome.recorded);
        if outcome.active_document_object != self.session.active_document_object {
            self.session.active_document_object = outcome.active_document_object;
        }
        // The context was seeded with the pending request; the bench it
        // was for takes it (None comes back), any other leaves it.
        if site == HookSite::Interaction {
            self.session.pending_sketch_creation = outcome.attach_request;
        }
        for request in outcome.requests {
            self.apply_host_request(request, site);
        }
    }

    /// One request, as the host answers it.
    pub(crate) fn apply_host_request(&mut self, request: HostRequest, site: HookSite) {
        match request {
            HostRequest::ActivateTool(tool) => {
                self.session.active_tool.active_ids.clear();
                self.session.active_tool.active_ids.insert(tool);
            }
            HostRequest::SelectBody(body) => {
                self.apply_tree_selection(TreeItemId::Body(body));
            }
            HostRequest::JournalLabel(label) => self.session.journal.label_next(label),
            HostRequest::StartOn { workbench, attach } => {
                if site == HookSite::Interaction {
                    self.session.pending_sketch_creation = Some(attach);
                    self.switch_workbench_for_flow(workbench);
                }
            }
            HostRequest::SwitchWorkbench(workbench) => {
                if site == HookSite::Interaction {
                    self.switch_workbench_for_flow(workbench);
                }
            }
            HostRequest::OrientCamera(orient) => {
                self.session.camera.orient_to_plane(
                    glam::Vec3::from_array(orient.plane_origin),
                    glam::Vec3::from_array(orient.plane_normal),
                    glam::Vec3::from_array(orient.plane_up),
                    &self.user_settings.camera,
                );
            }
            HostRequest::FinishEditing => self.finish_active_workbench_editing(),
        }
    }

    /// Flush log entries to the app log panel.
    pub(crate) fn flush_logs(logs: Vec<LogEntry>) {
        for entry in logs {
            match entry.level {
                LogLevel::Info => app_log::info(entry.message),
                LogLevel::Warn => app_log::warn(entry.message),
                LogLevel::Error => app_log::error(entry.message),
            }
        }
    }
}
