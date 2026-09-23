//! The command palette: every shell command and every workbench tool
//! behind one search box (Ctrl+K or the toolbar's search field).

use core_document::{DocumentService, ToolDescriptor, WorkbenchId};
use egui::{Align2, Key, Modifiers, RichText, Sense, Stroke, Vec2, pos2, vec2};
use settings::ProjectionMode;
use ui_kit::tokens::*;
use ui_kit::widgets::key_chip;
use ui_kit::{sans, sans_medium};
use workbenches::REGISTERED_WORKBENCHES;

use super::{ActiveWorkbench, FileCommand, UiCommand};

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    query: String,
    selected: usize,
    /// The frame the palette opened on: the field takes focus once.
    just_opened: bool,
}

impl PaletteState {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.just_opened = true;
    }
}

/// What the palette asked for this frame.
#[derive(Default)]
pub struct PaletteResult {
    pub show_preferences: bool,
    /// Switch to this workbench, then activate the tool.
    pub activate_tool: Option<(ActiveWorkbench, String)>,
}

/// A shell action the palette can run.
#[derive(Clone, Copy)]
enum ShellAction {
    File(FileCommand),
    StartPage,
    Preferences,
    FitView,
    Undo,
    Redo,
    ToggleLog,
    RecomputeAll,
    Projection(ProjectionMode),
    Quit,
}

struct ShellEntry {
    label: &'static str,
    icon: &'static str,
    keys: Option<&'static str>,
    action: ShellAction,
}

const SHELL: &[ShellEntry] = &[
    ShellEntry {
        label: "New document",
        icon: "new-file",
        keys: Some("Ctrl N"),
        action: ShellAction::File(FileCommand::New),
    },
    ShellEntry {
        label: "Open…",
        icon: "open",
        keys: Some("Ctrl O"),
        action: ShellAction::File(FileCommand::Open),
    },
    ShellEntry {
        label: "Save",
        icon: "save",
        keys: Some("Ctrl S"),
        action: ShellAction::File(FileCommand::Save),
    },
    ShellEntry {
        label: "Save as…",
        icon: "save",
        keys: Some("Ctrl Shift S"),
        action: ShellAction::File(FileCommand::SaveAs),
    },
    ShellEntry {
        label: "Import STEP, IGES or mesh…",
        icon: "file-document",
        keys: Some("Ctrl I"),
        action: ShellAction::File(FileCommand::ImportStep),
    },
    ShellEntry {
        label: "Start page",
        icon: "tree-document",
        keys: None,
        action: ShellAction::StartPage,
    },
    ShellEntry {
        label: "Preferences…",
        icon: "settings",
        keys: Some("Ctrl ,"),
        action: ShellAction::Preferences,
    },
    ShellEntry {
        label: "Fit view",
        icon: "fit-all",
        keys: Some("F"),
        action: ShellAction::FitView,
    },
    ShellEntry {
        label: "Undo",
        icon: "undo",
        keys: Some("Ctrl Z"),
        action: ShellAction::Undo,
    },
    ShellEntry {
        label: "Redo",
        icon: "redo",
        keys: Some("Ctrl Shift Z"),
        action: ShellAction::Redo,
    },
    ShellEntry {
        label: "Toggle log panel",
        icon: "tree-document",
        keys: None,
        action: ShellAction::ToggleLog,
    },
    ShellEntry {
        label: "Recompute all",
        icon: "refresh",
        keys: None,
        action: ShellAction::RecomputeAll,
    },
    ShellEntry {
        label: "Orthographic view",
        icon: "view-orthographic",
        keys: None,
        action: ShellAction::Projection(ProjectionMode::Orthographic),
    },
    ShellEntry {
        label: "Perspective view",
        icon: "view-perspective",
        keys: None,
        action: ShellAction::Projection(ProjectionMode::Perspective),
    },
    ShellEntry {
        label: "Quit",
        icon: "close",
        keys: Some("Ctrl Q"),
        action: ShellAction::Quit,
    },
];

/// One row the palette can show.
struct Entry {
    label: String,
    /// "Sketcher", "Part Design" or "printCAD".
    scope: String,
    icon: &'static str,
    keys: Option<&'static str>,
    /// Disabled in the current state, or planned.
    inert: bool,
    planned: Option<&'static str>,
    kind: EntryKind,
}

enum EntryKind {
    Shell(ShellAction),
    Tool { bench: WorkbenchId, id: String },
}

