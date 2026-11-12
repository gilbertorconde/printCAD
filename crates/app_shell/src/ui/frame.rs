//! One UI frame: the window's regions drawn in order, each pushing the
//! commands it asks for onto the frame's list in the order it drew them.

use egui::Ui;
use settings::ProjectionMode;
use winit::window::Window;

use super::*;
use crate::orientation_cube::OrientationCubeResult;

/// What the frame was given, and what it works out once before drawing.
struct FrameIn<'a> {
    inputs: UiFrameInputs<'a>,
    keymap: keymap::Keymap,
    document_name: String,
    document_dirty: bool,
    breadcrumb: Option<String>,
    fullscreen: bool,
    cube_config: OrientationCubeConfig,
    /// The document's display unit as the menu bar was drawn.
    unit: core_document::Unit,
}

/// What the regions hand back as they draw.
struct FrameOut {
    active_workbench: ActiveWorkbench,
    active_tool: ActiveTool,
    commands: Vec<UiCommand>,
    settings_commit: Option<preferences::Commit>,
    palette_activate: Option<(ActiveWorkbench, String)>,
    cube_result: OrientationCubeResult,
    viewport: egui::Rect,
    side: Option<egui::Rect>,
    task_open: bool,
    card: Option<(String, bool)>,
    tree_selection: Option<TreeItemId>,
    tree_hovered: Option<TreeItemId>,
    step_import_dialog: StepImportDialogAction,
    export_dialog: StepImportDialogAction,
    /// Menu entries that change the window, acted on once the frame is
    /// drawn.
    locals: Vec<MenuLocal>,
    /// Tools whose keys were pressed, activated once their buttons say
    /// whether they would take a click.
    key_tools: Vec<String>,
}

impl UiLayer {
    pub fn run(&mut self, window: &Window, mut inputs: UiFrameInputs<'_>) -> UiFrameOutput {
        let mut raw_input = self.state.take_egui_input(window);
        if let Some(steps) = self.bench_ui.as_mut() {
            steps.feed(&mut raw_input);
        }
        if !self.swallowed_keys.is_empty() {
            let swallowed = std::mem::take(&mut self.swallowed_keys);
            raw_input
                .events
                .retain(|e| !matches!(e, egui::Event::Key { key, .. } if swallowed.contains(key)));
        }
        let prev_workbench = inputs.active_workbench.clone();
        // Seed tool state from the host, not a UiLayer copy: the host owns
        // consumption of Action tools, and a stale parallel copy here would
        // resurrect them every frame (infinite "New Body" loop).
        let mut out = FrameOut {
            active_workbench: inputs.active_workbench.clone(),
            active_tool: std::mem::take(&mut inputs.active_tool),
            commands: Vec::new(),
            settings_commit: None,
            palette_activate: None,
            cube_result: OrientationCubeResult::default(),
            viewport: egui::Rect::NOTHING,
            side: None,
            task_open: false,
            card: None,
            tree_selection: None,
            tree_hovered: None,
            step_import_dialog: StepImportDialogAction::default(),
            export_dialog: StepImportDialogAction::default(),
            locals: Vec::new(),
            key_tools: Vec::new(),
        };

        let mut cube_config = self.orientation_cube_config.clone();
        cube_config.planar_only = inputs.planar_view_lock;
        let document = &*inputs.document;
        let document_name = document.name().to_owned();
        let document_dirty = document.metadata().dirty();
        let breadcrumb = inputs
            .editing_feature
            .and_then(|id| document.get_feature_meta(id))
            .map(|node| node.name.clone());

        let keymap =
            keymap::Keymap::build(inputs.registry, &inputs.settings.keyboard, inputs.scripts);
        if inputs.console_attention {
            self.console.open = true;
        }
        if inputs.assistant_attention {
            self.assistant.open = true;
        }

        // The workbenches hear of their keys at the start and after every
        // change, not every frame.
        let workbench_keys = keymap.workbench_keys();
        if self.workbench_keys.as_ref() != Some(&workbench_keys) {
            inputs.registry.notify_shortcuts(&workbench_keys);
            self.workbench_keys = Some(workbench_keys);
        }

        self.follow_tabs_and_tasks(&inputs, &mut out.commands);
        let unit = inputs.document.display_unit();
        let mut f = FrameIn {
            inputs,
            keymap,
            document_name,
            document_dirty,
            breadcrumb,
            fullscreen: window.fullscreen().is_some(),
            cube_config,
            unit,
        };
        let ctx = self.ctx.clone();
        let full_output = ctx.run_ui(raw_input, |ui| self.draw_frame(ui, &mut f, &mut out));
        self.finish_frame(window, full_output, f, out, prev_workbench)
    }

