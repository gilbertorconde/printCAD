//! The sketcher as the host sees it: its `Workbench` impl and the method
//! groups of the trait (features, tools, panels, builds, parameters,
//! output, commands and the egui panels), each answered through the
//! sketcher's own state and code paths.

use super::*;
use std::borrow::Cow;

impl Workbench for SketchWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            crate::KIND,
            "Sketch",
            "2D sketching environment with constraints and profiles.",
        )
        .icon("workbench-sketcher")
        .feature_kinds([crate::KIND])
        .radial([
            "sketch.line",
            "sketch.polyline",
            "sketch.arc",
            "sketch.circle",
            "sketch.rect",
            "sketch.trim",
            "sketch.constrain.dimension",
            "sketch.constrain.coincident",
        ])
        .modal()
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        self.frame.clear();
        Some(&mut self.tab)
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        self.register_tools(context);
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench activated");
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench deactivated");
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        // A shape is recorded once it is done: its tool back at rest, or
        // another tool picked.
        let shape_done = self.tab.draw_record.as_ref().is_some_and(|r| {
            self.tab.tool_state.is_idle() || self.tab.last_tool.as_deref() != Some(r.tool.as_str())
        });
        if shape_done {
            self.flush_draw_record(ctx);
        }
        // The picker opens as the bench does, not at the next input.
        self.take_attach_request(ctx);
        self.sync_active_sketch_from_ctx(ctx);
        if self.tab.active_sketch_id.is_some()
            && self.tab.external_refreshed != self.tab.active_sketch_id
        {
            self.tab.external_refreshed = self.tab.active_sketch_id;
            self.refresh_external(ctx);
        }
        if self.tab.last_tool.as_deref() == Some("sketch.external") {
            if self.tab.external_intersection {
                self.take_intersection_picks(ctx);
            } else {
                self.take_external_picks(ctx);
            }
        }
        if self.tab.active_sketch_id.is_none() {
            self.tab.armed = None;
        }
        self.take_armed_picks(ctx);
        self.drop_stale_wall_check(ctx);
        if !chain::is_chain(&self.tab.tool_state) {
            self.tab.chain_links.clear();
        }
        let seq = ctx.document.mutation_seq();
        let from_outside = self
            .tab
            .own_seq
            .swap(seq, std::sync::atomic::Ordering::Relaxed)
            != seq;
        self.tab.selection_shape = match self.frame_sketch(ctx) {
            Some(feature) => {
                // What was selected may be gone, by an edit here that took
                // more with it (an element's constraints) or one from outside.
                self.tab.selected.retain(|id| {
                    feature.sketch.get_geometry(*id).is_some()
                        || sketch::Reference::of(*id).is_some()
                });
                self.tab
                    .selected_constraints
                    .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
                if from_outside {
                    // A segment taken back puts the gesture before it.
                    self.follow_chain(&feature.sketch);
                    // The verdict is of a sketch that is not there any more.
                    self.tab.last_solve = None;
                    self.tab.last_diagnosis = None;
                    // An undo may bring back geometry that meets its
                    // constraints or geometry that waits for a solve.
                    self.tab.unsolved_edits =
                        !self.options.auto_update && !solver::holds(&feature.sketch);
                }
                self.tab.section_view = feature.shows_section(self.options.section_on_open);
                let selected: Vec<Uuid> = self.tab.selected.iter().copied().collect();
                self.tab.internal_target =
                    !internal::curves_of(&feature.sketch, &selected).is_empty();
                constrain::SelectionShape::picked_in(
                    &feature.sketch,
                    &self.tab.selected,
                    self.tab.selected.in_order(),
                )
            }
            None => constrain::SelectionShape::default(),
        };
        if self.tab.armable.0.as_ref() != Some(&self.tab.selection_shape) {
            let armable = std::iter::once("dimension")
                .chain(constrain::TOOLS.iter().copied())
                .filter(|which| model_picks::completable(which, &self.tab.selection_shape))
                .collect();
            self.tab.armable = (Some(self.tab.selection_shape.clone()), armable);
        }
    }

    fn shortcuts_changed(&mut self, keys: &HashMap<String, Vec<core_document::Chord>>) {
        self.action_keys = keys
            .iter()
            .filter_map(|(id, chords)| Some((id.clone(), chords.first()?.to_string())))
            .collect();
    }

    /// The edited sketch's reference pictures, laid on its plane under
    /// its lines.
    /// Edges are being found in a reference picture.
    fn busy(&self) -> bool {
        self.tab.picture_edges.running()
    }
}

