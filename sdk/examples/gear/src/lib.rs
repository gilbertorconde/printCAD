//! Gears: a workbench package that makes spur gears. A gear is a feature
//! of its own kind with a tooth count, a module, a thickness and a bore;
//! its solid is an involute tooth outline extruded. It shows each part of
//! the interface: a tool, a command, a task panel whose numbers take
//! formulas, world-space drawing, a job with progress, a menu entry and a
//! settings page.

use printcad_bench_sdk::api::kernel_api::{
    BooleanOp, ExtrudeTermination, Profile, ProfilePlane, ProfileSegment, ProfileWire, SweepKind,
};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json};

const KIND: &str = "example.gear.gear";
const NEW: &str = "example.gear.new";
const MAKE: &str = "example.gear.make";
const TABLE: &str = "tooth-table";

/// A gear's data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Gear {
    teeth: u32,
    /// Millimetres of pitch diameter per tooth.
    module: f64,
    thickness: f64,
    bore: f64,
}

impl Gear {
    fn from(data: &Value) -> Option<Gear> {
        printcad_bench_sdk::serde_json::from_value(data.clone()).ok()
    }

    fn to_value(&self) -> Value {
        printcad_bench_sdk::serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn pitch_radius(&self) -> f64 {
        self.module * self.teeth as f64 / 2.0
    }

    fn problem(&self) -> Option<String> {
        if self.teeth < 6 {
            return Some("A gear needs at least 6 teeth.".into());
        }
        if self.module <= 0.0 || self.thickness <= 0.0 {
            return Some("The module and the thickness must be more than 0.".into());
        }
        let root = self.pitch_radius() - 1.25 * self.module;
        if self.bore < 0.0 || self.bore / 2.0 >= root * 0.9 {
            return Some("The bore must fit inside the teeth's roots.".into());
        }
        None
    }

    /// The tooth outline: involute flanks, 20° pressure angle.
    fn outline(&self) -> Vec<[f64; 2]> {
        let z = self.teeth as f64;
        let r = self.pitch_radius();
        let base = r * 20f64.to_radians().cos();
        let tip = r + self.module;
        let root = r - 1.25 * self.module;
        let inv = |radius: f64| {
            let a = (base / radius).min(1.0).acos();
            a.tan() - a
        };
        let half = std::f64::consts::PI / (2.0 * z) + inv(r);
        let start = base.max(root);
        let steps = 6;
        let mut points = Vec::new();
        for i in 0..self.teeth {
            let centre = std::f64::consts::TAU * i as f64 / z;
            let at = |radius: f64, angle: f64| [radius * angle.cos(), radius * angle.sin()];
            if root < start {
                points.push(at(root, centre - (half - inv(start))));
            }
            for s in 0..=steps {
                let radius = start + (tip - start) * s as f64 / steps as f64;
                points.push(at(radius, centre - (half - inv(radius))));
            }
            for s in (0..=steps).rev() {
                let radius = start + (tip - start) * s as f64 / steps as f64;
                points.push(at(radius, centre + (half - inv(radius))));
            }
            if root < start {
                points.push(at(root, centre + (half - inv(start))));
            }
        }
        points
    }
}

#[derive(Default)]
struct Gears {
    /// The gear whose task is open, and its data when the task opened.
    editing: Option<(String, Gear)>,
    /// The gear the tool just made: Cancel removes it.
    fresh: bool,
    /// The active feature last seen, so selecting a gear opens it once.
    seen_active: Option<String>,
    job: Option<u64>,
    table: Vec<Vec<String>>,
    defaults: Defaults,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Defaults {
    teeth: u32,
    module: f64,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            teeth: 20,
            module: 2.0,
        }
    }
}

impl Gears {
    fn new_gear(&self) -> Gear {
        Gear {
            teeth: self.defaults.teeth,
            module: self.defaults.module,
            thickness: 8.0,
            bore: 5.0,
        }
    }

    /// Make a body with a gear on it; the gear's id.
    fn make(&self, gear: &Gear) -> Result<(String, String), String> {
        let body = host::create_body(Some("Gear"))?;
        let name = format!("Gear z{}", gear.teeth);
        let feature = host::add_feature(KIND, &name, Some(&body), gear.to_value())?;
        Ok((body, feature))
    }

