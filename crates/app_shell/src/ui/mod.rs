mod assistant;
mod combo_view;
mod command_palette;
pub use command_palette::RECENT_KEPT as PALETTE_RECENT_KEPT;
mod context_menu;
mod variables_view;
pub use context_menu::ViewportMenu;
mod commands;
mod console_view;
mod details_modal;
mod export_modal;
mod feature_tree;
mod host_ctx;
mod hud;
pub(crate) use hud::VIEW_LAYER;
mod inputs;
pub(crate) mod keymap;
mod log_view;
mod menu_bar;
mod overlays;
mod preferences;
mod property_panel;
mod release_notes;
mod save_over_modal;
mod start_page;
pub(crate) mod status_bar;
mod step_import_modal;
mod tab_bar;
mod task_panel;
pub(crate) mod toolbar;
mod unsaved_modal;
pub(crate) mod view_toolbar;
pub use view_toolbar::PickFilter;

mod array_task;
mod body_menu;
mod host_tasks;
mod texture_task;
pub use body_menu::{MenuLocal, OpenTask};
pub use commands::{BodyEdit, ConfigEdit, EditCommand, FileCommand, StartKind, UiCommand};
pub use host_ctx::HostCtxParams;
pub use inputs::{FileMarks, HoverCard, Physical, UiFrameInputs};
pub use step_import_modal::StepImportDialogAction;
pub use tab_bar::TabInfo;

use core_document::WorkbenchId;
use egui::Context;
use egui_winit::{State, egui as egui_core};
use render_wgpu::EguiSubmission;
use settings::ProjectionMode;
use winit::{event::WindowEvent, window::Window};

use crate::orientation_cube::{self, OrientationCubeConfig, OrientationCubeResult};

/// Which top-level screen the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The launch page: new-document cards, recent files.
    Start,
    /// The modelling workspace.
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWorkbench(pub WorkbenchId);

/// The panels around the view as they were left: kept between runs.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PanelLayout {
    /// Each resizable panel's size across the window, by the panel's name.
    pub sizes: std::collections::BTreeMap<String, f32>,
    pub console_open: bool,
    pub assistant_open: bool,
    /// The property panel's page.
    pub property_page: property_panel::PropertyTab,
    /// The tree's share of the left column.
    pub tree_share: Option<f32>,
}

/// The panels whose size is dragged, by name, and whether each is sized
/// along the window's width.
const RESIZABLE_PANELS: [(&str, bool); 5] = [
    ("combo_view", true),
    ("task_panel", true),
    ("assistant_panel", true),
    ("console_panel", false),
    ("log_panel", false),
];

#[derive(Debug, Clone, Default)]
pub struct ActiveTool {
    /// Set of active tool IDs. For Radio tools, only one per group is active.
    /// For Check tools, multiple can be active. For Action tools, this is cleared after handling.
    pub active_ids: std::collections::HashSet<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Per-frame data (submission, viewport, tool/workbench state) plus the
/// list of actions the user triggered this frame.
pub struct UiFrameOutput {
    pub submission: EguiSubmission,
    /// egui's own repaint request: `Duration::ZERO` for "next frame please"
    /// (animations), a finite delay for timed repaints (caret blink), and
    /// effectively-infinite when content is static.
    pub repaint_delay: std::time::Duration,
    pub viewport: ViewportRect,
    pub active_tool: ActiveTool,
    pub active_workbench: ActiveWorkbench,
    pub commands: Vec<UiCommand>,
    /// A workbench task is open in the right panel after this frame.
    pub task_open: bool,
    /// The tree row under the pointer.
    pub tree_hovered: Option<TreeItemId>,
    /// The Print layout task is open after this frame.
    pub print_layout_open: bool,
}

pub struct UiLayer {
    ctx: Context,
    state: State,
    preferences: preferences::PreferencesState,
    /// `PRINTCAD_BENCH_PREFS=<page>` (or `<page>/<tab>`) opened Preferences
    /// there once.
    bench_prefs_opened: bool,
    /// The newer release whose notice was put away.
    dismissed_release: Option<String>,
    palette: command_palette::PaletteState,
    /// The pictures a bench lays over the viewport, uploaded once each.
    image_textures: std::collections::HashMap<u64, egui::TextureHandle>,
    orientation_cube_config: OrientationCubeConfig,
    /// Substring filter over the model tree; UI-local.
    tree_filter: String,
    property_tab: property_panel::PropertyTab,
    rename_buffer: Option<(TreeItemId, String)>,
    /// The start page's recent-files filter; UI-local.
    recent_search: String,
    start_view: start_page::StartView,
    /// A message opened from a tree row's "!": the row and the text.
    details: Option<(String, String)>,
    /// The workbench keys last sent to the workbenches.
    workbench_keys: Option<std::collections::HashMap<String, Vec<core_document::Chord>>>,
    /// The keys in effect as of the last frame.
    keymap: keymap::Keymap,
    recent_thumbnails: start_page::ThumbnailCache,
    /// Keys the workbench consumed that egui also queued; egui must not
    /// act on them (Tab would move focus, Enter would accept the task).
    swallowed_keys: Vec<egui::Key>,
    console: console_view::ConsoleState,
    assistant: assistant::AssistantState,
    variables: variables_view::VariablesState,
    /// The tree's share of the left column, as its divider was dragged.
    tree_share: f32,
    /// The application's own task in the right panel, and the tab it was
    /// opened in.
    host_task: Option<(Option<uuid::Uuid>, host_tasks::HostTask)>,
    /// A bench's task was open last frame, which an application task waits
    /// behind.
    bench_task_open: bool,
    /// The tab on screen last frame.
    active_tab: Option<uuid::Uuid>,
}

impl UiLayer {
    /// Open one of the application's tasks in the right panel, for the
    /// tab `tab`. A task already open is left as it stands.
    pub fn open_task(
        &mut self,
        document: &core_document::Document,
        open: OpenTask,
        tab: impl Into<Option<uuid::Uuid>>,
    ) {
        let task = match open {
            OpenTask::Placement(body) => host_tasks::HostTask::placement(document, body),
            OpenTask::Appearance(body, face) => {
                host_tasks::HostTask::appearance(document, body, face)
            }
            OpenTask::History(feature) => host_tasks::HostTask::history(document, feature),
            OpenTask::Texture(body, face) => {
                host_tasks::HostTask::Texture(texture_task::TextureTask::open(document, body, face))
            }
            OpenTask::PrintLayout => host_tasks::HostTask::PrintLayout,
            OpenTask::AddNote(body, at) => host_tasks::HostTask::add_note(document, body, at),
            OpenTask::EditNote(note) => {
                let Some(task) = host_tasks::HostTask::edit_note(document, note) else {
                    return;
                };
                task
            }
            OpenTask::Array(body) => {
                host_tasks::HostTask::Array(Box::new(array_task::ArrayTask::open(document, body)))
            }
            OpenTask::ComponentArray(component) => {
                let Some(task) = array_task::ArrayTask::open_component(document, component) else {
                    return;
                };
                host_tasks::HostTask::Array(Box::new(task))
            }
        };
        self.host_task = Some((tab.into(), task));
    }

