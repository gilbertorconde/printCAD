//! A joint's panel: its bodies, kind and faces, its drive and the sweep
//! that plays it, the check of its motion for collisions, and the edits
//! each field makes.

use super::*;

/// What a field of a joint's panel asks.
pub(super) enum JointEdit {
    /// Another kind picked, by its place in `JointTool::ALL`.
    Kind(usize),
    Repick,
    /// A number `asm.set` takes by this word.
    Number(&'static str, f32),
    Flip(bool),
    /// A fixed joint's shift along one of the other body's axes.
    Shift(usize, f32),
    /// A field of the drive whose fields' ids begin with this word.
    Drive(&'static str, DriveEdit),
    CheckSweep,
    StopSweep,
    SelectBody(BodyId),
    TurnBy(f32),
    Turn,
    TurnOver,
    Turn90,
    SelectJoint(FeatureId),
    Delete,
}

/// What a field of a drive asks.
pub(super) enum DriveEdit {
    /// Held at a value, or let go.
    Drive(bool),
    To(f32),
    /// Kept within limits, or not.
    Limits(bool),
    /// One end of the limits: 0 the lowest, 1 the highest.
    End(usize, f32),
    Record,
    Play,
}

/// Frames in a recorded sweep: there and back in four seconds.
const SWEEP_FRAMES: usize = 60;

/// Where a joint's motion stands: a hinge's angle or a slider's position,
/// and an alignment's turn and slide.
fn joint_now(
    ctx: &WorkbenchRuntimeContext,
    body: Option<BodyId>,
    joint: &JointFeature,
) -> (Option<f64>, Option<(f64, f64)>) {
    let placed = |b: BodyId| -> crate::Rigid { ctx.document.body_placement(b).into() };
    let other = placed(joint.other_body);
    (
        body.and_then(|b| joint.travel(&placed(b), &other)),
        body.and_then(|b| joint.align_travel(&placed(b), &other)),
    )
}

/// The range a drive sweeps through: its limits, or a whole turn, or
/// 25 mm either side of `centre`.
fn sweep_range(limits: Option<[f32; 2]>, angular: bool, centre: f64) -> (f64, f64) {
    match limits {
        Some([low, high]) => (f64::from(low), f64::from(high)),
        None if angular => (-179.0, 179.0),
        None => (centre - 25.0, centre + 25.0),
    }
}

/// The range a joint's motion is checked for collisions through: its
/// limits, or a whole turn, or 25 mm either side of where it stands.
fn motion_range(limits: Option<[f32; 2]>, hinge: bool, now: Option<f64>) -> (f32, f32) {
    match limits {
        Some([low, high]) => (low, high),
        None if hinge => (-180.0, 180.0),
        None => {
            let at = now.unwrap_or(0.0) as f32;
            (at - 25.0, at + 25.0)
        }
    }
}

impl AssemblyWorkbench {
    /// End a sweep, the drive back at the value it started from.
    fn stop_playing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(play) = self.tab.playing.take() else {
            return;
        };
        let Some(node) = ctx.document.get_feature_meta(play.joint) else {
            return;
        };
        let Ok(mut joint) = JointFeature::from_json(&node.data) else {
            return;
        };
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = &mut joint.kind {
            drive.to = drive.to.map(|_| play.start);
        }
        if let Err(why) = ctx
            .document
            .update_feature_data(play.joint, joint.to_json())
        {
            ctx.log_warn(why.to_string());
        }
        ctx.document.clear_feature_dirty(play.joint);
        self.solve_and_apply(ctx);
    }

    pub(super) fn joint_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        let created = before.is_none();
        // Cancel's edits, the joint and the placements its solves set,
        // are taken back by the host.
        if request.cancel {
            self.tab.playing = None;
            if created {
                ctx.active_document_object = None;
            }
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.stop_playing(ctx);
            crate::commands::record_joint(ctx, id, before.as_ref(), placements);
            self.tab.task = None;
            ctx.active_document_object = None;
            return TaskOutcome::Accepted {
                label: if created { "Add joint" } else { "Edit joint" }.to_string(),
            };
        }
        if ctx.document.get_feature_meta(id).is_none() {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_sweep(ctx);
        TaskOutcome::Open
    }

    /// Move a swept drive on by `dt` seconds, back and forth through its
    /// range every four seconds. A formula holding the drive's value stops
    /// the sweep, which would fight it every frame.
    pub(crate) fn advance_play(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        dt: f64,
    ) {
        let Some(joint) = ctx
            .document
            .get_feature_data(id)
            .and_then(|d| JointFeature::from_json(d).ok())
        else {
            return;
        };
        let (drive, base) = match &joint.kind {
            JointKind::Hinge { drive, .. } => (drive, "/kind/Hinge/drive"),
            JointKind::Slider { drive, .. } => (drive, "/kind/Slider/drive"),
            _ => return,
        };
        if drive.to.is_none() {
            return;
        }
        if ctx
            .document
            .feature_formula(id, &format!("{base}/to"))
            .is_some()
        {
            self.tab.playing = self.tab.playing.filter(|p| p.joint != id);
            return;
        }
        let Some(play) = self.tab.playing.as_mut().filter(|p| p.joint == id) else {
            return;
        };
        let angular = matches!(joint.kind, JointKind::Hinge { .. });
        let (low, high) = sweep_range(drive.limits, angular, f64::from(play.start));
        play.phase += dt * std::f64::consts::TAU / 4.0;
        let to = (low + (high - low) * (0.5 - 0.5 * play.phase.cos())) as f32;
        let _ = self.call(
            ctx,
            "asm.set",
            serde_json::json!({"joint": id.0.to_string(), "drive": to}),
        );
    }

    /// A joint's settings: its bodies, its kind and faces, what it holds
    /// and how its motion is driven or limited, the check of that motion
    /// for collisions, a turn of its body, then what the last solve said
    /// and the joint's Delete.
    pub(super) fn joint_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: FeatureId,
        form: &mut Form<'_>,
    ) -> Option<JointEdit> {
        let node = ctx.document.get_feature_meta(id)?;
        let Ok(joint) = JointFeature::from_json(&node.data) else {
            form.push(w::note(
                NoteKind::Error,
                Some("Unreadable joint"),
                "The stored joint does not parse.",
            ));
            return None;
        };
        let mut edit = None;
        let (now, align_now) = joint_now(ctx, node.body, &joint);
        /// A number of the joint's that `asm.set` takes by its field's
        /// id, `key`, a formula setting it at `path`.
        fn bound(
            form: &mut Form<'_>,
            id: FeatureId,
            key: &'static str,
            (label, value): (&str, f32),
            path: &str,
            hint: &str,
        ) -> Option<JointEdit> {
            let dim = if key == "degrees" {
                Dim::Angle
            } else {
                Dim::Length
            };
            let (field, typed) = f::bound(form, key, label, value, dim, (id, path));
            form.push(w::hinted(hint, field));
            typed.map(|value| JointEdit::Number(key, value as f32))
        }
        form.push(w::value(
            "Moves",
            node.body.map(|b| body_name(ctx, b)).unwrap_or_default(),
        ));
        form.push(w::value("Against", body_name(ctx, joint.other_body)));
        if let Some(current) = JointTool::of_kind(&joint.kind) {
            let (kind, picked) = f::select(
                form,
                "kind",
                bench_api::Select {
                    label: "Kind".into(),
                    options: JointTool::ALL
                        .iter()
                        .map(|t| t.label().to_string())
                        .collect(),
                    selected: JointTool::ALL
                        .iter()
                        .position(|t| *t == current)
                        .unwrap_or(0),
                    ..Default::default()
                },
            );
            form.push(kind);
            if let Some(index) = picked {
                edit = Some(JointEdit::Kind(index));
            }
            let (repick, pressed) =
                f::button(form, "repick", "Pick faces again", ButtonStyle::Secondary);
            form.push(w::hinted(
                "Pick the two faces afresh; the joint keeps its name and kind",
                repick,
            ));
            if pressed {
                edit = Some(JointEdit::Repick);
            }
        }
        match &joint.kind {
            JointKind::Mate { flip, offset } => {
                edit = bound(
                    form,
                    id,
                    "offset",
                    ("Gap", *offset),
                    "/kind/Mate/offset",
                    "How far apart the two faces sit",
                )
                .or(edit);
                let (same_way, on) = f::toggle(form, "flip", "Same way", *flip);
                form.push(w::hinted(
                    "The faces point the same way instead of at each other",
                    same_way,
                ));
                if let Some(on) = on {
                    edit = Some(JointEdit::Flip(on));
                }
            }
            JointKind::Angle { degrees } => {
                edit = bound(
                    form,
                    id,
                    "degrees",
                    ("Angle", *degrees),
                    "/kind/Angle/degrees",
                    "Between the faces' outward normals: 180 faces them at each other",
                )
                .or(edit);
                form.push(w::text(
                    "Only the turn is held: pair it with a mate or an alignment to \
                     say where the body sits.",
                ));
            }
            JointKind::Ground => form.push(w::text(
                "The body stays where it is; the bodies joined to it are \
                 placed against it.",
            )),
            JointKind::Align { turn, slide, .. } => {
                let (turn_now, slide_now) = align_now.unzip();
                for (title, prefix, drive, angular, now) in [
                    ("Turn", "turn", turn, true, turn_now),
                    ("Slide", "slide", slide, false, slide_now),
                ] {
                    let base = if angular {
                        "/kind/Align/turn"
                    } else {
                        "/kind/Align/slide"
                    };
                    let asked = form.nest(
                        |form| {
                            self.drive_walk(
                                ctx,
                                id,
                                (prefix, base),
                                (drive, now),
                                (angular, false),
                                form,
                            )
                        },
                        |children| w::section(title, children),
                    );
                    if let Some(asked) = asked {
                        edit = Some(JointEdit::Drive(prefix, asked));
                    }
                }
                form.push(w::text(
                    "The body can turn about the axis and slide along it, each \
                     free, held or kept within limits.",
                ));
            }
            JointKind::Hinge { offset, drive, .. } => {
                edit = bound(
                    form,
                    id,
                    "offset",
                    ("Height", *offset),
                    "/kind/Hinge/offset",
                    "How far along the axis the body sits from the other",
                )
                .or(edit);
                let base = "/kind/Hinge/drive";
                if let Some(asked) =
                    self.drive_walk(ctx, id, ("drive", base), (drive, now), (true, true), form)
                {
                    edit = Some(JointEdit::Drive("drive", asked));
                }
                form.push(w::text(
                    "The body can only turn about the axis. Its angle counts from \
                     where it sat when the joint was made.",
                ));
            }
            JointKind::Distance { offset } => {
                edit = bound(
                    form,
                    id,
                    "offset",
                    ("Distance", *offset),
                    "/kind/Distance/offset",
                    "Along the other face's normal",
                )
                .or(edit);
                form.push(w::text(
                    "Only the distance is held: the faces may turn and slide past \
                     each other.",
                ));
            }
            JointKind::Tangent { radius } => {
                edit = bound(
                    form,
                    id,
                    "radius",
                    ("Radius", *radius),
                    "/kind/Tangent/radius",
                    "The round face's radius",
                )
                .or(edit);
                form.push(w::text(
                    "The round face rests on the flat one; it can still roll and \
                     slide along it.",
                ));
            }
            JointKind::Slider { drive, .. } => {
                let base = "/kind/Slider/drive";
                if let Some(asked) =
                    self.drive_walk(ctx, id, ("drive", base), (drive, now), (false, true), form)
                {
                    edit = Some(JointEdit::Drive("drive", asked));
                }
                form.push(w::text(
                    "The body can only slide along the axis, turned as it was when \
                     the joint was made.",
                ));
            }
            JointKind::Fixed { shift, .. } => {
                for (k, label) in ["Shift x", "Shift y", "Shift z"].into_iter().enumerate() {
                    let (field, typed) = f::bound(
                        form,
                        &format!("shift_{k}"),
                        label,
                        shift[k] as f32,
                        Dim::Length,
                        (id, &format!("/kind/Fixed/shift/{k}")),
                    );
                    form.push(w::hinted(
                        "Where the body sits from the other, along the other's own axes",
                        field,
                    ));
                    if let Some(value) = typed {
                        edit = Some(JointEdit::Shift(k, value as f32));
                    }
                }
                form.push(w::text(
                    "The body is held to the other as it sat when the joint was made; \
                     it moves only with it.",
                ));
            }
            JointKind::Parallel => form.push(w::text(
                "Only the turn is held, the faces parallel: pair it with other joints \
                 to say where the body sits.",
            )),
            JointKind::Perpendicular => form.push(w::text(
                "Only the turn is held, the faces square: pair it with other joints \
                 to say where the body sits.",
            )),
            JointKind::Ball => form.push(w::text(
                "The two points are held as one; the body can still turn every way \
                 about them.",
            )),
            JointKind::Universal => form.push(w::text(
                "The pins cross at one point, square to each other: the body turns \
                 about either pin.",
            )),
            JointKind::Slot => form.push(w::text(
                "The pin stays on the slot's line: it slides along it and turns every way.",
            )),
            JointKind::Width => form.push(w::text(
                "The tab's two faces stay centred between the slot's two walls: it \
                 slides along the slot and turns in it.",
            )),
            JointKind::Path => form.push(w::text(
                "The point stays on the edge, whatever its shape: it runs along it and \
                 turns every way.",
            )),
            JointKind::Cam { radius } => {
                edit = bound(
                    form,
                    id,
                    "radius",
                    ("Radius", *radius),
                    "/kind/Cam/radius",
                    "The follower's roller radius; 0 for a point",
                )
                .or(edit);
                form.push(w::text(
                    "The follower stays on the cam's face, a radius off it.",
                ));
            }
        }
        if joint.kind != JointKind::Ground {
            for (end, (key, label)) in [("moving_end", "Moving end"), ("fixed_end", "Fixed end")]
                .into_iter()
                .enumerate()
            {
                edit = bound(
                    form,
                    id,
                    key,
                    (label, joint.ends[end]),
                    &format!("/ends/{end}"),
                    "How far this end moves along its own normal or axis before the \
                         joint holds it",
                )
                .or(edit);
            }
        }
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = joint.kind {
            let hinge = matches!(joint.kind, JointKind::Hinge { .. });
            let range = motion_range(drive.limits, hinge, now);
            let unit = if hinge { "°" } else { " mm" };
            if let Some(asked) = self.sweep_check_walk(ctx, id, range, unit, form) {
                edit = Some(asked);
            }
        }
        if joint.kind != JointKind::Ground {
            let by = self.tab.turn_by.unwrap_or(90.0);
            let (by, typed) = f::number(form, "turn_by", "", by, Dim::Angle);
            if let Some(value) = typed {
                edit = Some(JointEdit::TurnBy(value as f32));
            }
            let (turn, pressed) = f::button(form, "turn", "Turn", ButtonStyle::Secondary);
            if pressed {
                edit = Some(JointEdit::Turn);
            }
            let (over, pressed) = f::button(form, "turn_over", "Turn over", ButtonStyle::Secondary);
            if pressed {
                edit = Some(JointEdit::TurnOver);
            }
            form.push(w::row(vec![
                by,
                w::hinted(
                    "Turn the body about the joint's axis or normal by this much",
                    turn,
                ),
                w::hinted(
                    "Half a turn across the joint: the body the other way round",
                    over,
                ),
            ]));
        }
        if joint.kind.keeps_a_turn() {
            let (turn90, pressed) = f::button(form, "turn90", "Turn 90°", ButtonStyle::Secondary);
            form.push(w::hinted(
                "Turn what the joint counts its turn from a quarter turn: the body \
                 follows, and the joint reads as it did",
                turn90,
            ));
            if pressed {
                edit = Some(JointEdit::Turn90);
            }
        }
        if let Some(note) = self.verdict_note() {
            form.push(note);
        }
        if self.redundant_now(ctx).iter().any(|(j, _)| *j == id) {
            form.push(w::note(
                NoteKind::Warning,
                Some("Redundant"),
                "The body's other joints already hold everything this one does; it could go.",
            ));
        }
        if let Some(joint) = self.solver_messages(ctx, form) {
            edit = Some(JointEdit::SelectJoint(joint));
        }
        if let Some(text) = node.body.and_then(|body| freedom_text(ctx, body)) {
            form.push(text);
        }
        let (delete, pressed) = f::button(form, "delete", "Delete joint", ButtonStyle::Destructive);
        form.push(w::hinted(
            "Remove the joint; the bodies stay where they are",
            delete,
        ));
        if pressed {
            edit = Some(JointEdit::Delete);
        }
        edit
    }

