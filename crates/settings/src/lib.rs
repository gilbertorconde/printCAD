pub mod recent;

use axes::{AxisPreset, AxisSystem};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[cfg(not(target_arch = "wasm32"))]
use std::{fs::File, io::BufReader};
use thiserror::Error;

const QUALIFIER: &str = "com";
const ORGANIZATION: &str = "printcad";
const APPLICATION: &str = "printcad";
const SETTINGS_FILE: &str = "settings.json";
const RECENT_FILE_INFO: &str = "recent.json";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("unable to resolve platform config directory")]
    MissingProjectDirs,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid settings file: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserSettings {
    pub camera: CameraSettings,
    pub lighting: LightingSettings,
    pub rendering: RenderingSettings,
    /// Defaults for file import; the STEP dialog opens with these.
    #[serde(default)]
    pub import: ImportSettings,
    /// Preferred GPU name substring for Vulkan device selection (None = automatic)
    pub preferred_gpu: Option<String>,
    /// Optional FPS cap. 0.0 = uncapped (driven by vsync / driver).
    pub fps_cap: f32,
    /// How a 6-DoF mouse drives the view.
    #[serde(default)]
    pub sixdof: SixDofSettings,
    /// What the app writes down for someone else to read.
    #[serde(default)]
    pub diagnostics: DiagnosticsSettings,
    /// The printer the models are for.
    #[serde(default)]
    pub printing: PrintingSettings,
    /// Each workbench's own settings, by bench id, as the bench serialized
    /// them; the app stores and returns them without reading them.
    #[serde(default)]
    pub workbenches: std::collections::HashMap<String, serde_json::Value>,
    /// Keyboard shortcuts the user changed.
    #[serde(default)]
    pub keyboard: KeyboardSettings,
    /// The AI agents chats can talk to.
    #[serde(default)]
    pub ai: AiSettings,
    /// Workbench packages: which are turned off and what each may reach.
    #[serde(default)]
    pub packages: PackageSettings,
    /// Where the toolbar groups sit, as the user dragged them.
    #[serde(default)]
    pub toolbars: ToolbarLayout,
    /// What the command palette ran lately, the latest first, by the
    /// palette's own keys: it lists them first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_commands: Vec<String>,
    /// Minutes between autosaved copies of edited documents; 0 turns
    /// autosave off.
    #[serde(default = "default_autosave_minutes")]
    pub autosave_minutes: u32,
    /// Looking for newer printCAD releases.
    #[serde(default)]
    pub updates: UpdateSettings,
}

/// Looking for newer printCAD releases; nothing is ever downloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Look once when the app starts.
    pub check_at_start: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_at_start: true,
        }
    }
}

/// The toolbar rows, top to bottom, each the ids of its groups left to
/// right. Groups of every bench share it; a group not listed goes where
/// its bench puts it. Empty: every group where its bench puts it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarLayout {
    pub rows: Vec<Vec<String>>,
}

/// What the user allowed each installed workbench package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PackageSettings {
    /// Look for newer releases of packages installed from GitHub when
    /// the app starts.
    pub check_updates: bool,
    /// Package ids not to load.
    pub disabled: Vec<String>,
    /// What each package may reach, by id; a package missing here may ask
    /// where to save a file and nothing more.
    pub grants: std::collections::BTreeMap<String, PackageGrant>,
    /// The workbench stores whose lists Browse shows: each the address of
    /// a registry's index. A new install starts with printCAD's own.
    pub stores: Vec<String>,
}

/// Where printCAD's own workbench registry publishes its index.
pub const STORE_INDEX: &str = "https://gilbertorconde.github.io/PrintCAD-wb-repo/index.json";

/// What one package may reach beyond its own folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PackageGrant {
    /// Ask the user where to save a file it made.
    pub save_dialog: bool,
    /// Run the native programs it ships.
    pub helper: bool,
    /// Open network connections.
    pub network: bool,
}

impl Default for PackageGrant {
    fn default() -> Self {
        Self {
            save_dialog: true,
            helper: false,
            network: false,
        }
    }
}

impl Default for PackageSettings {
    fn default() -> Self {
        Self {
            check_updates: true,
            disabled: Vec::new(),
            grants: Default::default(),
            stores: vec![STORE_INDEX.to_string()],
        }
    }
}

impl PackageSettings {
    pub fn grant(&self, id: &str) -> PackageGrant {
        self.grants.get(id).copied().unwrap_or_default()
    }

