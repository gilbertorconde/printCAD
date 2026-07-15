mod feature;
mod overlay;
pub mod profile;
pub mod render;
pub mod sketch;
mod snap;
mod solver;
mod tools;

use std::collections::HashSet;

use core_document::{
    BodyId, CommandDescriptor, FeatureId, InputResult, ToolDescriptor, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
pub use feature::SketchFeature;
use overlay::SketchProjector;
use sketch::{Constraint, GeometryElement, Sketch, SketchPlane, Vec2D};
use solver::SolveOutcome;
use tools::ToolState;
use uuid::Uuid;

/// Snap / hit-test tolerance in screen pixels, converted to sketch units
/// per frame from the current zoom.
const SNAP_TOLERANCE_PX: f32 = 8.0;

/// Draft values for value-carrying constraints, edited in the left panel.
struct ConstraintDrafts {
    length: f32,
    radius: f32,
    distance: f32,
    angle_deg: f32,
}

impl Default for ConstraintDrafts {
    fn default() -> Self {
        Self {
            length: 10.0,
            radius: 5.0,
            distance: 10.0,
            angle_deg: 90.0,
        }
    }
}

/// A "create sketch" request waiting for the user to pick a plane.
struct PendingCreation {
    body: Option<BodyId>,
    /// Plane of the solid face that was selected when the request was made,
    /// offered as the first choice in the picker.
    face_plane: Option<SketchPlane>,
}

/// In-progress drag of a point in select mode.
struct DragState {
    point: Uuid,
    original: Vec2D,
    moved: bool,
}

/// Sketch workbench: 2D drawing with constraints.
#[derive(Default)]
pub struct SketchWorkbench {
    /// Currently active sketch feature ID (if any).
    active_sketch_id: Option<FeatureId>,
    /// Waiting for a plane choice before creating a sketch.
    pending_creation: Option<PendingCreation>,
    /// Point being dragged (select mode).
    dragging: Option<DragState>,
    /// In-progress drawing-tool state.
    tool_state: ToolState,
    /// Selected geometry ids (select mode; click toggles).
    selected: HashSet<Uuid>,
    /// Geometry under the cursor (select mode).
    hovered: Option<Uuid>,
    /// Cursor position in sketch coordinates (for previews), updated on
    /// mouse move while the cursor projects onto the sketch plane.
    cursor: Option<Vec2D>,
    /// Panel drafts for dimensional constraints.
    drafts: ConstraintDrafts,
    /// Last solver outcome, surfaced in the panel.
    last_solve: Option<SolveOutcome>,
}

impl SketchWorkbench {
    /// Get the active sketch from the document.
    fn get_active_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        self.active_sketch_id.and_then(|id| {
            ctx.document
                .get_feature_data(id)
                .and_then(|data| SketchFeature::from_json(data).ok())
        })
    }

    /// Persist a modified sketch feature back into the document and mark it
    /// dirty for recompute.
    fn store_sketch(&self, ctx: &mut WorkbenchRuntimeContext, feature: SketchFeature) -> bool {
        let Some(id) = self.active_sketch_id else {
            return false;
        };
        if let Err(e) = ctx.document.update_feature_data(id, feature.to_json()) {
            ctx.log_error(format!("Failed to update sketch: {e}"));
            return false;
        }
        ctx.document.mark_feature_dirty(id);
        true
    }

    /// Run the constraint solver on `feature`, record the outcome, and log
    /// failures. Returns the (possibly adjusted) feature.
    fn solve(&mut self, ctx: &mut WorkbenchRuntimeContext, feature: &mut SketchFeature) {
        let outcome = solver::solve(&mut feature.sketch);
        self.last_solve = Some(outcome);
        if let SolveOutcome::NotConverged { residual } = outcome {
            ctx.log_warn(format!(
                "Constraints did not converge (residual {residual:.2e}); check for contradictions"
            ));
        }
    }

    fn clear_interaction_state(&mut self) {
        self.tool_state = ToolState::Idle;
        self.selected.clear();
        self.hovered = None;
        self.cursor = None;
        self.dragging = None;
    }

    fn sync_active_sketch_from_ctx(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(feature_id) = ctx.active_document_object {
            if self.is_sketch_feature(ctx, feature_id) && self.active_sketch_id != Some(feature_id)
            {
                self.active_sketch_id = Some(feature_id);
                self.clear_interaction_state();

                if let Some(sketch_feature) = self.get_active_sketch(ctx) {
                    let plane = sketch_feature.plane;
                    ctx.camera_orient_request = Some(core_document::CameraOrientRequest {
                        plane_origin: plane.origin,
                        plane_normal: plane.normal,
                        plane_up: plane.y_axis,
                    });
                }
            }
        }
    }

    fn is_sketch_feature(&self, ctx: &WorkbenchRuntimeContext, feature_id: FeatureId) -> bool {
        ctx.document
            .get_feature_meta(feature_id)
            .map(|meta| meta.workbench_id.as_str() == "wb.sketch")
            .unwrap_or(false)
    }

    fn next_sketch_name(document: &core_document::Document) -> String {
        let mut max_index = None::<u32>;
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() == "wb.sketch" {
                if let Some(idx) = parse_sketch_index(&node.name) {
                    max_index = Some(max_index.map_or(idx, |m| m.max(idx)));
                }
            }
        }
        match max_index {
            None => "sketch".to_string(),
            Some(m) => format!("sketch_{}", m.saturating_add(1)),
        }
    }

    /// Project a viewport-local cursor position onto the sketch plane and
    /// express it in sketch coordinates.
    fn cursor_to_sketch(
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
        viewport_pos: (f32, f32),
    ) -> Option<Vec2D> {
        let world = ctx.viewport_to_plane(viewport_pos, plane.origin, plane.normal)?;
        let origin = glam::Vec3::from_array(plane.origin);
        let rel = glam::Vec3::from_array(world) - origin;
        Some(Vec2D::new(
            rel.dot(glam::Vec3::from_array(plane.x_axis)),
            rel.dot(glam::Vec3::from_array(plane.y_axis)),
        ))
    }

    /// Pixel tolerance converted into sketch units at the current zoom.
    fn snap_tolerance(ctx: &WorkbenchRuntimeContext, plane: &SketchPlane) -> f32 {
        let proj = SketchProjector::new(ctx, *plane);
        SNAP_TOLERANCE_PX * proj.units_per_px()
    }

    /// The "Create Sketch" action: open the plane picker. The sketch is
    /// created once a plane is chosen in the left panel.
    fn begin_sketch_creation(&mut self, body: Option<BodyId>, face_plane: Option<SketchPlane>) {
        self.pending_creation = Some(PendingCreation { body, face_plane });
    }

    fn create_sketch_on_plane(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: Option<BodyId>,
        plane: SketchPlane,
    ) {
        let sketch_name = Self::next_sketch_name(ctx.document);
        let mut sketch = Sketch::new(sketch_name.clone());
        sketch.plane = plane;
        let sketch_feature = SketchFeature::new(sketch, plane);

        match ctx
            .document
            .add_feature_in_body(sketch_feature, sketch_name.clone(), body)
        {
            Ok(feature_id) => {
                self.active_sketch_id = Some(feature_id);
                self.clear_interaction_state();
                ctx.active_document_object = Some(feature_id);
                ctx.camera_orient_request = Some(core_document::CameraOrientRequest {
                    plane_origin: plane.origin,
                    plane_normal: plane.normal,
                    plane_up: plane.y_axis,
                });
                ctx.log_info(format!("Created new sketch: {sketch_name}"));
            }
            Err(e) => {
                ctx.log_error(format!("Failed to create sketch: {e}"));
            }
        }
    }

    fn handle_left_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        let Some(cursor) = Self::cursor_to_sketch(ctx, &plane, viewport_pos) else {
            return InputResult::ignored();
        };
        let tol = Self::snap_tolerance(ctx, &plane);
        self.cursor = Some(cursor);

        match tool {
            Some(t) if t != "sketch.select" => {
                let effect =
                    tools::handle_click(&mut self.tool_state, t, &mut feature.sketch, cursor, tol);
                if effect.changed {
                    self.solve(ctx, &mut feature);
                    if let Some(log) = effect.log {
                        ctx.log_info(log);
                    }
                    self.store_sketch(ctx, feature);
                }
                InputResult::consumed()
            }
            _ => {
                // Select mode. Pressing on a point begins a drag (the
                // release decides between "click to toggle-select" and "drag
                // finished"); curves toggle immediately; empty space clears.
                match snap::hit_test(&feature.sketch, cursor, tol) {
                    Some(id) if feature.sketch.point_position(id).is_some() => {
                        self.dragging = Some(DragState {
                            point: id,
                            original: feature.sketch.point_position(id).unwrap(),
                            moved: false,
                        });
                        InputResult::consumed()
                    }
                    Some(id) => {
                        if !self.selected.remove(&id) {
                            self.selected.insert(id);
                        }
                        InputResult::consumed()
                    }
                    None => {
                        self.selected.clear();
                        InputResult::consumed()
                    }
                }
            }
        }
    }

    fn handle_mouse_move(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        self.cursor = Self::cursor_to_sketch(ctx, &plane, viewport_pos);

        // Constraint-aware point drag: move the point to the cursor and let
        // the solver re-project it onto whatever its constraints allow.
        // Consumed so the camera doesn't orbit underneath the drag.
        if let Some(drag) = self.dragging.as_mut() {
            if let Some(cursor) = self.cursor {
                let point = drag.point;
                drag.moved = true;
                if let Some(sketch::GeometryElement::Point(p)) =
                    feature.sketch.get_geometry_mut(point)
                {
                    p.position = cursor;
                }
                self.solve(ctx, &mut feature);
                self.store_sketch(ctx, feature);
            }
            return InputResult::consumed();
        }
        if matches!(tool, None | Some("sketch.select")) {
            let tol = Self::snap_tolerance(ctx, &plane);
            self.hovered = self
                .cursor
                .and_then(|c| snap::hit_test(&feature.sketch, c, tol));
        } else {
            self.hovered = None;
        }
        // Never consume moves — the camera still needs them for orbiting.
        InputResult::redraw_only()
    }

    fn handle_left_release(&mut self, _ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if let Some(drag) = self.dragging.take() {
            if !drag.moved {
                // A press+release without movement is a click: toggle-select.
                if !self.selected.remove(&drag.point) {
                    self.selected.insert(drag.point);
                }
            }
            return InputResult::consumed();
        }
        InputResult::ignored()
    }

    fn delete_selected(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if self.selected.is_empty() {
            return InputResult::ignored();
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let doomed: Vec<Uuid> = self.selected.drain().collect();
        let removed = feature.sketch.remove_geometry_cascade(&doomed);
        self.hovered = None;
        if removed.is_empty() {
            return InputResult::consumed();
        }
        self.solve(ctx, &mut feature);
        ctx.log_info(format!("Deleted {} sketch element(s)", removed.len()));
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    fn handle_escape(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if let Some(drag) = self.dragging.take() {
            // Restore the pre-drag position.
            if drag.moved {
                if let Some(mut feature) = self.get_active_sketch(ctx) {
                    if let Some(sketch::GeometryElement::Point(p)) =
                        feature.sketch.get_geometry_mut(drag.point)
                    {
                        p.position = drag.original;
                    }
                    self.solve(ctx, &mut feature);
                    self.store_sketch(ctx, feature);
                }
            }
            return InputResult::consumed();
        }
        if self.pending_creation.is_some() {
            self.pending_creation = None;
            return InputResult::consumed();
        }
        if !self.tool_state.is_idle() {
            self.tool_state = ToolState::Idle;
            ctx.log_info("Sketch: cancelled current tool operation");
        } else if !self.selected.is_empty() {
            self.selected.clear();
        }
        InputResult::consumed()
    }

    /// Add a constraint from the panel, then re-solve and persist.
    fn add_constraint(&mut self, ctx: &mut WorkbenchRuntimeContext, constraint: Constraint) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        ctx.log_info(format!(
            "Added constraint: {}",
            sketch::constraint_label(&constraint)
        ));
        feature.sketch.constraints.push(constraint);
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
    }
}

