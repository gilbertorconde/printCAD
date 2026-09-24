//! The widget vocabulary of the mockups: cards, badges, toggles, quantity
//! and select fields, check rows, note cards, buttons, tool buttons and
//! section headers. Each is a plain function over `egui::Ui` so any crate
//! that draws UI can use it.

use egui::{
    Color32, CornerRadius, Frame, InnerResponse, Layout, Margin, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2, WidgetText,
};

use crate::icon;
use crate::theme::{mono, sans, sans_semibold};
use crate::tokens::*;

/// A bordered surface. `primary` cards carry the accent border and tint.
pub struct Card {
    fill: Color32,
    border: Color32,
    padding: f32,
    radius: f32,
}

impl Default for Card {
    fn default() -> Self {
        Self {
            fill: BG1,
            border: BORDER,
            padding: SPACE_3,
            radius: RADIUS_MD,
        }
    }
}

impl Card {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn primary() -> Self {
        Self {
            fill: ACCENT_FAINT,
            border: ACCENT,
            ..Self::default()
        }
    }

    /// A card that sits over the viewport: translucent surface.
    pub fn floating() -> Self {
        Self {
            fill: OVERLAY_CARD,
            border: BORDER,
            padding: SPACE_2,
            radius: RADIUS_MD,
        }
    }

    pub fn fill(mut self, fill: Color32) -> Self {
        self.fill = fill;
        self
    }

    pub fn border(mut self, border: Color32) -> Self {
        self.border = border;
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn frame(&self) -> Frame {
        Frame::new()
            .fill(self.fill)
            .stroke(Stroke::new(1.0, self.border))
            .corner_radius(CornerRadius::same(self.radius as u8))
            .inner_margin(Margin::same(self.padding as i8))
    }

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        self.frame().show(ui, add)
    }
}

/// Small uppercase label above a section.
pub fn overline(ui: &mut Ui, text: &str) -> Response {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(sans_semibold(FONT_XS))
            .color(TEXT3)
            .extra_letter_spacing(0.6),
    )
}

/// Monospace text in `color`.
pub fn mono_label(ui: &mut Ui, text: impl Into<String>, size: f32, color: Color32) -> Response {
    ui.label(RichText::new(text).font(mono(size)).color(color))
}

/// A tiny bordered pill: `TIP`, `EDITING`, a key hint.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    Frame::new()
        .stroke(Stroke::new(1.0, with_alpha(color, 0.4)))
        .corner_radius(CornerRadius::same(RADIUS_SM as u8))
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .font(sans_semibold(10.0))
                    .color(color)
                    .extra_letter_spacing(0.4),
            )
        })
        .response
}

/// A key cap: mono 10px in a bordered box.
pub fn key_chip(ui: &mut Ui, text: &str) -> Response {
    Frame::new()
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_SM as u8))
        .inner_margin(Margin::symmetric(4, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(mono(10.0)).color(TEXT2))
        })
        .response
}

/// A 1px vertical rule of `height` px.
pub fn vseparator(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
}

/// A 34×18 switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(34.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let track = if *on { ACCENT } else { BG4 };
    let knob = if *on { ACCENT_TEXT } else { TEXT2 };
    let painter = ui.painter();
    painter.rect_filled(rect, 9.0, track);
    let x = rect.left() + 9.0 + t * (size.x - 18.0);
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, knob);
    response
}

/// A checkbox row: 14px box, accent when on, label text1/text2.
pub fn check_row(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let text = RichText::new(label)
        .font(sans(FONT_SM))
        .color(if *on { TEXT1 } else { TEXT2 });
    let galley = ui.painter().layout_no_wrap(
        text.text().to_owned(),
        sans(FONT_SM),
        if *on { TEXT1 } else { TEXT2 },
    );
    let width = 14.0 + SPACE_2 + galley.size().x;
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let painter = ui.painter();
    let box_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 7.0),
        Vec2::splat(14.0),
    );
    if *on {
        painter.rect_filled(box_rect, RADIUS_SM, ACCENT);
        let c = box_rect.center();
        painter.line_segment(
            [egui::pos2(c.x - 3.5, c.y), egui::pos2(c.x - 1.0, c.y + 2.5)],
            Stroke::new(1.5, ACCENT_TEXT),
        );
        painter.line_segment(
            [
                egui::pos2(c.x - 1.0, c.y + 2.5),
                egui::pos2(c.x + 3.5, c.y - 2.5),
            ],
            Stroke::new(1.5, ACCENT_TEXT),
        );
    } else {
        painter.rect_filled(box_rect, RADIUS_SM, BG2);
        painter.rect_stroke(
            box_rect,
            RADIUS_SM,
            Stroke::new(1.0, BORDER_STRONG),
            egui::StrokeKind::Inside,
        );
    }
    painter.galley(
        egui::pos2(
            box_rect.right() + SPACE_2,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        TEXT1,
    );
    response
}

