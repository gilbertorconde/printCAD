//! The Assembly workbench: bodies placed against each other with joints.
//!
//! A joint belongs to the body it moves and names the body it holds
//! against. Mate puts two flat faces together (with a gap, or facing the
//! same way); Align puts two round faces on one axis. Joints are solved when
//! one is made or edited and when a body is moved, and the bodies' new
//! placements are ordinary edits, so a joint and the move it causes undo as
//! one step and reach every copy of the document the same way.

mod joint;
#[cfg(feature = "egui")]
mod panel;
mod solve;

use core_document::{
    BodyId, BodyPlacement, FaceRef, FeatureId, FeatureInfo, FeatureNode, HostRequest, InputResult,
    ToolDescriptor, ToolHint, ViewportHud, Workbench, WorkbenchContext, WorkbenchDescriptor,
    WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

pub use joint::{Anchor, JOINT_KIND, JointFeature, JointKind, Rigid};
pub use solve::{HOLDS_MM, Joint, SolveError, joints, solve};

/// Which joint a pick sequence makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickKind {
    Mate,
    Align,
    Angle,
}

impl PickKind {
    fn label(self) -> &'static str {
        match self {
            PickKind::Mate => "Mate faces",
            PickKind::Align => "Align axes",
            PickKind::Angle => "Angle between faces",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            PickKind::Mate => "joint-mate",
            PickKind::Align => "joint-align",
            PickKind::Angle => "constraint-angle",
        }
    }

    /// What to click next.
    fn prompt(self, first_done: bool) -> &'static str {
        match (self, first_done) {
            (PickKind::Mate, false) => "Click a flat face on the body to move",
            (PickKind::Mate, true) => "Click the face it goes against, on another body",
            (PickKind::Align, false) => "Click a round face on the body to move",
            (PickKind::Align, true) => "Click the round face it lines up with, on another body",
            (PickKind::Angle, false) => "Click a flat face on the body to turn",
            (PickKind::Angle, true) => "Click the face it keeps its angle to, on another body",
        }
    }
}

/// A joint being made: the kind, and the first face once picked.
#[derive(Debug, Clone)]
struct Picking {
    kind: PickKind,
    first: Option<(BodyId, Anchor)>,
}

/// What the task panel holds open.
#[derive(Debug, Clone)]
enum Task {
    /// A joint's settings. `before` is its data when the task opened, or
    /// `None` for a joint the tool just made.
    Joint {
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: Vec<(BodyId, BodyPlacement)>,
    },
    /// A body moved by hand.
    Move {
        body: BodyId,
        placements: Vec<(BodyId, BodyPlacement)>,
    },
}

#[derive(Default)]
pub struct AssemblyWorkbench {
    picking: Option<Picking>,
    /// The selection a pick was last taken from, so a pick is a change.
    seen: Option<(uuid::Uuid, [u32; 3])>,
    task: Option<Task>,
    /// What the last solve said, for the task panel.
    verdict: Option<Result<String, String>>,
}

/// Every body's placement, to put back when a task is cancelled.
fn all_placements(ctx: &WorkbenchRuntimeContext) -> Vec<(BodyId, BodyPlacement)> {
    ctx.document
        .bodies()
        .iter()
        .map(|b| (b.id, b.placement))
        .collect()
}

fn restore_placements(ctx: &mut WorkbenchRuntimeContext, placements: &[(BodyId, BodyPlacement)]) {
    for (body, placement) in placements {
        ctx.document.set_body_placement(*body, *placement);
    }
}

fn body_name(ctx: &WorkbenchRuntimeContext, body: BodyId) -> String {
    ctx.document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "a removed body".to_string())
}

