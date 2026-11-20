//! The Preferences dialog: a modal with a group rail, a tab strip per
//! group and rows edited on a draft that Apply or OK commit.

use axes::AxisPreset;
use core_document::{DocumentService, Unit};
use egui::{
    Align, Context, CornerRadius, Frame, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    Vec2, pos2, vec2,
};
use kernel_api::LinearDeflectionMode;
use settings::{
    NavigationStyle, OrbitYawAxis, ProjectionMode, SixDofMotion, SlicerFormat, UserSettings,
};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Card, Note, PrefRow, QtyField, note_card, overline, pref_block, pref_group, pref_matches,
    primary_button, secondary_button, select_field,
};
use ui_kit::{mono, sans, sans_medium, sans_semibold};

mod ai;
mod display;
mod general;
mod import;
mod input;
mod keyboard;
mod packages;
mod printing;
mod radial;
mod units;
mod updates;

use ai::ai_page;
use display::display_page;
use general::general_page;
use import::import_page;
use input::input_page;
use keyboard::keyboard_page;
use packages::*;
use printing::printing_page;
use radial::radial_page;
use units::units_page;
use updates::updates_page;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PrefGroup {
    #[default]
    General,
    Display,
    Input,
    Keyboard,
    /// A registered workbench's own page, by its place in registration
    /// order.
    Workbench(usize),
    /// Installed workbench packages.
    Packages,
    Units,
    ImportExport,
    Printing,
    Ai,
    Updates,
}

impl PrefGroup {
    /// The rail, top to bottom: the app's groups with one page per
    /// registered workbench after Keyboard, then the packages page.
    pub fn all(registry: &DocumentService) -> Vec<PrefGroup> {
        let mut groups = vec![
            PrefGroup::General,
            PrefGroup::Display,
            PrefGroup::Input,
            PrefGroup::Keyboard,
        ];
        // A bench with nothing to set gets no page.
        let ids = registry.ids();
        groups.extend(
            (0..ids.len())
                .filter(|&i| {
                    registry
                        .workbench(&ids[i])
                        .is_ok_and(|bench| bench.has_settings())
                })
                .map(PrefGroup::Workbench),
        );
        groups.extend([
            PrefGroup::Packages,
            PrefGroup::Units,
            PrefGroup::ImportExport,
            PrefGroup::Printing,
        ]);
        groups.extend([PrefGroup::Ai, PrefGroup::Updates]);
        groups
    }

    /// The group whose label is `name`, in any case.
    pub fn named(name: &str, registry: &DocumentService) -> Option<PrefGroup> {
        Self::all(registry)
            .into_iter()
            .find(|g| g.label(registry).eq_ignore_ascii_case(name))
    }

    pub fn label(self, registry: &DocumentService) -> String {
        match self {
            PrefGroup::General => "General".to_string(),
            PrefGroup::Display => "Display".to_string(),
            PrefGroup::Input => "Input".to_string(),
            PrefGroup::Keyboard => "Keyboard".to_string(),
            PrefGroup::Workbench(i) => registry
                .ids()
                .get(i)
                .and_then(|id| registry.descriptor(id))
                .map(|d| d.label.clone())
                .unwrap_or_default(),
            PrefGroup::Packages => "Workbench packages".to_string(),
            PrefGroup::Units => "Units".to_string(),
            PrefGroup::ImportExport => "Import / Export".to_string(),
            PrefGroup::Printing => "3D printing".to_string(),
            PrefGroup::Ai => "AI agents".to_string(),
            PrefGroup::Updates => "Updates".to_string(),
        }
    }

    pub fn tabs(self) -> &'static [&'static str] {
        match self {
            PrefGroup::General => &["Interface", "About"],
            PrefGroup::Display => &["Camera", "Lighting", "Rendering"],
            PrefGroup::Input => &["Mouse", "6-DoF mouse", "Radial menu"],
            PrefGroup::Keyboard => &["Shortcuts"],
            PrefGroup::Workbench(_) => &["General"],
            PrefGroup::Packages => &["Installed", "Browse", "Stores"],
            PrefGroup::Units => &["Units"],
            PrefGroup::ImportExport => &["STEP", "IGES"],
            PrefGroup::Printing => &["Printer"],
            PrefGroup::Ai => &["Agents"],
            PrefGroup::Updates => &["Updates"],
        }
    }
}

