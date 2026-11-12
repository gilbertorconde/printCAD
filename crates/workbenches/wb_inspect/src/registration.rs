//! The bench's tools: their ids, their registration and when each is
//! enabled.

use crate::{
    BEST_FIT, CLEARANCE, COMPARE_TOOL, ELEMENT_MAP, EXTRACT, InspectionWorkbench, OVERHANG, PIN,
    RESET_ALIGNMENT, SAVE_IGES, SAVE_REPORT, SAVE_STEP, SECTION, SHADOW, THICKNESS, TOLERANCE,
    align_commands, commands, construction, construction::Construction, construction::Made,
    dimension::DimKind, element, element::Element, making::characteristic_for, making::element_at,
    making::element_solid, making::selected_elements, mapping::compare_pair,
    mapping::thickness_body,
};
use core_document::{BodyId, ToolDescriptor, WorkbenchContext, WorkbenchRuntimeContext};
use kernel_api::FitKind;

pub(crate) fn tool_id(kind: FitKind) -> String {
    format!("inspect.fit_{}", element::kind_word(kind).to_lowercase())
}

pub(crate) fn kind_of_tool(id: &str) -> Option<FitKind> {
    FitKind::ALL.into_iter().find(|k| tool_id(*k) == id)
}

fn dim_tool_id(kind: DimKind) -> String {
    format!("inspect.{}", kind.word().to_lowercase())
}

pub(crate) fn dim_kind_of_tool(id: &str) -> Option<DimKind> {
    DimKind::ALL.into_iter().find(|k| dim_tool_id(*k) == id)
}

fn construct_tool_id(made: Made) -> String {
    format!("inspect.construct_{}", made.word().to_lowercase())
}

pub(crate) fn made_of_tool(id: &str) -> Option<Made> {
    Made::ALL.into_iter().find(|m| construct_tool_id(*m) == id)
}

