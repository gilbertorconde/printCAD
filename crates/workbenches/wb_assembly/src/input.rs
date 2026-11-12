//! Pointer and key input, and the work each frame does.

use crate::{
    AssemblyWorkbench, COUPLING_KIND, JointKind, JointTool, MotionStudy, Picking, Stepping,
    Studying, Task, all_placements, commands, coupling, exploded, frames::explode, joints, states,
};
use core_document::{
    BodyId, HostRequest, InputResult, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

impl AssemblyWorkbench {
    /// An input's work, before what the task left is noted.
    pub(crate) fn input_turn(
        &mut self,
        event: &WorkbenchInputEvent,
        tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.settle_held(ctx);
        if let Some(result) = self.handle_input(event, ctx) {
            return result;
        }
        match event {
            WorkbenchInputEvent::ToolActivated => {
                self.drop_grab(ctx);
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.drop_grab(ctx) => return InputResult::consumed(),
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
                self.tab.task = None;
                self.tab.seen = None;
                self.tab.picking = Some(Picking::new(kind, None));
                // A face already selected is the first pick.
                self.take_pick(ctx);
            }
            Some("asm.interference") => {
                self.tab.picking = None;
                // A selected body is checked against the rest.
                let around = ctx.selected_body_id.map(BodyId);
                self.check_interference(ctx, around);
            }
            Some("asm.couple") => self.make_coupling(ctx),
            Some("asm.explode") => {
                self.tab.picking = None;
                let placements = all_placements(ctx);
                explode(ctx, &placements, 1.0);
                self.tab.task = Some(Task::Explode {
                    placements,
                    spread: 1.0,
                    steps: Box::new(Stepping {
                        shift: [0.0, 0.0, 20.0],
                        ..Stepping::default()
                    }),
                });
            }
            Some("asm.collisions") => {
                self.tab.collisions_off = !self.tab.collisions_off;
                ctx.log_info(if self.tab.collisions_off {
                    "Drags go through other bodies"
                } else {
                    "Drags stop where bodies would collide"
                });
            }
            Some("asm.parts") => {
                self.tab.picking = None;
                self.tab.task = Some(Task::Parts);
            }
            Some("asm.copy") => match Self::body_to_move(ctx) {
                Some(body) => {
                    self.tab.picking = None;
                    self.tab.task = Some(Task::Copies {
                        body,
                        count: 1,
                        step: commands::copy_step(ctx.document, body).to_array(),
                        around: None,
                        mirror: None,
                    });
                }
                None => ctx.log_warn("Select a body to copy"),
            },
            Some("asm.motion") => {
                self.tab.picking = None;
                self.tab.task = Some(Task::Motion(Box::new(Studying {
                    study: None,
                    draft: MotionStudy::default(),
                    frames: None,
                    frame: 0,
                    playing: false,
                    clock: 0.0,
                    placements: all_placements(ctx),
                    traces: Vec::new(),
                    tracing: false,
                })));
            }
            Some("asm.save_state") => {
                let name = commands::next_name(ctx.document, "State");
                match states::save(ctx.document, name.clone()) {
                    Ok(id) => {
                        ctx.record(
                            "asm.save_state",
                            commands::object(serde_json::json!({"name": name})),
                            serde_json::json!(id.0.to_string()),
                        );
                        ctx.log_info(format!(
                            "Saved {name}: double-click it in the tree to return to it"
                        ));
                        ctx.request(HostRequest::JournalLabel("Save assembly state".into()));
                    }
                    Err(why) => ctx.log_warn(format!("Could not save the state: {why}")),
                }
            }
            Some("asm.replace") => match Self::body_to_move(ctx) {
                Some(old) => {
                    self.tab.picking = None;
                    self.tab.group_seen = ctx.selected_body_id;
                    self.tab.task = Some(Task::Replace { old, new: None });
                }
                None => ctx.log_warn("Select the body to replace"),
            },
            Some("asm.group") => {
                self.tab.picking = None;
                self.tab.group_seen = ctx.selected_body_id;
                self.tab.task = Some(Task::Group {
                    editing: None,
                    members: ctx.selected_body_id.map(BodyId).into_iter().collect(),
                });
            }
            Some("asm.mass") => {
                self.tab.picking = None;
                self.measure_mass(ctx, commands::DEFAULT_DENSITY);
            }
            Some("asm.move") => match Self::body_to_move(ctx) {
                Some(body) => {
                    self.tab.picking = None;
                    self.tab.task = Some(Task::Move {
                        body,
                        placements: all_placements(ctx),
                        turns: coupling::turn_counts(ctx.document),
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
                if let Some(Ok(message)) = &self.tab.verdict {
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

    /// A frame's work, before what the task left is noted.
    pub(crate) fn frame_turn(&mut self, dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        self.settle_held(ctx);
        self.collect_interference(ctx);
        self.collect_mass(ctx);
        self.collect_sweep(ctx);
        let mut advanced = false;
        if let Some(Task::Motion(studying)) = &mut self.tab.task
            && studying.playing
            && let Some(count) = studying.frames.as_ref().map(Vec::len).filter(|n| *n > 0)
        {
            // Frames at the study's own pace, round again at the end.
            studying.clock += dt.min(0.1);
            let step = studying.draft.step.max(1e-3);
            while studying.clock >= step {
                studying.clock -= step;
                studying.frame = (studying.frame + 1) % count;
                advanced = true;
            }
        }
        if advanced {
            self.show_frame(ctx);
        }
        if let Some(Task::Explode { steps, .. }) = &mut self.tab.task
            && steps.playing
            && let Some(count) = steps
                .view
                .and_then(|id| exploded::view_of(ctx.document, id))
                .map(|v| v.steps.len() as f32)
                .filter(|n| *n > 0.0)
        {
            // A step a second, round again once through.
            steps.at += dt.min(0.1);
            if steps.at > count {
                steps.at = 0.0;
            }
            self.show_steps(ctx);
        }
        #[cfg(feature = "egui")]
        if let Some(Task::Joint { id, .. }) = self.tab.task {
            self.advance_play(ctx, id, f64::from(dt.min(0.1)));
        }
        if self.tab.picking.is_some() {
            self.take_pick(ctx);
            return;
        }
        self.take_group_pick(ctx);
        // A joint's settings stay open while it is selected; picking
        // something else closes them, keeping what was set. They open on a
        // double click (`edit_feature`), never on selection alone.
        if let Some(Task::Joint { id, .. }) = &self.tab.task
            && Self::selected_joint(ctx) != Some(*id)
        {
            self.tab.task = None;
        }
        if let Some(Task::Coupling { id, .. }) = &self.tab.task
            && Self::selected_of(ctx, COUPLING_KIND) != Some(*id)
        {
            self.tab.task = None;
        }
    }
}
