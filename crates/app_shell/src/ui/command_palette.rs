//! The command palette: every shell command, every workbench tool and
//! every script of the scripts folder behind one search box (Ctrl+K or
//! the toolbar's search field), the document's features and their
//! parameters, and any command called by its id with JSON arguments
//! (`sketch.new {"plane": "XY"}`).

use core_document::{
    CommandArgs, CommandError, CommandSpec, Document, DocumentService, FeatureId, ToolDescriptor,
    WorkbenchId,
};
use egui::{Align2, Key, Modifiers, RichText, Sense, Stroke, Vec2, pos2, vec2};
use settings::ProjectionMode;
use ui_kit::tokens::*;
use ui_kit::widgets::key_chip;
use ui_kit::{sans, sans_medium};
use workbenches::REGISTERED_WORKBENCHES;

use super::{ActiveWorkbench, FileCommand, TreeItemId, UiCommand};

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    query: String,
    selected: usize,
    /// The frame the palette opened on: the field takes focus once.
    just_opened: bool,
    /// What was run lately, the latest first, as the settings keep it.
    pub recent: Vec<String>,
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
    pub toggle_console: bool,
    /// The Print layout task.
    pub print_layout: bool,
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
    Console,
    Quit,
    PrintLayout,
}

struct ShellEntry {
    label: &'static str,
    icon: &'static str,
    /// The keymap command whose keys the row shows.
    binding: Option<&'static str>,
    action: ShellAction,
}

const SHELL: &[ShellEntry] = &[
    ShellEntry {
        label: "New document",
        icon: "new-file",
        binding: Some("file.new"),
        action: ShellAction::File(FileCommand::New),
    },
    ShellEntry {
        label: "Open…",
        icon: "open",
        binding: Some("file.open"),
        action: ShellAction::File(FileCommand::Open),
    },
    ShellEntry {
        label: "Save",
        icon: "save",
        binding: Some("file.save"),
        action: ShellAction::File(FileCommand::Save),
    },
    ShellEntry {
        label: "Save as…",
        icon: "save",
        binding: Some("file.save_as"),
        action: ShellAction::File(FileCommand::SaveAs),
    },
    ShellEntry {
        label: "Import STEP, IGES or mesh…",
        icon: "file-document",
        binding: Some("file.import"),
        action: ShellAction::File(FileCommand::ImportStep),
    },
    ShellEntry {
        label: "Export as STEP, STL or 3MF…",
        icon: "export-stl",
        binding: Some("file.export"),
        action: ShellAction::File(FileCommand::Export),
    },
    ShellEntry {
        label: "Save view as picture…",
        icon: "export-stl",
        binding: None,
        action: ShellAction::File(FileCommand::SavePicture),
    },
    ShellEntry {
        label: "Send to slicer",
        icon: "workbench-print",
        binding: Some("file.send_to_slicer"),
        action: ShellAction::File(FileCommand::SendToSlicer),
    },
    ShellEntry {
        label: "Print layout…",
        icon: "workbench-print",
        binding: Some("file.print_layout"),
        action: ShellAction::PrintLayout,
    },
    ShellEntry {
        label: "Start page",
        icon: "tree-document",
        binding: None,
        action: ShellAction::StartPage,
    },
    ShellEntry {
        label: "Preferences…",
        icon: "settings",
        binding: Some("app.preferences"),
        action: ShellAction::Preferences,
    },
    ShellEntry {
        label: "Fit view",
        icon: "fit-all",
        binding: Some("view.fit_all"),
        action: ShellAction::FitView,
    },
    ShellEntry {
        label: "Undo",
        icon: "undo",
        binding: Some("edit.undo"),
        action: ShellAction::Undo,
    },
    ShellEntry {
        label: "Redo",
        icon: "redo",
        binding: Some("edit.redo"),
        action: ShellAction::Redo,
    },
    ShellEntry {
        label: "Toggle log panel",
        icon: "tree-document",
        binding: Some("app.log"),
        action: ShellAction::ToggleLog,
    },
    ShellEntry {
        label: "Recompute all",
        icon: "refresh",
        binding: Some("edit.recompute"),
        action: ShellAction::RecomputeAll,
    },
    ShellEntry {
        label: "Orthographic view",
        icon: "view-orthographic",
        binding: Some("view.orthographic"),
        action: ShellAction::Projection(ProjectionMode::Orthographic),
    },
    ShellEntry {
        label: "Perspective view",
        icon: "view-perspective",
        binding: Some("view.perspective"),
        action: ShellAction::Projection(ProjectionMode::Perspective),
    },
    ShellEntry {
        label: "Run script…",
        icon: "script",
        binding: Some("file.run_script"),
        action: ShellAction::File(FileCommand::RunScript),
    },
    ShellEntry {
        label: "Script console",
        icon: "console",
        binding: Some("app.console"),
        action: ShellAction::Console,
    },
    ShellEntry {
        label: "Quit",
        icon: "close",
        binding: Some("app.quit"),
        action: ShellAction::Quit,
    },
];

