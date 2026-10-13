//! Design workbench: feature-based solid modeling.
//!
//! The workbench edits the document's feature tree; the app shell watches
//! for dirty part features and drives the kernel rebuild (see `build.rs`).

mod borrow;
mod build;
mod centre;
mod clipboard;
mod commands;
#[cfg(feature = "egui")]
mod datum_panel;
mod datum_refs;
#[cfg(feature = "egui")]
mod editors;
mod feature;
mod generators;
mod handles;
mod hole_tables;
mod instances;
mod params;
mod recognize;
mod references;
mod region_pick;
#[cfg(feature = "egui")]
mod task;

pub use borrow::freeze;
pub use build::{
    BuildError, BuildPlan, NutPocket, body_build_ops, delete_feature, design_feature_ids,
    design_features_of_body, hole_diameter, invalidate_body, mark_all_design_features_dirty,
    nut_pocket, pending_body_rebuilds, rebuild_jobs, retarget_feature_sketch,
    sketch_plane_description, sketches_of_body,
};
pub use feature::{
    Attached, BaseAxis, BorrowOptions, BorrowSource, BorrowedRef, ChamferMode, DesignFeature,
    DrillPoint, EdgePick, EdgeSel, ExtrudeDirection, ExtrudeExtras, ExtrudeMode, FacePick,
    FrozenBorrow, FrozenEdge, FrozenFace, HelixMode, HoleCut, HoleFit, LoftSection, MirrorPlane,
    NutSide, NutTrap, PatternAxis, PipeCorner, PipeOrientation, PlaneTarget, RevolveAxis,
    RevolveMode, SketchAxis, ThreadSpec, TransformStep, primitive_icon, primitive_preset,
};
pub use hole_tables::{
    CUT_PROFILES_FILE, CutProfile, NutSize, NutStandard, ScrewSeat, ThreadSize, ThreadStandard,
    parse_cut_profiles, user_cut_profiles,
};

use core_document::{
    BodyId, Document, FeatureId, FeatureInfo, HostRequest, InputResult, MenuItem, MenuScope,
    TaskInfo, ToolDescriptor, ToolVariant, Workbench, WorkbenchContext, WorkbenchDescriptor,
    WorkbenchFeature, WorkbenchId, WorkbenchInputEvent, WorkbenchRuntimeContext, base_tool_id,
    tool_variant,
};
use wb_sketch::SketchFeature;

const MOVE_TO_BODY: &str = "design.move_to_body";
const DUPLICATE: &str = "design.duplicate";
const MAP_SKETCH: &str = "design.map_sketch";

/// Duplicate `feature` into `body` (its own when `None`), record it as the
/// command and select the copy.
fn duplicate_recorded(ctx: &mut WorkbenchRuntimeContext, feature: FeatureId, body: Option<BodyId>) {
    match commands::duplicate(ctx, feature, body) {
        Ok(made) => {
            let mut args = serde_json::json!({"feature": feature.0.to_string()});
            if let Some(body) = body {
                args["body"] = serde_json::json!(body.0.to_string());
            }
            let ids: Vec<String> = made.iter().map(|f| f.0.to_string()).collect();
            ctx.record(DUPLICATE, commands::object(args), serde_json::json!(ids));
            ctx.active_document_object = made.last().copied();
            ctx.request(HostRequest::JournalLabel("Duplicate".into()));
        }
        Err(message) => ctx.log_warn(format!("Cannot duplicate the feature: {message}")),
    }
}

/// The switches on the Design Preferences page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DesignOptions {
    /// The preview rebuilds on every field change in the task panel; off,
    /// it rebuilds when the task is accepted.
    pub update_while_editing: bool,
    /// A sketch a new feature consumes is hidden.
    pub hide_used_sketches: bool,
    /// A new feature that fuses or cuts merges the coplanar faces it leaves.
    pub refine_result: bool,
    /// The step a view handle's drag moves in, mm.
    pub handle_step: f32,
    /// The step while Shift is held, mm.
    pub handle_fine_step: f32,
}

impl Default for DesignOptions {
    fn default() -> Self {
        Self {
            update_while_editing: true,
            hide_used_sketches: true,
            refine_result: true,
            handle_step: 1.0,
            handle_fine_step: 0.1,
        }
    }
}

/// Design workbench: feature-based solid modeling.
#[derive(Default)]
pub struct DesignWorkbench {
    /// The Preferences page's switches.
    pub options: DesignOptions,
    /// The feature open in the task panel.
    #[cfg(feature = "egui")]
    task: Option<task::TaskState>,
    /// A feature a tool just created: the task that opens for it deletes it
    /// on Cancel, and records it on OK as the command that makes it.
    pending_task_from_tool: Option<ToolMade>,
    /// A feature the user asked to edit (a double click in the tree), for
    /// the task panel to open on.
    edit_request: Option<FeatureId>,
    /// The centre line tool, while it is out.
    centre: Option<centre::CentreTask>,
    /// Features Edit › Copy or Cut took.
    clipboard: Option<clipboard::Clipboard>,
    /// The drag handle held, as it was taken and at the number it sets.
    held: Option<handles::Handle>,
    /// The occurrence of the open pattern whose dot is under the pointer.
    hovered_occurrence: Option<u32>,
    /// The held handle has been dragged: a release then keeps the number
    /// rather than turning the feature round.
    held_moved: bool,
}

/// What a tool just made, for the task that opens on it.
#[derive(Debug, Clone)]
pub(crate) struct ToolMade {
    pub feature: FeatureId,
    /// The sketches it hid, shown again when it is cancelled.
    pub hidden: Vec<FeatureId>,
    /// The tool, with its variant (`design.primitive:box`).
    pub tool: String,
    /// The body it was made for.
    pub body: BodyId,
    /// The base shape the tool gave the body first, to make it take
    /// features; Cancel takes it away again.
    pub base: Option<FeatureId>,
}

