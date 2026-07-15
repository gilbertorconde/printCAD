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
    /// Exit the active workbench's editing session (e.g. "Exit Sketch
    /// Mode" in the sketcher panel).
    FinishSketch,
    /// Orient the camera to a plane (sketch created from the panel).
    OrientCameraToPlane(core_document::CameraOrientRequest),
    SwitchWorkbench {
        from: ActiveWorkbench,
        to: ActiveWorkbench,
    },
}
