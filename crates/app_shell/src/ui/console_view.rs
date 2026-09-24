//! The script console at the bottom of the window: a Lua line in, what it
//! printed and came to out. Up and Down walk back through the lines typed.

use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::small_secondary_button;
use ui_kit::{mono, sans, sans_semibold};

use super::UiCommand;
use crate::console::{self, LineKind};

/// The console's own state: whether it shows, the line being typed and
/// the lines typed before.
#[derive(Debug, Default)]
pub struct ConsoleState {
    pub open: bool,
    input: String,
    history: Vec<String>,
    /// How far back Up has gone; `None` while typing a new line.
    recalled: Option<usize>,
    /// Put the keyboard in the input line on the next draw.
    focus: bool,
}

impl ConsoleState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus = self.open;
    }

    /// Step through the lines typed: `back` toward the oldest.
    fn recall(&mut self, back: bool) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        self.recalled = match (self.recalled, back) {
            (None, true) => Some(last),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i < last => Some(i + 1),
            (Some(_), false) => None,
        };
        self.input = self
            .recalled
            .map(|i| self.history[i].clone())
            .unwrap_or_default();
    }
}

pub fn draw_console(ui: &mut egui::Ui, state: &mut ConsoleState, commands: &mut Vec<UiCommand>) {
    if !state.open {
        return;
    }
    egui::Panel::bottom("console_panel")
        .resizable(true)
        .default_size(200.0)
        .min_size(100.0)
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Console")
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                ui.add_space(SPACE_2);
                if small_secondary_button(ui, "Clear").clicked() {
                    console::clear();
                }
                ui.label(
                    RichText::new("Lua · help() lists the commands, pc.<id>{...} runs one")
                        .font(sans(FONT_XS))
                        .color(TEXT3),
                );
            });
            ui.add_space(SPACE_1);
            // The input line takes its own height from the bottom and the
            // output the rest, so the two never ask for more than the panel
            // has (a panel that is asked for more grows to fit).
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(">").font(mono(FONT_SM)).color(TEXT3));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut state.input)
                            .font(mono(FONT_SM))
                            .desired_width(f32::INFINITY)
                            .hint_text("pc.doc.bodies()"),
                    );
                    if std::mem::take(&mut state.focus) {
                        response.request_focus();
                    }
                    if response.has_focus() {
                        let (up, down) = ui.input_mut(|i| {
                            (
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                            )
                        });
                        if up || down {
                            state.recall(up);
                        }
                    }
                    let entered =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if entered {
                        let line = std::mem::take(&mut state.input);
                        if !line.trim().is_empty() {
                            if state.history.last() != Some(&line) {
                                state.history.push(line.clone());
                            }
                            commands.push(UiCommand::RunConsole(line));
                        }
                        state.recalled = None;
                        state.focus = true;
                    }
                });
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        // Oldest first, whatever the layout around.
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for line in console::entries() {
                                let (prefix, color) = match line.kind {
                                    LineKind::Input => ("> ", TEXT1),
                                    LineKind::Printed => ("", TEXT2),
                                    LineKind::Value => ("= ", INFO),
                                    LineKind::Error => ("! ", DANGER),
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("{prefix}{}", line.text))
                                            .font(mono(FONT_SM))
                                            .color(color),
                                    )
                                    .selectable(true),
                                );
                            }
                        });
                    });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The room the rest of the window keeps below the menu, frame after
    /// frame, with the console open and full.
    fn room_left_per_frame(frames: usize) -> Vec<f32> {
        for i in 0..40 {
            console::push(LineKind::Printed, format!("line {i}"));
        }
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let mut state = ConsoleState {
            open: true,
            ..Default::default()
        };
        let mut left = Vec::new();
        for _ in 0..frames {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                draw_console(ui, &mut state, &mut Vec::new());
                left.push(ui.available_rect_before_wrap().height());
            });
            output.textures_delta.clear();
        }
        left
    }

    #[test]
    fn the_panel_keeps_its_height() {
        let left = room_left_per_frame(30);
        let settled = left[2];
        assert!(
            settled > 500.0,
            "the console takes a strip, not the window: {left:?}"
        );
        assert!(
            left[2..].iter().all(|h| (h - settled).abs() < 0.5),
            "the panel does not grow: {left:?}"
        );
    }

    #[test]
    fn up_and_down_walk_the_lines_typed() {
        let mut state = ConsoleState {
            history: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        state.recall(true);
        assert_eq!(state.input, "b");
        state.recall(true);
        state.recall(true);
        assert_eq!(state.input, "a", "stops at the oldest");
        state.recall(false);
        assert_eq!(state.input, "b");
        state.recall(false);
        assert_eq!(state.input, "", "past the newest is a fresh line");
    }
}