    pub fn enabled(&self, id: &str) -> bool {
        !self.disabled.iter().any(|d| d == id)
    }
}

/// The AI agents a chat can talk to, and how their changes are allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub agents: Vec<AgentSettings>,
    /// A new chat holds each change an agent makes for the user's OK.
    pub ask_before_changes: bool,
    /// What every agent is told to follow, in every document.
    #[serde(default)]
    pub rules: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            agents: Vec::new(),
            ask_before_changes: true,
            rules: String::new(),
        }
    }
}

/// An agent that speaks the Agent Client Protocol: the program to start,
/// its arguments and its environment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// `NAME=value` pairs added to its environment.
    pub env: Vec<(String, String)>,
    /// The session options last chosen in its chats (mode, model, effort
    /// ...), by the agent's option id, applied to each new chat.
    pub choices: std::collections::BTreeMap<String, serde_json::Value>,
}

/// The user's keyboard shortcuts, as changes to the defaults: an action
/// listed here takes these keys instead of its own, and an empty list
/// leaves it without a key. Keys are chords as text (`Ctrl+Shift+S`), by
/// action id (`file.save`, `sketch.line`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardSettings {
    pub bindings: std::collections::BTreeMap<String, Vec<String>>,
}

/// The printer's build volume, drawn around the model on request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintingSettings {
    /// Bed width, depth and build height in millimetres.
    pub bed_mm: [f32; 3],
    /// The bed's origin sits at its centre rather than a corner.
    pub origin_center: bool,
    /// The build volume is drawn in the scene.
    pub show_bed: bool,
    /// The command that opens a model in the slicer. `{file}` stands for
    /// the model's path; without it the path goes last. Empty hands the
    /// file to the system's application for its type.
    pub slicer_command: String,
    /// The format a model goes to the slicer in.
    pub slicer_format: SlicerFormat,
    /// Send to slicer sends the print layout: each part flat on the bed,
    /// as many as the parts list prints, rather than the bodies where
    /// they sit.
    pub slicer_layout: bool,
    /// The gap the print layout leaves between parts, mm.
    pub layout_gap_mm: f32,
}

/// A mesh format a slicer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SlicerFormat {
    /// Named, closed meshes, one object per body.
    #[default]
    ThreeMf,
    /// Triangles only, read by every slicer.
    Stl,
}

impl Default for PrintingSettings {
    fn default() -> Self {
        Self {
            bed_mm: [220.0, 220.0, 250.0],
            origin_center: false,
            show_bed: false,
            slicer_command: String::new(),
            slicer_format: SlicerFormat::default(),
            slicer_layout: false,
            layout_gap_mm: 5.0,
        }
    }
}

/// How ids that were renamed are called now; see [`UserSettings::rename_ids`].
pub struct Renames<'a> {
    /// A workbench id.
    pub workbench: &'a dyn Fn(&str) -> String,
    /// A command, tool or action id.
    pub command: &'a dyn Fn(&str) -> String,
    /// A toolbar group's id.
    pub group: &'a dyn Fn(&str) -> String,
}

impl UserSettings {
    /// Settings kept under ids that were renamed move to the new ones:
    /// a workbench's own settings, keys bound to commands, toolbar groups.
    /// Where both an old and a new entry exist, the new one stays.
    pub fn rename_ids(&mut self, renames: &Renames<'_>) {
        let workbenches = std::mem::take(&mut self.workbenches);
        let (renamed, kept): (Vec<_>, Vec<_>) = workbenches
            .into_iter()
            .partition(|(id, _)| (renames.workbench)(id) != *id);
        self.workbenches.extend(kept);
        for (id, value) in renamed {
            self.workbenches
                .entry((renames.workbench)(&id))
                .or_insert(value);
        }
        let bindings = std::mem::take(&mut self.keyboard.bindings);
        let (renamed, kept): (Vec<_>, Vec<_>) = bindings
            .into_iter()
            .partition(|(id, _)| (renames.command)(id) != *id);
        self.keyboard.bindings.extend(kept);
        for (id, keys) in renamed {
            self.keyboard
                .bindings
                .entry((renames.command)(&id))
                .or_insert(keys);
        }
        for row in &mut self.toolbars.rows {
            for group in row.iter_mut() {
                *group = (renames.group)(group);
            }
        }
    }
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            camera: CameraSettings::default(),
            lighting: LightingSettings::default(),
            rendering: RenderingSettings::default(),
            import: ImportSettings::default(),
            preferred_gpu: None,
            fps_cap: 0.0,
            sixdof: SixDofSettings::default(),
            diagnostics: DiagnosticsSettings::default(),
            printing: PrintingSettings::default(),
            workbenches: std::collections::HashMap::new(),
            keyboard: KeyboardSettings::default(),
            ai: AiSettings::default(),
            packages: PackageSettings::default(),
            toolbars: ToolbarLayout::default(),
            recent_commands: Vec::new(),
            autosave_minutes: default_autosave_minutes(),
            updates: UpdateSettings::default(),
        }
    }
}

