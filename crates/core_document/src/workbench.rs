//! The workbench plugin surface: trait, descriptors, tools, and commands.
//!
//! This lives in `core_document` (not the `workbenches` crate) so that
//! third-party workbenches only need to depend on this crate; see
//! `docs/WORKBENCH_GUIDE.md`.

use serde::{Deserialize, Serialize};

use crate::runtime::{InputResult, WorkbenchInputEvent, WorkbenchRuntimeContext};
use crate::FeatureId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkbenchId(String);

impl WorkbenchId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WorkbenchId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A screen-space overlay line segment for constant-thickness visualization.
///
/// Screen-space overlays are rendered as 2D lines in screen coordinates, maintaining
/// constant thickness regardless of zoom or camera rotation. Ideal for grid lines,
/// guides, and reference geometry.
#[derive(Debug, Clone)]
pub struct ScreenSpaceOverlay {
    /// Starting point in screen coordinates (x, y) in pixels, relative to viewport origin.
    pub start: [f32; 2],
    /// Ending point in screen coordinates (x, y) in pixels, relative to viewport origin.
    pub end: [f32; 2],
    /// RGB color [r, g, b] in range 0.0-1.0.
    pub color: [f32; 3],
    /// Line thickness in pixels (constant screen-space).
    pub thickness: f32,
}

impl ScreenSpaceOverlay {
    /// Create a new screen-space overlay line.
    pub fn new(start: [f32; 2], end: [f32; 2], color: [f32; 3], thickness: f32) -> Self {
        Self {
            start,
            end,
            color,
            thickness,
        }
    }
}

/// A screen-space text label rendered in the viewport (dimension values,
/// constraint glyphs, on-view parameter readouts).
#[derive(Debug, Clone)]
pub struct ScreenSpaceLabel {
    /// Label center in screen coordinates (x, y) in pixels, relative to the
    /// viewport origin.
    pub pos: [f32; 2],
    pub text: String,
    /// RGB color [r, g, b] in range 0.0-1.0.
    pub color: [f32; 3],
    /// Font size in pixels.
    pub size: f32,
    /// Draw a rounded background pill behind the text (dimension values).
    pub background: bool,
}

/// User-facing description provided by workbenches to populate menus.
#[derive(Debug, Clone)]
pub struct WorkbenchDescriptor {
    pub id: WorkbenchId,
    pub label: String,
    pub description: String,
}

impl WorkbenchDescriptor {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: WorkbenchId::new(id),
            label: label.into(),
            description: description.into(),
        }
    }
}

/// Trait implemented by all workbench plugins.
///
/// Workbenches declare their tools/commands via `configure`, and can optionally
/// implement runtime hooks for input handling, per-frame updates, and custom UI.
pub trait Workbench: Send {
    /// Returns metadata describing this workbench.
    fn descriptor(&self) -> WorkbenchDescriptor;

    /// Called once at registration to declare tools and commands.
    fn configure(&self, context: &mut WorkbenchContext);

    /// Called when this workbench becomes active.
    fn on_activate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when this workbench is deactivated (another WB becomes active).
    fn on_deactivate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called every frame while this workbench is active.
    fn on_frame(&mut self, _dt: f32, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when an input event occurs while this workbench is active.
    /// Return `InputResult::consumed()` to prevent further event propagation.
    fn on_input(
        &mut self,
        _event: &WorkbenchInputEvent,
        _active_tool: Option<&str>,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        InputResult::ignored()
    }

    /// Draw custom UI in the left panel (below the tool list).
    /// Called every frame while this workbench is active.
    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Draw custom UI in the right panel (properties/inspector area).
    /// Called every frame while this workbench is active.
    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, _ui: &mut egui::Ui, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Whether this workbench exposes right-panel UI.
    #[cfg(feature = "egui")]
    fn wants_right_panel(&self) -> bool {
        false
    }

    /// Check if a tool is enabled given the current runtime context.
    /// Called by the UI to determine if a tool button should be enabled/disabled.
    /// Default implementation returns true for all tools.
    fn is_tool_enabled(&self, _tool_id: &str, _ctx: &WorkbenchRuntimeContext) -> bool {
        true
    }

    /// Draw custom settings UI in the Settings window.
    /// Called when the Settings window is open and this workbench's tab is selected.
    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, _ui: &mut egui::Ui) -> bool {
        false // Return true if settings changed
    }

    /// Finish/close the current editing session (e.g., finish sketch).
    /// Called when the user requests to finish editing (e.g., via UI button).
    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Deserialize a feature of this workbench's type from JSON.
    /// Called by the document when loading features from storage.
    /// Returns None if the feature type doesn't belong to this workbench.
    fn deserialize_feature(
        &self,
        _workbench_id: &WorkbenchId,
        _data: &serde_json::Value,
    ) -> Option<Box<dyn std::any::Any>> {
        None // Default: no feature deserialization
    }

