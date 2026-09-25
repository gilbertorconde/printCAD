//! Part Design workbench: feature-based solid modeling.
//!
//! The workbench edits the document's feature tree; the app shell watches
//! for dirty part features and drives the kernel rebuild (see `build.rs`).

mod build;
mod commands;
#[cfg(feature = "egui")]
mod editors;
mod feature;
mod params;
#[cfg(feature = "egui")]
mod task;

pub use build::{
    BuildError, BuildPlan, body_build_ops, delete_feature, hole_diameter, invalidate_body,
    mark_all_part_features_dirty, part_feature_ids, part_features_of_body, pending_body_rebuilds,
    rebuild_jobs, retarget_feature_sketch, sketch_plane_description, sketches_of_body,
};
pub use feature::{
    ChamferMode, EdgePick, EdgeSel, ExtrudeMode, FacePick, HelixMode, HoleCut, HoleFit,
    METRIC_SIZES, MirrorPlane, PartFeature, PatternAxis, RevolveAxis, TransformStep,
    primitive_icon, primitive_preset,
};

use core_document::{
    BodyId, Document, FeatureId, FeatureInfo, HostRequest, InputResult, TaskInfo, ToolDescriptor,
    ToolVariant, Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchId,
    WorkbenchInputEvent, WorkbenchRuntimeContext, base_tool_id, tool_variant,
};
use wb_sketch::SketchFeature;

/// The switches on the Part Design Preferences page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PartOptions {
    /// The preview rebuilds on every field change in the task panel; off,
    /// it rebuilds when the task is accepted.
    pub update_while_editing: bool,
    /// A sketch a new feature consumes is hidden.
    pub hide_used_sketches: bool,
    /// A new feature that fuses or cuts merges the coplanar faces it leaves.
    pub refine_result: bool,
}

impl Default for PartOptions {
    fn default() -> Self {
        Self {
            update_while_editing: true,
            hide_used_sketches: true,
            refine_result: true,
        }
    }
}

/// Part Design workbench: feature-based solid modeling.
#[derive(Default)]
pub struct PartDesignWorkbench {
    /// The Preferences page's switches.
    pub options: PartOptions,
    /// The feature open in the task panel.
    #[cfg(feature = "egui")]
    task: Option<task::TaskState>,
    /// A feature a tool just created: the task that opens for it deletes it
    /// on Cancel, and records it on OK as the command that makes it.
    pending_task_from_tool: Option<ToolMade>,
}

/// What a tool just made, for the task that opens on it.
#[derive(Debug, Clone)]
pub(crate) struct ToolMade {
    pub feature: FeatureId,
    /// The sketches it hid, shown again when it is cancelled.
    pub hidden: Vec<FeatureId>,
    /// The tool, with its variant (`part.primitive:box`).
    pub tool: String,
    /// The body it was made for.
    pub body: BodyId,
}

/// Primitive shapes offered from the primitive tools' dropdowns.
const PRIMITIVE_SHAPES: &[(&str, &str)] = &[
    ("box", "Box"),
    ("cylinder", "Cylinder"),
    ("sphere", "Sphere"),
    ("cone", "Cone"),
    ("torus", "Torus"),
    ("ellipsoid", "Ellipsoid"),
    ("prism", "Prism"),
    ("wedge", "Wedge"),
];

