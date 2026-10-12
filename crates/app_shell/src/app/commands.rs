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
/// intents. Phase 2 applies the intents in one fixed order, whatever order
/// their commands came in.
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
    toggle_annotations: bool,
    toggle_grid: bool,
    toggle_origin_planes: bool,
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
    /// The answer to the unsaved-edits question.
    unsaved_answer: Option<crate::app::unsaved::Unsaved>,
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
                UiCommand::File(FileCommand::SendToSlicer) => self.send_to_slicer(false),
                UiCommand::File(FileCommand::SendLaidOutToSlicer) => self.send_to_slicer(true),
                UiCommand::File(FileCommand::ExportLaidOut) => {
                    self.open_export_dialog();
                    if let Some(draft) = &mut self.session.export_pending {
                        draft.layout = true;
                        draft.selected_only = false;
                    }
                }
                UiCommand::SetLayoutGap(gap) => {
                    self.user_settings.printing.layout_gap_mm = gap.max(0.0);
                    intents.persist_settings = true;
                }
                UiCommand::SetSlicerLayout(on) => {
                    self.user_settings.printing.slicer_layout = on;
                    intents.persist_settings = true;
                }
                UiCommand::File(FileCommand::SavePicture) => {
                    if let Some(gfx) = self.gfx.as_mut() {
                        gfx.renderer.request_capture();
                        self.redraw_needed = true;
                    }
                }
                UiCommand::ConfirmExport => self.confirm_export(),
                UiCommand::CancelExport => self.session.export_pending = None,
                // One dialog a frame, by priority: import > open > save-as
                // > save.
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
                UiCommand::ToggleAnnotations => intents.toggle_annotations = true,
                UiCommand::ToggleGrid => intents.toggle_grid = true,
                UiCommand::ToggleOriginPlanes => intents.toggle_origin_planes = true,
                UiCommand::ToggleMeasure => intents.toggle_measure = true,
                UiCommand::Edit(command) => intents.edit.push(command),
                UiCommand::SetBodyDisplay { body, display } => {
                    intents.body_display.push((body, display));
                }
                UiCommand::SetPickFilter(filter) => {
                    self.session.pick_filter = filter;
                    self.session.hovered_edge = None;
                    app_log::info(format!("Picking: {}", filter.label()));
                }
                UiCommand::RepeatLastTool => {
                    self.session.viewport_menu = None;
                    self.repeat_last_tool();
                }
                UiCommand::LookAtFace { body, face } => {
                    self.session.viewport_menu = None;
                    self.look_at_face(body, face);
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
                UiCommand::BodyEdit { body, edit } => self.apply_body_edit(body, edit),
                UiCommand::SetCustomColors(colors) => {
                    self.user_settings.rendering.custom_colors = colors;
                    intents.persist_settings = true;
                }
                UiCommand::CheckForUpdates => self.check_app_release(false),
                UiCommand::PickTexturePicture { body, index } => {
                    self.start_file_dialog(FileDialogKind::TexturePicture(body, index));
                }
                UiCommand::ReplaceShape(body) => {
                    self.start_file_dialog(FileDialogKind::ReplaceShape(body));
                }
                UiCommand::SetToolbarLayout(rows) => {
                    self.user_settings.toolbars.rows = rows;
                    intents.persist_settings = true;
                }
                UiCommand::PasteFormulas { feature, formulas } => {
                    self.paste_formulas(feature, formulas);
                }
                UiCommand::ReloadLink(body) => self.reload_link(body),
                UiCommand::OpenLinkSource(body) => self.open_link_source(body),
                UiCommand::Isolate(body) => {
                    let body = body.or_else(|| self.selected_body_for_isolate());
                    match body {
                        Some(keep) => {
                            let others: Vec<core_document::BodyId> = self
                                .session
                                .document
                                .bodies()
                                .iter()
                                .map(|b| b.id)
                                .filter(|b| *b != keep)
                                .collect();
                            for other in others {
                                self.session.document.set_body_visible(other, false);
                            }
                            self.session.document.set_body_visible(keep, true);
                            self.session.selected_edges.retain(|e| e.body == keep.0);
                            self.session.journal.label_next("Isolate body");
                            self.close_gesture();
                        }
                        None => app_log::warn("Select a body to isolate"),
                    }
                }
                UiCommand::ShowAllBodies => {
                    let hidden: Vec<core_document::BodyId> = self
                        .session
                        .document
                        .bodies()
                        .iter()
                        .filter(|b| b.hidden)
                        .map(|b| b.id)
                        .collect();
                    for body in hidden {
                        self.session.document.set_body_visible(body, true);
                    }
                    self.session.journal.label_next("Show all bodies");
                    self.close_gesture();
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
                    self.close_gesture();
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
                UiCommand::InstallPackage => {
                    self.start_file_dialog(crate::app::doc_io::FileDialogKind::InstallPackage)
                }
                #[cfg(target_arch = "wasm32")]
                UiCommand::ChooseNavDevice => self.nav_device.choose_device(),
                UiCommand::RemovePackage(id) => self.remove_package(&id),
                UiCommand::InstallPackageFromGithub(text) => self.install_package_from_github(text),
                UiCommand::CheckPackageUpdates => self.check_package_updates(),
                UiCommand::LookAtStores => self.look_at_stores(false),
                UiCommand::InstallListed { store, id } => self.install_listed(store, id),
                UiCommand::UpdatePackage(id) => self.update_package(id),
                UiCommand::StopScript => self.stop_script(),
                UiCommand::ToggleRecording => self.toggle_recording(),
                UiCommand::Recorded(calls) => self.record_calls(calls),
                UiCommand::NewChat(agent) => self.new_chat(agent),
                UiCommand::SendChat { chat, text } => self.send_to_chat(&chat, text),
                UiCommand::CancelChat(chat) => self.cancel_chat(&chat),
                UiCommand::UnqueueChat { chat, queued, keep } => {
                    self.unqueue_chat(&chat, queued, keep);
                }
                UiCommand::ResumeChatQueue(chat) => self.resume_chat_queue(&chat),
                UiCommand::WakeChat(chat) => self.wake_chat(&chat),
                UiCommand::CloseChat(chat) => self.close_chat(&chat),
                UiCommand::AnswerPermission {
                    chat,
                    entry,
                    option,
                } => self.answer_permission(&chat, entry, option),
                UiCommand::SetChatAsk { chat, ask } => self.set_chat_asks(&chat, ask),
                UiCommand::SetDocumentAgentRules(rules) => {
                    self.session.document.set_agent_rules(rules);
                    self.session.journal.label_next("Agent rules");
                    self.close_gesture();
                }
                UiCommand::SetParameter {
                    feature,
                    parameter,
                    edit,
                } => self.set_parameter(feature, &parameter, edit),
                UiCommand::SetFeatureReference { feature, key, to } => {
                    self.set_feature_reference(feature, &key, to);
                }
                UiCommand::NewVariableSet => self.new_variable_set(),
                UiCommand::Config(edit) => self.apply_config_edit(edit),
                UiCommand::SetVariable {
                    set,
                    name,
                    formula,
                    comment,
                } => {
                    let label = format!("Set {name}");
                    let done = self.session.document.set_variable(
                        set,
                        &name,
                        &formula,
                        comment.as_deref(),
                    );
                    self.finish_variable_edit(done, label);
                }
                UiCommand::RemoveVariable { set, name } => {
                    let done = self.session.document.remove_variable(set, &name);
                    self.finish_variable_edit(done, format!("Remove {name}"));
                }
                UiCommand::RenameVariable { set, name, to } => {
                    let done = self.session.document.rename_variable(set, &name, &to);
                    self.finish_variable_edit(done, format!("Rename {name}"));
                }
                UiCommand::AttachFiles(chat) => {
                    self.start_file_dialog(FileDialogKind::Attach(chat))
                }
                UiCommand::AttachPaths { chat, paths } => self.attach_files(&chat, paths),
                UiCommand::AttachView(chat) => self.attach_view(&chat),
                UiCommand::Detach { chat, index } => self.detach(&chat, index),
                UiCommand::SetChatOption { chat, id, value } => {
                    self.set_chat_option(&chat, id, value)
                }
                UiCommand::SettleApproval { index, allow } => self.settle_approval(index, allow),
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
                        self.close_gesture();
                        app_log::info(format!(
                            "Conversion to a solid asked for {asked} mesh(es); undo history cleared"
                        ));
                    }
                }
                UiCommand::RefineShapes(bodies) => {
                    self.session.viewport_menu = None;
                    let asked = bodies
                        .into_iter()
                        .filter(|body| self.session.document.request_body_refine(*body))
                        .count();
                    if asked > 0 {
                        self.session.journal.label_next("Refine shape");
                        self.close_gesture();
                        app_log::info(format!(
                            "Refine asked for {asked} converted solid(s); undo history cleared"
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
                        self.close_gesture();
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
                UiCommand::ReopenTab => self.reopen_closed_tab(),
                UiCommand::OpenDropped(paths) => self.open_dropped(paths),
                UiCommand::RecoverDocument(copy) => self.recover_document(copy),
                UiCommand::DiscardRecovery(copy) => self.discard_recovery(copy),
                UiCommand::NoteRecentCommand(key) => {
                    let recent = &mut self.user_settings.recent_commands;
                    recent.retain(|k| *k != key);
                    recent.insert(0, key);
                    recent.truncate(crate::ui::PALETTE_RECENT_KEPT);
                    intents.persist_settings = true;
                }
                UiCommand::CloseTab(tab) => intents.close_tab = Some(tab),
                UiCommand::SelectTab(tab) => intents.select_tab = Some(tab),
                UiCommand::CycleTab(delta) => intents.cycle_tab = Some(delta),
                UiCommand::StartNew(kind) => intents.start_new = Some(kind),
                UiCommand::OpenRecent(path) => intents.open_recent = Some(path),
                UiCommand::RemoveRecent(path) => intents.remove_recent.push(path),
                UiCommand::DownloadDocument => self.download_document(),
                UiCommand::AnswerUnsaved(answer) => intents.unsaved_answer = Some(answer),
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
        if intents.toggle_annotations {
            self.user_settings.rendering.show_annotations =
                !self.user_settings.rendering.show_annotations;
            intents.persist_settings = true;
        }
        if intents.toggle_grid {
            self.user_settings.rendering.show_grid = !self.user_settings.rendering.show_grid;
            intents.persist_settings = true;
        }
        if intents.toggle_origin_planes {
            self.user_settings.rendering.show_origin_planes =
                !self.user_settings.rendering.show_origin_planes;
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
            self.close_gesture();
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
                TreeItemId::Component(id) => {
                    if let Some(mut component) = self.session.document.component(id).cloned() {
                        component.name = name;
                        if let Err(err) = self.session.document.update_component(component) {
                            app_log::warn(format!("Could not rename the component: {err}"));
                        }
                    }
                }
                TreeItemId::DocumentRoot | TreeItemId::ImportedObject(_) => {}
            }
        }
        if let Some(wb) = intents.request_workbench
            && wb != self.session.active_workbench
        {
            intents.workbench_switch = Some((self.session.active_workbench.clone(), wb));
        }

        // Phase 2: the intents applied in their fixed order.

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
                self.registry.invalidate_all(&mut self.session.document);
                for slot in &mut self.tabs {
                    if let Some(parked) = slot.parked.as_mut() {
                        self.registry.invalidate_all(&mut parked.document);
                    }
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
                req.centre.map(Vec3::from_array),
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
                app_log::info(
                    "Measure: click a point, an edge or a face, then another to measure between",
                );
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
            if crate::platform::exists(&path) {
                self.open_document_at(path);
            } else {
                app_log::error(format!("`{}` is gone; removed from recent", path.display()));
                self.remove_recent(&path);
            }
        }
        for path in intents.remove_recent {
            // A page's recent list is the documents it keeps: taking one off
            // deletes it from the browser, once the user says so.
            if crate::platform::ON_PAGE {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                if !crate::platform::confirm(&format!(
                    "Delete {name} from this browser? It cannot be brought back."
                )) {
                    continue;
                }
                crate::platform::remove(&path);
            }
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
        self.drain_package_news();
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
        if let Some(answer) = intents.unsaved_answer {
            self.answer_unsaved(answer, event_loop);
        }
        if intents.quit {
            self.request_quit(event_loop);
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
        self.close_gesture();
    }

    /// End the active workbench's editing session (e.g. Exit Sketch Mode)
    /// and drop the edited feature from the active-object slot so the
    /// workbench doesn't immediately re-enter editing on the next event.
    /// When the editing flow was started from another workbench (a new
    /// sketch made from a solid bench), jump back to it.
    pub(crate) fn finish_active_workbench_editing(&mut self) {
        let wb_id = self.session.active_workbench.0.clone();
        let edited = self
            .registry
            .workbench(&wb_id)
            .ok()
            .and_then(|wb| wb.editing_feature());
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
            // Back from an edit session, what was edited stays selected, so
            // the next tool takes it without a click in the tree. The body's
            // tip stays where it is.
            if let Some(id) = edited
                && self.session.document.get_feature_meta(id).is_some()
            {
                self.session.tree_selection = Some(TreeItemId::Feature(id));
                self.session.active_document_object = Some(id);
                self.session.last_select_click = None;
            }
        }
    }

    /// Start the last tool started again, as its button would, when its
    /// bench is the one in use.
    fn repeat_last_tool(&mut self) {
        let Some((bench, id)) = self.session.last_tool.clone() else {
            app_log::info("No tool to repeat yet");
            return;
        };
        if bench != self.session.active_workbench.0 {
            app_log::info("The last tool belongs to another workbench");
            return;
        }
        let Ok(tools) = self.registry.tools_for(&bench) else {
            return;
        };
        let base = core_document::base_tool_id(&id);
        let Some(tool) = tools.iter().find(|t| t.id == base) else {
            return;
        };
        if tool.planned.is_some() {
            return;
        }
        crate::ui::toolbar::activate_tool(&mut self.session.active_tool, tools, tool, &id);
        self.redraw_needed = true;
    }

    /// The name the last tool goes by, for Repeat's menu entry.
    pub(crate) fn last_tool_label(&self) -> Option<String> {
        let (bench, id) = self.session.last_tool.as_ref()?;
        if *bench != self.session.active_workbench.0 {
            return None;
        }
        let tool = self
            .registry
            .tools_for(bench)
            .ok()?
            .iter()
            .find(|t| t.id == core_document::base_tool_id(id))?;
        let variant = id
            .split_once(':')
            .and_then(|(_, v)| tool.variants.iter().find(|x| x.id == v));
        Some(match variant {
            Some(v) => format!("{} · {}", tool.label, v.label),
            None => tool.label.clone(),
        })
    }

    /// Turn the view square to a face of a body, looking at it from
    /// outside, centred on it, keeping the view's up as near as it goes.
    fn look_at_face(&mut self, body: core_document::BodyId, face: u32) {
        let Some(frame) = self
            .session
            .document
            .imported_geometry(body)
            .and_then(|g| crate::app::input::face_frame(&g.mesh, face))
        else {
            return;
        };
        let (centre, normal) = frame;
        let (forward, up) = self.session.camera.view_basis();
        // Looking straight along the view's up, the view's forward is up.
        let up = if up.dot(normal).abs() > 0.99 {
            forward
        } else {
            up
        };
        self.session.camera.orient_to_plane(
            centre,
            normal,
            up,
            Some(centre),
            &self.user_settings.camera,
        );
    }

    /// Host-driven workbench switch (create-sketch flow, return-on-finish).
    /// Remembers the outgoing workbench as the return target when jumping
    /// into an edit-session bench so finishing can jump back.
    pub(crate) fn switch_workbench_for_flow(&mut self, target: crate::WorkbenchId) {
        if self.session.active_workbench.0 == target {
            return;
        }
        if self.registry.is_modal(&target) {
            self.session.return_workbench = Some(self.session.active_workbench.clone());
            // The face or body picked to start the session has done its
            // work: left painted, it would cover what is edited on it.
            self.clear_view_selection();
        }
        let old = self.session.active_workbench.0.clone();
        self.call_workbench_deactivate(&old);
        self.session.active_workbench = ActiveWorkbench(target.clone());
        self.session.active_tool = Default::default();
        self.call_workbench_activate(&target);
    }

    /// Frame the selected body (else the active one); the whole scene when
    /// neither has geometry.
    fn fit_view_to_selection(&mut self) {
        if self.fit_edit_session() {
            return;
        }
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

    /// Frame the camera around the imported geometry (or the default box).
    fn fit_view_to_scene(&mut self) {
        app_log::info("Fit View requested");
        if self.fit_edit_session() {
            return;
        }
        self.frame_scene();
    }

    /// While an edit session holds the view square to its plane, frame
    /// what it edits without turning the view: the edited feature's
    /// geometry, else the bodies, else 50 mm about where the view is.
    /// False when no such session is open.
    fn fit_edit_session(&mut self) -> bool {
        if !self.sketch_editing_active() {
            return false;
        }
        let edited = self
            .registry
            .workbench(&self.session.active_workbench.0)
            .ok()
            .and_then(|wb| wb.editing_feature())
            .and_then(|id| self.registry.feature_geometry(&self.session.document, id))
            .and_then(|geometry| geometry.mesh.bounds())
            .map(|(mn, mx)| (Vec3::from_array(mn), Vec3::from_array(mx)));
        let (center, radius) =
            match edited.or_else(|| document_imported_aabb(&self.session.document)) {
                Some((mn, mx)) => aabb_fit_center_radius(mn, mx),
                None => (self.session.camera.focal_point_world(), 50.0),
            };
        self.session
            .camera
            .fit_keeping_orientation(center, radius, &self.user_settings.camera);
        true
    }

    /// [`Self::fit_view_to_scene`] without the log line.
    pub(crate) fn frame_scene(&mut self) {
        if let Some(aabb) = self.drawn_bodies_aabb() {
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

    /// The box around the bodies as the view draws them: the document's
    /// geometry and, while a feature is edited, its preview's tool, which
    /// stands in for a body that has no built shape yet.
    fn drawn_bodies_aabb(&self) -> Option<(Vec3, Vec3)> {
        let previews = self
            .session
            .previews
            .iter()
            .filter(|(body, _)| {
                self.session
                    .document
                    .imported_body_effective_visible(**body)
            })
            .filter_map(|(_, preview)| preview.tool.bounds())
            .map(|(lo, hi)| (Vec3::from_array(lo), Vec3::from_array(hi)));
        document_imported_aabb(&self.session.document)
            .into_iter()
            .chain(previews)
            .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
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
                self.move_in_time_to(id);
            }
            TreeItemId::ImportedObject(node_id) => {
                self.session.active_document_object = None;
                self.session.active_body_id =
                    self.session.document.body_of_imported_object(node_id);
                self.session.selected_body = self.session.active_body_id.map(|id| id.0);
            }
            TreeItemId::Component(_) => {
                self.session.active_document_object = None;
                self.session.active_body_id = None;
                self.session.selected_body = None;
            }
        }
    }

    /// Show `feature`'s body as it stands at `feature`: the body's tip
    /// moves there (off it, to the whole history, when `feature` is the
    /// last), and what comes after draws muted until something later is
    /// selected. A new feature goes in right there. Moving through history
    /// is not an edit: it is no undo step of its own.
    pub(crate) fn move_in_time_to(&mut self, feature: core_document::FeatureId) {
        let document = &self.session.document;
        let Some(body) = document.get_feature_meta(feature).and_then(|n| n.body) else {
            return;
        };
        let last = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.body == Some(body))
            .max_by_key(|(id, n)| (n.seq, **id))
            .map(|(id, _)| *id);
        let tip = (last != Some(feature)).then_some(feature);
        let now = document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .and_then(|b| b.tip);
        if now == tip {
            return;
        }
        let _ =
            crate::app::scripts::set_tip(&mut self.session.document, &self.registry, feature, tip);
        self.close_gesture();
    }

    /// A double click on a row: a feature opens for editing in the bench
    /// that claims its kind (a sketch in the sketcher, a solid feature or
    /// datum in its bench's task panel), through `Workbench::edit_feature`.
    /// A single click only selects.
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
        // active, finds the feature as the active document object and is
        // asked to edit it. A kind no bench claims is only selected.
        let Some(owner) = self.registry.owner_id_of(&kind).cloned() else {
            return;
        };
        if self.session.active_workbench.0 != owner {
            self.switch_workbench_for_flow(owner.clone());
        }
        self.session.active_document_object = Some(id);
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(&owner, params, |wb, ctx| wb.edit_feature(ctx, id))
        {
            self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
        }
    }

    /// Point input `key` of `feature` at `to` through the bench that claims
    /// the feature's kind: one undo step, what went wrong in the log.
    fn set_feature_reference(
        &mut self,
        feature: core_document::FeatureId,
        key: &str,
        to: core_document::ReferenceChoice,
    ) {
        let Some(kind) = self
            .session
            .document
            .get_feature_meta(feature)
            .map(|n| n.workbench_id.clone())
        else {
            return;
        };
        let Some(owner) = self.registry.owner_id_of(&kind).cloned() else {
            return;
        };
        let params = self.interaction_ctx_params();
        let Some((done, outcome)) = self.with_workbench_ctx(&owner, params, |wb, ctx| {
            wb.set_reference(ctx, feature, key, to)
        }) else {
            return;
        };
        self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
        match done {
            Ok(()) => {
                self.session.journal.label_next(format!("Change {key}"));
                self.close_gesture();
            }
            Err(why) => app_log::warn(format!("Could not change the {key}: {why}")),
        }
    }

    /// Delete what a tree row stands for: a feature, or a body with every
    /// feature and every bit of geometry on it.
    fn delete_tree_item(&mut self, item: TreeItemId) {
        match item {
            TreeItemId::Feature(feature) => {
                self.apply_tree_feature_command(feature, TreeFeatureCommand::Delete);
            }
            TreeItemId::Body(body) => self.delete_body(body),
            TreeItemId::ImportedObject(node) => self.delete_imported_node(node),
            TreeItemId::Component(component) => {
                let name = self
                    .session
                    .document
                    .component(component)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                match self.session.document.remove_component(component) {
                    Ok(()) => {
                        self.session
                            .journal
                            .label_next(format!("Take apart {name}"));
                        self.session.tree_selection = Some(TreeItemId::DocumentRoot);
                    }
                    Err(err) => app_log::warn(format!("Could not take {name} apart: {err}")),
                }
            }
            TreeItemId::DocumentRoot => {}
        }
    }

    /// A change a body menu asked for, one undo step, run as the command
    /// a script would call.
    fn apply_body_edit(&mut self, body: core_document::BodyId, edit: crate::ui::BodyEdit) {
        use crate::ui::BodyEdit;
        let (id, args) = crate::app::scripts::body_edit_call(body, &edit);
        let args = match args {
            serde_json::Value::Object(map) => map,
            _ => return,
        };
        let answer = crate::app::scripts::document_command(
            id,
            &args,
            &mut self.session.document,
            &self.registry,
            self.session.current_file.as_deref(),
        );
        let result = match answer {
            Some(Ok(result)) => result,
            Some(Err(why)) => {
                app_log::warn(format!("{why}"));
                return;
            }
            None => return,
        };
        let label = match edit {
            BodyEdit::Frozen(true) => "Freeze body",
            BodyEdit::Frozen(false) => "Thaw body",
            BodyEdit::Selectable(selectable) => {
                if !selectable && self.session.selected_body == Some(body.0) {
                    self.session.selected_body = None;
                    self.session.face_highlight = None;
                }
                "Body selectability"
            }
            BodyEdit::LinkedCopy => {
                if let Some(copy) = result.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()) {
                    self.session.tree_selection =
                        Some(TreeItemId::Body(core_document::BodyId(copy)));
                }
                "Linked copy"
            }
            BodyEdit::Recompute => {
                app_log::info("Rebuilding the body");
                return;
            }
        };
        self.session.journal.label_next(label);
        self.close_gesture();
    }

    /// Make the picture at `path` texture `index` of `body`'s pattern: the
    /// file kept in the document, its grey levels the heights.
    pub(crate) fn use_texture_picture(
        &mut self,
        body: core_document::BodyId,
        index: usize,
        path: &std::path::Path,
    ) {
        let bytes = match crate::platform::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                app_log::error(format!("Could not read {}: {err}", path.display()));
                return;
            }
        };
        if let Err(err) = surface_texture::HeightMap::from_image(&bytes, 16) {
            app_log::error(format!(
                "{} is not a picture printCAD reads: {err}",
                path.display()
            ));
            return;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "png".to_string());
        let document = &mut self.session.document;
        let mut textures = document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.textures.clone())
            .unwrap_or_default();
        let Some(texture) = textures.get_mut(index) else {
            return;
        };
        let asset = document.add_asset_with_data(
            core_document::AssetReference::new(
                format!("assets/texture-{}.{extension}", uuid::Uuid::new_v4()),
                core_document::AssetType::Other,
                serde_json::json!({
                    "kind": "texture picture",
                    "file": path.file_name().map(|n| n.to_string_lossy()),
                }),
            ),
            bytes,
        );
        texture.texture.pattern = surface_texture::Pattern::Image { asset };
        document.set_body_textures(body, textures);
        app_log::info(format!("Texture picture: {}", path.display()));
    }

    /// Read `body`'s shape from `path`, as `doc.replace_shape` does: one
    /// step of history, recorded as that command.
    pub(crate) fn replace_shape_from(
        &mut self,
        body: core_document::BodyId,
        path: &std::path::Path,
    ) {
        let args =
            serde_json::json!({"body": body.0.to_string(), "path": path.display().to_string()});
        let serde_json::Value::Object(args) = args else {
            return;
        };
        let answer = crate::app::scripts::document_command(
            "doc.replace_shape",
            &args,
            &mut self.session.document,
            &self.registry,
            self.session.current_file.as_deref(),
        );
        match answer {
            Some(Ok(result)) => {
                self.record_calls(vec![core_document::Recorded {
                    id: "doc.replace_shape".into(),
                    args,
                    result,
                }]);
                self.session.journal.label_next("Replace shape");
                self.close_gesture();
            }
            Some(Err(why)) => app_log::warn(format!("{why}")),
            None => {}
        }
    }

    /// Give `feature` the formulas of another, where it has the same
    /// parameters.
    fn paste_formulas(
        &mut self,
        feature: core_document::FeatureId,
        formulas: std::collections::BTreeMap<String, String>,
    ) {
        let keys: Vec<String> = self
            .session
            .document
            .get_feature_meta(feature)
            .map(|n| self.registry.parameters(n))
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.key)
            .collect();
        let mut pasted = 0;
        for (key, formula) in formulas {
            if keys.contains(&key)
                && self
                    .session
                    .document
                    .set_feature_formula(feature, key, Some(formula))
                    .is_ok()
            {
                pasted += 1;
            }
        }
        if pasted == 0 {
            app_log::warn("No formula fits that feature");
            return;
        }
        app_log::info(format!(
            "Pasted {pasted} formula{}",
            if pasted == 1 { "" } else { "s" }
        ));
        self.session.journal.label_next("Paste formulas");
        self.close_gesture();
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
            self.session.hovered_triangle = None;
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
        self.close_gesture();
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

    /// Apply a history context-menu action from the feature tree.
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
                crate::app::scripts::suppress(&mut self.session.document, feature, suppressed);
                self.session.journal.label_next("Suppress feature");
                self.close_gesture();
            }
            TreeFeatureCommand::SetVisible(visible) => {
                self.session.document.set_feature_visible(feature, visible);
            }
            TreeFeatureCommand::Delete => {
                // The bench that claimed the feature's kind removes it and
                // settles what depended on it; a kind no bench claims is
                // simply removed, unless a package that is not installed
                // made it: its data waits for the package.
                if let Some(node) = self.session.document.get_feature_meta(feature)
                    && self.registry.owner_id_of(&node.workbench_id).is_none()
                    && let Some(why) = core_document::FeatureInfo::missing_package(node)
                {
                    app_log::warn(why);
                    return;
                }
                let owner = self
                    .session
                    .document
                    .get_feature_meta(feature)
                    .and_then(|n| self.registry.owner_id_of(&n.workbench_id).cloned());
                let body = self
                    .session
                    .document
                    .get_feature_meta(feature)
                    .and_then(|n| n.body);
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
                    // The body rebuilds from what is left, or drops the
                    // shape nothing builds any more.
                    if let Some(body) = body {
                        self.registry
                            .invalidate_body(&mut self.session.document, body);
                    }
                    if self.session.active_document_object == Some(feature) {
                        self.session.active_document_object = None;
                    }
                    self.session.journal.label_next("Delete feature");
                    self.close_gesture();
                    app_log::info("Deleted feature");
                }
            }
            TreeFeatureCommand::MoveNextTo { target, before } => {
                let Some(after) =
                    crate::app::scripts::drop_place(&self.session.document, target, before)
                else {
                    app_log::warn("Cannot move: the two features are not in one body");
                    return;
                };
                match crate::app::scripts::move_after(&mut self.session.document, feature, after) {
                    Ok(()) => {
                        let mut args = serde_json::json!({"id": feature.0.to_string()});
                        if let Some(after) = after {
                            args["after"] = serde_json::json!(after.0.to_string());
                        }
                        self.record_calls(vec![core_document::Recorded {
                            id: "doc.move_after".into(),
                            args: match args {
                                serde_json::Value::Object(map) => map,
                                _ => Default::default(),
                            },
                            result: serde_json::Value::Null,
                        }]);
                        self.session.journal.label_next("Reorder history");
                        self.close_gesture();
                        app_log::info("Reordered build history");
                    }
                    Err(why) => app_log::warn(format!("Cannot move: {why}")),
                }
            }
            TreeFeatureCommand::MoveUp | TreeFeatureCommand::MoveDown => {
                let up = command == TreeFeatureCommand::MoveUp;
                match crate::app::scripts::move_in_history(&mut self.session.document, feature, up)
                {
                    Ok(()) => {
                        self.session.journal.label_next("Reorder history");
                        self.close_gesture();
                        app_log::info("Reordered build history");
                    }
                    Err(why) => app_log::warn(format!("Cannot move: {why}")),
                }
            }
            TreeFeatureCommand::SetTip | TreeFeatureCommand::ClearTip => {
                if body.is_none() {
                    return;
                }
                let tip = (command == TreeFeatureCommand::SetTip).then_some(feature);
                if crate::app::scripts::set_tip(
                    &mut self.session.document,
                    &self.registry,
                    feature,
                    tip,
                )
                .is_err()
                {
                    return;
                }
                self.session.journal.label_next("Move tip");
                self.close_gesture();
            }
        }
    }
}

