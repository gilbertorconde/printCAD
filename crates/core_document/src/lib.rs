pub mod asset;
pub mod feature;
pub mod registration;
pub mod runtime;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json;
use tar::{Archive, Builder, Header};
use thiserror::Error;
use uuid::Uuid;

pub use asset::{AssetReference, AssetType};
pub use feature::{BodyId, FeatureError, FeatureId, FeatureNode, FeatureTree, WorkbenchFeature};
pub use runtime::{
    CameraOrientRequest, InputResult, KeyCode, LogEntry, LogLevel, MouseButton,
    WorkbenchInputEvent, WorkbenchRuntimeContext,
};

/// Result type for document operations.
pub type DocumentResult<T> = std::result::Result<T, DocumentError>;

/// Type-erased storage for workbench-specific data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchStorage {
    /// Workbench ID this storage belongs to.
    pub workbench_id: WorkbenchId,
    /// Arbitrary JSON data (workbench-specific).
    pub data: serde_json::Value,
}

impl WorkbenchStorage {
    pub fn new(workbench_id: WorkbenchId, data: serde_json::Value) -> Self {
        Self { workbench_id, data }
    }
}

/// Primary data structure persisted by the application.
///
/// The document is saved as a `.prtcad` file, which is a ZIP archive containing:
/// - `document.json` - This document structure (serialized)
/// - `assets/` - External files (STEP, STL, etc.) referenced by the document
/// - `cache/` - Optional cached computed data (meshes, tessellations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    metadata: DocumentMetadata,
    feature_tree: FeatureTree,
    bodies: Vec<Body>,
    /// Workbench-specific data storage (type-erased).
    workbench_storage: HashMap<String, WorkbenchStorage>,
    /// References to external files stored in the .prtcad archive.
    assets: HashMap<Uuid, AssetReference>,
    history: Vec<DocumentRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub created_at: i64,
}

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: DocumentMetadata::new(name),
            feature_tree: FeatureTree::new(),
            bodies: Vec::new(),
            workbench_storage: HashMap::new(),
            assets: HashMap::new(),
            history: Vec::new(),
        }
    }

    pub fn id(&self) -> Uuid {
        self.metadata.id
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.metadata.name = name.into();
    }

    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }

    pub fn mark_dirty(&mut self) {
        self.metadata.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.metadata.dirty = false;
    }

    pub fn push_revision(&mut self, revision: DocumentRevision) {
        self.history.push(revision);
        self.metadata.revision += 1;
    }

    /// Add a feature to the tree without attaching it to a body.
    /// For body-scoped features, prefer `add_feature_in_body`.
    pub fn add_feature<F: WorkbenchFeature>(
        &mut self,
        feature: F,
        name: String,
    ) -> DocumentResult<FeatureId> {
        self.add_feature_in_body(feature, name, None)
    }

    /// Add a feature to the tree and optionally attach it to a body for hierarchy purposes.
    pub fn add_feature_in_body<F: WorkbenchFeature>(
        &mut self,
        feature: F,
        name: String,
        body: Option<BodyId>,
    ) -> DocumentResult<FeatureId> {
        let id = FeatureId::new();
        let deps = feature.dependencies();

        let node = FeatureNode {
            id,
            workbench_id: F::workbench_id(),
            name,
            body,
            visible: true,
            suppressed: false,
            dirty: false,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            data: feature.to_json(),
        };

        self.feature_tree.add_node(node);

        // Add dependencies
        for dep in deps {
            self.feature_tree.add_dependency(id, dep);
        }

        self.mark_dirty();
        Ok(id)
    }

    /// Get feature data (returns JSON, workbench must deserialize).
    pub fn get_feature_data(&self, id: FeatureId) -> Option<&serde_json::Value> {
        self.feature_tree.get_node(id).map(|n| &n.data)
    }

    /// Get feature metadata (id, name, dirty, etc.).
    pub fn get_feature_meta(&self, id: FeatureId) -> Option<&FeatureNode> {
        self.feature_tree.get_node(id)
    }

    /// Update feature data (workbench provides serialized JSON).
    pub fn update_feature_data(
        &mut self,
        id: FeatureId,
        data: serde_json::Value,
    ) -> DocumentResult<()> {
        if let Some(node) = self.feature_tree.get_node_mut(id) {
            node.data = data;
            self.mark_dirty();
            Ok(())
        } else {
            Err(DocumentError::FeatureNotFound(id))
        }
    }

    /// Mark feature dirty (triggers recomputation).
    pub fn mark_feature_dirty(&mut self, feature_id: FeatureId) {
        self.feature_tree.mark_dirty(feature_id);
        self.mark_dirty();
    }

    /// Get all dirty features.
    pub fn dirty_features(&self) -> Vec<FeatureId> {
        self.feature_tree.dirty_features()
    }

    /// Get recomputation order for dirty features.
    pub fn recompute_order(&self) -> Vec<FeatureId> {
        let dirty = self.dirty_features();
        self.feature_tree.recompute_order(&dirty)
    }

    /// Get workbench storage.
    pub fn get_workbench_storage(&self, wb_id: &WorkbenchId) -> Option<&WorkbenchStorage> {
        self.workbench_storage.get(wb_id.as_str())
    }

    /// Get mutable workbench storage.
    pub fn get_workbench_storage_mut(
        &mut self,
        wb_id: &WorkbenchId,
    ) -> Option<&mut WorkbenchStorage> {
        self.workbench_storage.get_mut(wb_id.as_str())
    }

    /// Set workbench storage.
    pub fn set_workbench_storage(&mut self, wb_id: WorkbenchId, data: serde_json::Value) {
        self.workbench_storage.insert(
            wb_id.as_str().to_string(),
            WorkbenchStorage::new(wb_id, data),
        );
        self.mark_dirty();
    }

    /// Get the feature tree.
    pub fn feature_tree(&self) -> &FeatureTree {
        &self.feature_tree
    }

    /// Get mutable feature tree.
    pub fn feature_tree_mut(&mut self) -> &mut FeatureTree {
        &mut self.feature_tree
    }

    /// All document bodies.
    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    /// Returns true if the document contains at least one body.
    pub fn has_bodies(&self) -> bool {
        !self.bodies.is_empty()
    }

    /// Create a new body entry in the document.
    pub fn create_body(&mut self, name: Option<String>) -> BodyId {
        let id = BodyId::new();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let body_name = match name {
            Some(explicit) => explicit,
            None => next_indexed_name("body", self.bodies.iter().map(|b| b.name.as_str())),
        };
        let body = Body {
            id,
            name: body_name,
            created_at,
        };
        self.bodies.push(body);
        self.mark_dirty();
        id
    }

    /// Add an asset reference to the document.
    pub fn add_asset(&mut self, asset: AssetReference) -> Uuid {
        let id = asset.id;
        self.assets.insert(id, asset);
        self.mark_dirty();
        id
    }

    /// Get an asset reference by ID.
    pub fn get_asset(&self, asset_id: Uuid) -> Option<&AssetReference> {
        self.assets.get(&asset_id)
    }

    /// Get asset path within the archive.
    pub fn get_asset_path(&self, asset_id: Uuid) -> Option<&str> {
        self.assets.get(&asset_id).map(|a| a.path.as_str())
    }

    /// Get all assets.
    pub fn assets(&self) -> impl Iterator<Item = &AssetReference> {
        self.assets.values()
    }

    /// Save document to a .prtcad file (tar archive, optionally compressed).
    pub fn save_to_file(&self, path: &Path, compression: Compression) -> DocumentResult<()> {
        let file = File::create(path)?;

        match compression {
            Compression::None => {
                let mut builder = Builder::new(file);
                Self::write_archive(&mut builder, self)?;
                builder.finish()?;
            }
            Compression::Gzip => {
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                let mut builder = Builder::new(encoder);
                Self::write_archive(&mut builder, self)?;
                let encoder = builder.into_inner().map_err(|e| {
                    DocumentError::Compression(format!("gzip encoder finalize failed: {e}"))
                })?;
                encoder.finish()?;
            }
            Compression::Zstd => {
                let mut encoder = zstd::Encoder::new(file, 0)
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                {
                    let mut builder = Builder::new(&mut encoder);
                    Self::write_archive(&mut builder, self)?;
                    builder.finish()?;
                }
                encoder
                    .finish()
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Load document from a .prtcad file (auto-detects compression).
    pub fn load_from_file(path: &Path) -> DocumentResult<Self> {
        let mut file = File::open(path)?;

        // Detect compression via extension and magic bytes.
        let mut magic = [0u8; 4];
        let _n = file.read(&mut magic)?;
        file.rewind()?;

        // Decide compression based on file name and magic bytes.
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let compression = if file_name.ends_with(".gz")
            || file_name.ends_with(".prtcad.gz")
            || magic.starts_with(&[0x1f, 0x8b])
        {
            Compression::Gzip
        } else if file_name.ends_with(".zst") || file_name.ends_with(".prtcad.zst") {
            Compression::Zstd
        } else {
            Compression::None
        };

        let mut archive: Archive<Box<dyn Read>> = match compression {
            Compression::None => Archive::new(Box::new(file)),
            Compression::Gzip => {
                let decoder = flate2::read::GzDecoder::new(file);
                Archive::new(Box::new(decoder))
            }
            Compression::Zstd => {
                let decoder = zstd::Decoder::new(file)
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                Archive::new(Box::new(decoder))
            }
        };

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;
            if path == Path::new("document.json") {
                let mut buf = String::new();
                entry.read_to_string(&mut buf)?;
                let doc: Document = serde_json::from_str(&buf)?;
                return Ok(doc);
            }
        }

        Err(DocumentError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "document.json not found in archive",
        )))
    }

    fn write_archive<W: Write>(builder: &mut Builder<W>, doc: &Document) -> DocumentResult<()> {
        let json = serde_json::to_vec_pretty(doc)?;
        let mut header = Header::new_gnu();
        header.set_path("document.json")?;
        header.set_size(json.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &json[..])?;
        Ok(())
    }
}