fn primitive_variants(subtractive: bool) -> Vec<ToolVariant> {
    PRIMITIVE_SHAPES
        .iter()
        .map(|(id, label)| ToolVariant::new(id, label, primitive_icon(id, subtractive)))
        .collect()
}

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
        if let Some(id) = ctx.active_document_object
            && let Some(node) = ctx.document.get_feature_meta(id)
            && node.body.is_some()
        {
            return node.body;
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

    /// `base` when no feature has that name, else `base_n` one past the
    /// highest `n` in use: a name never comes back while another has it.
    pub(crate) fn next_feature_name(ctx: &WorkbenchRuntimeContext, base: &str) -> String {
        next_name(
            ctx.document
                .feature_tree()
                .all_nodes()
                .map(|(_, n)| n.name.as_str()),
            base,
        )
    }

    /// The last non-modifier feature of a body (default pattern original).
    fn last_shape_feature(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<FeatureId> {
        part_features_of_body(ctx.document, body)
            .into_iter()
            .rev()
            .find(|(_, f)| !f.is_modifier())
            .map(|(id, _)| id)
    }

    /// The current face pick, in `body`'s frame, when the user has one
    /// selected in the viewport.
    fn selected_face_pick(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<FacePick> {
        ctx.selected_face_in(body).map(|face| FacePick {
            point: face.point,
            normal: face.normal,
        })
    }

    /// The flat face picked in the viewport, as a plane to mirror across;
    /// a curved face has no plane to offer.
    fn selected_mirror_face(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<MirrorPlane> {
        let face = ctx.selected_face_in(body)?;
        let flat = matches!(
            face.surface,
            None | Some(kernel_api::FaceSurface::Plane { .. })
        );
        flat.then_some(MirrorPlane::Face(FacePick {
            point: face.point,
            normal: face.normal,
        }))
    }

    /// What a dress-up takes from the viewport selection, in `body`'s
    /// frame: the picked edges first, else the edges of the picked face,
    /// else every edge.
    fn selected_edges(ctx: &WorkbenchRuntimeContext, body: BodyId) -> EdgeSel {
        let edges = ctx.selected_edges_in(body);
        if !edges.is_empty() {
            return EdgeSel::Edges(
                edges
                    .iter()
                    .map(|e| EdgePick {
                        point: e.point,
                        direction: e.direction,
                    })
                    .collect(),
            );
        }
        match Self::selected_face_pick(ctx, body) {
            Some(pick) => EdgeSel::Faces(vec![pick]),
            None => EdgeSel::All,
        }
    }

    /// Build the default feature payload for a toolbar action, or explain why
    /// it can't be created from the current selection.
    fn feature_for_tool(
        tool: &str,
        variant: Option<&str>,
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Result<(PartFeature, &'static str), String> {
        let sketch = Self::selected_sketch(ctx);
        let primitive = |subtractive: bool| PartFeature::Primitive {
            refine: false,
            kind: variant
                .and_then(primitive_preset)
                .unwrap_or_else(|| primitive_preset("box").expect("box preset")),
            placement: kernel_api::Placement::default(),
            subtractive,
        };
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
                    refine: false,
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
                        refine: false,
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
                    refine: false,
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
                        refine: false,
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
            "part.loft" | "part.subtractive_loft" => {
                let subtractive = tool == "part.subtractive_loft";
                if subtractive {
                    need_material(has_solid)?;
                }
                (
                    PartFeature::Loft {
                        refine: false,
                        sections: vec![need_sketch(sketch)?],
                        ruled: false,
                        closed: false,
                        subtractive,
                    },
                    "Loft",
                )
            }
            "part.pipe" | "part.subtractive_pipe" => {
                let subtractive = tool == "part.subtractive_pipe";
                if subtractive {
                    need_material(has_solid)?;
                }
                let profile = need_sketch(sketch)?;
                // The path is another sketch of the body, the latest made:
                // a pipe along its own profile builds nothing.
                let spine = ctx
                    .document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(id, n)| {
                        n.workbench_id.as_str() == "wb.sketch"
                            && n.body == Some(body)
                            && **id != profile
                    })
                    .max_by_key(|(_, n)| n.seq)
                    .map(|(id, _)| *id)
                    .ok_or("A pipe needs a second sketch for its path; draw one first")?;
                (
                    PartFeature::Pipe {
                        refine: false,
                        profile,
                        spine,
                        frenet: false,
                        subtractive,
                    },
                    "Pipe",
                )
            }
            "part.helix" | "part.subtractive_helix" => {
                let subtractive = tool == "part.subtractive_helix";
                if subtractive {
                    need_material(has_solid)?;
                }
                (
                    PartFeature::Helix {
                        refine: false,
                        sketch: need_sketch(sketch)?,
                        axis: RevolveAxis::default(),
                        mode: HelixMode::PitchHeight,
                        pitch: 5.0,
                        height: 20.0,
                        turns: 4.0,
                        left_handed: false,
                        cone_angle_deg: 0.0,
                        reversed: false,
                        subtractive,
                    },
                    "Helix",
                )
            }
            "part.primitive" => (primitive(false), "Primitive"),
            "part.subtractive_primitive" => {
                need_material(has_solid)?;
                (primitive(true), "Primitive")
            }
            "part.hole" => {
                need_material(has_solid)?;
                (
                    PartFeature::Hole {
                        refine: false,
                        sketch: need_sketch(sketch)?,
                        diameter: 5.0,
                        depth: 10.0,
                        through_all: false,
                        cut: HoleCut::None,
                        metric_index: None,
                        threaded: false,
                        modeled_thread: false,
                        thread_depth: 0.0,
                        fit: HoleFit::Normal,
                        reversed: false,
                    },
                    "Hole",
                )
            }
            "part.fillet" => {
                need_material(has_solid)?;
                let edges = Self::selected_edges(ctx, body);
                (PartFeature::Fillet { radius: 1.0, edges }, "Fillet")
            }
            "part.chamfer" => {
                need_material(has_solid)?;
                let edges = Self::selected_edges(ctx, body);
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
                let pick = Self::selected_face_pick(ctx, body)
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
                let pick = Self::selected_face_pick(ctx, body)
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
                        refine: false,
                        originals: original.into_iter().collect(),
                        plane: Self::selected_mirror_face(ctx, body).unwrap_or(MirrorPlane::YZ),
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
                        refine: false,
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
                        refine: false,
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
                    // One step to start from, so it builds as it opens.
                    PartFeature::MultiTransform {
                        refine: false,
                        originals: original.into_iter().collect(),
                        steps: vec![TransformStep::Linear {
                            axis: PatternAxis::X,
                            length: 20.0,
                            occurrences: 2,
                        }],
                    },
                    "MultiTransform",
                )
            }
            "part.clone" => {
                if has_solid {
                    return Err("A clone can only start an empty body".into());
                }
                let other = ctx
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.id != body && ctx.document.imported_brep_blob(b.id).is_some())
                    .map(|b| b.id)
                    .ok_or("Build another body first; the clone copies its solid")?;
                (PartFeature::Clone { source: other }, "Clone")
            }
            "part.scaled" => {
                need_material(has_solid)?;
                let original = Self::selected_part_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    PartFeature::MultiTransform {
                        refine: false,
                        originals: original.into_iter().collect(),
                        steps: vec![TransformStep::Scale {
                            factor: 1.5,
                            center: [0.0, 0.0, 0.0],
                            occurrences: 2,
                        }],
                    },
                    "Scaled",
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
                        refine: false,
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
    fn insert_datum(&mut self, ctx: &mut WorkbenchRuntimeContext, tool: &str) -> InputResult {
        use core_document::{AttachmentOffset, DatumAttachment, DatumFeature, DatumShape};
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        let shape = match tool {
            "part.datum_plane" => DatumShape::Plane { size: 30.0 },
            "part.datum_line" => DatumShape::Line { length: 40.0 },
            "part.coordinate_system" => DatumShape::CoordinateSystem { size: 20.0 },
            _ => DatumShape::Point,
        };
        let attachment = match ctx.selected_face_in(body) {
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
                self.pending_task_from_tool = Some(ToolMade {
                    feature: feature_id,
                    hidden: Vec::new(),
                    tool: tool.to_string(),
                    body,
                });
                ctx.active_document_object = Some(feature_id);
                ctx.log_info(format!("Created {name}"));
            }
            Err(e) => ctx.log_error(format!("Failed to create datum: {e}")),
        }
        InputResult::consumed()
    }

    /// Create a feature from a toolbar action and open its task.
    fn insert_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, tool: &str) -> InputResult {
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        match self.create_feature(ctx, tool, body, |_| Ok(())) {
            Ok(made) => {
                self.pending_task_from_tool = Some(ToolMade {
                    feature: made.id,
                    hidden: made.hidden,
                    tool: tool.to_string(),
                    body,
                });
                ctx.active_document_object = Some(made.id);
            }
            Err(message) => ctx.log_warn(message),
        }
        InputResult::consumed()
    }

    /// Add the feature `tool` makes, from the selection, to `body` and mark
    /// it for rebuild; `edit` changes it before it goes in. An imported
    /// body's solid lives in the import, not in the tree, so a feature can't
    /// extend it: it goes to a body of its own, which leaves the import as
    /// it was.
    pub(crate) fn create_feature(
        &self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        body: BodyId,
        edit: impl FnOnce(&mut PartFeature) -> Result<(), String>,
    ) -> Result<CreatedFeature, String> {
        let body = if ctx.document.body_solid_is_imported(body) {
            let imported = ctx
                .document
                .bodies()
                .iter()
                .find(|b| b.id == body)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "the imported body".to_string());
            let fresh = ctx.document.create_body(None);
            ctx.log_warn(format!(
                "`{imported}` came from an import and has no history to build on; \
                 the feature goes into a new body"
            ));
            fresh
        } else {
            body
        };
        let (mut feature, base) =
            Self::feature_for_tool(base_tool_id(tool), tool_variant(tool), ctx, body)?;
        feature.set_refine(self.options.refine_result);
        edit(&mut feature)?;
        let name = Self::next_feature_name(ctx, base);
        let sketches = feature.sketches();
        let id = ctx
            .document
            .add_feature_in_body(feature, name.clone(), Some(body))
            .map_err(|e| format!("Failed to create {base}: {e}"))?;
        ctx.document.mark_feature_dirty(id);
        // Consumed sketches are hidden; the solid takes over visually.
        let hidden = if self.options.hide_used_sketches {
            for sketch in &sketches {
                ctx.document.set_feature_visible(*sketch, false);
            }
            sketches
        } else {
            Vec::new()
        };
        ctx.log_info(format!("Created {name}"));
        Ok(CreatedFeature { id, hidden })
    }
}

