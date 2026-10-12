//! The viewport's context menu: what a right click on a body offers.
//!
//! The host owns the request and passes it in each frame; this draws it
//! where the click landed and answers with commands, the last of which
//! closes it. A click anywhere else, or Escape, closes it too.

use core_document::Document;
use egui::{Area, Context, Order, RichText};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::Card;

use super::UiCommand;
use super::keymap::Keymap;

const MENU_WIDTH: f32 = 150.0;

/// A right click on a body, and where it landed, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportMenu {
    pub body: core_document::BodyId,
    pub at: [f32; 2],
    /// The face it was opened on, when there was one: its index in the
    /// body's mesh.
    pub face: Option<u32>,
    /// The point it was opened on, where the body sits in the document.
    pub point: Option<[f32; 3]>,
    /// The last tool started, by name, which the menu offers to repeat.
    pub repeat: Option<String>,
}

pub fn draw(
    ctx: &Context,
    menu: &ViewportMenu,
    document: &Document,
    registry: &core_document::DocumentService,
    keymap: &Keymap,
    commands: &mut Vec<UiCommand>,
    local: &mut Option<super::MenuLocal>,
) {
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        commands.push(UiCommand::CloseViewportMenu);
        return;
    }
    let name = document
        .bodies()
        .iter()
        .find(|b| b.id == menu.body)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "Body".to_string());
    let imported = document.imported_object_for_body(menu.body);

    let response = Area::new(egui::Id::new("viewport_menu"))
        .order(Order::Foreground)
        .fixed_pos(egui::pos2(menu.at[0], menu.at[1]))
        .constrain(true)
        .interactable(true)
        .show(ctx, |ui| {
            Card::floating().padding(6.0).radius(5.0).show(ui, |ui| {
                // Taller than the window, the menu scrolls rather than
                // running off it.
                let room = ctx.content_rect().height() - 2.0 * SPACE_4;
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                            // An area offers the rest of the screen; the menu takes a
                            // column's worth of it.
                            ui.set_max_width(MENU_WIDTH);
                            ui.set_min_width(MENU_WIDTH);
                            // Rows as the tree's menus draw them: no frame until hovered,
                            // the full width of the menu.
                            egui::containers::menu::menu_style(ui.style_mut());
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(&name).font(sans(FONT_XS)).color(TEXT3));
                            ui.separator();
                            if let Some(tool) = &menu.repeat {
                                if item(ui, &format!("Repeat {tool}")) {
                                    commands.push(UiCommand::RepeatLastTool);
                                }
                                ui.separator();
                            }
                            if item(ui, "Show in tree") {
                                commands.push(UiCommand::RevealInTree(menu.body));
                            }
                            if item(ui, "Select body") {
                                commands.push(UiCommand::SelectBody(menu.body));
                            }
                            if let Some(face) = menu.face
                                && item(ui, "Look at")
                            {
                                commands.push(UiCommand::LookAtFace {
                                    body: menu.body,
                                    face,
                                });
                            }
                            let repairable = document
                                .body_health(menu.body)
                                .is_some_and(|h| h.is_broken())
                                && document
                                    .bodies()
                                    .iter()
                                    .any(|b| b.id == menu.body && !b.repair_requested);
                            if repairable && item(ui, "Repair shape") {
                                commands.push(UiCommand::RepairShapes(vec![menu.body]));
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            let convertible = document.is_mesh_body(menu.body)
                                && document
                                    .bodies()
                                    .iter()
                                    .any(|b| b.id == menu.body && !b.solid_requested);
                            if convertible && item(ui, "Convert to solid") {
                                commands.push(UiCommand::ConvertToSolid(vec![menu.body]));
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            if document.can_refine(menu.body) && item(ui, "Refine shape") {
                                commands.push(UiCommand::RefineShapes(vec![menu.body]));
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            if item(ui, "Hide") {
                                // An imported part hides as its row does; any other
                                // body hides itself.
                                commands.push(match imported {
                                    Some(node) => UiCommand::SetImportedVisibility {
                                        node,
                                        visible: false,
                                    },
                                    None => UiCommand::SetBodyVisible {
                                        body: menu.body,
                                        visible: false,
                                    },
                                });
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            let linked = document
                                .bodies()
                                .iter()
                                .any(|b| b.id == menu.body && b.link.is_some());
                            if linked {
                                if item(ui, "Reload from file") {
                                    commands.push(UiCommand::ReloadLink(menu.body));
                                    commands.push(UiCommand::CloseViewportMenu);
                                }
                                if item(ui, "Open the file") {
                                    commands.push(UiCommand::OpenLinkSource(menu.body));
                                    commands.push(UiCommand::CloseViewportMenu);
                                }
                            }
                            if item(ui, "Isolate") {
                                commands.push(UiCommand::Isolate(Some(menu.body)));
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            if document.bodies().iter().any(|b| b.hidden)
                                && item(ui, "Show all bodies")
                            {
                                commands.push(UiCommand::ShowAllBodies);
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            ui.separator();
                            if super::body_menu::body_entries(
                                ui, document, menu.body, menu.face, menu.point, commands, local,
                            ) {
                                commands.push(UiCommand::CloseViewportMenu);
                            }
                            // What the benches offer for this body, after the host's
                            // own entries.
                            let scope = core_document::MenuScope::ViewportBody(menu.body);
                            let bench_items = registry.menu_items(&scope, document);
                            if !bench_items.is_empty() {
                                ui.separator();
                            }
                            for (workbench, entry) in bench_items {
                                if entry.separator_before {
                                    ui.separator();
                                }
                                let mut button = egui::Button::new(
                                    RichText::new(&entry.label).font(sans(FONT_SM)),
                                );
                                if let Some(key) = keymap.text(&entry.id) {
                                    button = button.shortcut_text(
                                        RichText::new(key).font(sans(FONT_SM)).color(TEXT3),
                                    );
                                }
                                let button = ui.add_enabled(entry.enabled, button);
                                let button = match &entry.hint {
                                    Some(hint) => button.on_hover_text(hint),
                                    None => button,
                                };
                                if button.clicked() {
                                    commands.push(UiCommand::BenchCommand {
                                        workbench: workbench.clone(),
                                        id: entry.id.clone(),
                                        scope: scope.clone(),
                                    });
                                    commands.push(UiCommand::CloseViewportMenu);
                                }
                            }
                        });
                    });
            });
        });

    // A press anywhere but on the menu dismisses it; the press itself still
    // reaches whatever it landed on.
    let pressed_elsewhere = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !response.response.rect.contains(p))
    }) && !ctx
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| ctx.layer_id_at(p))
        // A submenu the menu opened lies outside it, above the view.
        .is_some_and(|layer| layer.order >= Order::Foreground);
    if pressed_elsewhere {
        commands.push(UiCommand::CloseViewportMenu);
    }
}

fn item(ui: &mut egui::Ui, label: &str) -> bool {
    ui.button(label).clicked()
}
