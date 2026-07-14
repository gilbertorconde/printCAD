//! Shared plumbing for handing a [`WorkbenchRuntimeContext`] to workbench
//! hooks. Every hook call site used to hand-roll context construction and
//! write-back extraction; this module is the single place that shape lives.

use core_document::{
    CameraOrientRequest, FeatureId, LogEntry, LogLevel, Workbench, WorkbenchId,
    WorkbenchRuntimeContext,
};
use uuid::Uuid;

use crate::log_panel as app_log;
use crate::PrintCadApp;

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
}

/// Everything a hook may have written back into the context, extracted
/// before the context borrow ends.
pub(crate) struct WbHookOutcome {
    pub camera_orient_request: Option<CameraOrientRequest>,
    pub active_document_object: Option<FeatureId>,
    /// Carried for parity with the context; no call site handles it yet
    /// (the pre-existing "finish sketch" flow was never wired up).
    #[allow(dead_code)]
    pub finish_sketch_requested: bool,
}

impl PrintCadApp {
    /// Context params for activate/deactivate/input hooks: camera-derived
    /// viewport plus the live hover/selection state.
    pub(crate) fn interaction_ctx_params(&self) -> WbCtxParams {
        let vp = self.camera.viewport_info();
        WbCtxParams {
            cam_pos: self.camera.position(),
            cam_target: self.camera.target(),
            viewport: (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            view_proj: Some(self.camera.view_projection()),
            hovered_world_pos: self.hovered_world_pos,
            hovered_body_id: self.hovered_body,
            selected_body_id: self.selected_body,
            cursor_viewport_pos: self.cursor_in_viewport,
            active_document_object: self.active_document_object,
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
            cam_pos: self.camera.position(),
            cam_target: self.camera.target(),
            viewport,
            view_proj: Some(self.camera.view_projection()),
            hovered_world_pos: None,
            hovered_body_id: None,
            selected_body_id: self.active_body_id.map(|id| id.0),
            cursor_viewport_pos: None,
            active_document_object: self.active_document_object,
        }
    }

    /// Run `f` with the workbench and a fully populated runtime context.
    ///
    /// Logs are flushed unconditionally; every other write-back is returned
    /// in [`WbHookOutcome`] so the call site decides what to apply (input
    /// dispatches apply camera-orient requests, lifecycle hooks do not).
    /// Returns `None` when the workbench id is not registered.
    pub(crate) fn with_workbench_ctx<R>(
        &mut self,
        wb_id: &WorkbenchId,
        params: WbCtxParams,
        f: impl FnOnce(&mut dyn Workbench, &mut WorkbenchRuntimeContext<'_>) -> R,
    ) -> Option<(R, WbHookOutcome)> {
        let Ok(wb) = self.registry.workbench_mut(wb_id) else {
            return None;
        };
        let mut ctx = WorkbenchRuntimeContext::new(
            &mut self.document,
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

        let result = f(wb.as_mut(), &mut ctx);

        let outcome = WbHookOutcome {
            camera_orient_request: ctx.camera_orient_request.take(),
            active_document_object: ctx.active_document_object,
            finish_sketch_requested: ctx.finish_sketch_requested,
        };
        Self::flush_logs(ctx.drain_logs());
        Some((result, outcome))
    }

    /// Apply an input hook's write-backs: sync the active document object
    /// and honour a camera orient request.
    pub(crate) fn apply_hook_outcome(&mut self, outcome: WbHookOutcome) {
        if outcome.active_document_object != self.active_document_object {
            self.active_document_object = outcome.active_document_object;
        }
        if let Some(orient_req) = outcome.camera_orient_request {
            self.camera.orient_to_plane(
                glam::Vec3::from_array(orient_req.plane_origin),
                glam::Vec3::from_array(orient_req.plane_normal),
                glam::Vec3::from_array(orient_req.plane_up),
                &self.user_settings.camera,
            );
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