/// One row the palette can show.
struct Entry {
    label: String,
    /// Where the entry comes from: a workbench's name, a keymap group, or
    /// "printCAD" for the shell's own.
    scope: String,
    icon: &'static str,
    keys: Option<String>,
    /// Disabled in the current state, or planned.
    inert: bool,
    planned: Option<&'static str>,
    kind: EntryKind,
}

enum EntryKind {
    Shell(ShellAction),
    Tool {
        bench: WorkbenchId,
        id: String,
    },
    Script(std::path::PathBuf),
    /// A feature, or one of its parameters: selected as a tree click would.
    Feature(FeatureId),
    /// A command called by id with JSON arguments, checked against its spec.
    Call {
        id: String,
        args: CommandArgs,
    },
}

/// The query as a command call, `<id> <JSON object>` or a bare id: the row
/// that runs it, or an inert row saying why it cannot. `None` when the
/// query does not read as a call (no dot in its first word).
fn call_entry(query: &str, specs: &[CommandSpec]) -> Option<Entry> {
    let query = query.trim();
    let (id, rest) = query
        .split_once(char::is_whitespace)
        .map_or((query, ""), |(id, rest)| (id, rest.trim()));
    if !id.contains('.') || id.ends_with('.') {
        return None;
    }
    let id = core_document::renamed::command(id).into_owned();
    let row = |label: String, inert: bool, kind: EntryKind| Entry {
        label,
        scope: "Command".to_string(),
        icon: "console",
        keys: None,
        inert,
        planned: None,
        kind,
    };
    let refused = |message: String| {
        Some(row(
            message,
            true,
            EntryKind::Call {
                id: String::new(),
                args: CommandArgs::new(),
            },
        ))
    };
    let Some(spec) = specs.iter().find(|s| s.id == id) else {
        // A command still being typed matches no id yet: say so only once
        // a JSON object follows it.
        if !rest.starts_with('{') {
            return None;
        }
        let error = CommandError::unknown_among(&id, specs.iter().map(|s| s.id.as_str()));
        return refused(error.to_string());
    };
    let args = if rest.is_empty() {
        CommandArgs::new()
    } else {
        match serde_json::from_str::<serde_json::Value>(rest) {
            Ok(serde_json::Value::Object(args)) => args,
            Ok(_) => return refused(format!("{id}: the arguments must be a JSON object")),
            Err(err) => return refused(format!("{id}: {err}")),
        }
    };
    if let Err(err) = spec.check(&args) {
        return refused(format!("{id}: {err}"));
    }
    Some(row(
        format!("Run {id} · {}", spec.summary),
        false,
        EntryKind::Call { id, args },
    ))
}