/// Files the app writes for a developer rather than for the user: off by
/// default, since they are only worth having when there is someone to send
/// them to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticsSettings {
    /// A STEP or IGES import writes what the reader had to say (the warnings
    /// by kind and in full, the faces that will draw with gaps, what was
    /// skipped) to a file in the temp dir, for the kernel's or printCAD's
    /// developers.
    pub import_report: bool,
}

/// How a 6-DoF mouse (a six-axis navigation puck) drives the view.
///
/// Readings arrive in the device's own units and are divided by
/// [`SixDofSettings::full_scale`] before anything else, so the speeds below
/// are "how far the view moves per second at full deflection" and stay
/// meaningful across devices. The daemon has its own sensitivity, dead zone
/// and inversion settings that act first; these sit on top, for tuning the
/// feel inside the app without changing what every other application sees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SixDofSettings {
    /// Whether a 6-DoF mouse steers the view at all.
    pub enabled: bool,
    /// The reading a fully deflected axis produces.
    pub full_scale: f32,
    /// Deflection below this fraction of full scale is treated as rest.
    pub dead_zone: f32,
    /// How fast each movement drives the view at full deflection, in the
    /// units of whatever it is assigned to.
    pub speed: [f32; 6],
    /// Pass only the axis that moved most, so a gesture stays square.
    pub dominant_axis: bool,
    /// What each of the puck's six movements does to the view, in the order
    /// the device reports them: three pushes, then three turns.
    pub assign: [SixDofMotion; 6],
    /// Which of those six read backwards.
    pub invert: [bool; 6],
    /// What each of the device's buttons does, by button number. Buttons
    /// past the end of the list do nothing.
    pub buttons: Vec<SixDofButtonAction>,
}

impl Default for SixDofSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            full_scale: 350.0,
            dead_zone: 0.02,
            dominant_axis: false,
            // Measured against a SpaceMouse Pro Wireless: axis 0 is the
            // sideways push, 1 the lift, 2 the forward push, 3 the tilt, 4
            // the twist and 5 the rocking.
            assign: [
                SixDofMotion::PanSideways,
                SixDofMotion::Zoom,
                SixDofMotion::PanUpDown,
                SixDofMotion::Tilt,
                SixDofMotion::Roll,
                SixDofMotion::Turn,
            ],
            invert: [false; 6],
            speed: [900.0, 6.0, 900.0, 90.0, 45.0, 90.0],
            buttons: vec![
                SixDofButtonAction::FitView,
                SixDofButtonAction::ToggleProjection,
            ],
        }
    }
}

/// What pressing a button on a 6-DoF mouse does.
///
/// These are the app's own actions. The daemon has button actions of its own
/// (hold rotation or translation at zero, pass only the dominant axis) that
/// act before anything reaches the app, and belong to it rather than here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SixDofButtonAction {
    #[default]
    None,
    /// Frame the whole model.
    FitView,
    ViewFront,
    ViewRear,
    ViewLeft,
    ViewRight,
    ViewTop,
    ViewBottom,
    /// The three-quarter view from the front, top and right.
    ViewIsometric,
    /// Switch between perspective and orthographic.
    ToggleProjection,
}

impl SixDofButtonAction {
    /// Every action, in the order a chooser should list them.
    pub const ALL: [SixDofButtonAction; 10] = [
        SixDofButtonAction::None,
        SixDofButtonAction::FitView,
        SixDofButtonAction::ViewIsometric,
        SixDofButtonAction::ViewFront,
        SixDofButtonAction::ViewRear,
        SixDofButtonAction::ViewLeft,
        SixDofButtonAction::ViewRight,
        SixDofButtonAction::ViewTop,
        SixDofButtonAction::ViewBottom,
        SixDofButtonAction::ToggleProjection,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SixDofButtonAction::None => "Nothing",
            SixDofButtonAction::FitView => "Fit the model",
            SixDofButtonAction::ViewIsometric => "Isometric view",
            SixDofButtonAction::ViewFront => "Front view",
            SixDofButtonAction::ViewRear => "Rear view",
            SixDofButtonAction::ViewLeft => "Left view",
            SixDofButtonAction::ViewRight => "Right view",
            SixDofButtonAction::ViewTop => "Top view",
            SixDofButtonAction::ViewBottom => "Bottom view",
            SixDofButtonAction::ToggleProjection => "Switch projection",
        }
    }
}

