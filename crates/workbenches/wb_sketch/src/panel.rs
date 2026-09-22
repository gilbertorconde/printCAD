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
        let sketch = feature.sketch;

        self.tool_section(ui);
        self.solver_section(ui, &sketch);
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

    /// Plane picker for a pending sketch creation.
    fn attachment_card(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        let Some(pending) = &self.pending_creation else {
            return;
        };
        let body = pending.body;
        let face_plane = pending.face_plane;
        let mut chosen: Option<SketchPlane> = None;
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
                chosen = Some(face);
            }
            ui.horizontal(|ui| {
                if secondary_button(ui, "Top (XY)").clicked() {
                    chosen = Some(SketchPlane::xy());
                }
                if secondary_button(ui, "Front (XZ)").clicked() {
                    chosen = Some(SketchPlane::xz());
                }
                if secondary_button(ui, "Side (YZ)").clicked() {
                    chosen = Some(SketchPlane::yz());
                }
            });
            // Datum planes of the target body attach the sketch to their
            // resolved frame (toponaming-safe anchor).
            if let Some(body) = body {
                for (_, name, datum) in core_document::datums_of_body(ctx.document, body) {
                    if !matches!(datum.shape, core_document::DatumShape::Plane { .. }) {
                        continue;
                    }
                    if secondary_button(ui, &name)
                        .on_hover_text("Sketch on this datum plane")
                        .clicked()
                    {
                        let frame = datum.frame();
                        chosen = Some(SketchPlane::from_frame(
                            frame.origin,
                            frame.normal,
                            frame.x_axis,
                        ));
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
        if let Some(plane) = chosen {
            self.pending_creation = None;
            self.create_sketch_on_plane(ctx, body, plane);
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
                    Some("sketch.fillet") => {
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

    fn solver_section(&mut self, ui: &mut egui::Ui, sketch: &Sketch) {
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
            // PLANNED: an automatic re-solve toggle and automatic removal of
            // redundant constraints; the solver runs after every edit today.
            planned(ui, "re-solves after every edit today", |ui| {
                let mut on = true;
                check_row(ui, &mut on, "Auto update");
            });
            planned(ui, "drops redundant constraints on detection", |ui| {
                let mut on = false;
                check_row(ui, &mut on, "Auto remove redundants");
            });
        });
    }

    fn edit_controls_section(&mut self, ui: &mut egui::Ui) {
        if !section_header(ui, "sketch_edit_controls", "Edit controls", None, false) {
            return;
        }
        // PLANNED: a world-unit grid on the sketch plane with snapping.
        planned(ui, "draws a grid on the sketch plane", |ui| {
            let mut on = false;
            check_row(ui, &mut on, "Show grid");
        });
        planned(ui, "picks the grid step from the zoom level", |ui| {
            let mut on = false;
            check_row(ui, &mut on, "Grid auto spacing");
        });
        planned(ui, "snaps points to the grid", |ui| {
            let mut on = false;
            check_row(ui, &mut on, "Snap to grid");
        });
        let mut snap = !self.snap_off;
        if check_row(ui, &mut snap, "Snap to objects").changed() {
            self.snap_off = !snap;
        }
        // PLANNED: auto-constraint preferences; endpoints, horizontals and
        // verticals snap into constraints today.
        planned(
            ui,
            "skips auto constraints the solver would call redundant",
            |ui| {
                let mut on = true;
                check_row(ui, &mut on, "Avoid redundant auto constraints");
            },
        );
        planned(
            ui,
            "adds coincident, horizontal and vertical while drawing",
            |ui| {
                let mut on = true;
                check_row(ui, &mut on, "Auto constraints");
            },
        );
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
                planned(ui, "sets the grid step", |ui| {
                    let mut size = 10.0f32;
                    QtyField::mm(&mut size).width(90.0).show(ui);
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
                    resp.request_focus();
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
                        idx,
                        &mut edited,
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
        if let Some((id, additive)) = clicked {
            if additive {
                if !self.selected_constraints.remove(&id) {
                    self.selected_constraints.insert(id);
                }
            } else {
                self.selected.clear();
                self.selected_constraints.clear();
                self.selected_constraints.insert(id);
            }
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
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut edit.text)
                                .desired_width(96.0)
                                .font(mono(FONT_SM)),
                        );
                        response.request_focus();
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
fn dimension_value_cell(
    ui: &mut egui::Ui,
    sketch: &Sketch,
    constraint: &Constraint,
    idx: usize,
    edited: &mut Option<(usize, Constraint)>,
    focus: bool,
) {
    let Some(value) = sketch::dimension_value(&constraint.kind) else {
        return;
    };
    let angular = sketch::is_angular(&constraint.kind);
    if constraint.driving && constraint.active {
        let mut v = value;
        let unit = if angular { "°" } else { "mm" };
        let mut field = QtyField::new(&mut v).unit(unit).width(96.0);
        field = if angular {
            field.speed(1.0).decimals(1)
        } else {
            field.speed(0.1).range(0.001..=1.0e6)
        };
        let changed = field.show(ui);
        if focus {
            ui.ctx().memory_mut(|m| m.request_focus(ui.id()));
        }
        if changed {
            let mut c = constraint.clone();
            c.kind = sketch::with_dimension_value(&c.kind, v);
            *edited = Some((idx, c));
        }
    } else {
        let measured = sketch::measured_value(sketch, &constraint.kind);
        let text = match measured {
            Some(m) if angular => format!("({m:.1}°)"),
            Some(m) => format!("({m:.2})"),
            None => "(—)".to_string(),
        };
        let color = if constraint.active { ACCENT } else { TEXT3 };
        mono_label(ui, text, FONT_SM, color).on_hover_text("Measured value (reference dimension)");
    }
}
