//! Moving bodies by hand: the Move task's handles, and a body taken by
//! the mouse and dragged as far as its joints let it.

use core_document::{
    BodyId, BodyPlacement, HostRequest, InputResult, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

use crate::joint::JointKind;
use crate::solve::{drag, draggable, joints, solve};
use crate::{AssemblyWorkbench, Grab, Task, all_placements, place_bodies};
use crate::{collide, commands, components, handles};

impl AssemblyWorkbench {
    /// The body the Move task has open, when no joint places it: its move
    /// handles show and drag.
    pub(crate) fn handled_body(&self, ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        match &self.task {
            Some(Task::Move { body, .. }) if !Self::held_by_joints(ctx, *body) => Some(*body),
            _ => None,
        }
    }

    /// A press on a move handle takes it; moves slide or turn the body,
    /// Shift in steps; the release lets go, Escape puts the body back. The
    /// task's OK records where it ends.
    pub(crate) fn handle_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        let Some(body) = self.handled_body(ctx) else {
            self.handles.reset();
            return None;
        };
        let taken = match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: (x, y),
            } => self.handles.press(ctx, body, emath::pos2(*x, *y)),
            WorkbenchInputEvent::MouseMove {
                viewport_pos: (x, y),
            } => self.handles.drag(ctx, body, emath::pos2(*x, *y)),
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                ..
            } => self.handles.release(ctx, body),
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } => self.handles.cancel(ctx, body),
            WorkbenchInputEvent::Action { id } if id == handles::MODE_ACTION => {
                self.handles.switch(ctx, body);
                true
            }
            _ => false,
        };
        taken.then(InputResult::consumed)
    }

    /// A press on a body its joints move, or on one no joint places and
    /// nothing grounds, takes hold of it; the press still goes on to the
    /// host, which selects on a click.
    pub(crate) fn take_hold(&mut self, ctx: &WorkbenchRuntimeContext, at: (f32, f32)) {
        self.grab = None;
        if self.picking.is_some()
            || matches!(self.task, Some(Task::Move { .. } | Task::Explode { .. }))
        {
            return;
        }
        let (Some(body), Some(point)) = (ctx.hovered_body_id, ctx.hovered_world_pos) else {
            return;
        };
        let body = BodyId(body);
        let Some((_, ray)) = ctx.viewport_to_ray(at) else {
            return;
        };
        // The plane square to the view, through the point taken.
        let forward =
            glam::Vec3::from_array(ctx.camera_target) - glam::Vec3::from_array(ctx.camera_position);
        let facing = if forward.length_squared() > 1e-12 {
            forward.normalize().to_array()
        } else {
            ray
        };
        let island = if ctx.shift_down {
            crate::solve::ungrounded_island(ctx.document, body).unwrap_or_default()
        } else {
            Vec::new()
        };
        let jointed = draggable(ctx.document, body);
        let unit = ctx.document.rigid_unit_of(body);
        let grounded = joints(ctx.document)
            .iter()
            .any(|j| unit.contains(&j.body) && j.feature.kind == JointKind::Ground);
        if !jointed && grounded {
            return;
        }
        self.grab = Some(Grab {
            body,
            point: ctx.document.body_placement(body).inverse().point(point),
            plane: (point, facing),
            press: at,
            dragging: false,
            baseline: collide::Baseline::new(&all_placements(ctx)),
            placements: all_placements(ctx),
            reached: None,
            free: !jointed,
            island,
        });
    }

    /// The held body follows the mouse across the plane it was taken in,
    /// as far as its joints let it. The moves still reach the host, so a
    /// drag is never taken for a click.
    pub(crate) fn drag_to(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        at: (f32, f32),
    ) -> InputResult {
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
        if !grab.island.is_empty() {
            // The whole island straight across the plane, as one.
            let by = glam::Vec3::from_array(target) - glam::Vec3::from_array(origin);
            let moves: Vec<(BodyId, BodyPlacement)> = grab
                .placements
                .iter()
                .filter(|(b, _)| grab.island.contains(b))
                .map(|(b, start)| (*b, BodyPlacement::new(start.quat(), start.offset() + by)))
                .collect();
            place_bodies(ctx.document, &moves);
            return InputResult::redraw_only();
        }
        if grab.free {
            // Straight across the plane; what is joined to it follows.
            let start = grab
                .placements
                .iter()
                .find(|(b, _)| *b == grab.body)
                .map(|(_, p)| *p)
                .unwrap_or_default();
            let by = glam::Vec3::from_array(target) - glam::Vec3::from_array(origin);
            components::move_with_unit(
                ctx.document,
                grab.body,
                BodyPlacement::new(start.quat(), start.offset() + by),
            );
            if let Ok(moves) = solve(ctx.document) {
                place_bodies(ctx.document, &moves);
            }
            return InputResult::redraw_only();
        }
        let kernel = ctx.kernel.filter(|_| !self.collisions_off);
        let proposed = drag(ctx.document, grab.body, grab.point, target);
        let moves = match kernel {
            None => Some(proposed),
            Some(kernel) => {
                let mut clear = |moves: &[(BodyId, BodyPlacement)]| {
                    collide::collides(ctx.document, kernel, moves, &mut grab.baseline)
                        .map(|hit| !hit)
                        .unwrap_or(true)
                };
                let from = glam::Vec3::from_array(grab.reached.unwrap_or(origin));
                let to = glam::Vec3::from_array(target);
                // The way there in steps short enough that nothing passes
                // through another body between two of them.
                let mut good = 0.0f32;
                let mut blocked = None;
                for t in collide::checkpoints(ctx.document, &proposed) {
                    let tried = if t >= 1.0 {
                        proposed.clone()
                    } else {
                        drag(
                            ctx.document,
                            grab.body,
                            grab.point,
                            from.lerp(to, t).to_array(),
                        )
                    };
                    if clear(&tried) {
                        good = t;
                    } else {
                        blocked = Some(t);
                        break;
                    }
                }
                if let Some(bad) = blocked {
                    // Between the last clear step and the first blocked one,
                    // halving, to the last pose that collides with nothing:
                    // the body stops at contact.
                    let (mut good, mut bad, mut best) = (good, bad, None);
                    for _ in 0..5 {
                        let mid = (good + bad) * 0.5;
                        let pull = from.lerp(to, mid).to_array();
                        let tried = drag(ctx.document, grab.body, grab.point, pull);
                        if clear(&tried) {
                            good = mid;
                            best = Some((tried, pull));
                        } else {
                            bad = mid;
                        }
                    }
                    if best.is_none() && good > 0.0 {
                        let pull = from.lerp(to, good).to_array();
                        best = Some((drag(ctx.document, grab.body, grab.point, pull), pull));
                    }
                    best.map(|(moves, pull)| {
                        grab.reached = Some(pull);
                        moves
                    })
                } else {
                    grab.reached = Some(target);
                    Some(proposed)
                }
            }
        };
        if let Some(moves) = moves {
            place_bodies(ctx.document, &moves);
        }
        InputResult::redraw_only()
    }

    /// The drag ends: the bodies stay where it left them, recorded as
    /// placements.
    pub(crate) fn let_go(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
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

    /// The body the Move tool moves: the selected one, else the one the
    /// tree has active.
    pub(crate) fn body_to_move(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        ctx.selected_body_id.map(BodyId).or_else(|| {
            ctx.active_document_object
                .and_then(|id| ctx.document.get_feature_meta(id))
                .and_then(|node| node.body)
        })
    }

    /// Whether a body's own joints place it: then moving it by hand is
    /// undone by the next solve.
    pub(crate) fn held_by_joints(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        joints(ctx.document).iter().any(|j| j.body == body)
    }
}

/// How far, in pixels, the mouse goes before a press is a drag.
const DRAG_PX: f32 = 4.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::joint;
    use crate::tests::flipped_view_proj;
    use crate::*;
    use core_document::{Document, Workbench};

    #[test]
    fn a_body_dragged_by_the_mouse_turns_on_its_hinge_and_is_recorded() {
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let door = doc.create_body(None);
        let pin = Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        };
        doc.add_feature_in_body(
            joint(
                JointTool::Hinge.joint(
                    &pin,
                    &Rigid::from(BodyPlacement::default()),
                    &pin,
                    &Rigid::from(BodyPlacement::default()),
                    0.0,
                ),
                pin,
                frame,
                pin,
            ),
            "Hinge 1".into(),
            Some(door),
        )
        .unwrap();
        let vp = flipped_view_proj(glam::Vec3::new(0.0, 0.0, 100.0), glam::Vec3::ZERO);
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
}
