//! Actions the UI requests from the app, emitted once on the frame they
//! were triggered. Adding a new UI action = one enum variant here + one
//! match arm in `PrintCadApp::apply_ui_commands`.

use core_document::TaskOutcome;
use settings::ProjectionMode;

use super::ActiveWorkbench;
use super::feature_tree::{TreeFeatureCommand, TreeItemId};
use crate::orientation_cube::{CameraSnapView, CubeDrag, RotateDelta};

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
    /// The export dialog, set to write the print layout.
    ExportLaidOut,
    /// The print layout written for the slicer and opened in it.
    SendLaidOutToSlicer,
    /// The view as drawn, saved as a PNG picture where the user picks.
    SavePicture,
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
    /// The gap the print layout leaves between parts, mm.
    SetLayoutGap(f32),
    /// Send to slicer sends the print layout, or the bodies where they sit.
    SetSlicerLayout(bool),
    /// The annotations imported files carry drawn over the scene, or not.
    ToggleAnnotations,
    /// The grid on the ground plane drawn, or not.
    ToggleGrid,
    /// The origin's planes drawn, or not.
    ToggleOriginPlanes,
    /// Arm the measure tool, or put it away.
    ToggleMeasure,
    Edit(EditCommand),
    /// A body's own look, or `None` for the one it came with.
    SetBodyDisplay {
        body: core_document::BodyId,
        display: Option<core_document::BodyDisplay>,
    },
    /// A change to one body from a tree or viewport menu.
    BodyEdit {
        body: core_document::BodyId,
        edit: BodyEdit,
    },
    /// Give a feature these formulas, by parameter key, where it has the
    /// parameter.
    PasteFormulas {
        feature: core_document::FeatureId,
        formulas: std::collections::BTreeMap<String, String>,
    },
    /// The colours kept under "Your colours".
    SetCustomColors(Vec<[f32; 3]>),
    /// Keep the toolbar groups where the user dragged them; empty puts
    /// every group back where its bench puts it.
    SetToolbarLayout(Vec<Vec<String>>),
    /// Look for a newer printCAD release now and say what was found.
    CheckForUpdates,
    /// Pick a file whose first solid becomes the body's shape.
    ReplaceShape(core_document::BodyId),
    /// The tree opens its way to this body and scrolls to it.
    RevealInTree(core_document::BodyId),
    /// What a click in the view picks from now on.
    SetPickFilter(crate::ui::PickFilter),
    /// Pick a picture for texture `index` of `body`'s pattern.
    PickTexturePicture {
        body: core_document::BodyId,
        index: usize,
    },
    /// Start the last tool started again, as its button would.
    RepeatLastTool,
    /// Turn the view square to face `face` of `body` (its index in the
    /// body's mesh), centred on it.
    LookAtFace {
        body: core_document::BodyId,
        face: u32,
    },
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
    /// The palette ran the entry this key names: it goes to the top of
    /// the recent list.
    NoteRecentCommand(String),
    /// Files dropped on the window, outside the assistant: documents open,
    /// model files import.
    OpenDropped(Vec<std::path::PathBuf>),
    /// Bring back an autosaved copy a crash left, by its path.
    RecoverDocument(std::path::PathBuf),
    /// Let an autosaved copy a crash left go.
    DiscardRecovery(std::path::PathBuf),
    /// Open again the file of the last tab closed.
    ReopenTab,
    /// Hand the document on screen to the browser as a download.
    DownloadDocument,
    /// The user answered the unsaved-edits question.
    AnswerUnsaved(crate::app::unsaved::Unsaved),
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
    /// Turn to a standard view and frame everything (a double click on
    /// the orientation cube).
    CameraSnapFit(CameraSnapView),
    /// The default corner view, framed (the cube's home glyph).
    CameraHome,
    /// A left drag on the orientation cube, orbiting the view.
    CubeDrag(CubeDrag),
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
    /// Read a part linked from another file again from it.
    ReloadLink(core_document::BodyId),
    /// Open the file a linked part comes from.
    OpenLinkSource(core_document::BodyId),
    /// Hide every body but this one (the selected one when `None`).
    Isolate(Option<core_document::BodyId>),
    /// Show every hidden body.
    ShowAllBodies,
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
    /// Ask for a workbench package archive and install it.
    InstallPackage,
    /// Open the browser's chooser for a 6-DoF mouse (a page's WebHID).
    #[cfg(target_arch = "wasm32")]
    ChooseNavDevice,
    /// Remove the installed workbench package with this id.
    RemovePackage(String),
    /// Install the package a GitHub repository or release address
    /// publishes.
    InstallPackageFromGithub(String),
    /// Look for newer releases of packages installed from GitHub.
    CheckPackageUpdates,
    /// Fetch the workbench stores' lists again.
    LookAtStores,
    /// Install the package a workbench store lists.
    InstallListed {
        /// The store's index address.
        store: String,
        id: String,
    },
    /// Update the package with this id to its latest release.
    UpdatePackage(String),
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
    /// Take a queued message of `chat` back, its attachments going with
    /// the next prompt again when `keep` (it is being edited).
    UnqueueChat {
        chat: String,
        queued: u64,
        keep: bool,
    },
    /// Let the messages Stop held go on.
    ResumeChatQueue(String),
    /// Start a chat from an earlier visit, its conversation coming back.
    WakeChat(String),
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
    /// Set the rules agents keep to in the document on screen.
    SetDocumentAgentRules(String),
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
    /// Point one of a feature's inputs (its profile) at something else,
    /// through the bench that claims the feature.
    SetFeatureReference {
        feature: core_document::FeatureId,
        key: String,
        to: core_document::ReferenceChoice,
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
    /// Ask the kernel to rebuild these converted solids' facets on the
    /// surfaces they approximate.
    RefineShapes(Vec<core_document::BodyId>),
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
    /// The bodies a configuration leaves out.
    LeaveOut {
        name: String,
        bodies: Vec<core_document::BodyId>,
    },
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
            ConfigEdit::LeaveOut { name, bodies } => (
                "config.leave_out",
                json!({
                    "name": name,
                    "bodies": bodies.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
                }),
            ),
        }
    }
}

/// What a body menu changes.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyEdit {
    /// Keep it as it stands, or let it rebuild.
    Frozen(bool),
    /// Let clicks pick it, or pass through.
    Selectable(bool),
    /// A linked copy beside it.
    LinkedCopy,
    /// Build it again from its history.
    Recompute,
}
