//! Making sketches and placing them: on a base plane, a face or a datum,
//! turned or moved onto a face, and carbon copies, merges and mirrors of
//! other sketches mapped onto the edited one's plane.

use std::collections::HashSet;

use core_document::{
    BasePlane, BodyId, FeatureId, HostRequest, InputResult, WorkbenchRuntimeContext,
};
use uuid::Uuid;

use crate::boundary::{narrow3, widen3};
use crate::feature::SketchFeature;
use crate::sketch::{GeometryElement, Sketch, SketchPlane};
use crate::{NewSketchOn, PendingCreation, SketchPicker, SketchPickerMode, SketchWorkbench};
use crate::{boundary, commands, generator, tools};

impl SketchWorkbench {
    /// `base`, or `base_2`, `base_3`, ...: a generated sketch's name.
    pub(crate) fn next_generated_name(document: &core_document::Document, base: &str) -> String {
        document.unused_object_name(base)
    }

    pub(crate) fn next_sketch_name(document: &core_document::Document) -> String {
        let mut max_index = None::<u32>;
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() == crate::KIND
                && let Some(idx) = parse_sketch_index(&node.name)
            {
                max_index = Some(max_index.map_or(idx, |m| m.max(idx)));
            }
        }
        let candidate = match max_index {
            None => "sketch".to_string(),
            Some(m) => format!("sketch_{}", m.saturating_add(1)),
        };
        if document.has_object_named(&candidate) {
            document.unused_object_name("sketch")
        } else {
            candidate
        }
    }

    /// Another workbench (or the host) asked for a sketch on a specific
    /// body: take the request and open the plane picker, the face it
    /// offers in the body's own frame.
    pub(crate) fn take_attach_request(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(request) = ctx.attach_request.take() {
            let face = request
                .face
                .map(|f| f.moved(&ctx.document.body_placement(BodyId(request.body)).inverse()));
            self.begin_sketch_creation(Some(BodyId(request.body)), face, request.face_origin, None);
            if let Some(pending) = &mut self.tab.pending_creation {
                pending.generator = request.generator.and_then(generator::Generator::named);
            }
        }
    }

    /// Open the plane picker. The sketch is created once a plane is chosen
    /// in the task panel.
    pub(crate) fn begin_sketch_creation(
        &mut self,
        body: Option<BodyId>,
        face: Option<core_document::FaceRef>,
        face_origin: core_document::FaceOrigin,
        datum: Option<FeatureId>,
    ) {
        self.tab.pending_creation = Some(PendingCreation {
            body,
            face,
            face_origin,
            generator: None,
            datum,
        });
    }

    /// Close the plane picker with a sketch on the origin plane `plane`,
    /// chosen by its button or clicked in the view.
    pub(crate) fn create_on_base_plane(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        plane: BasePlane,
    ) {
        let Some(pending) = self.tab.pending_creation.take() else {
            return;
        };
        self.create_sketch_on_plane(
            ctx,
            pending.body,
            SketchPlane::of_base(plane),
            NewSketchOn {
                made_by: pending.generator,
                ..Default::default()
            },
        );
    }

    pub(crate) fn create_sketch_on_plane(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: Option<BodyId>,
        plane: SketchPlane,
        on: NewSketchOn,
    ) {
        let NewSketchOn {
            support,
            face,
            attached,
            made_by,
        } = on;
        let sketch_name = match &made_by {
            Some(made_by) => Self::next_generated_name(ctx.document, made_by.base_name()),
            None => Self::next_sketch_name(ctx.document),
        };
        let mut sketch = Sketch::new(sketch_name.clone());
        sketch.plane = plane;
        let mut sketch_feature = SketchFeature::new(sketch, plane);
        sketch_feature.support = support.clone();
        sketch_feature.face = face;
        sketch_feature.attached = attached;
        let kind = made_by.as_ref().map(generator::Generator::kind);
        if let Some(made_by) = made_by {
            sketch_feature.generator = Some(made_by);
            if let Err(why) = generator::regenerate(&mut sketch_feature) {
                ctx.log_error(format!("Failed to make {sketch_name}: {why}"));
                return;
            }
        }

        match ctx
            .document
            .add_feature_in_body(sketch_feature, sketch_name.clone(), body)
        {
            Ok(feature_id) => {
                let mut args = match &support {
                    Some(support) => {
                        let mut args = serde_json::json!({
                            "name": sketch_name,
                            "on": support.datum.0.to_string(),
                        });
                        if let Some(which) = &support.plane {
                            args["plane"] = serde_json::json!(which);
                        }
                        args
                    }
                    None => serde_json::json!({
                        "name": sketch_name,
                        "normal": plane.normal,
                        "origin": plane.origin,
                        "x_axis": plane.x_axis,
                    }),
                };
                if let Some(body) = body {
                    args["body"] = serde_json::json!(body.0.to_string());
                }
                if let Some(kind) = kind {
                    args["generator"] = serde_json::json!(kind);
                }
                if let Some(attached) = attached {
                    args["attachment"] = serde_json::json!(attached.attachment);
                    args["attachment_offset"] = serde_json::json!(attached.offset);
                }
                ctx.record(
                    "sketch.new",
                    commands::args(args),
                    serde_json::json!(feature_id.0.to_string()),
                );
                self.tab.active_sketch_id = Some(feature_id);
                self.tab.session_start = ctx.document.get_feature_data(feature_id).cloned();
                self.clear_interaction_state();
                ctx.active_document_object = Some(feature_id);
                // The view turns to the plane where the body has it, onto
                // the face the sketch was placed on when it was.
                // A generated sketch is centred on its origin, where the
                // generator stands.
                let placement = sketch_placement(ctx.document, feature_id);
                let centre = match kind {
                    Some(_) => Some(boundary::place_point(&placement, plane.origin)),
                    None => face
                        .as_ref()
                        .map(|f| boundary::place_point(&placement, f.point)),
                };
                let plane = placed_plane(&plane, &placement);
                ctx.request(HostRequest::OrientCamera(
                    core_document::CameraOrientRequest {
                        plane_origin: narrow3(plane.origin),
                        plane_normal: narrow3(plane.normal),
                        plane_up: narrow3(plane.y_axis),
                        centre: centre.map(narrow3),
                    },
                ));
                ctx.log_info(format!("Created new sketch: {sketch_name}"));
            }
            Err(e) => {
                ctx.log_error(format!("Failed to create sketch: {e}"));
            }
        }
    }

    /// Turn the view square onto the sketch's plane, as opening it does.
    pub(crate) fn view_sketch(ctx: &mut WorkbenchRuntimeContext, feature: &SketchFeature) {
        let plane = feature.plane;
        ctx.request(HostRequest::OrientCamera(
            core_document::CameraOrientRequest {
                plane_origin: narrow3(plane.origin),
                plane_normal: narrow3(plane.normal),
                plane_up: narrow3(plane.y_axis),
                centre: None,
            },
        ));
    }

    /// Turn the sketch plane over, or a quarter turn about its normal.
    pub(crate) fn reorient(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        rotate: bool,
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        let neg = |v: [f64; 3]| [-v[0], -v[1], -v[2]];
        let new_plane = if rotate {
            SketchPlane::from_frame(plane.origin, plane.normal, plane.y_axis)
        } else {
            SketchPlane::from_frame(plane.origin, neg(plane.normal), plane.x_axis)
        };
        self.set_plane(ctx, &mut feature, new_plane);
        ctx.log_info(if rotate {
            "Sketch plane rotated a quarter turn"
        } else {
            "Sketch plane flipped"
        });
        InputResult::consumed()
    }

    /// Move the sketch onto the face picked in the viewport.
    pub(crate) fn attach_to_face(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let body = self
            .tab
            .active_sketch_id
            .and_then(|id| ctx.document.get_feature_meta(id))
            .and_then(|n| n.body);
        // Only a face of the sketch's own body is followed.
        let own = body.filter(|b| ctx.selected_body_id == Some(b.0));
        let face = match body {
            Some(body) => ctx.selected_face_in(body),
            None => ctx.selected_face,
        };
        let Some(face) = face.map(|f| f.on_its_plane()) else {
            ctx.log_warn("Click a face of a solid first");
            return InputResult::consumed();
        };
        let plane = SketchPlane::from_face(widen3(face.point), widen3(face.normal));
        // On a face of its own body the sketch follows the face; the face
        // replaces any datum it stood on.
        feature.support = None;
        feature.face = own.map(|_| crate::feature::FaceSupport::on(&face, plane));
        self.set_plane(ctx, &mut feature, plane);
        ctx.log_info("Sketch attached to the picked face");
        InputResult::consumed()
    }

    fn set_plane(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &mut SketchFeature,
        plane: SketchPlane,
    ) {
        feature.plane = plane;
        feature.sketch.plane = plane;
        // Placed outright: no offset from what it was on before.
        feature.offset = Default::default();
        // A sketch on a face keeps following it from where the face is now,
        // on the plane just set.
        if let Some(face) = feature.face.as_mut() {
            let now = self
                .tab
                .active_sketch_id
                .and_then(|id| ctx.document.probed_references(id))
                .and_then(|probed| probed.answers.first())
                .and_then(|answer| match answer {
                    Ok(kernel_api::ProbeAnswer::Face { point, normal, .. }) => {
                        Some((point.map(|c| c), normal.map(|c| c)))
                    }
                    _ => None,
                });
            if let Some((point, normal)) = now {
                face.point = point;
                face.normal = normal;
            }
            face.placed = plane;
        }
        self.store_sketch(ctx, feature.clone());
        if let Some(id) = self.tab.active_sketch_id
            && let Some(stored) = stored_sketch(ctx.document, id)
        {
            ctx.record(
                "sketch.set_plane",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "normal": stored.plane.normal,
                    "origin": stored.plane.origin,
                    "x_axis": stored.plane.x_axis,
                })),
                serde_json::Value::Null,
            );
        }
        ctx.request(HostRequest::OrientCamera(
            core_document::CameraOrientRequest {
                plane_origin: narrow3(plane.origin),
                plane_normal: narrow3(plane.normal),
                plane_up: narrow3(plane.y_axis),
                centre: None,
            },
        ));
    }

    /// Open the panel's list of other sketches for carbon copy or merge; the
    /// same action again closes it.
    pub(crate) fn open_sketch_picker(&mut self, mode: SketchPickerMode) -> InputResult {
        self.tab.sketch_picker = match &self.tab.sketch_picker {
            Some(picker) if picker.mode == mode => None,
            _ => Some(SketchPicker {
                mode,
                checked: HashSet::new(),
            }),
        };
        InputResult::consumed()
    }

    /// Every sketch in the document but the one being edited, in history
    /// order: its id, name, and content.
    pub(crate) fn other_sketches(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Vec<(FeatureId, String, SketchFeature)> {
        let mut out: Vec<(u64, FeatureId, String, SketchFeature)> = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(id, node)| {
                node.workbench_id.as_str() == crate::KIND && Some(**id) != self.tab.active_sketch_id
            })
            .filter_map(|(id, node)| {
                // Seen where its body sits, as the edited sketch is.
                let mut feature = ctx.document.feature_as::<SketchFeature>(*id)?;
                let placement = sketch_placement(ctx.document, *id);
                feature.plane = placed_plane(&feature.plane, &placement);
                Some((node.seq, *id, node.name.clone(), feature))
            })
            .collect();
        out.sort_by_key(|(seq, id, _, _)| (*seq, *id));
        out.into_iter()
            .map(|(_, id, name, feature)| (id, name, feature))
            .collect()
    }

    /// Copy `source`'s geometry into the edited sketch, mapped from its plane
    /// onto this one; its constraints come along where the planes share
    /// their axes.
    pub(crate) fn carbon_copy(&mut self, ctx: &mut WorkbenchRuntimeContext, source: FeatureId) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let (Some(target), Some(name)) = (
            self.tab.active_sketch_id,
            ctx.document
                .get_feature_meta(source)
                .map(|n| n.name.clone()),
        ) else {
            return;
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let (count, constraints) =
            match commands::carbon_copy(ctx.document, target, &mut feature.sketch, source) {
                Ok(done) => done,
                Err(why) => {
                    ctx.log_warn(format!("Carbon copy of {name}: {why}"));
                    return;
                }
            };
        ctx.record(
            "sketch.carbon_copy",
            commands::args(serde_json::json!({
                "sketch": target.0.to_string(),
                "from": source.0.to_string(),
            })),
            commands::made_since(&feature.sketch, &before),
        );
        self.tab.sketch_picker = None;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        ctx.log_info(carbon_copy_log(&name, count, constraints));
    }

    /// A new sketch on the edited one's plane holding its geometry and the
    /// ticked sketches', each mapped onto the plane; the originals stay.
    pub(crate) fn merge_sketches(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(picker) = self.tab.sketch_picker.take() else {
            return;
        };
        let Some(active) = self.tab.active_sketch_id else {
            return;
        };
        // In history order, as the list shows them.
        let with: Vec<FeatureId> = self
            .other_sketches(ctx)
            .into_iter()
            .map(|(id, _, _)| id)
            .filter(|id| picker.checked.contains(id))
            .collect();
        match commands::merge(ctx.document, active, &with) {
            Ok(made) => {
                let name = ctx
                    .document
                    .get_feature_meta(made)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                ctx.log_info(format!("Created {name}"));
                ctx.record(
                    "sketch.merge",
                    commands::args(serde_json::json!({
                        "sketch": active.0.to_string(),
                        "with": with.iter().map(|id| id.0.to_string()).collect::<Vec<_>>(),
                    })),
                    serde_json::json!(made.0.to_string()),
                );
            }
            Err(err) => ctx.log_warn(format!("Could not merge: {err}")),
        }
    }

    /// A new sketch on the same plane and body: this one's geometry
    /// mirrored across the sketch's Y axis.
    pub(crate) fn mirror_sketch(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        match commands::mirror_sketch(ctx.document, id) {
            Ok(made) => {
                let name = ctx
                    .document
                    .get_feature_meta(made)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                ctx.log_info(format!("Created {name}"));
                ctx.record(
                    "sketch.mirror_sketch",
                    commands::args(serde_json::json!({"sketch": id.0.to_string()})),
                    serde_json::json!(made.0.to_string()),
                );
            }
            Err(err) => ctx.log_error(format!("Could not create the mirrored sketch: {err}")),
        }
        InputResult::consumed()
    }
}