    /// An application task belongs to its tab and its subject; either
    /// gone, it is left as it stands. A bench's task taking the panel ends
    /// the application's, its edits kept.
    fn follow_tabs_and_tasks(&mut self, inputs: &UiFrameInputs<'_>, commands: &mut Vec<UiCommand>) {
        let document = &*inputs.document;
        let active_tab = inputs.tabs.iter().find(|t| t.active).map(|t| t.tab);
        self.active_tab = active_tab;
        self.bench_task_open = inputs.task.is_some();
        if self
            .host_task
            .as_ref()
            .is_some_and(|(tab, t)| *tab != active_tab || !t.still_there(document))
        {
            self.host_task = None;
        }
        if inputs.task.is_some()
            && let Some((_, open)) = self.host_task.take()
        {
            commands.push(UiCommand::TaskClosed {
                outcome: core_document::TaskOutcome::Accepted {
                    label: open.title(document),
                },
                take_back: false,
            });
        }
    }

    /// The window's regions, in the order they are laid out.
    fn draw_frame(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        self.draw_questions(ui, f, out);
        self.take_shortcuts(ui, f, out);
        let open_palette = self.draw_menu_and_tabs(ui, f, out);
        if f.inputs.screen == Screen::Start {
            self.draw_start(ui, f, out);
            return;
        }
        self.draw_toolbars_and_palette(ui, f, out, open_palette);
        // Bottom bars before the side panels so they span the width.
        self.draw_status_and_console(ui, f, out);
        self.draw_combo(ui, f, out);
        self.draw_assistant(ui, f, out);
        self.draw_task(ui, f, out);
        self.draw_preferences(ui, f, out);
        if let Some(writeback) = task_panel::draw_viewport_strip(
            ui,
            &out.active_workbench,
            f.inputs.document,
            f.inputs.registry,
            &f.inputs.host,
            f.inputs.active_document_object,
        ) {
            apply_writeback(&writeback, &mut out.commands, &mut out.tree_selection);
        }
        Self::lay_out_viewport(ui, f, out);
        self.draw_viewport_overlays(ui, f, out);
        self.draw_viewport_cards(ui, f, out);
        self.draw_dialogs(ui, f, out);
    }

    /// A question in the window is drawn before anything else and owns
    /// the keyboard: Escape is its Cancel, and no shortcut, task or field
    /// under it sees a key.
    fn draw_questions(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let file = &f.inputs.file;
        if let Some(name) = &f.inputs.unsaved_question {
            unsaved_modal::draw(ui.ctx(), name, &mut out.commands);
        }
        if let Some(question) = &file.save_over {
            save_over_modal::draw(ui.ctx(), question, &mut out.commands);
        }
        if let Some((name, offered)) = &file.mesh_units {
            mesh_units_modal::draw(ui.ctx(), name, *offered, &mut out.commands);
        }
        if f.inputs.unsaved_question.is_some()
            || file.save_over.is_some()
            || file.mesh_units.is_some()
        {
            ui.input_mut(|i| {
                i.events.retain(|event| {
                    !matches!(
                        event,
                        egui::Event::Key { .. }
                            | egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                    )
                })
            });
        }
    }

    /// Shortcuts first, so their keys never reach a widget. They hold
    /// their fire while a dialog of their own has the keyboard.
    fn take_shortcuts(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        if self.preferences.open || self.palette.open {
            return;
        }
        let inputs = &f.inputs;
        let active_workbench = &out.active_workbench;
        let focus = keymap::KeyFocus {
            typing: ui.ctx().egui_wants_keyboard_input(),
            numeric: inputs
                .registry
                .workbench(&active_workbench.0)
                .is_ok_and(|wb| wb.takes_numeric_input()),
            text: inputs
                .registry
                .workbench(&active_workbench.0)
                .is_ok_and(|wb| wb.takes_text_input()),
            have_document: inputs.screen == Screen::Workspace,
        };
        let active_tab = inputs.tabs.iter().find(|t| t.active).map(|t| t.tab);
        for hit in keymap::take_pressed(ui.ctx(), &f.keymap, &active_workbench.0, focus) {
            match hit.target {
                keymap::Target::Host(action) => {
                    let state = keymap::HostState {
                        active_tab,
                        section_on: inputs.section.is_some(),
                        document: inputs.document,
                        tree_selection: inputs.tree_selection,
                        editing: inputs.editing_feature.is_some(),
                    };
                    match keymap::host_outcome(action, &state) {
                        keymap::HostOutcome::Command(command) => out.commands.push(command),
                        keymap::HostOutcome::OpenPalette => self.palette.open(),
                        keymap::HostOutcome::ToggleConsole => self.console.toggle(),
                        keymap::HostOutcome::ToggleAssistant => self.assistant.toggle(),
                        keymap::HostOutcome::Local(local) => out.locals.push(local),
                        keymap::HostOutcome::OpenPreferences => {
                            let (group, tab) = (self.preferences.group, self.preferences.tab);
                            self.preferences.open_at(
                                inputs.settings,
                                inputs.document.display_unit(),
                                group,
                                tab,
                            );
                        }
                        keymap::HostOutcome::Nothing => {}
                    }
                }
                keymap::Target::Tool => out.key_tools.push(hit.id),
                keymap::Target::Script(path) => {
                    out.commands.push(UiCommand::RunScriptFile(path));
                }
                keymap::Target::Action => out.commands.push(UiCommand::BenchAction {
                    workbench: active_workbench.0.clone(),
                    id: hit.id,
                }),
            }
        }
    }

