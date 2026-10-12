//! The Preferences dialog: a modal with a group rail, a tab strip per
//! group and rows edited on a draft that Apply or OK commit.

use axes::AxisPreset;
use core_document::{DocumentService, Unit};
use egui::{
    Align, Context, CornerRadius, Frame, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    Vec2, pos2, vec2,
};
use kernel_api::LinearDeflectionMode;
use settings::{OrbitYawAxis, ProjectionMode, SixDofMotion, SlicerFormat, UserSettings};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Card, PrefRow, QtyField, overline, pref_group, primary_button, secondary_button,
};
use ui_kit::{mono, sans, sans_medium, sans_semibold};

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
        // A page runs no agents and is always the latest release.
        if !crate::platform::ON_PAGE {
            groups.extend([PrefGroup::Ai, PrefGroup::Updates]);
        }
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
            PrefGroup::Input => &["Mouse", "6-DoF mouse"],
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
    #[cfg(not(target_arch = "wasm32"))]
    package_repo: String,
    /// What the store's list is narrowed to: the words typed, and the
    /// category picked (0 for every one).
    store_query: String,
    store_category: usize,
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
            #[cfg(not(target_arch = "wasm32"))]
            package_repo: String::new(),
            store_query: String::new(),
            store_category: 0,
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
    /// The connected 6-DoF mouse's name; `None` when there is none.
    #[cfg(target_arch = "wasm32")]
    pub nav_device: Option<&'a str>,
    /// How many buttons the connected 6-DoF mouse has, so the page offers a
    /// row per button it actually owns. Zero when none is connected.
    pub nav_buttons: u32,
    /// The scripts folder's scripts, which take keys like any command.
    pub scripts: &'a [crate::script_library::ScriptEntry],
    /// The workbench packages found at start and since.
    pub packages: &'a [workbenches::PackageStatus],
    /// What the last look for a newer printCAD found.
    pub release: &'a crate::app::updates::ReleaseCheck,
    /// The workbench store's list.
    pub store: &'a crate::app::packages::StoreView,
}

/// The six ways the puck moves, in the order the device reports them: what
/// the hand does, the drawing that shows it, and the id its chooser
/// needs.
struct Gesture {
    name: &'static str,
    drawing: &'static str,
    assign_id: &'static str,
}

const SIXDOF_GESTURES: [Gesture; 6] = [
    Gesture {
        name: "Push left and right",
        drawing: "motion-push-left-right",
        assign_id: "prefs_sixdof_does_1",
    },
    Gesture {
        name: "Pull up and push down",
        drawing: "motion-pull-up-push-down",
        assign_id: "prefs_sixdof_does_2",
    },
    Gesture {
        name: "Drag front and back",
        drawing: "motion-drag-front-back",
        assign_id: "prefs_sixdof_does_3",
    },
    Gesture {
        name: "Tilt forward and back",
        drawing: "motion-tilt-forward-back",
        assign_id: "prefs_sixdof_does_4",
    },
    Gesture {
        name: "Twist",
        drawing: "motion-twist",
        assign_id: "prefs_sixdof_does_5",
    },
    Gesture {
        name: "Tilt left and right",
        drawing: "motion-tilt-left-right",
        assign_id: "prefs_sixdof_does_6",
    },
];

/// How big the drawing of a movement is: large enough to read the gesture
/// off it, with the movement's own settings beside it.
const GESTURE_DRAWING: f32 = 200.0;

/// One widget id per 6-DoF mouse button row; a chooser needs its own.
const NAV_BUTTON_IDS: [&str; 16] = [
    "prefs_nav_button_1",
    "prefs_nav_button_2",
    "prefs_nav_button_3",
    "prefs_nav_button_4",
    "prefs_nav_button_5",
    "prefs_nav_button_6",
    "prefs_nav_button_7",
    "prefs_nav_button_8",
    "prefs_nav_button_9",
    "prefs_nav_button_10",
    "prefs_nav_button_11",
    "prefs_nav_button_12",
    "prefs_nav_button_13",
    "prefs_nav_button_14",
    "prefs_nav_button_15",
    "prefs_nav_button_16",
];

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
            state.draft.sixdof = defaults.sixdof;
        }
        PrefGroup::Keyboard => state.draft.keyboard = defaults.keyboard,
        PrefGroup::Units => state.draft_unit = Unit::Mm,
        PrefGroup::ImportExport => state.draft.import = defaults.import,
        PrefGroup::Printing => state.draft.printing = defaults.printing,
        PrefGroup::Ai => {
            state.draft.ai.ask_before_changes = defaults.ai.ask_before_changes;
            state.draft.ai.rules = defaults.ai.rules;
        }
        PrefGroup::Packages => state.draft.packages = defaults.packages,
        PrefGroup::Updates => state.draft.updates = defaults.updates,
        PrefGroup::Workbench(_) => {}
    }
}

/// Every shortcut, grouped by menu and then by workbench: its keys, and
/// buttons to record a new key, add one, clear them or go back to the
/// default. A key another command also answers to is called out under the
/// row.
fn keyboard_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    registry: &DocumentService,
    scripts: &[crate::script_library::ScriptEntry],
    filter: &str,
) {
    use super::keymap::{Keymap, set_keys};
    let keymap = Keymap::build(registry, &state.draft.keyboard, scripts);
    let key_text = |keys: &[core_document::Chord]| {
        keys.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    if filter.is_empty() {
        ui.label(
            RichText::new(
                "A workbench's keys work while it is active, and win over the \
                 application's there. Keys without Ctrl or Alt are left to text fields.",
            )
            .font(sans(FONT_SM))
            .color(TEXT3),
        );
        ui.add_space(SPACE_2);
    }
    let mut groups: Vec<&str> = Vec::new();
    for binding in keymap.bindings() {
        if !groups.contains(&binding.group.as_str()) {
            groups.push(&binding.group);
        }
    }
    let mut change: Option<(super::keymap::Binding, Vec<core_document::Chord>)> = None;
    for group in groups {
        let rows: Vec<_> = keymap
            .bindings()
            .iter()
            .filter(|b| b.group == group)
            .filter(|b| {
                filter.is_empty()
                    || b.label.to_lowercase().contains(filter)
                    || group.to_lowercase().contains(filter)
                    || key_text(&b.keys).to_lowercase().contains(filter)
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        overline(ui, group);
        ui.add_space(SPACE_1);
        // A card of rows split by lines, as every Preferences group draws,
        // so each action reads across to its keys and buttons.
        let count = rows.len();
        egui::Frame::new()
            .fill(BG1)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(RADIUS_MD)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, binding) in rows.into_iter().enumerate() {
                    let recording = state
                        .recording
                        .as_ref()
                        .is_some_and(|(id, _)| *id == binding.id);
                    key_row(ui, |ui| {
                        ui.add_space(14.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(240.0, INPUT),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_width(240.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&binding.label)
                                            .font(sans(FONT_SM))
                                            .color(TEXT1),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        if recording {
                            ui.label(
                                RichText::new("Press a key (Escape cancels)")
                                    .font(sans(FONT_SM))
                                    .color(ACCENT),
                            );
                        } else if binding.keys.is_empty() {
                            ui.label(RichText::new("No key").font(sans(FONT_SM)).color(TEXT3));
                        } else {
                            for key in &binding.keys {
                                ui_kit::widgets::key_chip(ui, &key.to_string());
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(14.0);
                            if binding.is_changed()
                                && ui_kit::widgets::small_secondary_button(ui, "Reset")
                                    .on_hover_text(if binding.defaults.is_empty() {
                                        "Back to no key".to_string()
                                    } else {
                                        format!("Back to {}", key_text(&binding.defaults))
                                    })
                                    .clicked()
                            {
                                change = Some((binding.clone(), binding.defaults.clone()));
                            }
                            if !binding.keys.is_empty()
                                && ui_kit::widgets::small_secondary_button(ui, "Clear")
                                    .on_hover_text("Leave it without a key")
                                    .clicked()
                            {
                                change = Some((binding.clone(), Vec::new()));
                            }
                            if !binding.keys.is_empty()
                                && ui_kit::widgets::small_secondary_button(ui, "Add")
                                    .on_hover_text("Give it another key as well")
                                    .clicked()
                            {
                                state.recording = Some((binding.id.clone(), true));
                            }
                            if ui_kit::widgets::small_secondary_button(ui, "Set")
                                .on_hover_text("Press the new key next")
                                .clicked()
                            {
                                state.recording = Some((binding.id.clone(), false));
                            }
                        });
                    });
                    for clash in keymap.clashes(&binding.id) {
                        let (text, color) = if !clash.shadowed {
                            (
                                format!(
                                    "{} also runs {} ({})",
                                    clash.key, clash.other, clash.other_group
                                ),
                                WARNING,
                            )
                        } else if binding.scope.is_some() {
                            (
                                format!("{} runs this instead of {} here", clash.key, clash.other),
                                TEXT3,
                            )
                        } else {
                            (
                                format!(
                                    "In {}, {} runs {} instead",
                                    clash.other_group, clash.key, clash.other
                                ),
                                TEXT3,
                            )
                        };
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.label(RichText::new(text).font(sans(FONT_XS)).color(color));
                        });
                        ui.add_space(SPACE_2);
                    }
                    if i + 1 < count {
                        let bottom = ui.cursor().top();
                        let x = ui.max_rect().x_range();
                        ui.painter()
                            .hline(x, bottom, egui::Stroke::new(1.0, BORDER));
                    }
                }
            });
        ui.add_space(SPACE_3);
    }
    if let Some((binding, keys)) = change {
        set_keys(&mut state.draft.keyboard, &binding, keys);
    }
}

