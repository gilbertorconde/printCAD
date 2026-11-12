//! A coupling's panel: its two joints, what ties them and the ratio.

use super::*;

/// What a field of a coupling's panel asks.
pub(super) enum CouplingEdit {
    /// The driving joint picked, by its place among the hinges and sliders.
    Driver(usize),
    Driven(usize),
    /// What ties them, by its place among the gearings offered.
    Gearing(usize),
    Ratio(f64),
    Reverse(bool),
    Delete,
}

/// What may tie a coupling's two joints, its own kind among them.
fn coupling_gearings(movable: &[crate::Joint], coupling: &Coupling) -> Vec<Gearing> {
    let kind_of = |id: FeatureId| movable.iter().find(|j| j.id == id).map(|j| j.feature.kind);
    let (driver, driven) = (kind_of(coupling.driver), kind_of(coupling.driven));
    Gearing::ALL
        .into_iter()
        .filter(|g| {
            *g == coupling.gearing || driver.zip(driven).is_some_and(|(a, b)| g.fits(&a, &b))
        })
        .collect()
}

impl AssemblyWorkbench {
    /// A coupling's settings: its two joints, what ties them, the ratio.
    /// Each change is made as `asm.set` makes it and solves at once.
    pub(super) fn coupling_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        id: FeatureId,
        before: Option<serde_json::Value>,
    ) -> TaskOutcome {
        let created = before.is_none();
        if request.cancel {
            if created {
                ctx.active_document_object = None;
            }
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            crate::commands::record_coupling(ctx, id, before.as_ref());
            self.tab.task = None;
            ctx.active_document_object = None;
            return TaskOutcome::Accepted {
                label: if created {
                    "Add coupling"
                } else {
                    "Edit coupling"
                }
                .to_string(),
            };
        }
        if ctx.document.get_feature_meta(id).is_none() {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        TaskOutcome::Open
    }

    /// A coupling's settings: its two joints, what ties them, the ratio,
    /// then what the last change's solve said and the coupling's Delete.
    pub(super) fn coupling_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: FeatureId,
        form: &mut Form<'_>,
    ) -> Option<CouplingEdit> {
        let node = ctx.document.get_feature_meta(id)?;
        let Ok(coupling) = Coupling::from_json(&node.data) else {
            form.push(w::note(
                NoteKind::Error,
                Some("Unreadable coupling"),
                "The stored coupling does not parse.",
            ));
            return None;
        };
        let mut edit = None;
        let movable = movable_joints(ctx);
        let named = |j: &crate::Joint| format!("{} ({})", j.name, j.feature.kind.label());
        let pick = |form: &mut Form<'_>, key: &str, label: &str, current: FeatureId| {
            let mut options: Vec<String> = movable.iter().map(named).collect();
            let selected = movable
                .iter()
                .position(|j| j.id == current)
                .unwrap_or_else(|| {
                    options.push("a removed joint".into());
                    options.len() - 1
                });
            let (field, picked) = f::select(
                form,
                key,
                bench_api::Select {
                    label: label.into(),
                    options,
                    selected,
                    ..Default::default()
                },
            );
            form.push(field);
            picked
        };
        if let Some(index) = pick(form, "driver", "Driving", coupling.driver) {
            edit = Some(CouplingEdit::Driver(index));
        }
        if let Some(index) = pick(form, "driven", "Driven", coupling.driven) {
            edit = Some(CouplingEdit::Driven(index));
        }
        let gearings = coupling_gearings(&movable, &coupling);
        let (gearing, picked) = f::select(
            form,
            "gearing",
            bench_api::Select {
                label: "Kind".into(),
                options: gearings.iter().map(|g| g.label().to_string()).collect(),
                selected: gearings
                    .iter()
                    .position(|g| *g == coupling.gearing)
                    .unwrap_or(0),
                ..Default::default()
            },
        );
        form.push(gearing);
        if let Some(index) = picked {
            edit = Some(CouplingEdit::Gearing(index));
        }
        let (ratio_label, length) = coupling.gearing.ratio_label();
        let (ratio, typed) = f::bound(
            form,
            "ratio",
            ratio_label,
            coupling.ratio,
            if length { Dim::Length } else { Dim::Number },
            (id, "/ratio"),
        );
        form.push(w::hinted(coupling.gearing.summary(), ratio));
        if let Some(value) = typed {
            edit = Some(CouplingEdit::Ratio(value));
        }
        let (reverse, on) = f::toggle(form, "reverse", "Reverse", coupling.reverse);
        form.push(w::hinted("The driven joint moves the other way", reverse));
        if let Some(on) = on {
            edit = Some(CouplingEdit::Reverse(on));
        }
        form.push(w::text(
            "Move the driving joint (drag its body, or drive it) and the driven \
             one follows.",
        ));
        if let Some(note) = self.verdict_note() {
            form.push(note);
        }
        if let Some(text) = node.body.and_then(|body| freedom_text(ctx, body)) {
            form.push(text);
        }
        let (delete, pressed) =
            f::button(form, "delete", "Delete coupling", ButtonStyle::Destructive);
        form.push(w::hinted(
            "Remove the coupling; the joints move apart again",
            delete,
        ));
        if pressed {
            edit = Some(CouplingEdit::Delete);
        }
        edit
    }

    /// A change to a coupling, made as `asm.set` makes it, or the coupling
    /// deleted.
    pub(super) fn coupling_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        (id, created): (FeatureId, bool),
        edit: CouplingEdit,
    ) -> Option<TaskOutcome> {
        let coupling = Coupling::from_json(ctx.document.get_feature_data(id)?).ok()?;
        let movable = movable_joints(ctx);
        let mut args = serde_json::Map::new();
        match edit {
            CouplingEdit::Driver(index) | CouplingEdit::Driven(index) => {
                let (key, current) = match edit {
                    CouplingEdit::Driver(_) => ("driver", coupling.driver),
                    _ => ("driven", coupling.driven),
                };
                let joint = movable.get(index).filter(|j| j.id != current)?;
                args.insert(key.into(), joint.id.0.to_string().into());
            }
            CouplingEdit::Gearing(index) => {
                let gearing = *coupling_gearings(&movable, &coupling).get(index)?;
                if gearing == coupling.gearing {
                    return None;
                }
                args.insert("gearing".into(), gearing.word().into());
            }
            CouplingEdit::Ratio(value) => {
                if value <= 0.0 {
                    return None;
                }
                args.insert("ratio".into(), serde_json::json!(value as f32));
            }
            CouplingEdit::Reverse(on) => {
                args.insert("reverse".into(), on.into());
            }
            CouplingEdit::Delete => {
                ctx.document.remove_feature(id).ok()?;
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
                    label: "Delete coupling".to_string(),
                });
            }
        }
        args.insert("joint".into(), id.0.to_string().into());
        match crate::commands::run("asm.set", &args, ctx) {
            Ok(_) => self.tab.verdict = None,
            Err(err) => self.tab.verdict = Some(Err(err.to_string())),
        }
        None
    }
}
