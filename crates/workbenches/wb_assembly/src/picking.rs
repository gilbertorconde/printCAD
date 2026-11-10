//! Picking a joint's faces: the picks taken from the view or the tree,
//! the joint or coupling they make, and picking a joint's faces again.

use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchRuntimeContext};

use crate::coupling::{Coupling, Gearing};
use crate::joint::{Anchor, JOINT_KIND, JointFeature, JointKind, JointTool, Takes, WORLD};
use crate::solve::{Joint, joints};
use crate::{AssemblyWorkbench, Picking, Task, all_placements};
use crate::{commands, shapes};

impl AssemblyWorkbench {
    /// Take a new face pick for the joint being made: a face or an edge
    /// picked in the view, or a datum plane or line selected in the tree.
    pub(crate) fn take_pick(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(picking) = self.picking.clone() else {
            return;
        };
        if let Some((body, anchor, signature)) = Self::datum_pick(ctx) {
            if self.seen == Some(signature) {
                return;
            }
            self.seen = Some(signature);
            if !picking
                .kind
                .takes_anchor(&anchor, picking.first.map(|f| f.1))
            {
                ctx.log_warn(picking.kind.refusal());
                return;
            }
            self.picked(ctx, body, anchor, None, 0);
            return;
        }
        // A face, or else the last edge picked, where the joint takes one.
        let edge = ctx.selected_edges.last().copied();
        let (body, point, face, edge) = match (ctx.selected_body_id, ctx.selected_face, edge) {
            (Some(body), Some(face), _) => (body, face.point, Some(face), None),
            (_, None, Some(edge)) => (edge.body, edge.point, None, Some(edge)),
            _ => return,
        };
        let signature = (body, point.map(f32::to_bits));
        if self.seen == Some(signature) {
            return;
        }
        self.seen = Some(signature);
        let body = BodyId(body);
        let to_local = ctx.document.body_placement(body).inverse();
        let face = face.map(|f| f.moved(&to_local));
        let edge = edge.map(|e| e.moved(&to_local));
        let flat = || face.as_ref().and_then(Anchor::plane_of);
        let round = || {
            face.as_ref()
                .and_then(|f| Some((Anchor::axis_of(f)?, Anchor::radius_of(f))))
                .or_else(|| edge.map(|e| (Anchor::axis_of_edge(&e), e.circle.map(|c| c.radius))))
        };
        let first_flat = picking
            .first
            .map(|(_, anchor, ..)| matches!(anchor, Anchor::Plane { .. }));
        let pointed = || {
            face.as_ref()
                .map(|f| (Anchor::point_of(f), Anchor::radius_of(f)))
                .or_else(|| edge.map(|e| (Anchor::point_of_edge(&e), e.circle.map(|c| c.radius))))
        };
        let sphere = face
            .as_ref()
            .is_some_and(|f| matches!(f.surface, Some(kernel_api::FaceSurface::Sphere { .. })));
        let name = face.as_ref().map_or(0, |f| f.name);
        let picked = match (picking.kind.takes(), first_flat) {
            (Takes::Flat, _) => flat().map(|a| (a, None)),
            (Takes::Round, _) => round(),
            (Takes::Any, _) => flat().map(|a| (a, None)).or_else(round),
            (Takes::FlatAndRound, None) => flat().map(|a| (a, None)).or_else(round),
            (Takes::FlatAndRound, Some(true)) => round(),
            (Takes::FlatAndRound, Some(false)) => flat().map(|a| (a, None)),
            (Takes::Point, _) => pointed(),
            (Takes::Directed, _) => flat().map(|a| (a, None)).or_else(round),
            // A ball is its centre; any other face its plane or axis.
            (Takes::Anything, _) if sphere => pointed(),
            (Takes::Anything, _) => flat().map(|a| (a, None)).or_else(round).or_else(pointed),
            (Takes::PointAndLine, None) => pointed(),
            (Takes::PointAndLine, Some(_)) => round(),
            (Takes::PointAndEdge, None) => pointed(),
            // Where on the edge it was picked: the edge itself is kept.
            (Takes::PointAndEdge, Some(_)) => edge
                .map(|e| (Anchor::Point { point: e.point }, None))
                .or_else(|| {
                    face.as_ref()
                        .map(|f| (Anchor::Point { point: f.point }, None))
                }),
            // A roller is the point of its axis beside the pick, and its
            // radius.
            (Takes::PointAndFace, None) => match face.as_ref().and_then(|f| {
                let (p, d) = f.surface?.axis()?;
                Some((f, p, d, Anchor::radius_of(f)?))
            }) {
                Some((f, p, d, radius)) => {
                    let (p, d, at) = (
                        glam::Vec3::from_array(p),
                        glam::Vec3::from_array(d).normalize_or_zero(),
                        glam::Vec3::from_array(f.point),
                    );
                    let centre = p + d * (at - p).dot(d);
                    Some((
                        Anchor::Point {
                            point: centre.to_array(),
                        },
                        Some(radius),
                    ))
                }
                None => pointed(),
            },
            (Takes::PointAndFace, Some(_)) => face
                .as_ref()
                .map(|f| (Anchor::Point { point: f.point }, None)),
        };
        let Some((anchor, radius)) = picked else {
            ctx.log_warn(picking.kind.refusal());
            return;
        };
        self.picked(ctx, body, anchor, radius, name);
    }