/// A keyboard row, 40 high like every Preferences row, its contents in an
/// input-high strip across its middle so chips and buttons keep their size.
fn key_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::hover());
    let strip = egui::Rect::from_x_y_ranges(
        rect.x_range(),
        (rect.center().y - INPUT / 2.0)..=(rect.center().y + INPUT / 2.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(strip)
            .layout(Layout::left_to_right(Align::Center)),
    );
    add(&mut child);
}

fn general_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &PreferencesInputs<'_>,
    filter: &str,
) {
    match state.tab {
        0 => {
            let draft = &mut state.draft;
            pref_group(
                ui,
                "Interface",
                vec![
                    PrefRow::toggle("Log panel", &mut draft.rendering.show_log_panel)
                        .hint("Show the in-app log under the viewport"),
                    PrefRow::qty(
                        "Frame rate cap",
                        QtyField::new(&mut draft.fps_cap)
                            .unit("fps")
                            .range(0.0..=480.0)
                            .speed(1.0)
                            .decimals(0),
                    )
                    .hint("0 leaves the rate to the display"),
                ],
                filter,
            );
            let autosave = &mut draft.autosave_minutes;
            pref_group(
                ui,
                "Autosave",
                vec![
                    PrefRow::new("Every", move |ui| {
                        ui.add(egui::DragValue::new(autosave).range(0..=120).suffix(" min"))
                            .changed()
                    })
                    .hint(
                        "A copy of each edited document, which the start page offers back after \
                         a crash; 0 turns it off",
                    ),
                ],
                filter,
            );
            // A page keeps no session between visits.
            if !crate::platform::ON_PAGE {
                pref_group(
                    ui,
                    "Session",
                    vec![
                        PrefRow::toggle(
                            "Reopen documents at start",
                            &mut draft.session.reopen_documents,
                        )
                        .hint(
                            "Open again the documents that were open when printCAD closed; off \
                             starts on the start page",
                        ),
                    ],
                    filter,
                );
            }
            pref_group(
                ui,
                "Files",
                vec![
                    PrefRow::toggle(
                        "Ask before saving over a file from an older version",
                        &mut draft.files.ask_before_saving_older,
                    )
                    .hint(
                        "Once saved by this version, the older one may no longer open it; a file \
                         from a newer version always asks",
                    ),
                ],
                filter,
            );
            // The report is written to the temp dir, which a page has not.
            if !crate::platform::ON_PAGE {
                pref_group(
                    ui,
                    "Diagnostics",
                    vec![
                        PrefRow::toggle(
                            "Write a report for every STEP or IGES import",
                            &mut draft.diagnostics.import_report,
                        )
                        .hint(
                            "Everything the reader had to say about the file, written to the temp \
                         dir for sending to the kernel or printCAD developers",
                        ),
                    ],
                    filter,
                );
            }
        }
        _ => {
            pref_group(
                ui,
                "About",
                vec![
                    PrefRow::text(
                        "Version",
                        format!("printCAD {} · dev", env!("CARGO_PKG_VERSION")),
                    ),
                    PrefRow::text("GPU", inputs.gpu_name.unwrap_or("Unknown").to_string()),
                    PrefRow::text(
                        "Graphics API",
                        inputs.graphics_api.unwrap_or("Unknown").to_string(),
                    ),
                    PrefRow::text("Geometry kernel", "ogeom (pure Rust)".to_string()),
                ],
                filter,
            );
        }
    }
}

