//! The exploded view's panel: the spread, or the steps once bodies are
//! picked for one.

use super::*;

/// What a field of the exploded view's panel asks.
pub(super) enum ExplodeEdit {
    Spread(f32),
    /// The view scrubbed to a step.
    At(f32),
    TurnAxis(usize),
    TurnDegrees(f32),
    Radial(f32),
    Shift(usize, f32),
    Play,
    AddStep,
    RemoveStep(usize),
}

impl AssemblyWorkbench {
    /// The exploded view's spread, or, once bodies are picked for a step,
    /// its steps; closing puts every body back.
    pub(super) fn explode_walk(
        ctx: &WorkbenchRuntimeContext,
        spread: f32,
        steps: &crate::Stepping,
        form: &mut Form<'_>,
    ) -> Option<ExplodeEdit> {
        let mut edit = None;
        let slider = |id: String, label: &str, value: f32, max: f64| Widget::Slider {
            id,
            label: label.into(),
            value: f64::from(value),
            min: 0.0,
            max,
            step: None,
            decimals: 2,
            show_value: true,
        };
        let number = |event: Option<&PanelEvent>| match event {
            Some(PanelEvent::Number { value, .. }) => Some(*value as f32),
            _ => None,
        };
        if steps.view.is_none() && steps.picked.is_empty() {
            let (id, event) = f::raw(form, "spread");
            form.push(w::hinted(
                "How far each body moves out, as a share of its distance from the middle",
                slider(id, "Spread", spread, 3.0),
            ));
            form.push(w::text(
                "Each body moves straight out from the middle of the assembly. \
                 Nothing is kept: the bodies go back when this closes. Click bodies to \
                 make a step of a view kept in the document.",
            ));
            return number(event).map(ExplodeEdit::Spread);
        }
        let view = steps
            .view
            .and_then(|id| crate::exploded::view_of(ctx.document, id))
            .unwrap_or_default();
        for (i, step) in view.steps.iter().enumerate() {
            let names: Vec<String> = step.bodies.iter().map(|b| body_name(ctx, *b)).collect();
            let (remove, pressed) = f::button(
                form,
                &format!("remove_step:{i}"),
                "Remove",
                ButtonStyle::Small,
            );
            if pressed {
                edit = Some(ExplodeEdit::RemoveStep(i));
            }
            // The button first, so a long line wraps beside it rather
            // than squeezing it.
            form.push(w::row(vec![
                remove,
                w::text(format!(
                    "{}. {}: {}",
                    i + 1,
                    names.join(", "),
                    step.summary()
                )),
            ]));
        }
        if !view.steps.is_empty() {
            let (id, event) = f::raw(form, "at");
            if let Some(at) = number(event) {
                edit = Some(ExplodeEdit::At(at));
            }
            form.push(slider(id, "Progress", steps.at, view.steps.len() as f64));
            let (play, pressed) = f::button(
                form,
                "play",
                if steps.playing { "Stop" } else { "Play" },
                ButtonStyle::Secondary,
            );
            if pressed {
                edit = Some(ExplodeEdit::Play);
            }
            form.push(w::hinted(
                "Play the steps in order, a step a second, round again",
                play,
            ));
        }
        let picked: Vec<String> = steps.picked.iter().map(|b| body_name(ctx, *b)).collect();
        let mut next = vec![w::text(if picked.is_empty() {
            "Click the bodies it moves".to_string()
        } else {
            picked.join(", ")
        })];
        for (k, label) in ["Shift x", "Shift y", "Shift z"].into_iter().enumerate() {
            let (shift, typed) = f::number(
                form,
                &format!("shift_{k}"),
                label,
                steps.shift[k],
                Dim::Length,
            );
            next.push(shift);
            if let Some(value) = typed {
                edit = Some(ExplodeEdit::Shift(k, value as f32));
            }
        }
        let (axis, picked_axis) = f::choice(
            form,
            "turn_axis",
            "Turn about",
            &["X", "Y", "Z"],
            steps.turn_axis.min(2),
        );
        next.push(axis);
        if let Some(index) = picked_axis {
            edit = Some(ExplodeEdit::TurnAxis(index));
        }
        let (turn, typed) = f::number(form, "turn_degrees", "Turn", steps.turn_degrees, Dim::Angle);
        next.push(turn);
        if let Some(value) = typed {
            edit = Some(ExplodeEdit::TurnDegrees(value as f32));
        }
        let (radial, typed) = f::number(form, "radial", "Spread out", steps.radial, Dim::Length);
        next.push(w::hinted(
            "How far each body moves straight out from the middle of the step's bodies",
            radial,
        ));
        if let Some(value) = typed {
            edit = Some(ExplodeEdit::Radial(value as f32));
        }
        if !steps.picked.is_empty() {
            let (add, pressed) = f::button(form, "add_step", "Add step", ButtonStyle::Secondary);
            next.push(add);
            if pressed {
                edit = Some(ExplodeEdit::AddStep);
            }
        }
        form.push(w::section("Next step", next));
        form.push(w::text(
            "The view is kept in the document: double-click it in the tree to show it \
             again. The bodies go back when this closes.",
        ));
        edit
    }

