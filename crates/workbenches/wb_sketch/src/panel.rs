//! The sketcher's task panel: attachment picker, tool settings, solver
//! messages, edit controls, and the constraint and element lists.

use egui::RichText;
use uuid::Uuid;

use core_document::{TaskOutcome, TaskRequest, WorkbenchRuntimeContext};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, QtyField, check_row, mono_label, note_card, secondary_button, section_header,
    select_field,
};
use ui_kit::{mono, sans};

use crate::sketch::{self, Constraint, Sketch, SketchPlane};
use crate::solver;
use crate::style::{constraint_icon, element_icon, element_kind, element_name};
use crate::{ConstraintFilter, ElementFilter, SketchWorkbench, overlay};

impl SketchWorkbench {
    /// The panel's Solve now: the solver runs whatever the auto-update
    /// switch says.
    fn solve_from_panel(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(mut feature) = self.get_active_sketch(ctx) {
            self.solve_now(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
    }

    /// Make sure a diagnosis is cached, then read the verdict.
    fn diagnosed_verdict(&mut self, sketch: &Sketch) -> crate::SolverVerdict {
        if self.last_diagnosis.is_none() {
            self.last_diagnosis = Some(solver::diagnose(sketch));
        }
        self.solver_verdict(sketch)
    }

    /// The task panel body. Returns how the task ended, if it did.
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        self.sync_active_sketch_from_ctx(ctx);
        self.dim_edit_window(ui, ctx);
        ui.spacing_mut().item_spacing.y = SPACE_2;

        if self.pending_creation.is_some() {
            if request.cancel {
                self.pending_creation = None;
                return TaskOutcome::Cancelled;
            }
            self.attachment_card(ui, ctx);
            return TaskOutcome::Open;
        }

        let Some(feature) = self.get_active_sketch(ctx) else {
            return TaskOutcome::Open;
        };
        if request.cancel
            && let (Some(id), Some(start)) = (self.active_sketch_id, self.session_start.clone())
            && ctx.document.get_feature_data(id) != Some(&start)
        {
            // Every edit so far recorded first, then the sketch put back.
            self.flush_draw_record(ctx);
            let args = crate::commands::args(serde_json::json!({
                "sketch": id.0.to_string(),
                "data": start,
            }));
            if crate::commands::run("sketch.restore", &args, ctx).is_ok() {
                ctx.record("sketch.restore", args, serde_json::Value::Null);
            }
            ctx.request(core_document::HostRequest::FinishEditing);
            return TaskOutcome::Accepted {
                label: format!("Cancel editing {}", feature.sketch.name),
            };
        }
        if request.accept || request.cancel {
            ctx.request(core_document::HostRequest::FinishEditing);
            return TaskOutcome::Accepted {
                label: format!("Edit {}", feature.sketch.name),
            };
        }
        // A generated sketch is edited by its numbers.
        if let (Some(id), true) = (self.active_sketch_id, feature.generator.is_some()) {
            crate::generator::panel::show(ui, ctx, id);
            return TaskOutcome::Open;
        }
        let plane = feature.plane;
        let support = feature.support.clone();
        let sketch = feature.sketch;

        if self.sketch_picker.is_some() {
            self.sketch_picker_section(ui, ctx, &plane);
        }
        self.tool_section(ui);
        self.attachment_section(ui, ctx, support.as_ref());
        self.array_section(ui, ctx);
        self.spline_section(ui, ctx, &sketch);
        self.text_section(ui, ctx, &sketch);
        self.images_section(ui, ctx, &sketch);
        self.solver_section(ui, ctx, &sketch);
        self.edit_controls_section(ui);
        self.constraints_section(ui, ctx, &sketch);
        self.elements_section(ui, ctx, &sketch);

        ui.add_space(SPACE_2);
        ui.label(
            RichText::new("Orbit is locked to the sketch plane while a sketch is in edit mode.")
                .font(sans(11.5))
                .color(TEXT3),
        );
        TaskOutcome::Open
    }

    /// The document's other sketches, for carbon copy (a click copies one in)
    /// or merge (tick some, then merge). A sketch whose plane is at an angle
    /// to this one is listed but not offered.
    fn sketch_picker_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        plane: &SketchPlane,
    ) {
        let Some(mode) = self.sketch_picker.as_ref().map(|p| p.mode) else {
            return;
        };
        if mode == crate::SketchPickerMode::ExternalFrom {
            self.external_from_section(ui, ctx);
            return;
        }
        let merging = mode == crate::SketchPickerMode::Merge;
        let title = if merging {
            "Merge sketches"
        } else {
            "Carbon copy"
        };
        if !section_header(ui, "sketch_picker", title, None, true) {
            return;
        }
        let others = self.other_sketches(ctx);
        if others.is_empty() {
            note_card(
                ui,
                Note::Info,
                None,
                "There is no other sketch in the document.",
            );
        }
        let mut copy_from = None;
        for (id, name, other) in &others {
            let parallel = crate::plane_map(&other.plane, plane).is_ok();
            let why = "Its plane is at an angle to this one";
            if merging {
                let picker = self.sketch_picker.as_mut().expect("open");
                let mut on = picker.checked.contains(id);
                let response = ui.add_enabled_ui(parallel, |ui| check_row(ui, &mut on, name));
                if response.inner.changed() {
                    if on {
                        picker.checked.insert(*id);
                    } else {
                        picker.checked.remove(id);
                    }
                }
                if !parallel {
                    response.response.on_hover_text(why);
                }
            } else {
                let response = ui.add_enabled(parallel, egui::Button::new(name.as_str()));
                if response.clicked() {
                    copy_from = Some(*id);
                }
                if !parallel {
                    response.on_disabled_hover_text(why);
                }
            }
        }
        if let Some(id) = copy_from {
            self.carbon_copy(ctx, id);
        }
        if merging {
            let ticked = self
                .sketch_picker
                .as_ref()
                .is_some_and(|p| !p.checked.is_empty());
            if ui
                .add_enabled(ticked, egui::Button::new("Merge into a new sketch"))
                .on_hover_text(
                    "A new sketch of this one and the ticked ones; they stay as they are",
                )
                .clicked()
            {
                self.merge_sketches(ctx);
            }
        }
        if secondary_button(ui, "Close").clicked() {
            self.sketch_picker = None;
        }
        ui.add_space(SPACE_2);
    }

