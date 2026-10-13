//! The Assembly task panels: the prompt while a joint's faces are picked,
//! a joint's or a coupling's settings, a body moved by numbers, and the
//! assembly's own tasks (interference, exploded views, motion over time,
//! linked copies, replacing a body, rigid groups, mass, the parts list).
//!
//! Each is declared as widgets (`task_widgets`) and drawn by the same
//! renderer a package's panel is; what the user does comes back as panel
//! events (`task_event`), which change the document through the bench's
//! own commands where one makes the change.

use core_document::{
    BodyId, BodyPlacement, FeatureId, TaskOutcome, TaskRequest, WorkbenchFeature,
    WorkbenchRuntimeContext,
};

use bench_api::{Bind, ButtonStyle, Dim, NoteKind, PanelEvent, Widget};

use crate::{
    AssemblyWorkbench, Coupling, Gearing, JointFeature, JointKind, JointTool, Task, body_name,
    restore_placements,
};

/// The declared panels' widgets, as the Assembly writes them.
mod w {
    use super::*;

    pub(super) fn header(icon: &str, title: &str) -> Widget {
        Widget::Header {
            icon: icon.into(),
            title: title.into(),
        }
    }

    pub(super) fn value(label: &str, value: impl Into<String>) -> Widget {
        Widget::Value {
            label: label.into(),
            value: value.into(),
            mono: false,
        }
    }

    pub(super) fn note(kind: NoteKind, title: Option<&str>, text: impl Into<String>) -> Widget {
        Widget::Note {
            kind,
            title: title.map(Into::into),
            text: text.into(),
        }
    }

    pub(super) fn button(id: &str, label: &str, style: ButtonStyle) -> Widget {
        Widget::Button {
            id: id.into(),
            label: label.into(),
            style,
            enabled: true,
        }
    }

    pub(super) fn hinted(hint: impl Into<String>, widget: Widget) -> Widget {
        Widget::Hinted {
            hint: hint.into(),
            widget: Box::new(widget),
        }
    }

    /// A number the bench keeps, not bound to a parameter.
    pub(super) fn number(id: &str, label: &str, value: f32, dim: Dim) -> Widget {
        ranged(id, label, value, dim, None)
    }

    /// A number within `min..=max`, when given.
    pub(super) fn ranged(
        id: &str,
        label: &str,
        value: f32,
        dim: Dim,
        range: Option<(f64, f64)>,
    ) -> Widget {
        Widget::Number {
            id: id.into(),
            label: label.into(),
            value: f64::from(value),
            dim,
            bind: None,
            min: range.map(|r| r.0),
            max: range.map(|r| r.1),
            decimals: 2,
            error: None,
        }
    }

    pub(super) fn toggle(id: &str, label: &str, on: bool) -> Widget {
        Widget::Toggle {
            id: id.into(),
            label: label.into(),
            on,
        }
    }

    /// A feature's number a formula can set: `key` is its parameter.
    pub(super) fn bound(
        id: &str,
        label: &str,
        value: f32,
        dim: Dim,
        (feature, key): (FeatureId, &str),
    ) -> Widget {
        Widget::Number {
            id: id.into(),
            label: label.into(),
            value: f64::from(value),
            dim,
            bind: Some(Bind {
                feature: feature.0.to_string(),
                key: key.into(),
            }),
            min: None,
            max: None,
            decimals: 2,
            error: None,
        }
    }

    pub(super) fn text(text: impl Into<String>) -> Widget {
        Widget::Text {
            text: text.into(),
            mono: false,
        }
    }

    pub(super) fn row(children: Vec<Widget>) -> Widget {
        Widget::Row { children }
    }
}

/// What to click while a joint's faces are picked, and the origin's planes
/// or axes the second pick may be instead.
fn picking_widgets(picking: &crate::Picking) -> Vec<Widget> {
    let mut widgets = vec![
        w::header(picking.kind.icon(), picking.kind.label()),
        w::text(picking.prompt()),
        w::text(
            "The first body moves; the second stays where it is. A body \
             with no joints of its own never moves. A datum plane or line \
             selected in the tree is taken as a face.",
        ),
    ];
    if let Some((_, first, ..)) = picking.first {
        let offered: Vec<Widget> = crate::ORIGIN
            .iter()
            .enumerate()
            .filter(|(_, (_, anchor))| picking.kind.takes_anchor(anchor, Some(first)))
            .map(|(i, (name, _))| w::button(&format!("origin:{i}"), name, ButtonStyle::Small))
            .collect();
        if !offered.is_empty() {
            widgets.push(Widget::Group {
                title: "Or the origin's".into(),
                open: true,
                children: vec![w::row(offered)],
            });
        }
    }
    widgets
}

/// The move panel's fields: the position along each axis, then the turn
/// about each.
const MOVE_FIELDS: [&str; 6] = ["x", "y", "z", "turn_x", "turn_y", "turn_z"];

/// A placement's turn as angles about X, Y and Z, degrees.
fn move_angles(placement: &BodyPlacement) -> [f32; 3] {
    let (ax, ay, az) = placement.quat().to_euler(glam::EulerRot::XYZ);
    [ax, ay, az].map(f32::to_degrees)
}

