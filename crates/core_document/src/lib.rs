pub mod runtime;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use runtime::{
    InputResult, KeyCode, LogEntry, LogLevel, MouseButton, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};

type DocumentResult<T> = std::result::Result<T, DocumentError>;

/// Primary data structure persisted by the application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    metadata: DocumentMetadata,
    history: Vec<DocumentRevision>,
}

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: DocumentMetadata::new(name),
            history: Vec::new(),
        }
    }

    pub fn id(&self) -> Uuid {
        self.metadata.id
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
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
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, _ctx: &WorkbenchRuntimeContext) {}

    /// Draw custom UI in the right panel (properties/inspector area).
    /// Called every frame while this workbench is active.
    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, _ui: &mut egui::Ui, _ctx: &WorkbenchRuntimeContext) {}

    /// Draw custom settings UI in the Settings window.
    /// Called when the Settings window is open and this workbench's tab is selected.
    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, _ui: &mut egui::Ui) -> bool {
        false // Return true if settings changed
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

/// Describes an interactive tool contributed by a workbench.
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub id: String,
    pub label: String,
    pub kind: ToolKind,
}

impl ToolDescriptor {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: ToolKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
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

/// Top-level categories of tools. This will expand as more workbenches land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Sketch,
    PartDesign,
    Utility,
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
}