/// Primitive shapes offered from the primitive tools' dropdowns.
pub(crate) const PRIMITIVE_SHAPES: &[(&str, &str)] = &[
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

impl DesignWorkbench {
    /// The sketch feature currently selected in the tree, if any: a sketch
    /// of the body, or one another body lends it.
    fn selected_sketch(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == "wb.sketch" || borrow::lends_sketch(ctx.document, id))
            .then_some(id)
    }

    /// The body a new feature goes in: the selected feature's, else the
    /// selected body. A linked copy takes its shape from its source and
    /// takes no features.
    /// Hand off to the sketch workbench: it opens its plane picker for the
    /// target body (offering the clicked face when the selection landed on
    /// solid geometry), and finishing the sketch returns here (the host
    /// tracks the return bench). With `generator` the sketch it makes is
    /// that generator's.
    /// A borrow selected in the tree offers its first flat face, which a
    /// sketch placed on it follows as the borrow does; a face clicked on the
    /// body's own solid is followed too, one of another body's is not.
    pub(crate) fn start_sketch(
        ctx: &mut WorkbenchRuntimeContext,
        generator: Option<&'static str>,
    ) -> InputResult {
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        let lent = ctx.active_document_object.and_then(|id| {
            borrow::flat_face_in_world(ctx.document, id)
                .map(|(face, index)| (face, core_document::FaceOrigin::Lent { borrow: id, index }))
        });
        let (face, face_origin) = match lent {
            Some((face, origin)) => (Some(face), origin),
            None => (
                ctx.selected_face,
                if ctx.selected_body_id == Some(body.0) {
                    core_document::FaceOrigin::OwnSolid
                } else {
                    core_document::FaceOrigin::Elsewhere
                },
            ),
        };
        ctx.request(HostRequest::StartOn {
            workbench: WorkbenchId::from("wb.sketch"),
            attach: core_document::SketchAttachRequest {
                body: body.0,
                face,
                face_origin,
                generator,
            },
        });
        InputResult::consumed()
    }

    fn target_body(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        let body = match ctx
            .active_document_object
            .and_then(|id| ctx.document.get_feature_meta(id))
        {
            Some(node) if node.body.is_some() => node.body,
            _ => ctx.selected_body_id.map(BodyId),
        };
        body.filter(|b| ctx.document.copy_source(*b).is_none())
    }

    /// The part feature currently selected in the tree, if any.
    fn selected_design_feature(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == "wb.design").then_some(id)
    }

    /// Whether the body's history builds anything: borrowed geometry
    /// alone does not.
    fn body_has_solid(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        design_features_of_body(ctx.document, body)
            .iter()
            .any(|(_, f)| !matches!(f, DesignFeature::Borrow { .. }))
            || Self::can_take_base(ctx, body)
    }

    /// An imported solid a feature can build on: it takes a base shape
    /// first (`take_base`).
    fn can_take_base(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        ctx.document.body_solid_is_imported(body)
            && ctx.document.imported_brep_blob(body).is_some()
            && ctx.document.copy_source(body).is_none()
            && !ctx
                .document
                .bodies()
                .iter()
                .any(|b| b.id == body && b.link.is_some())
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
        design_features_of_body(ctx.document, body)
            .into_iter()
            .rev()
            .find(|(_, f)| !f.is_modifier())
            .map(|(id, _)| id)
    }

    /// Every face picked in the viewport (Ctrl adds more), in `body`'s
    /// frame, in the order picked.
    fn selected_face_picks(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<FacePick> {
        ctx.selected_faces_in(body)
            .into_iter()
            .map(FacePick::of)
            .collect()
    }

    /// The flat face picked in the viewport, in `body`'s frame, as a
    /// profile to pad or pocket; a curved face has no plane to extrude.
    fn selected_profile_face(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<FacePick> {
        let face = ctx.selected_face_in(body)?;
        let flat = matches!(
            face.surface,
            None | Some(kernel_api::FaceSurface::Plane { .. })
        );
        flat.then_some(FacePick::of(face))
    }

    /// Whether a pad or a pocket has a profile without a sketch: a flat
    /// face of `body`'s solid picked in the viewport, or a borrow of faces
    /// selected in the tree that lends one.
    fn profile_without_sketch(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        Self::selected_profile_face(ctx, body).is_some()
            || ctx.active_document_object.is_some_and(|id| {
                crate::borrow::borrow_of(ctx.document, id).is_some_and(|b| b.body == body)
                    && crate::borrow::lent_face(ctx.document, id, 0).is_some()
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
        flat.then_some(MirrorPlane::Face(FacePick::of(face)))
    }

    /// What a dress-up takes from the viewport selection, in `body`'s
    /// frame: the picked edges first, else the edges of the picked face,
    /// else every edge.
    fn selected_edges(ctx: &WorkbenchRuntimeContext, body: BodyId) -> EdgeSel {
        let edges = ctx.selected_edges_in(body);
        if !edges.is_empty() {
            return EdgeSel::Edges(edges.iter().map(EdgePick::of).collect());
        }
        let faces = Self::selected_face_picks(ctx, body);
        if faces.is_empty() {
            EdgeSel::All
        } else {
            EdgeSel::Faces(faces)
        }
    }

    /// Build the default feature payload for a toolbar action, or explain why
    /// it can't be created from the current selection (and, for a command,
    /// its arguments).
    fn feature_for_tool(
        tool: &str,
        variant: Option<&str>,
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        given: Given,
    ) -> Result<(DesignFeature, &'static str), String> {
        let sketch = Self::selected_sketch(ctx);
        let primitive = |subtractive: bool| DesignFeature::Primitive {
            keep_inside: false,
            attached: None,
            refine: false,
            boolean_tolerance: None,
            kind: variant
                .and_then(primitive_preset)
                .unwrap_or_else(|| primitive_preset("box").expect("box preset")),
            placement: kernel_api::Placement::default(),
            subtractive,
        };
        let need_sketch = |value: Option<FeatureId>| {
            value.ok_or_else(|| {
                given.say(
                    "Select a sketch in the tree first",
                    "It needs a sketch: give `sketch`, or select a sketch in the tree",
                )
            })
        };
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

        // With no sketch chosen, a flat face picked on the solid is the
        // profile of a pad or a pocket.
        // A borrow of faces selected in the tree lends its first face.
        let lent = ctx.active_document_object.filter(|id| {
            crate::borrow::borrow_of(ctx.document, *id).is_some_and(|b| b.body == body)
                && crate::borrow::lent_face(ctx.document, *id, 0).is_some()
        });
        type Profile = (
            Option<FeatureId>,
            Option<FacePick>,
            Option<crate::feature::BorrowedRef>,
        );
        let extrude_profile = || -> Result<Profile, String> {
            if sketch.is_some() {
                return Ok((sketch, None, None));
            }
            if let Some(borrow) = lent {
                return Ok((
                    None,
                    None,
                    Some(crate::feature::BorrowedRef { borrow, index: 0 }),
                ));
            }
            match Self::selected_profile_face(ctx, body) {
                Some(face) if has_solid => Ok((None, Some(face), None)),
                _ => Err(given.say(
                    "Select a sketch in the tree, a flat face of the solid, or a borrow of \
                     faces, first",
                    "It needs a profile: give `sketch`, a flat face of the solid as \
                     `face_point` with `face_normal`, or `profile_borrowed`",
                )),
            }
        };

        let feature = match tool {
            "design.pad" => {
                let (sketch, profile_face, profile_borrowed) = extrude_profile()?;
                (
                    DesignFeature::Pad {
                        regions: Vec::new(),
                        profile_borrowed,
                        extras: Default::default(),
                        refine: false,
                        boolean_tolerance: None,
                        sketch,
                        length: 10.0,
                        reversed: false,
                        symmetric: false,
                        mode: ExtrudeMode::Dimension,
                        length2: 10.0,
                        taper_deg: 0.0,
                        up_to_face: None,
                        up_to_offset: 0.0,
                        profile_face,
                        direction: ExtrudeDirection::Normal,
                        up_to_shape: Vec::new(),
                        mode2: None,
                        up_to_face2: None,
                        up_to_offset2: 0.0,
                        up_to_shape2: Vec::new(),
                    },
                    "Pad",
                )
            }
            "design.pocket" => {
                need_material(has_solid)?;
                let (sketch, profile_face, profile_borrowed) = extrude_profile()?;
                (
                    DesignFeature::Pocket {
                        regions: Vec::new(),
                        keep_inside: false,
                        profile_borrowed,
                        extras: Default::default(),
                        refine: false,
                        boolean_tolerance: None,
                        sketch,
                        depth: 5.0,
                        reversed: false,
                        symmetric: false,
                        through_all: false,
                        mode: ExtrudeMode::Dimension,
                        depth2: 5.0,
                        taper_deg: 0.0,
                        up_to_face: None,
                        up_to_offset: 0.0,
                        profile_face,
                        direction: ExtrudeDirection::Normal,
                        up_to_shape: Vec::new(),
                        mode2: None,
                        up_to_face2: None,
                        up_to_offset2: 0.0,
                        up_to_shape2: Vec::new(),
                    },
                    "Pocket",
                )
            }
            "design.revolve" => (
                DesignFeature::Revolution {
                    regions: Vec::new(),
                    refine: false,
                    boolean_tolerance: None,
                    sketch: need_sketch(sketch)?,
                    angle_deg: 360.0,
                    axis: RevolveAxis::default(),
                    project_axis: false,
                    reversed: false,
                    midplane: false,
                    second_angle_deg: None,
                    mode: RevolveMode::Angle,
                    up_to_face: None,
                },
                "Revolution",
            ),
            "design.groove" => {
                need_material(has_solid)?;
                (
                    DesignFeature::Groove {
                        regions: Vec::new(),
                        keep_inside: false,
                        refine: false,
                        boolean_tolerance: None,
                        sketch: need_sketch(sketch)?,
                        angle_deg: 360.0,
                        axis: RevolveAxis::default(),
                        project_axis: false,
                        reversed: false,
                        midplane: false,
                        second_angle_deg: None,
                        mode: RevolveMode::Angle,
                        up_to_face: None,
                    },
                    "Groove",
                )
            }
            "design.loft" | "design.subtractive_loft" => {
                let subtractive = tool == "design.subtractive_loft";
                if subtractive {
                    need_material(has_solid)?;
                }
                (
                    DesignFeature::Loft {
                        keep_inside: false,
                        refine: false,
                        boolean_tolerance: None,
                        sections: vec![crate::feature::LoftSection::Feature(need_sketch(sketch)?)],
                        ruled: false,
                        closed: false,
                        subtractive,
                    },
                    "Loft",
                )
            }
            "design.pipe" | "design.subtractive_pipe" => {
                let subtractive = tool == "design.subtractive_pipe";
                if subtractive {
                    need_material(has_solid)?;
                }
                let profile = need_sketch(sketch)?;
                // The path is the one given, else another sketch of the
                // body, the latest made: a pipe along its own profile
                // builds nothing.
                let spine = given
                    .spine
                    .or_else(|| {
                        ctx.document
                            .feature_tree()
                            .all_nodes()
                            .filter(|(id, n)| {
                                n.workbench_id.as_str() == "wb.sketch"
                                    && n.body == Some(body)
                                    && **id != profile
                            })
                            .max_by_key(|(_, n)| n.seq)
                            .map(|(id, _)| *id)
                    })
                    .ok_or_else(|| {
                        given.say(
                            "A pipe needs a second sketch for its path; draw one first",
                            "A pipe needs a path: give `spine`, a sketch, or draw a second \
                             sketch in the profile's body",
                        )
                    })?;
                (
                    DesignFeature::Pipe {
                        keep_inside: false,
                        path_borrowed: Vec::new(),
                        path_edges: Vec::new(),
                        profile_face: None,
                        refine: false,
                        boolean_tolerance: None,
                        profile,
                        spine,
                        orientation: PipeOrientation::Standard,
                        corner: PipeCorner::Transformed,
                        sections: Vec::new(),
                        subtractive,
                    },
                    "Pipe",
                )
            }
            "design.helix" | "design.subtractive_helix" => {
                let subtractive = tool == "design.subtractive_helix";
                if subtractive {
                    need_material(has_solid)?;
                }
                (
                    DesignFeature::Helix {
                        refine: false,
                        boolean_tolerance: None,
                        sketch: need_sketch(sketch)?,
                        axis: RevolveAxis::default(),
                        project_axis: false,
                        mode: HelixMode::PitchHeight,
                        pitch: 5.0,
                        height: 20.0,
                        turns: 4.0,
                        left_handed: false,
                        cone_angle_deg: 0.0,
                        reversed: false,
                        subtractive,
                        growth: 0.0,
                        keep_inside: false,
                    },
                    "Helix",
                )
            }
            "design.primitive" => (primitive(false), "Primitive"),
            "design.subtractive_primitive" => {
                need_material(has_solid)?;
                (primitive(true), "Primitive")
            }
            "design.hole" => {
                need_material(has_solid)?;
                (
                    DesignFeature::Hole {
                        cosmetic_thread: false,
                        clearance: None,
                        thread_length: Default::default(),
                        refine: false,
                        boolean_tolerance: None,
                        sketch: need_sketch(sketch)?,
                        diameter: 5.0,
                        depth: 10.0,
                        through_all: false,
                        cut: HoleCut::None,
                        thread: None,
                        threaded: false,
                        modeled_thread: false,
                        thread_depth: 0.0,
                        fit: HoleFit::Normal,
                        drill_point: DrillPoint::Flat,
                        point_in_depth: false,
                        taper_deg: 0.0,
                        nut_trap: None,
                        reversed: false,
                    },
                    "Hole",
                )
            }
            "design.fillet" => {
                need_material(has_solid)?;
                let edges = Self::selected_edges(ctx, body);
                (
                    DesignFeature::Fillet {
                        radius: 1.0,
                        edges,
                        follow_tangent: true,
                    },
                    "Fillet",
                )
            }
            "design.chamfer" => {
                need_material(has_solid)?;
                let edges = Self::selected_edges(ctx, body);
                (
                    DesignFeature::Chamfer {
                        size: 1.0,
                        mode: ChamferMode::EqualDistance,
                        size2: 1.0,
                        angle_deg: 45.0,
                        flip: false,
                        edges,
                        follow_tangent: true,
                    },
                    "Chamfer",
                )
            }
            "design.draft" => {
                need_material(has_solid)?;
                // The first face picked is the neutral plane, any picked
                // after it the faces to draft.
                let mut picks = Self::selected_face_picks(ctx, body).into_iter();
                let pick = picks
                    .next()
                    .ok_or("Click a face in the viewport first (the neutral plane)")?;
                (
                    DesignFeature::Draft {
                        neutral_plane: None,
                        pull: None,
                        angle_deg: 1.5,
                        neutral: pick,
                        faces: picks.collect(),
                        reversed: false,
                    },
                    "Draft",
                )
            }
            "design.offset_faces" => {
                need_material(has_solid)?;
                let faces = Self::selected_face_picks(ctx, body);
                if faces.is_empty() {
                    return Err("Click the faces to offset in the viewport first".into());
                }
                (
                    DesignFeature::OffsetFaces {
                        faces,
                        distance: 1.0,
                    },
                    "OffsetFaces",
                )
            }
            "design.move_faces" => {
                need_material(has_solid)?;
                let faces = Self::selected_face_picks(ctx, body);
                let pick = *faces
                    .last()
                    .ok_or("Click the faces to move in the viewport first")?;
                // Out along the last face picked, as a first guess to change.
                let translation = pick.normal;
                let axis_point = pick.point;
                (
                    DesignFeature::MoveFaces {
                        faces,
                        translation,
                        angle_deg: 0.0,
                        axis_point,
                        axis_dir: [0.0, 0.0, 1.0],
                    },
                    "MoveFaces",
                )
            }
            "design.project" => {
                need_material(has_solid)?;
                // The faces may be picked now or added in its task.
                (
                    DesignFeature::Project {
                        sketch: need_sketch(sketch)?,
                        faces: Self::selected_face_picks(ctx, body),
                        mode: kernel_api::ProjectMode::Emboss,
                        height: 1.0,
                    },
                    "Project",
                )
            }
            "design.delete_faces" => {
                need_material(has_solid)?;
                let faces = Self::selected_face_picks(ctx, body);
                if faces.is_empty() {
                    return Err("Click the faces to delete in the viewport first".into());
                }
                (DesignFeature::DeleteFaces { faces }, "DeleteFaces")
            }
            "design.thickness" => {
                need_material(has_solid)?;
                let faces = Self::selected_face_picks(ctx, body);
                if faces.is_empty() {
                    return Err("Click the faces to open in the viewport first".into());
                }
                (
                    DesignFeature::Thickness {
                        both_sides: false,
                        value: 1.0,
                        faces,
                        inward: true,
                        join: kernel_api::ThicknessJoin::Intersection,
                    },
                    "Thickness",
                )
            }
            "design.mirror" => {
                need_material(has_solid)?;
                let original = Self::selected_design_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    DesignFeature::Mirrored {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        originals: original.into_iter().collect(),
                        plane: Self::selected_mirror_face(ctx, body).unwrap_or(MirrorPlane::YZ),
                    },
                    "Mirrored",
                )
            }
            "design.linear_pattern" => {
                need_material(has_solid)?;
                let original = Self::selected_design_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    DesignFeature::LinearPattern {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        originals: original.into_iter().collect(),
                        axis: PatternAxis::X,
                        length: 30.0,
                        occurrences: 3,
                        spacing_mode: false,
                        spacings: Vec::new(),
                        reversed: false,
                    },
                    "LinearPattern",
                )
            }
            "design.polar_pattern" => {
                need_material(has_solid)?;
                let original = Self::selected_design_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    DesignFeature::PolarPattern {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        originals: original.into_iter().collect(),
                        axis: PatternAxis::Z,
                        angle_deg: 360.0,
                        occurrences: 4,
                        reversed: false,
                        step_mode: false,
                        angles: Vec::new(),
                    },
                    "PolarPattern",
                )
            }
            "design.multi_transform" => {
                need_material(has_solid)?;
                let original = Self::selected_design_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    // One step to start from, so it builds as it opens.
                    DesignFeature::MultiTransform {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
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
            "design.clone" => {
                if has_solid {
                    return Err("A clone can only start an empty body".into());
                }
                let other = ctx
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.id != body && ctx.document.imported_brep_blob(b.id).is_some())
                    .map(|b| b.id)
                    .ok_or_else(|| {
                        given.say(
                            "Build another body first; the clone copies its solid",
                            "A clone copies another body's solid: build one first \
                             (`pc.doc.rebuild()`)",
                        )
                    })?;
                (DesignFeature::Clone { source: other }, "Clone")
            }
            "design.scaled" => {
                need_material(has_solid)?;
                let original = Self::selected_design_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    DesignFeature::MultiTransform {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
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
            "design.borrow" => (Self::borrow_from_selection(ctx, body)?, "Borrowed"),
            "design.boolean" => {
                need_material(has_solid)?;
                let other = ctx
                    .document
                    .bodies()
                    .iter()
                    .rev()
                    .find(|b| b.id != body)
                    .map(|b| b.id)
                    .ok_or_else(|| {
                        given.say(
                            "Create a second body to combine with first",
                            "A boolean needs a second body to combine with: make one and give \
                             it as `tool_body`",
                        )
                    })?;
                (
                    DesignFeature::BodyBoolean {
                        more_tools: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
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

    /// What a new borrow in `body` takes from the selection: the faces and
    /// edges picked on another body, else the latest sketch of another
    /// body.
    fn borrow_from_selection(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Result<DesignFeature, String> {
        let face_body = ctx
            .selected_body_id
            .map(BodyId)
            .filter(|b| *b != body && ctx.selected_face.is_some());
        let edge_body = ctx
            .selected_edges
            .first()
            .map(|e| BodyId(e.body))
            .filter(|b| *b != body);
        if let Some(from) = face_body.or(edge_body) {
            let faces = ctx
                .selected_face_in(from)
                .filter(|_| face_body == Some(from))
                .map(FacePick::of)
                .into_iter()
                .collect();
            let edges = ctx
                .selected_edges_in(from)
                .iter()
                .filter(|e| BodyId(e.body) == from)
                .map(EdgePick::of)
                .collect();
            return Ok(DesignFeature::Borrow {
                source: BorrowSource::Solid {
                    body: from,
                    faces,
                    edges,
                },
                frozen: None,
                options: Default::default(),
            });
        }
        let sketch = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body != Some(body))
            .max_by_key(|(id, n)| (n.seq, **id))
            .map(|(id, _)| *id)
            .ok_or(
                "Pick a face or an edge of another body, or draw a sketch in another body, first",
            )?;
        Ok(DesignFeature::Borrow {
            source: BorrowSource::Sketch(sketch),
            frozen: None,
            options: Default::default(),
        })
    }

    /// Create a datum feature anchored to what is picked (on a face; along
    /// an edge for a line, at a circle's centre for a point, square to the
    /// edge otherwise; the XY base plane with nothing picked) and select it
    /// for editing.
    fn insert_datum(&mut self, ctx: &mut WorkbenchRuntimeContext, tool: &str) -> InputResult {
        use core_document::{AttachmentOffset, DatumAttachment, DatumFeature, DatumShape};
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        let shape = match tool {
            "design.datum_plane" => DatumShape::Plane { size: 30.0 },
            "design.datum_line" => DatumShape::Line { length: 40.0 },
            "design.coordinate_system" => DatumShape::CoordinateSystem { size: 20.0 },
            _ => DatumShape::Point,
        };
        let on_body = ctx.selected_body_id == Some(body.0);
        let edge = ctx
            .selected_edges_in(body)
            .first()
            .map(|edge| datum_refs::edge_anchor(edge, edge.body == body.0));
        let attachment = match (ctx.selected_face_in(body), edge) {
            (Some(face), _) => DatumAttachment::Face {
                face: datum_refs::face_anchor(&face, on_body),
            },
            (None, Some(edge)) => match shape {
                DatumShape::Line { .. } => DatumAttachment::AlongEdge { edge },
                DatumShape::Point if edge.circle.is_some() => DatumAttachment::CurveCentre { edge },
                _ => DatumAttachment::NormalToEdge {
                    along: None,
                    edge,
                    spot: core_document::EdgeSpot::Picked,
                },
            },
            (None, None) => DatumAttachment::BasePlane(core_document::BasePlane::XY),
        };
        let mut datum = DatumFeature {
            shape,
            attachment,
            offset: AttachmentOffset::default(),
        };
        if let Err(problem) = datum_refs::settle(ctx, body, &mut datum) {
            ctx.log_warn(format!("The datum stays where it was picked: {problem}"));
        }
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
                    base: None,
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
        match self.create_feature(ctx, tool, body, Given::default(), |_| Ok(())) {
            Ok(made) => {
                self.pending_task_from_tool = Some(ToolMade {
                    feature: made.id,
                    hidden: made.hidden,
                    tool: tool.to_string(),
                    body,
                    base: made.base,
                });
                ctx.active_document_object = Some(made.id);
            }
            Err(message) => ctx.log_warn(message),
        }
        InputResult::consumed()
    }

    /// Add the feature `tool` makes, from the selection, to `body` and mark
    /// it for rebuild; `edit` changes it before it goes in. An imported
    /// body first takes its solid as a base (`take_base`); one that cannot
    /// (a mesh, a linked copy, a part linked from another file) has no
    /// history to build on, so the feature goes to a body of its own, which
    /// leaves the import as it was.
    pub(crate) fn create_feature(
        &self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        body: BodyId,
        given: Given,
        edit: impl FnOnce(&mut DesignFeature) -> Result<(), String>,
    ) -> Result<CreatedFeature, String> {
        let mut given_base = None;
        let body = if ctx.document.body_solid_is_imported(body)
            && let Some(taken) = take_base(ctx, body)
        {
            // Its imported solid becomes the start of its history.
            given_base = Some(taken);
            body
        } else if surface_body(ctx.document, body) {
            let name = ctx
                .document
                .bodies()
                .iter()
                .find(|b| b.id == body)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "the surface body".to_string());
            let fresh = ctx.document.create_body(None);
            ctx.log_warn(format!(
                "`{name}` is built from surfaces; the feature goes into a new body"
            ));
            fresh
        } else if ctx.document.body_solid_is_imported(body) {
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
            Self::feature_for_tool(base_tool_id(tool), tool_variant(tool), ctx, body, given)?;
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
        Ok(CreatedFeature {
            id,
            hidden,
            base: given_base,
        })
    }
}

/// What a feature command gives beside the selection a tool reads.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Given {
    /// Run as a command: a refusal names the arguments to give rather
    /// than what to select.
    pub scripted: bool,
    /// A pipe's path.
    pub spine: Option<FeatureId>,
}

impl Given {
    /// The refusal `ui` for the window, `script` for a command.
    fn say(&self, ui: &str, script: &str) -> String {
        if self.scripted { script } else { ui }.to_string()
    }
}

/// Whether the Surface bench builds `body`: its features are surface
/// steps, and a Design feature beside them would build over them.
fn surface_body(document: &Document, body: BodyId) -> bool {
    document
        .feature_tree()
        .all_nodes()
        .any(|(_, n)| n.workbench_id.as_str() == "wb.surface" && n.body == Some(body))
}

/// Give an imported body a history: its solid kept as its base, and a
/// Base feature first to start from it. The Base feature, or `None` for a
/// body that cannot take one (a mesh, a linked copy, a part linked from
/// another file, whose shapes come from elsewhere).
pub(crate) fn take_base(ctx: &mut WorkbenchRuntimeContext, body: BodyId) -> Option<FeatureId> {
    let linked = ctx
        .document
        .bodies()
        .iter()
        .any(|b| b.id == body && b.link.is_some());
    if linked || !ctx.document.set_body_base(body, true) {
        return None;
    }
    let name = DesignWorkbench::next_feature_name(ctx, "Base");
    let id = ctx
        .document
        .add_feature_in_body(DesignFeature::Base {}, name, Some(body))
        .ok()?;
    // First in the body's history, before any sketch already on it: the
    // body's shape starts there.
    if let Err(why) = ctx.document.move_feature_after(id, None) {
        ctx.log_warn(format!("The base could not go first in the body: {why:?}"));
    }
    ctx.document.mark_feature_dirty(id);
    Some(id)
}

/// A feature `create_feature` added, the sketches it hid, and the base
/// shape it gave its body first, if it did.
pub(crate) struct CreatedFeature {
    pub id: FeatureId,
    pub hidden: Vec<FeatureId>,
    pub base: Option<FeatureId>,
}

/// Default keys of the tools; the user can rebind them in Preferences.
/// Shift and a letter makes the subtractive form of what the letter adds.
const TOOL_KEYS: &[(&str, &str)] = &[
    ("design.new_body", "B"),
    ("design.new_sketch", "S"),
    ("design.pad", "E"),
    ("design.pocket", "Shift+E"),
    ("design.revolve", "R"),
    ("design.groove", "Shift+R"),
    ("design.loft", "L"),
    ("design.subtractive_loft", "Shift+L"),
    ("design.pipe", "W"),
    ("design.subtractive_pipe", "Shift+W"),
    ("design.hole", "Shift+H"),
    ("design.fillet", "U"),
    ("design.chamfer", "C"),
    ("design.mirror", "M"),
];

/// Register `tool` with its default key, if it has one.
fn register(context: &mut WorkbenchContext, tool: ToolDescriptor) {
    let key = TOOL_KEYS.iter().find(|(id, _)| *id == tool.id);
    context.register_tool(match key {
        Some((_, key)) => tool.shortcut(key),
        None => tool,
    });
}

impl DesignWorkbench {
    /// The feature whose task is open.
    fn task_feature(&self) -> Option<FeatureId> {
        #[cfg(feature = "egui")]
        {
            self.task.as_ref().map(|t| t.feature)
        }
        #[cfg(not(feature = "egui"))]
        {
            None
        }
    }

    /// The handle held, else the one the open task's feature offers.
    fn handle(&self, ctx: &WorkbenchRuntimeContext) -> Option<handles::Handle> {
        self.held.or_else(|| {
            let eye = glam::Vec3::from_array(ctx.camera_position);
            handles::handle_of(ctx.document, self.task_feature()?, eye)
        })
    }

    /// The occurrences of the open task's pattern, where their dots stand.
    fn occurrences(&self, ctx: &WorkbenchRuntimeContext) -> Vec<instances::Occurrence> {
        self.task_feature()
            .map(|feature| instances::occurrences_of(ctx.document, feature))
            .unwrap_or_default()
    }

    /// A click on an occurrence's dot of the open pattern leaves it out or
    /// brings it back; moves keep which dot is under the pointer.
    fn occurrence_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        use core_document::MouseButton;
        match event {
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                let occurrences = self.occurrences(ctx);
                self.hovered_occurrence = instances::hit(ctx, &occurrences, *viewport_pos);
                None
            }
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => {
                let feature = self.task_feature()?;
                let occurrences = instances::occurrences_of(ctx.document, feature);
                let index = instances::hit(ctx, &occurrences, *viewport_pos)?;
                let mut part = ctx
                    .document
                    .get_feature_data(feature)
                    .and_then(|d| DesignFeature::from_json(d).ok())?;
                if part.toggle_skipped(index)
                    && ctx
                        .document
                        .update_feature_data(feature, part.to_json())
                        .is_ok()
                {
                    ctx.document.mark_feature_dirty(feature);
                }
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    /// A press on the open task's handle takes hold of it; moves drag the
    /// number it stands for, written live as the panel's edits are.
    fn handle_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        use core_document::MouseButton;
        match event {
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => {
                if let Some(handle) = self.handle(ctx)
                    && handles::within_reach(ctx, &handle, *viewport_pos)
                {
                    self.held = Some(handle);
                    self.held_moved = false;
                    return Some(InputResult::consumed());
                }
                // A click on a region of the open task's sketch chooses it.
                let task = self.task_feature()?;
                region_pick::click(ctx, task, *viewport_pos).then(InputResult::consumed)
            }
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                let step = if ctx.shift_down {
                    self.options.handle_fine_step
                } else {
                    self.options.handle_step
                };
                let held = self.held.as_mut()?;
                let (origin, dir) = ctx.viewport_to_ray(*viewport_pos)?;
                let mut value = held.value_at(
                    glam::Vec3::from_array(origin),
                    glam::Vec3::from_array(dir),
                    step,
                )?;
                // Through its start, a pad or pocket turns round.
                let flip = value < 0.0;
                if flip {
                    value = -value;
                    held.dir = -held.dir;
                }
                if value != held.value || flip {
                    held.value = value;
                    self.held_moved = true;
                    let feature = held.feature;
                    if let Some(mut part) = ctx
                        .document
                        .get_feature_data(feature)
                        .and_then(|d| DesignFeature::from_json(d).ok())
                        && (!flip || handles::reverse(&mut part))
                        && handles::set_value(&mut part, value)
                        && ctx
                            .document
                            .update_feature_data(feature, part.to_json())
                            .is_ok()
                        && self.options.update_while_editing
                    {
                        ctx.document.mark_feature_dirty(feature);
                    }
                }
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                ..
            } => {
                let held = self.held.take()?;
                // A click on a pad's or pocket's handle turns it round.
                if !self.held_moved
                    && held.reversible
                    && let Some(mut part) = ctx
                        .document
                        .get_feature_data(held.feature)
                        .and_then(|d| DesignFeature::from_json(d).ok())
                    && handles::reverse(&mut part)
                    && let Err(e) = ctx
                        .document
                        .update_feature_data(held.feature, part.to_json())
                {
                    ctx.log_warn(format!("Could not turn the feature round: {e}"));
                }
                ctx.document.mark_feature_dirty(held.feature);
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    /// Edit › Copy (or Cut) of the feature selected in the tree.
    fn copy(&mut self, ctx: &mut WorkbenchRuntimeContext, cut: bool) -> bool {
        let Some(id) = ctx.active_document_object else {
            return false;
        };
        let Some(copied) = clipboard::Clipboard::copy(ctx.document, id) else {
            return false;
        };
        let count = copied.ids().len();
        if cut {
            clipboard::cut(ctx.document, &copied);
            ctx.active_document_object = None;
            ctx.request(HostRequest::JournalLabel("Cut".into()));
            ctx.log_info(format!("Cut {count} feature(s)"));
        } else {
            ctx.log_info(format!("Copied {count} feature(s)"));
        }
        self.clipboard = Some(copied);
        true
    }

    /// Edit › Paste: the copied features again, after the tip of the body
    /// selected (or the selected feature's body).
    fn paste(&mut self, ctx: &mut WorkbenchRuntimeContext) -> bool {
        let Some(copied) = self.clipboard.as_ref().filter(|c| !c.is_empty()) else {
            return false;
        };
        let body = ctx
            .active_document_object
            .and_then(|f| ctx.document.get_feature_meta(f))
            .and_then(|n| n.body)
            .or(ctx.selected_body_id.map(BodyId))
            .filter(|b| ctx.document.bodies().iter().any(|x| x.id == *b));
        let Some(body) = body else {
            ctx.log_warn("Select a body to paste into");
            return true;
        };
        let made = copied.paste(ctx.document, body);
        ctx.active_document_object = made.last().copied();
        ctx.request(HostRequest::JournalLabel("Paste".into()));
        ctx.log_info(format!("Pasted {} feature(s)", made.len()));
        true
    }
}

impl Workbench for DesignWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.design",
            "Design",
            "Feature-based solid modeling workbench.",
        )
        .icon("workbench-part-design")
        .feature_kinds(["wb.design", "core.datum"])
    }

    fn rebuild_jobs(&self, document: &mut Document) -> Vec<core_document::RebuildJob> {
        build::rebuild_jobs(document)
    }

    fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        build::invalidate_body(document, body);
    }

    fn invalidate_all(&self, document: &mut Document) {
        build::mark_all_design_features_dirty(document);
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.task_feature()
    }

    fn references(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Vec<core_document::FeatureReference> {
        references::references(document, id, node)
    }

    fn set_reference(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        key: &str,
        to: core_document::ReferenceChoice,
    ) -> Result<(), String> {
        references::set_reference(ctx, id, key, to)
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        #[cfg(feature = "egui")]
        if task::task_kind(ctx, id).is_some() {
            self.edit_request = Some(id);
        }
        #[cfg(not(feature = "egui"))]
        let _ = (ctx, id);
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
        let feature = DesignFeature::from_json(&node.data).ok();
        FeatureInfo {
            icon: feature.as_ref().map(|f| f.icon()).unwrap_or("tree-feature"),
            kind_label: feature
                .as_ref()
                .map(|f| match f.cosmetic_thread() {
                    Some(thread) => format!("{} · {thread} thread", f.kind_label()),
                    None => f.kind_label().to_string(),
                })
                .unwrap_or_else(|| "Design feature".to_string()),
            family_label: "Design feature".to_string(),
            builds_solid: !matches!(feature, Some(DesignFeature::Borrow { .. })),
        }
    }

    /// A borrow draws what it lends where its body sits, in the palette's
    /// external colour: the sketch's curves, or the outlines of the faces
    /// and edges.
    fn passive_geometry(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Option<core_document::PassiveGeometry> {
        // A hole's cosmetic thread: a helix down its bore.
        if let Some(feature @ DesignFeature::Hole { .. }) = document
            .feature_values(id)
            .and_then(|data| DesignFeature::from_json(data).ok())
            && feature.cosmetic_thread().is_some()
        {
            let body = node.body;
            let mesh = build::cosmetic_thread_lines(document, body, &feature);
            return (!mesh.positions.is_empty()).then(|| core_document::PassiveGeometry {
                revision: core_document::data_revision(&feature.to_json())
                    ^ body
                        .and_then(|b| document.imported_geometry(b))
                        .map_or(0, |g| g.revision),
                mesh,
                tint: core_document::PassiveTint::External,
                region: None,
            });
        }
        let borrow = borrow::borrow_of(document, id)?;
        let mesh = borrow::lines(document, &borrow);
        if mesh.positions.is_empty() {
            return None;
        }
        Some(core_document::PassiveGeometry {
            mesh,
            revision: borrow::lines_revision(document, id, &borrow),
            tint: core_document::PassiveTint::External,
            region: None,
        })
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        // The hole cuts the user keeps, read once as the bench starts.
        hole_tables::user_cut_profiles();
        let action = |id: &str, label: &str, icon: &'static str, category: &str| {
            ToolDescriptor::new_action(id, label, Some(category)).icon(icon)
        };
        // Structure and sketches.
        register(
            context,
            action("design.new_body", "Create body", "body", "structure"),
        );
        register(
            context,
            action(
                "design.new_sketch",
                "Create sketch",
                "sketch-new",
                "structure",
            ),
        );
        register(
            context,
            action(
                "design.edit_sketch",
                "Edit sketch",
                "sketch-edit",
                "structure",
            ),
        );
        register(
            context,
            action(
                "design.map_sketch",
                "Map sketch to face",
                "sketch-map",
                "structure",
            ),
        );
        // Datums.
        register(
            context,
            action("design.datum_point", "Datum point", "datum-point", "datum"),
        );
        register(
            context,
            action("design.datum_line", "Datum line", "datum-line", "datum"),
        );
        register(
            context,
            action("design.datum_plane", "Datum plane", "datum-plane", "datum"),
        );
        register(
            context,
            action(
                "design.coordinate_system",
                "Local coordinate system",
                "coordinate-system",
                "datum",
            ),
        );
        register(context, action("design.clone", "Clone", "clone", "datum"));
        register(
            context,
            action(
                "design.borrow",
                "Borrow geometry",
                "clone-geometry",
                "datum",
            ),
        );
        // Profiles made from numbers.
        register(context, generators::tool());
        generators::register(context);
        // Additive.
        register(context, action("design.pad", "Pad", "pad", "additive"));
        register(
            context,
            action("design.revolve", "Revolution", "revolution", "additive"),
        );
        register(
            context,
            action("design.loft", "Additive loft", "additive-loft", "additive"),
        );
        register(
            context,
            action("design.pipe", "Additive pipe", "additive-pipe", "additive"),
        );
        register(
            context,
            action(
                "design.helix",
                "Additive helix",
                "additive-helix",
                "additive",
            ),
        );
        register(
            context,
            action(
                "design.primitive",
                "Additive primitive",
                "additive-box",
                "additive",
            )
            .variants(primitive_variants(false)),
        );
        // Subtractive.
        register(
            context,
            action("design.pocket", "Pocket", "pocket", "subtractive"),
        );
        register(
            context,
            action("design.hole", "Hole", "hole", "subtractive"),
        );
        register(
            context,
            action("design.groove", "Groove", "groove", "subtractive"),
        );
        register(
            context,
            action(
                "design.subtractive_loft",
                "Subtractive loft",
                "subtractive-loft",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "design.subtractive_pipe",
                "Subtractive pipe",
                "subtractive-pipe",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "design.subtractive_helix",
                "Subtractive helix",
                "subtractive-helix",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "design.subtractive_primitive",
                "Subtractive primitive",
                "subtractive-box",
                "subtractive",
            )
            .variants(primitive_variants(true)),
        );
        // Transformations.
        register(
            context,
            action("design.mirror", "Mirrored", "mirrored", "transform"),
        );
        register(
            context,
            action(
                "design.linear_pattern",
                "Linear pattern",
                "linear-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.polar_pattern",
                "Polar pattern",
                "polar-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.multi_transform",
                "Multi-transform",
                "multi-transform",
                "transform",
            ),
        );
        register(
            context,
            action("design.scaled", "Scaled", "scaled", "transform"),
        );
        // Dress-up.
        register(
            context,
            action("design.fillet", "Fillet", "fillet", "dressup"),
        );
        register(
            context,
            action("design.chamfer", "Chamfer", "chamfer", "dressup"),
        );
        register(context, action("design.draft", "Draft", "draft", "dressup"));
        register(
            context,
            action("design.thickness", "Thickness", "thickness", "dressup"),
        );
        register(
            context,
            action(
                "design.offset_faces",
                "Offset faces",
                "offset-geometry",
                "dressup",
            ),
        );
        register(
            context,
            action(
                "design.move_faces",
                "Move faces",
                "move-geometry",
                "dressup",
            ),
        );
        register(
            context,
            action("design.delete_faces", "Delete faces", "delete", "dressup"),
        );
        register(
            context,
            action(
                "design.project",
                "Project sketch onto faces",
                "sketch-map",
                "dressup",
            ),
        );
        register(
            context,
            action(
                "design.recognize_holes",
                "Recognize holes",
                "hole",
                "dressup",
            ),
        );
        // Boolean.
        register(
            context,
            action("design.boolean", "Boolean", "boolean", "boolean"),
        );
        // Measure.
        register(
            context,
            action("design.centre_line", "Centre line", centre::ICON, "measure"),
        );
    }

    /// A feature row offers Duplicate, and a move into each other body
    /// built here (not one read from a file); a sketch's row offers each
    /// flat face its body borrows to map it onto.
    fn menu_items(&self, scope: &MenuScope, document: &Document) -> Vec<MenuItem> {
        let MenuScope::TreeFeature(id) = scope else {
            return Vec::new();
        };
        let Some(node) = document.get_feature_meta(*id) else {
            return Vec::new();
        };
        let Some(from) = node.body else {
            return Vec::new();
        };
        if node.workbench_id.as_str() == "wb.sketch" {
            return borrow::flat_faces_of_body(document, from)
                .into_iter()
                .enumerate()
                .map(|(i, (r, name))| {
                    let item = MenuItem::new(
                        format!("{MAP_SKETCH}:{}:{}", r.borrow.0, r.index),
                        format!("Map onto {name}"),
                    )
                    .hint("The sketch follows the borrowed face");
                    if i == 0 {
                        item.separator_before()
                    } else {
                        item
                    }
                })
                .collect();
        }
        if !matches!(node.workbench_id.as_str(), "wb.design" | "core.datum") {
            return Vec::new();
        }
        let duplicate = MenuItem::new(DUPLICATE, "Duplicate")
            .hint("A copy after the tip, with its own sketch")
            .separator_before();
        std::iter::once(duplicate)
            .chain(
                document
                    .bodies()
                    .iter()
                    .filter(|b| b.id != from && !document.body_solid_is_imported(b.id))
                    .enumerate()
                    .map(|(i, b)| {
                        let item = MenuItem::new(
                            format!("{MOVE_TO_BODY}:{}", b.id.0),
                            format!("Move to {}", b.name),
                        )
                        .hint("With the sketch and datums only it uses");
                        if i == 0 {
                            item.separator_before()
                        } else {
                            item
                        }
                    }),
            )
            .collect()
    }

    fn on_command(
        &mut self,
        id: &str,
        scope: &MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        match (scope, id) {
            (MenuScope::EditMenu, "edit.copy") => return self.copy(ctx, false),
            (MenuScope::EditMenu, "edit.cut") => return self.copy(ctx, true),
            (MenuScope::EditMenu, "edit.paste") => return self.paste(ctx),
            (MenuScope::TreeFeature(feature), DUPLICATE) => {
                duplicate_recorded(ctx, *feature, None);
                return true;
            }
            (MenuScope::TreeFeature(sketch), id) if id.starts_with(MAP_SKETCH) => {
                let face = id
                    .strip_prefix(MAP_SKETCH)
                    .and_then(|rest| rest.strip_prefix(':'))
                    .and_then(|rest| rest.split_once(':'))
                    .and_then(|(borrow, index)| {
                        Some(crate::feature::BorrowedRef {
                            borrow: FeatureId(uuid::Uuid::parse_str(borrow).ok()?),
                            index: index.parse().ok()?,
                        })
                    });
                let Some(face) = face else {
                    return false;
                };
                match borrow::map_sketch(ctx.document, *sketch, face) {
                    Ok(()) => {
                        ctx.record(
                            MAP_SKETCH,
                            commands::object(serde_json::json!({
                                "sketch": sketch.0.to_string(),
                                "borrow": face.borrow.0.to_string(),
                                "index": face.index,
                            })),
                            serde_json::Value::Null,
                        );
                        ctx.request(HostRequest::JournalLabel("Map sketch".into()));
                    }
                    Err(message) => ctx.log_warn(format!("Cannot map the sketch: {message}")),
                }
                return true;
            }
            _ => {}
        }
        let (MenuScope::TreeFeature(feature), Some(body)) = (scope, id.strip_prefix(MOVE_TO_BODY))
        else {
            return false;
        };
        let Some(body) = body
            .strip_prefix(':')
            .and_then(|b| uuid::Uuid::parse_str(b).ok())
            .map(BodyId)
        else {
            return false;
        };
        match commands::move_feature(ctx, *feature, body) {
            Ok(moved) => {
                ctx.record(
                    MOVE_TO_BODY,
                    commands::object(serde_json::json!({
                        "feature": feature.0.to_string(),
                        "body": body.0.to_string(),
                    })),
                    serde_json::json!(moved.iter().map(|f| f.0.to_string()).collect::<Vec<_>>()),
                );
                ctx.request(HostRequest::JournalLabel("Move to body".into()));
            }
            Err(message) => ctx.log_warn(format!("Cannot move the feature: {message}")),
        }
        true
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        if generators::is_command(id) {
            return generators::command(id, args, ctx);
        }
        commands::run(self, id, args, ctx)
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Design workbench activated");
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if let Some(result) = self.occurrence_input(event, ctx) {
            return result;
        }
        if let Some(result) = self.handle_input(event, ctx) {
            return result;
        }
        // Feature tools are Actions: the host hands them over the moment they
        // are activated and clears them once handled.
        let base = active_tool.map(base_tool_id);
        match base {
            Some("design.new_body") => {
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
            Some("design.recognize_holes") => {
                let Some(body) = Self::target_body(ctx) else {
                    ctx.log_warn("Select a body whose holes to recognize");
                    return InputResult::consumed();
                };
                match recognize::recognize_holes(ctx, body) {
                    Ok(made) => {
                        ctx.record(
                            "design.recognize_holes",
                            commands::object(serde_json::json!({"body": body.0.to_string()})),
                            serde_json::json!({
                                "holes": made.holes,
                                "left": made.left,
                                "features": made
                                    .features
                                    .iter()
                                    .map(|f| f.0.to_string())
                                    .collect::<Vec<_>>(),
                            }),
                        );
                        let left = if made.left > 0 {
                            format!(
                                "; {} bore(s) left as they are (counterbores, slots)",
                                made.left
                            )
                        } else {
                            String::new()
                        };
                        if made.holes == 0 {
                            ctx.log_info(format!("No round holes to recognize{left}"));
                        } else {
                            ctx.log_info(format!(
                                "{} hole(s) made Hole features{left}",
                                made.holes
                            ));
                        }
                        ctx.request(HostRequest::JournalLabel("Recognize holes".to_string()));
                    }
                    Err(message) => ctx.log_warn(format!("Cannot recognize holes: {message}")),
                }
                InputResult::consumed()
            }
            Some(
                tool @ ("design.datum_plane"
                | "design.datum_line"
                | "design.datum_point"
                | "design.coordinate_system"),
            ) => self.insert_datum(ctx, tool),
            Some("design.map_sketch") => {
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
                }
                .on_its_plane();
                let plane = wb_sketch::sketch::SketchPlane::from_face(
                    wb_sketch::boundary::widen3(face.point),
                    wb_sketch::boundary::widen3(face.normal),
                );
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
            Some("design.centre_line") => {
                self.start_centre_line(ctx);
                InputResult::consumed()
            }
            Some("design.edit_sketch") => {
                if Self::selected_sketch(ctx).is_some() {
                    // The sketcher picks the active object up as its edit
                    // session on activation.
                    ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from("wb.sketch")));
                } else {
                    ctx.log_warn("Select a sketch in the tree first");
                }
                InputResult::consumed()
            }
            Some("design.generator") => generators::insert(ctx, active_tool.unwrap_or_default()),
            Some("design.new_sketch") => Self::start_sketch(ctx, None),
            Some(tool) if tool.starts_with("design.") => {
                let full = active_tool.unwrap_or(tool);
                self.insert_feature(ctx, full)
            }
            _ => InputResult::ignored(),
        }
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        if self.centre.is_some() {
            return Some(TaskInfo {
                title: "Centre line".to_string(),
                icon: centre::ICON,
                confirmable: false,
                stepwise: false,
            });
        }
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
        if self.centre.is_some() {
            return self.centre_panel(ui, request);
        }
        self.draw_task_panel(ui, ctx, request)
    }

    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.centre = None;
        #[cfg(feature = "egui")]
        {
            self.task = None;
        }
    }

    fn on_deactivate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.centre = None;
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        if self.centre.is_some() {
            self.take_centre_pick(ctx);
        }
    }

    fn viewport_hud(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::ViewportHud> {
        self.centre_hud()
    }

    fn get_screen_space_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let mut lines = self.centre_overlays(ctx);
        if let Some(handle) = self.handle(ctx) {
            lines.extend(handles::overlays(ctx, &handle));
        }
        lines
    }

    fn get_screen_space_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        let mut marks = self.centre_marks(ctx);
        if let Some(handle) = self.handle(ctx) {
            marks.extend(handles::marks(ctx, &handle, self.held.is_some()));
        }
        marks.extend(instances::marks(
            ctx,
            &self.occurrences(ctx),
            self.hovered_occurrence,
        ));
        marks
    }

    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let mut labels = self.centre_labels(ctx);
        if let Some(handle) = self.held {
            labels.extend(handles::labels(ctx, &handle));
        }
        labels.extend(instances::labels(
            ctx,
            &self.occurrences(ctx),
            self.hovered_occurrence,
        ));
        labels
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let body = Self::target_body(ctx);
        let has_body = body.is_some();
        let has_sketch = Self::selected_sketch(ctx).is_some();
        let has_solid = body.map(|b| Self::body_has_solid(ctx, b)).unwrap_or(false);
        match base_tool_id(tool_id) {
            "design.new_body" => true,
            "design.edit_sketch" => has_sketch,
            "design.map_sketch" => has_sketch && ctx.selected_face.is_some(),
            "design.new_sketch"
            | "design.primitive"
            | "design.datum_plane"
            | "design.datum_line"
            | "design.datum_point"
            | "design.coordinate_system"
            | "design.generator" => has_body,
            "design.clone" => has_body && !has_solid,
            "design.borrow" => has_body && ctx.document.bodies().len() > 1,
            "design.scaled" => has_solid,
            // A pad or a pocket also takes a flat face of the solid, or a
            // borrow lending one, as its profile.
            "design.pad" => {
                has_sketch
                    || (has_solid && body.is_some_and(|b| Self::profile_without_sketch(ctx, b)))
            }
            "design.pocket" => {
                has_solid
                    && (has_sketch || body.is_some_and(|b| Self::profile_without_sketch(ctx, b)))
            }
            "design.revolve" | "design.loft" | "design.pipe" | "design.helix" => has_sketch,
            "design.groove"
            | "design.hole"
            | "design.project"
            | "design.subtractive_loft"
            | "design.subtractive_pipe"
            | "design.subtractive_helix" => has_sketch && has_solid,
            "design.subtractive_primitive" => has_solid,
            "design.fillet"
            | "design.chamfer"
            | "design.draft"
            | "design.thickness"
            | "design.delete_faces"
            | "design.offset_faces"
            | "design.move_faces"
            | "design.recognize_holes"
            | "design.mirror"
            | "design.linear_pattern"
            | "design.polar_pattern"
            | "design.multi_transform"
            | "design.boolean"
            | "design.centre_line" => has_solid,
            _ => false,
        }
    }

    fn has_settings(&self) -> bool {
        true
    }

    /// The Design preferences page.
    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui, filter: &str) {
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
                PrefRow::qty(
                    "Handle step",
                    ui_kit::widgets::QtyField::new(&mut self.options.handle_step)
                        .unit("mm")
                        .decimals(2)
                        .range(0.01..=100.0),
                )
                .hint("A view handle's drag moves the number in these steps"),
                PrefRow::qty(
                    "Fine handle step",
                    ui_kit::widgets::QtyField::new(&mut self.options.handle_fine_step)
                        .unit("mm")
                        .decimals(2)
                        .range(0.01..=100.0),
                )
                .hint("The step while Shift is held"),
            ],
            filter,
        );
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

    /// A primitive attached by a mode takes what the last build found of
    /// what it stands on.
    fn derive_on_solid(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        probed: &core_document::rebuild::ProbedReferences,
    ) -> bool {
        use core_document::WorkbenchFeature;
        if node.workbench_id.as_str() != "wb.design" {
            return false;
        }
        let Ok(mut feature) = DesignFeature::from_json(values) else {
            return false;
        };
        let DesignFeature::Primitive {
            attached: Some(attached),
            ..
        } = &mut feature
        else {
            return false;
        };
        if probed.probes != attached.probes() {
            return false;
        }
        let mut datum = attached.datum();
        datum.take_answers(&probed.answers);
        let followed = crate::feature::Attached::from_datum(&datum);
        if followed == **attached {
            return false;
        }
        **attached = followed;
        *values = feature.to_json();
        true
    }

    /// A datum made from a sketch's points or lines takes where they stand
    /// now; so does a primitive attached by a mode, which follows the
    /// datums it stands on too.
    fn derive(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
        _document: &core_document::Document,
    ) -> bool {
        use core_document::{LineAnchor, PointAnchor, WorkbenchFeature};
        if node.workbench_id.as_str() == "wb.design" {
            let Ok(mut feature) = DesignFeature::from_json(values) else {
                return false;
            };
            let DesignFeature::Primitive {
                attached: Some(attached),
                ..
            } = &mut feature
            else {
                return false;
            };
            let mut datum = attached.datum();
            let mut datum_values = datum.to_json();
            if !self.derive(
                &core_document::FeatureNode {
                    workbench_id: core_document::WorkbenchId::from(core_document::DATUM_KIND),
                    ..node.clone()
                },
                &mut datum_values,
                values_of,
                _document,
            ) {
                datum.follow_datums(&|id| {
                    let data = values_of(id)?;
                    core_document::DatumFeature::from_json(&data)
                        .ok()
                        .map(|d| d.frame())
                });
            } else if let Ok(followed) = core_document::DatumFeature::from_json(&datum_values) {
                datum = followed;
                datum.follow_datums(&|id| {
                    let data = values_of(id)?;
                    core_document::DatumFeature::from_json(&data)
                        .ok()
                        .map(|d| d.frame())
                });
            }
            let followed = crate::feature::Attached::from_datum(&datum);
            if followed == **attached {
                return false;
            }
            **attached = followed;
            *values = feature.to_json();
            return true;
        }
        if node.workbench_id.as_str() != core_document::DATUM_KIND {
            return false;
        }
        let Ok(mut datum) = core_document::DatumFeature::from_json(values) else {
            return false;
        };
        let before = datum;
        let at = |sketch: FeatureId, element: uuid::Uuid| {
            let feature = wb_sketch::SketchFeature::from_json(&values_of(sketch)?).ok()?;
            datum_refs::sketch_points_of(&feature, element)
        };
        for anchor in datum.sketch_points_mut() {
            if let PointAnchor::Sketch {
                sketch,
                element,
                point,
            } = anchor
                && let Some([now]) =
                    at(*sketch, *element).and_then(|p| <[[f32; 3]; 1]>::try_from(p).ok())
            {
                *point = now;
            }
        }
        for anchor in datum.sketch_lines_mut() {
            if let LineAnchor::Sketch {
                sketch,
                element,
                start,
                end,
            } = anchor
                && let Some([a, b]) =
                    at(*sketch, *element).and_then(|p| <[[f32; 3]; 2]>::try_from(p).ok())
            {
                *start = a;
                *end = b;
            }
        }
        if datum == before {
            return false;
        }
        *values = datum.to_json();
        true
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
        meshes.extend(
            self.task_feature()
                .and_then(|task| region_pick::overlay(ctx, task)),
        );
        meshes
    }
}

