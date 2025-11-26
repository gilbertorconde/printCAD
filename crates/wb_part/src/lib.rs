use core_document::{
    CommandDescriptor, ToolDescriptor, ToolKind, Workbench, WorkbenchContext, WorkbenchDescriptor,
};

#[derive(Default)]
pub struct PartDesignWorkbench;

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
            ToolKind::PartDesign,
        ));
        context.register_tool(ToolDescriptor::new(
            "part.pocket",
            "Pocket (Cut)",
            ToolKind::PartDesign,
        ));
        context.register_tool(ToolDescriptor::new(
            "part.fillet",
            "Fillet",
            ToolKind::PartDesign,
        ));
        context.register_command(CommandDescriptor::new(
            "part.recompute",
            "Recompute Feature Tree",
        ));
    }
}
