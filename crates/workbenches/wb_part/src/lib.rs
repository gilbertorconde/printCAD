//! Part Design workbench: sketch-based solid modeling (Pad / Pocket).
//!
//! The workbench edits the document's feature tree; the app shell watches
//! for dirty part features and drives the kernel rebuild (see `build.rs`).

mod build;
mod feature;

pub use build::{
    body_build_ops, mark_all_part_features_dirty, part_feature_ids, part_features_of_body,
    pending_body_rebuilds, retarget_feature_sketch, sketch_plane_description, sketches_of_body,
};
pub use feature::{PartFeature, RevolveAxis};

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
    Revolution,
    Groove,
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
        // Any selected feature (sketch OR part operation) carries its body.
        if let Some(id) = ctx.active_document_object {
            if let Some(node) = ctx.document.get_feature_meta(id) {
                if node.body.is_some() {
                    return node.body;
                }
            }
        }
        ctx.selected_body_id.map(BodyId)
    }

    /// The part feature currently selected in the tree, if any.
    fn selected_part_feature(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == "wb.part").then_some(id)
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
        let subtractive = matches!(kind, ExtrudeKind::Pocket | ExtrudeKind::Groove);
        if subtractive && part_features_of_body(ctx.document, body).is_empty() {
            ctx.log_warn("This feature needs existing material; add a Pad or Revolution first");
            return InputResult::consumed();
        }

        let (feature, base) = match kind {
            ExtrudeKind::Pad => (
                PartFeature::Pad {
                    sketch: sketch_id,
                    length: 10.0,
                    reversed: false,
                    symmetric: false,
                },
                "Pad",
            ),
            ExtrudeKind::Pocket => (
                PartFeature::Pocket {
                    sketch: sketch_id,
                    depth: 5.0,
                    reversed: false,
                    through_all: false,
                },
                "Pocket",
            ),
            ExtrudeKind::Revolution => (
                PartFeature::Revolution {
                    sketch: sketch_id,
                    angle_deg: 360.0,
                    axis: RevolveAxis::default(),
                    reversed: false,
                },
                "Revolution",
            ),
            ExtrudeKind::Groove => (
                PartFeature::Groove {
                    sketch: sketch_id,
                    angle_deg: 360.0,
                    axis: RevolveAxis::default(),
                    reversed: false,
                },
                "Groove",
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
            "part.new_sketch",
            "New Sketch",
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
        context.register_tool(ToolDescriptor::new_action(
            "part.revolve",
            "Revolution",
            Some("modeling"),
        ));
        context.register_tool(ToolDescriptor::new_action(
            "part.groove",
            "Groove (Revolved Cut)",
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
            Some("part.revolve") => self.insert_extrude(ctx, ExtrudeKind::Revolution),
            Some("part.groove") => self.insert_extrude(ctx, ExtrudeKind::Groove),
            Some("part.new_sketch") => {
                let Some(body) = Self::target_body(ctx) else {
                    ctx.log_warn("Select a body (or one of its features) first");
                    return InputResult::consumed();
                };
                // Hand off to the sketch workbench: it opens its plane
                // picker for this body (offering the clicked face when the
                // selection landed on solid geometry), and finishing the
                // sketch returns here (the host tracks the return bench).
                ctx.start_sketch_on_body = Some(core_document::SketchAttachRequest {
                    body: body.0,
                    face: ctx.selected_face,
                });
                ctx.workbench_switch_request = Some(WorkbenchId::from("wb.sketch"));
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            "part.new_body" => true,
            "part.new_sketch" => Self::target_body(ctx).is_some(),
            "part.pad" | "part.revolve" => Self::selected_sketch(ctx).is_some(),
            "part.pocket" | "part.groove" => {
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
        // Body rename.
        let mut edited_body_name = body_name.clone();
        ui.horizontal(|ui| {
            ui.label("Body:");
            if ui
                .add(egui::TextEdit::singleline(&mut edited_body_name).desired_width(140.0))
                .lost_focus()
                && edited_body_name != body_name
            {
                ctx.document.rename_body(body, edited_body_name.clone());
            }
        });
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

        let mut suppress_toggle: Option<(FeatureId, bool)> = None;

        for (feature_id, part_feature) in &features {
            let (node_name, suppressed) = ctx
                .document
                .get_feature_meta(*feature_id)
                .map(|n| (n.name.clone(), n.suppressed))
                .unwrap_or_else(|| (part_feature.kind_label().to_string(), false));
            let mut edited = part_feature.clone();
            let is_active = ctx.active_document_object == Some(*feature_id);
            let changed = ui
                .horizontal(|ui| {
                    if ui
                        .small_button("✕")
                        .on_hover_text("Delete feature")
                        .clicked()
                    {
                        removed = Some((*feature_id, part_feature.sketch()));
                    }
                    if ui
                        .selectable_label(is_active, &node_name)
                        .on_hover_text("Click to edit this operation's settings")
                        .clicked()
                    {
                        ctx.active_document_object = Some(*feature_id);
                    }
                    let mut changed = false;
                    match &mut edited {
                        PartFeature::Pad {
                            length,
                            reversed,
                            symmetric,
                            ..
                        } => {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(length)
                                        .speed(0.5)
                                        .range(0.01..=1.0e6)
                                        .suffix(" mm"),
                                )
                                .changed();
                            changed |= ui
                                .checkbox(reversed, "rev")
                                .on_hover_text("Extrude in the opposite direction")
                                .changed();
                            changed |= ui
                                .checkbox(symmetric, "sym")
                                .on_hover_text("Extrude half to each side of the plane")
                                .changed();
                        }
                        PartFeature::Pocket {
                            depth,
                            reversed,
                            through_all,
                            ..
                        } => {
                            changed |= ui
                                .add_enabled(
                                    !*through_all,
                                    egui::DragValue::new(depth)
                                        .speed(0.5)
                                        .range(0.01..=1.0e6)
                                        .suffix(" mm"),
                                )
                                .changed();
                            changed |= ui
                                .checkbox(reversed, "rev")
                                .on_hover_text("Cut in the opposite direction")
                                .changed();
                            changed |= ui
                                .checkbox(through_all, "thru")
                                .on_hover_text("Cut through the entire body")
                                .changed();
                        }
                        PartFeature::Revolution {
                            angle_deg,
                            axis,
                            reversed,
                            ..
                        }
                        | PartFeature::Groove {
                            angle_deg,
                            axis,
                            reversed,
                            ..
                        } => {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(angle_deg)
                                        .speed(1.0)
                                        .range(0.1..=360.0)
                                        .suffix("°"),
                                )
                                .changed();
                            let other = match axis {
                                RevolveAxis::SketchY => RevolveAxis::SketchX,
                                RevolveAxis::SketchX => RevolveAxis::SketchY,
                            };
                            if ui
                                .button(match axis {
                                    RevolveAxis::SketchY => "Y",
                                    RevolveAxis::SketchX => "X",
                                })
                                .on_hover_text(format!("Axis: {} (click to switch)", axis.label()))
                                .clicked()
                            {
                                *axis = other;
                                changed = true;
                            }
                            changed |= ui
                                .checkbox(reversed, "rev")
                                .on_hover_text("Revolve the other way around the axis")
                                .changed();
                        }
                    }
                    let mut is_suppressed = suppressed;
                    if ui
                        .checkbox(&mut is_suppressed, "off")
                        .on_hover_text("Suppress: exclude this feature from the build")
                        .changed()
                    {
                        suppress_toggle = Some((*feature_id, is_suppressed));
                    }
                    changed
                })
                .inner;
            if changed {
                updated = Some((*feature_id, edited));
            }
        }

        if let Some((feature_id, suppressed)) = suppress_toggle {
            ctx.document.set_feature_suppressed(feature_id, suppressed);
            ctx.document.mark_feature_dirty(feature_id);
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
        // ---- Detail editor for the operation selected in the tree ----
        if let Some(feature_id) = Self::selected_part_feature(ctx) {
            if let Some((part_feature, node_name)) = ctx
                .document
                .get_feature_meta(feature_id)
                .filter(|n| n.body == Some(body))
                .map(|n| (PartFeature::from_json(&n.data).ok(), n.name.clone()))
                .and_then(|(f, n)| f.map(|f| (f, n)))
            {
                ui.separator();
                ui.heading(format!("{} settings", part_feature.kind_label()));

                let mut edited_name = node_name.clone();
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    if ui
                        .add(egui::TextEdit::singleline(&mut edited_name).desired_width(140.0))
                        .lost_focus()
                        && edited_name != node_name
                    {
                        ctx.document.rename_feature(feature_id, edited_name);
                    }
                });

                // Attachment: which sketch, on which plane — retargetable.
                let sketch_id = part_feature.sketch();
                let sketches = sketches_of_body(ctx.document, body);
                let current_name = sketches
                    .iter()
                    .find(|(id, _)| *id == sketch_id)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| "(missing)".to_string());
                let mut retarget: Option<FeatureId> = None;
                ui.horizontal(|ui| {
                    ui.label("Sketch:");
                    egui::ComboBox::from_id_salt(("feature_sketch", feature_id))
                        .selected_text(current_name)
                        .show_ui(ui, |ui| {
                            for (id, name) in &sketches {
                                if ui.selectable_label(*id == sketch_id, name).clicked()
                                    && *id != sketch_id
                                {
                                    retarget = Some(*id);
                                }
                            }
                        });
                });
                ui.label(format!(
                    "Plane: {}",
                    sketch_plane_description(ctx.document, sketch_id)
                ));
                if let Some(new_sketch) = retarget {
                    match retarget_feature_sketch(ctx.document, feature_id, new_sketch) {
                        Ok(()) => ctx.log_info("Re-attached feature to a different sketch"),
                        Err(e) => ctx.log_error(format!("Re-attach failed: {e}")),
                    }
                }
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