/// A feature `create_feature` added, and the sketches it hid.
pub(crate) struct CreatedFeature {
    pub id: FeatureId,
    pub hidden: Vec<FeatureId>,
}

/// Default keys of the tools; the user can rebind them in Preferences.
/// Shift and a letter makes the subtractive form of what the letter adds.
const TOOL_KEYS: &[(&str, &str)] = &[
    ("part.new_body", "B"),
    ("part.new_sketch", "S"),
    ("part.pad", "E"),
    ("part.pocket", "Shift+E"),
    ("part.revolve", "R"),
    ("part.groove", "Shift+R"),
    ("part.loft", "L"),
    ("part.subtractive_loft", "Shift+L"),
    ("part.pipe", "W"),
    ("part.subtractive_pipe", "Shift+W"),
    ("part.hole", "Shift+H"),
    ("part.fillet", "U"),
    ("part.chamfer", "C"),
    ("part.mirror", "M"),
];

/// Register `tool` with its default key, if it has one.
fn register(context: &mut WorkbenchContext, tool: ToolDescriptor) {
    let key = TOOL_KEYS.iter().find(|(id, _)| *id == tool.id);
    context.register_tool(match key {
        Some((_, key)) => tool.shortcut(key),
        None => tool,
    });
}

impl Workbench for PartDesignWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.part",
            "Part Design",
            "Feature-based solid modeling workbench.",
        )
        .icon("workbench-part-design")
        .feature_kinds(["wb.part", "core.datum"])
    }

    fn rebuild_jobs(&self, document: &mut Document) -> Vec<core_document::RebuildJob> {
        build::rebuild_jobs(document)
    }

    fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        build::invalidate_body(document, body);
    }

    fn invalidate_all(&self, document: &mut Document) {
        build::mark_all_part_features_dirty(document);
    }

    fn feature_info(&self, node: &core_document::FeatureNode) -> FeatureInfo {
        if node.workbench_id.as_str() == "core.datum" {
            let datum = core_document::DatumFeature::from_json(&node.data).ok();
            return FeatureInfo {
                icon: datum.as_ref().map(datum_icon).unwrap_or("datum-plane"),
                kind_label: "Datum".to_string(),
                family_label: "Datum".to_string(),
                builds_solid: false,
            };
        }
        let feature = PartFeature::from_json(&node.data).ok();
        FeatureInfo {
            icon: feature.as_ref().map(|f| f.icon()).unwrap_or("tree-feature"),
            kind_label: feature
                .as_ref()
                .map(|f| f.kind_label().to_string())
                .unwrap_or_else(|| "Part design feature".to_string()),
            family_label: "Part design feature".to_string(),
            builds_solid: true,
        }
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        let action = |id: &str, label: &str, icon: &'static str, category: &str| {
            ToolDescriptor::new_action(id, label, Some(category)).icon(icon)
        };
        // Structure and sketches.
        register(
            context,
            action("part.new_body", "Create body", "body", "structure"),
        );
        register(
            context,
            action(
                "part.new_sketch",
                "Create sketch",
                "sketch-new",
                "structure",
            ),
        );
        register(
            context,
            action(
                "part.edit_sketch",
                "Edit sketch",
                "sketch-edit",
                "structure",
            ),
        );
        register(
            context,
            action(
                "part.map_sketch",
                "Map sketch to face",
                "sketch-map",
                "structure",
            ),
        );
        // Datums.
        register(
            context,
            action("part.datum_point", "Datum point", "datum-point", "datum"),
        );
        register(
            context,
            action("part.datum_line", "Datum line", "datum-line", "datum"),
        );
        register(
            context,
            action("part.datum_plane", "Datum plane", "datum-plane", "datum"),
        );
        register(
            context,
            action(
                "part.coordinate_system",
                "Local coordinate system",
                "coordinate-system",
                "datum",
            ),
        );
        register(context, action("part.clone", "Clone", "clone", "datum"));
        // Additive.
        register(context, action("part.pad", "Pad", "pad", "additive"));
        register(
            context,
            action("part.revolve", "Revolution", "revolution", "additive"),
        );
        register(
            context,
            action("part.loft", "Additive loft", "additive-loft", "additive"),
        );
        register(
            context,
            action("part.pipe", "Additive pipe", "additive-pipe", "additive"),
        );
        register(
            context,
            action("part.helix", "Additive helix", "additive-helix", "additive"),
        );
        register(
            context,
            action(
                "part.primitive",
                "Additive primitive",
                "additive-box",
                "additive",
            )
            .variants(primitive_variants(false)),
        );
        // Subtractive.
        register(
            context,
            action("part.pocket", "Pocket", "pocket", "subtractive"),
        );
        register(context, action("part.hole", "Hole", "hole", "subtractive"));
        register(
            context,
            action("part.groove", "Groove", "groove", "subtractive"),
        );
        register(
            context,
            action(
                "part.subtractive_loft",
                "Subtractive loft",
                "subtractive-loft",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "part.subtractive_pipe",
                "Subtractive pipe",
                "subtractive-pipe",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "part.subtractive_helix",
                "Subtractive helix",
                "subtractive-helix",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "part.subtractive_primitive",
                "Subtractive primitive",
                "subtractive-box",
                "subtractive",
            )
            .variants(primitive_variants(true)),
        );
        // Transformations.
        register(
            context,
            action("part.mirror", "Mirrored", "mirrored", "transform"),
        );
        register(
            context,
            action(
                "part.linear_pattern",
                "Linear pattern",
                "linear-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "part.polar_pattern",
                "Polar pattern",
                "polar-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "part.multi_transform",
                "Multi-transform",
                "multi-transform",
                "transform",
            ),
        );
        register(
            context,
            action("part.scaled", "Scaled", "scaled", "transform"),
        );
        // Dress-up.
        register(
            context,
            action("part.fillet", "Fillet", "fillet", "dressup"),
        );
        register(
            context,
            action("part.chamfer", "Chamfer", "chamfer", "dressup"),
        );
        register(context, action("part.draft", "Draft", "draft", "dressup"));
        register(
            context,
            action("part.thickness", "Thickness", "thickness", "dressup"),
        );
        // Boolean.
        register(
            context,
            action("part.boolean", "Boolean", "boolean", "boolean"),
        );
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        commands::run(self, id, args, ctx)
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
        // Feature tools are Actions: the host hands them over the moment they
        // are activated and clears them once handled.
        let base = active_tool.map(base_tool_id);
        match base {
            Some("part.new_body") => {
                let body = ctx.document.create_body(None);
                let name = ctx
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.id == body)
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| format!("body {:?}", body));
                ctx.log_info(format!("Created {name}"));
                ctx.record(
                    "doc.new_body",
                    commands::object(serde_json::json!({"name": name})),
                    serde_json::json!(body.0.to_string()),
                );
                ctx.request(HostRequest::SelectBody(body));
                ctx.request(HostRequest::JournalLabel("Create body".to_string()));
                InputResult::consumed()
            }
            Some(
                tool @ ("part.datum_plane"
                | "part.datum_line"
                | "part.datum_point"
                | "part.coordinate_system"),
            ) => self.insert_datum(ctx, tool),
            Some("part.map_sketch") => {
                // The selected sketch moves onto the face the last body
                // click landed on.
                let (Some(sketch_id), Some(face)) = (Self::selected_sketch(ctx), ctx.selected_face)
                else {
                    ctx.log_warn("Select a sketch in the tree, then click a face");
                    return InputResult::consumed();
                };
                let Some(mut feature) = ctx
                    .document
                    .get_feature_data(sketch_id)
                    .and_then(|d| SketchFeature::from_json(d).ok())
                else {
                    return InputResult::consumed();
                };
                // The sketch keeps its plane in its body's frame.
                let face = match ctx
                    .document
                    .get_feature_meta(sketch_id)
                    .and_then(|n| n.body)
                {
                    Some(body) => face.moved(&ctx.document.body_placement(body).inverse()),
                    None => face,
                };
                let plane = wb_sketch::sketch::SketchPlane::from_face(face.point, face.normal);
                feature.plane = plane;
                feature.sketch.plane = plane;
                // Mapped to the face, it follows the face, not the datum it was on.
                if feature.support.take().is_some() {
                    ctx.document
                        .set_feature_dependencies(sketch_id, feature.dependencies());
                }
                match ctx
                    .document
                    .update_feature_data(sketch_id, feature.to_json())
                {
                    Ok(()) => {
                        ctx.document.mark_feature_dirty(sketch_id);
                        ctx.log_info("Sketch mapped to the picked face");
                        ctx.record(
                            "sketch.set_plane",
                            commands::object(serde_json::json!({
                                "sketch": sketch_id.0.to_string(),
                                "normal": plane.normal,
                                "origin": plane.origin,
                                "x_axis": plane.x_axis,
                            })),
                            serde_json::Value::Null,
                        );
                    }
                    Err(err) => ctx.log_error(format!("Could not move the sketch: {err}")),
                }
                InputResult::consumed()
            }
            Some("part.edit_sketch") => {
                if Self::selected_sketch(ctx).is_some() {
                    // The sketcher picks the active object up as its edit
                    // session on activation.
                    ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from("wb.sketch")));
                } else {
                    ctx.log_warn("Select a sketch in the tree first");
                }
                InputResult::consumed()
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
                ctx.request(HostRequest::StartOn {
                    workbench: WorkbenchId::from("wb.sketch"),
                    attach: core_document::SketchAttachRequest {
                        body: body.0,
                        face: ctx.selected_face,
                    },
                });
                InputResult::consumed()
            }
            Some(tool) if tool.starts_with("part.") => {
                let full = active_tool.unwrap_or(tool);
                self.insert_feature(ctx, full)
            }
            _ => InputResult::ignored(),
        }
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        #[cfg(feature = "egui")]
        {
            self.task_info(ctx)
        }
        #[cfg(not(feature = "egui"))]
        {
            let _ = ctx;
            None
        }
    }

    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        self.draw_task_panel(ui, ctx, request)
    }

    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        #[cfg(feature = "egui")]
        {
            self.task = None;
        }
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let body = Self::target_body(ctx);
        let has_body = body.is_some();
        let has_sketch = Self::selected_sketch(ctx).is_some();
        let has_solid = body.map(|b| Self::body_has_solid(ctx, b)).unwrap_or(false);
        match base_tool_id(tool_id) {
            "part.new_body" => true,
            "part.edit_sketch" => has_sketch,
            "part.map_sketch" => has_sketch && ctx.selected_face.is_some(),
            "part.new_sketch"
            | "part.primitive"
            | "part.datum_plane"
            | "part.datum_line"
            | "part.datum_point"
            | "part.coordinate_system" => has_body,
            "part.clone" => has_body && !has_solid,
            "part.scaled" => has_solid,
            "part.pad" | "part.revolve" | "part.loft" | "part.pipe" | "part.helix" => has_sketch,
            "part.pocket"
            | "part.groove"
            | "part.hole"
            | "part.subtractive_loft"
            | "part.subtractive_pipe"
            | "part.subtractive_helix" => has_sketch && has_solid,
            "part.subtractive_primitive" => has_solid,
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

    /// The Part Design preferences page.
    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui, filter: &str) -> bool {
        use ui_kit::widgets::{PrefRow, pref_group};
        pref_group(
            ui,
            "Feature defaults",
            vec![
                PrefRow::toggle("Refine result", &mut self.options.refine_result)
                    .hint("New features merge the coplanar faces their fuse or cut leaves"),
                PrefRow::toggle(
                    "Update view while editing",
                    &mut self.options.update_while_editing,
                )
                .hint("Rebuild the preview on every field change; off, on OK"),
                PrefRow::toggle(
                    "Hide the sketch after a feature uses it",
                    &mut self.options.hide_used_sketches,
                )
                .hint("Keep used sketches out of the viewport"),
            ],
            filter,
        );
        false
    }

    /// Under the tree the bench only orients a new user; features are
    /// edited in the task panel and managed from the tree's menu.
    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        if Self::target_body(ctx).is_some() {
            return;
        }
        ui.label(
            egui::RichText::new("Create a body, then a sketch on it, then Pad the sketch.")
                .font(ui_kit::sans(ui_kit::tokens::FONT_SM))
                .color(ui_kit::tokens::TEXT3),
        );
        ui.label(
            egui::RichText::new("Select a body or sketch in the tree to see its features.")
                .font(ui_kit::sans(ui_kit::tokens::FONT_SM))
                .color(ui_kit::tokens::TEXT3),
        );
    }

    fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool {
        build::delete_feature(ctx.document, id)
    }

    fn settings_json(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.options).ok()
    }

    fn apply_settings_json(&mut self, value: &serde_json::Value) {
        if let Ok(options) = serde_json::from_value(value.clone()) {
            self.options = options;
        }
    }

    /// The open task and the feature a tool just created are the editing
    /// state; they belong to the tab they were opened in. The settings
    /// stay: a fresh bench still keeps the user's switches.
    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        let options = self.options;
        let mut state = std::mem::take(self);
        self.options = options;
        state.options = options;
        Some(Box::new(state))
    }

    fn resume_session(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        let options = self.options;
        *self = state
            .and_then(|s| s.downcast::<Self>().ok())
            .map(|s| *s)
            .unwrap_or_default();
        self.options = options;
    }

    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        if node.workbench_id.as_str() == "core.datum" {
            params::datum_parameters()
        } else {
            params::feature_parameters(node)
        }
    }

    fn property_hints(&self) -> core_document::PropertyHints {
        core_document::PropertyHints {
            length_keys: vec![
                "length",
                "length2",
                "depth",
                "depth2",
                "radius",
                "size",
                "size2",
                "value",
                "diameter",
                "pitch",
                "height",
                "up_to_offset",
                "width",
                "circumradius",
                "radius1",
                "radius2",
                "radius3",
            ],
            reference_keys: vec![
                "sketch",
                "profile",
                "spine",
                "sections",
                "originals",
                "tool_body",
            ],
        }
    }

    fn get_overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        let Some(body) = Self::target_body(ctx) else {
            return Vec::new();
        };
        let mut meshes = Vec::new();
        // Datums live in their body's frame and draw where the body sits.
        let placement = ctx.document.body_placement(body);
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
            meshes.push(core_document::OverlayMesh::wireframe(
                placement.mesh(&datum_mesh(&datum)),
                color,
            ));
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
        core_document::DatumShape::CoordinateSystem { size } => {
            // Three axes from the origin, each with an arrowhead, and a
            // corner square between x and y marking the frame's XY plane.
            let s = size;
            let h = size * 0.12;
            let c = size * 0.3;
            mesh.positions = vec![
                at(0.0, 0.0, 0.0),
                at(s, 0.0, 0.0),
                at(0.0, s, 0.0),
                at(0.0, 0.0, s),
                at(s - h, h * 0.5, 0.0),
                at(s - h, -h * 0.5, 0.0),
                at(h * 0.5, s - h, 0.0),
                at(-h * 0.5, s - h, 0.0),
                at(h * 0.5, 0.0, s - h),
                at(-h * 0.5, 0.0, s - h),
                at(c, 0.0, 0.0),
                at(c, c, 0.0),
                at(0.0, c, 0.0),
            ];
            mesh.normals = vec![n; mesh.positions.len()];
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            mesh.edges = vec![
                0, 1, 0, 2, 0, 3, 1, 4, 1, 5, 2, 6, 2, 7, 3, 8, 3, 9, 10, 11, 11, 12,
            ];
        }
    }
    mesh
}

