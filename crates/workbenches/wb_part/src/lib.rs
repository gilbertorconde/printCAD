use core_document::{
    CommandDescriptor, InputResult, ToolDescriptor, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

/// Part Design workbench: feature-based solid modeling.
#[derive(Default)]
pub struct PartDesignWorkbench {
    /// Example state: count of features (placeholder for real feature tree).
    feature_count: u32,
}

impl Workbench for PartDesignWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.part-design",
            "Part Design",
            "Feature-based solid modeling workbench.",
        )
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        context.register_tool(ToolDescriptor::new(
            "part.pad",
            "Pad (Extrude)",
            Some("modeling"),
        ));
        context.register_tool(ToolDescriptor::new(
            "part.pocket",
            "Pocket (Cut)",
            Some("modeling"),
        ));
        context.register_tool(ToolDescriptor::new(
            "part.fillet",
            "Fillet",
            Some("modeling"),
        ));
        context.register_command(CommandDescriptor::new(
            "part.recompute",
            "Recompute Feature Tree",
        ));
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Part Design workbench activated");
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Part Design workbench deactivated");
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        // Only handle input if a part design tool is active
        let tool = match active_tool {
            Some(t) if t.starts_with("part.") => t,
            _ => return InputResult::ignored(),
        };

        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } => match tool {
                "part.pad" => {
                    ctx.log_info(format!(
                        "Pad tool: click at ({:.1}, {:.1}) - select a sketch face to extrude",
                        viewport_pos.0, viewport_pos.1
                    ));
                    InputResult::consumed()
                }
                "part.pocket" => {
                    ctx.log_info(format!(
                        "Pocket tool: click at ({:.1}, {:.1}) - select a sketch face to cut",
                        viewport_pos.0, viewport_pos.1
                    ));
                    InputResult::consumed()
                }
                "part.fillet" => {
                    ctx.log_info(format!(
                        "Fillet tool: click at ({:.1}, {:.1}) - select edges to fillet",
                        viewport_pos.0, viewport_pos.1
                    ));
                    InputResult::consumed()
                }
                _ => InputResult::ignored(),
            },
            _ => InputResult::ignored(),
        }
    }

    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, _ctx: &mut WorkbenchRuntimeContext) {
        ui.separator();
        ui.heading("Part Info");
        ui.label(format!("Features: {}", self.feature_count));
    }

    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, ui: &mut egui::Ui, _ctx: &mut WorkbenchRuntimeContext) {
        ui.heading("Feature Properties");
        ui.label("Select a feature to edit its parameters.");
        ui.separator();
        ui.heading("Feature Tree");
        ui.label("(Feature tree will appear here)");
    }

    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label("Part Design workbench settings");
        ui.separator();
        ui.label("Default extrusion depth: (coming soon)");
        ui.label("Auto-recompute: (coming soon)");
        false
    }
}