impl Workbench for SketchWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.sketch",
            "Sketch",
            "2D sketching environment with constraints and profiles.",
        )
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        context.register_tool(ToolDescriptor::new_action(
            "sketch.create",
            "Create Sketch",
            Some("sketch"),
        ));
        context.register_tool(ToolDescriptor::new(
            "sketch.select",
            "Select",
            Some("sketch"),
        ));
        context.register_tool(ToolDescriptor::new("sketch.point", "Point", Some("sketch")));
        context.register_tool(ToolDescriptor::new("sketch.line", "Line", Some("sketch")));
        context.register_tool(ToolDescriptor::new(
            "sketch.rect",
            "Rectangle",
            Some("sketch"),
        ));
        context.register_tool(ToolDescriptor::new(
            "sketch.circle",
            "Circle",
            Some("sketch"),
        ));
        context.register_tool(ToolDescriptor::new("sketch.arc", "Arc", Some("sketch")));
        // The solver runs automatically after every geometry/constraint
        // edit, so no explicit solve command is registered.
        context.register_command(CommandDescriptor::new("sketch.finish", "Finish Sketch"));
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench activated");
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench deactivated");
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.sync_active_sketch_from_ctx(ctx);

        if active_tool == Some("sketch.finish") {
            return if self.active_sketch_id.is_some() {
                self.active_sketch_id = None;
                self.clear_interaction_state();
                ctx.log_info("Finished sketch editing");
                InputResult::consumed()
            } else {
                ctx.log_warn("No active sketch to finish");
                InputResult::consumed()
            };
        }

        // Another workbench (or the host) asked us to create a sketch on a
        // specific body: take the request and open the plane picker.
        if let Some(request) = ctx.start_sketch_on_body.take() {
            let face_plane = request
                .face
                .map(|f| SketchPlane::from_face(f.point, f.normal));
            self.begin_sketch_creation(Some(BodyId(request.body)), face_plane);
        }

        if active_tool == Some("sketch.create") {
            if self.pending_creation.is_none() && self.active_sketch_id.is_none() {
                let face_plane = ctx
                    .selected_face
                    .map(|f| SketchPlane::from_face(f.point, f.normal));
                self.begin_sketch_creation(ctx.selected_body_id.map(BodyId), face_plane);
            }
            return InputResult::consumed();
        }

        if self.active_sketch_id.is_none() {
            return InputResult::ignored();
        }

        // Every remaining interaction needs an editing sketch. `None`
        // active tool behaves as select mode.
        let tool = match active_tool {
            Some(t) if t.starts_with("sketch.") => Some(t),
            _ => None,
        };

        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } => self.handle_left_click(ctx, tool, *viewport_pos),
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Right,
                ..
            } => {
                // Right click ends a line chain; otherwise let the camera
                // have the event (right-drag pans).
                if matches!(self.tool_state, ToolState::LineFrom { chain: true, .. }) {
                    self.tool_state = ToolState::Idle;
                    InputResult::consumed()
                } else {
                    InputResult::ignored()
                }
            }
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                ..
            } => self.handle_left_release(ctx),
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                self.handle_mouse_move(ctx, tool, *viewport_pos)
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } => self.handle_escape(ctx),
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Delete,
            }
            | WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Backspace,
            } => self.delete_selected(ctx),
            _ => InputResult::ignored(),
        }
    }

    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        self.sync_active_sketch_from_ctx(ctx);

        ui.heading("Sketcher");

        // Plane picker for a pending sketch creation.
        if let Some(pending) = &self.pending_creation {
            let body = pending.body;
            let face_plane = pending.face_plane;
            ui.label("New sketch — choose a plane:");
            let mut chosen: Option<SketchPlane> = None;
            if let Some(face) = face_plane {
                if ui
                    .button("▸ Selected face")
                    .on_hover_text("Sketch on the face you clicked on the solid")
                    .clicked()
                {
                    chosen = Some(face);
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Top (XY)").clicked() {
                    chosen = Some(SketchPlane::xy());
                }
                if ui.button("Front (XZ)").clicked() {
                    chosen = Some(SketchPlane::xz());
                }
                if ui.button("Side (YZ)").clicked() {
                    chosen = Some(SketchPlane::yz());
                }
            });
            if ui.button("Cancel").clicked() {
                self.pending_creation = None;
            }
            if let Some(plane) = chosen {
                self.pending_creation = None;
                self.create_sketch_on_plane(ctx, body, plane);
            }
            ui.separator();
        }

        let Some(feature) = self.get_active_sketch(ctx) else {
            if self.pending_creation.is_none() {
                ui.label("Select a sketch in the tree or create a new one to begin editing.");
            }
            return;
        };
        let sketch = &feature.sketch;

        ui.label(format!("Editing {}", sketch.name));
        if let Some(status) = self.tool_state.status() {
            ui.colored_label(egui::Color32::from_rgb(140, 190, 255), status);
        }

        let dof = solver::dof_estimate(sketch);
        let (dof_text, dof_color) = if sketch.constraints.is_empty() {
            (format!("{dof} DOF"), egui::Color32::GRAY)
        } else if dof == 0 {
            (
                "Fully constrained".to_string(),
                egui::Color32::from_rgb(90, 220, 110),
            )
        } else {
            (
                format!("{dof} DOF remaining"),
                egui::Color32::from_rgb(240, 200, 90),
            )
        };
        ui.colored_label(dof_color, dof_text);
        if let Some(SolveOutcome::NotConverged { .. }) = self.last_solve {
            ui.colored_label(
                egui::Color32::from_rgb(240, 110, 90),
                "⚠ Conflicting constraints",
            );
        }
        ui.separator();

        self.constraint_buttons(ui, ctx, sketch.clone());

        ui.separator();
        ui.heading("Constraints");
        let mut delete_constraint: Option<usize> = None;
        if sketch.constraints.is_empty() {
            ui.label("None yet. Select geometry to add constraints.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("sketch_constraints")
                .max_height(140.0)
                .show(ui, |ui| {
                    for (idx, constraint) in sketch.constraints.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .small_button("✕")
                                .on_hover_text("Remove constraint")
                                .clicked()
                            {
                                delete_constraint = Some(idx);
                            }
                            ui.label(sketch::constraint_label(constraint));
                        });
                    }
                });
        }
        if let Some(idx) = delete_constraint {
            if let Some(mut feature) = self.get_active_sketch(ctx) {
                feature.sketch.constraints.remove(idx);
                self.solve(ctx, &mut feature);
                self.store_sketch(ctx, feature);
            }
        }

        ui.separator();
        ui.heading("Geometry");
        if sketch.geometry.is_empty() {
            ui.label("No geometry yet. Use the toolbar tools to draw.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("sketch_geometry_elements")
                .max_height(180.0)
                .show(ui, |ui| {
                    for geom in sketch.geometry.iter() {
                        let id = geom.id();
                        let is_selected = self.selected.contains(&id);
                        let text = describe_geometry(sketch, geom);
                        if ui.selectable_label(is_selected, text).clicked()
                            && !self.selected.remove(&id)
                        {
                            self.selected.insert(id);
                        }
                    }
                });
        }
    }

    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        self.sync_active_sketch_from_ctx(ctx);
        ui.heading("Sketch Info");
        let Some(feature) = self.get_active_sketch(ctx) else {
            ui.label("No sketch selected. Select one in the tree or create a new sketch.");
            return;
        };
        ui.label(format!("Active sketch: {}", feature.sketch.name));
        ui.label(format!("Geometry: {}", feature.sketch.geometry.len()));
        ui.label(format!("Constraints: {}", feature.sketch.constraints.len()));
        ui.label(format!("Selected: {}", self.selected.len()));
        ui.separator();
        ui.label("Del deletes selection · Esc cancels");
        if ui.button("Exit Sketch Mode").clicked() {
            ctx.finish_sketch_requested = true;
        }
    }

    #[cfg(feature = "egui")]
    fn wants_right_panel(&self) -> bool {
        self.active_sketch_id.is_some()
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            "sketch.create" => ctx.selected_body_id.is_some(),
            _ => self.active_sketch_id.is_some(),
        }
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.active_sketch_id.is_some() {
            self.active_sketch_id = None;
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

    fn get_screen_space_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        overlay::build_overlays(
            &proj,
            &feature.sketch,
            &self.selected,
            self.hovered,
            &self.tool_state,
            self.cursor,
        )
    }
}

