//! Pointer and key input: the reading tools, the view's drag handles,
//! a pattern's occurrence dots, values typed for a handle, and the
//! tools a click acts with.

use crate::{
    DesignFeature, DesignWorkbench, commands, face_snap, generators, handles, instances, params,
    read_scan, recognize, region_pick,
};
use core_document::{
    BodyId, HostRequest, InputResult, Workbench, WorkbenchFeature, WorkbenchId,
    WorkbenchInputEvent, WorkbenchRuntimeContext, base_tool_id,
};
use wb_sketch::SketchFeature;

impl DesignWorkbench {
    /// While a reading tool is armed: a click on a scan reads the round or
    /// thread there, Escape stops.
    fn read_hole_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        match event {
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } => {
                self.tab.reading = None;
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } => {
                let scan = ctx
                    .hovered_body_id
                    .map(BodyId)
                    .filter(|b| ctx.document.body_scan(*b).is_some())?;
                let at = ctx.hovered_face.as_ref()?.point.map(f64::from);
                let kernel = ctx.kernel?;
                if self.tab.reading == Some("design.snap_to_scan") {
                    let body =
                        Self::target_body(ctx).filter(|b| ctx.document.body_scan(*b).is_none());
                    let face = body.and_then(|b| ctx.selected_face_in(b));
                    let (Some(body), Some(face)) = (body, face) else {
                        ctx.log_warn("Select the model's face first, then click the scan");
                        return Some(InputResult::consumed());
                    };
                    let args = commands::object(serde_json::json!({
                        "body": body.0.to_string(),
                        "face_point": face.point.map(f64::from),
                        "face_normal": face.normal.map(f64::from),
                        "scan": scan.0.to_string(),
                        "at": at,
                    }));
                    match self.run_command("design.snap_to_scan", &args, ctx) {
                        Ok(answer) => {
                            ctx.log_info(match answer.get("moved").and_then(|v| v.as_f64()) {
                                Some(moved) => format!(
                                    "Face moved {moved:.3} mm onto the scan (turned {:.1}° from it, σ {:.3})",
                                    answer["angle"].as_f64().unwrap_or_default(),
                                    answer["sigma"].as_f64().unwrap_or_default()
                                ),
                                None => format!(
                                    "Round face R{:.3} → R{:.3} (likely R{}), axes {:.3} mm apart",
                                    answer["radius"].as_f64().unwrap_or_default(),
                                    answer["fitted"].as_f64().unwrap_or_default(),
                                    answer["design"].as_f64().unwrap_or_default(),
                                    answer["axis_offset"].as_f64().unwrap_or_default()
                                ),
                            });
                            ctx.record("design.snap_to_scan", args, answer);
                        }
                        Err(why) => ctx.log_warn(why.to_string()),
                    }
                    return Some(InputResult::consumed());
                }
                if self.tab.reading == Some("design.read_thread") {
                    match read_scan::read_thread(ctx.document, kernel, scan, at) {
                        Ok(t) => {
                            let near = read_scan::nearest_threads(t.major, t.pitch, 1);
                            let named = near.first().map_or(String::new(), |n| {
                                format!(
                                    "; nearest {} {}",
                                    n["standard"].as_str().unwrap_or_default(),
                                    n["size"].as_str().unwrap_or_default()
                                )
                            });
                            ctx.log_info(format!(
                                "{} thread, pitch {:.3} mm, {}-hand, major Ø{:.3}, minor Ø{:.3}, \
                                 clarity {:.2}{named}",
                                if t.internal { "Internal" } else { "External" },
                                t.pitch,
                                if t.right_handed { "right" } else { "left" },
                                t.major,
                                t.minor,
                                t.clarity
                            ));
                        }
                        Err(why) => ctx.log_warn(why),
                    }
                    return Some(InputResult::consumed());
                }
                match read_scan::read_round(ctx.document, kernel, scan, at) {
                    Ok(r) if r.fillet() => {
                        let radius = r.diameter / 2.0;
                        ctx.log_info(format!(
                            "{} fillet R{radius:.3} mm (likely R{}), over {:.0}°, σ {:.3}",
                            if r.hole { "Inside" } else { "Outside" },
                            metrology::design::radius_mm(radius),
                            r.sweep_deg,
                            r.sigma
                        ));
                    }
                    Ok(r) => {
                        let near = read_scan::nearest_sizes(r.diameter, r.hole, 1);
                        let named = near.first().map_or(String::new(), |n| {
                            format!(
                                "; nearest {} {} {} (Ø{})",
                                n["standard"].as_str().unwrap_or_default(),
                                n["size"].as_str().unwrap_or_default(),
                                n["as"].as_str().unwrap_or_default(),
                                n["diameter"].as_f64().unwrap_or_default()
                            )
                        });
                        ctx.log_info(format!(
                            "{} Ø{:.3} mm, {:.2} mm long, σ {:.3}{named}",
                            if r.hole { "Hole" } else { "Boss" },
                            r.diameter,
                            r.depth,
                            r.sigma
                        ));
                    }
                    Err(why) => ctx.log_warn(why),
                }
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    /// The handle held, else the ones the open task's feature offers.
    pub(crate) fn handles(&self, ctx: &WorkbenchRuntimeContext) -> Vec<handles::Handle> {
        if let Some(held) = &self.tab.held {
            return vec![held.clone()];
        }
        let eye = glam::Vec3::from_array(ctx.camera_position);
        self.task_feature()
            .map(|feature| handles::handles_of(ctx.document, feature, eye))
            .unwrap_or_default()
    }

    /// A held handle's steps: coarse, and fine with Shift.
    pub(crate) fn handle_steps(&self, handle: &handles::Handle) -> (f32, f32) {
        if handle.angle {
            (
                self.options.handle_angle_step,
                self.options.handle_fine_angle_step,
            )
        } else {
            (self.options.handle_step, self.options.handle_fine_step)
        }
    }

    /// The occurrences of the open task's pattern, where their dots stand.
    pub(crate) fn occurrences(&self, ctx: &WorkbenchRuntimeContext) -> Vec<instances::Occurrence> {
        self.task_feature()
            .map(|feature| instances::occurrences_of(ctx.document, ctx.kernel, feature))
            .unwrap_or_default()
    }

    /// A click on an occurrence's dot of the open pattern leaves it out or
    /// brings it back; moves keep which dot is under the pointer.
    fn occurrence_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        use core_document::MouseButton;
        match event {
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                let occurrences = self.occurrences(ctx);
                self.tab.hovered_occurrence = instances::hit(ctx, &occurrences, *viewport_pos);
                None
            }
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => {
                let feature = self.task_feature()?;
                let occurrences = instances::occurrences_of(ctx.document, ctx.kernel, feature);
                let index = instances::hit(ctx, &occurrences, *viewport_pos)?;
                let mut part = ctx
                    .document
                    .get_feature_data(feature)
                    .and_then(|d| DesignFeature::from_json(d).ok())?;
                if part.toggle_skipped(index)
                    && ctx
                        .document
                        .update_feature_data(feature, part.to_json())
                        .is_ok()
                {
                    ctx.document.mark_feature_dirty(feature);
                }
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    /// A press on the open task's handle takes hold of it; moves drag the
    /// number it stands for, written live as the panel's edits are.
    fn handle_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        use core_document::MouseButton;
        match event {
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => {
                let mut offered = self.handles(ctx);
                if let Some(i) = handles::under(ctx, &offered, *viewport_pos) {
                    let held = offered.swap_remove(i);
                    self.tab.held_from = ctx.document.get_feature_data(held.feature).cloned();
                    self.tab.dragged = vec![held.value];
                    self.tab.snapped = None;
                    self.tab.held = Some(held);
                    self.tab.held_moved = false;
                    return Some(InputResult::consumed());
                }
                // A click on a region of the open task's sketch chooses it.
                let task = self.task_feature()?;
                region_pick::click(ctx, task, *viewport_pos).then(InputResult::consumed)
            }
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                let (coarse, fine) = self.handle_steps(self.tab.held.as_ref()?);
                let step = if ctx.shift_down { fine } else { coarse };
                let held = self.tab.held.as_mut()?;
                // The limit may come while the handle is held.
                held.stop = handles::drag_stop(ctx.document, held);
                let (origin, dir) = ctx.viewport_to_ray(*viewport_pos)?;
                let mut value = held.value_at(
                    glam::Vec3::from_array(origin),
                    glam::Vec3::from_array(dir),
                    step,
                )?;
                // Near a flat face square to its way, a length lands on it;
                // Alt drags past.
                self.tab.snapped = (face_snap::snaps(held) && !ctx.alt_down)
                    .then(|| face_snap::nearest(ctx, held, value, &self.tab.dragged))
                    .flatten();
                if let Some(snap) = self.tab.snapped {
                    value = held.dragged_within(snap.value);
                }
                // Through its start, a reversible one turns its feature
                // round.
                let flip = held.range == handles::Range::Reversible && value < 0.0;
                if value == held.value && !flip {
                    return Some(InputResult::consumed());
                }
                if flip {
                    held.turn_round();
                    held.value = -value;
                } else {
                    held.value = value;
                }
                self.tab.held_moved = true;
                self.tab.dragged.push(held.value);
                let (feature, pointer, value) = (held.feature, held.pointer.clone(), held.value);
                let Some(mut data) = ctx.document.get_feature_data(feature).cloned() else {
                    return Some(InputResult::consumed());
                };
                if flip {
                    let Ok(mut part) = DesignFeature::from_json(&data) else {
                        return Some(InputResult::consumed());
                    };
                    if !handles::reverse(&mut part) {
                        return Some(InputResult::consumed());
                    }
                    data = part.to_json();
                }
                if handles::write(&mut data, &pointer, value)
                    && ctx.document.update_feature_data(feature, data).is_ok()
                    && self.options.update_while_editing
                {
                    ctx.document.mark_feature_dirty(feature);
                }
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                ..
            } => {
                let held = self.tab.held.take()?;
                self.tab.held_from = None;
                self.tab.snapped = None;
                self.tab.dragged.clear();
                // A click on a reversible handle turns its feature round.
                if !self.tab.held_moved
                    && held.range == handles::Range::Reversible
                    && let Some(mut part) = ctx
                        .document
                        .get_feature_data(held.feature)
                        .and_then(|d| DesignFeature::from_json(d).ok())
                    && handles::reverse(&mut part)
                    && let Err(e) = ctx
                        .document
                        .update_feature_data(held.feature, part.to_json())
                {
                    ctx.log_warn(format!("Could not turn the feature round: {e}"));
                }
                ctx.document.mark_feature_dirty(held.feature);
                Some(InputResult::consumed())
            }
            // Escape puts the held number back where the press found it.
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.tab.held.is_some() => {
                self.put_back_handle(ctx);
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    /// Let go of the held handle, its number left where the drag set it,
    /// and of a value typed for it.
    pub(crate) fn let_go_of_handle(&mut self) {
        self.tab.held = None;
        self.tab.held_from = None;
        self.tab.held_moved = false;
        self.tab.typed = None;
        self.tab.snapped = None;
        self.tab.dragged.clear();
    }

    /// Let go of the held handle and write its feature back as the press
    /// found it.
    pub(crate) fn put_back_handle(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.tab.held.is_none() {
            return;
        }
        let moved = self.tab.held_moved;
        let (held, from) = (self.tab.held.take(), self.tab.held_from.take());
        self.let_go_of_handle();
        if let (true, Some(held), Some(data)) = (moved, held, from)
            && ctx.document.update_feature_data(held.feature, data).is_ok()
        {
            ctx.document.mark_feature_dirty(held.feature);
        }
    }

    /// A value typed while a handle is held: digits start it, and then any
    /// text a formula takes; Backspace takes a character back, Enter sets
    /// it, Escape drops it, a click elsewhere sets it.
    fn typed_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        use core_document::KeyCode;
        match event {
            WorkbenchInputEvent::Text { text } => {
                if self.tab.typed.is_none() {
                    self.tab.typed = Some((self.tab.held.clone()?, String::new()));
                }
                let (_, typed) = self.tab.typed.as_mut()?;
                typed.extend(text.chars().filter(|c| !c.is_control()));
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::KeyPress { key } if self.tab.typed.is_some() => match key {
                KeyCode::Backspace => {
                    if let Some((_, typed)) = self.tab.typed.as_mut() {
                        typed.pop();
                    }
                    Some(InputResult::consumed())
                }
                KeyCode::Enter => {
                    self.set_typed(ctx);
                    Some(InputResult::consumed())
                }
                KeyCode::Escape => {
                    self.tab.typed = None;
                    Some(InputResult::consumed())
                }
                _ => None,
            },
            WorkbenchInputEvent::MousePress { .. }
                if self.tab.typed.is_some() && self.tab.held.is_none() =>
            {
                self.set_typed(ctx);
                None
            }
            _ => None,
        }
    }

    /// Set the typed value on its handle's number: a number as it is (in
    /// the handle's range), anything else as the number's formula.
    fn set_typed(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some((handle, text)) = self.tab.typed.take() else {
            return;
        };
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let feature = handle.feature;
        match text.parse::<f32>() {
            Ok(value) => {
                let value = handle.clamped(value);
                let Some(mut data) = ctx.document.get_feature_data(feature).cloned() else {
                    return;
                };
                if handles::write(&mut data, &handle.pointer, value)
                    && let Err(e) = ctx.document.update_feature_data(feature, data)
                {
                    ctx.log_warn(format!("Could not set {}: {e}", handle.label));
                }
            }
            Err(_) => {
                let key = ctx
                    .document
                    .get_feature_meta(feature)
                    .map(params::feature_parameters)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|p| p.pointer == handle.pointer)
                    .map(|p| p.key);
                let Some(key) = key else {
                    return;
                };
                if let Err(e) =
                    ctx.document
                        .set_feature_formula(feature, key, Some(text.to_string()))
                {
                    ctx.log_warn(format!("Could not set {} to {text}: {e}", handle.label));
                }
            }
        }
        ctx.document.mark_feature_dirty(feature);
    }

    pub(crate) fn take_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if self.tab.reading.is_some()
            && let Some(result) = self.read_hole_input(event, ctx)
        {
            return result;
        }
        if let Some(result) = self.occurrence_input(event, ctx) {
            return result;
        }
        if let Some(result) = self.typed_input(event, ctx) {
            return result;
        }
        if let Some(result) = self.handle_input(event, ctx) {
            return result;
        }
        // Feature tools are Actions: the host hands them over the moment they
        // are activated and clears them once handled.
        let base = active_tool.map(base_tool_id);
        match base {
            Some("design.new_body") => {
                let body = ctx.document.create_body(None);
                let name = ctx
                    .document
                    .body(body)
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| format!("body {:?}", body));
                ctx.log_info(format!("Created {name}"));
                ctx.record(
                    "doc.new_body",
                    commands::object(serde_json::json!({"name": name})),
                    serde_json::json!(body.0.to_string()),
                );
                ctx.request(HostRequest::SelectBody(body));
                ctx.request(HostRequest::JournalLabel("Create body".to_string()));
                InputResult::consumed()
            }
            Some("design.scan_deviation") => {
                self.tab.scan_deviation = !self.tab.scan_deviation;
                InputResult::consumed()
            }
            Some("design.scan_symmetry") => {
                let body = Self::target_body(ctx).filter(|b| ctx.document.body_scan(*b).is_none());
                let scan = ctx.document.bodies().iter().map(|b| b.id).find(|b| {
                    ctx.document.body_scan(*b).is_some()
                        && ctx.document.imported_body_effective_visible(*b)
                });
                let (Some(body), Some(scan)) = (body, scan) else {
                    ctx.log_warn("Select the body to model in; a visible scan is read");
                    return InputResult::consumed();
                };
                match commands::scan_symmetry(ctx, scan, body, commands::SYMMETRY_ACCEPT) {
                    Ok(answer) => {
                        ctx.log_info(format!(
                            "Scan symmetry: mirrors to {:.3} mm RMS; Mirror uses it",
                            answer["rms"].as_f64().unwrap_or_default()
                        ));
                        ctx.record(
                            "design.scan_symmetry",
                            commands::object(serde_json::json!({
                                "scan": scan.0.to_string(),
                                "body": body.0.to_string(),
                            })),
                            answer,
                        );
                    }
                    Err(why) => ctx.log_warn(why.to_string()),
                }
                InputResult::consumed()
            }
            Some(id @ ("design.read_hole" | "design.read_thread" | "design.snap_to_scan")) => {
                let id = match id {
                    "design.read_hole" => "design.read_hole",
                    "design.read_thread" => "design.read_thread",
                    _ => "design.snap_to_scan",
                };
                self.tab.reading = (self.tab.reading != Some(id)).then_some(id);
                if self.tab.reading.is_some() {
                    ctx.log_info("Click on a scan to read it; Escape stops");
                }
                InputResult::consumed()
            }
            Some("design.recognize_holes") => {
                let Some(body) = Self::target_body(ctx) else {
                    ctx.log_warn("Select a body whose holes to recognize");
                    return InputResult::consumed();
                };
                match recognize::recognize_holes(ctx, body) {
                    Ok(made) => {
                        ctx.record(
                            "design.recognize_holes",
                            commands::object(serde_json::json!({"body": body.0.to_string()})),
                            serde_json::json!({
                                "holes": made.holes,
                                "left": made.left,
                                "features": made
                                    .features
                                    .iter()
                                    .map(|f| f.0.to_string())
                                    .collect::<Vec<_>>(),
                            }),
                        );
                        let left = if made.left > 0 {
                            format!(
                                "; {} bore(s) left as they are (counterbores, slots)",
                                made.left
                            )
                        } else {
                            String::new()
                        };
                        if made.holes == 0 {
                            ctx.log_info(format!("No round holes to recognize{left}"));
                        } else {
                            ctx.log_info(format!(
                                "{} hole(s) made Hole features{left}",
                                made.holes
                            ));
                        }
                        ctx.request(HostRequest::JournalLabel("Recognize holes".to_string()));
                    }
                    Err(message) => ctx.log_warn(format!("Cannot recognize holes: {message}")),
                }
                InputResult::consumed()
            }
            Some(
                tool @ ("design.datum_plane"
                | "design.datum_line"
                | "design.datum_point"
                | "design.coordinate_system"),
            ) => self.insert_datum(ctx, tool),
            Some("design.map_sketch") => {
                // The selected sketch moves onto the face the last body
                // click landed on.
                let (Some(sketch_id), Some(face)) = (Self::selected_sketch(ctx), ctx.selected_face)
                else {
                    ctx.log_warn("Select a sketch in the tree, then click a face");
                    return InputResult::consumed();
                };
                let Some(mut feature) = ctx
                    .document
                    .get_feature_data(sketch_id)
                    .and_then(|d| SketchFeature::from_json(d).ok())
                else {
                    return InputResult::consumed();
                };
                // The sketch keeps its plane in its body's frame.
                let face = match ctx
                    .document
                    .get_feature_meta(sketch_id)
                    .and_then(|n| n.body)
                {
                    Some(body) => face.moved(&ctx.document.body_placement(body).inverse()),
                    None => face,
                }
                .on_its_plane();
                let plane = wb_sketch::sketch::SketchPlane::from_face(
                    wb_sketch::boundary::widen3(face.point),
                    wb_sketch::boundary::widen3(face.normal),
                );
                feature.plane = plane;
                feature.sketch.plane = plane;
                feature.offset = Default::default();
                // Mapped to the face, it follows the face, not the datum it was on.
                if feature.support.take().is_some() {
                    ctx.document
                        .set_feature_dependencies(sketch_id, feature.dependencies());
                }
                match ctx
                    .document
                    .update_feature_data(sketch_id, feature.to_json())
                {
                    Ok(()) => {
                        ctx.document.mark_feature_dirty(sketch_id);
                        ctx.log_info("Sketch mapped to the picked face");
                        ctx.record(
                            "sketch.set_plane",
                            commands::object(serde_json::json!({
                                "sketch": sketch_id.0.to_string(),
                                "normal": plane.normal,
                                "origin": plane.origin,
                                "x_axis": plane.x_axis,
                            })),
                            serde_json::Value::Null,
                        );
                    }
                    Err(err) => ctx.log_error(format!("Could not move the sketch: {err}")),
                }
                InputResult::consumed()
            }
            Some("design.centre_line") => {
                self.start_centre_line(ctx);
                InputResult::consumed()
            }
            Some("design.edit_sketch") => {
                if Self::selected_sketch(ctx).is_some() {
                    // The sketcher picks the active object up as its edit
                    // session on activation.
                    ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from(
                        wb_sketch::KIND,
                    )));
                } else {
                    ctx.log_warn("Select a sketch in the tree first");
                }
                InputResult::consumed()
            }
            Some("design.generator") => generators::insert(ctx, active_tool.unwrap_or_default()),
            Some("design.new_sketch") => Self::start_sketch(ctx, None),
            Some(tool) if tool.starts_with("design.") => {
                let full = active_tool.unwrap_or(tool);
                self.insert_feature(ctx, full)
            }
            _ => InputResult::ignored(),
        }
    }
}