    /// The spread changed, or a step added or taken away (kept as
    /// `asm.exploded_view` keeps it), or the view scrubbed or played.
    pub(super) fn explode_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        edit: ExplodeEdit,
    ) -> Option<TaskOutcome> {
        let Some(Task::Explode {
            placements,
            spread,
            steps,
        }) = &mut self.tab.task
        else {
            return None;
        };
        let placements = placements.clone();
        let view = steps
            .view
            .and_then(|id| crate::exploded::view_of(ctx.document, id))
            .unwrap_or_default();
        let mut edited = None;
        match edit {
            ExplodeEdit::Spread(value) => {
                *spread = value;
                crate::explode(ctx, &placements, *spread);
                return None;
            }
            ExplodeEdit::At(value) => steps.at = value,
            ExplodeEdit::TurnAxis(index) => {
                steps.turn_axis = index.min(2);
                return None;
            }
            ExplodeEdit::TurnDegrees(value) => {
                steps.turn_degrees = value;
                return None;
            }
            ExplodeEdit::Radial(value) => {
                steps.radial = value;
                return None;
            }
            ExplodeEdit::Shift(k, value) => {
                steps.shift[k] = value;
                return None;
            }
            ExplodeEdit::Play => {
                steps.playing = !steps.playing;
                return None;
            }
            ExplodeEdit::AddStep => {
                let mut changed = view.clone();
                let bodies = std::mem::take(&mut steps.picked);
                // Turned about, and spread from, the middle of its bodies.
                let middles: Vec<glam::Vec3> = bodies
                    .iter()
                    .filter_map(|b| {
                        let at = placements.iter().find(|(p, _)| p == b)?.1;
                        crate::exploded::centre(ctx.document, *b, &at).map(glam::Vec3::from_array)
                    })
                    .collect();
                let middle = (middles.iter().copied().sum::<glam::Vec3>()
                    / middles.len().max(1) as f32)
                    .to_array();
                changed.steps.push(crate::ExplodeStep {
                    bodies,
                    shift: steps.shift,
                    turn: (steps.turn_degrees != 0.0).then_some(crate::exploded::StepTurn {
                        point: middle,
                        axis: AXES[steps.turn_axis.min(2)],
                        degrees: steps.turn_degrees,
                    }),
                    radial: (steps.radial != 0.0).then_some(crate::exploded::StepRadial {
                        centre: middle,
                        axis: None,
                        distance: steps.radial,
                    }),
                });
                steps.at = changed.steps.len() as f32;
                edited = Some(changed);
            }
            ExplodeEdit::RemoveStep(i) => {
                let mut changed = view.clone();
                changed.steps.remove(i);
                steps.at = steps.at.min(changed.steps.len() as f32);
                edited = Some(changed);
            }
        }
        if let Some(changed) = edited {
            // Back where they sat, so the view plays from there.
            crate::restore_placements(ctx, &placements);
            let mut args = serde_json::json!({
                "steps": serde_json::to_value(&changed.steps).unwrap_or_default(),
            });
            if let Some(id) = steps.view {
                args["view"] = serde_json::json!(id.0.to_string());
            }
            let made = crate::commands::run(
                "asm.exploded_view",
                &crate::commands::object(args.clone()),
                ctx,
            );
            match made
                .ok()
                .and_then(|id| uuid::Uuid::parse_str(id.as_str()?).ok())
            {
                Some(id) => {
                    steps.view = Some(FeatureId(id));
                    args["view"] = serde_json::json!(id.to_string());
                    ctx.record(
                        "asm.exploded_view",
                        crate::commands::object(args),
                        serde_json::json!(id.to_string()),
                    );
                }
                None => ctx.log_warn("Could not keep the exploded view"),
            }
        }
        self.show_steps(ctx);
        None
    }
}
