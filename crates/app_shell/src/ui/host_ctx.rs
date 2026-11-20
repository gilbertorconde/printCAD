//! The runtime context panel hooks receive. Panels run inside the UI pass,
//! so the camera and viewport they see are the ones the previous frame
//! rendered with.

use core_document::{Document, FeatureId, WorkbenchRuntimeContext};

use crate::contract::HostCtxParams;

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
    ctx.pixels_per_point = params.pixels_per_point;
    ctx.active_document_object = active_document_object;
    ctx.selected_body_id = params.selected_body_id;
    ctx.selected_face = params.selected_face;
    ctx.selected_faces = params.selected_faces.clone();
    ctx.selected_edges = params.selected_edges.clone();
    ctx.selected_parts = params.selected_parts.clone();
    if ctx.active_document_object == params.selected_features.first().copied() {
        ctx.selected_features = params.selected_features.clone();
    }
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    ctx.bought_kinds = params.bought_kinds.clone();
    ctx.undo_entry = params.undo_entry;
    ctx
}

/// Panel UI hooks log through the runtime context; route those entries to
/// the app log panel instead of dropping them.
pub fn flush_ctx_logs(ctx: &mut WorkbenchRuntimeContext) {
    for entry in ctx.drain_logs() {
        match entry.level {
            core_document::LogLevel::Info => app_log::info(entry.message),
            core_document::LogLevel::Warn => app_log::warn(entry.message),
            core_document::LogLevel::Error => app_log::error(entry.message),
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
    /// What the hook recorded, for a recording in progress.
    pub recorded: Vec<core_document::Recorded>,
}

impl PanelWriteback {
    pub fn take(ctx: &mut WorkbenchRuntimeContext, before: Option<FeatureId>) -> Self {
        let outcome = core_document::HookOutcome::take(ctx);
        let active_object_changed =
            (outcome.active_document_object != before).then_some(outcome.active_document_object);
        Self {
            active_object_changed,
            requests: outcome.requests,
            recorded: outcome.recorded,
        }
    }
}