/// What one movement of the puck does to the view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SixDofMotion {
    /// The movement is ignored.
    #[default]
    None,
    /// Slide the view left and right.
    PanSideways,
    /// Slide the view up and down.
    PanUpDown,
    /// Move closer and further away.
    Zoom,
    /// Turn about the screen's horizontal.
    Tilt,
    /// Turn about the screen's vertical.
    Turn,
    /// Spin about the direction the view points.
    Roll,
}

impl SixDofMotion {
    /// Every motion, in the order a chooser should list them.
    pub const ALL: [SixDofMotion; 7] = [
        SixDofMotion::None,
        SixDofMotion::PanSideways,
        SixDofMotion::PanUpDown,
        SixDofMotion::Zoom,
        SixDofMotion::Tilt,
        SixDofMotion::Turn,
        SixDofMotion::Roll,
    ];

    /// What one unit of speed means for this motion.
    pub fn speed_unit(self) -> &'static str {
        match self {
            SixDofMotion::None => "",
            SixDofMotion::PanSideways | SixDofMotion::PanUpDown => "px/s",
            SixDofMotion::Zoom => "steps/s",
            SixDofMotion::Tilt | SixDofMotion::Turn | SixDofMotion::Roll => "deg/s",
        }
    }

    /// A speed worth starting from, since the units differ per motion.
    pub fn default_speed(self) -> f32 {
        match self {
            SixDofMotion::None => 0.0,
            SixDofMotion::PanSideways | SixDofMotion::PanUpDown => 900.0,
            SixDofMotion::Zoom => 6.0,
            SixDofMotion::Tilt | SixDofMotion::Turn => 90.0,
            SixDofMotion::Roll => 45.0,
        }
    }

    /// The range a speed field should offer for this motion.
    pub fn speed_range(self) -> std::ops::RangeInclusive<f64> {
        match self {
            SixDofMotion::None => 0.0..=0.0,
            SixDofMotion::PanSideways | SixDofMotion::PanUpDown => 50.0..=4000.0,
            SixDofMotion::Zoom => 0.5..=40.0,
            SixDofMotion::Tilt | SixDofMotion::Turn | SixDofMotion::Roll => 5.0..=360.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SixDofMotion::None => "Nothing",
            SixDofMotion::PanSideways => "Pan sideways",
            SixDofMotion::PanUpDown => "Pan up and down",
            SixDofMotion::Zoom => "Zoom",
            SixDofMotion::Tilt => "Tilt",
            SixDofMotion::Turn => "Turn",
            SixDofMotion::Roll => "Roll",
        }
    }
}

/// Import defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportSettings {
    pub tessellation: kernel_api::TessellationSettings,
}

/// Rendering quality settings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderingSettings {
    /// MSAA sample count (1 = disabled, 2, 4, or 8)
    pub msaa_samples: u8,
    /// Whether to show the in-app log panel at the bottom of the viewport
    pub show_log_panel: bool,
    /// The colour a selected face is painted.
    #[serde(default = "default_selection_color")]
    pub selection_color: [f32; 3],
    /// How much of that paint, up to [`MAX_SELECTION_OPACITY`]: the face
    /// always shows through.
    #[serde(default = "default_selection_opacity")]
    pub selection_opacity: f32,
    /// The colour a feature being edited shows what it adds or takes in:
    /// its faces in it at `preview_opacity`, its edges in it whole.
    #[serde(default = "default_preview_color")]
    pub preview_color: [f32; 3],
    /// How much of that colour a preview's faces take, up to
    /// [`MAX_SELECTION_OPACITY`]: what is behind them shows through.
    #[serde(default = "default_preview_opacity")]
    pub preview_opacity: f32,
    /// How every body in the scene is drawn.
    #[serde(default)]
    pub draw_style: DrawStyle,
    /// Whether the annotations imported files carry (dimensions,
    /// tolerances, datums, notes) are drawn over the scene.
    #[serde(default = "default_show_annotations")]
    pub show_annotations: bool,
    /// Whether a grid lies on the ground plane through the origin, the
    /// plane square to the axis preset's up.
    #[serde(default)]
    pub show_grid: bool,
    /// Whether the origin's three planes (XY, XZ, YZ) draw as see-through
    /// squares around it.
    #[serde(default)]
    pub show_origin_planes: bool,
    /// Colours the user keeps to pick again, in the order added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_colors: Vec<[f32; 3]>,
    /// The largest turn, in degrees, a curved face of a built solid takes
    /// between two drawn facets: smaller draws rounder circles with more
    /// triangles. The solid itself is exact whatever this is.
    #[serde(default = "default_curve_step_deg")]
    pub curve_step_deg: f32,
}