/// The dialog's state: what is shown and the uncommitted draft.
pub struct PreferencesState {
    pub open: bool,
    pub group: PrefGroup,
    pub tab: usize,
    pub draft: UserSettings,
    pub draft_unit: Unit,
    pub search: String,
    /// Where the dialog was dragged to, and how big it was left. `None`
    /// means it has not been moved, so it opens centred.
    pos: Option<egui::Pos2>,
    size: Vec2,
    /// The frame the dialog opened on: the search field takes focus once.
    just_opened: bool,
    /// The shortcut waiting for a key press: its id, and whether the key
    /// is added to its keys rather than replacing them.
    recording: Option<(String, bool)>,
    /// An install or a removal the packages page asked for, for the host.
    pub package_request: Option<super::UiCommand>,
    /// The GitHub address typed on the packages page.
    package_repo: String,
    /// What the store's list is narrowed to: the words typed, and the
    /// category picked (0 for every one).
    store_query: String,
    store_category: usize,
    /// The workbench whose radial menu the Input page edits, by its place
    /// in registration order.
    radial_bench: usize,
    /// The store picked to narrow the list to (0 for every one), and the
    /// address typed to add one.
    store_pick: usize,
    store_add: String,
}

impl Default for PreferencesState {
    fn default() -> Self {
        Self {
            open: false,
            group: PrefGroup::General,
            tab: 0,
            draft: UserSettings::default(),
            draft_unit: Unit::Mm,
            search: String::new(),
            pos: None,
            size: DIALOG,
            just_opened: false,
            recording: None,
            package_request: None,
            package_repo: String::new(),
            store_query: String::new(),
            store_category: 0,
            radial_bench: 0,
            store_pick: 0,
            store_add: String::new(),
        }
    }
}

impl PreferencesState {
    /// Open on `group`/`tab` with a fresh draft of the live values.
    pub fn open_at(&mut self, current: &UserSettings, unit: Unit, group: PrefGroup, tab: usize) {
        if !self.open {
            self.draft = current.clone();
            self.draft_unit = unit;
            self.search.clear();
            self.just_opened = true;
        }
        self.open = true;
        self.group = group;
        self.tab = tab;
    }
}

pub struct PreferencesInputs<'a> {
    pub registry: &'a mut DocumentService,
    pub gpus: &'a [String],
    pub gpu_name: Option<&'a str>,
    pub graphics_api: Option<&'a str>,
    /// How many buttons the connected 6-DoF mouse has, so the page offers a
    /// row per button it actually owns. Zero when none is connected.
    pub nav_buttons: u32,
    /// The scripts folder's scripts, which take keys like any command.
    pub scripts: &'a [crate::script_library::ScriptEntry],
    /// The workbench packages found at start and since.
    pub packages: &'a [workbenches::PackageStatus],
    /// What the last look for a newer printCAD found.
    pub release: &'a package_store::ReleaseCheck,
    /// The workbench store's list.
    pub store: &'a package_store::StoreView,
}

/// The values to commit, when Apply or OK was pressed this frame.
pub struct Commit {
    pub settings: Box<UserSettings>,
    pub display_unit: Unit,
}

const DIALOG: Vec2 = vec2(900.0, 620.0);
/// Small enough to tuck out of the way, large enough that the rail, a tab
/// strip and a row still fit.
const DIALOG_MIN: Vec2 = vec2(640.0, 420.0);
/// The corner that resizes the dialog.
const GRIP: f32 = 16.0;
const HEADER: f32 = 44.0;
const RAIL: f32 = 200.0;
const TABS: f32 = 36.0;
const FOOTER: f32 = 52.0;

