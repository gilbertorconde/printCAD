//! The toolbar rows under the menu bar: standard tools, the workbench
//! switcher and the active workbench's tools, one row per
//! `ToolDescriptor::row`, separated where the category changes.

use core_document::{DocumentService, ToolBehavior, ToolDescriptor, WorkbenchId};
use egui::{Popup, RichText, Vec2};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{ToolButtonState, tool_button, vseparator};
use workbenches::REGISTERED_WORKBENCHES;

use super::host_ctx::{HostCtxParams, panel_ctx};
use super::{ActiveTool, ActiveWorkbench, UiCommand};

/// A button on the standard row that maps to an app command.
struct ShellItem {
    /// The button draws pressed.
    on: bool,
    icon: &'static str,
    label: &'static str,
    /// The keymap command whose key the tooltip names.
    binding: &'static str,
    command: Option<UiCommand>,
    planned: Option<&'static str>,
}

fn shell(
    icon: &'static str,
    label: &'static str,
    binding: &'static str,
    command: UiCommand,
) -> ShellItem {
    ShellItem {
        on: false,
        icon,
        label,
        binding,
        command: Some(command),
        planned: None,
    }
}

fn toggle(
    icon: &'static str,
    label: &'static str,
    binding: &'static str,
    on: bool,
    command: UiCommand,
) -> ShellItem {
    ShellItem {
        on,
        icon,
        label,
        binding,
        command: Some(command),
        planned: None,
    }
}

/// A button's tooltip: its name, and its key when it has one.
fn with_key(label: &str, key: Option<&core_document::Chord>) -> String {
    match key {
        Some(key) => format!("{label} ({key})"),
        None => label.to_string(),
    }
}

fn standard_items(
    show_print_bed: bool,
    show_annotations: bool,
    measuring: bool,
) -> Vec<Option<ShellItem>> {
    use super::{EditCommand, FileCommand};
    vec![
        Some(shell(
            "new-file",
            "New",
            "file.new",
            UiCommand::File(FileCommand::New),
        )),
        Some(shell(
            "open",
            "Open",
            "file.open",
            UiCommand::File(FileCommand::Open),
        )),
        Some(shell(
            "save",
            "Save",
            "file.save",
            UiCommand::File(FileCommand::Save),
        )),
        None,
        Some(shell("undo", "Undo", "edit.undo", UiCommand::Undo)),
        Some(shell("redo", "Redo", "edit.redo", UiCommand::Redo)),
        None,
        Some(shell(
            "cut",
            "Cut",
            "edit.cut",
            UiCommand::Edit(EditCommand::Cut),
        )),
        Some(shell(
            "copy",
            "Copy",
            "edit.copy",
            UiCommand::Edit(EditCommand::Copy),
        )),
        Some(shell(
            "paste",
            "Paste",
            "edit.paste",
            UiCommand::Edit(EditCommand::Paste),
        )),
        None,
        Some(shell(
            "refresh",
            "Recompute",
            "edit.recompute",
            UiCommand::RecomputeAll,
        )),
        Some(toggle(
            "measure",
            "Measure",
            "view.measure",
            measuring,
            UiCommand::ToggleMeasure,
        )),
        Some(toggle(
            "print-bed",
            "Print bed",
            "view.print_bed",
            show_print_bed,
            UiCommand::TogglePrintBed,
        )),
        Some(toggle(
            "dimensional-constraint",
            "Annotations",
            "view.annotations",
            show_annotations,
            UiCommand::ToggleAnnotations,
        )),
    ]
}

/// Everything the toolbar needs from the frame.
pub struct ToolbarInputs<'a> {
    pub registry: &'a mut DocumentService,
    pub document: &'a mut core_document::Document,
    pub host: HostCtxParams,
    pub active_document_object: Option<core_document::FeatureId>,
    /// The print-bed button's state.
    pub show_print_bed: bool,
    /// The annotations button's state.
    pub show_annotations: bool,
    /// The measure button's state.
    pub measuring: bool,
    /// The keys buttons name in their tooltips.
    pub keymap: &'a super::keymap::Keymap,
    /// The scripts folder's scripts, for the Scripts button.
    pub scripts: &'a [crate::script_library::ScriptEntry],
    /// The console is showing.
    pub console_open: bool,
    /// A recording is on.
    pub recording: bool,
    /// Where the user put the toolbar groups.
    pub layout: &'a [Vec<String>],
    /// Choose a workbench through tabs rather than the dropdown.
    pub workbench_tabs: bool,
    /// Show category tabs with labeled tool buttons.
    pub ribbon: bool,
    /// Selected category per workbench for this session.
    pub tool_tabs: &'a mut std::collections::BTreeMap<String, String>,
}

/// What the Scripts button's menu shows: the scripts folder's scripts,
/// then running a file, the console and the folder itself.
struct ScriptsMenu<'a> {
    scripts: &'a [crate::script_library::ScriptEntry],
    console_open: bool,
    recording: bool,
}