fn default_show_annotations() -> bool {
    true
}

fn default_autosave_minutes() -> u32 {
    5
}

fn default_curve_step_deg() -> f32 {
    10.0
}

/// How the scene draws its bodies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum DrawStyle {
    /// Lit surfaces with their face-boundary edges drawn over them.
    #[default]
    ShadedEdges,
    /// Lit surfaces alone.
    Shaded,
    /// Every triangle as lines, nothing filled.
    Wireframe,
}

impl DrawStyle {
    pub const ALL: [DrawStyle; 3] = [
        DrawStyle::ShadedEdges,
        DrawStyle::Shaded,
        DrawStyle::Wireframe,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DrawStyle::ShadedEdges => "Shaded with edges",
            DrawStyle::Shaded => "Shaded",
            DrawStyle::Wireframe => "Wireframe",
        }
    }
}

/// The most opaque the selection paint gets. Short of fully opaque, so the
/// paint always draws as an overlay over the surface it selects rather than
/// competing with it for the same depth.
pub const MAX_SELECTION_OPACITY: f32 = 0.9;

/// The selection paint: the interface's accent blue, apart from the
/// sketches' white and green and from the hover's amber.
fn default_selection_color() -> [f32; 3] {
    [0.24, 0.56, 1.0]
}

fn default_selection_opacity() -> f32 {
    0.42
}

/// Selection colours earlier versions set as the default. A settings file
/// holding one of them never chose it, so it takes today's default.
const FORMER_SELECTION_COLORS: [[f32; 3]; 1] = [[0.35, 0.95, 0.45]];

fn default_preview_color() -> [f32; 3] {
    [0.10, 0.90, 0.60]
}

fn default_preview_opacity() -> f32 {
    0.35
}

impl RenderingSettings {
    /// A colour that was only ever the default of an earlier version takes
    /// today's default.
    pub fn retire_former_defaults(&mut self) {
        if FORMER_SELECTION_COLORS.contains(&self.selection_color) {
            self.selection_color = default_selection_color();
            self.selection_opacity = default_selection_opacity();
        }
    }
}

impl Default for RenderingSettings {
    fn default() -> Self {
        Self {
            msaa_samples: 4,
            show_log_panel: false,
            selection_color: default_selection_color(),
            selection_opacity: default_selection_opacity(),
            preview_color: default_preview_color(),
            preview_opacity: default_preview_opacity(),
            draw_style: DrawStyle::default(),
            show_annotations: default_show_annotations(),
            show_grid: false,
            show_origin_planes: false,
            custom_colors: Vec::new(),
            curve_step_deg: default_curve_step_deg(),
        }
    }
}

fn default_edge_line_color() -> [f32; 3] {
    [0.08, 0.08, 0.08]
}

fn default_edge_line_width() -> f32 {
    3.0
}

fn default_specular_shininess() -> f32 {
    64.0
}

fn default_specular_intensity() -> f32 {
    0.38
}