impl PrintCadApp {
    /// A fresh document from a start-page card: one body in the landing
    /// bench, plus an XY sketch open for editing when asked.
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
                    // The bare sketch opens in the sketcher; a padded or
                    // pocketed example is selected, ready to look at.
                    Ok(handles) if scene == bench_fixtures::Scene::Sketch => {
                        self.apply_tree_activation(TreeItemId::Feature(handles.feature));
                    }
                    Ok(handles) => self.apply_tree_selection(TreeItemId::Feature(handles.feature)),
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

impl PrintCadApp {
    /// The body Isolate keeps: the one selected in the view, else the
    /// body of the tree's selection.
    fn selected_body_for_isolate(&self) -> Option<core_document::BodyId> {
        use crate::ui::TreeItemId;
        if let Some(body) = self.session.selected_body {
            return Some(core_document::BodyId(body));
        }
        match self.session.tree_selection? {
            TreeItemId::Body(body) => Some(body),
            TreeItemId::Feature(f) => self.session.document.get_feature_meta(f)?.body,
            TreeItemId::ImportedObject(id) => self.session.document.body_of_imported_object(id),
            TreeItemId::Component(id) => {
                self.session.document.component_bodies(id).first().copied()
            }
            TreeItemId::DocumentRoot => None,
        }
    }