    fn open(&mut self, id: &str) {
        if let Some(node) = host::feature(id)
            && node.kind == KIND
            && let Some(gear) = Gear::from(&node.data)
        {
            self.editing = Some((id.to_string(), gear));
            self.fresh = false;
            self.table.clear();
        }
    }

    fn edited(&self) -> Option<(String, Gear)> {
        let (id, _) = self.editing.as_ref()?;
        let node = host::feature(id)?;
        Some((id.clone(), Gear::from(&node.data)?))
    }
}

impl Bench for Gears {
    fn describe(&self) -> Registration {
        Registration {
            label: "Gears".into(),
            description: "Spur gears from a tooth count and a module".into(),
            icon: "gear".into(),
            tools: vec![Tool {
                id: NEW.into(),
                label: "New gear".into(),
                icon: "gear".into(),
                behavior: ToolBehavior::Action,
                shortcuts: vec!["G".into()],
                ..Default::default()
            }],
            commands: vec![Command {
                id: MAKE.into(),
                summary: "Make a spur gear on a body of its own".into(),
                params: vec![
                    Param {
                        name: "teeth".into(),
                        kind: ParamKind::Integer,
                        required: true,
                        doc: "how many teeth".into(),
                    },
                    Param {
                        name: "module".into(),
                        kind: ParamKind::Number,
                        required: true,
                        doc: "pitch diameter per tooth, mm".into(),
                    },
                    Param {
                        name: "thickness".into(),
                        kind: ParamKind::Number,
                        required: false,
                        doc: "mm, 8 when left out".into(),
                    },
                    Param {
                        name: "bore".into(),
                        kind: ParamKind::Number,
                        required: false,
                        doc: "hole diameter, mm, 5 when left out".into(),
                    },
                ],
                returns: "{body, feature}".into(),
                read_only: false,
            }],
            length_keys: vec!["module".into(), "thickness".into(), "bore".into()],
            ..Default::default()
        }
    }

    fn feature_info(&self, node: &Node) -> FeatureInfo {
        let label = match Gear::from(&node.data) {
            Some(gear) => format!("Spur gear, {} teeth", gear.teeth),
            None => "Spur gear".into(),
        };
        FeatureInfo {
            icon: "gear".into(),
            kind_label: label,
            family_label: "Gear".into(),
            builds_solid: true,
        }
    }

    fn parameters(&self, _node: &Node) -> Vec<Parameter> {
        let p = |key: &str, label: &str, dim: Dim| Parameter {
            key: format!("/{key}"),
            name: Some(key.into()),
            label: label.into(),
            dim,
            pointer: format!("/{key}"),
            scale: 1.0,
            integer: false,
        };
        vec![
            Parameter {
                integer: true,
                ..p("teeth", "Teeth", Dim::Number)
            },
            p("module", "Module", Dim::Length),
            p("thickness", "Thickness", Dim::Length),
            p("bore", "Bore", Dim::Length),
        ]
    }

    fn rebuild(&mut self, request: RebuildRequest) -> Vec<Rebuild> {
        request
            .bodies
            .into_iter()
            .map(|history| {
                let mut ops = Vec::new();
                let mut op_features = Vec::new();
                for node in &history.features {
                    let Some(gear) = Gear::from(&node.data) else {
                        return Rebuild {
                            body: history.body,
                            plan: Plan::Error {
                                feature: Some(node.id.clone()),
                                message: "The gear's data does not read.".into(),
                            },
                        };
                    };
                    if let Some(problem) = gear.problem() {
                        return Rebuild {
                            body: history.body,
                            plan: Plan::Error {
                                feature: Some(node.id.clone()),
                                message: problem,
                            },
                        };
                    }
                    let outline = gear.outline();
                    let mut segments: Vec<ProfileSegment> = outline
                        .iter()
                        .zip(outline.iter().cycle().skip(1))
                        .map(|(a, b)| ProfileSegment::Line { start: *a, end: *b })
                        .collect();
                    segments.retain(|s| match s {
                        ProfileSegment::Line { start, end } => start != end,
                        _ => true,
                    });
                    let mut wires = vec![ProfileWire { segments }];
                    if gear.bore > 0.0 {
                        wires.push(ProfileWire {
                            segments: vec![ProfileSegment::Circle {
                                center: [0.0, 0.0],
                                radius: gear.bore / 2.0,
                            }],
                        });
                    }
                    ops.push(SolidOp::Sweep {
                        profile: Profile {
                            plane: ProfilePlane {
                                origin: [0.0; 3],
                                x_axis: [1.0, 0.0, 0.0],
                                y_axis: [0.0, 1.0, 0.0],
                                normal: [0.0, 0.0, 1.0],
                            },
                            wires,
                        },
                        kind: SweepKind::Extrude {
                            termination: ExtrudeTermination::Blind {
                                distance: gear.thickness,
                            },
                            second_side: None,
                            symmetric: false,
                            reversed: false,
                            taper_deg: 0.0,
                            direction: None,
                        },
                        op: if ops.is_empty() {
                            BooleanOp::NewSolid
                        } else {
                            BooleanOp::Fuse
                        },
                    });
                    op_features.push(node.id.clone());
                }
                let plan = if ops.is_empty() {
                    Plan::Empty
                } else {
                    Plan::Ops { ops, op_features }
                };
                Rebuild {
                    body: history.body,
                    plan,
                }
            })
            .collect()
    }

