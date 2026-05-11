//! STEP import options shown after the user picks a file, before the kernel runs.

use std::path::Path;

use egui::{Align2, Context};
use kernel_api::{LinearDeflectionMode, TessellationSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepImportDialogAction {
    #[default]
    None,
    Confirmed,
    Cancelled,
}

pub fn draw_step_import_modal(
    ctx: &Context,
    path: &Path,
    draft: &mut TessellationSettings,
) -> StepImportDialogAction {
    let mut action = StepImportDialogAction::None;
    let path_label = path.display().to_string();

    egui::Window::new("Import STEP")
        .collapsible(false)
        .resizable(true)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("File: {path_label}"));
            ui.add_space(8.0);

            egui::Grid::new("step_import_grid")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Linear deflection");
                    ui.horizontal(|ui| {
                        let mut mode = draft.linear_deflection_mode;
                        egui::ComboBox::from_id_salt("step_linear_mode")
                            .selected_text(match mode {
                                LinearDeflectionMode::BboxScaled => {
                                    "Bbox-scaled (bbox × deviation)"
                                }
                                LinearDeflectionMode::AbsoluteMm => "Absolute chord height (mm)",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut mode,
                                    LinearDeflectionMode::BboxScaled,
                                    "Bbox-scaled (bbox × deviation)",
                                );
                                ui.selectable_value(
                                    &mut mode,
                                    LinearDeflectionMode::AbsoluteMm,
                                    "Absolute chord height (mm)",
                                );
                            });
                        draft.linear_deflection_mode = mode;
                    });
                    ui.end_row();

                    match draft.linear_deflection_mode {
                        LinearDeflectionMode::BboxScaled => {
                            ui.label("Mesh deviation");
                            ui.add(egui::Slider::new(&mut draft.mesh_deviation, 0.01..=1.0));
                            ui.end_row();
                        }
                        LinearDeflectionMode::AbsoluteMm => {
                            ui.label("Chord tolerance (mm)");
                            ui.add(egui::Slider::new(&mut draft.chord_tolerance, 0.001..=5.0));
                            ui.end_row();
                        }
                    }

                    ui.label("Angular tolerance (°)");
                    ui.add(egui::Slider::new(
                        &mut draft.angular_tolerance_deg,
                        0.5..=90.0,
                    ));
                    ui.end_row();

                    ui.label("Weld across faces");
                    ui.checkbox(&mut draft.weld_cross_face, "Merge coplanar-adjacent verts");
                    ui.end_row();

                    if draft.weld_cross_face {
                        ui.label("Weld angle threshold (°)");
                        ui.add(egui::Slider::new(
                            &mut draft.weld_angle_threshold_deg,
                            0.0..=90.0,
                        ));
                        ui.end_row();
                    }

                    ui.label("Deferred tessellation");
                    ui.checkbox(
                        &mut draft.persist_brep_snapshot,
                        "Serialize BRep, mesh in background (recommended for large STEP)",
                    );
                    ui.end_row();

                    ui.label("Boundary edges");
                    ui.checkbox(
                        &mut draft.generate_boundary_edges,
                        "Outline segments for viewport (extra CPU on large meshes)",
                    );
                    ui.end_row();
                });

            if !draft.persist_brep_snapshot {
                ui.label(
                    egui::RichText::new(
                        "With deferred tessellation off, meshing runs entirely inside the first import (one long step; see stderr `inline_mesh_ms`).",
                    )
                    .small()
                    .italics(),
                );
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Import").clicked() {
                    action = StepImportDialogAction::Confirmed;
                }
                if ui.button("Cancel").clicked() {
                    action = StepImportDialogAction::Cancelled;
                }
            });
        });

    action
}