impl AssemblyWorkbench {
    /// Place every jointed body, as edits, and say how it went.
    fn solve_and_apply(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.verdict = Some(match solve(ctx.document) {
            Ok(moves) => {
                let count = moves.len();
                for (body, placement) in moves {
                    ctx.document.set_body_placement(body, placement);
                }
                Ok(match count {
                    0 => "Every joint holds".to_string(),
                    1 => "Moved 1 body; every joint holds".to_string(),
                    n => format!("Moved {n} bodies; every joint holds"),
                })
            }
            Err(SolveError::Loop(bodies)) => {
                let names: Vec<String> = bodies.iter().map(|b| body_name(ctx, *b)).collect();
                Err(format!(
                    "These bodies are joined in a ring, so none can go first: {}",
                    names.join(", ")
                ))
            }
            Err(SolveError::Conflict { body, joints }) => Err(format!(
                "{} cannot hold all its joints at once: {}",
                body_name(ctx, body),
                joints.join(", ")
            )),
        });
        if let Some(Err(message)) = &self.verdict {
            ctx.log_warn(message.clone());
        }
    }

    /// Take a new face pick for the joint being made.
    fn take_pick(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(picking) = self.picking.clone() else {
            return;
        };
        let (Some(body), Some(face)) = (ctx.selected_body_id, ctx.selected_face) else {
            return;
        };
        let signature = (body, face.point.map(f32::to_bits));
        if self.seen == Some(signature) {
            return;
        }
        self.seen = Some(signature);
        let body = BodyId(body);
        let local: FaceRef = face.moved(&ctx.document.body_placement(body).inverse());
        let anchor = match picking.kind {
            PickKind::Mate | PickKind::Angle => Anchor::plane_of(&local),
            PickKind::Align => Anchor::axis_of(&local),
        };
        let Some(anchor) = anchor else {
            ctx.log_warn(match picking.kind {
                PickKind::Mate | PickKind::Angle => "This joint takes flat faces",
                PickKind::Align => "An alignment takes round faces: a hole, a pin, a boss",
            });
            return;
        };
        match picking.first {
            None => {
                self.picking = Some(Picking {
                    first: Some((body, anchor)),
                    ..picking
                });
            }
            Some((first_body, _)) if first_body == body => {
                ctx.log_warn("Pick the second face on another body");
            }
            Some((first_body, first_anchor)) => {
                self.picking = None;
                let kind = match picking.kind {
                    PickKind::Mate => JointKind::Mate {
                        flip: false,
                        offset: 0.0,
                    },
                    PickKind::Align => JointKind::Align,
                    // It starts at the angle the faces make now, so making
                    // it moves nothing until the angle is set.
                    PickKind::Angle => JointKind::Angle {
                        degrees: first_anchor.angle_to(
                            &ctx.document.body_placement(first_body).into(),
                            &anchor,
                            &ctx.document.body_placement(body).into(),
                        ),
                    },
                };
                self.make_joint(
                    ctx,
                    first_body,
                    JointFeature {
                        kind,
                        moving: first_anchor,
                        other_body: body,
                        fixed: anchor,
                    },
                );
            }
        }
    }

    /// Add a joint, solve, and open its settings.
    fn make_joint(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId, joint: JointFeature) {
        let placements = all_placements(ctx);
        let label = joint.kind.label();
        let number = joints(ctx.document)
            .iter()
            .filter(|j| j.feature.kind.label() == label)
            .count()
            + 1;
        let name = format!("{label} {number}");
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
    fn selected_joint(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == JOINT_KIND).then_some(id)
    }

    /// The body the Move tool moves: the selected one, else the one the
    /// tree has active.
    fn body_to_move(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        ctx.selected_body_id.map(BodyId).or_else(|| {
            ctx.active_document_object
                .and_then(|id| ctx.document.get_feature_meta(id))
                .and_then(|node| node.body)
        })
    }

    /// Whether a body's own joints place it: then moving it by hand is
    /// undone by the next solve.
    fn held_by_joints(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        joints(ctx.document).iter().any(|j| j.body == body)
    }
}