    fn run_command(&mut self, id: &str, args: Value) -> Result<Value, String> {
        if id != MAKE {
            return Err(format!("no command `{id}`"));
        }
        let number =
            |key: &str, default: f64| args.get(key).and_then(Value::as_f64).unwrap_or(default);
        let gear = Gear {
            teeth: args.get("teeth").and_then(Value::as_u64).unwrap_or(20) as u32,
            module: number("module", 2.0),
            thickness: number("thickness", 8.0),
            bore: number("bore", 5.0),
        };
        if let Some(problem) = gear.problem() {
            return Err(problem);
        }
        let (body, feature) = self.make(&gear)?;
        Ok(json!({"body": body, "feature": feature}))
    }

    fn input(&mut self, input: &Input) -> bool {
        match &input.event {
            Event::ToolActivated if input.tool.as_deref() == Some(NEW) => {
                let gear = self.new_gear();
                match self.make(&gear) {
                    Ok((body, feature)) => {
                        host::request(Request::SelectBody { body });
                        host::request(Request::JournalLabel {
                            label: "New gear".into(),
                        });
                        self.editing = Some((feature, gear));
                        self.fresh = true;
                        self.table.clear();
                    }
                    Err(e) => host::error(&format!("No gear: {e}")),
                }
                true
            }
            Event::JobFinished { job, result } if Some(*job) == self.job => {
                self.job = None;
                match result {
                    Ok(json) => {
                        self.table =
                            printcad_bench_sdk::serde_json::from_str(json).unwrap_or_default();
                    }
                    Err(e) => host::warn(&format!("The tooth table stopped: {e}")),
                }
                true
            }
            Event::Key { key, down: true } if key == "Escape" && self.editing.is_some() => {
                self.task_close(false);
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, pointer: &Pointer) -> Frame {
        if pointer.active_feature != self.seen_active {
            self.seen_active = pointer.active_feature.clone();
            if self.editing.is_none()
                && let Some(id) = pointer.active_feature.clone()
            {
                self.open(&id);
            }
        }
        let mut frame = Frame::default();
        let Some((id, gear)) = self.edited() else {
            self.editing = None;
            return frame;
        };
        let body = host::feature(&id).and_then(|n| n.body);
        let place = body
            .and_then(|b| host::bodies().into_iter().find(|x| x.id == b))
            .map(|b| b.placement)
            .unwrap_or([
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]);
        let world = |x: f64, y: f64, z: f64| -> [f32; 3] {
            let m = &place;
            [
                (m[0] * x + m[1] * y + m[2] * z + m[3]) as f32,
                (m[4] * x + m[5] * y + m[6] * z + m[7]) as f32,
                (m[8] * x + m[9] * y + m[10] * z + m[11]) as f32,
            ]
        };
        let r = gear.pitch_radius();
        let top = gear.thickness;
        frame.lines.push(Polyline {
            points: (0..96)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / 96.0;
                    world(r * a.cos(), r * a.sin(), top)
                })
                .collect(),
            color: [0.31, 0.64, 0.9],
            width: 1.5,
            dashed: true,
            closed: true,
        });
        frame.labels.push(Label {
            at: world(0.0, 0.0, top),
            text: format!("z {} · m {}", gear.teeth, gear.module),
            color: [0.9, 0.92, 0.94],
            size: 12.0,
            pill: true,
            mono: true,
        });
        frame.hud.tool = Some(ToolHint {
            icon: "gear".into(),
            name: "Gear".into(),
            prompt: "Set its teeth and module in the panel".into(),
            keys: vec![("Esc".into(), "cancel".into())],
        });
        frame.status.selection = Some(format!(
            "Gear: {} teeth, pitch diameter {:.2} mm",
            gear.teeth,
            2.0 * r
        ));
        frame.editing = Some(id.clone());
        frame.task = Some(Task {
            title: "Gear".into(),
            icon: "gear".into(),
            confirmable: true,
        });
        let bind = |key: &str| {
            Some(Bind {
                feature: id.clone(),
                key: format!("/{key}"),
            })
        };
        let number =
            |key: &str, label: &str, value: f64, dim: Dim, decimals: usize| Widget::Number {
                id: key.into(),
                label: label.into(),
                value,
                dim,
                bind: bind(key),
                min: Some(0.0),
                max: None,
                decimals,
                error: None,
            };
        frame.panel = vec![
            number("teeth", "Teeth", gear.teeth as f64, Dim::Number, 0),
            number("module", "Module", gear.module, Dim::Length, 2),
            number("thickness", "Thickness", gear.thickness, Dim::Length, 2),
            number("bore", "Bore", gear.bore, Dim::Length, 2),
            Widget::Text {
                text: format!(
                    "Pitch diameter {:.2} mm, outside {:.2} mm",
                    2.0 * r,
                    2.0 * (r + gear.module)
                ),
                mono: true,
            },
        ];
        if let Some(problem) = gear.problem() {
            frame.panel.push(Widget::Note {
                kind: NoteKind::Error,
                title: None,
                text: problem,
            });
        }
        frame.panel.push(Widget::Separator);
        match self.job {
            Some(job) => {
                frame.panel.push(Widget::Progress {
                    label: "Working out the tooth table".into(),
                    fraction: None,
                    job: Some(job),
                });
                frame.panel.push(Widget::Button {
                    id: "stop".into(),
                    label: "Stop".into(),
                    style: ButtonStyle::Secondary,
                    enabled: true,
                });
            }
            None => frame.panel.push(Widget::Button {
                id: "table".into(),
                label: "Tooth table".into(),
                style: ButtonStyle::Secondary,
                enabled: gear.problem().is_none(),
            }),
        }
        if !self.table.is_empty() {
            frame.panel.push(Widget::Table {
                id: "table".into(),
                columns: vec!["Radius".into(), "Tooth width".into()],
                rows: self.table.clone(),
                selected: None,
            });
            frame.panel.push(Widget::Button {
                id: "save".into(),
                label: "Save as CSV".into(),
                style: ButtonStyle::Secondary,
                enabled: true,
            });
        }
        frame
    }

