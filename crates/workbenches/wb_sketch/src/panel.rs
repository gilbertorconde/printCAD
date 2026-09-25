//! The sketcher's task panel: attachment picker, tool settings, solver
//! messages, edit controls, and the constraint and element lists.

use egui::RichText;
use uuid::Uuid;

use core_document::{TaskOutcome, TaskRequest, WorkbenchRuntimeContext};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, QtyField, check_row, mono_label, note_card, planned, secondary_button, section_header,
    select_field,
};
use ui_kit::{mono, sans};

use crate::sketch::{self, Constraint, Sketch, SketchPlane};
use crate::solver;
use crate::style::{constraint_icon, element_icon, element_kind, element_name};
use crate::{ElementFilter, SketchWorkbench, overlay};

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
        if request.accept || request.cancel {
            ctx.request(core_document::HostRequest::FinishEditing);
            return TaskOutcome::Accepted {
                label: format!("Edit {}", feature.sketch.name),
            };
        }
        let plane = feature.plane;
        let sketch = feature.sketch;

        if self.sketch_picker.is_some() {
            self.sketch_picker_section(ui, ctx, &plane);
        }
        self.tool_section(ui);
        self.array_section(ui, ctx);
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

    /// Plane picker for a pending sketch creation.
    fn attachment_card(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        let Some(pending) = &self.pending_creation else {
            return;
        };
        let body = pending.body;
        let face_plane = pending.face_plane;
        let mut chosen: Option<(SketchPlane, Option<crate::feature::DatumSupport>)> = None;
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
            ui.label(
                RichText::new("Choose the plane to sketch on.")
                    .font(sans(FONT_SM))
                    .color(TEXT2),
            );
            ui.add_space(SPACE_1);
            if let Some(face) = face_plane
                && secondary_button(ui, "Selected face")
                    .on_hover_text("Sketch on the face you clicked on the solid")
                    .clicked()
            {
                chosen = Some((face, None));
            }
            ui.horizontal(|ui| {
                if secondary_button(ui, "Top (XY)").clicked() {
                    chosen = Some((SketchPlane::xy(), None));
                }
                if secondary_button(ui, "Front (XZ)").clicked() {
                    chosen = Some((SketchPlane::xz(), None));
                }
                if secondary_button(ui, "Side (YZ)").clicked() {
                    chosen = Some((SketchPlane::yz(), None));
                }
            });
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
                                    }),
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
                                        }),
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
        if let Some((plane, support)) = chosen {
            self.pending_creation = None;
            self.create_sketch_on_plane(ctx, body, plane, support);
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
                QtyField::mm(&mut params.array_dx).show(ui);
                ui.end_row();
                ui_kit::widgets::field_label(ui, "Row step");
                QtyField::mm(&mut params.array_dy).show(ui);
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
                    | "sketch.bspline"
                    | "sketch.rect_rounded"
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
                    Some("sketch.slot" | "sketch.arc_slot") => {
                        ui_kit::widgets::field_label(ui, "Width");
                        QtyField::mm(&mut params.slot_width).show(ui);
                        ui.end_row();
                    }
                    Some("sketch.fillet" | "sketch.rect_rounded") => {
                        ui_kit::widgets::field_label(ui, "Radius");
                        QtyField::mm(&mut params.fillet_radius).show(ui);
                        ui.end_row();
                    }
                    Some("sketch.chamfer") => {
                        ui_kit::widgets::field_label(ui, "Length");
                        QtyField::mm(&mut params.chamfer_length).show(ui);
                        ui.end_row();
                    }
                    Some("sketch.offset") => {
                        ui_kit::widgets::field_label(ui, "Distance");
                        QtyField::mm(&mut params.offset_distance).show(ui);
                        ui.end_row();
                    }
                    Some("sketch.translate" | "sketch.rotate") => {
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
                    }
                    Some("sketch.bspline") => {
                        ui_kit::widgets::field_label(ui, "Closed");
                        check_row(ui, &mut params.bspline_periodic, "Periodic");
                        ui.end_row();
                    }
                    _ => {}
                }
            });
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
                planned(ui, "draws construction or normal geometry on top", |ui| {
                    let mut order = 0u8;
                    select_field(
                        ui,
                        "sketch_render_order",
                        &mut order,
                        &[(0, "Normal")],
                        110.0,
                    );
                });
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
                    .desired_width(120.0)
                    .hint_text(RichText::new("Filter").color(TEXT3))
                    .font(sans(FONT_XS)),
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
        let filter = self.constraint_filter.trim().to_lowercase();
        let mut delete: Option<usize> = None;
        let mut edited: Option<(usize, Constraint)> = None;
        let mut typed: Option<(Uuid, String)> = None;
        let cell = DimensionCell {
            document: ctx.document,
            sketch_id: self.active_sketch_id,
        };
        let mut clicked: Option<(Uuid, bool)> = None;
        for (idx, constraint) in sketch.constraints.iter().enumerate() {
            let label = constraint.name.clone().unwrap_or_else(|| {
                format!(
                    "Constraint{} · {}",
                    idx + 1,
                    sketch::constraint_label(&constraint.kind)
                )
            });
            if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                continue;
            }
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

    /// In-viewport dimension editor (opened by double-clicking a
    /// dimensional glyph), drawn as a floating card near the label.
    pub(crate) fn dim_edit_window(&mut self, ui: &egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        let self_angular = self
            .dim_edit
            .as_ref()
            .and_then(|edit| {
                let sketch = self.get_active_sketch(ctx)?;
                let c = sketch
                    .sketch
                    .constraints
                    .iter()
                    .find(|c| c.id == edit.constraint)?;
                Some(sketch::is_angular(&c.kind))
            })
            .unwrap_or(false);
        let Some(edit) = self.dim_edit.as_mut() else {
            return;
        };
        let ppp = ui.ctx().pixels_per_point().max(0.1);
        let (vx, vy, ..) = ctx.viewport;
        let pos = egui::pos2(
            (vx as f32 + edit.screen_pos[0]) / ppp + 12.0,
            (vy as f32 + edit.screen_pos[1]) / ppp + 12.0,
        );
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("sketch_dim_edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                ui_kit::widgets::Card::floating()
                    .border(BORDER_STRONG)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = SPACE_1;
                        ui.label(
                            RichText::new("Dimension")
                                .font(ui_kit::sans_medium(FONT_SM))
                                .color(TEXT1),
                        );
                        // Names complete as they are typed.
                        let document: &core_document::Document = ctx.document;
                        let completed = ui_kit::completion::completing_text_edit(
                            ui,
                            egui::Id::new("sketch_dim_edit_text"),
                            &mut edit.text,
                            &|| core_document::formula_candidates(document),
                            |edit| edit.desired_width(180.0).font(mono(FONT_SM)),
                        );
                        let response = completed.response;
                        // Focus once, as the editor opens: asking again
                        // every frame would interrupt the input method's
                        // composition every frame, and keys would arrive
                        // late, several at once.
                        if !response.has_focus() && !response.lost_focus() {
                            response.request_focus();
                        }
                        // What it comes to, as typed: a value, or a formula.
                        let angular = self_angular;
                        let preview = ctx.document.evaluate_formula(
                            &edit.text,
                            Some(if angular {
                                core_document::expr::Dim::ANGLE
                            } else {
                                core_document::expr::Dim::LENGTH
                            }),
                        );
                        let (line, color) = match preview {
                            Ok(q) if core_document::expr::is_constant(&edit.text) => (
                                format!("{:.3}{}", q.value, if angular { "°" } else { " mm" }),
                                TEXT3,
                            ),
                            Ok(q) => (
                                format!("ƒ = {:.3}{}", q.value, if angular { "°" } else { " mm" }),
                                SKETCH_FORMULA,
                            ),
                            Err(why) => (why, DANGER),
                        };
                        ui.label(RichText::new(line).font(sans(FONT_XS)).color(color));
                        ui.label(
                            RichText::new("A value (1 in) or a formula (Sizes.width / 2)")
                                .font(sans(FONT_XS))
                                .color(TEXT3),
                        );
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            commit = true;
                        }
                        check_row(ui, &mut edit.driving, "Driving")
                            .on_hover_text("Off = reference dimension (measured, not enforced)");
                        ui.horizontal(|ui| {
                            if ui_kit::widgets::primary_button(ui, "OK").clicked() {
                                commit = true;
                            }
                            if secondary_button(ui, "Cancel").clicked() {
                                cancel = true;
                            }
                        });
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

/// The value cell of a constraint row: an editable driving value, or the
/// measured reference value in parentheses.
/// What a dimension's value cell reads besides the constraint: the
/// document, for its formula and to read typed formulas against.
struct DimensionCell<'a> {
    document: &'a core_document::Document,
    sketch_id: Option<core_document::FeatureId>,
}

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
    let angular = sketch::is_angular(&constraint.kind);
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
            dim: if angular {
                core_document::expr::Dim::ANGLE
            } else {
                core_document::expr::Dim::LENGTH
            },
        };
        let field = ui_kit::widgets::FormulaField::new(
            egui::Id::new(("dimension", constraint.id)),
            f64::from(value),
            &host,
        )
        .formula(formula)
        .error(error)
        .unit(if angular { "°" } else { "mm" })
        .speed(if angular { 1.0 } else { 0.1 })
        .decimals(if angular { 1 } else { 2 })
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