/// Tone of a note card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Info,
    Warning,
    Error,
    Success,
}

impl Note {
    pub fn color(self) -> Color32 {
        match self {
            Note::Info => INFO,
            Note::Warning => WARNING,
            Note::Error => DANGER,
            Note::Success => SUCCESS,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Note::Info => "info",
            Note::Warning => "warning",
            Note::Error => "error",
            Note::Success => "check",
        }
    }
}

/// A tinted message box: icon, optional bold title, body.
pub fn note_card(ui: &mut Ui, note: Note, title: Option<&str>, body: &str) -> Response {
    let color = note.color();
    Frame::new()
        .fill(with_alpha(color, 0.08))
        .stroke(Stroke::new(1.0, with_alpha(color, 0.35)))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                icon::draw(ui, note.icon(), 16.0, color);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if let Some(title) = title {
                        ui.label(
                            RichText::new(title)
                                .font(sans_semibold(FONT_SM))
                                .color(color),
                        );
                    }
                    ui.label(RichText::new(body).font(sans(11.5)).color(TEXT2));
                });
            });
        })
        .response
}

/// Colors and metrics of a text button.
struct ButtonStyle {
    fill: Color32,
    hover_fill: Color32,
    border: Color32,
    text: Color32,
    font: egui::FontId,
    height: f32,
    padding: f32,
}

fn text_button(ui: &mut Ui, label: &str, style: ButtonStyle) -> Response {
    let id = ui.next_auto_id();
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let fill = if hovered {
        style.hover_fill
    } else {
        style.fill
    };
    let inner = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, style.border))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(
            style.padding as i8,
            ((style.height - 16.0) / 2.0) as i8,
        ))
        .show(ui, |ui| {
            ui.add(egui::Label::new(
                RichText::new(label).font(style.font).color(style.text),
            ));
        });
    ui.interact(inner.response.rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Accent-filled action button (OK, Import).
pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
    text_button(
        ui,
        label,
        ButtonStyle {
            fill: ACCENT,
            hover_fill: ACCENT_HOVER,
            border: ACCENT,
            text: ACCENT_TEXT,
            font: sans_semibold(FONT_SM),
            height: 28.0,
            padding: SPACE_4,
        },
    )
}

/// Bordered transparent button (Cancel, Remove).
pub fn secondary_button(ui: &mut Ui, label: &str) -> Response {
    text_button(
        ui,
        label,
        ButtonStyle {
            fill: Color32::TRANSPARENT,
            hover_fill: BG3,
            border: BORDER_STRONG,
            text: TEXT1,
            font: sans(FONT_SM),
            height: 28.0,
            padding: 14.0,
        },
    )
}

/// Bordered accent button (Add).
pub fn accent_outline_button(ui: &mut Ui, label: &str) -> Response {
    text_button(
        ui,
        label,
        ButtonStyle {
            fill: with_alpha(ACCENT, 0.14),
            hover_fill: ACCENT_DIM,
            border: ACCENT,
            text: ACCENT,
            font: sans(FONT_SM),
            height: 26.0,
            padding: SPACE_3,
        },
    )
}

/// 22px bordered button for status bars and row actions.
pub fn small_secondary_button(ui: &mut Ui, label: &str) -> Response {
    text_button(
        ui,
        label,
        ButtonStyle {
            fill: Color32::TRANSPARENT,
            hover_fill: BG3,
            border: BORDER_STRONG,
            text: TEXT1,
            font: sans(FONT_XS),
            height: 22.0,
            padding: SPACE_2,
        },
    )
}

/// Red-tinted button for destructive actions.
pub fn destructive_button(ui: &mut Ui, label: &str) -> Response {
    text_button(
        ui,
        label,
        ButtonStyle {
            fill: with_alpha(DANGER, 0.1),
            hover_fill: with_alpha(DANGER, 0.2),
            border: with_alpha(DANGER, 0.5),
            text: DANGER,
            font: sans(FONT_SM),
            height: 28.0,
            padding: 14.0,
        },
    )
}

/// The visual state of a toolbar button.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolButtonState {
    pub enabled: bool,
    pub active: bool,
    /// Present in the design but not implemented: drawn dimmed with a note.
    pub planned: Option<&'static str>,
    /// Draw a small chevron after the icon: the button opens a menu.
    pub menu: bool,
}