fn display_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &PreferencesInputs<'_>,
    filter: &str,
) {
    let draft = &mut state.draft;
    match state.tab {
        0 => {
            let camera = &mut draft.camera;
            let preset_hint = camera.axis_preset.description();
            pref_group(
                ui,
                "Camera",
                vec![
                    PrefRow::select(
                        "Projection",
                        "prefs_projection",
                        &mut camera.projection,
                        &[
                            (ProjectionMode::Perspective, "Perspective"),
                            (ProjectionMode::Orthographic, "Orthographic"),
                        ],
                    ),
                    PrefRow::qty(
                        "Field of view",
                        QtyField::degrees(&mut camera.fov_degrees).range(10.0..=120.0),
                    )
                    .hint("Vertical, for perspective"),
                    PrefRow::qty(
                        "Orthographic height",
                        QtyField::mm(&mut camera.ortho_height_mm).range(1.0..=500_000.0),
                    ),
                    PrefRow::qty(
                        "Minimum focal distance",
                        QtyField::mm(&mut camera.min_focal_distance).range(0.1..=50.0),
                    ),
                    PrefRow::qty(
                        "Maximum focal distance",
                        QtyField::mm(&mut camera.max_focal_distance).range(50.0..=500_000.0),
                    ),
                    PrefRow::toggle("Auto near / far planes", &mut camera.auto_near_far)
                        .hint("Clip planes follow the scene bounds"),
                    PrefRow::qty(
                        "Near distance ratio",
                        QtyField::new(&mut camera.near_far_near_ratio)
                            .range(0.00001..=0.1)
                            .speed(0.0001)
                            .decimals(5),
                    )
                    .hint("Times the focal distance"),
                    PrefRow::qty(
                        "Far / near ratio cap",
                        QtyField::new(&mut camera.near_far_depth_ratio_cap)
                            .range(1000.0..=500_000.0)
                            .speed(100.0)
                            .decimals(0),
                    ),
                    PrefRow::qty(
                        "Far plane margin",
                        QtyField::mm(&mut camera.near_far_margin).range(1.0..=10_000.0),
                    ),
                    PrefRow::qty(
                        "View transition",
                        QtyField::new(&mut camera.view_transition_ms)
                            .unit("ms")
                            .range(120.0..=1200.0)
                            .speed(10.0)
                            .decimals(0),
                    ),
                    PrefRow::select(
                        "Axis preset",
                        "prefs_axis_preset",
                        &mut camera.axis_preset,
                        &[
                            (AxisPreset::ALL[0], AxisPreset::ALL[0].label()),
                            (AxisPreset::ALL[1], AxisPreset::ALL[1].label()),
                            (AxisPreset::ALL[2], AxisPreset::ALL[2].label()),
                        ],
                    )
                    .hint(preset_hint),
                ],
                filter,
            );
        }
        1 => {
            let lighting = &mut draft.lighting;
            let mut rows = Vec::new();
            for (label, light) in [
                ("Main light", &mut lighting.main_light),
                ("Backlight", &mut lighting.backlight),
                ("Fill light", &mut lighting.fill_light),
            ] {
                rows.push(
                    PrefRow::new(label, move |ui| {
                        let mut changed = false;
                        changed |= QtyField::new(&mut light.intensity)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .width(70.0)
                            .show(ui);
                        let mut color = egui::Color32::from_rgb(
                            (light.color[0] * 255.0) as u8,
                            (light.color[1] * 255.0) as u8,
                            (light.color[2] * 255.0) as u8,
                        );
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            light.color = [
                                color.r() as f32 / 255.0,
                                color.g() as f32 / 255.0,
                                color.b() as f32 / 255.0,
                            ];
                            changed = true;
                        }
                        changed |= QtyField::degrees(&mut light.vertical_angle)
                            .range(-90.0..=90.0)
                            .decimals(0)
                            .width(70.0)
                            .show(ui);
                        changed |= QtyField::degrees(&mut light.horizontal_angle)
                            .range(-180.0..=180.0)
                            .decimals(0)
                            .width(70.0)
                            .show(ui);
                        changed |= ui_kit::widgets::toggle(ui, &mut light.enabled).changed();
                        changed
                    })
                    .hint("On · horizontal · vertical · color · intensity"),
                );
            }
            pref_group(ui, "Light sources", rows, filter);
            pref_group(
                ui,
                "Ambient and specular",
                vec![
                    PrefRow::color("Ambient color", &mut lighting.ambient_color),
                    PrefRow::qty(
                        "Ambient intensity",
                        QtyField::new(&mut lighting.ambient_intensity)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    ),
                    PrefRow::qty(
                        "Specular shininess",
                        QtyField::new(&mut lighting.specular_shininess)
                            .range(8.0..=128.0)
                            .speed(1.0)
                            .decimals(0),
                    )
                    .hint("Larger is a tighter highlight"),
                    PrefRow::qty(
                        "Specular intensity",
                        QtyField::new(&mut lighting.specular_intensity)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    ),
                ],
                filter,
            );
            pref_group(
                ui,
                "Edge lines",
                vec![
                    PrefRow::color("Edge color", &mut lighting.edge_line_color),
                    PrefRow::qty(
                        "Edge width",
                        QtyField::new(&mut lighting.edge_line_width)
                            .unit("px")
                            .range(0.5..=8.0)
                            .speed(0.1)
                            .decimals(1),
                    ),
                ],
                filter,
            );
        }
        _ => {
            let gpus = inputs.gpus;
            let preferred_gpu = &mut draft.preferred_gpu;
            let msaa = &mut draft.rendering.msaa_samples;
            let mut gpu_rows = vec![
                PrefRow::new("Preferred GPU", move |ui| {
                    let current = preferred_gpu
                        .clone()
                        .unwrap_or_else(|| "Automatic".to_string());
                    let mut selected = current.clone();
                    let options: Vec<(String, String)> = std::iter::once("Automatic".to_string())
                        .chain(gpus.iter().cloned())
                        .map(|g| (g.clone(), g))
                        .collect();
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("prefs_gpu")
                        .width(220.0)
                        .selected_text(RichText::new(&current).font(sans(FONT_SM)))
                        .show_ui(ui, |ui| {
                            for (value, label) in &options {
                                if ui
                                    .selectable_value(&mut selected, value.clone(), label)
                                    .clicked()
                                {
                                    changed = true;
                                }
                            }
                        });
                    if changed {
                        *preferred_gpu = (selected != "Automatic").then_some(selected);
                    }
                    changed
                })
                .hint("Takes effect after a restart"),
            ];
            gpu_rows.push(
                PrefRow::select(
                    "Anti-aliasing",
                    "prefs_msaa",
                    msaa,
                    &[(1, "Off"), (2, "2× MSAA"), (4, "4× MSAA"), (8, "8× MSAA")],
                )
                .hint("Takes effect after a restart"),
            );
            gpu_rows.push(
                PrefRow::qty(
                    "Curve smoothness",
                    QtyField::new(&mut draft.rendering.curve_step_deg)
                        .unit("°")
                        .range(2.0..=45.0)
                        .speed(0.5)
                        .decimals(1),
                )
                .hint("The turn between two drawn facets of a curved face; smaller is rounder"),
            );
            pref_group(ui, "Rendering", gpu_rows, filter);

            let selection_color = &mut draft.rendering.selection_color;
            pref_group(
                ui,
                "Selection",
                vec![
                    PrefRow::new("Face colour", move |ui| {
                        let mut color = egui::Color32::from_rgb(
                            (selection_color[0] * 255.0) as u8,
                            (selection_color[1] * 255.0) as u8,
                            (selection_color[2] * 255.0) as u8,
                        );
                        let changed = ui.color_edit_button_srgba(&mut color).changed();
                        if changed {
                            *selection_color = [
                                color.r() as f32 / 255.0,
                                color.g() as f32 / 255.0,
                                color.b() as f32 / 255.0,
                            ];
                        }
                        changed
                    })
                    .hint("What a selected face is painted"),
                    PrefRow::qty(
                        "Face opacity",
                        QtyField::new(&mut draft.rendering.selection_opacity)
                            .range(0.1..=f64::from(settings::MAX_SELECTION_OPACITY))
                            .speed(0.01)
                            .decimals(2),
                    )
                    .hint("How much paint goes over a selected face or body"),
                ],
                filter,
            );

            let preview_color = &mut draft.rendering.preview_color;
            pref_group(
                ui,
                "Feature preview",
                vec![
                    PrefRow::new("Colour", move |ui| {
                        let mut color = egui::Color32::from_rgb(
                            (preview_color[0] * 255.0) as u8,
                            (preview_color[1] * 255.0) as u8,
                            (preview_color[2] * 255.0) as u8,
                        );
                        let changed = ui.color_edit_button_srgba(&mut color).changed();
                        if changed {
                            *preview_color = [
                                color.r() as f32 / 255.0,
                                color.g() as f32 / 255.0,
                                color.b() as f32 / 255.0,
                            ];
                        }
                        changed
                    })
                    .hint("What a feature being edited adds or takes is drawn in"),
                    PrefRow::qty(
                        "Opacity",
                        QtyField::new(&mut draft.rendering.preview_opacity)
                            .range(0.05..=f64::from(settings::MAX_SELECTION_OPACITY))
                            .speed(0.01)
                            .decimals(2),
                    )
                    .hint("How solid its faces look; its edges are drawn whole"),
                ],
                filter,
            );
        }
    }
}