    /// A datum plane or line selected in the tree, as an anchor on its
    /// body (or the world's, for one of no body), with the signature a
    /// repeat of it is known by.
    fn datum_pick(
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(BodyId, Anchor, (uuid::Uuid, [u32; 3]))> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        if node.workbench_id.as_str() != "core.datum" {
            return None;
        }
        let values = ctx.document.feature_values(id).unwrap_or(&node.data);
        let datum = core_document::DatumFeature::from_json(values).ok()?;
        let frame = datum.frame();
        let anchor = match datum.shape {
            core_document::DatumShape::Plane { .. } => Anchor::Plane {
                point: frame.origin,
                normal: frame.normal,
            },
            core_document::DatumShape::Line { .. } => Anchor::Axis {
                point: frame.origin,
                direction: frame.x_axis,
            },
            _ => return None,
        };
        Some((node.body.unwrap_or(WORLD), anchor, (id.0, [0; 3])))
    }

    /// An anchor picked on `body` (the world for the origin's planes and
    /// axes): the first end, or the second, which makes the joint.
    pub(crate) fn picked(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: BodyId,
        anchor: Anchor,
        radius: Option<f32>,
        name: kernel_api::TopoName,
    ) {
        let Some(picking) = self.picking.clone() else {
            return;
        };
        if picking.kind.picks_each() == 2
            && let Some((first_body, first_anchor, _, first_name)) = picking.first
        {
            match (picking.tab, picking.slot) {
                (None, _) if body != first_body => {
                    ctx.log_warn("Click the tab's other face, on the same body");
                }
                (None, _) => {
                    self.picking = Some(Picking {
                        tab: Some(anchor),
                        ..picking
                    });
                }
                (Some(_), None) if body == first_body || body == WORLD => {
                    ctx.log_warn("Click a wall of the slot, on another body");
                }
                (Some(_), None) => {
                    self.picking = Some(Picking {
                        slot: Some((body, anchor)),
                        ..picking
                    });
                }
                (Some(_), Some((slot_body, _))) if body != slot_body => {
                    ctx.log_warn("Click the slot's other wall, on the same body");
                }
                (Some(tab), Some((slot_body, wall))) => {
                    self.picking = None;
                    let kind = picking.kind.joint(
                        &first_anchor,
                        &ctx.document.body_placement(first_body).into(),
                        &wall,
                        &ctx.document.body_placement(slot_body).into(),
                        0.0,
                    );
                    self.make_joint(
                        ctx,
                        first_body,
                        JointFeature {
                            names: [first_name, 0],
                            kind,
                            moving: first_anchor,
                            other_body: slot_body,
                            fixed: wall,
                            ends: [0.0; 2],
                            shape: Vec::new(),
                            second: Some([tab, anchor]),
                        },
                    );
                }
            }
            return;
        }
        match picking.first {
            None if body == WORLD => {
                ctx.log_warn("Pick the body to move first; the origin stays where it is");
            }
            None => {
                self.picking = Some(Picking {
                    first: Some((body, anchor, radius, name)),
                    ..picking
                });
            }
            Some((first_body, ..)) if first_body == body => {
                ctx.log_warn("Pick the second face on another body");
            }
            Some((first_body, first_anchor, first_radius, first_name)) => {
                self.picking = None;
                let radius = first_radius.or(radius);
                if picking.kind == JointTool::Tangent && radius.is_none() {
                    ctx.log_warn("A tangent takes a round face with a radius: a cylinder");
                    return;
                }
                if let Some(joint) = picking.repick {
                    self.rejoin(
                        ctx,
                        joint,
                        picking.kind,
                        (first_body, first_anchor),
                        (body, anchor),
                        (radius, [first_name, name]),
                    );
                    return;
                }
                let kind = picking.kind.joint(
                    &first_anchor,
                    &ctx.document.body_placement(first_body).into(),
                    &anchor,
                    &ctx.document.body_placement(body).into(),
                    radius.unwrap_or_default(),
                );
                self.make_joint(
                    ctx,
                    first_body,
                    JointFeature {
                        second: None,
                        shape: Vec::new(),
                        ends: [0.0; 2],
                        names: [first_name, name],
                        kind,
                        moving: first_anchor,
                        other_body: body,
                        fixed: anchor,
                    },
                );
            }
        }
    }