pub fn draw_preferences(
    ctx: &Context,
    state: &mut PreferencesState,
    mut inputs: PreferencesInputs<'_>,
) -> Option<Commit> {
    if !state.open {
        return None;
    }
    // A shortcut being recorded takes the next key before any widget can
    // act on it; Escape gives up.
    if let Some((id, add)) = state.recording.clone()
        && let Some(chord) = super::keymap::take_any_chord(ctx)
    {
        state.recording = None;
        let escape = core_document::Chord::key(core_document::KeyCode::Escape);
        let keymap =
            super::keymap::Keymap::build(inputs.registry, &state.draft.keyboard, inputs.scripts);
        if chord != escape
            && let Some(binding) = keymap.get(&id)
        {
            let mut keys = if add {
                binding.keys.clone()
            } else {
                Vec::new()
            };
            if !keys.contains(&chord) {
                keys.push(chord);
            }
            super::keymap::set_keys(&mut state.draft.keyboard, binding, keys);
        }
    }
    let mut commit = None;
    let mut close = false;
    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(0);
    // The dialog keeps whatever the user dragged and resized it to; until
    // then it opens centred at its own size.
    // Built by hand rather than from `Modal::default_area`, which anchors
    // itself to the centre every frame: an anchored area cannot be dragged.
    let id = egui::Id::new("preferences");
    let area = egui::Area::new(id)
        .kind(egui::UiKind::Modal)
        .sense(Sense::hover())
        .order(egui::Order::Foreground)
        .interactable(true);
    let area = match state.pos {
        Some(pos) => area.fixed_pos(pos),
        None => area.anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO),
    };
    let screen = ctx.content_rect();
    state.size = state.size.max(DIALOG_MIN).min(screen.size());
    let mut drag = Vec2::ZERO;
    let mut resize = Vec2::ZERO;

    let modal = egui::Modal::new(id)
        .area(area)
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(state.size, Sense::hover());
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
            let footer = Rect::from_min_max(
                pos2(rect.left(), rect.bottom() - FOOTER),
                rect.right_bottom(),
            );
            let rail = Rect::from_min_max(
                pos2(rect.left(), header.bottom()),
                pos2(rect.left() + RAIL, footer.top()),
            );
            let content =
                Rect::from_min_max(pos2(rail.right(), header.bottom()), footer.right_top());

            drag = draw_header(ui, header, state, &mut close);
            draw_rail(ui, rail, state, inputs.registry);
            draw_content(ui, content, state, &mut inputs);
            draw_footer(ui, footer, state, &mut commit, &mut close);

            resize = draw_resize_grip(ui, rect);

            let painter = ui.painter();
            painter.hline(rect.x_range(), header.bottom(), Stroke::new(1.0, BORDER));
            painter.vline(rail.right(), rail.y_range(), Stroke::new(1.0, BORDER));
            painter.hline(rect.x_range(), footer.top(), Stroke::new(1.0, BORDER));
        });

    if resize != Vec2::ZERO {
        state.size = (state.size + resize).max(DIALOG_MIN).min(screen.size());
    }
    if drag != Vec2::ZERO {
        // Keep the header reachable: the dialog can go off the edges, but
        // never so far that there is nothing left to grab.
        let at = state.pos.unwrap_or(modal.response.rect.min) + drag;
        state.pos = Some(egui::pos2(
            at.x.clamp(screen.left() - state.size.x + RAIL, screen.right() - RAIL),
            at.y.clamp(screen.top(), screen.bottom() - HEADER),
        ));
    }
    if modal.should_close() {
        close = true;
    }
    if close {
        state.open = false;
    }
    commit
}

fn region(ui: &mut Ui, rect: Rect, layout: Layout) -> Ui {
    ui.new_child(UiBuilder::new().max_rect(rect).layout(layout))
}

/// The corner grip: drag it to resize. Returns this frame's change.
fn draw_resize_grip(ui: &mut Ui, rect: Rect) -> Vec2 {
    let corner = Rect::from_min_max(rect.right_bottom() - vec2(GRIP, GRIP), rect.right_bottom());
    let grip = ui.interact(corner, ui.id().with("resize"), Sense::drag());
    if grip.hovered() || grip.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
    let painter = ui.painter();
    let stroke = Stroke::new(1.0, if grip.dragged() { TEXT2 } else { TEXT3 });
    for step in 1..=3 {
        let inset = GRIP - step as f32 * 4.0;
        painter.line_segment(
            [
                corner.right_bottom() - vec2(inset, 3.0),
                corner.right_bottom() - vec2(3.0, inset),
            ],
            stroke,
        );
    }
    grip.drag_delta()
}

