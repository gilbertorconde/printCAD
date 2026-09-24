//! The Assembly workbench: bodies placed against each other with joints.
//!
//! A joint belongs to the body it moves and names the body it holds
//! against. Mate puts two flat faces together (with a gap, or facing the
//! same way); Align puts two round faces on one axis; a hinge, a slider, a
//! fixed joint, parallel, perpendicular, distance, angle and tangent joints
//! hold what their names say (`JointTool` lists them, what each takes and
//! how each starts). An axis comes from a round face or an edge. Joints are solved when
//! one is made or edited and when a body is moved, and the bodies' new
//! placements are ordinary edits, so a joint and the move it causes undo as
//! one step and reach every copy of the document the same way.

mod commands;
mod interference;
mod joint;
#[cfg(feature = "egui")]
mod panel;
mod solve;

use core_document::{
    BodyId, BodyPlacement, FeatureId, FeatureInfo, FeatureNode, HostRequest, InputResult,
    ToolDescriptor, ToolHint, ViewportHud, Workbench, WorkbenchContext, WorkbenchDescriptor,
    WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

pub use interference::{Clash, Interference, interference};
pub use joint::{Anchor, Drive, JOINT_KIND, JointFeature, JointKind, JointTool, Rigid, Takes};
pub use solve::{HOLDS_MM, Joint, Motion, SolveError, drag, draggable, freedom, joints, solve};

/// A joint being made: the kind, and the first face once picked.
#[derive(Debug, Clone)]
struct Picking {
    kind: JointTool,
    /// The first body, its anchor, and the radius of a round face.
    first: Option<(BodyId, Anchor, Option<f32>)>,
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
    /// Where bodies clash, as found at edit `seq` of the document.
    Interference { found: Interference, seq: u64 },
}

#[derive(Default)]
pub struct AssemblyWorkbench {
    picking: Option<Picking>,
    /// The selection a pick was last taken from, so a pick is a change.
    seen: Option<(uuid::Uuid, [u32; 3])>,
    task: Option<Task>,
    /// What the last solve said, for the task panel.
    verdict: Option<Result<String, String>>,
    /// What each jointed body may still do, and at which edit of the
    /// document that was worked out: the status bar asks every frame.
    freedom: std::sync::Mutex<Option<(u64, Freedom)>>,
    /// A body held by the mouse, dragged with its joints holding.
    grab: Option<Grab>,
    /// A driven hinge or slider swept through its range to show it move.
    #[cfg(feature = "egui")]
    playing: Option<Play>,
}

/// A body taken by the mouse: the point taken, in the body's own frame,
/// and the plane facing the view it is dragged across.
#[derive(Debug, Clone)]
struct Grab {
    body: BodyId,
    point: [f32; 3],
    plane: ([f32; 3], [f32; 3]),
    press: (f32, f32),
    dragging: bool,
    placements: Vec<(BodyId, BodyPlacement)>,
}

/// How far, in pixels, the mouse goes before a press is a drag.
const DRAG_PX: f32 = 4.0;

/// A joint's drive being swept: the value it held before, to go back to,
/// and how far round the sweep is (radians of a cosine).
#[cfg(feature = "egui")]
#[derive(Debug, Clone, Copy)]
struct Play {
    joint: FeatureId,
    start: f32,
    phase: f64,
}

/// Each jointed body and the motions its joints leave it.
type Freedom = Vec<(BodyId, Vec<Motion>)>;

impl AssemblyWorkbench {
    /// Each jointed body's free motions, worked out again only after an
    /// edit.
    fn freedom_now(&self, ctx: &WorkbenchRuntimeContext) -> Freedom {
        let seq = ctx.document.mutation_seq();
        let mut cache = self.freedom.lock().unwrap();
        match &*cache {
            Some((at, found)) if *at == seq => found.clone(),
            _ => {
                let found = freedom(ctx.document);
                *cache = Some((seq, found.clone()));
                found
            }
        }
    }
}

/// A drive's numbers, those it has: what it holds the motion at and its
/// limits.
fn drive_parameters(
    variant: &str,
    drive: &Drive,
    dim: core_document::expr::Dim,
) -> Vec<core_document::Parameter> {
    use core_document::Parameter;
    let mut out = Vec::new();
    if drive.to.is_some() {
        out.push(Parameter::new(
            "drive",
            "Drive",
            dim,
            format!("/kind/{variant}/drive/to"),
        ));
    }
    if drive.limits.is_some() {
        for (end, name, label) in [(0, "lowest", "Lowest"), (1, "highest", "Highest")] {
            out.push(Parameter::new(
                name,
                label,
                dim,
                format!("/kind/{variant}/drive/limits/{end}"),
            ));
        }
    }
    out
}

impl AssemblyWorkbench {
    /// Look for clashes among the visible solid bodies and show them.
    pub(crate) fn check_interference(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to check interference with");
            return;
        };
        match interference(ctx.document, kernel, None) {
            Ok(found) => {
                ctx.log_info(match found.clashes.len() {
                    0 => format!("No interference among {} bodies", found.checked),
                    n => format!(
                        "{n} clash{} among {} bodies",
                        if n == 1 { "" } else { "es" },
                        found.checked
                    ),
                });
                self.task = Some(Task::Interference {
                    found,
                    seq: ctx.document.mutation_seq(),
                });
            }
            Err(why) => ctx.log_warn(format!("The interference check failed: {why}")),
        }
    }

    /// The clashes shown, each where it sits on screen.
    fn clashes_on_screen<'a>(
        &'a self,
        ctx: &'a WorkbenchRuntimeContext,
    ) -> impl Iterator<Item = ([f32; 2], &'a Clash)> + 'a {
        let clashes = match &self.task {
            Some(Task::Interference { found, .. }) => found.clashes.as_slice(),
            _ => &[],
        };
        clashes.iter().filter_map(|clash| {
            let (x, y) = ctx.world_to_viewport(clash.centre)?;
            Some(([x, y], clash))
        })
    }
}