/// Settings for the 3D viewport lighting system
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightingSettings {
    pub main_light: LightSource,
    pub backlight: LightSource,
    pub fill_light: LightSource,
    pub ambient_intensity: f32,
    pub ambient_color: [f32; 3],
    /// Blinn-Phong specular exponent (larger = tighter highlight). Typical CAD-style
    /// shaded modes land visually in roughly the 40-96 range for machined metal.
    #[serde(default = "default_specular_shininess")]
    pub specular_shininess: f32,
    /// Specular strength (0 = matte Lambert; ~0.3-0.5 matches the default
    /// shape specular channel in many CAD viewers).
    #[serde(default = "default_specular_intensity")]
    pub specular_intensity: f32,
    /// RGB color (0–1) for face-boundary edge lines drawn over solid bodies.
    #[serde(default = "default_edge_line_color")]
    pub edge_line_color: [f32; 3],
    /// Raster line width in pixels for those edges (clamped to the GPU at draw time).
    #[serde(default = "default_edge_line_width")]
    pub edge_line_width: f32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            main_light: LightSource {
                enabled: true,
                horizontal_angle: 100.0,
                vertical_angle: -46.0,
                color: [0.9, 0.9, 0.9],
                intensity: 0.9,
            },
            backlight: LightSource {
                enabled: true,
                horizontal_angle: -130.0,
                vertical_angle: -10.0,
                color: [0.8, 0.8, 0.85],
                intensity: 0.6,
            },
            fill_light: LightSource {
                enabled: true,
                horizontal_angle: -40.0,
                vertical_angle: 5.0,
                color: [0.7, 0.8, 1.0],
                intensity: 0.4,
            },
            ambient_intensity: 0.2,
            ambient_color: [1.0, 1.0, 1.0],
            specular_shininess: default_specular_shininess(),
            specular_intensity: default_specular_intensity(),
            edge_line_color: default_edge_line_color(),
            edge_line_width: default_edge_line_width(),
        }
    }
}

/// A single light source with direction defined by angles
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightSource {
    pub enabled: bool,
    /// Horizontal angle in degrees (0 = front, 90 = right, -90 = left, 180 = back)
    pub horizontal_angle: f32,
    /// Vertical angle in degrees (0 = horizon, 90 = top, -90 = bottom)
    pub vertical_angle: f32,
    /// RGB color (0.0 - 1.0)
    pub color: [f32; 3],
    /// Intensity multiplier (0.0 - 1.0)
    pub intensity: f32,
}

impl LightSource {
    /// Direction in **CAD navigation canonical** space (Y-up: +X right, +Y up, +Z forward).
    /// For rendering, use [`Self::direction_world`] so Z-up imports match the axis preset.
    pub fn direction(&self) -> [f32; 3] {
        let h = self.horizontal_angle.to_radians();
        let v = self.vertical_angle.to_radians();
        let cos_v = v.cos();
        [
            h.sin() * cos_v, // X
            -v.sin(),        // Y (up)
            h.cos() * cos_v, // Z (forward)
        ]
    }

    /// World-space direction for the active axis preset (`CameraSettings::axis_preset`).
    pub fn direction_world(&self, axis_preset: AxisPreset) -> [f32; 3] {
        AxisSystem::from_preset(axis_preset).canonical_light_direction_world_array(self.direction())
    }
}

/// Camera / navigation preferences (focal-distance viewport model).
///
/// Distances are **millimetres** (printCAD world unit; matches STEP import).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    /// Zoom toward cursor (focal-plane correction after dolly / ortho scale).
    #[serde(default = "default_true")]
    pub zoom_to_cursor: bool,
    #[serde(default)]
    pub invert_zoom: bool,
    /// Per scroll-line multiplicative factor (`factor^steps`). Slightly below 1.0 zooms in.
    #[serde(default = "default_wheel_zoom_factor")]
    pub wheel_zoom_factor: f32,
    #[serde(default)]
    pub orbit_sensitivity: f32,
    /// Orbit rotates about the GPU-picked point under the cursor when a middle-button orbit drag
    /// begins, without reframing (no jump to frame center).
    #[serde(default)]
    pub orbit_pivot_pick: bool,
    #[serde(default = "default_pan_sensitivity")]
    pub pan_sensitivity: f32,
    #[serde(default)]
    pub orbit_yaw_axis: OrbitYawAxis,
    /// Minimum focal distance (mm); prevents dollying through the pivot.
    #[serde(default = "default_min_focal_distance")]
    pub min_focal_distance: f32,
    #[serde(default = "default_max_focal_distance")]
    pub max_focal_distance: f32,
    pub projection: ProjectionMode,
    pub fov_degrees: f32,
    /// World-space height of the ortho frustum (mm), independent of perspective FOV.
    #[serde(default = "default_ortho_height_mm")]
    pub ortho_height_mm: f32,
    #[serde(default = "default_true")]
    pub auto_near_far: bool,
    #[serde(default = "default_near_far_near_ratio")]
    pub near_far_near_ratio: f32,
    #[serde(default = "default_near_far_depth_ratio_cap")]
    pub near_far_depth_ratio_cap: f32,
    #[serde(default = "default_near_far_margin")]
    pub near_far_margin: f32,
    #[serde(default = "default_view_transition_ms")]
    pub view_transition_ms: f32,
    #[serde(default = "default_click_drag_threshold_px")]
    pub click_drag_threshold_px: f32,
    pub axis_preset: AxisPreset,
}

