mod feature;
pub mod render;
mod sketch;

use core_document::{
    CommandDescriptor, FeatureId, InputResult, ToolDescriptor, ToolKind, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
pub use feature::SketchFeature;
use sketch::{GeometryElement, Line, Point, Sketch};
use uuid::Uuid;

/// Sketch workbench: 2D drawing with constraints.
pub struct SketchWorkbench {
    /// Currently active sketch feature ID (if any).
    active_sketch_id: Option<FeatureId>,
    /// Line tool state: first point (if clicking to create a line).
    line_tool_state: Option<Uuid>,
    /// Circle tool state: center point (if clicking to create a circle).
    circle_tool_state: Option<Uuid>,
    /// Arc tool state: (center, start) points (if clicking to create an arc).
    arc_tool_state: Option<(Uuid, Uuid)>,
}

impl Default for SketchWorkbench {
    fn default() -> Self {
        Self {
            active_sketch_id: None,
            line_tool_state: None,
            circle_tool_state: None,
            arc_tool_state: None,
        }
    }
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

    /// Get mutable access to the active sketch (requires updating the document).
    fn get_active_sketch_mut(
        &self,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<(FeatureId, SketchFeature)> {
        self.active_sketch_id.and_then(|id| {
            ctx.document
                .get_feature_data(id)
                .and_then(|data| SketchFeature::from_json(data).ok().map(|feat| (id, feat)))
        })
    }

    /// Update the active sketch in the document.
    fn update_active_sketch(
        &self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: SketchFeature,
    ) -> bool {
        if let Some(id) = self.active_sketch_id {
            if let Err(e) = ctx.document.update_feature_data(id, feature.to_json()) {
                ctx.log_error(format!("Failed to update sketch: {}", e));
                return false;
            }
            true
        } else {
            false
        }
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
        // Register "Create Sketch" as an action (checkbox-like behavior)
        context.register_tool(ToolDescriptor::new(
            "sketch.create",
            "Create Sketch",
            ToolKind::Action,
        ));
        // Register sketch tools (radio-like behavior)
        context.register_tool(ToolDescriptor::new("sketch.line", "Line", ToolKind::Sketch));
        context.register_tool(ToolDescriptor::new("sketch.arc", "Arc", ToolKind::Sketch));
        context.register_tool(ToolDescriptor::new(
            "sketch.circle",
            "Circle",
            ToolKind::Sketch,
        ));
        context.register_command(CommandDescriptor::new(
            "sketch.constraints.solve",
            "Solve Constraints",
        ));
        context.register_command(CommandDescriptor::new("sketch.finish", "Finish Sketch"));
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench activated");
        // Don't auto-create sketch - user must use "Create Sketch" action
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
        // Handle "Finish Sketch" action
        if active_tool == Some("sketch.finish") {
            if self.active_sketch_id.is_some() {
                self.active_sketch_id = None;
                self.line_tool_state = None;
                self.circle_tool_state = None;
                self.arc_tool_state = None;
                ctx.log_info("Finished sketch editing");
                return InputResult::consumed();
            } else {
                ctx.log_warn("No active sketch to finish");
                return InputResult::consumed();
            }
        }

        // Handle "Create Sketch" action
        if active_tool == Some("sketch.create") {
            // For now, create sketch with default plane
            // TODO: Show dialog to select plane/face
            if self.active_sketch_id.is_none() {
                let sketch = Sketch::new("Sketch.001");
                let sketch_feature = SketchFeature::from_sketch(sketch.clone());

                match ctx
                    .document
                    .add_feature(sketch_feature, sketch.name.clone())
                {
                    Ok(feature_id) => {
                        self.active_sketch_id = Some(feature_id);
                        ctx.log_info(format!("Created new sketch: {}", sketch.name));

                        // Request camera orientation to sketch plane
                        let plane = sketch::SketchPlane::default();
                        ctx.camera_orient_request = Some(core_document::CameraOrientRequest {
                            plane_origin: plane.origin,
                            plane_normal: plane.normal,
                            plane_up: plane.y_axis,
                        });

                        // Deactivate the create tool after creation
                        return InputResult::consumed();
                    }
                    Err(e) => {
                        ctx.log_error(format!("Failed to create sketch: {}", e));
                        return InputResult::consumed();
                    }
                }
            } else {
                ctx.log_warn("A sketch already exists. Please finish or delete it first.");
                return InputResult::consumed();
            }
        }

        // Only handle input if a sketch tool is active
        let tool = match active_tool {
            Some(t) if t.starts_with("sketch.") && t != "sketch.create" => t,
            _ => return InputResult::ignored(),
        };

        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: _,
            } => {
                // Get active sketch to access its plane
                let sketch_feature = match self.get_active_sketch(ctx) {
                    Some(sf) => sf,
                    None => {
                        ctx.log_warn("No active sketch");
                        return InputResult::consumed();
                    }
                };

                // Convert viewport position to sketch coordinates
                // Use hovered_world_pos if available (from picking), otherwise project onto plane
                let world_pos = if let Some(hovered) = ctx.hovered_world_pos {
                    hovered
                } else {
                    // Project viewport coordinates onto sketch plane
                    // For now, use a simple approach: if we can't get world pos, use viewport center
                    // TODO: Implement proper viewport-to-plane projection using camera controller
                    let plane_origin = glam::Vec3::from_array(sketch_feature.plane.origin);
                    let _plane_normal = glam::Vec3::from_array(sketch_feature.plane.normal);

                    // Simple fallback: use plane origin if we can't project
                    plane_origin.to_array()
                };

                // Convert world position to sketch 2D coordinates
                let plane_origin = glam::Vec3::from_array(sketch_feature.plane.origin);
                let plane_x = glam::Vec3::from_array(sketch_feature.plane.x_axis);
                let plane_y = glam::Vec3::from_array(sketch_feature.plane.y_axis);
                let world_vec = glam::Vec3::from_array(world_pos) - plane_origin;
                let sketch_x = world_vec.dot(plane_x);
                let sketch_y = world_vec.dot(plane_y);
                let sketch_pos = sketch::Vec2D::new(sketch_x, sketch_y);

                match tool {
                    "sketch.line" => {
                        // Require active sketch - don't auto-create
                        if self.active_sketch_id.is_none() {
                            ctx.log_warn("No active sketch. Please create a sketch first.");
                            return InputResult::consumed();
                        }

                        // Get the sketch from document
                        if let Some((feature_id, mut sketch_feature)) =
                            self.get_active_sketch_mut(ctx)
                        {
                            if let Some(first_point_id) = self.line_tool_state {
                                // Second click: create line from first point to this point
                                let end_point = Point::new(sketch_pos);
                                let end_id = sketch_feature
                                    .sketch
                                    .add_geometry(GeometryElement::Point(end_point.clone()));

                                let line = Line::new(first_point_id, end_id);
                                let line_id = sketch_feature
                                    .sketch
                                    .add_geometry(GeometryElement::Line(line));

                                ctx.log_info(format!(
                                    "Created line from point {:?} to {:?} (line ID: {:?})",
                                    first_point_id, end_id, line_id
                                ));

                                // Update sketch in document
                                if self.update_active_sketch(ctx, sketch_feature) {
                                    ctx.document.mark_feature_dirty(feature_id);
                                }

                                self.line_tool_state = None;
                                InputResult::consumed()
                            } else {
                                // First click: create start point
                                let start_point = Point::new(sketch_pos);
                                let start_id = sketch_feature
                                    .sketch
                                    .add_geometry(GeometryElement::Point(start_point.clone()));

                                // Update sketch in document
                                if self.update_active_sketch(ctx, sketch_feature) {
                                    self.line_tool_state = Some(start_id);
                                    ctx.log_info(format!(
                                        "Line tool: start point at ({:.1}, {:.1}) - click again for end point",
                                        sketch_pos.x, sketch_pos.y
                                    ));
                                }
                                InputResult::consumed()
                            }
                        } else {
                            ctx.log_error("Failed to get active sketch from document");
                            InputResult::consumed()
                        }
                    }
                    "sketch.circle" => {
                        // Require active sketch - don't auto-create
                        if self.active_sketch_id.is_none() {
                            ctx.log_warn("No active sketch. Please create a sketch first.");
                            return InputResult::consumed();
                        }

                        // Get the sketch from document
                        if let Some((feature_id, mut sketch_feature)) =
                            self.get_active_sketch_mut(ctx)
                        {
                            if let Some(center_id) = self.circle_tool_state {
                                // Second click: create circle with radius from center to this point
                                let center_point = sketch_feature
                                    .sketch
                                    .get_geometry(center_id)
                                    .and_then(|g| match g {
                                        GeometryElement::Point(p) => Some(p),
                                        _ => None,
                                    });

                                if let Some(center) = center_point {
                                    let center_glam = center.position.to_glam();
                                    let pos_glam = sketch_pos.to_glam();
                                    let radius = (pos_glam - center_glam).length();
                                    let circle = sketch::Circle::new(center_id, radius);
                                    let circle_id = sketch_feature
                                        .sketch
                                        .add_geometry(GeometryElement::Circle(circle));

                                    ctx.log_info(format!(
                                        "Created circle with center {:?} and radius {:.2} (circle ID: {:?})",
                                        center_id, radius, circle_id
                                    ));

                                    // Update sketch in document
                                    if self.update_active_sketch(ctx, sketch_feature) {
                                        ctx.document.mark_feature_dirty(feature_id);
                                    }

                                    self.circle_tool_state = None;
                                    InputResult::consumed()
                                } else {
                                    ctx.log_error("Circle center point not found");
                                    self.circle_tool_state = None;
                                    InputResult::consumed()
                                }
                            } else {
                                // First click: create center point
                                let center_point = Point::new(sketch_pos);
                                let center_id = sketch_feature
                                    .sketch
                                    .add_geometry(GeometryElement::Point(center_point.clone()));

                                // Update sketch in document
                                if self.update_active_sketch(ctx, sketch_feature) {
                                    self.circle_tool_state = Some(center_id);
                                    ctx.log_info(format!(
                                        "Circle tool: center point at ({:.1}, {:.1}) - click again for radius",
                                        sketch_pos.x, sketch_pos.y
                                    ));
                                }
                                InputResult::consumed()
                            }
                        } else {
                            ctx.log_error("Failed to get active sketch from document");
                            InputResult::consumed()
                        }
                    }
                    "sketch.arc" => {
                        // Require active sketch - don't auto-create
                        if self.active_sketch_id.is_none() {
                            ctx.log_warn("No active sketch. Please create a sketch first.");
                            return InputResult::consumed();
                        }

                        // Get the sketch from document
                        if let Some((feature_id, mut sketch_feature)) =
                            self.get_active_sketch_mut(ctx)
                        {
                            if let Some((center_id, start_id)) = self.arc_tool_state {
                                // Third click: create arc from center, start to this point
                                let center_point = sketch_feature
                                    .sketch
                                    .get_geometry(center_id)
                                    .and_then(|g| match g {
                                        GeometryElement::Point(p) => Some(p.position),
                                        _ => None,
                                    });
                                let start_point = sketch_feature
                                    .sketch
                                    .get_geometry(start_id)
                                    .and_then(|g| match g {
                                        GeometryElement::Point(p) => Some(p.position),
                                        _ => None,
                                    });

                                if let (Some(center), Some(start)) = (center_point, start_point) {
                                    let end_point = Point::new(sketch_pos);
                                    let end_id = sketch_feature
                                        .sketch
                                        .add_geometry(GeometryElement::Point(end_point.clone()));

                                    // Calculate radius from center to start
                                    let center_glam = center.to_glam();
                                    let start_glam = start.to_glam();
                                    let radius = (start_glam - center_glam).length();

                                    let arc = sketch::Arc::new(center_id, start_id, end_id, radius);
                                    let arc_id = sketch_feature
                                        .sketch
                                        .add_geometry(GeometryElement::Arc(arc));

                                    ctx.log_info(format!(
                                        "Created arc with center {:?}, start {:?}, end {:?}, radius {:.2} (arc ID: {:?})",
                                        center_id, start_id, end_id, radius, arc_id
                                    ));

                                    // Update sketch in document
                                    if self.update_active_sketch(ctx, sketch_feature) {
                                        ctx.document.mark_feature_dirty(feature_id);
                                    }

                                    self.arc_tool_state = None;
                                    InputResult::consumed()
                                } else {
                                    ctx.log_error("Arc center or start point not found");
                                    self.arc_tool_state = None;
                                    InputResult::consumed()
                                }
                            } else if let Some(center_id) = self.circle_tool_state {
                                // Second click: create start point
                                let start_point = Point::new(sketch_pos);
                                let start_id = sketch_feature
                                    .sketch
                                    .add_geometry(GeometryElement::Point(start_point.clone()));

                                // Update sketch in document
                                if self.update_active_sketch(ctx, sketch_feature) {
                                    self.arc_tool_state = Some((center_id, start_id));
                                    self.circle_tool_state = None; // Clear circle state
                                    ctx.log_info(format!(
                                        "Arc tool: start point at ({:.1}, {:.1}) - click again for end point",
                                        sketch_pos.x, sketch_pos.y
                                    ));
                                }
                                InputResult::consumed()
                            } else {
                                // First click: create center point
                                let center_point = Point::new(sketch_pos);
                                let center_id = sketch_feature
                                    .sketch
                                    .add_geometry(GeometryElement::Point(center_point.clone()));

                                // Update sketch in document
                                if self.update_active_sketch(ctx, sketch_feature) {
                                    self.circle_tool_state = Some(center_id); // Reuse circle state for center
                                    ctx.log_info(format!(
                                        "Arc tool: center point at ({:.1}, {:.1}) - click again for start point",
                                        sketch_pos.x, sketch_pos.y
                                    ));
                                }
                                InputResult::consumed()
                            }
                        } else {
                            ctx.log_error("Failed to get active sketch from document");
                            InputResult::consumed()
                        }
                    }
                    _ => InputResult::ignored(),
                }
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } => {
                // Cancel current tool operation
                if self.line_tool_state.is_some()
                    || self.circle_tool_state.is_some()
                    || self.arc_tool_state.is_some()
                {
                    self.line_tool_state = None;
                    self.circle_tool_state = None;
                    self.arc_tool_state = None;
                    ctx.log_info("Sketch: Cancelled current tool operation");
                } else {
                    ctx.log_info("Sketch: Escape pressed");
                }
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, _ctx: &mut WorkbenchRuntimeContext) {}

    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(sketch_feature) = self.get_active_sketch(ctx) {
            ui.heading("Sketch Info");
            ui.label(format!("Active sketch: {}", sketch_feature.sketch.name));
            ui.label(format!(
                "Geometry elements: {}",
                sketch_feature.sketch.geometry.len()
            ));
            ui.label(format!(
                "Constraints: {}",
                sketch_feature.sketch.constraints.len()
            ));

            if let Some(id) = self.active_sketch_id {
                if let Some(meta) = ctx.document.get_feature_meta(id) {
                    ui.label(format!("Feature ID: {:?}", id));
                    ui.label(format!("Dirty: {}", meta.dirty));
                }
            }

            if self.line_tool_state.is_some() {
                ui.label("Line tool: click for end point");
            }
            if self.circle_tool_state.is_some() {
                ui.label("Circle tool: click for radius");
            }
            if let Some((_center_id, _start_id)) = self.arc_tool_state {
                ui.label("Arc tool: click for end point");
            }

            ui.separator();
            ui.label("Exit sketch mode to return to normal view.");
            if ui.button("Exit Sketch Mode").clicked() {
                ctx.finish_sketch_requested = true;
            }
        }
    }

    #[cfg(feature = "egui")]
    fn wants_right_panel(&self) -> bool {
        self.active_sketch_id.is_some()
    }

    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label("Sketch workbench settings");
        ui.separator();
        ui.label("Grid snap: (coming soon)");
        ui.label("Constraint display: (coming soon)");
        false
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        // Exit sketch editing mode - clear editing state but keep sketch as active document object
        if self.active_sketch_id.is_some() {
            // Note: active_document_object remains set (sketch stays selected in tree)
            self.active_sketch_id = None; // Exit editing mode
            self.line_tool_state = None;
            self.circle_tool_state = None;
            self.arc_tool_state = None;
            ctx.log_info("Exited sketch editing mode (sketch remains selected)");
        } else {
            ctx.log_warn("Not in sketch editing mode");
        }
    }
}