/// 0 = substring, 1 = subsequence, None = no match.
fn match_rank(haystack: &str, needle: &str) -> Option<u8> {
    if needle.is_empty() || haystack.contains(needle) {
        return Some(0);
    }
    let mut chars = needle.chars();
    let mut want = chars.next();
    for c in haystack.chars() {
        if Some(c) == want {
            want = chars.next();
            if want.is_none() {
                return Some(1);
            }
        }
    }
    None
}

fn entries(
    registry: &mut DocumentService,
    active: &ActiveWorkbench,
    enabled_active: &dyn Fn(&str) -> bool,
) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let benches: Vec<(WorkbenchId, String)> = REGISTERED_WORKBENCHES
        .lock()
        .unwrap()
        .iter()
        .map(|wb| (WorkbenchId::from(wb.id.as_str()), wb.label.clone()))
        .collect();
    // The active bench's tools first, then the others'.
    let mut ordered = benches.clone();
    ordered.sort_by_key(|(id, _)| *id != active.0);
    for (bench, bench_label) in ordered {
        let tools: Vec<ToolDescriptor> = registry
            .tools_for(&bench)
            .map(|t| t.to_vec())
            .unwrap_or_default();
        let is_active = bench == active.0;
        for tool in tools {
            let enabled = !is_active || enabled_active(&tool.id);
            out.push(Entry {
                label: tool.label.clone(),
                scope: bench_label.clone(),
                icon: tool.icon.unwrap_or("tree-feature"),
                keys: None,
                inert: !enabled || tool.planned.is_some(),
                planned: tool.planned,
                kind: EntryKind::Tool {
                    bench: bench.clone(),
                    id: tool.id.clone(),
                },
            });
        }
    }
    for shell in SHELL {
        out.push(Entry {
            label: shell.label.to_string(),
            scope: "printCAD".to_string(),
            icon: shell.icon,
            keys: shell.keys,
            inert: false,
            planned: None,
            kind: EntryKind::Shell(shell.action),
        });
    }
    out
}

fn run(entry: &Entry, commands: &mut Vec<UiCommand>, result: &mut PaletteResult) {
    match &entry.kind {
        EntryKind::Shell(action) => match action {
            ShellAction::File(f) => commands.push(UiCommand::File(*f)),
            ShellAction::StartPage => commands.push(UiCommand::ShowStartPage),
            ShellAction::Preferences => result.show_preferences = true,
            ShellAction::FitView => commands.push(UiCommand::FitView),
            ShellAction::Undo => commands.push(UiCommand::Undo),
            ShellAction::Redo => commands.push(UiCommand::Redo),
            ShellAction::ToggleLog => commands.push(UiCommand::ToggleLogPanel),
            ShellAction::RecomputeAll => commands.push(UiCommand::RecomputeAll),
            ShellAction::Projection(mode) => commands.push(UiCommand::SetProjection(*mode)),
            ShellAction::Quit => commands.push(UiCommand::Quit),
        },
        EntryKind::Tool { bench, id } => {
            result.activate_tool = Some((ActiveWorkbench(bench.clone()), id.clone()));
        }
    }
}

const WIDTH: f32 = 560.0;
const ROW: f32 = 34.0;
const MAX_ROWS: usize = 10;

