//! The question a save asks before it writes over a file another printCAD
//! version wrote: a newer one may hold what this version drops, an older
//! one may no longer open what this version writes. Answered with Save,
//! Save As or Cancel; for an older file, "Don't ask again" turns the
//! question off (Preferences › General).

use egui::{Context, RichText};
use ui_kit::sans_semibold;
use ui_kit::tokens::*;
use ui_kit::widgets::{check_row, destructive_button, primary_button, secondary_button};

use super::UiCommand;
use crate::app::file_safety::{SaveOverAnswer, SaveOverQuestion};

/// Ask about saving `question.name` over its file; the answer, once
/// given, as a command.
pub fn draw(ctx: &Context, question: &SaveOverQuestion, commands: &mut Vec<UiCommand>) {
    let id = egui::Id::new("save_over_modal");
    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(egui::Margin::symmetric(18, 16));
    let mut quiet = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let mut answer = None;
    egui::Modal::new(id)
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.spacing_mut().item_spacing.y = SPACE_2;
            ui.label(
                RichText::new(format!("Save {} over its file?", question.name))
                    .font(sans_semibold(FONT_LG))
                    .color(TEXT1),
            );
            let text = if question.newer {
                format!(
                    "The file was written by {}, newer than this one. Saving over it may lose \
                     what this version does not know; Save As keeps the file as it is.",
                    question.writer
                )
            } else {
                format!(
                    "The file was written by {}. Once saved by this version, that one may no \
                     longer open it; Save As keeps the file as it is.",
                    question.writer
                )
            };
            ui.label(RichText::new(text).font(ui_kit::sans(FONT_SM)).color(TEXT2));
            if !question.newer {
                check_row(
                    ui,
                    &mut quiet,
                    "Don't ask again for files from older versions",
                );
            }
            ui.add_space(SPACE_2);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = if question.newer {
                    destructive_button(ui, "Save anyway")
                } else {
                    primary_button(ui, "Save")
                };
                if save.clicked() {
                    answer = Some(SaveOverAnswer::Save { quiet });
                }
                if secondary_button(ui, "Save As…").clicked() {
                    answer = Some(SaveOverAnswer::SaveAs);
                }
                if secondary_button(ui, "Cancel").clicked() {
                    answer = Some(SaveOverAnswer::Cancel);
                }
            });
        });
    if answer.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(SaveOverAnswer::Cancel);
    }
    if let Some(answer) = answer {
        ctx.data_mut(|d| d.remove::<bool>(id));
        commands.push(UiCommand::AnswerSaveOver(answer));
    } else {
        ctx.data_mut(|d| d.insert_temp(id, quiet));
    }
}
