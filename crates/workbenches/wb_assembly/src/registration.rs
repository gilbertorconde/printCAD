//! The bench's tools: their registration and when each is enabled.

use crate::{AssemblyWorkbench, Gearing, JointKind, JointTool, commands, handles, joints};
use core_document::{ToolDescriptor, WorkbenchContext, WorkbenchRuntimeContext};

impl AssemblyWorkbench {
    pub(crate) fn register_tools(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        let tool = |id: &str, label: &str, icon: &'static str| {
            let category = match id {
                "asm.couple" | "asm.motion" => "motion",
                "asm.parts" => "parts",
                _ if JointTool::ALL.iter().any(|j| j.command() == id) => "joints",
                _ => "arrange",
            };
            ToolDescriptor::new_action(id, label, Some(category))
                .icon(icon)
                .row(1)
        };
        let check = |id: &str, label: &str, icon: &'static str| {
            ToolDescriptor::new_action(id, label, Some(core_document::INSPECT_CATEGORY))
                .icon(icon)
                .row(1)
        };
        for joint in JointTool::ALL {
            context.register_tool(
                tool(joint.command(), joint.label(), joint.icon()).shortcut(joint.shortcut()),
            );
        }
        context.register_tool(tool("asm.couple", "Couple joints", "involute-gear").shortcut("K"));
        context.register_tool(
            tool("asm.motion", "Motion over time", "polar-pattern").shortcut("Shift+M"),
        );
        context.register_tool(tool("asm.move", "Move body", "move-geometry").shortcut("G"));
        context.register_action(
            core_document::ActionDescriptor::new(
                handles::MODE_ACTION,
                "Move body: switch the handles between moving and turning",
            )
            .category("arrange")
            .shortcut("Shift+G"),
        );
        context.register_tool(tool("asm.explode", "Exploded view", "scale-geometry").shortcut("E"));
        context.register_tool(
            tool("asm.collisions", "Stop drags at collisions", "boolean").shortcut("C"),
        );
        context.register_tool(tool("asm.group", "Rigid group", "tree-group").shortcut("U"));
        context.register_tool(tool("asm.copy", "Insert linked copies", "clone").shortcut("Y"));
        context
            .register_tool(tool("asm.replace", "Replace body", "carbon-copy").shortcut("Shift+Y"));
        context.register_tool(tool("asm.ground", "Ground body", "constraint-lock").shortcut("F"));
        context.register_tool(tool("asm.solve", "Solve joints", "refresh").shortcut("S"));
        context.register_tool(
            tool("asm.save_state", "Save assembly state", "save").shortcut("Shift+E"),
        );
        context.register_tool(tool("asm.parts", "Parts list", "file-document").shortcut("B"));
        // The checks, which Inspect lists too.
        context.register_tool(
            check("asm.interference", "Check interference", "check-geometry").shortcut("I"),
        );
        context
            .register_tool(check("asm.mass", "Mass and centre of mass", "measure").shortcut("W"));
    }

    pub(crate) fn tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            id if JointTool::of_command(id).is_some() => ctx.document.bodies().len() >= 2,
            "asm.interference" | "asm.explode" => ctx.document.bodies().len() >= 2,
            "asm.parts" | "asm.mass" => !ctx.document.bodies().is_empty(),
            "asm.group" => ctx.document.bodies().len() >= 2,
            "asm.save_state" => !ctx.document.bodies().is_empty(),
            "asm.motion" => joints(ctx.document).iter().any(|j| {
                matches!(
                    j.feature.kind,
                    JointKind::Hinge { .. } | JointKind::Slider { .. }
                )
            }),
            "asm.copy" => Self::body_to_move(ctx).is_some(),
            "asm.replace" => Self::body_to_move(ctx).is_some() && ctx.document.bodies().len() >= 2,
            "asm.collisions" => true,
            "asm.move" | "asm.ground" => Self::body_to_move(ctx).is_some(),
            "asm.solve" => !joints(ctx.document).is_empty(),
            "asm.couple" => {
                let all = joints(ctx.document);
                all.iter().any(|a| {
                    all.iter().any(|b| {
                        a.id != b.id && Gearing::suiting(&a.feature.kind, &b.feature.kind).is_some()
                    })
                })
            }
            _ => false,
        }
    }
}
