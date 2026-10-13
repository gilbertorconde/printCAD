//! A package reaching what the examples leave out, for the host's tests: a
//! sketch's profile read in world space, a long job that its task's Escape
//! stops, an editable table cell, a number a formula drives, a file
//! handed to the save dialog, the host's geometry questions and a rod
//! built with a cosmetic thread.

#![allow(clippy::needless_update)]

use printcad_bench_sdk::api::kernel_api::{BooleanOp, Placement, PrimitiveKind};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json, serde_json};

const KIND: &str = "test.probe.thing";

#[derive(Default)]
struct Probe {
    /// The thing whose task is open.
    editing: Option<String>,
    /// The job the task started, until it is heard of.
    job: Option<u64>,
    /// How the last job stands: `running`, `stopped`, `finished`, `failed`.
    state: Option<&'static str>,
    /// The settings table's one cell.
    cell: String,
}

impl Bench for Probe {
    fn describe(&self) -> Registration {
        let command = |id: &str| Command {
            id: format!("test.probe.{id}"),
            summary: id.into(),
            ..Default::default()
        };
        Registration {
            label: "Probe".into(),
            commands: [
                "profile", "add", "size", "state", "cell", "save", "geom", "rod",
            ]
            .into_iter()
            .map(command)
            .collect(),
            ..Default::default()
        }
    }

    fn feature_info(&self, node: &Node) -> FeatureInfo {
        FeatureInfo {
            kind_label: "Thing".into(),
            builds_solid: node.data.get("rod").is_some(),
            ..Default::default()
        }
    }

    /// A rod is a cylinder standing on its base, threaded from its top.
    fn rebuild(&mut self, request: RebuildRequest) -> Vec<Rebuild> {
        request
            .bodies
            .into_iter()
            .filter_map(|history| {
                let node = history
                    .features
                    .iter()
                    .find(|n| n.data.get("rod").is_some())?;
                let number = |key: &str| node.data["rod"][key].as_f64().unwrap_or_default();
                let (radius, height) = (number("radius"), number("height"));
                Some(Rebuild {
                    body: history.body.clone(),
                    plan: Plan::Ops {
                        ops: vec![SolidOp::Primitive {
                            kind: PrimitiveKind::Cylinder {
                                radius,
                                height,
                                angle_deg: 360.0,
                            },
                            placement: Placement::default(),
                            op: BooleanOp::NewSolid,
                        }],
                        op_features: vec![node.id.clone()],
                    },
                    threads: vec![CosmeticThread {
                        feature: node.id.clone(),
                        point: [radius, 0.0, height],
                        pitch: number("pitch"),
                        length: number("length"),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
            })
            .collect()
    }

    fn parameters(&self, _node: &Node) -> Vec<Parameter> {
        vec![Parameter {
            key: "/size".into(),
            name: Some("size".into()),
            label: "Size".into(),
            dim: Dim::Length,
            pointer: "/size".into(),
            ..Default::default()
        }]
    }

    fn run_command(&mut self, id: &str, args: Value) -> Result<Value, String> {
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        match id {
            "test.probe.profile" => {
                let profile = host::profile(text("sketch")).ok_or("no closed profile")?;
                serde_json::to_value(profile).map_err(|e| e.to_string())
            }
            "test.probe.add" => {
                let size = args.get("size").cloned().unwrap_or(json!(1.0));
                host::add_feature(KIND, "Thing", None, json!({ "size": size })).map(|f| json!(f))
            }
            "test.probe.size" => {
                let node = host::feature(text("id")).ok_or("no such thing")?;
                Ok(node.data["size"].clone())
            }
            "test.probe.geom" => host::call(
                text("command"),
                args.get("args").cloned().unwrap_or(Value::Null),
            ),
            // A rod in a body of its own standing at (100, 0, 0).
            "test.probe.rod" => {
                let body = host::create_body(Some("Rod"))?;
                let rod = json!({
                    "radius": args.get("radius").cloned().unwrap_or(json!(5.0)),
                    "height": args.get("height").cloned().unwrap_or(json!(20.0)),
                    "pitch": args.get("pitch").cloned().unwrap_or(json!(1.0)),
                    "length": args.get("length").cloned().unwrap_or(json!(0.0)),
                });
                let feature = host::add_feature(KIND, "Rod", Some(&body), json!({ "rod": rod }))?;
                let mut matrix = [0.0; 16];
                for i in 0..4 {
                    matrix[i * 5] = 1.0;
                }
                matrix[3] = 100.0;
                host::call(
                    calls::SET_PLACEMENT,
                    json!({"body": body, "matrix": matrix.to_vec()}),
                )?;
                Ok(json!({"body": body, "feature": feature}))
            }
            "test.probe.state" => Ok(json!(self.state.unwrap_or("none"))),
            "test.probe.cell" => Ok(json!(self.cell)),
            "test.probe.save" => {
                host::request(Request::SaveFile {
                    name: "probe.txt".into(),
                    kind: "Text".into(),
                    extension: "txt".into(),
                    contents: text("text").as_bytes().to_vec(),
                });
                Ok(Value::Null)
            }
            _ => Err(format!("no command `{id}`")),
        }
    }

    fn input(&mut self, input: &Input) -> bool {
        match &input.event {
            Event::EditFeature { feature } => {
                self.editing = Some(feature.clone());
                self.job = host::start_job("spin", "").ok();
                self.state = Some("running");
                true
            }
            Event::JobFinished { job, result } if Some(*job) == self.job => {
                self.job = None;
                self.state = Some(match result {
                    Ok(_) => "finished",
                    Err(e) if e == "stopped" => {
                        host::warn("The job was stopped.");
                        "stopped"
                    }
                    Err(_) => "failed",
                });
                true
            }
            Event::Key { key, down: true } if key == "Escape" && self.editing.is_some() => {
                self.task_close(false);
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, _pointer: &Pointer) -> Frame {
        let mut frame = Frame::default();
        if let Some(id) = &self.editing {
            frame.editing = Some(id.clone());
            frame.task = Some(Task {
                title: "Probe".into(),
                confirmable: true,
                ..Default::default()
            });
            frame.panel.push(Widget::Progress {
                label: "Spinning".into(),
                fraction: None,
                job: self.job,
            });
        }
        frame
    }

    fn panel_event(&mut self, slot: PanelSlot, event: PanelEvent) {
        if let (PanelSlot::Settings, PanelEvent::Cell { id, value, .. }) = (slot, event)
            && id == "cells"
        {
            self.cell = value;
        }
    }

    fn task_close(&mut self, accept: bool) -> Option<String> {
        self.editing.take()?;
        if let Some(job) = self.job {
            host::cancel_job(job);
        }
        accept.then(|| "Probe".into())
    }

    fn settings_panel(&mut self) -> Vec<Widget> {
        vec![Widget::Table {
            id: "cells".into(),
            columns: vec!["Value".into()],
            rows: vec![vec![self.cell.clone()]],
            selected: None,
            editable: vec![true],
        }]
    }

    /// Spins until stopped: far longer than any test waits.
    fn job(_entry: &str, _input: &str) -> Result<String, String> {
        let steps = 1_000_000_000u64;
        let mut sum = 0u64;
        for i in 0..steps {
            if host::cancelled() {
                return Err("stopped".into());
            }
            for k in 0..1000u64 {
                sum = std::hint::black_box(sum.wrapping_add(i ^ k));
            }
            host::progress(i + 1, steps);
        }
        Ok(sum.to_string())
    }
}

bench!(Probe);
