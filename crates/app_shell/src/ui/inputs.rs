//! Per-frame inputs handed from the app to [`super::UiLayer::run`].

use std::path::PathBuf;

use axes::AxisSystem;
use settings::UserSettings;

use super::feature_tree::TreeItemId;
use super::host_ctx::HostCtxParams;
use settings::recent::RecentEntry;

use super::{ActiveTool, ActiveWorkbench, Screen};
use crate::orientation_cube::OrientationCubeInput;

/// The body under the cursor and the point hit on it.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverCard {
    pub title: String,
    pub point_mm: [f32; 3],
}

/// Everything the UI needs to draw one frame. Constructed as a literal at
/// the call site — the fields borrow disjoint pieces of `PrintCadApp`, which
/// a `&mut self` builder method could not express.
pub struct UiFrameInputs<'a> {
    pub screen: Screen,
    /// Recently opened documents, most recent first.
    pub recent: &'a [RecentEntry],
    /// The host's tool state — authoritative. The host consumes Action tool
    /// ids (e.g. `part.new_body`, a used `sketch.create`) from its copy, so
    /// the UI must re-seed from it each frame rather than keeping its own.
    pub active_tool: ActiveTool,
    /// The host's active workbench — authoritative (the host can switch
    /// benches itself, e.g. the create-sketch flow).
    pub active_workbench: ActiveWorkbench,
    pub settings: &'a UserSettings,
    pub document: &'a mut core_document::Document,
    pub registry: &'a mut core_document::DocumentService,
    /// Camera and viewport facts for the contexts panel hooks receive.
    pub host: HostCtxParams,
    pub orientation_input: Option<&'a OrientationCubeInput>,
    /// A sketch is open: the view only rotates about its plane normal.
    pub planar_view_lock: bool,
    /// Smoothed frames-per-second while rendering; `None` when the
    /// render loop is about to sleep (render on demand), so the display
    /// can say "idle" instead of freezing at the last busy number.
    pub fps: Option<f32>,
    /// Scene redraws in the last second. Zero while the cached scene is
    /// being reused under fresh UI frames.
    pub scene_redraws_per_s: u32,
    pub gpu_name: Option<&'a str>,
    pub gpus: &'a [String],
    pub hovered_point: Option<[f32; 3]>,
    pub pivot_screen_pos: Option<(f32, f32)>,
    pub axis_system: AxisSystem,
    pub tree_selection: Option<TreeItemId>,
    pub active_document_object: Option<core_document::FeatureId>,
    /// The feature whose edit session is open (tree badge, breadcrumb).
    pub editing_feature: Option<core_document::FeatureId>,
    /// The active workbench's viewport widgets this frame.
    pub viewport_hud: Option<core_document::ViewportHud>,
    /// The active workbench's status-bar items this frame.
    pub status_items: Option<core_document::StatusItems>,
    /// The active workbench's open task, if any.
    pub task: Option<core_document::TaskInfo>,
    /// What sits under the cursor in the viewport.
    pub hover_card: Option<HoverCard>,
    /// "w × h × d" of the selected body, already formatted.
    pub dimensions: Option<String>,
    pub screen_space_overlays: &'a [core_document::ScreenSpaceOverlay],
    pub screen_space_marks: &'a [core_document::ScreenSpaceMark],
    pub screen_space_labels: &'a [core_document::ScreenSpaceLabel],
    pub pending_imports: u32,
    pub pending_document_open: u32,
    /// What the kernel worker is doing right now, for the status bar.
    pub kernel_status: Option<String>,
    /// Whether the running kernel job can be stopped.
    pub kernel_cancellable: bool,
    /// `(done, total)` of the current kernel stage, when counts are known.
    pub kernel_progress: Option<(u64, u64)>,
    /// A document write is in flight.
    pub document_saving: bool,
    /// Bytes packed into the archive being saved, out of the whole.
    pub save_progress: Option<(u64, u64)>,
    /// Which document server serves this session, with a degraded marker —
    /// e.g. "local daemon" or "local daemon (disconnected)".
    pub server_label: String,
    /// A body double-clicked in the viewport: the tree jumps to its row.
    pub reveal_body: Option<core_document::BodyId>,
    /// The 6-DoF mouse the reader thread has, if any.
    pub nav_device: Option<String>,
    /// How many buttons it has, so Preferences offers a row per button.
    pub nav_buttons: u32,
    pub step_import_pending: Option<&'a mut (PathBuf, kernel_api::TessellationSettings)>,
}