fn scripts_button(
    ui: &mut egui::Ui,
    menu: ScriptsMenu<'_>,
    keymap: &super::keymap::Keymap,
    commands: &mut Vec<UiCommand>,
    toggle_console: &mut bool,
) {
    let ScriptsMenu {
        scripts,
        console_open,
        recording,
    } = menu;
    let state = ToolButtonState {
        enabled: true,
        active: false,
        planned: None,
        menu: true,
    };
    let response = tool_button(ui, "script", "Scripts", TOOLBAR_BUTTON, state);
    super::widget_map::note(ui.ctx(), || "button:std.scripts".into(), response.rect);
    Popup::menu(&response).show(|ui| {
        // One row: an icon, the label and its key; whether it was clicked.
        let entry = |ui: &mut egui::Ui, icon: &str, label: &str, key: Option<String>, tip: &str| {
            let response = ui.add(menu_row(ui.ctx(), icon, TEXT2, label, TEXT1, key));
            if tip.is_empty() {
                response.clicked()
            } else {
                response.on_hover_text(tip).clicked()
            }
        };
        for script in scripts {
            let tip = script.about.as_deref().unwrap_or("");
            if entry(ui, "script", &script.name, keymap.text(&script.id), tip) {
                commands.push(UiCommand::RunScriptFile(script.path.clone()));
                ui.close();
            }
        }
        if !scripts.is_empty() {
            ui.separator();
        }
        let run = keymap.text("file.run_script");
        if entry(ui, "open", "Run script…", run, "Run a Lua file") {
            commands.push(UiCommand::File(super::FileCommand::RunScript));
            ui.close();
        }
        let (record, tip) = if recording {
            ("Stop recording", "Save what was recorded as a new script")
        } else {
            (
                "Record…",
                "Record what you do from now on as a script that does it again",
            )
        };
        if entry(ui, "script", record, keymap.text("app.record"), tip) {
            commands.push(UiCommand::ToggleRecording);
            ui.close();
        }
        let console = if console_open {
            "Hide console"
        } else {
            "Console"
        };
        if entry(ui, "console", console, keymap.text("app.console"), "") {
            *toggle_console = true;
            ui.close();
        }
        if entry(
            ui,
            "new-file",
            "New script",
            None,
            "A new script in the scripts folder",
        ) {
            commands.push(UiCommand::NewScript);
            ui.close();
        }
        if entry(
            ui,
            "open",
            "Open scripts folder",
            None,
            "Its .lua files are the scripts above",
        ) {
            commands.push(UiCommand::EditScript(None));
            ui.close();
        }
    });
}

/// A menu row with an icon before its label and its keys at the end.
fn menu_row<'a>(
    ctx: &egui::Context,
    icon: &str,
    icon_tint: egui::Color32,
    label: &str,
    color: egui::Color32,
    key: Option<String>,
) -> egui::Button<'a> {
    let text = RichText::new(label).font(sans(FONT_SM)).color(color);
    let mut button = match ui_kit::icon::image(ctx, icon, 16.0, icon_tint) {
        Some(image) => egui::Button::image_and_text(image, text),
        None => egui::Button::new(text),
    };
    if let Some(key) = key {
        button = button.shortcut_text(RichText::new(key).font(sans(FONT_XS)).color(TEXT3));
    }
    button
}

/// The tool a variant dropdown last picked, remembered per tool id.
fn remembered_variant(ctx: &egui::Context, tool_id: &str) -> Option<usize> {
    ctx.data_mut(|d| d.get_persisted::<usize>(egui::Id::new(("toolbar_variant", tool_id))))
}

fn remember_variant(ctx: &egui::Context, tool_id: &str, index: usize) {
    ctx.data_mut(|d| d.insert_persisted(egui::Id::new(("toolbar_variant", tool_id)), index));
}

/// Activate `id` following `tool`'s behavior: Actions fire once, Checks
/// toggle, Radios clear their group first.
pub fn activate_tool(
    active_tool: &mut ActiveTool,
    tools: &[ToolDescriptor],
    tool: &ToolDescriptor,
    id: &str,
) {
    let is_active = active_tool.active_ids.contains(id);
    match tool.behavior {
        ToolBehavior::Action => {
            // Fire-and-forget: the host clears it after handling the input.
            active_tool.active_ids.insert(id.to_owned());
        }
        ToolBehavior::Check => {
            if is_active {
                active_tool.active_ids.remove(id);
            } else {
                active_tool.active_ids.insert(id.to_owned());
            }
        }
        ToolBehavior::Radio => {
            if is_active {
                active_tool.active_ids.remove(id);
            } else {
                match &tool.group {
                    Some(group) => active_tool.active_ids.retain(|active_id| {
                        tools
                            .iter()
                            .find(|t| t.id == core_document::base_tool_id(active_id))
                            .map(|t| t.group.as_deref() != Some(group))
                            .unwrap_or(true)
                    }),
                    None => active_tool.active_ids.clear(),
                }
                active_tool.active_ids.insert(id.to_owned());
            }
        }
    }
}

fn tool_is_active(active_tool: &ActiveTool, tool_id: &str) -> bool {
    active_tool
        .active_ids
        .iter()
        .any(|id| core_document::base_tool_id(id) == tool_id)
}

