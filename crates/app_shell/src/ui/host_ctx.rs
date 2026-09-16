//! The runtime context panel hooks receive. Panels run inside the UI pass,
//! so the camera and viewport they see are the ones the previous frame
//! rendered with.

use core_document::{Document, FeatureId, WorkbenchRuntimeContext};

use crate::log_panel;

/// Camera and viewport facts the app hands the UI each frame.
#[derive(Debug, Clone, Copy)]
pub struct HostCtxParams {
    pub camera_position: [f32; 3],
    pub camera_target: [f32; 3],
    /// `(x, y, width, height)` in physical pixels.
    pub viewport: (u32, u32, u32, u32),
    pub view_proj: Option<[[f32; 4]; 4]>,
    pub selected_body_id: Option<uuid::Uuid>,
    pub selected_face: Option<core_document::FaceRef>,
}

impl Default for HostCtxParams {
    fn default() -> Self {
        Self {
            camera_position: [0.0, 0.0, 5.0],
            camera_target: [0.0; 3],
            viewport: (0, 0, 1, 1),
            view_proj: None,
            selected_body_id: None,
            selected_face: None,
        }
    }
}

/// A context for a panel hook.
pub fn panel_ctx<'a>(
    document: &'a mut Document,
    params: HostCtxParams,
    active_document_object: Option<FeatureId>,
) -> WorkbenchRuntimeContext<'a> {
    let mut ctx = WorkbenchRuntimeContext::new(
        document,
        params.camera_position,
        params.camera_target,
        params.viewport,
    );
    ctx.view_proj = params.view_proj;
    ctx.active_document_object = active_document_object;
    ctx.selected_body_id = params.selected_body_id;
    ctx.selected_face = params.selected_face;
    ctx
}

/// Panel UI hooks log through the runtime context; route those entries to
/// the app log panel instead of dropping them.
pub fn flush_ctx_logs(ctx: &mut WorkbenchRuntimeContext) {
    for entry in ctx.drain_logs() {
        match entry.level {
            core_document::LogLevel::Info => log_panel::info(entry.message),
            core_document::LogLevel::Warn => log_panel::warn(entry.message),
            core_document::LogLevel::Error => log_panel::error(entry.message),
        }
    }
}

/// Write-backs a panel hook can leave on its context.
#[derive(Debug, Default)]
pub struct PanelWriteback {
    pub finish_sketch_requested: bool,
    pub camera_orient_request: Option<core_document::CameraOrientRequest>,
    /// The hook changed the active document object (feature created or
    /// released).
    pub active_object_changed: Option<Option<FeatureId>>,
    pub workbench_switch_request: Option<core_document::WorkbenchId>,
}

impl PanelWriteback {
    pub fn take(ctx: &mut WorkbenchRuntimeContext, before: Option<FeatureId>) -> Self {
        let active_object_changed =
            (ctx.active_document_object != before).then_some(ctx.active_document_object);
        Self {
            finish_sketch_requested: ctx.finish_sketch_requested,
            camera_orient_request: ctx.camera_orient_request.take(),
            active_object_changed,
            workbench_switch_request: ctx.workbench_switch_request.take(),
        }
    }
}