impl BenchFeatures for SketchWorkbench {
    fn feature_warning(
        &self,
        document: &core_document::Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
        id: FeatureId,
    ) -> Option<String> {
        let lost = self.lost_external(document, kernel, id).len();
        (lost > 0).then(|| {
            format!(
                "{lost} projected element(s) lost: the edge, face, element or datum they came \
                 from is gone. They stay where they were; edit the sketch to remove or replace \
                 them."
            )
        })
    }

    fn feature_info(&self, node: &core_document::FeatureNode) -> FeatureInfo {
        if let Some(generator) = generator::generator_of(node) {
            return FeatureInfo {
                icon: generator.icon(),
                kind_label: format!("{} profile", generator.label()),
                family_label: "Sketch".to_string(),
                builds_solid: false,
            };
        }
        FeatureInfo {
            icon: "tree-sketch",
            kind_label: "Sketch".to_string(),
            family_label: "Sketch".to_string(),
            builds_solid: false,
        }
    }

    fn passive_geometry(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
    ) -> Option<core_document::PassiveGeometry> {
        // As its formulas leave it, solved.
        let feature = document.read_feature::<SketchFeature>(id)?;
        let revision = document.values_revision(id)?;
        // Every region its curves close, shaded on its own.
        let regions = if self.options.shade_regions {
            self.tab.closed_parts.passive_regions(
                id,
                revision,
                &feature,
                REGION_LIFT_MM,
                self.options.appearance.region_opacity,
            )
        } else {
            Vec::new()
        };
        Some(core_document::PassiveGeometry {
            mesh: render::sketch_to_lines(&feature.sketch, &feature.plane),
            revision,
            tint: core_document::PassiveTint::Plain,
            regions,
        })
    }

    fn pick_part(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
        pick: &core_document::ViewportPick,
        tolerance_px: f32,
    ) -> Option<core_document::PartPick> {
        let feature = document.read_feature::<SketchFeature>(id)?;
        self.tab.closed_parts.pick(
            id,
            document.values_revision(id)?,
            &feature,
            pick,
            tolerance_px,
            self.options.shade_regions,
        )
    }

    fn part_lines(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        part: &core_document::FeaturePart,
    ) -> Option<kernel_api::TriMesh> {
        let core_document::FeaturePart::Edge(curve) = part else {
            return None;
        };
        let feature = document.feature_as::<SketchFeature>(id)?;
        render::curve_lines(&feature.sketch, &feature.plane, curve.parse().ok()?)
    }

    /// Its chains of curves (construction left out, as a profile leaves
    /// it) and its points that are no curve's end or centre.
    fn feature_paths(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
    ) -> Option<core_document::FeaturePaths> {
        let feature = document.feature_as::<SketchFeature>(id)?;
        let sketch = &feature.sketch;
        let chains = profile::extract_chains(sketch).unwrap_or_default();
        let held: HashSet<Uuid> = sketch
            .geometry
            .iter()
            .flat_map(Sketch::curve_point_ids)
            .collect();
        let points: Vec<[f64; 2]> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) if !held.contains(&p.id) => {
                    Some([p.position.x, p.position.y])
                }
                _ => None,
            })
            .collect();
        if chains.is_empty() && points.is_empty() {
            return None;
        }
        Some(core_document::FeaturePaths {
            plane: profile::plane_of(&feature.plane),
            chains,
            points,
        })
    }

    /// The distance in pixels from the cursor to the sketch's nearest
    /// curve: the cursor unprojected onto the sketch plane, the distance
    /// measured in sketch units, then scaled by the pixels one unit spans
    /// at the sketch origin so the answer is zoom-independent.
    fn note_spot(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        item: Option<Uuid>,
        at: [f32; 3],
        label: [f32; 3],
    ) -> Option<core_document::SketchNoteSpot> {
        let feature = document.read_feature::<SketchFeature>(id)?;
        Some(crate::notes::spot(&feature, item, at, label))
    }

    fn note_pick(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        pick: &core_document::ViewportPick,
        tolerance_px: f32,
        plane_points: bool,
    ) -> Option<core_document::SketchNotePick> {
        let feature = document.read_feature::<SketchFeature>(id)?;
        let hovered = (self.tab.active_sketch_id == Some(id))
            .then(|| self.tab.hovered_constraint.or(self.tab.hovered))
            .flatten();
        crate::notes::pick(&feature, pick, tolerance_px, plane_points, hovered)
    }

    fn pick_feature(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
        pick: &core_document::ViewportPick,
    ) -> Option<f32> {
        let feature = document.feature_as::<SketchFeature>(id)?;
        let (_, pos, px_per_unit) = closed_parts::cursor_in_sketch(&feature, pick)?;
        snap::nearest_curve_distance(&feature.sketch, pos).map(|units| narrow(units) * px_per_unit)
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.tab.active_sketch_id
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        // What a click left waiting is written as it stands.
        self.store_corner(ctx, false);
        self.discard_corner();
        self.flush_draw_record(ctx);
        if self.tab.active_sketch_id.is_some() {
            self.tab.active_sketch_id = None;
            self.tab.session_start = None;
            self.tab.sketch_picker = None;
            self.clear_interaction_state();
            // Deselect the feature: with it still active the next input
            // event would immediately re-enter editing via
            // `sync_active_sketch_from_ctx`.
            ctx.active_document_object = None;
            ctx.log_info("Exited sketch editing mode");
        } else {
            ctx.log_warn("Not in sketch editing mode");
        }
    }
}