/// Draws one workbench tool button (with its variant dropdown) and
/// activates it on click.
fn draw_tool(
    ui: &mut egui::Ui,
    tool: &ToolDescriptor,
    tools: &[ToolDescriptor],
    status: (bool, bool),
    ribbon: bool,
    active_tool: &mut ActiveTool,
) {
    let (enabled, toggled) = status;
    let variant_index = remembered_variant(ui.ctx(), &tool.id).filter(|i| *i < tool.variants.len());
    let (icon, label, planned): (&str, String, Option<&'static str>) = match variant_index {
        Some(i) => {
            let v = &tool.variants[i];
            (
                v.icon,
                format!("{} · {}", tool.label, v.label),
                v.planned.or(tool.planned),
            )
        }
        None => (
            tool.icon.unwrap_or("more"),
            tool.label.clone(),
            tool.planned,
        ),
    };
    let state = ToolButtonState {
        enabled,
        active: toggled || tool_is_active(active_tool, &tool.id),
        planned,
        menu: !tool.variants.is_empty(),
    };
    let label = with_key(&label, tool.shortcuts.first());
    let response = if ribbon {
        let caption = variant_index.map_or(tool.label.as_str(), |i| tool.variants[i].label);
        ui_kit::widgets::ribbon_button(ui, icon, &label, caption, state)
    } else {
        tool_button(ui, icon, &label, TOOLBAR_BUTTON, state)
    };
    super::widget_map::note(ui.ctx(), || format!("button:{}", tool.id), response.rect);

    let activate_id = match variant_index {
        Some(i) => format!("{}:{}", tool.id, tool.variants[i].id),
        None => tool.id.clone(),
    };
    if tool.variants.is_empty() {
        if response.clicked() && enabled && planned.is_none() {
            activate_tool(active_tool, tools, tool, &activate_id);
        }
        return;
    }

    // The chevron strip at the button's end opens the variant list; the rest
    // of the button activates the remembered variant. A right click or a
    // long press opens the list from anywhere on the button.
    let chevron = egui::Rect::from_min_max(
        egui::pos2(response.rect.right() - 12.0, response.rect.top()),
        response.rect.right_bottom(),
    );
    let chevron_response = ui.interact(chevron, response.id.with("chevron"), egui::Sense::click());
    let popup_id = response.id.with("variants");
    if response.clicked() && enabled && planned.is_none() {
        activate_tool(active_tool, tools, tool, &activate_id);
    }
    if response.secondary_clicked() || response.long_touched() {
        Popup::toggle_id(ui.ctx(), popup_id);
    }
    Popup::menu(&chevron_response).id(popup_id).show(|ui| {
        for (i, variant) in tool.variants.iter().enumerate() {
            let planned = variant.planned.is_some();
            let row = menu_row(
                ui.ctx(),
                variant.icon,
                if planned { TEXT3 } else { TEXT2 },
                variant.label,
                if planned { TEXT3 } else { TEXT1 },
                None,
            );
            let r = ui.add_enabled(!planned, row);
            if let Some(note) = variant.planned {
                r.on_disabled_hover_text(format!("{} (planned)\n{note}", variant.label));
            } else if r.clicked() {
                remember_variant(ui.ctx(), &tool.id, i);
                if enabled {
                    activate_tool(
                        active_tool,
                        tools,
                        tool,
                        &format!("{}:{}", tool.id, variant.id),
                    );
                }
                ui.close();
            }
        }
    });
}

fn tool_category(tool: &ToolDescriptor) -> &str {
    tool.category
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("General")
}

fn tool_tab(tool: &ToolDescriptor) -> &str {
    tool_category(tool).split('.').next().unwrap_or("General")
}

fn category_label(category: &str) -> String {
    let mut chars = category.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn tool_categories(tools: &[ToolDescriptor]) -> Vec<&str> {
    let mut categories = Vec::new();
    for tool in tools {
        let category = tool_tab(tool);
        if !categories.contains(&category) {
            categories.push(category);
        }
    }
    categories
}

fn workbench_combo(ui: &mut egui::Ui, active_workbench: &mut ActiveWorkbench) {
    let workbenches = REGISTERED_WORKBENCHES.lock().unwrap();
    let (current, icon) = workbenches
        .iter()
        .find(|wb| wb.id == active_workbench.0)
        .map(|wb| ((wb.label.clone(), wb.description.clone()), wb.icon))
        .unwrap_or_else(|| (("(none)".to_string(), String::new()), "workbench-print"));
    // An exact footprint: a frame grown inside the row would claim the
    // rest of it.
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(180.0, INPUT + 2.0), egui::Sense::click());
    ui.painter().rect(
        rect,
        5.0,
        BG2,
        egui::Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = SPACE_2;
    ui_kit::icon::draw(&mut inner, icon, 16.0, ACCENT);
    inner.label(RichText::new(&current.0).font(sans(FONT_SM)).color(TEXT1));
    inner.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui_kit::icon::draw(ui, "chevron-down", 14.0, TEXT3);
    });
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if current.1.is_empty() {
            "Active workbench".to_string()
        } else {
            format!("Active workbench: {}", current.1)
        });
    Popup::menu(&response).show(|ui| {
        for wb in workbenches.iter() {
            let target = ActiveWorkbench(WorkbenchId::from(wb.id.as_str()));
            let selected = *active_workbench == target;
            let row = menu_row(
                ui.ctx(),
                wb.icon,
                if selected { ACCENT } else { TEXT2 },
                &wb.label,
                TEXT1,
                None,
            )
            .selected(selected);
            let r = ui.add(row).on_hover_text(&wb.description);
            if r.clicked() {
                *active_workbench = target;
                ui.close();
            }
        }
    });
}

fn row_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG1)
        .inner_margin(egui::Margin::symmetric(6, 0))
}

/// A run of buttons the user can drag, by the grip at its start, to
/// another place in its row, to another row or to a row of its own.
struct Group {
    /// Its name in the saved layout.
    id: String,
    /// What its grip calls it.
    label: String,
    /// The row its bench puts it on.
    default_row: usize,
    /// It sits at its row's right end.
    end: bool,
    content: GroupContent,
}

