mod assistant;
mod bench_ui;
mod combo_view;
mod command_palette;
mod context_menu;
mod variables_view;
pub use context_menu::{OtherPick, ViewportMenu};
mod commands;
mod console_view;
mod details_modal;
mod export_modal;
mod feature_tree;
mod floating_card;
mod frame;
mod host_ctx;
mod hud;
pub(crate) use hud::VIEW_LAYER;
mod inputs;
pub(crate) mod keymap;
mod legend;
mod log_view;
mod menu_bar;
mod mesh_units_modal;
mod note_card;
mod overlays;
mod preferences;
mod property_panel;
pub(crate) mod radial;
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
pub(crate) mod widget_map;
pub use view_toolbar::PickFilter;

mod array_task;
mod body_menu;
mod host_tasks;
mod task_blocks;
mod texture_task;
pub use body_menu::{MenuLocal, OpenTask};
pub use commands::{
    BodyEdit, ConfigEdit, EditCommand, FileCommand, SavedSectionAction, SettingsEdit, StartKind,
    UiCommand,
};
pub use host_ctx::HostCtxParams;
pub use inputs::{FileMarks, HoverCard, Physical, SelectionMass, UiFrameInputs};
pub use property_panel::InertiaReference;
pub use step_import_modal::StepImportDialogAction;
pub use tab_bar::TabInfo;

use core_document::WorkbenchId;
use egui::Context;
use egui_winit::{State, egui as egui_core};
use render_wgpu::EguiSubmission;
use winit::{event::WindowEvent, window::Window};

use crate::orientation_cube::{self, OrientationCubeConfig};

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
    /// The pane beside the viewport when the view is split.
    pub side_viewport: Option<ViewportRect>,
    pub active_tool: ActiveTool,
    pub active_workbench: ActiveWorkbench,
    pub commands: Vec<UiCommand>,
    /// A workbench task is open in the right panel after this frame.
    pub task_open: bool,
    /// The bench's floating card on screen this frame: its id, and
    /// whether its OK is held back.
    pub card: Option<(String, bool)>,
    /// The tree row under the pointer.
    pub tree_hovered: Option<TreeItemId>,
    /// The Print layout task is open after this frame.
    pub print_layout_open: bool,
    /// The marking tool in hand, while the Mark surface task is open.
    pub mark_arm: Option<crate::app::marking::MarkArm>,
}

pub struct UiLayer {
    ctx: Context,
    state: State,
    preferences: preferences::PreferencesState,
    /// `PRINTCAD_BENCH_PREFS=<page>` (or `<page>/<tab>`) opened Preferences
    /// there once.
    bench_prefs_opened: bool,
    /// `PRINTCAD_BENCH_CHAT`'s chat has had `@` typed into it.
    bench_chat_typed: bool,
    /// `PRINTCAD_BENCH_UI`'s pointer steps, played once the scene settles.
    bench_ui: Option<bench_ui::BenchUi>,
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
    /// What the Physical group takes the inertia about, by name; the
    /// centre of mass when unset or no longer offered.
    inertia_about: Option<String>,
    /// The start page's recent-files filter; UI-local.
    recent_search: String,
    /// The material the status bar's mass is reckoned in, an index into
    /// the materials list; UI-local, PLA at start.
    mass_material: usize,
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
    /// What is typed on the note card.
    note_draft: note_card::NoteDraft,
    /// The bench's floating card as last drawn.
    card: floating_card::CardState,
}

impl UiLayer {
    /// Let `PRINTCAD_BENCH_UI`'s steps play: the scene has settled.
    pub fn arm_bench_ui(&mut self) {
        if let Some(steps) = self.bench_ui.as_mut() {
            steps.arm();
        }
    }

    /// `PRINTCAD_BENCH_UI`'s viewport click this frame, as fractions of
    /// the viewport.
    pub fn take_bench_view_click(&mut self) -> Option<egui::Pos2> {
        self.bench_ui.as_mut()?.take_view_click()
    }

    /// `PRINTCAD_BENCH_UI` has steps still to play.
    pub fn bench_ui_pending(&self) -> bool {
        self.bench_ui
            .as_ref()
            .is_some_and(bench_ui::BenchUi::pending)
    }

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
            OpenTask::Mark(body) => host_tasks::HostTask::Mark(host_tasks::MarkTask::new(body)),
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
            bench_chat_typed: false,
            bench_ui: bench_ui::BenchUi::from_env(),
            mass_material: 0,
            dismissed_release: None,
            palette: command_palette::PaletteState::default(),
            image_textures: std::collections::HashMap::new(),
            orientation_cube_config: OrientationCubeConfig::default(),
            tree_filter: String::new(),
            property_tab: property_panel::PropertyTab::default(),
            rename_buffer: None,
            inertia_about: None,
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
            note_draft: None,
            card: floating_card::CardState::default(),
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
    /// Whether Preferences is open, holding its draft of the settings.
    pub fn preferences_open(&self) -> bool {
        self.preferences.open
    }

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

    /// Put `text` on the system clipboard with the next frame's output.
    pub fn copy_text(&self, text: String) {
        self.ctx.copy_text(text);
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