fn parse_sketch_index(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("sketch_") {
        r
    } else {
        lower.strip_prefix("sketch")?
    };

    let trimmed = rest.trim_start_matches(&['_', '.', ' '][..]);
    if trimmed.is_empty() {
        Some(0)
    } else {
        trimmed.parse().ok()
    }
}

/// A sketch as the document has it, its plane in its body's frame: its
/// formulas' values in, and solved for them.
pub(crate) fn stored_sketch(
    document: &core_document::Document,
    id: FeatureId,
) -> Option<SketchFeature> {
    document.feature_as::<SketchFeature>(id)
}

/// Where the body a sketch belongs to sits.
pub(crate) fn sketch_placement(
    document: &core_document::Document,
    id: FeatureId,
) -> core_document::BodyPlacement {
    document
        .get_feature_meta(id)
        .and_then(|node| node.body)
        .map(|body| document.body_placement(body))
        .unwrap_or_default()
}

/// A plane moved by a body's placement.
pub(crate) fn placed_plane(
    plane: &SketchPlane,
    placement: &core_document::BodyPlacement,
) -> SketchPlane {
    SketchPlane {
        origin: boundary::place_point(placement, plane.origin),
        normal: boundary::place_direction(placement, plane.normal),
        x_axis: boundary::place_direction(placement, plane.x_axis),
        y_axis: boundary::place_direction(placement, plane.y_axis),
    }
}

