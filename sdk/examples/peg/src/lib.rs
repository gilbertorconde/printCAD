//! Pegs: a workbench package showing a tool's whole gesture. A drag with
//! the Peg tool previews a round peg standing where the press went down,
//! its diameter following the pointer; the release opens a floating card
//! for its diameter and height, the preview following what is typed. OK
//! writes the peg, which the app makes one undo step; Cancel, Escape, a
//! tool change or a tab closing leave the document as it was, since
//! nothing is written before OK. A double click on a peg's row opens the
//! same card on it.

// Every literal of the contract's structs ends in `..Default::default()`,
// even one naming every field, so a field a later 0.1.x adds leaves the
// package building.
#![allow(clippy::needless_update)]

use printcad_bench_sdk::api::kernel_api::{BooleanOp, Placement, PrimitiveKind};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json, serde_json};

const KIND: &str = "example.peg.peg";
const PLACE: &str = "example.peg.place";
const MAKE: &str = "example.peg.make";
const DIAMETER: &str = "diameter";
const HEIGHT: &str = "height";
const PREVIEW: Rgb = [0.35, 0.75, 1.0];

/// A peg's data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Peg {
    /// The middle of its foot, in the body's frame.
    at: [f64; 3],
    diameter: f64,
    height: f64,
}

impl Peg {
    fn from(data: &Value) -> Option<Peg> {
        serde_json::from_value(data.clone()).ok()
    }

    fn problem(&self) -> Option<&'static str> {
        (self.diameter <= 0.0 || self.height <= 0.0)
            .then_some("The diameter and the height must be more than 0.")
    }

    /// Make a body with this peg on it; the body's id and the peg's.
    fn make(&self) -> Result<(String, String), String> {
        if let Some(problem) = self.problem() {
            return Err(problem.into());
        }
        let data = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let body = host::create_body(Some("Peg"))?;
        let feature = host::add_feature(KIND, "Peg", Some(&body), data)?;
        Ok((body, feature))
    }
}

/// The peg the card holds, as typed: nothing of it is in the document
/// until OK.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Draft {
    /// The card's id, new for each gesture, so a card naming another is
    /// one this draft never showed.
    card: String,
    /// The peg it edits; none for a new one.
    peg: Option<String>,
    at: [f32; 3],
    diameter: String,
    height: String,
}

impl Draft {
    fn peg(&self) -> Option<Peg> {
        let number = |text: &str| text.trim().parse::<f64>().ok();
        let peg = Peg {
            at: self.at.map(f64::from),
            diameter: number(&self.diameter)?,
            height: number(&self.height)?,
        };
        peg.problem().is_none().then_some(peg)
    }

    /// OK: the new peg made, or the edited one's data set.
    fn write(&self) -> Result<(), String> {
        let peg = self
            .peg()
            .ok_or("The diameter and the height must be numbers above 0.")?;
        match &self.peg {
            None => peg.make().map(|_| ()),
            Some(id) => {
                let data = serde_json::to_value(&peg).map_err(|e| e.to_string())?;
                host::set_feature_data(id, data)
            }
        }
    }
}

#[derive(Default)]
struct Pegs {
    /// The Peg tool is the active tool.
    armed: bool,
    /// A press held: where it went down and where the pointer is now.
    drag: Option<([f32; 3], [f32; 3])>,
    draft: Option<Draft>,
    /// Numbers the cards' ids.
    cards: u64,
}

impl Pegs {
    fn open(&mut self, peg: Option<String>, at: [f32; 3], diameter: f64, height: f64) {
        self.cards += 1;
        self.draft = Some(Draft {
            card: format!("example.peg.card-{}", self.cards),
            peg,
            at,
            diameter: format!("{diameter}"),
            height: format!("{height}"),
        });
    }

    /// Whatever was under way is dropped, unwritten.
    fn drop_gesture(&mut self) {
        self.drag = None;
        self.draft = None;
    }
}

/// Where the pointer meets what it is over, else the ground plane.
fn landing(pointer: &Pointer) -> Option<[f32; 3]> {
    pointer.world.or_else(|| on_level(pointer, 0.0))
}

/// Where the pointer's ray meets the level plane at height `z`.
fn on_level(pointer: &Pointer, z: f32) -> Option<[f32; 3]> {
    let (o, d) = pointer.ray?;
    let t = (z - o[2]) / d[2];
    (d[2].abs() > 1e-6 && t > 0.0).then(|| [o[0] + t * d[0], o[1] + t * d[1], z])
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

/// A circle of `radius` about `at`, raised `up`.
fn circle(at: [f32; 3], radius: f32, up: f32) -> Vec<[f32; 3]> {
    (0..48)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 48.0;
            [
                at[0] + radius * a.cos(),
                at[1] + radius * a.sin(),
                at[2] + up,
            ]
        })
        .collect()
}