    fn panel_event(&mut self, slot: PanelSlot, event: PanelEvent) {
        if slot == PanelSlot::Settings {
            match event {
                PanelEvent::Number { id, value } if id == "teeth" => {
                    self.defaults.teeth = value.round().max(6.0) as u32
                }
                PanelEvent::Number { id, value } if id == "module" && value > 0.0 => {
                    self.defaults.module = value
                }
                _ => {}
            }
            return;
        }
        let Some((id, mut gear)) = self.edited() else {
            return;
        };
        match event {
            PanelEvent::Number { id: field, value } => {
                match field.as_str() {
                    "teeth" => gear.teeth = value.round().max(0.0) as u32,
                    "module" => gear.module = value,
                    "thickness" => gear.thickness = value,
                    "bore" => gear.bore = value,
                    _ => return,
                }
                if let Err(e) = host::set_feature_data(&id, gear.to_value()) {
                    host::error(&e);
                }
            }
            PanelEvent::Button { id: button } if button == "table" => {
                let input =
                    printcad_bench_sdk::serde_json::to_string(&gear.to_value()).unwrap_or_default();
                match host::start_job(TABLE, &input) {
                    Ok(job) => self.job = Some(job),
                    Err(e) => host::error(&e),
                }
            }
            PanelEvent::Button { id: button } if button == "stop" => {
                if let Some(job) = self.job {
                    host::cancel_job(job);
                }
            }
            PanelEvent::Button { id: button } if button == "save" => {
                let mut csv = String::from("radius_mm,tooth_width_mm\n");
                for row in &self.table {
                    csv.push_str(&row.join(","));
                    csv.push('\n');
                }
                host::request(Request::SaveFile {
                    name: format!("gear-z{}.csv", gear.teeth),
                    kind: "CSV".into(),
                    extension: "csv".into(),
                    contents: csv.into_bytes(),
                });
            }
            _ => {}
        }
    }

