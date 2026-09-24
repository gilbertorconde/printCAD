//! Actions the UI requests from the app, emitted once on the frame they
//! were triggered. Adding a new UI action = one enum variant here + one
//! match arm in `PrintCadApp::apply_ui_commands`.

use core_document::TaskOutcome;
use settings::ProjectionMode;

use super::ActiveWorkbench;
use super::feature_tree::{TreeFeatureCommand, TreeItemId};
use crate::orientation_cube::{CameraSnapView, RotateDelta};

/// The Edit menu's clipboard entries; the active bench answers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Cut,
    Copy,
    Paste,
}

impl EditCommand {
    /// The command id the bench receives, in `MenuScope::EditMenu`.
    pub fn id(self) -> &'static str {
        match self {
            EditCommand::Cut => "edit.cut",
            EditCommand::Copy => "edit.copy",
            EditCommand::Paste => "edit.paste",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCommand {
    New,
    Open,
    Save,
    SaveAs,
    ImportStep,
    /// The export dialog: the bodies written as STEP, STL or 3MF.
    Export,
    /// Every visible body written for the slicer and opened in it.
    SendToSlicer,
    /// Pick a Lua script and run it.
    RunScript,
}

/// What a start-page NEW card creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartKind {
    /// A document with one body, in the bench new documents land in.
    Landing,
    /// The same, then one of a bench's start-page commands run in that
    /// bench (the sketcher's "Empty sketch" opens an XY sketch).
    Bench {
        workbench: core_document::WorkbenchId,
        command: String,
    },
    /// A document with one of the bundled example scenes on its body.
    Example(bench_fixtures::Scene),
    /// The exporting walkthrough: the pocketed example, and the export
    /// dialog once its solid is built.
    ExportWalkthrough,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    File(FileCommand),
    Quit,
    FitView,
    /// Frame the selected body, or the active one.
    FitSelection,
    SetDrawStyle(settings::DrawStyle),
    /// The print bed drawn around the model, or not.
    TogglePrintBed,
    /// Arm the measure tool, or put it away.
    ToggleMeasure,
    Edit(EditCommand),
    /// A body's own look, or `None` for the one it came with.
    SetBodyDisplay {
        body: core_document::BodyId,
        display: Option<core_document::BodyDisplay>,
    },
    /// The tree opens its way to this body and scrolls to it.
    RevealInTree(core_document::BodyId),
    /// The whole body, as a double click would.
    SelectBody(core_document::BodyId),
    /// The viewport's context menu was dismissed or used.
    CloseViewportMenu,
    /// A workbench panel hook asked the host for something.
    HostRequest(core_document::HostRequest),
    /// Back to the workspace of the tab on screen, from its start page.
    ShowWorkspace,
    /// A blank tab, on the start page.
    NewTab,
    CloseTab(uuid::Uuid),
    SelectTab(uuid::Uuid),
    /// The next (`1`) or previous (`-1`) tab, wrapping.
    CycleTab(i32),
    /// One of a bench's own menu entries was picked.
    BenchCommand {
        workbench: core_document::WorkbenchId,
        id: String,
        scope: core_document::MenuScope,
    },
    CameraSnap(CameraSnapView),
    CameraRotate(RotateDelta),
    /// Stop the kernel job that is running now.
    CancelKernelJob,
    /// The Preferences dialog applied its draft.
    CommitSettings {
        settings: Box<settings::UserSettings>,
        display_unit: core_document::Unit,
    },
    SelectTreeItem(TreeItemId),
    ActivateTreeItem(TreeItemId),
    /// History context-menu action on a tree feature row.
    TreeFeature {
        feature: core_document::FeatureId,
        command: TreeFeatureCommand,
    },
    SetImportedVisibility {
        node: uuid::Uuid,
        visible: bool,
    },
    /// Set the perspective's field of view, keeping the framing; `settled`
    /// when the edit is done and the setting is to be saved.
    SetFieldOfView {
        degrees: f32,
        settled: bool,
    },
    /// Show or hide a body, from its tree row, the property panel or the
    /// viewport's menu.
    SetBodyVisible {
        body: core_document::BodyId,
        visible: bool,
    },
    ConfirmStepImport,
    CancelStepImport,
    ConfirmExport,
    CancelExport,
    SwitchWorkbench {
        from: ActiveWorkbench,
        to: ActiveWorkbench,
    },
    Undo,
    Redo,
    ToggleLogPanel,
    /// Orbit around the point on the focal plane under the cursor.
    PivotAtCursor,
    /// Run a workbench's keyboard action, bound to a key.
    BenchAction {
        workbench: core_document::WorkbenchId,
        id: String,
    },
    SetProjection(ProjectionMode),
    /// Turn the clipping plane on, move it, or (`None`) put it away.
    SetSection(Option<crate::camera::section::SectionToggle>),
    /// Mark every part feature dirty so the next frame rebuilds them all.
    RecomputeAll,
    /// Run a line typed in the script console.
    RunConsole(String),
    /// Run a Lua script file.
    RunScriptFile(std::path::PathBuf),
    /// Stop the running script.
    StopScript,
    /// Start recording what is done through the UI as a script, or stop
    /// and save the recording.
    ToggleRecording,
    /// What a panel hook recorded.
    Recorded(Vec<core_document::Recorded>),
    /// Start a chat with agent number `.0` of the Preferences.
    NewChat(usize),
    SendChat {
        chat: String,
        text: String,
    },
    /// Ask a chat's agent to stop its turn.
    CancelChat(String),
    CloseChat(String),
    /// Answer an agent's request for permission, entry `entry` of `chat`.
    AnswerPermission {
        chat: String,
        entry: usize,
        option: Option<String>,
    },
    SetChatAsk {
        chat: String,
        ask: bool,
    },
    /// Pick files to go with the chat's next prompt.
    AttachFiles(String),
    /// These files go with the chat's next prompt.
    AttachPaths {
        chat: String,
        paths: Vec<std::path::PathBuf>,
    },
    /// A picture of the view goes with the chat's next prompt.
    AttachView(String),
    /// Take attachment `index` off the chat's next prompt.
    Detach {
        chat: String,
        index: usize,
    },
    /// Make a variable set with a fresh name.
    NewVariableSet,
    /// An edit of the configurations table.
    Config(ConfigEdit),
    /// Set a variable's formula (and comment, when given), adding it when
    /// new.
    SetVariable {
        set: core_document::FeatureId,
        name: String,
        formula: String,
        comment: Option<String>,
    },
    RemoveVariable {
        set: core_document::FeatureId,
        name: String,
    },
    RenameVariable {
        set: core_document::FeatureId,
        name: String,
        to: String,
    },
    /// Set one of a feature's numbers to a value or a formula.
    SetParameter {
        feature: core_document::FeatureId,
        parameter: Box<core_document::Parameter>,
        edit: ui_kit::widgets::FormulaEdit,
    },
    /// Change one of the agent's session options (mode, model ...).
    SetChatOption {
        chat: String,
        id: String,
        value: serde_json::Value,
    },
    /// Run the change held at `index`, or tell its agent no.
    SettleApproval {
        index: usize,
        allow: bool,
    },
    /// Make a new script in the scripts folder and open it for editing.
    NewScript,
    /// Write what the console ran as a new script in the scripts folder.
    SaveRunsAsScript(Vec<String>),
    /// Open a script for editing, or the scripts folder when `None`.
    EditScript(Option<std::path::PathBuf>),
    /// The task panel closed with this outcome.
    TaskClosed(TaskOutcome),
    /// Delete a tree item: a feature, or a body with everything on it.
    DeleteTreeItem(TreeItemId),
    /// Ask the kernel to repair these imported bodies' shapes.
    RepairShapes(Vec<core_document::BodyId>),
    /// Ask the kernel to turn these mesh bodies into solids.
    ConvertToSolid(Vec<core_document::BodyId>),
    /// The property panel's Label row renamed a tree item.
    RenameTreeItem {
        item: TreeItemId,
        name: String,
    },
    /// A panel hook released the active document object (task accepted,
    /// sketch closed): the selection falls back to the body.
    ReleaseActiveObject,
    /// Leave the workspace for the start page.
    ShowStartPage,
    /// A start-page NEW card.
    StartNew(StartKind),
    /// Open a document from the recent list.
    OpenRecent(std::path::PathBuf),
    /// Forget a document in the recent list.
    RemoveRecent(std::path::PathBuf),
}

/// An edit of the configurations table.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigEdit {
    /// Make the configurations table, empty.
    NewTable,
    New {
        name: String,
        like: Option<String>,
    },
    Remove(String),
    Rename {
        name: String,
        to: String,
    },
    AddVariable(String),
    RemoveVariable(String),
    Set {
        name: String,
        variable: String,
        value: String,
    },
    Activate(Option<String>),
}

impl ConfigEdit {
    /// The command it is, for recordings and scripts.
    pub fn command(&self) -> (&'static str, serde_json::Value) {
        use serde_json::json;
        match self {
            ConfigEdit::NewTable => ("config.list", json!({})),
            ConfigEdit::New { name, like } => ("config.new", json!({"name": name, "like": like})),
            ConfigEdit::Remove(name) => ("config.remove", json!({"name": name})),
            ConfigEdit::Rename { name, to } => ("config.rename", json!({"name": name, "to": to})),
            ConfigEdit::AddVariable(v) => ("config.add_variable", json!({"variable": v})),
            ConfigEdit::RemoveVariable(v) => ("config.remove_variable", json!({"variable": v})),
            ConfigEdit::Set {
                name,
                variable,
                value,
            } => (
                "config.set",
                json!({"name": name, "variable": variable, "value": value}),
            ),
            ConfigEdit::Activate(name) => ("config.activate", json!({"name": name})),
        }
    }
}