impl AssemblyWorkbench {
    /// A press on a body its joints move takes hold of it; the press still
    /// goes on to the host, which selects on a click.
    fn take_hold(&mut self, ctx: &WorkbenchRuntimeContext, at: (f32, f32)) {
        self.grab = None;
        if self.picking.is_some() || matches!(self.task, Some(Task::Move { .. })) {
            return;
        }
        let (Some(body), Some(point)) = (ctx.hovered_body_id, ctx.hovered_world_pos) else {
            return;
        };
        let body = BodyId(body);
        let Some((_, facing)) = ctx.viewport_to_ray(at) else {
            return;
        };
        if !draggable(ctx.document, body) {
            return;
        }
        self.grab = Some(Grab {
            body,
            point: ctx.document.body_placement(body).inverse().point(point),
            plane: (point, facing),
            press: at,
            dragging: false,
            placements: all_placements(ctx),
        });
    }

    /// The held body follows the mouse across the plane it was taken in,
    /// as far as its joints let it. The moves still reach the host, so a
    /// drag is never taken for a click.
    fn drag_to(&mut self, ctx: &mut WorkbenchRuntimeContext, at: (f32, f32)) -> InputResult {
        let Some(grab) = self.grab.as_mut() else {
            return InputResult::ignored();
        };
        let moved = (at.0 - grab.press.0).hypot(at.1 - grab.press.1);
        if !grab.dragging && moved < DRAG_PX {
            return InputResult::ignored();
        }
        grab.dragging = true;
        let (origin, normal) = grab.plane;
        let Some(target) = ctx.viewport_to_plane(at, origin, normal) else {
            return InputResult::ignored();
        };
        for (body, placement) in drag(ctx.document, grab.body, grab.point, target) {
            ctx.document.set_body_placement(body, placement);
        }
        InputResult::redraw_only()
    }

    /// The drag ends: the bodies stay where it left them, recorded as
    /// placements.
    fn let_go(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(grab) = self.grab.take() else {
            return InputResult::ignored();
        };
        if !grab.dragging {
            return InputResult::ignored();
        }
        for (body, before) in grab.placements {
            let now = ctx.document.body_placement(body);
            if now.after(&before.inverse()).is_identity() {
                continue;
            }
            ctx.record(
                "asm.place",
                commands::object(serde_json::json!({
                    "body": body.0.to_string(),
                    "translation": now.translation,
                    "rotation": now.rotation,
                })),
                serde_json::Value::Null,
            );
        }
        ctx.request(HostRequest::JournalLabel("Drag body".into()));
        InputResult::redraw_only()
    }
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

/// Place every jointed body, as edits: how many moved, or why they could
/// not all be placed.
pub(crate) fn apply_solve(ctx: &mut WorkbenchRuntimeContext) -> Result<String, String> {
    match solve(ctx.document) {
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
        Err(SolveError::Conflict { body, joints }) => Err(format!(
            "{} cannot hold all its joints at once: {}",
            body_name(ctx, body),
            joints.join(", ")
        )),
    }
}

impl AssemblyWorkbench {
    /// Place every jointed body, as edits, and say how it went.
    fn solve_and_apply(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.verdict = Some(apply_solve(ctx));
        if let Some(Err(message)) = &self.verdict {
            ctx.log_warn(message.clone());
        }
    }