/// An icon-only toolbar button. `label` is the tooltip.
pub fn tool_button(
    ui: &mut Ui,
    icon_name: &str,
    label: &str,
    size: f32,
    state: ToolButtonState,
) -> Response {
    let planned = state.planned.is_some();
    let enabled = state.enabled && !planned;
    let width = if state.menu { size + 12.0 } else { size };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, size), Sense::click());
    let hovered = enabled && response.hovered();
    let tint = if !enabled {
        TEXT3
    } else if state.active {
        ACCENT
    } else if hovered {
        TEXT1
    } else {
        ICON
    };
    let fill = if state.active {
        ACCENT_DIM
    } else if hovered {
        BG3
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, fill);
    let icon_px = (size * 2.0 / 3.0).round();
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + size / 2.0, rect.center().y),
        Vec2::splat(icon_px),
    );
    if let Some(handle) = icon::texture(ui.ctx(), icon_name) {
        painter.image(
            handle.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    } else {
        painter.text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            label.chars().next().unwrap_or('?'),
            sans(FONT_SM),
            tint,
        );
    }
    if state.menu
        && let Some(chev) = icon::texture(ui.ctx(), "chevron-down")
    {
        let r = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 7.0, rect.center().y),
            Vec2::splat(10.0),
        );
        painter.image(
            chev.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            TEXT3,
        );
    }
    let response = match state.planned {
        Some(note) => response.on_hover_text(format!("{label} (planned)\n{note}")),
        None if !enabled => response.on_hover_text(label),
        None => response.on_hover_text(label),
    };
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A collapsible section title with a chevron and an optional count. Returns
/// whether the body is open.
pub fn section_header(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    count: Option<usize>,
    default_open: bool,
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let mut open = ui
        .data_mut(|d| d.get_persisted::<bool>(id))
        .unwrap_or(default_open);
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_2;
            icon::draw(
                ui,
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                12.0,
                TEXT3,
            );
            ui.label(
                RichText::new(title)
                    .font(sans_semibold(FONT_SM))
                    .color(TEXT1),
            );
            if let Some(n) = count {
                mono_label(ui, n.to_string(), FONT_XS, TEXT3);
            }
        })
        .response;
    let response = ui.interact(response.rect, id.with("hit"), Sense::click());
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    open
}

/// The label column of a two-column parameter grid.
pub fn field_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT2))
}

/// A numeric field: mono value with a unit suffix, dragging and typing both
/// work. `dim` renders it as not applicable; `error` paints it red with an
/// inline message.
pub struct QtyField<'a> {
    value: &'a mut f32,
    unit: &'static str,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
    dim: bool,
    error: Option<&'a str>,
    width: f32,
}

impl<'a> QtyField<'a> {
    pub fn new(value: &'a mut f32) -> Self {
        Self {
            value,
            unit: "",
            speed: 0.1,
            range: f64::NEG_INFINITY..=f64::INFINITY,
            decimals: 2,
            dim: false,
            error: None,
            width: 120.0,
        }
    }

    pub fn mm(value: &'a mut f32) -> Self {
        Self::new(value).unit("mm").range(0.001..=1.0e6)
    }

    pub fn degrees(value: &'a mut f32) -> Self {
        Self::new(value).unit("°").speed(0.5)
    }

    pub fn unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn range(mut self, range: std::ops::RangeInclusive<f64>) -> Self {
        self.range = range;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Draws the field; returns whether the value changed.
    pub fn show(self, ui: &mut Ui) -> bool {
        self.show_settling(ui).changed
    }

    /// Draws the field and says whether the value changed and whether an
    /// edit settled this frame: a drag let go, or typing finished. A caller
    /// that applies every change live saves on the settle.
    pub fn show_settling(self, ui: &mut Ui) -> QtyEdit {
        let border = if self.error.is_some() { DANGER } else { BORDER };
        let text = if self.dim {
            TEXT3
        } else if self.error.is_some() {
            DANGER
        } else {
            TEXT1
        };
        let mut changed = false;
        let mut settled = false;
        let mut hover: Option<Response> = None;
        Frame::new()
            .fill(BG2)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(SPACE_2 as i8, 0))
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(self.width, INPUT - 2.0));
                ui.set_max_width(self.width);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_1;
                    let focus = ui.visuals_mut();
                    focus.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                    focus.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    focus.widgets.inactive.bg_stroke = Stroke::NONE;
                    focus.widgets.hovered.bg_fill = Color32::TRANSPARENT;
                    focus.widgets.hovered.weak_bg_fill = Color32::TRANSPARENT;
                    focus.widgets.hovered.bg_stroke = Stroke::NONE;
                    focus.widgets.active.bg_stroke = Stroke::NONE;
                    focus.widgets.active.bg_fill = Color32::TRANSPARENT;
                    focus.override_text_color = Some(text);
                    ui.style_mut().override_font_id = Some(mono(FONT_SM));
                    let suffix = if self.unit.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", self.unit)
                    };
                    let mut v = *self.value as f64;
                    let resp = ui.add_enabled(
                        !self.dim,
                        egui::DragValue::new(&mut v)
                            .speed(self.speed)
                            .range(self.range.clone())
                            .fixed_decimals(self.decimals)
                            .suffix(suffix),
                    );
                    if resp.changed() {
                        *self.value = v as f32;
                        changed = true;
                    }
                    settled = resp.drag_stopped() || resp.lost_focus();
                    hover = Some(resp);
                    if let Some(err) = self.error {
                        ui.label(RichText::new(err).font(sans(FONT_XS)).color(DANGER));
                    }
                });
            });
        QtyEdit {
            changed,
            settled,
            response: hover,
        }
    }
}