/// The page of the `index`-th registered workbench: whatever it draws.
fn workbench_page(ui: &mut Ui, registry: &mut DocumentService, index: usize, filter: &str) {
    let Some(id) = registry.ids().get(index).cloned() else {
        return;
    };
    if let Ok(wb) = registry.workbench_mut(&id) {
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

/// The printer's build volume (what the print-bed overlay draws and the
/// print layout fills), the slicer and the layout's gap.
fn printing_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    let printing = &mut state.draft.printing;
    let [x, y, z] = &mut printing.bed_mm;
    pref_group(
        ui,
        "Build volume",
        vec![
            PrefRow::qty(
                "Bed width (X)",
                QtyField::mm(x).range(10.0..=2000.0).speed(1.0).decimals(0),
            ),
            PrefRow::qty(
                "Bed depth (Y)",
                QtyField::mm(y).range(10.0..=2000.0).speed(1.0).decimals(0),
            ),
            PrefRow::qty(
                "Build height (Z)",
                QtyField::mm(z).range(10.0..=2000.0).speed(1.0).decimals(0),
            ),
            PrefRow::toggle("Origin at the bed centre", &mut printing.origin_center)
                .hint("Off, the origin is the bed's front-left corner"),
            PrefRow::toggle("Show the print bed", &mut printing.show_bed)
                .hint("Draw the build volume around the model; the toolbar toggles it too"),
        ],
        filter,
    );
    let command = &mut printing.slicer_command;
    // A page cannot start the slicer.
    if !crate::platform::ON_PAGE {
        pref_group(
            ui,
            "Slicer",
            vec![
                PrefRow::new("Slicer command", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(command)
                            .hint_text("the system's app for the file")
                            .desired_width(260.0)
                            .font(mono(FONT_SM)),
                    )
                    .changed()
                })
                .hint("Send to slicer (Ctrl+P) runs this with the model's file; {file} places it"),
                PrefRow::select(
                    "Format",
                    "prefs_slicer_format",
                    &mut printing.slicer_format,
                    &[
                        (SlicerFormat::ThreeMf, "3MF: one named object per body"),
                        (SlicerFormat::Stl, "STL: triangles only"),
                    ],
                ),
                PrefRow::toggle("Send the print layout", &mut printing.slicer_layout).hint(
                    "Each part flat on the bed, as many as the parts list prints, rather than the \
                 bodies where they sit",
                ),
            ],
            filter,
        );
    }
    pref_group(
        ui,
        "Print layout",
        vec![
            PrefRow::qty(
                "Gap between parts",
                QtyField::mm(&mut printing.layout_gap_mm)
                    .range(0.0..=50.0)
                    .speed(0.5),
            )
            .hint("Half of it is kept from the bed's edges"),
        ],
        filter,
    );
}

/// Agents that speak the Agent Client Protocol, known to work with it.
const AGENT_PRESETS: &[(&str, &str, &[&str])] = &[
    ("Claude", "claude-agent-acp", &[]),
    ("Gemini CLI", "gemini", &["--experimental-acp"]),
];

/// The agents chats can talk to, and whether their changes wait for an OK.
fn ai_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    let ai = &mut state.draft.ai;
    pref_group(
        ui,
        "Changes",
        vec![
            PrefRow::toggle(
                "Ask before an agent changes the document",
                &mut ai.ask_before_changes,
            )
            .hint("New chats start this way; each chat has its own switch"),
        ],
        filter,
    );
    if !filter.is_empty() && !"agents ai command chat rules".contains(filter) {
        return;
    }
    ui.add_space(SPACE_2);
    ui.label(
        RichText::new("Rules for every document")
            .font(sans_semibold(FONT_SM))
            .color(TEXT1),
    );
    ui.label(
        RichText::new(
            "What every agent keeps to, in every document: units, wall thicknesses, how to \
             name things, what never to change. A document can add its own in the Assistant \
             panel. Chats already open take a change with their next message.",
        )
        .font(sans(FONT_XS))
        .color(TEXT3),
    );
    ui.add_space(SPACE_1);
    ui.add(
        egui::TextEdit::multiline(&mut ai.rules)
            .hint_text(
                "Keep every wall at least 1.2 mm thick.\nName features after what they are for.",
            )
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .font(mono(FONT_SM)),
    );
    ui.add_space(SPACE_2);
    ui.label(
        RichText::new("Agents")
            .font(sans_semibold(FONT_SM))
            .color(TEXT1),
    );
    ui.label(
        RichText::new(
            "Programs that speak the Agent Client Protocol. A chat starts one and gives it \
             printCAD's tools, which reach the document through `printcad --mcp`; any MCP \
             client can use that command too.",
        )
        .font(sans(FONT_XS))
        .color(TEXT3),
    );
    ui.add_space(SPACE_1);
    let mut remove = None;
    for (i, agent) in ai.agents.iter_mut().enumerate() {
        Card::new().padding(10.0).show(ui, |ui| {
            egui::Grid::new(("agent_row", i))
                .num_columns(2)
                .spacing([SPACE_3, SPACE_2])
                .show(ui, |ui| {
                    let field = |ui: &mut Ui, label: &str, text: &mut String, hint: &str| {
                        ui.label(RichText::new(label).font(sans(FONT_SM)).color(TEXT2));
                        let changed = ui
                            .add(
                                egui::TextEdit::singleline(text)
                                    .hint_text(hint)
                                    .desired_width(300.0)
                                    .font(mono(FONT_SM)),
                            )
                            .changed();
                        ui.end_row();
                        changed
                    };
                    field(ui, "Name", &mut agent.name, "What the chat shows");
                    field(ui, "Command", &mut agent.command, "The program to start");
                    let mut args = join_words(&agent.args);
                    if field(ui, "Arguments", &mut args, "Words after the command") {
                        agent.args = split_words(&args);
                    }
                    let mut env = join_words(
                        &agent
                            .env
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>(),
                    );
                    if field(ui, "Environment", &mut env, "NAME=value ...") {
                        agent.env = split_words(&env)
                            .into_iter()
                            .filter_map(|pair| {
                                let (k, v) = pair.split_once('=')?;
                                Some((k.to_string(), v.to_string()))
                            })
                            .collect();
                    }
                });
            ui.add_space(SPACE_1);
            if ui_kit::widgets::small_secondary_button(ui, "Remove").clicked() {
                remove = Some(i);
            }
        });
        ui.add_space(SPACE_2);
    }
    if let Some(i) = remove {
        ai.agents.remove(i);
    }
    ui.horizontal_wrapped(|ui| {
        for (name, command, args) in AGENT_PRESETS {
            if ui_kit::widgets::small_secondary_button(ui, &format!("Add {name}")).clicked() {
                ai.agents.push(settings::AgentSettings {
                    name: name.to_string(),
                    command: command.to_string(),
                    args: args.iter().map(|a| a.to_string()).collect(),
                    ..Default::default()
                });
            }
        }
        if ui_kit::widgets::small_secondary_button(ui, "Add another").clicked() {
            ai.agents.push(settings::AgentSettings {
                name: "Agent".to_string(),
                ..Default::default()
            });
        }
    });
}

/// Words joined with spaces, a word with a space in it quoted.
fn join_words(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if w.contains(char::is_whitespace) || w.is_empty() {
                format!("\"{}\"", w.replace('"', "\\\""))
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words split at spaces, a quoted run kept as one word.
fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut escaped, mut any) = (false, false, false);
    for c in text.chars() {
        match c {
            _ if escaped => {
                word.push(c);
                escaped = false;
            }
            '\\' if quoted => escaped = true,
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                any = false;
            }
            c => word.push(c),
        }
    }
    if any || !word.is_empty() {
        words.push(word);
    }
    words
}