    /// Where a sketch attached to a datum sits on it: along the normal,
    /// across the plane and turned about it.
    fn attachment_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        support: Option<&crate::feature::DatumSupport>,
    ) {
        let (Some(support), Some(id)) = (support, self.active_sketch_id) else {
            return;
        };
        if !section_header(ui, "sketch_attachment", "Attachment", None, false) {
            return;
        }
        let (mut offset, mut shift, mut turn) = (support.offset, support.shift, support.turn);
        let mut changed = false;
        egui::Grid::new("sketch_attachment_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_1])
            .show(ui, |ui| {
                ui_kit::widgets::field_label(ui, "Along normal");
                changed |= QtyField::offset(&mut offset).show(ui);
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Across x");
                changed |= QtyField::offset(&mut shift[0]).show(ui);
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Across y");
                changed |= QtyField::offset(&mut shift[1]).show(ui);
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Turned");
                changed |= QtyField::degrees(&mut turn).show(ui);
                ui.end_row();
            });
        if changed {
            let args = crate::commands::args(serde_json::json!({
                "sketch": id.0.to_string(),
                "offset": offset,
                "shift": shift,
                "turn": turn,
            }));
            match crate::commands::run("sketch.attachment", &args, ctx) {
                Ok(_) => ctx.record("sketch.attachment", args, serde_json::Value::Null),
                Err(err) => ctx.log_warn(err.to_string()),
            }
        }
    }

    /// The document's other sketches and its datums, one click bringing
    /// one in as external geometry that follows it.
    fn external_from_section(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        if !section_header(ui, "sketch_external_from", "External from", None, true) {
            return;
        }
        let mut choices: Vec<(core_document::FeatureId, String)> = self
            .other_sketches(ctx)
            .into_iter()
            .map(|(id, name, _)| (id, name))
            .collect();
        let mut datums: Vec<(u64, core_document::FeatureId, String)> = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == core_document::DATUM_KIND)
            .map(|(id, n)| (n.seq, *id, n.name.clone()))
            .collect();
        datums.sort_by_key(|(seq, id, _)| (*seq, *id));
        choices.extend(datums.into_iter().map(|(_, id, name)| (id, name)));
        if choices.is_empty() {
            note_card(
                ui,
                Note::Info,
                None,
                "There is no other sketch and no datum in the document.",
            );
        }
        let mut chosen = None;
        for (id, name) in &choices {
            if ui.button(name.as_str()).clicked() {
                chosen = Some(*id);
            }
        }
        if let (Some(from), Some(sketch)) = (chosen, self.active_sketch_id) {
            let args = crate::commands::args(serde_json::json!({
                "sketch": sketch.0.to_string(),
                "from": from.0.to_string(),
                "counts": !self.construction_mode,
            }));
            match crate::commands::run("sketch.external_from", &args, ctx) {
                Ok(made) => {
                    ctx.record("sketch.external_from", args, made);
                    self.sketch_picker = None;
                }
                Err(err) => ctx.log_warn(err.to_string()),
            }
        }
        if secondary_button(ui, "Close").clicked() {
            self.sketch_picker = None;
        }
        ui.add_space(SPACE_2);
    }

    /// Plane picker for a pending sketch creation.
    fn attachment_card(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        let Some(pending) = &self.pending_creation else {
            return;
        };
        let body = pending.body;
        let face = pending.face;
        let face_origin = pending.face_origin;
        let made_by = pending.generator.clone();
        type Choice = (
            SketchPlane,
            Option<crate::feature::DatumSupport>,
            Option<crate::feature::FaceSupport>,
        );
        let mut chosen: Option<Choice> = None;
        let mut by_mode: Option<core_document::DatumAttachment> = None;
        let mut base: Option<core_document::BasePlane> = None;
        let mut cancel = false;
        ui_kit::widgets::Card::new().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui_kit::icon::draw(ui, "sketch-map", 18.0, ACCENT);
                ui.label(
                    RichText::new("Sketch attachment")
                        .font(ui_kit::sans_semibold(FONT_MD))
                        .color(TEXT1),
                );
            });
            let ask = match &made_by {
                Some(made_by) => format!("Choose the plane for the {}.", made_by.base_name()),
                None => "Choose the plane to sketch on.".to_string(),
            };
            let ask = format!("{ask} An origin plane can be clicked in the view too.");
            ui.label(RichText::new(ask).font(sans(FONT_SM)).color(TEXT2));
            ui.add_space(SPACE_1);
            if let Some(face) = face
                && secondary_button(ui, "Selected face")
                    .on_hover_text(
                        "Sketch on the face you clicked on the solid; the sketch follows the face",
                    )
                    .clicked()
            {
                // On the face itself, not the drawn mesh the click met.
                let face = face.on_its_plane();
                let mut plane = SketchPlane::from_face(face.point, face.normal);
                // A generator stands where the face was clicked, a keyway
                // running along the shaft it is clicked on.
                if let Some(made_by) = &made_by {
                    let x_axis = made_by.runs_along(&face).unwrap_or(plane.x_axis);
                    plane = SketchPlane::from_frame(face.point, plane.normal, x_axis);
                }
                // A face of the sketch's own body, or one it borrows, is
                // followed.
                let follows = body.and_then(|_| {
                    crate::feature::FaceSupport::from_origin(&face, face_origin, plane)
                });
                chosen = Some((plane, None, follows));
            }
            ui.horizontal(|ui| {
                if secondary_button(ui, "Top (XY)").clicked() {
                    base = Some(core_document::BasePlane::XY);
                }
                if secondary_button(ui, "Front (XZ)").clicked() {
                    base = Some(core_document::BasePlane::XZ);
                }
                if secondary_button(ui, "Side (YZ)").clicked() {
                    base = Some(core_document::BasePlane::YZ);
                }
            });
            // Attached as a datum plane would be, by a mode on what is
            // selected on the body.
            if let Some(body) = body {
                use core_document::attach::{PICK_MODES, Picked, candidate, mode_label};
                let picked = Picked::of(ctx, body);
                let frame = core_document::DatumFrame {
                    origin: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                    x_axis: [1.0, 0.0, 0.0],
                };
                ui.label(
                    RichText::new("Or by a mode, on what is selected")
                        .font(sans(FONT_XS))
                        .color(TEXT3),
                );
                ui.horizontal_wrapped(|ui| {
                    for (mode, needs) in PICK_MODES {
                        let made = candidate(mode, ctx, body, frame, None, &picked);
                        let response = ui.add_enabled(
                            made.is_some(),
                            egui::Button::new(RichText::new(mode_label(mode)).font(sans(FONT_XS))),
                        );
                        let response = if needs.is_empty() {
                            response
                        } else {
                            response.on_disabled_hover_text(*needs)
                        };
                        if response.clicked() {
                            by_mode = made;
                        }
                    }
                });
            }
            // Datum planes of the target body attach the sketch to their
            // resolved frame (toponaming-safe anchor).
            if let Some(body) = body {
                for (datum_id, name, datum) in core_document::datums_of_body(ctx.document, body) {
                    let frame = datum.frame();
                    match datum.shape {
                        core_document::DatumShape::Plane { .. } => {
                            if secondary_button(ui, &name)
                                .on_hover_text("Sketch on this datum plane")
                                .clicked()
                            {
                                chosen = Some((
                                    SketchPlane::from_frame(
                                        frame.origin,
                                        frame.normal,
                                        frame.x_axis,
                                    ),
                                    Some(crate::feature::DatumSupport {
                                        datum: datum_id,
                                        plane: None,
                                        offset: 0.0,
                                        shift: [0.0, 0.0],
                                        turn: 0.0,
                                    }),
                                    None,
                                ));
                            }
                        }
                        core_document::DatumShape::CoordinateSystem { .. } => {
                            for (plane, at) in frame.planes() {
                                if secondary_button(ui, &format!("{name} {plane}"))
                                    .on_hover_text("Sketch on this plane of the coordinate system")
                                    .clicked()
                                {
                                    chosen = Some((
                                        SketchPlane::from_frame(at.origin, at.normal, at.x_axis),
                                        Some(crate::feature::DatumSupport {
                                            datum: datum_id,
                                            plane: Some(plane.to_string()),
                                            offset: 0.0,
                                            shift: [0.0, 0.0],
                                            turn: 0.0,
                                        }),
                                        None,
                                    ));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            ui.add_space(SPACE_1);
            if secondary_button(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
        if cancel {
            self.pending_creation = None;
        }
        if let Some(plane) = base {
            self.create_on_base_plane(ctx, plane);
        }
        if let Some((plane, support, face)) = chosen {
            self.pending_creation = None;
            self.create_sketch_on_plane(
                ctx,
                body,
                plane,
                crate::NewSketchOn {
                    support,
                    face,
                    attached: None,
                    made_by: made_by.clone(),
                },
            );
        }
        if let (Some(attachment), Some(on)) = (by_mode, body) {
            match crate::commands::settle_attached(ctx, on, attachment, Default::default()) {
                Ok(attached) => {
                    self.pending_creation = None;
                    self.create_sketch_on_plane(
                        ctx,
                        body,
                        attached.plane(),
                        crate::NewSketchOn {
                            attached: Some(attached),
                            made_by,
                            ..Default::default()
                        },
                    );
                }
                Err(why) => ctx.log_warn(format!("Cannot attach the sketch there: {why}")),
            }
        }
    }

    /// With something selected: an array of it, its rows, columns and
    /// steps set here before it is made.
    fn array_section(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        if self.selected.is_empty() || !self.tool_state.is_idle() {
            return;
        }
        if !section_header(ui, "sketch_array", "Array", None, false) {
            return;
        }
        let params = &mut self.tool_params;
        egui::Grid::new("sketch_array_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_1])
            .show(ui, |ui| {
                for (label, value) in [
                    ("Rows", &mut params.array_rows),
                    ("Columns", &mut params.array_cols),
                ] {
                    ui_kit::widgets::field_label(ui, label);
                    let mut n = *value as f32;
                    if QtyField::new(&mut n)
                        .decimals(0)
                        .speed(0.1)
                        .range(1.0..=64.0)
                        .show(ui)
                    {
                        *value = n.round() as u32;
                    }
                    ui.end_row();
                }
                ui_kit::widgets::field_label(ui, "Column step");
                QtyField::offset(&mut params.array_dx).show(ui);
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Row step");
                QtyField::offset(&mut params.array_dy).show(ui);
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Linked");
                check_row(ui, &mut params.copies_linked, "Follow the original").on_hover_text(
                    "Copies stay the original's size, spaced by one pitch along the rows and one down the columns",
                );
                ui.end_row();
            });
        if ui_kit::widgets::secondary_button(ui, "Make array")
            .on_hover_text("Copy the selection into these rows and columns")
            .clicked()
        {
            self.array_selection(ctx);
        }
    }

    /// Settings of the active drawing tool, when it has any.
    fn tool_section(&mut self, ui: &mut egui::Ui) {
        let tool = self.last_tool.clone();
        let has_settings = matches!(
            tool.as_deref(),
            Some(
                "sketch.polygon"
                    | "sketch.slot"
                    | "sketch.arc_slot"
                    | "sketch.fillet"
                    | "sketch.chamfer"
                    | "sketch.offset"
                    | "sketch.translate"
                    | "sketch.rotate"
                    | "sketch.scale"
                    | "sketch.mirror"
                    | "sketch.bspline"
                    | "sketch.rect_rounded"
                    | "sketch.rect_frame"
                    | "sketch.text"
            )
        );
        if !has_settings {
            return;
        }
        if !section_header(ui, "sketch_tool", "Tool", None, true) {
            return;
        }
        egui::Grid::new("sketch_tool_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_2])
            .show(ui, |ui| {
                let params = &mut self.tool_params;
                match tool.as_deref() {
                    Some("sketch.polygon") => {
                        ui_kit::widgets::field_label(ui, "Sides");
                        let mut sides = params.polygon_sides as f32;
                        if QtyField::new(&mut sides)
                            .decimals(0)
                            .speed(0.1)
                            .range(3.0..=12.0)
                            .show(ui)
                        {
                            params.polygon_sides = sides.round() as u32;
                        }
                        ui.end_row();
                    }
                    Some("sketch.text") => {
                        text_fields(ui, "sketch_text_draft", &mut self.text_draft);
                    }
                    Some("sketch.slot" | "sketch.arc_slot") => {
                        ui_kit::widgets::field_label(ui, "Width");
                        QtyField::mm(&mut params.slot_width).show(ui);
                        ui.end_row();
                    }
                    Some("sketch.fillet" | "sketch.rect_rounded") => {
                        ui_kit::widgets::field_label(ui, "Radius");
                        QtyField::mm(&mut params.fillet_radius).show(ui);
                        ui.end_row();
                        if tool.as_deref() == Some("sketch.fillet") {
                            corner_row(ui, &mut params.corner_keep);
                        }
                    }
                    Some("sketch.chamfer") => {
                        ui_kit::widgets::field_label(ui, "Length");
                        QtyField::mm(&mut params.chamfer_length).show(ui);
                        ui.end_row();
                        corner_row(ui, &mut params.corner_keep);
                    }
                    Some("sketch.rect_frame") => {
                        ui_kit::widgets::field_label(ui, "Wall");
                        QtyField::mm(&mut params.offset_distance).show(ui);
                        ui.end_row();
                    }
                    Some("sketch.offset") => {
                        ui_kit::widgets::field_label(ui, "Distance");
                        QtyField::mm(&mut params.offset_distance).show(ui);
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Corners");
                        check_row(ui, &mut params.offset_round, "Round")
                            .on_hover_text("Arcs join the copies where they part at a corner");
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Sides");
                        check_row(ui, &mut params.offset_both, "Both");
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Original");
                        check_row(ui, &mut params.offset_delete, "Replace it");
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Copy");
                        ui.add_enabled_ui(!params.offset_delete, |ui| {
                            check_row(ui, &mut params.offset_linked, "Follows the original")
                                .on_hover_text(
                                    "One offset dimension holds the copy to the original",
                                );
                        });
                        ui.end_row();
                    }
                    Some("sketch.mirror") => {
                        ui_kit::widgets::field_label(ui, "Original");
                        check_row(ui, &mut params.mirror_keep, "Keep it");
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Image");
                        ui.add_enabled_ui(params.mirror_keep, |ui| {
                            check_row(ui, &mut params.mirror_linked, "Follows the original")
                                .on_hover_text(
                                    "Symmetric constraints hold the image to the original",
                                );
                        });
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "About");
                        check_row(ui, &mut params.mirror_center, "A point")
                            .on_hover_text("One click, the centre; off, a line or two points");
                        ui.end_row();
                    }
                    Some("sketch.translate" | "sketch.rotate" | "sketch.scale") => {
                        ui_kit::widgets::field_label(ui, "Copies");
                        let mut copies = params.copies as f32;
                        if QtyField::new(&mut copies)
                            .decimals(0)
                            .speed(0.1)
                            .range(0.0..=64.0)
                            .show(ui)
                        {
                            params.copies = copies.round() as u32;
                        }
                        ui.end_row();
                        if tool.as_deref() != Some("sketch.scale") {
                            ui_kit::widgets::field_label(ui, "Linked");
                            ui.add_enabled_ui(params.copies > 0, |ui| {
                                check_row(ui, &mut params.copies_linked, "Follow the original")
                                    .on_hover_text(
                                        "Copies stay the original's size, spaced by one pitch",
                                    );
                            });
                            ui.end_row();
                        }
                    }
                    Some("sketch.bspline") => {
                        ui_kit::widgets::field_label(ui, "Closed");
                        check_row(ui, &mut params.bspline_periodic, "Periodic");
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Clicks");
                        check_row(ui, &mut params.bspline_interpolate, "Through points");
                        ui.end_row();
                        ui_kit::widgets::field_label(ui, "Degree");
                        let mut degree = params.bspline_degree as f32;
                        if QtyField::new(&mut degree)
                            .decimals(0)
                            .speed(0.05)
                            .range(2.0..=f64::from(crate::spline::MAX_DEGREE))
                            .show(ui)
                        {
                            params.bspline_degree = degree.round() as u32;
                        }
                        ui.end_row();
                    }
                    _ => {}
                }
            });
    }

    /// With a piece of a text block selected: what it says and how, made
    /// again when a field is changed.
    fn text_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) {
        let Some(block) = self
            .selected
            .iter()
            .find_map(|id| crate::text::block_of(sketch, *id))
        else {
            return;
        };
        if !section_header(ui, "sketch_text", "Text", None, true) {
            return;
        }
        let id = block.id;
        let mut spec = crate::text::TextSpec::of(block);
        let mut changed = false;
        egui::Grid::new("sketch_text_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_1])
            .show(ui, |ui| {
                changed = text_fields(ui, "sketch_text_block", &mut spec);
            });
        if changed {
            let mut args = crate::text_args(&spec, None);
            args["block"] = serde_json::json!(id.to_string());
            self.sketch_edit(
                ctx,
                "sketch.text_edit",
                args,
                |s| match crate::text::change(s, id, &spec) {
                    Ok(()) => crate::tools::ToolEffect::changed("Text changed"),
                    Err(why) => crate::tools::ToolEffect::log(why),
                },
            );
        }
    }

    /// With one spline selected: its degree and its knots, each with how
    /// many times it stands; with one of a spline's control points, its
    /// weight.
    fn spline_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) {
        let splines: Vec<&sketch::BSpline> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                sketch::GeometryElement::BSpline(b) if self.selected.contains(&b.id) => Some(b),
                _ => None,
            })
            .collect();
        let weighted = self.selected.iter().find_map(|id| {
            sketch.geometry.iter().find_map(|g| match g {
                sketch::GeometryElement::BSpline(b) if b.control_points.contains(id) => {
                    Some((b.id, *id))
                }
                _ => None,
            })
        });
        if splines.len() != 1 && weighted.is_none() {
            return;
        }
        if !section_header(ui, "sketch_spline", "Spline", None, true) {
            return;
        }
        enum Change {
            Degree(i32),
            Multiplicity(f64, usize),
            Weight(Uuid, Uuid, f64),
        }
        let mut change = None;
        let spline = splines.first().map(|b| (b.id, b.degree));
        egui::Grid::new("sketch_spline_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_1])
            .show(ui, |ui| {
                if let Some((id, degree)) = spline {
                    ui_kit::widgets::field_label(ui, "Degree");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(degree > 1, egui::Button::new("−"))
                            .on_hover_text("Lower the degree: the nearest curve of one less")
                            .clicked()
                        {
                            change = Some(Change::Degree(-1));
                        }
                        mono_label(ui, degree.to_string(), FONT_SM, TEXT1);
                        if ui
                            .add_enabled(degree < crate::spline::MAX_DEGREE, egui::Button::new("+"))
                            .on_hover_text("Raise the degree, the curve unchanged")
                            .clicked()
                        {
                            change = Some(Change::Degree(1));
                        }
                    });
                    ui.end_row();
                    for (knot, times) in crate::spline_edit::knots_of(sketch, id) {
                        ui_kit::widgets::field_label(ui, &format!("Knot {knot:.3}"));
                        ui.horizontal(|ui| {
                            if ui
                                .button("−")
                                .on_hover_text(if times > 1 {
                                    "Stand once less: a smoother curve near it"
                                } else {
                                    "Remove the knot: the nearest curve without it"
                                })
                                .clicked()
                            {
                                change = Some(Change::Multiplicity(knot, times - 1));
                            }
                            mono_label(ui, format!("×{times}"), FONT_SM, TEXT1);
                            if ui
                                .add_enabled(times < degree as usize, egui::Button::new("+"))
                                .on_hover_text("Stand once more, the curve unchanged")
                                .clicked()
                            {
                                change = Some(Change::Multiplicity(knot, times + 1));
                            }
                        });
                        ui.end_row();
                    }
                }
                if let Some((owner, point)) = weighted {
                    ui_kit::widgets::field_label(ui, "Weight");
                    let now = crate::spline_edit::weight_of(sketch, owner, point).unwrap_or(1.0);
                    let mut w = now as f32;
                    if QtyField::new(&mut w)
                        .decimals(3)
                        .speed(0.01)
                        .range(0.01..=100.0)
                        .show(ui)
                        && f64::from(w) != now
                    {
                        change = Some(Change::Weight(owner, point, f64::from(w)));
                    }
                    ui.end_row();
                }
            });
        let ids = |id: Uuid| serde_json::json!(id.to_string());
        match change {
            Some(Change::Degree(by)) => {
                self.selection_edit(
                    ctx,
                    "sketch.spline_degree",
                    serde_json::json!({ "by": by }),
                    |s, sel| crate::spline_edit::change_degree(s, sel, by),
                );
            }
            Some(Change::Multiplicity(knot, times)) => {
                let Some((id, _)) = spline else {
                    return;
                };
                self.sketch_edit(
                    ctx,
                    "sketch.knot_multiplicity",
                    serde_json::json!({ "spline": ids(id), "knot": knot, "multiplicity": times }),
                    |s| {
                        if times == 0 {
                            crate::spline_edit::remove_knot(s, id, knot)
                        } else {
                            crate::spline_edit::set_multiplicity(s, id, knot, times)
                        }
                    },
                );
            }
            Some(Change::Weight(owner, point, w)) => {
                self.sketch_edit(
                    ctx,
                    "sketch.spline_weight",
                    serde_json::json!({ "spline": ids(owner), "point": ids(point), "weight": w }),
                    |s| crate::spline_edit::set_weight(s, owner, point, w),
                );
            }
            None => {}
        }
    }

    /// The sketch's reference pictures: each one's middle, width, turn and
    /// how much of it shows, and a way to take it away.
    fn images_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) {
        if sketch.images.is_empty()
            || !section_header(ui, "sketch_images", "Reference images", None, true)
        {
            return;
        }
        let Some(sketch_id) = self.active_sketch_id else {
            return;
        };
        let mut change: Option<serde_json::Value> = None;
        for (n, image) in sketch.images.iter().enumerate() {
            let (mut x, mut y, mut width, mut angle, mut opacity) = (
                image.center.x,
                image.center.y,
                image.width,
                image.angle_deg,
                image.opacity,
            );
            let id = serde_json::json!(image.id.to_string());
            ui.label(
                RichText::new(format!("Picture {}", n + 1))
                    .font(sans(FONT_XS))
                    .color(TEXT3),
            );
            egui::Grid::new(("sketch_image_grid", image.id))
                .num_columns(2)
                .spacing([SPACE_2, SPACE_1])
                .show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, label: &str, field: QtyField, key: &str| {
                        ui_kit::widgets::field_label(ui, label);
                        let changed = field.show(ui);
                        ui.end_row();
                        changed.then(|| key.to_string())
                    };
                    let edits = [
                        row(ui, "Middle x", QtyField::new(&mut x).unit("mm"), "x"),
                        row(ui, "Middle y", QtyField::new(&mut y).unit("mm"), "y"),
                        row(
                            ui,
                            "Width",
                            QtyField::new(&mut width).unit("mm").range(0.01..=1.0e6),
                            "width",
                        ),
                        row(ui, "Turn", QtyField::degrees(&mut angle), "angle"),
                        row(
                            ui,
                            "Opacity",
                            QtyField::new(&mut opacity)
                                .range(0.05..=1.0)
                                .speed(0.01)
                                .decimals(2),
                            "opacity",
                        ),
                    ];
                    if edits.iter().any(Option::is_some) {
                        change = Some(serde_json::json!({
                            "image": id, "x": x, "y": y, "width": width,
                            "angle": angle, "opacity": opacity,
                        }));
                    }
                });
            if secondary_button(ui, "Remove").clicked() {
                change = Some(serde_json::json!({"image": id, "remove": true}));
            }
            ui.add_space(SPACE_1);
        }
        if let Some(mut args) = change {
            args["sketch"] = serde_json::json!(sketch_id.0.to_string());
            let args = crate::commands::args(args);
            if crate::commands::run("sketch.set_image", &args, ctx).is_ok() {
                ctx.record("sketch.set_image", args, serde_json::Value::Null);
            }
        }
    }

    fn solver_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) {
        if !section_header(ui, "sketch_solver", "Solver messages", None, true) {
            return;
        }
        let state = self.diagnosed_verdict(sketch);
        let note = match state.kind {
            crate::SolverKind::Fully => Note::Success,
            crate::SolverKind::Conflicting => Note::Error,
            crate::SolverKind::Redundant => Note::Warning,
            _ => Note::Info,
        };
        let response = note_card(ui, note, Some(&state.title), &state.body);
        if !state.offenders.is_empty()
            && response
                .interact(egui::Sense::click())
                .on_hover_text("Click to select the offending constraints' geometry")
                .clicked()
        {
            self.selected.clear();
            for constraint in &sketch.constraints {
                if state.offenders.contains(&constraint.id) {
                    self.selected
                        .extend(sketch::constraint_refs(&constraint.kind));
                }
            }
        }
        ui.horizontal(|ui| {
            check_row(ui, &mut self.options.auto_update, "Auto update");
            check_row(
                ui,
                &mut self.options.auto_remove_redundant,
                "Auto remove redundants",
            );
            if !self.options.auto_update
                && ui
                    .button(RichText::new("Solve now").font(sans(FONT_XS)))
                    .clicked()
            {
                self.solve_from_panel(ctx);
            }
        });
        // How the last solve went, and how far the solver may go.
        let last = match self.last_solve {
            Some(crate::solver::SolveOutcome::Converged { iterations }) => {
                format!("Last solve: settled in {iterations} step(s)")
            }
            Some(crate::solver::SolveOutcome::NotConverged { residual }) => {
                format!("Last solve: stopped with {residual:.2e} left")
            }
            Some(crate::solver::SolveOutcome::NothingToSolve) => {
                "Last solve: nothing to solve".to_string()
            }
            None => String::new(),
        };
        if !last.is_empty() {
            ui.label(RichText::new(last).font(sans(FONT_XS)).color(TEXT3));
        }
        let mut iterations = sketch.solver.max_iterations as f32;
        let mut exponent = -(sketch.solver.tolerance.log10().round() as i32);
        let mut changed = None;
        egui::Grid::new("sketch_solver_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_1])
            .show(ui, |ui| {
                ui_kit::widgets::field_label(ui, "Most steps");
                if QtyField::new(&mut iterations)
                    .decimals(0)
                    .speed(1.0)
                    .range(1.0..=10_000.0)
                    .show(ui)
                {
                    changed = Some(serde_json::json!({"iterations": iterations.round()}));
                }
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Solved within");
                let options: Vec<(i32, String)> =
                    (4..=12).map(|e| (e, format!("1e-{e}"))).collect();
                let options: Vec<(i32, &str)> =
                    options.iter().map(|(e, t)| (*e, t.as_str())).collect();
                if select_field(ui, "sketch_solver_tolerance", &mut exponent, &options, 90.0) {
                    changed = Some(serde_json::json!({"tolerance": 10f64.powi(-exponent)}));
                }
                ui.end_row();
            });
        if let (Some(mut args), Some(id)) = (changed, self.active_sketch_id) {
            args["sketch"] = serde_json::json!(id.0.to_string());
            let args = crate::commands::args(args);
            if crate::commands::run("sketch.solver_settings", &args, ctx).is_ok() {
                ctx.record("sketch.solver_settings", args, serde_json::Value::Null);
                self.last_diagnosis = None;
            }
        }
    }

    fn edit_controls_section(&mut self, ui: &mut egui::Ui) {
        if !section_header(ui, "sketch_edit_controls", "Edit controls", None, false) {
            return;
        }
        check_row(ui, &mut self.options.grid_on, "Show grid");
        check_row(ui, &mut self.options.grid_auto, "Grid auto spacing");
        check_row(ui, &mut self.options.grid_snap, "Snap to grid");
        let mut snap = !self.snap_off;
        if check_row(ui, &mut snap, "Snap to objects").changed() {
            self.snap_off = !snap;
        }
        check_row(
            ui,
            &mut self.options.avoid_redundant_auto,
            "Avoid redundant auto constraints",
        );
        check_row(ui, &mut self.options.auto_constraints, "Auto constraints");
        egui::Grid::new("sketch_edit_grid")
            .num_columns(2)
            .spacing([SPACE_2, SPACE_1])
            .show(ui, |ui| {
                ui.label(RichText::new("Grid size").font(sans(FONT_XS)).color(TEXT2));
                ui.label(
                    RichText::new("Rendering order")
                        .font(sans(FONT_XS))
                        .color(TEXT2),
                );
                ui.end_row();
                ui.add_enabled_ui(!self.options.grid_auto, |ui| {
                    QtyField::mm(&mut self.options.grid_size)
                        .width(90.0)
                        .show(ui);
                });
                select_field(
                    ui,
                    "sketch_render_order",
                    &mut self.options.construction_on_top,
                    &[(false, "Normal on top"), (true, "Construction on top")],
                    130.0,
                );
                ui.end_row();
            });
    }

    fn constraints_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) {
        let open = section_header(
            ui,
            "sketch_constraints",
            "Constraints",
            Some(sketch.constraints.len()),
            true,
        );
        if !open {
            return;
        }
        ui.horizontal(|ui| {
            ui_kit::icon::draw(ui, "search", 11.0, TEXT3);
            ui.add(
                egui::TextEdit::singleline(&mut self.constraint_filter)
                    .desired_width(90.0)
                    .hint_text(RichText::new("Filter").color(TEXT3))
                    .font(sans(FONT_XS)),
            );
            select_field(
                ui,
                "sketch_constraint_filter",
                &mut self.constraint_kind_filter,
                &ConstraintFilter::ALL,
                130.0,
            );
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Labels show")
                    .font(sans(FONT_XS))
                    .color(TEXT2),
            );
            select_field(
                ui,
                "sketch_dimension_labels",
                &mut self.options.dimension_labels,
                &crate::glyphs::DimensionLabels::ALL,
                130.0,
            );
        });
        if sketch.constraints.is_empty() {
            ui.label(
                RichText::new("None yet. Select geometry and pick a constraint tool.")
                    .font(sans(FONT_SM))
                    .color(TEXT3),
            );
            return;
        }
        let diagnosis = self.last_diagnosis.clone().unwrap_or_default();
        let listed: std::collections::HashSet<Uuid> =
            self.listed_constraints(sketch).into_iter().collect();
        let mut delete: Option<usize> = None;
        let mut edited: Option<(usize, Constraint)> = None;
        let mut typed: Option<(Uuid, String)> = None;
        let cell = DimensionCell {
            document: ctx.document,
            sketch_id: self.active_sketch_id,
        };
        let mut clicked: Option<(Uuid, bool)> = None;
        for (idx, constraint) in sketch.constraints.iter().enumerate() {
            if !listed.contains(&constraint.id) {
                continue;
            }
            let label = crate::constraint_row_label(idx, constraint);
            let label = if constraint.parked {
                format!("{label} (parked)")
            } else {
                label
            };
            let selected = self.selected_constraints.contains(&constraint.id);
            let icon_color = if diagnosis.conflicting.contains(&constraint.id) {
                DANGER
            } else if diagnosis.redundant.contains(&constraint.id) {
                WARNING
            } else if !constraint.active {
                TEXT3
            } else if constraint.kind.is_dimensional() && !constraint.driving {
                ACCENT
            } else if cell.sketch_id.is_some_and(|id| {
                cell.document
                    .feature_formula(id, &constraint.id.to_string())
                    .is_some()
            }) {
                SKETCH_FORMULA
            } else {
                SKETCH_CONSTRAINT
            };
            let fill = if selected {
                ACCENT_DIM
            } else {
                egui::Color32::TRANSPARENT
            };
            let hit = list_row(ui, fill, ("constraint_row", idx), |ui| {
                ui_kit::icon::draw(ui, constraint_icon(&constraint.kind), 14.0, icon_color);
                let text_color = if constraint.active { TEXT1 } else { TEXT3 };
                if self.renaming_constraint == Some(constraint.id) {
                    let mut name = constraint.name.clone().unwrap_or_default();
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(120.0)
                            .font(sans(FONT_SM)),
                    );
                    // Focus once, as renaming starts (see the dimension
                    // editor).
                    if !resp.has_focus() && !resp.lost_focus() {
                        resp.request_focus();
                    }
                    if resp.changed() {
                        let mut c = constraint.clone();
                        c.name = (!name.is_empty()).then_some(name);
                        edited = Some((idx, c));
                    }
                    if resp.lost_focus() {
                        self.renaming_constraint = None;
                    }
                } else {
                    ui.label(RichText::new(&label).font(sans(FONT_SM)).color(text_color))
                        .on_hover_text(sketch::constraint_label(&constraint.kind));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dimension_value_cell(
                        ui,
                        sketch,
                        constraint,
                        &cell,
                        &mut typed,
                        self.pending_focus == Some(constraint.id),
                    );
                });
            });
            if hit.clicked() {
                clicked = Some((constraint.id, ctx.ctrl_down));
            }
            hit.context_menu(|ui| {
                let active_label = if constraint.active {
                    "Deactivate"
                } else {
                    "Activate"
                };
                if ui.button(active_label).clicked() {
                    let mut c = constraint.clone();
                    c.active = !c.active;
                    edited = Some((idx, c));
                    ui.close();
                }
                if constraint.kind.is_dimensional() {
                    let driving_label = if constraint.driving {
                        "Make reference"
                    } else {
                        "Make driving"
                    };
                    if ui
                        .button(driving_label)
                        .on_hover_text("Reference dimensions are measured, not enforced")
                        .clicked()
                    {
                        let mut c = constraint.clone();
                        c.driving = !c.driving;
                        edited = Some((idx, c));
                        ui.close();
                    }
                }
                if ui.button("Rename").clicked() {
                    self.renaming_constraint = Some(constraint.id);
                    ui.close();
                }
                let park_label = if constraint.parked { "Unpark" } else { "Park" };
                if ui
                    .button(park_label)
                    .on_hover_text("Parked symbols draw only while the parked layer shows")
                    .clicked()
                {
                    let mut c = constraint.clone();
                    c.parked = !c.parked;
                    edited = Some((idx, c));
                    ui.close();
                }
                ui.separator();
                if ui.button("Delete").clicked() {
                    delete = Some(idx);
                    ui.close();
                }
            });
        }
        self.pending_focus = None; // one-shot: the row grabbed focus
        // As the element rows and the viewport do: a click adds to the
        // selection, a second click takes it out.
        if let Some((id, _)) = clicked
            && !self.selected_constraints.remove(&id)
        {
            self.selected_constraints.insert(id);
        }
        if let Some(idx) = delete
            && let Some(mut feature) = self.get_active_sketch(ctx)
        {
            feature.sketch.constraints.remove(idx);
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
        if let Some((idx, constraint)) = edited {
            self.update_constraint(ctx, idx, constraint);
        }
        if let Some((constraint, text)) = typed {
            self.set_dimension(ctx, constraint, &text, true);
        }
    }

    fn elements_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) {
        let open = section_header(
            ui,
            "sketch_elements",
            "Elements",
            Some(sketch.geometry.len()),
            true,
        );
        if !open {
            return;
        }
        select_field(
            ui,
            "sketch_element_filter",
            &mut self.element_filter,
            &[
                (ElementFilter::All, "All"),
                (ElementFilter::Normal, "Normal"),
                (ElementFilter::Construction, "Construction"),
            ],
            140.0,
        );
        if sketch.geometry.is_empty() {
            ui.label(
                RichText::new("No geometry yet. Draw with the tools above.")
                    .font(sans(FONT_SM))
                    .color(TEXT3),
            );
            return;
        }
        let pal = ctx.sketch_palette;
        let mut delete_element: Option<Uuid> = None;
        let mut toggle_construction: Option<Uuid> = None;
        let mut hovered_row: Option<Uuid> = None;
        let mut clicked: Option<Uuid> = None;

        // The reference geometry every sketch has: pickable from here as
        // well as from the viewport, but never drawn into a shape.
        if self.element_filter == ElementFilter::All {
            for reference in [
                crate::sketch::Reference::Origin,
                crate::sketch::Reference::XAxis,
                crate::sketch::Reference::YAxis,
            ] {
                let id = match reference {
                    crate::sketch::Reference::Origin => crate::sketch::ORIGIN_ID,
                    crate::sketch::Reference::XAxis => crate::sketch::X_AXIS_ID,
                    crate::sketch::Reference::YAxis => crate::sketch::Y_AXIS_ID,
                };
                let fill = if self.selected.contains(&id) {
                    ACCENT_DIM
                } else {
                    egui::Color32::TRANSPARENT
                };
                let hit = list_row(ui, fill, ("reference_row", id), |ui| {
                    let icon = if reference.is_point() {
                        "point"
                    } else {
                        "line"
                    };
                    ui_kit::icon::draw(ui, icon, 14.0, TEXT3);
                    ui.label(
                        RichText::new(reference.label())
                            .font(sans(FONT_SM))
                            .color(TEXT2),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        mono_label(ui, "Reference", FONT_XS, TEXT3);
                    });
                });
                if hit.hovered() {
                    hovered_row = Some(id);
                }
                if hit.clicked() {
                    clicked = Some(id);
                }
            }
        }

        for geom in &sketch.geometry {
            let id = geom.id();
            if !self.element_filter.accepts(sketch, id) {
                continue;
            }
            let style = overlay::element_style(sketch, id, &self.selected, None, &pal);
            let color = egui::Color32::from_rgb(
                (style.color[0] * 255.0) as u8,
                (style.color[1] * 255.0) as u8,
                (style.color[2] * 255.0) as u8,
            );
            let fill = if self.selected.contains(&id) {
                ACCENT_DIM
            } else {
                egui::Color32::TRANSPARENT
            };
            let hit = list_row(ui, fill, ("element_row", id), |ui| {
                ui_kit::icon::draw(ui, element_icon(geom), 14.0, color);
                let mut name = element_name(sketch, id);
                if sketch.is_construction(id) {
                    name.push_str(" (construction)");
                }
                ui.label(RichText::new(name).font(sans(FONT_SM)).color(color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    mono_label(ui, element_kind(geom), FONT_XS, TEXT3);
                });
            });
            if hit.hovered() {
                hovered_row = Some(id);
            }
            if hit.clicked() {
                clicked = Some(id);
            }
            hit.context_menu(|ui| {
                let label = if sketch.is_construction(id) {
                    "Make normal geometry"
                } else {
                    "Make construction"
                };
                if ui.button(label).clicked() {
                    toggle_construction = Some(id);
                    ui.close();
                }
                ui.separator();
                if ui.button("Delete").clicked() {
                    delete_element = Some(id);
                    ui.close();
                }
            });
        }

        if let Some(id) = clicked
            && !self.selected.remove(&id)
        {
            self.selected.insert(id);
        }
        // Hover-in-list highlights in the viewport; when the panel stops
        // hovering, release the highlight for the viewport hit-test.
        if let Some(id) = hovered_row {
            self.hovered = Some(id);
            self.hover_from_panel = true;
        } else if self.hover_from_panel {
            self.hovered = None;
            self.hover_from_panel = false;
        }

        if let Some(id) = delete_element
            && let Some(mut feature) = self.get_active_sketch(ctx)
        {
            let removed = feature.sketch.remove_geometry_cascade(&[id]);
            if !removed.is_empty() {
                for rid in &removed {
                    self.selected.remove(rid);
                }
                self.hovered = None;
                self.solve(ctx, &mut feature);
                ctx.log_info(format!("Deleted {} sketch element(s)", removed.len()));
                self.store_sketch(ctx, feature);
            }
        }
        if let Some(id) = toggle_construction
            && let Some(mut feature) = self.get_active_sketch(ctx)
        {
            let flag = !feature.sketch.is_construction(id);
            feature.sketch.set_construction(id, flag);
            self.store_sketch(ctx, feature);
        }
    }

    /// The dimension editor, a card by the dimension's label: opened by a
    /// double click on the label, or as a tool adds the dimension. The
    /// value is typed and Enter applies it; a variable's chip puts its
    /// name in instead, and "Save as variable" puts the value in a new
    /// variable the dimension then reads.
    pub(crate) fn dim_edit_window(&mut self, ui: &egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        if self.dim_edit.is_none() {
            return;
        }
        let edited = self.dim_edit.as_ref().and_then(|edit| {
            let sketch = self.get_active_sketch(ctx)?;
            let c = sketch
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == edit.constraint)?;
            Some((c.kind.clone(), c.name.clone()))
        });
        // The dimension went (an undo, a delete): so does its editor.
        let Some((kind, constraint_name)) = edited else {
            self.dim_edit = None;
            return;
        };
        let unit = sketch::dimension_unit(&kind);
        let dim = crate::params::formula_dim(&kind);
        let kind_label = sketch::constraint_label(&kind);
        let document: &core_document::Document = ctx.document;
        let choices = document.variables_of(dim);
        let sets: Vec<(core_document::FeatureId, String)> = document
            .variable_sets()
            .into_iter()
            .map(|(id, name, _)| (id, name))
            .collect();
        let length_unit = document.display_unit();
        let Some(edit) = self.dim_edit.as_mut() else {
            return;
        };
        let title = match constraint_name.filter(|n| !n.trim().is_empty()) {
            Some(name) => name,
            None if edit.new => format!("New {}", kind_label.to_lowercase()),
            None => kind_label.clone(),
        };
        let ppp = ui.ctx().pixels_per_point().max(0.1);
        let (vx, vy, ..) = ctx.viewport;
        let pos = egui::pos2(
            (vx as f32 + edit.screen_pos[0]) / ppp + 12.0,
            (vy as f32 + edit.screen_pos[1]) / ppp + 12.0,
        );
        let text_id = egui::Id::new("sketch_dim_edit_text");
        let name_id = egui::Id::new("sketch_dim_edit_name");
        let suffix = unit_suffix(unit);
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("sketch_dim_edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                ui_kit::widgets::Card::floating()
                    .border(BORDER_STRONG)
                    .show(ui, |ui| {
                        ui.set_max_width(FIELD_WIDTH + SPACE_4);
                        ui.spacing_mut().item_spacing.y = SPACE_1;
                        ui.horizontal(|ui| {
                            ui_kit::icon::draw(ui, constraint_icon(&kind), 14.0, TEXT2);
                            ui.label(
                                RichText::new(&title)
                                    .font(ui_kit::sans_medium(FONT_SM))
                                    .color(TEXT1),
                            );
                        });

                        // The value, or a formula; names complete as they
                        // are typed.
                        let completed = ui_kit::completion::completing_text_edit(
                            ui,
                            text_id,
                            &mut edit.text,
                            &|| core_document::formula_candidates(document),
                            |e| {
                                e.desired_width(FIELD_WIDTH)
                                    .font(mono(FONT_MD))
                                    .hint_text("A value, or a formula")
                            },
                        );
                        let response = completed.response;
                        if edit.select_all {
                            // The first key typed takes the whole value.
                            response.request_focus();
                            let n = edit.text.chars().count();
                            select_chars(ui, text_id, 0, n);
                            edit.select_all = false;
                        } else if edit.save_as.is_none()
                            && !response.has_focus()
                            && !response.lost_focus()
                        {
                            // Focus stays here unless the name has it; asked
                            // only while it is elsewhere, so the input
                            // method's composition is left alone.
                            response.request_focus();
                        }
                        let entered = response.lost_focus()
                            && !completed.picked
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));

                        // What it comes to, as typed.
                        let typed = edit.text.trim().to_string();
                        let typed = typed.as_str();
                        let constant = core_document::expr::is_constant(typed);
                        let preview = document.evaluate_formula(typed, Some(dim));
                        let (line, color) = match &preview {
                            Ok(q) if constant => (format!("= {}{suffix}", fmt_value(q.value)), TEXT3),
                            Ok(q) => (
                                format!("ƒ = {}{suffix}", fmt_value(q.value)),
                                SKETCH_FORMULA,
                            ),
                            Err(_) if typed.is_empty() => (String::new(), TEXT3),
                            Err(why) => (why.clone(), DANGER),
                        };
                        if !line.is_empty() {
                            ui.label(RichText::new(line).font(sans(FONT_XS)).color(color));
                        }

                        // The variables it can read, one click each.
                        let shown: Vec<_> = choices.iter().take(MAX_CHIPS).collect();
                        if !shown.is_empty() {
                            ui.add_space(SPACE_1);
                            ui_kit::widgets::overline(ui, "Variables");
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(SPACE_1, SPACE_1);
                                for choice in &shown {
                                    let label = if sets.len() > 1 {
                                        &choice.reference
                                    } else {
                                        &choice.name
                                    };
                                    let chip = ui
                                        .selectable_label(
                                            typed == choice.reference,
                                            RichText::new(format!(
                                                "{label}  {}",
                                                choice.value.display(length_unit, 3)
                                            ))
                                            .font(mono(FONT_XS)),
                                        )
                                        .on_hover_text(format!(
                                            "Set by {}: follows it when it changes",
                                            choice.reference
                                        ));
                                    if chip.clicked() {
                                        edit.text = choice.reference.clone();
                                        edit.save_as = None;
                                        let n = edit.text.chars().count();
                                        select_chars(ui, text_id, n, n);
                                        ui.memory_mut(|m| m.request_focus(text_id));
                                    }
                                }
                            });
                            if choices.len() > shown.len() {
                                ui.label(
                                    RichText::new("Type a name for the rest")
                                        .font(sans(FONT_XS))
                                        .color(TEXT3),
                                );
                            }
                        }

                        // A new variable, from what is typed.
                        let reads_a_variable =
                            choices.iter().any(|c| c.reference == edit.text.trim());
                        let mut name_ok = true;
                        if edit.save_as.is_none() && !reads_a_variable {
                            ui.add_space(SPACE_1);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("+ Save as variable")
                                            .font(sans(FONT_XS))
                                            .color(ACCENT),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(
                                    "Keep this value in a variable, and have the dimension read it",
                                )
                                .clicked()
                            {
                                let set = sets.last().map(|(id, _)| *id);
                                let base = variable_base_name(&kind_label);
                                edit.save_as = Some(crate::SaveAs {
                                    name: document.unused_variable_name(set, &base),
                                    set,
                                });
                                ui.memory_mut(|m| m.request_focus(name_id));
                            }
                        }
                        if let Some(save) = edit.save_as.as_mut() {
                            ui.add_space(SPACE_1);
                            ui_kit::widgets::overline(ui, "Save as variable");
                            let mut close = false;
                            ui.horizontal(|ui| {
                                let name = ui.add(
                                    egui::TextEdit::singleline(&mut save.name)
                                        .id(name_id)
                                        .desired_width(if sets.is_empty() {
                                            FIELD_WIDTH - 24.0
                                        } else {
                                            FIELD_WIDTH * 0.5
                                        })
                                        .font(mono(FONT_SM))
                                        .hint_text("name"),
                                );
                                if name.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    commit = true;
                                }
                                if !sets.is_empty() {
                                    let mut options: Vec<(Option<core_document::FeatureId>, &str)> =
                                        sets.iter().map(|(id, n)| (Some(*id), n.as_str())).collect();
                                    options.push((None, "New set"));
                                    select_field(
                                        ui,
                                        "sketch_dim_edit_set",
                                        &mut save.set,
                                        &options,
                                        FIELD_WIDTH * 0.5 - 30.0,
                                    );
                                }
                                if ui
                                    .add(egui::Button::new(RichText::new("×").color(TEXT3)).frame(false))
                                    .on_hover_text("Keep the value in the dimension only")
                                    .clicked()
                                {
                                    close = true;
                                }
                            });
                            let name = save.name.trim();
                            let set_name = match save.set {
                                Some(set) => sets
                                    .iter()
                                    .find(|(id, _)| *id == set)
                                    .map(|(_, n)| n.clone())
                                    .unwrap_or_default(),
                                None => document.unused_object_name("Variables"),
                            };
                            let exists = choices
                                .iter()
                                .any(|c| Some(c.set) == save.set && c.name == name);
                            let (note, color) = if !core_document::expr::is_valid_name(name) {
                                name_ok = false;
                                ("Give it a name".to_string(), DANGER)
                            } else if exists {
                                (format!("{set_name}.{name} exists: it takes this value"), WARNING)
                            } else if save.set.is_none() {
                                (format!("Makes the set {set_name}; formulas read {set_name}.{name}"), TEXT3)
                            } else {
                                (format!("Formulas read it as {set_name}.{name}"), TEXT3)
                            };
                            ui.label(RichText::new(note).font(sans(FONT_XS)).color(color));
                            if close {
                                edit.save_as = None;
                                ui.memory_mut(|m| m.request_focus(text_id));
                            }
                        }

                        // A plain value can stand as a reference dimension;
                        // one a formula sets drives.
                        if constant && edit.save_as.is_none() {
                            check_row(ui, &mut edit.driving, "Driving")
                                .on_hover_text("Off = reference dimension (measured, not enforced)");
                        }

                        let valid = preview.is_ok() && name_ok;
                        if entered && valid {
                            commit = true;
                        }
                        ui.add_space(SPACE_1);
                        ui.horizontal(|ui| {
                            let ok = if edit.save_as.is_some() { "Save" } else { "OK" };
                            if ui
                                .add_enabled_ui(valid, |ui| ui_kit::widgets::primary_button(ui, ok))
                                .inner
                                .clicked()
                            {
                                commit = true;
                            }
                            if secondary_button(ui, "Cancel").clicked() {
                                cancel = true;
                            }
                        });
                        if edit.new {
                            ui.label(
                                RichText::new("Esc keeps the measured value")
                                    .font(sans(FONT_XS))
                                    .color(TEXT3),
                            );
                        }
                        if commit && !valid {
                            commit = false;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        }
                    });
            });
        if commit {
            self.commit_dim_edit(ctx);
        } else if cancel {
            self.dim_edit = None;
        }
    }
}