impl InspectionWorkbench {
    pub(super) fn register_tools(&self, context: &mut WorkbenchContext) {
        align_commands::configure(context);
        for kind in FitKind::ALL {
            context.register_tool(
                ToolDescriptor::new_action(
                    tool_id(kind),
                    format!("Fit {}", element::kind_word(kind).to_lowercase()),
                    Some("fit"),
                )
                .icon(element::icon(kind)),
            );
        }
        context.register_tool(
            ToolDescriptor::new_action(BEST_FIT, "Best fit to reference", Some("align"))
                .icon("joint-align"),
        );
        context.register_tool(
            ToolDescriptor::new_action(RESET_ALIGNMENT, "Reset alignment", Some("align"))
                .icon("home"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SECTION, "Section", Some("section")).icon("clipping-plane"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SHADOW, "Shadow outline", Some("section")).icon("cut"),
        );
        context.register_tool(
            ToolDescriptor::new_action(PIN, "Pin reading", Some("compare")).icon("datum-point"),
        );
        context.register_tool(
            ToolDescriptor::new_action(ELEMENT_MAP, "Map against element", Some("compare"))
                .icon("constraint-distance"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SAVE_REPORT, "Save report…", Some("report")).icon("save"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SAVE_STEP, "Save elements as STEP…", Some("report"))
                .icon("save"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SAVE_IGES, "Save elements as IGES…", Some("report"))
                .icon("save"),
        );
        context.register_tool(
            ToolDescriptor::new_action(TOLERANCE, "Tolerance", Some("dimension"))
                .icon("constraint-perpendicular"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                THICKNESS,
                "Wall thickness",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                EXTRACT,
                "Element as body",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("body"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                CLEARANCE,
                "Fit check",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                OVERHANG,
                "Overhangs",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_tool(
            ToolDescriptor::new_action(COMPARE_TOOL, "Compare with reference", Some("compare"))
                .icon("measure"),
        );
        for made in Made::ALL {
            context.register_tool(
                ToolDescriptor::new_action(
                    construct_tool_id(made),
                    format!("Construct {}", made.word().to_lowercase()),
                    Some("construct"),
                )
                .icon(made.icon()),
            );
        }
        for kind in DimKind::ALL {
            context.register_tool(
                ToolDescriptor::new_action(dim_tool_id(kind), kind.word(), Some("dimension"))
                    .icon(kind.icon()),
            );
        }
        commands::configure(context);
    }

    pub(super) fn tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        if tool_id == align_commands::POINTS {
            let bodies = ctx.document.bodies();
            return bodies.len() > 1
                && bodies
                    .iter()
                    .any(|b| ctx.document.body_scan(b.id).is_some());
        }
        if align_commands::owns(tool_id) {
            return align_commands::selected_args(tool_id, ctx).is_some();
        }
        if let Some(kind) = dim_kind_of_tool(tool_id) {
            return selected_elements(ctx).len() == kind.operands();
        }
        if tool_id == COMPARE_TOOL {
            return compare_pair(ctx).is_some();
        }
        if tool_id == PIN {
            return ctx
                .document
                .bodies()
                .iter()
                .any(|b| self.shown_map(ctx.document, b.id).is_some());
        }
        if tool_id == ELEMENT_MAP {
            return selected_elements(ctx).len() == 1;
        }
        if tool_id == SHADOW {
            return match selected_elements(ctx).as_slice() {
                [one] => element_at(ctx.document, *one).is_some_and(|e| {
                    !matches!(e.result.shape, kernel_api::FittedShape::Sphere { .. })
                }),
                [] => ctx
                    .selected_body_id
                    .is_some_and(|b| ctx.document.body_scan(BodyId(b)).is_some()),
                _ => false,
            };
        }
        if tool_id == SECTION {
            return match selected_elements(ctx).as_slice() {
                [one] => element_at(ctx.document, *one).is_some_and(|e| {
                    !matches!(e.result.shape, kernel_api::FittedShape::Sphere { .. })
                }),
                [] => ctx
                    .selected_body_id
                    .is_some_and(|b| ctx.document.body_scan(BodyId(b)).is_some()),
                _ => false,
            };
        }
        if matches!(tool_id, SAVE_STEP | SAVE_IGES) {
            return !self.construction_items(ctx.document, None).is_empty();
        }
        if tool_id == SAVE_REPORT {
            return ctx
                .document
                .feature_tree()
                .all_nodes()
                .any(|(_, n)| n.workbench_id.as_str() == element::KIND);
        }
        if tool_id == TOLERANCE {
            let picked: Vec<Element> = selected_elements(ctx)
                .into_iter()
                .filter_map(|id| element_at(ctx.document, id))
                .collect();
            return characteristic_for(&picked).is_some();
        }
        if tool_id == CLEARANCE {
            return compare_pair(ctx).is_some();
        }
        if tool_id == EXTRACT {
            let picked = selected_elements(ctx);
            return picked.len() == 1 && element_solid(ctx.document, picked[0], 2.0).is_ok();
        }
        if tool_id == THICKNESS || tool_id == OVERHANG {
            return thickness_body(ctx).is_some();
        }
        if tool_id == BEST_FIT {
            // While a fit runs, a click stops it.
            return self.tab.fitting.running() || compare_pair(ctx).is_some();
        }
        if tool_id == RESET_ALIGNMENT {
            return compare_pair(ctx)
                .map(|(scan, _)| scan)
                .or_else(|| ctx.selected_body_id.map(BodyId))
                .is_some_and(|b| {
                    ctx.document.body_scan(b).is_some()
                        && !ctx.document.body_placement(b).is_identity()
                });
        }
        if let Some(made) = made_of_tool(tool_id) {
            let picked: Vec<Element> = selected_elements(ctx)
                .into_iter()
                .filter_map(|id| element_at(ctx.document, id))
                .collect();
            let trial = Construction {
                name: String::new(),
                scan: BodyId(uuid::Uuid::nil()),
                made,
                from: Vec::new(),
                offset: 0.0,
            };
            return construction::build(&trial, &picked).is_ok();
        }
        match kind_of_tool(tool_id) {
            Some(_) => {
                ctx.current_marking().is_some()
                    || ctx
                        .document
                        .bodies()
                        .iter()
                        .any(|b| ctx.document.body_scan(b.id).is_some())
            }
            None => true,
        }
    }
}