    /// Act on a menu entry that changes the window: a page of the property
    /// panel, a rename, the console's input, an application task.
    fn apply_local(
        &mut self,
        ctx: &Context,
        local: MenuLocal,
        document: &core_document::Document,
        commands: &mut Vec<UiCommand>,
    ) {
        match local {
            MenuLocal::Properties(item, tab) => {
                commands.push(UiCommand::SelectTreeItem(item));
                self.property_tab = tab;
            }
            MenuLocal::Rename(item) => {
                commands.push(UiCommand::SelectTreeItem(item));
                self.property_tab = property_panel::PropertyTab::Data;
                self.rename_buffer = None;
                ctx.data_mut(|d| d.insert_temp(egui::Id::new(property_panel::FOCUS_LABEL), true));
            }
            MenuLocal::Console(text) => self.console.insert(&text),
            MenuLocal::Task(open) => {
                if self.bench_task_open {
                    crate::log_panel::warn("Finish the task that is open first");
                    return;
                }
                self.open_task(document, open, self.active_tab);
            }
        }
    }

    pub fn new(window: &Window) -> Self {
        let ctx = Context::default();
        ui_kit::apply_theme(&ctx);
        let state = State::new(
            ctx.clone(),
            egui_core::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );

        orientation_cube::warm_face_textures(&ctx);

        Self {
            ctx,
            state,
            preferences: preferences::PreferencesState::default(),
            bench_prefs_opened: false,
            dismissed_release: None,
            palette: command_palette::PaletteState::default(),
            image_textures: std::collections::HashMap::new(),
            orientation_cube_config: OrientationCubeConfig::default(),
            tree_filter: String::new(),
            property_tab: property_panel::PropertyTab::default(),
            rename_buffer: None,
            recent_search: String::new(),
            start_view: Default::default(),
            details: None,
            workbench_keys: None,
            keymap: keymap::Keymap::default(),
            recent_thumbnails: Default::default(),
            swallowed_keys: Vec::new(),
            console: console_view::ConsoleState::load(),
            assistant: Default::default(),
            variables: Default::default(),
            host_task: None,
            bench_task_open: false,
            active_tab: None,
            tree_share: combo_view::TREE_SHARE,
        }
    }

    /// The panels as they stand now.
    pub fn panel_layout(&self) -> PanelLayout {
        let sizes = RESIZABLE_PANELS
            .iter()
            .filter_map(|&(name, across)| {
                let state =
                    egui::containers::panel::PanelState::load(&self.ctx, egui::Id::new(name))?;
                let size = state.size();
                Some((name.to_string(), if across { size.x } else { size.y }))
            })
            .collect();
        PanelLayout {
            sizes,
            console_open: self.console.open,
            assistant_open: self.assistant.open,
            property_page: self.property_tab,
            tree_share: Some(self.tree_share),
        }
    }