/// The header doubles as the dialog's handle; returns how far it was dragged
/// this frame.
fn draw_header(ui: &mut Ui, rect: Rect, state: &mut PreferencesState, close: &mut bool) -> Vec2 {
    let handle = ui.interact(rect, ui.id().with("drag"), Sense::drag());
    if handle.hovered() || handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let mut h = region(
        ui,
        rect.shrink2(vec2(16.0, 0.0)),
        Layout::left_to_right(Align::Center),
    );
    h.label(
        RichText::new("Preferences")
            .font(sans_semibold(FONT_LG))
            .color(TEXT1),
    );
    h.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = SPACE_3;
        let (x_rect, x) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
        if x.hovered() {
            ui.painter().rect_filled(x_rect, RADIUS_SM, BG2);
        }
        if let Some(tex) = ui_kit::icon::texture(ui.ctx(), "close") {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            ui.painter().image(tex.id(), x_rect.shrink(4.0), uv, TEXT2);
        }
        if x.on_hover_text("Close without applying").clicked() {
            *close = true;
        }
        let (box_rect, _) = ui.allocate_exact_size(vec2(220.0, INPUT), Sense::hover());
        ui.painter().rect(
            box_rect,
            5.0,
            BG2,
            Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
        let mut inner = ui.new_child(
            UiBuilder::new()
                .max_rect(box_rect.shrink2(vec2(10.0, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        inner.spacing_mut().item_spacing.x = SPACE_2;
        ui_kit::icon::draw(&mut inner, "search", 14.0, TEXT3);
        let edit = inner.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("Search settings…")
                .frame(egui::Frame::NONE)
                .font(sans(FONT_SM))
                .desired_width(f32::INFINITY),
        );
        if state.just_opened {
            edit.request_focus();
            state.just_opened = false;
        }
    });
    handle.drag_delta()
}

fn draw_rail(ui: &mut Ui, rect: Rect, state: &mut PreferencesState, registry: &DocumentService) {
    let mut rail = region(
        ui,
        rect.shrink2(vec2(10.0, 12.0)),
        Layout::top_down(Align::Min),
    );
    rail.spacing_mut().item_spacing.y = 2.0;
    for group in PrefGroup::all(registry) {
        let active = state.group == group;
        let (row, response) =
            rail.allocate_exact_size(vec2(rail.available_width(), 30.0), Sense::click());
        if active {
            rail.painter().rect_filled(row, RADIUS_SM, BG2);
        } else if response.hovered() {
            rail.painter()
                .rect_filled(row, RADIUS_SM, with_alpha(BG2, 0.6));
        }
        let color = if active { TEXT1 } else { TEXT2 };
        rail.painter().text(
            pos2(row.left() + 26.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            group.label(registry),
            sans_medium(FONT_SM),
            color,
        );
        if response.clicked() && state.group != group {
            state.group = group;
            state.tab = 0;
        }
    }
}

fn draw_content(
    ui: &mut Ui,
    rect: Rect,
    state: &mut PreferencesState,
    inputs: &mut PreferencesInputs<'_>,
) {
    let tabs = Rect::from_min_size(rect.min, vec2(rect.width(), TABS));
    let mut strip = region(
        ui,
        tabs.shrink2(vec2(16.0, 0.0)),
        Layout::left_to_right(Align::Center),
    );
    strip.spacing_mut().item_spacing.x = 2.0;
    for (i, tab) in state.group.tabs().iter().enumerate() {
        if ui_kit::widgets::Tab::new(tab, state.tab == i)
            .height(TAB_BAR - 4.0)
            .show(&mut strip)
            .selected
        {
            state.tab = i;
        }
    }
    ui.painter()
        .hline(rect.x_range(), tabs.bottom(), Stroke::new(1.0, BORDER));

    let body = Rect::from_min_max(pos2(rect.left(), tabs.bottom()), rect.max);
    let mut page = region(ui, body, Layout::top_down(Align::Min));
    page.set_clip_rect(body);
    egui::ScrollArea::vertical()
        .id_salt(("prefs_page", state.group, state.tab))
        .auto_shrink([false, false])
        .show(&mut page, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = SPACE_2;
                    let filter = state.search.trim().to_lowercase();
                    if !filter.is_empty() {
                        // A search spans every group and tab.
                        search_results(ui, state, inputs, &filter);
                        return;
                    }
                    match state.group {
                        PrefGroup::General => general_page(ui, state, inputs, &filter),
                        PrefGroup::Display => display_page(ui, state, inputs, &filter),
                        PrefGroup::Input => input_page(ui, state, inputs, &filter),
                        PrefGroup::Keyboard => {
                            keyboard_page(ui, state, inputs.registry, inputs.scripts, &filter)
                        }
                        PrefGroup::Workbench(i) => workbench_page(ui, inputs.registry, i, &filter),
                        PrefGroup::Packages => {
                            packages_page(ui, state, inputs.packages, inputs.store, &filter)
                        }
                        PrefGroup::Units => units_page(ui, state, &filter),
                        PrefGroup::ImportExport => import_page(ui, state, &filter),
                        PrefGroup::Printing => printing_page(ui, state, &filter),
                        PrefGroup::Ai => ai_page(ui, state, &filter),
                        PrefGroup::Updates => updates_page(ui, state, inputs.release, &filter),
                    }
                });
        });
}

