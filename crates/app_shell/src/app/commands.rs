//! Application of per-frame [`UiCommand`]s. THE place a new UI action lands:
//! add a variant in `ui/commands.rs`, fold it into [`FrameIntents`], apply it
//! in [`PrintCadApp::apply_ui_commands`].

use glam::Vec3;
use winit::event_loop::ActiveEventLoop;

use crate::app::doc_io::FileDialogKind;
use crate::app::frame::{aabb_fit_center_radius, document_imported_aabb};
use crate::log_panel as app_log;
use crate::orientation_cube::{CameraSnapView, RotateDelta};
use core_document::WorkbenchFeature;

use crate::ui::{ActiveWorkbench, FileCommand, TreeFeatureCommand, TreeItemId, UiCommand};
use crate::PrintCadApp;

/// Phase 1 of the two-phase dispatch: commands folded into per-frame
/// intents. Phase 2 applies them in the frame order the pre-command code
/// used, so a single frame carrying several actions behaves identically.
#[derive(Default)]
struct FrameIntents {
    camera_snap: Option<CameraSnapView>,
    camera_rotate: Option<RotateDelta>,
    persist_settings: bool,
    apply_camera_settings: bool,
    confirm_step_import: bool,
    cancel_step_import: bool,
    fit_view: bool,
    set_visibility: Vec<(uuid::Uuid, bool)>,
    select_tree_item: Option<TreeItemId>,
    activate_tree_item: Option<TreeItemId>,
    tree_feature: Option<(core_document::FeatureId, TreeFeatureCommand)>,
    new_document: bool,
    file_dialog: Option<FileDialogKind>,
    workbench_switch: Option<(ActiveWorkbench, ActiveWorkbench)>,
    orient_to_plane: Option<core_document::CameraOrientRequest>,
    finish_sketch: bool,
    quit: bool,
}

impl PrintCadApp {
    pub(crate) fn apply_ui_commands(
        &mut self,
        commands: Vec<UiCommand>,
        new_body_requested: bool,
        event_loop: &ActiveEventLoop,
    ) {
        let mut intents = FrameIntents::default();
        for command in commands {
            match command {
                UiCommand::File(FileCommand::New) => intents.new_document = true,
                // Dialog-kind priority (import > open > save-as > save)
                // mirrors the old boolean-cascade in `start_file_dialog`.
                UiCommand::File(FileCommand::ImportStep) => {
                    intents.file_dialog = Some(FileDialogKind::ImportStep);
                }
                UiCommand::File(FileCommand::Open) => {
                    if !matches!(intents.file_dialog, Some(FileDialogKind::ImportStep)) {
                        intents.file_dialog = Some(FileDialogKind::Open);
                    }
                }
                UiCommand::File(FileCommand::SaveAs) => {
                    if matches!(intents.file_dialog, None | Some(FileDialogKind::Save)) {
                        intents.file_dialog = Some(FileDialogKind::SaveAs);
                    }
                }
                UiCommand::File(FileCommand::Save) => {
                    if intents.file_dialog.is_none() {
                        intents.file_dialog = Some(FileDialogKind::Save);
                    }
                }
                UiCommand::Quit => intents.quit = true,
                UiCommand::CancelKernelJob => {
                    self.kernel_worker.cancel_current();
                    crate::log_panel::info("Cancelling the running kernel job…");
                }
                UiCommand::FitView => intents.fit_view = true,
                UiCommand::CameraSnap(view) => intents.camera_snap = Some(view),
                UiCommand::CameraRotate(delta) => intents.camera_rotate = Some(delta),
                UiCommand::PersistSettings => intents.persist_settings = true,
                UiCommand::ApplyCameraSettings => intents.apply_camera_settings = true,
                UiCommand::SelectTreeItem(item) => intents.select_tree_item = Some(item),
                UiCommand::ActivateTreeItem(item) => intents.activate_tree_item = Some(item),
                UiCommand::TreeFeature { feature, command } => {
                    intents.tree_feature = Some((feature, command));
                }
                UiCommand::SetImportedVisibility { node, visible } => {
                    intents.set_visibility.push((node, visible));
                }
                UiCommand::ConfirmStepImport => intents.confirm_step_import = true,
                UiCommand::CancelStepImport => intents.cancel_step_import = true,
                UiCommand::FinishSketch => intents.finish_sketch = true,
                UiCommand::OrientCameraToPlane(req) => intents.orient_to_plane = Some(req),
                UiCommand::SwitchWorkbench { from, to } => {
                    intents.workbench_switch = Some((from, to));
                }
            }
        }

        // ---- Phase 2: apply in legacy frame order ----

        // Orientation-cube rotations are locked during sketch editing:
        // the view must stay planar to the sketch.
        if self.sketch_editing_active() {
            if intents.camera_snap.is_some() || intents.camera_rotate.is_some() {
                app_log::info("View rotation is locked while editing a sketch");
            }
        } else {
            if let Some(view) = intents.camera_snap {
                self.camera.snap_to_view(view, &self.user_settings.camera);
            }
            if let Some(ref delta) = intents.camera_rotate {
                self.camera
                    .apply_rotate_delta(delta, &self.user_settings.camera);
            }
        }

        if intents.persist_settings {
            if let Err(err) = self.settings_store.save(&self.user_settings) {
                app_log::warn(format!("Failed to save settings: {err}"));
            }
        }
        if intents.apply_camera_settings {
            self.camera.sync_with_settings(&self.user_settings.camera);
        }

        let mut step_import_to_run = None;
        if intents.confirm_step_import {
            step_import_to_run = self.step_import_pending.take();
        }
        if intents.cancel_step_import {
            self.step_import_pending = None;
        }

        if intents.fit_view {
            self.fit_view_to_scene();
        }

        for (node_id, visible) in intents.set_visibility {
            self.document
                .set_imported_object_visibility(node_id, visible);
        }

        if let Some(selection) = intents.select_tree_item {
            self.apply_tree_selection(selection);
        }
        if let Some(item) = intents.activate_tree_item {
            self.apply_tree_activation(item);
        }
        if let Some((feature, command)) = intents.tree_feature {
            self.apply_tree_feature_command(feature, command);
        }

        if let Some(req) = intents.orient_to_plane {
            self.camera.orient_to_plane(
                Vec3::from_array(req.plane_origin),
                Vec3::from_array(req.plane_normal),
                Vec3::from_array(req.plane_up),
                &self.user_settings.camera,
            );
        }

        if intents.finish_sketch {
            self.finish_active_workbench_editing();
        }

        if let Some((path, detail)) = step_import_to_run {
            self.last_step_import_detail = detail.clone();
            self.import_step_at(&path, detail);
        }

        if intents.new_document && self.confirm_discard_or_save() {
            self.reset_to_new_document();
        }

        match intents.file_dialog {
            Some(FileDialogKind::Open) => {
                // Opening replaces the document; give unsaved edits a chance first.
                if self.confirm_discard_or_save() {
                    self.start_file_dialog(FileDialogKind::Open);
                }
            }
            Some(kind) => self.start_file_dialog(kind),
            None => {}
        }

        self.poll_file_dialog();

        if new_body_requested {
            self.create_new_body();
        }

        // Workbench change last-but-one so the outgoing workbench sees the
        // frame's selection updates in its deactivate hook.
        if let Some((old_wb, new_wb)) = intents.workbench_switch {
            // A deliberate user switch cancels any pending return-to-bench.
            self.return_workbench = None;
            self.call_workbench_deactivate(&old_wb.0);
            self.call_workbench_activate(&new_wb.0);
        }

        // File > Quit / Ctrl+Q. Applied here so the rest of the frame
        // (rendering, picks, dialogs) finishes cleanly before the loop ends.
        if intents.quit && self.confirm_discard_or_save() {
            app_log::info("Quit requested via menu / shortcut");
            // A save started by the dialog above is still being written; the
            // exit would kill it mid-file.
            self.wait_for_document_saves();
            event_loop.exit();
        }
    }

