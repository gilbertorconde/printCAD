//! The script console at the bottom of the window: Lua in, what it
//! printed and came to out. Enter runs what is typed and Shift+Enter starts
//! another line of it; Up and Down walk back through what was run before,
//! kept between sessions; Tab completes a command's name.

use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::small_secondary_button;
use ui_kit::{mono, sans, sans_semibold};

use super::UiCommand;
use crate::console::{self, LineKind};

/// How many runs the history keeps.
const HISTORY_KEPT: usize = 200;
const HISTORY_FILE: &str = "console_history.lua";
/// Runs in the history file are apart by this line, which Lua reads as a
/// comment.
const SEPARATOR: &str = "--~~";

/// The console's own state: whether it shows, what is being typed and what
/// was run before.
#[derive(Debug, Default)]
pub struct ConsoleState {
    pub open: bool,
    input: String,
    history: Vec<String>,
    /// How far back Up has gone; `None` while typing something new.
    recalled: Option<usize>,
    /// Put the keyboard in the input on the next draw.
    focus: bool,
    /// Where the history is kept; `None` keeps it for this session only.
    history_file: Option<std::path::PathBuf>,
}

impl ConsoleState {
    /// The console with the history of earlier sessions.
    pub fn load() -> Self {
        let history_file = settings::config_path(HISTORY_FILE);
        let history = history_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| parse_history(&text))
            .unwrap_or_default();
        Self {
            history,
            history_file,
            ..Default::default()
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus = self.open;
    }

    /// Step through what was run: `back` toward the oldest.
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

    /// Take what is typed to run, into the history.
    fn submit(&mut self) -> Option<String> {
        let text = std::mem::take(&mut self.input);
        self.recalled = None;
        if text.trim().is_empty() {
            return None;
        }
        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
            if self.history.len() > HISTORY_KEPT {
                let extra = self.history.len() - HISTORY_KEPT;
                self.history.drain(..extra);
            }
            if let Some(path) = &self.history_file {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(path, write_history(&self.history));
            }
        }
        Some(text)
    }
}

fn parse_history(text: &str) -> Vec<String> {
    text.split(&format!("\n{SEPARATOR}\n"))
        .map(|run| run.trim_end_matches('\n').to_string())
        .filter(|run| !run.trim().is_empty())
        .collect()
}

fn write_history(history: &[String]) -> String {
    history.join(&format!("\n{SEPARATOR}\n")) + "\n"
}

/// What Tab does to `input` with the cursor at its end: the command name
/// being typed (after `pc.`) completed as far as every match agrees, and
/// the matches when there is more than one.
fn complete(input: &str, ids: &[String]) -> (String, Vec<String>) {
    let start = input
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .map_or(0, |i| i + 1);
    let Some(typed) = input[start..].strip_prefix("pc.") else {
        return (input.to_string(), Vec::new());
    };
    let matches: Vec<&String> = ids.iter().filter(|id| id.starts_with(typed)).collect();
    let Some(first) = matches.first() else {
        return (input.to_string(), Vec::new());
    };
    let mut common = first.as_str();
    for m in &matches[1..] {
        let shared = common
            .char_indices()
            .zip(m.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8());
        common = &common[..shared];
    }
    let mut out = format!("{}pc.{common}", &input[..start]);
    if matches.len() == 1 {
        out.push('{');
        return (out, Vec::new());
    }
    (out, matches.into_iter().cloned().collect())
}

pub fn draw_console(
    ui: &mut egui::Ui,
    state: &mut ConsoleState,
    command_ids: &[String],
    commands: &mut Vec<UiCommand>,
) {
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
                    RichText::new(
                        "Lua · help() lists the commands · Tab completes · \
                         Shift+Enter for another line",
                    )
                    .font(sans(FONT_XS))
                    .color(TEXT3),
                );
            });
            ui.add_space(SPACE_1);
            // The input takes its own height from the bottom and the output
            // the rest, so the two never ask for more than the panel has (a
            // panel that is asked for more grows to fit).
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                draw_input(ui, state, command_ids, commands);
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