fn draw_footer(
    ui: &mut Ui,
    rect: Rect,
    state: &mut PreferencesState,
    commit: &mut Option<Commit>,
    close: &mut bool,
) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: RADIUS_LG as u8,
            se: RADIUS_LG as u8,
        },
        BG2,
    );
    // A one-button-tall strip, centred: buttons do not stretch to the
    // footer's height.
    let strip = Rect::from_x_y_ranges(
        (rect.left() + 16.0)..=(rect.right() - 16.0),
        (rect.center().y - 14.0)..=(rect.center().y + 14.0),
    );
    let mut f = region(ui, strip, Layout::left_to_right(Align::Center));
    if secondary_button(&mut f, "Reset page")
        .on_hover_text("Put this group's settings back to their defaults")
        .clicked()
    {
        reset_group(state);
    }
    f.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        if primary_button(ui, "OK").clicked() {
            *commit = Some(Commit {
                settings: Box::new(state.draft.clone()),
                display_unit: state.draft_unit,
            });
            *close = true;
        }
        if secondary_button(ui, "Apply").clicked() {
            *commit = Some(Commit {
                settings: Box::new(state.draft.clone()),
                display_unit: state.draft_unit,
            });
        }
        if secondary_button(ui, "Cancel").clicked() {
            *close = true;
        }
    });
}