    /// Give `joint` the faces just picked, as `asm.set` would, and open its
    /// settings again.
    fn rejoin(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        joint: FeatureId,
        tool: JointTool,
        moving: (BodyId, Anchor),
        fixed: (BodyId, Anchor),
        (radius, names): (Option<f32>, [kernel_api::TopoName; 2]),
    ) {
        let kept_radius = ctx
            .document
            .get_feature_data(joint)
            .and_then(|d| JointFeature::from_json(d).ok())
            .and_then(|j| match j.kind {
                JointKind::Tangent { radius } => Some(radius),
                _ => None,
            });
        let radius = radius.or(kept_radius).unwrap_or(0.0);
        match commands::rejoined(ctx, tool, moving, fixed, radius) {
            Ok(mut feature) => {
                feature.names = names;
                let face = |anchor: Anchor, body: BodyId, name: kernel_api::TopoName| {
                    let mut face = match anchor.moved(&ctx.document.body_placement(body)) {
                        Anchor::Plane { point, normal } => {
                            serde_json::json!({"point": point, "normal": normal})
                        }
                        Anchor::Axis { point, direction } => serde_json::json!({
                            "axis": {"point": point, "direction": direction},
                            "radius": radius,
                        }),
                        Anchor::Point { point } => serde_json::json!({"point": point}),
                    };
                    if name != 0 {
                        face["name"] = serde_json::json!(name);
                    }
                    face
                };
                let args = commands::object(serde_json::json!({
                    "joint": joint.0.to_string(),
                    "kind": tool.word(),
                    "face": face(moving.1, moving.0, names[0]),
                    "other": fixed.0.0.to_string(),
                    "other_face": face(fixed.1, fixed.0, names[1]),
                }));
                if ctx
                    .document
                    .update_feature_data(joint, feature.to_json())
                    .is_ok()
                {
                    ctx.document.clear_feature_dirty(joint);
                    self.solve_and_apply(ctx);
                    ctx.record("asm.set", args, serde_json::Value::Null);
                    ctx.log_info("Picked the joint's faces again");
                }
            }
            Err(why) => ctx.log_warn(why.to_string()),
        }
        ctx.active_document_object = Some(joint);
        self.task = Some(Task::Joint {
            id: joint,
            before: ctx.document.get_feature_data(joint).cloned(),
            placements: all_placements(ctx),
        });
    }

    /// Start picking `joint`'s faces again, for the kind `tool`.
    pub(crate) fn start_repick(&mut self, joint: FeatureId, tool: JointTool) {
        self.task = None;
        self.seen = None;
        self.picking = Some(Picking::new(tool, Some(joint)));
    }

