//! The linked copies panel, the panel that replaces a body's shape, and
//! a rigid group's panel.

use super::*;

/// What a field of the copies panel asks.
pub(super) enum CopiesEdit {
    Mirror(bool),
    Around(bool),
    Plane(usize),
    Axis(usize),
    Count(f32),
    Over(f32),
    MirrorAt(usize, f32),
    Through(usize, f32),
    Step(usize, f32),
}

/// What a field of the rigid group's panel asks.
pub(super) enum GroupEdit {
    Remove(usize),
    Dissolve,
}

impl AssemblyWorkbench {
    /// Linked copies of a body: how many and how far apart; OK inserts them
    /// in a row, to be dragged where they go.
    pub(super) fn copies_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        body: BodyId,
        (count, step, around): (u32, [f32; 3], Option<crate::Around>),
        mirror: Option<([f32; 3], [f32; 3])>,
    ) -> TaskOutcome {
        if request.cancel {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept
            && let Some((point, normal)) = mirror
        {
            let args = crate::commands::object(serde_json::json!({
                "body": body.0.to_string(),
                "point": point,
                "normal": normal,
            }));
            return match crate::commands::run("asm.mirror", &args, ctx) {
                Ok(copy) => {
                    ctx.record("asm.mirror", args, copy);
                    self.tab.task = None;
                    TaskOutcome::Accepted {
                        label: "Insert mirrored copy".to_string(),
                    }
                }
                Err(_) => {
                    ctx.log_warn("A mirrored copy cannot be mirrored again");
                    TaskOutcome::Open
                }
            };
        }
        if request.accept {
            let mut args = serde_json::json!({"body": body.0.to_string(), "count": count});
            match around {
                Some((point, direction, angle)) => {
                    args["around"] =
                        serde_json::json!({"point": point, "direction": direction, "angle": angle});
                }
                None => args["step"] = serde_json::json!(step),
            }
            let args = crate::commands::object(args);
            let made = crate::commands::run("asm.copy", &args, ctx)
                .unwrap_or_else(|_| serde_json::json!([]));
            let n = made.as_array().map_or(0, Vec::len);
            ctx.record("asm.copy", args, made);
            ctx.log_info(format!(
                "Inserted {n} linked cop{} of {}: drag them where they go",
                if n == 1 { "y" } else { "ies" },
                body_name(ctx, body)
            ));
            self.tab.task = None;
            return TaskOutcome::Accepted {
                label: "Insert linked copies".to_string(),
            };
        }
        TaskOutcome::Open
    }

    /// How many copies and how far apart, or turned about an axis, or one
    /// mirrored across a plane.
    pub(super) fn copies_walk(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        (count, step, around): (u32, [f32; 3], Option<crate::Around>),
        mirror: Option<([f32; 3], [f32; 3])>,
        form: &mut Form<'_>,
    ) -> Option<CopiesEdit> {
        let mut edit = None;
        form.push(w::value("Of", body_name(ctx, body)));
        let (how_many, typed) = f::quantity(
            form,
            "count",
            f64::from(count),
            bench_api::Quantity {
                label: "How many".into(),
                dim: Dim::Number,
                bind: None,
                min: Some(1.0),
                max: Some(500.0),
                decimals: 0,
                error: None,
                ..Default::default()
            },
        );
        form.push(how_many);
        if let Some(value) = typed {
            edit = Some(CopiesEdit::Count(value as f32));
        }
        let (mirrored, on) = f::toggle(form, "mirror", "A mirror image", mirror.is_some());
        form.push(w::hinted(
            "One copy mirrored across a plane, rather than copies as they are",
            mirrored,
        ));
        if let Some(on) = on {
            edit = Some(CopiesEdit::Mirror(on));
        }
        /// A field for each axis, named `prefix` and its place; the one
        /// typed into.
        fn axes(
            form: &mut Form<'_>,
            prefix: &str,
            labels: [&str; 3],
            at: [f32; 3],
        ) -> Option<(usize, f32)> {
            let mut typed = None;
            for (k, label) in labels.into_iter().enumerate() {
                let (field, value) =
                    f::number(form, &format!("{prefix}{k}"), label, at[k], Dim::Length);
                form.push(field);
                if let Some(value) = value {
                    typed = Some((k, value as f32));
                }
            }
            typed
        }
        let through = ["Through x", "Through y", "Through z"];
        if let Some((point, normal)) = mirror {
            let selected = AXES.iter().position(|a| *a == normal).unwrap_or(0);
            let (plane, picked) = f::choice(form, "plane", "Across", &["YZ", "XZ", "XY"], selected);
            form.push(plane);
            if let Some(index) = picked {
                edit = Some(CopiesEdit::Plane(index));
            }
            if let Some((k, value)) = axes(form, "mirror_", through, point) {
                edit = Some(CopiesEdit::MirrorAt(k, value));
            }
            form.push(w::text(
                "The mirror image takes the body's shape, mirrored, and follows every change \
                 to it.",
            ));
            return edit;
        }
        let (turned, on) = f::toggle(form, "around", "Around an axis", around.is_some());
        form.push(w::hinted(
            "Turn the copies about an axis instead of setting them in a row",
            turned,
        ));
        if let Some(on) = on {
            edit = Some(CopiesEdit::Around(on));
        }
        match around {
            Some((point, axis, angle)) => {
                let selected = AXES.iter().position(|a| *a == axis).unwrap_or(2);
                let (axis, picked) = f::choice(form, "axis", "Axis", &["X", "Y", "Z"], selected);
                form.push(axis);
                if let Some(index) = picked {
                    edit = Some(CopiesEdit::Axis(index));
                }
                if let Some((k, value)) = axes(form, "through_", through, point) {
                    edit = Some(CopiesEdit::Through(k, value));
                }
                let (over, typed) = f::number(form, "over", "Over", angle, Dim::Angle);
                form.push(over);
                if let Some(value) = typed {
                    edit = Some(CopiesEdit::Over(value as f32));
                }
            }
            None => {
                if let Some((k, value)) = axes(form, "step_", ["Step x", "Step y", "Step z"], step)
                {
                    edit = Some(CopiesEdit::Step(k, value));
                }
            }
        }
        form.push(w::text(
            "Each copy takes the body's shape and follows every change to it. They go in a \
             row, each a step from the one before; drag one to put it where it goes.",
        ));
        edit
    }

    /// A change to the copies' settings, kept in the task until OK.
    pub(super) fn copies_edit(&mut self, edit: CopiesEdit) -> Option<TaskOutcome> {
        let Some(Task::Copies {
            count,
            step,
            around,
            mirror,
            ..
        }) = &mut self.tab.task
        else {
            return None;
        };
        let mut pivot = around.unwrap_or(([0.0; 3], [0.0, 0.0, 1.0], 360.0));
        let mut plane = mirror.unwrap_or(([0.0; 3], [1.0, 0.0, 0.0]));
        match edit {
            CopiesEdit::Mirror(on) => *mirror = on.then_some(plane),
            CopiesEdit::Around(on) => *around = on.then_some(pivot),
            CopiesEdit::Plane(index) => {
                plane.1 = AXES[index.min(2)];
                *mirror = Some(plane);
            }
            CopiesEdit::Axis(index) => {
                pivot.1 = AXES[index.min(2)];
                *around = Some(pivot);
            }
            CopiesEdit::Count(value) => *count = value.round().max(1.0) as u32,
            CopiesEdit::Over(value) => {
                pivot.2 = value;
                *around = Some(pivot);
            }
            CopiesEdit::MirrorAt(k, value) => {
                plane.0[k] = value;
                *mirror = Some(plane);
            }
            CopiesEdit::Through(k, value) => {
                pivot.0[k] = value;
                *around = Some(pivot);
            }
            CopiesEdit::Step(k, value) => step[k] = value,
        }
        None
    }

    /// A body and the one picked to take its place and joints.
    pub(super) fn replace_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        old: BodyId,
        new: Option<BodyId>,
    ) -> TaskOutcome {
        if request.cancel {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            let Some(new) = new else {
                ctx.log_warn("Click the body that takes its place");
                return TaskOutcome::Open;
            };
            match crate::replace::replace(ctx.document, old, new) {
                Ok(report) => {
                    self.solve_and_apply(ctx);
                    ctx.record(
                        "asm.replace",
                        crate::commands::object(serde_json::json!({
                            "body": old.0.to_string(),
                            "with": new.0.to_string(),
                        })),
                        serde_json::json!({"kept": report.kept, "unmatched": report.unmatched}),
                    );
                    if report.unmatched.is_empty() {
                        ctx.log_info(format!(
                            "{} took the place of {}; every joint found its faces",
                            body_name(ctx, new),
                            body_name(ctx, old)
                        ));
                    } else {
                        ctx.log_warn(format!(
                            "{} took the place of {}; no face matched for {}",
                            body_name(ctx, new),
                            body_name(ctx, old),
                            report.unmatched.join(", ")
                        ));
                    }
                    self.tab.task = None;
                    return TaskOutcome::Accepted {
                        label: "Replace body".to_string(),
                    };
                }
                Err(why) => {
                    ctx.log_warn(format!("Could not replace the body: {why}"));
                    return TaskOutcome::Open;
                }
            }
        }
        TaskOutcome::Open
    }

    /// The body replaced and the one taking its place.
    pub(super) fn replace_widgets(
        ctx: &WorkbenchRuntimeContext,
        old: BodyId,
        new: Option<BodyId>,
    ) -> Vec<Widget> {
        vec![
            w::value("Replace", body_name(ctx, old)),
            w::value(
                "With",
                new.map_or("click a body".to_string(), |b| body_name(ctx, b)),
            ),
            w::text(
                "The new body goes where the old one sits and takes its joints, each end on \
                 the new body's nearest face of the same kind; the old body is hidden.",
            ),
        ]
    }

    /// The bodies of a rigid group, picked one click each; OK locks them
    /// where they sit.
    pub(super) fn group_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        editing: Option<FeatureId>,
        members: &[BodyId],
    ) -> TaskOutcome {
        if request.cancel {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            let Some(id) = self.make_group(ctx, editing, members) else {
                return TaskOutcome::Open;
            };
            let bodies: Vec<String> = members.iter().map(|b| b.0.to_string()).collect();
            let args = match editing {
                Some(_) => serde_json::json!({"group": id.0.to_string(), "bodies": bodies}),
                None => serde_json::json!({"bodies": bodies}),
            };
            ctx.record(
                "asm.group",
                crate::commands::object(args),
                serde_json::json!(id.0.to_string()),
            );
            self.tab.task = None;
            ctx.active_document_object = Some(id);
            return TaskOutcome::Accepted {
                label: if editing.is_some() {
                    "Edit rigid group"
                } else {
                    "Rigid group"
                }
                .to_string(),
            };
        }
        TaskOutcome::Open
    }

    /// The bodies picked for a group, each with its Remove; the group's
    /// Dissolve when it is one already.
    pub(super) fn group_walk(
        ctx: &WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        members: &[BodyId],
        form: &mut Form<'_>,
    ) -> Option<GroupEdit> {
        let mut edit = None;
        form.push(w::text(
            "Click the bodies to lock together; a second click takes one out. They \
                 move as one, held as they sit now, the first the one the rest hold to.",
        ));
        for (i, body) in members.iter().enumerate() {
            let (remove, pressed) =
                f::button(form, &format!("remove:{i}"), "Remove", ButtonStyle::Small);
            if pressed {
                edit = Some(GroupEdit::Remove(i));
            }
            form.push(w::row(vec![w::text(body_name(ctx, *body)), remove]));
        }
        if members.len() < 2 {
            form.push(w::note(
                NoteKind::Info,
                None,
                "A group takes two bodies or more",
            ));
        }
        if editing.is_some() {
            let (dissolve, pressed) =
                f::button(form, "dissolve", "Dissolve group", ButtonStyle::Destructive);
            form.push(w::hinted(
                "Remove the group; the bodies stay where they are",
                dissolve,
            ));
            if pressed {
                edit = Some(GroupEdit::Dissolve);
            }
        }
        edit
    }

    /// A body taken out of the group, or the group dissolved.
    pub(super) fn group_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        edit: GroupEdit,
    ) -> Option<TaskOutcome> {
        let id = match edit {
            GroupEdit::Remove(i) => {
                if let Some(Task::Group { members, .. }) = &mut self.tab.task
                    && i < members.len()
                {
                    members.remove(i);
                }
                return None;
            }
            GroupEdit::Dissolve => editing?,
        };
        ctx.document.remove_feature(id).ok()?;
        ctx.record(
            "doc.delete",
            crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
            serde_json::Value::Null,
        );
        self.tab.task = None;
        ctx.active_document_object = None;
        Some(TaskOutcome::Accepted {
            label: "Dissolve rigid group".to_string(),
        })
    }
}