/// A plane seen where the body sits, back in the body's frame: `stored`
/// when it is where `stored` is seen, else moved back.
pub(crate) fn local_plane(
    seen: &SketchPlane,
    stored: &SketchPlane,
    placement: &core_document::BodyPlacement,
) -> SketchPlane {
    let same = |a: [f64; 3], b: [f64; 3], tol: f64| (0..3).all(|k| (a[k] - b[k]).abs() <= tol);
    let placed = placed_plane(stored, placement);
    if same(placed.origin, seen.origin, 1e-4)
        && same(placed.normal, seen.normal, 1e-6)
        && same(placed.x_axis, seen.x_axis, 1e-6)
    {
        *stored
    } else {
        placed_plane(seen, &placement.inverse())
    }
}

/// The map from one sketch plane's coordinates onto another's, for planes
/// that face the same line: rotation, translation, and a mirror when their
/// normals oppose. A plane at an angle to the other has no such map (its
/// geometry would come out foreshortened), and is refused.
pub(crate) fn plane_map(
    from: &SketchPlane,
    onto: &SketchPlane,
) -> Result<tools::Similarity, &'static str> {
    use glam::DVec3;
    let v = |a: [f64; 3]| DVec3::from_array(a);
    if v(from.normal).cross(v(onto.normal)).length() > 1e-4 {
        return Err("its plane is not parallel to this one");
    }
    let (ox, oy) = (v(onto.x_axis), v(onto.y_axis));
    let m = glam::DMat2::from_cols(
        glam::DVec2::new(v(from.x_axis).dot(ox), v(from.x_axis).dot(oy)),
        glam::DVec2::new(v(from.y_axis).dot(ox), v(from.y_axis).dot(oy)),
    );
    let d = v(from.origin) - v(onto.origin);
    Ok(tools::Similarity::linear(
        m,
        glam::DVec2::new(d.dot(ox), d.dot(oy)),
    ))
}

