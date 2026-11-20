//! The Keyboard page: every shortcut, grouped by menu and then by
//! workbench, with its keys and the buttons that change them.

use super::*;

/// Every shortcut, grouped by menu and then by workbench: its keys, and
/// buttons to record a new key, add one, clear them or go back to the
/// default. A key another command also answers to is called out under the
/// row.
pub(super) fn keyboard_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    registry: &DocumentService,
    scripts: &[crate::script_library::ScriptEntry],
    filter: &str,
) {
    use super::super::keymap::{Keymap, set_keys};
    let keymap = Keymap::build(registry, &state.draft.keyboard, scripts);
    let key_text = |keys: &[core_document::Chord]| {
        keys.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    pref_block(ui, &[], filter, |ui| {
        ui.label(
            RichText::new(
                "A workbench's keys work while it is active, and win over the \
                 application's there. Keys without Ctrl or Alt are left to text fields.",
            )
            .font(sans(FONT_SM))
            .color(TEXT3),
        );
        ui.add_space(SPACE_2);
    });
    let mut groups: Vec<&str> = Vec::new();
    for binding in keymap.bindings() {
        if !groups.contains(&binding.group.as_str()) {
            groups.push(&binding.group);
        }
    }
    let mut change: Option<(super::super::keymap::Binding, Vec<core_document::Chord>)> = None;
    for group in groups {
        let rows: Vec<_> = keymap
            .bindings()
            .iter()
            .filter(|b| b.group == group)
            .filter(|b| pref_matches(filter, &[&b.label, group, &key_text(&b.keys)]))
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
