//! Controls for the selected corner's solved candidate.

use crate::SketchWorkbench;
use core_document::{TaskRequest, WorkbenchRuntimeContext};
use ui_kit::widgets::{
    Note, QtyField, field_label, note_card, primary_button, secondary_button, section_header,
};

impl SketchWorkbench {
    pub(crate) fn corner_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) {
        let session = self.session.corner().unwrap();
        let bevel = session.bevel;
        section_header(
            ui,
            "corner_draft",
            if bevel { "Chamfer" } else { "Fillet" },
            None,
            true,
        );
        let feature = self.get_active_sketch(ctx).unwrap();
        let operands = session
            .target
            .curves()
            .map(|id| crate::style::element_name(&feature.sketch, id));
        ui.label(format!("{} + {}", operands[0], operands[1]));
        if secondary_button(ui, "Choose again").clicked() {
            self.discard_corner();
            return;
        }
        let speed = 1.6 * crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px();
        let size = if bevel {
            &mut self.tool_params.chamfer_length
        } else {
            &mut self.tool_params.fillet_radius
        };
        let before = *size;
        egui::Grid::new("corner_parameters")
            .num_columns(2)
            .show(ui, |ui| {
                let label = field_label(ui, if bevel { "Setback" } else { "Radius" });
                QtyField::new(size)
                    .unit("mm")
                    .speed(speed)
                    .show_settling(ui)
                    .response
                    .unwrap()
                    .labelled_by(label.id);
                ui.end_row();
            });
        let changed = *size != before;
        let changed = ui
            .checkbox(&mut self.tool_params.corner_keep, "Keep corner")
            .changed()
            || changed;
        if changed {
            self.recalculate_corner();
        }
        let session = self.session.corner().unwrap();
        if let Err(reason) = &session.draft.working {
            note_card(ui, Note::Warning, None, reason);
        }
        let valid = session.draft.working.is_ok();
        let (mut apply, mut cancel) = (request.accept, request.cancel);
        ui.horizontal(|ui| {
            apply |= ui
                .add_enabled_ui(valid, |ui| primary_button(ui, "Apply"))
                .inner
                .clicked();
            cancel |= secondary_button(ui, "Cancel").clicked();
        });
        if cancel {
            self.discard_corner();
        } else if apply && valid {
            self.accept_corner(ctx);
        }
    }
}