/// What [`QtyField::show_settling`] saw this frame.
pub struct QtyEdit {
    pub changed: bool,
    /// A drag was let go or typing finished.
    pub settled: bool,
    /// The value's own response, for a tooltip.
    pub response: Option<Response>,
}

/// A dropdown of labelled options; returns whether the selection changed.
pub fn select_field<T: PartialEq + Copy>(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    current: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let label = options
        .iter()
        .find(|(v, _)| *v == *current)
        .map(|(_, l)| *l)
        .unwrap_or("-");
    let mut changed = false;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(RichText::new(label).font(sans(FONT_SM)))
        .width(width)
        .show_ui(ui, |ui| {
            for (value, text) in options {
                if ui
                    .selectable_label(*value == *current, RichText::new(*text).font(sans(FONT_SM)))
                    .clicked()
                    && *value != *current
                {
                    *current = *value;
                    changed = true;
                }
            }
        });
    changed
}

/// A widget the design shows but the app has not built yet: drawn disabled
/// with the note as its tooltip.
pub fn planned<R>(ui: &mut Ui, note: &str, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let inner = ui.add_enabled_ui(false, add);
    let response = inner
        .response
        .clone()
        .on_disabled_hover_text(format!("Planned: {note}"));
    InnerResponse::new(inner.inner, response)
}

/// Text for a label: proportional at `size` in `color`.
pub fn text(s: impl Into<String>, size: f32, color: Color32) -> WidgetText {
    RichText::new(s).font(sans(size)).color(color).into()
}

/// One preferences row: a label with an optional hint on the left and a
/// control on the right. The control closure returns whether it changed
/// the value.
pub struct PrefRow<'a> {
    pub label: &'a str,
    pub hint: Option<&'a str>,
    /// An icon drawn ahead of the label, for rows whose subject is easier to
    /// show than to name.
    pub icon: Option<&'a str>,
    control: Box<dyn FnOnce(&mut Ui) -> bool + 'a>,
}

impl<'a> PrefRow<'a> {
    pub fn new(label: &'a str, control: impl FnOnce(&mut Ui) -> bool + 'a) -> Self {
        Self {
            label,
            hint: None,
            icon: None,
            control: Box::new(control),
        }
    }

    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn toggle(label: &'a str, on: &'a mut bool) -> Self {
        Self::new(label, move |ui| toggle(ui, on).changed())
    }

    /// A toggle for something not built yet: disabled, showing `on`.
    pub fn planned_toggle(label: &'a str, note: &'a str, on: bool) -> Self {
        Self::new(label, move |ui| {
            let mut value = on;
            planned(ui, note, |ui| toggle(ui, &mut value));
            false
        })
    }

