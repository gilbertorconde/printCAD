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
/// the call site: the fields borrow disjoint pieces of `PrintCadApp`, which
/// a `&mut self` builder method could not express.
pub struct UiFrameInputs<'a> {
    pub screen: Screen,
    /// The view is split: scans in the viewport, the other bodies in a
    /// pane beside it.
    pub split_view: bool,
    /// The split view's panes share one camera (View › Link split cameras).
    pub split_linked: bool,
    /// Recently opened documents, most recent first.
    pub recent: &'a [RecentEntry],
    /// Autosaved copies a crash left, which the start page offers back.
    pub recoverable: &'a [crate::app::recovery::Recoverable],
    /// The document the unsaved-edits question asks about, while it is open.
    pub unsaved_question: Option<String>,
    /// The host's tool state, authoritative. The host consumes a used
    /// Action tool id from its copy once its bench has handled it, so the
    /// UI must re-seed from it each frame rather than keeping its own.
    pub active_tool: ActiveTool,
    /// The host's active workbench, authoritative (the host can switch
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
    /// The graphics API the renderer runs on.
    pub graphics_api: Option<&'a str>,
    pub gpus: &'a [String],
    pub hovered_point: Option<[f32; 3]>,
    pub pivot_screen_pos: Option<(f32, f32)>,
    pub axis_system: AxisSystem,
    pub tree_selection: Option<TreeItemId>,
    /// Features Ctrl-clicked into the tree's selection beside it.
    pub tree_also: &'a [core_document::FeatureId],
    pub active_document_object: Option<core_document::FeatureId>,
    /// The feature whose edit session is open (tree badge, breadcrumb).
    pub editing_feature: Option<core_document::FeatureId>,
    /// The active workbench's viewport widgets this frame.
    pub viewport_hud: Option<core_document::ViewportHud>,
    pub scalar_legend: Option<&'a core_document::ScalarMap>,
    /// The active workbench's status-bar items this frame.
    pub status_items: Option<core_document::StatusItems>,
    /// The active workbench's open task, if any.
    pub task: Option<core_document::TaskInfo>,
    /// What sits under the cursor in the viewport.
    pub hover_card: Option<HoverCard>,
    /// What is picked in the view when it is more than one thing ("3
    /// faces, 2 edges"), for the status bar.
    pub picks: Option<String>,
    /// "w × h × d" of the selected body, already formatted.
    pub dimensions: Option<String>,
    /// What the selection measures, a line each, while the status bar's
    /// quick measure is on.
    pub quick_measure: Vec<String>,
    /// The selected bodies' volumes, for the status bar's mass.
    pub selection_mass: Option<super::SelectionMass>,
    /// Volume, area and centre of the body the property panel shows.
    pub physical: Option<Physical>,
    /// What the Physical group can take the inertia about, the centre of
    /// mass first.
    pub inertia_references: Vec<super::property_panel::InertiaReference>,
    /// The camera's field of view, in degrees, for the view toolbar.
    pub field_of_view_deg: f32,
    /// The projection the tab's view draws with.
    pub projection: settings::ProjectionMode,
    /// The clipping plane, when on, and the scene box it moves across.
    pub section: Option<crate::camera::section::SectionPlane>,
    /// What the clipping plane waits for a pick in the view to aim it.
    pub section_prompt: Option<String>,
    /// What a click in the view picks.
    pub pick_filter: super::view_toolbar::PickFilter,
    pub scene_bounds: Option<(glam::Vec3, glam::Vec3)>,
    pub screen_space_overlays: &'a [core_document::ScreenSpaceOverlay],
    /// Filled shapes the active bench draws beneath its lines.
    pub screen_space_polygons: &'a [core_document::ScreenSpacePolygon],
    /// Pictures the active bench lays over the viewport.
    pub screen_space_images: &'a [core_document::ScreenSpaceImage],
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
    /// The document server serving this session, and how it is doing.
    pub server: super::status_bar::ServerBadge,
    /// The strip of open documents, in order.
    pub tabs: Vec<super::TabInfo>,
    /// The measure tool is armed.
    pub measuring: bool,
    /// The radial menu, while a held right press has it open.
    pub radial: Option<super::radial::RadialView>,
    /// The measure tool's own pill under the view toolbar, while it is armed.
    pub measure_bar: Option<super::view_toolbar::MeasureBar>,
    /// A body double-clicked in the viewport: the tree jumps to its row.
    pub reveal_body: Option<core_document::BodyId>,
    /// The feature that made the face picked in the view: its tree row is
    /// lit, and opened and scrolled to when `reveal_face_maker` is set.
    pub face_maker: Option<core_document::FeatureId>,
    pub reveal_face_maker: bool,
    /// The context menu a right click asked for, if one is open.
    pub viewport_menu: Option<super::ViewportMenu>,
    /// The faces selected in the view, in the order picked: each its body
    /// and index in the body's mesh.
    pub picked_faces: Vec<(core_document::BodyId, u32)>,
    /// The edges selected in the view: each its body and a point on it
    /// where the body sits.
    pub picked_edges: Vec<(core_document::BodyId, [f32; 3])>,
    /// The 6-DoF mouse the reader thread has, if any.
    pub nav_device: Option<String>,
    /// How many buttons it has, so Preferences offers a row per button.
    pub nav_buttons: u32,
    pub step_import_pending: Option<&'a mut (PathBuf, kernel_api::TessellationSettings)>,
    pub export_pending: Option<&'a mut crate::app::export::ExportDraft>,
    /// The scripts folder's scripts.
    pub scripts: &'a [crate::script_library::ScriptEntry],
    /// The workbench packages found when the app started, and since.
    pub packages: &'a [workbenches::PackageStatus],
    /// What the last look for a newer printCAD found.
    pub release: &'a crate::app::updates::ReleaseCheck,
    /// The workbench store's list, and whether it is being fetched.
    pub store: &'a crate::app::packages::StoreView,
    /// A script printed or failed since the last frame: show the console.
    pub console_attention: bool,
    /// Every command's id, for the console's completion.
    pub command_ids: &'a [String],
    /// The script running, by name, while one is.
    pub script_running: Option<&'a str>,
    /// A recording is on.
    pub recording: bool,
    /// The chats with agents.
    pub chats: &'a [crate::app::chats::Chat],
    /// Changes agents asked for, waiting for the user's OK.
    pub approvals: &'a [crate::app::mcp::Approval],
    /// What each chat's `@` list offers, by the tab the chat works on.
    pub mentions: Vec<(uuid::Uuid, Vec<crate::app::mentions::MentionOption>)>,
    /// An agent needs the user: the assistant panel opens.
    pub assistant_attention: bool,
    /// The paste key went down: a chat whose box has the keyboard and got
    /// no text from it takes a picture from the clipboard.
    pub paste_pressed: bool,
    /// The print layout the view shows, while its task is open.
    pub print_layout: Option<&'a crate::app::print_layout::Layout>,
    /// How many vertices and triangles of the armed scan are marked.
    pub marking: Option<(usize, usize)>,
    /// What the active tab's file and its linked files ask of the user.
    pub file: FileMarks<'a>,
}