fn next_indexed_name<'a>(base: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let mut max_suffix: Option<u32> = None;

    for name in existing {
        if name.eq_ignore_ascii_case(base) {
            max_suffix = Some(max_suffix.map_or(0, |m| m.max(0)));
        } else if let Some(rest) = name
            .to_ascii_lowercase()
            .strip_prefix(&(base.to_ascii_lowercase() + "_"))
        {
            if let Ok(n) = rest.parse::<u32>() {
                max_suffix = Some(max_suffix.map_or(n, |m| m.max(n)));
            }
        }
    }

    let new_suffix = match max_suffix {
        None => 0,
        Some(m) => m.saturating_add(1),
    };

    if new_suffix == 0 {
        base.to_string()
    } else {
        format!("{base}_{new_suffix}")
    }
}

/// Lightweight metadata block stored alongside the document payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    id: Uuid,
    name: String,
    revision: u64,
    dirty: bool,
}

impl DocumentMetadata {
    fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            revision: 0,
            dirty: false,
        }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }
}

/// Snapshot representing a committed state of the document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRevision {
    pub message: String,
    pub timestamp_epoch_ms: i64,
}

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
    /// Returns a vector of (mesh, color) tuples where:
    /// - mesh: The triangular mesh to render
    /// - color: RGB color [r, g, b] in range 0.0-1.0
    ///
    /// These meshes are rendered in 3D world space and will scale with zoom and rotate with the camera.
    /// For constant-thickness lines that don't change with zoom/rotation, use `get_screen_space_overlays` instead.
    /// Default implementation returns empty vector.
    fn get_overlay_meshes(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<(kernel_api::TriMesh, [f32; 3])> {
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

/// Central registry tracking workbenches and their declared capabilities.
#[derive(Default)]
pub struct DocumentService {
    workbenches: HashMap<String, WorkbenchEntry>,
}

struct WorkbenchEntry {
    descriptor: WorkbenchDescriptor,
    workbench: Box<dyn Workbench>,
    context: WorkbenchContext,
}

impl DocumentService {
    pub fn register_workbench(&mut self, workbench: Box<dyn Workbench>) -> DocumentResult<()> {
        let descriptor = workbench.descriptor();
        if self.workbenches.contains_key(descriptor.id.as_str()) {
            return Err(DocumentError::WorkbenchExists(
                descriptor.id.as_str().to_owned(),
            ));
        }

        let mut context = WorkbenchContext::default();
        workbench.configure(&mut context);

        self.workbenches.insert(
            descriptor.id.as_str().to_owned(),
            WorkbenchEntry {
                descriptor,
                workbench,
                context,
            },
        );

        Ok(())
    }

    pub fn workbench_descriptors(&self) -> impl Iterator<Item = &WorkbenchDescriptor> {
        self.workbenches.values().map(|entry| &entry.descriptor)
    }

    pub fn tools_for(&self, id: &WorkbenchId) -> DocumentResult<&[ToolDescriptor]> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.context.tools())
    }

    pub fn commands_for(&self, id: &WorkbenchId) -> DocumentResult<&[CommandDescriptor]> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.context.commands())
    }

    pub fn workbench(&self, id: &WorkbenchId) -> DocumentResult<&dyn Workbench> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.workbench.as_ref())
    }

    pub fn workbench_mut(&mut self, id: &WorkbenchId) -> DocumentResult<&mut Box<dyn Workbench>> {
        let entry = self
            .workbenches
            .get_mut(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(&mut entry.workbench)
    }
}

/// Errors surfaced when interacting with documents or workbench registries.
#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("workbench `{0}` already registered")]
    WorkbenchExists(String),
    #[error("workbench `{0}` is not registered")]
    WorkbenchMissing(String),
    #[error("document serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("feature not found: {0:?}")]
    FeatureNotFound(FeatureId),
    #[error("feature error: {0}")]
    Feature(#[from] FeatureError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("compression error: {0}")]
    Compression(String),
}

#[derive(Debug, Clone, Copy)]
pub enum Compression {
    None,
    Gzip,
    Zstd,
}
