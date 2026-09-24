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

/// Frames in a recorded sweep: there and back in four seconds.
const SWEEP_FRAMES: usize = 60;

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
            Some(Task::Interference { found, seq }) => {
                self.interference_panel(ui, ctx, request, found.as_ref(), seq)
            }
            Some(Task::Explode { placements, spread }) => {
                self.explode_panel(ui, ctx, request, placements, spread)
            }
            Some(Task::Parts) => self.parts_panel(ui, ctx, request),
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

    /// What an interference check found: each clash, a click selecting
    /// the first of its bodies.
    fn interference_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        found: Option<&crate::Interference>,
        seq: u64,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.checking = None;
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        header(ui, "check-geometry", "Interference");
        ui.add_space(SPACE_2);
        self.collect_interference(ctx);
        let Some(found) = found else {
            let (done, total) = self.interference_progress().unwrap_or((0, 0));
            ui.label(
                RichText::new(format!("Checking {done} of {total} pairs that may touch"))
                    .font(sans(FONT_SM))
                    .color(TEXT1),
            );
            ui.add(egui::ProgressBar::new(if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            }));
            ui.add_space(SPACE_2);
            if ui_kit::widgets::secondary_button(ui, "Stop")
                .on_hover_text("Stop checking; the clashes found so far stay")
                .clicked()
            {
                self.stop_interference();
            }
            // The answer arrives on another thread, with no event to wake
            // the window.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            return TaskOutcome::Open;
        };
        let bodies = format!(
            "{} bod{}",
            found.checked,
            if found.checked == 1 { "y" } else { "ies" }
        );
        match found.clashes.len() {
            0 => note_card(
                ui,
                Note::Success,
                None,
                &format!("No interference among {bodies}"),
            ),
            n => note_card(
                ui,
                Note::Error,
                Some(&format!("{n} clash{}", if n == 1 { "" } else { "es" })),
                &format!("Among {bodies}; click one to select its first body"),
            ),
        };
        ui.add_space(SPACE_2);
        for clash in &found.clashes {
            let text = format!(
                "{} and {}: {:.2} mm³",
                body_name(ctx, clash.a),
                body_name(ctx, clash.b),
                clash.volume_mm3
            );
            let row = ui.add(
                egui::Button::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT1))
                    .frame(false),
            );
            if row.clicked() {
                ctx.request(core_document::HostRequest::SelectBody(clash.a));
            }
        }
        if found.stopped {
            ui.add_space(SPACE_1);
            note_card(
                ui,
                Note::Warning,
                None,
                "Stopped early: some pairs were not checked",
            );
        }
        if found.skipped > 0 {
            ui.add_space(SPACE_1);
            ui.label(
                RichText::new(format!(
                    "{} visible bod{} without a solid (a mesh, or not built yet) left out",
                    found.skipped,
                    if found.skipped == 1 { "y" } else { "ies" }
                ))
                .font(sans(FONT_XS))
                .color(TEXT3),
            );
        }
        if ctx.document.mutation_seq() != seq {
            ui.add_space(SPACE_1);
            note_card(
                ui,
                Note::Warning,
                None,
                "The assembly has changed since this check",
            );
        }
        ui.add_space(SPACE_2);
        if ui_kit::widgets::secondary_button(ui, "Check again").clicked() {
            self.check_interference(ctx);
        }
        TaskOutcome::Open
    }

    /// The exploded view's spread; closing puts every body back.
    fn explode_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        placements: Vec<(BodyId, BodyPlacement)>,
        mut spread: f32,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.put_back_explosion(ctx);
            return TaskOutcome::Cancelled;
        }
        header(ui, "scale-geometry", "Exploded view");
        ui.add_space(SPACE_2);
        let changed = ui
            .horizontal(|ui| {
                ui.add_sized(
                    [90.0, INPUT],
                    egui::Label::new(RichText::new("Spread").font(sans(FONT_SM)).color(TEXT2)),
                );
                ui.add(egui::Slider::new(&mut spread, 0.0..=3.0).fixed_decimals(2))
                    .on_hover_text(
                        "How far each body moves out, as a share of its distance from the middle",
                    )
                    .changed()
            })
            .inner;
        if changed {
            crate::explode(ctx, &placements, spread);
            self.task = Some(Task::Explode { placements, spread });
        }
        ui.add_space(SPACE_2);
        ui.label(
            RichText::new(
                "Each body moves straight out from the middle of the assembly. \
                 Nothing is kept: the bodies go back when this closes.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        TaskOutcome::Open
    }

    /// Every part, how many of it and its size, with a copy for a
    /// spreadsheet.
    fn parts_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        header(ui, "file-document", "Parts list");
        ui.add_space(SPACE_2);
        let parts = crate::parts_list(ctx.document);
        let total: usize = parts.iter().map(|p| p.bodies.len()).sum();
        ui.label(
            RichText::new(format!(
                "{} part{}, {total} bod{}",
                parts.len(),
                if parts.len() == 1 { "" } else { "s" },
                if total == 1 { "y" } else { "ies" }
            ))
            .font(sans(FONT_SM))
            .color(TEXT2),
        );
        ui.add_space(SPACE_1);
        egui::Grid::new("assembly_parts")
            .num_columns(3)
            .striped(true)
            .spacing([SPACE_3, SPACE_1])
            .show(ui, |ui| {
                for heading in ["Part", "Qty", "Size (mm)"] {
                    ui.label(RichText::new(heading).font(sans(FONT_XS)).color(TEXT3));
                }
                ui.end_row();
                for part in &parts {
                    let name = ui.add(
                        egui::Button::new(
                            RichText::new(&part.name).font(sans(FONT_SM)).color(TEXT1),
                        )
                        .frame(false),
                    );
                    if name.clicked() {
                        ctx.request(core_document::HostRequest::SelectBody(part.bodies[0]));
                    }
                    ui.label(
                        RichText::new(part.bodies.len().to_string())
                            .font(ui_kit::mono(FONT_SM))
                            .color(TEXT1),
                    );
                    let size = part.size_mm.map_or_else(
                        || "-".to_string(),
                        |s| format!("{:.1} × {:.1} × {:.1}", s[0], s[1], s[2]),
                    );
                    ui.label(RichText::new(size).font(ui_kit::mono(FONT_SM)).color(TEXT1));
                    ui.end_row();
                }
            });
        ui.add_space(SPACE_2);
        ui.horizontal(|ui| {
            if ui_kit::widgets::secondary_button(ui, "Copy as CSV")
                .on_hover_text("For a spreadsheet: part, quantity, size and kind")
                .clicked()
            {
                ui.ctx().copy_text(crate::parts_csv(&parts));
                ctx.log_info("Parts list copied");
            }
            if ui_kit::widgets::secondary_button(ui, "Save as CSV")
                .on_hover_text("Write the list to a file a spreadsheet opens")
                .clicked()
            {
                ctx.request(core_document::HostRequest::SaveFile {
                    name: "parts.csv".into(),
                    kind: "Comma-separated values".into(),
                    extension: "csv".into(),
                    contents: crate::parts_csv(&parts).into_bytes(),
                });
            }
        });
        TaskOutcome::Open
    }

    /// End a sweep, the drive back at the value it started from.
    fn stop_playing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(play) = self.playing.take() else {
            return;
        };
        let Some(node) = ctx.document.get_feature_meta(play.joint) else {
            return;
        };
        let Ok(mut joint) = JointFeature::from_json(&node.data) else {
            return;
        };
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = &mut joint.kind {
            drive.to = drive.to.map(|_| play.start);
        }
        let _ = ctx
            .document
            .update_feature_data(play.joint, joint.to_json());
        ctx.document.clear_feature_dirty(play.joint);
        self.solve_and_apply(ctx);
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
            self.playing = None;
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
            self.stop_playing(ctx);
            crate::commands::record_joint(ctx, id, before.as_ref(), placements);
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
        let mut formula_edits: Vec<(String, Option<String>)> = Vec::new();
        let document: &core_document::Document = ctx.document;
        let placed = |b: BodyId| -> crate::Rigid { document.body_placement(b).into() };
        let now = node
            .body
            .and_then(|b| joint.travel(&placed(b), &placed(joint.other_body)));
        let dt = f64::from(ui.input(|i| i.stable_dt).min(0.1));
        let playing = &mut self.playing;
        let mut record = None;
        Card::new().padding(SPACE_3).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &mut joint.kind {
                JointKind::Mate { flip, offset } => {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, INPUT],
                            egui::Label::new(RichText::new("Gap").font(sans(FONT_SM)).color(TEXT2)),
                        );
                        changed |= formula_field(
                            ui,
                            document,
                            id,
                            "/kind/Mate/offset",
                            core_document::expr::Dim::LENGTH,
                            offset,
                            &mut formula_edits,
                        );
                    })
                    .response
                    .on_hover_text("How far apart the two faces sit");
                    changed |= check_row(ui, flip, "Same way")
                        .on_hover_text("The faces point the same way instead of at each other")
                        .changed();
                }
                JointKind::Angle { degrees } => {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, INPUT],
                            egui::Label::new(
                                RichText::new("Angle").font(sans(FONT_SM)).color(TEXT2),
                            ),
                        );
                        changed |= formula_field(
                            ui,
                            document,
                            id,
                            "/kind/Angle/degrees",
                            core_document::expr::Dim::ANGLE,
                            degrees,
                            &mut formula_edits,
                        );
                    })
                    .response
                    .on_hover_text(
                        "Between the faces' outward normals: 180 faces them at each other",
                    );
                    ui.label(
                        RichText::new(
                            "Only the turn is held: pair it with a mate or an alignment to \
                             say where the body sits.",
                        )
                        .font(sans(FONT_SM))
                        .color(TEXT2),
                    );
                }
                JointKind::Ground => {
                    ui.label(
                        RichText::new(
                            "The body stays where it is; the bodies joined to it are \
                             placed against it.",
                        )
                        .font(sans(FONT_SM))
                        .color(TEXT2),
                    );
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
                JointKind::Hinge { offset, drive, .. } => {
                    changed |= number_row(
                        ui,
                        (document, id, &mut formula_edits),
                        (
                            "Height",
                            "How far along the axis the body sits from the other",
                        ),
                        "/kind/Hinge/offset",
                        core_document::expr::Dim::LENGTH,
                        offset,
                    );
                    changed |= drive_rows(
                        ui,
                        (document, id, &mut formula_edits),
                        "Hinge",
                        drive,
                        (now, dt),
                        playing,
                        &mut record,
                    );
                    note(
                        ui,
                        "The body can only turn about the axis. Its angle counts from \
                         where it sat when the joint was made.",
                    );
                }
                JointKind::Distance { offset } => {
                    changed |= number_row(
                        ui,
                        (document, id, &mut formula_edits),
                        ("Distance", "Along the other face's normal"),
                        "/kind/Distance/offset",
                        core_document::expr::Dim::LENGTH,
                        offset,
                    );
                    note(
                        ui,
                        "Only the distance is held: the faces may turn and slide past \
                         each other.",
                    );
                }
                JointKind::Tangent { radius } => {
                    changed |= number_row(
                        ui,
                        (document, id, &mut formula_edits),
                        ("Radius", "The round face's radius"),
                        "/kind/Tangent/radius",
                        core_document::expr::Dim::LENGTH,
                        radius,
                    );
                    note(
                        ui,
                        "The round face rests on the flat one; it can still roll and \
                         slide along it.",
                    );
                }
                JointKind::Slider { drive, .. } => {
                    changed |= drive_rows(
                        ui,
                        (document, id, &mut formula_edits),
                        "Slider",
                        drive,
                        (now, dt),
                        playing,
                        &mut record,
                    );
                    note(
                        ui,
                        "The body can only slide along the axis, turned as it was when \
                         the joint was made.",
                    );
                }
                JointKind::Fixed { .. } => note(
                    ui,
                    "The body is held to the other as it sat when the joint was made; \
                     it moves only with it.",
                ),
                JointKind::Parallel => note(
                    ui,
                    "Only the turn is held, the faces parallel: pair it with other joints \
                     to say where the body sits.",
                ),
                JointKind::Perpendicular => note(
                    ui,
                    "Only the turn is held, the faces square: pair it with other joints \
                     to say where the body sits.",
                ),
            }
        });
        if changed {
            let _ = ctx.document.update_feature_data(id, joint.to_json());
            ctx.document.clear_feature_dirty(id);
            self.solve_and_apply(ctx);
        }
        if let Some((low, high)) = record {
            let frames = crate::sweep_frames(ctx.document, id, low, high, SWEEP_FRAMES);
            if frames.is_empty() {
                ctx.log_warn("Nothing moves through this joint's range");
            } else {
                ctx.request(core_document::HostRequest::RecordAnimation {
                    name: node.name.clone(),
                    frames,
                    frame_ms: 4000 / SWEEP_FRAMES as u32,
                });
            }
        }
        ui.add_space(SPACE_2);
        self.verdict_card(ui);
        if let Some(body) = node.body {
            freedom_line(ui, ctx, body);
        }
        ui.add_space(SPACE_2);
        if destructive_button(ui, "Delete joint")
            .on_hover_text("Remove the joint; the bodies stay where they are")
            .clicked()
            && ctx.document.remove_feature(id).is_ok()
        {
            self.playing = None;
            // A joint the task made has nothing to undo in a recording.
            if !created {
                ctx.record(
                    "doc.delete",
                    crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
                    serde_json::Value::Null,
                );
            }
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
            let placement = ctx.document.body_placement(body);
            ctx.record(
                "asm.place",
                crate::commands::object(serde_json::json!({
                    "body": body.0.to_string(),
                    "translation": placement.translation,
                    "rotation": placement.rotation,
                })),
                serde_json::Value::Null,
            );
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

/// A joint's number as a formula field: a value typed or dragged goes into
/// `value`; a formula goes into `edits` and what it comes to into `value`,
/// so the body moves while the panel is open.
/// A hinge's or a slider's drive: held at a value, kept within limits,
/// and, while held, swept through its range to show the motion.
fn drive_rows(
    ui: &mut egui::Ui,
    (document, joint, edits): (
        &core_document::Document,
        core_document::FeatureId,
        &mut Vec<(String, Option<String>)>,
    ),
    variant: &str,
    drive: &mut crate::Drive,
    (now, dt): (Option<f64>, f64),
    playing: &mut Option<crate::Play>,
    record: &mut Option<(f32, f32)>,
) -> bool {
    use core_document::expr::Dim;
    let angular = variant == "Hinge";
    let (dim, unit) = if angular {
        (Dim::ANGLE, "°")
    } else {
        (Dim::LENGTH, " mm")
    };
    let mut changed = false;
    if let Some(now) = now {
        row(
            ui,
            if angular { "Angle now" } else { "Position now" },
            &format!("{now:.2}{unit}"),
        );
    }
    let to_key = format!("/kind/{variant}/drive/to");
    let mut driven = drive.to.is_some();
    if check_row(ui, &mut driven, "Drive")
        .on_hover_text(if angular {
            "Hold the hinge at an angle"
        } else {
            "Hold the slider at a position"
        })
        .changed()
    {
        drive.to = driven.then(|| now.unwrap_or(0.0) as f32);
        if !driven {
            edits.push((to_key.clone(), None));
            *playing = None;
        }
        changed = true;
    }
    if let Some(to) = &mut drive.to {
        changed |= number_row(
            ui,
            (document, joint, edits),
            (
                if angular { "Angle" } else { "Position" },
                "Where the drive holds it",
            ),
            &to_key,
            dim,
            to,
        );
    }
    let mut limited = drive.limits.is_some();
    if check_row(ui, &mut limited, "Limits")
        .on_hover_text("Keep the motion within a range while it is not driven")
        .changed()
    {
        let at = now.unwrap_or(0.0) as f32;
        drive.limits = limited.then(|| {
            if angular {
                [(at - 45.0).max(-180.0), (at + 45.0).min(180.0)]
            } else {
                [at - 10.0, at + 10.0]
            }
        });
        if !limited {
            for end in 0..2 {
                edits.push((format!("/kind/{variant}/drive/limits/{end}"), None));
            }
        }
        changed = true;
    }
    if let Some([low, high]) = &mut drive.limits {
        for (end, value, label) in [(0, &mut *low, "Lowest"), (1, &mut *high, "Highest")] {
            changed |= number_row(
                ui,
                (document, joint, edits),
                (label, "An end of the range the motion stays in"),
                &format!("/kind/{variant}/drive/limits/{end}"),
                dim,
                value,
            );
        }
        if *low > *high {
            std::mem::swap(low, high);
        }
    }
    let limits = drive.limits;
    let Some(to) = &mut drive.to else {
        return changed;
    };
    // The sweep: through the limits, or a whole turn, or 25 mm either side
    // of where it started.
    let mine = playing.filter(|p| p.joint == joint);
    let centre = f64::from(mine.map_or(*to, |p| p.start));
    let (low, high) = match limits {
        Some([low, high]) => (f64::from(low), f64::from(high)),
        None if angular => (-179.0, 179.0),
        None => (centre - 25.0, centre + 25.0),
    };
    let label = if mine.is_some() { "Stop" } else { "Play" };
    if ui_kit::widgets::secondary_button(ui, "Record")
        .on_hover_text("Save the sweep through its range as an animation")
        .clicked()
    {
        *record = Some((low as f32, high as f32));
    }
    if document.feature_formula(joint, &to_key).is_some() {
        // A formula holds the value; a sweep would fight it every frame.
        *playing = playing.filter(|p| p.joint != joint);
    } else if ui_kit::widgets::secondary_button(ui, label)
        .on_hover_text("Sweep the drive through its range; stopping puts it back")
        .clicked()
    {
        match mine {
            Some(p) => {
                *to = p.start;
                *playing = None;
            }
            None => {
                let span = (high - low).max(1e-6);
                let from = ((f64::from(*to) - low) / span).clamp(0.0, 1.0);
                *playing = Some(crate::Play {
                    joint,
                    start: *to,
                    phase: (1.0 - 2.0 * from).acos(),
                });
            }
        }
        changed = true;
    } else if let Some(play) = playing.as_mut().filter(|p| p.joint == joint) {
        // Back and forth every four seconds.
        play.phase += dt * std::f64::consts::TAU / 4.0;
        *to = (low + (high - low) * (0.5 - 0.5 * play.phase.cos())) as f32;
        changed = true;
        ui.ctx().request_repaint();
    }
    changed
}

/// A labelled number a formula can set, in a joint's settings.
fn number_row(
    ui: &mut egui::Ui,
    (document, joint, edits): (
        &core_document::Document,
        core_document::FeatureId,
        &mut Vec<(String, Option<String>)>,
    ),
    (label, hover): (&str, &str),
    key: &str,
    dim: core_document::expr::Dim,
    value: &mut f32,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            [90.0, INPUT],
            egui::Label::new(RichText::new(label).font(sans(FONT_SM)).color(TEXT2)),
        );
        changed = formula_field(ui, document, joint, key, dim, value, edits);
    })
    .response
    .on_hover_text(hover);
    changed
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT2));
}