    /// Lay the panels out as `layout` left them: each panel takes its size
    /// when it is next drawn, within the range it allows.
    pub fn restore_panel_layout(&mut self, layout: &PanelLayout) {
        for &(name, across) in &RESIZABLE_PANELS {
            let Some(&size) = layout.sizes.get(name) else {
                continue;
            };
            if !size.is_finite() || size <= 0.0 {
                continue;
            }
            let extent = if across {
                egui::vec2(size, 1.0)
            } else {
                egui::vec2(1.0, size)
            };
            let state = egui::containers::panel::PanelState {
                outer_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, extent),
            };
            self.ctx
                .data_mut(|d| d.insert_persisted(egui::Id::new(name), state));
        }
        self.console.open = layout.console_open;
        self.assistant.open = layout.assistant_open;
        self.property_tab = layout.property_page;
        if let Some(share) = layout.tree_share.filter(|s| (0.0..=1.0).contains(s)) {
            self.tree_share = share;
        }
    }

    /// The row of copies the open array task lays out, for the view's
    /// spacing handle: the array, its direction and its spacing.
    pub fn array_row(&self) -> Option<(core_document::ArrayId, [f32; 3], f32)> {
        match &self.host_task {
            Some((tab, host_tasks::HostTask::Array(task))) if *tab == self.active_tab => {
                task.row_handle()
            }
            _ => None,
        }
    }

    /// The open array task's row spacing, set by the view's handle.
    pub fn drag_array_spacing(&mut self, document: &mut core_document::Document, spacing: f32) {
        if let Some((_, host_tasks::HostTask::Array(task))) = &mut self.host_task {
            task.drag_spacing(document, spacing);
        }
    }

