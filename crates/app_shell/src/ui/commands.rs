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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartKind {
    /// A document with one body, in the Part Design workbench.
    PartDesign,
    /// A document with one body and an XY sketch open for editing.
    EmptySketch,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    File(FileCommand),
    Quit,
    FitView,
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
    /// Exit the active workbench's editing session (e.g. "Exit Sketch
    /// Mode" in the sketcher panel).
    FinishSketch,
    /// Orient the camera to a plane (sketch created from the panel).
    OrientCameraToPlane(core_document::CameraOrientRequest),
    SwitchWorkbench {
        from: ActiveWorkbench,
        to: ActiveWorkbench,
    },
    /// A panel hook asked for another workbench (e.g. New Sketch jumps to
    /// the sketcher).
    RequestWorkbench(ActiveWorkbench),
    Undo,
    Redo,
    ToggleLogPanel,
    SetProjection(ProjectionMode),
    /// Mark every part feature dirty so the next frame rebuilds them all.
    RecomputeAll,
    /// The task panel closed with this outcome.
    TaskClosed(TaskOutcome),
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