fn formula_field(
    ui: &mut egui::Ui,
    document: &core_document::Document,
    joint: core_document::FeatureId,
    key: &str,
    dim: core_document::expr::Dim,
    value: &mut f32,
    edits: &mut Vec<(String, Option<String>)>,
) -> bool {
    let formula = document.feature_formula(joint, key);
    let slot = document
        .evaluated_slots(joint)
        .iter()
        .find(|s| s.key == key);
    let shown = match (formula, slot.map(|s| &s.result)) {
        (Some(_), Some(Ok(q))) => q.value,
        _ => f64::from(*value),
    };
    let host = core_document::DocumentFormulas { document, dim };
    let angle = dim == core_document::expr::Dim::ANGLE;
    let edit = ui_kit::widgets::FormulaField::new(
        egui::Id::new(("joint_field", joint, key)),
        shown,
        &host,
    )
    .formula(formula)
    .error(
        slot.and_then(|s| s.result.as_ref().err())
            .map(String::as_str),
    )
    .unit(if angle { "°" } else { "mm" })
    .speed(if angle { 1.0 } else { 0.1 })
    .show(ui);
    match edit {
        Some(ui_kit::widgets::FormulaEdit::Value(v)) => {
            if formula.is_some() {
                edits.push((key.to_string(), None));
            }
            *value = v as f32;
            true
        }
        Some(ui_kit::widgets::FormulaEdit::Formula(text)) => {
            let now = document.evaluate_formula(&text, Some(dim));
            edits.push((key.to_string(), Some(text)));
            match now {
                Ok(q) => {
                    *value = q.value as f32;
                    true
                }
                Err(_) => false,
            }
        }
        None => false,
    }
}

/// What `body` may still do, its joints holding: "fully placed", or its
/// free motions.
fn freedom_line(ui: &mut egui::Ui, ctx: &WorkbenchRuntimeContext, body: BodyId) {
    let Some((_, motions)) = crate::freedom(ctx.document)
        .into_iter()
        .find(|(b, _)| *b == body)
    else {
        return;
    };
    let text = if motions.is_empty() {
        "Its joints place this body fully.".to_string()
    } else {
        let words: Vec<String> = motions.iter().map(crate::Motion::describe).collect();
        format!("It may still {}.", words.join(", "))
    };
    ui.add_space(SPACE_1);
    ui.add(egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT2)).wrap());
}