impl Workbench for AssemblyWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            JOINT_KIND,
            "Assembly",
            "Place bodies against each other with joints",
        )
        .icon("workbench-assembly")
        .feature_kinds([JOINT_KIND])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        let tool = |id: &str, label: &str, icon: &'static str| {
            ToolDescriptor::new_action(id, label, Some("joints"))
                .icon(icon)
                .row(1)
        };
        context.register_tool(tool("asm.mate", "Mate faces", "joint-mate").shortcut("M"));
        context.register_tool(tool("asm.align", "Align axes", "joint-align").shortcut("A"));
        context.register_tool(tool("asm.angle", "Angle between faces", "constraint-angle"));
        context.register_tool(tool("asm.move", "Move body", "move-geometry").shortcut("G"));
        context.register_tool(tool("asm.solve", "Solve joints", "refresh"));
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        let kind = JointFeature::from_json(&node.data).map(|j| j.kind).ok();
        FeatureInfo {
            icon: kind.map_or("joint-mate", |k| k.icon()),
            kind_label: kind.map_or("Joint", |k| k.label()).to_string(),
            family_label: "Assembly joint".to_string(),
            builds_solid: false,
        }
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            "asm.mate" | "asm.align" | "asm.angle" => ctx.document.bodies().len() >= 2,
            "asm.move" => Self::body_to_move(ctx).is_some(),
            "asm.solve" => !joints(ctx.document).is_empty(),
            _ => false,
        }
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if !matches!(event, WorkbenchInputEvent::ToolActivated) {
            return InputResult::ignored();
        }
        match tool {
            Some(id @ ("asm.mate" | "asm.align" | "asm.angle")) => {
                let kind = match id {
                    "asm.mate" => PickKind::Mate,
                    "asm.align" => PickKind::Align,
                    _ => PickKind::Angle,
                };
                self.task = None;
                self.seen = None;
                self.picking = Some(Picking { kind, first: None });
                // A face already selected is the first pick.
                self.take_pick(ctx);
            }
            Some("asm.move") => match Self::body_to_move(ctx) {
                Some(body) => {
                    self.picking = None;
                    self.task = Some(Task::Move {
                        body,
                        placements: all_placements(ctx),
                    });
                }
                None => ctx.log_warn("Select a body to move"),
            },
            Some("asm.solve") => {
                self.solve_and_apply(ctx);
                if let Some(Ok(message)) = &self.verdict {
                    ctx.log_info(message.clone());
                }
                ctx.request(HostRequest::JournalLabel("Solve joints".into()));
            }
            _ => return InputResult::ignored(),
        }
        InputResult::consumed()
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        if self.picking.is_some() {
            self.take_pick(ctx);
            return;
        }
        // A joint picked in the tree opens its settings; picking something
        // else closes them, keeping what was set.
        let selected = Self::selected_joint(ctx);
        match (&self.task, selected) {
            (Some(Task::Joint { id, .. }), Some(joint)) if *id == joint => {}
            (Some(Task::Move { .. }), _) => {}
            (_, Some(joint)) => {
                self.task = Some(Task::Joint {
                    id: joint,
                    before: ctx.document.get_feature_data(joint).cloned(),
                    placements: all_placements(ctx),
                });
                self.verdict = None;
            }
            (Some(Task::Joint { .. }), None) => self.task = None,
            (None, None) => {}
        }
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        if let Some(picking) = &self.picking {
            return Some(core_document::TaskInfo {
                title: picking.kind.label().to_string(),
                icon: picking.kind.icon(),
                confirmable: false,
            });
        }
        match self.task.as_ref()? {
            Task::Joint { id, .. } => {
                let kind = ctx
                    .document
                    .get_feature_data(*id)
                    .and_then(|d| JointFeature::from_json(d).ok())
                    .map(|j| j.kind)?;
                Some(core_document::TaskInfo {
                    title: kind.label().to_string(),
                    icon: kind.icon(),
                    confirmable: true,
                })
            }
            Task::Move { .. } => Some(core_document::TaskInfo {
                title: "Move body".to_string(),
                icon: "move-geometry",
                confirmable: true,
            }),
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

    fn viewport_hud(&self, _ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let picking = self.picking.as_ref()?;
        Some(ViewportHud {
            tool: Some(ToolHint {
                icon: picking.kind.icon(),
                name: picking.kind.label().to_string(),
                prompt: picking.kind.prompt(picking.first.is_some()).to_string(),
                keys: vec![("Esc".to_string(), "cancel")],
            }),
            ..ViewportHud::default()
        })
    }

    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.picking = None;
        self.task = None;
    }

    fn on_deactivate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.picking = None;
        self.task = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, ImportedGeometry, TriMesh};
    use kernel_api::FaceSurface;
    use std::sync::Arc;

    /// Two bodies, each a flat square facing up at its own height.
    fn scene() -> (Document, BodyId, BodyId) {
        let mut doc = Document::new("t");
        let base = doc.create_body(Some("Base".into()));
        let part = doc.create_body(Some("Part".into()));
        for body in [base, part] {
            doc.set_imported_geometry(
                body,
                ImportedGeometry {
                    mesh: Arc::new(TriMesh {
                        positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                        normals: vec![[0.0, 0.0, 1.0]; 3],
                        indices: vec![0, 1, 2],
                        ..TriMesh::default()
                    }),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: None,
                    brep_blob_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
        }
        doc.set_body_placement(
            part,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(30.0, 0.0, 40.0)),
        );
        (doc, base, part)
    }

    fn face_up(z: f32) -> FaceRef {
        FaceRef {
            point: [2.0, 2.0, z],
            normal: [0.0, 0.0, 1.0],
            surface: Some(FaceSurface::Plane {
                origin: [0.0, 0.0, z],
                normal: [0.0, 0.0, 1.0],
            }),
        }
    }

    fn frame(wb: &mut AssemblyWorkbench, doc: &mut Document, pick: Option<(BodyId, FaceRef)>) {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.selected_body_id = pick.map(|(b, _)| b.0);
        ctx.selected_face = pick.map(|(_, f)| f);
        wb.on_frame(0.016, &mut ctx);
    }

    #[test]
    fn two_picks_make_a_mate_that_moves_the_first_body_onto_the_second() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.mate"),
                &mut ctx,
            );
        }
        assert!(wb.picking.is_some());
        // The part's own face, as the scene shows it (placed at z = 40).
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        // The same pick again is not a second pick.
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        assert!(wb.picking.as_ref().unwrap().first.is_some());
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
        assert!(wb.picking.is_none());
        let joints = joints(&doc);
        assert_eq!(joints.len(), 1);
        assert_eq!(joints[0].body, part);
        assert_eq!(joints[0].name, "Mate 1");
        // The part's face now lies on the base's, turned to face it.
        let placed = doc.body_placement(part);
        assert!((placed.point([-28.0, 2.0, 0.0])[2]).abs() < 1e-3);
        assert!((placed.direction([0.0, 0.0, 1.0])[2] + 1.0).abs() < 1e-4);
        assert!(matches!(wb.task, Some(Task::Joint { before: None, .. })));
    }

    #[test]
    fn a_round_face_is_asked_for_where_an_alignment_needs_one() {
        let (mut doc, _, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.align"),
                &mut ctx,
            );
        }
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        assert!(
            wb.picking.as_ref().unwrap().first.is_none(),
            "a flat face is no axis"
        );
    }
}

#[cfg(all(test, feature = "egui"))]
mod icon_coverage {
    use super::*;

    #[test]
    fn the_bench_its_tools_and_its_joints_name_icons_in_the_set() {
        let wb = AssemblyWorkbench::default();
        assert!(ui_kit::icon::exists(wb.descriptor().icon));
        let mut context = WorkbenchContext::default();
        wb.configure(&mut context);
        for tool in context.tools() {
            let icon = tool.icon.expect("every tool has an icon");
            assert!(ui_kit::icon::exists(icon), "{icon}");
        }
        for kind in [
            JointKind::Mate {
                flip: false,
                offset: 0.0,
            },
            JointKind::Align,
            JointKind::Angle { degrees: 90.0 },
        ] {
            assert!(ui_kit::icon::exists(kind.icon()), "{}", kind.icon());
        }
    }
}