    /// Add a joint, solve, and open its settings.
    pub(crate) fn make_joint(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: BodyId,
        mut joint: JointFeature,
    ) {
        shapes::take_shape(ctx.document, &mut joint);
        let placements = all_placements(ctx);
        let name = commands::next_name(ctx.document, joint.kind.label());
        commands::ground_first(ctx, joint.other_body);
        match ctx
            .document
            .add_feature_in_body(joint, name.clone(), Some(body))
        {
            Ok(id) => {
                // A joint has no solid to rebuild.
                ctx.document.clear_feature_dirty(id);
                ctx.active_document_object = Some(id);
                self.task = Some(Task::Joint {
                    id,
                    before: None,
                    placements,
                });
                self.solve_and_apply(ctx);
                ctx.log_info(format!("Added {name}"));
            }
            Err(err) => ctx.log_error(format!("Could not add the joint: {err}")),
        }
    }

    /// The joint the tree has selected, if a joint is selected.
    pub(crate) fn selected_joint(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        Self::selected_of(ctx, JOINT_KIND)
    }

    /// The feature of `kind` the tree has selected.
    pub(crate) fn selected_of(ctx: &WorkbenchRuntimeContext, kind: &str) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == kind).then_some(id)
    }

    /// Couple two joints: the selected one leading, when it is a hinge or
    /// a slider, else the first pair that can be tied; then open its
    /// settings.
    pub(crate) fn make_coupling(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let all = joints(ctx.document);
        let movable: Vec<&Joint> = all
            .iter()
            .filter(|j| {
                matches!(
                    j.feature.kind,
                    JointKind::Hinge { .. } | JointKind::Slider { .. }
                )
            })
            .collect();
        let selected = Self::selected_joint(ctx);
        let mut pairs: Vec<(&Joint, &Joint)> = movable
            .iter()
            .flat_map(|a| movable.iter().map(move |b| (*a, *b)))
            .filter(|(a, b)| {
                a.id != b.id && Gearing::suiting(&a.feature.kind, &b.feature.kind).is_some()
            })
            .collect();
        pairs.sort_by_key(|(a, _)| Some(a.id) != selected);
        let Some((driver, driven)) = pairs.first().copied() else {
            ctx.log_warn("Couple joints needs two hinges, or a hinge and a slider");
            return;
        };
        let placements = crate::solve::rigid_placements(ctx.document);
        let gearing =
            Gearing::suiting(&driver.feature.kind, &driven.feature.kind).unwrap_or(Gearing::Gears);
        let Some(coupling) = Coupling::new(
            gearing,
            driver,
            driven,
            gearing.default_ratio(),
            false,
            &placements,
        ) else {
            ctx.log_warn("These joints cannot be coupled where they stand");
            return;
        };
        let body = driven.body;
        let before = all_placements(ctx);
        let name = commands::next_name(ctx.document, gearing.label());
        match ctx
            .document
            .add_feature_in_body(coupling, name.clone(), Some(body))
        {
            Ok(id) => {
                ctx.document.clear_feature_dirty(id);
                ctx.active_document_object = Some(id);
                self.picking = None;
                self.task = Some(Task::Coupling {
                    id,
                    before: None,
                    placements: before,
                });
                self.solve_and_apply(ctx);
                ctx.log_info(format!("Added {name}"));
            }
            Err(err) => ctx.log_error(format!("Could not add the coupling: {err}")),
        }
    }
}

impl Picking {
    pub(crate) fn new(kind: JointTool, repick: Option<FeatureId>) -> Self {
        Self {
            kind,
            first: None,
            repick,
            tab: None,
            slot: None,
        }
    }

