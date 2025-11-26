use core_document::{
    CommandDescriptor, ToolDescriptor, ToolKind, Workbench, WorkbenchContext, WorkbenchDescriptor,
};

#[derive(Default)]
pub struct SketchWorkbench;

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
}