impl BenchTools for SketchWorkbench {
    fn tool_history(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        reply: core_document::ToolHistoryReply,
    ) {
        match reply {
            core_document::ToolHistoryReply::Closed(entry) => self.tab.chain_links.closed(entry),
            core_document::ToolHistoryReply::Undone { .. } => {
                if let Some(feature) = self.get_active_sketch(ctx) {
                    self.follow_history(ctx, &feature.sketch);
                }
            }
        }
    }

    fn locks_view_to_plane(&self) -> bool {
        true
    }

    fn shows_origin_planes(&self) -> bool {
        self.tab.pending_creation.is_some() && self.tab.active_sketch_id.is_none()
    }

    fn takes_numeric_input(&self) -> bool {
        self.tab.active_sketch_id.is_some()
            && !ovp::fields_for(&self.tab.tool_state, self.tab.last_tool.as_deref()).is_empty()
    }

    fn takes_text_input(&self) -> bool {
        self.tab.active_sketch_id.is_some() && self.tab.dim_capture.takes_text()
    }

    fn cancel_pointer_gesture(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.cancel_pointer(ctx);
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.handle_input(event, active_tool, ctx)
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        if self.editing_generated(ctx) {
            return tool_id == "sketch.finish";
        }
        let editing = self.tab.active_sketch_id.is_some();
        if let Some(rest) = tool_id.strip_prefix("sketch.constrain.") {
            let which = tool_variant(tool_id).unwrap_or(rest);
            // A tool the selection does not fit yet arms, to take more picks.
            let armable = self.tab.armable.1.contains(which);
            if which == "dimension" {
                return editing && (dimension_for(&self.tab.selection_shape).is_some() || armable);
            }
            return editing && (constrain::fits(which, &self.tab.selection_shape) || armable);
        }
        match tool_id {
            "sketch.create" => ctx.selected_body_id.is_some() || ctx.active_object_plane.is_some(),
            "sketch.edit" => {
                !editing
                    && ctx
                        .active_document_object
                        .is_some_and(|id| self.is_sketch_feature(ctx, id))
            }
            "sketch.attach" => editing && ctx.selected_face.is_some(),
            "sketch.fit_scan" => editing && !self.scan_guides(ctx).is_empty(),
            "sketch.design_values" => editing,
            "sketch.fit_edge" => {
                editing
                    && self
                        .frame_sketch(ctx)
                        .is_some_and(|f| !f.sketch.images.is_empty())
            }
            "sketch.save_dxf" | "sketch.save_svg" | "sketch.save_csv" => editing,
            "sketch.array" => editing && !self.tab.selected.is_empty(),
            "sketch.bspline_convert" => {
                editing
                    && self.tab.selection_shape.all.len() > self.tab.selection_shape.points.len()
            }
            "sketch.bspline_degree_up" | "sketch.bspline_degree_down" => {
                editing && !self.tab.selection_shape.others.is_empty()
            }
            "sketch.internal_geometry" => editing && self.tab.internal_target,
            "sketch.remove_axis_alignment" => editing && !self.tab.selection_shape.lines.is_empty(),
            "sketch.join" => {
                editing
                    && self.tab.selection_shape.all.len() - self.tab.selection_shape.points.len()
                        >= 2
            }
            "sketch.toggle_driving" | "sketch.toggle_active" | "sketch.park_constraints" => {
                editing && !self.tab.selected_constraints.is_empty()
            }
            "sketch.select_constraints" => editing && !self.tab.selected.is_empty(),
            "sketch.select_elements" => editing && !self.tab.selected_constraints.is_empty(),
            "sketch.select_conflicting" => {
                editing
                    && self
                        .tab
                        .last_diagnosis
                        .as_ref()
                        .is_some_and(|d| !d.conflicting.is_empty())
            }
            "sketch.select_redundant" => {
                editing
                    && self
                        .tab
                        .last_diagnosis
                        .as_ref()
                        .is_some_and(|d| !d.redundant.is_empty())
            }
            _ => editing,
        }
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        match tool_id {
            _ if self.armed_as(tool_id) => true,
            "sketch.construction" => self.construction_mode(),
            "sketch.snap" => !self.tab.snap_off,
            "sketch.show_constraints" => self.options.constraints_hidden,
            "sketch.grid" => self.options.grid_on,
            "sketch.rendering_order" => self.options.construction_on_top,
            "sketch.bspline_comb" => self.options.spline_comb,
            "sketch.bspline_knots" => self.options.spline_knots,
            "sketch.bspline_polygon" => self.options.spline_polygon,
            "sketch.bspline_degree_label" => self.options.spline_degree,
            "sketch.bspline_multiplicity" => self.options.spline_multiplicity,
            "sketch.bspline_weights" => self.options.spline_weights,
            "sketch.parked_layer" => self.tab.parked_layer,
            "sketch.section_view" => self.tab.section_view,
            "sketch.carbon_copy" | "sketch.merge" => {
                self.tab.sketch_picker.as_ref().is_some_and(|p| {
                    (p.mode == SketchPickerMode::Merge) == (tool_id == "sketch.merge")
                })
            }
            _ => false,
        }
    }

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            meshes: self.overlay_meshes(ctx, ctx.active_document_object),
            overlays: self.screen_overlays(ctx, ctx.active_document_object),
            images: self.screen_images(ctx, ctx.active_document_object),
            marks: self.screen_marks(ctx, ctx.active_document_object),
            labels: self.screen_labels(ctx, ctx.active_document_object),
            hud: self.viewport_hud(ctx),
            status: self.status_items(ctx),
            clip_plane: self.clip_plane(ctx),
            view_grid: self.view_grid(ctx),
            ..Default::default()
        }
    }
}

