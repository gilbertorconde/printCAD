//! Phase 2 of the two-phase dispatch: the frame's intents applied in one
//! fixed order, a family at a time, whatever order their commands came in.

use glam::Vec3;
use winit::event_loop::ActiveEventLoop;

use super::FrameIntents;
use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::orientation_cube::CubeDrag;
use crate::ui::{Screen, TreeItemId};

/// A STEP import confirmed this frame: the file and the detail asked for.
type StepImport = (std::path::PathBuf, kernel_api::TessellationSettings);

impl PrintCadApp {
    pub(super) fn apply_intents(
        &mut self,
        mut intents: FrameIntents,
        event_loop: &ActiveEventLoop,
    ) {
        self.apply_history_and_display(&mut intents);
        self.apply_document_intents(&mut intents);
        self.apply_camera_intents(&intents);
        self.apply_settings_intents(&mut intents);
        let step_import = self.take_step_import(&intents);
        self.apply_scene_intents(&mut intents);
        self.apply_file_intents(&mut intents, step_import);
        self.apply_switch_and_tabs(&mut intents, event_loop);
    }

    /// Undo and redo first: they replace what every later intent acts on.
    /// Then the display switches, each a settings change.
    fn apply_history_and_display(&mut self, intents: &mut FrameIntents) {
        if intents.undo {
            self.perform_undo();
        }
        if intents.redo {
            self.perform_redo();
        }
        let shown = &mut self.user_settings;
        for (asked, switch) in [
            (
                intents.toggle_log_panel,
                &mut shown.rendering.show_log_panel,
            ),
            (intents.toggle_print_bed, &mut shown.printing.show_bed),
            (
                intents.toggle_annotations,
                &mut shown.rendering.show_annotations,
            ),
            (intents.toggle_grid, &mut shown.rendering.show_grid),
            (
                intents.toggle_origin_planes,
                &mut shown.rendering.show_origin_planes,
            ),
        ] {
            if asked {
                *switch = !*switch;
                intents.persist_settings = true;
            }
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
    }

    /// Recompute, the task closed, the active object let go, a rename, and
    /// a bench's request to switch turned into the frame's switch.
    fn apply_document_intents(&mut self, intents: &mut FrameIntents) {
        if intents.recompute_all {
            self.registry.invalidate_all(&mut self.session.document);
            app_log::info("Recomputing every feature");
        }
        if let Some((outcome, take_back)) = intents.task_closed.take() {
            // What the view held for the task is let go, nothing put back.
            self.end_gestures(crate::app::gesture::GestureEnd::TaskClosed);
            // A cancelled task leaves the selection it found, not the
            // body its bench lets go of.
            if self.end_task_step(&outcome, take_back) {
                intents.release_active_object = false;
            }
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
        if let Some((item, name)) = intents.rename.take() {
            self.rename_tree_item(item, name);
        }
        if let Some(wb) = intents.request_workbench.take()
            && wb != self.session.active_workbench
        {
            intents.workbench_switch = Some((self.session.active_workbench.clone(), wb));
        }
    }

    fn rename_tree_item(&mut self, item: TreeItemId, name: String) {
        match item {
            TreeItemId::Feature(id) => self.session.document.rename_feature(id, name),
            TreeItemId::Body(id) => self.session.document.rename_body(id, name),
            TreeItemId::Component(id) => {
                if let Some(mut component) = self.session.document.component(id).cloned() {
                    component.name = name;
                    if let Err(err) = self.session.document.update_component(component) {
                        app_log::warn(format!("Could not rename the component: {err}"));
                    }
                }
            }
            TreeItemId::Note(id) => {
                if let Some((_, note)) = self.session.document.note(id) {
                    let mut changed = note.clone();
                    changed.text = name;
                    self.session.document.set_note(changed);
                }
            }
            TreeItemId::DocumentRoot
            | TreeItemId::ImportedObject(_)
            | TreeItemId::Notes(_)
            | TreeItemId::Array(_) => {}
        }
    }

    /// The standard views, the cube's drag and turns.
    fn apply_camera_intents(&mut self, intents: &FrameIntents) {
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
                .view
                .camera
                .snap_to_view(view, &self.user_settings.camera),
            None => {}
        }
        match intents.camera_fit {
            Some(_) if planar_only => {
                app_log::info("Standard views are locked while a sketch is open")
            }
            Some(view) => self.fly_to_scene(view),
            None => {}
        }
        match intents.cube_drag {
            Some(CubeDrag::Moved(offset)) => {
                if self
                    .session
                    .view
                    .camera
                    .orbit_from_cube(offset, &self.user_settings.camera)
                {
                    self.redraw_needed = true;
                }
            }
            Some(CubeDrag::Released) => self.session.view.camera.release_cube_orbit(),
            None => {}
        }
        if let Some(ref delta) = intents.camera_rotate {
            if !planar_only || delta.axis.keeps_view_direction() {
                self.session
                    .view
                    .camera
                    .apply_rotate_delta(delta, &self.user_settings.camera);
            } else {
                app_log::info("The view only turns in the sketch plane while a sketch is open");
            }
        }
    }

    /// Preferences' commit, then the frame's settings saved once and the
    /// camera synced when its settings moved.
    fn apply_settings_intents(&mut self, intents: &mut FrameIntents) {
        if let Some((settings, display_unit)) = intents.commit_settings.take() {
            // Only a changed camera section re-syncs the controller: a sync
            // resets the wheel zoom and orbit framing from the saved values.
            if settings.camera != self.user_settings.camera {
                intents.apply_camera_settings = true;
            }
            let packages_before = self.user_settings.packages.clone();
            let detail_before = self.user_settings.rendering.curve_step_deg;
            if *settings != self.user_settings {
                self.user_settings = *settings;
            }
            // A bench page applies to the bench as it is drawn; the commit
            // writes what the benches hold now, keeping the settings of
            // packages turned off.
            self.user_settings
                .workbenches
                .extend(self.registry.collect_settings());
            intents.persist_settings = true;
            if self.user_settings.packages != packages_before {
                self.packages_changed(&packages_before);
            }
            // Solids are meshed at the detail they were built with: a new
            // curve smoothness rebuilds every tab's.
            if self.user_settings.rendering.curve_step_deg != detail_before {
                for session in crate::app::tabs::sessions_mut(&mut self.session, &mut self.tabs) {
                    self.registry.invalidate_all(&mut session.document);
                }
            }
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
                .view
                .camera
                .sync_with_settings(&self.user_settings.camera);
        }
    }

    /// The STEP import dialog answered: a confirmed import runs once the
    /// scene's intents are applied.
    fn take_step_import(&mut self, intents: &FrameIntents) -> Option<StepImport> {
        let mut step_import_to_run = None;
        if intents.confirm_step_import {
            step_import_to_run = self.session.file.step_import_pending.take();
        }
        if intents.cancel_step_import {
            self.session.file.step_import_pending = None;
        }
        step_import_to_run
    }

    /// Framing, bodies' looks and visibility, the tree, the view turned to
    /// a plane, and what goes to the benches.
    fn apply_scene_intents(&mut self, intents: &mut FrameIntents) {
        if intents.fit_view {
            self.fit_view_to_scene();
        }
        if intents.fit_selection {
            self.fit_view_to_selection();
        }
        for (body, display) in std::mem::take(&mut intents.body_display) {
            self.session.document.set_body_display(body, display);
        }
        for (node_id, visible) in std::mem::take(&mut intents.set_visibility) {
            self.session
                .document
                .set_imported_object_visibility(node_id, visible);
        }
        if let Some(selection) = intents.select_tree_item.take() {
            self.apply_tree_selection(selection);
        }
        if let Some(item) = intents.activate_tree_item.take() {
            self.apply_tree_activation(item);
        }
        if let Some((feature, command)) = intents.tree_feature.take() {
            self.apply_tree_feature_command(feature, command);
        }
        if let Some(item) = intents.delete_item.take() {
            let done = self.delete_tree_item(item);
            Self::log_refusal(done);
        }
        if let Some(req) = intents.orient_to_plane.take() {
            self.session.view.camera.orient_to_plane(
                Vec3::from_array(req.plane_origin),
                Vec3::from_array(req.plane_normal),
                Vec3::from_array(req.plane_up),
                req.centre.map(Vec3::from_array),
                &self.user_settings.camera,
            );
        }
        if intents.finish_sketch {
            self.finish_active_workbench_editing();
        }
        for request in std::mem::take(&mut intents.host_requests) {
            self.apply_host_request(request, crate::app::workbench_host::HookSite::Interaction);
        }
        for (workbench, id, scope) in std::mem::take(&mut intents.bench_commands) {
            self.run_bench_command(workbench, &id, scope);
        }
        for (workbench, id) in std::mem::take(&mut intents.bench_actions) {
            self.run_bench_action(&workbench, &id);
        }
        for command in std::mem::take(&mut intents.edit) {
            let bench = self.session.active_workbench.0.clone();
            self.run_bench_command(bench, command.id(), core_document::MenuScope::EditMenu);
        }
        if intents.toggle_measure {
            self.session.tools.measure = match self.session.tools.measure {
                Some(_) => None,
                None => Some(Vec::new()),
            };
            if self.session.tools.measure.is_some() {
                app_log::info(
                    "Measure: click a point, an edge or a face, then another to measure between",
                );
            }
        }
    }

    /// Imports, new and opened documents, the screen, the frame's file
    /// dialog, and the file work that finished away from the window.
    fn apply_file_intents(&mut self, intents: &mut FrameIntents, step_import: Option<StepImport>) {
        let answered = step_import.is_some() || intents.cancel_step_import;
        if let Some((path, detail)) = step_import {
            self.imports.last_detail = detail.clone();
            self.import_step_at(&path, detail);
        }
        if answered {
            self.import_next_waiting();
        }

        // New and Open never discard anything: a tab with edits keeps
        // them and the new document opens beside it.
        if intents.new_document {
            self.reset_to_new_document();
        }
        if let Some(kind) = intents.start_new.take() {
            self.start_new_document(kind);
        }
        if let Some(path) = intents.open_recent.take() {
            if crate::platform::exists(&path) {
                self.open_document_at(path);
            } else {
                app_log::error(format!("`{}` is gone; removed from recent", path.display()));
                self.remove_recent(&path);
            }
        }
        for path in std::mem::take(&mut intents.remove_recent) {
            self.remove_recent(&path);
        }
        if intents.show_start_page {
            self.session.screen = Screen::Start;
        }
        if intents.show_workspace {
            self.session.screen = Screen::Workspace;
        }

        if let Some(kind) = intents.file_dialog.take() {
            self.start_file_dialog(kind);
        }

        self.poll_file_dialog();
        self.drain_package_news();
        for path in std::mem::take(&mut self.scripting.to_run) {
            self.run_script_file(&path);
        }
        self.poll_export();
        self.open_export_when_ready();
    }

    /// The bench switch, then the tabs, then the answers that may end the
    /// run.
    fn apply_switch_and_tabs(&mut self, intents: &mut FrameIntents, event_loop: &ActiveEventLoop) {
        // Workbench change last-but-one so the outgoing workbench sees the
        // frame's selection updates in its deactivate hook.
        if let Some((old_wb, new_wb)) = intents.workbench_switch.take() {
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
        if let Some(answer) = intents.unsaved_answer.take() {
            self.answer_unsaved(answer, event_loop);
        }
        if intents.quit {
            self.request_quit(event_loop);
        }
    }
}