/// Copy all of `source` into `target` under `xf`, with its constraints:
/// those stated against the sketch axes follow a turn or a mirror as the
/// transform tools' copies do (swapped on a quarter turn, made relative to
/// one another at any other angle). Geometry projected from a solid,
/// another sketch or a datum stays projected from it, at the place `xf`
/// carries it to until the sketch is edited and it is projected again.
/// Returns the elements and the constraints copied.
pub(crate) fn copy_sketch_into(
    source: &Sketch,
    target: &mut Sketch,
    xf: &tools::Similarity,
) -> (usize, usize) {
    let all: HashSet<Uuid> = source.geometry.iter().map(GeometryElement::id).collect();
    let before = target.constraints.len();
    let map = tools::copy_mapped(source, target, &all, xf);
    for (old, new) in &map {
        if let Some(external) = source.external.get(old) {
            target.external.insert(*new, *external);
        }
    }
    tools::copy_constraints(source, target, &map, xf);
    let copied: HashSet<Uuid> = map.values().copied().collect();
    tools::adapt_constraints(target, &copied, xf);
    (map.len(), target.constraints.len().saturating_sub(before))
}

pub(crate) fn carbon_copy_log(name: &str, count: usize, constraints: usize) -> String {
    format!("Carbon copy of {name}: {count} elements, {constraints} constraints")
}

