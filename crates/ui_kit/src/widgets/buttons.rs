//! Buttons and switches: toggles, check rows, the text button styles,
//! tool buttons and icon buttons.

use super::*;

/// A 34×18 switch. `label` names what it switches for assistive
/// technology; it is not drawn, since the row beside it shows it.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let size = Vec2::new(34.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let checked = *on;
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), checked, label)
    });
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
    let checked = *on;
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), checked, label)
    });
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

/// A button whose label stays on one line: its whole size is taken at
/// once, so in a wrapping row it moves to the next line rather than
/// squeezing its label into what is left of this one.
fn text_button(ui: &mut Ui, label: &str, style: ButtonStyle) -> Response {
    let galley = WidgetText::from(RichText::new(label).font(style.font).color(style.text))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        );
    let pad = Vec2::new(style.padding, ((style.height - 16.0) / 2.0).floor());
    // The border's width on each side, as a frame's stroke adds it.
    let size = galley.size() + 2.0 * pad + Vec2::splat(2.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            style.hover_fill
        } else {
            style.fill
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(5),
            fill,
            Stroke::new(1.0, style.border),
            StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, style.text);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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

/// The width a menu button's chevron strip adds at its right end.
pub const TOOL_MENU_STRIP: f32 = 12.0;

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
    let width = if state.menu {
        size + TOOL_MENU_STRIP
    } else {
        size
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, size), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
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
    let icon_center = egui::pos2(rect.left() + size / 2.0, rect.center().y);
    let icon_rect = egui::Rect::from_center_size(icon_center, Vec2::splat(icon_px));
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
        None => response.on_hover_text(label),
    };
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A frameless icon the size of a line of small text, lit on hover: for
/// actions that sit beside content (copy, jump) rather than in a toolbar.
pub fn icon_button(ui: &mut Ui, name: &str, hint: &str) -> Response {
    let size = 14.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size + 6.0), Sense::click());
    let label = if hint.is_empty() { name } else { hint };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(RADIUS_SM as u8), BG3);
        }
        let tint = if hovered { TEXT1 } else { TEXT3 };
        if let Some(image) = icon::image(ui.ctx(), name, size, tint) {
            image.paint_at(ui, rect.shrink(3.0));
        }
    }
    if hint.is_empty() {
        response
    } else {
        response.on_hover_text(hint)
    }
}