enum GroupContent {
    /// A run of the standard buttons, by index.
    Standard(Vec<usize>),
    Scripts,
    Workbench,
    /// The active bench's tools, by index.
    Tools(Vec<usize>),
}

/// The standard buttons' groups: what each is called and the buttons it
/// holds, in the order `standard_items` lists them between separators.
const STANDARD_GROUPS: [(&str, &str); 4] = [
    ("std.file", "File"),
    ("std.history", "Undo and redo"),
    ("std.clipboard", "Clipboard"),
    ("std.view", "View"),
];

fn groups(standard: &[Option<ShellItem>], tools: &[ToolDescriptor], bench: &str) -> Vec<Group> {
    let mut out = Vec::new();
    let mut runs = standard.split(Option::is_none);
    let mut first = 0;
    for (id, label) in STANDARD_GROUPS {
        let run = runs.next().unwrap_or_default();
        out.push(Group {
            id: id.to_string(),
            label: label.to_string(),
            default_row: 0,
            end: false,
            content: GroupContent::Standard((first..first + run.len()).collect()),
        });
        first += run.len() + 1;
    }
    out.push(Group {
        id: "std.scripts".into(),
        label: "Scripts".into(),
        default_row: 0,
        end: false,
        content: GroupContent::Scripts,
    });
    out.push(Group {
        id: "std.workbench".into(),
        label: "Workbench".into(),
        default_row: 0,
        end: false,
        content: GroupContent::Workbench,
    });
    // The bench's tools, one group per run of a category along a row and
    // side.
    let mut open: Vec<((u8, bool), usize)> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    for (i, tool) in tools.iter().enumerate() {
        let side = (tool.row, tool.align_end);
        let category = tool.category.clone();
        let current = open.iter().find(|(s, _)| *s == side).map(|(_, g)| *g);
        if let Some(g) = current
            && let GroupContent::Tools(members) = &mut out[g].content
            && tools[members[0]].category == category
        {
            members.push(i);
            continue;
        }
        let name = category.clone().unwrap_or_else(|| "tools".into());
        let count = seen.entry(name.clone()).or_default();
        *count += 1;
        let id = if *count == 1 {
            format!("{bench}/{name}")
        } else {
            format!("{bench}/{name}#{count}")
        };
        out.push(Group {
            id,
            label: category.unwrap_or_else(|| "Tools".into()),
            default_row: tool.row as usize,
            end: tool.align_end,
            content: GroupContent::Tools(vec![i]),
        });
        let g = out.len() - 1;
        match open.iter_mut().find(|(s, _)| *s == side) {
            Some(slot) => slot.1 = g,
            None => open.push((side, g)),
        }
    }
    out
}

/// The saved rows with every group this frame knows that they do not
/// list put where its bench puts it: after the group before it, when that
/// one sits on the same row, else at the row's end. Ids of groups not
/// drawn now (another bench's) stay where they are.
fn full_layout(groups: &[Group], saved: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<Vec<String>> = saved
        .iter()
        .map(|row| {
            row.iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        })
        .collect();
    for (i, group) in groups.iter().enumerate() {
        if seen.contains(&group.id) {
            continue;
        }
        while rows.len() <= group.default_row {
            rows.push(Vec::new());
        }
        let row = &mut rows[group.default_row];
        let at = groups[..i]
            .iter()
            .rev()
            .find_map(|before| row.iter().position(|id| *id == before.id))
            .map_or(row.len(), |k| k + 1);
        row.insert(at, group.id.clone());
    }
    rows
}

/// The rows drawn: each the index of its row in the full layout and its
/// groups, left to right. A row with nothing to draw is left out, except
/// the first, which holds the tool search.
fn visible_rows(groups: &[Group], full: &[Vec<String>]) -> Vec<(usize, Vec<usize>)> {
    full.iter()
        .enumerate()
        .filter_map(|(k, row)| {
            let members: Vec<usize> = row
                .iter()
                .filter_map(|id| groups.iter().position(|g| g.id == *id))
                .collect();
            (k == 0 || !members.is_empty()).then_some((k, members))
        })
        .collect()
}

/// Where a dragged group would land.
#[derive(Debug, Clone, PartialEq)]
enum Drop {
    /// On row `row` of the full layout, before `before` (after the last
    /// group of its side when `None`).
    Row { row: usize, before: Option<String> },
    /// On a new row under the others.
    NewRow,
}

/// The full layout with `id` moved to `to`; rows left empty are dropped,
/// the first excepted.
fn moved(
    mut rows: Vec<Vec<String>>,
    id: &str,
    to: &Drop,
    same_side: impl Fn(&str) -> bool,
) -> Vec<Vec<String>> {
    for row in &mut rows {
        row.retain(|x| x != id);
    }
    match to {
        Drop::NewRow => rows.push(vec![id.to_string()]),
        Drop::Row { row, before } => {
            while rows.len() <= *row {
                rows.push(Vec::new());
            }
            let row = &mut rows[*row];
            let at = match before {
                Some(before) => row.iter().position(|x| x == before),
                None => row.iter().rposition(|x| same_side(x)).map(|k| k + 1),
            }
            .unwrap_or(row.len());
            row.insert(at, id.to_string());
        }
    }
    let mut kept = Vec::new();
    for (k, row) in rows.into_iter().enumerate() {
        if k == 0 || !row.is_empty() {
            kept.push(row);
        }
    }
    kept
}