#[cfg(test)]
mod sketch_picker {
    use super::*;
    use crate::sketch::ConstraintKind;
    use crate::test_kinds::{all_points, line_of};
    use crate::*;
    use core_document::Document;
    use sketch::{Line, Point};

    /// A document with the edited sketch on `onto` and one other sketch on
    /// `from`: a 10 mm line along its X axis, horizontal and dimensioned.
    fn scene(from: SketchPlane, onto: SketchPlane) -> (Document, SketchWorkbench, FeatureId) {
        let mut doc = Document::new("t");
        let mut other = Sketch::new("other");
        let a = other.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = other.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = other.add_geometry(GeometryElement::Line(Line::new(a, b)));
        other.add_constraint(ConstraintKind::Horizontal { element: line });
        other.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        let other_id = doc
            .add_feature_in_body(SketchFeature::new(other, from), "other".into(), None)
            .unwrap();
        let edited = add_sketch(&mut doc, Sketch::new("edited"), onto, "edited", None);
        let wb = SketchWorkbench {
            tab: crate::Tab {
                active_sketch_id: Some(edited),
                ..Default::default()
            },
            ..Default::default()
        };
        (doc, wb, other_id)
    }

    fn ctx(doc: &mut Document) -> WorkbenchRuntimeContext<'_> {
        WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600))
    }

    fn points(sketch: &Sketch) -> Vec<Vec2D> {
        all_points(sketch).into_iter().map(|p| p.position).collect()
    }

    fn shifted(dx: f64, dy: f64, dz: f64) -> SketchPlane {
        SketchPlane {
            origin: [dx, dy, dz],
            ..SketchPlane::xy()
        }
    }

    /// A carbon copy from a parallel plane keeps the line where it stands
    /// and its constraints, turned with it: from a shifted plane in place,
    /// from a facing one mirrored (still level), from a quarter-turned one
    /// upright.
    #[test]
    fn a_carbon_copy_from_a_parallel_plane_lands_where_the_line_stands() {
        let facing = SketchPlane {
            normal: [0.0, 0.0, -1.0],
            x_axis: [-1.0, 0.0, 0.0],
            ..SketchPlane::xy()
        };
        let turned = SketchPlane {
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [-1.0, 0.0, 0.0],
            ..SketchPlane::xy()
        };
        // The plane, where the line's ends land, and what holds it there.
        for (what, from, ends, held) in [
            (
                "shifted",
                shifted(5.0, 0.0, 20.0),
                [(5.0, 0.0), (15.0, 0.0)],
                "Horizontal",
            ),
            ("facing", facing, [(0.0, 0.0), (-10.0, 0.0)], "Horizontal"),
            ("turned", turned, [(0.0, 0.0), (0.0, 10.0)], "Vertical"),
        ] {
            let (mut doc, mut wb, other) = scene(from, SketchPlane::xy());
            let mut ctx = ctx(&mut doc);
            wb.carbon_copy(&mut ctx, other);
            let mut copy = wb.get_active_sketch(&ctx).unwrap().sketch;
            assert_eq!(copy.geometry.len(), 3, "{what}");
            for (x, y) in ends {
                let lands = |p: &Vec2D| (p.x - x).abs() < 1e-4 && (p.y - y).abs() < 1e-4;
                assert!(points(&copy).iter().any(lands), "{what}: ({x}, {y})");
            }
            let kinds: Vec<String> = copy
                .constraints
                .iter()
                .map(|c| format!("{:?}", c.kind))
                .collect();
            assert_eq!(kinds.len(), 2, "{what}");
            assert!(
                kinds.iter().any(|k| k.starts_with(held)),
                "{what}: {kinds:?}"
            );
            assert!(
                copy.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Length { length, .. } if (length - 10.0).abs() < 1e-4)),
                "{what}: the length comes along"
            );
            let solved = crate::solver::solve(&mut copy);
            assert!(
                matches!(solved, crate::operations::SolveOutcome::Converged { .. }),
                "{what}"
            );
        }
    }

    /// Geometry the other sketch projected from elsewhere is projected in
    /// the copy too, from the same source.
    #[test]
    fn a_carbon_copy_keeps_projected_geometry_projected() {
        let (mut doc, mut wb, other) = scene(shifted(0.0, 0.0, 5.0), SketchPlane::xy());
        let source = sketch::ExternalSource::of_reference(sketch::ExternalReference::Datum {
            datum: Uuid::new_v4(),
        });
        let mut data = SketchFeature::from_json(doc.get_feature_data(other).unwrap()).unwrap();
        let line = line_of(&data.sketch).id;
        data.sketch.external.insert(line, source);
        doc.update_feature_data(other, data.to_json()).unwrap();
        let mut ctx = ctx(&mut doc);
        wb.carbon_copy(&mut ctx, other);
        let copy = wb.get_active_sketch(&ctx).unwrap().sketch;
        let copied = line_of(&copy).id;
        assert_eq!(copy.external.get(&copied), Some(&source));
    }

    #[test]
    fn a_plane_at_an_angle_is_refused() {
        assert!(plane_map(&SketchPlane::xz(), &SketchPlane::xy()).is_err());
        let (mut doc, mut wb, other) = scene(SketchPlane::xz(), SketchPlane::xy());
        let mut ctx = ctx(&mut doc);
        wb.carbon_copy(&mut ctx, other);
        assert!(
            wb.get_active_sketch(&ctx)
                .unwrap()
                .sketch
                .geometry
                .is_empty()
        );
    }

    #[test]
    fn merging_makes_a_new_sketch_and_keeps_the_originals() {
        let (mut doc, mut wb, other) = scene(shifted(0.0, 3.0, 0.0), SketchPlane::xy());
        let before = doc.feature_tree().all_nodes().count();
        let mut ctx = ctx(&mut doc);
        wb.open_sketch_picker(SketchPickerMode::Merge);
        wb.tab.sketch_picker.as_mut().unwrap().checked.insert(other);
        wb.merge_sketches(&mut ctx);
        assert!(wb.tab.sketch_picker.is_none());
        let merged: Vec<SketchFeature> = doc
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.name == "edited merged")
            .map(|(_, n)| SketchFeature::from_json(&n.data).unwrap())
            .collect();
        assert_eq!(doc.feature_tree().all_nodes().count(), before + 1);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sketch.geometry.len(), 3);
        assert_eq!(merged[0].sketch.constraints.len(), 2);
        assert!(
            points(&merged[0].sketch)
                .iter()
                .all(|p| (p.y - 3.0).abs() < 1e-4)
        );
    }
}

