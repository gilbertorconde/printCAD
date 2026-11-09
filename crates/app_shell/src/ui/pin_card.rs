//! The pin card: beside a pin in the view, the note it says, typed in
//! place. Enter saves it (a new pin is put down with it), Escape drops
//! what was typed; a pin already down can be marked done or deleted here.

use egui::{Align2, Area, Context, Key, Vec2};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, text};

use super::{TreeItemId, UiCommand};
use crate::app::pins::PinCardView;

/// The note being typed, and the pin it is for: its number and whether
/// it is a new one. UI-local.
pub type NoteDraft = Option<(u32, bool, String)>;

/// How wide the note field is, in logical pixels.
const FIELD: f32 = 260.0;

/// Draw the card for `view` in `viewport` (logical), `ppp` the UI's scale.
pub fn draw(
    ctx: &Context,
    viewport: egui::Rect,
    ppp: f32,
    view: &PinCardView,
    draft: &mut NoteDraft,
    commands: &mut Vec<UiCommand>,
) {
    let fresh = !matches!(draft, Some((n, new, _)) if *n == view.number && *new == view.new);
    if fresh {
        *draft = Some((view.number, view.new, view.note.clone()));
    }
    let Some((_, _, note)) = draft.as_mut() else {
        return;
    };
    let at = view.at.map_or(viewport.center(), |[x, y]| {
        viewport.min + Vec2::new(x, y) / ppp
    });
    let pos = (at + Vec2::new(18.0, 12.0))
        .min(viewport.right_bottom() - Vec2::new(FIELD + 2.0 * SPACE_3, 110.0))
        .max(viewport.min);
    Area::new(egui::Id::new("pin_card"))
        .order(super::VIEW_LAYER)
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            Card::floating().padding(SPACE_3).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                let mut title = format!("Pin {}", view.number);
                if view.author == core_document::PinAuthor::Agent {
                    title.push_str(" · by the agent");
                }
                if view.done {
                    title.push_str(" · done");
                }
                ui.label(text(title, FONT_SM, TEXT1));
                if view.lost {
                    ui.label(text(
                        "Lost: what it was put on is gone from the part",
                        FONT_XS,
                        DANGER,
                    ));
                }
                let field = ui.add(
                    egui::TextEdit::singleline(note)
                        .hint_text("A note for the agent, or for people")
                        .desired_width(FIELD),
                );
                if fresh {
                    field.request_focus();
                }
                let (enter, escape) =
                    ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                ui.horizontal(|ui| {
                    if ui_kit::widgets::primary_button(ui, "Save").clicked()
                        || (enter && field.lost_focus())
                    {
                        commands.push(UiCommand::SavePin(note.clone()));
                    }
                    if ui_kit::widgets::secondary_button(ui, "Cancel").clicked() || escape {
                        commands.push(UiCommand::ClosePinCard);
                    }
                    if !view.new {
                        let done = if view.done { "Reopen" } else { "Mark done" };
                        if ui_kit::widgets::secondary_button(ui, done).clicked() {
                            commands.push(UiCommand::SetPinDone {
                                number: view.number,
                                done: !view.done,
                            });
                        }
                        if ui_kit::widgets::destructive_button(ui, "Delete").clicked() {
                            commands.push(UiCommand::ClosePinCard);
                            commands.push(UiCommand::DeleteTreeItem(TreeItemId::Pin(view.number)));
                        }
                    }
                });
            });
        });
}