    /// What to click next.
    pub(crate) fn prompt(&self) -> &'static str {
        if self.kind.picks_each() == 2 {
            return match (&self.first, &self.tab, &self.slot) {
                (None, ..) => "Click one face of the tab, on the body to move",
                (Some(_), None, _) => "Click the tab's other face",
                (_, Some(_), None) => "Click one wall of the slot, on another body",
                _ => "Click the slot's other wall",
            };
        }
        self.kind.prompt(self.first.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{activate, context};
    use crate::tests::{face_up, frame, scene, task_frame};
    use crate::*;
    use core_document::{Document, Workbench};

    /// A datum plane selected in the tree while a joint is picked is taken
    /// as its body's face.
    #[test]
    fn a_datum_is_the_other_end() {
        let (mut doc, base, part) = scene();
        let datum = doc
            .add_feature_in_body(
                core_document::DatumFeature {
                    shape: core_document::DatumShape::Plane { size: 10.0 },
                    attachment: core_document::datum::DatumAttachment::BasePlane(
                        core_document::BasePlane::XY,
                    ),
                    offset: core_document::datum::AttachmentOffset {
                        translation: [0.0, 0.0, 7.0],
                        ..Default::default()
                    },
                },
                "Datum".into(),
                Some(base),
            )
            .unwrap();
        let mut wb = AssemblyWorkbench::default();
        activate(&mut wb, &mut context(&mut doc), "asm.mate");
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        {
            let mut ctx = context(&mut doc);
            ctx.active_document_object = Some(datum);
            wb.on_frame(0.016, &mut ctx);
        }
        assert!(wb.picking.is_none(), "the datum made the joint");
        let mate = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .unwrap();
        assert_eq!(mate.feature.other_body, base);
        let placed = doc.body_placement(part);
        assert!((placed.point([2.0, 2.0, 0.0])[2] - 7.0).abs() < 1e-3);
    }

    /// A joint's faces picked again keep the joint: its name, and it is
    /// still the only mate; the part moves to the new face.
    #[test]
    fn a_joint_s_faces_are_picked_again_in_place() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        activate(&mut wb, &mut context(&mut doc), "asm.mate");
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
        let mate = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .unwrap();
        wb.start_repick(mate.id, JointTool::Mate);
        let placed = doc.body_placement(part);
        // The part's face where the mate put it, onto a face 5 up.
        let top = placed.point([2.0, 2.0, 0.0]);
        frame(&mut wb, &mut doc, Some((part, face_up(top[2]))));
        frame(&mut wb, &mut doc, Some((base, face_up(5.0))));
        assert!(wb.picking.is_none());
        let all: Vec<_> = joints(&doc)
            .into_iter()
            .filter(|j| j.feature.kind != JointKind::Ground)
            .collect();
        assert_eq!(all.len(), 1, "the same joint");
        assert_eq!(all[0].id, mate.id);
        assert_eq!(all[0].name, "Mate 1");
        let Anchor::Plane { point, .. } = all[0].feature.fixed else {
            panic!()
        };
        assert!((point[2] - 5.0).abs() < 1e-4, "{point:?}");
        assert!(matches!(wb.task, Some(Task::Joint { id, .. }) if id == mate.id));
    }

    /// Picking fades every body but the one under the cursor and the one
    /// picked first; nothing fades otherwise.
    #[test]
    fn bodies_fade_while_a_joint_is_picked() {
        let (mut doc, base, part) = scene();
        let third = doc.create_body(None);
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = context(&mut doc);
        assert!(wb.faded_bodies(&ctx).is_empty());
        wb.picking = Some(Picking {
            kind: JointTool::Mate,
            first: None,
            repick: None,
            tab: None,
            slot: None,
        });
        ctx.hovered_body_id = Some(base.0);
        let mut faded = wb.faded_bodies(&ctx);
        faded.sort();
        let mut want = vec![part, third];
        want.sort();
        assert_eq!(faded, want);
        wb.picking = Some(Picking {
            kind: JointTool::Mate,
            first: Some((
                part,
                Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
                None,
                0,
            )),
            repick: None,
            tab: None,
            slot: None,
        });
        assert_eq!(wb.faded_bodies(&ctx), vec![third]);
    }

