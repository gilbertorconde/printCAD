//! The menu bar: app mark, the menus, and the document name at the right.

use core_document::{DocumentService, WorkbenchId};
use egui::{Key, KeyboardShortcut, Modifiers, RichText};
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
    pub registry: &'a DocumentService,
    pub document_name: &'a str,
    pub document_dirty: bool,
    /// "Body › Sketch001" while a sketch is being edited.
    pub breadcrumb: Option<&'a str>,
    pub show_log_panel: bool,
    pub projection: ProjectionMode,
    pub recent: &'a [settings::recent::RecentEntry],
    pub screen: Screen,
}

/// Menu-driven requests that are UI-local state rather than app commands.
#[derive(Default)]
pub struct MenuBarResult {
    pub show_preferences: bool,
    pub show_about: bool,
    pub open_palette: bool,
}

fn shortcut(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

fn item(ui: &mut egui::Ui, label: &str, shortcut: Option<&KeyboardShortcut>) -> bool {
    let mut button = egui::Button::new(RichText::new(label).font(sans(FONT_SM)));
    if let Some(sc) = shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(sc));
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
    shortcut: Option<&KeyboardShortcut>,
    have_document: bool,
) -> bool {
    let mut button = egui::Button::new(RichText::new(label).font(sans(FONT_SM)));
    if let Some(sc) = shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(sc));
    }
    let response = ui
        .add_enabled(have_document, button)
        .on_disabled_hover_text(format!("{label} — open or create a document first"));
    let clicked = response.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn planned_item(ui: &mut egui::Ui, label: &str, note: &str) {
    ui.add_enabled(
        false,
        egui::Button::new(RichText::new(label).font(sans(FONT_SM))),
    )
    .on_disabled_hover_text(format!("{label} — planned\n{note}"));
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

    // `Modifiers::COMMAND` maps to Ctrl on Linux and Windows and Cmd on
    // macOS.
    let sc_new = shortcut(Modifiers::COMMAND, Key::N);
    let sc_open = shortcut(Modifiers::COMMAND, Key::O);
    let sc_save = shortcut(Modifiers::COMMAND, Key::S);
    let sc_save_as = shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    let sc_import = shortcut(Modifiers::COMMAND, Key::I);
    let sc_quit = shortcut(Modifiers::COMMAND, Key::Q);
    let sc_fit = shortcut(Modifiers::NONE, Key::F);
    let sc_undo = shortcut(Modifiers::COMMAND, Key::Z);
    let sc_redo = shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    let sc_redo_y = shortcut(Modifiers::COMMAND, Key::Y);
    let sc_palette = shortcut(Modifiers::COMMAND, Key::K);
    let sc_prefs = shortcut(Modifiers::COMMAND, Key::Comma);

    // Consume shortcuts up-front so a menu row clicked in the same frame
    // does not double-fire. Shift variants come before their plain form.
    let typing = ui.ctx().egui_wants_keyboard_input();
    ui.ctx().input_mut(|i| {
        if i.consume_shortcut(&sc_new) {
            commands.push(UiCommand::File(FileCommand::New));
        }
        if i.consume_shortcut(&sc_open) {
            commands.push(UiCommand::File(FileCommand::Open));
        }
        if i.consume_shortcut(&sc_save_as) && have_document {
            commands.push(UiCommand::File(FileCommand::SaveAs));
        }
        if i.consume_shortcut(&sc_save) && have_document {
            commands.push(UiCommand::File(FileCommand::Save));
        }
        if i.consume_shortcut(&sc_import) && have_document {
            commands.push(UiCommand::File(FileCommand::ImportStep));
        }
        if i.consume_shortcut(&sc_quit) {
            commands.push(UiCommand::Quit);
        }
        if i.consume_shortcut(&sc_palette) {
            result.open_palette = true;
        }
        if i.consume_shortcut(&sc_prefs) {
            result.show_preferences = true;
        }
        // Text fields own their own undo and the letter F.
        if !typing {
            let redo = i.consume_shortcut(&sc_redo) || i.consume_shortcut(&sc_redo_y);
            if redo && have_document {
                commands.push(UiCommand::Redo);
            }
            if i.consume_shortcut(&sc_undo) && have_document {
                commands.push(UiCommand::Undo);
            }
            if i.consume_shortcut(&sc_fit) && have_document {
                commands.push(UiCommand::FitView);
            }
        }
    });

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
                        if item(ui, "New", Some(&sc_new)) {
                            commands.push(UiCommand::File(FileCommand::New));
                        }
                        if item(ui, "Open…", Some(&sc_open)) {
                            commands.push(UiCommand::File(FileCommand::Open));
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
                        if item_needing_document(ui, "Save", Some(&sc_save), have_document) {
                            commands.push(UiCommand::File(FileCommand::Save));
                        }
                        if item_needing_document(ui, "Save As…", Some(&sc_save_as), have_document)
                        {
                            commands.push(UiCommand::File(FileCommand::SaveAs));
                        }
                        ui.separator();
                        if item_needing_document(
                            ui,
                            "Import STEP…",
                            Some(&sc_import),
                            have_document,
                        ) {
                            commands.push(UiCommand::File(FileCommand::ImportStep));
                        }
                        ui.separator();
                        if inputs.screen == Screen::Start {
                            if item(ui, "Workspace", None) {
                                commands.push(UiCommand::StartNew(super::StartKind::PartDesign));
                            }
                        } else if item(ui, "Start page", None) {
                            commands.push(UiCommand::ShowStartPage);
                        }
                        ui.separator();
                        if item(ui, "Preferences…", Some(&sc_prefs)) {
                            result.show_preferences = true;
                        }
                        ui.separator();
                        if item(ui, "Quit", Some(&sc_quit)) {
                            commands.push(UiCommand::Quit);
                        }
                    });
                    ui.menu_button(menu_title("Edit"), |ui| {
                        if item_needing_document(ui, "Undo", Some(&sc_undo), have_document) {
                            commands.push(UiCommand::Undo);
                        }
                        if item_needing_document(ui, "Redo", Some(&sc_redo), have_document) {
                            commands.push(UiCommand::Redo);
                        }
                        ui.separator();
                        // PLANNED: clipboard operations on features and
                        // sketch geometry.
                        planned_item(ui, "Cut", "moves the selection to the clipboard");
                        planned_item(ui, "Copy", "copies the selection");
                        planned_item(ui, "Paste", "pastes the clipboard");
                        ui.separator();
                        if item(ui, "Preferences…", Some(&sc_prefs)) {
                            result.show_preferences = true;
                        }
                    });
                    ui.menu_button(menu_title("View"), |ui| {
                        if item(ui, "Fit view", Some(&sc_fit)) {
                            commands.push(UiCommand::FitView);
                        }
                        // PLANNED: frame the selection instead of the scene.
                        planned_item(ui, "Fit selection", "frames the selected bodies");
                        ui.separator();
                        ui.menu_button(RichText::new("Standard views").font(sans(FONT_SM)), |ui| {
                            for (label, view) in [
                                ("Isometric", CameraSnapView::FrontTopRight),
                                ("Front", CameraSnapView::Front),
                                ("Top", CameraSnapView::Top),
                                ("Right", CameraSnapView::Right),
                                ("Rear", CameraSnapView::Rear),
                                ("Bottom", CameraSnapView::Bottom),
                                ("Left", CameraSnapView::Left),
                            ] {
                                if item(ui, label, None) {
                                    commands.push(UiCommand::CameraSnap(view));
                                }
                            }
                        });
                        let ortho = inputs.projection == ProjectionMode::Orthographic;
                        if ui
                            .radio(ortho, RichText::new("Orthographic").font(sans(FONT_SM)))
                            .clicked()
                        {
                            commands.push(UiCommand::SetProjection(ProjectionMode::Orthographic));
                            ui.close();
                        }
                        if ui
                            .radio(!ortho, RichText::new("Perspective").font(sans(FONT_SM)))
                            .clicked()
                        {
                            commands.push(UiCommand::SetProjection(ProjectionMode::Perspective));
                            ui.close();
                        }
                        ui.separator();
                        // PLANNED: scene-wide draw styles.
                        planned_item(ui, "Draw style", "shaded, wireframe or flat lines");
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
                                });
                                let r = ui.interact(
                                    row.response.rect,
                                    ui.id().with(&tool.id),
                                    egui::Sense::click(),
                                );
                                if let Some(note) = tool.planned {
                                    r.on_hover_text(format!("{} — planned\n{note}", tool.label));
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
