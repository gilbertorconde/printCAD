//! The question an STL import asks: the format does not say what units
//! its numbers are in. Answered with a unit, offered the last one given,
//! or Cancel; "Don't ask again" reads every later STL in the answer
//! (Preferences › General › Files).

use egui::{Context, RichText};
use kernel_api::LengthUnit;
use ui_kit::sans_semibold;
use ui_kit::tokens::*;
use ui_kit::widgets::{check_row, primary_button, secondary_button, select_field};

use super::UiCommand;

/// Ask what units `file` is in, offering `offered` first; the answer, once
/// given, as a command.
pub fn draw(ctx: &Context, file: &str, offered: LengthUnit, commands: &mut Vec<UiCommand>) {
    let id = egui::Id::new("mesh_units_modal");
    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(egui::Margin::symmetric(18, 16));
    let (mut unit, mut quiet) = ctx
        .data(|d| d.get_temp::<(LengthUnit, bool)>(id))
        .unwrap_or((offered, false));
    let mut answer = None;
    egui::Modal::new(id)
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.spacing_mut().item_spacing.y = SPACE_2;
            ui.label(
                RichText::new(format!("What units is {file} in?"))
                    .font(sans_semibold(FONT_LG))
                    .color(TEXT1),
            );
            ui.label(
                RichText::new(
                    "An STL file holds bare numbers. They are read in the units chosen here \
                     and kept in millimetres.",
                )
                .font(ui_kit::sans(FONT_SM))
                .color(TEXT2),
            );
            select_field(
                ui,
                "mesh_units_choice",
                &mut unit,
                &crate::app::scans::MESH_UNITS,
                200.0,
            );
            check_row(
                ui,
                &mut quiet,
                "Don't ask again: read every STL in these units",
            );
            ui.add_space(SPACE_2);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, "Import").clicked() {
                    answer = Some(Some((unit, quiet)));
                }
                if secondary_button(ui, "Cancel").clicked() {
                    answer = Some(None);
                }
            });
        });
    if answer.is_none() {
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                answer = Some(None);
            } else if i.key_pressed(egui::Key::Enter) {
                answer = Some(Some((unit, quiet)));
            }
        });
    }
    if let Some(answer) = answer {
        ctx.data_mut(|d| d.remove::<(LengthUnit, bool)>(id));
        commands.push(UiCommand::AnswerMeshUnits(answer));
    } else {
        ctx.data_mut(|d| d.insert_temp(id, (unit, quiet)));
    }
}
