//! The runtime context panel hooks receive. Panels run inside the UI pass,
//! so the camera and viewport they see are the ones the previous frame
//! rendered with.

use core_document::{Document, FeatureId, WorkbenchRuntimeContext};

use crate::log_panel;

/// Camera and viewport facts the app hands the UI each frame.
#[derive(Debug, Clone)]
pub struct HostCtxParams {
    pub camera_position: [f32; 3],
    pub camera_target: [f32; 3],
    /// `(x, y, width, height)` in physical pixels.
    pub viewport: (u32, u32, u32, u32),
    pub view_proj: Option<[[f32; 4]; 4]>,
    pub selected_body_id: Option<uuid::Uuid>,
    pub selected_face: Option<core_document::FaceRef>,
    pub selected_edges: Vec<core_document::EdgeRef>,
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
            selected_edges: Vec::new(),
        }
    }
}

/// A context for a panel hook.
pub fn panel_ctx<'a>(
    document: &'a mut Document,
    params: &HostCtxParams,
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
    ctx.selected_edges = params.selected_edges.clone();
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

/// What a panel hook left on its context, for the UI to turn into
/// commands. Every request survives the trip.
#[derive(Debug, Default)]
pub struct PanelWriteback {
    /// The hook changed the active document object (feature created or
    /// released).
    pub active_object_changed: Option<Option<FeatureId>>,
    pub requests: Vec<core_document::HostRequest>,
}

impl PanelWriteback {
    pub fn take(ctx: &mut WorkbenchRuntimeContext, before: Option<FeatureId>) -> Self {
        let outcome = core_document::HookOutcome::take(ctx);
        let active_object_changed =
            (outcome.active_document_object != before).then_some(outcome.active_document_object);
        Self {
            active_object_changed,
            requests: outcome.requests,
        }
    }
}