/// Wireframe mesh for a datum, in its body's frame.
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
            // A degenerate-thin triangle along the x-axis; the edge list is
            // what the viewer reads.
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
pub(crate) fn next_name<'a>(names: impl Iterator<Item = &'a str>, base: &str) -> String {
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
        let mut wb = DesignWorkbench::default();
        let mut doc = Document::new("t");
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        let result = wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("design.new_body"),
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
            let mut wb = DesignWorkbench::default();
            let mut doc = Document::new("t");
            let body = doc.create_body(None);
            let pad = doc
                .add_feature_in_body(
                    DesignFeature::Primitive {
                        keep_inside: false,
                        attached: None,
                        refine: false,
                        boolean_tolerance: None,
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
                name: 0,
                point: [10.0, 2.5, 4.0],
                normal: [1.0, 0.0, 0.0],
                surface,
            });
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("design.mirror"),
                &mut ctx,
            );
            let mirrored = doc
                .feature_tree()
                .all_nodes()
                .find_map(|(_, n)| match DesignFeature::from_json(&n.data).ok()? {
                    DesignFeature::Mirrored {
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
                name: 0,
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

    /// Pad and Pocket take a flat face of the solid as their profile: with
    /// one picked they are there to click, with a curved one or nothing
    /// they are not, and the pad made has the face for its profile.
    #[test]
    fn a_picked_flat_face_is_a_pad_s_or_pocket_s_profile() {
        let mut wb = DesignWorkbench::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        doc.add_feature_in_body(
            DesignFeature::Primitive {
                keep_inside: false,
                attached: None,
                refine: false,
                boolean_tolerance: None,
                kind: primitive_preset("box").unwrap(),
                placement: kernel_api::Placement::default(),
                subtractive: false,
            },
            "Box".into(),
            Some(body),
        )
        .unwrap();
        let top = |surface| core_document::FaceRef {
            name: 0,
            point: [5.0, 5.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            surface,
        };
        let flat = kernel_api::FaceSurface::Plane {
            origin: [0.0, 0.0, 10.0],
            normal: [0.0, 0.0, 1.0],
        };
        let round = kernel_api::FaceSurface::Cylinder {
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 5.0,
        };
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.selected_body_id = Some(body.0);
        let tools = |wb: &DesignWorkbench, ctx: &WorkbenchRuntimeContext| {
            (
                wb.is_tool_enabled("design.pad", ctx),
                wb.is_tool_enabled("design.pocket", ctx),
            )
        };
        assert_eq!(tools(&wb, &ctx), (false, false), "nothing picked");
        ctx.selected_face = Some(top(Some(round)));
        assert_eq!(
            tools(&wb, &ctx),
            (false, false),
            "a curved face has no plane"
        );
        ctx.selected_face = Some(top(Some(flat)));
        assert_eq!(tools(&wb, &ctx), (true, true), "a flat face is a profile");

        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("design.pad"),
            &mut ctx,
        );
        let profile = doc
            .feature_tree()
            .all_nodes()
            .find_map(|(_, n)| match DesignFeature::from_json(&n.data).ok()? {
                DesignFeature::Pad {
                    sketch,
                    profile_face,
                    ..
                } => Some((sketch, profile_face)),
                _ => None,
            })
            .expect("the tool made a pad");
        assert_eq!(profile.0, None, "no sketch");
        assert_eq!(profile.1.map(|f| f.point), Some([5.0, 5.0, 10.0]));
    }

    #[test]
    fn the_coordinate_system_tool_places_a_frame_on_the_body() {
        let mut wb = DesignWorkbench::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.selected_body_id = Some(body.0);
        assert!(wb.is_tool_enabled("design.coordinate_system", &ctx));
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("design.coordinate_system"),
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
        let wb = DesignWorkbench::default();
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
            &DesignFeature::Pad {
                regions: Vec::new(),
                profile_borrowed: None,
                extras: Default::default(),
                refine: false,
                boolean_tolerance: None,
                sketch: Some(FeatureId(uuid::Uuid::new_v4())),
                length: 10.0,
                reversed: false,
                symmetric: false,
                mode: ExtrudeMode::Dimension,
                length2: 0.0,
                taper_deg: 0.0,
                up_to_face: None,
                up_to_offset: 0.0,
                profile_face: None,
                direction: Default::default(),
                up_to_shape: Vec::new(),
                mode2: None,
                up_to_face2: None,
                up_to_offset2: 0.0,
                up_to_shape2: Vec::new(),
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
        let projection = DesignFeature::Project {
            sketch: FeatureId(uuid::Uuid::new_v4()),
            faces: Vec::new(),
            mode: kernel_api::ProjectMode::Emboss,
            height: 1.0,
        };
        assert!(ui_kit::icon::exists(projection.icon()));
    }

    /// A hole with a cosmetic thread names its designation where the tree
    /// says what the feature is; the same hole cutting nothing extra does
    /// not.
    #[test]
    fn a_cosmetic_thread_names_its_designation_in_the_feature_kind() {
        let hole = |cosmetic_thread: bool| DesignFeature::Hole {
            boolean_tolerance: None,
            cosmetic_thread,
            clearance: None,
            thread_length: Default::default(),
            refine: false,
            sketch: FeatureId(uuid::Uuid::new_v4()),
            diameter: 10.2,
            depth: 10.0,
            through_all: true,
            cut: crate::feature::HoleCut::None,
            thread: Some(crate::feature::ThreadSpec::new(
                crate::hole_tables::ThreadStandard::IsoMetricCoarse,
                "M12",
            )),
            threaded: true,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: crate::feature::HoleFit::Normal,
            drill_point: crate::feature::DrillPoint::Flat,
            point_in_depth: false,
            taper_deg: 0.0,
            nut_trap: None,
            reversed: false,
        };
        let kind = |feature: &DesignFeature| {
            let node = core_document::FeatureNode::new(FeatureId(uuid::Uuid::new_v4()), feature);
            DesignWorkbench::default().feature_info(&node).kind_label
        };
        assert_eq!(kind(&hole(true)), "Hole · M12 thread");
        assert_eq!(kind(&hole(false)), "Hole");
    }

    #[test]
    fn every_default_key_lands_on_a_tool_and_no_two_share_one() {
        let mut ctx = WorkbenchContext::default();
        DesignWorkbench::default().configure(&mut ctx);
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
        DesignWorkbench::default().configure(&mut ctx);
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