#[cfg(test)]
mod placed_body {
    use super::*;
    use crate::*;
    use core_document::{BodyPlacement, Document};

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4)
    }

    #[test]
    fn a_sketch_on_a_moved_body_is_edited_where_it_sits_and_stored_in_its_frame() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch = add_sketch(
            &mut doc,
            Sketch::new("s"),
            SketchPlane::xy(),
            "s",
            Some(body),
        );
        let placement = BodyPlacement::new(
            glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            glam::Vec3::new(0.0, 0.0, 20.0),
        );
        doc.set_body_placement(body, placement);
        let wb = SketchWorkbench {
            tab: crate::Tab {
                active_sketch_id: Some(sketch),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));

        let seen = wb.get_active_sketch(&ctx).unwrap();
        assert!(close(seen.plane.origin, [0.0, 0.0, 20.0]));
        assert!(
            close(seen.plane.normal, [0.0, -1.0, 0.0]),
            "{:?}",
            seen.plane.normal
        );

        // An edit that does not move the plane leaves it exactly as stored.
        assert!(wb.store_sketch(&mut ctx, seen.clone()));
        let kept = stored_sketch(ctx.document, sketch).unwrap().plane;
        let xy = SketchPlane::xy();
        assert_eq!(
            (kept.origin, kept.normal, kept.x_axis, kept.y_axis),
            (xy.origin, xy.normal, xy.x_axis, xy.y_axis),
            "bit for bit"
        );

        // A plane set where the body sits is kept in the body's frame.
        let mut moved = seen;
        moved.plane.origin = [0.0, 0.0, 25.0];
        assert!(wb.store_sketch(&mut ctx, moved));
        let stored = stored_sketch(ctx.document, sketch).unwrap().plane;
        assert!(close(stored.origin, [0.0, 5.0, 0.0]), "{:?}", stored.origin);
        assert!(close(stored.normal, [0.0, 0.0, 1.0]));
    }
}

