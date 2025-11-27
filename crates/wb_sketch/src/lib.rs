use core_document::{
    CommandDescriptor, InputResult, ToolDescriptor, ToolKind, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

/// Sketch workbench: 2D drawing with constraints.
#[derive(Default)]
pub struct SketchWorkbench {
    /// Example state: count of lines created (placeholder for real sketch data).
    line_count: u32,
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
        // Only handle input if a sketch tool is active
        let tool = match active_tool {
            Some(t) if t.starts_with("sketch.") => t,
            _ => return InputResult::ignored(),
        };

        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } => {
                // Placeholder: log the click and increment counter
                match tool {
                    "sketch.line" => {
                        self.line_count += 1;
                        ctx.log_info(format!(
                            "Line tool: click at ({:.1}, {:.1}) - line #{}",
                            viewport_pos.0, viewport_pos.1, self.line_count
                        ));
                        ctx.document.mark_dirty();
                        InputResult::consumed()
                    }
                    "sketch.circle" => {
                        ctx.log_info(format!(
                            "Circle tool: click at ({:.1}, {:.1})",
                            viewport_pos.0, viewport_pos.1
                        ));
                        InputResult::consumed()
                    }
                    "sketch.arc" => {
                        ctx.log_info(format!(
                            "Arc tool: click at ({:.1}, {:.1})",
                            viewport_pos.0, viewport_pos.1
                        ));
                        InputResult::consumed()
                    }
                    _ => InputResult::ignored(),
                }
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } => {
                ctx.log_info("Sketch: Escape pressed, could cancel current operation");
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, _ctx: &WorkbenchRuntimeContext) {
        ui.separator();
        ui.heading("Sketch Info");
        ui.label(format!("Lines created: {}", self.line_count));
    }

    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, ui: &mut egui::Ui, _ctx: &WorkbenchRuntimeContext) {
        ui.heading("Sketch Properties");
        ui.label("Select a sketch element to edit its properties.");
        ui.separator();
        ui.heading("Constraints");
        ui.label("No constraints defined yet.");
    }

    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label("Sketch workbench settings");
        ui.separator();
        ui.label("Grid snap: (coming soon)");
        ui.label("Constraint display: (coming soon)");
        false
    }
}