fn default_true() -> bool {
    true
}

fn default_wheel_zoom_factor() -> f32 {
    0.95
}

fn default_pan_sensitivity() -> f32 {
    1.0
}

fn default_min_focal_distance() -> f32 {
    1.0
}

fn default_max_focal_distance() -> f32 {
    5_000.0
}

fn default_ortho_height_mm() -> f32 {
    150.0
}

fn default_near_far_near_ratio() -> f32 {
    0.001
}

fn default_near_far_depth_ratio_cap() -> f32 {
    100_000.0
}

fn default_near_far_margin() -> f32 {
    100.0
}

fn default_view_transition_ms() -> f32 {
    400.0
}

fn default_click_drag_threshold_px() -> f32 {
    4.0
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            zoom_to_cursor: default_true(),
            invert_zoom: false,
            wheel_zoom_factor: default_wheel_zoom_factor(),
            orbit_sensitivity: 0.4,
            orbit_pivot_pick: false,
            pan_sensitivity: default_pan_sensitivity(),
            orbit_yaw_axis: OrbitYawAxis::default(),
            min_focal_distance: default_min_focal_distance(),
            max_focal_distance: default_max_focal_distance(),
            projection: ProjectionMode::Perspective,
            fov_degrees: 50.0,
            ortho_height_mm: default_ortho_height_mm(),
            auto_near_far: default_true(),
            near_far_near_ratio: default_near_far_near_ratio(),
            near_far_depth_ratio_cap: default_near_far_depth_ratio_cap(),
            near_far_margin: default_near_far_margin(),
            view_transition_ms: default_view_transition_ms(),
            click_drag_threshold_px: default_click_drag_threshold_px(),
            axis_preset: AxisPreset::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OrbitYawAxis {
    #[default]
    WorldUp,
    CameraUp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectionMode {
    Perspective,
    Orthographic,
}

/// The page's local storage, where a browser keeps the settings.
#[cfg(target_arch = "wasm32")]
fn page_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub struct SettingsStore {
    path: PathBuf,
}

/// A file of the application's configuration folder, by name. `None` when
/// the system names no configuration folder.
pub fn config_path(name: &str) -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION).map(|dirs| dirs.config_dir().join(name))
}

/// Where workbench packages are installed, one folder each.
pub fn workbenches_dir() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().join("workbenches"))
}

/// The index of the workbench store at `url` as last fetched, for
/// browsing offline: one file per store, named after its address.
pub fn store_cache(url: &str) -> Option<PathBuf> {
    // FNV-1a: the same name for the same address in every run and build.
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION).map(|dirs| {
        dirs.data_dir()
            .join("stores")
            .join(format!("{hash:016x}.json"))
    })
}

/// Where autosaved copies of edited documents wait, until saved, closed
/// or recovered.
pub fn recovery_dir() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().join("recovery"))
}

/// The folder the user's scripts live in: every `.lua` file there is a
/// command of the application. `None` when the system names no
/// configuration folder.
pub fn scripts_dir() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.config_dir().join("scripts"))
}

impl SettingsStore {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Result<Self, SettingsError> {
        let dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or(SettingsError::MissingProjectDirs)?;
        let config_dir = dirs.config_dir();
        fs::create_dir_all(config_dir)?;
        let path = config_dir.join(SETTINGS_FILE);
        Ok(Self { path })
    }

