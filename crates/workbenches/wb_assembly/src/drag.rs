//! Moving bodies by hand: the Move task's handles, and a body taken by
//! the mouse and dragged as far as its joints let it.

use core_document::{
    BodyId, BodyPlacement, HostRequest, InputResult, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

use crate::joint::JointKind;
use crate::solve::{drag, draggable, joints, solve};
use crate::{AssemblyWorkbench, Grab, Task, all_placements, place_bodies};
use crate::{collide, commands, components, coupling, handles, restore_placements};

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
            turns: coupling::turn_counts(ctx.document),
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

    /// The held body let go of where it was taken, its bodies and the
    /// couplings' counts put back: whether a drag was under way.
    pub(crate) fn drop_grab(&mut self, ctx: &mut WorkbenchRuntimeContext) -> bool {
        match self.grab.take() {
            Some(grab) if grab.dragging => {
                restore_placements(ctx, &grab.placements);
                coupling::set_turn_counts(ctx.document, &grab.turns);
                true
            }
            _ => false,
        }
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
        let mut calls: Vec<serde_json::Value> = grab
            .placements
            .iter()
            .filter_map(|(body, before)| {
                let now = ctx.document.body_placement(*body);
                (!now.after(&before.inverse()).is_identity()).then(|| {
                    serde_json::json!({
                        "body": body.0.to_string(),
                        "translation": now.translation,
                        "rotation": now.rotation,
                    })
                })
            })
            .collect();
        // The couplings' counts the drag carried past a whole turn go
        // with the first placement, so a replay counts as the drag did.
        let turns: Vec<serde_json::Value> = coupling::turn_counts(ctx.document)
            .into_iter()
            .filter(|now| !grab.turns.contains(now))
            .map(|(id, turns)| serde_json::json!({"coupling": id.0.to_string(), "turns": turns}))
            .collect();
        if let Some(first) = calls.first_mut().filter(|_| !turns.is_empty()) {
            first["turns"] = serde_json::Value::Array(turns);
        }
        for call in calls {
            ctx.record("asm.place", commands::object(call), serde_json::Value::Null);
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
    use crate::coupling::{Coupling, Gearing};
    use crate::test_support::{context, hinge, joint};
    use crate::tests::flipped_view_proj;
    use crate::*;
    use core_document::history::OpJournal;
    use core_document::{Document, HookOutcome, KeyCode, MouseButton, Workbench};

    /// The mouse over a view from 100 mm up Z, pressing on `body` at a
    /// point and moving through points in the world.
    struct Hand {
        wb: AssemblyWorkbench,
        vp: [[f32; 4]; 4],
    }

    impl Hand {
        fn new() -> Self {
            Self {
                wb: AssemblyWorkbench::default(),
                vp: flipped_view_proj(glam::Vec3::new(0.0, 0.0, 100.0), glam::Vec3::ZERO),
            }
        }

        fn send(
            &mut self,
            doc: &mut Document,
            hovered: Option<(BodyId, [f32; 3])>,
            event: WorkbenchInputEvent,
        ) -> (InputResult, HookOutcome) {
            let mut ctx =
                WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
            ctx.view_proj = Some(self.vp);
            ctx.hovered_body_id = hovered.map(|(b, _)| b.0);
            ctx.hovered_world_pos = hovered.map(|(_, p)| p);
            let result = self.wb.on_input(&event, None, &mut ctx);
            (result, HookOutcome::take(&mut ctx))
        }

        fn screen(&self, p: [f32; 3]) -> (f32, f32) {
            core_document::runtime::world_to_viewport(self.vp, (0, 0, 800, 600), p).unwrap()
        }

        /// A press on `body` at `at`, then the mouse through `path`.
        fn drag(&mut self, doc: &mut Document, body: BodyId, at: [f32; 3], path: &[[f32; 3]]) {
            let viewport_pos = self.screen(at);
            let press = WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            };
            let (result, _) = self.send(doc, Some((body, at)), press);
            assert!(!result.consumed, "the host still sees the press");
            for p in path {
                let viewport_pos = self.screen(*p);
                self.send(doc, None, WorkbenchInputEvent::MouseMove { viewport_pos });
            }
        }

        fn release(&mut self, doc: &mut Document) -> HookOutcome {
            let viewport_pos = (0.0, 0.0);
            let event = WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos,
            };
            self.send(doc, None, event).1
        }

        fn escape(&mut self, doc: &mut Document) -> bool {
            let key = KeyCode::Escape;
            self.send(doc, None, WorkbenchInputEvent::KeyPress { key })
                .0
                .consumed
        }
    }

    /// Points at 10 mm round Z, every `step` degrees up to `steps` of them.
    fn round(step: f32, steps: usize) -> Vec<[f32; 3]> {
        (1..=steps)
            .map(|i| (i as f32 * step).to_radians())
            .map(|t| [10.0 * t.cos(), 10.0 * t.sin(), 0.0])
            .collect()
    }

    fn quarter_turned(doc: &Document, body: BodyId) -> bool {
        let x = doc.body_placement(body).direction([1.0, 0.0, 0.0]);
        (x[1].atan2(x[0]).to_degrees() - 90.0).abs() < 2.0
    }

    #[test]
    fn a_body_dragged_by_the_mouse_turns_on_its_hinge_and_is_recorded() {
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let door = doc.create_body(None);
        let pin = Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        };
        let rest = Rigid::from(BodyPlacement::default());
        let kind = JointTool::Hinge.joint(&pin, &rest, &pin, &rest, 0.0);
        let hinge = joint(kind, pin, frame, pin);
        doc.add_feature_in_body(hinge, "Hinge 1".into(), Some(door))
            .unwrap();
        let mut hand = Hand::new();
        hand.drag(&mut doc, door, [10.0, 0.0, 0.0], &round(9.0, 10));
        let outcome = hand.release(&mut doc);
        assert!(quarter_turned(&doc, door));
        assert!(doc.body_placement(door).translation[2].abs() < 1e-3);
        assert_eq!(outcome.recorded.len(), 1);
        assert_eq!(outcome.recorded[0].id, "asm.place");
        let label = HostRequest::JournalLabel("Drag body".into());
        assert!(outcome.requests.contains(&label));
    }

    /// A grounded frame with two gears on hinges 30 mm apart, tied 1:1.
    fn gears() -> (Document, BodyId, FeatureId) {
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let first = doc.create_body(None);
        let second = doc.create_body(None);
        let apart = glam::Vec3::new(30.0, 0.0, 0.0);
        doc.set_body_placement(second, BodyPlacement::new(glam::Quat::IDENTITY, apart));
        let ground = Anchor::Plane {
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        };
        let grounded = joint(JointKind::Ground, ground, frame, ground);
        doc.add_feature_in_body(grounded, "Ground".into(), Some(frame))
            .unwrap();
        let pin = |point| Anchor::Axis {
            point,
            direction: [0.0, 0.0, 1.0],
        };
        let hinges: Vec<FeatureId> = [(first, [0.0; 3]), (second, apart.to_array())]
            .into_iter()
            .map(|(body, at)| {
                let held = joint(hinge(0.0), pin([0.0; 3]), frame, pin(at));
                doc.add_feature_in_body(held, "Hinge".into(), Some(body))
                    .unwrap()
            })
            .collect();
        let all = joints(&doc);
        let find = |id| all.iter().find(|j| j.id == id).unwrap();
        let starts = doc
            .bodies()
            .iter()
            .map(|b| (b.id, Rigid::from(b.placement)))
            .collect();
        let coupling = Coupling::new(
            Gearing::Gears,
            find(hinges[0]),
            find(hinges[1]),
            1.0,
            false,
            &starts,
        )
        .unwrap();
        let id = doc
            .add_feature_in_body(coupling, "Gears".into(), Some(second))
            .unwrap();
        (doc, first, id)
    }

    fn turns(doc: &Document, coupling: FeatureId) -> i64 {
        crate::coupling::turn_counts(doc)
            .into_iter()
            .find(|(id, _)| *id == coupling)
            .map_or(0, |(_, n)| n)
    }

    fn placements(doc: &Document) -> Vec<(BodyId, BodyPlacement)> {
        doc.bodies().iter().map(|b| (b.id, b.placement)).collect()
    }

    fn same_places(a: &Document, b: &Document) -> bool {
        placements(a)
            .iter()
            .zip(placements(b))
            .all(|((_, p), (_, q))| {
                (p.offset() - q.offset()).length() < 1e-3
                    && p.quat().dot(q.quat()).abs() > 1.0 - 1e-6
            })
    }

    /// Three quarters of a turn of the driving gear carries it past the
    /// hinge's wrap at half a turn: Escape puts the bodies and the count
    /// back; a release records what a replay needs to agree with the
    /// drag, and an undo takes back the whole gesture.
    #[test]
    fn a_drag_past_the_wrap_cancels_replays_and_undoes_with_its_turn_count() {
        let (start, gear, coupling) = gears();
        let (mut doc, mut hand) = (start.clone(), Hand::new());
        hand.drag(&mut doc, gear, [10.0, 0.0, 0.0], &round(27.0, 10));
        assert_eq!(turns(&doc, coupling), 1, "the drag counted a whole turn");
        assert!(hand.escape(&mut doc), "Escape takes the drag");
        assert!(same_places(&doc, &start) && turns(&doc, coupling) == 0);
        assert!(!hand.escape(&mut doc), "nothing is held after");

        let mut journal = OpJournal::new(8);
        journal.note(&mut doc);
        hand.drag(&mut doc, gear, [10.0, 0.0, 0.0], &round(27.0, 10));
        let recorded = hand.release(&mut doc).recorded;
        journal.note(&mut doc);
        let mut replayed = start.clone();
        for call in &recorded {
            let mut ctx = context(&mut replayed);
            AssemblyWorkbench::default()
                .run_command(&call.id, &call.args, &mut ctx)
                .unwrap();
        }
        assert!(same_places(&replayed, &doc), "the replay puts every body");
        assert_eq!(turns(&replayed, coupling), 1, "and counts the turn");
        journal.undo(&mut doc).expect("the drag undoes");
        assert!(same_places(&doc, &start) && turns(&doc, coupling) == 0);
    }

    /// A body of a rigid component dragged free takes the others along;
    /// one undo puts every member back.
    #[test]
    fn a_rigid_component_s_members_drag_and_undo_together() {
        let mut doc = Document::new("t");
        let (lid, knob) = (doc.create_body(None), doc.create_body(None));
        let component = doc.create_component("Lid".into(), None).unwrap();
        for body in [lid, knob] {
            doc.set_body_component(body, Some(component)).unwrap();
        }
        let start = doc.clone();
        let mut journal = OpJournal::new(8);
        journal.note(&mut doc);
        let mut hand = Hand::new();
        hand.drag(
            &mut doc,
            lid,
            [0.0; 3],
            &[[5.0, 0.0, 0.0], [10.0, 5.0, 0.0]],
        );
        hand.release(&mut doc);
        journal.note(&mut doc);
        for body in [lid, knob] {
            let at = doc.body_placement(body).offset();
            assert!(
                (at - glam::Vec3::new(10.0, 5.0, 0.0)).length() < 1e-2,
                "{at:?}"
            );
        }
        journal.undo(&mut doc).expect("the drag undoes");
        assert!(same_places(&doc, &start), "both members are back");
    }

    /// A drag held in one tab stays with it: another tab's Escape finds
    /// nothing held, and the first tab's puts its own bodies back.
    #[test]
    fn a_held_drag_stays_with_its_tab() {
        let (start, gear, _) = gears();
        let (mut first, mut second) = (start.clone(), start.clone());
        let mut hand = Hand::new();
        hand.drag(&mut first, gear, [10.0, 0.0, 0.0], &round(30.0, 3));
        let kept = hand.wb.suspend_session();
        hand.wb.resume_session(None);
        assert!(!hand.escape(&mut second), "the other tab holds nothing");
        assert!(same_places(&second, &start));
        hand.wb.suspend_session();
        hand.wb.resume_session(kept);
        assert!(hand.escape(&mut first), "the drag is back with its tab");
        assert!(same_places(&first, &start));
    }

    /// Taking up a tool lets go of a held body, its drag put back.
    #[test]
    fn a_tool_taken_up_lets_go_of_the_held_body() {
        let (start, gear, _) = gears();
        let (mut doc, mut hand) = (start.clone(), Hand::new());
        hand.drag(&mut doc, gear, [10.0, 0.0, 0.0], &round(30.0, 3));
        assert!(!same_places(&doc, &start), "the drag moved the gears");
        let mut ctx = context(&mut doc);
        crate::test_support::activate(&mut hand.wb, &mut ctx, "asm.parts");
        assert!(same_places(&doc, &start));
        assert!(hand.release(&mut doc).recorded.is_empty());
    }
}
