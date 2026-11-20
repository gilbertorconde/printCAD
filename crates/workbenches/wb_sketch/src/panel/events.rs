//! The panel's events read back: buttons, numbers, toggles, choices and
//! text, each handled through the sketcher's own commands and code paths.

use super::*;

impl SketchWorkbench {
    /// A change to the panel.
    pub(crate) fn task_event(&mut self, ctx: &mut WorkbenchRuntimeContext, event: &PanelEvent) {
        let id = event.id();
        if let PanelEvent::Button { id } = event
            && self.pending_button(ctx, id)
        {
            return;
        }
        // A tool setting changed while a click's result waits works it
        // out again.
        let retool = self.tab.corner_task.is_some()
            && (id.starts_with("tool.") || id.starts_with("text_draft."));
        self.task_event_inner(ctx, event);
        if retool {
            self.recalculate_corner(ctx);
        }
    }

    fn task_event_inner(&mut self, ctx: &mut WorkbenchRuntimeContext, event: &PanelEvent) {
        let id = event.id();
        if let Some(rest) = id.strip_prefix("attach.") {
            self.attachment_event(ctx, rest);
            return;
        }
        match event {
            PanelEvent::Button { id } => self.button_event(ctx, id),
            PanelEvent::Number { id, value } => self.number_event(ctx, id, *value),
            PanelEvent::Toggle { id, on } => self.toggle_event(ctx, id, *on),
            PanelEvent::Choice { id, index } => self.choice_event(ctx, id, *index),
            PanelEvent::Text { id, value } => self.text_event(ctx, id, value),
            PanelEvent::Filter { text, kind, .. } => {
                self.tab.constraint_filter.clone_from(text);
                match ListFilter::ALL.get(*kind).map(|(f, _)| *f) {
                    Some(ListFilter::Elements(e)) => {
                        self.tab.element_filter = e;
                        self.tab.constraint_kind_filter = ConstraintFilter::All;
                    }
                    Some(ListFilter::Constraints(c)) => {
                        self.tab.constraint_kind_filter = c;
                        self.tab.element_filter = ElementFilter::All;
                    }
                    None => {}
                }
            }
            PanelEvent::HoverPart {
                id,
                index,
                tag,
                badge,
            } => {
                let list = usize::from(id == "constraints");
                self.tab.panel.pointed[list] = (*index, *tag, *badge);
            }
            // As the viewport does: a click adds to the selection, a
            // second click takes it out.
            PanelEvent::Select { id, index } => match id.as_str() {
                "elements" => {
                    let tab = &mut *self.tab;
                    if let Some(row) = tab.panel.elements.get(*index)
                        && !tab.selected.remove(&row.id)
                    {
                        tab.selected.insert(row.id);
                    }
                }
                "constraints" => {
                    let tab = &mut *self.tab;
                    if let Some(row) = tab.panel.constraints.get(*index)
                        && !tab.selected_constraints.remove(&row.id)
                    {
                        tab.selected_constraints.insert(row.id);
                    }
                }
                _ => {}
            },
            // A chip picks the element it names as its row does, and
            // brings that row into view.
            PanelEvent::Tag { index, tag, .. } => {
                if let Some(element) = self
                    .tab
                    .panel
                    .constraints
                    .get(*index)
                    .and_then(|r| r.tags.get(*tag).copied())
                {
                    if !self.tab.selected.remove(&element) {
                        self.tab.selected.insert(element);
                    }
                    self.tab.scroll_to_row = Some(element);
                }
            }
            // A badge picks its constraint as the constraint's row does.
            PanelEvent::Badge { index, badge, .. } => {
                if let Some(constraint) = self
                    .tab
                    .panel
                    .elements
                    .get(*index)
                    .and_then(|r| r.badges.get(*badge).copied())
                {
                    if !self.tab.selected_constraints.remove(&constraint) {
                        self.tab.selected_constraints.insert(constraint);
                    }
                    self.tab.scroll_to_row = Some(constraint);
                }
            }
            PanelEvent::ContextMenu { id, index, action } => match id.as_str() {
                "elements" => {
                    if let Some(row) = self.tab.panel.elements.get(*index) {
                        let element = row.id;
                        self.element_menu(ctx, element, action);
                    }
                }
                "constraints" => {
                    if let Some(row) = self.tab.panel.constraints.get(*index) {
                        let constraint = row.id;
                        self.constraint_menu(ctx, constraint, action);
                    }
                }
                _ => {}
            },
            PanelEvent::ItemText {
                index, value, done, ..
            } => {
                let Some(id) = self.tab.panel.constraints.get(*index).map(|r| r.id) else {
                    return;
                };
                if let Some((idx, mut c)) = self.constraint_at(ctx, id)
                    && c.name.as_deref().unwrap_or_default() != value
                {
                    c.name = (!value.is_empty()).then(|| value.clone());
                    self.update_constraint(ctx, idx, c);
                }
                if *done {
                    self.tab.renaming_constraint = None;
                }
            }
            _ => {}
        }
    }

