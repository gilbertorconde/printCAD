//! Parameters of the pending rectangle, kept outside the saved sketch.
use crate::SketchWorkbench;
use crate::rect_session::CornerStyle;
use core_document::{TaskRequest, WorkbenchRuntimeContext};
use ui_kit::widgets::{
    Note, QtyField, field_label, note_card, primary_button, secondary_button, section_header,
};

impl SketchWorkbench {
    pub(crate) fn rectangle_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) {
        section_header(ui, "rectangle_draft", "Rectangle", None, true);
        let session = self.session.rectangle().unwrap();
        let mut parameters = session.parameters;
        let mut construction = session.construction;
        let before = parameters;
        let feature = self.get_active_sketch(ctx).unwrap();
        let speed = 1.6 * crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px();
        egui::Grid::new("rectangle_parameters")
            .num_columns(2)
            .spacing([ui_kit::tokens::SPACE_2; 2])
            .show(ui, |ui| {
                for (label, value) in [
                    ("Center X", &mut parameters.x),
                    ("Center Y", &mut parameters.y),
                ] {
                    rectangle_field(ui, label, QtyField::offset(value).speed(speed));
                    ui.end_row();
                }
                for (label, value) in [
                    ("Width", &mut parameters.width),
                    ("Height", &mut parameters.height),
                ] {
                    rectangle_field(ui, label, QtyField::mm(value).speed(speed));
                    ui.end_row();
                }
                rectangle_field(ui, "Angle", QtyField::degrees(&mut parameters.angle));
                ui.end_row();
                field_label(ui, "Corners");
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut parameters.corners, CornerStyle::Sharp, "Sharp");
                    ui.selectable_value(&mut parameters.corners, CornerStyle::Fillet, "Fillet");
                    ui.selectable_value(&mut parameters.corners, CornerStyle::Chamfer, "Chamfer");
                });
                ui.end_row();
                if parameters.corners != CornerStyle::Sharp {
                    rectangle_field(
                        ui,
                        if parameters.corners == CornerStyle::Fillet {
                            "Radius"
                        } else {
                            "Setback"
                        },
                        QtyField::mm(&mut parameters.size).speed(speed),
                    );
                    ui.end_row();
                }
                field_label(ui, "Frame");
                let mut frame = parameters.wall.is_some();
                if ui.checkbox(&mut frame, "Inner contour").changed() {
                    parameters.wall = frame.then_some(self.tool_params.offset_distance);
                }
                ui.end_row();
                if let Some(wall) = &mut parameters.wall {
                    rectangle_field(ui, "Wall", QtyField::mm(wall).speed(speed));
                    ui.end_row();
                }
            });
        if parameters.corners != before.corners && parameters.size <= 0. {
            parameters.size = match parameters.corners {
                CornerStyle::Sharp => parameters.size,
                CornerStyle::Fillet => self.tool_params.fillet_radius,
                CornerStyle::Chamfer => self.tool_params.chamfer_length,
            };
        }
        let construction_changed = ui.checkbox(&mut construction, "Construction").changed();
        if parameters != before || construction_changed {
            self.session.rectangle_mut().unwrap().parameters = parameters;
            self.session.rectangle_mut().unwrap().construction = construction;
            self.recalculate_rectangle();
        }
        let session = self.session.rectangle().unwrap();
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
            self.discard_rectangle();
        } else if apply && valid {
            self.accept_rectangle(ctx);
        }
    }
}

fn rectangle_field(ui: &mut egui::Ui, name: &str, field: QtyField<'_>) {
    let label = field_label(ui, name);
    field
        .show_settling(ui)
        .response
        .unwrap()
        .labelled_by(label.id);
}