pub fn draw_command_palette(
    ctx: &egui::Context,
    state: &mut PaletteState,
    registry: &mut DocumentService,
    active: &ActiveWorkbench,
    enabled_active: &dyn Fn(&str) -> bool,
    commands: &mut Vec<UiCommand>,
) -> PaletteResult {
    let mut result = PaletteResult::default();
    if !state.open {
        return result;
    }
    let all = entries(registry, active, enabled_active);
    let needle = state.query.trim().to_lowercase();
    let mut shown: Vec<(u8, usize)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let text = format!("{} {}", e.label.to_lowercase(), e.scope.to_lowercase());
            match_rank(&text, &needle).map(|rank| (rank, i))
        })
        .collect();
    shown.sort_by_key(|(rank, i)| (*rank, *i));
    let shown: Vec<usize> = shown.into_iter().map(|(_, i)| i).collect();
    if shown.is_empty() {
        state.selected = 0;
    } else if state.selected >= shown.len() {
        state.selected = shown.len() - 1;
    }

    let mut close = false;
    let mut chosen: Option<usize> = None;
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            close = true;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) && !shown.is_empty() {
            state.selected = (state.selected + 1).min(shown.len() - 1);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            state.selected = state.selected.saturating_sub(1);
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) {
            chosen = shown.get(state.selected).copied();
        }
    });

    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(0);
    let id = egui::Id::new("command_palette");
    let modal = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, 90.0)))
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(110))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 0.0;
            // Search row.
            let (row, _) = ui.allocate_exact_size(vec2(WIDTH, 46.0), Sense::hover());
            let mut search = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row.shrink2(vec2(14.0, 0.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            search.spacing_mut().item_spacing.x = SPACE_2;
            ui_kit::icon::draw(&mut search, "search", 16.0, TEXT3);
            let edit = search.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Type a command or tool…")
                    .frame(egui::Frame::NONE)
                    .font(sans(FONT_MD))
                    .desired_width(f32::INFINITY),
            );
            if state.just_opened {
                edit.request_focus();
                state.just_opened = false;
            }
            if edit.changed() {
                state.selected = 0;
            }
            ui.painter()
                .hline(row.x_range(), row.bottom(), Stroke::new(1.0, BORDER));

            if shown.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(WIDTH, ROW + 8.0), Sense::hover());
                ui.painter().text(
                    pos2(r.left() + 14.0, r.center().y),
                    Align2::LEFT_CENTER,
                    "No command matches.",
                    sans(FONT_SM),
                    TEXT3,
                );
            }
            egui::ScrollArea::vertical()
                .max_height(ROW * MAX_ROWS as f32)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (pos, index) in shown.iter().enumerate() {
                        let entry = &all[*index];
                        let selected = pos == state.selected;
                        let (r, response) =
                            ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::click());
                        if selected {
                            ui.painter().rect_filled(r, 0.0, ACCENT_DIM);
                        } else if response.hovered() {
                            ui.painter().rect_filled(r, 0.0, BG2);
                        }
                        if selected {
                            ui.scroll_to_rect(r, None);
                        }
                        let text = if entry.inert { TEXT3 } else { TEXT1 };
                        let icon_rect = egui::Rect::from_center_size(
                            pos2(r.left() + 24.0, r.center().y),
                            Vec2::splat(16.0),
                        );
                        if let Some(tex) = ui_kit::icon::texture(ui.ctx(), entry.icon) {
                            let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                            ui.painter().image(
                                tex.id(),
                                icon_rect,
                                uv,
                                if entry.inert { TEXT3 } else { TEXT2 },
                            );
                        }
                        ui.painter().text(
                            pos2(r.left() + 42.0, r.center().y),
                            Align2::LEFT_CENTER,
                            &entry.label,
                            sans_medium(FONT_SM),
                            text,
                        );
                        let mut right = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(r.shrink2(vec2(14.0, 6.0)))
                                .layout(egui::Layout::right_to_left(egui::Align::Center)),
                        );
                        right.spacing_mut().item_spacing.x = SPACE_2;
                        if let Some(keys) = entry.keys {
                            key_chip(&mut right, keys);
                        }
                        let scope = match entry.planned {
                            Some(_) => "planned".to_string(),
                            None => entry.scope.clone(),
                        };
                        right.label(RichText::new(scope).font(sans(FONT_XS)).color(TEXT3));
                        let response = match entry.planned {
                            Some(note) => response.on_hover_text(format!("Planned — {note}")),
                            None => response,
                        };
                        if response.hovered() {
                            state.selected = pos;
                        }
                        if response.clicked() {
                            chosen = Some(*index);
                        }
                    }
                });
            let (foot, _) = ui.allocate_exact_size(vec2(WIDTH, 30.0), Sense::hover());
            ui.painter()
                .hline(foot.x_range(), foot.top(), Stroke::new(1.0, BORDER));
            ui.painter().text(
                pos2(foot.left() + 14.0, foot.center().y),
                Align2::LEFT_CENTER,
                "↑↓ move · Enter run · Esc close",
                sans(FONT_XS),
                TEXT3,
            );
        });
    if modal.should_close() {
        close = true;
    }
    if let Some(index) = chosen {
        let entry = &all[index];
        if !entry.inert {
            run(entry, commands, &mut result);
            close = true;
        }
    }
    if close {
        state.open = false;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_beats_subsequence_and_misses_are_none() {
        assert_eq!(match_rank("pad sketch", "pad"), Some(0));
        assert_eq!(match_rank("pocket", "pkt"), Some(1));
        assert_eq!(match_rank("pocket", "xyz"), None);
        assert_eq!(match_rank("anything", ""), Some(0));
    }
}
