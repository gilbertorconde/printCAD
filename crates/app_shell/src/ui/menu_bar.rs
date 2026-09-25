//! The menu bar: app mark, the menus, and the document name at the right.

use core_document::{DocumentService, WorkbenchId};
use egui::RichText;
use settings::ProjectionMode;
use ui_kit::tokens::*;
use ui_kit::widgets::mono_label;
use ui_kit::{sans, sans_semibold};
use workbenches::REGISTERED_WORKBENCHES;

use super::toolbar::activate_tool;
use super::{ActiveTool, ActiveWorkbench, FileCommand, Screen, UiCommand};
use crate::orientation_cube::CameraSnapView;

/// What the menu bar reads this frame.
pub struct MenuBarInputs<'a> {
    /// The keys each command answers to, shown beside its row.
    pub keymap: &'a super::keymap::Keymap,
    pub registry: &'a DocumentService,
    pub document_name: &'a str,
    pub document_dirty: bool,
    /// "Body › Sketch001" while a sketch is being edited.
    pub breadcrumb: Option<&'a str>,
    pub show_log_panel: bool,
    /// Imported annotations are drawn over the scene.
    pub show_annotations: bool,
    pub show_console: bool,
    pub show_assistant: bool,
    pub projection: ProjectionMode,
    pub draw_style: settings::DrawStyle,
    pub recent: &'a [settings::recent::RecentEntry],
    pub screen: Screen,
    /// The tab on screen has nothing in it yet: leaving its start page
    /// means starting a document, not returning to one.
    pub active_tab_blank: bool,
    /// The tab on screen, for Close tab.
    pub active_tab: Option<uuid::Uuid>,
    /// The scripts folder's scripts.
    pub scripts: &'a [crate::script_library::ScriptEntry],
    /// A recording is on.
    pub recording: bool,
}

/// Menu-driven requests that are UI-local state rather than app commands.
#[derive(Default)]
pub struct MenuBarResult {
    pub show_preferences: bool,
    pub show_about: bool,
    pub open_palette: bool,
    pub toggle_console: bool,
    pub toggle_assistant: bool,
}