    pub fn select<T: PartialEq + Copy + 'a>(
        label: &'a str,
        id_salt: &'static str,
        current: &'a mut T,
        options: &'a [(T, &'a str)],
    ) -> Self {
        Self::new(label, move |ui| {
            select_field(ui, id_salt, current, options, 180.0)
        })
    }

    pub fn qty(label: &'a str, field: QtyField<'a>) -> Self {
        Self::new(label, move |ui| field.width(120.0).show(ui))
    }

    pub fn color(label: &'a str, rgb: &'a mut [f32; 3]) -> Self {
        Self::new(label, move |ui| {
            let mut color = Color32::from_rgb(
                (rgb[0] * 255.0) as u8,
                (rgb[1] * 255.0) as u8,
                (rgb[2] * 255.0) as u8,
            );
            if ui.color_edit_button_srgba(&mut color).changed() {
                *rgb = [
                    color.r() as f32 / 255.0,
                    color.g() as f32 / 255.0,
                    color.b() as f32 / 255.0,
                ];
                true
            } else {
                false
            }
        })
    }

    /// A read-only value.
    pub fn text(label: &'a str, value: String) -> Self {
        Self::new(label, move |ui| {
            ui.label(RichText::new(value).font(sans(FONT_SM)).color(TEXT2));
            false
        })
    }

    /// A read-only color swatch with its hex value.
    pub fn swatch(label: &'a str, rgb: [f32; 3]) -> Self {
        Self::new(label, move |ui| {
            let color = Color32::from_rgb(
                (rgb[0] * 255.0) as u8,
                (rgb[1] * 255.0) as u8,
                (rgb[2] * 255.0) as u8,
            );
            mono_label(
                ui,
                format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b()),
                FONT_XS,
                TEXT3,
            );
            let (rect, _) = ui.allocate_exact_size(Vec2::new(28.0, 16.0), Sense::hover());
            ui.painter().rect(
                rect,
                3.0,
                color,
                Stroke::new(1.0, BORDER_STRONG),
                StrokeKind::Inside,
            );
            false
        })
    }

    fn matches(&self, filter: &str) -> bool {
        filter.is_empty()
            || self.label.to_lowercase().contains(filter)
            || self.hint.is_some_and(|h| h.to_lowercase().contains(filter))
    }
}

/// A titled, bordered group of preference rows. Rows whose label or hint
/// does not contain `filter` (lowercase) are left out; a group with no
/// rows left draws nothing. Returns whether any control changed.
pub fn pref_group(ui: &mut Ui, title: &str, rows: Vec<PrefRow<'_>>, filter: &str) -> bool {
    let rows: Vec<PrefRow<'_>> = rows.into_iter().filter(|r| r.matches(filter)).collect();
    if rows.is_empty() {
        return false;
    }
    let mut changed = false;
    if !title.is_empty() {
        overline(ui, title);
        ui.add_space(SPACE_1);
    }
    Frame::new()
        .fill(BG1)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_MD as u8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = rows.len();
            for (i, row) in rows.into_iter().enumerate() {
                let PrefRow {
                    label,
                    hint,
                    icon,
                    control,
                } = row;
                let height = if hint.is_some() { 44.0 } else { 40.0 };
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
                let mut inner = rect.shrink2(Vec2::new(14.0, 0.0));
                if let Some(icon) = icon {
                    let box_size = 20.0;
                    let at = egui::Rect::from_center_size(
                        egui::pos2(inner.left() + box_size / 2.0, inner.center().y),
                        Vec2::splat(box_size),
                    );
                    let mut slot = ui.new_child(egui::UiBuilder::new().max_rect(at));
                    crate::icon::draw(&mut slot, icon, box_size, TEXT2);
                    inner.set_left(at.right() + SPACE_3);
                }
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(Layout::top_down(egui::Align::Min)),
                );
                left.spacing_mut().item_spacing.y = 2.0;
                let text_top = if hint.is_some() { 7.0 } else { 12.0 };
                left.add_space(text_top);
                left.label(RichText::new(label).font(sans(FONT_SM)).color(TEXT1));
                if let Some(hint) = hint {
                    left.label(RichText::new(hint).font(sans(FONT_XS)).color(TEXT3));
                }
                // The control strip is one input tall, centred in the row,
                // so fields keep their own height.
                let strip = egui::Rect::from_x_y_ranges(
                    inner.x_range(),
                    (inner.center().y - INPUT / 2.0)..=(inner.center().y + INPUT / 2.0),
                );
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(strip)
                        .layout(Layout::right_to_left(egui::Align::Center)),
                );
                right.spacing_mut().item_spacing.x = SPACE_2;
                changed |= control(&mut right);
                if i + 1 < count {
                    ui.painter()
                        .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, BORDER));
                }
            }
        });
    ui.add_space(SPACE_4);
    changed
}

/// One tab of a strip: the open documents, a panel's pages, the chats.
/// The active tab is filled and underlined in the accent; a closable tab
/// shows its close while hovered or active, and a middle click closes it
/// too.
pub struct Tab<'a> {
    label: &'a str,
    active: bool,
    /// Fixed width; `None` fits the label.
    width: Option<f32>,
    height: f32,
    /// A dot for unsaved edits.
    dirty: bool,
    /// Closable, with this hover text on its close.
    close: Option<&'a str>,
}

