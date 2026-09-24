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
    fit_selection: bool,
    set_draw_style: Option<settings::DrawStyle>,
    toggle_print_bed: bool,
    toggle_measure: bool,
    edit: Vec<crate::ui::EditCommand>,
    body_display: Vec<(core_document::BodyId, Option<core_document::BodyDisplay>)>,
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
    /// Bench menu entries picked this frame, in order.
    bench_commands: Vec<(core_document::WorkbenchId, String, core_document::MenuScope)>,
    bench_actions: Vec<(core_document::WorkbenchId, String)>,
    new_tab: bool,
    close_tab: Option<uuid::Uuid>,
    select_tab: Option<uuid::Uuid>,
    cycle_tab: Option<i32>,
    show_workspace: bool,
}

impl PrintCadApp {
    pub(crate) fn apply_ui_commands(
        &mut self,
        commands: Vec<UiCommand>,
        event_loop: &ActiveEventLoop,
    ) {
        let mut intents = FrameIntents::default();
        for command in commands {
            if let Some(call) = crate::app::scripts::recorded_of(&command) {
                self.record_calls(vec![call]);
            }
            if self.recording.is_some() && matches!(command, UiCommand::Undo | UiCommand::Redo) {
                crate::app_log::warn(
                    "Undo and Redo are not in the recording: what they took back stays in it",
                );
            }
            match command {
                UiCommand::File(FileCommand::New) => intents.new_document = true,
                UiCommand::File(FileCommand::Export) => self.open_export_dialog(),
                UiCommand::File(FileCommand::SendToSlicer) => self.send_to_slicer(),
                UiCommand::ConfirmExport => self.confirm_export(),
                UiCommand::CancelExport => self.session.export_pending = None,
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
                UiCommand::FitSelection => intents.fit_selection = true,
                UiCommand::SetDrawStyle(style) => intents.set_draw_style = Some(style),
                UiCommand::TogglePrintBed => intents.toggle_print_bed = true,
                UiCommand::ToggleMeasure => intents.toggle_measure = true,
                UiCommand::Edit(command) => intents.edit.push(command),
                UiCommand::SetBodyDisplay { body, display } => {
                    intents.body_display.push((body, display));
                }
                UiCommand::RevealInTree(body) => {
                    self.session.viewport_menu = None;
                    self.session.reveal_body = Some(body);
                }
                UiCommand::SelectBody(body) => {
                    self.session.viewport_menu = None;
                    self.session.face_highlight = None;
                    self.session.last_face_hit = None;
                    self.session.selected_body = Some(body.0);
                    app_log::info(format!("Selected body: {:?}", body.0));
                }
                UiCommand::CloseViewportMenu => self.session.viewport_menu = None,
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
                UiCommand::SetFieldOfView { degrees, settled } => {
                    self.session
                        .camera
                        .set_field_of_view(degrees, &self.user_settings.camera);
                    // A later fit frames with the setting, so it follows
                    // the edit at once; the file waits for the edit to end.
                    self.user_settings.camera.fov_degrees = self.session.camera.field_of_view_deg();
                    if settled {
                        intents.persist_settings = true;
                    }
                }
                UiCommand::SetBodyVisible { body, visible } => {
                    self.session.viewport_menu = None;
                    self.session.document.set_body_visible(body, visible);
                    if !visible && self.session.selected_body == Some(body.0) {
                        // A hidden body is not what the next click acts on.
                        self.session.selected_body = None;
                        self.session.face_highlight = None;
                        self.session.selected_edges.retain(|e| e.body != body.0);
                    }
                    self.session.journal.label_next(if visible {
                        "Show body"
                    } else {
                        "Hide body"
                    });
                    self.session.journal.note(&mut self.session.document);
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
                UiCommand::BenchCommand {
                    workbench,
                    id,
                    scope,
                } => intents.bench_commands.push((workbench, id, scope)),
                UiCommand::SwitchWorkbench { from, to } => {
                    intents.workbench_switch = Some((from, to));
                }
                UiCommand::Undo => intents.undo = true,
                UiCommand::RunConsole(line) => self.run_console_line(&line),
                UiCommand::RunScriptFile(path) => self.run_script_file(&path),
                UiCommand::StopScript => self.stop_script(),
                UiCommand::ToggleRecording => self.toggle_recording(),
                UiCommand::Recorded(calls) => self.record_calls(calls),
                UiCommand::NewScript => self.new_script(None),
                UiCommand::SaveRunsAsScript(runs) => self.new_script(Some(runs)),
                UiCommand::EditScript(path) => self.edit_script(path),
                UiCommand::File(FileCommand::RunScript) => {
                    intents.file_dialog = Some(FileDialogKind::RunScript);
                }
                UiCommand::PivotAtCursor => {
                    if self.cursor_in_viewport.is_some()
                        && self
                            .session
                            .camera
                            .pivot_from_key_h(&self.user_settings.camera)
                    {
                        self.redraw_needed = true;
                    }
                }
                UiCommand::BenchAction { workbench, id } => {
                    intents.bench_actions.push((workbench, id));
                }
                UiCommand::Redo => intents.redo = true,
                UiCommand::ToggleLogPanel => intents.toggle_log_panel = true,
                UiCommand::SetProjection(mode) => intents.set_projection = Some(mode),
                UiCommand::SetSection(toggle) => {
                    use crate::camera::section::SectionToggle;
                    let camera = &self.session.camera;
                    self.session.section = toggle.map(|toggle| match toggle {
                        SectionToggle::Set(plane) => plane,
                        SectionToggle::On => {
                            let eye = glam::Vec3::from_array(camera.position());
                            let forward = glam::Vec3::from_array(camera.target()) - eye;
                            crate::camera::section::SectionPlane::facing(
                                forward.normalize_or_zero(),
                                camera.scene_bounds(),
                            )
                        }
                    });
                }
                UiCommand::RecomputeAll => intents.recompute_all = true,
                UiCommand::TaskClosed(outcome) => intents.task_closed = Some(outcome),
                UiCommand::DeleteTreeItem(item) => intents.delete_item = Some(item),
                UiCommand::ConvertToSolid(bodies) => {
                    self.session.viewport_menu = None;
                    let asked = bodies
                        .into_iter()
                        .filter(|body| self.session.document.request_mesh_solid(*body))
                        .count();
                    if asked > 0 {
                        self.session.journal.label_next("Convert to solid");
                        self.session.journal.note(&mut self.session.document);
                        app_log::info(format!(
                            "Conversion to a solid asked for {asked} mesh(es); undo history cleared"
                        ));
                    }
                }
                UiCommand::RepairShapes(bodies) => {
                    self.session.viewport_menu = None;
                    let asked = bodies
                        .into_iter()
                        .filter(|body| self.session.document.request_body_repair(*body))
                        .count();
                    if asked > 0 {
                        self.session.journal.label_next("Repair shape");
                        self.session.journal.note(&mut self.session.document);
                        app_log::info(format!(
                            "Repair asked for {asked} shape(s); undo history cleared"
                        ));
                    }
                }
                UiCommand::RenameTreeItem { item, name } => intents.rename = Some((item, name)),
                UiCommand::ReleaseActiveObject => intents.release_active_object = true,
                UiCommand::ShowStartPage => intents.show_start_page = true,
                UiCommand::ShowWorkspace => intents.show_workspace = true,
                UiCommand::NewTab => intents.new_tab = true,
                UiCommand::CloseTab(tab) => intents.close_tab = Some(tab),
                UiCommand::SelectTab(tab) => intents.select_tab = Some(tab),
                UiCommand::CycleTab(delta) => intents.cycle_tab = Some(delta),
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
        if intents.toggle_print_bed {
            self.user_settings.printing.show_bed = !self.user_settings.printing.show_bed;
            intents.persist_settings = true;
        }
        if let Some(style) = intents.set_draw_style
            && self.user_settings.rendering.draw_style != style
        {
            self.user_settings.rendering.draw_style = style;
            intents.persist_settings = true;
        }
        if let Some(mode) = intents.set_projection {
            self.user_settings.camera.projection = mode;
            intents.persist_settings = true;
            intents.apply_camera_settings = true;
        }
        if intents.recompute_all {
            self.registry.invalidate_all(&mut self.session.document);
            app_log::info("Recomputing every feature");
        }
        if let Some(outcome) = intents.task_closed {
            // The task's edits form one undo entry, named by the task; a
            // closed task ends it.
            if let core_document::TaskOutcome::Accepted { label } = &outcome {
                self.session.journal.label_next(label.clone());
            }
            self.session.journal.note(&mut self.session.document);
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
                .session
                .active_body_id
                .map(TreeItemId::Body)
                .unwrap_or(TreeItemId::DocumentRoot);
            self.apply_tree_selection(target);
        }
        if let Some((item, name)) = intents.rename {
            match item {
                TreeItemId::Feature(id) => self.session.document.rename_feature(id, name),
                TreeItemId::Body(id) => self.session.document.rename_body(id, name),
                TreeItemId::DocumentRoot | TreeItemId::ImportedObject(_) => {}
            }
        }
        if let Some(wb) = intents.request_workbench
            && wb != self.session.active_workbench
        {
            intents.workbench_switch = Some((self.session.active_workbench.clone(), wb));
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
            Some(view) => self
                .session
                .camera
                .snap_to_view(view, &self.user_settings.camera),
            None => {}
        }
        if let Some(ref delta) = intents.camera_rotate {
            if !planar_only || delta.axis.keeps_view_direction() {
                self.session
                    .camera
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
            }
            // A bench page applies to the bench as it is drawn; the commit
            // writes what the benches hold now.
            self.user_settings.workbenches = self.registry.collect_settings();
            intents.persist_settings = true;
            if display_unit != self.session.document.display_unit() {
                self.session.document.set_display_unit(display_unit);
            }
        }
        if intents.persist_settings
            && let Err(err) = self.settings_store.save(&self.user_settings)
        {
            app_log::warn(format!("Failed to save settings: {err}"));
        }
        if intents.apply_camera_settings {
            self.session
                .camera
                .sync_with_settings(&self.user_settings.camera);
        }

        let mut step_import_to_run = None;
        if intents.confirm_step_import {
            step_import_to_run = self.session.step_import_pending.take();
        }
        if intents.cancel_step_import {
            self.session.step_import_pending = None;
        }

        if intents.fit_view {
            self.fit_view_to_scene();
        }
        if intents.fit_selection {
            self.fit_view_to_selection();
        }
        for (body, display) in intents.body_display {
            self.session.document.set_body_display(body, display);
        }

        for (node_id, visible) in intents.set_visibility {
            self.session
                .document
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
            self.session.camera.orient_to_plane(
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
        for (workbench, id, scope) in intents.bench_commands {
            self.run_bench_command(workbench, &id, scope);
        }
        for (workbench, id) in intents.bench_actions {
            self.run_bench_action(&workbench, &id);
        }
        for command in intents.edit {
            let bench = self.session.active_workbench.0.clone();
            self.run_bench_command(bench, command.id(), core_document::MenuScope::EditMenu);
        }
        if intents.toggle_measure {
            self.session.measure = match self.session.measure {
                Some(_) => None,
                None => Some(Vec::new()),
            };
            if self.session.measure.is_some() {
                app_log::info("Measure: click two points on the model");
            }
        }

        if let Some((path, detail)) = step_import_to_run {
            self.last_step_import_detail = detail.clone();
            self.import_step_at(&path, detail);
        }

        // New and Open never discard anything: a tab with edits keeps
        // them and the new document opens beside it.
        if intents.new_document {
            self.reset_to_new_document();
        }
        if let Some(kind) = intents.start_new {
            self.start_new_document(kind);
        }
        if let Some(path) = intents.open_recent {
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
            self.session.screen = Screen::Start;
        }
        if intents.show_workspace {
            self.session.screen = Screen::Workspace;
        }

        if let Some(kind) = intents.file_dialog {
            self.start_file_dialog(kind);
        }

        self.poll_file_dialog();
        for path in std::mem::take(&mut self.scripts_to_run) {
            self.run_script_file(&path);
        }
        self.poll_export();
        self.open_export_when_ready();

        // Workbench change last-but-one so the outgoing workbench sees the
        // frame's selection updates in its deactivate hook.
        if let Some((old_wb, new_wb)) = intents.workbench_switch {
            // A deliberate user switch cancels any pending return-to-bench.
            self.session.return_workbench = None;
            self.session.active_workbench = new_wb.clone();
            self.call_workbench_deactivate(&old_wb.0);
            self.call_workbench_activate(&new_wb.0);
        }

        // Tab changes after everything else acted on the tab it was meant
        // for; a new tab opens on the start page.
        if intents.new_tab {
            let session = self.new_session(Screen::Start);
            self.open_tab(session);
        }
        if let Some(tab) = intents.select_tab
            && let Some(index) = self.tab_index_of(tab)
        {
            self.switch_tab(index);
        }
        if let Some(delta) = intents.cycle_tab {
            self.cycle_tab(delta);
        }
        if let Some(tab) = intents.close_tab
            && let Some(index) = self.tab_index_of(tab)
        {
            self.close_tab_interactive(index);
        }

        // File > Quit / Ctrl+Q. Applied here so the rest of the frame
        // (rendering, picks, dialogs) finishes cleanly before the loop ends.
        if intents.quit && self.confirm_close_all() {
            app_log::info("Quit requested via menu / shortcut");
            // A save started by the dialogs above is still being written;
            // the exit would kill it mid-file.
            self.wait_for_all_document_saves();
            event_loop.exit();
        }
    }

    pub(crate) fn create_new_body(&mut self) {
        let body_id = self.session.document.create_body(None);
        if let Some(body) = self
            .session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body_id)
        {
            app_log::info(format!("Created {}", body.name));
        } else {
            app_log::info(format!("Created body {:?}", body_id));
        }
        self.session.active_body_id = Some(body_id);
        self.session.active_document_object = None;
        self.session.tree_selection = Some(TreeItemId::Body(body_id));
        self.session.selected_body = Some(body_id.0);
        self.session.journal.label_next("Create body");
        self.session.journal.note(&mut self.session.document);
    }

    /// End the active workbench's editing session (e.g. Exit Sketch Mode)
    /// and drop the edited feature from the active-object slot so the
    /// workbench doesn't immediately re-enter editing on the next event.
    /// When the editing flow was started from another workbench (Part
    /// Design's "New Sketch"), jump back to it.
    pub(crate) fn finish_active_workbench_editing(&mut self) {
        let wb_id = self.session.active_workbench.0.clone();
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.finish_editing(ctx))
        {
            self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
        }
        self.session.active_document_object = None;
        self.session.tree_selection = Some(TreeItemId::DocumentRoot);

        if let Some(previous) = self.session.return_workbench.take()
            && previous != self.session.active_workbench
        {
            self.switch_workbench_for_flow(previous.0);
        }
    }

    /// Host-driven workbench switch (create-sketch flow, return-on-finish).
    /// Remembers the outgoing workbench as the return target when jumping
    /// INTO an edit-session bench so finishing can jump back.
    pub(crate) fn switch_workbench_for_flow(&mut self, target: crate::WorkbenchId) {
        if self.session.active_workbench.0 == target {
            return;
        }
        if self.registry.is_modal(&target) {
            self.session.return_workbench = Some(self.session.active_workbench.clone());
        }
        let old = self.session.active_workbench.0.clone();
        self.call_workbench_deactivate(&old);
        self.session.active_workbench = ActiveWorkbench(target.clone());
        self.session.active_tool = Default::default();
        self.call_workbench_activate(&target);
    }

    /// Frame the camera around the imported geometry (or the default box).
    /// Frame the selected body (else the active one); the whole scene when
    /// neither has geometry.
    fn fit_view_to_selection(&mut self) {
        let body = self
            .session
            .selected_body
            .map(core_document::BodyId)
            .or(self.session.active_body_id);
        let aabb = body.and_then(|body| {
            let geometry = self.session.document.imported_geometry(body)?;
            geometry.bounds_mm.or_else(|| geometry.mesh.bounds())
        });
        let Some((mn, mx)) = aabb else {
            self.fit_view_to_scene();
            return;
        };
        let (mn, mx) = (Vec3::from_array(mn), Vec3::from_array(mx));
        let (center, radius) = aabb_fit_center_radius(mn, mx);
        self.session
            .camera
            .reset_to_fit(center, radius, None, &self.user_settings.camera);
    }

    fn fit_view_to_scene(&mut self) {
        app_log::info("Fit View requested");
        if let Some(aabb) = document_imported_aabb(&self.session.document) {
            let (center, radius) = aabb_fit_center_radius(aabb.0, aabb.1);
            self.session.camera.reset_to_fit(
                center,
                radius,
                Some(aabb),
                &self.user_settings.camera,
            );
        } else {
            self.session
                .camera
                .reset_to_fit(Vec3::ZERO, 50.0, None, &self.user_settings.camera);
        }
    }

    pub(crate) fn apply_tree_selection(&mut self, selection: TreeItemId) {
        self.session.tree_selection = Some(selection);
        // A selection made here is not a viewport click, so the next click
        // on that body picks a face instead of undoing this.
        self.session.last_select_click = None;
        match selection {
            TreeItemId::DocumentRoot => {
                self.session.active_document_object = None;
                self.session.active_body_id = None;
                self.session.selected_body = None;
            }
            TreeItemId::Body(id) => {
                self.session.active_body_id = Some(id);
                self.session.active_document_object = None;
                self.session.selected_body = Some(id.0);
            }
            TreeItemId::Feature(id) => {
                if self.session.active_document_object != Some(id) {
                    app_log::info(format!("Selected feature {:?}", id));
                }
                self.session.active_document_object = Some(id);
            }
            TreeItemId::ImportedObject(node_id) => {
                self.session.active_document_object = None;
                self.session.active_body_id =
                    self.session.document.body_of_imported_object(node_id);
                self.session.selected_body = self.session.active_body_id.map(|id| id.0);
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
        let Some(node) = self.session.document.get_feature_meta(id) else {
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
        if self.session.active_workbench.0 != owner {
            self.switch_workbench_for_flow(owner);
        }
        self.session.active_document_object = Some(id);
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
            .session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "body".to_string());
        if !self.session.document.remove_body(body) {
            return;
        }
        if self.session.active_body_id == Some(body) {
            self.session.active_body_id = None;
        }
        if self.session.selected_body == Some(body.0) {
            self.session.selected_body = None;
        }
        if self.session.hovered_body == Some(body.0) {
            self.session.hovered_body = None;
            self.session.hovered_world_pos = None;
        }
        if self.session.face_highlight.as_ref().map(|f| f.body) == Some(body.0) {
            self.session.face_highlight = None;
        }
        if self.session.last_face_hit.map(|(b, _)| b) == Some(body.0) {
            self.session.last_face_hit = None;
        }
        // Features of the body went with it; anything pointing at one of
        // them now points at nothing.
        if self
            .session
            .active_document_object
            .is_some_and(|id| self.session.document.get_feature_meta(id).is_none())
        {
            self.session.active_document_object = None;
        }
        self.session.tree_selection = Some(TreeItemId::DocumentRoot);
        // Deleting a body has no inverse: the entry closes the history.
        self.session.journal.label_next("Delete body");
        self.session.journal.note(&mut self.session.document);
        app_log::info(format!("Deleted `{name}` and everything on it"));
    }

    /// Delete an imported row: its subtree leaves the graph and the bodies
    /// those rows stood for go with it.
    fn delete_imported_node(&mut self, node: uuid::Uuid) {
        let mut doomed: Vec<uuid::Uuid> = Vec::new();
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            let Some(entry) = self.session.document.imported_object(id) else {
                continue;
            };
            stack.extend(entry.children.iter().copied());
            doomed.push(id);
        }
        let bodies: Vec<core_document::BodyId> = doomed
            .iter()
            .filter_map(|id| {
                self.session
                    .document
                    .imported_object(*id)
                    .and_then(|n| n.body_id)
            })
            .collect();

        // Rebuild what stays, with the deleted rows dropped from their
        // parents' children.
        let roots: Vec<uuid::Uuid> = self
            .session
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
            let Some(entry) = self.session.document.imported_object(id) else {
                continue;
            };
            let mut entry = entry.clone();
            entry.children.retain(|child| !doomed.contains(child));
            stack.extend(entry.children.iter().copied());
            kept.insert(id, entry);
        }
        self.session.document.set_imported_object_graph(roots, kept);
        for body in bodies {
            self.delete_body(body);
        }
    }

    fn apply_tree_feature_command(
        &mut self,
        feature: core_document::FeatureId,
        command: TreeFeatureCommand,
    ) {
        let body = self
            .session
            .document
            .get_feature_meta(feature)
            .and_then(|n| n.body);
        match command {
            TreeFeatureCommand::Suppress(suppressed) => {
                self.session
                    .document
                    .set_feature_suppressed(feature, suppressed);
                self.session.document.mark_feature_dirty(feature);
                self.session.journal.label_next("Suppress feature");
                self.session.journal.note(&mut self.session.document);
            }
            TreeFeatureCommand::SetVisible(visible) => {
                self.session.document.set_feature_visible(feature, visible);
            }
            TreeFeatureCommand::Delete => {
                // The bench that claimed the feature's kind removes it and
                // settles what depended on it; a kind no bench claims is
                // simply removed.
                let owner = self
                    .session
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
                    None => self.session.document.remove_feature(feature).is_ok(),
                };
                if removed {
                    if self.session.active_document_object == Some(feature) {
                        self.session.active_document_object = None;
                    }
                    self.session.journal.label_next("Delete feature");
                    self.session.journal.note(&mut self.session.document);
                    app_log::info("Deleted feature");
                }
            }
            TreeFeatureCommand::MoveUp | TreeFeatureCommand::MoveDown => {
                let up = command == TreeFeatureCommand::MoveUp;
                if self.session.document.move_feature_in_history(feature, up) {
                    self.session.journal.label_next("Reorder history");
                    self.session.journal.note(&mut self.session.document);
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
                self.session.document.set_body_tip(body, tip);
                // The chain changes shape: rebuild from the first feature.
                self.registry
                    .invalidate_body(&mut self.session.document, body);
                self.session.journal.label_next("Move tip");
                self.session.journal.note(&mut self.session.document);
            }
        }
    }
}

impl PrintCadApp {
    /// A fresh document from a start-page card: one body in Part Design,
    /// plus an XY sketch open for editing when asked.
    fn start_new_document(&mut self, kind: StartKind) {
        let walkthrough = kind == StartKind::ExportWalkthrough;
        let kind = if walkthrough {
            StartKind::Example(bench_fixtures::Scene::Pocket)
        } else {
            kind
        };
        // The blank document first: it may be a new tab, and the bench
        // switch below belongs to that tab.
        self.reset_to_new_document();
        self.session.export_when_ready = walkthrough;
        let part = self.landing_workbench();
        if self.session.active_workbench != part {
            let old = self.session.active_workbench.0.clone();
            self.call_workbench_deactivate(&old);
            self.session.active_workbench = part.clone();
            self.call_workbench_activate(&part.0);
        }
        self.session.return_workbench = None;
        self.create_new_body();
        match kind {
            StartKind::Bench { workbench, command } => {
                self.switch_workbench_for_flow(workbench.clone());
                self.run_bench_command(workbench, &command, core_document::MenuScope::StartPage);
            }
            StartKind::Example(scene) => {
                match bench_fixtures::open_sketch_scene(
                    &mut self.session.document,
                    self.session.active_body_id,
                    scene,
                ) {
                    Ok(handles) => {
                        if let Some(feature) = handles.activate {
                            self.apply_tree_activation(TreeItemId::Feature(feature));
                        }
                        if let Some(feature) = handles.select {
                            self.apply_tree_selection(TreeItemId::Feature(feature));
                        }
                    }
                    Err(err) => app_log::error(format!("Example: {err}")),
                }
            }
            StartKind::Landing | StartKind::ExportWalkthrough => {}
        }
    }

    /// Run one of a bench's menu entries. A feature the bench made the
    /// active object becomes the tree selection, as a tree click would.
    pub(crate) fn run_bench_command(
        &mut self,
        workbench: core_document::WorkbenchId,
        id: &str,
        scope: core_document::MenuScope,
    ) {
        let before = self.session.active_document_object;
        let params = self.interaction_ctx_params();
        let Some((known, outcome)) =
            self.with_workbench_ctx(&workbench, params, |wb, ctx| wb.on_command(id, &scope, ctx))
        else {
            return;
        };
        self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
        if !known {
            app_log::warn(match scope {
                core_document::MenuScope::EditMenu => {
                    "Nothing here to cut, copy or paste".to_string()
                }
                _ => format!("{} offers no command `{id}`", workbench.as_str()),
            });
        }
        if self.session.active_document_object != before
            && let Some(feature) = self.session.active_document_object
        {
            self.session.tree_selection = Some(TreeItemId::Feature(feature));
        }
    }
}