/// The axes in the order the copies panel offers them: X, Y, Z, which
/// are also the normals of the YZ, XZ and XY planes.
const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A dropdown of `options`.
fn choice(id: &str, label: &str, options: &[&str], selected: usize) -> Widget {
    Widget::Choice {
        id: id.into(),
        label: label.into(),
        options: options.iter().map(|o| o.to_string()).collect(),
        selected,
    }
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

/// The hinges and sliders: the joints a coupling ties and a motion drives.
fn movable_joints(ctx: &WorkbenchRuntimeContext) -> Vec<crate::Joint> {
    crate::joints(ctx.document)
        .into_iter()
        .filter(|j| {
            matches!(
                j.feature.kind,
                JointKind::Hinge { .. } | JointKind::Slider { .. }
            )
        })
        .collect()
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

/// What `body` may still do, its joints holding: "fully placed", or its
/// free motions.
fn freedom_text(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<Widget> {
    let (_, motions) = crate::freedom(ctx.document)
        .into_iter()
        .find(|(b, _)| *b == body)?;
    Some(w::text(if motions.is_empty() {
        "Its joints place this body fully.".to_string()
    } else {
        let words: Vec<String> = motions.iter().map(crate::Motion::describe).collect();
        format!("It may still {}.", words.join(", "))
    }))
}

/// Where a joint's motion stands: a hinge's angle or a slider's position,
/// and an alignment's turn and slide.
fn joint_now(
    ctx: &WorkbenchRuntimeContext,
    body: Option<BodyId>,
    joint: &JointFeature,
) -> (Option<f64>, Option<(f64, f64)>) {
    let placed = |b: BodyId| -> crate::Rigid { ctx.document.body_placement(b).into() };
    let other = placed(joint.other_body);
    (
        body.and_then(|b| joint.travel(&placed(b), &other)),
        body.and_then(|b| joint.align_travel(&placed(b), &other)),
    )
}

/// The range a drive sweeps through: its limits, or a whole turn, or
/// 25 mm either side of `centre`.
fn sweep_range(limits: Option<[f32; 2]>, angular: bool, centre: f64) -> (f64, f64) {
    match limits {
        Some([low, high]) => (f64::from(low), f64::from(high)),
        None if angular => (-179.0, 179.0),
        None => (centre - 25.0, centre + 25.0),
    }
}

/// The range a joint's motion is checked for collisions through: its
/// limits, or a whole turn, or 25 mm either side of where it stands.
fn motion_range(limits: Option<[f32; 2]>, hinge: bool, now: Option<f64>) -> (f32, f32) {
    match limits {
        Some([low, high]) => (low, high),
        None if hinge => (-180.0, 180.0),
        None => {
            let at = now.unwrap_or(0.0) as f32;
            (at - 25.0, at + 25.0)
        }
    }
}

/// The value a declared number came back with, by its id.
fn number_event(event: &PanelEvent) -> Option<(&str, f32)> {
    match event {
        PanelEvent::Number { id, value } => Some((id.as_str(), *value as f32)),
        _ => None,
    }
}

/// Frames in a recorded sweep: there and back in four seconds.
const SWEEP_FRAMES: usize = 60;

impl AssemblyWorkbench {
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if self.picking.is_some() {
            if request.accept || request.cancel {
                self.picking = None;
                return TaskOutcome::Cancelled;
            }
            return self.declared_panel(ui, ctx);
        }
        match self.task.clone() {
            Some(Task::Joint {
                id,
                before,
                placements,
            }) => self.joint_panel(ui, ctx, request, id, before, &placements),
            Some(Task::Coupling {
                id,
                before,
                placements,
            }) => self.coupling_panel(ui, ctx, request, id, before, &placements),
            Some(Task::Move { body, placements }) => {
                self.move_panel(ui, ctx, request, body, &placements)
            }
            Some(Task::Interference { .. }) => self.interference_panel(ui, ctx, request),
            Some(Task::Explode { steps, .. }) => {
                if request.accept || request.cancel {
                    self.put_back_explosion(ctx);
                    return TaskOutcome::Cancelled;
                }
                let view = steps
                    .view
                    .and_then(|id| crate::exploded::view_of(ctx.document, id));
                if view.is_none_or(|v| v.steps.is_empty()) {
                    self.show_steps(ctx);
                }
                self.declared_panel(ui, ctx)
            }
            Some(Task::Parts) => self.parts_panel(ui, ctx, request),
            Some(Task::Motion(_)) => self.motion_panel(ui, ctx, request),
            Some(Task::Copies {
                body,
                count,
                step,
                around,
                mirror,
            }) => self.copies_panel(ui, ctx, request, body, (count, step, around), mirror),
            Some(Task::Replace { old, new }) => self.replace_panel(ui, ctx, request, old, new),
            Some(Task::Group { editing, members }) => {
                self.group_panel(ui, ctx, request, editing, &members)
            }
            Some(Task::Mass { .. }) => self.mass_panel(ui, ctx, request),
            None => TaskOutcome::Open,
        }
    }

    /// Draw a declared panel and hand back what the user did. Formulas set
    /// on bound numbers go into the document here, before the events that
    /// carry their values.
    fn show_declared(
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        widgets: &[Widget],
    ) -> Vec<PanelEvent> {
        let out = core_document::panel::show(
            ui,
            egui::Id::new("assembly_task"),
            widgets,
            Some(&*ctx.document),
        );
        for (bind, formula) in out.formulas {
            let Ok(uuid) = uuid::Uuid::parse_str(&bind.feature) else {
                continue;
            };
            if let Err(why) = ctx
                .document
                .set_feature_formula(FeatureId(uuid), &bind.key, formula)
            {
                ctx.log_warn(why.to_string());
            }
        }
        out.events
    }

    /// The open task's panel drawn as declared widgets, and what the user
    /// did to it handled: a package's panel takes the same way.
    fn declared_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> TaskOutcome {
        let widgets = self.task_widgets(ctx);
        for event in Self::show_declared(ui, ctx, &widgets) {
            if let Some(outcome) = self.task_event(ctx, &event) {
                return outcome;
            }
        }
        TaskOutcome::Open
    }

    /// The open task's panel, as widgets: the prompt while a joint's faces
    /// are picked, else the task's own.
    pub(crate) fn task_widgets(&self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        if let Some(picking) = &self.picking {
            return picking_widgets(picking);
        }
        match &self.task {
            Some(Task::Move { body, .. }) => self.move_widgets(ctx, *body),
            Some(Task::Copies {
                body,
                count,
                step,
                around,
                mirror,
            }) => Self::copies_widgets(ctx, *body, (*count, *step, *around), *mirror),
            Some(Task::Replace { old, new }) => Self::replace_widgets(ctx, *old, *new),
            Some(Task::Group { editing, members }) => Self::group_widgets(ctx, *editing, members),
            Some(Task::Interference {
                found,
                seq,
                around,
                clearance,
            }) => self.interference_widgets(ctx, found.as_ref(), (*seq, *around, *clearance)),
            Some(Task::Mass { found, density }) => self.mass_widgets(ctx, found.as_ref(), *density),
            Some(Task::Explode { spread, steps, .. }) => Self::explode_widgets(ctx, *spread, steps),
            Some(Task::Parts) => self.parts_widgets(ctx),
            Some(Task::Coupling { id, .. }) => self.coupling_widgets(ctx, *id),
            Some(Task::Joint { id, .. }) => self.joint_widgets(ctx, *id),
            Some(Task::Motion(studying)) => Self::motion_widgets(ctx, studying),
            _ => Vec::new(),
        }
    }

    /// A change to the open task's panel; an outcome when it closes the
    /// task.
    pub(crate) fn task_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        if self.picking.is_some() {
            // One of the origin's planes or axes, as the second pick.
            if let PanelEvent::Button { id } = event
                && let Some((_, anchor)) = id
                    .strip_prefix("origin:")
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| crate::ORIGIN.get(i))
            {
                self.picked(ctx, crate::WORLD, *anchor, None, 0);
            }
            return None;
        }
        match self.task.clone() {
            Some(Task::Move { body, .. }) => self.move_event(ctx, body, event),
            Some(Task::Copies { .. }) => self.copies_event(event),
            Some(Task::Group { editing, .. }) => self.group_event(ctx, editing, event),
            Some(Task::Interference {
                found,
                around,
                clearance,
                ..
            }) => self.interference_event(ctx, found.as_ref(), (around, clearance), event),
            Some(Task::Mass { found, .. }) => self.mass_event(ctx, found.as_ref(), event),
            Some(Task::Explode { .. }) => self.explode_event(ctx, event),
            Some(Task::Parts) => self.parts_event(ctx, event),
            Some(Task::Coupling { id, before, .. }) => {
                self.coupling_event(ctx, (id, before.is_none()), event)
            }
            Some(Task::Joint {
                id,
                before,
                placements,
            }) => self.joint_event(ctx, (id, before.is_none(), &placements), event),
            Some(Task::Motion(_)) => self.motion_event(ctx, event),
            _ => None,
        }
    }

    /// The assembly's joints left apart, then those it could do without,
    /// one row each by name; a click selects the joint. Nothing while
    /// every joint holds and none is redundant.
    fn solver_messages(&self, ctx: &WorkbenchRuntimeContext) -> Option<Widget> {
        let messages = self.messages(ctx);
        if messages.is_empty() {
            return None;
        }
        let items = messages
            .into_iter()
            .map(|(_, name, apart)| bench_api::ListItem {
                label: name,
                detail: Some(
                    if apart {
                        "Left apart: its bodies cannot sit as it asks"
                    } else {
                        "Redundant: other joints already hold all it holds"
                    }
                    .to_string(),
                ),
                icon: Some(if apart { "warning" } else { "info" }.to_string()),
            })
            .collect();
        Some(Widget::Group {
            title: "Solver messages".into(),
            open: true,
            children: vec![
                w::text("Click one to select its joint."),
                Widget::List {
                    id: "solver_messages".into(),
                    items,
                    selected: None,
                },
            ],
        })
    }

    /// The joints and couplings left apart, then the redundant joints:
    /// each with its name and whether it is apart.
    fn messages(&self, ctx: &WorkbenchRuntimeContext) -> Vec<(FeatureId, String, bool)> {
        let mut messages: Vec<(FeatureId, String, bool)> = self
            .unmet_now(ctx)
            .into_iter()
            .map(|(id, name)| (id, name, true))
            .collect();
        for (id, name) in self.redundant_now(ctx) {
            if !messages.iter().any(|(a, _, _)| *a == id) {
                messages.push((id, name, false));
            }
        }
        messages
    }

    /// What the last solve said, as a note.
    fn verdict_note(&self) -> Option<Widget> {
        match &self.verdict {
            Some(Ok(message)) => Some(w::note(NoteKind::Success, None, message)),
            Some(Err(message)) => {
                Some(w::note(NoteKind::Error, Some("Joints left apart"), message))
            }
            None => None,
        }
    }

    /// Run one of the bench's own commands from a panel, as a package's
    /// `call` runs it, and keep what the solve that ends it said: the
    /// bodies it moved, or the joints it left apart.
    fn call(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let before = crate::all_placements(ctx);
        let args = crate::commands::object(args);
        match crate::commands::run(command, &args, ctx) {
            Ok(value) => {
                let moved = before
                    .iter()
                    .filter(|(b, p)| ctx.document.body_placement(*b) != *p)
                    .count();
                self.verdict = Some(Ok(crate::moved_words(moved)));
                Ok(value)
            }
            Err(why) => {
                let message = why.to_string();
                self.verdict = Some(Err(message.clone()));
                ctx.log_warn(message.clone());
                Err(message)
            }
        }
    }

    /// What an interference check found: each clash, a click selecting
    /// the first of its bodies.
    fn interference_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.checking = None;
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_interference(ctx);
        if matches!(&self.task, Some(Task::Interference { found: None, .. })) {
            // The answer arrives on another thread, with no event to wake
            // the window.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.declared_panel(ui, ctx)
    }

    /// The check's progress while it runs; then what it found, each pair a
    /// row, and the checks to run again.
    fn interference_widgets(
        &self,
        ctx: &WorkbenchRuntimeContext,
        found: Option<&crate::Interference>,
        (seq, around, clearance): (u64, Option<BodyId>, Option<f32>),
    ) -> Vec<Widget> {
        let mut widgets = vec![w::header("check-geometry", "Interference")];
        let Some(found) = found else {
            let (done, total) = self.interference_progress().unwrap_or((0, 0));
            widgets.push(Widget::Progress {
                label: format!("Checking {done} of {total} pairs that may touch"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            widgets.push(w::hinted(
                "Stop checking; the clashes found so far stay",
                w::button("stop", "Stop", ButtonStyle::Secondary),
            ));
            return widgets;
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
            widgets.push(match found.near.len() {
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
            widgets.push(Widget::List {
                id: "near".into(),
                items: found
                    .near
                    .iter()
                    .map(|n| pair(n.a, n.b, format!("{:.2} mm", n.distance_mm)))
                    .collect(),
                selected: None,
            });
        } else {
            widgets.push(match found.clashes.len() {
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
        widgets.push(Widget::List {
            id: "clashes".into(),
            items: found
                .clashes
                .iter()
                .map(|c| pair(c.a, c.b, format!("{:.2} mm³", c.volume_mm3)))
                .collect(),
            selected: None,
        });
        if !found.unchecked.is_empty() {
            let n = found.unchecked.len();
            widgets.push(w::note(
                NoteKind::Warning,
                Some(&format!(
                    "{n} pair{} could not be checked",
                    if n == 1 { "" } else { "s" }
                )),
                "The kernel failed on these; every other pair was checked",
            ));
            widgets.push(Widget::List {
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
            widgets.push(w::note(
                NoteKind::Warning,
                None,
                "Stopped early: some pairs were not checked",
            ));
        }
        if found.skipped > 0 {
            widgets.push(w::text(format!(
                "{} visible bod{} without a solid (a mesh, or not built yet) left out",
                found.skipped,
                if found.skipped == 1 { "y" } else { "ies" }
            )));
        }
        if ctx.document.mutation_seq() != seq {
            widgets.push(w::note(
                NoteKind::Warning,
                None,
                "The assembly has changed since this check",
            ));
        }
        if let Some(body) = around {
            widgets.push(w::note(
                NoteKind::Info,
                None,
                format!("{} against every other body", body_name(ctx, body)),
            ));
        }
        widgets.push(w::ranged(
            "clearance",
            "Clearance",
            self.clearance_mm.or(clearance).unwrap_or(0.5),
            Dim::Length,
            Some((0.0, 1000.0)),
        ));
        widgets.push(w::row(vec![
            w::hinted(
                "Pairs that share material",
                w::button("check_clashes", "Check clashes", ButtonStyle::Secondary),
            ),
            w::hinted(
                "Pairs nearer to each other than the clearance",
                w::button("check_clearance", "Check clearance", ButtonStyle::Secondary),
            ),
        ]));
        if around.is_some() {
            widgets.push(w::button(
                "every_pair",
                "Check every pair",
                ButtonStyle::Secondary,
            ));
        }
        widgets
    }

    /// A pair clicked selects its first body; the buttons stop the check
    /// or run another.
    fn interference_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        found: Option<&crate::Interference>,
        (around, clearance): (Option<BodyId>, Option<f32>),
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let gap = self.clearance_mm.or(clearance).unwrap_or(0.5);
        match event {
            PanelEvent::Select { id, index } => {
                let found = found?;
                let body = match id.as_str() {
                    "near" => found.near.get(*index)?.a,
                    "clashes" => found.clashes.get(*index)?.a,
                    _ => return None,
                };
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            PanelEvent::Number { id, value } if id == "clearance" => {
                self.clearance_mm = Some(*value as f32);
            }
            PanelEvent::Button { id } => match id.as_str() {
                "stop" => self.stop_interference(),
                "check_clashes" => self.check_interference(ctx, around),
                "check_clearance" => self.check_clearance(ctx, around, gap),
                "every_pair" => match clearance {
                    Some(gap) => self.check_clearance(ctx, None, gap),
                    None => self.check_interference(ctx, None),
                },
                _ => {}
            },
            _ => {}
        }
        None
    }

    /// The exploded view's spread, or, once bodies are picked for a step,
    /// its steps; closing puts every body back.
    fn explode_widgets(
        ctx: &WorkbenchRuntimeContext,
        spread: f32,
        steps: &crate::Stepping,
    ) -> Vec<Widget> {
        let mut widgets = vec![w::header("scale-geometry", "Exploded view")];
        if steps.view.is_none() && steps.picked.is_empty() {
            widgets.push(w::hinted(
                "How far each body moves out, as a share of its distance from the middle",
                Widget::Slider {
                    id: "spread".into(),
                    label: "Spread".into(),
                    value: f64::from(spread),
                    min: 0.0,
                    max: 3.0,
                    step: None,
                    decimals: 2,
                    show_value: true,
                },
            ));
            widgets.push(w::text(
                "Each body moves straight out from the middle of the assembly. \
                 Nothing is kept: the bodies go back when this closes. Click bodies to \
                 make a step of a view kept in the document.",
            ));
            return widgets;
        }
        let view = steps
            .view
            .and_then(|id| crate::exploded::view_of(ctx.document, id))
            .unwrap_or_default();
        for (i, step) in view.steps.iter().enumerate() {
            let names: Vec<String> = step.bodies.iter().map(|b| body_name(ctx, *b)).collect();
            // The button first, so a long line wraps beside it rather
            // than squeezing it.
            widgets.push(w::row(vec![
                w::button(&format!("remove_step:{i}"), "Remove", ButtonStyle::Small),
                w::text(format!(
                    "{}. {}: {}",
                    i + 1,
                    names.join(", "),
                    step.summary()
                )),
            ]));
        }
        if !view.steps.is_empty() {
            widgets.push(Widget::Slider {
                id: "at".into(),
                label: "Progress".into(),
                value: f64::from(steps.at),
                min: 0.0,
                max: view.steps.len() as f64,
                step: None,
                decimals: 2,
                show_value: true,
            });
            widgets.push(w::hinted(
                "Play the steps in order, a step a second, round again",
                w::button(
                    "play",
                    if steps.playing { "Stop" } else { "Play" },
                    ButtonStyle::Secondary,
                ),
            ));
        }
        let picked: Vec<String> = steps.picked.iter().map(|b| body_name(ctx, *b)).collect();
        let mut next = vec![w::text(if picked.is_empty() {
            "Click the bodies it moves".to_string()
        } else {
            picked.join(", ")
        })];
        for (k, label) in ["Shift x", "Shift y", "Shift z"].into_iter().enumerate() {
            next.push(w::number(
                &format!("shift_{k}"),
                label,
                steps.shift[k],
                Dim::Length,
            ));
        }
        next.push(choice(
            "turn_axis",
            "Turn about",
            &["X", "Y", "Z"],
            steps.turn_axis.min(2),
        ));
        next.push(w::number(
            "turn_degrees",
            "Turn",
            steps.turn_degrees,
            Dim::Angle,
        ));
        next.push(w::hinted(
            "How far each body moves straight out from the middle of the step's bodies",
            w::number("radial", "Spread out", steps.radial, Dim::Length),
        ));
        if !steps.picked.is_empty() {
            next.push(w::button("add_step", "Add step", ButtonStyle::Secondary));
        }
        widgets.push(Widget::Group {
            title: "Next step".into(),
            open: true,
            children: next,
        });
        widgets.push(w::text(
            "The view is kept in the document: double-click it in the tree to show it \
             again. The bodies go back when this closes.",
        ));
        widgets
    }

    /// The spread changed, or a step added or taken away (kept as
    /// `asm.exploded_view` keeps it), or the view scrubbed or played.
    fn explode_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let Some(Task::Explode {
            placements,
            spread,
            steps,
        }) = &mut self.task
        else {
            return None;
        };
        let placements = placements.clone();
        let view = steps
            .view
            .and_then(|id| crate::exploded::view_of(ctx.document, id))
            .unwrap_or_default();
        let mut edited = None;
        match event {
            PanelEvent::Number { id, value } if id == "spread" => {
                *spread = *value as f32;
                crate::explode(ctx, &placements, *spread);
                return None;
            }
            PanelEvent::Number { id, value } if id == "at" => {
                steps.at = *value as f32;
            }
            PanelEvent::Choice { id, index } if id == "turn_axis" => {
                steps.turn_axis = (*index).min(2);
                return None;
            }
            PanelEvent::Number { id, value } if id == "turn_degrees" => {
                steps.turn_degrees = *value as f32;
                return None;
            }
            PanelEvent::Number { id, value } if id == "radial" => {
                steps.radial = *value as f32;
                return None;
            }
            PanelEvent::Number { id, value } => {
                let k = id
                    .strip_prefix("shift_")
                    .and_then(|k| k.parse::<usize>().ok())
                    .filter(|k| *k < 3)?;
                steps.shift[k] = *value as f32;
                return None;
            }
            PanelEvent::Button { id } if id == "play" => {
                steps.playing = !steps.playing;
                return None;
            }
            PanelEvent::Button { id } if id == "add_step" && !steps.picked.is_empty() => {
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
            PanelEvent::Button { id } => {
                let i = id
                    .strip_prefix("remove_step:")
                    .and_then(|i| i.parse::<usize>().ok())
                    .filter(|i| *i < view.steps.len())?;
                let mut changed = view.clone();
                changed.steps.remove(i);
                steps.at = steps.at.min(changed.steps.len() as f32);
                edited = Some(changed);
            }
            _ => return None,
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

    /// A motion over time: when it starts and ends, its step, and each
    /// joint driven by a formula of `t`; worked out into frames to scrub,
    /// play and record. The bodies go back when it closes.
    fn motion_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.put_back_motion(ctx);
            return TaskOutcome::Cancelled;
        }
        let outcome = self.declared_panel(ui, ctx);
        if matches!(&self.task, Some(Task::Motion(s)) if s.playing) {
            ui.ctx().request_repaint();
        }
        outcome
    }

    fn motion_widgets(ctx: &WorkbenchRuntimeContext, studying: &crate::Studying) -> Vec<Widget> {
        let draft = &studying.draft;
        let mut widgets = vec![w::header("polar-pattern", "Motion over time")];
        for (key, label, value) in [
            ("start", "Start, s", draft.start),
            ("end", "End, s", draft.end),
            ("step", "Step, s", draft.step),
        ] {
            widgets.push(w::number(key, label, value, Dim::Number));
        }
        let movable = movable_joints(ctx);
        let name_of = |id: FeatureId| {
            movable
                .iter()
                .find(|j| j.id == id)
                .map_or("a removed joint".to_string(), |j| j.name.clone())
        };
        let mut drives: Vec<Widget> = draft
            .drives
            .iter()
            .enumerate()
            .map(|(i, drive)| {
                w::row(vec![
                    w::text(name_of(drive.joint)),
                    Widget::TextField {
                        id: format!("formula:{i}"),
                        label: String::new(),
                        value: drive.formula.clone(),
                    },
                    w::button(&format!("remove_drive:{i}"), "Remove", ButtonStyle::Small),
                ])
            })
            .collect();
        let unused = motion_unused(&movable, draft);
        if !unused.is_empty() {
            let mut options = vec!["Drive a joint…".to_string()];
            options.extend(unused.iter().map(|j| j.name.clone()));
            drives.push(Widget::Choice {
                id: "add_drive".into(),
                label: String::new(),
                options,
                selected: 0,
            });
        }
        drives.push(w::text(
            "Each formula gives the joint's drive at time t, in seconds: a hinge's angle \
             in degrees, a slider's position in millimetres (30 * sin(t * 180°)).",
        ));
        widgets.push(Widget::Group {
            title: "Drives".into(),
            open: true,
            children: drives,
        });
        let Some(frames) = &studying.frames else {
            if !draft.drives.is_empty() {
                widgets.push(w::button(
                    "work_out",
                    "Work out the motion",
                    ButtonStyle::Secondary,
                ));
            }
            return widgets;
        };
        let time = frames.get(studying.frame).map_or(0.0, |(t, _)| *t);
        widgets.push(Widget::Slider {
            id: "frame".into(),
            label: format!("t = {time:.2} s"),
            value: studying.frame as f64,
            min: 0.0,
            max: frames.len().saturating_sub(1) as f64,
            step: Some(1.0),
            decimals: 0,
            show_value: false,
        });
        widgets.push(w::row(vec![
            w::button(
                "play",
                if studying.playing { "Stop" } else { "Play" },
                ButtonStyle::Secondary,
            ),
            w::hinted(
                "Save the frames as an animation seen from the current view",
                w::button("record", "Record", ButtonStyle::Secondary),
            ),
        ]));
        let mut traces = vec![w::hinted(
            "Click a face of a body: its path and speed through the motion",
            w::button(
                "trace",
                if studying.tracing {
                    "Click a point on a body…"
                } else {
                    "Follow a point"
                },
                ButtonStyle::Small,
            ),
        )];
        let mut curves: Vec<Vec<(f32, f32)>> = Vec::new();
        for (i, (body, point)) in studying.traces.iter().enumerate() {
            let path = crate::motion::trace(frames, *body, *point);
            let now = path.get(studying.frame).map_or(0.0, |(_, _, v)| *v);
            let top = path.iter().map(|(_, _, v)| *v).fold(0.0f32, f32::max);
            traces.push(w::row(vec![
                w::text(format!(
                    "{}: {now:.1} mm/s now, {top:.1} at most",
                    body_name(ctx, *body)
                )),
                w::button(&format!("remove_trace:{i}"), "Remove", ButtonStyle::Small),
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
        widgets.push(Widget::Group {
            title: "Traces".into(),
            open: true,
            children: traces,
        });
        widgets
    }

    /// A change to the motion's settings (which drops the frames worked
    /// out), the motion worked out and kept as `asm.motion` keeps it, or
    /// its frames scrubbed, played, recorded or traced.
    fn motion_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let movable = movable_joints(ctx);
        let Some(Task::Motion(studying)) = &mut self.task else {
            return None;
        };
        let draft = &mut studying.draft;
        let mut settings_changed = true;
        let mut show = false;
        match event {
            PanelEvent::Number { id, value } => match id.as_str() {
                "start" => draft.start = *value as f32,
                "end" => draft.end = *value as f32,
                "step" => draft.step = *value as f32,
                "frame" => {
                    settings_changed = false;
                    studying.frame = *value as usize;
                    show = true;
                }
                _ => return None,
            },
            PanelEvent::Text { id, value } => {
                let i = id
                    .strip_prefix("formula:")
                    .and_then(|i| i.parse::<usize>().ok())?;
                draft.drives.get_mut(i)?.formula = value.clone();
            }
            PanelEvent::Choice { id, index } if id == "add_drive" => {
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
            PanelEvent::Button { id } => {
                settings_changed = false;
                match id.as_str() {
                    "work_out" => {
                        let mut studying = (**studying).clone();
                        self.work_out_motion(ctx, &mut studying);
                        self.task = Some(Task::Motion(Box::new(studying)));
                        return None;
                    }
                    "play" => studying.playing = !studying.playing,
                    "record" => {
                        let frames = studying.frames.as_ref()?;
                        ctx.request(core_document::HostRequest::RecordAnimation {
                            name: "motion".into(),
                            frames: frames.iter().map(|(_, p)| p.clone()).collect(),
                            frame_ms: (draft.step.max(0.01) * 1000.0) as u32,
                        });
                    }
                    "trace" => studying.tracing = !studying.tracing,
                    other => {
                        if let Some(i) = other
                            .strip_prefix("remove_drive:")
                            .and_then(|i| i.parse::<usize>().ok())
                            .filter(|i| *i < draft.drives.len())
                        {
                            draft.drives.remove(i);
                            settings_changed = true;
                        } else if let Some(i) = other
                            .strip_prefix("remove_trace:")
                            .and_then(|i| i.parse::<usize>().ok())
                            .filter(|i| *i < studying.traces.len())
                        {
                            studying.traces.remove(i);
                        }
                    }
                }
            }
            _ => return None,
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

    /// Linked copies of a body: how many and how far apart; OK inserts them
    /// in a row, to be dragged where they go.
    fn copies_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        body: BodyId,
        (count, step, around): (u32, [f32; 3], Option<crate::Around>),
        mirror: Option<([f32; 3], [f32; 3])>,
    ) -> TaskOutcome {
        if request.cancel {
            self.task = None;
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
                    self.task = None;
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
            self.task = None;
            return TaskOutcome::Accepted {
                label: "Insert linked copies".to_string(),
            };
        }
        self.declared_panel(ui, ctx)
    }

    /// How many copies and how far apart, or turned about an axis, or one
    /// mirrored across a plane.
    fn copies_widgets(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        (count, step, around): (u32, [f32; 3], Option<crate::Around>),
        mirror: Option<([f32; 3], [f32; 3])>,
    ) -> Vec<Widget> {
        let mut widgets = vec![
            w::header("clone", "Insert linked copies"),
            w::value("Of", body_name(ctx, body)),
            Widget::Number {
                id: "count".into(),
                label: "How many".into(),
                value: f64::from(count),
                dim: Dim::Number,
                bind: None,
                min: Some(1.0),
                max: Some(500.0),
                decimals: 0,
                error: None,
            },
            w::hinted(
                "One copy mirrored across a plane, rather than copies as they are",
                w::toggle("mirror", "A mirror image", mirror.is_some()),
            ),
        ];
        if let Some((point, normal)) = mirror {
            let selected = AXES.iter().position(|a| *a == normal).unwrap_or(0);
            widgets.push(choice("plane", "Across", &["YZ", "XZ", "XY"], selected));
            for (k, label) in ["Through x", "Through y", "Through z"]
                .into_iter()
                .enumerate()
            {
                widgets.push(w::number(
                    &format!("mirror_{k}"),
                    label,
                    point[k],
                    Dim::Length,
                ));
            }
            widgets.push(w::text(
                "The mirror image takes the body's shape, mirrored, and follows every change \
                 to it.",
            ));
            return widgets;
        }
        widgets.push(w::hinted(
            "Turn the copies about an axis instead of setting them in a row",
            w::toggle("around", "Around an axis", around.is_some()),
        ));
        match around {
            Some((point, axis, angle)) => {
                let selected = AXES.iter().position(|a| *a == axis).unwrap_or(2);
                widgets.push(choice("axis", "Axis", &["X", "Y", "Z"], selected));
                for (k, label) in ["Through x", "Through y", "Through z"]
                    .into_iter()
                    .enumerate()
                {
                    widgets.push(w::number(
                        &format!("through_{k}"),
                        label,
                        point[k],
                        Dim::Length,
                    ));
                }
                widgets.push(w::number("over", "Over", angle, Dim::Angle));
            }
            None => {
                for (k, label) in ["Step x", "Step y", "Step z"].into_iter().enumerate() {
                    widgets.push(w::number(&format!("step_{k}"), label, step[k], Dim::Length));
                }
            }
        }
        widgets.push(w::text(
            "Each copy takes the body's shape and follows every change to it. They go in a \
             row, each a step from the one before; drag one to put it where it goes.",
        ));
        widgets
    }

    /// A change to the copies' settings, kept in the task until OK.
    fn copies_event(&mut self, event: &PanelEvent) -> Option<TaskOutcome> {
        let Some(Task::Copies {
            count,
            step,
            around,
            mirror,
            ..
        }) = &mut self.task
        else {
            return None;
        };
        let mut pivot = around.unwrap_or(([0.0; 3], [0.0, 0.0, 1.0], 360.0));
        let mut plane = mirror.unwrap_or(([0.0; 3], [1.0, 0.0, 0.0]));
        match event {
            PanelEvent::Toggle { id, on } if id == "mirror" => {
                *mirror = on.then_some(plane);
            }
            PanelEvent::Toggle { id, on } if id == "around" => {
                *around = on.then_some(pivot);
            }
            PanelEvent::Choice { id, index } if id == "plane" => {
                plane.1 = AXES[(*index).min(2)];
                *mirror = Some(plane);
            }
            PanelEvent::Choice { id, index } if id == "axis" => {
                pivot.1 = AXES[(*index).min(2)];
                *around = Some(pivot);
            }
            PanelEvent::Number { id, value } => {
                let value = *value as f32;
                let k = |prefix: &str| {
                    id.strip_prefix(prefix)
                        .and_then(|k| k.parse::<usize>().ok())
                        .filter(|k| *k < 3)
                };
                if id == "count" {
                    *count = value.round().max(1.0) as u32;
                } else if id == "over" {
                    pivot.2 = value;
                    *around = Some(pivot);
                } else if let Some(k) = k("mirror_") {
                    plane.0[k] = value;
                    *mirror = Some(plane);
                } else if let Some(k) = k("through_") {
                    pivot.0[k] = value;
                    *around = Some(pivot);
                } else if let Some(k) = k("step_") {
                    step[k] = value;
                }
            }
            _ => {}
        }
        None
    }

    /// A body and the one picked to take its place and joints.
    fn replace_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        old: BodyId,
        new: Option<BodyId>,
    ) -> TaskOutcome {
        if request.cancel {
            self.task = None;
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
                    self.task = None;
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
        self.declared_panel(ui, ctx)
    }

    /// The body replaced and the one taking its place.
    fn replace_widgets(
        ctx: &WorkbenchRuntimeContext,
        old: BodyId,
        new: Option<BodyId>,
    ) -> Vec<Widget> {
        vec![
            w::header("carbon-copy", "Replace body"),
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
    fn group_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        editing: Option<FeatureId>,
        members: &[BodyId],
    ) -> TaskOutcome {
        if request.cancel {
            self.task = None;
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
            self.task = None;
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
        self.declared_panel(ui, ctx)
    }

    /// The bodies picked for a group, each with its Remove; the group's
    /// Dissolve when it is one already.
    fn group_widgets(
        ctx: &WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        members: &[BodyId],
    ) -> Vec<Widget> {
        let mut widgets = vec![
            w::header("tree-group", "Rigid group"),
            w::text(
                "Click the bodies to lock together; a second click takes one out. They \
                 move as one, held as they sit now, the first the one the rest hold to.",
            ),
        ];
        for (i, body) in members.iter().enumerate() {
            widgets.push(w::row(vec![
                w::text(body_name(ctx, *body)),
                w::button(&format!("remove:{i}"), "Remove", ButtonStyle::Small),
            ]));
        }
        if members.len() < 2 {
            widgets.push(w::note(
                NoteKind::Info,
                None,
                "A group takes two bodies or more",
            ));
        }
        if editing.is_some() {
            widgets.push(w::hinted(
                "Remove the group; the bodies stay where they are",
                w::button("dissolve", "Dissolve group", ButtonStyle::Destructive),
            ));
        }
        widgets
    }

    /// A body taken out of the group, or the group dissolved.
    fn group_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let PanelEvent::Button { id: button } = event else {
            return None;
        };
        if let Some(i) = button
            .strip_prefix("remove:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            if let Some(Task::Group { members, .. }) = &mut self.task
                && i < members.len()
            {
                members.remove(i);
            }
            return None;
        }
        let id = editing.filter(|_| button == "dissolve")?;
        ctx.document.remove_feature(id).ok()?;
        ctx.record(
            "doc.delete",
            crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
            serde_json::Value::Null,
        );
        self.task = None;
        ctx.active_document_object = None;
        Some(TaskOutcome::Accepted {
            label: "Dissolve rigid group".to_string(),
        })
    }

    /// The mass of the visible bodies at one density, their centre of mass,
    /// and each body's share.
    fn mass_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.measuring = None;
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_mass(ctx);
        if matches!(&self.task, Some(Task::Mass { found: None, .. })) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.declared_panel(ui, ctx)
    }

    /// The measuring's progress; then the density, the totals and each
    /// body's mass, a click on one selecting it.
    fn mass_widgets(
        &self,
        ctx: &WorkbenchRuntimeContext,
        found: Option<&crate::MassReport>,
        density: f32,
    ) -> Vec<Widget> {
        let mut widgets = vec![w::header("measure", "Mass")];
        let Some(report) = found else {
            let (done, total) = self.mass_progress().unwrap_or((0, 0));
            widgets.push(Widget::Progress {
                label: format!("Measuring {done} of {total} bodies"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            return widgets;
        };
        widgets.push(w::ranged(
            "density",
            "Density, g/cm³",
            density,
            Dim::Number,
            Some((0.0, 100.0)),
        ));
        let density = f64::from(density);
        let unit = ctx.document.display_unit();
        let line = |label: &str, value: String| Widget::Value {
            label: label.into(),
            value,
            mono: true,
        };
        widgets.push(line("Mass", mass_text(report.mass_g(density))));
        widgets.push(line(
            "Volume",
            core_document::format_volume_mm3(report.volume_mm3(), unit, 2),
        ));
        if let Some(c) = report.centre(density) {
            let f = |v: f64| core_document::format_length_mm(v as f32, unit, 2);
            widgets.push(line(
                "Centre of mass",
                format!("{}, {}, {}", f(c[0]), f(c[1]), f(c[2])),
            ));
        }
        widgets.push(Widget::Table {
            id: "bodies".into(),
            columns: vec!["Body".into(), "Mass".into()],
            rows: report
                .bodies
                .iter()
                .map(|b| vec![body_name(ctx, b.body), mass_text(b.mass_g(density))])
                .collect(),
            selected: None,
            editable: Vec::new(),
        });
        if report.skipped > 0 {
            widgets.push(w::text(format!(
                "{} visible bod{} without a closed solid left out",
                report.skipped,
                if report.skipped == 1 { "y" } else { "ies" }
            )));
        }
        widgets.push(w::button("again", "Measure again", ButtonStyle::Secondary));
        widgets
    }

    /// The density changed, a body clicked, or the bodies measured again.
    fn mass_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        found: Option<&crate::MassReport>,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        match event {
            PanelEvent::Number { id, value } if id == "density" => {
                if let Some(Task::Mass { density, .. }) = &mut self.task {
                    *density = *value as f32;
                }
            }
            PanelEvent::Select { index, .. } => {
                let body = found?.bodies.get(*index)?.body;
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            PanelEvent::Button { id } if id == "again" => {
                if let Some(Task::Mass { density, .. }) = self.task {
                    self.measure_mass(ctx, density);
                }
            }
            _ => {}
        }
        None
    }

    /// Every part, how many of it and its size, with a copy for a
    /// spreadsheet; its item number, whether it is bought and the added
    /// columns, kept in the document.
    fn parts_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        let parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
        if self.volumes.refresh(ctx.document, &parts, ctx.kernel) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let outcome = self.declared_panel(ui, ctx);
        if let Some(csv) = self.copied.take() {
            ui.ctx().copy_text(csv);
            ctx.log_info("Parts list copied");
        }
        outcome
    }

    /// The rows of the parts list as it shows: by component when asked
    /// for and there are components.
    fn parts_rows(
        document: &core_document::Document,
        parts: &[crate::Part],
        by_component: bool,
    ) -> Vec<crate::parts::LevelRow> {
        if by_component {
            return crate::parts::parts_by_component(document, parts);
        }
        parts
            .iter()
            .enumerate()
            .map(|(i, p)| crate::parts::LevelRow::Part {
                depth: 0,
                part: i,
                bodies: p.bodies.clone(),
            })
            .collect()
    }

    /// The parts list with each part's volume, as far as it is measured.
    fn measured_parts(&self, ctx: &WorkbenchRuntimeContext) -> Vec<crate::Part> {
        let mut parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
        self.volumes.fill(ctx.document, &mut parts);
        parts
    }

    fn parts_widgets(&self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        use bench_api::Cell;
        let parts = self.measured_parts(ctx);
        let unit = ctx.document.display_unit();
        let pending = if self.volumes.running() { "…" } else { "-" };
        let table = crate::parts::table_of(ctx.document)
            .map(|(_, t)| t)
            .unwrap_or_default();
        let total: usize = parts.iter().map(|p| p.bodies.len()).sum();
        let mut widgets = vec![
            w::header("file-document", "Parts list"),
            w::text(format!(
                "{} part{}, {total} bod{}",
                parts.len(),
                if parts.len() == 1 { "" } else { "s" },
                if total == 1 { "y" } else { "ies" }
            )),
        ];
        let components = !ctx.document.components().is_empty();
        if components {
            widgets.push(w::hinted(
                "Each component's parts under it, nested as the components are",
                w::toggle("by_component", "By component", table.by_component),
            ));
        }
        let text = |text: String, mono: bool, strong: bool| Cell::Text { text, mono, strong };
        let rows =
            Self::parts_rows(ctx.document, &parts, table.by_component && components)
                .iter()
                .map(|row| match row {
                    crate::parts::LevelRow::Component { depth, name, .. } => vec![
                        text(String::new(), false, false),
                        text(format!("{}{name}", "    ".repeat(*depth)), false, true),
                        text("1".into(), true, false),
                    ],
                    crate::parts::LevelRow::Part {
                        depth,
                        part,
                        bodies,
                    } => {
                        let part = &parts[*part];
                        let mut cells =
                            vec![
                        text(part.number.map_or("-".to_string(), |n| n.to_string()), true, false),
                        Cell::Link {
                            text: format!("{}{}", "    ".repeat(*depth), part.name),
                        },
                        text(bodies.len().to_string(), true, false),
                        Cell::Edit {
                            text: part.print.to_string(),
                        },
                        text(
                            part.size_mm.map_or_else(
                                || "-".to_string(),
                                |s| format!("{:.1} × {:.1} × {:.1}", s[0], s[1], s[2]),
                            ),
                            true,
                            false,
                        ),
                        text(
                            part.volume_mm3.map_or_else(
                                || pending.to_string(),
                                |v| core_document::format_volume_mm3(v, unit, 2),
                            ),
                            true,
                            false,
                        ),
                        text(
                            part.mass_g().map_or_else(|| pending.to_string(), mass_text),
                            true,
                            false,
                        ),
                        Cell::Check {
                            on: part.bought,
                            hint: Some(
                                "Bought rather than made: left out of exports and the slicer"
                                    .into(),
                            ),
                        },
                        text(part.threads_text(), false, false),
                    ];
                        cells.extend(table.columns.iter().map(|column| Cell::Edit {
                            text: part.values.get(column).cloned().unwrap_or_default(),
                        }));
                        cells
                    }
                })
                .collect();
        let mut columns: Vec<String> = PARTS_COLUMNS.into_iter().map(String::from).collect();
        columns.extend(table.columns.iter().cloned());
        widgets.push(Widget::Sheet {
            id: "parts".into(),
            columns,
            rows,
        });
        let preset = crate::parts::PRINT_MATERIALS
            .iter()
            .position(|(name, density)| {
                *name == table.material.name && *density == table.material.density
            });
        let mut options: Vec<String> = crate::parts::PRINT_MATERIALS
            .iter()
            .map(|(name, density)| format!("{name} ({density} g/cm³)"))
            .collect();
        options.push("Custom".into());
        widgets.push(w::hinted(
            "The filament the parts print in; a body with a material of its own is \
             weighed at its own",
            Widget::Choice {
                id: "print_material".into(),
                label: "Printed in".into(),
                selected: preset.unwrap_or(options.len() - 1),
                options,
            },
        ));
        if preset.is_none() {
            widgets.push(w::ranged(
                "print_density",
                "Density, g/cm³",
                table.material.density,
                Dim::Number,
                Some((0.01, 100.0)),
            ));
        }
        widgets.push(w::text(print_summary(&parts, &table.material.name, unit)));
        widgets.push(w::hinted(
            "Give every part without an item number the next one, in list order",
            w::button("number", "Number the parts", ButtonStyle::Secondary),
        ));
        widgets.push(w::row(vec![
            Widget::TextField {
                id: "new_column".into(),
                label: "New column".into(),
                value: self.parts_column.clone(),
            },
            w::button("add_column", "Add column", ButtonStyle::Small),
        ]));
        if !table.columns.is_empty() {
            widgets.push(w::row(
                table
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, column)| {
                        w::button(
                            &format!("remove_column:{i}"),
                            &format!("Remove {column}"),
                            ButtonStyle::Small,
                        )
                    })
                    .collect(),
            ));
        }
        widgets.push(w::row(vec![
            w::hinted(
                "For a spreadsheet: every column of the list",
                w::button("copy", "Copy as CSV", ButtonStyle::Secondary),
            ),
            w::hinted(
                "Write the list to a file a spreadsheet opens",
                w::button("save", "Save as CSV", ButtonStyle::Secondary),
            ),
        ]));
        widgets
    }

    /// A change to the parts list, kept as `asm.parts_table` keeps it; a
    /// part clicked, selected; the list copied or saved as CSV.
    fn parts_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let parts = self.measured_parts(ctx);
        let mut table = crate::parts::table_of(ctx.document)
            .map(|(_, t)| t)
            .unwrap_or_default();
        let before = table.clone();
        let components = !ctx.document.components().is_empty();
        let rows = Self::parts_rows(ctx.document, &parts, table.by_component && components);
        let part_at = |row: usize| match rows.get(row)? {
            crate::parts::LevelRow::Part { part, bodies, .. } => Some((&parts[*part], bodies)),
            crate::parts::LevelRow::Component { .. } => None,
        };
        const FIXED: usize = PARTS_COLUMNS.len();
        match event {
            PanelEvent::Toggle { id, on } if id == "by_component" => table.by_component = *on,
            PanelEvent::Choice { id, index } if id == "print_material" => {
                table.material = match crate::parts::PRINT_MATERIALS.get(*index) {
                    Some((name, _)) => crate::parts::PrintMaterial::named(name, None),
                    None => crate::parts::PrintMaterial {
                        name: "Custom".into(),
                        density: table.material.density,
                    },
                };
            }
            PanelEvent::Number { id, value } if id == "print_density" && *value > 0.0 => {
                table.material.density = *value as f32;
            }
            PanelEvent::CellText {
                row, column, value, ..
            } if *column == PRINT_COLUMN => {
                let (part, _) = part_at(*row)?;
                let value = value.trim();
                let count = if value.is_empty() {
                    None
                } else {
                    Some(value.parse::<u32>().ok()?)
                };
                table.entry_mut(&part.bodies).print = count;
            }
            PanelEvent::Select { index, .. } => {
                let (_, bodies) = part_at(*index)?;
                ctx.request(core_document::HostRequest::SelectBody(*bodies.first()?));
            }
            PanelEvent::CellCheck { row, on, .. } => {
                let (part, _) = part_at(*row)?;
                table.set_bought(part, *on);
            }
            PanelEvent::CellText {
                row, column, value, ..
            } => {
                let (part, _) = part_at(*row)?;
                let column = table.columns.get(column.checked_sub(FIXED)?)?.clone();
                table
                    .entry_mut(&part.bodies)
                    .values
                    .insert(column, value.clone());
            }
            PanelEvent::Text { id, value } if id == "new_column" => {
                self.parts_column = value.clone();
            }
            PanelEvent::Button { id } => match id.as_str() {
                "number" => table.number(&parts),
                "add_column" => {
                    let name = self.parts_column.trim().to_string();
                    if !name.is_empty() && !table.columns.contains(&name) {
                        table.columns.push(name);
                        self.parts_column.clear();
                    }
                }
                "copy" | "save" => {
                    let csv = if table.by_component && components {
                        crate::parts::levels_csv(&parts, &rows, &table.columns)
                    } else {
                        crate::parts_csv(&parts, &table.columns)
                    };
                    if id == "copy" {
                        self.copied = Some(csv);
                    } else {
                        ctx.request(core_document::HostRequest::SaveFile {
                            name: "parts.csv".into(),
                            kind: "Comma-separated values".into(),
                            extension: "csv".into(),
                            contents: csv.into_bytes(),
                        });
                    }
                }
                other => {
                    let i = other
                        .strip_prefix("remove_column:")
                        .and_then(|i| i.parse::<usize>().ok())?;
                    let column = table.columns.get(i)?.clone();
                    table.columns.retain(|c| *c != column);
                    for entry in table.entries.values_mut() {
                        entry.values.remove(&column);
                    }
                }
            },
            _ => {}
        }
        if table != before {
            let args = crate::commands::object(serde_json::json!({
                "table": serde_json::to_value(&table).unwrap_or_default(),
            }));
            match crate::commands::run("asm.parts_table", &args, ctx) {
                Ok(_) => {
                    ctx.record("asm.parts_table", args, serde_json::Value::Null);
                    ctx.request(core_document::HostRequest::JournalLabel(
                        "Edit parts list".into(),
                    ));
                }
                Err(why) => ctx.log_warn(format!("Could not keep the parts list: {why}")),
            }
        }
        None
    }

    /// End a sweep, the drive back at the value it started from.
    fn stop_playing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(play) = self.playing.take() else {
            return;
        };
        let Some(node) = ctx.document.get_feature_meta(play.joint) else {
            return;
        };
        let Ok(mut joint) = JointFeature::from_json(&node.data) else {
            return;
        };
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = &mut joint.kind {
            drive.to = drive.to.map(|_| play.start);
        }
        if let Err(why) = ctx
            .document
            .update_feature_data(play.joint, joint.to_json())
        {
            ctx.log_warn(why.to_string());
        }
        ctx.document.clear_feature_dirty(play.joint);
        self.solve_and_apply(ctx);
    }

    fn joint_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        let created = before.is_none();
        if request.cancel {
            self.playing = None;
            match &before {
                Some(data) => {
                    if let Err(why) = ctx.document.update_feature_data(id, data.clone()) {
                        ctx.log_warn(why.to_string());
                    }
                    ctx.document.clear_feature_dirty(id);
                }
                None => {
                    if let Err(why) = ctx.document.remove_feature(id) {
                        ctx.log_warn(why.to_string());
                    }
                    ctx.active_document_object = None;
                }
            }
            restore_placements(ctx, placements);
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.stop_playing(ctx);
            crate::commands::record_joint(ctx, id, before.as_ref(), placements);
            self.task = None;
            ctx.active_document_object = None;
            return TaskOutcome::Accepted {
                label: if created { "Add joint" } else { "Edit joint" }.to_string(),
            };
        }
        let Some(node) = ctx.document.get_feature_meta(id).cloned() else {
            self.task = None;
            return TaskOutcome::Cancelled;
        };
        if JointFeature::from_json(&node.data).is_ok() {
            let dt = f64::from(ui.input(|i| i.stable_dt).min(0.1));
            if self.advance_play(ctx, id, dt) {
                ui.ctx().request_repaint();
            }
        }
        self.collect_sweep(ctx);
        if self.sweep_progress().is_some_and(|(joint, ..)| joint == id) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.declared_panel(ui, ctx)
    }

    /// Move a swept drive on by `dt` seconds, back and forth through its
    /// range every four seconds; whether it moved. A formula holding the
    /// drive's value stops the sweep, which would fight it every frame.
    fn advance_play(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId, dt: f64) -> bool {
        let Some(joint) = ctx
            .document
            .get_feature_data(id)
            .and_then(|d| JointFeature::from_json(d).ok())
        else {
            return false;
        };
        let (drive, base) = match &joint.kind {
            JointKind::Hinge { drive, .. } => (drive, "/kind/Hinge/drive"),
            JointKind::Slider { drive, .. } => (drive, "/kind/Slider/drive"),
            _ => return false,
        };
        if drive.to.is_none() {
            return false;
        }
        if ctx
            .document
            .feature_formula(id, &format!("{base}/to"))
            .is_some()
        {
            self.playing = self.playing.filter(|p| p.joint != id);
            return false;
        }
        let Some(play) = self.playing.as_mut().filter(|p| p.joint == id) else {
            return false;
        };
        let angular = matches!(joint.kind, JointKind::Hinge { .. });
        let (low, high) = sweep_range(drive.limits, angular, f64::from(play.start));
        play.phase += dt * std::f64::consts::TAU / 4.0;
        let to = (low + (high - low) * (0.5 - 0.5 * play.phase.cos())) as f32;
        let _ = self.call(
            ctx,
            "asm.set",
            serde_json::json!({"joint": id.0.to_string(), "drive": to}),
        );
        true
    }

    /// A joint's settings: its bodies, its kind and faces, what it holds
    /// and how its motion is driven or limited, the check of that motion
    /// for collisions, a turn of its body, then what the last solve said
    /// and the joint's Delete.
    fn joint_widgets(&self, ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Vec<Widget> {
        let Some(node) = ctx.document.get_feature_meta(id) else {
            return Vec::new();
        };
        let Ok(joint) = JointFeature::from_json(&node.data) else {
            return vec![w::note(
                NoteKind::Error,
                Some("Unreadable joint"),
                "The stored joint does not parse.",
            )];
        };
        let (now, align_now) = joint_now(ctx, node.body, &joint);
        let bound = |key: &str, label: &str, value: f32, dim: Dim, path: &str, hint: &str| {
            w::hinted(hint, w::bound(key, label, value, dim, (id, path)))
        };
        let mut widgets = vec![
            w::header(joint.kind.icon(), &node.name),
            w::value(
                "Moves",
                node.body.map(|b| body_name(ctx, b)).unwrap_or_default(),
            ),
            w::value("Against", body_name(ctx, joint.other_body)),
        ];
        if let Some(current) = JointTool::of_kind(&joint.kind) {
            widgets.push(Widget::Choice {
                id: "kind".into(),
                label: "Kind".into(),
                options: JointTool::ALL
                    .iter()
                    .map(|t| t.label().to_string())
                    .collect(),
                selected: JointTool::ALL
                    .iter()
                    .position(|t| *t == current)
                    .unwrap_or(0),
            });
            widgets.push(w::hinted(
                "Pick the two faces afresh; the joint keeps its name and kind",
                w::button("repick", "Pick faces again", ButtonStyle::Secondary),
            ));
        }
        let drives = |prefix: &str, base: &str, drive: &crate::Drive, angular, sweep, now| {
            self.drive_widgets(ctx, id, (prefix, base), drive, (angular, sweep), now)
        };
        match &joint.kind {
            JointKind::Mate { flip, offset } => {
                widgets.push(bound(
                    "offset",
                    "Gap",
                    *offset,
                    Dim::Length,
                    "/kind/Mate/offset",
                    "How far apart the two faces sit",
                ));
                widgets.push(w::hinted(
                    "The faces point the same way instead of at each other",
                    w::toggle("flip", "Same way", *flip),
                ));
            }
            JointKind::Angle { degrees } => {
                widgets.push(bound(
                    "degrees",
                    "Angle",
                    *degrees,
                    Dim::Angle,
                    "/kind/Angle/degrees",
                    "Between the faces' outward normals: 180 faces them at each other",
                ));
                widgets.push(w::text(
                    "Only the turn is held: pair it with a mate or an alignment to \
                     say where the body sits.",
                ));
            }
            JointKind::Ground => widgets.push(w::text(
                "The body stays where it is; the bodies joined to it are \
                 placed against it.",
            )),
            JointKind::Align { turn, slide, .. } => {
                let (turn_now, slide_now) = align_now.unzip();
                widgets.push(Widget::Group {
                    title: "Turn".into(),
                    open: true,
                    children: drives("turn", "/kind/Align/turn", turn, true, false, turn_now),
                });
                widgets.push(Widget::Group {
                    title: "Slide".into(),
                    open: true,
                    children: drives("slide", "/kind/Align/slide", slide, false, false, slide_now),
                });
                widgets.push(w::text(
                    "The body can turn about the axis and slide along it, each \
                     free, held or kept within limits.",
                ));
            }
            JointKind::Hinge { offset, drive, .. } => {
                widgets.push(bound(
                    "offset",
                    "Height",
                    *offset,
                    Dim::Length,
                    "/kind/Hinge/offset",
                    "How far along the axis the body sits from the other",
                ));
                widgets.extend(drives("drive", "/kind/Hinge/drive", drive, true, true, now));
                widgets.push(w::text(
                    "The body can only turn about the axis. Its angle counts from \
                     where it sat when the joint was made.",
                ));
            }
            JointKind::Distance { offset } => {
                widgets.push(bound(
                    "offset",
                    "Distance",
                    *offset,
                    Dim::Length,
                    "/kind/Distance/offset",
                    "Along the other face's normal",
                ));
                widgets.push(w::text(
                    "Only the distance is held: the faces may turn and slide past \
                     each other.",
                ));
            }
            JointKind::Tangent { radius } => {
                widgets.push(bound(
                    "radius",
                    "Radius",
                    *radius,
                    Dim::Length,
                    "/kind/Tangent/radius",
                    "The round face's radius",
                ));
                widgets.push(w::text(
                    "The round face rests on the flat one; it can still roll and \
                     slide along it.",
                ));
            }
            JointKind::Slider { drive, .. } => {
                widgets.extend(drives(
                    "drive",
                    "/kind/Slider/drive",
                    drive,
                    false,
                    true,
                    now,
                ));
                widgets.push(w::text(
                    "The body can only slide along the axis, turned as it was when \
                     the joint was made.",
                ));
            }
            JointKind::Fixed { shift, .. } => {
                for (k, label) in ["Shift x", "Shift y", "Shift z"].into_iter().enumerate() {
                    widgets.push(bound(
                        &format!("shift_{k}"),
                        label,
                        shift[k] as f32,
                        Dim::Length,
                        &format!("/kind/Fixed/shift/{k}"),
                        "Where the body sits from the other, along the other's own axes",
                    ));
                }
                widgets.push(w::text(
                    "The body is held to the other as it sat when the joint was made; \
                     it moves only with it.",
                ));
            }
            JointKind::Parallel => widgets.push(w::text(
                "Only the turn is held, the faces parallel: pair it with other joints \
                 to say where the body sits.",
            )),
            JointKind::Perpendicular => widgets.push(w::text(
                "Only the turn is held, the faces square: pair it with other joints \
                 to say where the body sits.",
            )),
            JointKind::Ball => widgets.push(w::text(
                "The two points are held as one; the body can still turn every way \
                 about them.",
            )),
            JointKind::Universal => widgets.push(w::text(
                "The pins cross at one point, square to each other: the body turns \
                 about either pin.",
            )),
            JointKind::Slot => widgets.push(w::text(
                "The pin stays on the slot's line: it slides along it and turns every way.",
            )),
            JointKind::Width => widgets.push(w::text(
                "The tab's two faces stay centred between the slot's two walls: it \
                 slides along the slot and turns in it.",
            )),
            JointKind::Path => widgets.push(w::text(
                "The point stays on the edge, whatever its shape: it runs along it and \
                 turns every way.",
            )),
            JointKind::Cam { radius } => {
                widgets.push(bound(
                    "radius",
                    "Radius",
                    *radius,
                    Dim::Length,
                    "/kind/Cam/radius",
                    "The follower's roller radius; 0 for a point",
                ));
                widgets.push(w::text(
                    "The follower stays on the cam's face, a radius off it.",
                ));
            }
        }
        if joint.kind != JointKind::Ground {
            for (end, (key, label)) in [("moving_end", "Moving end"), ("fixed_end", "Fixed end")]
                .into_iter()
                .enumerate()
            {
                widgets.push(bound(
                    key,
                    label,
                    joint.ends[end],
                    Dim::Length,
                    &format!("/ends/{end}"),
                    "How far this end moves along its own normal or axis before the \
                     joint holds it",
                ));
            }
        }
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = joint.kind {
            let hinge = matches!(joint.kind, JointKind::Hinge { .. });
            let range = motion_range(drive.limits, hinge, now);
            widgets.extend(self.sweep_check_widgets(
                ctx,
                id,
                range,
                if hinge { "°" } else { " mm" },
            ));
        }
        if joint.kind != JointKind::Ground {
            widgets.push(w::row(vec![
                w::number("turn_by", "", self.turn_by.unwrap_or(90.0), Dim::Angle),
                w::hinted(
                    "Turn the body about the joint's axis or normal by this much",
                    w::button("turn", "Turn", ButtonStyle::Secondary),
                ),
                w::hinted(
                    "Half a turn across the joint: the body the other way round",
                    w::button("turn_over", "Turn over", ButtonStyle::Secondary),
                ),
            ]));
        }
        if joint.kind.keeps_a_turn() {
            widgets.push(w::hinted(
                "Turn what the joint counts its turn from a quarter turn: the body \
                 follows, and the joint reads as it did",
                w::button("turn90", "Turn 90°", ButtonStyle::Secondary),
            ));
        }
        widgets.extend(self.verdict_note());
        if self.redundant_now(ctx).iter().any(|(j, _)| *j == id) {
            widgets.push(w::note(
                NoteKind::Warning,
                Some("Redundant"),
                "The body's other joints already hold everything this one does; it could go.",
            ));
        }
        widgets.extend(self.solver_messages(ctx));
        widgets.extend(node.body.and_then(|body| freedom_text(ctx, body)));
        widgets.push(w::hinted(
            "Remove the joint; the bodies stay where they are",
            w::button("delete", "Delete joint", ButtonStyle::Destructive),
        ));
        widgets
    }

    /// A hinge's or a slider's drive, or an alignment's turn or slide:
    /// where it stands, held at a value, kept within limits and, while
    /// held, swept through its range to show the motion or record it.
    /// The fields' ids begin with `prefix`; `base` is the drive's place in
    /// the joint's data.
    fn drive_widgets(
        &self,
        ctx: &WorkbenchRuntimeContext,
        joint: FeatureId,
        (prefix, base): (&str, &str),
        drive: &crate::Drive,
        (angular, sweep): (bool, bool),
        now: Option<f64>,
    ) -> Vec<Widget> {
        let (dim, unit) = if angular {
            (Dim::Angle, "°")
        } else {
            (Dim::Length, " mm")
        };
        let mut widgets = Vec::new();
        if let Some(now) = now {
            widgets.push(w::value(
                if angular { "Angle now" } else { "Position now" },
                format!("{now:.2}{unit}"),
            ));
        }
        widgets.push(w::hinted(
            if angular {
                "Hold the hinge at an angle"
            } else {
                "Hold the slider at a position"
            },
            w::toggle(&format!("{prefix}.drive"), "Drive", drive.to.is_some()),
        ));
        if let Some(to) = drive.to {
            widgets.push(w::hinted(
                "Where the drive holds it",
                w::bound(
                    &format!("{prefix}.to"),
                    if angular { "Angle" } else { "Position" },
                    to,
                    dim,
                    (joint, &format!("{base}/to")),
                ),
            ));
        }
        widgets.push(w::hinted(
            "Keep the motion within a range while it is not driven",
            w::toggle(
                &format!("{prefix}.limits"),
                "Limits",
                drive.limits.is_some(),
            ),
        ));
        if let Some([low, high]) = drive.limits {
            for (end, value, label) in [(0, low, "Lowest"), (1, high, "Highest")] {
                widgets.push(w::hinted(
                    "An end of the range the motion stays in",
                    w::bound(
                        &format!("{prefix}.{}", ["low", "high"][end]),
                        label,
                        value,
                        dim,
                        (joint, &format!("{base}/limits/{end}")),
                    ),
                ));
            }
        }
        if drive.to.is_none() || !sweep {
            return widgets;
        }
        widgets.push(w::hinted(
            "Save the sweep through its range as an animation",
            w::button(
                &format!("{prefix}.record"),
                "Record",
                ButtonStyle::Secondary,
            ),
        ));
        if ctx
            .document
            .feature_formula(joint, &format!("{base}/to"))
            .is_none()
        {
            let mine = self.playing.is_some_and(|p| p.joint == joint);
            widgets.push(w::hinted(
                "Sweep the drive through its range; stopping puts it back",
                w::button(
                    &format!("{prefix}.play"),
                    if mine { "Stop" } else { "Play" },
                    ButtonStyle::Secondary,
                ),
            ));
        }
        widgets
    }

    /// The check of a joint's motion for collisions: its button, its
    /// progress, what it found.
    fn sweep_check_widgets(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: FeatureId,
        (low, high): (f32, f32),
        unit: &str,
    ) -> Vec<Widget> {
        if let Some((joint, done, total)) = self.sweep_progress()
            && joint == id
        {
            return vec![
                Widget::Progress {
                    label: format!("Checking the motion: {done} of {total} pairs"),
                    fraction: Some(if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    }),
                    job: None,
                },
                w::button("stop_sweep", "Stop", ButtonStyle::Secondary),
            ];
        }
        let mut widgets = vec![w::hinted(
            format!(
                "Step the drive from {low:.1}{unit} to {high:.1}{unit} and look for \
                 bodies that share material on the way"
            ),
            w::button(
                "check_sweep",
                "Check collisions through the motion",
                ButtonStyle::Secondary,
            ),
        )];
        match &self.motion_clashes {
            Some((joint, Ok(found))) if *joint == id && found.is_empty() => widgets.push(w::note(
                NoteKind::Success,
                None,
                "No collisions through the motion",
            )),
            Some((joint, Ok(found))) if *joint == id => {
                widgets.push(w::note(
                    NoteKind::Error,
                    Some(&format!(
                        "{} collision{}",
                        found.len(),
                        if found.len() == 1 { "" } else { "s" }
                    )),
                    "Click one to select its first body",
                ));
                widgets.push(Widget::List {
                    id: "motion_clashes".into(),
                    items: found
                        .iter()
                        .map(|clash| bench_api::ListItem {
                            label: format!(
                                "At {:.1}{unit}: {} and {}, {:.2} mm³",
                                clash.at,
                                body_name(ctx, clash.a),
                                body_name(ctx, clash.b),
                                clash.volume_mm3
                            ),
                            detail: None,
                            icon: None,
                        })
                        .collect(),
                    selected: None,
                });
            }
            Some((joint, Err(why))) if *joint == id => {
                widgets.push(w::note(NoteKind::Error, Some("Not checked"), why.clone()))
            }
            _ => {}
        }
        widgets
    }

    /// A change to a joint's settings, made as `asm.set` makes it; its
    /// kind or faces changed, its motion swept, recorded or checked, its
    /// body turned, or the joint deleted.
    fn joint_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        (id, created, placements): (FeatureId, bool, &[(BodyId, BodyPlacement)]),
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let node = ctx.document.get_feature_meta(id)?.clone();
        let mut joint = JointFeature::from_json(&node.data).ok()?;
        let (now, align_now) = joint_now(ctx, node.body, &joint);
        let set = |this: &mut Self, ctx: &mut WorkbenchRuntimeContext, args: serde_json::Value| {
            let mut args = args;
            args["joint"] = serde_json::json!(id.0.to_string());
            let _ = this.call(ctx, "asm.set", args);
        };
        let (key, rest) = match event {
            PanelEvent::Number { id: key, .. }
            | PanelEvent::Toggle { id: key, .. }
            | PanelEvent::Button { id: key }
            | PanelEvent::Choice { id: key, .. }
            | PanelEvent::Select { id: key, .. } => key
                .split_once('.')
                .map_or((key.as_str(), ""), |(a, b)| (a, b)),
            _ => return None,
        };
        // A drive's or a limit's field: which drive, and the joint's word
        // for its value and its range.
        let drive_of = |joint: &JointFeature, prefix: &str| match (&joint.kind, prefix) {
            (JointKind::Hinge { drive, .. }, "drive") => {
                Some((*drive, "/kind/Hinge/drive", "drive", "limits", true, now))
            }
            (JointKind::Slider { drive, .. }, "drive") => {
                Some((*drive, "/kind/Slider/drive", "drive", "limits", false, now))
            }
            (JointKind::Align { turn, .. }, "turn") => Some((
                *turn,
                "/kind/Align/turn",
                "turn_drive",
                "turn_limits",
                true,
                align_now.map(|a| a.0),
            )),
            (JointKind::Align { slide, .. }, "slide") => Some((
                *slide,
                "/kind/Align/slide",
                "slide_drive",
                "slide_limits",
                false,
                align_now.map(|a| a.1),
            )),
            _ => None,
        };
        if !rest.is_empty() {
            let (drive, base, to_arg, range_arg, angular, now) = drive_of(&joint, key)?;
            match (rest, event) {
                ("drive", PanelEvent::Toggle { on, .. }) => {
                    if *on {
                        set(
                            self,
                            ctx,
                            serde_json::json!({to_arg: now.unwrap_or(0.0) as f32}),
                        );
                    } else {
                        self.clear_formula(ctx, id, &format!("{base}/to"));
                        self.playing = None;
                        set(self, ctx, serde_json::json!({to_arg: false}));
                    }
                }
                ("to", PanelEvent::Number { value, .. }) => {
                    set(self, ctx, serde_json::json!({to_arg: *value as f32}));
                }
                ("limits", PanelEvent::Toggle { on, .. }) => {
                    if *on {
                        let at = now.unwrap_or(0.0) as f32;
                        let range = if angular {
                            [(at - 45.0).max(-180.0), (at + 45.0).min(180.0)]
                        } else {
                            [at - 10.0, at + 10.0]
                        };
                        set(self, ctx, serde_json::json!({range_arg: range}));
                    } else {
                        for end in 0..2 {
                            self.clear_formula(ctx, id, &format!("{base}/limits/{end}"));
                        }
                        set(self, ctx, serde_json::json!({range_arg: false}));
                    }
                }
                ("low" | "high", PanelEvent::Number { value, .. }) => {
                    let [mut low, mut high] = drive.limits?;
                    if rest == "low" {
                        low = *value as f32;
                    } else {
                        high = *value as f32;
                    }
                    if low > high {
                        std::mem::swap(&mut low, &mut high);
                    }
                    set(self, ctx, serde_json::json!({range_arg: [low, high]}));
                }
                ("record", _) => {
                    let to = drive.to?;
                    let mine = self.playing.filter(|p| p.joint == id);
                    let centre = f64::from(mine.map_or(to, |p| p.start));
                    let (low, high) = sweep_range(drive.limits, angular, centre);
                    let frames = crate::sweep_frames(
                        ctx.document,
                        id,
                        low as f32,
                        high as f32,
                        SWEEP_FRAMES,
                    );
                    if frames.is_empty() {
                        ctx.log_warn("Nothing moves through this joint's range");
                    } else {
                        ctx.request(core_document::HostRequest::RecordAnimation {
                            name: node.name.clone(),
                            frames,
                            frame_ms: 4000 / SWEEP_FRAMES as u32,
                        });
                    }
                }
                ("play", _) => {
                    let to = drive.to?;
                    let back = match self.playing.filter(|p| p.joint == id) {
                        Some(p) => {
                            self.playing = None;
                            p.start
                        }
                        None => {
                            let (low, high) = sweep_range(drive.limits, angular, f64::from(to));
                            let span = (high - low).max(1e-6);
                            let from = ((f64::from(to) - low) / span).clamp(0.0, 1.0);
                            self.playing = Some(crate::Play {
                                joint: id,
                                start: to,
                                phase: (1.0 - 2.0 * from).acos(),
                            });
                            to
                        }
                    };
                    set(self, ctx, serde_json::json!({to_arg: back}));
                }
                _ => {}
            }
            return None;
        }
        match (key, event) {
            ("kind", PanelEvent::Choice { index, .. }) => {
                let current = JointTool::of_kind(&joint.kind)?;
                let tool = *JointTool::ALL.get(*index).filter(|t| **t != current)?;
                if tool.fits(&joint.moving, &joint.fixed) {
                    self.change_kind(ctx, id, created, placements, tool);
                } else {
                    // Faces of the wrong sort for it: pick them afresh.
                    if created {
                        crate::commands::record_joint(ctx, id, None, placements);
                    }
                    self.start_repick(id, tool);
                }
                return Some(TaskOutcome::Open);
            }
            ("repick", _) => {
                let current = JointTool::of_kind(&joint.kind)?;
                if created {
                    crate::commands::record_joint(ctx, id, None, placements);
                }
                self.start_repick(id, current);
                return Some(TaskOutcome::Open);
            }
            (
                "offset" | "degrees" | "radius" | "moving_end" | "fixed_end",
                PanelEvent::Number { value, .. },
            ) => {
                set(self, ctx, serde_json::json!({key: *value as f32}));
            }
            ("flip", PanelEvent::Toggle { on, .. }) => {
                set(self, ctx, serde_json::json!({"flip": *on}));
            }
            (shift, PanelEvent::Number { value, .. }) if shift.starts_with("shift_") => {
                // A fixed joint's shift, which `asm.set` does not take.
                let k = shift["shift_".len()..]
                    .parse::<usize>()
                    .ok()
                    .filter(|k| *k < 3)?;
                let JointKind::Fixed { shift, .. } = &mut joint.kind else {
                    return None;
                };
                shift[k] = f64::from(*value as f32);
                if let Err(why) = ctx.document.update_feature_data(id, joint.to_json()) {
                    ctx.log_warn(why.to_string());
                }
                ctx.document.clear_feature_dirty(id);
                self.solve_and_apply(ctx);
            }
            ("check_sweep", _) => {
                if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = joint.kind
                {
                    let hinge = matches!(joint.kind, JointKind::Hinge { .. });
                    let range = motion_range(drive.limits, hinge, now);
                    self.check_sweep(ctx, id, range);
                }
            }
            ("stop_sweep", _) => self.sweeping = None,
            ("motion_clashes", PanelEvent::Select { index, .. }) => {
                let Some((joint, Ok(found))) = &self.motion_clashes else {
                    return None;
                };
                let clash = found.get(*index).filter(|_| *joint == id)?;
                ctx.request(core_document::HostRequest::SelectBody(clash.a));
            }
            ("turn_by", PanelEvent::Number { value, .. }) => self.turn_by = Some(*value as f32),
            ("turn", _) => {
                let by = f64::from(self.turn_by.unwrap_or(90.0));
                self.turn_in_task(ctx, id, created, placements, by, false);
            }
            ("turn_over", _) => self.turn_in_task(ctx, id, created, placements, 0.0, true),
            ("turn90", _) => {
                let args = serde_json::json!({"joint": id.0.to_string()});
                self.run_in_task(ctx, id, created, placements, "asm.turn90", args);
            }
            ("solver_messages", PanelEvent::Select { index, .. }) => {
                let (joint, _, _) = self.messages(ctx).into_iter().nth(*index)?;
                ctx.request(core_document::HostRequest::SelectFeatures(vec![joint]));
            }
            ("delete", _) => {
                ctx.document.remove_feature(id).ok()?;
                self.playing = None;
                // A joint the task made has nothing to undo in a recording.
                if !created {
                    ctx.record(
                        "doc.delete",
                        crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
                        serde_json::Value::Null,
                    );
                }
                self.task = None;
                ctx.active_document_object = None;
                return Some(TaskOutcome::Accepted {
                    label: "Delete joint".to_string(),
                });
            }
            _ => {}
        }
        None
    }

    /// Take the formula off a joint's number, which then keeps the value it
    /// came to.
    fn clear_formula(&self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId, key: &str) {
        if ctx.document.feature_formula(id, key).is_some()
            && let Err(why) = ctx.document.set_feature_formula(id, key, None)
        {
            ctx.log_warn(why.to_string());
        }
    }

    /// Make the joint another kind, from where the bodies stand, as
    /// `asm.set` with a `kind` does.
    fn change_kind(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        created: bool,
        placements: &[(BodyId, BodyPlacement)],
        tool: JointTool,
    ) {
        if created {
            crate::commands::record_joint(ctx, id, None, placements);
        }
        let args = crate::commands::object(
            serde_json::json!({"joint": id.0.to_string(), "kind": tool.word()}),
        );
        match crate::commands::run("asm.set", &args, ctx) {
            Ok(_) => ctx.record("asm.set", args, serde_json::Value::Null),
            Err(why) => ctx.log_warn(why.to_string()),
        }
        self.task = Some(crate::Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: crate::all_placements(ctx),
        });
    }

    /// Turn the joint's body from its task, as `asm.turn` or `asm.flip`
    /// turns it.
    fn turn_in_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        created: bool,
        placements: &[(BodyId, BodyPlacement)],
        degrees: f64,
        over: bool,
    ) {
        let (command, args) = if over {
            ("asm.flip", serde_json::json!({"joint": id.0.to_string()}))
        } else {
            (
                "asm.turn",
                serde_json::json!({"joint": id.0.to_string(), "degrees": degrees}),
            )
        };
        // A hinge's motion moves on, a coupled one taking its partner along.
        self.run_in_task(ctx, id, created, placements, command, args);
    }

    /// Run `command` on the joint from its task. A joint the task just
    /// made is recorded first, so the recording has it to turn; the task
    /// goes on as an edit of it from where the command left it.
    fn run_in_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        created: bool,
        placements: &[(BodyId, BodyPlacement)],
        command: &str,
        args: serde_json::Value,
    ) {
        if !crate::joints(ctx.document).iter().any(|j| j.id == id) {
            return;
        }
        if created {
            crate::commands::record_joint(ctx, id, None, placements);
        }
        let args = crate::commands::object(args);
        if let Err(why) = crate::commands::run(command, &args, ctx) {
            ctx.log_warn(format!("Could not turn the body: {why}"));
            return;
        }
        self.solve_and_apply(ctx);
        ctx.record(command, args, serde_json::Value::Null);
        self.task = Some(crate::Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: crate::all_placements(ctx),
        });
    }

    /// A coupling's settings: its two joints, what ties them, the ratio.
    /// Each change is made as `asm.set` makes it and solves at once.
    fn coupling_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        let created = before.is_none();
        if request.cancel {
            match &before {
                Some(data) => {
                    if let Err(why) = ctx.document.update_feature_data(id, data.clone()) {
                        ctx.log_warn(why.to_string());
                    }
                    ctx.document.clear_feature_dirty(id);
                }
                None => {
                    if let Err(why) = ctx.document.remove_feature(id) {
                        ctx.log_warn(why.to_string());
                    }
                    ctx.active_document_object = None;
                }
            }
            restore_placements(ctx, placements);
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            crate::commands::record_coupling(ctx, id, before.as_ref());
            self.task = None;
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
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        self.declared_panel(ui, ctx)
    }

    /// A coupling's settings: its two joints, what ties them, the ratio,
    /// then what the last change's solve said and the coupling's Delete.
    fn coupling_widgets(&self, ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Vec<Widget> {
        let Some(node) = ctx.document.get_feature_meta(id) else {
            return Vec::new();
        };
        let Ok(coupling) = Coupling::from_json(&node.data) else {
            return vec![w::note(
                NoteKind::Error,
                Some("Unreadable coupling"),
                "The stored coupling does not parse.",
            )];
        };
        let movable = movable_joints(ctx);
        let named = |j: &crate::Joint| format!("{} ({})", j.name, j.feature.kind.label());
        let pick = |key: &str, label: &str, current: FeatureId| {
            let mut options: Vec<String> = movable.iter().map(named).collect();
            let selected = movable
                .iter()
                .position(|j| j.id == current)
                .unwrap_or_else(|| {
                    options.push("a removed joint".into());
                    options.len() - 1
                });
            Widget::Choice {
                id: key.into(),
                label: label.into(),
                options,
                selected,
            }
        };
        let mut widgets = vec![
            w::header("involute-gear", &node.name),
            pick("driver", "Driving", coupling.driver),
            pick("driven", "Driven", coupling.driven),
        ];
        let gearings = coupling_gearings(&movable, &coupling);
        widgets.push(Widget::Choice {
            id: "gearing".into(),
            label: "Kind".into(),
            options: gearings.iter().map(|g| g.label().to_string()).collect(),
            selected: gearings
                .iter()
                .position(|g| *g == coupling.gearing)
                .unwrap_or(0),
        });
        let (ratio_label, length) = coupling.gearing.ratio_label();
        widgets.push(w::hinted(
            coupling.gearing.summary(),
            w::bound(
                "ratio",
                ratio_label,
                coupling.ratio,
                if length { Dim::Length } else { Dim::Number },
                (id, "/ratio"),
            ),
        ));
        widgets.push(w::hinted(
            "The driven joint moves the other way",
            w::toggle("reverse", "Reverse", coupling.reverse),
        ));
        widgets.push(w::text(
            "Move the driving joint (drag its body, or drive it) and the driven \
             one follows.",
        ));
        widgets.extend(self.verdict_note());
        widgets.extend(node.body.and_then(|body| freedom_text(ctx, body)));
        widgets.push(w::hinted(
            "Remove the coupling; the joints move apart again",
            w::button("delete", "Delete coupling", ButtonStyle::Destructive),
        ));
        widgets
    }

    /// A change to a coupling, made as `asm.set` makes it, or the coupling
    /// deleted.
    fn coupling_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        (id, created): (FeatureId, bool),
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        let coupling = Coupling::from_json(ctx.document.get_feature_data(id)?).ok()?;
        let movable = movable_joints(ctx);
        let mut args = serde_json::Map::new();
        match event {
            PanelEvent::Choice { id: key, index } if key == "driver" || key == "driven" => {
                let current = if key == "driver" {
                    coupling.driver
                } else {
                    coupling.driven
                };
                let joint = movable.get(*index).filter(|j| j.id != current)?;
                args.insert(key.clone(), joint.id.0.to_string().into());
            }
            PanelEvent::Choice { id: key, index } if key == "gearing" => {
                let gearing = *coupling_gearings(&movable, &coupling).get(*index)?;
                if gearing == coupling.gearing {
                    return None;
                }
                args.insert("gearing".into(), gearing.word().into());
            }
            PanelEvent::Number { id: key, value } if key == "ratio" => {
                if *value <= 0.0 {
                    return None;
                }
                args.insert("ratio".into(), serde_json::json!(*value as f32));
            }
            PanelEvent::Toggle { id: key, on } if key == "reverse" => {
                args.insert("reverse".into(), (*on).into());
            }
            PanelEvent::Button { id: key } if key == "delete" => {
                ctx.document.remove_feature(id).ok()?;
                if !created {
                    ctx.record(
                        "doc.delete",
                        crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
                        serde_json::Value::Null,
                    );
                }
                self.task = None;
                ctx.active_document_object = None;
                return Some(TaskOutcome::Accepted {
                    label: "Delete coupling".to_string(),
                });
            }
            _ => return None,
        }
        args.insert("joint".into(), id.0.to_string().into());
        match crate::commands::run("asm.set", &args, ctx) {
            Ok(_) => self.verdict = None,
            Err(err) => self.verdict = Some(Err(err.to_string())),
        }
        None
    }

    fn move_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        body: BodyId,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        if request.cancel {
            self.handles.reset();
            restore_placements(ctx, placements);
            self.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.handles.reset();
            let placement = ctx.document.body_placement(body);
            ctx.record(
                "asm.place",
                crate::commands::object(serde_json::json!({
                    "body": body.0.to_string(),
                    "translation": placement.translation,
                    "rotation": placement.rotation,
                })),
                serde_json::Value::Null,
            );
            self.task = None;
            return TaskOutcome::Accepted {
                label: "Move body".to_string(),
            };
        }
        self.declared_panel(ui, ctx)
    }

    /// A body's place by numbers: its position and its turn about each
    /// axis, shown and not edited while its joints place it.
    fn move_widgets(&self, ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<Widget> {
        let mut widgets = vec![w::header("move-geometry", &body_name(ctx, body))];
        let held = Self::held_by_joints(ctx, body);
        if held {
            widgets.push(w::note(
                NoteKind::Info,
                Some("Placed by its joints"),
                "Its joints decide where it sits. Move the body it is joined to, \
                 or delete a joint to free it.",
            ));
        }
        if !held {
            widgets.push(Widget::Choice {
                id: "handles".into(),
                label: "Handles".into(),
                options: crate::handles::MODES.map(String::from).to_vec(),
                selected: self.handles.mode_index(),
            });
        }
        let placement = ctx.document.body_placement(body);
        let angles = move_angles(&placement);
        for (k, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let label = format!("Position {axis}");
            let value = placement.translation[k];
            widgets.push(if held {
                w::value(&label, format!("{value:.2} mm"))
            } else {
                w::number(MOVE_FIELDS[k], &label, value, Dim::Length)
            });
        }
        for (k, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let label = format!("Turn about {axis}");
            widgets.push(if held {
                w::value(&label, format!("{:.2} °", angles[k]))
            } else {
                w::ranged(
                    MOVE_FIELDS[3 + k],
                    &label,
                    angles[k],
                    Dim::Angle,
                    Some((-180.0, 180.0)),
                )
            });
        }
        if !held {
            widgets.push(w::hinted(
                "No move, no turn",
                w::button("home", "Back to where it was made", ButtonStyle::Secondary),
            ));
        }
        widgets.extend(self.verdict_note());
        widgets
    }

    /// A field of the move panel changed: the body goes there as
    /// `asm.place` puts it, and whatever is joined to it follows. The
    /// handles' choice only changes what the view shows.
    fn move_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: BodyId,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        if let PanelEvent::Choice { id, index } = event
            && id == "handles"
        {
            self.handles.set_mode(ctx, body, *index);
            return None;
        }
        let moved = match (event, number_event(event)) {
            (PanelEvent::Button { id }, _) if id == "home" => BodyPlacement::IDENTITY,
            (_, Some((field, value))) => {
                let k = MOVE_FIELDS.iter().position(|f| *f == field)?;
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
            _ => return None,
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

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;

/// A mass in grams, or kilograms from a thousand.
/// The parts list's own columns, before the ones added to it.
const PARTS_COLUMNS: [&str; 9] = [
    "No.",
    "Part",
    "Qty",
    "Print",
    "Size (mm)",
    "Volume",
    "Mass",
    "Bought",
    "Threads",
];

/// The parts list's column of how many of a part to print.
const PRINT_COLUMN: usize = 3;

/// What the print takes: how many pieces, their volume and the
/// filament's mass, as far as the parts are measured.
fn print_summary(parts: &[crate::Part], material: &str, unit: core_document::Unit) -> String {
    let printed: Vec<&crate::Part> = parts.iter().filter(|p| p.print > 0).collect();
    let pieces: u32 = printed.iter().map(|p| p.print).sum();
    if pieces == 0 {
        return "Nothing to print".into();
    }
    let s = if pieces == 1 { "" } else { "s" };
    if printed.iter().any(|p| p.volume_mm3.is_none()) {
        return format!("To print: {pieces} piece{s}, measuring");
    }
    let volume: f64 = printed
        .iter()
        .map(|p| p.volume_mm3.unwrap_or(0.0) * f64::from(p.print))
        .sum();
    let mass: f64 = printed
        .iter()
        .map(|p| p.mass_g().unwrap_or(0.0) * f64::from(p.print))
        .sum();
    format!(
        "To print: {pieces} piece{s}, {}, {} of {material}",
        core_document::format_volume_mm3(volume, unit, 2),
        mass_text(mass)
    )
}

fn mass_text(grams: f64) -> String {
    if grams >= 1000.0 {
        format!("{:.3} kg", grams / 1000.0)
    } else {
        format!("{grams:.2} g")
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