/// What a click on a tab asked for.
pub struct TabResponse {
    /// The tab itself, for hover text.
    pub response: Response,
    /// Clicked while not active.
    pub selected: bool,
    /// Its close was clicked, or it was middle-clicked.
    pub closed: bool,
}

const TAB_CLOSE: f32 = 16.0;
const TAB_PAD: f32 = 10.0;

impl<'a> Tab<'a> {
    pub fn new(label: &'a str, active: bool) -> Self {
        Self {
            label,
            active,
            width: None,
            height: TAB_BAR - 4.0,
            dirty: false,
            close: None,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    pub fn closable(mut self, hint: &'a str) -> Self {
        self.close = Some(hint);
        self
    }

    pub fn show(self, ui: &mut Ui) -> TabResponse {
        let font = if self.active {
            crate::theme::sans_medium(FONT_SM)
        } else {
            sans(FONT_SM)
        };
        let text_color = if self.active { TEXT1 } else { TEXT2 };
        // Room right of the label: the close, and the dot before it.
        let trailing =
            self.close.map_or(0.0, |_| TAB_CLOSE + 4.0) + if self.dirty { 12.0 } else { 0.0 };
        let width = self.width.unwrap_or_else(|| {
            let text =
                ui.painter()
                    .layout_no_wrap(self.label.to_string(), font.clone(), text_color);
            text.size().x + 2.0 * TAB_PAD + trailing
        });
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, self.height), Sense::click());
        let hovered = response.hovered();
        let painter = ui.painter();
        if self.active {
            painter.rect_filled(rect, RADIUS_SM, BG0);
            painter.hline(
                rect.x_range(),
                rect.bottom() - 1.0,
                Stroke::new(2.0, ACCENT),
            );
        } else if hovered {
            painter.rect_filled(rect, RADIUS_SM, BG2);
        }

        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 6.0 - TAB_CLOSE / 2.0, rect.center().y),
            Vec2::splat(TAB_CLOSE),
        );
        let mut text_right = if self.close.is_some() {
            close_rect.left() - 4.0
        } else {
            rect.right() - TAB_PAD
        };
        if self.dirty {
            painter.circle_filled(egui::pos2(text_right - 4.0, rect.center().y), 3.0, WARNING);
            text_right -= 12.0;
        }
        let max_width = (text_right - rect.left() - TAB_PAD).max(0.0);
        let galley = painter.layout(self.label.to_string(), font, text_color, max_width);
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(text_right, rect.max.y));
        painter.with_clip_rect(clip).galley(
            egui::pos2(
                rect.left() + TAB_PAD,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            text_color,
        );

        let mut closed = false;
        if let Some(hint) = self.close {
            let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
            if hovered || self.active || close.hovered() {
                if close.hovered() {
                    ui.painter().rect_filled(close_rect, RADIUS_SM, BG2);
                }
                ui.painter().text(
                    close_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "×",
                    sans(FONT_MD),
                    if close.hovered() { TEXT1 } else { TEXT3 },
                );
            }
            closed = close.on_hover_text(hint).clicked() || response.middle_clicked();
        }
        TabResponse {
            selected: !closed && response.clicked() && !self.active,
            closed,
            response,
        }
    }
}

/// The "+" at the end of a tab strip.
pub fn tab_plus(ui: &mut Ui, height: f32) -> Response {
    let (rect, plus) = ui.allocate_exact_size(Vec2::new(26.0, height), Sense::click());
    if plus.hovered() {
        ui.painter().rect_filled(rect, RADIUS_SM, BG2);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        crate::theme::sans_medium(FONT_MD),
        if plus.hovered() { TEXT1 } else { TEXT2 },
    );
    plus
}

/// What a [`FormulaField`] asks of whoever shows it: how typed text reads.
pub trait FormulaHost {
    /// The value `text` gives this field, in the field's own unit, or why
    /// it gives none.
    fn evaluate(&self, text: &str) -> Result<f64, String>;
    /// Whether `text` reads nothing but numbers: it becomes the value
    /// rather than a formula.
    fn is_constant(&self, text: &str) -> bool;
    /// The names a formula can read, as it writes them
    /// (`Printer.nozzle`), each with what it is, for completion.
    fn candidates(&self) -> Vec<crate::completion::Candidate>;
}

/// What a [`FormulaField`] was set to this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum FormulaEdit {
    /// A number: dragged, typed, or a formula taken away.
    Value(f64),
    /// A formula that reads something.
    Formula(String),
}

