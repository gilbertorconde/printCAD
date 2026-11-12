//! The sketcher's pointer and key handling: snapping and landing, clicks,
//! drags, releases, box selection, the edge loupe and the keyboard.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use core_document::{
    FeatureId, HostRequest, InputResult, KeyCode, WorkbenchRuntimeContext, base_tool_id,
};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::overlay::SketchProjector;
use crate::selection::{connected_curves, element_fully_inside, element_touches};
use crate::sketch::{Sketch, SketchPlane, Vec2D};
use crate::tools::{ToolParams, ToolState};
use crate::{
    BoxSelect, DragState, DrawRecord, KIND, LabelDrag, LinePress, MAX_STROKE_TRIMS,
    POLYLINE_ARC_ACTION, SELECTION_TOOLS, SNAP_TOLERANCE_PX, SketchWorkbench, ids_json,
    is_draw_tool, is_transform_tool, text_args,
};
use crate::{DimEdit, Loupe, stored_sketch};
use crate::{
    chain, commands, glyphs, overlay, ovp, picture_edges, sketch, snap, solver, step, text, tools,
};

impl SketchWorkbench {
    /// Press on `id`: it joins the selection when it is new, and a drag is
    /// armed: of the whole selection when the press landed inside it, of
    /// this element alone otherwise. Selection inside a sketch accumulates,
    /// so no modifier is needed; clicking empty space clears it.
    fn begin_drag(&mut self, sketch: &Sketch, id: Uuid, cursor: Vec2D) {
        let was_selected = self.tab.selected.contains(&id);
        if !was_selected {
            self.tab.selected.insert(id);
        }
        let moving: Vec<Uuid> = if was_selected {
            self.tab.selected.iter().copied().collect()
        } else {
            vec![id]
        };
        let points = step::drag_targets(sketch, &moving);
        self.tab.dragging = Some(DragState {
            points,
            grab: cursor,
            hit: id,
            was_selected,
            moved: false,
            elements: moving,
            delta: Vec2D::new(0.0, 0.0),
            target: None,
        });
    }

    /// Project a viewport-local cursor position onto the sketch plane and
    /// express it in sketch coordinates.
    pub(crate) fn cursor_to_sketch(
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
        viewport_pos: (f32, f32),
    ) -> Option<Vec2D> {
        SketchProjector::new(ctx, *plane).sketch_at([viewport_pos.0, viewport_pos.1])
    }

    /// The tool settings a snap reads, the panel's switches in them.
    pub(crate) fn snap_params(&self) -> ToolParams {
        ToolParams {
            arc_helpers: self.options.arc_helpers,
            ..self.tab.tool_params
        }
    }

    /// The tool settings a click runs with, the panel's switches in them.
    pub(crate) fn click_params(&self) -> ToolParams {
        // The copy tool is the move tool with at least one copy.
        ToolParams {
            copies: self.effective_copies(&self.tab.tool_params),
            auto_constraints: self.options.auto_constraints,
            arc_helpers: self.options.arc_helpers,
            ..self.tab.tool_params
        }
    }

    /// Bring a line or polyline gesture up to the sketch when the document
    /// moved on without a write of ours.
    pub(crate) fn follow_history(&mut self, ctx: &WorkbenchRuntimeContext, sketch: &Sketch) {
        let own = self.tab.own_seq.load(std::sync::atomic::Ordering::Relaxed);
        if own != ctx.document.mutation_seq() {
            self.follow_chain(sketch);
        }
    }

    /// Bring a line or polyline gesture up to `sketch` after a change it did
    /// not make: a segment taken back puts the gesture before it, and its
    /// click leaves the recording.
    pub(crate) fn follow_chain(&mut self, sketch: &Sketch) {
        let tab = &mut *self.tab;
        let taken = tab.chain_links.follow(&mut tab.tool_state, sketch);
        if let Some(record) = tab.draw_record.as_mut()
            && !taken.is_empty()
        {
            for event in taken {
                if event < record.events.len() {
                    record.events.remove(event);
                }
            }
            record.changed &= !tab.chain_links.is_empty();
        }
    }

    /// Where a click of `tool` at `cursor` goes before its snap, and the
    /// snap's reach (off for drawing tools when snapping is switched off;
    /// modify tools keep their pick tolerance): a point within reach as it
    /// is, anything else rounded to the grid first. The click and the cue before it
    /// both come through here, so they cannot disagree.
    pub(crate) fn landing(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
    ) -> (Vec2D, f64) {
        let plane = feature.plane;
        let tol = if self.tab.snap_off && is_draw_tool(tool) {
            0.0
        } else {
            Self::snap_tolerance(ctx, &plane)
        };
        if is_transform_tool(tool) {
            // A base, target or centre lands on a point within reach as
            // the tool takes it, anywhere else on the grid.
            let on_point = matches!(
                crate::snap::snap_to_point(&feature.sketch, cursor, tol, &[]),
                crate::snap::SnapTarget::Existing(_)
            );
            return if on_point {
                (cursor, tol)
            } else {
                (self.grid_snapped(ctx, &plane, cursor), tol)
            };
        }
        if !is_draw_tool(tool) {
            return (cursor, tol);
        }
        // With Shift held, a segment leaves its start in steps of the angle
        // snap's step, as far as the cursor reaches along that direction.
        if ctx.shift_down
            && let Some(from) = tools::snap_context(&self.tab.tool_state, &feature.sketch).from
        {
            return (
                snap::angle_step(from, cursor, self.options.angle_step_deg),
                tol,
            );
        }
        // A snap onto a point wins over the grid; one onto a curve, an axis
        // or an alignment leaves a direction free for the grid to round.
        // A direction carried onto a curve is a point as much.
        use crate::snap::SnapKind;
        let snapped = tools::snap_at(
            &self.tab.tool_state,
            &feature.sketch,
            cursor,
            tol,
            &self.snap_params(),
        );
        let on_point = snapped.on.is_some()
            || matches!(
                snapped.kind,
                Some(
                    SnapKind::Endpoint
                        | SnapKind::Center
                        | SnapKind::Origin
                        | SnapKind::Intersection
                        | SnapKind::Midpoint
                )
            );
        if on_point {
            (cursor, tol)
        } else if let Some((edge, _)) = self.edge_snap(ctx, feature, tool, cursor, tol) {
            // On a picture's edge, exactly: nothing else moves it.
            (edge, 0.0)
        } else {
            (self.grid_snapped(ctx, &plane, cursor), tol)
        }
    }