fn item(ui: &mut egui::Ui, label: &str, shortcut: Option<String>) -> bool {
    let mut button = egui::Button::new(RichText::new(label).font(sans(FONT_SM)));
    if let Some(sc) = shortcut {
        button = button.shortcut_text(sc);
    }
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// One of a set of choices, the current one marked, with its key.
fn choice(ui: &mut egui::Ui, on: bool, label: &str, shortcut: Option<String>) -> bool {
    let mut button = egui::Button::selectable(on, RichText::new(label).font(sans(FONT_SM)));
    if let Some(sc) = shortcut {
        button = button.shortcut_text(sc);
    }
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// A row that needs a document on screen. Disabled rows say what they are
/// waiting for rather than doing nothing when clicked.
fn item_needing_document(
    ui: &mut egui::Ui,
    label: &str,
    shortcut: Option<String>,
    have_document: bool,
) -> bool {
    let mut button = egui::Button::new(RichText::new(label).font(sans(FONT_SM)));
    if let Some(sc) = shortcut {
        button = button.shortcut_text(sc);
    }
    let response = ui
        .add_enabled(have_document, button)
        .on_disabled_hover_text(format!("{label}: open or create a document first"));
    let clicked = response.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn menu_title(label: &str) -> RichText {
    RichText::new(label).font(sans(FONT_SM)).color(TEXT2)
}

/// Draws the bar and consumes the global shortcuts.
pub fn draw_menu_bar(
    ui: &mut egui::Ui,
    inputs: MenuBarInputs<'_>,
    active_workbench: &mut ActiveWorkbench,
    active_tool: &mut ActiveTool,
    commands: &mut Vec<UiCommand>,
) -> MenuBarResult {
    let mut result = MenuBarResult::default();
    // The start page has no document and no viewport: the rows that act on
    // one say so instead of doing nothing, and their shortcuts hold their
    // fire.
    let have_document = inputs.screen == Screen::Workspace;

    let key = |id: &str| inputs.keymap.text(id);

    egui::Panel::top("menu_bar")
        .exact_size(MENU_BAR)
        .frame(
            egui::Frame::new()
                .fill(BG0)
                .inner_margin(egui::Margin::symmetric(8, 0)),
        )
        .show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                egui::Stroke::new(1.0, BORDER),
            );
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    ui_kit::icon::draw(ui, "workbench-print", 16.0, ACCENT);
                    ui.label(
                        RichText::new("printCAD")
                            .font(sans_semibold(FONT_SM))
                            .color(TEXT1),
                    );
                });
                ui.add_space(6.0);
                ui_kit::widgets::vseparator(ui, MENU_BAR);
                ui.add_space(4.0);

                egui::MenuBar::new().ui(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_2;
                    ui.spacing_mut().button_padding = egui::vec2(SPACE_2, 4.0);
                    ui.menu_button(menu_title("File"), |ui| {
                        if item(ui, "New", key("file.new")) {
                            commands.push(UiCommand::File(FileCommand::New));
                        }
                        if item(ui, "Open…", key("file.open")) {
                            commands.push(UiCommand::File(FileCommand::Open));
                        }
                        if item(ui, "New tab", key("tab.new")) {
                            commands.push(UiCommand::NewTab);
                        }
                        ui.menu_button(RichText::new("Open recent").font(sans(FONT_SM)), |ui| {
                            if inputs.recent.is_empty() {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(
                                        RichText::new("Nothing yet").font(sans(FONT_SM)),
                                    ),
                                );
                            }
                            for entry in inputs.recent {
                                if item(ui, &entry.name(), None) {
                                    commands.push(UiCommand::OpenRecent(entry.path.clone()));
                                    ui.close();
                                }
                            }
                        });
                        ui.separator();
                        if item_needing_document(ui, "Save", key("file.save"), have_document) {
                            commands.push(UiCommand::File(FileCommand::Save));
                        }
                        if item_needing_document(ui, "Save As…", key("file.save_as"), have_document)
                        {
                            commands.push(UiCommand::File(FileCommand::SaveAs));
                        }
                        ui.separator();
                        if item_needing_document(ui, "Import…", key("file.import"), have_document)
                        {
                            commands.push(UiCommand::File(FileCommand::ImportStep));
                        }
                        if item_needing_document(ui, "Export…", key("file.export"), have_document)
                        {
                            commands.push(UiCommand::File(FileCommand::Export));
                        }
                        if item_needing_document(
                            ui,
                            "Send to slicer",
                            key("file.send_to_slicer"),
                            have_document,
                        ) {
                            commands.push(UiCommand::File(FileCommand::SendToSlicer));
                        }
                        ui.separator();
                        if let Some(active) = inputs.active_tab
                            && item(ui, "Close tab", key("tab.close"))
                        {
                            commands.push(UiCommand::CloseTab(active));
                        }
                        ui.separator();
                        if inputs.screen == Screen::Start {
                            if item(ui, "Workspace", None) {
                                commands.push(if inputs.active_tab_blank {
                                    UiCommand::StartNew(super::StartKind::Landing)
                                } else {
                                    UiCommand::ShowWorkspace
                                });
                            }
                        } else if item(ui, "Start page", None) {
                            commands.push(UiCommand::ShowStartPage);
                        }
                        ui.separator();
                        if item(ui, "Preferences…", key("app.preferences")) {
                            result.show_preferences = true;
                        }
                        ui.separator();
                        if item(ui, "Quit", key("app.quit")) {
                            commands.push(UiCommand::Quit);
                        }
                    });
                    ui.menu_button(menu_title("Edit"), |ui| {
                        if item_needing_document(ui, "Undo", key("edit.undo"), have_document) {
                            commands.push(UiCommand::Undo);
                        }
                        if item_needing_document(ui, "Redo", key("edit.redo"), have_document) {
                            commands.push(UiCommand::Redo);
                        }
                        ui.separator();
                        for (label, command, id) in [
                            ("Cut", super::EditCommand::Cut, "edit.cut"),
                            ("Copy", super::EditCommand::Copy, "edit.copy"),
                            ("Paste", super::EditCommand::Paste, "edit.paste"),
                        ] {
                            if item_needing_document(ui, label, key(id), have_document) {
                                commands.push(UiCommand::Edit(command));
                            }
                        }
                        ui.separator();
                        if item(ui, "Preferences…", key("app.preferences")) {
                            result.show_preferences = true;
                        }
                    });
                    ui.menu_button(menu_title("View"), |ui| {
                        if item(ui, "Fit view", key("view.fit_all")) {
                            commands.push(UiCommand::FitView);
                        }
                        if item_needing_document(
                            ui,
                            "Fit selection",
                            key("view.fit_selection"),
                            have_document,
                        ) {
                            commands.push(UiCommand::FitSelection);
                        }
                        ui.separator();
                        ui.menu_button(RichText::new("Standard views").font(sans(FONT_SM)), |ui| {
                            for (label, view, id) in [
                                ("Isometric", CameraSnapView::FrontTopRight, "view.isometric"),
                                ("Front", CameraSnapView::Front, "view.front"),
                                ("Top", CameraSnapView::Top, "view.top"),
                                ("Right", CameraSnapView::Right, "view.right"),
                                ("Rear", CameraSnapView::Rear, "view.rear"),
                                ("Bottom", CameraSnapView::Bottom, "view.bottom"),
                                ("Left", CameraSnapView::Left, "view.left"),
                            ] {
                                if item(ui, label, key(id)) {
                                    commands.push(UiCommand::CameraSnap(view));
                                }
                            }
                        });
                        let ortho = inputs.projection == ProjectionMode::Orthographic;
                        if choice(ui, ortho, "Orthographic", key("view.orthographic")) {
                            commands.push(UiCommand::SetProjection(ProjectionMode::Orthographic));
                        }
                        if choice(ui, !ortho, "Perspective", key("view.perspective")) {
                            commands.push(UiCommand::SetProjection(ProjectionMode::Perspective));
                        }
                        ui.separator();
                        ui.menu_button(RichText::new("Draw style").font(sans(FONT_SM)), |ui| {
                            for style in settings::DrawStyle::ALL {
                                let id = match style {
                                    settings::DrawStyle::ShadedEdges => "view.shaded_edges",
                                    settings::DrawStyle::Shaded => "view.shaded",
                                    settings::DrawStyle::Wireframe => "view.wireframe",
                                };
                                if choice(ui, inputs.draw_style == style, style.label(), key(id)) {
                                    commands.push(UiCommand::SetDrawStyle(style));
                                }
                            }
                        });
                        if ui
                            .checkbox(
                                &mut inputs.show_annotations.clone(),
                                RichText::new("Annotations").font(sans(FONT_SM)),
                            )
                            .on_hover_text(
                                "Dimensions, tolerances, datums and notes \
                                 carried by imported files",
                            )
                            .clicked()
                        {
                            commands.push(UiCommand::ToggleAnnotations);
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .checkbox(
                                &mut inputs.show_log_panel.clone(),
                                RichText::new("Log panel").font(sans(FONT_SM)),
                            )
                            .clicked()
                        {
                            commands.push(UiCommand::ToggleLogPanel);
                            ui.close();
                        }
                        if ui
                            .checkbox(
                                &mut inputs.show_console.clone(),
                                RichText::new("Console").font(sans(FONT_SM)),
                            )
                            .clicked()
                        {
                            result.toggle_console = true;
                            ui.close();
                        }
                        if ui
                            .checkbox(
                                &mut inputs.show_assistant.clone(),
                                RichText::new("Assistant").font(sans(FONT_SM)),
                            )
                            .clicked()
                        {
                            result.toggle_assistant = true;
                            ui.close();
                        }
                        ui.separator();
                        ui.menu_button(RichText::new("Workbench").font(sans(FONT_SM)), |ui| {
                            let workbenches = REGISTERED_WORKBENCHES.lock().unwrap();
                            for wb in workbenches.iter() {
                                let target = ActiveWorkbench(WorkbenchId::from(wb.id.as_str()));
                                let is_active = *active_workbench == target;
                                if ui
                                    .selectable_label(
                                        is_active,
                                        RichText::new(&wb.label).font(sans(FONT_SM)),
                                    )
                                    .on_hover_text(&wb.description)
                                    .clicked()
                                {
                                    *active_workbench = target;
                                    ui.close();
                                }
                            }
                        });
                    });

                    // The active workbench's menu, listing its tools. Another
                    // bench's menu would name work that belongs to a bench
                    // the user is not in; View › Workbench is how they get
                    // there, and the start page has no bench at all.
                    let benches: Vec<(WorkbenchId, String)> = REGISTERED_WORKBENCHES
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|wb| {
                            have_document && active_workbench.0 == WorkbenchId::from(wb.id.as_str())
                        })
                        .map(|wb| (WorkbenchId::from(wb.id.as_str()), wb.label.clone()))
                        .collect();
                    for (wb_id, label) in benches {
                        let tools: Vec<_> = inputs
                            .registry
                            .tools_for(&wb_id)
                            .map(|t| t.to_vec())
                            .unwrap_or_default();
                        ui.menu_button(menu_title(&label), |ui| {
                            let mut last_category: Option<String> = None;
                            for tool in &tools {
                                if last_category.is_some() && tool.category != last_category {
                                    ui.separator();
                                }
                                last_category = tool.category.clone();
                                let row = ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = SPACE_2;
                                    ui_kit::icon::draw(
                                        ui,
                                        tool.icon.unwrap_or("more"),
                                        16.0,
                                        if tool.planned.is_some() { TEXT3 } else { TEXT2 },
                                    );
                                    let text = RichText::new(&tool.label).font(sans(FONT_SM));
                                    ui.label(if tool.planned.is_some() {
                                        text.color(TEXT3)
                                    } else {
                                        text.color(TEXT1)
                                    });
                                    if let Some(keys) = key(&tool.id) {
                                        ui.add_space(SPACE_3);
                                        mono_label(ui, keys, FONT_XS, TEXT3);
                                    }
                                });
                                let r = ui.interact(
                                    row.response.rect,
                                    ui.id().with(&tool.id),
                                    egui::Sense::click(),
                                );
                                if let Some(note) = tool.planned {
                                    r.on_hover_text(format!("{} (planned)\n{note}", tool.label));
                                    continue;
                                }
                                if r.clicked() {
                                    activate_tool(active_tool, &tools, tool, &tool.id);
                                    ui.close();
                                }
                            }
                            if tools.is_empty() {
                                ui.label(
                                    RichText::new("No tools").font(sans(FONT_SM)).color(TEXT3),
                                );
                            }
                        });
                    }

                    ui.menu_button(menu_title("Scripts"), |ui| {
                        if item(ui, "Run script…", key("file.run_script")) {
                            commands.push(UiCommand::File(super::FileCommand::RunScript));
                        }
                        if choice(ui, inputs.show_console, "Console", key("app.console")) {
                            result.toggle_console = true;
                        }
                        let record = if inputs.recording {
                            "Stop recording"
                        } else {
                            "Record…"
                        };
                        if item(ui, record, key("app.record")) {
                            commands.push(UiCommand::ToggleRecording);
                        }
                        ui.separator();
                        if inputs.scripts.is_empty() {
                            ui.label(
                                RichText::new("No scripts in the scripts folder yet")
                                    .font(sans(FONT_SM))
                                    .color(TEXT3),
                            );
                        }
                        for script in inputs.scripts {
                            let mut button =
                                egui::Button::new(RichText::new(&script.name).font(sans(FONT_SM)));
                            if let Some(k) = key(&script.id) {
                                button = button.shortcut_text(k);
                            }
                            let response = ui.add(button);
                            let response = match &script.about {
                                Some(about) => response.on_hover_text(about),
                                None => response,
                            };
                            if response.clicked() {
                                commands.push(UiCommand::RunScriptFile(script.path.clone()));
                                ui.close();
                            }
                        }
                        ui.separator();
                        if item(ui, "New script", None) {
                            commands.push(UiCommand::NewScript);
                        }
                        if item(ui, "Open scripts folder", None) {
                            commands.push(UiCommand::EditScript(None));
                        }
                    });

                    ui.menu_button(menu_title("Windows"), |ui| {
                        if ui
                            .checkbox(
                                &mut inputs.show_log_panel.clone(),
                                RichText::new("Log panel").font(sans(FONT_SM)),
                            )
                            .clicked()
                        {
                            commands.push(UiCommand::ToggleLogPanel);
                            ui.close();
                        }
                        if ui
                            .checkbox(
                                &mut inputs.show_console.clone(),
                                RichText::new("Console").font(sans(FONT_SM)),
                            )
                            .clicked()
                        {
                            result.toggle_console = true;
                            ui.close();
                        }
                        if ui
                            .checkbox(
                                &mut inputs.show_assistant.clone(),
                                RichText::new("Assistant").font(sans(FONT_SM)),
                            )
                            .clicked()
                        {
                            result.toggle_assistant = true;
                            ui.close();
                        }
                    });
                    ui.menu_button(menu_title("Help"), |ui| {
                        if item(ui, "About printCAD", None) {
                            result.show_about = true;
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(4.0);
                    if inputs.document_dirty {
                        let (rect, r) =
                            ui.allocate_exact_size(egui::Vec2::splat(6.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 3.0, WARNING);
                        r.on_hover_text("Unsaved changes");
                    }
                    let title = match inputs.breadcrumb {
                        Some(crumb) => format!("{} › {crumb}", inputs.document_name),
                        None => inputs.document_name.to_owned(),
                    };
                    mono_label(ui, title, FONT_XS, TEXT3);
                });
            });
        });
    result
}
