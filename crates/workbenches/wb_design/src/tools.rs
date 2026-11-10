//! The feature a toolbar action makes from the selection.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{
    ChamferMode, DesignFeature, DrillPoint, EdgePick, ExtrudeDirection, ExtrudeMode, FacePick,
    HelixMode, HoleCut, HoleFit, MirrorPlane, PatternAxis, PatternPath, PipeCorner,
    PipeOrientation, RevolveAxis, RevolveMode, ScaleCentre, TransformStep, primitive_preset,
};
use crate::{DesignWorkbench, Given, build, region_pick};

impl DesignWorkbench {
    /// Build the default feature payload for a toolbar action, or explain why
    /// it can't be created from the current selection (and, for a command,
    /// its arguments).
    pub(crate) fn feature_for_tool(
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
        // The sketch's regions and straight curve picked in the view.
        let picked_regions = |sketch: Option<FeatureId>| {
            sketch.map_or_else(Vec::new, |s| region_pick::picked_regions(ctx, s))
        };
        let picked_axis = |sketch: Option<FeatureId>| {
            sketch
                .and_then(|s| region_pick::picked_axis(ctx, s))
                .unwrap_or_default()
        };

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
                        regions: picked_regions(sketch),
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
                        regions: picked_regions(sketch),
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
                    regions: picked_regions(sketch),
                    refine: false,
                    boolean_tolerance: None,
                    sketch: need_sketch(sketch)?,
                    angle_deg: 360.0,
                    axis: picked_axis(sketch),
                    project_axis: false,
                    reversed: false,
                    midplane: false,
                    second_angle_deg: None,
                    start_angle_deg: 0.0,
                    mode: RevolveMode::Angle,
                    up_to_face: None,
                },
                "Revolution",
            ),
            "design.groove" => {
                need_material(has_solid)?;
                (
                    DesignFeature::Groove {
                        regions: picked_regions(sketch),
                        keep_inside: false,
                        refine: false,
                        boolean_tolerance: None,
                        sketch: need_sketch(sketch)?,
                        angle_deg: 360.0,
                        axis: picked_axis(sketch),
                        project_axis: false,
                        reversed: false,
                        midplane: false,
                        second_angle_deg: None,
                        start_angle_deg: 0.0,
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
                // The latest sketch of the body other than `not`: a pipe
                // along its own profile builds nothing.
                let other_sketch = |not: FeatureId| {
                    ctx.document
                        .feature_tree()
                        .all_nodes()
                        .filter(|(id, n)| {
                            n.workbench_id.as_str() == wb_sketch::KIND
                                && n.body == Some(body)
                                && **id != not
                        })
                        .max_by_key(|(_, n)| n.seq)
                        .map(|(id, _)| *id)
                };
                // Curves picked on the selected sketch are the path, and
                // the profile is the body's latest other sketch.
                let path_curves = match (sketch, given.spine) {
                    (Some(s), None) => region_pick::picked_curves(ctx, s),
                    _ => Vec::new(),
                };
                let (profile, spine) = if let Some(path) =
                    sketch.filter(|_| !path_curves.is_empty())
                {
                    let profile = other_sketch(path).ok_or_else(|| {
                        "A pipe along picked curves needs a second sketch for its profile; \
                         draw one first"
                            .to_string()
                    })?;
                    (profile, path)
                } else {
                    let profile = need_sketch(sketch)?;
                    // The path is the one given, else the body's latest
                    // other sketch.
                    let spine = given
                        .spine
                        .or_else(|| other_sketch(profile))
                        .ok_or_else(|| {
                            given.say(
                                "A pipe needs a second sketch for its path; draw one first",
                                "A pipe needs a path: give `spine`, a sketch, or draw a second \
                                 sketch in the profile's body",
                            )
                        })?;
                    (profile, spine)
                };
                (
                    DesignFeature::Pipe {
                        keep_inside: false,
                        path_borrowed: Vec::new(),
                        path_curves,
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
                        start: Default::default(),
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
            "design.offset_solid" => {
                need_material(has_solid)?;
                (DesignFeature::OffsetSolid { distance: 1.0 }, "OffsetSolid")
            }
            "design.scale_body" => {
                need_material(has_solid)?;
                (
                    DesignFeature::Scale {
                        factor: 2.0,
                        per_axis: false,
                        factors: [1.0; 3],
                        about: ScaleCentre::Origin,
                    },
                    "Scale",
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
                        wrap: false,
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
                (
                    DesignFeature::Mirrored {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        // A script names its originals or takes the last
                        // feature, as its reference says; the toolbar reads
                        // the tree's selection.
                        originals: if given.scripted {
                            Self::selected_design_feature(ctx)
                                .or_else(|| Self::last_shape_feature(ctx, body))
                                .into_iter()
                                .collect()
                        } else {
                            Self::mirror_originals(ctx, body)
                        },
                        plane: Self::selected_mirror_face(ctx, body)
                            .or_else(|| Self::scan_symmetry_plane(ctx, body))
                            .unwrap_or(MirrorPlane::YZ),
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
            "design.circular_pattern" => {
                need_material(has_solid)?;
                let original = Self::selected_design_feature(ctx)
                    .or_else(|| Self::last_shape_feature(ctx, body));
                (
                    DesignFeature::CircularPattern {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        originals: original.into_iter().collect(),
                        axis: PatternAxis::Z,
                        outward: PatternAxis::X,
                        spacing: 10.0,
                        counts: vec![6, 12],
                    },
                    "CircularPattern",
                )
            }
            "design.point_pattern" => {
                need_material(has_solid)?;
                let sketch = sketch
                    .or_else(|| Self::last_sketch_where(ctx, body, build::sketch_point_count))
                    .ok_or_else(|| {
                        given.say(
                            "Select a sketch of points in the tree first",
                            "It needs a sketch of points: give `sketch`",
                        )
                    })?;
                let original = Self::last_shape_feature(ctx, body);
                (
                    DesignFeature::PointPattern {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        originals: original.into_iter().collect(),
                        sketch,
                    },
                    "PointPattern",
                )
            }
            "design.path_pattern" => {
                need_material(has_solid)?;
                let edge = ctx
                    .selected_edges_in(body)
                    .iter()
                    .find(|e| BodyId(e.body) == body)
                    .map(EdgePick::of);
                let path = match (sketch, edge) {
                    (Some(sketch), _) => PatternPath::Sketch { sketch, chain: 0 },
                    (None, Some(edge)) => PatternPath::Edge(edge),
                    (None, None) => Self::last_sketch_where(ctx, body, build::sketch_chain_count)
                        .map(|sketch| PatternPath::Sketch { sketch, chain: 0 })
                        .ok_or_else(|| {
                            given.say(
                                "Select a sketch of curves in the tree, or an edge of the \
                                     solid, first",
                                "It needs a path: give `sketch`, or `path = {Edge = \
                                     {point = {x, y, z}, direction = {x, y, z}}}`",
                            )
                        })?,
                };
                let original = if sketch.is_some() || edge.is_some() {
                    Self::last_shape_feature(ctx, body)
                } else {
                    Self::selected_design_feature(ctx)
                        .or_else(|| Self::last_shape_feature(ctx, body))
                };
                (
                    DesignFeature::PathPattern {
                        skipped: Vec::new(),
                        refine: false,
                        boolean_tolerance: None,
                        originals: original.into_iter().collect(),
                        path,
                        layout: Default::default(),
                    },
                    "PathPattern",
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
}