impl BenchPanels for SketchWorkbench {
    fn floating_card(&mut self, ctx: &WorkbenchRuntimeContext) -> Option<bench_api::FloatingCard> {
        self.dimension_card(ctx)
    }

    fn card_event(&mut self, ctx: &mut WorkbenchRuntimeContext, event: &bench_api::PanelEvent) {
        self.dimension_event(ctx, event);
    }

    fn settings_json(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.options).ok()
    }

    fn apply_settings_json(&mut self, value: &serde_json::Value) {
        if let Ok(options) = serde_json::from_value(value.clone()) {
            self.options = options;
        }
    }

    fn menu_items(&self, scope: &MenuScope, _document: &core_document::Document) -> Vec<MenuItem> {
        match scope {
            MenuScope::StartPage => vec![
                MenuItem::new("sketch.start_blank", "Empty sketch")
                    .icon("sketch-new")
                    .hint("2D on XY plane"),
            ],
            _ => Vec::new(),
        }
    }

    /// `sketch.start_blank`: a sketch on the XY plane of the selected body,
    /// open for editing. The Edit menu's clipboard entries act on the
    /// selection of the sketch under edit.
    fn on_command(
        &mut self,
        id: &str,
        scope: &MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        match (scope, id) {
            (MenuScope::StartPage, "sketch.start_blank") => {
                let body = ctx.selected_body_id.map(BodyId);
                self.create_sketch_on_plane(
                    ctx,
                    body,
                    SketchPlane::default(),
                    NewSketchOn::default(),
                );
                true
            }
            (MenuScope::EditMenu, "edit.copy") => self.clipboard_copy(ctx, false),
            (MenuScope::EditMenu, "edit.cut") => self.clipboard_copy(ctx, true),
            (MenuScope::EditMenu, "edit.paste") => self.clipboard_paste(ctx),
            (MenuScope::Popup(tag), _) => self.node_command(ctx, id, tag),
            _ => false,
        }
    }

    fn has_settings(&self) -> bool {
        true
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        if self.tab.pending_creation.is_some() {
            return Some(TaskInfo {
                title: "New sketch".to_string(),
                icon: "sketch-new",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            });
        }
        let id = self.tab.active_sketch_id?;
        let name = ctx.document.get_feature_meta(id)?.name.clone();
        let feature = self.frame_displayed(ctx);
        Some(TaskInfo {
            title: name,
            subtitle: feature
                .as_ref()
                .map(|f| crate::style::placement_label(f))
                .unwrap_or_default(),
            status: feature.map(|f| self.task_status(&f)),
            icon: "sketch-edit",
            // Cancel puts the sketch back as the session found it.
            confirmable: true,
            // Every line drawn or constraint added is a step of its own.
            stepwise: true,
        })
    }
}

