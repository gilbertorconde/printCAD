//! The motion panel: a motion over time, its driven joints and traced
//! points, worked out into frames to scrub, play and record.

use super::*;

/// What a field of the motion panel asks.
pub(super) enum MotionEdit {
    Start(f32),
    End(f32),
    Step(f32),
    /// The frames scrubbed to one.
    Frame(usize),
    Formula(usize, String),
    /// A joint picked to drive, by its place in the dropdown.
    AddDrive(usize),
    RemoveDrive(usize),
    WorkOut,
    Play,
    Record,
    Trace,
    RemoveTrace(usize),
}

/// The hinges and sliders a motion does not drive yet.
fn motion_unused<'a>(
    movable: &'a [crate::Joint],
    draft: &crate::MotionStudy,
) -> Vec<&'a crate::Joint> {
    movable
        .iter()
        .filter(|j| !draft.drives.iter().any(|d| d.joint == j.id))
        .collect()
}

impl AssemblyWorkbench {
    /// A motion over time: when it starts and ends, its step, and each
    /// joint driven by a formula of `t`; worked out into frames to scrub,
    /// play and record. The bodies go back when it closes.
    pub(super) fn motion_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.put_back_motion(ctx);
            return TaskOutcome::Cancelled;
        }
        TaskOutcome::Open
    }

    pub(super) fn motion_walk(
        ctx: &WorkbenchRuntimeContext,
        studying: &crate::Studying,
        form: &mut Form<'_>,
    ) -> Option<MotionEdit> {
        let draft = &studying.draft;
        let mut edit = None;
        for (key, label, value) in [
            ("start", "Start, s", draft.start),
            ("end", "End, s", draft.end),
            ("step", "Step, s", draft.step),
        ] {
            let (field, typed) = f::number(form, key, label, value, Dim::Number);
            form.push(field);
            if let Some(value) = typed {
                let value = value as f32;
                edit = Some(match key {
                    "start" => MotionEdit::Start(value),
                    "end" => MotionEdit::End(value),
                    _ => MotionEdit::Step(value),
                });
            }
        }
        let movable = movable_joints(ctx);
        let name_of = |id: FeatureId| {
            movable
                .iter()
                .find(|j| j.id == id)
                .map_or("a removed joint".to_string(), |j| j.name.clone())
        };
        let mut drives = Vec::new();
        for (i, drive) in draft.drives.iter().enumerate() {
            let (id, event) = f::raw(form, &format!("formula:{i}"));
            if let Some(PanelEvent::Text { value, .. }) = event {
                edit = Some(MotionEdit::Formula(i, value.clone()));
            }
            let (remove, pressed) = f::button(
                form,
                &format!("remove_drive:{i}"),
                "Remove",
                ButtonStyle::Small,
            );
            if pressed {
                edit = Some(MotionEdit::RemoveDrive(i));
            }
            drives.push(w::row(vec![
                w::text(name_of(drive.joint)),
                Widget::TextField {
                    id,
                    label: String::new(),
                    value: drive.formula.clone(),
                },
                remove,
            ]));
        }
        let unused = motion_unused(&movable, draft);
        if !unused.is_empty() {
            let mut options = vec!["Drive a joint…".to_string()];
            options.extend(unused.iter().map(|j| j.name.clone()));
            let (add, picked) = f::select(
                form,
                "add_drive",
                bench_api::Select {
                    options,
                    ..Default::default()
                },
            );
            drives.push(add);
            if let Some(index) = picked {
                edit = Some(MotionEdit::AddDrive(index));
            }
        }
        drives.push(w::text(
            "Each formula gives the joint's drive at time t, in seconds: a hinge's angle \
             in degrees, a slider's position in millimetres (30 * sin(t * 180°)).",
        ));
        form.push(w::section("Drives", drives));
        let Some(frames) = &studying.frames else {
            if !draft.drives.is_empty() {
                let (work_out, pressed) = f::button(
                    form,
                    "work_out",
                    "Work out the motion",
                    ButtonStyle::Secondary,
                );
                form.push(work_out);
                if pressed {
                    edit = Some(MotionEdit::WorkOut);
                }
            }
            return edit;
        };
        let time = frames.get(studying.frame).map_or(0.0, |(t, _)| *t);
        let (id, event) = f::raw(form, "frame");
        if let Some(PanelEvent::Number { value, .. }) = event {
            edit = Some(MotionEdit::Frame(*value as usize));
        }
        form.push(Widget::Slider {
            id,
            label: format!("t = {time:.2} s"),
            value: studying.frame as f64,
            min: 0.0,
            max: frames.len().saturating_sub(1) as f64,
            step: Some(1.0),
            decimals: 0,
            show_value: false,
        });
        let (play, pressed) = f::button(
            form,
            "play",
            if studying.playing { "Stop" } else { "Play" },
            ButtonStyle::Secondary,
        );
        if pressed {
            edit = Some(MotionEdit::Play);
        }
        let (record, pressed) = f::button(form, "record", "Record", ButtonStyle::Secondary);
        if pressed {
            edit = Some(MotionEdit::Record);
        }
        form.push(w::row(vec![
            play,
            w::hinted(
                "Save the frames as an animation seen from the current view",
                record,
            ),
        ]));
        let (trace, pressed) = f::button(
            form,
            "trace",
            if studying.tracing {
                "Click a point on a body…"
            } else {
                "Follow a point"
            },
            ButtonStyle::Small,
        );
        if pressed {
            edit = Some(MotionEdit::Trace);
        }
        let mut traces = vec![w::hinted(
            "Click a face of a body: its path and speed through the motion",
            trace,
        )];
        let mut curves: Vec<Vec<(f32, f32)>> = Vec::new();
        for (i, (body, point)) in studying.traces.iter().enumerate() {
            let path = crate::motion::trace(frames, *body, *point);
            let now = path.get(studying.frame).map_or(0.0, |(_, _, v)| *v);
            let top = path.iter().map(|(_, _, v)| *v).fold(0.0f32, f32::max);
            let (remove, pressed) = f::button(
                form,
                &format!("remove_trace:{i}"),
                "Remove",
                ButtonStyle::Small,
            );
            if pressed {
                edit = Some(MotionEdit::RemoveTrace(i));
            }
            traces.push(w::row(vec![
                w::text(format!(
                    "{}: {now:.1} mm/s now, {top:.1} at most",
                    body_name(ctx, *body)
                )),
                remove,
            ]));
            curves.push(path.iter().map(|(t, _, v)| (*t, *v)).collect());
        }
        // Each driven joint's value over time too, scaled to the plot.
        for drive in &draft.drives {
            let Ok(want) = crate::motion::drive_dim(ctx.document, drive.joint) else {
                continue;
            };
            curves.push(
                frames
                    .iter()
                    .filter_map(|(t, _)| {
                        Some((
                            *t,
                            crate::motion::value_at(&drive.formula, f64::from(*t), want).ok()?
                                as f32,
                        ))
                    })
                    .collect(),
            );
        }
        if !curves.is_empty() {
            traces.extend(plot(&curves, studying.traces.len(), Some(time)));
        }
        form.push(w::section("Traces", traces));
        edit
    }

    /// A change to the motion's settings (which drops the frames worked
    /// out), the motion worked out and kept as `asm.motion` keeps it, or
    /// its frames scrubbed, played, recorded or traced.
    pub(super) fn motion_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        edit: MotionEdit,
    ) -> Option<TaskOutcome> {
        let movable = movable_joints(ctx);
        let Some(Task::Motion(studying)) = &mut self.tab.task else {
            return None;
        };
        let draft = &mut studying.draft;
        let mut settings_changed = true;
        let mut show = false;
        match edit {
            MotionEdit::Start(value) => draft.start = value,
            MotionEdit::End(value) => draft.end = value,
            MotionEdit::Step(value) => draft.step = value,
            MotionEdit::Frame(frame) => {
                settings_changed = false;
                studying.frame = frame;
                show = true;
            }
            MotionEdit::Formula(i, value) => draft.drives.get_mut(i)?.formula = value,
            MotionEdit::AddDrive(index) => {
                let joint = *motion_unused(&movable, draft).get(index.checked_sub(1)?)?;
                let formula = match joint.feature.kind {
                    JointKind::Hinge { .. } => "90 * t",
                    _ => "10 * t",
                };
                draft.drives.push(crate::TimedDrive {
                    joint: joint.id,
                    formula: formula.to_string(),
                });
            }
            MotionEdit::RemoveDrive(i) => {
                draft.drives.remove(i);
            }
            MotionEdit::WorkOut => {
                let mut studying = (**studying).clone();
                self.work_out_motion(ctx, &mut studying);
                self.tab.task = Some(Task::Motion(Box::new(studying)));
                return None;
            }
            MotionEdit::Play => {
                settings_changed = false;
                studying.playing = !studying.playing;
            }
            MotionEdit::Record => {
                settings_changed = false;
                let frames = studying.frames.as_ref()?;
                ctx.request(core_document::HostRequest::RecordAnimation {
                    name: "motion".into(),
                    frames: frames.iter().map(|(_, p)| p.clone()).collect(),
                    frame_ms: (draft.step.max(0.01) * 1000.0) as u32,
                });
            }
            MotionEdit::Trace => {
                settings_changed = false;
                studying.tracing = !studying.tracing;
            }
            MotionEdit::RemoveTrace(i) => {
                settings_changed = false;
                studying.traces.remove(i);
            }
        }
        if settings_changed {
            studying.frames = None;
            studying.playing = false;
        }
        if show {
            self.show_frame(ctx);
        }
        None
    }

    /// Work the motion out into frames from where the bodies sat, and keep
    /// it in the document as `asm.motion` keeps it.
    fn work_out_motion(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        studying: &mut crate::Studying,
    ) {
        crate::restore_placements(ctx, &studying.placements);
        let frames = match studying.draft.frames(ctx.document) {
            Ok(frames) => frames,
            Err(why) => {
                ctx.log_warn(format!("The motion could not be worked out: {why}"));
                return;
            }
        };
        studying.frames = Some(frames);
        studying.frame = 0;
        let mut args = serde_json::to_value(&studying.draft).unwrap_or_default();
        if let Some(id) = studying.study {
            args["study"] = serde_json::json!(id.0.to_string());
        }
        let made = crate::commands::run("asm.motion", &crate::commands::object(args.clone()), ctx);
        match made
            .ok()
            .and_then(|id| uuid::Uuid::parse_str(id.as_str()?).ok())
        {
            Some(id) => {
                studying.study = Some(FeatureId(id));
                args["study"] = serde_json::json!(id.to_string());
                ctx.record(
                    "asm.motion",
                    crate::commands::object(args),
                    serde_json::json!(id.to_string()),
                );
            }
            None => ctx.log_warn("Could not keep the motion"),
        }
    }
}