/// How wide the dimension editor's value field is.
const FIELD_WIDTH: f32 = 220.0;

/// How many variables the dimension editor offers as chips.
const MAX_CHIPS: usize = 12;

/// A value as the dimension editor writes it: up to four places, no
/// trailing zeros.
fn fmt_value(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// What a variable made from a dimension of kind `label` is first called:
/// `Distance X` gives `distance_x`.
fn variable_base_name(label: &str) -> String {
    let name: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let name = name.trim_matches('_').to_string();
    if name.is_empty() {
        "value".to_string()
    } else {
        name
    }
}

/// Select characters `from..to` of text edit `id`.
fn select_chars(ui: &egui::Ui, id: egui::Id, from: usize, to: usize) {
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(from),
            egui::text::CCursor::new(to),
        )));
    state.store(ui.ctx(), id);
}

/// What follows a dimension's value when it is written out.
fn unit_suffix(unit: sketch::DimensionUnit) -> &'static str {
    match unit {
        sketch::DimensionUnit::Length => " mm",
        sketch::DimensionUnit::Angle => "°",
        sketch::DimensionUnit::Ratio => "",
    }
}

/// One 24px list row with a fill behind it; the row itself is clickable
/// and the widgets inside it keep their own interactions.
fn list_row(
    ui: &mut egui::Ui,
    fill: egui::Color32,
    id_salt: impl egui::AsIdSalt,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let size = egui::Vec2::new(ui.available_width(), TREE_ROW);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, 3.0, fill);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::Vec2::new(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt(id_salt),
    );
    child.spacing_mut().item_spacing.x = SPACE_2;
    add(&mut child);
    response
}