/// The document's features and, under each, its parameters with their
/// values, as rows that select the feature.
fn feature_entries(document: &Document, registry: &DocumentService) -> Vec<Entry> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n)
        .collect();
    nodes.sort_by_key(|n| n.seq);
    let mut out = Vec::new();
    for node in nodes {
        let info = registry.feature_info(node);
        out.push(Entry {
            label: node.name.clone(),
            scope: info
                .as_ref()
                .map_or_else(|| "Feature".to_string(), |i| i.kind_label.clone()),
            icon: info.as_ref().map_or("tree-feature", |i| i.icon),
            keys: None,
            inert: false,
            planned: None,
            kind: EntryKind::Feature(node.id),
        });
        let values = document.feature_values(node.id).unwrap_or(&node.data);
        for parameter in registry.parameters(node) {
            let value = values
                .pointer(&parameter.pointer)
                .and_then(serde_json::Value::as_f64)
                .map(|v| v / parameter.scale);
            let unit = if parameter.dim == core_document::expr::Dim::LENGTH {
                " mm"
            } else if parameter.dim == core_document::expr::Dim::ANGLE {
                "°"
            } else {
                ""
            };
            let shown = value.map_or_else(String::new, |v| {
                format!(" = {}{unit}", (v * 1000.0).round() / 1000.0)
            });
            out.push(Entry {
                label: format!("{} › {}{shown}", node.name, parameter.label),
                scope: "Parameter".to_string(),
                icon: "expression",
                keys: None,
                inert: false,
                planned: None,
                kind: EntryKind::Feature(node.id),
            });
        }
    }
    out
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
    keymap: &super::keymap::Keymap,
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
                keys: keymap.text(&tool.id),
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
            keys: shell.binding.and_then(|id| keymap.text(id)),
            inert: false,
            planned: None,
            kind: EntryKind::Shell(shell.action),
        });
    }
    // The scripts folder's scripts are in the keymap, as every command is.
    for binding in keymap.bindings() {
        if let super::keymap::Target::Script(path) = &binding.target {
            out.push(Entry {
                label: binding.label.clone(),
                scope: binding.group.clone(),
                icon: "script",
                keys: binding.keys.first().map(ToString::to_string),
                inert: false,
                planned: None,
                kind: EntryKind::Script(path.clone()),
            });
        }
    }
    out
}

/// How many recent commands the palette keeps.
pub const RECENT_KEPT: usize = 8;

impl Entry {
    /// What names the entry among runs: the recent list keeps these.
    fn key(&self) -> String {
        match &self.kind {
            EntryKind::Shell(_) => format!("shell:{}", self.label),
            EntryKind::Tool { bench, id } => format!("tool:{}:{id}", bench.as_str()),
            EntryKind::Script(path) => format!("script:{}", path.display()),
            EntryKind::Feature(id) => format!("feature:{}", id.0),
            EntryKind::Call { id, .. } => format!("call:{id}"),
        }
    }
}

/// The order entries show in: the better match first; among matches as
/// good, what was run lately, the latest first; then as listed.
fn order(all: &[Entry], needle: &str, recent: &[String]) -> Vec<usize> {
    let mut shown: Vec<(u8, usize, usize)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let text = format!("{} {}", e.label.to_lowercase(), e.scope.to_lowercase());
            let lately = recent
                .iter()
                .position(|k| *k == e.key())
                .unwrap_or(usize::MAX);
            match_rank(&text, needle).map(|rank| (rank, lately, i))
        })
        .collect();
    shown.sort();
    shown.into_iter().map(|(_, _, i)| i).collect()
}

fn run(entry: &Entry, commands: &mut Vec<UiCommand>, result: &mut PaletteResult) {
    // What the document holds, and a call's arguments, are not commands
    // to bring back.
    if !matches!(entry.kind, EntryKind::Feature(_) | EntryKind::Call { .. }) {
        commands.push(UiCommand::NoteRecentCommand(entry.key()));
    }
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
            ShellAction::Console => result.toggle_console = true,
            ShellAction::Quit => commands.push(UiCommand::Quit),
            ShellAction::PrintLayout => result.print_layout = true,
        },
        EntryKind::Tool { bench, id } => {
            result.activate_tool = Some((ActiveWorkbench(bench.clone()), id.clone()));
        }
        EntryKind::Script(path) => commands.push(UiCommand::RunScriptFile(path.clone())),
        EntryKind::Feature(id) => {
            commands.push(UiCommand::SelectTreeItem(TreeItemId::Feature(*id)))
        }
        EntryKind::Call { id, args } => commands.push(UiCommand::RunCommand {
            id: id.clone(),
            args: args.clone(),
        }),
    }
}

const WIDTH: f32 = 560.0;
const ROW: f32 = 34.0;
const MAX_ROWS: usize = 10;