    /// Get feature dependencies from serialized feature data.
    /// Used by the document to build the dependency graph.
    fn feature_dependencies(
        &self,
        _workbench_id: &WorkbenchId,
        _data: &serde_json::Value,
    ) -> Vec<FeatureId> {
        Vec::new() // Default: no dependencies
    }

    /// Get additional render meshes for overlay/helper visualization.
    /// Called every frame to allow workbenches to contribute visual aids (grid lines, guides, etc.).
    /// Returns a vector of (mesh, color, is_wireframe) tuples where:
    /// - mesh: The triangular mesh to render
    /// - color: RGB color [r, g, b] in range 0.0-1.0
    /// - is_wireframe: If true, render as wireframe with depth bias (appears on top of solid geometry)
    ///
    /// These meshes are rendered in 3D world space and will scale with zoom and rotate with the camera.
    /// For constant-thickness lines that don't change with zoom/rotation, use `get_screen_space_overlays` instead.
    /// Default implementation returns empty vector.
    fn get_overlay_meshes(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<(kernel_api::TriMesh, [f32; 3], bool)> {
        Vec::new()
    }

    /// Get screen-space overlays for constant-thickness visualization.
    /// Called every frame to allow workbenches to contribute visual aids that maintain
    /// constant screen-space thickness regardless of zoom or camera rotation.
    ///
    /// Screen-space overlays are rendered as 2D lines in screen coordinates, making them
    /// ideal for grid lines, guides, and other reference geometry that should remain visible
    /// and maintain consistent appearance regardless of camera position.
    ///
    /// Returns a vector of screen-space line segments where:
    /// - start: Starting point in screen coordinates (x, y) in pixels, relative to viewport origin
    /// - end: Ending point in screen coordinates (x, y) in pixels, relative to viewport origin
    /// - color: RGB color [r, g, b] in range 0.0-1.0
    /// - thickness: Line thickness in pixels (constant screen-space)
    ///
    /// Default implementation returns empty vector.
    fn get_screen_space_overlays(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceOverlay> {
        Vec::new()
    }

    /// Get screen-space text labels for constant-size viewport annotations
    /// (dimension values, constraint glyphs, on-view parameter readouts).
    /// Same coordinate convention as [`Self::get_screen_space_overlays`].
    fn get_screen_space_labels(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceLabel> {
        Vec::new()
    }
}

/// Registry used by workbenches to declare the tools/commands they expose.
#[derive(Debug, Default)]
pub struct WorkbenchContext {
    tools: Vec<ToolDescriptor>,
    commands: Vec<CommandDescriptor>,
}

impl WorkbenchContext {
    pub fn register_tool(&mut self, tool: ToolDescriptor) {
        self.tools.push(tool);
    }

    pub fn register_command(&mut self, command: CommandDescriptor) {
        self.commands.push(command);
    }

    pub fn tools(&self) -> &[ToolDescriptor] {
        &self.tools
    }

    pub fn commands(&self) -> &[CommandDescriptor] {
        &self.commands
    }
}

/// Describes how a tool button should behave in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolBehavior {
    /// Radio button behavior: only one tool in the same group can be active at a time.
    /// Clicking an active tool deactivates it. Tools in different groups are independent.
    /// This is the default.
    #[default]
    Radio,
    /// Check button behavior: independent toggle. Each tool can be on or off independently.
    /// Multiple check tools can be active simultaneously.
    Check,
    /// Action button behavior: fire-and-forget. Clicking triggers the action
    /// but doesn't keep the tool "active". The tool is cleared after handling.
    Action,
}

/// Describes an interactive tool contributed by a workbench.
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub id: String,
    pub label: String,
    /// Optional category for grouping/organization (e.g., "drawing", "modeling", "utility").
    /// This is informational and doesn't affect behavior.
    pub category: Option<String>,
    /// How the tool button should behave in the UI.
    pub behavior: ToolBehavior,
    /// Optional group name for Radio tools. Tools in the same group are mutually exclusive.
    /// Only one tool per group can be active at a time. If None, each tool is its own group.
    /// Ignored for Check and Action tools.
    pub group: Option<String>,
}

impl ToolDescriptor {
    /// Create a new tool descriptor with radio button behavior (default).
    /// Tools in the same group are mutually exclusive.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Radio,
            group: None, // Each tool is its own group by default
        }
    }

    /// Create a new tool descriptor with radio button behavior in a specific group.
    /// Tools in the same group are mutually exclusive.
    pub fn new_radio_group(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
        group: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Radio,
            group: Some(group.into()),
        }
    }

    /// Create a new tool descriptor with check button behavior.
    /// Check tools are independent - multiple can be active simultaneously.
    pub fn new_check(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Check,
            group: None, // Groups don't apply to Check tools
        }
    }

    /// Create a new tool descriptor with action button behavior.
    pub fn new_action(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Action,
            group: None, // Groups don't apply to Action tools
        }
    }
}

/// Simple metadata for commands that may be bound to shortcuts or macros.
#[derive(Debug, Clone)]
pub struct CommandDescriptor {
    pub id: String,
    pub label: String,
}

impl CommandDescriptor {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}