    /// The menu bar and the tab strip, and what their entries open here.
    /// Whether the palette was asked for.
    fn draw_menu_and_tabs(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) -> bool {
        let inputs = &f.inputs;
        let settings = inputs.settings;
        let tabs = &inputs.tabs;
        let menu = menu_bar::draw_menu_bar(
            ui,
            menu_bar::MenuBarInputs {
                keymap: &f.keymap,
                registry: inputs.registry,
                document_name: &f.document_name,
                document_dirty: f.document_dirty,
                breadcrumb: f.breadcrumb.as_deref(),
                show_log_panel: settings.rendering.show_log_panel,
                show_annotations: settings.rendering.show_annotations,
                show_print_bed: settings.printing.show_bed,
                show_grid: settings.rendering.show_grid,
                show_origin_planes: settings.rendering.show_origin_planes,
                split_view: inputs.split_view,
                split_linked: inputs.split_linked,
                fullscreen: f.fullscreen,
                show_console: self.console.open,
                show_assistant: self.assistant.open,
                projection: inputs.projection,
                draw_style: settings.rendering.draw_style,
                recent: inputs.recent,
                screen: inputs.screen,
                active_tab_blank: tabs.iter().any(|t| t.active && t.blank),
                active_tab: tabs.iter().find(|t| t.active).map(|t| t.tab),
                scripts: inputs.scripts,
                recording: inputs.recording,
                section_on: inputs.section.is_some(),
                measuring: inputs.measuring,
            },
            &mut out.active_workbench,
            &mut out.active_tool,
            &mut out.commands,
        );

        tab_bar::draw_tab_bar(ui, tabs, &mut out.commands);

        // About lands on its page; Preferences keeps the last group.
        let unit = f.unit;
        if menu.toggle_console {
            self.console.toggle();
        }
        if menu.toggle_assistant {
            self.assistant.toggle();
        }
        if menu.print_layout {
            out.locals.push(MenuLocal::Task(OpenTask::PrintLayout));
        }
        if menu.activate_tool.is_some() {
            out.palette_activate = menu.activate_tool;
        }

        if menu.check_updates {
            self.preferences
                .open_at(settings, unit, preferences::PrefGroup::Updates, 0);
            out.commands.push(UiCommand::CheckForUpdates);
        }
        if menu.show_about {
            self.preferences
                .open_at(settings, unit, preferences::PrefGroup::General, 1);
        }
        if menu.show_preferences {
            let (group, tab) = (self.preferences.group, self.preferences.tab);
            self.preferences.open_at(settings, unit, group, tab);
        }
        // A capture of a Preferences page: open it once at start.
        if !self.bench_prefs_opened
            && let Some(page) = &crate::app::bench_hooks::hooks().prefs
        {
            self.bench_prefs_opened = true;
            // `<page>/<tab>` opens on that tab.
            let (page, tab) = page.split_once('/').unwrap_or((page.as_str(), ""));
            if let Some(group) = preferences::PrefGroup::named(page, inputs.registry) {
                let tab = group.tabs().iter().position(|t| *t == tab).unwrap_or(0);
                self.preferences.open_at(settings, unit, group, tab);
            }
        }
        menu.open_palette
    }

    /// The start page covers the whole viewport; the scene underneath
    /// carries no bodies.
    fn draw_start(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        out.viewport = ui.available_rect_before_wrap();
        let start = start_page::draw_start_page(
            ui,
            start_page::StartPageInputs {
                recent: f.inputs.recent,
                recoverable: f.inputs.recoverable,
                search: &mut self.recent_search,
                view: &mut self.start_view,
                thumbnails: &mut self.recent_thumbnails,
                document: f.inputs.document,
                registry: f.inputs.registry,
            },
            &mut out.commands,
        );
        if start.show_preferences {
            let (group, tab) = (self.preferences.group, self.preferences.tab);
            self.preferences
                .open_at(f.inputs.settings, f.unit, group, tab);
        }
        self.draw_preferences(ui, f, out);
        hud::draw_toasts(ui.ctx(), out.viewport);
        Self::release_notice(
            &mut self.preferences,
            &mut self.dismissed_release,
            ui.ctx(),
            f.inputs.release,
            out.viewport,
            f.inputs.settings,
            f.unit,
        );
    }

    /// Preferences, when open, and what its pages ask of the packages.
    fn draw_preferences(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &mut f.inputs;
        out.settings_commit = preferences::draw_preferences(
            ui.ctx(),
            &mut self.preferences,
            preferences::PreferencesInputs {
                registry: inputs.registry,
                gpus: inputs.gpus,
                gpu_name: inputs.gpu_name,
                graphics_api: inputs.graphics_api,
                nav_buttons: inputs.nav_buttons,
                scripts: inputs.scripts,
                packages: inputs.packages,
                release: inputs.release,
                store: inputs.store,
            },
        );
        if let Some(request) = self.preferences.package_request.take() {
            out.commands.push(request);
        }
    }