/// Installed workbench packages: whether each loads, what it may reach,
/// removing it, and installing another. Installs, updates and removals
/// take effect at once; turning one on or off and what it may reach, when
/// the draft is applied.
fn packages_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    packages: &[workbenches::PackageStatus],
    store: &crate::app::packages::StoreView,
    filter: &str,
) {
    use workbenches::PackageState;
    if filter.is_empty() {
        match state.tab {
            1 => return browse_page(ui, state, packages, store),
            2 => return stores_page(ui, state, store),
            _ => {}
        }
    }
    if !filter.is_empty()
        && !"workbench packages plugins install remove extensions".contains(filter)
        && !packages
            .iter()
            .any(|p| p.name.to_lowercase().contains(filter) || p.id.contains(filter))
    {
        return;
    }
    ui.label(
        RichText::new(
            "Workbenches others made, each run apart from the app and reaching only its own \
             folder and what you allow here. Installs, updates and removals take effect at once; \
             turning a package on or off and what it may reach, when you apply.",
        )
        .font(sans(FONT_XS))
        .color(TEXT3),
    );
    ui.add_space(SPACE_1);
    if packages.is_empty() {
        ui.label(
            RichText::new("No packages are installed.")
                .font(sans(FONT_SM))
                .color(TEXT2),
        );
    }
    let draft = &mut state.draft.packages;
    for package in packages {
        Card::new().padding(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&package.name)
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                ui.label(
                    RichText::new(&package.version)
                        .font(mono(FONT_XS))
                        .color(TEXT3),
                );
                let (text, color) = match &package.state {
                    PackageState::Loaded => ("Loaded", SUCCESS),
                    PackageState::Disabled => ("Turned off", TEXT3),
                    PackageState::Failed(_) => ("Did not load", DANGER),
                };
                ui_kit::widgets::badge(ui, text, color);
                if store.lists(package).is_some() {
                    ui_kit::widgets::badge(ui, "Listed", ACCENT)
                        .on_hover_text("In the workbench store's list");
                }
            });
            if let Some(why) = store.lists(package).and_then(|l| l.removed.as_deref()) {
                ui.label(
                    RichText::new(format!("Taken off the store's list: {why}"))
                        .font(sans(FONT_XS))
                        .color(WARNING),
                );
            }
            if !package.description.is_empty() {
                ui.label(
                    RichText::new(&package.description)
                        .font(sans(FONT_XS))
                        .color(TEXT2),
                );
            }
            if let PackageState::Failed(reason) = &package.state {
                ui.label(RichText::new(reason).font(sans(FONT_XS)).color(DANGER));
            }
            ui.label(
                RichText::new(format!("{} · {}", package.id, package.dir.display()))
                    .font(mono(FONT_XS))
                    .color(TEXT3),
            );
            if let Some(source) = &package.source {
                ui.label(
                    RichText::new(format!("github.com/{} · {}", source.repo, source.tag))
                        .font(mono(FONT_XS))
                        .color(TEXT3),
                );
            }
            if let Some(tag) = &package.update
                && primary_button(ui, &format!("Update to {tag}")).clicked()
            {
                state.package_request = Some(super::UiCommand::UpdatePackage(package.id.clone()));
            }
            let mut enabled = draft.enabled(&package.id);
            if ui_kit::widgets::check_row(ui, &mut enabled, "Turned on").changed() {
                draft.disabled.retain(|d| d != &package.id);
                if !enabled {
                    draft.disabled.push(package.id.clone());
                }
            }
            let mut grant = draft.grant(&package.id);
            let mut changed = false;
            if package.requested.save_dialog {
                changed |= ui_kit::widgets::check_row(
                    ui,
                    &mut grant.save_dialog,
                    "May ask where to save a file",
                )
                .changed();
            }
            if package.requested.helper {
                changed |= ui_kit::widgets::check_row(
                    ui,
                    &mut grant.helper,
                    "May run the programs it ships (they run outside its sandbox)",
                )
                .changed();
            }
            if package.requested.network {
                changed |=
                    ui_kit::widgets::check_row(ui, &mut grant.network, "May use the network")
                        .changed();
            }
            if changed {
                draft.grants.insert(package.id.clone(), grant);
            }
            if ui_kit::widgets::small_secondary_button(ui, "Remove").clicked() {
                state.package_request = Some(super::UiCommand::RemovePackage(package.id.clone()));
            }
        });
        ui.add_space(SPACE_1);
    }
    ui.add_space(SPACE_2);
    // GitHub's downloads refuse a page's requests: a page installs from a
    // file or a store.
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.label(
            RichText::new("Install from GitHub")
                .font(sans_semibold(FONT_SM))
                .color(TEXT1),
        );
        ui.label(
            RichText::new(
                "A repository whose releases carry a .pcbench file: its address takes the latest \
                 release, a release's address that one.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.package_repo)
                    .hint_text("https://github.com/owner/repo")
                    .font(mono(FONT_SM))
                    .desired_width(320.0),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let typed = !state.package_repo.trim().is_empty();
            let click = ui
                .add_enabled_ui(typed, |ui| secondary_button(ui, "Install"))
                .inner
                .clicked();
            if typed && (click || enter) {
                state.package_request = Some(super::UiCommand::InstallPackageFromGithub(
                    std::mem::take(&mut state.package_repo),
                ));
            }
        });
    }
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        if primary_button(ui, "Install from a file…").clicked() {
            state.package_request = Some(super::UiCommand::InstallPackage);
        }
        if !cfg!(target_arch = "wasm32")
            && packages.iter().any(|p| p.source.is_some())
            && secondary_button(ui, "Check for updates").clicked()
        {
            state.package_request = Some(super::UiCommand::CheckPackageUpdates);
        }
    });
    ui_kit::widgets::check_row(
        ui,
        &mut state.draft.packages.check_updates,
        "Check for updates of packages from GitHub when printCAD starts",
    );
}