#[allow(clippy::too_many_arguments)]
pub fn draw_command_palette(
    ctx: &egui::Context,
    state: &mut PaletteState,
    document: &Document,
    specs: &[CommandSpec],
    registry: &mut DocumentService,
    keymap: &super::keymap::Keymap,
    active: &ActiveWorkbench,
    enabled_active: &dyn Fn(&str) -> bool,
    commands: &mut Vec<UiCommand>,
) -> PaletteResult {
    let mut result = PaletteResult::default();
    if !state.open {
        return result;
    }
    let mut all = entries(registry, keymap, active, enabled_active);
    let needle = state.query.trim().to_lowercase();
    // The document's features list only once something is typed, after
    // the commands and tools as good a match.
    if !needle.is_empty() {
        all.extend(feature_entries(document, registry));
    }
    let mut shown = order(&all, &needle, &state.recent);
    if let Some(call) = call_entry(&state.query, specs) {
        shown.insert(0, all.len());
        all.push(call);
    }
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
                    .hint_text("Type a command, tool or feature, or an id and JSON…")
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
                        if let Some(keys) = &entry.keys {
                            key_chip(&mut right, keys);
                        }
                        let scope = match entry.planned {
                            Some(_) => "planned".to_string(),
                            None => entry.scope.clone(),
                        };
                        right.label(RichText::new(scope).font(sans(FONT_XS)).color(TEXT3));
                        let response = match entry.planned {
                            Some(note) => response.on_hover_text(format!("Planned: {note}")),
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

    /// A query reads as a call by its dotted first word: a runnable row
    /// when its JSON checks against the command, an inert row saying why
    /// when it does not, and nothing while plain words are typed.
    #[test]
    fn a_typed_call_runs_only_when_its_arguments_check() {
        use core_document::ParamKind;
        let specs = [CommandSpec::new("doc.move", "Move a body")
            .param("body", ParamKind::String, "the body")
            .optional("x", ParamKind::Number, "mm")];
        let row = |query: &str| call_entry(query, &specs).map(|e| (e.inert, e.label));
        let runs = row(r#"doc.move {"body": "Box", "x": 5}"#).unwrap();
        assert_eq!(runs, (false, "Run doc.move · Move a body".to_string()));
        for (query, says) in [
            ("doc.move", "`body`"),
            (r#"doc.move {"body": 3}"#, "body"),
            (r#"doc.move {"body": "#, "EOF"),
            ("doc.move [1]", "JSON object"),
            (r#"doc.mvoe {"body": "Box"}"#, "did you mean `doc.move`"),
        ] {
            let (inert, label) = row(query).unwrap_or_else(|| panic!("{query}: no row"));
            assert!(inert && label.contains(says), "{query}: {label}");
        }
        for plain in ["pad", "fit view", "doc.", "doc.mo"] {
            assert!(row(plain).is_none(), "{plain}");
        }
    }

    /// What ran lately comes first, the latest first, but never ahead of
    /// a better match: a substring before letters spread out, and a miss
    /// not at all.
    #[test]
    fn recent_commands_come_first_among_equal_matches() {
        let tool = |label: &str| Entry {
            label: label.to_string(),
            scope: "Design".to_string(),
            icon: "pad",
            keys: None,
            inert: false,
            planned: None,
            kind: EntryKind::Tool {
                bench: WorkbenchId::from("wb.test"),
                id: format!("test.{}", label.to_lowercase()),
            },
        };
        let all = [tool("Pad"), tool("Pocket"), tool("Fillet"), tool("Chamfer")];
        let recent = vec![all[2].key(), all[1].key()];
        let labels = |needle: &str| -> Vec<&str> {
            order(&all, needle, &recent)
                .into_iter()
                .map(|i| all[i].label.as_str())
                .collect()
        };
        assert_eq!(labels(""), ["Fillet", "Pocket", "Pad", "Chamfer"]);
        // "pa" is in Pad itself; Pocket only matches it spread out.
        assert_eq!(labels("pa"), ["Pad"]);
        assert_eq!(match_rank("pocket", "pkt"), Some(1));
        assert!(labels("xyz").is_empty());
    }
}
