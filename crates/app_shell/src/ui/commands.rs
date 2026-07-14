//! Actions the UI requests from the app, emitted once on the frame they
//! were triggered. Adding a new UI action = one enum variant here + one
//! match arm in `PrintCadApp::apply_ui_commands`.

use super::feature_tree::TreeItemId;
use super::ActiveWorkbench;
use crate::orientation_cube::{CameraSnapView, RotateDelta};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCommand {
    New,
    Open,
    Save,
    SaveAs,
    ImportStep,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    File(FileCommand),
    Quit,
    FitView,
    CameraSnap(CameraSnapView),
    CameraRotate(RotateDelta),
    /// Settings were edited this frame; persist them to disk.
    PersistSettings,
    /// Camera preferences were edited; push them into the camera controller.
    ApplyCameraSettings,
    SelectTreeItem(TreeItemId),
    ActivateTreeItem(TreeItemId),
    SetImportedVisibility {
        node: uuid::Uuid,
        visible: bool,
    },
    ConfirmStepImport,
    CancelStepImport,
    /// Emitted by the sketch left panel. Not handled anywhere — the flag was
    /// already dead before the command refactor; kept so the wiring is
    /// visible when the finish-sketch flow gets implemented.
    FinishSketch,
    SwitchWorkbench {
        from: ActiveWorkbench,
        to: ActiveWorkbench,
    },
}