    /// A browser page keeps the settings in its own storage, under the
    /// file's name.
    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Result<Self, SettingsError> {
        Ok(Self {
            path: PathBuf::from(SETTINGS_FILE),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(&self) -> Result<UserSettings, SettingsError> {
        if !self.path.exists() {
            return Ok(UserSettings::default());
        }
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut settings: UserSettings = serde_json::from_reader(reader)?;
        settings.rendering.retire_former_defaults();
        Ok(settings)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn load(&self) -> Result<UserSettings, SettingsError> {
        let Some(text) = page_storage().and_then(|s| s.get_item(SETTINGS_FILE).ok().flatten())
        else {
            return Ok(UserSettings::default());
        };
        let mut settings: UserSettings = serde_json::from_str(&text)?;
        settings.rendering.retire_former_defaults();
        Ok(settings)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, settings: &UserSettings) -> Result<(), SettingsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(&self.path)?;
        serde_json::to_writer_pretty(file, settings)?;
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn save(&self, settings: &UserSettings) -> Result<(), SettingsError> {
        let text = serde_json::to_string(settings)?;
        page_storage()
            .and_then(|s| s.set_item(SETTINGS_FILE, &text).ok())
            .ok_or_else(|| std::io::Error::other("the page's storage is unavailable"))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the chats kept with each document are written.
    pub fn chats_file_path() -> Result<PathBuf, SettingsError> {
        let dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or(SettingsError::MissingProjectDirs)?;
        let config_dir = dirs.config_dir();
        fs::create_dir_all(config_dir)?;
        Ok(config_dir.join("chats.json"))
    }

    pub fn recent_file_path() -> Result<PathBuf, SettingsError> {
        let dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or(SettingsError::MissingProjectDirs)?;
        let config_dir = dirs.config_dir();
        fs::create_dir_all(config_dir)?;
        Ok(config_dir.join(RECENT_FILE_INFO))
    }
}

impl Clone for SettingsStore {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_colour_only_ever_the_default_takes_today_s() {
        let mut old = RenderingSettings {
            selection_color: [0.35, 0.95, 0.45],
            selection_opacity: 0.45,
            ..RenderingSettings::default()
        };
        old.retire_former_defaults();
        assert_eq!(
            old.selection_color,
            RenderingSettings::default().selection_color
        );
        // A colour the user chose stays.
        let mut chosen = RenderingSettings {
            selection_color: [1.0, 0.2, 0.6],
            ..RenderingSettings::default()
        };
        chosen.retire_former_defaults();
        assert_eq!(chosen.selection_color, [1.0, 0.2, 0.6]);
    }

    #[test]
    fn diagnostics_are_off_until_asked_for() {
        assert!(!UserSettings::default().diagnostics.import_report);
        // A settings file written before the field existed still loads, and
        // lands on the default rather than failing.
        let older = serde_json::to_value(UserSettings::default()).unwrap();
        let mut older = older.as_object().unwrap().clone();
        older.remove("diagnostics");
        let loaded: UserSettings =
            serde_json::from_value(serde_json::Value::Object(older)).unwrap();
        assert!(!loaded.diagnostics.import_report);
    }
    #[test]
    fn curves_draw_finer_than_the_kernel_s_default_and_old_files_take_it() {
        let mut older = serde_json::to_value(RenderingSettings::default()).unwrap();
        older.as_object_mut().unwrap().remove("curve_step_deg");
        let loaded: RenderingSettings = serde_json::from_value(older).unwrap();
        assert_eq!(loaded.curve_step_deg, 10.0);
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    #[test]
    fn settings_under_old_ids_move_to_the_new_ones() {
        let mut s = UserSettings::default();
        s.workbenches
            .insert("wb.old".into(), serde_json::json!({"a": 1}));
        s.keyboard
            .bindings
            .insert("old.pad".into(), vec!["P".into()]);
        s.keyboard
            .bindings
            .insert("new.pocket".into(), vec!["Q".into()]);
        s.keyboard
            .bindings
            .insert("old.pocket".into(), vec!["W".into()]);
        s.toolbars.rows = vec![vec!["wb.old/Make".into(), "std.file".into()]];
        s.rename_ids(&Renames {
            workbench: &|id| id.replace("wb.old", "wb.new"),
            command: &|id| id.replace("old.", "new."),
            group: &|id| id.replace("wb.old/", "wb.new/"),
        });
        assert_eq!(
            s.workbenches.get("wb.new"),
            Some(&serde_json::json!({"a": 1}))
        );
        assert!(!s.workbenches.contains_key("wb.old"));
        assert_eq!(
            s.keyboard.bindings.get("new.pad"),
            Some(&vec!["P".to_string()])
        );
        assert_eq!(
            s.keyboard.bindings.get("new.pocket"),
            Some(&vec!["Q".to_string()]),
            "a key set under the new id wins"
        );
        assert!(!s.keyboard.bindings.contains_key("old.pad"));
        assert_eq!(s.toolbars.rows, [vec!["wb.new/Make", "std.file"]]);
    }
}
