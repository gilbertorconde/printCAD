//! Actions the UI requests from the app, emitted once on the frame they
//! were triggered. Adding a new UI action = one enum variant here + one
//! match arm in `PrintCadApp::apply_ui_commands`.

use core_document::TaskOutcome;
use settings::ProjectionMode;

use super::ActiveWorkbench;
use super::feature_tree::{TreeFeatureCommand, TreeItemId};
use crate::orientation_cube::{CameraSnapView, RotateDelta};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCommand {
    New,
    Open,
    Save,
    SaveAs,
    ImportStep,
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
    ConfirmStepImport,
    CancelStepImport,
    SwitchWorkbench {
        from: ActiveWorkbench,
        to: ActiveWorkbench,
    },
    Undo,
    Redo,
    ToggleLogPanel,
    SetProjection(ProjectionMode),
    /// Mark every part feature dirty so the next frame rebuilds them all.
    RecomputeAll,
    /// The task panel closed with this outcome.
    TaskClosed(TaskOutcome),
    /// Delete a tree item: a feature, or a body with everything on it.
    DeleteTreeItem(TreeItemId),
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
