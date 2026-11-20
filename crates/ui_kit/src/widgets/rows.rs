//! List rows, the filter field over them, and the label column that
//! lines up a form's labels and fields.

use super::*;

/// A panel's filter: a full-width field with a search mark before the
/// text, `trailing` drawn at its right end (a kind choice, say). Whether
/// the text changed.
pub fn filter_field(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    trailing: impl FnOnce(&mut Ui),
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(BG2)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                icon::draw(ui, "search", 13.0, TEXT3);
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    trailing(ui);
                    changed = ui
                        .add(
                            egui::TextEdit::singleline(text)
                                .frame(egui::Frame::NONE)
                                .desired_width(ui.available_width())
                                .hint_text(RichText::new(hint).color(TEXT3))
                                .font(sans(FONT_SM)),
                        )
                        .changed();
                });
            });
        });
    changed
}

/// How a [`list_row_with`] row reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowLook {
    /// Picked: the accent tint and a bar at its left edge.
    pub selected: bool,
    /// A problem: its name in the danger colour.
    pub problem: bool,
    /// Out of play (inactive, a reference): its name dim.
    pub dim: bool,
    /// Tied to the item in hand (hovered or selected elsewhere): a faint
    /// [`RELATED`] tint and a thin bar at its left edge.
    pub related: bool,
    /// The item in hand itself, hovered through something else (a chip,
    /// the view): lit as under the pointer, with the related bar.
    pub lit: bool,
}

/// A row's dim detail after its name, cut to the room, for a
/// [`list_row_with`] whose detail is words.
pub fn row_detail(ui: &mut Ui, detail: &str) {
    if !detail.is_empty() {
        ui.add(
            egui::Label::new(RichText::new(detail).font(sans(FONT_DETAIL)).color(TEXT3)).truncate(),
        );
    }
}

/// One row of a panel's list: `icon` in `icon_color`, the name, then what
/// `detail` draws left to right in the room before `trailing` (dim words
/// through [`row_detail`], or [`chips`]), and `trailing` at the right end,
/// a value or a field. The whole row is clickable; widgets inside keep
/// their own interactions. The row is lit while the pointer is on it.
pub fn list_row_with(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    look: RowLook,
    (icon_name, icon_color): (&str, Color32),
    name: &str,
    detail: impl FnOnce(&mut Ui),
    trailing: impl FnOnce(&mut Ui),
) -> Response {
    let size = Vec2::new(ui.available_width(), LIST_ROW);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            look.selected,
            name,
        )
    });
    let lit = look.lit || ui.rect_contains_pointer(rect);
    let fill = if look.selected {
        ACCENT_DIM
    } else if lit {
        BG3
    } else if look.related {
        RELATED_FAINT
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, RADIUS_SM, fill);
    let bar = if look.selected {
        Some((ACCENT, 2.0))
    } else if look.related || look.lit {
        Some((RELATED, 1.5))
    } else {
        None
    };
    if let Some((color, width)) = bar {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(width, rect.height())),
            0.0,
            color,
        );
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(SPACE_2, 0.0)))
            .layout(Layout::left_to_right(egui::Align::Center))
            .id_salt(id_salt),
    );
    child.spacing_mut().item_spacing.x = SPACE_2;
    icon::draw(&mut child, icon_name, 14.0, icon_color);
    let name_color = if look.problem {
        DANGER
    } else if look.dim {
        TEXT3
    } else {
        TEXT1
    };
    child.add(egui::Label::new(RichText::new(name).font(sans(FONT_SM)).color(name_color)).extend());
    child.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
        trailing(ui);
        ui.with_layout(Layout::left_to_right(egui::Align::Center), detail);
    });
    response
}

/// A row's plain value at its right end, in the mono face.
pub fn row_value(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.label(RichText::new(text).font(mono(FONT_SM)).color(color))
}

/// The label column of a two-column parameter grid.
pub fn field_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT2))
}

/// The label column the fields of one section share: every label left in
/// a column as wide as the section's longest label, between
/// [`LabelColumn::MIN`] and [`LabelColumn::MAX`] (and never more than
/// [`LabelColumn::SHARE`] of the room), so each control starts at the same
/// x. A label past the column is cut, and whole on hover.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelColumn {
    /// `None` for a field with its label at its own width.
    width: Option<f32>,
}

impl LabelColumn {
    /// The narrowest column, so sections of short labels still line up
    /// with each other.
    pub const MIN: f32 = 96.0;
    /// The widest column, past which labels are cut.
    pub const MAX: f32 = 160.0;
    /// The largest part of the room the column takes, leaving the rest to
    /// the controls.
    pub const SHARE: f32 = 0.45;
    /// The gap between the column and the control.
    pub const GAP: f32 = SPACE_2;

    /// The column for a section with these labels, in `ui`'s room.
    pub fn fitting<'a>(ui: &Ui, labels: impl IntoIterator<Item = &'a str>) -> Self {
        let longest = labels
            .into_iter()
            .map(|label| {
                ui.painter()
                    .layout_no_wrap(label.to_owned(), sans(FONT_SM), TEXT2)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        let cap = Self::MAX.min(ui.available_width() * Self::SHARE);
        Self {
            width: Some(longest.ceil().clamp(Self::MIN.min(cap), cap)),
        }
    }

    /// The column for a compact card's fields: as wide as its widest
    /// label, so the controls stand right after the labels.
    pub fn snug<'a>(ui: &Ui, labels: impl IntoIterator<Item = &'a str>) -> Self {
        let longest = labels
            .into_iter()
            .map(|label| {
                ui.painter()
                    .layout_no_wrap(label.to_owned(), sans(FONT_SM), TEXT2)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        Self {
            width: Some(longest.ceil().min(Self::MAX)),
        }
    }

    /// A column `width` wide, whatever the labels.
    pub fn fixed(width: f32) -> Self {
        Self { width: Some(width) }
    }

    /// No column: a compact field's label at its own width, the control
    /// right after it.
    pub fn inline() -> Self {
        Self { width: None }
    }

    /// The column's width, the gap before the control not counted; `None`
    /// for [`Self::inline`].
    pub fn width(&self) -> Option<f32> {
        self.width
    }

    /// One labelled field: `label` in the column, then `control` from the
    /// column's end plus [`Self::GAP`].
    pub fn row<R>(&self, ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = Self::GAP;
            self.cell(ui, label);
            control(ui)
        })
        .inner
    }

    /// `label` as the column holds it, where the cursor stands: cut to
    /// the column, whole on hover; at its own width with no column.
    pub fn cell(&self, ui: &mut Ui, label: &str) {
        let Some(width) = self.width else {
            ui.label(RichText::new(label).font(sans(FONT_SM)).color(TEXT2));
            return;
        };
        // Laid out in a child of exactly the column's size, so the
        // cursor ends at its edge however long the label is.
        ui.allocate_ui_with_layout(
            Vec2::new(width, INPUT),
            Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(Vec2::new(width, INPUT));
                ui.set_max_width(width);
                let galley = ui
                    .painter()
                    .layout_no_wrap(label.to_owned(), sans(FONT_SM), TEXT2);
                let cut = galley.size().x > width;
                let response = ui.add(
                    egui::Label::new(RichText::new(label).font(sans(FONT_SM)).color(TEXT2))
                        .truncate(),
                );
                if cut {
                    response.on_hover_text(label);
                }
            },
        );
    }
}