    /// Set one of a feature's numbers: a formula is kept as typed; a value
    /// takes any formula away and goes into the feature's data, settled
    /// by its bench (a sketch solves), with what depends on it marked.
    pub(crate) fn set_parameter(
        &mut self,
        feature: core_document::FeatureId,
        parameter: &core_document::Parameter,
        edit: ui_kit::widgets::FormulaEdit,
    ) {
        use ui_kit::widgets::FormulaEdit;
        let doc = &mut self.session.document;
        match edit {
            FormulaEdit::Formula(text) => {
                if let Err(why) =
                    doc.set_feature_formula(feature, parameter.key.clone(), Some(text))
                {
                    crate::log_panel::warn(why.to_string());
                }
            }
            FormulaEdit::Value(v) => {
                if let Err(why) = self
                    .registry
                    .set_parameter_value(doc, feature, parameter, v)
                {
                    crate::log_panel::warn(why);
                }
            }
        }
        self.session
            .journal
            .label_next(format!("Set {}", parameter.label.to_lowercase()));
        self.close_gesture();
    }
}

impl PrintCadApp {
    /// A variable set with the first free name of Variables, Variables 2 …
    fn new_variable_set(&mut self) {
        let doc = &mut self.session.document;
        let name = (1..)
            .map(|n| match n {
                1 => "Variables".to_string(),
                n => format!("Variables {n}"),
            })
            .find(|name| !doc.has_object_named(name))
            .expect("a free name");
        match doc.add_variable_set(&name) {
            Ok(id) => {
                self.apply_tree_selection(crate::ui::TreeItemId::Feature(id));
                self.record_calls(vec![core_document::Recorded {
                    id: "var.new".to_string(),
                    args: serde_json::json!({"name": name})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                    result: serde_json::json!(id.0.to_string()),
                }]);
                self.session.journal.label_next("New variable set");
                self.close_gesture();
            }
            Err(why) => crate::log_panel::warn(why),
        }
    }