/// A number that a formula may set. Without one it drags and takes typed
/// quantities (`1 in`, `2 * 3 mm`); the `fx` beside it, or a click on a
/// bound field, opens the formula, where Enter keeps it, Escape leaves it,
/// Tab takes the first suggested name, and an empty formula leaves the
/// number as it stands.
pub struct FormulaField<'a> {
    id: egui::Id,
    value: f64,
    formula: Option<&'a str>,
    error: Option<&'a str>,
    unit: &'a str,
    speed: f64,
    decimals: usize,
    width: f32,
    host: &'a dyn FormulaHost,
}

impl<'a> FormulaField<'a> {
    pub fn new(id: egui::Id, value: f64, host: &'a dyn FormulaHost) -> Self {
        Self {
            id: id.with("formula_field"),
            value,
            formula: None,
            error: None,
            unit: "",
            speed: 0.1,
            decimals: 2,
            width: 140.0,
            host,
        }
    }

    pub fn formula(mut self, formula: Option<&'a str>) -> Self {
        self.formula = formula;
        self
    }

    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Option<FormulaEdit> {
        let editing: Option<String> = ui.data(|d| d.get_temp(self.id));
        match editing {
            Some(text) => self.show_editing(ui, text),
            None => self.show_value(ui),
        }
    }

    fn start_editing(&self, ui: &Ui, text: String) {
        ui.data_mut(|d| d.insert_temp(self.id, text));
        ui.memory_mut(|m| m.request_focus(self.id.with("text")));
    }

    fn stop_editing(&self, ui: &Ui) {
        ui.data_mut(|d| d.remove::<String>(self.id));
    }

    fn frame(&self, border: Color32) -> Frame {
        Frame::new()
            .fill(BG2)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(SPACE_2 as i8, 0))
    }

    fn show_value(self, ui: &mut Ui) -> Option<FormulaEdit> {
        let mut out = None;
        let border = match (self.error, self.formula) {
            (Some(_), _) => DANGER,
            (None, Some(_)) => ACCENT,
            (None, None) => BORDER,
        };
        let suffix = if self.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.unit)
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_1;
            let field = self.frame(border).show(ui, |ui| {
                ui.set_min_size(Vec2::new(self.width, INPUT - 2.0));
                ui.set_max_width(self.width);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_1;
                    ui.style_mut().override_font_id = Some(mono(FONT_SM));
                    match self.formula {
                        Some(_) => {
                            icon::draw(ui, "expression", 12.0, ACCENT);
                            let text = format!("{:.*}{suffix}", self.decimals, self.value);
                            ui.label(
                                RichText::new(text)
                                    .font(mono(FONT_SM))
                                    .color(if self.error.is_some() { DANGER } else { TEXT1 }),
                            );
                        }
                        None => {
                            let visuals = ui.visuals_mut();
                            for w in [
                                &mut visuals.widgets.inactive,
                                &mut visuals.widgets.hovered,
                                &mut visuals.widgets.active,
                            ] {
                                w.bg_fill = Color32::TRANSPARENT;
                                w.weak_bg_fill = Color32::TRANSPARENT;
                                w.bg_stroke = Stroke::NONE;
                            }
                            visuals.override_text_color = Some(TEXT1);
                            let mut v = self.value;
                            let host = self.host;
                            let resp = ui.add(
                                egui::DragValue::new(&mut v)
                                    .speed(self.speed)
                                    .fixed_decimals(self.decimals)
                                    .suffix(suffix.clone())
                                    .custom_parser(move |text| {
                                        host.is_constant(text)
                                            .then(|| host.evaluate(text).ok())
                                            .flatten()
                                    }),
                            );
                            if resp.changed() {
                                out = Some(FormulaEdit::Value(v));
                            }
                        }
                    }
                });
            });
            let bound_click = self.formula.is_some()
                && ui
                    .interact(field.response.rect, self.id.with("open"), Sense::click())
                    .clicked();
            let button = match icon::image(ui.ctx(), "expression", 12.0, TEXT2) {
                Some(image) => egui::Button::image(image),
                None => egui::Button::new(RichText::new("fx").font(mono(FONT_XS))),
            };
            let fx = ui.add(button).on_hover_text(match self.formula {
                Some(f) => format!("Formula: {f}"),
                None => "Set by a formula".to_string(),
            });
            if bound_click || fx.clicked() {
                let text = self
                    .formula
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{:.*}", self.decimals, self.value));
                self.start_editing(ui, text);
            }
            if let Some(err) = self.error {
                field.response.on_hover_text(err);
            }
        });
        out
    }

    fn show_editing(self, ui: &mut Ui, mut text: String) -> Option<FormulaEdit> {
        let text_id = self.id.with("text");
        let host = self.host;
        let mut out = None;
        let mut done = false;
        ui.vertical(|ui| {
            let mut preview = host.evaluate(&text);
            self.frame(if preview.is_err() { DANGER } else { ACCENT })
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(self.width, INPUT - 2.0));
                    ui.horizontal_centered(|ui| {
                        icon::draw(ui, "expression", 12.0, ACCENT);
                        let edit = crate::completion::completing_text_edit(
                            ui,
                            text_id,
                            &mut text,
                            &|| host.candidates(),
                            |edit| {
                                edit.frame(Frame::NONE)
                                    .font(mono(FONT_SM))
                                    .desired_width(self.width * 1.6)
                            },
                        );
                        let resp = edit.response;
                        // A name picked from the dropdown is not the end
                        // of the formula: the edit gets the keyboard back.
                        if resp.lost_focus() && !edit.picked {
                            let (enter, escape) = ui.input(|i| {
                                (
                                    i.key_pressed(egui::Key::Enter),
                                    i.key_pressed(egui::Key::Escape),
                                )
                            });
                            if escape {
                                done = true;
                            } else if enter || !resp.has_focus() {
                                let typed = text.trim();
                                if typed.is_empty() {
                                    if self.formula.is_some() {
                                        out = Some(FormulaEdit::Value(self.value));
                                    }
                                    done = true;
                                } else if host.is_constant(typed) {
                                    if let Ok(v) = host.evaluate(typed) {
                                        out = Some(FormulaEdit::Value(v));
                                        done = true;
                                    }
                                } else {
                                    out = Some(FormulaEdit::Formula(typed.to_string()));
                                    done = true;
                                }
                            }
                        }
                    });
                });
            preview = host.evaluate(&text);
            let (line, color) = match &preview {
                Ok(v) => (format!("= {v:.*} {}", self.decimals, self.unit), TEXT3),
                Err(why) => (why.clone(), DANGER),
            };
            ui.label(RichText::new(line).font(sans(FONT_XS)).color(color));
        });
        if done {
            self.stop_editing(ui);
        } else {
            ui.data_mut(|d| d.insert_temp(self.id, text));
        }
        out
    }
}