impl Bench for Pegs {
    fn describe(&self) -> Registration {
        let number = |name: &str, doc: &str| Param {
            name: name.into(),
            kind: ParamKind::Number,
            required: true,
            doc: doc.into(),
            ..Default::default()
        };
        Registration {
            label: "Pegs".into(),
            description: "Round pegs placed by a drag, sized in a card".into(),
            icon: "peg".into(),
            tools: vec![Tool {
                id: PLACE.into(),
                label: "Peg".into(),
                icon: "peg".into(),
                shortcuts: vec!["P".into()],
                ..Default::default()
            }],
            commands: vec![Command {
                id: MAKE.into(),
                summary: "Make a peg on a body of its own".into(),
                params: vec![
                    number("x", "where its foot's middle is, mm"),
                    number("y", "where its foot's middle is, mm"),
                    number(DIAMETER, "mm"),
                    number(HEIGHT, "mm"),
                ],
                returns: "{body, feature}".into(),
                ..Default::default()
            }],
            length_keys: vec![DIAMETER.into(), HEIGHT.into()],
            ..Default::default()
        }
    }

    fn feature_info(&self, node: &Node) -> FeatureInfo {
        let label = match Peg::from(&node.data) {
            Some(peg) => format!("Peg Ø{} × {}", peg.diameter, peg.height),
            None => "Peg".into(),
        };
        FeatureInfo {
            icon: "peg".into(),
            kind_label: label,
            family_label: "Peg".into(),
            builds_solid: true,
            ..Default::default()
        }
    }

    fn parameters(&self, _node: &Node) -> Vec<Parameter> {
        [(DIAMETER, "Diameter"), (HEIGHT, "Height")]
            .into_iter()
            .map(|(key, label)| Parameter {
                key: format!("/{key}"),
                name: Some(key.into()),
                label: label.into(),
                dim: Dim::Length,
                pointer: format!("/{key}"),
                ..Default::default()
            })
            .collect()
    }

    fn rebuild(&mut self, request: RebuildRequest) -> Vec<Rebuild> {
        request
            .bodies
            .into_iter()
            .map(|history| {
                let mut ops = Vec::new();
                let mut op_features = Vec::new();
                for node in &history.features {
                    let peg = Peg::from(&node.data);
                    let problem = match &peg {
                        Some(peg) => peg.problem(),
                        None => Some("The peg's data does not read."),
                    };
                    if let Some(message) = problem {
                        return Rebuild {
                            body: history.body,
                            plan: Plan::Error {
                                feature: Some(node.id.clone()),
                                message: message.into(),
                            },
                            ..Default::default()
                        };
                    }
                    let peg = peg.expect("read above");
                    ops.push(SolidOp::Primitive {
                        kind: PrimitiveKind::Cylinder {
                            radius: peg.diameter / 2.0,
                            height: peg.height,
                            angle_deg: 360.0,
                        },
                        placement: Placement {
                            origin: peg.at,
                            ..Default::default()
                        },
                        op: if ops.is_empty() {
                            BooleanOp::NewSolid
                        } else {
                            BooleanOp::Fuse
                        },
                    });
                    op_features.push(node.id.clone());
                }
                Rebuild {
                    body: history.body,
                    plan: Plan::Ops { ops, op_features },
                    ..Default::default()
                }
            })
            .collect()
    }

    fn run_command(&mut self, id: &str, args: Value) -> Result<Value, String> {
        if id != MAKE {
            return Err(format!("no command `{id}`"));
        }
        let number = |key: &str| args.get(key).and_then(Value::as_f64).unwrap_or_default();
        let (body, feature) = Peg {
            at: [number("x"), number("y"), 0.0],
            diameter: number(DIAMETER),
            height: number(HEIGHT),
        }
        .make()?;
        Ok(json!({"body": body, "feature": feature}))
    }

