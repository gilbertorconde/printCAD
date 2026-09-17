pub mod recent;

use axes::{AxisPreset, AxisSystem};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};
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
    pub spacenav: SpaceNavSettings,
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
            spacenav: SpaceNavSettings::default(),
        }
    }
}

/// How a 6-DoF mouse — a six-axis navigation puck — drives the view.
///
/// Readings arrive in the device's own units and are divided by
/// [`SpaceNavSettings::full_scale`] before anything else, so the speeds below
/// are "how far the view moves per second at full deflection" and stay
/// meaningful across devices. The daemon has its own sensitivity, dead zone
/// and inversion settings that act first; these sit on top, for tuning the
/// feel inside the app without changing what every other application sees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpaceNavSettings {
    /// Whether a 6-DoF mouse steers the view at all.
    pub enabled: bool,
    /// The reading a fully deflected axis produces.
    pub full_scale: f32,
    /// Deflection below this fraction of full scale is treated as rest.
    pub dead_zone: f32,
    /// Pixels per second of pan at full deflection.
    pub translate_speed: f32,
    /// Degrees per second of orbit at full deflection.
    pub rotate_speed: f32,
    /// Zoom steps per second at full deflection.
    pub zoom_speed: f32,
    /// Roll, as a fraction of the orbit speed.
    pub roll_speed: f32,
    /// Push and pull the view sideways and up.
    pub translation: bool,
    /// Tilt and turn the view.
    pub rotation: bool,
    /// Push and pull to zoom.
    pub zoom: bool,
    /// Pass only the axis that moved most, so a gesture stays square.
    pub dominant_axis: bool,
    /// Which of the six axes read backwards, in the order pan-x, pan-y,
    /// zoom, pitch, yaw, roll.
    pub invert: [bool; 6],
    /// What each of the device's buttons does, by button number. Buttons
    /// past the end of the list do nothing.
    pub buttons: Vec<SpaceNavButtonAction>,
}

impl Default for SpaceNavSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            full_scale: 350.0,
            dead_zone: 0.02,
            translate_speed: 900.0,
            rotate_speed: 90.0,
            zoom_speed: 6.0,
            roll_speed: 0.5,
            translation: true,
            rotation: true,
            zoom: true,
            dominant_axis: false,
            invert: [false; 6],
            buttons: vec![
                SpaceNavButtonAction::FitView,
                SpaceNavButtonAction::ToggleProjection,
            ],
        }
    }
}

/// What pressing a button on a 6-DoF mouse does.
///
/// These are the app's own actions. The daemon has button actions of its own
/// — hold rotation or translation at zero, pass only the dominant axis — that
/// act before anything reaches the app, and belong to it rather than here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceNavButtonAction {
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

impl SpaceNavButtonAction {
    /// Every action, in the order a chooser should list them.
    pub const ALL: [SpaceNavButtonAction; 10] = [
        SpaceNavButtonAction::None,
        SpaceNavButtonAction::FitView,
        SpaceNavButtonAction::ViewIsometric,
        SpaceNavButtonAction::ViewFront,
        SpaceNavButtonAction::ViewRear,
        SpaceNavButtonAction::ViewLeft,
        SpaceNavButtonAction::ViewRight,
        SpaceNavButtonAction::ViewTop,
        SpaceNavButtonAction::ViewBottom,
        SpaceNavButtonAction::ToggleProjection,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SpaceNavButtonAction::None => "Nothing",
            SpaceNavButtonAction::FitView => "Fit the model",
            SpaceNavButtonAction::ViewIsometric => "Isometric view",
            SpaceNavButtonAction::ViewFront => "Front view",
            SpaceNavButtonAction::ViewRear => "Rear view",
            SpaceNavButtonAction::ViewLeft => "Left view",
            SpaceNavButtonAction::ViewRight => "Right view",
            SpaceNavButtonAction::ViewTop => "Top view",
            SpaceNavButtonAction::ViewBottom => "Bottom view",
            SpaceNavButtonAction::ToggleProjection => "Switch projection",
        }
    }
}

/// Which camera motion each device axis drives. The order is the one the
/// device reports: three translations, then three rotations.
pub const SPACENAV_AXIS_LABELS: [&str; 6] = [
    "Pan sideways",
    "Pan up and down",
    "Zoom",
    "Tilt",
    "Turn",
    "Roll",
];

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
}

impl Default for RenderingSettings {
    fn default() -> Self {
        Self {
            msaa_samples: 4, // 4x MSAA by default
            show_log_panel: false,
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
    /// Specular strength (0 = matte Lambert like before; ~0.3-0.5 matches
    /// default shape specular channel in many CAD viewers).
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
            edge_line_color: [0.08, 0.08, 0.08],
            edge_line_width: 3.0,
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

    /// World-space direction for the active axis preset (`UserSettings.axis_preset`).
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
    #[serde(default)]
    pub navigation_style: NavigationStyle,
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
    /// Orbit rotates about the GPU-picked point under the cursor when an LMB orbit drag begins,
    /// without reframing (no jump to frame center).
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
            navigation_style: NavigationStyle::default(),
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
pub enum NavigationStyle {
    #[default]
    Gesture,
    /// Reserved for future remappable styles.
    Cad,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OrbitYawAxis {
    WorldUp,
    #[default]
    CameraUp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectionMode {
    Perspective,
    Orthographic,
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new() -> Result<Self, SettingsError> {
        let dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or(SettingsError::MissingProjectDirs)?;
        let config_dir = dirs.config_dir();
        fs::create_dir_all(config_dir)?;
        let path = config_dir.join(SETTINGS_FILE);
        Ok(Self { path })
    }

    pub fn load(&self) -> Result<UserSettings, SettingsError> {
        if !self.path.exists() {
            return Ok(UserSettings::default());
        }
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let settings = serde_json::from_reader(reader)?;
        Ok(settings)
    }

    pub fn save(&self, settings: &UserSettings) -> Result<(), SettingsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(&self.path)?;
        serde_json::to_writer_pretty(file, settings)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
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
