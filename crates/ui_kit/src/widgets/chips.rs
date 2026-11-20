//! Chips: a row of them, the pick chip that arms a selection, and badges
//! wrapped to the width.

use super::*;

/// How a [`chip`] reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChipLook {
    /// What it names is selected: accent text on the accent tint.
    pub selected: bool,
    /// What it names is in hand or tied to it: the [`RELATED`] outline.
    pub lit: bool,
    /// A problem it names: its text in the note's colour on that colour's
    /// faint tint.
    pub tone: Option<Note>,
}

/// How wide a [`chip`] of `text` draws.
pub fn chip_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), sans(FONT_XS), TEXT2);
    galley.size().x + 2.0 * PILL_PAD[0]
}

/// A small rounded chip naming one item (an element a row ties), which
/// lights under the pointer and takes clicks.
pub fn chip(ui: &mut Ui, text: &str, look: ChipLook) -> Response {
    let size = Vec2::new(chip_width(ui, text), CHIP);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            look.selected,
            text,
        )
    });
    let hovered = response.hovered();
    let (fill, color) = if let Some(note) = look.tone {
        let color = note.color();
        (with_alpha(color, if hovered { 0.22 } else { 0.12 }), color)
    } else if look.selected {
        (ACCENT_DIM, ACCENT)
    } else if hovered {
        (BG4, TEXT1)
    } else {
        (BG3, TEXT2)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, RADIUS_SM, fill);
    if look.lit || hovered {
        painter.rect_stroke(
            rect,
            RADIUS_SM,
            egui::Stroke::new(1.0, RELATED),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        sans(FONT_XS),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A run of [`chip`]s, left to right, as many as fit the room left; the
/// rest are counted in a dim "+N" that names them on hover. The answer
/// holds the drawn chips' responses, in order.
pub fn chips(ui: &mut Ui, items: &[(&str, ChipLook)]) -> Vec<Response> {
    let gap = SPACE_1;
    let room = ui.available_width();
    let widths: Vec<f32> = items.iter().map(|(t, _)| chip_width(ui, t)).collect();
    let more_width = |n: usize| {
        ui.painter()
            .layout_no_wrap(format!("+{n}"), sans(FONT_XS), TEXT3)
            .size()
            .x
            + gap
    };
    let mut shown = 0;
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let left = items.len() - i - 1;
        let reserve = if left > 0 { more_width(left) } else { 0.0 };
        if used + w + reserve > room {
            break;
        }
        used += w + gap;
        shown += 1;
    }
    let mut out = Vec::with_capacity(shown);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (text, look) in &items[..shown] {
            out.push(chip(ui, text, *look));
        }
        if shown < items.len() {
            let rest: Vec<&str> = items[shown..].iter().map(|(t, _)| *t).collect();
            ui.label(
                RichText::new(format!("+{}", items.len() - shown))
                    .font(sans(FONT_XS))
                    .color(TEXT3),
            )
            .on_hover_text(rest.join(", "));
        }
    });
    out
}

/// What a [`pick_chip`] reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PickChipOutcome {
    /// The chip clicked: arm the pick.
    pub arm: bool,
    /// Its cross clicked: clear what is picked.
    pub clear: bool,
}

/// What is picked in the view as a compact chip: `value`, or `placeholder`
/// dim while nothing is, outlined in the accent while `armed`; a cross
/// at its end clears a value.
pub fn pick_chip(
    ui: &mut Ui,
    value: Option<&str>,
    placeholder: &str,
    armed: bool,
) -> PickChipOutcome {
    let text = value.unwrap_or(placeholder);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), sans(FONT_XS), TEXT2);
    let cross = if value.is_some() { CHIP } else { 0.0 };
    let size = Vec2::new(galley.size().x + 2.0 * PILL_PAD[0] + cross, CHIP);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let cross_rect =
        egui::Rect::from_min_max(egui::pos2(rect.right() - cross, rect.top()), rect.max);
    let cross_response = (value.is_some())
        .then(|| ui.interact(cross_rect, response.id.with("clear"), Sense::click()));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    if let Some(cross) = &cross_response {
        cross.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Clear"));
    }
    let hovered = response.hovered();
    let (fill, color) = if armed {
        (ACCENT_DIM, ACCENT)
    } else if value.is_none() {
        (Color32::TRANSPARENT, TEXT3)
    } else if hovered {
        (BG4, TEXT1)
    } else {
        (BG3, TEXT1)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, RADIUS_SM, fill);
    let outline = if armed {
        Some(ACCENT)
    } else if value.is_none() || hovered {
        Some(BORDER_STRONG)
    } else {
        None
    };
    if let Some(outline) = outline {
        painter.rect_stroke(
            rect,
            RADIUS_SM,
            egui::Stroke::new(1.0, outline),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        egui::pos2(rect.left() + PILL_PAD[0], rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        sans(FONT_XS),
        color,
    );
    let mut clear = false;
    if let Some(cross_response) = cross_response {
        let tint = if cross_response.hovered() {
            TEXT1
        } else {
            TEXT3
        };
        if let Some(image) = icon::image(ui.ctx(), "close", 10.0, tint) {
            image.paint_at(
                ui,
                egui::Rect::from_center_size(cross_rect.center(), Vec2::splat(10.0)),
            );
        }
        clear = cross_response.on_hover_text("Clear").clicked();
    }
    PickChipOutcome {
        arm: !clear
            && response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked(),
        clear,
    }
}

/// A row of glyph badges (`icon` in its colour, `lit` outlined in
/// [`RELATED`]), left to right, at most `most` and no more than fit the
/// room left, the rest counted in a dim "+N". Placed at the layout's
/// cursor, so a right-to-left row puts it before what came before. The
/// answer holds the drawn badges' responses, in order.
pub fn badges(ui: &mut Ui, items: &[(&str, Color32, bool)], most: usize) -> Vec<Response> {
    if items.is_empty() {
        return Vec::new();
    }
    let gap = 2.0;
    let more = |n: usize| format!("+{n}");
    let more_width = |n: usize| {
        ui.painter()
            .layout_no_wrap(more(n), sans(FONT_XS), TEXT3)
            .size()
            .x
            + gap
    };
    let room = ui.available_width() - SPACE_2;
    let mut shown = items.len().min(most);
    let width = |shown: usize| {
        let rest = items.len() - shown;
        shown as f32 * (CHIP + gap) + if rest > 0 { more_width(rest) } else { 0.0 }
    };
    while shown > 0 && width(shown) > room {
        shown -= 1;
    }
    if shown == 0 && width(0) > room {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(shown);
    ui.allocate_ui_with_layout(
        Vec2::new(width(shown), CHIP),
        Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (icon_name, color, lit) in &items[..shown] {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(CHIP), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(rect, RADIUS_SM, BG4);
                }
                if *lit || response.hovered() {
                    ui.painter().rect_stroke(
                        rect,
                        RADIUS_SM,
                        egui::Stroke::new(1.0, RELATED),
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(image) = icon::image(ui.ctx(), icon_name, 12.0, *color) {
                    image.paint_at(
                        ui,
                        egui::Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
                    );
                }
                out.push(response.on_hover_cursor(egui::CursorIcon::PointingHand));
            }
            if shown < items.len() {
                ui.label(
                    RichText::new(more(items.len() - shown))
                        .font(sans(FONT_XS))
                        .color(TEXT3),
                );
            }
        },
    );
    out
}