/// What a dimension's value cell reads besides the constraint: the
/// document, for its formula and to read typed formulas against.
struct DimensionCell<'a> {
    document: &'a core_document::Document,
    sketch_id: Option<core_document::FeatureId>,
}

/// The value cell of a constraint row: an editable driving value, or the
/// measured reference value in parentheses.
fn dimension_value_cell(
    ui: &mut egui::Ui,
    sketch: &Sketch,
    constraint: &Constraint,
    cell: &DimensionCell,
    typed: &mut Option<(Uuid, String)>,
    focus: bool,
) {
    let Some(value) = sketch::dimension_value(&constraint.kind) else {
        return;
    };
    let unit = sketch::dimension_unit(&constraint.kind);
    let angular = unit == sketch::DimensionUnit::Angle;
    if constraint.driving && constraint.active {
        let key = constraint.id.to_string();
        let formula = cell
            .sketch_id
            .and_then(|id| cell.document.feature_formula(id, &key));
        let error = cell.sketch_id.and_then(|id| {
            cell.document
                .evaluated_slots(id)
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.result.as_ref().err())
                .map(String::as_str)
        });
        let host = core_document::DocumentFormulas {
            document: cell.document,
            dim: crate::params::formula_dim(&constraint.kind),
        };
        let field = ui_kit::widgets::FormulaField::new(
            egui::Id::new(("dimension", constraint.id)),
            f64::from(value),
            &host,
        )
        .formula(formula)
        .error(error)
        .unit(unit_suffix(unit).trim_start())
        .speed(match unit {
            sketch::DimensionUnit::Angle => 1.0,
            sketch::DimensionUnit::Length => 0.1,
            sketch::DimensionUnit::Ratio => 0.01,
        })
        .decimals(match unit {
            sketch::DimensionUnit::Angle => 1,
            sketch::DimensionUnit::Length => 2,
            sketch::DimensionUnit::Ratio => 3,
        })
        .width(96.0);
        if focus {
            ui.ctx()
                .memory_mut(|m| m.request_focus(egui::Id::new(("dimension", constraint.id))));
        }
        match field.show(ui) {
            Some(ui_kit::widgets::FormulaEdit::Value(v)) => {
                *typed = Some((constraint.id, format!("{v}")));
            }
            Some(ui_kit::widgets::FormulaEdit::Formula(text)) => {
                *typed = Some((constraint.id, text));
            }
            None => {}
        }
    } else {
        let measured = sketch::measured_value(sketch, &constraint.kind);
        let text = match measured {
            Some(m) if angular => format!("({m:.1}°)"),
            Some(m) => format!("({m:.2})"),
            None => "(-)".to_string(),
        };
        let color = if constraint.active { ACCENT } else { TEXT3 };
        mono_label(ui, text, FONT_SM, color).on_hover_text("Measured value (reference dimension)");
    }
}

