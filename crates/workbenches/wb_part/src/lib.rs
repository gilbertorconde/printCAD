//! Part Design workbench: sketch-based solid modeling (Pad / Pocket).
//!
//! The workbench edits the document's feature tree; the app shell watches
//! for dirty part features and drives the kernel rebuild (see `build.rs`).

mod build;
mod feature;

pub use build::{
    body_build_ops, mark_all_part_features_dirty, part_feature_ids, part_features_of_body,
    pending_body_rebuilds,
};
pub use feature::PartFeature;

use core_document::{
    BodyId, FeatureId, InputResult, ToolDescriptor, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchFeature, WorkbenchId, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};

/// Part Design workbench: feature-based solid modeling.
#[derive(Default)]
pub struct PartDesignWorkbench;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtrudeKind {
    Pad,
    Pocket,
}

impl PartDesignWorkbench {
    /// The sketch feature currently selected in the tree, if any.
    fn selected_sketch(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == "wb.sketch").then_some(id)
    }

    /// The body the current selection belongs to: the selected sketch's
    /// owning body, or the selected body itself.
    fn target_body(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        if let Some(sketch_id) = Self::selected_sketch(ctx) {
            if let Some(node) = ctx.document.get_feature_meta(sketch_id) {
                if node.body.is_some() {
                    return node.body;
                }
            }
        }
        ctx.selected_body_id.map(BodyId)
    }

    fn next_feature_name(ctx: &WorkbenchRuntimeContext, base: &str) -> String {
        let count = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == "wb.part" && n.name.starts_with(base))
            .count();
        if count == 0 {
            base.to_string()
        } else {
            format!("{base}_{count}")
        }
    }

    /// Create a Pad/Pocket from the selected sketch and mark it for rebuild.
    fn insert_extrude(&self, ctx: &mut WorkbenchRuntimeContext, kind: ExtrudeKind) -> InputResult {
        let Some(sketch_id) = Self::selected_sketch(ctx) else {
            ctx.log_warn("Select a sketch in the tree first");
            return InputResult::consumed();
        };
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("The selected sketch does not belong to a body");
            return InputResult::consumed();
        };
        if kind == ExtrudeKind::Pocket && part_features_of_body(ctx.document, body).is_empty() {
            ctx.log_warn("Pocket needs existing material; add a Pad first");
            return InputResult::consumed();
        }

        let (feature, base) = match kind {
            ExtrudeKind::Pad => (
                PartFeature::Pad {
                    sketch: sketch_id,
                    length: 10.0,
                    reversed: false,
                },
                "Pad",
            ),
            ExtrudeKind::Pocket => (
                PartFeature::Pocket {
                    sketch: sketch_id,
                    depth: 5.0,
                    reversed: false,
                },
                "Pocket",
            ),
        };
        let name = Self::next_feature_name(ctx, base);

        match ctx
            .document
            .add_feature_in_body(feature, name.clone(), Some(body))
        {
            Ok(feature_id) => {
                ctx.document.mark_feature_dirty(feature_id);
                // FreeCAD-style: the consumed sketch is hidden; the solid
                // takes over visually.
                ctx.document.set_feature_visible(sketch_id, false);
                ctx.active_document_object = Some(feature_id);
                ctx.log_info(format!("Created {name}"));
            }
            Err(e) => ctx.log_error(format!("Failed to create {base}: {e}")),
        }
        InputResult::consumed()
    }
}