#[cfg(feature = "egui")]
impl SketchWorkbench {
    /// Constraint buttons applicable to the current selection.
    fn constraint_buttons(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        sketch: Sketch,
    ) {
        use GeometryElement as GE;

        let selected: Vec<&GE> = sketch
            .geometry
            .iter()
            .filter(|g| self.selected.contains(&g.id()))
            .collect();
        let lines: Vec<Uuid> = selected
            .iter()
            .filter(|g| matches!(g, GE::Line(_)))
            .map(|g| g.id())
            .collect();
        let points: Vec<Uuid> = selected
            .iter()
            .filter(|g| matches!(g, GE::Point(_)))
            .map(|g| g.id())
            .collect();
        let circles: Vec<Uuid> = selected
            .iter()
            .filter(|g| matches!(g, GE::Circle(_) | GE::Arc(_)))
            .map(|g| g.id())
            .collect();

        ui.heading("Add Constraint");
        if selected.is_empty() {
            ui.label("Select geometry in the viewport first.");
            return;
        }

        let mut pending: Option<Constraint> = None;

        if lines.len() == 1 && selected.len() == 1 {
            let line = lines[0];
            ui.horizontal(|ui| {
                if ui.button("Horizontal").clicked() {
                    pending = Some(Constraint::Horizontal { element: line });
                }
                if ui.button("Vertical").clicked() {
                    pending = Some(Constraint::Vertical { element: line });
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.drafts.length)
                        .speed(0.1)
                        .range(0.001..=1.0e6),
                );
                if ui.button("Length").clicked() {
                    pending = Some(Constraint::Length {
                        line,
                        length: self.drafts.length,
                    });
                }
            });
        }
        if lines.len() == 2 && selected.len() == 2 {
            ui.horizontal(|ui| {
                if ui.button("Parallel").clicked() {
                    pending = Some(Constraint::Parallel {
                        line1: lines[0],
                        line2: lines[1],
                    });
                }
                if ui.button("Perpendicular").clicked() {
                    pending = Some(Constraint::Perpendicular {
                        line1: lines[0],
                        line2: lines[1],
                    });
                }
                if ui.button("Equal").clicked() {
                    pending = Some(Constraint::EqualLength {
                        line1: lines[0],
                        line2: lines[1],
                    });
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.drafts.angle_deg)
                        .speed(1.0)
                        .suffix("°"),
                );
                if ui.button("Angle").clicked() {
                    pending = Some(Constraint::Angle {
                        line1: lines[0],
                        line2: lines[1],
                        angle_rad: self.drafts.angle_deg.to_radians(),
                    });
                }
            });
        }
        if circles.len() == 1 && selected.len() == 1 {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.drafts.radius)
                        .speed(0.1)
                        .range(0.001..=1.0e6),
                );
                if ui.button("Radius").clicked() {
                    pending = Some(Constraint::Radius {
                        circle: circles[0],
                        radius: self.drafts.radius,
                    });
                }
            });
        }
        if circles.len() == 2 && selected.len() == 2 && ui.button("Equal radius").clicked() {
            pending = Some(Constraint::EqualRadius {
                circle1: circles[0],
                circle2: circles[1],
            });
        }
        if points.len() == 2 && selected.len() == 2 {
            ui.horizontal(|ui| {
                if ui.button("Coincident").clicked() {
                    pending = Some(Constraint::Coincident {
                        point1: points[0],
                        point2: points[1],
                    });
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.drafts.distance)
                        .speed(0.1)
                        .range(0.001..=1.0e6),
                );
                if ui.button("Distance").clicked() {
                    pending = Some(Constraint::Distance {
                        point1: points[0],
                        point2: points[1],
                        distance: self.drafts.distance,
                    });
                }
            });
        }
        if points.len() == 1
            && lines.len() == 1
            && selected.len() == 2
            && ui.button("Point on line").clicked()
        {
            pending = Some(Constraint::PointOnLine {
                point: points[0],
                line: lines[0],
            });
        }
        if points.len() == 1
            && circles.len() == 1
            && selected.len() == 2
            && ui.button("Point on circle").clicked()
        {
            pending = Some(Constraint::PointOnCircle {
                point: points[0],
                circle: circles[0],
            });
        }
        if points.len() == 1 && selected.len() == 1 && ui.button("Fix point").clicked() {
            let position = sketch
                .point_position(points[0])
                .unwrap_or(Vec2D::new(0.0, 0.0));
            pending = Some(Constraint::FixedPoint {
                point: points[0],
                position,
            });
        }

        if let Some(constraint) = pending {
            self.add_constraint(ctx, constraint);
        }
    }
}