    /// A text field owns the keyboard.
    pub fn wants_keyboard_input(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Whether something floating owns the pointer: a modal, a menu, a
    /// tooltip, a combo list.
    ///
    /// The viewport and the panels around it live in egui's background
    /// layer, so anything above that is drawn over the 3D scene and should
    /// be the only thing acting on the wheel or showing what is under the
    /// cursor. egui answers with the modal's own layer whenever one is open,
    /// wherever the pointer is, which is what makes a modal modal. Overlays
    /// the shell paints without interaction (the HUD cards, the axis
    /// triad) are built `interactable(false)` and are skipped here.
    pub fn pointer_over_floating_ui(&self) -> bool {
        let Some(pos) = self.ctx.pointer_latest_pos() else {
            return false;
        };
        self.ctx
            .layer_id_at(pos)
            .is_some_and(|layer| layer.order != egui::Order::Background)
    }

    /// A newer printCAD is out: a card at the view's bottom right, kept
    /// until put away, with the release and the Updates page a click away.
    /// Whether the card is up.
    fn release_notice(
        preferences: &mut preferences::PreferencesState,
        dismissed_release: &mut Option<String>,
        ctx: &Context,
        release: &crate::app::updates::ReleaseCheck,
        viewport: egui::Rect,
        settings: &settings::UserSettings,
        unit: core_document::Unit,
    ) -> bool {
        let crate::app::updates::ReleaseCheck::Found {
            tag,
            page,
            newer: true,
        } = release
        else {
            return false;
        };
        if dismissed_release.as_ref() == Some(tag) {
            return false;
        }
        let (details, dismiss) = hud::draw_release_notice(ctx, viewport, tag, page);
        if details {
            preferences.open_at(settings, unit, preferences::PrefGroup::Updates, 0);
        }
        if dismiss || details {
            *dismissed_release = Some(tag.clone());
        }
        true
    }

    /// A newer printCAD wrote the document's file: a card at the view's
    /// bottom right, above the release card when that is up, offering
    /// Save As.
    fn file_notice(
        ctx: &Context,
        viewport: egui::Rect,
        text: &str,
        release_up: bool,
        commands: &mut Vec<UiCommand>,
    ) {
        let lift = if release_up {
            ctx.memory(|m| m.area_rect(egui::Id::new("release_notice")))
                .map_or(0.0, |card| card.height() + ui_kit::tokens::SPACE_2)
        } else {
            0.0
        };
        let (save_as, dismiss) = hud::draw_file_notice(ctx, viewport, lift, text);
        if save_as {
            commands.push(UiCommand::File(FileCommand::SaveAs));
        }
        if save_as || dismiss {
            commands.push(UiCommand::DismissFileNotice);
        }
    }

    /// Whether `chord` runs a command while `active` is the workbench, as
    /// the keys stood at the last frame.
    pub fn is_shortcut(&self, chord: core_document::Chord, active: &WorkbenchId) -> bool {
        self.keymap.lookup(chord, active).is_some()
    }

    /// What the system clipboard holds as text.
    pub fn clipboard_text(&mut self) -> Option<String> {
        self.state.clipboard_text()
    }

    /// The workbench consumed `key`: keep egui from acting on it too.
    pub fn swallow_key(&mut self, key: egui::Key) {
        self.swallowed_keys.push(key);
    }

    /// Physical pixels per logical UI pixel, including the UI's zoom.
    pub fn pixels_per_point(&self) -> f32 {
        self.ctx.pixels_per_point()
    }

    /// Drop keyboard focus (a click landed on the viewport, not a widget).
    pub fn release_focus(&self) {
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    pub fn run(&mut self, window: &Window, inputs: UiFrameInputs<'_>) -> UiFrameOutput {
        let UiFrameInputs {
            screen,
            recent,
            recoverable,
            unsaved_question,
            active_tool: host_active_tool,
            active_workbench: host_active_workbench,
            settings,
            document,
            registry,
            host,
            orientation_input,
            planar_view_lock,
            fps,
            scene_redraws_per_s,
            gpu_name,
            graphics_api,
            gpus,
            hovered_point,
            pivot_screen_pos,
            axis_system,
            tree_selection: active_tree_selection,
            tree_also,
            active_document_object,
            editing_feature,
            viewport_hud,
            status_items,
            task,
            hover_card,
            picks,
            dimensions,
            physical,
            field_of_view_deg,
            projection,
            section,
            pick_filter,
            scene_bounds,
            screen_space_overlays,
            screen_space_polygons,
            screen_space_images,
            screen_space_marks,
            screen_space_labels,
            pending_imports,
            pending_document_open,
            kernel_status,
            kernel_cancellable,
            kernel_progress,
            server,
            tabs,
            measuring,
            measure_bar,
            reveal_body,
            viewport_menu,
            picked_faces,
            picked_edges,
            nav_device,
            nav_buttons,
            document_saving,
            save_progress,
            mut step_import_pending,
            mut export_pending,
            scripts,
            packages,
            release,
            store,
            console_attention,
            command_ids,
            script_running,
            recording,
            chats,
            approvals,
            assistant_attention,
            print_layout,
            file,
        } = inputs;

        let mut raw_input = self.state.take_egui_input(window);
        if !self.swallowed_keys.is_empty() {
            let swallowed = std::mem::take(&mut self.swallowed_keys);
            raw_input
                .events
                .retain(|e| !matches!(e, egui::Event::Key { key, .. } if swallowed.contains(key)));
        }
        let prev_workbench = host_active_workbench.clone();
        let mut active_workbench = host_active_workbench;
        // Seed tool state from the host, not a UiLayer copy: the host owns
        // consumption of Action tools, and a stale parallel copy here would
        // resurrect them every frame (infinite "New Body" loop).
        let mut active_tool = host_active_tool;

        let mut cube_config = self.orientation_cube_config.clone();
        cube_config.planar_only = planar_view_lock;
        let mut commands: Vec<UiCommand> = Vec::new();
        let mut settings_commit: Option<preferences::Commit> = None;
        let mut palette_activate: Option<(ActiveWorkbench, String)> = None;
        let mut cube_result = OrientationCubeResult::default();
        let mut viewport_rect_logical = egui::Rect::NOTHING;
        let mut task_open = false;
        let mut tree_selection = None;
        let mut tree_hovered = None;
        let mut step_import_dialog = StepImportDialogAction::default();
        let mut export_dialog = StepImportDialogAction::default();

        let document_name = document.name().to_owned();
        let document_dirty = document.metadata().dirty();
        let breadcrumb = editing_feature
            .and_then(|id| document.get_feature_meta(id))
            .map(|node| node.name.clone());

        let keymap = keymap::Keymap::build(registry, &settings.keyboard, scripts);
        if console_attention {
            self.console.open = true;
        }
        if assistant_attention {
            self.assistant.open = true;
        }

        // The workbenches hear of their keys at the start and after every
        // change, not every frame.
        let workbench_keys = keymap.workbench_keys();
        if self.workbench_keys.as_ref() != Some(&workbench_keys) {
            registry.notify_shortcuts(&workbench_keys);
            self.workbench_keys = Some(workbench_keys);
        }

        // An application task belongs to its tab and its subject; either
        // gone, it is left as it stands.
        let active_tab = tabs.iter().find(|t| t.active).map(|t| t.tab);
        self.active_tab = active_tab;
        self.bench_task_open = task.is_some();
        if self
            .host_task
            .as_ref()
            .is_some_and(|(tab, t)| *tab != active_tab || !t.still_there(document))
        {
            self.host_task = None;
        }
        // A bench's task taking the panel ends the application's, its
        // edits kept.
        if task.is_some()
            && let Some((_, open)) = self.host_task.take()
        {
            commands.push(UiCommand::TaskClosed(
                core_document::TaskOutcome::Accepted {
                    label: open.title(document),
                },
            ));
        }
        // Menu entries that change the window, acted on once the frame is
        // drawn.
        let mut locals: Vec<MenuLocal> = Vec::new();
        let full_output = self.ctx.run_ui(raw_input, |ui| {
            // Shortcuts first, so their keys never reach a widget. They hold
            // their fire while a dialog of their own has the keyboard.
            let mut key_tools: Vec<String> = Vec::new();
            if !self.preferences.open && !self.palette.open {
                let focus = keymap::KeyFocus {
                    typing: ui.ctx().egui_wants_keyboard_input(),
                    numeric: registry
                        .workbench(&active_workbench.0)
                        .is_ok_and(|wb| wb.takes_numeric_input()),
                    text: registry
                        .workbench(&active_workbench.0)
                        .is_ok_and(|wb| wb.takes_text_input()),
                    have_document: screen == Screen::Workspace,
                };
                let active_tab = tabs.iter().find(|t| t.active).map(|t| t.tab);
                for hit in keymap::take_pressed(ui.ctx(), &keymap, &active_workbench.0, focus) {
                    match hit.target {
                        keymap::Target::Host(action) => {
                            let state = keymap::HostState {
                                active_tab,
                                section_on: section.is_some(),
                                document,
                                tree_selection: active_tree_selection,
                                editing: editing_feature.is_some(),
                            };
                            match keymap::host_outcome(action, &state) {
                                keymap::HostOutcome::Command(command) => commands.push(command),
                                keymap::HostOutcome::OpenPalette => self.palette.open(),
                                keymap::HostOutcome::ToggleConsole => self.console.toggle(),
                                keymap::HostOutcome::ToggleAssistant => self.assistant.toggle(),
                                keymap::HostOutcome::Local(local) => {
                                    locals.push(local);
                                }
                                keymap::HostOutcome::OpenPreferences => {
                                    let (group, tab) =
                                        (self.preferences.group, self.preferences.tab);
                                    self.preferences.open_at(
                                        settings,
                                        document.display_unit(),
                                        group,
                                        tab,
                                    );
                                }
                                keymap::HostOutcome::Nothing => {}
                            }
                        }
                        keymap::Target::Tool => key_tools.push(hit.id),
                        keymap::Target::Script(path) => {
                            commands.push(UiCommand::RunScriptFile(path));
                        }
                        keymap::Target::Action => commands.push(UiCommand::BenchAction {
                            workbench: active_workbench.0.clone(),
                            id: hit.id,
                        }),
                    }
                }
            }

            let menu = menu_bar::draw_menu_bar(
                ui,
                menu_bar::MenuBarInputs {
                    keymap: &keymap,
                    registry,
                    document_name: &document_name,
                    document_dirty,
                    breadcrumb: breadcrumb.as_deref(),
                    show_log_panel: settings.rendering.show_log_panel,
                    show_annotations: settings.rendering.show_annotations,
                    show_grid: settings.rendering.show_grid,
                    show_origin_planes: settings.rendering.show_origin_planes,
                    fullscreen: window.fullscreen().is_some(),
                    show_console: self.console.open,
                    show_assistant: self.assistant.open,
                    projection,
                    draw_style: settings.rendering.draw_style,
                    recent,
                    screen,
                    active_tab_blank: tabs.iter().any(|t| t.active && t.blank),
                    active_tab: tabs.iter().find(|t| t.active).map(|t| t.tab),
                    scripts,
                    recording,
                },
                &mut active_workbench,
                &mut active_tool,
                &mut commands,
            );

            tab_bar::draw_tab_bar(ui, &tabs, &mut commands);

            // About lands on its page; Preferences keeps the last group.
            let unit = document.display_unit();
            if menu.toggle_console {
                self.console.toggle();
            }
            if menu.toggle_assistant {
                self.assistant.toggle();
            }
            if menu.print_layout {
                locals.push(MenuLocal::Task(OpenTask::PrintLayout));
            }

            if menu.check_updates {
                self.preferences
                    .open_at(settings, unit, preferences::PrefGroup::Updates, 0);
                commands.push(UiCommand::CheckForUpdates);
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
                && let Ok(page) = std::env::var("PRINTCAD_BENCH_PREFS")
            {
                self.bench_prefs_opened = true;
                // `<page>/<tab>` opens on that tab.
                let (page, tab) = page.split_once('/').unwrap_or((page.as_str(), ""));
                if let Some(group) = preferences::PrefGroup::named(page, registry) {
                    let tab = group.tabs().iter().position(|t| *t == tab).unwrap_or(0);
                    self.preferences.open_at(settings, unit, group, tab);
                }
            }
            let mut open_palette = menu.open_palette;

            if screen == Screen::Start {
                // The start page covers the whole viewport; the scene
                // underneath carries no bodies.
                viewport_rect_logical = ui.available_rect_before_wrap();
                let start = start_page::draw_start_page(
                    ui,
                    start_page::StartPageInputs {
                        recent,
                        recoverable,
                        search: &mut self.recent_search,
                        view: &mut self.start_view,
                        thumbnails: &mut self.recent_thumbnails,
                        document,
                        registry,
                    },
                    &mut commands,
                );
                if start.show_preferences {
                    let (group, tab) = (self.preferences.group, self.preferences.tab);
                    self.preferences.open_at(settings, unit, group, tab);
                }
                settings_commit = preferences::draw_preferences(
                    ui.ctx(),
                    &mut self.preferences,
                    preferences::PreferencesInputs {
                        registry,
                        gpus,
                        gpu_name,
                        graphics_api,
                        #[cfg(target_arch = "wasm32")]
                        nav_device: nav_device.as_deref(),
                        nav_buttons,
                        scripts,
                        packages,
                        release,
                        store,
                    },
                );
                if let Some(request) = self.preferences.package_request.take() {
                    commands.push(request);
                }
                hud::draw_toasts(ui.ctx(), viewport_rect_logical);
                if let Some(name) = &unsaved_question {
                    unsaved_modal::draw(ui.ctx(), name, &mut commands);
                }
                if let Some(question) = &file.save_over {
                    save_over_modal::draw(ui.ctx(), question, &mut commands);
                }
                Self::release_notice(
                    &mut self.preferences,
                    &mut self.dismissed_release,
                    ui.ctx(),
                    release,
                    viewport_rect_logical,
                    settings,
                    unit,
                );
                return;
            }

            let mut toggle_console = false;
            toolbar::draw_toolbars(
                ui,
                toolbar::ToolbarInputs {
                    registry,
                    document,
                    host: host.clone(),
                    active_document_object,
                    show_print_bed: settings.printing.show_bed,
                    show_annotations: settings.rendering.show_annotations,
                    measuring,
                    keymap: &keymap,
                    scripts,
                    console_open: self.console.open,
                    recording,
                    layout: &settings.toolbars.rows,
                },
                &mut active_workbench,
                &mut active_tool,
                &mut commands,
                &mut open_palette,
                &mut toggle_console,
            );
            if toggle_console {
                self.console.toggle();
            }

            if open_palette {
                self.palette.open();
            }
            {
                // Enablement for the active bench's tools, against the real
                // camera and viewport.
                let ids: Vec<String> = registry
                    .tools_for(&active_workbench.0)
                    .map(|t| t.iter().map(|d| d.id.clone()).collect())
                    .unwrap_or_default();
                let enabled: Vec<(String, bool)> = match registry.workbench_mut(&active_workbench.0)
                {
                    Ok(wb) => {
                        let ctx = host_ctx::panel_ctx(document, &host, active_document_object);
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
                // A tool's key acts as a click on its button, when the
                // button would take one.
                for id in key_tools.drain(..) {
                    if enabled_active(&id) {
                        palette_activate = Some((active_workbench.clone(), id));
                    }
                }
                self.palette.recent.clone_from(&settings.recent_commands);
                let palette = command_palette::draw_command_palette(
                    ui.ctx(),
                    &mut self.palette,
                    registry,
                    &keymap,
                    &active_workbench,
                    &enabled_active,
                    &mut commands,
                );
                if palette.toggle_console {
                    self.console.toggle();
                }
                if palette.print_layout {
                    locals.push(MenuLocal::Task(OpenTask::PrintLayout));
                }
                if palette.show_preferences {
                    let (group, tab) = (self.preferences.group, self.preferences.tab);
                    self.preferences.open_at(settings, unit, group, tab);
                }
                if palette.activate_tool.is_some() {
                    palette_activate = palette.activate_tool;
                }
            }

            // Bottom bars before the side panels so they span the width.
            let status = status_bar::draw_status_bar(
                ui,
                &status_bar::StatusBarInputs {
                    fps,
                    scene_redraws_per_s,
                    hovered_point,
                    axis_system,
                    display_unit: document.display_unit(),
                    pending_imports,
                    pending_document_open,
                    kernel_status: kernel_status.as_deref(),
                    kernel_cancellable,
                    kernel_progress,
                    server: &server,
                    document_saving,
                    save_progress,
                    nav_device: nav_device.as_deref(),
                    items: status_items.as_ref(),
                    preselect: hover_card.as_ref().map(|h| h.title.as_str()),
                    picks: picks.as_deref(),
                    dimensions: dimensions.as_deref(),
                    script_running,
                    recording,
                    panels: status_bar::Panels {
                        log: settings.rendering.show_log_panel,
                        console: self.console.open,
                        assistant: self.assistant.open,
                    },
                    shown: settings.status_bar,
                },
            );
            if let Some(shown) = status.shown {
                commands.push(UiCommand::SetStatusBar(shown));
            }
            if status.toggle_log {
                commands.push(UiCommand::ToggleLogPanel);
            }
            if status.toggle_console {
                self.console.toggle();
            }
            if status.toggle_assistant {
                self.assistant.toggle();
            }
            if status.stop_recording {
                commands.push(UiCommand::ToggleRecording);
            }
            if status.cancel_kernel {
                commands.push(UiCommand::CancelKernelJob);
            }
            if status.stop_script {
                commands.push(UiCommand::StopScript);
            }
            log_view::draw_log_panel(ui, settings.rendering.show_log_panel);
            console_view::draw_console(
                ui,
                &mut self.console,
                command_ids,
                script_running,
                &mut commands,
            );

            let combo = combo_view::draw_combo_view(
                ui,
                combo_view::ComboViewInputs {
                    reveal_body,
                    active_workbench: active_workbench.clone(),
                    document,
                    registry,
                    host: host.clone(),
                    active_tree_selection,
                    tree_also,
                    active_document_object,
                    editing_feature,
                    filter: &mut self.tree_filter,
                    property_tab: &mut self.property_tab,
                    rename_buffer: &mut self.rename_buffer,
                    variables: &mut self.variables,
                    tree_share: &mut self.tree_share,
                    physical: physical.as_ref(),
                    keymap: &keymap,
                    read_only: file.read_only,
                    read_only_links: file.read_only_links,
                },
            );
            apply_writeback(&combo.writeback, &mut commands, &mut tree_selection);
            tree_hovered = combo.tree_hovered;
            // An explicit tree click wins over a panel-created feature.
            if combo.tree_selection.is_some() {
                tree_selection = combo.tree_selection;
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
                locals.push(local);
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

            let assistant = assistant::draw_assistant(
                ui,
                &mut self.assistant,
                assistant::AssistantInputs {
                    chats,
                    approvals,
                    agents: settings.ai.agents.iter().map(|a| a.name.clone()).collect(),
                    document_name: document.name(),
                    document_rules: document.agent_rules(),
                },
                &mut commands,
            );
            if assistant.open_agent_settings {
                self.preferences.open_at(
                    settings,
                    document.display_unit(),
                    preferences::PrefGroup::Ai,
                    0,
                );
            }
            // Files dropped anywhere the assistant did not take them open or
            // import.
            let dropped: Vec<std::path::PathBuf> = ui
                .ctx()
                .input_mut(|i| std::mem::take(&mut i.raw.dropped_files))
                .into_iter()
                .map(|f| f.path().to_path_buf())
                .collect();
            if !dropped.is_empty() {
                commands.push(UiCommand::OpenDropped(dropped));
            }

            let task_result = task_panel::draw_task_panel(
                ui,
                task_panel::TaskPanelInputs {
                    active_workbench: active_workbench.clone(),
                    document,
                    registry,
                    host: host.clone(),
                    active_document_object,
                    task: task.as_ref(),
                    host_task: self.host_task.as_mut().map(|(_, t)| t),
                    picked_faces: &picked_faces,
                    picked_edges: &picked_edges,
                    custom_colors: &settings.rendering.custom_colors,
                    printing: &settings.printing,
                    print_layout,
                },
            );
            apply_writeback(&task_result.writeback, &mut commands, &mut tree_selection);
            if let Some(outcome) = task_result.outcome {
                commands.push(UiCommand::TaskClosed(outcome));
            }
            commands.extend(task_result.commands);
            match task_result.host_end {
                Some(host_tasks::HostTaskEnd::Accepted(label, recorded)) => {
                    self.host_task = None;
                    commands.push(UiCommand::TaskClosed(
                        core_document::TaskOutcome::Accepted { label },
                    ));
                    if !recorded.is_empty() {
                        commands.push(UiCommand::Recorded(recorded));
                    }
                }
                Some(host_tasks::HostTaskEnd::Cancelled) => {
                    self.host_task = None;
                    commands.push(UiCommand::TaskClosed(core_document::TaskOutcome::Cancelled));
                }
                None => {}
            }
            task_open = task_result.open;

            settings_commit = preferences::draw_preferences(
                ui.ctx(),
                &mut self.preferences,
                preferences::PreferencesInputs {
                    registry,
                    gpus,
                    gpu_name,
                    graphics_api,
                    #[cfg(target_arch = "wasm32")]
                    nav_device: nav_device.as_deref(),
                    nav_buttons,
                    scripts,
                    packages,
                    release,
                    store,
                },
            );
            if let Some(request) = self.preferences.package_request.take() {
                commands.push(request);
            }

            viewport_rect_logical = ui.available_rect_before_wrap();

            overlays::draw_screen_space_images(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_images,
                &mut self.image_textures,
            );
            overlays::draw_screen_space_polygons(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_polygons,
            );
            overlays::draw_screen_space_overlays(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_overlays,
            );
            overlays::draw_screen_space_marks(ui.ctx(), viewport_rect_logical, screen_space_marks);
            overlays::draw_screen_space_labels(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_labels,
            );
            let footer = [
                match projection {
                    ProjectionMode::Orthographic => "Orthographic".to_string(),
                    ProjectionMode::Perspective => "Perspective".to_string(),
                },
                "Shaded + edges".to_string(),
            ];
            hud::draw_viewport_hud(
                ui.ctx(),
                viewport_rect_logical,
                viewport_hud.as_ref(),
                &footer,
            );
            if let Some(menu) = &viewport_menu {
                let mut local = None;
                context_menu::draw(
                    ui.ctx(),
                    menu,
                    document,
                    registry,
                    &keymap,
                    &mut commands,
                    &mut local,
                );
                if let Some(local) = local {
                    locals.push(local);
                }
            }
            if let Some(card) = &hover_card {
                hud::draw_hover_card(
                    ui.ctx(),
                    viewport_rect_logical,
                    card,
                    document.display_unit(),
                );
            }
            view_toolbar::draw_view_toolbar(
                ui.ctx(),
                viewport_rect_logical,
                &view_toolbar::ViewToolbarState {
                    pick_filter,
                    projection,
                    field_of_view_deg,
                    draw_style: settings.rendering.draw_style,
                    section,
                    scene_bounds,
                    measure: measure_bar,
                },
                &keymap,
                &mut commands,
            );
            hud::draw_toasts(ui.ctx(), viewport_rect_logical);
            if let Some(name) = &unsaved_question {
                unsaved_modal::draw(ui.ctx(), name, &mut commands);
            }
            if let Some(question) = &file.save_over {
                save_over_modal::draw(ui.ctx(), question, &mut commands);
            }
            let release_up = Self::release_notice(
                &mut self.preferences,
                &mut self.dismissed_release,
                ui.ctx(),
                release,
                viewport_rect_logical,
                settings,
                unit,
            );
            if let Some(text) = &file.newer_notice {
                Self::file_notice(
                    ui.ctx(),
                    viewport_rect_logical,
                    text,
                    release_up,
                    &mut commands,
                );
            }

            if let Some(input) = orientation_input {
                cube_result =
                    orientation_cube::draw(ui.ctx(), viewport_rect_logical, input, &cube_config);
            }

            if let Some((path, draft)) = step_import_pending.as_mut() {
                step_import_dialog =
                    step_import_modal::draw_step_import_modal(ui.ctx(), path, draft);
            }
            if let Some(draft) = export_pending.as_mut() {
                let configurations = document.configurations().map_or(0, |(_, t)| t.rows.len());
                export_dialog = export_modal::draw_export_modal(ui.ctx(), draft, configurations);
            }
            if let Some((title, text)) = &self.details
                && !details_modal::draw_details_modal(ui.ctx(), title, text)
            {
                self.details = None;
            }

            if let Some((px, py)) = pivot_screen_pos {
                overlays::draw_pivot_indicator(ui.ctx(), px, py);
            }
        });

        // A palette pick on another bench switches first, then activates.
        let plain_switch = palette_activate.is_none();
        if let Some((bench, id)) = palette_activate {
            if bench != active_workbench {
                active_workbench = bench;
                active_tool = ActiveTool::default();
            }
            if let Ok(tools) = registry.tools_for(&active_workbench.0)
                && let Some(tool) = tools.iter().find(|t| t.id == id)
            {
                toolbar::activate_tool(&mut active_tool, tools, tool, &id);
            }
        }
        let ctx = self.ctx.clone();
        for local in locals {
            self.apply_local(&ctx, local, document, &mut commands);
        }
        let workbench_changed = active_workbench != prev_workbench;
        if workbench_changed && plain_switch {
            active_tool = ActiveTool::default();
        }

        // While work runs away from the window (what spins in the status
        // bar), the pointer says so where nothing else sets it.
        let mut platform_output = full_output.platform_output.clone();
        platform_output.cursor_icon = working_cursor(
            platform_output.cursor_icon,
            pending_imports > 0 || pending_document_open > 0 || script_running.is_some(),
        );
        self.state.handle_platform_output(window, platform_output);
        let primitives = self
            .ctx
            .tessellate(full_output.shapes.clone(), full_output.pixels_per_point);

        let ppp = full_output.pixels_per_point;
        let viewport = ViewportRect {
            x: (viewport_rect_logical.min.x * ppp).max(0.0) as u32,
            y: (viewport_rect_logical.min.y * ppp).max(0.0) as u32,
            width: (viewport_rect_logical.width() * ppp).max(1.0) as u32,
            height: (viewport_rect_logical.height() * ppp).max(1.0) as u32,
        };

        // Fold the remaining interactions into commands. Application order
        // is decided by `apply_ui_commands`, not by the order of this list.
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
        if let Some(commit) = settings_commit {
            commands.push(UiCommand::CommitSettings {
                settings: commit.settings,
                display_unit: commit.display_unit,
            });
        }
        if let Some(item) = tree_selection {
            commands.push(UiCommand::SelectTreeItem(item));
        }
        match step_import_dialog {
            StepImportDialogAction::Confirmed => commands.push(UiCommand::ConfirmStepImport),
            StepImportDialogAction::Cancelled => commands.push(UiCommand::CancelStepImport),
            StepImportDialogAction::None => {}
        }
        match export_dialog {
            StepImportDialogAction::Confirmed => commands.push(UiCommand::ConfirmExport),
            StepImportDialogAction::Cancelled => commands.push(UiCommand::CancelExport),
            StepImportDialogAction::None => {}
        }
        if workbench_changed {
            commands.push(UiCommand::SwitchWorkbench {
                from: prev_workbench,
                to: active_workbench.clone(),
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
            active_tool,
            active_workbench,
            commands,
            task_open,
            tree_hovered,
            print_layout_open: matches!(
                self.host_task,
                Some((_, host_tasks::HostTask::PrintLayout))
            ),
        }
    }
}

/// The pointer while work runs: the system's busy-in-the-background
/// arrow over anything showing the plain arrow; a text field's caret, a
/// grip's hand and the like stay.
fn working_cursor(cursor: egui::CursorIcon, working: bool) -> egui::CursorIcon {
    if working && cursor == egui::CursorIcon::Default {
        egui::CursorIcon::Progress
    } else {
        cursor
    }
}

/// Turn a panel hook's write-backs into commands. A feature the hook
/// created becomes the tree selection so the host's active object follows;
/// every request the hook made goes to the host as it is.
fn apply_writeback(
    writeback: &host_ctx::PanelWriteback,
    commands: &mut Vec<UiCommand>,
    tree_selection: &mut Option<TreeItemId>,
) {
    match writeback.active_object_changed {
        Some(Some(id)) => *tree_selection = Some(TreeItemId::Feature(id)),
        Some(None) => commands.push(UiCommand::ReleaseActiveObject),
        None => {}
    }
    for request in &writeback.requests {
        commands.push(UiCommand::HostRequest(request.clone()));
    }
    if !writeback.recorded.is_empty() {
        commands.push(UiCommand::Recorded(writeback.recorded.clone()));
    }
}

pub use feature_tree::{TreeFeatureCommand, TreeItemId};

#[cfg(test)]
mod working_cursor_tests {
    use egui::CursorIcon;

    /// While work runs the plain arrow becomes the busy arrow; a cursor a
    /// widget chose stays; idle, nothing changes.
    #[test]
    fn the_pointer_shows_work_running() {
        assert_eq!(
            super::working_cursor(CursorIcon::Default, true),
            CursorIcon::Progress
        );
        assert_eq!(
            super::working_cursor(CursorIcon::Text, true),
            CursorIcon::Text
        );
        assert_eq!(
            super::working_cursor(CursorIcon::Default, false),
            CursorIcon::Default
        );
    }
}
