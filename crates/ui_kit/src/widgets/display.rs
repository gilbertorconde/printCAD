//! Surfaces and marks that show rather than take input: cards, overlines,
//! labels, badges, key chips, separators, note cards, status pills and
//! the planned-feature mark.

use super::*;

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
                    .font(sans_semibold(FONT_XXS))
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
            ui.label(RichText::new(text).font(mono(FONT_XXS)).color(TEXT2))
        })
        .response
}

/// A 1px vertical rule of `height` px.
pub fn vseparator(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
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
                    ui.label(RichText::new(body).font(sans(FONT_DETAIL)).color(TEXT2));
                });
            });
        })
        .response
}

/// A small rounded status mark: a dot and a few words in the note's
/// colour on its faint tint, the longer reason on hover.
pub fn status_pill(ui: &mut Ui, note: Note, text: &str, detail: &str) -> Response {
    let color = note.color();
    let response = Frame::new()
        .fill(with_alpha(color, 0.10))
        .stroke(Stroke::new(1.0, with_alpha(color, 0.35)))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| {
            // Drawn into one rect of the content's own size, so the dot
            // leads the text whatever the parent's direction and the pill
            // never stretches to the room around it.
            let gap = SPACE_1 + 2.0;
            let galley = ui
                .painter()
                .layout_no_wrap(text.to_owned(), sans(FONT_XS), color);
            let size = Vec2::new(6.0 + gap + galley.size().x, galley.size().y.max(6.0));
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter()
                .circle_filled(egui::pos2(rect.left() + 3.0, rect.center().y), 3.0, color);
            let text_at = egui::pos2(
                rect.left() + 6.0 + gap,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(text_at, galley, color);
        })
        .response;
    if detail.is_empty() {
        response
    } else {
        response.on_hover_text(detail)
    }
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