fn parse_sketch_index(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("sketch_") {
        r
    } else {
        lower.strip_prefix("sketch")?
    };

    let trimmed = rest.trim_start_matches(&['_', '.', ' '][..]);
    if trimmed.is_empty() {
        Some(0)
    } else {
        trimmed.parse().ok()
    }
}

#[cfg(feature = "egui")]
fn describe_geometry(sketch: &Sketch, element: &GeometryElement) -> String {
    match element {
        GeometryElement::Point(point) => {
            format!("Point ({:.2}, {:.2})", point.position.x, point.position.y)
        }
        GeometryElement::Line(line) => {
            let start = sketch.point_position(line.start);
            let end = sketch.point_position(line.end);
            match (start, end) {
                (Some(s), Some(e)) => {
                    format!("Line ({:.2}, {:.2}) → ({:.2}, {:.2})", s.x, s.y, e.x, e.y)
                }
                _ => "Line (incomplete)".to_string(),
            }
        }
        GeometryElement::Circle(circle) => match sketch.point_position(circle.center) {
            Some(c) => format!("Circle ({:.2}, {:.2}) r={:.2}", c.x, c.y, circle.radius),
            None => format!("Circle r={:.2}", circle.radius),
        },
        GeometryElement::Arc(arc) => match sketch.point_position(arc.center) {
            Some(c) => format!("Arc ({:.2}, {:.2}) r={:.2}", c.x, c.y, arc.radius),
            None => format!("Arc r={:.2}", arc.radius),
        },
    }
}
