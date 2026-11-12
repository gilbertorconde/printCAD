//! The move panel: a body moved by numbers, along and about each axis.

use super::*;

/// What a field of the move panel asks.
pub(super) enum MoveEdit {
    Handles(usize),
    /// A position (0 to 2) or a turn (3 to 5) typed.
    Field(usize, f32),
    Home,
}

/// The move panel's fields: the position along each axis, then the turn
/// about each.
const MOVE_FIELDS: [&str; 6] = ["x", "y", "z", "turn_x", "turn_y", "turn_z"];

/// A placement's turn as angles about X, Y and Z, degrees.
fn move_angles(placement: &BodyPlacement) -> [f32; 3] {
    let (ax, ay, az) = placement.quat().to_euler(glam::EulerRot::XYZ);
    [ax, ay, az].map(f32::to_degrees)
}

impl AssemblyWorkbench {
    pub(super) fn move_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        body: BodyId,
        placements: &[(BodyId, BodyPlacement)],
        turns: &[(FeatureId, i64)],
    ) -> TaskOutcome {
        // Cancel's placements and coupling counts are taken back by the
        // host.
        if request.cancel {
            self.tab.handles.reset();
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.tab.handles.reset();
            // Every body the task moved, its handles' moves and the solves
            // its numbers ran, with the couplings' counts they changed.
            crate::drag::record_placed(ctx, placements, turns, Some(body));
            self.tab.task = None;
            return TaskOutcome::Accepted {
                label: "Move body".to_string(),
            };
        }
        TaskOutcome::Open
    }

    /// A body's place by numbers: its position and its turn about each
    /// axis, shown and not edited while its joints place it.
    pub(super) fn move_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        form: &mut Form<'_>,
    ) -> Option<MoveEdit> {
        let mut edit = None;
        let held = Self::held_by_joints(ctx, body);
        if held {
            form.push(w::note(
                NoteKind::Info,
                Some("Placed by its joints"),
                "Its joints decide where it sits. Move the body it is joined to, \
                 or delete a joint to free it.",
            ));
        }
        if !held {
            let (handles, picked) = f::select(
                form,
                "handles",
                bench_api::Select {
                    label: "Handles".into(),
                    options: crate::handles::MODES.map(String::from).to_vec(),
                    selected: self.tab.handles.mode_index(),
                    ..Default::default()
                },
            );
            form.push(handles);
            if let Some(index) = picked {
                edit = Some(MoveEdit::Handles(index));
            }
        }
        let placement = ctx.document.body_placement(body);
        let angles = move_angles(&placement);
        for (k, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let label = format!("Position {axis}");
            let value = placement.translation[k];
            if held {
                form.push(w::value(&label, format!("{value:.2} mm")));
                continue;
            }
            let (field, typed) = f::number(form, MOVE_FIELDS[k], &label, value, Dim::Length);
            form.push(field);
            if let Some(value) = typed {
                edit = Some(MoveEdit::Field(k, value as f32));
            }
        }
        for (k, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let label = format!("Turn about {axis}");
            if held {
                form.push(w::value(&label, format!("{:.2} °", angles[k])));
                continue;
            }
            let (field, typed) = f::ranged(
                form,
                MOVE_FIELDS[3 + k],
                &label,
                angles[k],
                Dim::Angle,
                Some((-180.0, 180.0)),
            );
            form.push(field);
            if let Some(value) = typed {
                edit = Some(MoveEdit::Field(3 + k, value as f32));
            }
        }
        if !held {
            let (home, pressed) = f::button(
                form,
                "home",
                "Back to where it was made",
                ButtonStyle::Secondary,
            );
            form.push(w::hinted("No move, no turn", home));
            if pressed {
                edit = Some(MoveEdit::Home);
            }
        }
        if let Some(note) = self.verdict_note() {
            form.push(note);
        }
        edit
    }

    /// A field of the move panel changed: the body goes there as
    /// `asm.place` puts it, and whatever is joined to it follows. The
    /// handles' choice only changes what the view shows.
    pub(super) fn move_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: BodyId,
        edit: MoveEdit,
    ) -> Option<TaskOutcome> {
        let moved = match edit {
            MoveEdit::Handles(index) => {
                self.tab.handles.set_mode(ctx, body, index);
                return None;
            }
            MoveEdit::Home => BodyPlacement::IDENTITY,
            MoveEdit::Field(k, value) => {
                let placement = ctx.document.body_placement(body);
                let mut offset = placement.translation;
                let mut angles = move_angles(&placement);
                match k {
                    0..3 => offset[k] = value,
                    _ => angles[k - 3] = value,
                }
                let [ax, ay, az] = angles.map(f32::to_radians);
                BodyPlacement::new(
                    glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, az),
                    glam::Vec3::from_array(offset),
                )
            }
        };
        let args = serde_json::json!({
            "body": body.0.to_string(),
            "translation": moved.translation,
            "rotation": moved.rotation,
        });
        if self.call(ctx, "asm.place", args).is_ok() {
            self.solve_and_apply(ctx);
        }
        None
    }
}