impl BenchBuild for SketchWorkbench {
    fn settle(&self, _node: &core_document::FeatureNode, values: &mut serde_json::Value) {
        params::settle(values);
    }

    /// A sketch drawn on a datum sits on the datum's plane as the datum is
    /// now, and its geometry from other sketches and datums stands where
    /// their working data places it.
    fn derive(
        &self,
        node: &core_document::FeatureNode,
        values: &mut Cow<'_, serde_json::Value>,
        values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
        document: &core_document::Document,
    ) -> bool {
        let datum_plane = |feature: &SketchFeature| {
            (feature.support.as_ref())
                .and_then(|s| values_of(s.datum).and_then(|data| s.plane_from(&data)))
                .filter(|plane| feature.plane != *plane || feature.sketch.plane != *plane)
        };
        // Nothing to follow: no copy of the sketch.
        let still = |feature: &SketchFeature| {
            datum_plane(feature).is_none()
                && feature.sketch.external.is_empty()
                && feature.sketch.texts.iter().all(|b| !b.glyphs.is_empty())
        };
        if borrowed_holds(node, values, still) {
            return false;
        }
        let Some(mut feature) = read_values(node, values) else {
            return false;
        };
        let mut moved = false;
        if let Some(plane) = datum_plane(&feature) {
            feature.plane = plane;
            feature.sketch.plane = plane;
            moved = true;
        }
        moved |= follow_references(&mut feature, node.id, values_of, document);
        // Text an older file stored as outlines, in a font the document
        // keeps, reads its letters from it.
        moved |= crate::text::upgrade_stored(&mut feature.sketch, document);
        if moved {
            *values = Cow::Owned(feature.to_json());
        }
        moved
    }

    /// A sketch's geometry projected from solids' edges and faces follows
    /// the solids: projected again when what it comes from changed, and
    /// moved in place where it is the same kinds of curve. What comes from
    /// the sketch's own body is what the body's build projected of its
    /// solid where the sketch stands in the history, so the features built
    /// from the sketch never move it; until a build has, a solid built
    /// from the sketch waits. An edge gone or one now another kind of
    /// curve waits for the sketch to be edited.
    fn derive_on_geometry(
        &self,
        node: &core_document::FeatureNode,
        values: &mut Cow<'_, serde_json::Value>,
        document: &core_document::Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
    ) -> bool {
        if borrowed_holds(node, values, |f| f.sketch.external.is_empty()) {
            return false;
        }
        let Some(mut feature) = read_values(node, values) else {
            return false;
        };
        let built = document.derived().projected_references(node.id);
        let groups: Vec<_> = crate::sketch::external_groups(&feature.sketch)
            .into_iter()
            .filter(|(source, _)| source.reference.is_none())
            .filter_map(|(source, group)| {
                let answered = (node.body == Some(BodyId(source.body)))
                    .then(|| built?.answer(&feature::projection_probe(&feature.plane, &source)))
                    .flatten()
                    .cloned();
                if answered.is_none() && solid_built_from(document, node.id, BodyId(source.body)) {
                    return None;
                }
                Some((source, group, answered))
            })
            .collect();
        if groups.is_empty() {
            return false;
        }
        let plane = placed_plane(&feature.plane, &sketch_placement(document, node.id));
        let Ok(mut cache) = self.tab.solid_projections.lock() else {
            return false;
        };
        let known = cache.remove(&node.id).unwrap_or_default();
        let mut kept = HashMap::new();
        let mut moved = false;
        for (source, group, answered) in groups {
            let projected = match answered {
                Some(answered) => answered,
                None => {
                    let Some(kernel) = kernel else {
                        continue;
                    };
                    let key = projection_key(document, &plane, &source);
                    let projected = known.get(&key).cloned().unwrap_or_else(|| {
                        project_source_in(document, Some(kernel), &plane, &source)
                    });
                    kept.insert(key, projected.clone());
                    projected
                }
            };
            if let Ok(curves) = &projected
                && !external::already_holds(&feature.sketch, &group, curves)
            {
                let was = feature.sketch.geometry.clone();
                if external::refresh_in_place(&mut feature.sketch, &group, curves) {
                    moved = true;
                } else {
                    feature.sketch.geometry = was;
                }
            }
        }
        cache.insert(node.id, kept);
        if moved {
            *values = Cow::Owned(feature.to_json());
        }
        moved
    }