    /// The point on a reference picture's found edges nearest `cursor`
    /// within `tol`, and which picture, for a tool that places points,
    /// unless Alt is held.
    fn edge_snap(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        tol: f64,
    ) -> Option<(Vec2D, usize)> {
        if tol <= 0.0 || ctx.alt_down || !EDGE_SNAP_TOOLS.contains(&tool) {
            return None;
        }
        let off = |p: Vec2D| (p.x - cursor.x).hypot(p.y - cursor.y);
        feature
            .sketch
            .images
            .iter()
            .enumerate()
            .filter_map(|(i, image)| {
                let set = self.tab.picture_edges.get(image.asset, || {
                    ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
                })?;
                Some((picture_edges::nearest_on_edge(image, &set, cursor, tol)?, i))
            })
            .min_by(|a, b| off(a.0).total_cmp(&off(b.0)))
    }

    /// The loupe over a click snapping to a picture's edge: the picture
    /// round the point magnified [`LOUPE_ZOOM`] times in a square beside
    /// the cursor, and in viewport pixels its outline, its middle (the
    /// point) and the edges inside it.
    pub(crate) fn loupe(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        proj: &SketchProjector,
    ) -> Option<Loupe> {
        let (Some(tool), Some(cursor)) = (self.tab.last_tool.as_deref(), self.tab.cursor) else {
            return None;
        };
        if self.tab.box_select.is_some() || !self.tab.dim_capture.typed(ctx.document).is_empty() {
            return None;
        }
        let tol = Self::snap_tolerance(ctx, &feature.plane);
        if self.tab.snap_off {
            return None;
        }
        // A point within reach wins over the edge, as the click has it.
        let (landed, _) = self.landing(ctx, feature, tool, cursor);
        let (at, index) = self.edge_snap(ctx, feature, tool, cursor, tol)?;
        if landed != at {
            return None;
        }
        let image = &feature.sketch.images[index];
        let decoded = self.tab.image_cache.get(image.asset, || {
            ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
        })?;
        let (w, h) = (decoded.width as usize, decoded.height as usize);
        let frame = picture_edges::EdgeSet {
            width: w,
            height: h,
            chains: Vec::new(),
        };
        let px = picture_edges::to_pixel(image, &frame, at);
        let screen = |p: [f64; 2]| proj.to_px(picture_edges::to_sketch(image, &frame, p));
        let [ax, ay] = screen(px)?;
        let [bx, by] = screen([px[0] + 1.0, px[1]])?;
        let texel = f64::from((bx - ax).hypot(by - ay)).max(1e-6);
        let side = LOUPE_PX * f64::from(ctx.pixels_per_point);
        let half = (side / (2.0 * LOUPE_ZOOM) / texel).max(2.0);
        let x0 = (px[0] - half).floor().max(0.0) as usize;
        let y0 = (px[1] - half).floor().max(0.0) as usize;
        let x1 = ((px[0] + half).ceil() as usize).min(w);
        let y1 = ((px[1] + half).ceil() as usize).min(h);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
        for row in y0..y1 {
            rgba.extend_from_slice(&decoded.rgba[(row * w + x0) * 4..(row * w + x1) * 4]);
        }
        // Up and to the right of the cursor, below it near the top.
        let gap = side / 2.0 + 16.0 * f64::from(ctx.pixels_per_point);
        let (cx, cy) = (f64::from(ax), f64::from(ay));
        let middle = [
            cx + gap,
            if cy - gap - side / 2.0 < 0.0 {
                cy + gap
            } else {
                cy - gap
            },
        ];
        let zoom = move |p: [f32; 2]| {
            [
                (middle[0] + (f64::from(p[0]) - cx) * LOUPE_ZOOM) as f32,
                (middle[1] + (f64::from(p[1]) - cy) * LOUPE_ZOOM) as f32,
            ]
        };
        let corner = |x: usize, y: usize| screen([x as f64 - 0.5, y as f64 - 0.5]).map(zoom);
        let corners = [
            corner(x0, y0)?,
            corner(x1, y0)?,
            corner(x1, y1)?,
            corner(x0, y1)?,
        ];
        let (hi, lo) = image.asset.as_u64_pair();
        let key = (hi ^ lo).rotate_left(17)
            ^ ((x0 as u64) << 48 | (y0 as u64) << 32)
            ^ ((x1 as u64) << 16 | y1 as u64);
        let mut edges = Vec::new();
        if let Some(set) = self.tab.picture_edges.get(image.asset, || None) {
            let (lo, hi) = (
                picture_edges::to_sketch(image, &frame, [x0 as f64 - 0.5, y0 as f64 - 0.5]),
                picture_edges::to_sketch(image, &frame, [x1 as f64 - 0.5, y1 as f64 - 0.5]),
            );
            let (a, b) = (
                picture_edges::to_pixel(image, &set, lo),
                picture_edges::to_pixel(image, &set, hi),
            );
            let (min, max) = (
                [a[0].min(b[0]), a[1].min(b[1])],
                [a[0].max(b[0]), a[1].max(b[1])],
            );
            let inside =
                |p: &[f64; 2]| p[0] >= min[0] && p[0] <= max[0] && p[1] >= min[1] && p[1] <= max[1];
            for chain in &set.chains {
                for pair in chain.points.windows(2) {
                    if inside(&pair[0]) && inside(&pair[1]) {
                        let to = |p| {
                            proj.to_px(picture_edges::to_sketch(image, &set, p))
                                .map(zoom)
                        };
                        if let (Some(p), Some(q)) = (to(pair[0]), to(pair[1])) {
                            edges.push((p, q));
                        }
                    }
                }
            }
        }
        Some(Loupe {
            image: core_document::ScreenSpaceImage {
                key,
                size: [(x1 - x0) as u32, (y1 - y0) as u32],
                rgba: std::sync::Arc::new(rgba),
                corners,
                opacity: 1.0,
            },
            middle: [middle[0] as f32, middle[1] as f32],
            edges,
        })
    }

    /// Pixel tolerance converted into sketch units at the current zoom.
    pub(crate) fn snap_tolerance(ctx: &WorkbenchRuntimeContext, plane: &SketchPlane) -> f64 {
        let proj = SketchProjector::new(ctx, *plane);
        SNAP_TOLERANCE_PX * proj.units_per_px()
    }