/// The grip a group is dragged by: two columns of dots.
fn grip(ui: &mut egui::Ui, group: &Group) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(8.0, 22.0), egui::Sense::click_and_drag());
    super::widget_map::note(ui.ctx(), || format!("group:{}", group.id), rect);
    let color = if response.dragged() {
        ACCENT
    } else if response.hovered() {
        TEXT2
    } else {
        BORDER_STRONG
    };
    for column in [-1.5_f32, 1.5] {
        for row in [-5.0_f32, 0.0, 5.0] {
            ui.painter()
                .circle_filled(rect.center() + Vec2::new(column, row), 1.0, color);
        }
    }
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    response.on_hover_text(format!(
        "{}: drag to move it along the row, to another row or to a row of its own",
        group.label
    ))
}

/// Draws every toolbar row. Tool activation goes straight into
/// `active_tool`; shell buttons push commands.
pub fn draw_toolbars(
    ui: &mut egui::Ui,
    inputs: ToolbarInputs<'_>,
    active_workbench: &mut ActiveWorkbench,
    active_tool: &mut ActiveTool,
    commands: &mut Vec<UiCommand>,
    open_palette: &mut bool,
    toggle_console: &mut bool,
) {
    let ToolbarInputs {
        registry,
        document,
        host,
        active_document_object,
        show_print_bed,
        show_annotations,
        measuring,
        keymap,
        scripts,
        console_open,
        recording,
        layout,
        workbench_tabs,
        ribbon,
        tool_tabs,
    } = inputs;
    // This copy carries the keys in effect, which the tooltips name.
    let mut tools: Vec<ToolDescriptor> = registry
        .tools_for(&active_workbench.0)
        .map(|t| t.to_vec())
        .unwrap_or_default();
    for tool in &mut tools {
        tool.shortcuts = keymap
            .get(&tool.id)
            .map(|b| b.keys.clone())
            .unwrap_or_default();
    }
    // Enablement and toggle state come from the workbench, evaluated once
    // per frame against a context with the real camera and viewport.
    let (enabled, toggled): (Vec<bool>, Vec<bool>) =
        match registry.workbench_mut(&active_workbench.0) {
            Ok(wb) => {
                let ctx = panel_ctx(document, &host, active_document_object);
                tools
                    .iter()
                    .map(|t| (wb.is_tool_enabled(&t.id, &ctx), wb.tool_toggled(&t.id)))
                    .unzip()
            }
            Err(_) => (vec![false; tools.len()], vec![false; tools.len()]),
        };

    let tools_bench = active_workbench.0.clone();
    let standard = standard_items(show_print_bed, show_annotations, measuring);
    if workbench_tabs {
        egui::Panel::top("workbench_tabs")
            .frame(row_frame())
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("workbench_tabs_scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for wb in REGISTERED_WORKBENCHES.lock().unwrap().iter() {
                                let selected = active_workbench.0 == wb.id;
                                let tab = ui_kit::widgets::Tab::new(&wb.label, selected)
                                    .marked(Some((wb.icon, ACCENT)))
                                    .show(ui);
                                super::widget_map::note(
                                    ui.ctx(),
                                    || format!("tab:{}", wb.id.as_str()),
                                    tab.response.rect,
                                );
                                tab.response.on_hover_text(&wb.description);
                                if tab.selected {
                                    *active_workbench =
                                        ActiveWorkbench(WorkbenchId::from(wb.id.as_str()));
                                }
                            }
                        });
                    });
            });
    }
    if ribbon {
        egui::Panel::top("tool_ribbon")
            .frame(row_frame())
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("ribbon_host")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for item in standard.into_iter().flatten() {
                                let label = with_key(
                                    item.label,
                                    keymap.get(item.binding).and_then(|b| b.keys.first()),
                                );
                                let response = tool_button(
                                    ui,
                                    item.icon,
                                    &label,
                                    TOOLBAR_BUTTON,
                                    ToolButtonState {
                                        enabled: item.command.is_some(),
                                        active: item.on,
                                        planned: item.planned,
                                        menu: false,
                                    },
                                );
                                super::widget_map::note(
                                    ui.ctx(),
                                    || format!("button:{}", item.binding),
                                    response.rect,
                                );
                                if response.clicked()
                                    && let Some(command) = item.command
                                {
                                    commands.push(command);
                                }
                            }
                            separator(ui);
                            scripts_button(
                                ui,
                                ScriptsMenu {
                                    scripts,
                                    console_open,
                                    recording,
                                },
                                keymap,
                                commands,
                                toggle_console,
                            );
                            if !workbench_tabs {
                                workbench_combo(ui, active_workbench);
                            }
                            if search_box(ui, keymap.text("app.palette")).clicked() {
                                *open_palette = true;
                            }
                        });
                    });
                let categories = tool_categories(&tools);
                let selected = tool_tabs
                    .entry(tools_bench.as_str().to_owned())
                    .or_default();
                if !categories.contains(&selected.as_str()) {
                    *selected = categories.first().copied().unwrap_or_default().to_owned();
                }
                egui::ScrollArea::horizontal()
                    .id_salt("ribbon_categories")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for category in &categories {
                                let label = category_label(category);
                                let tab = ui_kit::widgets::Tab::new(&label, selected == category)
                                    .show(ui);
                                super::widget_map::note(
                                    ui.ctx(),
                                    || format!("tooltab:{category}"),
                                    tab.response.rect,
                                );
                                if tab.selected {
                                    *selected = (*category).to_owned();
                                }
                            }
                        });
                    });
                egui::ScrollArea::horizontal()
                    .id_salt((
                        "ribbon_tools",
                        active_workbench.0.as_str(),
                        selected.as_str(),
                    ))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(ui_kit::widgets::ribbon_button_height(ui));
                            let mut previous = None;
                            for (i, tool) in tools
                                .iter()
                                .enumerate()
                                .filter(|(_, tool)| tool_tab(tool) == selected)
                            {
                                let category = tool_category(tool);
                                if previous.is_some_and(|last| last != category) {
                                    separator(ui);
                                }
                                previous = Some(category);
                                draw_tool(
                                    ui,
                                    tool,
                                    &tools,
                                    (enabled[i], toggled[i]),
                                    true,
                                    active_tool,
                                );
                            }
                        });
                    });
            });
        return;
    }
    let mut groups = groups(&standard, &tools, active_workbench.0.as_str());
    if workbench_tabs {
        groups.retain(|group| !matches!(group.content, GroupContent::Workbench));
    }
    let full = full_layout(&groups, layout);
    let rows = visible_rows(&groups, &full);
    let drag_id = egui::Id::new("toolbar_drag");
    let dragging: Option<String> = ui.ctx().data(|d| d.get_temp(drag_id));
    // While a group is dragged, a strip under the rows takes it to a row
    // of its own.
    let strip = dragging.is_some() as usize;
    let height = TOOLBAR * (rows.len() + strip) as f32;

    // What was drawn where, for the drop: each row's rect and each group's.
    let mut row_rects: Vec<(usize, egui::Rect)> = Vec::new();
    let mut group_rects: Vec<(usize, usize, egui::Rect)> = Vec::new();
    let mut drag: Option<(usize, bool)> = None;
    let mut reset = false;
    let mut standard = standard.into_iter().map(Some).collect::<Vec<_>>();

    egui::Panel::top("toolbars")
        .exact_size(height)
        .frame(row_frame())
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut draw_group = |ui: &mut egui::Ui, g: usize, first: bool, reversed: bool| {
                let group = &groups[g];
                let inner = ui.scope(|ui| {
                    let mut grip_response = None;
                    if !reversed {
                        if !first {
                            separator(ui);
                        }
                        grip_response = Some(grip(ui, group));
                        ui.add_space(2.0);
                    }
                    match &group.content {
                        GroupContent::Standard(items) => {
                            for &i in items {
                                let Some(Some(item)) = standard[i].take() else {
                                    continue;
                                };
                                let state = ToolButtonState {
                                    enabled: item.command.is_some(),
                                    active: item.on,
                                    planned: item.planned,
                                    menu: false,
                                };
                                let key = keymap.get(item.binding).and_then(|b| b.keys.first());
                                let label = with_key(item.label, key);
                                let response =
                                    tool_button(ui, item.icon, &label, TOOLBAR_BUTTON, state);
                                super::widget_map::note(
                                    ui.ctx(),
                                    || format!("button:{}", item.binding),
                                    response.rect,
                                );
                                if response.clicked()
                                    && let Some(command) = item.command
                                {
                                    commands.push(command);
                                }
                            }
                        }
                        GroupContent::Scripts => scripts_button(
                            ui,
                            ScriptsMenu {
                                scripts,
                                console_open,
                                recording,
                            },
                            keymap,
                            commands,
                            toggle_console,
                        ),
                        GroupContent::Workbench => workbench_combo(ui, active_workbench),
                        GroupContent::Tools(members) => {
                            let order: Vec<usize> = if reversed {
                                members.iter().rev().copied().collect()
                            } else {
                                members.clone()
                            };
                            for i in order {
                                draw_tool(
                                    ui,
                                    &tools[i],
                                    &tools,
                                    (enabled[i], toggled[i]),
                                    false,
                                    active_tool,
                                );
                            }
                        }
                    }
                    if reversed {
                        ui.add_space(2.0);
                        grip_response = Some(grip(ui, group));
                        if !first {
                            separator(ui);
                        }
                    }
                    grip_response
                });
                (inner.response.rect, inner.inner)
            };
            for (k, (full_row, members)) in rows.iter().enumerate() {
                let row_rect = row(ui, k, |ui| {
                    let mut first = true;
                    for &g in members.iter().filter(|&&g| !groups[g].end) {
                        let (rect, grip) = draw_group(ui, g, first, false);
                        first = false;
                        group_rects.push((*full_row, g, rect));
                        if let Some(grip) = grip {
                            watch_grip(&grip, g, &mut drag, &mut reset);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if k == 0 && search_box(ui, keymap.text("app.palette")).clicked() {
                            *open_palette = true;
                        }
                        let ends: Vec<usize> =
                            members.iter().copied().filter(|&g| groups[g].end).collect();
                        let count = ends.len();
                        for (n, &g) in ends.iter().rev().enumerate() {
                            // The leftmost end group takes no separator.
                            let (rect, grip) = draw_group(ui, g, n + 1 == count, true);
                            group_rects.push((*full_row, g, rect));
                            if let Some(grip) = grip {
                                watch_grip(&grip, g, &mut drag, &mut reset);
                            }
                        }
                    });
                });
                row_rects.push((*full_row, row_rect));
            }
            if strip == 1 {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), TOOLBAR),
                    egui::Sense::hover(),
                );
                let over = ui
                    .ctx()
                    .pointer_interact_pos()
                    .is_some_and(|p| p.y >= rect.top());
                ui.painter().rect(
                    rect.shrink(4.0),
                    RADIUS_MD,
                    if over { ACCENT_FAINT } else { BG2 },
                    egui::Stroke::new(1.0, if over { ACCENT } else { BORDER }),
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop here for a row of its own",
                    sans(FONT_SM),
                    if over { TEXT1 } else { TEXT3 },
                );
                row_rects.push((usize::MAX, rect));
            }
        });

    if reset {
        commands.push(UiCommand::SetToolbarLayout(Vec::new()));
        return;
    }
    let Some((g, stopped)) = drag else {
        return;
    };
    let ctx = ui.ctx().clone();
    ctx.data_mut(|d| {
        if stopped {
            d.remove::<String>(drag_id);
        } else {
            d.insert_temp(drag_id, groups[g].id.clone());
        }
    });
    let Some(pointer) = ctx.pointer_interact_pos() else {
        return;
    };
    let target = drop_target(&groups, g, pointer, &row_rects, &group_rects);
    if stopped {
        if let Some(to) = target {
            let side = groups[g].end;
            let same_side = |id: &str| {
                groups
                    .iter()
                    .find(|x| x.id == id)
                    .is_some_and(|x| x.end == side)
            };
            let rows = moved(full, &groups[g].id, &to, same_side);
            commands.push(UiCommand::SetToolbarLayout(rows));
        }
        return;
    }
    // The group follows the pointer as a card, and a line shows where it
    // would land.
    egui::Area::new(egui::Id::new("toolbar_drag_card"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + Vec2::new(12.0, 12.0))
        .interactable(false)
        .show(&ctx, |ui| {
            ui_kit::widgets::Card::floating().show(ui, |ui| {
                ui.label(
                    RichText::new(&groups[g].label)
                        .font(sans(FONT_SM))
                        .color(TEXT1),
                );
            });
        });
    if let Some(Drop::Row { row, before }) = &target {
        let row_rect = row_rects.iter().find(|(r, _)| r == row).map(|(_, r)| *r);
        let side = groups[g].end;
        let x = match before {
            Some(before) => group_rects
                .iter()
                .find(|(_, b, _)| groups[*b].id == *before)
                .map(|(_, _, r)| r.left() - 2.0),
            None => group_rects
                .iter()
                .filter(|(r, b, _)| r == row && groups[*b].end == side && *b != g)
                .map(|(_, _, r)| r.right() + 2.0)
                .reduce(f32::max),
        };
        if let (Some(row_rect), Some(x)) = (row_rect, x.or(row_rect.map(|r| r.left() + 4.0))) {
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("toolbar_drop"),
            ))
            .vline(
                x,
                row_rect.y_range().shrink(6.0),
                egui::Stroke::new(2.0, ACCENT),
            );
        }
    }
    ctx.request_repaint();
}