/// `base` when none of `names` is it, else `base_n` one past the highest
/// `n` among the names.
fn next_name<'a>(names: impl Iterator<Item = &'a str>, base: &str) -> String {
    let mut taken = false;
    let mut highest = 0u32;
    for name in names {
        if name == base {
            taken = true;
        } else if let Some(n) = name
            .strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('_'))
            .and_then(|n| n.parse::<u32>().ok())
        {
            taken = true;
            highest = highest.max(n);
        }
    }
    if taken {
        format!("{base}_{}", highest + 1)
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod naming {
    use super::next_name;

    #[test]
    fn a_new_name_is_one_past_the_highest_in_use() {
        assert_eq!(next_name([].into_iter(), "Pad"), "Pad");
        assert_eq!(next_name(["Padding"].into_iter(), "Pad"), "Pad");
        assert_eq!(next_name(["Pad"].into_iter(), "Pad"), "Pad_1");
        // Pad deleted, Pad_1 kept: the next is Pad_2, not Pad_1 again.
        assert_eq!(next_name(["Pad_1"].into_iter(), "Pad"), "Pad_2");
        assert_eq!(
            next_name(["Pad", "Pad_3", "Pocket"].into_iter(), "Pad"),
            "Pad_4"
        );
    }
}

#[cfg(test)]
mod body_tool {
    use super::*;
    use core_document::{Document, WorkbenchInputEvent, WorkbenchRuntimeContext};

    #[test]
    fn the_body_tool_creates_a_body_and_asks_the_host_to_select_and_label_it() {
        let mut wb = PartDesignWorkbench::default();
        let mut doc = Document::new("t");
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        let result = wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("part.new_body"),
            &mut ctx,
        );
        assert!(result.consumed);
        let requests = ctx.take_requests();
        assert_eq!(doc.bodies().len(), 1);
        let body = doc.bodies()[0].id;
        assert_eq!(
            requests,
            vec![
                HostRequest::SelectBody(body),
                HostRequest::JournalLabel("Create body".to_string()),
            ]
        );
    }
    /// The Mirror tool with a face picked: a flat face is the plane it
    /// starts with, a curved one leaves it on YZ.
    #[test]
    fn a_picked_flat_face_is_the_plane_a_new_mirror_starts_with() {
        let mirror_with = |surface: Option<kernel_api::FaceSurface>| {
            let mut wb = PartDesignWorkbench::default();
            let mut doc = Document::new("t");
            let body = doc.create_body(None);
            let pad = doc
                .add_feature_in_body(
                    PartFeature::Primitive {
                        refine: false,
                        kind: primitive_preset("box").unwrap(),
                        placement: kernel_api::Placement::default(),
                        subtractive: false,
                    },
                    "Box".into(),
                    Some(body),
                )
                .unwrap();
            let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            ctx.selected_body_id = Some(body.0);
            ctx.active_document_object = Some(pad);
            ctx.selected_face = Some(core_document::FaceRef {
                point: [10.0, 2.5, 4.0],
                normal: [1.0, 0.0, 0.0],
                surface,
            });
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("part.mirror"),
                &mut ctx,
            );
            let mirrored = doc
                .feature_tree()
                .all_nodes()
                .find_map(|(_, n)| match PartFeature::from_json(&n.data).ok()? {
                    PartFeature::Mirrored {
                        plane, originals, ..
                    } => Some((plane, originals)),
                    _ => None,
                })
                .expect("the tool made a mirror");
            assert_eq!(
                mirrored.1,
                vec![pad],
                "the selected feature is the original"
            );
            mirrored.0
        };
        let flat = kernel_api::FaceSurface::Plane {
            origin: [10.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        assert_eq!(
            mirror_with(Some(flat)),
            MirrorPlane::Face(FacePick {
                point: [10.0, 2.5, 4.0],
                normal: [1.0, 0.0, 0.0],
            })
        );
        let round = kernel_api::FaceSurface::Cylinder {
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 5.0,
        };
        assert_eq!(mirror_with(Some(round)), MirrorPlane::YZ);
    }

    #[test]
    fn the_coordinate_system_tool_places_a_frame_on_the_body() {
        let mut wb = PartDesignWorkbench::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.selected_body_id = Some(body.0);
        assert!(wb.is_tool_enabled("part.coordinate_system", &ctx));
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("part.coordinate_system"),
            &mut ctx,
        );
        let datums = core_document::datums_of_body(&doc, body);
        assert_eq!(datums.len(), 1);
        assert!(matches!(
            datums[0].2.shape,
            core_document::DatumShape::CoordinateSystem { .. }
        ));
        assert!(!datum_mesh(&datums[0].2).edges.is_empty());
    }
}

