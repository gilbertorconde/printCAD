//! The sketcher's actions on the whole sketch or its selection: scan
//! guides and fits, drawings saved, repair, selection by relation, arrays
//! and joins, the clipboard and deletion.

use std::collections::HashSet;

use core_document::{InputResult, Workbench, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::sketch::{GeometryElement, Sketch, Vec2D};
use crate::tools::ToolState;
use crate::{SketchWorkbench, commands, drawing, ids_json, repair, scan, sketch, solver, tools};

impl SketchWorkbench {
    /// Repair the sketch being edited, ends within the snap reach joined,
    /// as `sketch.repair` does.
    /// Where the visible scans cross the open sketch's plane, cut again
    /// when the plane or a scan moves.
    pub(crate) fn scan_guides(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> std::sync::Arc<Vec<scan::Chain2>> {
        let (Some(feature), Some(kernel)) = (self.frame_sketch(ctx), ctx.kernel) else {
            return Default::default();
        };
        let Some(key) = scan::key(ctx.document, &feature.plane) else {
            return Default::default();
        };
        let mut cached = self.tab.scan_guides.borrow_mut();
        if let Some((k, chains)) = cached.as_ref()
            && *k == key
        {
            return chains.clone();
        }
        let chains = std::sync::Arc::new(scan::chains(ctx.document, kernel, &feature.plane));
        *cached = Some((key, chains.clone()));
        chains
    }

    /// The selected lines and rounds (every unsized one when none is)
    /// held at their probable design values.
    pub(crate) fn design_values(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        let picked: Vec<String> = self
            .tab
            .selected
            .in_order()
            .iter()
            .map(|u| u.to_string())
            .collect();
        let mut args = serde_json::json!({"sketch": id.0.to_string()});
        if !picked.is_empty() {
            args["elements"] = serde_json::json!(picked);
        }
        let args = commands::object(args);
        match self.run_command("sketch.design_values", &args, ctx) {
            Ok(answer) => ctx.record("sketch.design_values", args, answer),
            Err(why) => ctx.log_warn(why.to_string()),
        }
        InputResult::consumed()
    }

    /// How many of the scan slice's points lie within range of the
    /// sketch's curves, and how many there are; `None` without a slice or
    /// curves.
    pub(crate) fn scan_fit(
        &self,
        sketch: &Sketch,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(usize, usize)> {
        let guides = self.scan_guides(ctx);
        let off = scan::distance_to_sketch(sketch);
        let mut counts = (0, 0);
        for p in guides.iter().flat_map(|c| &c.points) {
            counts.0 += usize::from(off(*p)? <= self.options.scan_range);
            counts.1 += 1;
        }
        (counts.1 > 0).then_some(counts)
    }

    /// The open sketch at true scale as `kind` (DXF, SVG or CSV), to a
    /// file the user picks.
    pub(crate) fn save_drawing(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        kind: &str,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let sketch = &feature.sketch;
        let pictures = commands::pictures(sketch, ctx, &self.tab.picture_edges);
        let trace = drawing::traceability(sketch, &feature.plane, &pictures, "mm");
        let contents = match kind {
            "SVG" => {
                let (drawn, guides) = drawing::polylines(sketch);
                Ok(drawing::svg(&drawn, &guides, &trace))
            }
            "CSV" => Ok(drawing::csv(sketch, &trace).0),
            _ => ctx
                .kernel
                .ok_or_else(|| "there is no kernel to write with".to_string())
                .and_then(|k| {
                    k.write_dxf_drawing(&drawing::sheet(sketch, &pictures, &trace, false))
                        .map_err(|e| e.to_string())
                }),
        };
        match contents {
            Ok(text) => ctx.request(core_document::HostRequest::SaveFile {
                name: feature.sketch.name.clone(),
                kind: kind.to_string(),
                extension: kind.to_ascii_lowercase(),
                contents: text.into_bytes(),
            }),
            Err(why) => ctx.log_warn(format!("Save as {kind}: {why}")),
        }
        InputResult::consumed()
    }

    pub(crate) fn fit_scan(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        let args = commands::object(serde_json::json!({
            "sketch": id.0.to_string(),
            "construction": self.construction_mode(),
        }));
        match self.run_command("sketch.fit_scan", &args, ctx) {
            Ok(answer) => {
                ctx.log_info(format!(
                    "Fitted the scan's section: {} lines, {} arcs, {} circles",
                    answer["lines"], answer["arcs"], answer["circles"]
                ));
                ctx.record("sketch.fit_scan", args, answer);
            }
            Err(why) => ctx.log_warn(why.to_string()),
        }
        InputResult::consumed()
    }

    pub(crate) fn repair(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        let tolerance = Self::snap_tolerance(ctx, &feature.plane) * 0.5;
        let done = repair::repair(&mut feature.sketch, tolerance);
        if !done.is_empty() {
            self.tab.selected.clear();
            self.tab.selected_constraints.clear();
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.record(
                "sketch.repair",
                commands::object(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "tolerance": tolerance,
                })),
                serde_json::json!(done.describe()),
            );
        }
        ctx.log_info(format!("Sketch repair: {}", done.describe()));
        InputResult::consumed()
    }

    /// Select the constraints on the selected elements (a curve's own
    /// points count as the curve), leaving the elements.
    pub(crate) fn select_constraints_of(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        self.tab.selected_constraints = constraints_on(&feature.sketch, &self.tab.selected);
        self.tab.selected.clear();
        ctx.log_info(format!(
            "{} constraint(s) on the selection",
            self.tab.selected_constraints.len()
        ));
        InputResult::consumed()
    }

    /// Select the elements the selected constraints hold, leaving the
    /// constraints.
    pub(crate) fn select_elements_of(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let elements = elements_of(&feature.sketch, &self.tab.selected_constraints);
        self.tab.selected_constraints.clear();
        self.tab.selected.clear();
        self.tab.selected.extend(elements);
        ctx.log_info(format!("{} element(s) selected", self.tab.selected.len()));
        InputResult::consumed()
    }

    /// Add the sketch's reference `id` (its origin or an axis) to the
    /// selection, or, while a constraint tool waits for its picks, hand it
    /// to the tool as the next one.
    pub(crate) fn select_reference(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: Uuid,
        name: &str,
    ) -> InputResult {
        if self.get_active_sketch(ctx).is_none() {
            return InputResult::ignored();
        }
        if self.tab.armed.is_some() {
            self.armed_take(ctx, id);
        } else {
            self.tab.selected.insert(id);
            ctx.log_info(format!("Selected the {name}"));
        }
        InputResult::consumed()
    }

    /// Select the constraints whose geometry is gone.
    pub(crate) fn select_malformed(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let ids: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        for c in &feature.sketch.constraints {
            if sketch::constraint_refs(&c.kind)
                .iter()
                .any(|r| !ids.contains(r))
            {
                self.tab.selected_constraints.insert(c.id);
            }
        }
        ctx.log_info(format!(
            "{} malformed constraint(s)",
            self.tab.selected_constraints.len()
        ));
        InputResult::consumed()
    }

    /// Select what the constraints still let move: the free points, and,
    /// asked for the under-constrained geometry, the curves they belong to.
    pub(crate) fn select_free(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        curves_too: bool,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let free = solver::free_points(&feature.sketch);
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        self.tab.selected.extend(free.iter().copied());
        if curves_too {
            for g in &feature.sketch.geometry {
                let refs: Vec<Uuid> = match g {
                    GeometryElement::Point(_) => Vec::new(),
                    GeometryElement::Line(l) => vec![l.start, l.end],
                    GeometryElement::Arc(a) => vec![a.center, a.start, a.end],
                    GeometryElement::Circle(c) => vec![c.center],
                    curve @ (GeometryElement::Ellipse(_)
                    | GeometryElement::BSpline(_)
                    | GeometryElement::Conic(_)) => Sketch::curve_point_ids(curve),
                };
                if refs.iter().any(|r| free.contains(r)) {
                    self.tab.selected.insert(g.id());
                }
            }
        }
        ctx.log_info(format!(
            "{} element(s) with a degree of freedom",
            self.tab.selected.len()
        ));
        InputResult::consumed()
    }

    /// The selection repeated in a grid, as the panel's array settings say.
    pub(crate) fn array_selection(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let p = self.tab.tool_params;
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let effect = tools::array(
            &mut feature.sketch,
            &self.tab.selected,
            p.array_rows,
            p.array_cols,
            p.array_dx,
            p.array_dy,
            p.copies_linked,
        );
        if effect.changed {
            if let Some(log) = effect.log {
                ctx.log_info(log);
            }
            if let Some(id) = self.tab.active_sketch_id {
                let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
                items.sort();
                ctx.record(
                    "sketch.array",
                    commands::object(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "items": ids_json(&items),
                        "rows": p.array_rows,
                        "cols": p.array_cols,
                        "dx": p.array_dx,
                        "dy": p.array_dy,
                        "linked": p.copies_linked,
                    })),
                    commands::made_since(&feature.sketch, &before),
                );
            }
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        } else {
            ctx.log_warn("Select geometry and give the array at least two rows or columns");
        }
        InputResult::consumed()
    }

    /// Merge the selected curves into one spline, recorded as `sketch.join`.
    pub(crate) fn join_selection(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let effect = tools::join(
            &mut feature.sketch,
            &self.tab.selected,
            tools::JOIN_TOLERANCE,
        );
        if !effect.changed {
            if let Some(why) = effect.log {
                ctx.log_warn(why);
            }
            return InputResult::consumed();
        }
        if let Some(log) = effect.log {
            ctx.log_info(log);
        }
        if let Some(id) = self.tab.active_sketch_id {
            let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
            items.sort();
            ctx.record(
                "sketch.join",
                commands::object(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "items": ids_json(&items),
                })),
                commands::made_since(&feature.sketch, &before),
            );
        }
        let alive: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        self.tab.selected.retain(|id| alive.contains(id));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// Apply `edit` to the edited sketch's selection and record it as
    /// `command`, its arguments `extra` with the sketch and the selection.
    pub(crate) fn selection_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        command: &str,
        extra: serde_json::Value,
        edit: impl FnOnce(&mut Sketch, &HashSet<Uuid>) -> tools::ToolEffect,
    ) -> InputResult {
        let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
        items.sort();
        let mut args = extra;
        args["items"] = ids_json(&items);
        let selected: HashSet<Uuid> = items.into_iter().collect();
        self.sketch_edit(ctx, command, args, |s| edit(s, &selected))
    }

    /// Apply `edit` to the edited sketch and record it as `command` with
    /// `args` (the sketch's id added), solved and stored.
    pub(crate) fn sketch_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        command: &str,
        mut args: serde_json::Value,
        edit: impl FnOnce(&mut Sketch) -> tools::ToolEffect,
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let effect = edit(&mut feature.sketch);
        if !effect.changed {
            if let Some(why) = effect.log {
                ctx.log_warn(why);
            }
            return InputResult::consumed();
        }
        if let Some(log) = effect.log {
            ctx.log_info(log);
        }
        if let Some(id) = self.tab.active_sketch_id {
            args["sketch"] = serde_json::json!(id.0.to_string());
            ctx.record(
                command,
                commands::object(args),
                commands::made_since(&feature.sketch, &before),
            );
        }
        let alive: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        self.tab.selected.retain(|id| alive.contains(id));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// Copy the selection to the clipboard; cut removes it too.
    pub(crate) fn clipboard_copy(&mut self, ctx: &mut WorkbenchRuntimeContext, cut: bool) -> bool {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return false;
        };
        if self.tab.selected.is_empty() {
            ctx.log_warn("Nothing selected to copy");
            return true;
        }
        let mut clip = Sketch::new("clipboard");
        let count = tools::copy_from(
            &feature.sketch,
            &mut clip,
            &self.tab.selected,
            &tools::Similarity::translation(glam::DVec2::ZERO),
        );
        self.tab.clipboard = Some(clip);
        if cut {
            let mut ids: Vec<Uuid> = self.tab.selected.iter().copied().collect();
            ids.sort();
            commands::delete_items(&mut feature.sketch, &ids);
            if let Some(id) = self.tab.active_sketch_id {
                ctx.record(
                    "sketch.delete",
                    commands::object(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "items": ids_json(&ids),
                    })),
                    serde_json::Value::Null,
                );
            }
            self.tab.selected.clear();
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.log_info(format!("Cut {count} element(s)"));
        } else {
            ctx.log_info(format!("Copied {count} element(s)"));
        }
        true
    }

    /// Paste the clipboard at the cursor, or a little off where it was
    /// copied when the cursor is off the plane. The pasted elements
    /// become the selection.
    pub(crate) fn clipboard_paste(&mut self, ctx: &mut WorkbenchRuntimeContext) -> bool {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return false;
        };
        let Some(clip) = self.tab.clipboard.clone() else {
            ctx.log_warn("The clipboard is empty");
            return true;
        };
        let delta = match self.tab.cursor {
            // The middle of what was copied lands under the cursor.
            Some(cursor) => {
                let (lo, hi) = clip.geometry.iter().fold(
                    (glam::DVec2::splat(f64::MAX), glam::DVec2::splat(f64::MIN)),
                    |(lo, hi), g| match g {
                        GeometryElement::Point(p) => {
                            (lo.min(p.position.to_glam()), hi.max(p.position.to_glam()))
                        }
                        _ => (lo, hi),
                    },
                );
                let anchor = if lo.x <= hi.x {
                    (lo + hi) * 0.5
                } else {
                    glam::DVec2::ZERO
                };
                cursor.to_glam() - anchor
            }
            None => glam::DVec2::new(5.0, 5.0),
        };
        let before: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        let constraints_before: HashSet<Uuid> =
            feature.sketch.constraints.iter().map(|c| c.id).collect();
        let by = Vec2D::new(delta.x, delta.y);
        let count = commands::paste(&mut feature.sketch, &clip, by);
        if let Some(id) = self.tab.active_sketch_id {
            // What the paste made, its constraints among it, so a replay
            // can name them.
            let made = commands::made_since(&feature.sketch, &(before.clone(), constraints_before));
            ctx.record(
                "sketch.paste",
                commands::object(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "clipboard": serde_json::to_value(&clip).unwrap_or_default(),
                    "by": [by.x, by.y],
                })),
                made,
            );
        }
        self.tab.selected = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .filter(|id| !before.contains(id) && !feature.sketch.derived.contains(id))
            .collect();
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        ctx.log_info(format!("Pasted {count} element(s)"));
        true
    }

    /// Select the conflicting (or redundant) constraints of the last
    /// diagnosis and the geometry they reference.
    pub(crate) fn select_offenders(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        conflicting: bool,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let diag = match &self.tab.last_diagnosis {
            Some(d) => d.clone(),
            None => {
                let d = solver::diagnose(&feature.sketch);
                self.tab.last_diagnosis = Some(d.clone());
                d
            }
        };
        let offenders = if conflicting {
            &diag.conflicting
        } else {
            &diag.redundant
        };
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        for c in &feature.sketch.constraints {
            if offenders.contains(&c.id) {
                self.tab.selected_constraints.insert(c.id);
                self.tab.selected.extend(sketch::constraint_refs(&c.kind));
            }
        }
        InputResult::consumed()
    }

    /// Remove every element (and the constraints on them), or every
    /// constraint, then re-solve.
    pub(crate) fn delete_all(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        geometry: bool,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let ids: Vec<Uuid> = if geometry {
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect()
        } else {
            feature.sketch.constraints.iter().map(|c| c.id).collect()
        };
        self.delete_items(ctx, ids)
    }

    /// Delete `ids` (elements or constraints) from the edited sketch, as
    /// `sketch.delete` does, and record it.
    pub(crate) fn delete_items(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        ids: Vec<Uuid>,
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let removed = commands::delete_items(&mut feature.sketch, &ids);
        ctx.log_info(format!("Deleted {removed} item(s)"));
        if let (Some(id), false) = (self.tab.active_sketch_id, ids.is_empty()) {
            ctx.record(
                "sketch.delete",
                commands::object(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "items": ids_json(&ids),
                })),
                serde_json::Value::Null,
            );
        }
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        self.tab.hovered = None;
        self.tab.tool_state = ToolState::Idle;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }
}

/// The constraints on `elements` (a curve's own points count as the curve).
pub(crate) fn constraints_on(sketch: &Sketch, elements: &HashSet<Uuid>) -> HashSet<Uuid> {
    let mut touched = elements.clone();
    for id in elements {
        if let Some(geom) = sketch.get_geometry(*id) {
            touched.extend(Sketch::curve_point_ids(geom));
        }
    }
    sketch
        .constraints
        .iter()
        .filter(|c| {
            sketch::constraint_refs(&c.kind)
                .iter()
                .any(|r| touched.contains(r))
        })
        .map(|c| c.id)
        .collect()
}

/// The elements `constraints` hold.
pub(crate) fn elements_of(sketch: &Sketch, constraints: &HashSet<Uuid>) -> Vec<Uuid> {
    sketch
        .constraints
        .iter()
        .filter(|c| constraints.contains(&c.id))
        .flat_map(|c| sketch::constraint_refs(&c.kind))
        .collect()
}