    fn input(&mut self, input: &Input) -> bool {
        match &input.event {
            Event::ToolActivated => {
                self.armed = input.tool.as_deref() == Some(PLACE);
                self.drop_gesture();
                false
            }
            Event::Deactivated => {
                self.armed = false;
                self.drop_gesture();
                false
            }
            Event::EditFeature { feature } => {
                let Some(peg) = host::feature(feature).and_then(|n| Peg::from(&n.data)) else {
                    return false;
                };
                let at = peg.at.map(|v| v as f32);
                self.open(Some(feature.clone()), at, peg.diameter, peg.height);
                true
            }
            Event::Press {
                button: Button::Left,
            } if self.armed && self.draft.is_none() => {
                let Some(at) = landing(&input.pointer) else {
                    return false;
                };
                self.drag = Some((at, at));
                true
            }
            Event::Move => {
                let Some((at, end)) = self.drag.as_mut() else {
                    return false;
                };
                if let Some(point) = on_level(&input.pointer, at[2]) {
                    *end = point;
                }
                true
            }
            Event::Release {
                button: Button::Left,
            } => {
                let Some((at, end)) = self.drag.take() else {
                    return false;
                };
                let diameter = (2.0 * distance(at, end) * 10.0).round() / 10.0;
                let diameter = if diameter < 1.0 { 10.0 } else { diameter };
                self.open(None, at, f64::from(diameter), 20.0);
                true
            }
            Event::Key { key, down: true } if key == "Escape" && self.drag.is_some() => {
                self.drag = None;
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, _pointer: &Pointer) -> Frame {
        let mut frame = Frame::default();
        if self.armed {
            frame.toggled.push(PLACE.into());
            frame.hud.tool = Some(ToolHint {
                icon: "peg".into(),
                name: "Peg".into(),
                prompt: "Drag from where the peg stands out to its edge".into(),
                keys: vec![("Esc".into(), "drop the drag".into())],
                ..Default::default()
            });
        }
        let line = |points, closed| Polyline {
            points,
            color: PREVIEW,
            dashed: true,
            closed,
            ..Default::default()
        };
        if let Some((at, end)) = self.drag {
            let radius = distance(at, end);
            frame.lines.push(line(circle(at, radius, 0.0), true));
            frame.labels.push(Label {
                at: end,
                text: format!("Ø {:.1}", 2.0 * radius),
                color: PREVIEW,
                pill: true,
                ..Default::default()
            });
        }
        let Some(draft) = &self.draft else {
            return frame;
        };
        // The preview follows what is typed, while it reads.
        if let Some(peg) = draft.peg() {
            let (r, h) = ((peg.diameter / 2.0) as f32, peg.height as f32);
            frame.lines.push(line(circle(draft.at, r, 0.0), true));
            frame.lines.push(line(circle(draft.at, r, h), true));
            for a in [0.0f32, 90.0, 180.0, 270.0] {
                let (s, c) = a.to_radians().sin_cos();
                let foot = [draft.at[0] + r * c, draft.at[1] + r * s, draft.at[2]];
                frame
                    .lines
                    .push(line(vec![foot, [foot[0], foot[1], foot[2] + h]], false));
            }
        }
        let entry = |id: &str, label: &str, value: &str| {
            let mut entry = panel::entry_with(id, label, value, "mm");
            entry.live = true;
            Widget::from(entry)
        };
        let title = if draft.peg.is_some() {
            "Peg"
        } else {
            "New peg"
        };
        let mut card = panel::floating_card(
            &draft.card,
            title,
            draft.at,
            Some(DIAMETER),
            [
                entry(DIAMETER, "Diameter", &draft.diameter),
                entry(HEIGHT, "Height", &draft.height),
            ],
        );
        card.icon = Some("peg".into());
        card.refusal = draft
            .peg()
            .is_none()
            .then(|| "The diameter and the height must be numbers above 0.".into());
        frame.card = Some(card);
        frame
    }

    fn panel_event(&mut self, slot: PanelSlot, event: PanelEvent) {
        if slot != PanelSlot::Card {
            return;
        }
        match event {
            PanelEvent::Text { id, value } => {
                if let Some(draft) = self.draft.as_mut() {
                    match id.as_str() {
                        DIAMETER => draft.diameter = value,
                        HEIGHT => draft.height = value,
                        _ => {}
                    }
                }
            }
            // A card this draft never showed closes nothing.
            PanelEvent::CardClose { id, accept } => {
                if self.draft.as_ref().is_none_or(|d| d.card != id) {
                    return;
                }
                let draft = self.draft.take().expect("checked above");
                if accept && let Err(e) = draft.write() {
                    host::warn(&format!("The peg was not kept: {e}"));
                }
            }
            _ => {}
        }
    }

    /// The card's draft goes with its tab and comes back with it.
    fn suspend(&mut self) -> Option<Vec<u8>> {
        self.drag = None;
        let draft = self.draft.take()?;
        serde_json::to_vec(&draft).ok()
    }

    fn resume(&mut self, state: Option<Vec<u8>>) {
        self.draft = state.and_then(|bytes| serde_json::from_slice(&bytes).ok());
    }
}

bench!(Pegs);