/// The design set's icon for a datum's shape.
pub(crate) fn datum_icon(datum: &core_document::DatumFeature) -> &'static str {
    match datum.shape {
        core_document::DatumShape::Plane { .. } => "datum-plane",
        core_document::DatumShape::Line { .. } => "datum-line",
        core_document::DatumShape::Point => "datum-point",
        core_document::DatumShape::CoordinateSystem { .. } => "coordinate-system",
    }
}

#[cfg(all(test, feature = "egui"))]
mod icon_coverage {
    use super::*;
    use core_document::{Workbench, WorkbenchContext};

    #[test]
    fn the_bench_and_every_feature_family_name_an_icon_in_the_set() {
        let wb = PartDesignWorkbench::default();
        assert!(ui_kit::icon::exists(wb.descriptor().icon));
        use core_document::{DatumAttachment, DatumFeature, DatumShape};
        for shape in [
            DatumShape::Plane { size: 1.0 },
            DatumShape::Line { length: 1.0 },
            DatumShape::Point,
            DatumShape::CoordinateSystem { size: 1.0 },
        ] {
            let datum = DatumFeature {
                shape,
                attachment: DatumAttachment::BasePlane(core_document::BasePlane::XY),
                offset: Default::default(),
            };
            let icon = datum_icon(&datum);
            assert!(ui_kit::icon::exists(icon), "unknown icon {icon}");
        }
        let node = core_document::FeatureNode::new(
            FeatureId(uuid::Uuid::new_v4()),
            &PartFeature::Pad {
                refine: false,
                sketch: FeatureId(uuid::Uuid::new_v4()),
                length: 10.0,
                reversed: false,
                symmetric: false,
                mode: ExtrudeMode::Dimension,
                length2: 0.0,
                taper_deg: 0.0,
                up_to_face: None,
                up_to_offset: 0.0,
            },
        );
        let info = wb.feature_info(&node);
        assert!(
            ui_kit::icon::exists(info.icon),
            "unknown icon {}",
            info.icon
        );
        assert_eq!(info.kind_label, "Pad");
        assert!(info.builds_solid);
    }

