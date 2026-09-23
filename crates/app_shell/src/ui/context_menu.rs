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

const MENU_WIDTH: f32 = 150.0;

/// A right click on a body, and where it landed, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportMenu {
    pub body: core_document::BodyId,
    pub at: [f32; 2],
}

pub fn draw(
    ctx: &Context,
    menu: &ViewportMenu,
    document: &Document,
    registry: &core_document::DocumentService,
    commands: &mut Vec<UiCommand>,
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
                // An area offers the rest of the screen; the menu takes a
                // column's worth of it.
                ui.set_max_width(MENU_WIDTH);
                ui.set_min_width(MENU_WIDTH);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(&name).font(sans(FONT_XS)).color(TEXT3));
                ui.separator();
                if item(ui, "Show in tree") {
                    commands.push(UiCommand::RevealInTree(menu.body));
                }
                if item(ui, "Select body") {
                    commands.push(UiCommand::SelectBody(menu.body));
                }
                let repairable = document
                    .imported_geometry(menu.body)
                    .and_then(|g| g.health.as_ref())
                    .is_some_and(|h| h.is_broken())
                    && document
                        .bodies()
                        .iter()
                        .any(|b| b.id == menu.body && !b.repair_requested);
                if repairable && item(ui, "Repair shape") {
                    commands.push(UiCommand::RepairShapes(vec![menu.body]));
                    commands.push(UiCommand::CloseViewportMenu);
                }
                if let Some(node) = imported
                    && item(ui, "Hide")
                {
                    commands.push(UiCommand::SetImportedVisibility {
                        node,
                        visible: false,
                    });
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
                    let button = ui.add_enabled(
                        entry.enabled,
                        egui::Button::new(RichText::new(&entry.label).font(sans(FONT_SM)))
                            .frame(false)
                            .min_size(egui::vec2(MENU_WIDTH, 22.0)),
                    );
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

    // A press anywhere but on the menu dismisses it; the press itself still
    // reaches whatever it landed on.
    let pressed_elsewhere = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !response.response.rect.contains(p))
    });
    if pressed_elsewhere {
        commands.push(UiCommand::CloseViewportMenu);
    }
}

fn item(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).font(sans(FONT_SM)))
            .frame(false)
            .min_size(egui::vec2(MENU_WIDTH, 22.0)),
    )
    .clicked()
}