#[cfg(test)]
mod face_view {
    use super::*;
    use crate::*;

    /// A sketch made on a slanted face turns the view square to that face,
    /// centred on the point clicked on it.
    #[test]
    fn a_sketch_on_a_face_turns_the_view_to_the_face() {
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let normal = {
            let n = glam::Vec3::new(1.0, 0.0, 1.0).normalize();
            [n.x, n.y, n.z]
        };
        let clicked = [10.0, 2.0, 5.0];
        let plane = SketchPlane::from_face(widen3(clicked), widen3(normal));
        let face = core_document::FaceRef {
            point: clicked,
            normal,
            surface: None,
            name: 0,
        };
        let support = crate::feature::FaceSupport::on(&face, plane);
        let mut wb = SketchWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.create_sketch_on_plane(
            &mut ctx,
            Some(BodyId(body.0)),
            plane,
            NewSketchOn {
                face: Some(support),
                ..Default::default()
            },
        );
        let orient = ctx
            .take_requests()
            .into_iter()
            .find_map(|r| match r {
                HostRequest::OrientCamera(o) => Some(o),
                _ => None,
            })
            .expect("the view is turned");
        assert_eq!(orient.plane_normal, narrow3(plane.normal));
        assert_eq!(orient.centre, Some(clicked), "on the face clicked");
    }