/// The workbench stores: every package their lists hold, to read about
/// and install, under a word of care about software from other people.
fn browse_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    packages: &[workbenches::PackageStatus],
    store: &crate::app::packages::StoreView,
) {
    use crate::app::packages::store_name;
    // The page on screen with a store not fetched yet fetches by itself.
    if store.unlooked() {
        state.package_request = Some(super::UiCommand::LookAtStores);
    }
    ui_kit::widgets::note_card(
        ui,
        ui_kit::widgets::Note::Warning,
        Some("Software from other people"),
        "These workbenches are made by people outside printCAD. A store checks that each \
         is what it says it is; it does not check what its code does. Install the ones you \
         trust, from stores you trust, and allow each only what it needs: the network lets it \
         reach the internet, and running its own programs takes it outside its sandbox.",
    );
    ui.add_space(SPACE_1);

    let listings: Vec<(&str, &workbenches::Listing)> = store
        .listings()
        // A package taken off a list shows only to whoever has it.
        .filter(|(_, l)| l.removed.is_none() || packages.iter().any(|p| p.id == l.id))
        .collect();
    let mut categories: Vec<&str> = listings
        .iter()
        .flat_map(|(_, l)| l.categories.iter().map(String::as_str))
        .collect();
    categories.sort_unstable();
    categories.dedup();
    let category_names: Vec<String> = std::iter::once("Every category".to_string())
        .chain(categories.iter().map(|c| c.replace('-', " ")))
        .collect();
    let category_options: Vec<(usize, &str)> = category_names
        .iter()
        .enumerate()
        .map(|(i, name)| (i, name.as_str()))
        .collect();
    let names: Vec<String> = std::iter::once("Every store".to_string())
        .chain(
            store
                .stores
                .iter()
                .map(|(url, s)| store_name(url, s.index.as_ref())),
        )
        .collect();
    let store_options: Vec<(usize, &str)> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (i, name.as_str()))
        .collect();
    if state.store_category >= category_options.len() {
        state.store_category = 0;
    }
    if state.store_pick >= store_options.len() {
        state.store_pick = 0;
    }
    let several = store.stores.len() > 1;
    let looking = store.stores.iter().any(|(_, s)| s.looking);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.store_query)
                .hint_text("Search the stores")
                .font(sans(FONT_SM))
                .desired_width(220.0),
        );
        ui_kit::widgets::select_field(
            ui,
            "store_category",
            &mut state.store_category,
            &category_options,
            150.0,
        );
        if several {
            ui_kit::widgets::select_field(
                ui,
                "store_pick",
                &mut state.store_pick,
                &store_options,
                150.0,
            );
        }
        let refresh = ui
            .add_enabled_ui(!looking && !store.stores.is_empty(), |ui| {
                ui_kit::widgets::small_secondary_button(ui, "Refresh")
            })
            .inner;
        if refresh.clicked() {
            state.package_request = Some(super::UiCommand::LookAtStores);
        }
    });
    let status = if store.stores.is_empty() {
        "No stores: add one on the Stores tab".to_string()
    } else if looking {
        "Reading the stores' lists…".to_string()
    } else {
        format!(
            "{} package{} listed by {} store{}",
            listings.len(),
            if listings.len() == 1 { "" } else { "s" },
            store.stores.len(),
            if several { "s" } else { "" }
        )
    };
    ui.label(RichText::new(status).font(sans(FONT_XS)).color(TEXT3));
    ui.add_space(SPACE_1);

    let query = state.store_query.trim().to_lowercase();
    let category = (state.store_category > 0).then(|| categories[state.store_category - 1]);
    let picked = (state.store_pick > 0).then(|| store.stores[state.store_pick - 1].0.as_str());
    let shown = listings.iter().filter(|(url, l)| {
        let words = format!("{} {} {}", l.name, l.id, l.description).to_lowercase();
        (query.is_empty() || words.contains(&query))
            && category.is_none_or(|c| l.categories.iter().any(|lc| lc == c))
            && picked.is_none_or(|p| p == *url)
    });
    for (url, listing) in shown {
        let from = several.then(|| {
            let index = store
                .stores
                .iter()
                .find(|(u, _)| u == url)
                .and_then(|(_, s)| s.index.as_ref());
            store_name(url, index)
        });
        listing_card(ui, state, packages, url, from.as_deref(), listing);
        ui.add_space(SPACE_1);
    }
}

/// The Stores tab: the stores Browse reads, as the draft keeps them, each
/// with what its last read found and a Remove; an address to add, and
/// printCAD's own to put back when it was removed.
fn stores_page(ui: &mut Ui, state: &mut PreferencesState, store: &crate::app::packages::StoreView) {
    use crate::app::packages::store_name;
    // What each store holds shows once it is read.
    if store.unlooked() {
        state.package_request = Some(super::UiCommand::LookAtStores);
    }
    ui.label(
        RichText::new(
            "Each is the address of a registry's index; Browse lists the packages of all of \
             them. Changes take effect when you apply.",
        )
        .font(sans(FONT_XS))
        .color(TEXT3),
    );
    let mut remove = None;
    for (at, url) in state.draft.packages.stores.iter().enumerate() {
        let known = store.stores.iter().find(|(u, _)| u == url).map(|(_, s)| s);
        Card::new().padding(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(store_name(url, known.and_then(|s| s.index.as_ref())))
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                if url == settings::STORE_INDEX {
                    ui_kit::widgets::badge(ui, "printCAD's", ACCENT);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui_kit::widgets::small_secondary_button(ui, "Remove").clicked() {
                        remove = Some(at);
                    }
                });
            });
            ui.label(RichText::new(url).font(mono(FONT_XS)).color(TEXT3));
            let (said, color) = match known {
                None => ("Read when you apply".to_string(), TEXT3),
                Some(s) if s.looking => ("Reading…".to_string(), TEXT3),
                Some(s) => match (&s.error, &s.index) {
                    (Some(e), Some(_)) => (format!("Showing the list as last read: {e}"), WARNING),
                    (Some(e), None) => (e.clone(), WARNING),
                    (None, Some(index)) => (
                        format!(
                            "{} package{}, as of {}",
                            index.packages.len(),
                            if index.packages.len() == 1 { "" } else { "s" },
                            index.generated.replace('T', " ").trim_end_matches('Z')
                        ),
                        TEXT3,
                    ),
                    (None, None) => ("Not read yet".to_string(), TEXT3),
                },
            };
            ui.label(RichText::new(said).font(sans(FONT_XS)).color(color));
        });
        ui.add_space(SPACE_1);
    }
    if let Some(at) = remove {
        state.draft.packages.stores.remove(at);
    }
    let typed = state.store_add.trim().to_string();
    let valid = (typed.starts_with("https://") || typed.starts_with("http://"))
        && !state.draft.packages.stores.contains(&typed);
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.store_add)
                .hint_text("https://…/index.json")
                .font(mono(FONT_XS))
                .desired_width(320.0),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add = ui
            .add_enabled_ui(valid, |ui| secondary_button(ui, "Add store"))
            .inner
            .clicked();
        if valid && (add || enter) {
            state.draft.packages.stores.push(typed.clone());
            state.store_add.clear();
        }
    });
    if !state
        .draft
        .packages
        .stores
        .iter()
        .any(|u| u == settings::STORE_INDEX)
        && ui_kit::widgets::small_secondary_button(ui, "Add printCAD's store").clicked()
    {
        state
            .draft
            .packages
            .stores
            .insert(0, settings::STORE_INDEX.to_string());
    }
}

/// One package the store lists: what it is, what it asks to reach, and
/// installing or updating it.
fn listing_card(
    ui: &mut Ui,
    state: &mut PreferencesState,
    packages: &[workbenches::PackageStatus],
    url: &str,
    from: Option<&str>,
    listing: &workbenches::Listing,
) {
    let installed = packages.iter().find(|p| p.id == listing.id);
    let installable = listing.installable();
    Card::new().padding(10.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&listing.name)
                    .font(sans_semibold(FONT_SM))
                    .color(TEXT1),
            );
            if let Some(release) = &listing.release {
                ui.label(
                    RichText::new(&release.version)
                        .font(mono(FONT_XS))
                        .color(TEXT3),
                );
            }
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match (installed, &installable) {
                    (Some(status), Ok(release)) => {
                        let tag = status.source.as_ref().map_or("", |s| s.tag.as_str());
                        let newer = status.source.is_some()
                            && workbenches::is_newer(&release.tag, tag, &status.version);
                        if newer {
                            if primary_button(ui, &format!("Update to {}", release.tag)).clicked() {
                                state.package_request =
                                    Some(super::UiCommand::UpdatePackage(listing.id.clone()));
                            }
                        } else {
                            ui_kit::widgets::badge(ui, "Installed", SUCCESS);
                        }
                    }
                    (Some(_), Err(_)) => {
                        ui_kit::widgets::badge(ui, "Installed", SUCCESS);
                    }
                    (None, Ok(_)) => {
                        if primary_button(ui, "Install").clicked() {
                            state.package_request = Some(super::UiCommand::InstallListed {
                                store: url.to_string(),
                                id: listing.id.clone(),
                            });
                        }
                    }
                    (None, Err(_)) => {}
                },
            );
        });
        if !listing.description.is_empty() {
            ui.label(
                RichText::new(&listing.description)
                    .font(sans(FONT_XS))
                    .color(TEXT2),
            );
        }
        if let Err(why) = &installable {
            let color = if listing.removed.is_some() {
                WARNING
            } else {
                DANGER
            };
            ui.label(RichText::new(why).font(sans(FONT_XS)).color(color));
        }
        // What it asks to reach beyond its own folder, the weightiest first.
        ui.horizontal_wrapped(|ui| {
            if let Some(release) = &listing.release {
                let asks = &release.capabilities;
                if asks.helper {
                    ui_kit::widgets::badge(ui, "Runs its own programs", DANGER)
                        .on_hover_text("Outside its sandbox, once you allow it");
                }
                if asks.network {
                    ui_kit::widgets::badge(ui, "Uses the network", WARNING)
                        .on_hover_text("Once you allow it");
                }
                if asks.save_dialog {
                    ui_kit::widgets::badge(ui, "Asks where to save files", TEXT3);
                }
                if !(asks.helper || asks.network || asks.save_dialog) {
                    ui.label(
                        RichText::new("Reaches nothing beyond its own folder")
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                    );
                }
            }
            for category in &listing.categories {
                ui_kit::widgets::badge(ui, &category.replace('-', " "), TEXT3);
            }
            if let Some(from) = from {
                ui_kit::widgets::badge(ui, &format!("from {from}"), ACCENT);
            }
        });
        let by = if listing.maintainers.is_empty() {
            String::new()
        } else {
            format!(" · by {}", listing.maintainers.join(", "))
        };
        ui.label(
            RichText::new(format!("{} · {}{by}", listing.id, listing.license))
                .font(mono(FONT_XS))
                .color(TEXT3),
        );
        ui.horizontal(|ui| {
            ui.hyperlink_to(
                RichText::new(format!("github.com/{}", listing.repository)).font(mono(FONT_XS)),
                format!("https://github.com/{}", listing.repository),
            );
            if let Some(homepage) = &listing.homepage {
                ui.hyperlink_to(RichText::new("Documentation").font(sans(FONT_XS)), homepage);
            }
        });
    });
}

