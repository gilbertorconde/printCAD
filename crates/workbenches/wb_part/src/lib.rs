//! Part Design workbench: feature-based solid modeling.
//!
//! The workbench edits the document's feature tree; the app shell watches
//! for dirty part features and drives the kernel rebuild (see `build.rs`).

mod build;
#[cfg(feature = "egui")]
mod editors;
mod feature;

pub use build::{
    body_build_ops, hole_diameter, mark_all_part_features_dirty, part_feature_ids,
    part_features_of_body, pending_body_rebuilds, retarget_feature_sketch,
    sketch_plane_description, sketches_of_body, BuildError, BuildPlan,
};
pub use feature::{
    ChamferMode, EdgeSel, ExtrudeMode, FacePick, HelixMode, HoleCut, HoleFit, MirrorPlane,
    PartFeature, PatternAxis, RevolveAxis, TransformStep, METRIC_SIZES,
};

use core_document::{
    BodyId, FeatureId, InputResult, ToolDescriptor, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchFeature, WorkbenchId, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};

/// Part Design workbench: feature-based solid modeling.
#[derive(Default)]
pub struct PartDesignWorkbench;

impl PartDesignWorkbench {
    /// The sketch feature currently selected in the tree, if any.
    fn selected_sketch(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == "wb.sketch").then_some(id)
    }

    /// The body the current selection belongs to: the selected feature's
    /// owning body, or the selected body itself.
    fn target_body(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
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

    fn body_has_solid(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        !part_features_of_body(ctx.document, body).is_empty()
    }

    fn next_feature_name(ctx: &WorkbenchRuntimeContext, base: &str) -> String {
        let count = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.name.starts_with(base))
            .count();
        if count == 0 {
            base.to_string()
        } else {
            format!("{base}_{count}")
        }
    }

    /// The last non-modifier feature of a body (default pattern original).
    fn last_shape_feature(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<FeatureId> {
        part_features_of_body(ctx.document, body)
            .into_iter()
            .rev()
            .find(|(_, f)| !f.is_modifier())
            .map(|(id, _)| id)
    }

    /// The current face pick, when the user has one selected in the viewport.
    fn selected_face_pick(ctx: &WorkbenchRuntimeContext) -> Option<FacePick> {
        ctx.selected_face.map(|face| FacePick {
            point: face.point,
            normal: face.normal,
        })
    }

    /// Build the default feature payload for a toolbar action, or explain why
    /// it can't be created from the current selection.
    fn feature_for_tool(
        tool: &str,
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Result<(PartFeature, &'static str), String> {
        let sketch = Self::selected_sketch(ctx);
        let need_sketch =
            |value: Option<FeatureId>| value.ok_or("Select a sketch in the tree first".to_string());
        let need_material = |ok: bool| {
            if ok {
                Ok(())
            } else {
                Err(
                    "This feature needs existing material; add a Pad or Revolution first"
                        .to_string(),
                )
            }
        };
        let has_solid = Self::body_has_solid(ctx, body);

        let feature = match tool {
            "part.pad" => (
                PartFeature::Pad {
                    sketch: need_sketch(sketch)?,
                    length: 10.0,
                    reversed: false,
                    symmetric: false,
                    mode: ExtrudeMode::Dimension,
                    length2: 10.0,
                    taper_deg: 0.0,
                    up_to_face: None,
                    up_to_offset: 0.0,
                },
                "Pad",
            ),
            "part.pocket" => {
                need_material(has_solid)?;
                (
                    PartFeature::Pocket {
                        sketch: need_sketch(sketch)?,
                        depth: 5.0,
                        reversed: false,
                        through_all: false,
                        mode: ExtrudeMode::Dimension,
                        depth2: 5.0,
                        taper_deg: 0.0,
                        up_to_face: None,
                        up_to_offset: 0.0,
                    },
                    "Pocket",
                )
            }
            "part.revolve" => (
                PartFeature::Revolution {
                    sketch: need_sketch(sketch)?,
                    angle_deg: 360.0,
                    axis: RevolveAxis::default(),
                    reversed: false,
                    midplane: false,
                    second_angle_deg: None,
                },
                "Revolution",
            ),
            "part.groove" => {
                need_material(has_solid)?;
                (
                    PartFeature::Groove {
                        sketch: need_sketch(sketch)?,
                        angle_deg: 360.0,
                        axis: RevolveAxis::default(),
                        reversed: false,
                        midplane: false,
                        second_angle_deg: None,
                    },
                    "Groove",
                )
            }
            "part.loft" => (
                PartFeature::Loft {
                    sections: vec![need_sketch(sketch)?],
                    ruled: false,
                    closed: false,
                    subtractive: false,
                },
                "Loft",
            ),
            "part.pipe" => (
                PartFeature::Pipe {
                    profile: need_sketch(sketch)?,
                    spine: need_sketch(sketch)?,
                    frenet: false,
                    subtractive: false,
                },
                "Pipe",
            ),
            "part.helix" => (
                PartFeature::Helix {
                    sketch: need_sketch(sketch)?,
                    axis: RevolveAxis::default(),
                    mode: HelixMode::PitchHeight,
                    pitch: 5.0,
                    height: 20.0,
                    turns: 4.0,
                    left_handed: false,
                    cone_angle_deg: 0.0,
                    reversed: false,
                    subtractive: false,
                },
                "Helix",
            ),
            "part.primitive" => (
                PartFeature::Primitive {
                    kind: kernel_api::PrimitiveKind::Box {
                        length: 10.0,
                        width: 10.0,
                        height: 10.0,
                    },
                    placement: kernel_api::Placement::default(),
                    subtractive: false,
                },
                "Primitive",
            ),
            "part.hole" => {
                need_material(has_solid)?;
                (
                    PartFeature::Hole {
                        sketch: need_sketch(sketch)?,
                        diameter: 5.0,
                        depth: 10.0,
                        through_all: false,
                        cut: HoleCut::None,
                        metric_index: None,
                        threaded: false,
                        fit: HoleFit::Normal,
                        reversed: false,
                    },
                    "Hole",
                )
            }
            "part.fillet" => {
                need_material(has_solid)?;
                let edges = match Self::selected_face_pick(ctx) {
                    Some(pick) => EdgeSel::Faces(vec![pick]),
                    None => EdgeSel::All,
                };
                (PartFeature::Fillet { radius: 1.0, edges }, "Fillet")
            }
            "part.chamfer" => {
                need_material(has_solid)?;
                let edges = match Self::selected_face_pick(ctx) {
                    Some(pick) => EdgeSel::Faces(vec![pick]),
                    None => EdgeSel::All,
                };
                (
                    PartFeature::Chamfer {
                        size: 1.0,
                        mode: ChamferMode::EqualDistance,
                        size2: 1.0,
                        angle_deg: 45.0,
                        flip: false,
                        edges,
                    },
                    "Chamfer",
                )
            }
            "part.draft" => {
                need_material(has_solid)?;
                let pick = Self::selected_face_pick(ctx)
                    .ok_or("Click a face in the viewport first (the neutral plane)")?;
                (
                    PartFeature::Draft {
                        angle_deg: 1.5,
                        neutral: pick,
                        faces: Vec::new(),
                        reversed: false,
                    },
                    "Draft",
                )
            }
            "part.thickness" => {
                need_material(has_solid)?;
                let pick = Self::selected_face_pick(ctx)
                    .ok_or("Click the face to open in the viewport first")?;
                (
                    PartFeature::Thickness {
                        value: 1.0,
                        faces: vec![pick],
                        inward: true,
                    },
                    "Thickness",
                )
            }
            "part.mirror" => {
                need_material(has_solid)?;
                let original = Self::selected_part_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    PartFeature::Mirrored {
                        originals: original.into_iter().collect(),
                        plane: MirrorPlane::YZ,
                    },
                    "Mirrored",
                )
            }
            "part.linear_pattern" => {
                need_material(has_solid)?;
                let original = Self::selected_part_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    PartFeature::LinearPattern {
                        originals: original.into_iter().collect(),
                        axis: PatternAxis::X,
                        length: 30.0,
                        occurrences: 3,
                        spacing_mode: false,
                        reversed: false,
                    },
                    "LinearPattern",
                )
            }
            "part.polar_pattern" => {
                need_material(has_solid)?;
                let original = Self::selected_part_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    PartFeature::PolarPattern {
                        originals: original.into_iter().collect(),
                        axis: PatternAxis::Z,
                        angle_deg: 360.0,
                        occurrences: 4,
                        reversed: false,
                    },
                    "PolarPattern",
                )
            }
            "part.multi_transform" => {
                need_material(has_solid)?;
                let original = Self::selected_part_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    PartFeature::MultiTransform {
                        originals: original.into_iter().collect(),
                        steps: Vec::new(),
                    },
                    "MultiTransform",
                )
            }
            "part.boolean" => {
                need_material(has_solid)?;
                let other = ctx
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.id != body)
                    .map(|b| b.id)
                    .ok_or("Create a second body to combine with first")?;
                (
                    PartFeature::BodyBoolean {
                        tool_body: other,
                        kind: kernel_api::BoolKind::Fuse,
                    },
                    "Boolean",
                )
            }
            _ => return Err(format!("unknown tool {tool}")),
        };
        Ok(feature)
    }

    /// Create a datum feature anchored to the selected face (or the XY base
    /// plane) and select it for editing.
    fn insert_datum(&self, ctx: &mut WorkbenchRuntimeContext, tool: &str) -> InputResult {
        use core_document::{AttachmentOffset, DatumAttachment, DatumFeature, DatumShape};
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        let shape = match tool {
            "part.datum_plane" => DatumShape::Plane { size: 30.0 },
            "part.datum_line" => DatumShape::Line { length: 40.0 },
            _ => DatumShape::Point,
        };
        let attachment = match ctx.selected_face {
            Some(face) => DatumAttachment::FlatFace {
                point: face.point,
                normal: face.normal,
            },
            None => DatumAttachment::BasePlane(core_document::BasePlane::XY),
        };
        let datum = DatumFeature {
            shape,
            attachment,
            offset: AttachmentOffset::default(),
        };
        let name = Self::next_feature_name(ctx, shape.label());
        match ctx
            .document
            .add_feature_in_body(datum, name.clone(), Some(body))
        {
            Ok(feature_id) => {
                ctx.active_document_object = Some(feature_id);
                ctx.log_info(format!("Created {name}"));
            }
            Err(e) => ctx.log_error(format!("Failed to create datum: {e}")),
        }
        InputResult::consumed()
    }

    /// Create a feature from a toolbar action and mark it for rebuild.
    fn insert_feature(&self, ctx: &mut WorkbenchRuntimeContext, tool: &str) -> InputResult {
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        let (feature, base) = match Self::feature_for_tool(tool, ctx, body) {
            Ok(pair) => pair,
            Err(message) => {
                ctx.log_warn(message);
                return InputResult::consumed();
            }
        };
        let name = Self::next_feature_name(ctx, base);
        let sketches = feature.sketches();

        match ctx
            .document
            .add_feature_in_body(feature, name.clone(), Some(body))
        {
            Ok(feature_id) => {
                ctx.document.mark_feature_dirty(feature_id);
                // Consumed sketches are hidden; the solid takes over visually.
                for sketch in sketches {
                    ctx.document.set_feature_visible(sketch, false);
                }
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
        let structure = [
            ("part.new_body", "New Body"),
            ("part.new_sketch", "New Sketch"),
            ("part.datum_plane", "Datum Plane"),
            ("part.datum_line", "Datum Line"),
            ("part.datum_point", "Datum Point"),
        ];
        let modeling = [
            ("part.pad", "Pad (Extrude)"),
            ("part.pocket", "Pocket (Cut)"),
            ("part.revolve", "Revolution"),
            ("part.groove", "Groove (Revolved Cut)"),
            ("part.loft", "Loft"),
            ("part.pipe", "Pipe (Sweep)"),
            ("part.helix", "Helix"),
            ("part.primitive", "Primitive"),
            ("part.hole", "Hole"),
        ];
        let dressup = [
            ("part.fillet", "Fillet"),
            ("part.chamfer", "Chamfer"),
            ("part.draft", "Draft"),
            ("part.thickness", "Thickness (Shell)"),
        ];
        let transform = [
            ("part.mirror", "Mirrored"),
            ("part.linear_pattern", "Linear Pattern"),
            ("part.polar_pattern", "Polar Pattern"),
            ("part.multi_transform", "Multi Transform"),
            ("part.boolean", "Boolean"),
        ];
        for (id, label) in structure {
            context.register_tool(ToolDescriptor::new_action(id, label, Some("structure")));
        }
        for (id, label) in modeling {
            context.register_tool(ToolDescriptor::new_action(id, label, Some("modeling")));
        }
        for (id, label) in dressup {
            context.register_tool(ToolDescriptor::new_action(id, label, Some("dressup")));
        }
        for (id, label) in transform {
            context.register_tool(ToolDescriptor::new_action(id, label, Some("transform")));
        }
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
        // Feature tools are Actions: they fire once on the first input event
        // after the toolbar click (the host clears consumed actions).
        // `part.new_body` is handled host-side.
        match active_tool {
            Some(tool @ ("part.datum_plane" | "part.datum_line" | "part.datum_point")) => {
                self.insert_datum(ctx, tool)
            }
            Some("part.new_sketch") => {
                let Some(body) = Self::target_body(ctx) else {
                    ctx.log_warn("Select a body (or one of its features) first");
                    return InputResult::consumed();
                };
                // Hand off to the sketch workbench: it opens its plane picker
                // for this body (offering the clicked face when the selection
                // landed on solid geometry), and finishing the sketch returns
                // here (the host tracks the return bench).
                ctx.start_sketch_on_body = Some(core_document::SketchAttachRequest {
                    body: body.0,
                    face: ctx.selected_face,
                });
                ctx.workbench_switch_request = Some(WorkbenchId::from("wb.sketch"));
                InputResult::consumed()
            }
            Some(tool) if tool.starts_with("part.") && tool != "part.new_body" => {
                self.insert_feature(ctx, tool)
            }
            _ => InputResult::ignored(),
        }
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let body = Self::target_body(ctx);
        let has_body = body.is_some();
        let has_sketch = Self::selected_sketch(ctx).is_some();
        let has_solid = body.map(|b| Self::body_has_solid(ctx, b)).unwrap_or(false);
        match tool_id {
            "part.new_body" => true,
            "part.new_sketch" | "part.primitive" | "part.datum_plane" | "part.datum_line"
            | "part.datum_point" => has_body,
            "part.pad" | "part.revolve" | "part.loft" | "part.pipe" | "part.helix" => has_sketch,
            "part.pocket" | "part.groove" | "part.hole" => has_sketch && has_solid,
            "part.fillet"
            | "part.chamfer"
            | "part.draft"
            | "part.thickness"
            | "part.mirror"
            | "part.linear_pattern"
            | "part.polar_pattern"
            | "part.multi_transform"
            | "part.boolean" => has_solid,
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
        let mut removed: Option<(FeatureId, Vec<FeatureId>)> = None;
        let mut suppress_toggle: Option<(FeatureId, bool)> = None;

        for (feature_id, part_feature) in &features {
            let (node_name, suppressed, has_error) = ctx
                .document
                .get_feature_meta(*feature_id)
                .map(|n| (n.name.clone(), n.suppressed, n.error.is_some()))
                .unwrap_or_else(|| (part_feature.kind_label().to_string(), false, false));
            let is_active = ctx.active_document_object == Some(*feature_id);
            ui.horizontal(|ui| {
                if ui
                    .small_button("✕")
                    .on_hover_text("Delete feature")
                    .clicked()
                {
                    removed = Some((*feature_id, part_feature.sketches()));
                }
                let mut label = egui::RichText::new(&node_name);
                if has_error {
                    label = label.color(egui::Color32::from_rgb(240, 90, 90));
                }
                if suppressed {
                    label = label.strikethrough();
                }
                let response = ui
                    .selectable_label(is_active, label)
                    .on_hover_text("Click to edit this operation's settings");
                if response.clicked() {
                    ctx.active_document_object = Some(*feature_id);
                }
                let mut is_suppressed = suppressed;
                if ui
                    .checkbox(&mut is_suppressed, "off")
                    .on_hover_text("Suppress: exclude this feature from the build")
                    .changed()
                {
                    suppress_toggle = Some((*feature_id, is_suppressed));
                }
            });
            if has_error {
                if let Some(message) = ctx
                    .document
                    .get_feature_meta(*feature_id)
                    .and_then(|n| n.error.clone())
                {
                    ui.colored_label(egui::Color32::from_rgb(240, 90, 90), message);
                }
            }
        }

        if let Some((feature_id, suppressed)) = suppress_toggle {
            ctx.document.set_feature_suppressed(feature_id, suppressed);
            ctx.document.mark_feature_dirty(feature_id);
        }

        // ---- Detail editor for the operation selected in the tree ----
        if let Some(feature_id) = Self::selected_part_feature(ctx) {
            if let Some((mut part_feature, node_name)) = ctx
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
                if let Some(sketch_id) = part_feature.sketch() {
                    ui.label(format!(
                        "Plane: {}",
                        sketch_plane_description(ctx.document, sketch_id)
                    ));
                }

                let deps_before = part_feature.dependencies();
                if editors::feature_editor(ui, ctx, body, feature_id, &mut part_feature) {
                    let deps_after = part_feature.dependencies();
                    if ctx
                        .document
                        .update_feature_data(feature_id, part_feature.to_json())
                        .is_ok()
                    {
                        if deps_before != deps_after {
                            ctx.document
                                .set_feature_dependencies(feature_id, deps_after);
                        }
                        ctx.document.mark_feature_dirty(feature_id);
                    }
                }
            }
        }

        // ---- Datum editor when a datum is selected in the tree ----
        if let Some(datum_id) = ctx.active_document_object.filter(|id| {
            ctx.document
                .get_feature_meta(*id)
                .map(|n| n.workbench_id.as_str() == "core.datum" && n.body == Some(body))
                .unwrap_or(false)
        }) {
            if let Some(mut datum) = ctx
                .document
                .get_feature_data(datum_id)
                .and_then(|d| core_document::DatumFeature::from_json(d).ok())
            {
                ui.separator();
                ui.heading(datum.shape.label());
                if editors::datum_editor(ui, ctx, datum_id, &mut datum) {
                    let _ = ctx.document.update_feature_data(datum_id, datum.to_json());
                    // Sketches attached to this datum re-derive their plane
                    // from it on their next edit; solids are unaffected.
                }
                if ui
                    .small_button("Delete datum")
                    .on_hover_text("Remove this datum")
                    .clicked()
                    && ctx.document.remove_feature(datum_id).is_ok()
                {
                    ctx.active_document_object = None;
                }
            }
        }

        if let Some((feature_id, sketches)) = removed {
            if ctx.document.remove_feature(feature_id).is_ok() {
                ctx.log_info("Deleted feature");
                // Reveal consumed sketches again so they can be reused.
                for sketch_id in sketches {
                    ctx.document.set_feature_visible(sketch_id, true);
                }
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

    fn get_overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        active_feature: Option<FeatureId>,
    ) -> Vec<(kernel_api::TriMesh, [f32; 3], bool)> {
        let Some(body) = Self::target_body(ctx) else {
            return Vec::new();
        };
        let mut meshes = Vec::new();
        for (id, _, datum) in core_document::datums_of_body(ctx.document, body) {
            let visible = ctx
                .document
                .get_feature_meta(id)
                .map(|n| n.visible)
                .unwrap_or(true);
            if !visible {
                continue;
            }
            let color = if active_feature == Some(id) {
                [1.0, 0.75, 0.2]
            } else {
                [0.55, 0.55, 0.95]
            };
            meshes.push((datum_mesh(&datum), color, true));
        }
        meshes
    }
}

/// Wireframe visualization mesh for a datum in world space.
fn datum_mesh(datum: &core_document::DatumFeature) -> kernel_api::TriMesh {
    let frame = datum.frame();
    let o = frame.origin;
    let x = frame.x_axis;
    let y = frame.y_axis();
    let n = frame.normal;
    let at = |sx: f32, sy: f32, sn: f32| -> [f32; 3] {
        [
            o[0] + x[0] * sx + y[0] * sy + n[0] * sn,
            o[1] + x[1] * sx + y[1] * sy + n[1] * sn,
            o[2] + x[2] * sx + y[2] * sy + n[2] * sn,
        ]
    };
    let mut mesh = kernel_api::TriMesh::default();
    match datum.shape {
        core_document::DatumShape::Plane { size } => {
            let h = size * 0.5;
            mesh.positions = vec![
                at(-h, -h, 0.0),
                at(h, -h, 0.0),
                at(h, h, 0.0),
                at(-h, h, 0.0),
            ];
            mesh.normals = vec![n; 4];
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            // Border + diagonal edges make the plane readable as wireframe.
            mesh.edges = vec![0, 1, 1, 2, 2, 3, 3, 0, 0, 2];
        }
        core_document::DatumShape::Line { length } => {
            let h = length * 0.5;
            // A degenerate-thin quad along the x-axis; the edge list is what
            // the viewer actually reads.
            mesh.positions = vec![at(-h, 0.0, 0.0), at(h, 0.0, 0.0), at(h, 0.2, 0.0)];
            mesh.normals = vec![n; 3];
            mesh.indices = vec![0, 1, 2];
            mesh.edges = vec![0, 1];
        }
        core_document::DatumShape::Point => {
            let s = 1.5;
            mesh.positions = vec![
                at(-s, 0.0, 0.0),
                at(s, 0.0, 0.0),
                at(0.0, -s, 0.0),
                at(0.0, s, 0.0),
                at(0.0, 0.0, -s),
                at(0.0, 0.0, s),
            ];
            mesh.normals = vec![n; 6];
            mesh.indices = vec![0, 1, 2, 3, 4, 5];
            mesh.edges = vec![0, 1, 2, 3, 4, 5];
        }
    }
    mesh
}