    fn task_close(&mut self, accept: bool) -> Option<String> {
        let (id, opened) = self.editing.take()?;
        if let Some(job) = self.job.take() {
            host::cancel_job(job);
        }
        self.table.clear();
        if accept {
            return Some("Edit gear".into());
        }
        let result = if self.fresh {
            host::remove_feature(&id)
        } else {
            host::set_feature_data(&id, opened.to_value())
        };
        if let Err(e) = result {
            host::error(&e);
        }
        None
    }

    fn menu_items(&mut self, scope: &MenuScope) -> Vec<MenuItem> {
        let item = |id: &str, label: &str| MenuItem {
            id: id.into(),
            label: label.into(),
            icon: Some("gear".into()),
            hint: None,
            enabled: true,
            separator_before: false,
        };
        match scope {
            MenuScope::StartPage => vec![MenuItem {
                hint: Some("A spur gear to change and print".into()),
                ..item("example.gear.start", "New gear")
            }],
            MenuScope::TreeFeature(id) if host::feature(id).is_some_and(|n| n.kind == KIND) => {
                vec![item("example.gear.edit", "Edit gear")]
            }
            _ => Vec::new(),
        }
    }

    fn menu_command(&mut self, id: &str, scope: &MenuScope) -> bool {
        match (id, scope) {
            ("example.gear.start", _) => {
                let gear = self.new_gear();
                if let Ok((body, _)) = self.make(&gear) {
                    host::request(Request::SelectBody { body });
                }
                true
            }
            ("example.gear.edit", MenuScope::TreeFeature(feature)) => {
                self.open(feature);
                true
            }
            _ => false,
        }
    }

    fn settings_panel(&mut self) -> Vec<Widget> {
        vec![
            Widget::Heading {
                text: "New gears".into(),
            },
            Widget::Number {
                id: "teeth".into(),
                label: "Teeth".into(),
                value: self.defaults.teeth as f64,
                dim: Dim::Number,
                bind: None,
                min: Some(6.0),
                max: Some(400.0),
                decimals: 0,
                error: None,
            },
            Widget::Number {
                id: "module".into(),
                label: "Module".into(),
                value: self.defaults.module,
                dim: Dim::Length,
                bind: None,
                min: Some(0.1),
                max: None,
                decimals: 2,
                error: None,
            },
        ]
    }

    fn settings(&self) -> Option<Value> {
        printcad_bench_sdk::serde_json::to_value(&self.defaults).ok()
    }

    fn apply_settings(&mut self, settings: Value) {
        if let Ok(defaults) = printcad_bench_sdk::serde_json::from_value(settings) {
            self.defaults = defaults;
        }
    }

    fn job(entry: &str, input: &str) -> Result<String, String> {
        if entry != TABLE {
            return Err(format!("no job `{entry}`"));
        }
        let gear: Gear =
            printcad_bench_sdk::serde_json::from_str(input).map_err(|e| e.to_string())?;
        let z = gear.teeth as f64;
        let r = gear.pitch_radius();
        let base = r * 20f64.to_radians().cos();
        let tip = r + gear.module;
        let inv = |radius: f64| {
            let a = (base / radius).min(1.0).acos();
            a.tan() - a
        };
        let half = std::f64::consts::PI / (2.0 * z) + inv(r);
        let rows = 12u64;
        let mut table = Vec::new();
        for i in 0..=rows {
            if host::cancelled() {
                return Err("stopped".into());
            }
            let radius = base.max(r - 1.25 * gear.module) + (tip - base) * i as f64 / rows as f64;
            let radius = radius.min(tip);
            // The chord across the tooth at this radius.
            let width = 2.0 * radius * (half - inv(radius.max(base))).sin();
            table.push(vec![
                format!("{radius:.3}"),
                format!("{:.3}", width.max(0.0)),
            ]);
            host::progress(i + 1, rows + 1);
        }
        printcad_bench_sdk::serde_json::to_string(&table).map_err(|e| e.to_string())
    }
}

bench!(Gears);