    fn derive_on_solid(
        &self,
        node: &core_document::FeatureNode,
        values: &mut Cow<'_, serde_json::Value>,
        probed: &core_document::ProbedReferences,
    ) -> bool {
        let Some(mut feature) = read_values(node, values) else {
            return false;
        };
        // Attached by a mode: the plane the attachment makes of what the
        // solid answered.
        if let Some(attached) = feature.attached {
            if probed.probes != attached.probes() {
                return false;
            }
            let plane = feature
                .offset
                .apply(attached.answered(&probed.answers).plane());
            if feature.plane == plane && feature.sketch.plane == plane {
                return false;
            }
            feature.plane = plane;
            feature.sketch.plane = plane;
            *values = Cow::Owned(feature.to_json());
            return true;
        }
        let (Some(face), Some(Ok(kernel_api::ProbeAnswer::Face { point, normal, .. }))) =
            (feature.face, probed.answers.first())
        else {
            return false;
        };
        let plane = feature.offset.apply(face.plane_at(*point, *normal));
        if feature.plane == plane && feature.sketch.plane == plane {
            return false;
        }
        feature.plane = plane;
        feature.sketch.plane = plane;
        *values = Cow::Owned(feature.to_json());
        true
    }
}

impl BenchParameters for SketchWorkbench {
    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        params::parameters(node)
    }
}

impl BenchOutput for SketchWorkbench {}

impl BenchCommands for SketchWorkbench {
    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        commands::run_on(self, id, args, ctx)
    }
}

#[cfg(feature = "egui")]
impl bench_panel::BenchUi for SketchWorkbench {
    /// The sketcher draws nothing under the tree: its UI lives in the
    /// task panel.
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        self.sync_active_sketch_from_ctx(ctx);
    }

    /// The Sketcher preferences page: the solver, snap and view switches,
    /// the minimum wall, and the palette, read-only.
    fn ui_settings(&mut self, ui: &mut egui::Ui, filter: &str) {
        let mut snap = !self.tab.snap_off;
        let widgets = settings::widgets(&self.options, snap);
        let out = bench_panel::show_filtered(
            ui,
            egui::Id::new("sketch_settings"),
            &widgets,
            None,
            filter,
        );
        for event in &out.events {
            settings::apply(&mut self.options, &mut snap, event);
        }
        self.tab.snap_off = !snap;
    }

    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        self.draw_task_panel(ui, ctx, request)
    }
}

/// The sketch `values` holds, read once per write of the node's data
/// while `values` is that data ([`core_document::FeatureNode::read`]).
/// Whether `values`, where they are the node's own data, read as a sketch
/// that `test` holds, looked at without a copy; false for values a step
/// changed.
fn borrowed_holds(
    node: &core_document::FeatureNode,
    values: &serde_json::Value,
    test: impl FnOnce(&SketchFeature) -> bool,
) -> bool {
    std::ptr::eq(values, &*node.data) && node.read_with(test) == Some(true)
}

fn read_values(
    node: &core_document::FeatureNode,
    values: &serde_json::Value,
) -> Option<SketchFeature> {
    if std::ptr::eq(values, &*node.data) {
        node.read()
    } else {
        SketchFeature::from_json(values).ok()
    }
}