#[cfg(test)]
mod formula_field_tests {
    use super::*;

    /// A length field in a document with one variable, `Printer.x` = 2.
    struct Host;

    impl FormulaHost for Host {
        fn evaluate(&self, text: &str) -> Result<f64, String> {
            match text.trim() {
                "Printer.x * 2" => Ok(4.0),
                "1 in" => Ok(25.4),
                other => other.parse().map_err(|_| format!("cannot read {other}")),
            }
        }
        fn is_constant(&self, text: &str) -> bool {
            !text.contains('.') || text.trim().parse::<f64>().is_ok()
        }
        fn candidates(&self) -> Vec<crate::completion::Candidate> {
            ["Printer.x", "Printer.y"]
                .iter()
                .map(|t| crate::completion::Candidate {
                    text: t.to_string(),
                    detail: String::new(),
                })
                .collect()
        }
    }

    /// Open the field's formula with `typed` in it, then press `key`: what
    /// the field answers.
    fn type_and_press(formula: Option<&str>, typed: &str, key: egui::Key) -> Option<FormulaEdit> {
        let ctx = egui::Context::default();
        let id = egui::Id::new("f");
        let field_id = id.with("formula_field");
        ctx.data_mut(|d| d.insert_temp(field_id, typed.to_string()));
        ctx.memory_mut(|m| m.request_focus(field_id.with("text")));
        let mut answer = None;
        for events in [
            Vec::new(),
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                if let Some(edit) = FormulaField::new(id, 10.0, &Host)
                    .formula(formula)
                    .unit("mm")
                    .show(ui)
                {
                    answer = Some(edit);
                }
            });
            output.textures_delta.clear();
        }
        answer
    }

    #[test]
    fn a_formula_is_kept_a_quantity_becomes_the_value_and_escape_leaves_it() {
        assert_eq!(
            type_and_press(None, "Printer.x * 2", egui::Key::Enter),
            Some(FormulaEdit::Formula("Printer.x * 2".into()))
        );
        assert_eq!(
            type_and_press(None, "1 in", egui::Key::Enter),
            Some(FormulaEdit::Value(25.4))
        );
        assert_eq!(
            type_and_press(Some("Printer.x * 2"), "", egui::Key::Enter),
            Some(FormulaEdit::Value(10.0)),
            "an empty formula leaves the number"
        );
        assert_eq!(
            type_and_press(None, "Printer.x * 2", egui::Key::Escape),
            None
        );
    }
}