    /// The toolbars, the tools' keys and the command palette.
    fn draw_toolbars_and_palette(
        &mut self,
        ui: &mut Ui,
        f: &mut FrameIn<'_>,
        out: &mut FrameOut,
        open_palette: bool,
    ) {
        let inputs = &mut f.inputs;
        let settings = inputs.settings;
        toolbar::draw_toolbars(
            ui,
            toolbar::ToolbarInputs {
                registry: inputs.registry,
                document: inputs.document,
                host: inputs.host.clone(),
                active_document_object: inputs.active_document_object,
                keymap: &f.keymap,
                layout: &settings.toolbars.rows,
                workbench_tabs: settings.workbench_tabs,
                panels: settings.tabbed_tool_ribbon,
                pins: &settings.tool_panel_pins,
            },
            &mut out.active_workbench,
            &mut out.active_tool,
            &mut out.commands,
        );

        if open_palette {
            self.palette.open();
        }
        // Enablement for the active bench's tools, against the real
        // camera and viewport.
        let ids: Vec<String> = inputs
            .registry
            .tools_for(&out.active_workbench.0)
            .map(|t| t.iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();
        let enabled: Vec<(String, bool)> =
            match inputs.registry.workbench_mut(&out.active_workbench.0) {
                Ok(wb) => {
                    let ctx = host_ctx::panel_ctx(
                        inputs.document,
                        &inputs.host,
                        inputs.active_document_object,
                    );
                    ids.into_iter()
                        .map(|id| {
                            let on = wb.is_tool_enabled(&id, &ctx);
                            (id, on)
                        })
                        .collect()
                }
                Err(_) => Vec::new(),
            };
        let enabled_active = |id: &str| enabled.iter().any(|(i, on)| i == id && *on);
        // A tool's key acts as a click on its button, when the button
        // would take one.
        for id in out.key_tools.drain(..) {
            if enabled_active(&id) {
                out.palette_activate = Some((out.active_workbench.clone(), id));
            }
        }
        self.palette.recent.clone_from(&settings.recent_commands);
        // Every command a call typed into the palette may name.
        let specs = if self.palette.open {
            crate::app::scripts::command_specs(inputs.registry)
        } else {
            Vec::new()
        };
        let palette = command_palette::draw_command_palette(
            ui.ctx(),
            &mut self.palette,
            inputs.document,
            &specs,
            inputs.registry,
            &f.keymap,
            &out.active_workbench,
            &enabled_active,
            &mut out.commands,
        );
        if palette.toggle_console {
            self.console.toggle();
        }
        if palette.print_layout {
            out.locals.push(MenuLocal::Task(OpenTask::PrintLayout));
        }
        if palette.show_preferences {
            let (group, tab) = (self.preferences.group, self.preferences.tab);
            self.preferences.open_at(settings, f.unit, group, tab);
        }
        if palette.activate_tool.is_some() {
            out.palette_activate = palette.activate_tool;
        }
    }

    /// The status bar, the log and the console.
    fn draw_status_and_console(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &f.inputs;
        let settings = inputs.settings;
        let status = status_bar::draw_status_bar(
            ui,
            &status_bar::StatusBarInputs {
                fps: inputs.fps,
                scene_redraws_per_s: inputs.scene_redraws_per_s,
                hovered_point: inputs.hovered_point,
                axis_system: inputs.axis_system,
                display_unit: inputs.document.display_unit(),
                pending_imports: inputs.pending_imports,
                pending_document_open: inputs.pending_document_open,
                kernel_status: inputs.kernel_status.as_deref(),
                kernel_cancellable: inputs.kernel_cancellable,
                kernel_progress: inputs.kernel_progress,
                server: &inputs.server,
                document_saving: inputs.document_saving,
                save_progress: inputs.save_progress,
                nav_device: inputs.nav_device.as_deref(),
                items: inputs.status_items.as_ref(),
                preselect: inputs.hover_card.as_ref().map(|h| h.title.as_str()),
                picks: inputs.picks.as_deref(),
                dimensions: inputs.dimensions.as_deref(),
                quick_measure: &inputs.quick_measure,
                script_running: inputs.script_running,
                recording: inputs.recording,
                panels: status_bar::Panels {
                    log: settings.rendering.show_log_panel,
                    console: self.console.open,
                    assistant: self.assistant.open,
                },
                shown: settings.status_bar,
                recompute_key: f.keymap.text("edit.recompute"),
                mass: inputs.selection_mass.as_ref(),
                mass_material: self.mass_material,
            },
        );
        if let Some(material) = status.mass_material {
            self.mass_material = material;
        }
        if let Some(shown) = status.shown {
            out.commands.push(UiCommand::EditSettings(SettingsEdit::set(
                |s| &mut s.status_bar,
                shown,
            )));
        }
        if status.toggle_log {
            out.commands.push(UiCommand::ToggleLogPanel);
        }
        if status.toggle_console {
            self.console.toggle();
        }
        if status.toggle_assistant {
            self.assistant.toggle();
        }
        if status.recompute {
            out.commands.push(UiCommand::RecomputeAll);
        }
        if status.stop_recording {
            out.commands.push(UiCommand::ToggleRecording);
        }
        if status.cancel_kernel {
            out.commands.push(UiCommand::CancelKernelJob);
        }
        if status.stop_script {
            out.commands.push(UiCommand::StopScript);
        }
        log_view::draw_log_panel(ui, settings.rendering.show_log_panel);
        console_view::draw_console(
            ui,
            &mut self.console,
            inputs.command_ids,
            inputs.script_running,
            &mut out.commands,
        );
    }

    /// The tree and the property panel, and the edits they ask for.
    fn draw_combo(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &mut f.inputs;
        let combo = combo_view::draw_combo_view(
            ui,
            combo_view::ComboViewInputs {
                reveal_body: inputs.reveal_body,
                face_maker: inputs.face_maker,
                reveal_face_maker: inputs.reveal_face_maker,
                active_workbench: out.active_workbench.clone(),
                document: inputs.document,
                registry: inputs.registry,
                host: inputs.host.clone(),
                active_tree_selection: inputs.tree_selection,
                tree_also: inputs.tree_also,
                active_document_object: inputs.active_document_object,
                editing_feature: inputs.editing_feature,
                filter: &mut self.tree_filter,
                property_tab: &mut self.property_tab,
                rename_buffer: &mut self.rename_buffer,
                variables: &mut self.variables,
                tree_share: &mut self.tree_share,
                physical: inputs.physical.as_ref(),
                inertia_references: &inputs.inertia_references,
                inertia_about: &mut self.inertia_about,
                keymap: &f.keymap,
                read_only: inputs.file.read_only,
                read_only_links: inputs.file.read_only_links,
            },
        );
        let commands = &mut out.commands;
        apply_writeback(&combo.writeback, commands, &mut out.tree_selection);
        out.tree_hovered = combo.tree_hovered;
        // An explicit tree click wins over a panel-created feature.
        if combo.tree_selection.is_some() {
            out.tree_selection = combo.tree_selection;
        }
        if let Some(item) = combo.tree_activation {
            commands.push(UiCommand::ActivateTreeItem(item));
        }
        if let Some((node, visible)) = combo.imported_visibility_change {
            commands.push(UiCommand::SetImportedVisibility { node, visible });
        }
        if let Some((body, visible)) = combo.body_visibility_change {
            commands.push(UiCommand::SetBodyVisible { body, visible });
        }
        if let Some((feature, command)) = combo.tree_feature_command {
            commands.push(UiCommand::TreeFeature { feature, command });
        }
        if let Some((item, name)) = combo.rename {
            commands.push(UiCommand::RenameTreeItem { item, name });
        }
        if let Some(item) = combo.delete_item {
            commands.push(UiCommand::DeleteTreeItem(item));
        }
        if let Some(bodies) = combo.repair {
            commands.push(UiCommand::RepairShapes(bodies));
        }
        if combo.details.is_some() {
            self.details = combo.details;
        }
        if let Some(bodies) = combo.convert {
            commands.push(UiCommand::ConvertToSolid(bodies));
        }
        commands.extend(combo.commands);
        if let Some(local) = combo.local {
            out.locals.push(local);
        }
        if let Some((feature, parameter, edit)) = combo.parameter {
            commands.push(UiCommand::SetParameter {
                feature,
                parameter: Box::new(parameter),
                edit,
            });
        }
        if let Some((body, display)) = combo.body_display {
            commands.push(UiCommand::SetBodyDisplay { body, display });
        }
        if let Some((workbench, id, scope)) = combo.bench_command {
            commands.push(UiCommand::BenchCommand {
                workbench,
                id,
                scope,
            });
        }
    }

    /// The assistant's panel, then files dropped anywhere it did not take
    /// them, which open or import.
    fn draw_assistant(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &f.inputs;
        // A capture of the `@` list: typed once the hook's chat is there.
        if !self.bench_chat_typed
            && crate::app::bench_hooks::hooks().chat.is_some()
            && let Some(chat) = inputs.chats.last()
        {
            self.bench_chat_typed = true;
            self.assistant.type_into(ui.ctx(), &chat.id, "@");
        }
        let assistant = assistant::draw_assistant(
            ui,
            &mut self.assistant,
            assistant::AssistantInputs {
                chats: inputs.chats,
                approvals: inputs.approvals,
                agents: inputs
                    .settings
                    .ai
                    .agents
                    .iter()
                    .map(|a| a.name.clone())
                    .collect(),
                document_name: inputs.document.name(),
                document_rules: inputs.document.agent_rules(),
                paste_pressed: inputs.paste_pressed,
                mentions: &inputs.mentions,
                past_chats: &inputs.past_chats,
            },
            &mut out.commands,
        );
        if assistant.open_agent_settings {
            self.preferences.open_at(
                inputs.settings,
                inputs.document.display_unit(),
                preferences::PrefGroup::Ai,
                0,
            );
        }
        let dropped: Vec<std::path::PathBuf> = ui
            .ctx()
            .input_mut(|i| std::mem::take(&mut i.raw.dropped_files))
            .into_iter()
            .map(|f| f.path().to_path_buf())
            .collect();
        if !dropped.is_empty() {
            out.commands.push(UiCommand::OpenDropped(dropped));
        }
    }

    /// The task panel: a bench's task or the application's own.
    fn draw_task(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &mut f.inputs;
        let task_result = task_panel::draw_task_panel(
            ui,
            task_panel::TaskPanelInputs {
                active_workbench: out.active_workbench.clone(),
                document: inputs.document,
                registry: inputs.registry,
                host: inputs.host.clone(),
                active_document_object: inputs.active_document_object,
                task: inputs.task.as_ref(),
                host_task: self.host_task.as_mut().map(|(_, t)| t),
                picked_faces: &inputs.picked_faces,
                picked_edges: &inputs.picked_edges,
                custom_colors: &inputs.settings.rendering.custom_colors,
                printing: &inputs.settings.printing,
                print_layout: inputs.print_layout,
                marking: inputs.marking,
            },
        );
        let commands = &mut out.commands;
        apply_writeback(&task_result.writeback, commands, &mut out.tree_selection);
        if let Some(outcome) = task_result.outcome {
            commands.push(UiCommand::TaskClosed {
                outcome,
                take_back: task_result.take_back,
            });
        }
        commands.extend(task_result.commands);
        match task_result.host_end {
            Some(host_tasks::HostTaskEnd::Accepted(label, recorded)) => {
                self.host_task = None;
                commands.push(UiCommand::TaskClosed {
                    outcome: core_document::TaskOutcome::Accepted { label },
                    take_back: false,
                });
                if !recorded.is_empty() {
                    commands.push(UiCommand::Recorded(recorded));
                }
            }
            Some(host_tasks::HostTaskEnd::Cancelled) => {
                self.host_task = None;
                commands.push(UiCommand::TaskClosed {
                    outcome: core_document::TaskOutcome::Cancelled,
                    take_back: task_result.take_back,
                });
            }
            None => {}
        }
        out.task_open = task_result.open;
    }

    /// The viewport takes what the panels left; split, it is two equal
    /// panes and a divider, the viewport on the left, the pane beside it
    /// on the right.
    fn lay_out_viewport(ui: &mut Ui, f: &FrameIn<'_>, out: &mut FrameOut) {
        out.viewport = ui.available_rect_before_wrap();
        if !f.inputs.split_view {
            return;
        }
        let full = out.viewport;
        let gap = 2.0;
        let half = ((full.width() - gap) * 0.5).floor();
        out.viewport = egui::Rect::from_min_size(full.min, egui::vec2(half, full.height()));
        let side = egui::Rect::from_min_size(
            egui::pos2(full.right() - half, full.top()),
            egui::vec2(half, full.height()),
        );
        out.side = Some(side);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("split_divider"),
        ));
        painter.rect_filled(
            egui::Rect::from_x_y_ranges(out.viewport.right()..=side.left(), full.y_range()),
            0.0,
            ui_kit::tokens::BORDER,
        );
        for (rect, label) in [(out.viewport, "Scan"), (side, "Model")] {
            painter.text(
                rect.center_bottom() - egui::vec2(0.0, ui_kit::tokens::SPACE_6),
                egui::Align2::CENTER_BOTTOM,
                label,
                ui_kit::sans_medium(ui_kit::tokens::FONT_SM),
                ui_kit::tokens::TEXT3,
            );
        }
    }

    /// What the benches lay over the viewport, its HUD and legend, the
    /// context menu, the hover card and the view toolbar.
    fn draw_viewport_overlays(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &f.inputs;
        let ctx = ui.ctx();
        let viewport = out.viewport;
        overlays::draw_screen_space_images(
            ctx,
            viewport,
            inputs.screen_space_images,
            &mut self.image_textures,
        );
        overlays::draw_screen_space_polygons(ctx, viewport, inputs.screen_space_polygons);
        overlays::draw_screen_space_overlays(ctx, viewport, inputs.screen_space_overlays);
        let marks_clip = out.side.map_or(viewport, |side| viewport.union(side));
        overlays::draw_screen_space_marks(ctx, viewport, marks_clip, inputs.screen_space_marks);
        overlays::draw_screen_space_labels(ctx, viewport, marks_clip, inputs.screen_space_labels);
        let footer = [
            match inputs.projection {
                ProjectionMode::Orthographic => "Orthographic".to_string(),
                ProjectionMode::Perspective => "Perspective".to_string(),
            },
            "Shaded + edges".to_string(),
        ];
        match hud::draw_viewport_hud(
            ctx,
            viewport,
            inputs.viewport_hud.as_ref(),
            &footer,
            &inputs.settings.tool_hint,
        ) {
            Some(hud::ToolCardAnswer::Moved(offset)) => {
                out.commands.push(UiCommand::EditSettings(SettingsEdit::set(
                    |s| &mut s.tool_hint.offset,
                    offset,
                )));
            }
            Some(hud::ToolCardAnswer::Hidden) => {
                out.commands.push(UiCommand::EditSettings(
                    SettingsEdit::set(|s| &mut s.tool_hint.show, false)
                        .noting("Tool card hidden; Preferences, General, Tool card shows it again"),
                ));
            }
            None => {}
        }
        if let Some(map) = inputs.scalar_legend {
            let lift = if inputs
                .viewport_hud
                .as_ref()
                .is_some_and(|hud| !hud.legend.is_empty())
            {
                ui_kit::tokens::FONT_SM + ui_kit::tokens::SPACE_2
            } else {
                0.0
            };
            legend::draw(ctx, viewport, map, lift, inputs.document.display_unit());
        }
        if let Some(menu) = &inputs.viewport_menu {
            let mut local = None;
            context_menu::draw(
                ctx,
                menu,
                inputs.document,
                inputs.registry,
                &f.keymap,
                &mut out.commands,
                &mut local,
            );
            if let Some(local) = local {
                out.locals.push(local);
            }
        }
        if let Some(card) = &inputs.hover_card {
            hud::draw_hover_card(ctx, viewport, card, inputs.document.display_unit());
        }
        view_toolbar::draw_view_toolbar(
            ctx,
            viewport,
            &view_toolbar::ViewToolbarState {
                pick_filter: inputs.pick_filter,
                projection: inputs.projection,
                field_of_view_deg: inputs.field_of_view_deg,
                draw_style: inputs.settings.rendering.draw_style,
                section: inputs.section.clone(),
                section_prompt: inputs.section_prompt.clone(),
                scene_bounds: inputs.scene_bounds,
                measure: inputs.measure_bar,
                notes: inputs.note_tool,
                print_bed: inputs.settings.printing.show_bed,
                annotations: inputs.settings.rendering.show_annotations,
            },
            inputs.document,
            &f.keymap,
            &mut out.commands,
        );
    }

    /// The cards over the viewport: the note card, the bench's floating
    /// card, toasts, the radial menu, the notices, isolation and the cube.
    fn draw_viewport_cards(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &mut f.inputs;
        let viewport = out.viewport;
        match &inputs.note_card {
            Some(view) => note_card::draw(
                ui.ctx(),
                viewport,
                ui.ctx().pixels_per_point(),
                view,
                &mut self.note_draft,
                &mut out.commands,
            ),
            None => self.note_draft = None,
        }
        out.card = floating_card::draw(
            ui.ctx(),
            viewport,
            floating_card::Drawn {
                lines: inputs.screen_space_overlays,
                marks: inputs.screen_space_marks,
                labels: inputs.screen_space_labels,
            },
            &mut self.card,
            inputs
                .viewport_hud
                .as_ref()
                .and_then(|hud| hud.card.as_ref()),
            &out.active_workbench,
            inputs.document,
            inputs.registry,
            &inputs.host,
            inputs.active_document_object,
        )
        .and_then(|shown| {
            apply_writeback(&shown.writeback, &mut out.commands, &mut out.tree_selection);
            if let Some(accept) = shown.close {
                out.commands.push(UiCommand::CloseCard { accept });
            }
            // A readout holds nothing for the host to close.
            (!shown.readout).then_some((shown.id, shown.refused))
        });
        hud::draw_toasts(ui.ctx(), viewport);
        if let Some(view) = &inputs.radial {
            radial::draw(ui.ctx(), viewport, view);
        }
        let release_up = Self::release_notice(
            &mut self.preferences,
            &mut self.dismissed_release,
            ui.ctx(),
            inputs.release,
            viewport,
            inputs.settings,
            f.unit,
        );
        if let Some(text) = &inputs.file.newer_notice {
            Self::file_notice(ui.ctx(), viewport, text, release_up, &mut out.commands);
        }
        let document = &*inputs.document;
        if let Some(kept) = document.isolation() {
            let names: Vec<&str> = kept
                .iter()
                .filter_map(|id| document.body(*id))
                .map(|b| b.name.as_str())
                .collect();
            if hud::draw_isolation(ui.ctx(), viewport, &names) {
                out.commands.push(UiCommand::LeaveIsolation);
            }
        }
        if let Some(input) = inputs.orientation_input {
            out.cube_result = orientation_cube::draw(ui.ctx(), viewport, input, &f.cube_config);
        }
    }

    /// The import and export dialogs, a row's details, and the orbit
    /// pivot's mark.
    fn draw_dialogs(&mut self, ui: &mut Ui, f: &mut FrameIn<'_>, out: &mut FrameOut) {
        let inputs = &mut f.inputs;
        if let Some((path, draft)) = inputs.step_import_pending.as_mut() {
            out.step_import_dialog =
                step_import_modal::draw_step_import_modal(ui.ctx(), path, draft);
        }
        if let Some(draft) = inputs.export_pending.as_mut() {
            let configurations = inputs
                .document
                .configurations()
                .map_or(0, |(_, t)| t.rows.len());
            out.export_dialog = export_modal::draw_export_modal(ui.ctx(), draft, configurations);
        }
        if let Some((title, text)) = &self.details
            && !details_modal::draw_details_modal(ui.ctx(), title, text)
        {
            self.details = None;
        }
        if let Some((px, py)) = inputs.pivot_screen_pos {
            overlays::draw_pivot_indicator(ui.ctx(), px, py);
        }
    }

    /// After drawing: the tool a palette or key picked, the menu entries
    /// that change the window, the pointer, the tessellated shapes, and
    /// the remaining interactions folded into commands. Application order
    /// is decided by `apply_ui_commands`, not by the order of this list.
    fn finish_frame(
        &mut self,
        window: &Window,
        full_output: egui::FullOutput,
        f: FrameIn<'_>,
        mut out: FrameOut,
        prev_workbench: ActiveWorkbench,
    ) -> UiFrameOutput {
        let FrameIn { inputs, keymap, .. } = f;
        // A palette pick on another bench switches first, then activates.
        let plain_switch = out.palette_activate.is_none();
        if let Some((bench, id)) = out.palette_activate.take() {
            if bench != out.active_workbench {
                out.active_workbench = bench;
                out.active_tool = ActiveTool::default();
            }
            if let Ok(tools) = inputs.registry.tools_for(&out.active_workbench.0)
                && let Some(tool) = tools.iter().find(|t| t.id == id)
            {
                toolbar::activate_tool(&mut out.active_tool, tools, tool, &id);
            }
        }
        let ctx = self.ctx.clone();
        for local in std::mem::take(&mut out.locals) {
            self.apply_local(&ctx, local, inputs.document, &mut out.commands);
        }
        let workbench_changed = out.active_workbench != prev_workbench;
        if workbench_changed && plain_switch {
            out.active_tool = ActiveTool::default();
        }

        // While work runs away from the window (what spins in the status
        // bar), the pointer says so where nothing else sets it.
        let mut platform_output = full_output.platform_output.clone();
        platform_output.cursor_icon = working_cursor(
            platform_output.cursor_icon,
            inputs.pending_imports > 0
                || inputs.pending_document_open > 0
                || inputs.script_running.is_some(),
        );
        self.state.handle_platform_output(window, platform_output);
        let primitives = self
            .ctx
            .tessellate(full_output.shapes.clone(), full_output.pixels_per_point);

        let ppp = full_output.pixels_per_point;
        let viewport_rect = out.viewport;
        let viewport = ViewportRect {
            x: (viewport_rect.min.x * ppp).max(0.0) as u32,
            y: (viewport_rect.min.y * ppp).max(0.0) as u32,
            width: (viewport_rect.width() * ppp).max(1.0) as u32,
            height: (viewport_rect.height() * ppp).max(1.0) as u32,
        };
        // The same size as the viewport, so one projection serves both.
        let side_viewport = out.side.map(|side| ViewportRect {
            x: (side.min.x * ppp).max(0.0) as u32,
            y: viewport.y,
            width: viewport.width,
            height: viewport.height,
        });

        let mut commands = out.commands;
        let cube_result = out.cube_result;
        if let Some(view) = cube_result.snap_to_view {
            commands.push(if cube_result.fit {
                UiCommand::CameraSnapFit(view)
            } else {
                UiCommand::CameraSnap(view)
            });
        }
        if cube_result.home {
            commands.push(UiCommand::CameraHome);
        }
        if let Some(drag) = cube_result.drag {
            commands.push(UiCommand::CubeDrag(drag));
        }
        if let Some(delta) = cube_result.rotate_delta {
            commands.push(UiCommand::CameraRotate(delta));
        }
        if let Some(commit) = out.settings_commit {
            commands.push(UiCommand::CommitSettings {
                settings: commit.settings,
                display_unit: commit.display_unit,
            });
        }
        if let Some(item) = out.tree_selection {
            commands.push(UiCommand::SelectTreeItem(item));
        }
        match out.step_import_dialog {
            StepImportDialogAction::Confirmed => commands.push(UiCommand::ConfirmStepImport),
            StepImportDialogAction::Cancelled => commands.push(UiCommand::CancelStepImport),
            StepImportDialogAction::None => {}
        }
        match out.export_dialog {
            StepImportDialogAction::Confirmed => commands.push(UiCommand::ConfirmExport),
            StepImportDialogAction::Cancelled => commands.push(UiCommand::CancelExport),
            StepImportDialogAction::None => {}
        }
        if workbench_changed {
            commands.push(UiCommand::SwitchWorkbench {
                from: prev_workbench,
                to: out.active_workbench.clone(),
            });
        }

        self.keymap = keymap;

        let repaint_delay = full_output
            .viewport_output
            .values()
            .map(|v| v.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::MAX);

        UiFrameOutput {
            submission: EguiSubmission {
                pixels_per_point: full_output.pixels_per_point,
                textures_delta: full_output.textures_delta,
                primitives,
            },
            repaint_delay,
            viewport,
            side_viewport,
            active_tool: out.active_tool,
            active_workbench: out.active_workbench,
            commands,
            task_open: out.task_open,
            card: out.card,
            tree_hovered: out.tree_hovered,
            print_layout_open: matches!(
                self.host_task,
                Some((_, host_tasks::HostTask::PrintLayout))
            ),
            mark_arm: match &self.host_task {
                Some((_, host_tasks::HostTask::Mark(task))) => Some(task.arm),
                _ => None,
            },
        }
    }
}
