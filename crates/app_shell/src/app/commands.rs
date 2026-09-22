//! Application of per-frame [`UiCommand`]s. THE place a new UI action lands:
//! add a variant in `ui/commands.rs`, fold it into [`FrameIntents`], apply it
//! in [`PrintCadApp::apply_ui_commands`].

use glam::Vec3;
use winit::event_loop::ActiveEventLoop;

use crate::app::doc_io::FileDialogKind;
use crate::app::frame::{aabb_fit_center_radius, document_imported_aabb};
use crate::log_panel as app_log;
use crate::orientation_cube::{CameraSnapView, RotateDelta};

use crate::PrintCadApp;
use crate::ui::{
    ActiveWorkbench, FileCommand, Screen, StartKind, TreeFeatureCommand, TreeItemId, UiCommand,
};

/// Phase 1 of the two-phase dispatch: commands folded into per-frame
/// intents. Phase 2 applies them in the frame order the pre-command code
/// used, so a single frame carrying several actions behaves identically.
#[derive(Default)]
struct FrameIntents {
    camera_snap: Option<CameraSnapView>,
    camera_rotate: Option<RotateDelta>,
    persist_settings: bool,
    apply_camera_settings: bool,
    commit_settings: Option<(Box<settings::UserSettings>, core_document::Unit)>,
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
    request_workbench: Option<ActiveWorkbench>,
    undo: bool,
    redo: bool,
    toggle_log_panel: bool,
    set_projection: Option<settings::ProjectionMode>,
    recompute_all: bool,
    task_closed: Option<core_document::TaskOutcome>,
    rename: Option<(TreeItemId, String)>,
    delete_item: Option<TreeItemId>,
    show_start_page: bool,
    release_active_object: bool,
    start_new: Option<StartKind>,
    open_recent: Option<std::path::PathBuf>,
    remove_recent: Vec<std::path::PathBuf>,
    /// Bench requests from panel hooks that are not one of the intents
    /// above, applied in the order they arrived.
    host_requests: Vec<core_document::HostRequest>,
}