    fn finish_variable_edit(&mut self, done: Result<(), String>, label: String) {
        match done {
            Ok(()) => {
                self.session.journal.label_next(label);
                self.close_gesture();
            }
            Err(why) => crate::log_panel::warn(why),
        }
    }
}

impl PrintCadApp {
    /// Apply an edit of the configurations table as one undo step.
    fn apply_config_edit(&mut self, edit: crate::ui::ConfigEdit) {
        use crate::ui::ConfigEdit;
        let doc = &mut self.session.document;
        let (done, label) = match &edit {
            ConfigEdit::NewTable => {
                match doc.add_configurations_table() {
                    Ok(id) => {
                        self.session.journal.label_next("New configurations table");
                        self.close_gesture();
                        self.apply_tree_selection(crate::ui::TreeItemId::Feature(id));
                    }
                    Err(why) => crate::log_panel::warn(why),
                }
                return;
            }
            ConfigEdit::New { name, like } => (
                doc.add_configuration(name, like.as_deref()),
                format!("Add {name}"),
            ),
            ConfigEdit::Remove(name) => (doc.remove_configuration(name), format!("Remove {name}")),
            ConfigEdit::Rename { name, to } => {
                (doc.rename_configuration(name, to), format!("Rename {name}"))
            }
            ConfigEdit::AddVariable(v) => (
                doc.add_configuration_column(v),
                "Configure a variable".to_string(),
            ),
            ConfigEdit::RemoveVariable(v) => (
                doc.remove_configuration_column(v),
                "Stop configuring a variable".to_string(),
            ),
            ConfigEdit::Set {
                name,
                variable,
                value,
            } => (
                doc.set_configuration_value(name, variable, value),
                format!("Set {variable} in {name}"),
            ),
            ConfigEdit::LeaveOut { name, bodies } => (
                doc.set_configuration_left_out(name, bodies.clone()),
                format!("Leave bodies out of {name}"),
            ),
            ConfigEdit::Activate(name) => (
                doc.activate_configuration(name.as_deref()),
                match name {
                    Some(name) => format!("Switch to {name}"),
                    None => "No configuration".to_string(),
                },
            ),
        };
        self.finish_variable_edit(done, label);
    }
}