    /// Take a new face pick for the joint being made.
    fn take_pick(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(picking) = self.picking.clone() else {
            return;
        };
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
            .map(|(_, anchor, _)| matches!(anchor, Anchor::Plane { .. }));
        let picked = match (picking.kind.takes(), first_flat) {
            (Takes::Flat, _) => flat().map(|a| (a, None)),
            (Takes::Round, _) => round(),
            (Takes::Any, _) => flat().map(|a| (a, None)).or_else(round),
            (Takes::FlatAndRound, None) => flat().map(|a| (a, None)).or_else(round),
            (Takes::FlatAndRound, Some(true)) => round(),
            (Takes::FlatAndRound, Some(false)) => flat().map(|a| (a, None)),
        };
        let Some((anchor, radius)) = picked else {
            ctx.log_warn(picking.kind.refusal());
            return;
        };
        match picking.first {
            None => {
                self.picking = Some(Picking {
                    first: Some((body, anchor, radius)),
                    ..picking
                });
            }
            Some((first_body, ..)) if first_body == body => {
                ctx.log_warn("Pick the second face on another body");
            }
            Some((first_body, first_anchor, first_radius)) => {
                self.picking = None;
                let radius = first_radius.or(radius);
                if picking.kind == JointTool::Tangent && radius.is_none() {
                    ctx.log_warn("A tangent takes a round face with a radius: a cylinder");
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
    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        use core_document::Parameter;
        use core_document::expr::Dim;
        match JointFeature::from_json(&node.data).map(|j| j.kind) {
            Ok(JointKind::Mate { .. }) => {
                vec![Parameter::new(
                    "offset",
                    "Offset",
                    Dim::LENGTH,
                    "/kind/Mate/offset",
                )]
            }
            Ok(JointKind::Angle { .. }) => {
                vec![Parameter::new(
                    "angle",
                    "Angle",
                    Dim::ANGLE,
                    "/kind/Angle/degrees",
                )]
            }
            Ok(JointKind::Hinge { drive, .. }) => {
                let mut out = vec![Parameter::new(
                    "offset",
                    "Height",
                    Dim::LENGTH,
                    "/kind/Hinge/offset",
                )];
                out.extend(drive_parameters("Hinge", &drive, Dim::ANGLE));
                out
            }
            Ok(JointKind::Slider { drive, .. }) => drive_parameters("Slider", &drive, Dim::LENGTH),
            Ok(JointKind::Distance { .. }) => {
                vec![Parameter::new(
                    "offset",
                    "Distance",
                    Dim::LENGTH,
                    "/kind/Distance/offset",
                )]
            }
            Ok(JointKind::Tangent { .. }) => {
                vec![Parameter::new(
                    "radius",
                    "Radius",
                    Dim::LENGTH,
                    "/kind/Tangent/radius",
                )]
            }
            _ => Vec::new(),
        }
    }

    /// A joint whose offset or angle a formula moved: the bodies follow.
    fn values_moved(&mut self, ctx: &mut WorkbenchRuntimeContext, moved: &[FeatureId]) {
        let joint_moved = moved.iter().any(|id| {
            ctx.document
                .get_feature_meta(*id)
                .is_some_and(|n| n.workbench_id.as_str() == JOINT_KIND)
        });
        if joint_moved {
            self.solve_and_apply(ctx);
        }
    }

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
        commands::register(context);
        let tool = |id: &str, label: &str, icon: &'static str| {
            ToolDescriptor::new_action(id, label, Some("joints"))
                .icon(icon)
                .row(1)
        };
        for joint in JointTool::ALL {
            context.register_tool(
                tool(joint.command(), joint.label(), joint.icon()).shortcut(joint.shortcut()),
            );
        }
        context.register_tool(tool("asm.move", "Move body", "move-geometry").shortcut("G"));
        context.register_tool(
            tool("asm.interference", "Check interference", "check-geometry").shortcut("I"),
        );
        context.register_tool(tool("asm.ground", "Ground body", "constraint-lock").shortcut("F"));
        context.register_tool(tool("asm.solve", "Solve joints", "refresh").shortcut("S"));
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        commands::run(id, args, ctx)
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

    /// Whether the assembly is fully placed, or how many motions its
    /// joints leave open; the selected body's, in words.
    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        let free = self.freedom_now(ctx);
        if free.is_empty() {
            return None;
        }
        let pal = ctx.sketch_palette;
        let open: usize = free.iter().map(|(_, m)| m.len()).sum();
        let state = if open == 0 {
            (pal.fully_constrained, "Fully placed".to_string())
        } else {
            (
                pal.constraint,
                format!("{open} motion{} free", if open == 1 { "" } else { "s" }),
            )
        };
        let selection = Self::body_to_move(ctx).and_then(|body| {
            let (_, motions) = free.iter().find(|(b, _)| *b == body)?;
            let name = ctx
                .document
                .bodies()
                .iter()
                .find(|b| b.id == body)
                .map(|b| b.name.clone())
                .unwrap_or_default();
            Some(if motions.is_empty() {
                format!("{name}: fully placed")
            } else {
                let words: Vec<String> = motions.iter().map(Motion::describe).collect();
                format!("{name}: {}", words.join(", "))
            })
        });
        Some(core_document::StatusItems {
            state: Some(state),
            selection,
            coords: None,
            mode: None,
        })
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            id if JointTool::of_command(id).is_some() => ctx.document.bodies().len() >= 2,
            "asm.interference" => ctx.document.bodies().len() >= 2,
            "asm.move" | "asm.ground" => Self::body_to_move(ctx).is_some(),
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
        match event {
            WorkbenchInputEvent::ToolActivated => {}
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } if tool.is_none() => {
                self.take_hold(ctx, *viewport_pos);
                return InputResult::ignored();
            }
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                return self.drag_to(ctx, *viewport_pos);
            }
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                ..
            } => return self.let_go(ctx),
            _ => return InputResult::ignored(),
        }
        match tool {
            Some(id) if JointTool::of_command(id).is_some() => {
                let kind = JointTool::of_command(id).unwrap_or(JointTool::Mate);
                self.task = None;
                self.seen = None;
                self.picking = Some(Picking { kind, first: None });
                // A face already selected is the first pick.
                self.take_pick(ctx);
            }
            Some("asm.interference") => {
                self.picking = None;
                self.check_interference(ctx);
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
            Some("asm.ground") => match Self::body_to_move(ctx) {
                Some(body) => {
                    let grounded = joints(ctx.document)
                        .iter()
                        .any(|j| j.body == body && j.feature.kind == JointKind::Ground);
                    let made = commands::set_grounded(ctx, body, !grounded);
                    ctx.record(
                        "asm.ground",
                        commands::object(serde_json::json!({
                            "body": body.0.to_string(),
                            "grounded": !grounded,
                        })),
                        made.map_or(serde_json::Value::Null, |id| {
                            serde_json::json!(id.0.to_string())
                        }),
                    );
                    self.solve_and_apply(ctx);
                    ctx.request(HostRequest::JournalLabel(
                        if grounded {
                            "Unground body"
                        } else {
                            "Ground body"
                        }
                        .into(),
                    ));
                }
                None => ctx.log_warn("Select a body to ground"),
            },
            Some("asm.solve") => {
                self.solve_and_apply(ctx);
                if let Some(Ok(message)) = &self.verdict {
                    ctx.log_info(message.clone());
                    // A solve that left joints apart would stop a replay.
                    ctx.record(
                        "asm.solve",
                        core_document::CommandArgs::new(),
                        serde_json::json!(message),
                    );
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
            (Some(Task::Move { .. } | Task::Interference { .. }), _) => {}
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
            Task::Interference { .. } => Some(core_document::TaskInfo {
                title: "Interference".to_string(),
                icon: "check-geometry",
                confirmable: false,
            }),
        }
    }

    /// Each clash found, marked where it is.
    fn get_screen_space_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        self.clashes_on_screen(ctx)
            .map(|(pos, _)| {
                core_document::ScreenSpaceMark::icon(
                    pos,
                    "warning",
                    18.0,
                    ctx.sketch_palette.conflict,
                )
            })
            .collect()
    }

    /// How much each clash shares, beside its mark.
    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        self.clashes_on_screen(ctx)
            .map(|([x, y], clash)| {
                core_document::ScreenSpaceLabel::new(
                    [x, y + 20.0],
                    format!("{:.1} mm³", clash.volume_mm3),
                    ctx.sketch_palette.conflict,
                    12.0,
                )
                .pill()
                .mono()
            })
            .collect()
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
    use core_document::{Document, FaceRef, ImportedGeometry, TriMesh};
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

    /// One frame of the task panel, as the host runs it; what it recorded.
    fn task_frame(
        wb: &mut AssemblyWorkbench,
        doc: &mut Document,
        request: core_document::TaskRequest,
    ) -> Vec<core_document::Recorded> {
        let egui_ctx = egui::Context::default();
        ui_kit::apply_theme(&egui_ctx);
        let mut recorded = Vec::new();
        let mut output = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.ui_task_panel(ui, &mut ctx, request);
            recorded = core_document::HookOutcome::take(&mut ctx).recorded;
        });
        output.textures_delta.clear();
        recorded
    }

    #[test]
    fn a_mate_made_by_picks_records_as_the_command_and_replays_to_the_same_place() {
        let (mut doc, base, part) = scene();
        let before = doc.clone();
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
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
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
    fn a_body_dragged_by_the_mouse_turns_on_its_hinge_and_is_recorded() {
        use glam::{Mat4, Vec3};
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let door = doc.create_body(None);
        let pin = Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        };
        doc.add_feature_in_body(
            JointFeature {
                kind: JointTool::Hinge.joint(
                    &pin,
                    &Rigid::from(BodyPlacement::default()),
                    &pin,
                    &Rigid::from(BodyPlacement::default()),
                    0.0,
                ),
                moving: pin,
                other_body: frame,
                fixed: pin,
            },
            "Hinge 1".into(),
            Some(door),
        )
        .unwrap();
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1000.0,
        );
        let view =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO, Vec3::Y);
        let vp = (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d();
        let mut wb = AssemblyWorkbench::default();
        let mut send = |doc: &mut Document, event: WorkbenchInputEvent| {
            let mut ctx =
                WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
            ctx.view_proj = Some(vp);
            ctx.hovered_body_id = Some(door.0);
            ctx.hovered_world_pos = Some([10.0, 0.0, 0.0]);
            let result = wb.on_input(&event, None, &mut ctx);
            let outcome = core_document::HookOutcome::take(&mut ctx);
            (result, outcome.requests, outcome.recorded)
        };
        let screen = |p: [f32; 3]| {
            core_document::runtime::world_to_viewport(vp, (0, 0, 800, 600), p).unwrap()
        };
        let press = screen([10.0, 0.0, 0.0]);
        let (result, ..) = send(
            &mut doc,
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: press,
            },
        );
        assert!(!result.consumed, "the host still sees the press");
        for step in 1..=10 {
            let turn = step as f32 * 9f32.to_radians();
            let at = screen([10.0 * turn.cos(), 10.0 * turn.sin(), 0.0]);
            send(
                &mut doc,
                WorkbenchInputEvent::MouseMove { viewport_pos: at },
            );
        }
        let (_, requests, recorded) = send(
            &mut doc,
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                viewport_pos: screen([0.0, 10.0, 0.0]),
            },
        );
        let x = doc.body_placement(door).direction([1.0, 0.0, 0.0]);
        assert!(
            (x[1].atan2(x[0]).to_degrees() - 90.0).abs() < 2.0,
            "a quarter turn: {x:?}"
        );
        assert!(doc.body_placement(door).translation[2].abs() < 1e-3);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, "asm.place");
        assert!(requests.contains(&HostRequest::JournalLabel("Drag body".into())));
    }

    #[test]
    fn a_hole_s_rim_is_an_axis_for_a_hinge() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        let rim = |body: BodyId, center: [f32; 3]| core_document::EdgeRef {
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
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            ctx.selected_edges = vec![edge];
            wb.on_frame(0.016, &mut ctx);
        };
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.hinge"),
                &mut ctx,
            );
        }
        // The part's rim where the scene shows it (placed at x + 30, z + 40).
        edge_frame(&mut wb, &mut doc, rim(part, [35.0, 5.0, 40.0]));
        edge_frame(&mut wb, &mut doc, rim(base, [5.0, 5.0, 0.0]));
        assert!(wb.picking.is_none(), "two rims make the hinge");
        let joint = joints(&doc).into_iter().next().expect("a hinge");
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
        for tool in JointTool::ALL {
            let kind = tool.joint(
                &Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
                &Rigid::from(BodyPlacement::default()),
                &Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
                &Rigid::from(BodyPlacement::default()),
                1.0,
            );
            assert!(ui_kit::icon::exists(kind.icon()), "{}", kind.icon());
            assert!(ui_kit::icon::exists(tool.icon()), "{}", tool.icon());
            assert_eq!(JointTool::of_kind(&kind), Some(tool));
        }
        assert!(ui_kit::icon::exists(JointKind::Ground.icon()));
    }
}