/// Notes a group's grip being dragged or let go, and its menu.
fn watch_grip(grip: &egui::Response, g: usize, drag: &mut Option<(usize, bool)>, reset: &mut bool) {
    if grip.drag_stopped() {
        *drag = Some((g, true));
    } else if grip.dragged() {
        *drag = Some((g, false));
    }
    grip.context_menu(|ui| {
        if ui
            .button("Reset toolbars")
            .on_hover_text("Put every group back where its workbench puts it")
            .clicked()
        {
            *reset = true;
            ui.close();
        }
    });
}

/// Where group `g`, dragged, would land with the pointer at `pointer`.
fn drop_target(
    groups: &[Group],
    g: usize,
    pointer: egui::Pos2,
    row_rects: &[(usize, egui::Rect)],
    group_rects: &[(usize, usize, egui::Rect)],
) -> Option<Drop> {
    let (row, _) = row_rects
        .iter()
        .find(|(_, r)| r.y_range().contains(pointer.y))
        .or_else(|| {
            // Above the rows is the first; below them, the last.
            let first = row_rects.first()?;
            let last = row_rects.last()?;
            if pointer.y < first.1.top() {
                Some(first)
            } else if pointer.y > last.1.bottom() {
                Some(last)
            } else {
                None
            }
        })?;
    if *row == usize::MAX {
        return Some(Drop::NewRow);
    }
    let side = groups[g].end;
    let before = group_rects
        .iter()
        .filter(|(r, b, _)| r == row && groups[*b].end == side && *b != g)
        .map(|(_, b, rect)| (*b, rect.center().x))
        .filter(|(_, x)| *x > pointer.x)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(b, _)| groups[b].id.clone());
    Some(Drop::Row { row: *row, before })
}