    /// A generator's sketch on a turned body: it is made by the generator,
    /// recorded as `sketch.new` naming it, and the view turns to the plane
    /// where the body has it, centred on where the generator stands.
    #[test]
    fn a_generated_sketch_turns_the_view_to_where_its_body_has_it() {
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let turn = glam::DQuat::from_rotation_x(30f64.to_radians());
        let mut placement = doc.body_placement(body);
        placement.rotation = turn.as_quat().to_array();
        placement.translation = [5.0, 0.0, 0.0];
        doc.set_body_placement(body, placement);
        let plane = SketchPlane::from_frame([1.0, 2.0, 10.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let mut wb = SketchWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.create_sketch_on_plane(
            &mut ctx,
            Some(body),
            plane,
            NewSketchOn {
                made_by: generator::Generator::named("sprocket"),
                ..Default::default()
            },
        );
        let outcome = core_document::HookOutcome::take(&mut ctx);
        let orient = outcome
            .requests
            .into_iter()
            .find_map(|r| match r {
                HostRequest::OrientCamera(o) => Some(o),
                _ => None,
            })
            .expect("the view is turned");
        let recorded = outcome.recorded;
        assert_eq!(recorded[0].id, "sketch.new");
        assert_eq!(recorded[0].args["generator"], "sprocket");
        let normal = turn * glam::DVec3::Z;
        assert!(glam::DVec3::from_array(widen3(orient.plane_normal)).distance(normal) < 1e-5);
        let centre = turn * glam::DVec3::new(1.0, 2.0, 10.0) + glam::DVec3::new(5.0, 0.0, 0.0);
        let seen = glam::DVec3::from_array(widen3(orient.centre.expect("centred")));
        assert!(seen.distance(centre) < 1e-4, "{seen} is not {centre}");
        let id = wb.tab.active_sketch_id.expect("the sketch is open");
        let made = stored_sketch(&doc, id).unwrap();
        assert!(made.generator.is_some(), "made by the sprocket generator");
        assert!(!made.sketch.geometry.is_empty(), "with its outline");
        assert_eq!(doc.get_feature_meta(id).unwrap().name, "Sprocket");
    }
}