    #[test]
    fn every_default_key_lands_on_a_tool_and_no_two_share_one() {
        let mut ctx = WorkbenchContext::default();
        PartDesignWorkbench::default().configure(&mut ctx);
        for (id, _) in TOOL_KEYS {
            assert!(ctx.tools().iter().any(|t| t.id == *id), "no tool {id}");
        }
        let mut seen = std::collections::HashMap::new();
        let keyed = ctx
            .tools()
            .iter()
            .map(|t| (&t.id, &t.shortcuts))
            .chain(ctx.actions().iter().map(|a| (&a.id, &a.shortcuts)));
        for (id, keys) in keyed {
            for key in keys {
                if let Some(other) = seen.insert(*key, id.clone()) {
                    panic!("{id} and {other} share {key}");
                }
            }
        }
    }

    #[test]
    fn every_tool_names_an_icon_in_the_set() {
        let mut ctx = WorkbenchContext::default();
        PartDesignWorkbench::default().configure(&mut ctx);
        for tool in ctx.tools() {
            let icon = tool
                .icon
                .unwrap_or_else(|| panic!("{} has no icon", tool.id));
            assert!(
                ui_kit::icon::exists(icon),
                "{}: unknown icon {icon}",
                tool.id
            );
            for variant in &tool.variants {
                assert!(
                    ui_kit::icon::exists(variant.icon),
                    "{}:{}: unknown icon {}",
                    tool.id,
                    variant.id,
                    variant.icon
                );
            }
        }
    }
}
