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

fn standard_items(show_print_bed: bool, measuring: bool) -> Vec<Option<ShellItem>> {
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
}

/// The Scripts button: the scripts folder's scripts, then running a file,
/// the console and the folder itself.
/// What the Scripts button's menu shows.
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
    Popup::menu(&response).show(|ui| {
        // One row: an icon, the label and its key; whether it was clicked.
        let entry = |ui: &mut egui::Ui, icon: &str, label: &str, key: Option<String>, tip: &str| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                ui_kit::icon::draw(ui, icon, 14.0, TEXT2);
                let mut button =
                    egui::Button::new(RichText::new(label).font(sans(FONT_SM))).frame(false);
                if let Some(key) = key {
                    button =
                        button.shortcut_text(RichText::new(key).font(sans(FONT_XS)).color(TEXT3));
                }
                let response = ui.add(button);
                if tip.is_empty() {
                    response.clicked()
                } else {
                    response.on_hover_text(tip).clicked()
                }
            })
            .inner
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
    enabled: bool,
    toggled: bool,
    active_tool: &mut ActiveTool,
) {
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
    let response = tool_button(ui, icon, &label, TOOLBAR_BUTTON, state);

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
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                ui_kit::icon::draw(ui, variant.icon, 16.0, TEXT2);
                let text = RichText::new(variant.label).font(sans(FONT_SM));
                ui.label(if variant.planned.is_some() {
                    text.color(TEXT3)
                } else {
                    text.color(TEXT1)
                });
            });
            let r = ui.interact(row.response.rect, ui.id().with(i), egui::Sense::click());
            if let Some(note) = variant.planned {
                r.on_hover_text(format!("{} (planned)\n{note}", variant.label));
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
    let response = response.on_hover_text(if current.1.is_empty() {
        "Active workbench".to_string()
    } else {
        format!("Active workbench: {}", current.1)
    });
    Popup::menu(&response).show(|ui| {
        for wb in workbenches.iter() {
            let target = ActiveWorkbench(WorkbenchId::from(wb.id.as_str()));
            let selected = *active_workbench == target;
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                ui_kit::icon::draw(ui, wb.icon, 16.0, if selected { ACCENT } else { TEXT2 });
                ui.label(RichText::new(&wb.label).font(sans(FONT_SM)).color(TEXT1));
            });
            let r = ui
                .interact(
                    row.response.rect,
                    ui.id().with(&wb.id),
                    egui::Sense::click(),
                )
                .on_hover_text(&wb.description);
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
        measuring,
        keymap,
        scripts,
        console_open,
        recording,
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

    // Row 0 always exists; the bench adds rows 1.. as it declares them.
    let rows = tools
        .iter()
        .map(|t| t.row as usize + 1)
        .max()
        .unwrap_or(1)
        .max(1);
    let height = TOOLBAR * (rows as f32);

    egui::Panel::top("toolbars")
        .exact_size(height)
        .frame(row_frame())
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Row 0: standard tools, the workbench switcher, the bench's
            // row-0 tools, then the tool search at the right.
            row(ui, 0, |ui| {
                for item in standard_items(show_print_bed, measuring) {
                    match item {
                        None => separator(ui),
                        Some(item) => {
                            let state = ToolButtonState {
                                enabled: item.command.is_some(),
                                active: item.on,
                                planned: item.planned,
                                menu: false,
                            };
                            let key = keymap.get(item.binding).and_then(|b| b.keys.first());
                            let label = with_key(item.label, key);
                            if tool_button(ui, item.icon, &label, TOOLBAR_BUTTON, state).clicked()
                                && let Some(command) = item.command
                            {
                                commands.push(command);
                            }
                        }
                    }
                }
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
                separator(ui);
                workbench_combo(ui, active_workbench);
                separator(ui);
                draw_tools_of_row(ui, &tools, 0, &enabled, &toggled, active_tool, false);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if search_box(ui, keymap.text("app.palette")).clicked() {
                        *open_palette = true;
                    }
                    draw_tools_of_row(ui, &tools, 0, &enabled, &toggled, active_tool, true);
                });
            });
            for r in 1..rows {
                row(ui, r, |ui| {
                    draw_tools_of_row(ui, &tools, r as u8, &enabled, &toggled, active_tool, false);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        draw_tools_of_row(
                            ui,
                            &tools,
                            r as u8,
                            &enabled,
                            &toggled,
                            active_tool,
                            true,
                        );
                    });
                });
            }
        });
}

fn row(ui: &mut egui::Ui, index: usize, add: impl FnOnce(&mut egui::Ui)) {
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
}

fn separator(ui: &mut egui::Ui) {
    ui.add_space(5.0);
    vseparator(ui, 22.0);
    ui.add_space(5.0);
}

fn draw_tools_of_row(
    ui: &mut egui::Ui,
    tools: &[ToolDescriptor],
    row: u8,
    enabled: &[bool],
    toggled: &[bool],
    active_tool: &mut ActiveTool,
    end_aligned: bool,
) {
    let mut last_category: Option<&str> = None;
    let mut first = true;
    for (i, tool) in tools.iter().enumerate() {
        if tool.row != row || tool.align_end != end_aligned {
            continue;
        }
        let category = tool.category.as_deref();
        if !first && category != last_category {
            separator(ui);
        }
        first = false;
        last_category = category;
        draw_tool(ui, tool, tools, enabled[i], toggled[i], active_tool);
    }
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
    response.on_hover_text("Search tools and commands")
}