    pub(crate) fn create_new_body(&mut self) {
        let body_id = self.document.create_body(None);
        if let Some(body) = self.document.bodies().iter().find(|b| b.id == body_id) {
            app_log::info(format!("Created {}", body.name));
        } else {
            app_log::info(format!("Created body {:?}", body_id));
        }
        self.active_body_id = Some(body_id);
        self.active_document_object = None;
        self.tree_selection = Some(TreeItemId::Body(body_id));
        self.selected_body = Some(body_id.0);
        self.journal.label_next("Create body");
        self.journal.note(&mut self.document);
    }

    /// End the active workbench's editing session (e.g. Exit Sketch Mode)
    /// and drop the edited feature from the active-object slot so the
    /// workbench doesn't immediately re-enter editing on the next event.
    /// When the editing flow was started from another workbench (Part
    /// Design's "New Sketch"), jump back to it.
    fn finish_active_workbench_editing(&mut self) {
        let wb_id = self.active_workbench.0.clone();
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.finish_editing(ctx))
        {
            self.apply_hook_outcome(outcome);
        }
        self.active_document_object = None;
        self.tree_selection = Some(TreeItemId::DocumentRoot);

        if let Some(previous) = self.return_workbench.take() {
            if previous != self.active_workbench {
                self.switch_workbench_for_flow(previous.0);
            }
        }
    }

    /// Host-driven workbench switch (create-sketch flow, return-on-finish).
    /// Remembers the outgoing workbench as the return target when jumping
    /// INTO the sketcher so finishing can jump back.
    pub(crate) fn switch_workbench_for_flow(&mut self, target: crate::WorkbenchId) {
        if self.active_workbench.0 == target {
            return;
        }
        if target.as_str() == "wb.sketch" {
            self.return_workbench = Some(self.active_workbench.clone());
        }
        let old = self.active_workbench.0.clone();
        self.call_workbench_deactivate(&old);
        self.active_workbench = ActiveWorkbench(target.clone());
        self.active_tool = Default::default();
        self.call_workbench_activate(&target);
    }

    /// Frame the camera around the imported geometry (or the default box).
    fn fit_view_to_scene(&mut self) {
        app_log::info("Fit View requested");
        if let Some(aabb) = document_imported_aabb(&self.document) {
            let (center, radius) = aabb_fit_center_radius(aabb.0, aabb.1);
            self.camera
                .reset_to_fit(center, radius, Some(aabb), &self.user_settings.camera);
        } else {
            self.camera
                .reset_to_fit(Vec3::ZERO, 50.0, None, &self.user_settings.camera);
        }
    }

    pub(crate) fn apply_tree_selection(&mut self, selection: TreeItemId) {
        self.tree_selection = Some(selection);
        match selection {
            TreeItemId::DocumentRoot => {
                self.active_document_object = None;
                self.active_body_id = None;
                self.selected_body = None;
            }
            TreeItemId::Body(id) => {
                self.active_body_id = Some(id);
                self.active_document_object = None;
                self.selected_body = Some(id.0);
            }
            TreeItemId::Feature(id) => {
                if self.active_document_object != Some(id) {
                    app_log::info(format!("Selected feature {:?}", id));
                }
                self.active_document_object = Some(id);
            }
            TreeItemId::ImportedObject(node_id) => {
                self.active_document_object = None;
                self.active_body_id = self
                    .document
                    .imported_object(node_id)
                    .and_then(|n| n.body_id);
                self.selected_body = self.active_body_id.map(|id| id.0);
            }
        }
    }

    /// Double-click "jump" semantics: a sketch opens straight in the
    /// sketcher's edit mode; a part feature or datum jumps to the Part
    /// Design panel with its settings editor open.
    fn apply_tree_activation(&mut self, item: TreeItemId) {
        let TreeItemId::Feature(id) = item else {
            return;
        };
        let Some(node) = self.document.get_feature_meta(id) else {
            return;
        };
        let workbench = node.workbench_id.clone();
        self.apply_tree_selection(item);
        match workbench.as_str() {
            "wb.sketch" => {
                // The sketcher enters edit mode when the active document
                // object is one of its sketches.
                if self.active_workbench.0.as_str() != "wb.sketch" {
                    self.switch_workbench_for_flow(core_document::WorkbenchId::from("wb.sketch"));
                }
                self.active_document_object = Some(id);
            }
            "wb.part" | "core.datum" => {
                if self.active_workbench.0.as_str() != "wb.part" {
                    self.switch_workbench_for_flow(core_document::WorkbenchId::from("wb.part"));
                }
                self.active_document_object = Some(id);
            }
            _ => {}
        }
    }

    /// Apply a history context-menu action from the feature tree.
    fn apply_tree_feature_command(
        &mut self,
        feature: core_document::FeatureId,
        command: TreeFeatureCommand,
    ) {
        let body = self.document.get_feature_meta(feature).and_then(|n| n.body);
        match command {
            TreeFeatureCommand::Suppress(suppressed) => {
                self.document.set_feature_suppressed(feature, suppressed);
                self.document.mark_feature_dirty(feature);
                self.journal.label_next("Suppress feature");
                self.journal.note(&mut self.document);
            }
            TreeFeatureCommand::SetVisible(visible) => {
                self.document.set_feature_visible(feature, visible);
            }
            TreeFeatureCommand::Delete => {
                // Reveal sketches the feature consumed so they stay usable.
                let sketches = self
                    .document
                    .get_feature_data(feature)
                    .and_then(|d| wb_part::PartFeature::from_json(d).ok())
                    .map(|f: wb_part::PartFeature| f.sketches())
                    .unwrap_or_default();
                if self.document.remove_feature(feature).is_ok() {
                    for sketch in sketches {
                        self.document.set_feature_visible(sketch, true);
                    }
                    if let Some(body) = body {
                        match wb_part::part_feature_ids(&self.document, body).first() {
                            Some(first) => self.document.mark_feature_dirty(*first),
                            None => self.document.remove_imported_geometry(body),
                        }
                    }
                    if self.active_document_object == Some(feature) {
                        self.active_document_object = None;
                    }
                    self.journal.label_next("Delete feature");
                    self.journal.note(&mut self.document);
                    app_log::info("Deleted feature");
                }
            }
            TreeFeatureCommand::MoveUp | TreeFeatureCommand::MoveDown => {
                let up = command == TreeFeatureCommand::MoveUp;
                if self.document.move_feature_in_history(feature, up) {
                    self.journal.label_next("Reorder history");
                    self.journal.note(&mut self.document);
                    app_log::info("Reordered build history");
                } else {
                    app_log::warn(
                        "Cannot move: already at the end, or the move would break a dependency",
                    );
                }
            }
            TreeFeatureCommand::SetTip | TreeFeatureCommand::ClearTip => {
                let Some(body) = body else {
                    return;
                };
                let tip = (command == TreeFeatureCommand::SetTip).then_some(feature);
                self.document.set_body_tip(body, tip);
                // The chain changes shape: rebuild from the first feature.
                if let Some(first) = wb_part::part_feature_ids(&self.document, body).first() {
                    self.document.mark_feature_dirty(*first);
                }
                self.journal.label_next("Move tip");
                self.journal.note(&mut self.document);
            }
        }
    }
}