impl Workbench for PartDesignWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.part",
            "Part Design",
            "Feature-based solid modeling workbench.",
        )
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        context.register_tool(ToolDescriptor::new_action(
            "part.new_body",
            "New Body",
            Some("structure"),
        ));
        context.register_tool(ToolDescriptor::new_action(
            "part.pad",
            "Pad (Extrude)",
            Some("modeling"),
        ));
        context.register_tool(ToolDescriptor::new_action(
            "part.pocket",
            "Pocket (Cut)",
            Some("modeling"),
        ));
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Part Design workbench activated");
    }

    fn on_input(
        &mut self,
        _event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        // Pad/Pocket are Action tools: they fire once on the first input
        // event after the toolbar click (the host clears consumed actions).
        // `part.new_body` is handled host-side.
        match active_tool {
            Some("part.pad") => self.insert_extrude(ctx, ExtrudeKind::Pad),
            Some("part.pocket") => self.insert_extrude(ctx, ExtrudeKind::Pocket),
            _ => InputResult::ignored(),
        }
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            "part.new_body" => true,
            "part.pad" => Self::selected_sketch(ctx).is_some(),
            "part.pocket" => {
                let Some(body) = Self::target_body(ctx) else {
                    return false;
                };
                Self::selected_sketch(ctx).is_some()
                    && !part_features_of_body(ctx.document, body).is_empty()
            }
            _ => false,
        }
    }

    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        ui.heading("Part Design");

        let Some(body) = Self::target_body(ctx) else {
            ui.label("Create a body, then a sketch on it, then Pad the sketch.");
            ui.label("Select a body or sketch in the tree to see its features.");
            return;
        };
        let body_name = ctx
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Body".to_string());
        ui.label(format!("Features of {body_name}"));
        ui.separator();

        let features = part_features_of_body(ctx.document, body);
        if features.is_empty() {
            ui.label("No features yet.");
            ui.label("Select a sketch and use Pad to create a solid.");
            return;
        }

        // Collect edits first; apply after the iteration ends.
        let mut updated: Option<(FeatureId, PartFeature)> = None;
        let mut removed: Option<(FeatureId, FeatureId)> = None; // (feature, its sketch)

        for (feature_id, part_feature) in &features {
            let node_name = ctx
                .document
                .get_feature_meta(*feature_id)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| part_feature.kind_label().to_string());
            let mut edited = part_feature.clone();
            let changed = ui
                .horizontal(|ui| {
                    if ui
                        .small_button("✕")
                        .on_hover_text("Delete feature")
                        .clicked()
                    {
                        removed = Some((*feature_id, part_feature.sketch()));
                    }
                    ui.label(&node_name);
                    let mut changed = false;
                    let (value, reversed) = match &mut edited {
                        PartFeature::Pad {
                            length, reversed, ..
                        } => (length, reversed),
                        PartFeature::Pocket {
                            depth, reversed, ..
                        } => (depth, reversed),
                    };
                    changed |= ui
                        .add(
                            egui::DragValue::new(value)
                                .speed(0.5)
                                .range(0.01..=1.0e6)
                                .suffix(" mm"),
                        )
                        .changed();
                    changed |= ui
                        .checkbox(reversed, "rev")
                        .on_hover_text("Extrude in the opposite direction")
                        .changed();
                    changed
                })
                .inner;
            if changed {
                updated = Some((*feature_id, edited));
            }
        }

        if let Some((feature_id, edited)) = updated {
            if ctx
                .document
                .update_feature_data(feature_id, edited.to_json())
                .is_ok()
            {
                ctx.document.mark_feature_dirty(feature_id);
            }
        }
        if let Some((feature_id, sketch_id)) = removed {
            if ctx.document.remove_feature(feature_id).is_ok() {
                ctx.log_info("Deleted feature");
                // Reveal the sketch again so it can be reused/edited.
                ctx.document.set_feature_visible(sketch_id, true);
                let remaining = part_feature_ids(ctx.document, body);
                match remaining.first() {
                    // Rebuild the rest of the history.
                    Some(first) => ctx.document.mark_feature_dirty(*first),
                    // Last feature gone: the body has no solid any more.
                    None => ctx.document.remove_imported_geometry(body),
                }
            }
        }
    }

    fn feature_dependencies(
        &self,
        _workbench_id: &WorkbenchId,
        data: &serde_json::Value,
    ) -> Vec<FeatureId> {
        PartFeature::from_json(data)
            .map(|f| f.dependencies())
            .unwrap_or_default()
    }
}