impl PrintCadApp {
    pub(crate) fn apply_ui_commands(
        &mut self,
        commands: Vec<UiCommand>,
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
                UiCommand::RevealInTree(body) => {
                    self.viewport_menu = None;
                    self.reveal_body = Some(body);
                }
                UiCommand::SelectBody(body) => {
                    self.viewport_menu = None;
                    self.face_highlight = None;
                    self.last_face_hit = None;
                    self.selected_body = Some(body.0);
                    app_log::info(format!("Selected body: {:?}", body.0));
                }
                UiCommand::CloseViewportMenu => self.viewport_menu = None,
                UiCommand::CameraSnap(view) => intents.camera_snap = Some(view),
                UiCommand::CameraRotate(delta) => intents.camera_rotate = Some(delta),
                UiCommand::CommitSettings {
                    settings,
                    display_unit,
                } => intents.commit_settings = Some((settings, display_unit)),
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
                // A panel's requests take the same paths the UI's own
                // commands take, so a frame carrying both stays in order.
                UiCommand::HostRequest(core_document::HostRequest::FinishEditing) => {
                    intents.finish_sketch = true;
                }
                UiCommand::HostRequest(core_document::HostRequest::OrientCamera(req)) => {
                    intents.orient_to_plane = Some(req);
                }
                UiCommand::HostRequest(core_document::HostRequest::SwitchWorkbench(wb)) => {
                    intents.request_workbench = Some(ActiveWorkbench(wb));
                }
                UiCommand::HostRequest(request) => intents.host_requests.push(request),
                UiCommand::SwitchWorkbench { from, to } => {
                    intents.workbench_switch = Some((from, to));
                }
                UiCommand::Undo => intents.undo = true,
                UiCommand::Redo => intents.redo = true,
                UiCommand::ToggleLogPanel => intents.toggle_log_panel = true,
                UiCommand::SetProjection(mode) => intents.set_projection = Some(mode),
                UiCommand::RecomputeAll => intents.recompute_all = true,
                UiCommand::TaskClosed(outcome) => intents.task_closed = Some(outcome),
                UiCommand::DeleteTreeItem(item) => intents.delete_item = Some(item),
                UiCommand::RenameTreeItem { item, name } => intents.rename = Some((item, name)),
                UiCommand::ReleaseActiveObject => intents.release_active_object = true,
                UiCommand::ShowStartPage => intents.show_start_page = true,
                UiCommand::StartNew(kind) => intents.start_new = Some(kind),
                UiCommand::OpenRecent(path) => intents.open_recent = Some(path),
                UiCommand::RemoveRecent(path) => intents.remove_recent.push(path),
            }
        }

        // Undo and redo first: they replace what every later intent acts on.
        if intents.undo {
            self.perform_undo();
        }
        if intents.redo {
            self.perform_redo();
        }
        if intents.toggle_log_panel {
            self.user_settings.rendering.show_log_panel =
                !self.user_settings.rendering.show_log_panel;
            intents.persist_settings = true;
        }
        if let Some(mode) = intents.set_projection {
            self.user_settings.camera.projection = mode;
            intents.persist_settings = true;
            intents.apply_camera_settings = true;
        }
        if intents.recompute_all {
            self.registry.invalidate_all(&mut self.document);
            app_log::info("Recomputing every feature");
        }
        if let Some(outcome) = intents.task_closed {
            // The task's edits form one undo entry; a closed task ends it.
            self.journal.note(&mut self.document);
            match outcome {
                core_document::TaskOutcome::Accepted { label } => app_log::info(label),
                core_document::TaskOutcome::Cancelled => app_log::info("Edit cancelled"),
                core_document::TaskOutcome::Open => {}
            }
        }
        if intents.release_active_object {
            // The body the task or sketch belonged to stays selected, so the
            // next feature has a target without another trip to the tree.
            let target = self
                .active_body_id
                .map(TreeItemId::Body)
                .unwrap_or(TreeItemId::DocumentRoot);
            self.apply_tree_selection(target);
        }
        if let Some((item, name)) = intents.rename {
            match item {
                TreeItemId::Feature(id) => self.document.rename_feature(id, name),
                TreeItemId::Body(id) => self.document.rename_body(id, name),
                TreeItemId::DocumentRoot | TreeItemId::ImportedObject(_) => {}
            }
        }
        if let Some(wb) = intents.request_workbench
            && wb != self.active_workbench
        {
            intents.workbench_switch = Some((self.active_workbench.clone(), wb));
        }

        // ---- Phase 2: apply in legacy frame order ----

        // An open sketch keeps the view square to its plane: standard
        // views and out-of-plane rotation are refused, but rolling about
        // the plane normal (the direction the camera looks down) is not.
        let planar_only = self.sketch_editing_active();
        match intents.camera_snap {
            Some(_) if planar_only => {
                app_log::info("Standard views are locked while a sketch is open")
            }
            Some(view) => self.camera.snap_to_view(view, &self.user_settings.camera),
            None => {}
        }
        if let Some(ref delta) = intents.camera_rotate {
            if !planar_only || delta.axis.keeps_view_direction() {
                self.camera
                    .apply_rotate_delta(delta, &self.user_settings.camera);
            } else {
                app_log::info("The view only turns in the sketch plane while a sketch is open");
            }
        }

        if let Some((settings, display_unit)) = intents.commit_settings {
            // Only a changed camera section re-syncs the controller: a sync
            // resets the wheel zoom and orbit framing from the saved values.
            if settings.camera != self.user_settings.camera {
                intents.apply_camera_settings = true;
            }
            if *settings != self.user_settings {
                self.user_settings = *settings;
                intents.persist_settings = true;
            }
            if display_unit != self.document.display_unit() {
                self.document.set_display_unit(display_unit);
            }
        }
        if intents.persist_settings
            && let Err(err) = self.settings_store.save(&self.user_settings)
        {
            app_log::warn(format!("Failed to save settings: {err}"));
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

        if let Some(item) = intents.delete_item {
            self.delete_tree_item(item);
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
        for request in intents.host_requests {
            self.apply_host_request(request, crate::app::workbench_host::HookSite::Interaction);
        }

        if let Some((path, detail)) = step_import_to_run {
            self.last_step_import_detail = detail.clone();
            self.import_step_at(&path, detail);
        }

        if intents.new_document && self.confirm_discard_or_save() {
            self.reset_to_new_document();
        }
        if let Some(kind) = intents.start_new
            && self.confirm_discard_or_save()
        {
            self.start_new_document(kind);
        }
        if let Some(path) = intents.open_recent
            && self.confirm_discard_or_save()
        {
            if path.exists() {
                self.open_document_at(path);
            } else {
                app_log::error(format!("`{}` is gone; removed from recent", path.display()));
                self.remove_recent(&path);
            }
        }
        for path in intents.remove_recent {
            self.remove_recent(&path);
        }
        if intents.show_start_page {
            self.screen = Screen::Start;
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

        // Workbench change last-but-one so the outgoing workbench sees the
        // frame's selection updates in its deactivate hook.
        if let Some((old_wb, new_wb)) = intents.workbench_switch {
            // A deliberate user switch cancels any pending return-to-bench.
            self.return_workbench = None;
            self.active_workbench = new_wb.clone();
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
    pub(crate) fn finish_active_workbench_editing(&mut self) {
        let wb_id = self.active_workbench.0.clone();
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.finish_editing(ctx))
        {
            self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
        }
        self.active_document_object = None;
        self.tree_selection = Some(TreeItemId::DocumentRoot);

        if let Some(previous) = self.return_workbench.take()
            && previous != self.active_workbench
        {
            self.switch_workbench_for_flow(previous.0);
        }
    }

    /// Host-driven workbench switch (create-sketch flow, return-on-finish).
    /// Remembers the outgoing workbench as the return target when jumping
    /// INTO an edit-session bench so finishing can jump back.
    pub(crate) fn switch_workbench_for_flow(&mut self, target: crate::WorkbenchId) {
        if self.active_workbench.0 == target {
            return;
        }
        if self.registry.is_modal(&target) {
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
        // A selection made here is not a viewport click, so the next click
        // on that body picks a face instead of undoing this.
        self.last_select_click = None;
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
                self.active_body_id = self.document.body_of_imported_object(node_id);
                self.selected_body = self.active_body_id.map(|id| id.0);
            }
        }
    }

    /// Double-click "jump" semantics: a sketch opens straight in the
    /// sketcher's edit mode; a part feature or datum jumps to the Part
    /// Design panel with its settings editor open.
    pub(crate) fn apply_tree_activation(&mut self, item: TreeItemId) {
        let TreeItemId::Feature(id) = item else {
            // A body or an imported part: the whole body is the selection,
            // the same answer the row's menu gives.
            if matches!(item, TreeItemId::Body(_) | TreeItemId::ImportedObject(_)) {
                self.apply_tree_selection(item);
            }
            return;
        };
        let Some(node) = self.document.get_feature_meta(id) else {
            return;
        };
        let kind = node.workbench_id.clone();
        self.apply_tree_selection(item);
        // The bench that claimed the feature's kind edits it: it becomes
        // active and finds the feature as the active document object. A
        // kind no bench claims is only selected.
        let Some(owner) = self.registry.owner_id_of(&kind).cloned() else {
            return;
        };
        if self.active_workbench.0 != owner {
            self.switch_workbench_for_flow(owner);
        }
        self.active_document_object = Some(id);
    }

    /// Apply a history context-menu action from the feature tree.
    /// Delete what a tree row stands for: a feature, or a body with every
    /// feature and every bit of geometry on it.
    fn delete_tree_item(&mut self, item: TreeItemId) {
        match item {
            TreeItemId::Feature(feature) => {
                self.apply_tree_feature_command(feature, TreeFeatureCommand::Delete);
            }
            TreeItemId::Body(body) => self.delete_body(body),
            TreeItemId::ImportedObject(node) => self.delete_imported_node(node),
            TreeItemId::DocumentRoot => {}
        }
    }

    /// Remove `body` and forget every reference the app holds to it.
    fn delete_body(&mut self, body: core_document::BodyId) {
        let name = self
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "body".to_string());
        if !self.document.remove_body(body) {
            return;
        }
        if self.active_body_id == Some(body) {
            self.active_body_id = None;
        }
        if self.selected_body == Some(body.0) {
            self.selected_body = None;
        }
        if self.hovered_body == Some(body.0) {
            self.hovered_body = None;
            self.hovered_world_pos = None;
        }
        if self.face_highlight.as_ref().map(|f| f.body) == Some(body.0) {
            self.face_highlight = None;
        }
        if self.last_face_hit.map(|(b, _)| b) == Some(body.0) {
            self.last_face_hit = None;
        }
        // Features of the body went with it; anything pointing at one of
        // them now points at nothing.
        if self
            .active_document_object
            .is_some_and(|id| self.document.get_feature_meta(id).is_none())
        {
            self.active_document_object = None;
        }
        self.tree_selection = Some(TreeItemId::DocumentRoot);
        // Deleting a body has no inverse: the entry closes the history.
        self.journal.label_next("Delete body");
        self.journal.note(&mut self.document);
        app_log::info(format!("Deleted `{name}` and everything on it"));
    }

    /// Delete an imported row: its subtree leaves the graph and the bodies
    /// those rows stood for go with it.
    fn delete_imported_node(&mut self, node: uuid::Uuid) {
        let mut doomed: Vec<uuid::Uuid> = Vec::new();
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            let Some(entry) = self.document.imported_object(id) else {
                continue;
            };
            stack.extend(entry.children.iter().copied());
            doomed.push(id);
        }
        let bodies: Vec<core_document::BodyId> = doomed
            .iter()
            .filter_map(|id| self.document.imported_object(*id).and_then(|n| n.body_id))
            .collect();

        // Rebuild what stays, with the deleted rows dropped from their
        // parents' children.
        let roots: Vec<uuid::Uuid> = self
            .document
            .imported_object_roots()
            .iter()
            .copied()
            .filter(|id| !doomed.contains(id))
            .collect();
        let mut kept: std::collections::HashMap<uuid::Uuid, core_document::ImportedObjectNode> =
            std::collections::HashMap::new();
        let mut stack: Vec<uuid::Uuid> = roots.clone();
        while let Some(id) = stack.pop() {
            let Some(entry) = self.document.imported_object(id) else {
                continue;
            };
            let mut entry = entry.clone();
            entry.children.retain(|child| !doomed.contains(child));
            stack.extend(entry.children.iter().copied());
            kept.insert(id, entry);
        }
        self.document.set_imported_object_graph(roots, kept);
        for body in bodies {
            self.delete_body(body);
        }
    }

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
                // The bench that claimed the feature's kind removes it and
                // settles what depended on it; a kind no bench claims is
                // simply removed.
                let owner = self
                    .document
                    .get_feature_meta(feature)
                    .and_then(|n| self.registry.owner_id_of(&n.workbench_id).cloned());
                let removed = match owner {
                    Some(owner) => {
                        let params = self.interaction_ctx_params();
                        match self.with_workbench_ctx(&owner, params, |wb, ctx| {
                            wb.delete_feature(ctx, feature)
                        }) {
                            Some((removed, outcome)) => {
                                self.apply_hook_outcome(
                                    outcome,
                                    crate::app::workbench_host::HookSite::Interaction,
                                );
                                removed
                            }
                            None => false,
                        }
                    }
                    None => self.document.remove_feature(feature).is_ok(),
                };
                if removed {
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
                self.registry.invalidate_body(&mut self.document, body);
                self.journal.label_next("Move tip");
                self.journal.note(&mut self.document);
            }
        }
    }
}

impl PrintCadApp {
    /// A fresh document from a start-page card: one body in Part Design,
    /// plus an XY sketch open for editing when asked.
    fn start_new_document(&mut self, kind: StartKind) {
        let part = self.landing_workbench();
        if self.active_workbench != part {
            let old = self.active_workbench.0.clone();
            self.call_workbench_deactivate(&old);
            self.active_workbench = part.clone();
            self.call_workbench_activate(&part.0);
        }
        self.return_workbench = None;
        self.reset_to_new_document();
        self.create_new_body();
        if kind == StartKind::EmptySketch && self.active_body_id.is_some() {
            let sketch = wb_sketch::sketch::Sketch::new("Sketch");
            let plane = sketch.plane;
            match self.document.add_feature_in_body(
                wb_sketch::SketchFeature::new(sketch, plane),
                "Sketch".into(),
                self.active_body_id,
            ) {
                Ok(id) => self.apply_tree_activation(TreeItemId::Feature(id)),
                Err(err) => app_log::error(format!("Failed to create sketch: {err}")),
            }
        }
    }
}