    /// Constraint `id` of the open sketch, and its place.
    fn constraint_at(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: Uuid,
    ) -> Option<(usize, Constraint)> {
        let feature = self.get_active_sketch(ctx)?;
        feature
            .sketch
            .constraints
            .iter()
            .position(|c| c.id == id)
            .map(|i| (i, feature.sketch.constraints[i].clone()))
    }

    fn element_menu(&mut self, ctx: &mut WorkbenchRuntimeContext, id: Uuid, action: &str) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        match action {
            "construction" => {
                let flag = !feature.sketch.is_construction(id);
                feature.sketch.set_construction(id, flag);
                self.store_sketch(ctx, feature);
            }
            "delete" => {
                let removed = feature.sketch.remove_geometry_cascade(&[id]);
                if !removed.is_empty() {
                    for rid in &removed {
                        self.tab.selected.remove(rid);
                    }
                    self.tab.hovered = None;
                    self.solve(ctx, &mut feature);
                    ctx.log_info(format!("Deleted {} sketch element(s)", removed.len()));
                    self.store_sketch(ctx, feature);
                }
            }
            _ => {}
        }
    }

    fn constraint_menu(&mut self, ctx: &mut WorkbenchRuntimeContext, id: Uuid, action: &str) {
        let Some((idx, mut c)) = self.constraint_at(ctx, id) else {
            return;
        };
        match action {
            "active" => c.active = !c.active,
            "driving" => c.driving = !c.driving,
            "park" => c.parked = !c.parked,
            "rename" => {
                self.tab.renaming_constraint = Some(id);
                return;
            }
            "delete" => {
                if let Some(mut feature) = self.get_active_sketch(ctx) {
                    feature.sketch.constraints.remove(idx);
                    self.solve_and_store(ctx, feature);
                }
                return;
            }
            _ => return,
        }
        self.update_constraint(ctx, idx, c);
    }

    fn button_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str) {
        match id {
            "problems" => {
                let Some(feature) = self.frame_sketch(ctx) else {
                    return;
                };
                let state = self.diagnosed_verdict(&feature);
                self.tab.selected.clear();
                for constraint in &feature.sketch.constraints {
                    if state.offenders.contains(&constraint.id) {
                        self.tab
                            .selected
                            .extend(sketch::constraint_refs(&constraint.kind));
                    }
                }
            }
            "solve_now" => self.solve_from_panel(ctx),
            "picker.close" => self.tab.sketch_picker = None,
            "picker.merge" => self.merge_sketches(ctx),
            "array.make" => {
                self.array_selection(ctx);
            }
            "text_block.explode" => self.explode_selected_text(ctx),
            "delete_shown" => {
                if let Some(feature) = self.get_active_sketch(ctx) {
                    let shown = self.listed_constraints(&feature.sketch);
                    self.delete_items(ctx, shown);
                }
            }
            "calib.apply" => {
                let placed = self.calibration_result(ctx);
                self.tab.calibrating = None;
                if let Some((image, placed)) = placed {
                    self.set_image(
                        ctx,
                        serde_json::json!({
                            "image": image.to_string(), "x": placed.center.x, "y": placed.center.y,
                            "width": placed.width, "angle": placed.angle_deg,
                        }),
                    );
                }
            }
            "calib.cancel" => self.tab.calibrating = None,
            _ => {
                if let Some(button) = id.strip_prefix("text_draft.") {
                    text_button(&mut self.tab.text_draft, button);
                } else if let Some(button) = id.strip_prefix("text_block.") {
                    self.text_block_edit(ctx, |spec| text_button(spec, button));
                } else if let Some(i) = panel::indexed(id, "picker.copy:") {
                    self.picker_choice(ctx, i);
                } else if let Some((n, key)) = image_field(id) {
                    self.image_button(ctx, n, key);
                }
            }
        }
    }

    /// The picker's `i`th sketch or datum clicked: copied in, or brought
    /// in as external geometry.
    fn picker_choice(&mut self, ctx: &mut WorkbenchRuntimeContext, i: usize) {
        let Some(from) = self.tab.panel.picked_from.get(i).copied() else {
            return;
        };
        let external = self
            .tab
            .sketch_picker
            .as_ref()
            .is_some_and(|p| p.mode == crate::SketchPickerMode::ExternalFrom);
        if !external {
            self.carbon_copy(ctx, from);
            return;
        }
        let Some(sketch) = self.tab.active_sketch_id else {
            return;
        };
        let args = crate::commands::object(serde_json::json!({
            "sketch": sketch.0.to_string(),
            "from": from.0.to_string(),
            "counts": !self.construction_mode(),
        }));
        match crate::commands::run("sketch.external_from", &args, ctx) {
            Ok(made) => {
                ctx.record("sketch.external_from", args, made);
                self.tab.sketch_picker = None;
            }
            Err(err) => ctx.log_warn(err.to_string()),
        }
    }

    fn image_button(&mut self, ctx: &mut WorkbenchRuntimeContext, n: usize, key: &str) {
        let Some(image) = self.image_at(ctx, n).map(|i| i.id) else {
            return;
        };
        match key {
            "calibrate" => {
                self.tab.calibrating = Some(crate::Calibrating {
                    image,
                    from: None,
                    to: None,
                    length: String::new(),
                    straighten: false,
                    centre: false,
                });
            }
            "remove" => self.set_image(
                ctx,
                serde_json::json!({"image": image.to_string(), "remove": true}),
            ),
            _ => {}
        }
    }

    /// The open sketch's `n`th picture.
    fn image_at(&self, ctx: &WorkbenchRuntimeContext, n: usize) -> Option<sketch::ReferenceImage> {
        self.get_active_sketch(ctx)?.sketch.images.get(n).cloned()
    }

    /// `sketch.set_image` with `args`, run and recorded.
    fn set_image(&mut self, ctx: &mut WorkbenchRuntimeContext, mut args: serde_json::Value) {
        let Some(sketch_id) = self.tab.active_sketch_id else {
            return;
        };
        args["sketch"] = serde_json::json!(sketch_id.0.to_string());
        let args = crate::commands::object(args);
        if crate::commands::run("sketch.set_image", &args, ctx).is_ok() {
            ctx.record("sketch.set_image", args, serde_json::Value::Null);
        }
    }

    /// The picture under calibration as it would lie once applied, when
    /// both points are picked and the length reads.
    fn calibration_result(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(Uuid, sketch::ReferenceImage)> {
        let calibrating = self.tab.calibrating.as_ref()?;
        let feature = self.get_active_sketch(ctx)?;
        let image = feature
            .sketch
            .images
            .iter()
            .find(|i| i.id == calibrating.image)?;
        calibrated(ctx, image, calibrating)
            .ok()
            .flatten()
            .map(|placed| (image.id, placed))
    }

    fn number_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, value: f64) {
        let whole = || value.round().max(0.0) as u32;
        let p = &mut self.tab.tool_params;
        match id {
            "tool.polygon_sides" => p.polygon_sides = whole(),
            "tool.slot_width" => p.slot_width = value,
            "tool.fillet_radius" => p.fillet_radius = value,
            "tool.chamfer_length" => p.chamfer_length = value,
            "tool.offset_distance" => p.offset_distance = value,
            "tool.copies" => p.copies = whole(),
            "tool.bspline_degree" => p.bspline_degree = whole(),
            "array.rows" => p.array_rows = whole(),
            "array.cols" => p.array_cols = whole(),
            "array.dx" => p.array_dx = value,
            "array.dy" => p.array_dy = value,
            "display.grid_size" => self.options.grid_size = value,
            "solver.iterations" => {
                self.solver_settings(ctx, serde_json::json!({"iterations": value.round()}));
            }
            "spline.degree" => self.spline_degree(ctx, value),
            "spline.weight" => self.spline_weight(ctx, value),
            _ => {
                if let Some(constraint) = id
                    .strip_prefix("dim:")
                    .and_then(|u| Uuid::parse_str(u).ok())
                {
                    // A value typed into a dimension's row opens its task.
                    self.open_dimension_task(ctx, constraint, &format!("{value}"));
                } else if let Some(i) = panel::indexed(id, "spline.knot:") {
                    self.knot_multiplicity(ctx, i, whole() as usize);
                } else if let Some(key) = id.strip_prefix("placement.") {
                    self.placement_event(ctx, key, value);
                } else if let Some((n, key)) = image_field(id) {
                    self.image_number(ctx, n, key, value);
                } else if let Some(field) = id.strip_prefix("text_draft.") {
                    text_number(&mut self.tab.text_draft, field, value);
                } else if let Some(field) = id.strip_prefix("text_block.") {
                    self.text_block_edit(ctx, |spec| text_number(spec, field, value));
                }
            }
        }
    }

    pub(super) fn spline_degree(&mut self, ctx: &mut WorkbenchRuntimeContext, value: f64) {
        let Some(sketch) = self.get_active_sketch(ctx).map(|f| f.sketch) else {
            return;
        };
        let Some((_, degree)) = self.selected_spline(&sketch).0 else {
            return;
        };
        let by = value.round() as i32 - degree as i32;
        if by != 0 {
            self.selection_edit(
                ctx,
                "sketch.spline_degree",
                serde_json::json!({ "by": by }),
                |s, sel| crate::spline_edit::change_degree(s, sel, by),
            );
        }
    }

    fn knot_multiplicity(&mut self, ctx: &mut WorkbenchRuntimeContext, i: usize, times: usize) {
        let Some(sketch) = self.get_active_sketch(ctx).map(|f| f.sketch) else {
            return;
        };
        let (Some((id, _)), Some(&(knot, _))) =
            (self.selected_spline(&sketch).0, self.tab.panel.knots.get(i))
        else {
            return;
        };
        self.sketch_edit(
            ctx,
            "sketch.knot_multiplicity",
            serde_json::json!({ "spline": id.to_string(), "knot": knot, "multiplicity": times }),
            |s| {
                if times == 0 {
                    crate::spline_edit::remove_knot(s, id, knot)
                } else {
                    crate::spline_edit::set_multiplicity(s, id, knot, times)
                }
            },
        );
    }

    fn spline_weight(&mut self, ctx: &mut WorkbenchRuntimeContext, w: f64) {
        let Some(sketch) = self.get_active_sketch(ctx).map(|f| f.sketch) else {
            return;
        };
        let Some((owner, point)) = self.selected_spline(&sketch).1 else {
            return;
        };
        if crate::spline_edit::weight_of(&sketch, owner, point) == Some(w) {
            return;
        }
        self.sketch_edit(
            ctx,
            "sketch.spline_weight",
            serde_json::json!({ "spline": owner.to_string(), "point": point.to_string(), "weight": w }),
            |s| crate::spline_edit::set_weight(s, owner, point, w),
        );
    }

    fn placement_event(&mut self, ctx: &mut WorkbenchRuntimeContext, key: &str, value: f64) {
        let (Some(id), Some(feature)) = (self.tab.active_sketch_id, self.get_active_sketch(ctx))
        else {
            return;
        };
        let placed =
            feature
                .support
                .as_ref()
                .map_or(feature.offset, |s| crate::feature::PlaneShift {
                    offset: s.offset,
                    shift: s.shift,
                    turn: s.turn,
                });
        let (mut offset, mut shift, mut turn) = (placed.offset, placed.shift, placed.turn);
        match key {
            "offset" => offset = value,
            "x" => shift[0] = value,
            "y" => shift[1] = value,
            "turn" => turn = value,
            _ => return,
        }
        let args = crate::commands::object(serde_json::json!({
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

    fn image_number(&mut self, ctx: &mut WorkbenchRuntimeContext, n: usize, key: &str, value: f64) {
        let Some(image) = self.image_at(ctx, n) else {
            return;
        };
        let (mut x, mut y, mut width, mut angle, mut opacity) = (
            image.center.x,
            image.center.y,
            image.width,
            image.angle_deg,
            image.opacity,
        );
        match key {
            "x" => x = value,
            "y" => y = value,
            "width" => width = value,
            "angle" => angle = value,
            "opacity" => opacity = value,
            _ => return,
        }
        self.set_image(
            ctx,
            serde_json::json!({
                "image": image.id.to_string(), "x": x, "y": y, "width": width,
                "angle": angle, "opacity": opacity,
            }),
        );
    }

    /// `sketch.solver_settings` with `args`, run and recorded.
    fn solver_settings(&mut self, ctx: &mut WorkbenchRuntimeContext, mut args: serde_json::Value) {
        let Some(id) = self.tab.active_sketch_id else {
            return;
        };
        args["sketch"] = serde_json::json!(id.0.to_string());
        let args = crate::commands::object(args);
        if crate::commands::run("sketch.solver_settings", &args, ctx).is_ok() {
            ctx.record("sketch.solver_settings", args, serde_json::Value::Null);
            self.tab.last_diagnosis = None;
        }
    }

    fn toggle_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, on: bool) {
        let p = &mut self.tab.tool_params;
        let o = &mut self.options;
        let flag: Option<&mut bool> = match id {
            "tool.corner_keep" => Some(&mut p.corner_keep),
            "tool.polygon_flats" => Some(&mut p.polygon_flats),
            "tool.offset_round" => Some(&mut p.offset_round),
            "tool.offset_both" => Some(&mut p.offset_both),
            "tool.offset_delete" => Some(&mut p.offset_delete),
            "tool.offset_linked" => Some(&mut p.offset_linked),
            "tool.mirror_keep" => Some(&mut p.mirror_keep),
            "tool.mirror_linked" => Some(&mut p.mirror_linked),
            "tool.mirror_center" => Some(&mut p.mirror_center),
            "tool.copies_total" => Some(&mut p.copies_total),
            "tool.copies_symmetric" => Some(&mut p.copies_symmetric),
            "tool.rotate_full_turn" => Some(&mut p.rotate_full_turn),
            "tool.copies_linked" | "array.linked" => Some(&mut p.copies_linked),
            "tool.bspline_periodic" => Some(&mut p.bspline_periodic),
            "tool.bspline_interpolate" => Some(&mut p.bspline_interpolate),
            "solver.auto_update" => Some(&mut o.auto_update),
            "solver.auto_remove" => Some(&mut o.auto_remove_redundant),
            "display.grid_on" => Some(&mut o.grid_on),
            "display.grid_auto" => Some(&mut o.grid_auto),
            "display.grid_snap" => Some(&mut o.grid_snap),
            "display.avoid_redundant" => Some(&mut o.avoid_redundant_auto),
            "display.reuse_lines" => Some(&mut o.reuse_lines),
            "display.auto_constraints" => Some(&mut o.auto_constraints),
            "display.continue_drawing" => Some(&mut o.continue_drawing),
            "display.continue_constraining" => Some(&mut o.continue_constraining),
            "display.arc_helpers" => Some(&mut o.arc_helpers),
            "display.spline_polygon" => Some(&mut o.spline_polygon),
            "display.spline_degree" => Some(&mut o.spline_degree),
            "display.spline_multiplicity" => Some(&mut o.spline_multiplicity),
            "display.spline_weights" => Some(&mut o.spline_weights),
            _ => None,
        };
        if let Some(flag) = flag {
            *flag = on;
            return;
        }
        if let Some(field) = id.strip_prefix("text_draft.") {
            text_toggle(&mut self.tab.text_draft, field, on);
            return;
        }
        if let Some(field) = id.strip_prefix("text_block.") {
            self.text_block_edit(ctx, |spec| text_toggle(spec, field, on));
            return;
        }
        match id {
            "display.snap" => self.tab.snap_off = !on,
            "calib.straighten" | "calib.centre" => {
                if let Some(c) = self.tab.calibrating.as_mut() {
                    if id == "calib.straighten" {
                        c.straighten = on;
                    } else {
                        c.centre = on;
                    }
                }
            }
            _ => {
                let Some(i) = panel::indexed(id, "picker.tick:") else {
                    return;
                };
                let (Some(sketch), Some(picker)) = (
                    self.tab.panel.picked_from.get(i).copied(),
                    self.tab.sketch_picker.as_mut(),
                ) else {
                    return;
                };
                if on {
                    picker.checked.insert(sketch);
                } else {
                    picker.checked.remove(&sketch);
                }
                let _ = ctx;
            }
        }
    }

    fn choice_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, index: usize) {
        match id {
            "display.labels" => {
                if let Some((v, _)) = crate::glyphs::DimensionLabels::ALL.get(index) {
                    self.options.dimension_labels = *v;
                }
            }
            "display.angle_step" => {
                if let Some((v, _)) = crate::ANGLE_STEPS.get(index) {
                    self.options.angle_step_deg = *v;
                }
            }
            "display.render_order" => self.options.construction_on_top = index == 1,
            "solver.tolerance" => {
                if let Some(e) = TOLERANCES.get(index) {
                    self.solver_settings(ctx, serde_json::json!({"tolerance": 10f64.powi(-e)}));
                }
            }
            _ => {
                if let Some(field) = id.strip_prefix("text_draft.") {
                    let tab = &mut *self.tab;
                    text_choice(
                        &mut tab.text_draft,
                        &mut tab.panel.font_modes[0],
                        field,
                        index,
                    );
                } else if let Some(field) = id.strip_prefix("text_block.") {
                    let mut mode = self.tab.panel.font_modes[1];
                    self.text_block_edit(ctx, |spec| text_choice(spec, &mut mode, field, index));
                    self.tab.panel.font_modes[1] = mode;
                }
            }
        }
    }

    fn text_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, value: &str) {
        if id == "calib.length" {
            if let Some(c) = self.tab.calibrating.as_mut() {
                c.length = value.to_string();
            }
        } else if let Some(field) = id.strip_prefix("text_draft.") {
            text_string(&mut self.tab.text_draft, field, value);
        } else if let Some(field) = id.strip_prefix("text_block.") {
            self.text_block_edit(ctx, |spec| text_string(spec, field, value));
        }
    }

    /// The selected text block turned into plain curves, which stay
    /// selected.
    fn explode_selected_text(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(id) = self
            .tab
            .selected
            .iter()
            .find_map(|id| crate::text::block_of(&feature.sketch, *id))
            .map(|b| b.id)
        else {
            return;
        };
        let mut left = Vec::new();
        self.sketch_edit(
            ctx,
            "sketch.explode_text",
            serde_json::json!({"block": id.to_string()}),
            |s| match crate::text::explode(s, id) {
                Ok(curves) => {
                    left = curves;
                    crate::tools::ToolEffect::changed("Text exploded to curves")
                }
                Err(why) => crate::tools::ToolEffect::log(why),
            },
        );
        crate::keep_text_formulas(ctx, self.tab.active_sketch_id, Some(id));
        if let Some(feature) = self.get_active_sketch(ctx) {
            self.tab.selected = left
                .into_iter()
                .filter(|e| {
                    !matches!(
                        feature.sketch.get_geometry(*e),
                        Some(sketch::GeometryElement::Point(_)) | None
                    )
                })
                .collect();
        }
    }

    /// The selected text block made again with `change` applied to what
    /// it says and how, when `change` says it changed.
    fn text_block_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        change: impl FnOnce(&mut crate::text::TextSpec) -> bool,
    ) {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(block) = self
            .tab
            .selected
            .iter()
            .find_map(|id| crate::text::block_of(&feature.sketch, *id))
        else {
            return;
        };
        let id = block.id;
        let mut spec = crate::text::TextSpec::of(block);
        if !change(&mut spec) {
            return;
        }
        if let Err(why) = crate::text::load_font(ctx.document, &mut spec) {
            ctx.log_warn(why);
            return;
        }
        match crate::text::values_of(ctx.document, &spec.text) {
            Ok(values) => spec.values = values,
            Err(why) => {
                ctx.log_warn(why);
                return;
            }
        }
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
        crate::keep_text_formulas(ctx, self.tab.active_sketch_id, None);
    }

    /// A button of the plane picker.
    fn attachment_event(&mut self, ctx: &mut WorkbenchRuntimeContext, what: &str) {
        let Some(pending) = &self.tab.pending_creation else {
            return;
        };
        let body = pending.body;
        let made_by = pending.generator.clone();
        let face = pending.face;
        let face_origin = pending.face_origin;
        let mut chosen: Option<Choice> = None;
        match what {
            "cancel" => self.tab.pending_creation = None,
            "face" => {
                let Some(face) = face else {
                    return;
                };
                // On the face itself, not the drawn mesh the click met.
                let face = face.on_its_plane();
                let mut plane = SketchPlane::from_face(widen3(face.point), widen3(face.normal));
                // A generator stands where the face was clicked, a keyway
                // running along the shaft it is clicked on.
                if let Some(made_by) = &made_by {
                    let x_axis = made_by.runs_along(&face).unwrap_or(plane.x_axis);
                    plane = SketchPlane::from_frame(widen3(face.point), plane.normal, x_axis);
                }
                // A face of the sketch's own body, or one it borrows, is
                // followed.
                let follows = body.and_then(|_| {
                    crate::feature::FaceSupport::from_origin(&face, face_origin, plane)
                });
                chosen = Some((plane, None, follows));
            }
            "base:xy" => self.create_on_base_plane(ctx, core_document::BasePlane::XY),
            "base:xz" => self.create_on_base_plane(ctx, core_document::BasePlane::XZ),
            "base:yz" => self.create_on_base_plane(ctx, core_document::BasePlane::YZ),
            _ => {
                if let Some(i) = panel::indexed(what, "datum:") {
                    chosen = self.tab.panel.datum_planes.get(i).cloned();
                } else if let (Some(i), Some(on)) = (panel::indexed(what, "mode:"), body)
                    && let Some(Some(attachment)) = self.tab.panel.modes.get(i).cloned()
                {
                    match crate::commands::settle_attached(ctx, on, attachment, Default::default())
                    {
                        Ok(attached) => {
                            self.tab.pending_creation = None;
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
                    return;
                }
            }
        }
        if let Some((plane, support, face)) = chosen {
            self.tab.pending_creation = None;
            self.create_sketch_on_plane(
                ctx,
                body,
                plane,
                crate::NewSketchOn {
                    support,
                    face,
                    attached: None,
                    made_by,
                },
            );
        }
    }
}

/// A picture's field: its place among the sketch's pictures and its key.
fn image_field(id: &str) -> Option<(usize, &str)> {
    let (n, key) = id.strip_prefix("image:")?.split_once('.')?;
    Some((n.parse().ok()?, key))
}
