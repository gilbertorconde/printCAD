//! The Preferences dialog: a modal with a group rail, a tab strip per
//! group and rows edited on a draft that Apply or OK commit.

use axes::AxisPreset;
use core_document::{DocumentService, Unit, WorkbenchId};
use egui::{
    Align, Context, CornerRadius, Frame, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    Vec2, pos2, vec2,
};
use kernel_api::LinearDeflectionMode;
use settings::{NavigationStyle, OrbitYawAxis, ProjectionMode, SixDofMotion, UserSettings};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, PrefRow, QtyField, note_card, overline, pref_group, primary_button, secondary_button,
};
use ui_kit::{mono, sans, sans_medium, sans_semibold};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrefGroup {
    #[default]
    General,
    Display,
    Input,
    Sketcher,
    PartDesign,
    Units,
    ImportExport,
    Printing,
    Updates,
}

impl PrefGroup {
    pub const ALL: [PrefGroup; 9] = [
        PrefGroup::General,
        PrefGroup::Display,
        PrefGroup::Input,
        PrefGroup::Sketcher,
        PrefGroup::PartDesign,
        PrefGroup::Units,
        PrefGroup::ImportExport,
        PrefGroup::Printing,
        PrefGroup::Updates,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrefGroup::General => "General",
            PrefGroup::Display => "Display",
            PrefGroup::Input => "Input",
            PrefGroup::Sketcher => "Sketcher",
            PrefGroup::PartDesign => "Part Design",
            PrefGroup::Units => "Units",
            PrefGroup::ImportExport => "Import / Export",
            PrefGroup::Printing => "3D printing",
            PrefGroup::Updates => "Updates",
        }
    }

    pub fn tabs(self) -> &'static [&'static str] {
        match self {
            PrefGroup::General => &["Interface", "About"],
            PrefGroup::Display => &["Camera", "Lighting", "Rendering"],
            PrefGroup::Input => &["Mouse", "6-DoF mouse"],
            PrefGroup::Sketcher => &["General"],
            PrefGroup::PartDesign => &["General"],
            PrefGroup::Units => &["Units"],
            PrefGroup::ImportExport => &["STEP"],
            PrefGroup::Printing => &["Printer"],
            PrefGroup::Updates => &["Updates"],
        }
    }

    /// Groups the app has no settings for yet.
    fn planned(self) -> bool {
        matches!(self, PrefGroup::Printing | PrefGroup::Updates)
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
    /// How many buttons the connected 6-DoF mouse has, so the page offers a
    /// row per button it actually owns. Zero when none is connected.
    pub nav_buttons: u32,
}

/// The six ways the puck moves, in the order the device reports them: what
/// the hand does, and the icon that shows it.
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
    // itself to the centre every frame — an anchored area cannot be dragged.
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
            draw_rail(ui, rail, state);
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

