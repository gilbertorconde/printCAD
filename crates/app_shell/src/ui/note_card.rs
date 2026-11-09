//! The note card: beside a note in the view, its text typed in place, or
//! the answer to its question. Enter saves (a new note is put down with
//! it), Escape drops what was typed; a note already down can be resolved,
//! opened again or deleted here.

use egui::{Align2, Area, Context, Key, Vec2};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, text};

use super::{TreeItemId, UiCommand};
use crate::app::note_tool::NoteCardView;

/// What is typed on the card, and the note it is for: its id and whether
/// it is a new one. UI-local.
pub type NoteDraft = Option<(core_document::NoteId, bool, String)>;

/// How wide the text field is, in logical pixels.
const FIELD: f32 = 280.0;

/// About how wide the card is with its row of buttons, in logical pixels.
const CARD_WIDTH: f32 = 340.0;

/// Draw the card for `view` in `viewport` (logical), `ppp` the UI's scale.
pub fn draw(
    ctx: &Context,
    viewport: egui::Rect,
    ppp: f32,
    view: &NoteCardView,
    draft: &mut NoteDraft,
    commands: &mut Vec<UiCommand>,
) {
    let fresh = !matches!(draft, Some((id, new, _)) if *id == view.id && *new == view.new);
    if fresh {
        *draft = Some((view.id, view.new, view.text.clone()));
    }
    let Some((_, _, typed)) = draft.as_mut() else {
        return;
    };
    let at = view.at.map_or(viewport.center(), |[x, y]| {
        viewport.min + Vec2::new(x, y) / ppp
    });
    // Beside the point, on the side with room for the card.
    let left = at.x + CARD_WIDTH + 18.0 > viewport.right();
    let (pivot, offset) = if left {
        (Align2::RIGHT_TOP, Vec2::new(-18.0, 12.0))
    } else {
        (Align2::LEFT_TOP, Vec2::new(18.0, 12.0))
    };
    let pos = (at + offset)
        .min(viewport.right_bottom() - Vec2::new(0.0, 150.0))
        .max(viewport.min);
    Area::new(egui::Id::new("note_card"))
        .order(super::VIEW_LAYER)
        .pivot(pivot)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            Card::floating().padding(SPACE_3).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                let mut title = format!("Note #{}", view.number);
                if view.author == core_document::NoteAuthor::Agent {
                    title.push_str(" · by the agent");
                }
                if view.resolved {
                    title.push_str(" · resolved");
                }
                ui.label(text(title, FONT_SM, TEXT1));
                if view.lost {
                    ui.label(text(
                        "Lost: what it was put on is gone from the part",
                        FONT_XS,
                        DANGER,
                    ));
                }
                if view.from_file {
                    ui.label(text("Came with the file", FONT_XS, TEXT3));
                }
                if view.answering {
                    ui.label(text(format!("? {}", view.question), FONT_SM, WARNING));
                }
                let hint = if view.answering {
                    "Your answer"
                } else {
                    "A note for the agent, or for people"
                };
                let field = ui.add(
                    egui::TextEdit::singleline(typed)
                        .hint_text(hint)
                        .desired_width(FIELD),
                );
                if fresh {
                    field.request_focus();
                }
                let (enter, escape) =
                    ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                ui.horizontal(|ui| {
                    let save = if view.answering { "Answer" } else { "Save" };
                    if ui_kit::widgets::primary_button(ui, save).clicked()
                        || (enter && field.lost_focus())
                    {
                        commands.push(UiCommand::SaveNoteCard(typed.clone()));
                    }
                    if ui_kit::widgets::secondary_button(ui, "Cancel").clicked() || escape {
                        commands.push(UiCommand::CloseNoteCard);
                    }
                    if !view.new {
                        let resolve = if view.resolved { "Reopen" } else { "Resolve" };
                        if ui_kit::widgets::secondary_button(ui, resolve).clicked() {
                            commands.push(UiCommand::ResolveNote {
                                note: view.id,
                                resolved: (!view.resolved).then(String::new),
                            });
                        }
                        if ui_kit::widgets::destructive_button(ui, "Delete").clicked() {
                            commands.push(UiCommand::CloseNoteCard);
                            commands.push(UiCommand::DeleteTreeItem(TreeItemId::Note(view.id)));
                        }
                    }
                });
            });
        });
}