fn draw_input(
    ui: &mut egui::Ui,
    state: &mut ConsoleState,
    command_ids: &[String],
    commands: &mut Vec<UiCommand>,
) {
    let id = egui::Id::new("console_input");
    // The keys the input's own editing would take are read first, while it
    // has the keyboard: Enter runs (Shift+Enter still starts a line), Tab
    // completes, and Up and Down recall while the input is one line.
    if ui.memory(|m| m.has_focus(id)) {
        let one_line = !state.input.contains('\n');
        let (run, tab, up, down) = ui.input_mut(|i| {
            (
                take_bare(i, egui::Key::Enter),
                take_bare(i, egui::Key::Tab),
                one_line && take_bare(i, egui::Key::ArrowUp),
                one_line && take_bare(i, egui::Key::ArrowDown),
            )
        });
        if up || down {
            state.recall(up);
        }
        if tab {
            let (completed, choices) = complete(&state.input, command_ids);
            if !choices.is_empty() {
                console::push(LineKind::Printed, choices.join("  "));
            }
            if completed != state.input {
                state.input = completed;
                move_cursor_to_end(ui, id, &state.input);
            }
        }
        if run && let Some(text) = state.submit() {
            commands.push(UiCommand::RunConsole(text));
        }
        if up || down {
            move_cursor_to_end(ui, id, &state.input);
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(">").font(mono(FONT_SM)).color(TEXT3));
        let response = ui.add(
            egui::TextEdit::multiline(&mut state.input)
                .id(id)
                // Tab completes rather than moving the keyboard on.
                .lock_focus(true)
                .font(mono(FONT_SM))
                .desired_rows(1)
                .desired_width(f32::INFINITY)
                .hint_text("pc.doc.bodies()"),
        );
        if std::mem::take(&mut state.focus) {
            response.request_focus();
        }
    });
}

/// Take `key` out of the input when it was pressed with no modifier at
/// all: Shift+Enter stays for the text field, where egui's own matching
/// would take it as Enter.
fn take_bare(input: &mut egui::InputState, key: egui::Key) -> bool {
    let mut found = false;
    input.events.retain(|event| {
        let hit = matches!(
            event,
            egui::Event::Key { key: k, pressed: true, modifiers, .. }
                if *k == key && modifiers.is_none()
        );
        found |= hit;
        !hit
    });
    found
}

/// Put the input's cursor after its last character.
fn move_cursor_to_end(ui: &egui::Ui, id: egui::Id, text: &str) {
    if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), id) {
        let end = egui::text::CCursor::new(text.chars().count());
        edit.cursor
            .set_char_range(Some(egui::text::CCursorRange::one(end)));
        edit.store(ui.ctx(), id);
    }
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
                draw_console(ui, &mut state, &[], &mut Vec::new());
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
    fn tab_completes_a_command_name_as_far_as_the_matches_agree() {
        let ids: Vec<String> = ["part.pad", "part.pocket", "part.set", "sketch.new"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            complete("x = pc.ske", &ids),
            ("x = pc.sketch.new{".into(), vec![])
        );
        let (text, choices) = complete("pc.part.p", &ids);
        assert_eq!(text, "pc.part.p");
        assert_eq!(choices, ["part.pad", "part.pocket"]);
        assert_eq!(complete("pc.part.po", &ids).0, "pc.part.pocket{");
        assert_eq!(complete("print(1)", &ids).0, "print(1)", "only after pc.");
        assert_eq!(complete("pc.nothing", &ids).0, "pc.nothing");
    }

    #[test]
    fn the_history_keeps_several_line_runs_whole() {
        let runs = vec![
            "x = 1".to_string(),
            "for i = 1, 3 do\n  print(i)\nend".to_string(),
        ];
        assert_eq!(parse_history(&write_history(&runs)), runs);
        let mut state = ConsoleState {
            input: "  ".into(),
            ..Default::default()
        };
        assert_eq!(state.submit(), None, "blank runs nothing");
        state.input = "a = 2".into();
        assert_eq!(state.submit().as_deref(), Some("a = 2"));
        state.input = "a = 2".into();
        state.submit();
        assert_eq!(state.history, ["a = 2"], "a repeat is kept once");
    }

    /// Run frames of the console with these events each, and what it
    /// asked for.
    fn type_into(state: &mut ConsoleState, frames: Vec<Vec<egui::Event>>) -> Vec<UiCommand> {
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let ids = vec!["part.pad".to_string(), "sketch.new".to_string()];
        let mut out = Vec::new();
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| draw_console(ui, state, &ids, &mut out));
            output.textures_delta.clear();
        }
        out
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn enter_runs_shift_enter_adds_a_line_and_tab_completes() {
        let mut state = ConsoleState {
            open: true,
            focus: true,
            ..Default::default()
        };
        let text = |t: &str| egui::Event::Text(t.to_string());
        let ran = type_into(
            &mut state,
            vec![
                vec![],
                vec![text("x = pc.ske")],
                vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
                vec![text("}")],
                vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)],
                vec![text("print(x)")],
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            ],
        );
        assert!(
            matches!(ran.as_slice(), [UiCommand::RunConsole(t)] if t == "x = pc.sketch.new{}\nprint(x)"),
            "one run, of both lines: {ran:?}"
        );
        assert!(state.input.is_empty());
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