    /// A hinge's or a slider's drive, or an alignment's turn or slide:
    /// where it stands, held at a value, kept within limits and, while
    /// held, swept through its range to show the motion or record it.
    /// The fields' ids begin with `prefix`; `base` is the drive's place in
    /// the joint's data.
    fn drive_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        joint: FeatureId,
        (prefix, base): (&str, &str),
        (drive, now): (&crate::Drive, Option<f64>),
        (angular, sweep): (bool, bool),
        form: &mut Form<'_>,
    ) -> Option<DriveEdit> {
        let (dim, unit) = if angular {
            (Dim::Angle, "°")
        } else {
            (Dim::Length, " mm")
        };
        let mut edit = None;
        if let Some(now) = now {
            form.push(w::value(
                if angular { "Angle now" } else { "Position now" },
                format!("{now:.2}{unit}"),
            ));
        }
        let (driven, on) = f::toggle(
            form,
            &format!("{prefix}.drive"),
            "Drive",
            drive.to.is_some(),
        );
        form.push(w::hinted(
            if angular {
                "Hold the hinge at an angle"
            } else {
                "Hold the slider at a position"
            },
            driven,
        ));
        if let Some(on) = on {
            edit = Some(DriveEdit::Drive(on));
        }
        if let Some(to) = drive.to {
            // A drag stays within the limits, when there are any.
            let [low, high] = drive.limits.map_or([None, None], |l| l.map(Some));
            let (field, typed) = f::bound_within(
                form,
                &format!("{prefix}.to"),
                if angular { "Angle" } else { "Position" },
                to,
                dim,
                (joint, &format!("{base}/to")),
                (low, high),
            );
            form.push(w::hinted("Where the drive holds it", field));
            if let Some(value) = typed {
                edit = Some(DriveEdit::To(value as f32));
            }
        }
        let (limited, on) = f::toggle(
            form,
            &format!("{prefix}.limits"),
            "Limits",
            drive.limits.is_some(),
        );
        form.push(w::hinted(
            "Keep the motion within a range while it is not driven",
            limited,
        ));
        if let Some(on) = on {
            edit = Some(DriveEdit::Limits(on));
        }
        if let Some([low, high]) = drive.limits {
            // Dragged, an end stops at the other.
            for (end, value, label, stops) in [
                (0, low, "Lowest", (None, Some(high))),
                (1, high, "Highest", (Some(low), None)),
            ] {
                let (field, typed) = f::bound_within(
                    form,
                    &format!("{prefix}.{}", ["low", "high"][end]),
                    label,
                    value,
                    dim,
                    (joint, &format!("{base}/limits/{end}")),
                    stops,
                );
                form.push(w::hinted("An end of the range the motion stays in", field));
                if let Some(value) = typed {
                    edit = Some(DriveEdit::End(end, value as f32));
                }
            }
        }
        if drive.to.is_none() || !sweep {
            return edit;
        }
        let (record, pressed) = f::button(
            form,
            &format!("{prefix}.record"),
            "Record",
            ButtonStyle::Secondary,
        );
        form.push(w::hinted(
            "Save the sweep through its range as an animation",
            record,
        ));
        if pressed {
            edit = Some(DriveEdit::Record);
        }
        if ctx
            .document
            .feature_formula(joint, &format!("{base}/to"))
            .is_none()
        {
            let mine = self.tab.playing.is_some_and(|p| p.joint == joint);
            let (play, pressed) = f::button(
                form,
                &format!("{prefix}.play"),
                if mine { "Stop" } else { "Play" },
                ButtonStyle::Secondary,
            );
            form.push(w::hinted(
                "Sweep the drive through its range; stopping puts it back",
                play,
            ));
            if pressed {
                edit = Some(DriveEdit::Play);
            }
        }
        edit
    }

    /// The check of a joint's motion for collisions: its button, its
    /// progress, what it found.
    fn sweep_check_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: FeatureId,
        (low, high): (f32, f32),
        unit: &str,
        form: &mut Form<'_>,
    ) -> Option<JointEdit> {
        if let Some((joint, done, total)) = self.sweep_progress()
            && joint == id
        {
            form.push(Widget::Progress {
                label: format!("Checking the motion: {done} of {total} pairs"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            let (stop, pressed) = f::button(form, "stop_sweep", "Stop", ButtonStyle::Secondary);
            form.push(stop);
            return pressed.then_some(JointEdit::StopSweep);
        }
        let mut edit = None;
        let (check, pressed) = f::button(
            form,
            "check_sweep",
            "Check collisions through the motion",
            ButtonStyle::Secondary,
        );
        form.push(w::hinted(
            format!(
                "Step the drive from {low:.1}{unit} to {high:.1}{unit} and look for \
                 bodies that share material on the way"
            ),
            check,
        ));
        if pressed {
            edit = Some(JointEdit::CheckSweep);
        }
        match &self.tab.motion_clashes {
            Some((joint, Ok(found))) if *joint == id && found.is_empty() => form.push(w::note(
                NoteKind::Success,
                None,
                "No collisions through the motion",
            )),
            Some((joint, Ok(found))) if *joint == id => {
                form.push(w::note(
                    NoteKind::Error,
                    Some(&format!(
                        "{} collision{}",
                        found.len(),
                        if found.len() == 1 { "" } else { "s" }
                    )),
                    "Click one to select its first body",
                ));
                let (list, event) = f::raw(form, "motion_clashes");
                if let Some(clash) = f::picked(event).and_then(|i| found.get(i)) {
                    edit = Some(JointEdit::SelectBody(clash.a));
                }
                form.push(Widget::List {
                    id: list,
                    items: found
                        .iter()
                        .map(|clash| bench_api::ListItem {
                            label: format!(
                                "At {:.1}{unit}: {} and {}, {:.2} mm³",
                                clash.at,
                                body_name(ctx, clash.a),
                                body_name(ctx, clash.b),
                                clash.volume_mm3
                            ),
                            detail: None,
                            icon: None,
                        })
                        .collect(),
                    selected: None,
                });
            }
            Some((joint, Err(why))) if *joint == id => {
                form.push(w::note(NoteKind::Error, Some("Not checked"), why.clone()))
            }
            _ => {}
        }
        edit
    }

    /// A change to a joint's settings, made as `asm.set` makes it; its
    /// kind or faces changed, its motion swept, recorded or checked, its
    /// body turned, or the joint deleted.
    pub(super) fn joint_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        (id, created, placements): (FeatureId, bool, &[(BodyId, BodyPlacement)]),
        edit: JointEdit,
    ) -> Option<TaskOutcome> {
        let node = ctx.document.get_feature_meta(id)?.clone();
        let mut joint = JointFeature::from_json(&node.data).ok()?;
        let (now, align_now) = joint_now(ctx, node.body, &joint);
        let set = |this: &mut Self, ctx: &mut WorkbenchRuntimeContext, args: serde_json::Value| {
            let mut args = args;
            args["joint"] = serde_json::json!(id.0.to_string());
            let _ = this.call(ctx, "asm.set", args);
        };
        // A drive's or a limit's field: which drive, and the joint's word
        // for its value and its range.
        let drive_of = |joint: &JointFeature, prefix: &str| match (&joint.kind, prefix) {
            (JointKind::Hinge { drive, .. }, "drive") => {
                Some((*drive, "/kind/Hinge/drive", "drive", "limits", true, now))
            }
            (JointKind::Slider { drive, .. }, "drive") => {
                Some((*drive, "/kind/Slider/drive", "drive", "limits", false, now))
            }
            (JointKind::Align { turn, .. }, "turn") => Some((
                *turn,
                "/kind/Align/turn",
                "turn_drive",
                "turn_limits",
                true,
                align_now.map(|a| a.0),
            )),
            (JointKind::Align { slide, .. }, "slide") => Some((
                *slide,
                "/kind/Align/slide",
                "slide_drive",
                "slide_limits",
                false,
                align_now.map(|a| a.1),
            )),
            _ => None,
        };
        match edit {
            JointEdit::Drive(prefix, asked) => {
                let (drive, base, to_arg, range_arg, angular, now) = drive_of(&joint, prefix)?;
                match asked {
                    DriveEdit::Drive(true) => {
                        set(
                            self,
                            ctx,
                            serde_json::json!({to_arg: now.unwrap_or(0.0) as f32}),
                        );
                    }
                    DriveEdit::Drive(false) => {
                        self.clear_formula(ctx, id, &format!("{base}/to"));
                        self.tab.playing = None;
                        set(self, ctx, serde_json::json!({to_arg: false}));
                    }
                    DriveEdit::To(value) => set(self, ctx, serde_json::json!({to_arg: value})),
                    DriveEdit::Limits(true) => {
                        let at = now.unwrap_or(0.0) as f32;
                        let range = if angular {
                            [(at - 45.0).max(-180.0), (at + 45.0).min(180.0)]
                        } else {
                            [at - 10.0, at + 10.0]
                        };
                        set(self, ctx, serde_json::json!({range_arg: range}));
                    }
                    DriveEdit::Limits(false) => {
                        for end in 0..2 {
                            self.clear_formula(ctx, id, &format!("{base}/limits/{end}"));
                        }
                        set(self, ctx, serde_json::json!({range_arg: false}));
                    }
                    DriveEdit::End(end, value) => {
                        let mut range = drive.limits?;
                        range[end] = value;
                        let [mut low, mut high] = range;
                        if low > high {
                            std::mem::swap(&mut low, &mut high);
                        }
                        set(self, ctx, serde_json::json!({range_arg: [low, high]}));
                    }
                    DriveEdit::Record => {
                        let to = drive.to?;
                        let mine = self.tab.playing.filter(|p| p.joint == id);
                        let centre = f64::from(mine.map_or(to, |p| p.start));
                        let (low, high) = sweep_range(drive.limits, angular, centre);
                        let frames = crate::sweep_frames(
                            ctx.document,
                            id,
                            low as f32,
                            high as f32,
                            SWEEP_FRAMES,
                        );
                        if frames.is_empty() {
                            ctx.log_warn("Nothing moves through this joint's range");
                        } else {
                            ctx.request(core_document::HostRequest::RecordAnimation {
                                name: node.name.clone(),
                                frames,
                                frame_ms: 4000 / SWEEP_FRAMES as u32,
                            });
                        }
                    }
                    DriveEdit::Play => {
                        let to = drive.to?;
                        let back = match self.tab.playing.filter(|p| p.joint == id) {
                            Some(p) => {
                                self.tab.playing = None;
                                p.start
                            }
                            None => {
                                let (low, high) = sweep_range(drive.limits, angular, f64::from(to));
                                let span = (high - low).max(1e-6);
                                let from = ((f64::from(to) - low) / span).clamp(0.0, 1.0);
                                self.tab.playing = Some(crate::Play {
                                    joint: id,
                                    start: to,
                                    phase: (1.0 - 2.0 * from).acos(),
                                });
                                to
                            }
                        };
                        set(self, ctx, serde_json::json!({to_arg: back}));
                    }
                }
            }
            JointEdit::Kind(index) => {
                let current = JointTool::of_kind(&joint.kind)?;
                let tool = *JointTool::ALL.get(index).filter(|t| **t != current)?;
                if tool.fits(&joint.moving, &joint.fixed) {
                    self.change_kind(ctx, id, created, placements, tool);
                } else {
                    // Faces of the wrong sort for it: pick them afresh.
                    if created {
                        crate::commands::record_joint(ctx, id, None, placements);
                    }
                    self.start_repick(id, tool);
                }
                return Some(TaskOutcome::Open);
            }
            JointEdit::Repick => {
                let current = JointTool::of_kind(&joint.kind)?;
                if created {
                    crate::commands::record_joint(ctx, id, None, placements);
                }
                self.start_repick(id, current);
                return Some(TaskOutcome::Open);
            }
            JointEdit::Number(key, value) => set(self, ctx, serde_json::json!({key: value})),
            JointEdit::Flip(on) => set(self, ctx, serde_json::json!({"flip": on})),
            JointEdit::Shift(k, value) => {
                // A fixed joint's shift, which `asm.set` does not take.
                let JointKind::Fixed { shift, .. } = &mut joint.kind else {
                    return None;
                };
                shift[k] = f64::from(value);
                if let Err(why) = ctx.document.update_feature_data(id, joint.to_json()) {
                    ctx.log_warn(why.to_string());
                }
                ctx.document.clear_feature_dirty(id);
                self.solve_and_apply(ctx);
            }
            JointEdit::CheckSweep => {
                if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = joint.kind
                {
                    let hinge = matches!(joint.kind, JointKind::Hinge { .. });
                    let range = motion_range(drive.limits, hinge, now);
                    self.check_sweep(ctx, id, range);
                }
            }
            JointEdit::StopSweep => self.tab.sweeping = None,
            JointEdit::SelectBody(body) => {
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            JointEdit::TurnBy(value) => self.tab.turn_by = Some(value),
            JointEdit::Turn => {
                let by = f64::from(self.tab.turn_by.unwrap_or(90.0));
                self.turn_in_task(ctx, id, created, placements, by, false);
            }
            JointEdit::TurnOver => self.turn_in_task(ctx, id, created, placements, 0.0, true),
            JointEdit::Turn90 => {
                let args = serde_json::json!({"joint": id.0.to_string()});
                self.run_in_task(ctx, id, created, placements, "asm.turn90", args);
            }
            JointEdit::SelectJoint(joint) => {
                ctx.request(core_document::HostRequest::SelectFeatures(vec![joint]));
            }
            JointEdit::Delete => {
                ctx.document.remove_feature(id).ok()?;
                self.tab.playing = None;
                // A joint the task made has nothing to undo in a recording.
                if !created {
                    ctx.record(
                        "doc.delete",
                        crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
                        serde_json::Value::Null,
                    );
                }
                self.tab.task = None;
                ctx.active_document_object = None;
                return Some(TaskOutcome::Accepted {
                    label: "Delete joint".to_string(),
                });
            }
        }
        None
    }

    /// Take the formula off a joint's number, which then keeps the value it
    /// came to.
    fn clear_formula(&self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId, key: &str) {
        if ctx.document.feature_formula(id, key).is_some()
            && let Err(why) = ctx.document.set_feature_formula(id, key, None)
        {
            ctx.log_warn(why.to_string());
        }
    }

    /// Make the joint another kind, from where the bodies stand, as
    /// `asm.set` with a `kind` does.
    fn change_kind(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        created: bool,
        placements: &[(BodyId, BodyPlacement)],
        tool: JointTool,
    ) {
        if created {
            crate::commands::record_joint(ctx, id, None, placements);
        }
        let args = crate::commands::object(
            serde_json::json!({"joint": id.0.to_string(), "kind": tool.word()}),
        );
        match crate::commands::run("asm.set", &args, ctx) {
            Ok(_) => ctx.record("asm.set", args, serde_json::Value::Null),
            Err(why) => ctx.log_warn(why.to_string()),
        }
        crate::keep_recorded(ctx, created);
        self.tab.task = Some(crate::Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: crate::all_placements(ctx),
        });
    }

    /// Turn the joint's body from its task, as `asm.turn` or `asm.flip`
    /// turns it.
    fn turn_in_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        created: bool,
        placements: &[(BodyId, BodyPlacement)],
        degrees: f64,
        over: bool,
    ) {
        let (command, args) = if over {
            ("asm.flip", serde_json::json!({"joint": id.0.to_string()}))
        } else {
            (
                "asm.turn",
                serde_json::json!({"joint": id.0.to_string(), "degrees": degrees}),
            )
        };
        // A hinge's motion moves on, a coupled one taking its partner along.
        self.run_in_task(ctx, id, created, placements, command, args);
    }

    /// Run `command` on the joint from its task. A joint the task just
    /// made is recorded first, so the recording has it to turn; the task
    /// goes on as an edit of it from where the command left it.
    fn run_in_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        created: bool,
        placements: &[(BodyId, BodyPlacement)],
        command: &str,
        args: serde_json::Value,
    ) {
        if !crate::joints(ctx.document).iter().any(|j| j.id == id) {
            return;
        }
        if created {
            crate::commands::record_joint(ctx, id, None, placements);
        }
        let args = crate::commands::object(args);
        if let Err(why) = crate::commands::run(command, &args, ctx) {
            ctx.log_warn(format!("Could not turn the body: {why}"));
            return;
        }
        self.solve_and_apply(ctx);
        ctx.record(command, args, serde_json::Value::Null);
        crate::keep_recorded(ctx, created);
        self.tab.task = Some(crate::Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: crate::all_placements(ctx),
        });
    }
}