/// The fillet and chamfer tools' switch that keeps the corner.
fn corner_row(ui: &mut egui::Ui, keep: &mut bool) {
    ui_kit::widgets::field_label(ui, "Corner");
    check_row(ui, keep, "Keep it").on_hover_text(
        "The corner stays as a construction point on both curves, with its constraints",
    );
    ui.end_row();
}

/// A text's string, font, size, spacing and turn as grid rows. Whether one
/// was changed: the string once its field is left, the rest at once.
fn text_fields(ui: &mut egui::Ui, salt: &str, spec: &mut crate::text::TextSpec) -> bool {
    let mut changed = false;
    ui_kit::widgets::field_label(ui, "Text");
    let mut draft = spec.text.clone();
    let edit = ui.add(
        egui::TextEdit::multiline(&mut draft)
            .id_salt((salt, "text"))
            .desired_rows(1)
            .desired_width(160.0),
    );
    if edit.changed() {
        spec.text = draft;
    }
    changed |= edit.lost_focus();
    ui.end_row();
    ui_kit::widgets::field_label(ui, "Font");
    let fonts = crate::text::FONTS;
    let mut which = fonts
        .iter()
        .position(|(name, _)| *name == spec.font)
        .unwrap_or(usize::MAX);
    let mut options: Vec<(usize, &str)> = fonts
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (i, *name))
        .collect();
    options.push((usize::MAX, "A font file"));
    if select_field(ui, (salt, "font"), &mut which, &options, 160.0) && which != usize::MAX {
        spec.font = fonts[which].0.to_string();
        changed = true;
    }
    ui.end_row();
    if which == usize::MAX {
        ui_kit::widgets::field_label(ui, "File");
        let mut path = if fonts.iter().any(|(n, _)| *n == spec.font) {
            String::new()
        } else {
            spec.font.clone()
        };
        let edit = ui.add(
            egui::TextEdit::singleline(&mut path)
                .id_salt((salt, "path"))
                .hint_text("/path/to/font.ttf")
                .desired_width(160.0),
        );
        if edit.changed() {
            spec.font = path;
        }
        changed |= edit.lost_focus();
        ui.end_row();
    }
    for (label, value, angle) in [
        ("Size", &mut spec.size, false),
        ("Spacing", &mut spec.spacing, false),
        ("Angle", &mut spec.angle, true),
    ] {
        ui_kit::widgets::field_label(ui, label);
        let field = if angle {
            QtyField::degrees(value)
        } else {
            QtyField::mm(value)
        };
        changed |= field.show(ui);
        ui.end_row();
    }
    spec.size = spec.size.max(0.1);
    changed
}