/// Defaults for the fields the current group edits.
fn reset_group(state: &mut PreferencesState) {
    let defaults = UserSettings::default();
    match state.group {
        PrefGroup::General => {
            state.draft.fps_cap = defaults.fps_cap;
            state.draft.rendering.show_log_panel = defaults.rendering.show_log_panel;
            state.draft.diagnostics = defaults.diagnostics;
            state.draft.session = defaults.session;
        }
        PrefGroup::Display => {
            let camera = &mut state.draft.camera;
            camera.projection = defaults.camera.projection;
            camera.fov_degrees = defaults.camera.fov_degrees;
            camera.ortho_height_mm = defaults.camera.ortho_height_mm;
            camera.min_focal_distance = defaults.camera.min_focal_distance;
            camera.max_focal_distance = defaults.camera.max_focal_distance;
            camera.auto_near_far = defaults.camera.auto_near_far;
            camera.near_far_near_ratio = defaults.camera.near_far_near_ratio;
            camera.near_far_depth_ratio_cap = defaults.camera.near_far_depth_ratio_cap;
            camera.near_far_margin = defaults.camera.near_far_margin;
            camera.view_transition_ms = defaults.camera.view_transition_ms;
            camera.axis_preset = defaults.camera.axis_preset;
            state.draft.lighting = defaults.lighting;
            state.draft.rendering.show_grid = defaults.rendering.show_grid;
            state.draft.rendering.grid_plane = defaults.rendering.grid_plane;
            state.draft.rendering.grid_color = defaults.rendering.grid_color;
            state.draft.rendering.grid_opacity = defaults.rendering.grid_opacity;
            state.draft.rendering.grid_style = defaults.rendering.grid_style;
            state.draft.rendering.show_origin_planes = defaults.rendering.show_origin_planes;
            state.draft.rendering.show_origin_lines = defaults.rendering.show_origin_lines;
            state.draft.rendering.solid_section_caps = defaults.rendering.solid_section_caps;
            state.draft.rendering.msaa_samples = defaults.rendering.msaa_samples;
            state.draft.rendering.curve_step_deg = defaults.rendering.curve_step_deg;
            state.draft.rendering.selection_color = defaults.rendering.selection_color;
            state.draft.rendering.selection_opacity = defaults.rendering.selection_opacity;
            state.draft.rendering.preview_color = defaults.rendering.preview_color;
            state.draft.rendering.preview_opacity = defaults.rendering.preview_opacity;
            state.draft.preferred_gpu = defaults.preferred_gpu;
        }
        PrefGroup::Input => {
            let camera = &mut state.draft.camera;
            camera.zoom_to_cursor = defaults.camera.zoom_to_cursor;
            camera.invert_zoom = defaults.camera.invert_zoom;
            camera.wheel_zoom_factor = defaults.camera.wheel_zoom_factor;
            camera.orbit_sensitivity = defaults.camera.orbit_sensitivity;
            camera.orbit_pivot_pick = defaults.camera.orbit_pivot_pick;
            camera.pan_sensitivity = defaults.camera.pan_sensitivity;
            camera.orbit_yaw_axis = defaults.camera.orbit_yaw_axis;
            camera.click_drag_threshold_px = defaults.camera.click_drag_threshold_px;
            camera.navigation = defaults.camera.navigation;
            state.draft.radial = defaults.radial.clone();
            state.draft.sixdof = defaults.sixdof;
        }
        PrefGroup::Keyboard => state.draft.keyboard = defaults.keyboard,
        PrefGroup::Units => state.draft_unit = Unit::Mm,
        PrefGroup::ImportExport => state.draft.import = defaults.import,
        PrefGroup::Printing => state.draft.printing = defaults.printing,
        PrefGroup::Ai => {
            state.draft.ai.ask_before_changes = defaults.ai.ask_before_changes;
            state.draft.ai.notify = defaults.ai.notify;
            state.draft.ai.rules = defaults.ai.rules;
        }
        PrefGroup::Packages => state.draft.packages = defaults.packages,
        PrefGroup::Updates => state.draft.updates = defaults.updates,
        PrefGroup::Workbench(_) => {}
    }
}

/// The page of the `index`-th registered workbench: whatever it draws.
fn workbench_page(ui: &mut Ui, registry: &mut DocumentService, index: usize, filter: &str) {
    let Some(id) = registry.ids().get(index).cloned() else {
        return;
    };
    if let Some(wb) = bench_panel::bench_ui(registry, &id) {
        wb.ui_settings(ui, filter);
    }
}

/// Every page in turn, each group filtered; pages with no match draw
/// nothing, so only hits remain.
fn search_results(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &mut PreferencesInputs<'_>,
    filter: &str,
) {
    let (group, tab) = (state.group, state.tab);
    for g in PrefGroup::all(inputs.registry) {
        for t in 0..g.tabs().len() {
            state.group = g;
            state.tab = t;
            match g {
                PrefGroup::General => general_page(ui, state, inputs, filter),
                PrefGroup::Display => display_page(ui, state, inputs, filter),
                PrefGroup::Input => input_page(ui, state, inputs, filter),
                PrefGroup::Keyboard => {
                    keyboard_page(ui, state, inputs.registry, inputs.scripts, filter)
                }
                PrefGroup::Workbench(i) => workbench_page(ui, inputs.registry, i, filter),
                PrefGroup::Packages => {
                    packages_page(ui, state, inputs.packages, inputs.store, filter)
                }
                PrefGroup::Units => units_page(ui, state, filter),
                PrefGroup::ImportExport => import_page(ui, state, filter),
                PrefGroup::Printing => printing_page(ui, state, filter),
                PrefGroup::Ai => ai_page(ui, state, filter),
                PrefGroup::Updates => updates_page(ui, state, inputs.release, filter),
            }
        }
    }
    state.group = group;
    state.tab = tab;
    if ui.min_rect().height() < 4.0 {
        ui.label(
            RichText::new("No setting matches.")
                .font(sans(FONT_SM))
                .color(TEXT3),
        );
    }
}

