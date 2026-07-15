//! Per-frame inputs handed from the app to [`super::UiLayer::run`].

use std::path::PathBuf;

use axes::AxisSystem;
use settings::UserSettings;

use super::feature_tree::TreeItemId;
use super::{ActiveTool, ActiveWorkbench};
use crate::orientation_cube::OrientationCubeInput;

/// Everything the UI needs to draw one frame. Constructed as a literal at
/// the call site — the fields borrow disjoint pieces of `PrintCadApp`, which
/// a `&mut self` builder method could not express.
pub struct UiFrameInputs<'a> {
    /// The host's tool state — authoritative. The host consumes Action tool
    /// ids (e.g. `part.new_body`, a used `sketch.create`) from its copy, so
    /// the UI must re-seed from it each frame rather than keeping its own.
    pub active_tool: ActiveTool,
    /// The host's active workbench — authoritative (the host can switch
    /// benches itself, e.g. the create-sketch flow).
    pub active_workbench: ActiveWorkbench,
    pub settings: &'a mut UserSettings,
    pub document: &'a mut core_document::Document,
    pub registry: &'a mut core_document::DocumentService,
    pub orientation_input: Option<&'a OrientationCubeInput>,
    pub fps: f32,
    pub gpu_name: Option<&'a str>,
    pub gpus: &'a [String],
    pub hovered_point: Option<[f32; 3]>,
    pub pivot_screen_pos: Option<(f32, f32)>,
    pub axis_system: AxisSystem,
    pub tree_selection: Option<TreeItemId>,
    pub active_document_object: Option<core_document::FeatureId>,
    pub selected_body_id: Option<core_document::BodyId>,
    pub screen_space_overlays: &'a [core_document::ScreenSpaceOverlay],
    pub pending_imports: u32,
    pub pending_document_open: u32,
    pub step_import_pending: Option<&'a mut (PathBuf, kernel_api::TessellationSettings)>,
}
