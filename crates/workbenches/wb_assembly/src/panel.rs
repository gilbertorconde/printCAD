//! The Assembly task panels: the prompt while a joint's faces are picked,
//! a joint's settings, and a body moved by numbers.

use core_document::{
    BodyId, BodyPlacement, FeatureId, TaskOutcome, TaskRequest, WorkbenchFeature,
    WorkbenchRuntimeContext,
};
use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, Note, QtyField, check_row, destructive_button, note_card};
use ui_kit::{sans, sans_semibold};

use crate::{AssemblyWorkbench, JointFeature, JointKind, Task, body_name, restore_placements};

fn header(ui: &mut egui::Ui, icon: &str, title: &str) {
    egui::Frame::new()
        .fill(BG2)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                ui_kit::icon::draw(ui, icon, 18.0, ACCENT);
                ui.label(
                    RichText::new(title)
                        .font(sans_semibold(FONT_MD))
                        .color(TEXT1),
                );
            });
        });
}

fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [90.0, INPUT],
            egui::Label::new(RichText::new(label).font(sans(FONT_SM)).color(TEXT2)),
        );
        ui.label(RichText::new(value).font(sans(FONT_SM)).color(TEXT1));
    });
}

impl AssemblyWorkbench {
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if let Some(picking) = &self.picking {
            if request.accept || request.cancel {
                self.picking = None;
                return TaskOutcome::Cancelled;
            }
            header(ui, picking.kind.icon(), picking.kind.label());
            ui.add_space(SPACE_2);
            ui.label(
                RichText::new(picking.kind.prompt(picking.first.is_some()))
                    .font(sans(FONT_SM))
                    .color(TEXT1),
            );
            ui.add_space(SPACE_1);
            ui.label(
                RichText::new(
                    "The first body moves; the second stays where it is. A body \
                     with no joints of its own never moves.",
                )
                .font(sans(FONT_XS))
                .color(TEXT3),
            );
            return TaskOutcome::Open;
        }
        match self.task.clone() {
            Some(Task::Joint {
                id,
                before,
                placements,
            }) => self.joint_panel(ui, ctx, request, id, before, &placements),
            Some(Task::Move { body, placements }) => {
                self.move_panel(ui, ctx, request, body, &placements)
            }
            None => TaskOutcome::Open,
        }
    }

    fn verdict_card(&self, ui: &mut egui::Ui) {
        match &self.verdict {
            Some(Ok(message)) => {
                note_card(ui, Note::Success, None, message);
            }
            Some(Err(message)) => {
                note_card(ui, Note::Error, Some("Joints left apart"), message);
            }
            None => {}
        }
    }

    fn joint_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        let created = before.is_none();
        if request.cancel {
            match &before {
                Some(data) => {
                    let _ = ctx.document.update_feature_data(id, data.clone());
                    ctx.document.clear_feature_dirty(id);
                }
                None => {
                    let _ = ctx.document.remove_feature(id);
                    ctx.active_document_object = None;
                }
            }
            restore_placements(ctx, placements);
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.task = None;
            ctx.active_document_object = None;
            return TaskOutcome::Accepted {
                label: if created { "Add joint" } else { "Edit joint" }.to_string(),
            };
        }
        let Some(node) = ctx.document.get_feature_meta(id).cloned() else {
            self.task = None;
            return TaskOutcome::Cancelled;
        };
        let Ok(mut joint) = JointFeature::from_json(&node.data) else {
            note_card(
                ui,
                Note::Error,
                Some("Unreadable joint"),
                "The stored joint does not parse.",
            );
            return TaskOutcome::Open;
        };
        header(ui, joint.kind.icon(), &node.name);
        ui.add_space(SPACE_2);
        let moving = node.body.map(|b| body_name(ctx, b)).unwrap_or_default();
        row(ui, "Moves", &moving);
        row(ui, "Against", &body_name(ctx, joint.other_body));
        ui.add_space(SPACE_2);
        let mut changed = false;
        Card::new().padding(SPACE_3).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &mut joint.kind {
                JointKind::Mate { flip, offset } => {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, INPUT],
                            egui::Label::new(RichText::new("Gap").font(sans(FONT_SM)).color(TEXT2)),
                        );
                        changed |= QtyField::mm(offset).range(-1000.0..=1000.0).show(ui);
                    })
                    .response
                    .on_hover_text("How far apart the two faces sit");
                    changed |= check_row(ui, flip, "Same way")
                        .on_hover_text("The faces point the same way instead of at each other")
                        .changed();
                }
                JointKind::Align => {
                    ui.label(
                        RichText::new(
                            "The body can still turn about the axis and slide along \
                             it; a mate on an end face holds the slide.",
                        )
                        .font(sans(FONT_SM))
                        .color(TEXT2),
                    );
                }
            }
        });
        if changed {
            let _ = ctx.document.update_feature_data(id, joint.to_json());
            ctx.document.clear_feature_dirty(id);
            self.solve_and_apply(ctx);
        }
        ui.add_space(SPACE_2);
        self.verdict_card(ui);
        ui.add_space(SPACE_2);
        if destructive_button(ui, "Delete joint")
            .on_hover_text("Remove the joint; the bodies stay where they are")
            .clicked()
            && ctx.document.remove_feature(id).is_ok()
        {
            self.task = None;
            ctx.active_document_object = None;
            return TaskOutcome::Accepted {
                label: "Delete joint".to_string(),
            };
        }
        TaskOutcome::Open
    }

    fn move_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        body: BodyId,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        if request.cancel {
            restore_placements(ctx, placements);
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.task = None;
            return TaskOutcome::Accepted {
                label: "Move body".to_string(),
            };
        }
        header(ui, "move-geometry", &body_name(ctx, body));
        ui.add_space(SPACE_2);
        let held = Self::held_by_joints(ctx, body);
        if held {
            note_card(
                ui,
                Note::Info,
                Some("Placed by its joints"),
                "Its joints decide where it sits. Move the body it is joined to, \
                 or delete a joint to free it.",
            );
        }
        let placement = ctx.document.body_placement(body);
        let mut offset = placement.translation;
        let (ax, ay, az) = placement.quat().to_euler(glam::EulerRot::XYZ);
        let mut angles = [ax, ay, az].map(f32::to_degrees);
        let mut changed = false;
        Card::new().padding(SPACE_3).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(!held, |ui| {
                for (axis, value) in ["X", "Y", "Z"].iter().zip(offset.iter_mut()) {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, INPUT],
                            egui::Label::new(
                                RichText::new(format!("Position {axis}"))
                                    .font(sans(FONT_SM))
                                    .color(TEXT2),
                            ),
                        );
                        changed |= QtyField::mm(value).show(ui);
                    });
                }
                for (axis, value) in ["X", "Y", "Z"].iter().zip(angles.iter_mut()) {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, INPUT],
                            egui::Label::new(
                                RichText::new(format!("Turn about {axis}"))
                                    .font(sans(FONT_SM))
                                    .color(TEXT2),
                            ),
                        );
                        changed |= QtyField::degrees(value).range(-180.0..=180.0).show(ui);
                    });
                }
            });
        });
        if changed {
            let [ax, ay, az] = angles.map(f32::to_radians);
            let moved = BodyPlacement::new(
                glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, az),
                glam::Vec3::from_array(offset),
            );
            ctx.document.set_body_placement(body, moved);
            // Whatever is joined to it follows.
            self.solve_and_apply(ctx);
        }
        ui.add_space(SPACE_2);
        if !held
            && ui_kit::widgets::secondary_button(ui, "Back to where it was made")
                .on_hover_text("No move, no turn")
                .clicked()
        {
            ctx.document
                .set_body_placement(body, BodyPlacement::IDENTITY);
            self.solve_and_apply(ctx);
        }
        self.verdict_card(ui);
        TaskOutcome::Open
    }
}
