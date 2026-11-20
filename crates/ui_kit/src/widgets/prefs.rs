//! Preference rows and groups: one setting per row, its hint and control,
//! and the search that filters them.

use super::*;

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
        Self::new(label, move |ui| toggle(ui, on, label).changed())
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
        pref_matches(filter, &[self.label, self.hint.unwrap_or_default()])
    }
}

/// Whether a Preferences search for `filter` (lowercase) finds one of
/// `words`: an empty search finds everything, and no words are found by
/// none. Every row, group and block of a Preferences page is filtered by
/// this one rule.
pub fn pref_matches(filter: &str, words: &[&str]) -> bool {
    filter.is_empty() || words.iter().any(|w| w.to_lowercase().contains(filter))
}

/// A block of a Preferences page that is no group of rows (a note, a
/// picker laid out its own way, a free text field), found by `words`:
/// `add` draws it only when [`pref_matches`] finds them, so a search
/// leaves it out as it leaves out a group with no matching rows. A note
/// that only explains its page passes no words and shows with no search.
/// Returns whether it drew.
pub fn pref_block(ui: &mut Ui, words: &[&str], filter: &str, add: impl FnOnce(&mut Ui)) -> bool {
    if !pref_matches(filter, words) {
        return false;
    }
    add(ui);
    true
}

/// The width a preference row's control is taken to need before it has
/// been drawn once.
const PREF_CONTROL_GUESS: f32 = 120.0;

/// A preference row's height before its words are measured.
const PREF_ROW_TALL: f32 = 44.0;

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
                // The words wrap in the width the control leaves, as wide as
                // it was last drawn, and the row grows to hold them.
                let control_id = ui.id().with(("pref_row_control", i, label));
                let control_width = ui
                    .data(|d| d.get_temp::<f32>(control_id))
                    .unwrap_or(PREF_CONTROL_GUESS);
                let cursor = ui.cursor();
                let top = cursor.top();
                let mut inner = egui::Rect::from_min_size(
                    cursor.min,
                    Vec2::new(ui.available_width(), PREF_ROW_TALL),
                )
                .shrink2(Vec2::new(14.0, 0.0));
                let icon_size = 20.0;
                let text_left = if icon.is_some() {
                    inner.left() + icon_size + SPACE_3
                } else {
                    inner.left()
                };
                let text_top = if hint.is_some() { 7.0 } else { 12.0 };
                let text_right = (inner.right() - control_width - SPACE_3).max(text_left + 80.0);
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_x_y_ranges(
                            text_left..=text_right,
                            top + text_top..=f32::INFINITY,
                        ))
                        .layout(Layout::top_down(egui::Align::Min)),
                );
                left.spacing_mut().item_spacing.y = 2.0;
                left.add(egui::Label::new(
                    RichText::new(label).font(sans(FONT_SM)).color(TEXT1),
                ));
                if let Some(hint) = hint {
                    left.add(
                        egui::Label::new(RichText::new(hint).font(sans(FONT_XS)).color(TEXT3))
                            .wrap(),
                    );
                }
                let least = if hint.is_some() { 44.0 } else { 40.0 };
                let height = (left.min_rect().height() + 2.0 * text_top).max(least);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
                inner.set_top(rect.top());
                inner.set_bottom(rect.bottom());
                if let Some(icon) = icon {
                    let at = egui::Rect::from_center_size(
                        egui::pos2(inner.left() + icon_size / 2.0, inner.center().y),
                        Vec2::splat(icon_size),
                    );
                    let mut slot = ui.new_child(egui::UiBuilder::new().max_rect(at));
                    crate::icon::draw(&mut slot, icon, icon_size, TEXT2);
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
                let drawn = right.min_rect().width();
                if (drawn - control_width).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(control_id, drawn));
                    ui.ctx().request_repaint();
                }
                if i + 1 < count {
                    ui.painter()
                        .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, BORDER));
                }
            }
        });
    ui.add_space(SPACE_4);
    changed
}
