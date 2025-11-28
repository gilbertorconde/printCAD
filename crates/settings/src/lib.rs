use axes::AxisPreset;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    pub camera: CameraSettings,
    pub lighting: LightingSettings,
    pub rendering: RenderingSettings,
    /// Preferred GPU name substring for Vulkan device selection (None = automatic)
    pub preferred_gpu: Option<String>,
    /// Optional FPS cap. 0.0 = uncapped (driven by vsync / driver).
    pub fps_cap: f32,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            camera: CameraSettings::default(),
            lighting: LightingSettings::default(),
            rendering: RenderingSettings::default(),
            preferred_gpu: None,
            fps_cap: 0.0,
        }
    }
}

/// Rendering quality settings
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Settings for the 3D viewport lighting system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightingSettings {
    pub main_light: LightSource,
    pub backlight: LightSource,
    pub fill_light: LightSource,
    pub ambient_intensity: f32,
    pub ambient_color: [f32; 3],
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
        }
    }
}

/// A single light source with direction defined by angles
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Convert angles to a normalized direction vector
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraSettings {
    pub orbit_button: MouseButtonSetting,
    pub pan_button: MouseButtonSetting,
    pub orbit_sensitivity: f32,
    pub zoom_sensitivity: f32,
    pub invert_zoom: bool,
    pub min_distance: f32,
    pub max_distance: f32,
    pub projection: ProjectionMode,
    pub fov_degrees: f32,
    pub axis_preset: AxisPreset,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            orbit_button: MouseButtonSetting::Right,
            pan_button: MouseButtonSetting::Middle,
            orbit_sensitivity: 0.4,
            zoom_sensitivity: 0.15,
            invert_zoom: false,
            min_distance: 0.2,
            max_distance: 500.0,
            projection: ProjectionMode::Perspective,
            fov_degrees: 50.0,
            axis_preset: AxisPreset::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectionMode {
    Perspective,
    Orthographic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MouseButtonSetting {
    Left,
    Middle,
    Right,
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