#[cfg(test)]
mod rail {
    use super::*;
    use core_document::{
        BenchBuild, BenchCommands, BenchFeatures, BenchOutput, BenchPanels, BenchParameters,
        BenchTools, Workbench, WorkbenchContext, WorkbenchDescriptor,
    };

    struct Bench(&'static str, &'static str, bool);

    impl Workbench for Bench {
        fn descriptor(&self) -> WorkbenchDescriptor {
            WorkbenchDescriptor::new(self.0, self.1, "")
        }
        fn configure(&self, _context: &mut WorkbenchContext) {}
    }

    impl BenchFeatures for Bench {}

    impl BenchTools for Bench {}

    impl BenchPanels for Bench {
        fn has_settings(&self) -> bool {
            self.2
        }
    }

    impl BenchBuild for Bench {}

    impl BenchParameters for Bench {}

    impl BenchOutput for Bench {}

    impl BenchCommands for Bench {}

    #[test]
    fn every_workbench_with_settings_gets_a_page_in_registration_order() {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(Bench("z.second", "Second", true)))
            .unwrap();
        registry
            .register_workbench(Box::new(Bench("m.none", "Nothing to set", false)))
            .unwrap();
        registry
            .register_workbench(Box::new(Bench("a.first", "First", true)))
            .unwrap();
        let groups = PrefGroup::all(&registry);
        let benches: Vec<PrefGroup> = groups
            .iter()
            .copied()
            .filter(|g| matches!(g, PrefGroup::Workbench(_)))
            .collect();
        assert_eq!(
            benches,
            vec![PrefGroup::Workbench(0), PrefGroup::Workbench(2)]
        );
        assert_eq!(PrefGroup::Workbench(0).label(&registry), "Second");
        assert_eq!(PrefGroup::Workbench(2).label(&registry), "First");
        let keyboard = groups
            .iter()
            .position(|g| *g == PrefGroup::Keyboard)
            .unwrap();
        let packages = groups
            .iter()
            .position(|g| *g == PrefGroup::Packages)
            .unwrap();
        let units = groups.iter().position(|g| *g == PrefGroup::Units).unwrap();
        assert_eq!(
            packages - keyboard,
            3,
            "the bench pages sit between Keyboard and the packages page"
        );
        assert_eq!(units, packages + 1, "then Units");
    }

    /// Every word a search over all of Preferences draws for `filter`.
    fn search_words(filter: &str) -> Vec<String> {
        let mut registry = DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let mut state = PreferencesState::default();
        let release = package_store::ReleaseCheck::NotChecked;
        let store = package_store::StoreView::default();
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut inputs = PreferencesInputs {
                registry: &mut registry,
                gpus: &[],
                gpu_name: None,
                graphics_api: None,
                nav_buttons: 0,
                scripts: &[],
                packages: &[],
                release: &release,
                store: &store,
            };
            search_results(ui, &mut state, &mut inputs, filter);
        });
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        output.textures_delta.clear();
        let mut out = Vec::new();
        output.shapes.iter().for_each(|c| texts(&c.shape, &mut out));
        out
    }

    #[test]
    fn a_search_leaves_out_every_block_it_does_not_find() {
        assert_eq!(search_words("zqxj"), ["No setting matches."]);
        let words = search_words("existing");
        assert!(
            words.iter().any(|w| w == "Reuse existing lines"),
            "{words:?}"
        );
        assert!(!words.iter().any(|w| w.starts_with("Slice")), "{words:?}");
    }
}
