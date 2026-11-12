//! The interference panel: the check's progress, then each clash found.

use super::*;

/// What a field of the interference panel asks.
pub(super) enum InterferenceEdit {
    /// A pair clicked: its first body selected.
    Select(BodyId),
    Clearance(f32),
    Stop,
    CheckClashes,
    CheckClearance,
    EveryPair,
}

impl AssemblyWorkbench {
    /// What an interference check found: each clash, a click selecting
    /// the first of its bodies.
    pub(super) fn interference_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.tab.checking = None;
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_interference(ctx);
        TaskOutcome::Open
    }

    /// The check's progress while it runs; then what it found, each pair a
    /// row, and the checks to run again.
    pub(super) fn interference_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        found: Option<&crate::Interference>,
        (seq, around, clearance): (u64, Option<BodyId>, Option<f32>),
        form: &mut Form<'_>,
    ) -> Option<InterferenceEdit> {
        let mut edit = None;
        let Some(found) = found else {
            let (done, total) = self.interference_progress().unwrap_or((0, 0));
            form.push(Widget::Progress {
                label: format!("Checking {done} of {total} pairs that may touch"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            let (stop, pressed) = f::button(form, "stop", "Stop", ButtonStyle::Secondary);
            form.push(w::hinted(
                "Stop checking; the clashes found so far stay",
                stop,
            ));
            return pressed.then_some(InterferenceEdit::Stop);
        };
        let bodies = format!(
            "{} bod{}",
            found.checked,
            if found.checked == 1 { "y" } else { "ies" }
        );
        let pair = |a: BodyId, b: BodyId, what: String| bench_api::ListItem {
            label: format!("{} and {}: {what}", body_name(ctx, a), body_name(ctx, b)),
            detail: None,
            icon: None,
        };
        if let Some(gap) = clearance {
            form.push(match found.near.len() {
                0 => w::note(
                    NoteKind::Success,
                    None,
                    format!("No pair nearer than {gap} mm among {bodies}"),
                ),
                n => w::note(
                    NoteKind::Warning,
                    Some(&format!(
                        "{n} pair{} nearer than {gap} mm",
                        if n == 1 { "" } else { "s" }
                    )),
                    format!("Among {bodies}; click one to select its first body"),
                ),
            });
            let (id, event) = f::raw(form, "near");
            if let Some(near) = f::picked(event).and_then(|i| found.near.get(i)) {
                edit = Some(InterferenceEdit::Select(near.a));
            }
            form.push(Widget::List {
                id,
                items: found
                    .near
                    .iter()
                    .map(|n| pair(n.a, n.b, format!("{:.2} mm", n.distance_mm)))
                    .collect(),
                selected: None,
            });
        } else {
            form.push(match found.clashes.len() {
                0 => w::note(
                    NoteKind::Success,
                    None,
                    format!("No interference among {bodies}"),
                ),
                n => w::note(
                    NoteKind::Error,
                    Some(&format!("{n} clash{}", if n == 1 { "" } else { "es" })),
                    format!("Among {bodies}; click one to select its first body"),
                ),
            });
        }
        let (id, event) = f::raw(form, "clashes");
        if let Some(clash) = f::picked(event).and_then(|i| found.clashes.get(i)) {
            edit = Some(InterferenceEdit::Select(clash.a));
        }
        form.push(Widget::List {
            id,
            items: found
                .clashes
                .iter()
                .map(|c| pair(c.a, c.b, format!("{:.2} mm³", c.volume_mm3)))
                .collect(),
            selected: None,
        });
        if !found.unchecked.is_empty() {
            let n = found.unchecked.len();
            form.push(w::note(
                NoteKind::Warning,
                Some(&format!(
                    "{n} pair{} could not be checked",
                    if n == 1 { "" } else { "s" }
                )),
                "The kernel failed on these; every other pair was checked",
            ));
            form.push(Widget::List {
                id: "unchecked".into(),
                items: found
                    .unchecked
                    .iter()
                    .map(|u| pair(u.a, u.b, u.why.clone()))
                    .collect(),
                selected: None,
            });
        }
        if found.stopped {
            form.push(w::note(
                NoteKind::Warning,
                None,
                "Stopped early: some pairs were not checked",
            ));
        }
        if found.skipped > 0 {
            form.push(w::text(format!(
                "{} visible bod{} without a solid (a mesh, or not built yet) left out",
                found.skipped,
                if found.skipped == 1 { "y" } else { "ies" }
            )));
        }
        if ctx.document.mutation_seq() != seq {
            form.push(w::note(
                NoteKind::Warning,
                None,
                "The assembly has changed since this check",
            ));
        }
        if let Some(body) = around {
            form.push(w::note(
                NoteKind::Info,
                None,
                format!("{} against every other body", body_name(ctx, body)),
            ));
        }
        let (gap, typed) = f::ranged(
            form,
            "clearance",
            "Clearance",
            self.tab.clearance_mm.or(clearance).unwrap_or(0.5),
            Dim::Length,
            Some((0.0, 1000.0)),
        );
        form.push(gap);
        if let Some(value) = typed {
            edit = Some(InterferenceEdit::Clearance(value as f32));
        }
        let (clashes, check_clashes) = f::button(
            form,
            "check_clashes",
            "Check clashes",
            ButtonStyle::Secondary,
        );
        let (near, check_clearance) = f::button(
            form,
            "check_clearance",
            "Check clearance",
            ButtonStyle::Secondary,
        );
        form.push(w::row(vec![
            w::hinted("Pairs that share material", clashes),
            w::hinted("Pairs nearer to each other than the clearance", near),
        ]));
        if check_clashes {
            edit = Some(InterferenceEdit::CheckClashes);
        }
        if check_clearance {
            edit = Some(InterferenceEdit::CheckClearance);
        }
        if around.is_some() {
            let (every, pressed) = f::button(
                form,
                "every_pair",
                "Check every pair",
                ButtonStyle::Secondary,
            );
            form.push(every);
            if pressed {
                edit = Some(InterferenceEdit::EveryPair);
            }
        }
        edit
    }

    /// A pair clicked selects its first body; the buttons stop the check
    /// or run another.
    pub(super) fn interference_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        (around, clearance): (Option<BodyId>, Option<f32>),
        edit: InterferenceEdit,
    ) -> Option<TaskOutcome> {
        let gap = self.tab.clearance_mm.or(clearance).unwrap_or(0.5);
        match edit {
            InterferenceEdit::Select(body) => {
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            InterferenceEdit::Clearance(value) => self.tab.clearance_mm = Some(value),
            InterferenceEdit::Stop => self.stop_interference(),
            InterferenceEdit::CheckClashes => self.check_interference(ctx, around),
            InterferenceEdit::CheckClearance => self.check_clearance(ctx, around, gap),
            InterferenceEdit::EveryPair => match clearance {
                Some(gap) => self.check_clearance(ctx, None, gap),
                None => self.check_interference(ctx, None),
            },
        }
        None
    }
}