    /// Two picks make a mate that moves the first body onto the second; it
    /// records as the command, which replays it to the same place.
    #[test]
    fn two_picks_make_a_mate_that_moves_the_first_body_and_replays_as_the_command() {
        let (mut doc, base, part) = scene();
        let before = doc.clone();
        let mut wb = AssemblyWorkbench::default();
        activate(&mut wb, &mut context(&mut doc), "asm.mate");
        assert!(wb.picking.is_some());
        // The part's own face, as the scene shows it (placed at z = 40).
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        // The same pick again is not a second pick.
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        assert!(wb.picking.as_ref().unwrap().first.is_some());
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
        assert!(wb.picking.is_none());
        let joints = joints(&doc);
        assert_eq!(joints.len(), 2, "the mate, and the base grounded");
        assert!(
            joints
                .iter()
                .any(|j| j.body == base && j.feature.kind == JointKind::Ground),
            "the first joint grounds the body it holds to"
        );
        let joints: Vec<_> = joints
            .into_iter()
            .filter(|j| j.feature.kind != JointKind::Ground)
            .collect();
        assert_eq!(joints[0].body, part);
        assert_eq!(joints[0].name, "Mate 1");
        // The part's face now lies on the base's, turned to face it.
        let placed = doc.body_placement(part);
        assert!((placed.point([-28.0, 2.0, 0.0])[2]).abs() < 1e-3);
        assert!((placed.direction([0.0, 0.0, 1.0])[2] + 1.0).abs() < 1e-4);
        assert!(matches!(wb.task, Some(Task::Joint { before: None, .. })));

        let recorded = task_frame(
            &mut wb,
            &mut doc,
            core_document::TaskRequest {
                accept: true,
                cancel: false,
            },
        );
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].id, "asm.mate");

        let mut replay = before;
        let mut ctx = WorkbenchRuntimeContext::new(&mut replay, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        AssemblyWorkbench::default()
            .run_command(&recorded[0].id, &recorded[0].args, &mut ctx)
            .unwrap();
        let (a, b) = (replay.body_placement(part), doc.body_placement(part));
        for i in 0..3 {
            assert!(
                (a.translation[i] - b.translation[i]).abs() < 1e-4,
                "{a:?} {b:?}"
            );
        }
        for i in 0..4 {
            assert!((a.rotation[i] - b.rotation[i]).abs() < 1e-4, "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_hole_s_rim_is_an_axis_for_a_hinge() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        let rim = |body: BodyId, center: [f32; 3]| core_document::EdgeRef {
            faces: [0, 0],
            point: [center[0] + 3.0, center[1], center[2]],
            direction: [0.0, 1.0, 0.0],
            length_mm: 18.85,
            body: body.0,
            circle: Some(core_document::EdgeCircle {
                center,
                normal: [0.0, 0.0, 1.0],
                radius: 3.0,
            }),
        };
        let edge_frame = |wb: &mut AssemblyWorkbench, doc: &mut Document, edge| {
            let mut ctx = context(doc);
            ctx.selected_edges = vec![edge];
            wb.on_frame(0.016, &mut ctx);
        };
        activate(&mut wb, &mut context(&mut doc), "asm.hinge");
        // The part's rim where the scene shows it (placed at x + 30, z + 40).
        edge_frame(&mut wb, &mut doc, rim(part, [35.0, 5.0, 40.0]));
        edge_frame(&mut wb, &mut doc, rim(base, [5.0, 5.0, 0.0]));
        assert!(wb.picking.is_none(), "two rims make the hinge");
        let joint = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .expect("a hinge");
        assert!(matches!(joint.feature.kind, JointKind::Hinge { .. }));
        let origin = doc.body_placement(part).point([0.0; 3]);
        assert!(
            origin[0].abs() < 1e-3 && origin[1].abs() < 1e-3 && origin[2].abs() < 1e-3,
            "the rims on one axis at one height: {origin:?}"
        );
    }

    #[test]
    fn a_round_face_is_asked_for_where_an_alignment_needs_one() {
        let (mut doc, _, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        activate(&mut wb, &mut context(&mut doc), "asm.align");
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        assert!(
            wb.picking.as_ref().unwrap().first.is_none(),
            "a flat face is no axis"
        );
    }
}