/// What the active tab's file, and the files its parts link to, ask of
/// the user.
#[derive(Debug, Clone, Default)]
pub struct FileMarks<'a> {
    /// The document's file cannot be written in place.
    pub read_only: bool,
    /// Linked parts whose file cannot be written in place.
    pub read_only_links: &'a [core_document::BodyId],
    /// A newer printCAD wrote the file: what the card says, while it is up.
    pub newer_notice: Option<String>,
    /// Saving over a file another version wrote, waiting for an answer.
    pub save_over: Option<crate::app::file_safety::SaveOverQuestion>,
    /// An STL waiting for its units: its file name and the answer offered.
    pub mesh_units: Option<(String, kernel_api::LengthUnit)>,
}

/// The selected bodies' volumes for the status bar's mass: each body's
/// volume in mm³ with its own material's density (g/cm³) when it has one;
/// the rest are reckoned in the material the bar's picker names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectionMass {
    pub parts: Vec<(f64, Option<f32>)>,
    /// A body is still being measured.
    pub measuring: bool,
    /// A volume was integrated over a tessellation.
    pub approximate: bool,
}

impl SelectionMass {
    /// The whole volume, mm³, and mass, g, at `density` (g/cm³) for the
    /// bodies with no material of their own.
    pub fn totals(&self, density: f32) -> (f64, f64) {
        self.parts.iter().fold((0.0, 0.0), |(v, m), (volume, own)| {
            let d = f64::from(own.unwrap_or(density));
            (v + volume, m + volume / 1000.0 * d)
        })
    }

    /// How many bodies take their own material's density.
    pub fn own_materials(&self) -> usize {
        self.parts.iter().filter(|(_, own)| own.is_some()).count()
    }
}

/// A body's measure as the property panel shows it.
#[derive(Debug, Clone)]
pub enum Physical {
    /// The kernel worker is measuring it.
    Measuring,
    Ready(Box<kernel_api::PhysicalProperties>),
    /// The kernel could not measure it, and why.
    Failed(String),
}