/// The plot's space, as wide and as tall as it shows in a panel of the
/// usual width.
const PLOT: [f32; 2] = [300.0, 90.0];

/// Curves over time as a diagram, each scaled to its own range: the first
/// `speeds` (traced points' speeds) in the accent colour, the rest (driven
/// joints' values) as outlines; a line where the frame shown stands. Then
/// what the lines are.
fn plot(curves: &[Vec<(f32, f32)>], speeds: usize, at: Option<f32>) -> Vec<Widget> {
    use bench_api::{DiagramShape, DiagramStroke};
    let [width, height] = PLOT;
    let (t0, t1) = curves
        .iter()
        .flatten()
        .fold((f32::MAX, f32::MIN), |(lo, hi), (t, _)| {
            (lo.min(*t), hi.max(*t))
        });
    let span = (t1 - t0).max(1e-6);
    let x = |t: f32| width * (t - t0) / span;
    let mut shapes = vec![DiagramShape::Path {
        points: vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
        closed: true,
        stroke: DiagramStroke::Thin,
        fill: true,
    }];
    for (i, curve) in curves.iter().enumerate() {
        let (lo, hi) = curve.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (_, v)| {
            (lo.min(*v), hi.max(*v))
        });
        let range = (hi - lo).max(1e-6);
        shapes.push(DiagramShape::Path {
            points: curve
                .iter()
                .map(|(t, v)| [x(*t), 4.0 + (height - 8.0) * (v - lo) / range])
                .collect(),
            closed: false,
            stroke: if i < speeds {
                DiagramStroke::Accent
            } else {
                DiagramStroke::Outline
            },
            fill: false,
        });
    }
    if let Some(t) = at {
        shapes.push(DiagramShape::Path {
            points: vec![[x(t), 0.0], [x(t), height]],
            closed: false,
            stroke: DiagramStroke::Thin,
            fill: false,
        });
    }
    let legend = match (speeds > 0, curves.len() > speeds) {
        (true, true) => "Speed in the accent colour, drives in white, each to its own scale",
        (true, false) => "Speed, each point to its own scale",
        _ => "Drives, each to its own scale",
    };
    vec![
        Widget::Diagram {
            id: "plot".into(),
            width,
            height,
            shapes,
            dimensions: Vec::new(),
            callouts: Vec::new(),
        },
        w::text(legend),
    ]
}
