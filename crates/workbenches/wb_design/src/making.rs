//! Making features: what the selection offers a tool, sketches started,
//! borrows, datums and features inserted, and imported bodies given a
//! base.

use crate::{
    BorrowSource, DesignFeature, DesignWorkbench, EdgePick, EdgeSel, FacePick, MirrorPlane, borrow,
    build, commands, datum_refs, design_features_of_body, profile_repair, text_feature,
};
use core_document::{
    BodyId, Document, FeatureId, HostRequest, InputResult, WorkbenchId, WorkbenchRuntimeContext,
    base_tool_id, tool_variant,
};

/// What a tool just made, for the task that opens on it.
#[derive(Debug, Clone)]
pub(crate) struct ToolMade {
    pub feature: FeatureId,
    /// The tool, with its variant (`design.primitive:box`).
    pub tool: String,
    /// The body it was made for.
    pub body: BodyId,
    /// The bodies a split made, its pieces.
    pub pieces: Vec<BodyId>,
    /// The sketch closed for it, which the task's notice puts back.
    pub repaired: Option<profile_repair::SketchRepair>,
}

impl DesignWorkbench {
    /// The sketch feature currently selected in the tree, if any: a sketch
    /// of the body, or one another body lends it.
    pub(crate) fn selected_sketch(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == wb_sketch::KIND || borrow::lends_sketch(ctx.document, id))
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
            workbench: WorkbenchId::from(wb_sketch::KIND),
            attach: core_document::SketchAttachRequest {
                body: body.0,
                face,
                face_origin,
                generator,
            },
        });
        InputResult::consumed()
    }

    pub(crate) fn target_body(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
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
    pub(crate) fn selected_design_feature(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == crate::KIND).then_some(id)
    }

    /// Whether the body's history builds anything: borrowed geometry
    /// alone does not.
    pub(crate) fn body_has_solid(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
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
            && !ctx.document.body(body).is_some_and(|b| b.link.is_some())
    }

    /// `base`, or `base_2`, `base_3`, ...: the name
    /// [`core_document::Document::unused_object_name`] gives.
    pub(crate) fn next_feature_name(ctx: &WorkbenchRuntimeContext, base: &str) -> String {
        ctx.document.unused_object_name(base)
    }

    /// The body's last sketch (its own, or one lent it) of which `count`
    /// finds any: points to stand at, curves to follow.
    pub(crate) fn last_sketch_where(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        count: fn(&core_document::Document, FeatureId) -> usize,
    ) -> Option<FeatureId> {
        build::sketches_of_body(ctx.document, body)
            .into_iter()
            .rev()
            .map(|(id, _)| id)
            .find(|id| count(ctx.document, *id) > 0)
    }

    /// The last non-modifier feature of a body (default pattern original).
    /// The latest datum plane of `body` a scan's symmetry was kept as.
    pub(crate) fn scan_symmetry_plane(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Option<MirrorPlane> {
        let mut planes: Vec<_> = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| {
                n.body == Some(body)
                    && n.workbench_id.as_str() == core_document::DATUM_KIND
                    && n.name == commands::SYMMETRY_NAME
            })
            .map(|(id, n)| (n.seq, *id))
            .collect();
        planes.sort();
        planes.last().map(|(_, datum)| MirrorPlane::Datum {
            datum: *datum,
            plane: None,
        })
    }

    /// What a new mirror copies, from the tree's selection: the features
    /// picked there (several with Ctrl), or with the body itself picked
    /// none, which mirrors the whole body; with nothing picked, the last
    /// feature that makes shape.
    pub(crate) fn mirror_originals(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<FeatureId> {
        let shapes: Vec<FeatureId> = design_features_of_body(ctx.document, body)
            .into_iter()
            .filter(|(_, f)| !f.is_modifier())
            .map(|(id, _)| id)
            .collect();
        let picked: Vec<FeatureId> = if ctx.selected_features.is_empty() {
            ctx.active_document_object.into_iter().collect()
        } else {
            ctx.selected_features.clone()
        };
        let chosen: Vec<FeatureId> = shapes
            .iter()
            .copied()
            .filter(|id| picked.contains(id))
            .collect();
        if !chosen.is_empty() {
            return chosen;
        }
        if picked.is_empty() && ctx.selected_body_id == Some(body.0) {
            return Vec::new();
        }
        Self::last_shape_feature(ctx, body).into_iter().collect()
    }

    pub(crate) fn last_shape_feature(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Option<FeatureId> {
        design_features_of_body(ctx.document, body)
            .into_iter()
            .rev()
            .find(|(_, f)| !f.is_modifier())
            .map(|(id, _)| id)
    }

    /// Every face picked in the viewport (Ctrl adds more), in `body`'s
    /// frame, in the order picked.
    pub(crate) fn selected_face_picks(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Vec<FacePick> {
        ctx.selected_faces_in(body)
            .into_iter()
            .map(FacePick::of)
            .collect()
    }

    /// The flat face picked in the viewport, in `body`'s frame, as a
    /// profile to pad or pocket; a curved face has no plane to extrude.
    pub(crate) fn selected_profile_face(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Option<FacePick> {
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
    pub(crate) fn profile_without_sketch(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        Self::selected_profile_face(ctx, body).is_some()
            || ctx.active_document_object.is_some_and(|id| {
                crate::borrow::borrow_of(ctx.document, id).is_some_and(|b| b.body == body)
                    && crate::borrow::lent_face(ctx.document, id, 0).is_some()
            })
    }

    /// The flat face picked in the viewport, as a plane to mirror across;
    /// a curved face has no plane to offer.
    pub(crate) fn selected_mirror_face(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
    ) -> Option<MirrorPlane> {
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
    pub(crate) fn selected_edges(ctx: &WorkbenchRuntimeContext, body: BodyId) -> EdgeSel {
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

    /// What a new borrow in `body` takes from the selection: the faces and
    /// edges picked on another body, else the latest sketch of another
    /// body.
    pub(crate) fn borrow_from_selection(
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
            .filter(|(_, n)| n.workbench_id.as_str() == wb_sketch::KIND && n.body != Some(body))
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
    pub(crate) fn insert_datum(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
    ) -> InputResult {
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
                self.tab.pending_task_from_tool = Some(ToolMade {
                    feature: feature_id,
                    tool: tool.to_string(),
                    body,
                    pieces: Vec::new(),
                    repaired: None,
                });
                ctx.active_document_object = Some(feature_id);
                ctx.log_info(format!("Created {name}"));
            }
            Err(e) => ctx.log_error(format!("Failed to create datum: {e}")),
        }
        InputResult::consumed()
    }

    /// Create a feature from a toolbar action and open its task.
    pub(crate) fn insert_feature(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
    ) -> InputResult {
        let Some(body) = Self::target_body(ctx) else {
            ctx.log_warn("Select a body (or one of its features) first");
            return InputResult::consumed();
        };
        match self.create_feature(ctx, tool, body, Given::default(), |_| Ok(())) {
            Ok(made) => {
                self.tab.pending_task_from_tool = Some(ToolMade {
                    feature: made.id,
                    tool: tool.to_string(),
                    body,
                    pieces: Vec::new(),
                    repaired: made.repaired,
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
        let body = if ctx.document.body_solid_is_imported(body) && take_base(ctx, body).is_some() {
            // Its imported solid becomes the start of its history.
            body
        } else if surface_body(ctx.document, body) {
            let name = ctx
                .document
                .body(body)
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
                .body(body)
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
        // Text writes in a sketch of its own, made on the face picked.
        let own_sketch = match &mut feature {
            DesignFeature::Text {
                sketch,
                wrap,
                radius,
                ..
            } => {
                let (made, curved, r) = text_feature::make_sketch(ctx, body)?;
                (*sketch, *wrap, *radius) = (made, curved, r);
                Some(made)
            }
            _ => None,
        };
        if let Err(why) = edit(&mut feature) {
            if let Some(own) = own_sketch {
                let _ = ctx.document.remove_feature(own);
            }
            return Err(why);
        }
        let repaired = profile_repair::close_profile(ctx.document, &feature);
        if let Some(repair) = &repaired {
            ctx.log_info(repair.describe(ctx.document));
        }
        let name = Self::next_feature_name(ctx, base);
        let sketches: Vec<FeatureId> = feature
            .sketches()
            .into_iter()
            .filter(|s| Some(*s) != own_sketch)
            .collect();
        let id = ctx
            .document
            .add_feature_in_body(feature, name.clone(), Some(body))
            .map_err(|e| format!("Failed to create {base}: {e}"))?;
        ctx.document.mark_feature_dirty(id);
        if own_sketch.is_some()
            && let Err(why) = text_feature::sync(ctx.document, id)
        {
            ctx.log_warn(why);
        }
        // Consumed sketches are hidden; the solid takes over visually.
        if self.options.hide_used_sketches {
            for sketch in &sketches {
                ctx.document.set_feature_visible(*sketch, false);
            }
        }
        ctx.log_info(format!("Created {name}"));
        Ok(CreatedFeature { id, repaired })
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
    pub(crate) fn say(&self, ui: &str, script: &str) -> String {
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
    let linked = ctx.document.body(body).is_some_and(|b| b.link.is_some());
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

/// A feature `create_feature` added.
pub(crate) struct CreatedFeature {
    pub id: FeatureId,
    /// The sketch closed so the feature builds.
    pub repaired: Option<profile_repair::SketchRepair>,
}