fn row(ui: &mut egui::Ui, index: usize, add: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TOOLBAR),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, BORDER),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(0.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt(("toolbar_row", index)),
    );
    child.spacing_mut().item_spacing.x = 2.0;
    add(&mut child);
    rect
}

fn separator(ui: &mut egui::Ui) {
    ui.add_space(5.0);
    vseparator(ui, 22.0);
    ui.add_space(5.0);
}

fn search_box(ui: &mut egui::Ui, key: Option<String>) -> egui::Response {
    // Allocate the exact footprint first: a frame grown inside a
    // right-to-left layout reports its size after placement and overflows
    // the row.
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(220.0, INPUT + 2.0), egui::Sense::click());
    ui.painter().rect(
        rect,
        5.0,
        BG2,
        egui::Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = SPACE_2;
    ui_kit::icon::draw(&mut inner, "search", 14.0, TEXT3);
    inner.label(
        RichText::new("Search tools…")
            .font(sans(FONT_SM))
            .color(TEXT3),
    );
    inner.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if let Some(key) = &key {
            ui_kit::widgets::key_chip(ui, key);
        }
    });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Search tools and commands")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str, category: &str, row: u8) -> ToolDescriptor {
        ToolDescriptor::new(id, id, Some(category)).row(row)
    }

    fn bench_tools() -> Vec<ToolDescriptor> {
        vec![
            tool("a", "Make", 0),
            tool("b", "Make", 0),
            tool("c", "Change", 1),
            tool("d", "Make", 1),
            tool("e", "Finish", 0).align_end(),
        ]
    }

    #[test]
    fn ribbon_categories_keep_first_occurrence_and_collect_uncategorized_tools() {
        let mut tools = bench_tools();
        tools.extend([
            tool("curve", "geometry.curves", 0),
            tool("line", "geometry.basic", 1),
            tool("rule", "constraints.geometric", 0),
            tool("f", "", 0),
            ToolDescriptor::new("g", "g", None::<String>),
        ]);
        assert_eq!(
            tool_categories(&tools),
            [
                "Make",
                "Change",
                "Finish",
                "geometry",
                "constraints",
                "General"
            ]
        );
        assert_eq!(category_label("geometry"), "Geometry");
        assert!(tool_categories(&[]).is_empty());
    }

    fn ids(rows: &[Vec<String>]) -> Vec<Vec<&str>> {
        rows.iter()
            .map(|r| r.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn groups_follow_the_bench_s_rows_and_categories() {
        let standard = standard_items(false, false, false);
        let groups = groups(&standard, &bench_tools(), "b");
        let names: Vec<(&str, usize, bool)> = groups
            .iter()
            .map(|g| (g.id.as_str(), g.default_row, g.end))
            .collect();
        assert_eq!(
            names,
            [
                ("std.file", 0, false),
                ("std.history", 0, false),
                ("std.clipboard", 0, false),
                ("std.view", 0, false),
                ("std.scripts", 0, false),
                ("std.workbench", 0, false),
                ("b/Make", 0, false),
                ("b/Change", 1, false),
                ("b/Make#2", 1, false),
                ("b/Finish", 0, true),
            ]
        );
        // Every standard button is in exactly one group.
        let held: usize = groups
            .iter()
            .map(|g| match &g.content {
                GroupContent::Standard(items) => items.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(held, standard.iter().flatten().count());
    }

    #[test]
    fn nothing_saved_is_the_bench_s_own_layout() {
        let groups = groups(&standard_items(false, false, false), &bench_tools(), "b");
        let full = full_layout(&groups, &[]);
        assert_eq!(
            ids(&full),
            [
                vec![
                    "std.file",
                    "std.history",
                    "std.clipboard",
                    "std.view",
                    "std.scripts",
                    "std.workbench",
                    "b/Make",
                    "b/Finish",
                ],
                vec!["b/Change", "b/Make#2"],
            ]
        );
    }

    #[test]
    fn a_saved_layout_keeps_groups_it_cannot_see_and_places_new_ones() {
        let groups = groups(&standard_items(false, false, false), &bench_tools(), "b");
        let saved = vec![
            vec!["std.workbench".to_string(), "other/Tools".to_string()],
            vec!["b/Make".to_string(), "std.file".to_string()],
        ];
        let full = full_layout(&groups, &saved);
        assert_eq!(full[0][0], "std.workbench");
        assert!(full[0].iter().any(|g| g == "other/Tools"));
        assert_eq!(full[1][0], "b/Make");
        // A group the layout does not list goes on its own row, after the
        // group before it there.
        let history = full[0].iter().position(|g| g == "std.history").unwrap();
        assert_eq!(full[0][history - 1], "other/Tools");
        // Only drawn groups draw; the other bench's is skipped.
        let rows = visible_rows(&groups, &full);
        assert!(
            rows.iter()
                .all(|(_, members)| members.iter().all(|&g| g < groups.len()))
        );
        let drawn: usize = rows.iter().map(|(_, m)| m.len()).sum();
        assert_eq!(drawn, groups.len());
    }

    #[test]
    fn a_group_moves_along_its_row_to_another_and_to_a_new_one() {
        let rows = vec![
            vec!["x".to_string(), "y".to_string(), "z".to_string()],
            vec!["w".to_string()],
        ];
        let any = |_: &str| true;
        let to_front = moved(
            rows.clone(),
            "z",
            &Drop::Row {
                row: 0,
                before: Some("x".into()),
            },
            any,
        );
        assert_eq!(ids(&to_front), [vec!["z", "x", "y"], vec!["w"]]);
        // The last group of a row moved out takes its row with it.
        let up = moved(
            rows.clone(),
            "w",
            &Drop::Row {
                row: 0,
                before: None,
            },
            any,
        );
        assert_eq!(ids(&up), [vec!["x", "y", "z", "w"]]);
        let alone = moved(rows, "y", &Drop::NewRow, any);
        assert_eq!(ids(&alone), [vec!["x", "z"], vec!["w"], vec!["y"]]);
    }
}
