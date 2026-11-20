//! Menus and select fields: the menu fitted to its entries and the
//! select control with its options.

use super::*;

/// A menu's rows, scrolling when they would run past the bottom of the
/// window. Wraps what a menu or context menu shows. A popup measures
/// itself in a box of egui's default area size, so the room down to the
/// window's bottom is given to the rows here, or every menu would stop at
/// that box's height.
pub fn fitted_menu<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let room = (ui.ctx().content_rect().bottom() - ui.cursor().top() - SPACE_3).max(MENU_BAR * 3.0);
    ui.set_max_height(room);
    egui::ScrollArea::vertical()
        .max_height(room)
        .show(ui, add)
        .inner
}

/// What a [`select_control`] offers.
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectOptions<'a> {
    pub options: &'a [String],
    pub selected: usize,
    /// Shown in the closed field in place of the option chosen.
    pub text: Option<&'a str>,
    /// The options that cannot be chosen now, by their place.
    pub disabled: &'a [usize],
    /// Words on hover over each option, by its place; empty for none.
    pub hints: &'a [String],
    /// The options with a line above them.
    pub separated: &'a [usize],
    /// The options side by side, the chosen one lit, in place of a
    /// dropdown.
    pub segmented: bool,
    /// A field atop the open dropdown narrowing its options to those
    /// matching what is typed, the best first; Enter picks the first.
    pub search: bool,
}

/// A choice among `select.options`, `width` wide as a dropdown, or side
/// by side when `segmented`; the place of an option picked other than the
/// one chosen.
pub fn select_control(
    ui: &mut Ui,
    key: egui::Id,
    select: SelectOptions<'_>,
    width: f32,
) -> Option<usize> {
    let mut picked = None;
    let mut option = |ui: &mut Ui, i: usize, text: RichText| {
        let on = i == select.selected;
        let response = ui.add_enabled(
            !select.disabled.contains(&i),
            egui::Button::selectable(on, text),
        );
        let response = match select.hints.get(i).filter(|h| !h.is_empty()) {
            Some(hint) => response.on_hover_text(hint).on_disabled_hover_text(hint),
            None => response,
        };
        if response.clicked() && !on {
            picked = Some(i);
        }
    };
    if select.segmented {
        for (i, text) in select.options.iter().enumerate() {
            option(ui, i, RichText::new(text));
        }
        return picked;
    }
    let shown = select
        .text
        .or_else(|| select.options.get(select.selected).map(String::as_str))
        .unwrap_or("-");
    let query_key = key.with("search");
    let mut entered = None;
    let open = egui::ComboBox::from_id_salt(key)
        .selected_text(RichText::new(shown).font(sans(FONT_SM)))
        .width(width)
        .height(if select.search { 320.0 } else { 200.0 })
        .show_ui(ui, |ui| {
            if !select.search {
                for (i, text) in select.options.iter().enumerate() {
                    if select.separated.contains(&i) {
                        ui.separator();
                    }
                    option(ui, i, RichText::new(text).font(sans(FONT_SM)));
                }
                return;
            }
            let mut query: String = ui.data(|d| d.get_temp(query_key)).unwrap_or_default();
            let field = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .hint_text("Search")
                    .font(sans(FONT_SM))
                    .desired_width(f32::INFINITY),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let shown = crate::search::ranked(select.options, &query);
            if field.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(&first) = shown.iter().find(|i| !select.disabled.contains(i))
            {
                if first != select.selected {
                    entered = Some(first);
                }
                ui.close();
            }
            ui.data_mut(|d| d.insert_temp(query_key, query));
            if shown.is_empty() {
                ui.label(text("No match", FONT_SM, ui.visuals().weak_text_color()));
            }
            for i in shown {
                option(ui, i, RichText::new(&select.options[i]).font(sans(FONT_SM)));
            }
        });
    // The search starts empty each time the dropdown opens.
    if open.inner.is_none() {
        ui.data_mut(|d| d.remove::<String>(query_key));
    }
    picked.or(entered)
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

#[cfg(test)]
mod fitted_menu_tests {
    /// How tall a menu of fifty 20 px rows opening near the top of a
    /// window `window` px tall comes out, after the frames a popup takes
    /// to size itself.
    fn menu_height(window: f32) -> f32 {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        for time in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, window),
                )),
                time: Some(f64::from(time)),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                height = egui::Area::new(egui::Id::new("menu"))
                    .fixed_pos(egui::pos2(0.0, 20.0))
                    .show(ui.ctx(), |ui| {
                        super::fitted_menu(ui, |ui| {
                            for _ in 0..50 {
                                ui.allocate_exact_size(
                                    egui::vec2(100.0, 20.0),
                                    egui::Sense::hover(),
                                );
                            }
                        })
                    })
                    .response
                    .rect
                    .height();
            });
            out.textures_delta.clear();
        }
        height
    }

    #[test]
    fn a_menu_scrolls_only_past_the_bottom_of_the_window() {
        let tall = menu_height(1900.0);
        assert!(tall >= 1000.0, "a 1000 px menu in a tall window: {tall}");
        let short = menu_height(600.0);
        assert!(short <= 600.0, "a 1000 px menu in a short window: {short}");
    }
}
