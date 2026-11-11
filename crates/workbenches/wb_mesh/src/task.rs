//! Live mesh tasks, their declared controls and build measurements.
use super::*;
use bench_api::{Check, Dim, PanelEvent, Quantity, Select, Widget};
use core_document::{TaskOutcome, TaskRequest};
use kernel_api::{MeshFairing, MeshStepReport};

pub(super) struct Task {
    pub id: FeatureId,
    created: Option<(String, CommandArgs, Value)>,
    pick: bool,
}

fn read(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Option<MeshFeature> {
    MeshFeature::from_json(ctx.document.feature_values(id)?).ok()
}

fn number(id: &str, label: &str, value: f64, dim: Dim, min: Option<f64>) -> Widget {
    Quantity {
        id: id.into(),
        label: label.into(),
        value,
        dim,
        min,
        decimals: if dim == Dim::Number { 0 } else { 4 },
        ..Default::default()
    }
    .into()
}

fn choice(id: &str, label: &str, options: &[&str], selected: usize) -> Widget {
    Select {
        id: id.into(),
        label: label.into(),
        options: options.iter().map(|s| (*s).into()).collect(),
        selected,
        ..Default::default()
    }
    .into()
}

fn check(id: &str, text: &str, on: bool) -> Widget {
    Check {
        id: id.into(),
        text: text.into(),
        on,
        ..Default::default()
    }
    .into()
}

fn value(label: &str, text: impl Into<String>) -> Widget {
    Widget::Value {
        label: label.into(),
        value: text.into(),
        mono: false,
    }
}

impl MeshWorkbench {
    pub(super) fn open_created(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        command: &str,
        args: CommandArgs,
        answer: Value,
    ) {
        let Some(id) = answer["feature"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(FeatureId)
        else {
            return;
        };
        self.tab.task = Some(Task {
            id,
            created: Some((command.into(), args, answer)),
            pick: false,
        });
        ctx.request(core_document::HostRequest::SelectFeatures(vec![id]));
    }

    pub(super) fn open_edit(&mut self, ctx: &WorkbenchRuntimeContext, id: FeatureId) {
        if self.tab.task.is_some() {
            return;
        }
        if let Some(node) = ctx.document.get_feature_meta(id)
            && node.workbench_id.as_str() == KIND
        {
            self.tab.task = Some(Task {
                id,
                created: None,
                pick: false,
            });
        }
    }

    pub(super) fn step_command(
        &mut self,
        command: &str,
        a: &Args,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let id = FeatureId(a.id("feature")?);
        let node = ctx
            .document
            .get_feature_meta(id)
            .filter(|n| n.workbench_id.as_str() == KIND)
            .ok_or_else(|| CommandError::bad("feature", "is not a mesh step"))?;
        let mut f =
            MeshFeature::from_json(&node.data).map_err(|e| CommandError::failed(e.to_string()))?;
        if command == "mesh.result" {
            let plan = self
                .mesh_plans(ctx.document)
                .into_iter()
                .find(|p| p.body == f.scan)
                .ok_or_else(|| CommandError::failed("the step has no active history"))?;
            let index = plan
                .features
                .iter()
                .position(|v| *v == id)
                .ok_or_else(|| CommandError::failed("the step is after the tip"))?;
            let report = ctx
                .document
                .mesh_steps(&plan)
                .and_then(|steps| steps.get(index))
                .ok_or_else(|| {
                    CommandError::failed("the step has not been built; rebuild first")
                })?;
            return Ok(serde_json::to_value(report).unwrap_or_default());
        }
        if let Some(op) = a.0.get("op") {
            let op: MeshOp = serde_json::from_value(op.clone())
                .map_err(|e| CommandError::bad("op", e.to_string()))?;
            if std::mem::discriminant(&op) != std::mem::discriminant(&f.op) {
                return Err(CommandError::bad("op", "must keep the step's kind"));
            }
            f.op = op;
        }
        if let Some(offset) = a.opt_number("offset")? {
            f.offset = offset;
        }
        ctx.document
            .update_feature_data(id, f.to_json())
            .map_err(|e| CommandError::failed(e.to_string()))?;
        Ok(json!({"feature": id.0, "name": f.name}))
    }

    pub(super) fn task_widgets(&self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        let Some(t) = &self.tab.task else {
            return Vec::new();
        };
        let Some(f) = read(ctx, t.id) else {
            return Vec::new();
        };
        let mm = |id, label, v| number(id, label, v, Dim::Length, Some(0.0));
        let mut out = Vec::new();
        let region = match &f.op {
            MeshOp::Delete { region }
            | MeshOp::Flip { region }
            | MeshOp::Smooth { region, .. }
            | MeshOp::Offset { region, .. } => Some(region),
            _ => None,
        };
        if let Some(region) = region {
            out.push(choice(
                "region",
                "Region",
                &["Whole mesh", "Current marking"],
                usize::from(!matches!(region, MeshRegion::All)),
            ));
        }
        match &f.op {
            MeshOp::RemoveIslands { smaller_than } => out.push(mm(
                "/op/RemoveIslands/smaller_than",
                "Smaller than",
                *smaller_than,
            )),
            MeshOp::Cut { close, bridge, .. } => out.extend([
                number("/offset", "Offset", f.offset, Dim::Length, None),
                mm("/op/Cut/bridge", "Bridge gaps up to", *bridge),
                check("close", "Close flat", *close),
            ]),
            MeshOp::Fill {
                holes_up_to,
                near,
                fairing,
            } => out.extend([
                mm("/op/Fill/holes_up_to", "Holes up to", *holes_up_to),
                choice(
                    "fairing",
                    "Fairing",
                    &["Curved", "Flat"],
                    usize::from(*fairing == MeshFairing::Flat),
                ),
                choice(
                    "holes",
                    "Holes",
                    &["All within size", "One near a point"],
                    usize::from(near.is_some() || t.pick),
                ),
                Widget::Pick {
                    id: "rim".into(),
                    label: "Point near rim".into(),
                    value: near.map(|p| format!("{:.3}, {:.3}, {:.3} mm", p[0], p[1], p[2])),
                    armed: t.pick,
                },
            ]),
            MeshOp::Smooth {
                bumps_up_to,
                move_at_most,
                keep_edges_deg,
                ..
            } => {
                out.extend([
                    mm("/op/Smooth/bumps_up_to", "Bumps up to", *bumps_up_to),
                    mm("/op/Smooth/move_at_most", "Move at most", *move_at_most),
                    check("edges", "Keep sharp edges", keep_edges_deg.is_some()),
                ]);
                if let Some(angle) = keep_edges_deg {
                    out.push(number(
                        "/op/Smooth/keep_edges_deg",
                        "Edge angle",
                        *angle,
                        Dim::Angle,
                        Some(0.0),
                    ));
                }
            }
            MeshOp::Offset { distance, .. } => out.push(number(
                "/op/Offset/distance",
                "Distance",
                *distance,
                Dim::Length,
                None,
            )),
            MeshOp::Boolean {
                operation, chord, ..
            } => out.extend([
                choice(
                    "operation",
                    "Operation",
                    &["Subtract", "Add", "Intersect"],
                    match operation {
                        kernel_api::BooleanOp::Fuse => 1,
                        kernel_api::BooleanOp::Common => 2,
                        _ => 0,
                    },
                ),
                mm("/op/Boolean/chord", "Operand tolerance", *chord),
            ]),
            MeshOp::Shell { thickness, side } => out.extend([
                mm("/op/Shell/thickness", "Thickness", *thickness),
                choice(
                    "side",
                    "Open sheet side",
                    &["Inward", "Outward", "Both"],
                    match side {
                        kernel_api::MeshSide::Inward => 0,
                        kernel_api::MeshSide::Outward => 1,
                        kernel_api::MeshSide::Both => 2,
                    },
                ),
            ]),
            MeshOp::Reduce {
                tolerance,
                triangle_limit,
            } => out.extend([
                mm("/op/Reduce/tolerance", "Tolerance", *tolerance),
                number(
                    "/op/Reduce/triangle_limit",
                    "Triangle limit",
                    *triangle_limit as f64,
                    Dim::Number,
                    Some(1.0),
                ),
            ]),
            _ => {}
        }
        let plan = self
            .mesh_plans(ctx.document)
            .into_iter()
            .find(|p| p.body == f.scan);
        if let Some(plan) = plan {
            let steps = ctx.document.mesh_steps(&plan);
            let rows = plan
                .features
                .iter()
                .enumerate()
                .map(|(k, id)| {
                    vec![
                        ctx.document
                            .get_feature_meta(*id)
                            .map(|n| n.name.clone())
                            .unwrap_or_default(),
                        steps
                            .and_then(|s| s.get(k))
                            .map(|s| s.faces.to_string())
                            .unwrap_or_else(|| "—".into()),
                    ]
                })
                .collect();
            out.push(Widget::Table {
                id: "history".into(),
                columns: vec!["Step".into(), "Triangles".into()],
                rows,
                selected: None,
                editable: Vec::new(),
            });
            if let Some(k) = plan.features.iter().position(|id| *id == t.id)
                && let Some(step) = steps.and_then(|s| s.get(k))
            {
                out.push(Widget::Text {
                    text: step.note.clone(),
                    mono: false,
                });
                let mm = |v| core_document::units::show_mm(v, ctx.document.display_unit());
                match &step.report {
                    Some(MeshStepReport::Smooth { max, rms }) => out.extend([
                        value("Max movement", mm(*max)),
                        value("RMS movement", mm(*rms)),
                    ]),
                    Some(MeshStepReport::Reduce {
                        within_share,
                        p999,
                        furthest,
                        max,
                        rms,
                        ..
                    }) => out.extend([
                        value("Within tolerance", format!("{:.3} %", within_share * 100.0)),
                        value("99.9% distance", mm(*p999)),
                        value("Furthest distance", mm(*max)),
                        value(
                            "Furthest point",
                            format!(
                                "{}, {}, {}",
                                mm(furthest[0]),
                                mm(furthest[1]),
                                mm(furthest[2])
                            ),
                        ),
                        value("RMS", mm(*rms)),
                    ]),
                    Some(
                        MeshStepReport::Boolean { volume, .. }
                        | MeshStepReport::Shell { volume, .. },
                    ) => out.push(value("Volume", format!("{volume:.3} mm³"))),
                    Some(MeshStepReport::Cut {
                        capped_loops,
                        open_loops,
                        ..
                    }) => out.extend([
                        value("Cap loops", capped_loops.to_string()),
                        value("Open loops", open_loops.to_string()),
                    ]),
                    None => {}
                }
            }
        }
        if let Some(error) = ctx
            .document
            .get_feature_meta(t.id)
            .and_then(|n| n.error.as_ref())
        {
            out.push(Widget::Text {
                text: error.clone(),
                mono: false,
            });
        }
        out
    }

    pub(super) fn task_event(&mut self, ctx: &mut WorkbenchRuntimeContext, event: &PanelEvent) {
        let Some(t) = &mut self.tab.task else { return };
        let Some(mut f) = read(ctx, t.id) else { return };
        match event {
            PanelEvent::Number { id, value } => {
                let mut data = f.to_json();
                let Some(slot) = data.pointer_mut(id) else {
                    return;
                };
                *slot = if id.ends_with("triangle_limit") {
                    json!(value.max(1.0).round() as u64)
                } else {
                    json!(value)
                };
                if let Ok(next) = MeshFeature::from_json(&data) {
                    f = next;
                } else {
                    return;
                }
            }
            PanelEvent::Choice { id, index } if id == "operation" => {
                if let MeshOp::Boolean { operation, .. } = &mut f.op {
                    *operation = match index {
                        1 => kernel_api::BooleanOp::Fuse,
                        2 => kernel_api::BooleanOp::Common,
                        _ => kernel_api::BooleanOp::Cut,
                    };
                }
            }
            PanelEvent::Choice { id, index } if id == "side" => {
                if let MeshOp::Shell { side, .. } = &mut f.op {
                    *side = match index {
                        0 => kernel_api::MeshSide::Inward,
                        1 => kernel_api::MeshSide::Outward,
                        _ => kernel_api::MeshSide::Both,
                    };
                }
            }
            PanelEvent::Choice {
                id,
                index: selected,
            } if id == "fairing" => {
                if let MeshOp::Fill { fairing, .. } = &mut f.op {
                    *fairing = if *selected == 0 {
                        MeshFairing::Curved
                    } else {
                        MeshFairing::Flat
                    };
                }
            }
            PanelEvent::Choice {
                id,
                index: selected,
            } if id == "holes" => {
                t.pick = *selected == 1;
                if let MeshOp::Fill { near, .. } = &mut f.op {
                    *near = None;
                }
            }
            PanelEvent::Pick { id } if id == "rim" => t.pick = true,
            PanelEvent::Choice {
                id,
                index: selected,
            } if id == "region" => {
                let r = if *selected == 0 {
                    MeshRegion::All
                } else {
                    match region(&Args(&CommandArgs::new()), ctx, f.scan, false) {
                        Ok(r) => r,
                        Err(e) => {
                            ctx.log_warn(e.to_string());
                            return;
                        }
                    }
                };
                match &mut f.op {
                    MeshOp::Delete { region }
                    | MeshOp::Flip { region }
                    | MeshOp::Smooth { region, .. }
                    | MeshOp::Offset { region, .. } => *region = r,
                    _ => return,
                }
            }
            PanelEvent::Toggle { id, on } if id == "edges" => {
                if let MeshOp::Smooth { keep_edges_deg, .. } = &mut f.op {
                    *keep_edges_deg = on.then_some(30.0);
                }
            }
            PanelEvent::Toggle { id, on } if id == "close" => {
                if let MeshOp::Cut { close, .. } = &mut f.op {
                    *close = *on;
                }
            }
            _ => return,
        }
        if let Err(e) = ctx.document.update_feature_data(t.id, f.to_json()) {
            ctx.log_warn(e.to_string());
        }
    }

    pub(super) fn pick_rim(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        let Some(t) = &mut self.tab.task else {
            return false;
        };
        if !t.pick
            || !matches!(
                event,
                WorkbenchInputEvent::MousePress {
                    button: core_document::MouseButton::Left,
                    ..
                }
            )
        {
            return false;
        }
        let Some(mut f) = read(ctx, t.id) else {
            return false;
        };
        if ctx.hovered_body_id != Some(f.scan.0) {
            return false;
        }
        let Some(p) = ctx.hovered_world_pos else {
            return false;
        };
        if let MeshOp::Fill { near, .. } = &mut f.op {
            *near = Some(
                ctx.document
                    .body_placement(f.scan)
                    .inverse()
                    .point(p)
                    .map(f64::from),
            );
            t.pick = false;
            if let Err(e) = ctx.document.update_feature_data(t.id, f.to_json()) {
                ctx.log_warn(e.to_string());
            }
            return true;
        }
        false
    }

    pub(super) fn close_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if !request.accept && !request.cancel {
            return TaskOutcome::Open;
        }
        let Some(t) = self.tab.task.take() else {
            return TaskOutcome::Open;
        };
        let Some(f) = read(ctx, t.id) else {
            return TaskOutcome::Cancelled;
        };
        // Cancel's edits, the step and a start it set, are taken back by
        // the host.
        if request.cancel {
            return TaskOutcome::Cancelled;
        }
        if let Some((id, mut args, answer)) = t.created {
            args.insert("scan".into(), json!(f.scan.0));
            match &f.op {
                MeshOp::Delete { region }
                | MeshOp::Flip { region }
                | MeshOp::Smooth { region, .. }
                | MeshOp::Offset { region, .. } => {
                    args.insert("region".into(), json!(region));
                }
                _ => {}
            }
            ctx.record(&id, args, answer);
        }
        let args = json!({"feature": t.id.0, "op": f.op, "offset": f.offset})
            .as_object()
            .unwrap()
            .clone();
        ctx.record("mesh.set", args, json!({"feature": t.id.0, "name": f.name}));
        TaskOutcome::Accepted { label: f.name }
    }
}