/// The running version and where releases are published.
fn updates_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    release: &crate::app::updates::ReleaseCheck,
    filter: &str,
) {
    use crate::app::updates::{RELEASES_PAGE, ReleaseCheck};
    let checking = *release == ReleaseCheck::Checking;
    let (status, color) = match release {
        ReleaseCheck::NotChecked => ("Not looked yet".to_string(), TEXT3),
        ReleaseCheck::Checking => ("Looking…".to_string(), TEXT3),
        ReleaseCheck::Found {
            tag, newer: true, ..
        } => (format!("{tag} is out"), SUCCESS),
        ReleaseCheck::Found { tag, .. } => (format!("Up to date ({tag})"), TEXT2),
        ReleaseCheck::Failed(_) => ("Could not look".to_string(), WARNING),
    };
    let failure = match release {
        ReleaseCheck::Failed(e) => Some(e.clone()),
        _ => None,
    };
    let newer_page = match release {
        ReleaseCheck::Found {
            page, newer: true, ..
        } => Some(page.clone()),
        _ => None,
    };
    let check = std::cell::Cell::new(false);
    pref_group(
        ui,
        "Version",
        vec![
            PrefRow::text("This build", env!("CARGO_PKG_VERSION").to_string()),
            PrefRow::new("Latest release", |ui| {
                if ui
                    .add_enabled_ui(!checking, |ui| {
                        ui_kit::widgets::small_secondary_button(ui, "Check now")
                    })
                    .inner
                    .on_hover_text("Look at the releases on GitHub; nothing is downloaded")
                    .clicked()
                {
                    check.set(true);
                }
                if let Some(page) = &newer_page {
                    ui.hyperlink_to(RichText::new("Get it").font(sans(FONT_SM)), page);
                }
                let label = ui.label(RichText::new(&status).font(sans(FONT_SM)).color(color));
                if let Some(e) = &failure {
                    label.on_hover_text(e);
                }
                false
            })
            .hint("Updates are never installed by themselves"),
            PrefRow::new("Releases", |ui| {
                ui.hyperlink_to(
                    RichText::new("github.com/gilbertorconde/printCAD/releases")
                        .font(sans(FONT_SM)),
                    RELEASES_PAGE,
                );
                false
            })
            .hint("Every build is published there"),
        ],
        filter,
    );
    pref_group(
        ui,
        "Looking for updates",
        vec![
            PrefRow::toggle("Look at start", &mut state.draft.updates.check_at_start)
                .hint("Once when printCAD opens; a newer release shows in the log"),
        ],
        filter,
    );
    // The page on screen with nothing looked at yet looks by itself.
    if check.get() || (*release == ReleaseCheck::NotChecked && filter.is_empty()) {
        state.package_request = Some(super::UiCommand::CheckForUpdates);
    }
}

fn units_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    pref_group(
        ui,
        "Units",
        vec![
            PrefRow::select(
                "Display unit",
                "prefs_unit",
                &mut state.draft_unit,
                &[
                    (Unit::Mm, Unit::Mm.long_label()),
                    (Unit::Cm, Unit::Cm.long_label()),
                    (Unit::M, Unit::M.long_label()),
                    (Unit::In, Unit::In.long_label()),
                    (Unit::Ft, Unit::Ft.long_label()),
                ],
            )
            .hint("Lengths are stored in millimetres; this only changes how they read"),
        ],
        filter,
    );
    if filter.is_empty() {
        ui.label(
            RichText::new("Saved with the document, not the app.")
                .font(mono(FONT_XS))
                .color(TEXT3),
        );
    }
}

fn import_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    let t = &mut state.draft.import.tessellation;
    let absolute = t.linear_deflection_mode == LinearDeflectionMode::AbsoluteMm;
    let mut rows = vec![PrefRow::select(
        "Linear deflection",
        "prefs_step_linear",
        &mut t.linear_deflection_mode,
        &[
            (
                LinearDeflectionMode::BboxScaled,
                "Scaled by the bounding box",
            ),
            (LinearDeflectionMode::AbsoluteMm, "Absolute chord height"),
        ],
    )];
    if absolute {
        rows.push(PrefRow::qty(
            "Chord tolerance",
            QtyField::mm(&mut t.chord_tolerance)
                .range(0.001..=5.0)
                .speed(0.01)
                .decimals(3),
        ));
    } else {
        rows.push(PrefRow::qty(
            "Mesh deviation",
            QtyField::new(&mut t.mesh_deviation)
                .range(0.01..=1.0)
                .speed(0.005)
                .decimals(3),
        ));
    }
    rows.push(PrefRow::qty(
        "Angular tolerance",
        QtyField::degrees(&mut t.angular_tolerance_deg).range(0.5..=90.0),
    ));
    rows.push(
        PrefRow::toggle("Weld across faces", &mut t.weld_cross_face)
            .hint("Merge vertices shared by faces with close normals"),
    );
    rows.push(PrefRow::qty(
        "Weld angle threshold",
        QtyField::degrees(&mut t.weld_angle_threshold_deg).range(0.0..=90.0),
    ));
    rows.push(
        PrefRow::toggle("Keep shape snapshots", &mut t.persist_brep_snapshot)
            .hint("Serialize each body's shape and mesh in the background"),
    );
    rows.push(
        PrefRow::toggle("Boundary edges", &mut t.generate_boundary_edges)
            .hint("Face boundaries as edge lines"),
    );
    pref_group(ui, "STEP and IGES import defaults", rows, filter);
    if filter.is_empty() {
        ui.label(
            RichText::new("The import dialog opens with these values.")
                .font(mono(FONT_XS))
                .color(TEXT3),
        );
    }
}