    /// Run the tool at `cursor` (typed dimensions override it). `constrain`
    /// turns the typed values into constraints as well; a plain click keeps
    /// the geometry free.
    fn apply_tool_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        cursor: Vec2D,
        constrain: bool,
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let (cursor, tol) = self.landing(ctx, &feature, tool, cursor);
        if tool == "sketch.text" {
            return self.text_click(ctx, &feature, cursor, tol);
        }
        let params = self.click_params();
        let chain = matches!(tool, "sketch.line" | "sketch.polyline");
        if chain {
            self.follow_history(ctx, &feature.sketch);
        }
        let tab = &mut *self.tab;
        tab.dim_capture.sync(&tab.tool_state, Some(tool));
        let typed = self.tab.dim_capture.typed(ctx.document);
        if let Some(why) = self.tab.dim_capture.error(ctx.document) {
            ctx.log_warn(format!("Not used: {why}"));
        }
        // Typed values read the cursor as keeping the way it was going.
        let cursor = self
            .tab
            .dim_capture
            .steer(&self.tab.tool_state, &feature.sketch, cursor);
        let settings = step::StepSettings {
            tol,
            params,
            construction: self.construction_mode(),
            avoid_redundant: self.options.avoid_redundant_auto,
        };
        // The line tool's arc: let go after a drag from its end, or typed
        // while the drag is under way.
        let arc = tool == "sketch.line"
            && (std::mem::take(&mut self.tab.arc_click)
                || matches!(self.tab.tool_state, ToolState::PolylineFrom { .. }));
        self.note_draw_click(ctx, &feature, tool, cursor, &typed, constrain, &settings);
        if !arc
            && typed.is_empty()
            && self
                .open_corner_task(ctx, &feature, tool, cursor, &settings)
                .or_else(|| self.open_rect_task(ctx, &feature, tool, cursor, &settings))
                .is_some()
        {
            return InputResult::consumed();
        }
        let state_before = self.tab.tool_state.clone();
        let event = self
            .tab
            .draw_record
            .as_ref()
            .map_or(0, |r| r.events.len().saturating_sub(1));
        let outcome = if arc {
            // Recorded as the arc it is.
            if let Some(event) = self
                .tab
                .draw_record
                .as_mut()
                .and_then(|r| r.events.last_mut())
            {
                if !event.is_object() {
                    *event = serde_json::json!({"x": cursor.x, "y": cursor.y});
                }
                event["arc"] = serde_json::json!(true);
            }
            let tab = &mut *self.tab;
            step::line_arc_click(
                &mut tab.tool_state,
                &mut tab.dim_capture,
                &mut feature.sketch,
                cursor,
                &typed,
                constrain,
                &settings,
                &tab.selected,
            )
        } else {
            let tab = &mut *self.tab;
            step::click(
                &mut tab.tool_state,
                &mut tab.dim_capture,
                tool,
                &mut feature.sketch,
                cursor,
                &typed,
                constrain,
                &settings,
                &tab.selected,
            )
        };
        if outcome.skipped > 0 {
            ctx.log_info(format!(
                "Skipped {} redundant auto constraint(s)",
                outcome.skipped
            ));
        }
        let (effect, added) = (
            tools::ToolEffect {
                changed: outcome.changed,
                log: outcome.log,
            },
            outcome.added,
        );
        if (effect.changed || added > 0)
            && let Some(record) = self.tab.draw_record.as_mut()
        {
            record.changed = true;
        }
        if effect.changed {
            self.tab.dim_capture.clear_buffers();
        }
        if effect.changed || added > 0 {
            self.solve(ctx, &mut feature);
            if let Some(log) = effect.log {
                ctx.log_info(log);
            }
            let curve = feature.sketch.geometry.last().map(|g| g.id());
            let stored = self.store_sketch(ctx, feature);
            // Each segment is an undo step of its own, whose step the
            // gesture keeps: taking it back puts the gesture before it.
            if chain
                && effect.changed
                && stored
                && let Some(curve) = curve
            {
                let mut before = state_before;
                if tool == "sketch.line" {
                    tools::line_arc_end(&mut before);
                }
                self.tab.chain_links.push(chain::Link {
                    before,
                    curve,
                    entry: None,
                    event,
                });
                ctx.request(HostRequest::CloseToolGesture {
                    bench: core_document::WorkbenchId::from(KIND),
                });
            }
        } else if let Some(why) = effect.log {
            // A click that did nothing says why.
            ctx.log_warn(why);
        }
        // With Continue drawing off, a finished shape hands the pointer
        // back to the Select tool; a line is finished at its first segment.
        let finished = self.tab.tool_state.is_idle()
            || matches!(self.tab.tool_state, ToolState::LineFrom { chain: true, .. });
        if effect.changed && finished && !self.options.continue_drawing && is_draw_tool(tool) {
            self.tab.tool_state = ToolState::Idle;
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        }
        InputResult::consumed()
    }

    /// The Text tool's click: text from the point clicked, a frame's
    /// corner (its second one writes the text in it), or the curve
    /// clicked to write along.
    fn text_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
        cursor: Vec2D,
        tol: f64,
    ) -> InputResult {
        let mut spec = self.tab.text_draft.clone();
        let mut args = text_args(&spec, None);
        enum Where {
            Point(Vec2D),
            Frame(Vec2D, Vec2D),
            Path(Uuid, Vec2D),
        }
        let at = match &spec.layout {
            text::TextLayout::Point => {
                args["at"] = serde_json::json!([cursor.x, cursor.y]);
                Where::Point(cursor)
            }
            text::TextLayout::Frame { .. } => {
                let ToolState::TextFrame { corner } = self.tab.tool_state else {
                    self.tab.tool_state = ToolState::TextFrame { corner: cursor };
                    return InputResult::consumed();
                };
                self.tab.tool_state = ToolState::Idle;
                args["frame"] = serde_json::json!([[corner.x, corner.y], [cursor.x, cursor.y]]);
                Where::Frame(corner, cursor)
            }
            text::TextLayout::Path { .. } => {
                let Some((curve, at)) = snap::snap_to_curve(&feature.sketch, cursor, tol, &[])
                    .filter(|(c, _)| feature.sketch.get_geometry(*c).is_some())
                else {
                    ctx.log_warn("Text: click the curve to write along");
                    return InputResult::consumed();
                };
                args["path"] = serde_json::json!([curve.to_string()]);
                args["at"] = serde_json::json!([at.x, at.y]);
                Where::Path(curve, at)
            }
        };
        if let Err(why) = text::load_font(ctx.document, &mut spec) {
            ctx.log_warn(why);
            return InputResult::consumed();
        }
        match text::values_of(ctx.document, &spec.text) {
            Ok(values) => spec.values = values,
            Err(why) => {
                ctx.log_warn(why);
                return InputResult::consumed();
            }
        }
        if let Some(asset) = spec.font_asset {
            args["font_asset"] = serde_json::json!(asset.to_string());
        }
        let add = move |s: &mut sketch::Sketch, spec: &text::TextSpec| match at {
            Where::Point(at) => text::add(s, at, spec),
            Where::Frame(a, b) => text::add_in_frame(s, a, b, spec),
            Where::Path(curve, at) => text::add_on_path(s, &[curve], Some(at), spec),
        };
        let result = self.sketch_edit(ctx, "sketch.text", args, move |s| match add(s, &spec) {
            Ok(_) => tools::ToolEffect::changed(format!("Text \"{}\"", spec.text)),
            Err(why) => tools::ToolEffect::log(why),
        });
        crate::keep_text_formulas(ctx, self.tab.active_sketch_id, None);
        result
    }

    /// Note a click for the recording: the start of a new `sketch.draw`
    /// call when the tool starts a shape, else one more of its points.
    #[allow(clippy::too_many_arguments)]
    fn note_draw_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        typed: &[(ovp::FieldKind, f64)],
        constrain: bool,
        settings: &step::StepSettings,
    ) {
        let Some(sketch_id) = self.tab.active_sketch_id else {
            return;
        };
        // A trim stroke's clicks are one call, as a shape's are.
        let fresh = match &self.tab.draw_record {
            Some(r) => {
                r.tool != tool
                    || r.sketch != sketch_id
                    || (self.tab.tool_state.is_idle() && self.tab.trim_stroke.is_none())
            }
            None => true,
        };
        if fresh {
            self.flush_draw_record(ctx);
            let mut named = serde_json::Map::new();
            named.insert("tolerance".into(), serde_json::json!(settings.tol));
            let params = step::params_to_json(&settings.params);
            if !params.is_empty() {
                named.insert("params".into(), serde_json::Value::Object(params));
            }
            if settings.construction {
                named.insert("construction".into(), serde_json::json!(true));
            }
            if !settings.avoid_redundant {
                named.insert("avoid_redundant".into(), serde_json::json!(false));
            }
            if SELECTION_TOOLS.contains(&tool) && !self.tab.selected.is_empty() {
                let mut selected: Vec<Uuid> = self.tab.selected.iter().copied().collect();
                selected.sort();
                named.insert("selection".into(), ids_json(&selected));
            }
            self.tab.draw_record = Some(DrawRecord {
                sketch: sketch_id,
                tool: tool.to_string(),
                events: Vec::new(),
                settings: named,
                changed: false,
                elements_before: feature.sketch.geometry.iter().map(|g| g.id()).collect(),
                constraints_before: feature.sketch.constraints.iter().map(|c| c.id).collect(),
            });
        }
        let event = if typed.is_empty() {
            serde_json::json!([cursor.x, cursor.y])
        } else {
            let values: serde_json::Map<String, serde_json::Value> = typed
                .iter()
                .map(|(k, v)| (step::field_name(*k).to_string(), serde_json::json!(v)))
                .collect();
            serde_json::json!({"x": cursor.x, "y": cursor.y, "typed": values, "constrain": constrain})
        };
        if let Some(record) = self.tab.draw_record.as_mut() {
            record.events.push(event);
        }
    }

    /// Record the shape drawn so far as one `sketch.draw` call, with what
    /// it made, when it made anything.
    pub(crate) fn flush_draw_record(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(record) = self.tab.draw_record.take() else {
            return;
        };
        if !record.changed {
            return;
        }
        let Some(feature) = stored_sketch(ctx.document, record.sketch) else {
            return;
        };
        let elements: Vec<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(|g| g.id())
            .filter(|id| !record.elements_before.contains(id))
            .collect();
        let constraints: Vec<Uuid> = feature
            .sketch
            .constraints
            .iter()
            .map(|c| c.id)
            .filter(|id| !record.constraints_before.contains(id))
            .collect();
        let mut args = record.settings;
        args.insert(
            "sketch".into(),
            serde_json::json!(record.sketch.0.to_string()),
        );
        args.insert(
            "tool".into(),
            serde_json::json!(record.tool.trim_start_matches("sketch.")),
        );
        args.insert("points".into(), serde_json::Value::Array(record.events));
        ctx.record(
            "sketch.draw",
            args,
            serde_json::json!({
                "elements": ids_json(&elements),
                "constraints": ids_json(&constraints),
            }),
        );
    }

    pub(crate) fn handle_left_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        let Some(cursor) = Self::cursor_to_sketch(ctx, &plane, viewport_pos) else {
            return InputResult::ignored();
        };
        let tol = Self::snap_tolerance(ctx, &plane);
        self.tab.cursor = Some(cursor);
        // A new press ends whatever the last one armed.
        self.tab.dragging = None;
        self.tab.box_select = None;
        self.tab.line_press = None;

        // A picture's calibration takes its two points before anything
        // else takes a click.
        if let Some(calibrating) = self.tab.calibrating.as_mut()
            && calibrating.to.is_none()
        {
            match calibrating.from {
                None => calibrating.from = Some(cursor),
                Some(from) => {
                    calibrating.to = Some(cursor);
                    calibrating.length = glyphs::fmt_num((cursor - from).to_glam().length());
                }
            }
            return InputResult::consumed();
        }

        // The line tool: a press on the end it draws from, dragged, is an
        // arc tangent to what ends there; a press anywhere else clicks, and
        // dragged from a first point draws the line to where it is let go.
        if tool == Some("sketch.line") {
            let on_end = match self.tab.tool_state {
                ToolState::LineFrom {
                    from: snap::SnapTarget::Existing(from),
                    ..
                } => feature
                    .sketch
                    .point_position(from)
                    .is_some_and(|at| (at - cursor).to_glam().length() <= tol),
                _ => false,
            };
            let press = LinePress {
                at: viewport_pos,
                dragged: false,
                arcing: false,
            };
            if on_end {
                self.tab.line_press = Some(press);
                return InputResult::consumed();
            }
            let result = self.apply_tool_click(ctx, "sketch.line", cursor, false);
            if matches!(
                self.tab.tool_state,
                ToolState::LineFrom { chain: false, .. }
            ) {
                self.tab.line_press = Some(press);
            }
            return result;
        }

        match tool {
            // A press trims what it lands on and starts a stroke: dragging
            // on trims whatever the pointer crosses.
            Some("sketch.trim") => {
                self.tab.trim_stroke = None;
                self.flush_draw_record(ctx);
                let result = if tools::trim_preview(
                    &feature.sketch,
                    cursor,
                    tol,
                    self.tab.tool_params.cut_at_axes,
                )
                .is_some()
                {
                    self.apply_tool_click(ctx, "sketch.trim", cursor, false)
                } else {
                    InputResult::consumed()
                };
                self.tab.trim_stroke = Some(cursor);
                result
            }
            Some(t) if t != "sketch.select" => self.apply_tool_click(ctx, t, cursor, false),
            _ => {
                // Select mode. Constraint glyphs sit on top of geometry, so
                // they win the hit-test. Pressing on geometry selects it and
                // arms a drag: moving carries the element (and everything
                // sharing its points) to the cursor, releasing in place
                // leaves it selected. Empty space starts a box selection
                // (resolved on release).
                if let Some(hit) = self.glyph_hit(ctx, &feature, viewport_pos) {
                    return self.handle_glyph_press(ctx, &feature, hit, cursor);
                }
                let now = Instant::now();
                match snap::hit_test(&feature.sketch, cursor, tol) {
                    Some(id) => {
                        // A double click on a curve takes the chain it is
                        // part of, drawn and projected alike.
                        let double =
                            self.tab
                                .last_geometry_click
                                .take()
                                .is_some_and(|(last, t)| {
                                    last == id && now.duration_since(t) < DOUBLE_CLICK_WINDOW
                                });
                        if double
                            && !matches!(
                                feature.sketch.get_geometry(id),
                                Some(sketch::GeometryElement::Point(_)) | None
                            )
                        {
                            self.tab
                                .selected
                                .extend(connected_curves(&feature.sketch, id));
                            return InputResult::consumed();
                        }
                        self.tab.last_geometry_click = Some((id, now));
                        self.begin_drag(&feature.sketch, id, cursor);
                        InputResult::consumed()
                    }
                    None => {
                        // A double click on empty space turns the view
                        // square onto the sketch again.
                        if self
                            .tab
                            .last_empty_click
                            .take()
                            .is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK_WINDOW)
                        {
                            Self::view_sketch(ctx, &feature);
                        } else {
                            self.tab.last_empty_click = Some(now);
                        }
                        // Empty space: begin a box selection. The release
                        // decides between a real box (which adds what it
                        // covers) and a plain click (which clears).
                        self.tab.box_select = Some(BoxSelect {
                            anchor: cursor,
                            current: cursor,
                        });
                        InputResult::consumed()
                    }
                }
            }
        }
    }

    /// The constraint glyph under a viewport click, if any.
    fn glyph_hit(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        viewport_pos: (f32, f32),
    ) -> Option<GlyphHit> {
        // Hidden glyphs take no clicks.
        if self.options.constraints_hidden {
            return None;
        }
        let proj = SketchProjector::new(ctx, feature.plane);
        let glyphs = self.glyphs(ctx, &feature.sketch, &proj);
        glyphs::hit_test(
            &glyphs,
            [viewport_pos.0, viewport_pos.1],
            self.options.appearance.label_px(ctx.pixels_per_point),
        )
        .map(|g| GlyphHit {
            constraint: g.constraint,
            dimensional: g.dimensional,
            pos: g.pos,
        })
    }

    /// Click on a constraint glyph: select it (ctrl additive), start a
    /// label drag on dimension labels, open the value editor on
    /// double-click.
    fn handle_glyph_press(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        hit: GlyphHit,
        cursor: Vec2D,
    ) -> InputResult {
        let now = Instant::now();
        let double = self.tab.last_glyph_click.take().is_some_and(|(id, t)| {
            id == hit.constraint && now.duration_since(t) < DOUBLE_CLICK_WINDOW
        });
        self.tab.last_glyph_click = Some((hit.constraint, now));
        let constraint = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == hit.constraint);

        if double && hit.dimensional {
            if let Some(c) = constraint {
                // A dimension a formula sets opens on its formula.
                let formula = self.tab.active_sketch_id.and_then(|sketch| {
                    ctx.document
                        .feature_formula(sketch, &c.id.to_string())
                        .map(str::to_string)
                });
                self.tab.dim_draft = None;
                self.tab.dim_edit = Some(DimEdit::new(
                    c.id,
                    hit.pos,
                    formula.unwrap_or_else(|| {
                        sketch::dimension_value(&c.kind)
                            .map(glyphs::fmt_num)
                            .unwrap_or_default()
                    }),
                    c.driving,
                ));
            }
            self.tab.label_drag = None;
            return InputResult::consumed();
        }

        // A geometric constraint's double click names it, in its row of
        // the constraint list.
        if double && !hit.dimensional {
            self.tab.selected_constraints.insert(hit.constraint);
            self.tab.renaming_constraint = Some(hit.constraint);
            self.tab.label_drag = None;
            return InputResult::consumed();
        }

        // Selection accumulates: a second click on the same glyph takes it
        // back out, and empty space clears everything.
        if !self.tab.selected_constraints.remove(&hit.constraint) {
            self.tab.selected_constraints.insert(hit.constraint);
        }
        if hit.dimensional {
            let proj = SketchProjector::new(ctx, feature.plane);
            let base = constraint.map(|c| {
                // Capture the resolved center, including automatic collision
                // clearance, independently of where inside the pill we grabbed.
                proj.sketch_at(hit.pos)
                    .expect("picked label projects onto its sketch")
                    - glyphs::dim_anchor(&feature.sketch, &c.kind)
                        .expect("drawn dimension has an anchor")
            });
            self.tab.label_drag = Some(LabelDrag {
                constraint: hit.constraint,
                moved: false,
                grab: cursor,
                base: base.unwrap_or(Vec2D::new(0.0, 0.0)),
                current: None,
            });
        }
        InputResult::consumed()
    }

    pub(crate) fn handle_mouse_move(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        self.tab.cursor = Self::cursor_to_sketch(ctx, &plane, viewport_pos);
        if let Some(cursor) = self.tab.cursor {
            let tab = &mut *self.tab;
            tab.dim_capture
                .track(&tab.tool_state, &feature.sketch, cursor);
        }

        // A line tool press that moves far enough is a drag; from an end
        // something runs into, the drag draws a tangent arc.
        let tab = &mut *self.tab;
        if tool == Some("sketch.line")
            && let Some(press) = tab.line_press.as_mut()
            && !press.dragged
            && (viewport_pos.0 - press.at.0).hypot(viewport_pos.1 - press.at.1) > LINE_DRAG_PX
        {
            press.dragged = true;
            press.arcing = tools::line_arc_begin(&mut tab.tool_state, &feature.sketch);
        }

        // Dimension label drag: purely cosmetic, no solver run needed.
        let tol = Self::snap_tolerance(ctx, &plane);
        let tab = &mut *self.tab;
        if let Some(ld) = tab.label_drag.as_mut() {
            if let Some(cursor) = tab.cursor
                && (ld.moved || (cursor - ld.grab).to_glam().length() > tol)
            {
                ld.moved = true;
                ld.current = Some(ld.base + (cursor - ld.grab));
            }
            return InputResult::consumed();
        }
        // Constraint-aware drag: carry every point of the grabbed geometry
        // by the cursor delta and let the solver settle the rest: whatever
        // shares those points comes along, and the dimensions that are not
        // driving adjust. Consumed so the camera doesn't orbit underneath.
        let joins = self.options.auto_constraints && !self.tab.snap_off;
        let cursor = self.tab.cursor;
        let tab = &mut *self.tab;
        if let Some(drag) = tab.dragging.as_mut() {
            let Some(cursor) = cursor else {
                return InputResult::consumed();
            };
            let delta = cursor - drag.grab;
            // A dead zone the size of the snap tolerance: a click that
            // trembles selects instead of nudging the geometry.
            if !drag.moved && delta.to_glam().length() <= Self::snap_tolerance(ctx, &plane) {
                return InputResult::consumed();
            }
            drag.moved = true;
            drag.delta = delta;
            step::drag(&mut feature.sketch, &drag.points, delta);
            // The dragged points stay under the cursor and the rest gives
            // way; constraints hold during a drag whatever auto update says.
            let held: Vec<Uuid> = drag.points.iter().map(|(id, _)| *id).collect();
            // A point dragged alone rests on what it is over, lit, the
            // rest timed from when it got there.
            let over = match drag.points.as_slice() {
                [(point, _)] if joins && drag.elements == [*point] => {
                    step::drag_target(&feature.sketch, *point, cursor, tol)
                }
                _ => None,
            };
            drag.target = match (drag.target, over) {
                (Some((was, since)), Some(now)) if was == now => Some((was, since)),
                (_, Some(now)) => Some((now, Instant::now())),
                _ => None,
            };
            tab.hovered = over;
            tab.last_solve = Some(solver::solve_holding(&mut feature.sketch, &held));
            // A drag solves the whole sketch, edits waiting for a solve too.
            tab.unsolved_edits = false;
            tab.last_diagnosis = None;
            self.store_sketch(ctx, feature);
            return InputResult::consumed();
        }
        // A trim stroke: every curve the pointer crossed since the last
        // move is trimmed where it was crossed, first crossed first.
        if let Some(mut from) = self.tab.trim_stroke
            && tool == Some("sketch.trim")
        {
            let Some(to) = self.tab.cursor else {
                return InputResult::consumed();
            };
            // Each trim can leave new pieces in the way; the bound only
            // guards against a curve that never stops being crossed.
            for _ in 0..MAX_STROKE_TRIMS {
                let Some(sketch) = self.get_active_sketch(ctx) else {
                    break;
                };
                let Some(at) = tools::next_stroke_crossing(&sketch.sketch, from, to) else {
                    break;
                };
                self.apply_tool_click(ctx, "sketch.trim", at, false);
                from = at;
            }
            self.tab.trim_stroke = Some(to);
            return InputResult::consumed();
        }
        // Box selection in progress: track the moving corner. Consumed so
        // the camera doesn't move underneath the box.
        let tab = &mut *self.tab;
        if let Some(bs) = tab.box_select.as_mut() {
            if let Some(cursor) = tab.cursor {
                bs.current = cursor;
            }
            return InputResult::consumed();
        }
        if matches!(tool, None | Some("sketch.select")) {
            let tol = Self::snap_tolerance(ctx, &plane);
            self.tab.hovered = self
                .tab
                .cursor
                .and_then(|c| snap::hit_test(&feature.sketch, c, tol));
        } else {
            self.tab.hovered = None;
        }
        // Never consume moves: the camera still needs them for orbiting.
        InputResult::redraw_only()
    }

    pub(crate) fn handle_left_release(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        // A line tool drag lands: the arc, or the line, ends where it is let go.
        if let Some(press) = self.tab.line_press.take()
            && press.dragged
            && !self.tab.tool_state.is_idle()
            && let Some(cursor) = self.tab.cursor
        {
            self.tab.arc_click = press.arcing;
            return self.apply_tool_click(ctx, "sketch.line", cursor, false);
        }
        if self.tab.trim_stroke.take().is_some() {
            return InputResult::consumed();
        }
        if let Some(ld) = &self.tab.label_drag {
            // The label lands: one write for the whole drag.
            let landed = ld.current.is_some();
            let feature = self.get_active_sketch(ctx);
            self.tab.label_drag = None;
            if landed && let Some(feature) = feature {
                self.store_sketch_data(ctx, feature);
            }
            return InputResult::consumed();
        }
        if let Some(bs) = self.tab.box_select.take() {
            return self.finish_box_select(ctx, bs);
        }
        if let Some(drag) = self.tab.dragging.take() {
            // A press+release without movement is a click. The press put a
            // new element in the selection already, so only one that was
            // there before comes back out.
            if !drag.moved && drag.was_selected {
                self.tab.selected.remove(&drag.hit);
                // Where several lie on one another, a click on the selected
                // one takes the next instead: a line over a guide, then the
                // guide, then the line again.
                if let Some(feature) = self.get_active_sketch(ctx) {
                    let tol = Self::snap_tolerance(ctx, &feature.plane);
                    let under = snap::hit_candidates(&feature.sketch, drag.grab, tol);
                    if under.len() > 1
                        && let Some(at) = under.iter().position(|id| *id == drag.hit)
                    {
                        self.tab.selected.insert(under[(at + 1) % under.len()]);
                    }
                }
            }
            if drag.moved
                && let Some(sketch_id) = self.tab.active_sketch_id
            {
                ctx.record(
                    "sketch.drag",
                    commands::args(serde_json::json!({
                        "sketch": sketch_id.0.to_string(),
                        "items": ids_json(&drag.elements),
                        "by": [drag.delta.x, drag.delta.y],
                    })),
                    serde_json::Value::Null,
                );
                self.join_dropped(ctx, sketch_id, &drag);
            }
            return InputResult::consumed();
        }
        InputResult::ignored()
    }

    /// A point dragged alone and let go on what it rested over for the
    /// delay is joined to it, recorded after the drag as the constraint
    /// tool's call.
    fn join_dropped(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        sketch_id: FeatureId,
        drag: &DragState,
    ) {
        let Some((target, since)) = drag.target else {
            return;
        };
        let delay = std::time::Duration::from_secs_f32(
            self.options.drag_constraint_delay_ms.max(0.0) / 1000.0,
        );
        let ([point], true) = (drag.elements.as_slice(), self.options.auto_constraints) else {
            return;
        };
        if since.elapsed() < delay {
            return;
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some((kind, made)) = step::join_dragged(
            &mut feature.sketch,
            *point,
            target,
            self.options.avoid_redundant_auto,
        ) else {
            return;
        };
        ctx.record(
            "sketch.constrain",
            commands::args(serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "kind": kind,
                "items": ids_json(&[*point, target]),
            })),
            serde_json::json!(made),
        );
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
    }

    /// Resolve a released box selection. A drag beyond the snap tolerance
    /// adds every element fully inside the rectangle to the selection
    /// (selection accumulates), a curve without the points that define it,
    /// so a boxed line is a line to the constraint tools; anything shorter
    /// is a click on empty space, which clears it.
    fn finish_box_select(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        bs: BoxSelect,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::consumed();
        };
        let tol = Self::snap_tolerance(ctx, &feature.plane);
        // A press and release in the same spot is a click on empty space:
        // the one gesture that clears the selection.
        if (bs.current - bs.anchor).to_glam().length() <= tol {
            self.tab.selected.clear();
            self.tab.selected_constraints.clear();
            return InputResult::consumed();
        }
        let min = Vec2D::new(bs.anchor.x.min(bs.current.x), bs.anchor.y.min(bs.current.y));
        let max = Vec2D::new(bs.anchor.x.max(bs.current.x), bs.anchor.y.max(bs.current.y));
        // Drawn left to right the box takes what it holds whole; right to
        // left, whatever it touches.
        let proj = SketchProjector::new(ctx, feature.plane);
        let crossing = overlay::is_crossing(&proj, bs.anchor, bs.current);
        let boxed: Vec<&sketch::GeometryElement> = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| !feature.sketch.derived.contains(&g.id()))
            .filter(|g| {
                if crossing {
                    element_touches(&feature.sketch, g, min, max)
                } else {
                    element_fully_inside(&feature.sketch, g, min, max)
                }
            })
            .collect();
        let owned: HashSet<Uuid> = boxed
            .iter()
            .filter(|g| !matches!(g, sketch::GeometryElement::Point(_)))
            .flat_map(|g| Sketch::curve_point_ids(g))
            .collect();
        for geom in boxed {
            if !owned.contains(&geom.id()) {
                self.tab.selected.insert(geom.id());
            }
        }
        InputResult::consumed()
    }

    /// Right-click / Enter: ends a line chain, completes an in-progress
    /// B-spline; anything else stays with the camera (right-drag pans).
    pub(crate) fn handle_finish_gesture(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        match &self.tab.tool_state {
            ToolState::LineFrom { chain: true, .. } | ToolState::PolylineFrom { .. } => {
                self.tab.tool_state = ToolState::Idle;
                InputResult::consumed()
            }
            ToolState::BSplineDraw { .. } => {
                let Some(mut feature) = self.get_active_sketch(ctx) else {
                    self.tab.tool_state = ToolState::Idle;
                    return InputResult::consumed();
                };
                let tab = &mut *self.tab;
                let effect = tools::finish_click_sequence(
                    &mut tab.tool_state,
                    &mut feature.sketch,
                    &tab.tool_params,
                );
                if let Some(record) = self.tab.draw_record.as_mut() {
                    record.events.push(serde_json::json!("finish"));
                    record.changed |= effect.changed;
                }
                if effect.changed {
                    self.solve(ctx, &mut feature);
                    if let Some(log) = effect.log {
                        ctx.log_info(log);
                    }
                    self.store_sketch(ctx, feature);
                }
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    /// A right click that stayed put (a pan moves the camera instead) with
    /// no gesture in flight hands the pointer back to the Select tool.
    pub(crate) fn handle_right_release(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        /// How far the pointer may travel and still count as a click.
        const CLICK_SLOP_PX: f32 = 4.0;
        let Some(press) = self.tab.right_press.take() else {
            return InputResult::ignored();
        };
        let travelled = (viewport_pos.0 - press.0).hypot(viewport_pos.1 - press.1);
        if travelled <= CLICK_SLOP_PX && self.tab.tool_state.is_idle() {
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        }
        // Never consumed: the camera still has a pan to finish.
        InputResult::ignored()
    }

    /// Put every point `drag` moved back where its press found it.
    fn put_back_drag(&mut self, ctx: &mut WorkbenchRuntimeContext, drag: DragState) {
        if drag.moved
            && let Some(mut feature) = self.get_active_sketch(ctx)
        {
            for (id, original) in drag.points {
                if let Some(sketch::GeometryElement::Point(p)) = feature.sketch.get_geometry_mut(id)
                {
                    p.position = original;
                }
            }
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
    }

    /// Let go of whatever the pointer holds, its release never to come: a
    /// dragged point goes back as Escape puts it, a label, a box, a line
    /// tool's press, a trim stroke and a right press are dropped. A shape
    /// half drawn, the tool and the selection stay.
    pub(crate) fn cancel_pointer(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.tab.label_drag = None;
        self.tab.box_select = None;
        self.tab.line_press = None;
        self.tab.trim_stroke = None;
        self.tab.right_press = None;
        if let Some(drag) = self.tab.dragging.take() {
            self.put_back_drag(ctx, drag);
        }
    }

    fn handle_escape(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
    ) -> InputResult {
        // Escape is an open dimension card's Cancel.
        if self.tab.dim_edit.is_some() {
            self.cancel_dim_edit(ctx);
            return InputResult::consumed();
        }
        if self.tab.calibrating.take().is_some() {
            return InputResult::consumed();
        }
        if self.tab.label_drag.take().is_some() {
            // Nothing is written until the release: the label stays put.
            return InputResult::consumed();
        }
        if self.tab.box_select.take().is_some() {
            // Cancel the box; the selection it would have replaced stays.
            return InputResult::consumed();
        }
        if let Some(drag) = self.tab.dragging.take() {
            self.put_back_drag(ctx, drag);
            return InputResult::consumed();
        }
        if self.tab.pending_creation.is_some() {
            self.tab.pending_creation = None;
            return InputResult::consumed();
        }
        if self.tab.tool_state.is_idle() && self.tab.wall_check.take().is_some() {
            return InputResult::consumed();
        }
        // A half-drawn shape first, then the tool itself (back to Select),
        // then the selection; the sketch stays open whatever is left.
        if !self.tab.tool_state.is_idle() {
            self.tab.tool_state = ToolState::Idle;
            ctx.log_info("Sketch: cancelled current tool operation");
        } else if tool.is_some_and(|t| base_tool_id(t) != "sketch.select") {
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        } else if !self.tab.selected.is_empty() || !self.tab.selected_constraints.is_empty() {
            self.tab.selected.clear();
            self.tab.selected_constraints.clear();
        }
        InputResult::consumed()
    }

    /// Enter with typed on-view parameters: commit the pending click at the
    /// derived position without a mouse click.
    fn commit_typed_enter(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
    ) -> InputResult {
        let Some(tool) = tool.filter(|t| *t != "sketch.select") else {
            return InputResult::ignored();
        };
        // A value that does not work out is said, and kept to be mended.
        if let Some(why) = self.tab.dim_capture.error(ctx.document) {
            ctx.log_warn(why);
            return InputResult::consumed();
        }
        // A press held through the typing lands here: its release is no
        // second click.
        self.tab.line_press = None;
        // Anchor the derived position on the last known cursor; the
        // override falls back to the +x direction when it is degenerate.
        let cursor = self.tab.cursor.unwrap_or(Vec2D::new(0.0, 0.0));
        self.apply_tool_click(ctx, tool, cursor, true)
    }

    /// A keyboard action registered in `configure`, by its key.
    pub(crate) fn handle_action(&mut self, id: &str) -> InputResult {
        match id {
            POLYLINE_ARC_ACTION if tools::toggle_polyline_arc(&mut self.tab.tool_state) => {
                let tab = &mut *self.tab;
                if let (Some(record), ToolState::PolylineFrom { segment, .. }) =
                    (tab.draw_record.as_mut(), &tab.tool_state)
                {
                    record.events.push(serde_json::json!(segment.word()));
                }
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    /// Consolidated key handling: on-view parameter capture first (typing
    /// digits into the focused dimension field), then the global keys.
    /// Text typed or pasted while a drawing tool takes values: into the
    /// focused field.
    pub(crate) fn handle_text(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        text: &str,
    ) -> InputResult {
        let tab = &mut *self.tab;
        tab.dim_capture.sync(&tab.tool_state, tool);
        if !self.tab.dim_capture.has_typed_input()
            && let (Some(cursor), Some(feature)) = (self.tab.cursor, self.get_active_sketch(ctx))
        {
            let tab = &mut *self.tab;
            tab.dim_capture
                .track(&tab.tool_state, &feature.sketch, cursor);
        }
        if self.tab.dim_capture.handle_text(text) {
            InputResult::consumed()
        } else {
            InputResult::ignored()
        }
    }

    pub(crate) fn handle_key_press(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        key: KeyCode,
    ) -> InputResult {
        // A key between two clicks makes them two clicks, not a double.
        self.tab.last_geometry_click = None;
        self.tab.last_empty_click = None;
        let tab = &mut *self.tab;
        tab.dim_capture.sync(&tab.tool_state, tool);
        // Backspace takes back the segment just drawn, while its undo step
        // is the newest.
        if key == KeyCode::Backspace
            && !self.tab.dim_capture.has_typed_input()
            && chain::is_chain(&self.tab.tool_state)
            && let Some(entry) = self.tab.chain_links.undoable(ctx.undo_entry)
        {
            ctx.request(HostRequest::UndoToolGesture {
                bench: core_document::WorkbenchId::from(KIND),
                entry,
            });
            return InputResult::consumed();
        }
        if self.tab.dim_capture.is_active() {
            match key {
                KeyCode::Enter if self.tab.dim_capture.has_typed_input() => {
                    return self.commit_typed_enter(ctx, tool);
                }
                KeyCode::Escape if self.tab.dim_capture.has_typed_input() => {
                    self.tab.dim_capture.clear_buffers();
                    return InputResult::consumed();
                }
                _ => {
                    if self.tab.dim_capture.handle_key(key) {
                        return InputResult::consumed();
                    }
                }
            }
        }
        match key {
            KeyCode::Escape => self.handle_escape(ctx, tool),
            KeyCode::Enter => self.handle_finish_gesture(ctx),
            // A shape half drawn: the keys belong to it, not the selection.
            KeyCode::Delete | KeyCode::Backspace if !self.tab.tool_state.is_idle() => {
                InputResult::consumed()
            }
            KeyCode::Delete | KeyCode::Backspace => self.delete_selected(ctx),
            KeyCode::U if matches!(tool, Some("sketch.trim" | "sketch.extend")) => {
                self.tab.tool_params.cut_at_axes = !self.tab.tool_params.cut_at_axes;
                ctx.log_info(if self.tab.tool_params.cut_at_axes {
                    "The axes cut"
                } else {
                    "The axes do not cut"
                });
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    /// The spacing the grid's decades are counted from: the size chosen,
    /// or, following the zoom, a unit, so its lines lie at the powers of
    /// ten as the scene's do.
    pub(crate) fn grid_base(&self) -> f64 {
        if self.options.grid_auto {
            1.0
        } else {
            self.options.grid_size.max(1e-3)
        }
    }

    /// Where a drawing click lands with the grid snapping on: the nearest
    /// point of the finest lines drawn clearly.
    fn grid_snapped(
        &self,
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
        cursor: Vec2D,
    ) -> Vec2D {
        if !(self.options.grid_on && self.options.grid_snap) {
            return cursor;
        }
        let units_per_px = SketchProjector::new(ctx, *plane).units_per_px();
        let step = core_document::grid::clear_step(self.grid_base(), units_per_px);
        Vec2D::new(
            (cursor.x / step).round() * step,
            (cursor.y / step).round() * step,
        )
    }
}

/// The tools whose clicks snap to a reference picture's edges.
const EDGE_SNAP_TOOLS: [&str; 6] = [
    "sketch.point",
    "sketch.line",
    "sketch.polyline",
    "sketch.arc3",
    "sketch.circle3",
    "sketch.bspline",
];

/// The loupe's side, logical pixels, and how much it magnifies.
const LOUPE_PX: f64 = 160.0;

const LOUPE_ZOOM: f64 = 4.0;

/// A constraint glyph resolved under a click.
struct GlyphHit {
    constraint: Uuid,
    dimensional: bool,
    pos: [f32; 2],
}

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// How far, in pixels, a press moves before it is a drag.
const LINE_DRAG_PX: f32 = 5.0;