fn draw_rail(ui: &mut Ui, rect: Rect, state: &mut PreferencesState) {
    let mut rail = region(
        ui,
        rect.shrink2(vec2(10.0, 12.0)),
        Layout::top_down(Align::Min),
    );
    rail.spacing_mut().item_spacing.y = 2.0;
    for group in PrefGroup::ALL {
        let active = state.group == group;
        let (row, response) =
            rail.allocate_exact_size(vec2(rail.available_width(), 30.0), Sense::click());
        if active {
            rail.painter().rect_filled(row, RADIUS_SM, BG2);
        } else if response.hovered() {
            rail.painter()
                .rect_filled(row, RADIUS_SM, with_alpha(BG2, 0.6));
        }
        let color = if active {
            TEXT1
        } else if group.planned() {
            TEXT3
        } else {
            TEXT2
        };
        rail.painter().text(
            pos2(row.left() + 26.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            group.label(),
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
    strip.spacing_mut().item_spacing.x = SPACE_4;
    for (i, tab) in state.group.tabs().iter().enumerate() {
        let active = state.tab == i;
        let galley = strip.painter().layout_no_wrap(
            tab.to_string(),
            sans_medium(FONT_SM),
            if active { TEXT1 } else { TEXT2 },
        );
        let (r, resp) = strip.allocate_exact_size(vec2(galley.size().x, TABS), Sense::click());
        strip.painter().galley(
            pos2(r.left(), r.center().y - galley.size().y / 2.0),
            galley,
            if active { TEXT1 } else { TEXT2 },
        );
        if active {
            strip
                .painter()
                .hline(r.x_range(), r.bottom() - 1.0, Stroke::new(2.0, ACCENT));
        }
        if resp.clicked() {
            state.tab = i;
        }
    }
    ui.painter()
        .hline(rect.x_range(), tabs.bottom(), Stroke::new(1.0, BORDER));

    let body = Rect::from_min_max(pos2(rect.left(), tabs.bottom()), rect.max);
    let mut page = region(ui, body, Layout::top_down(Align::Min));
    page.set_clip_rect(body);
    egui::ScrollArea::vertical()
        .id_salt(("prefs_page", state.group as u8, state.tab))
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
                        PrefGroup::Sketcher => {
                            workbench_page(ui, inputs.registry, "wb.sketch", &filter)
                        }
                        PrefGroup::PartDesign => {
                            workbench_page(ui, inputs.registry, "wb.part", &filter)
                        }
                        PrefGroup::Units => units_page(ui, state, &filter),
                        PrefGroup::ImportExport => import_page(ui, state, &filter),
                        PrefGroup::Printing => {
                            // PLANNED: printer profiles, bed size and export
                            // presets for slicers.
                            note_card(
                                ui,
                                Note::Info,
                                Some("Planned"),
                                "Printer profiles, bed size and slicer export presets live here once printing support lands.",
                            );
                        }
                        PrefGroup::Updates => {
                            // PLANNED: release channel and update checks.
                            note_card(
                                ui,
                                Note::Info,
                                Some("Planned"),
                                "Update checks and the release channel live here once builds are published.",
                            );
                        }
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
            state.draft.rendering.selection_color = defaults.rendering.selection_color;
            state.draft.rendering.selection_opacity = defaults.rendering.selection_opacity;
            state.draft.preferred_gpu = defaults.preferred_gpu;
        }
        PrefGroup::Input => {
            let camera = &mut state.draft.camera;
            camera.navigation_style = defaults.camera.navigation_style;
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
        PrefGroup::Units => state.draft_unit = Unit::Mm,
        PrefGroup::ImportExport => state.draft.import = defaults.import,
        PrefGroup::Sketcher | PrefGroup::PartDesign | PrefGroup::Printing | PrefGroup::Updates => {}
    }
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
            pref_group(
                ui,
                "Diagnostics",
                vec![
                    PrefRow::toggle(
                        "Write a report for every STEP import",
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
        }
    }
}

fn workbench_page(ui: &mut Ui, registry: &mut DocumentService, id: &str, filter: &str) {
    if let Ok(wb) = registry.workbench_mut(&WorkbenchId::from(id)) {
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
    for g in PrefGroup::ALL {
        for t in 0..g.tabs().len() {
            state.group = g;
            state.tab = t;
            match g {
                PrefGroup::General => general_page(ui, state, inputs, filter),
                PrefGroup::Display => display_page(ui, state, inputs, filter),
                PrefGroup::Input => input_page(ui, state, inputs, filter),
                PrefGroup::Sketcher => workbench_page(ui, inputs.registry, "wb.sketch", filter),
                PrefGroup::PartDesign => workbench_page(ui, inputs.registry, "wb.part", filter),
                PrefGroup::Units => units_page(ui, state, filter),
                PrefGroup::ImportExport => import_page(ui, state, filter),
                PrefGroup::Printing | PrefGroup::Updates => {}
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
    pref_group(ui, "STEP import defaults", rows, filter);
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
                    PrefRow::select(
                        "Navigation style",
                        "prefs_nav_style",
                        &mut camera.navigation_style,
                        &[
                            (NavigationStyle::Gesture, "Gesture"),
                            (NavigationStyle::Cad, "CAD"),
                        ],
                    ),
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