/// One movement of the puck: a drawing of the gesture, and beside it
/// everything about what that movement does.
///
/// Returns whether it drew, which is how the search leaves out the movements
/// it does not match.
fn movement_card(
    ui: &mut Ui,
    index: usize,
    device: &mut settings::SixDofSettings,
    filter: &str,
) -> bool {
    let gesture = &SIXDOF_GESTURES[index];
    let assigned = device.assign[index];
    let matches = filter.is_empty()
        || gesture.name.to_lowercase().contains(filter)
        || assigned.label().to_lowercase().contains(filter)
        || "reverse speed".contains(filter);
    if !matches {
        return false;
    }

    overline(ui, gesture.name);
    ui.add_space(SPACE_1);
    Frame::new()
        .fill(BG1)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_MD as u8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 24.0);
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(GESTURE_DRAWING, GESTURE_DRAWING), Sense::hover());
                if let Some(image) =
                    ui_kit::icon::drawing(ui.ctx(), gesture.drawing, GESTURE_DRAWING)
                {
                    image.paint_at(ui, rect);
                }
                ui.add_space(SPACE_3);
                ui.vertical(|ui| {
                    // The controls are shorter than the drawing beside them,
                    // so they sit in the middle of it rather than at the top.
                    // Row heights come from the design system: 40 plain, 44
                    // with a hint under the label.
                    let rows_height = if assigned == SixDofMotion::None {
                        40.0
                    } else {
                        40.0 + 44.0 + 44.0
                    };
                    ui.add_space(((GESTURE_DRAWING - rows_height) / 2.0).max(0.0));
                    let motions: Vec<(SixDofMotion, &str)> = SixDofMotion::ALL
                        .iter()
                        .map(|motion| (*motion, motion.label()))
                        .collect();
                    let mut rows = vec![PrefRow::select(
                        "What it does",
                        gesture.assign_id,
                        &mut device.assign[index],
                        &motions,
                    )];
                    if assigned != SixDofMotion::None {
                        rows.push(
                            PrefRow::toggle("Reverse it", &mut device.invert[index])
                                .hint("The same movement, the other way"),
                        );
                        rows.push(
                            PrefRow::new("Speed", {
                                let speed = &mut device.speed[index];
                                move |ui| {
                                    QtyField::new(speed)
                                        .unit(assigned.speed_unit())
                                        .range(assigned.speed_range())
                                        .decimals(if assigned == SixDofMotion::Zoom {
                                            2
                                        } else {
                                            0
                                        })
                                        .show(ui)
                                }
                            })
                            .hint("At full deflection"),
                        );
                    }
                    pref_group(ui, "", rows, "");
                });
            });
        });
    ui.add_space(SPACE_2);
    true
}

fn input_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &PreferencesInputs<'_>,
    filter: &str,
) {
    let draft = &mut state.draft;
    match state.tab {
        0 => {
            let camera = &mut draft.camera;
            pref_group(
                ui,
                "Navigation",
                vec![
                    PrefRow::toggle("Zoom to cursor", &mut camera.zoom_to_cursor)
                        .hint("Wheel zoom keeps the point under the cursor still"),
                    PrefRow::toggle("Invert zoom", &mut camera.invert_zoom),
                    PrefRow::qty(
                        "Wheel step factor",
                        QtyField::new(&mut camera.wheel_zoom_factor)
                            .range(0.7..=0.995)
                            .speed(0.005)
                            .decimals(3),
                    )
                    .hint("Scale per wheel notch; closer to 1 zooms slower"),
                    PrefRow::qty(
                        "Orbit sensitivity",
                        QtyField::new(&mut camera.orbit_sensitivity)
                            .range(0.05..=2.0)
                            .speed(0.01),
                    ),
                    PrefRow::toggle(
                        "Orbit around the point under the cursor",
                        &mut camera.orbit_pivot_pick,
                    ),
                    PrefRow::qty(
                        "Pan sensitivity",
                        QtyField::new(&mut camera.pan_sensitivity)
                            .range(0.1..=3.0)
                            .speed(0.01),
                    ),
                    PrefRow::select(
                        "Orbit yaw axis",
                        "prefs_yaw",
                        &mut camera.orbit_yaw_axis,
                        &[
                            (OrbitYawAxis::WorldUp, "World up"),
                            (OrbitYawAxis::CameraUp, "Camera up"),
                        ],
                    ),
                    PrefRow::qty(
                        "Click / drag threshold",
                        QtyField::new(&mut camera.click_drag_threshold_px)
                            .unit("px")
                            .range(2.0..=12.0)
                            .speed(0.5)
                            .decimals(1),
                    ),
                ],
                filter,
            );
        }
        1 => {
            // A page reaches a device only once the user picks it in the
            // browser's chooser.
            #[cfg(target_arch = "wasm32")]
            {
                let mut choose = false;
                let connected = inputs.nav_device.unwrap_or("None").to_string();
                pref_group(
                    ui,
                    "Device",
                    vec![
                        PrefRow::new("Connected", |ui| {
                            let button = ui_kit::widgets::small_secondary_button(ui, "Choose…");
                            choose = button.clicked();
                            ui.label(RichText::new(&connected).font(sans(FONT_SM)).color(TEXT1));
                            false
                        })
                        .hint("The browser lists the pucks plugged in; one chosen is found again"),
                    ],
                    filter,
                );
                if choose {
                    state.package_request = Some(super::UiCommand::ChooseNavDevice);
                }
            }
            let device = &mut draft.sixdof;
            pref_group(
                ui,
                "6-DoF mouse",
                vec![
                    PrefRow::toggle("Steer the view with a 6-DoF mouse", &mut device.enabled)
                        .hint("A six-axis puck moves the view while it is held"),
                    PrefRow::qty(
                        "Dead zone",
                        QtyField::new(&mut device.dead_zone)
                            .range(0.0..=0.5)
                            .speed(0.005)
                            .decimals(3),
                    )
                    .hint("Deflection below this share of full scale counts as rest"),
                    PrefRow::qty(
                        "Full deflection",
                        QtyField::new(&mut device.full_scale)
                            .range(50.0..=2000.0)
                            .speed(5.0)
                            .decimals(0),
                    )
                    .hint("The reading a fully pushed axis produces"),
                    PrefRow::toggle("One axis at a time", &mut device.dominant_axis)
                        .hint("Only the axis pushed hardest acts, so a gesture stays square"),
                ],
                filter,
            );

            let mut drawn = false;
            for index in 0..SIXDOF_GESTURES.len() {
                drawn |= movement_card(ui, index, device, filter);
            }
            if drawn {
                ui.add_space(SPACE_2);
            }

            // A device with no buttons still gets the two rows a common puck
            // has, so the page is not empty before one is plugged in.
            let count = (inputs.nav_buttons.max(2) as usize).min(NAV_BUTTON_IDS.len());
            if device.buttons.len() < count {
                device
                    .buttons
                    .resize(count, settings::SixDofButtonAction::None);
            }
            let actions: Vec<(settings::SixDofButtonAction, &str)> =
                settings::SixDofButtonAction::ALL
                    .iter()
                    .map(|action| (*action, action.label()))
                    .collect();
            let button_labels: Vec<String> = (1..=count)
                .map(|button| format!("Button {button}"))
                .collect();
            let rows = device
                .buttons
                .iter_mut()
                .take(count)
                .zip(&button_labels)
                .zip(NAV_BUTTON_IDS)
                .map(|((action, label), id)| PrefRow::select(label, id, action, &actions))
                .collect();
            pref_group(ui, "Buttons", rows, filter);
        }
        _ => {}
    }
}

#[cfg(test)]
mod rail {
    use super::*;
    use core_document::{Workbench, WorkbenchContext, WorkbenchDescriptor};

    struct Bench(&'static str, &'static str, bool);

    impl Workbench for Bench {
        fn descriptor(&self) -> WorkbenchDescriptor {
            WorkbenchDescriptor::new(self.0, self.1, "")
        }
        fn has_settings(&self) -> bool {
            self.2
        }
        fn configure(&self, _context: &mut WorkbenchContext) {}
    }

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
}
