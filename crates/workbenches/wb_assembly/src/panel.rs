//! The Assembly task panels: the prompt while a joint's faces are picked,
//! a joint's or a coupling's settings, a body moved by numbers, and the
//! assembly's own tasks (interference, exploded views, motion over time,
//! linked copies, replacing a body, rigid groups, mass, the parts list).
//!
//! Each is one walk over a `panel::Form`: declared, it lists the widgets
//! (`task_widgets`) the same renderer a package's panel takes draws;
//! given what the user did (`task_event`), the field the event names says
//! what edit it makes, and the edit changes the document through the
//! bench's own commands where one makes the change.

use core_document::{
    BodyId, BodyPlacement, FeatureId, TaskOutcome, TaskRequest, WorkbenchFeature,
    WorkbenchRuntimeContext,
};

use bench_api::panel::{self, Form};
use bench_api::{Bind, ButtonStyle, Dim, NoteKind, PanelEvent, Widget};

use crate::{
    AssemblyWorkbench, Coupling, Gearing, JointFeature, JointKind, JointTool, Task, body_name,
};

/// The declared panels' widgets, as the Assembly writes them.
mod w {
    use super::*;

    pub(super) fn value(label: &str, value: impl Into<String>) -> Widget {
        Widget::Value {
            label: label.into(),
            value: value.into(),
            mono: false,
        }
    }

    pub(super) fn note(kind: NoteKind, title: Option<&str>, text: impl Into<String>) -> Widget {
        panel::note(kind, title, &text.into())
    }

    pub(super) fn hinted(hint: impl Into<String>, widget: Widget) -> Widget {
        panel::hinted(&hint.into(), widget)
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

    pub(super) fn section(title: &str, children: Vec<Widget>) -> Widget {
        Widget::Section(bench_api::PanelSection {
            title: title.into(),
            open: true,
            children,
            ..Default::default()
        })
    }
}

/// The declared panels' fields, each named once in a [`Form`]: the widget,
/// and what the event the form applies did to it.
mod f {
    use super::*;

    pub(super) fn button(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        style: ButtonStyle,
    ) -> (Widget, bool) {
        let bound = form.button(id, label, style, true);
        (bound.widget, bound.set)
    }

    /// A number the bench keeps, not bound to a parameter; the value typed.
    pub(super) fn number(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        value: f32,
        dim: Dim,
    ) -> (Widget, Option<f64>) {
        ranged(form, id, label, value, dim, None)
    }

    /// A number within `min..=max`, when given.
    pub(super) fn ranged(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        value: f32,
        dim: Dim,
        range: Option<(f64, f64)>,
    ) -> (Widget, Option<f64>) {
        let shape = bench_api::Quantity {
            dim,
            min: range.map(|r| r.0),
            max: range.map(|r| r.1),
            ..panel::quantity("", label, 0.0)
        };
        quantity(form, id, f64::from(value), shape)
    }

    /// A feature's number a formula can set: `key` is its parameter.
    pub(super) fn bound(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        value: f32,
        dim: Dim,
        bind: (FeatureId, &str),
    ) -> (Widget, Option<f64>) {
        bound_within(form, id, label, value, dim, bind, (None, None))
    }

    /// A [`bound`] number whose drag stops at `drag_min` and `drag_max`;
    /// typing may go past them.
    pub(super) fn bound_within(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        value: f32,
        dim: Dim,
        (feature, key): (FeatureId, &str),
        (drag_min, drag_max): (Option<f32>, Option<f32>),
    ) -> (Widget, Option<f64>) {
        let shape = bench_api::Quantity {
            dim,
            bind: Some(Bind {
                feature: feature.0.to_string(),
                key: key.into(),
            }),
            drag_min: drag_min.map(f64::from),
            drag_max: drag_max.map(f64::from),
            ..panel::quantity("", label, 0.0)
        };
        quantity(form, id, f64::from(value), shape)
    }

    /// A number as `shape` draws it, showing `value`; the value typed.
    pub(super) fn quantity(
        form: &mut Form<'_>,
        id: &str,
        mut value: f64,
        shape: bench_api::Quantity,
    ) -> (Widget, Option<f64>) {
        let bound = form.number(id, &mut value, shape);
        (bound.widget.into(), bound.set.then_some(value))
    }

    pub(super) fn toggle(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        mut on: bool,
    ) -> (Widget, Option<bool>) {
        let bound = form.check(id, &mut on, panel::check("", "", label, false));
        (bound.widget.into(), bound.set.then_some(on))
    }

    /// A dropdown as `shape` draws it; the option picked.
    pub(super) fn select(
        form: &mut Form<'_>,
        id: &str,
        shape: bench_api::Select,
    ) -> (Widget, Option<usize>) {
        let mut selected = shape.selected;
        let bound = form.index(id, &mut selected, shape);
        (bound.widget.into(), bound.set.then_some(selected))
    }

    /// A dropdown of `options`.
    pub(super) fn choice(
        form: &mut Form<'_>,
        id: &str,
        label: &str,
        options: &[&str],
        selected: usize,
    ) -> (Widget, Option<usize>) {
        select(
            form,
            id,
            panel::select("", label, options.iter().copied(), selected),
        )
    }

    /// The id of a widget no binder draws (a slider, a list, a table, a
    /// text field), and the event when it names it.
    pub(super) fn raw<'e>(form: &mut Form<'e>, id: &str) -> (String, Option<&'e PanelEvent>) {
        let id = form.id(id);
        let event = form.take(&id);
        (id, event)
    }

    /// The row a click on the list or table `id` picked.
    pub(super) fn picked(event: Option<&PanelEvent>) -> Option<usize> {
        match event {
            Some(PanelEvent::Select { index, .. }) => Some(*index),
            _ => None,
        }
    }
}

/// The widgets `walk` declares.
fn declared<R>(walk: impl FnOnce(&mut Form<'_>) -> R) -> Vec<Widget> {
    let mut form = Form::declare();
    walk(&mut form);
    form.finish()
}

/// The edit `walk` makes of `event`: what the field it names stands for.
fn applied<'e, E>(
    event: &'e PanelEvent,
    walk: impl FnOnce(&mut Form<'e>) -> Option<E>,
) -> Option<E> {
    walk(&mut Form::apply(event))
}

/// What to click while a joint's faces are picked, and the origin's planes
/// or axes the second pick may be instead; the one clicked, by its place
/// in `ORIGIN`.
fn picking_walk(picking: &crate::Picking, form: &mut Form<'_>) -> Option<usize> {
    form.push(w::text(picking.prompt()));
    form.push(w::text(
        "The first body moves; the second stays where it is. A body \
         with no joints of its own never moves. A datum plane or line \
         selected in the tree is taken as a face.",
    ));
    let (_, first, ..) = picking.first?;
    let mut clicked = None;
    let mut offered = Vec::new();
    for (i, (name, anchor)) in crate::ORIGIN.iter().enumerate() {
        if !picking.kind.takes_anchor(anchor, Some(first)) {
            continue;
        }
        let (button, pressed) = f::button(form, &format!("origin:{i}"), name, ButtonStyle::Small);
        offered.push(button);
        if pressed {
            clicked = Some(i);
        }
    }
    if !offered.is_empty() {
        form.push(w::section("Or the origin's", vec![w::row(offered)]));
    }
    clicked
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

/// What a field of the interference panel asks.
enum InterferenceEdit {
    /// A pair clicked: its first body selected.
    Select(BodyId),
    Clearance(f32),
    Stop,
    CheckClashes,
    CheckClearance,
    EveryPair,
}

/// What a field of the exploded view's panel asks.
enum ExplodeEdit {
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

/// What a field of the motion panel asks.
enum MotionEdit {
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

/// What a field of the copies panel asks.
enum CopiesEdit {
    Mirror(bool),
    Around(bool),
    Plane(usize),
    Axis(usize),
    Count(f32),
    Over(f32),
    MirrorAt(usize, f32),
    Through(usize, f32),
    Step(usize, f32),
}

/// What a field of the rigid group's panel asks.
enum GroupEdit {
    Remove(usize),
    Dissolve,
}

/// What a field of the mass panel asks.
enum MassEdit {
    Density(f32),
    /// A body's row clicked: the body selected.
    Select(BodyId),
    Again,
}

/// What a field of the parts list asks.
enum PartsEdit {
    ByComponent(bool),
    /// A material picked, by its place among the presets; past them,
    /// Custom.
    Material(usize),
    Density(f64),
    /// Text typed into a cell.
    Cell {
        row: usize,
        column: usize,
        value: String,
    },
    Bought {
        row: usize,
        on: bool,
    },
    /// A row clicked: its part's first body selected.
    Select(usize),
    NewColumn(String),
    Number,
    AddColumn,
    RemoveColumn(usize),
    Copy,
    Save,
}

/// What a field of a joint's panel asks.
enum JointEdit {
    /// Another kind picked, by its place in `JointTool::ALL`.
    Kind(usize),
    Repick,
    /// A number `asm.set` takes by this word.
    Number(&'static str, f32),
    Flip(bool),
    /// A fixed joint's shift along one of the other body's axes.
    Shift(usize, f32),
    /// A field of the drive whose fields' ids begin with this word.
    Drive(&'static str, DriveEdit),
    CheckSweep,
    StopSweep,
    SelectBody(BodyId),
    TurnBy(f32),
    Turn,
    TurnOver,
    Turn90,
    SelectJoint(FeatureId),
    Delete,
}

/// What a field of a drive asks.
enum DriveEdit {
    /// Held at a value, or let go.
    Drive(bool),
    To(f32),
    /// Kept within limits, or not.
    Limits(bool),
    /// One end of the limits: 0 the lowest, 1 the highest.
    End(usize, f32),
    Record,
    Play,
}

/// What a field of a coupling's panel asks.
enum CouplingEdit {
    /// The driving joint picked, by its place among the hinges and sliders.
    Driver(usize),
    Driven(usize),
    /// What ties them, by its place among the gearings offered.
    Gearing(usize),
    Ratio(f64),
    Reverse(bool),
    Delete,
}

/// What a field of the move panel asks.
enum MoveEdit {
    Handles(usize),
    /// A position (0 to 2) or a turn (3 to 5) typed.
    Field(usize, f32),
    Home,
}

/// Frames in a recorded sweep: there and back in four seconds.
const SWEEP_FRAMES: usize = 60;

impl AssemblyWorkbench {
    /// The open task panel's frame, as the host draws it: the task's turn,
    /// then its declared widgets and what the user did to them.
    #[cfg(feature = "egui")]
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        match self.task_turn(ctx, request) {
            TaskOutcome::Open => self.declared_panel(ui, ctx),
            done => done,
        }
    }

    /// The open task's turn before its panel is drawn: OK or Cancel
    /// handled, work finished away from the window collected. `Open`
    /// while the task stays open.
    pub(crate) fn task_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        let outcome = self.turn_of_task(ctx, request);
        self.note_left(ctx.document);
        outcome
    }

    fn turn_of_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        let open = self.tab.task.is_some();
        self.settle_held(ctx);
        if open && self.tab.task.is_none() {
            return TaskOutcome::Cancelled;
        }
        if self.tab.picking.is_some() {
            if request.accept || request.cancel {
                self.tab.picking = None;
                return TaskOutcome::Cancelled;
            }
            return TaskOutcome::Open;
        }
        match self.tab.task.clone() {
            Some(Task::Joint {
                id,
                before,
                placements,
            }) => self.joint_turn(ctx, request, id, before, &placements),
            Some(Task::Coupling { id, before, .. }) => self.coupling_turn(ctx, request, id, before),
            Some(Task::Move {
                body,
                placements,
                turns,
            }) => self.move_turn(ctx, request, body, &placements, &turns),
            Some(Task::Interference { .. }) => self.interference_turn(ctx, request),
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
                TaskOutcome::Open
            }
            Some(Task::Parts) => self.parts_turn(ctx, request),
            Some(Task::Motion(_)) => self.motion_turn(ctx, request),
            Some(Task::Copies {
                body,
                count,
                step,
                around,
                mirror,
            }) => self.copies_turn(ctx, request, body, (count, step, around), mirror),
            Some(Task::Replace { old, new }) => self.replace_turn(ctx, request, old, new),
            Some(Task::Group { editing, members }) => {
                self.group_turn(ctx, request, editing, &members)
            }
            Some(Task::Mass { .. }) => self.mass_turn(ctx, request),
            None => TaskOutcome::Open,
        }
    }

    /// Draw a declared panel and hand back what the user did. Formulas set
    /// on bound numbers go into the document here; the values they give
    /// are not written into the joints, which keep plain numbers.
    #[cfg(feature = "egui")]
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
        let by_formula = out.write_formulas(ctx);
        out.events_besides(&by_formula).cloned().collect()
    }

    /// The open task's panel drawn as declared widgets, and what the user
    /// did to it handled: a package's panel takes the same way.
    #[cfg(feature = "egui")]
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
        if let Some(picking) = &self.tab.picking {
            return declared(|form| picking_walk(picking, form));
        }
        match &self.tab.task {
            Some(Task::Move { body, .. }) => declared(|form| self.move_walk(ctx, *body, form)),
            Some(Task::Copies {
                body,
                count,
                step,
                around,
                mirror,
            }) => declared(|form| {
                Self::copies_walk(ctx, *body, (*count, *step, *around), *mirror, form)
            }),
            Some(Task::Replace { old, new }) => Self::replace_widgets(ctx, *old, *new),
            Some(Task::Group { editing, members }) => {
                declared(|form| Self::group_walk(ctx, *editing, members, form))
            }
            Some(Task::Interference {
                found,
                seq,
                around,
                clearance,
            }) => declared(|form| {
                self.interference_walk(ctx, found.as_ref(), (*seq, *around, *clearance), form)
            }),
            Some(Task::Mass { found, density }) => {
                declared(|form| self.mass_walk(ctx, found.as_ref(), *density, form))
            }
            Some(Task::Explode { spread, steps, .. }) => {
                declared(|form| Self::explode_walk(ctx, *spread, steps, form))
            }
            Some(Task::Parts) => declared(|form| self.parts_walk(ctx, form)),
            Some(Task::Coupling { id, .. }) => declared(|form| self.coupling_walk(ctx, *id, form)),
            Some(Task::Joint { id, .. }) => declared(|form| self.joint_walk(ctx, *id, form)),
            Some(Task::Motion(studying)) => declared(|form| Self::motion_walk(ctx, studying, form)),
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
        let outcome = self.event_of_task(ctx, event);
        self.note_left(ctx.document);
        outcome
    }

    fn event_of_task(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> Option<TaskOutcome> {
        if let Some(picking) = &self.tab.picking {
            // One of the origin's planes or axes, as the second pick.
            let i = applied(event, |form| picking_walk(picking, form))?;
            let (_, anchor) = crate::ORIGIN[i];
            self.picked(ctx, crate::WORLD, anchor, None, 0);
            return None;
        }
        match self.tab.task.clone() {
            Some(Task::Move { body, .. }) => {
                let edit = applied(event, |form| self.move_walk(ctx, body, form))?;
                self.move_edit(ctx, body, edit)
            }
            Some(Task::Copies {
                body,
                count,
                step,
                around,
                mirror,
            }) => {
                let edit = applied(event, |form| {
                    Self::copies_walk(ctx, body, (count, step, around), mirror, form)
                })?;
                self.copies_edit(edit)
            }
            Some(Task::Group {
                editing, members, ..
            }) => {
                let edit = applied(event, |form| Self::group_walk(ctx, editing, &members, form))?;
                self.group_edit(ctx, editing, edit)
            }
            Some(Task::Interference {
                found,
                seq,
                around,
                clearance,
            }) => {
                let edit = applied(event, |form| {
                    self.interference_walk(ctx, found.as_ref(), (seq, around, clearance), form)
                })?;
                self.interference_edit(ctx, (around, clearance), edit)
            }
            Some(Task::Mass { found, density }) => {
                let edit = applied(event, |form| {
                    self.mass_walk(ctx, found.as_ref(), density, form)
                })?;
                self.mass_edit(ctx, edit)
            }
            Some(Task::Explode { spread, steps, .. }) => {
                let edit = applied(event, |form| Self::explode_walk(ctx, spread, &steps, form))?;
                self.explode_edit(ctx, edit)
            }
            Some(Task::Parts) => {
                let edit = applied(event, |form| self.parts_walk(ctx, form))?;
                self.parts_edit(ctx, edit)
            }
            Some(Task::Coupling { id, before, .. }) => {
                let edit = applied(event, |form| self.coupling_walk(ctx, id, form))?;
                self.coupling_edit(ctx, (id, before.is_none()), edit)
            }
            Some(Task::Joint {
                id,
                before,
                placements,
            }) => {
                let edit = applied(event, |form| self.joint_walk(ctx, id, form))?;
                self.joint_edit(ctx, (id, before.is_none(), &placements), edit)
            }
            Some(Task::Motion(studying)) => {
                let edit = applied(event, |form| Self::motion_walk(ctx, &studying, form))?;
                self.motion_edit(ctx, edit)
            }
            _ => None,
        }
    }

    /// The assembly's joints left apart, then those it could do without,
    /// one row each by name; a click selects the joint, the one clicked
    /// handed back. Nothing while every joint holds and none is redundant.
    fn solver_messages(
        &self,
        ctx: &WorkbenchRuntimeContext,
        form: &mut Form<'_>,
    ) -> Option<FeatureId> {
        let messages = self.messages(ctx);
        if messages.is_empty() {
            return None;
        }
        let (id, event) = f::raw(form, "solver_messages");
        let clicked = f::picked(event).and_then(|i| messages.get(i).map(|(j, ..)| *j));
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
        form.push(w::section(
            "Solver messages",
            vec![
                w::text("Click one to select its joint."),
                Widget::List {
                    id,
                    items,
                    selected: None,
                },
            ],
        ));
        clicked
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
        match &self.tab.verdict {
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
                self.tab.verdict = Some(Ok(crate::moved_words(moved)));
                Ok(value)
            }
            Err(why) => {
                let message = why.to_string();
                self.tab.verdict = Some(Err(message.clone()));
                ctx.log_warn(message.clone());
                Err(message)
            }
        }
    }

    /// What an interference check found: each clash, a click selecting
    /// the first of its bodies.
    fn interference_turn(
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
    fn interference_walk(
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
    fn interference_edit(
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

    /// The exploded view's spread, or, once bodies are picked for a step,
    /// its steps; closing puts every body back.
    fn explode_walk(
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
    fn explode_edit(
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

    /// A motion over time: when it starts and ends, its step, and each
    /// joint driven by a formula of `t`; worked out into frames to scrub,
    /// play and record. The bodies go back when it closes.
    fn motion_turn(
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

    fn motion_walk(
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
    fn motion_edit(
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

    /// Linked copies of a body: how many and how far apart; OK inserts them
    /// in a row, to be dragged where they go.
    fn copies_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        body: BodyId,
        (count, step, around): (u32, [f32; 3], Option<crate::Around>),
        mirror: Option<([f32; 3], [f32; 3])>,
    ) -> TaskOutcome {
        if request.cancel {
            self.tab.task = None;
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
                    self.tab.task = None;
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
            self.tab.task = None;
            return TaskOutcome::Accepted {
                label: "Insert linked copies".to_string(),
            };
        }
        TaskOutcome::Open
    }

    /// How many copies and how far apart, or turned about an axis, or one
    /// mirrored across a plane.
    fn copies_walk(
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        (count, step, around): (u32, [f32; 3], Option<crate::Around>),
        mirror: Option<([f32; 3], [f32; 3])>,
        form: &mut Form<'_>,
    ) -> Option<CopiesEdit> {
        let mut edit = None;
        form.push(w::value("Of", body_name(ctx, body)));
        let (how_many, typed) = f::quantity(
            form,
            "count",
            f64::from(count),
            bench_api::Quantity {
                label: "How many".into(),
                dim: Dim::Number,
                bind: None,
                min: Some(1.0),
                max: Some(500.0),
                decimals: 0,
                error: None,
                ..Default::default()
            },
        );
        form.push(how_many);
        if let Some(value) = typed {
            edit = Some(CopiesEdit::Count(value as f32));
        }
        let (mirrored, on) = f::toggle(form, "mirror", "A mirror image", mirror.is_some());
        form.push(w::hinted(
            "One copy mirrored across a plane, rather than copies as they are",
            mirrored,
        ));
        if let Some(on) = on {
            edit = Some(CopiesEdit::Mirror(on));
        }
        /// A field for each axis, named `prefix` and its place; the one
        /// typed into.
        fn axes(
            form: &mut Form<'_>,
            prefix: &str,
            labels: [&str; 3],
            at: [f32; 3],
        ) -> Option<(usize, f32)> {
            let mut typed = None;
            for (k, label) in labels.into_iter().enumerate() {
                let (field, value) =
                    f::number(form, &format!("{prefix}{k}"), label, at[k], Dim::Length);
                form.push(field);
                if let Some(value) = value {
                    typed = Some((k, value as f32));
                }
            }
            typed
        }
        let through = ["Through x", "Through y", "Through z"];
        if let Some((point, normal)) = mirror {
            let selected = AXES.iter().position(|a| *a == normal).unwrap_or(0);
            let (plane, picked) = f::choice(form, "plane", "Across", &["YZ", "XZ", "XY"], selected);
            form.push(plane);
            if let Some(index) = picked {
                edit = Some(CopiesEdit::Plane(index));
            }
            if let Some((k, value)) = axes(form, "mirror_", through, point) {
                edit = Some(CopiesEdit::MirrorAt(k, value));
            }
            form.push(w::text(
                "The mirror image takes the body's shape, mirrored, and follows every change \
                 to it.",
            ));
            return edit;
        }
        let (turned, on) = f::toggle(form, "around", "Around an axis", around.is_some());
        form.push(w::hinted(
            "Turn the copies about an axis instead of setting them in a row",
            turned,
        ));
        if let Some(on) = on {
            edit = Some(CopiesEdit::Around(on));
        }
        match around {
            Some((point, axis, angle)) => {
                let selected = AXES.iter().position(|a| *a == axis).unwrap_or(2);
                let (axis, picked) = f::choice(form, "axis", "Axis", &["X", "Y", "Z"], selected);
                form.push(axis);
                if let Some(index) = picked {
                    edit = Some(CopiesEdit::Axis(index));
                }
                if let Some((k, value)) = axes(form, "through_", through, point) {
                    edit = Some(CopiesEdit::Through(k, value));
                }
                let (over, typed) = f::number(form, "over", "Over", angle, Dim::Angle);
                form.push(over);
                if let Some(value) = typed {
                    edit = Some(CopiesEdit::Over(value as f32));
                }
            }
            None => {
                if let Some((k, value)) = axes(form, "step_", ["Step x", "Step y", "Step z"], step)
                {
                    edit = Some(CopiesEdit::Step(k, value));
                }
            }
        }
        form.push(w::text(
            "Each copy takes the body's shape and follows every change to it. They go in a \
             row, each a step from the one before; drag one to put it where it goes.",
        ));
        edit
    }

    /// A change to the copies' settings, kept in the task until OK.
    fn copies_edit(&mut self, edit: CopiesEdit) -> Option<TaskOutcome> {
        let Some(Task::Copies {
            count,
            step,
            around,
            mirror,
            ..
        }) = &mut self.tab.task
        else {
            return None;
        };
        let mut pivot = around.unwrap_or(([0.0; 3], [0.0, 0.0, 1.0], 360.0));
        let mut plane = mirror.unwrap_or(([0.0; 3], [1.0, 0.0, 0.0]));
        match edit {
            CopiesEdit::Mirror(on) => *mirror = on.then_some(plane),
            CopiesEdit::Around(on) => *around = on.then_some(pivot),
            CopiesEdit::Plane(index) => {
                plane.1 = AXES[index.min(2)];
                *mirror = Some(plane);
            }
            CopiesEdit::Axis(index) => {
                pivot.1 = AXES[index.min(2)];
                *around = Some(pivot);
            }
            CopiesEdit::Count(value) => *count = value.round().max(1.0) as u32,
            CopiesEdit::Over(value) => {
                pivot.2 = value;
                *around = Some(pivot);
            }
            CopiesEdit::MirrorAt(k, value) => {
                plane.0[k] = value;
                *mirror = Some(plane);
            }
            CopiesEdit::Through(k, value) => {
                pivot.0[k] = value;
                *around = Some(pivot);
            }
            CopiesEdit::Step(k, value) => step[k] = value,
        }
        None
    }

    /// A body and the one picked to take its place and joints.
    fn replace_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        old: BodyId,
        new: Option<BodyId>,
    ) -> TaskOutcome {
        if request.cancel {
            self.tab.task = None;
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
                    self.tab.task = None;
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
        TaskOutcome::Open
    }

    /// The body replaced and the one taking its place.
    fn replace_widgets(
        ctx: &WorkbenchRuntimeContext,
        old: BodyId,
        new: Option<BodyId>,
    ) -> Vec<Widget> {
        vec![
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
    fn group_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        editing: Option<FeatureId>,
        members: &[BodyId],
    ) -> TaskOutcome {
        if request.cancel {
            self.tab.task = None;
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
            self.tab.task = None;
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
        TaskOutcome::Open
    }

    /// The bodies picked for a group, each with its Remove; the group's
    /// Dissolve when it is one already.
    fn group_walk(
        ctx: &WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        members: &[BodyId],
        form: &mut Form<'_>,
    ) -> Option<GroupEdit> {
        let mut edit = None;
        form.push(w::text(
            "Click the bodies to lock together; a second click takes one out. They \
                 move as one, held as they sit now, the first the one the rest hold to.",
        ));
        for (i, body) in members.iter().enumerate() {
            let (remove, pressed) =
                f::button(form, &format!("remove:{i}"), "Remove", ButtonStyle::Small);
            if pressed {
                edit = Some(GroupEdit::Remove(i));
            }
            form.push(w::row(vec![w::text(body_name(ctx, *body)), remove]));
        }
        if members.len() < 2 {
            form.push(w::note(
                NoteKind::Info,
                None,
                "A group takes two bodies or more",
            ));
        }
        if editing.is_some() {
            let (dissolve, pressed) =
                f::button(form, "dissolve", "Dissolve group", ButtonStyle::Destructive);
            form.push(w::hinted(
                "Remove the group; the bodies stay where they are",
                dissolve,
            ));
            if pressed {
                edit = Some(GroupEdit::Dissolve);
            }
        }
        edit
    }

    /// A body taken out of the group, or the group dissolved.
    fn group_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        edit: GroupEdit,
    ) -> Option<TaskOutcome> {
        let id = match edit {
            GroupEdit::Remove(i) => {
                if let Some(Task::Group { members, .. }) = &mut self.tab.task
                    && i < members.len()
                {
                    members.remove(i);
                }
                return None;
            }
            GroupEdit::Dissolve => editing?,
        };
        ctx.document.remove_feature(id).ok()?;
        ctx.record(
            "doc.delete",
            crate::commands::object(serde_json::json!({"id": id.0.to_string()})),
            serde_json::Value::Null,
        );
        self.tab.task = None;
        ctx.active_document_object = None;
        Some(TaskOutcome::Accepted {
            label: "Dissolve rigid group".to_string(),
        })
    }

    /// The mass of the visible bodies at one density, their centre of mass,
    /// and each body's share.
    fn mass_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.tab.measuring = None;
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_mass(ctx);
        TaskOutcome::Open
    }

    /// The measuring's progress; then the density, the totals and each
    /// body's mass, a click on one selecting it.
    fn mass_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        found: Option<&crate::MassReport>,
        density: f32,
        form: &mut Form<'_>,
    ) -> Option<MassEdit> {
        let mut edit = None;
        let Some(report) = found else {
            let (done, total) = self.mass_progress().unwrap_or((0, 0));
            form.push(Widget::Progress {
                label: format!("Measuring {done} of {total} bodies"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            return None;
        };
        let (field, typed) = f::ranged(
            form,
            "density",
            "Density, g/cm³",
            density,
            Dim::Number,
            Some((0.0, 100.0)),
        );
        form.push(field);
        if let Some(value) = typed {
            edit = Some(MassEdit::Density(value as f32));
        }
        let density = f64::from(density);
        let unit = ctx.document.display_unit();
        let line = |label: &str, value: String| Widget::Value {
            label: label.into(),
            value,
            mono: true,
        };
        form.push(line("Mass", mass_text(report.mass_g(density))));
        form.push(line(
            "Volume",
            core_document::format_volume_mm3(report.volume_mm3(), unit, 2),
        ));
        if let Some(c) = report.centre(density) {
            let f = |v: f64| core_document::format_length_mm(v as f32, unit, 2);
            form.push(line(
                "Centre of mass",
                format!("{}, {}, {}", f(c[0]), f(c[1]), f(c[2])),
            ));
        }
        // The principal moments about the centre of mass, in g times the
        // display unit squared.
        if let Some(inertia) = report.inertia(density) {
            let per = f64::from(unit.mm_per_unit());
            for (i, (moment, axis)) in inertia.principal.iter().enumerate() {
                form.push(line(
                    &format!("I{}", i + 1),
                    format!(
                        "{} g·{}² about {}",
                        figure(moment / (per * per)),
                        unit.short_label(),
                        axis.map(|c| format!("{:.2}", if c.abs() < 0.005 { 0.0 } else { c }))
                            .join(" ")
                    ),
                ));
            }
        }
        let (id, event) = f::raw(form, "bodies");
        if let Some(b) = f::picked(event).and_then(|i| report.bodies.get(i)) {
            edit = Some(MassEdit::Select(b.body));
        }
        form.push(Widget::Table {
            id,
            columns: vec!["Body".into(), "Mass".into()],
            rows: report
                .bodies
                .iter()
                .map(|b| {
                    let name = body_name(ctx, b.body);
                    vec![
                        if b.bought {
                            format!("{name} (bought)")
                        } else {
                            name
                        },
                        mass_text(b.mass_g(density)),
                    ]
                })
                .collect(),
            selected: None,
            editable: Vec::new(),
        });
        if report.skipped > 0 {
            form.push(w::text(format!(
                "{} visible bod{} without a closed solid left out",
                report.skipped,
                if report.skipped == 1 { "y" } else { "ies" }
            )));
        }
        let (again, pressed) = f::button(form, "again", "Measure again", ButtonStyle::Secondary);
        form.push(again);
        if pressed {
            edit = Some(MassEdit::Again);
        }
        edit
    }

    /// The density changed, a body clicked, or the bodies measured again.
    fn mass_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        edit: MassEdit,
    ) -> Option<TaskOutcome> {
        match edit {
            MassEdit::Density(value) => {
                if let Some(Task::Mass { density, .. }) = &mut self.tab.task {
                    *density = value;
                }
            }
            MassEdit::Select(body) => {
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            MassEdit::Again => {
                if let Some(Task::Mass { density, .. }) = self.tab.task {
                    self.measure_mass(ctx, density);
                }
            }
        }
        None
    }

    /// Every part, how many of it and its size, with a copy for a
    /// spreadsheet; its item number, whether it is bought and the added
    /// columns, kept in the document.
    fn parts_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        let parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
        self.volumes.refresh(ctx.document, &parts, ctx.kernel);
        TaskOutcome::Open
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

    fn parts_walk(&self, ctx: &WorkbenchRuntimeContext, form: &mut Form<'_>) -> Option<PartsEdit> {
        use bench_api::Cell;
        let mut edit = None;
        let parts = self.measured_parts(ctx);
        let unit = ctx.document.display_unit();
        let pending = if self.volumes.running() { "…" } else { "-" };
        let table = crate::parts::table_of(ctx.document)
            .map(|(_, t)| t)
            .unwrap_or_default();
        let total: usize = parts.iter().map(|p| p.bodies.len()).sum();
        form.push(w::text(format!(
            "{} part{}, {total} bod{}",
            parts.len(),
            if parts.len() == 1 { "" } else { "s" },
            if total == 1 { "y" } else { "ies" }
        )));
        let components = !ctx.document.components().is_empty();
        if components {
            let (by_component, on) =
                f::toggle(form, "by_component", "By component", table.by_component);
            form.push(w::hinted(
                "Each component's parts under it, nested as the components are",
                by_component,
            ));
            if let Some(on) = on {
                edit = Some(PartsEdit::ByComponent(on));
            }
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
        let (id, event) = f::raw(form, "parts");
        match event {
            Some(PanelEvent::CellText {
                row, column, value, ..
            }) => {
                edit = Some(PartsEdit::Cell {
                    row: *row,
                    column: *column,
                    value: value.clone(),
                })
            }
            Some(PanelEvent::CellCheck { row, on, .. }) => {
                edit = Some(PartsEdit::Bought { row: *row, on: *on })
            }
            Some(PanelEvent::Select { index, .. }) => edit = Some(PartsEdit::Select(*index)),
            _ => {}
        }
        form.push(Widget::Sheet { id, columns, rows });
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
        let (material, picked) = f::select(
            form,
            "print_material",
            bench_api::Select {
                label: "Printed in".into(),
                selected: preset.unwrap_or(options.len() - 1),
                options,
                ..Default::default()
            },
        );
        form.push(w::hinted(
            "The filament the parts print in; a body with a material of its own is \
             weighed at its own",
            material,
        ));
        if let Some(index) = picked {
            edit = Some(PartsEdit::Material(index));
        }
        if preset.is_none() {
            let (density, typed) = f::ranged(
                form,
                "print_density",
                "Density, g/cm³",
                table.material.density,
                Dim::Number,
                Some((0.01, 100.0)),
            );
            form.push(density);
            if let Some(value) = typed {
                edit = Some(PartsEdit::Density(value));
            }
        }
        form.push(w::text(print_summary(&parts, &table.material.name, unit)));
        let (number, pressed) =
            f::button(form, "number", "Number the parts", ButtonStyle::Secondary);
        form.push(w::hinted(
            "Give every part without an item number the next one, in list order",
            number,
        ));
        if pressed {
            edit = Some(PartsEdit::Number);
        }
        let (id, event) = f::raw(form, "new_column");
        if let Some(PanelEvent::Text { value, .. }) = event {
            edit = Some(PartsEdit::NewColumn(value.clone()));
        }
        let (add, pressed) = f::button(form, "add_column", "Add column", ButtonStyle::Small);
        if pressed {
            edit = Some(PartsEdit::AddColumn);
        }
        form.push(w::row(vec![
            Widget::TextField {
                id,
                label: "New column".into(),
                value: self.tab.parts_column.clone(),
            },
            add,
        ]));
        if !table.columns.is_empty() {
            let mut removes = Vec::new();
            for (i, column) in table.columns.iter().enumerate() {
                let (remove, pressed) = f::button(
                    form,
                    &format!("remove_column:{i}"),
                    &format!("Remove {column}"),
                    ButtonStyle::Small,
                );
                removes.push(remove);
                if pressed {
                    edit = Some(PartsEdit::RemoveColumn(i));
                }
            }
            form.push(w::row(removes));
        }
        let (copy, pressed) = f::button(form, "copy", "Copy as CSV", ButtonStyle::Secondary);
        if pressed {
            edit = Some(PartsEdit::Copy);
        }
        let (save, pressed) = f::button(form, "save", "Save as CSV", ButtonStyle::Secondary);
        if pressed {
            edit = Some(PartsEdit::Save);
        }
        form.push(w::row(vec![
            w::hinted("For a spreadsheet: every column of the list", copy),
            w::hinted("Write the list to a file a spreadsheet opens", save),
        ]));
        edit
    }

    /// A change to the parts list, kept as `asm.parts_table` keeps it; a
    /// part clicked, selected; the list copied or saved as CSV.
    fn parts_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        edit: PartsEdit,
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
        let edit_copy = matches!(edit, PartsEdit::Copy);
        match edit {
            PartsEdit::ByComponent(on) => table.by_component = on,
            PartsEdit::Material(index) => {
                table.material = match crate::parts::PRINT_MATERIALS.get(index) {
                    Some((name, _)) => crate::parts::PrintMaterial::named(name, None),
                    None => crate::parts::PrintMaterial {
                        name: "Custom".into(),
                        density: table.material.density,
                    },
                };
            }
            PartsEdit::Density(value) => {
                if value > 0.0 {
                    table.material.density = value as f32;
                }
            }
            PartsEdit::Cell { row, column, value } if column == PRINT_COLUMN => {
                let (part, _) = part_at(row)?;
                let value = value.trim();
                let count = if value.is_empty() {
                    None
                } else {
                    Some(value.parse::<u32>().ok()?)
                };
                table.entry_mut(&part.bodies).print = count;
            }
            PartsEdit::Select(row) => {
                let (_, bodies) = part_at(row)?;
                ctx.request(core_document::HostRequest::SelectBody(*bodies.first()?));
            }
            PartsEdit::Bought { row, on } => {
                let (part, _) = part_at(row)?;
                table.set_bought(part, on);
            }
            PartsEdit::Cell { row, column, value } => {
                let (part, _) = part_at(row)?;
                let column = table.columns.get(column.checked_sub(FIXED)?)?.clone();
                table.entry_mut(&part.bodies).values.insert(column, value);
            }
            PartsEdit::NewColumn(value) => self.tab.parts_column = value,
            PartsEdit::Number => table.number(&parts),
            PartsEdit::AddColumn => {
                let name = self.tab.parts_column.trim().to_string();
                if !name.is_empty() && !table.columns.contains(&name) {
                    table.columns.push(name);
                    self.tab.parts_column.clear();
                }
            }
            PartsEdit::Copy | PartsEdit::Save => {
                let csv = if table.by_component && components {
                    crate::parts::levels_csv(&parts, &rows, &table.columns)
                } else {
                    crate::parts_csv(&parts, &table.columns)
                };
                if edit_copy {
                    ctx.request(core_document::HostRequest::CopyText(csv));
                    ctx.log_info("Parts list copied");
                } else {
                    ctx.request(core_document::HostRequest::SaveFile {
                        name: "parts.csv".into(),
                        kind: "Comma-separated values".into(),
                        extension: "csv".into(),
                        contents: csv.into_bytes(),
                    });
                }
            }
            PartsEdit::RemoveColumn(i) => {
                let column = table.columns.get(i)?.clone();
                table.columns.retain(|c| *c != column);
                for entry in table.entries.values_mut() {
                    entry.values.remove(&column);
                }
            }
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
        let Some(play) = self.tab.playing.take() else {
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

    fn joint_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: &[(BodyId, BodyPlacement)],
    ) -> TaskOutcome {
        let created = before.is_none();
        // Cancel's edits, the joint and the placements its solves set,
        // are taken back by the host.
        if request.cancel {
            self.tab.playing = None;
            if created {
                ctx.active_document_object = None;
            }
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        if request.accept {
            self.stop_playing(ctx);
            crate::commands::record_joint(ctx, id, before.as_ref(), placements);
            self.tab.task = None;
            ctx.active_document_object = None;
            return TaskOutcome::Accepted {
                label: if created { "Add joint" } else { "Edit joint" }.to_string(),
            };
        }
        if ctx.document.get_feature_meta(id).is_none() {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_sweep(ctx);
        TaskOutcome::Open
    }

    /// Move a swept drive on by `dt` seconds, back and forth through its
    /// range every four seconds. A formula holding the drive's value stops
    /// the sweep, which would fight it every frame.
    pub(crate) fn advance_play(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        dt: f64,
    ) {
        let Some(joint) = ctx
            .document
            .get_feature_data(id)
            .and_then(|d| JointFeature::from_json(d).ok())
        else {
            return;
        };
        let (drive, base) = match &joint.kind {
            JointKind::Hinge { drive, .. } => (drive, "/kind/Hinge/drive"),
            JointKind::Slider { drive, .. } => (drive, "/kind/Slider/drive"),
            _ => return,
        };
        if drive.to.is_none() {
            return;
        }
        if ctx
            .document
            .feature_formula(id, &format!("{base}/to"))
            .is_some()
        {
            self.tab.playing = self.tab.playing.filter(|p| p.joint != id);
            return;
        }
        let Some(play) = self.tab.playing.as_mut().filter(|p| p.joint == id) else {
            return;
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
    }

    /// A joint's settings: its bodies, its kind and faces, what it holds
    /// and how its motion is driven or limited, the check of that motion
    /// for collisions, a turn of its body, then what the last solve said
    /// and the joint's Delete.
    fn joint_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: FeatureId,
        form: &mut Form<'_>,
    ) -> Option<JointEdit> {
        let node = ctx.document.get_feature_meta(id)?;
        let Ok(joint) = JointFeature::from_json(&node.data) else {
            form.push(w::note(
                NoteKind::Error,
                Some("Unreadable joint"),
                "The stored joint does not parse.",
            ));
            return None;
        };
        let mut edit = None;
        let (now, align_now) = joint_now(ctx, node.body, &joint);
        /// A number of the joint's that `asm.set` takes by its field's
        /// id, `key`, a formula setting it at `path`.
        fn bound(
            form: &mut Form<'_>,
            id: FeatureId,
            key: &'static str,
            (label, value): (&str, f32),
            path: &str,
            hint: &str,
        ) -> Option<JointEdit> {
            let dim = if key == "degrees" {
                Dim::Angle
            } else {
                Dim::Length
            };
            let (field, typed) = f::bound(form, key, label, value, dim, (id, path));
            form.push(w::hinted(hint, field));
            typed.map(|value| JointEdit::Number(key, value as f32))
        }
        form.push(w::value(
            "Moves",
            node.body.map(|b| body_name(ctx, b)).unwrap_or_default(),
        ));
        form.push(w::value("Against", body_name(ctx, joint.other_body)));
        if let Some(current) = JointTool::of_kind(&joint.kind) {
            let (kind, picked) = f::select(
                form,
                "kind",
                bench_api::Select {
                    label: "Kind".into(),
                    options: JointTool::ALL
                        .iter()
                        .map(|t| t.label().to_string())
                        .collect(),
                    selected: JointTool::ALL
                        .iter()
                        .position(|t| *t == current)
                        .unwrap_or(0),
                    ..Default::default()
                },
            );
            form.push(kind);
            if let Some(index) = picked {
                edit = Some(JointEdit::Kind(index));
            }
            let (repick, pressed) =
                f::button(form, "repick", "Pick faces again", ButtonStyle::Secondary);
            form.push(w::hinted(
                "Pick the two faces afresh; the joint keeps its name and kind",
                repick,
            ));
            if pressed {
                edit = Some(JointEdit::Repick);
            }
        }
        match &joint.kind {
            JointKind::Mate { flip, offset } => {
                edit = bound(
                    form,
                    id,
                    "offset",
                    ("Gap", *offset),
                    "/kind/Mate/offset",
                    "How far apart the two faces sit",
                )
                .or(edit);
                let (same_way, on) = f::toggle(form, "flip", "Same way", *flip);
                form.push(w::hinted(
                    "The faces point the same way instead of at each other",
                    same_way,
                ));
                if let Some(on) = on {
                    edit = Some(JointEdit::Flip(on));
                }
            }
            JointKind::Angle { degrees } => {
                edit = bound(
                    form,
                    id,
                    "degrees",
                    ("Angle", *degrees),
                    "/kind/Angle/degrees",
                    "Between the faces' outward normals: 180 faces them at each other",
                )
                .or(edit);
                form.push(w::text(
                    "Only the turn is held: pair it with a mate or an alignment to \
                     say where the body sits.",
                ));
            }
            JointKind::Ground => form.push(w::text(
                "The body stays where it is; the bodies joined to it are \
                 placed against it.",
            )),
            JointKind::Align { turn, slide, .. } => {
                let (turn_now, slide_now) = align_now.unzip();
                for (title, prefix, drive, angular, now) in [
                    ("Turn", "turn", turn, true, turn_now),
                    ("Slide", "slide", slide, false, slide_now),
                ] {
                    let base = if angular {
                        "/kind/Align/turn"
                    } else {
                        "/kind/Align/slide"
                    };
                    let asked = form.nest(
                        |form| {
                            self.drive_walk(
                                ctx,
                                id,
                                (prefix, base),
                                (drive, now),
                                (angular, false),
                                form,
                            )
                        },
                        |children| w::section(title, children),
                    );
                    if let Some(asked) = asked {
                        edit = Some(JointEdit::Drive(prefix, asked));
                    }
                }
                form.push(w::text(
                    "The body can turn about the axis and slide along it, each \
                     free, held or kept within limits.",
                ));
            }
            JointKind::Hinge { offset, drive, .. } => {
                edit = bound(
                    form,
                    id,
                    "offset",
                    ("Height", *offset),
                    "/kind/Hinge/offset",
                    "How far along the axis the body sits from the other",
                )
                .or(edit);
                let base = "/kind/Hinge/drive";
                if let Some(asked) =
                    self.drive_walk(ctx, id, ("drive", base), (drive, now), (true, true), form)
                {
                    edit = Some(JointEdit::Drive("drive", asked));
                }
                form.push(w::text(
                    "The body can only turn about the axis. Its angle counts from \
                     where it sat when the joint was made.",
                ));
            }
            JointKind::Distance { offset } => {
                edit = bound(
                    form,
                    id,
                    "offset",
                    ("Distance", *offset),
                    "/kind/Distance/offset",
                    "Along the other face's normal",
                )
                .or(edit);
                form.push(w::text(
                    "Only the distance is held: the faces may turn and slide past \
                     each other.",
                ));
            }
            JointKind::Tangent { radius } => {
                edit = bound(
                    form,
                    id,
                    "radius",
                    ("Radius", *radius),
                    "/kind/Tangent/radius",
                    "The round face's radius",
                )
                .or(edit);
                form.push(w::text(
                    "The round face rests on the flat one; it can still roll and \
                     slide along it.",
                ));
            }
            JointKind::Slider { drive, .. } => {
                let base = "/kind/Slider/drive";
                if let Some(asked) =
                    self.drive_walk(ctx, id, ("drive", base), (drive, now), (false, true), form)
                {
                    edit = Some(JointEdit::Drive("drive", asked));
                }
                form.push(w::text(
                    "The body can only slide along the axis, turned as it was when \
                     the joint was made.",
                ));
            }
            JointKind::Fixed { shift, .. } => {
                for (k, label) in ["Shift x", "Shift y", "Shift z"].into_iter().enumerate() {
                    let (field, typed) = f::bound(
                        form,
                        &format!("shift_{k}"),
                        label,
                        shift[k] as f32,
                        Dim::Length,
                        (id, &format!("/kind/Fixed/shift/{k}")),
                    );
                    form.push(w::hinted(
                        "Where the body sits from the other, along the other's own axes",
                        field,
                    ));
                    if let Some(value) = typed {
                        edit = Some(JointEdit::Shift(k, value as f32));
                    }
                }
                form.push(w::text(
                    "The body is held to the other as it sat when the joint was made; \
                     it moves only with it.",
                ));
            }
            JointKind::Parallel => form.push(w::text(
                "Only the turn is held, the faces parallel: pair it with other joints \
                 to say where the body sits.",
            )),
            JointKind::Perpendicular => form.push(w::text(
                "Only the turn is held, the faces square: pair it with other joints \
                 to say where the body sits.",
            )),
            JointKind::Ball => form.push(w::text(
                "The two points are held as one; the body can still turn every way \
                 about them.",
            )),
            JointKind::Universal => form.push(w::text(
                "The pins cross at one point, square to each other: the body turns \
                 about either pin.",
            )),
            JointKind::Slot => form.push(w::text(
                "The pin stays on the slot's line: it slides along it and turns every way.",
            )),
            JointKind::Width => form.push(w::text(
                "The tab's two faces stay centred between the slot's two walls: it \
                 slides along the slot and turns in it.",
            )),
            JointKind::Path => form.push(w::text(
                "The point stays on the edge, whatever its shape: it runs along it and \
                 turns every way.",
            )),
            JointKind::Cam { radius } => {
                edit = bound(
                    form,
                    id,
                    "radius",
                    ("Radius", *radius),
                    "/kind/Cam/radius",
                    "The follower's roller radius; 0 for a point",
                )
                .or(edit);
                form.push(w::text(
                    "The follower stays on the cam's face, a radius off it.",
                ));
            }
        }
        if joint.kind != JointKind::Ground {
            for (end, (key, label)) in [("moving_end", "Moving end"), ("fixed_end", "Fixed end")]
                .into_iter()
                .enumerate()
            {
                edit = bound(
                    form,
                    id,
                    key,
                    (label, joint.ends[end]),
                    &format!("/ends/{end}"),
                    "How far this end moves along its own normal or axis before the \
                         joint holds it",
                )
                .or(edit);
            }
        }
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = joint.kind {
            let hinge = matches!(joint.kind, JointKind::Hinge { .. });
            let range = motion_range(drive.limits, hinge, now);
            let unit = if hinge { "°" } else { " mm" };
            if let Some(asked) = self.sweep_check_walk(ctx, id, range, unit, form) {
                edit = Some(asked);
            }
        }
        if joint.kind != JointKind::Ground {
            let by = self.tab.turn_by.unwrap_or(90.0);
            let (by, typed) = f::number(form, "turn_by", "", by, Dim::Angle);
            if let Some(value) = typed {
                edit = Some(JointEdit::TurnBy(value as f32));
            }
            let (turn, pressed) = f::button(form, "turn", "Turn", ButtonStyle::Secondary);
            if pressed {
                edit = Some(JointEdit::Turn);
            }
            let (over, pressed) = f::button(form, "turn_over", "Turn over", ButtonStyle::Secondary);
            if pressed {
                edit = Some(JointEdit::TurnOver);
            }
            form.push(w::row(vec![
                by,
                w::hinted(
                    "Turn the body about the joint's axis or normal by this much",
                    turn,
                ),
                w::hinted(
                    "Half a turn across the joint: the body the other way round",
                    over,
                ),
            ]));
        }
        if joint.kind.keeps_a_turn() {
            let (turn90, pressed) = f::button(form, "turn90", "Turn 90°", ButtonStyle::Secondary);
            form.push(w::hinted(
                "Turn what the joint counts its turn from a quarter turn: the body \
                 follows, and the joint reads as it did",
                turn90,
            ));
            if pressed {
                edit = Some(JointEdit::Turn90);
            }
        }
        if let Some(note) = self.verdict_note() {
            form.push(note);
        }
        if self.redundant_now(ctx).iter().any(|(j, _)| *j == id) {
            form.push(w::note(
                NoteKind::Warning,
                Some("Redundant"),
                "The body's other joints already hold everything this one does; it could go.",
            ));
        }
        if let Some(joint) = self.solver_messages(ctx, form) {
            edit = Some(JointEdit::SelectJoint(joint));
        }
        if let Some(text) = node.body.and_then(|body| freedom_text(ctx, body)) {
            form.push(text);
        }
        let (delete, pressed) = f::button(form, "delete", "Delete joint", ButtonStyle::Destructive);
        form.push(w::hinted(
            "Remove the joint; the bodies stay where they are",
            delete,
        ));
        if pressed {
            edit = Some(JointEdit::Delete);
        }
        edit
    }

    /// A hinge's or a slider's drive, or an alignment's turn or slide:
    /// where it stands, held at a value, kept within limits and, while
    /// held, swept through its range to show the motion or record it.
    /// The fields' ids begin with `prefix`; `base` is the drive's place in
    /// the joint's data.
    fn drive_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        joint: FeatureId,
        (prefix, base): (&str, &str),
        (drive, now): (&crate::Drive, Option<f64>),
        (angular, sweep): (bool, bool),
        form: &mut Form<'_>,
    ) -> Option<DriveEdit> {
        let (dim, unit) = if angular {
            (Dim::Angle, "°")
        } else {
            (Dim::Length, " mm")
        };
        let mut edit = None;
        if let Some(now) = now {
            form.push(w::value(
                if angular { "Angle now" } else { "Position now" },
                format!("{now:.2}{unit}"),
            ));
        }
        let (driven, on) = f::toggle(
            form,
            &format!("{prefix}.drive"),
            "Drive",
            drive.to.is_some(),
        );
        form.push(w::hinted(
            if angular {
                "Hold the hinge at an angle"
            } else {
                "Hold the slider at a position"
            },
            driven,
        ));
        if let Some(on) = on {
            edit = Some(DriveEdit::Drive(on));
        }
        if let Some(to) = drive.to {
            // A drag stays within the limits, when there are any.
            let [low, high] = drive.limits.map_or([None, None], |l| l.map(Some));
            let (field, typed) = f::bound_within(
                form,
                &format!("{prefix}.to"),
                if angular { "Angle" } else { "Position" },
                to,
                dim,
                (joint, &format!("{base}/to")),
                (low, high),
            );
            form.push(w::hinted("Where the drive holds it", field));
            if let Some(value) = typed {
                edit = Some(DriveEdit::To(value as f32));
            }
        }
        let (limited, on) = f::toggle(
            form,
            &format!("{prefix}.limits"),
            "Limits",
            drive.limits.is_some(),
        );
        form.push(w::hinted(
            "Keep the motion within a range while it is not driven",
            limited,
        ));
        if let Some(on) = on {
            edit = Some(DriveEdit::Limits(on));
        }
        if let Some([low, high]) = drive.limits {
            // Dragged, an end stops at the other.
            for (end, value, label, stops) in [
                (0, low, "Lowest", (None, Some(high))),
                (1, high, "Highest", (Some(low), None)),
            ] {
                let (field, typed) = f::bound_within(
                    form,
                    &format!("{prefix}.{}", ["low", "high"][end]),
                    label,
                    value,
                    dim,
                    (joint, &format!("{base}/limits/{end}")),
                    stops,
                );
                form.push(w::hinted("An end of the range the motion stays in", field));
                if let Some(value) = typed {
                    edit = Some(DriveEdit::End(end, value as f32));
                }
            }
        }
        if drive.to.is_none() || !sweep {
            return edit;
        }
        let (record, pressed) = f::button(
            form,
            &format!("{prefix}.record"),
            "Record",
            ButtonStyle::Secondary,
        );
        form.push(w::hinted(
            "Save the sweep through its range as an animation",
            record,
        ));
        if pressed {
            edit = Some(DriveEdit::Record);
        }
        if ctx
            .document
            .feature_formula(joint, &format!("{base}/to"))
            .is_none()
        {
            let mine = self.tab.playing.is_some_and(|p| p.joint == joint);
            let (play, pressed) = f::button(
                form,
                &format!("{prefix}.play"),
                if mine { "Stop" } else { "Play" },
                ButtonStyle::Secondary,
            );
            form.push(w::hinted(
                "Sweep the drive through its range; stopping puts it back",
                play,
            ));
            if pressed {
                edit = Some(DriveEdit::Play);
            }
        }
        edit
    }

    /// The check of a joint's motion for collisions: its button, its
    /// progress, what it found.
    fn sweep_check_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: FeatureId,
        (low, high): (f32, f32),
        unit: &str,
        form: &mut Form<'_>,
    ) -> Option<JointEdit> {
        if let Some((joint, done, total)) = self.sweep_progress()
            && joint == id
        {
            form.push(Widget::Progress {
                label: format!("Checking the motion: {done} of {total} pairs"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            let (stop, pressed) = f::button(form, "stop_sweep", "Stop", ButtonStyle::Secondary);
            form.push(stop);
            return pressed.then_some(JointEdit::StopSweep);
        }
        let mut edit = None;
        let (check, pressed) = f::button(
            form,
            "check_sweep",
            "Check collisions through the motion",
            ButtonStyle::Secondary,
        );
        form.push(w::hinted(
            format!(
                "Step the drive from {low:.1}{unit} to {high:.1}{unit} and look for \
                 bodies that share material on the way"
            ),
            check,
        ));
        if pressed {
            edit = Some(JointEdit::CheckSweep);
        }
        match &self.tab.motion_clashes {
            Some((joint, Ok(found))) if *joint == id && found.is_empty() => form.push(w::note(
                NoteKind::Success,
                None,
                "No collisions through the motion",
            )),
            Some((joint, Ok(found))) if *joint == id => {
                form.push(w::note(
                    NoteKind::Error,
                    Some(&format!(
                        "{} collision{}",
                        found.len(),
                        if found.len() == 1 { "" } else { "s" }
                    )),
                    "Click one to select its first body",
                ));
                let (list, event) = f::raw(form, "motion_clashes");
                if let Some(clash) = f::picked(event).and_then(|i| found.get(i)) {
                    edit = Some(JointEdit::SelectBody(clash.a));
                }
                form.push(Widget::List {
                    id: list,
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
                form.push(w::note(NoteKind::Error, Some("Not checked"), why.clone()))
            }
            _ => {}
        }
        edit
    }

    /// A change to a joint's settings, made as `asm.set` makes it; its
    /// kind or faces changed, its motion swept, recorded or checked, its
    /// body turned, or the joint deleted.
    fn joint_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        (id, created, placements): (FeatureId, bool, &[(BodyId, BodyPlacement)]),
        edit: JointEdit,
    ) -> Option<TaskOutcome> {
        let node = ctx.document.get_feature_meta(id)?.clone();
        let mut joint = JointFeature::from_json(&node.data).ok()?;
        let (now, align_now) = joint_now(ctx, node.body, &joint);
        let set = |this: &mut Self, ctx: &mut WorkbenchRuntimeContext, args: serde_json::Value| {
            let mut args = args;
            args["joint"] = serde_json::json!(id.0.to_string());
            let _ = this.call(ctx, "asm.set", args);
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
        match edit {
            JointEdit::Drive(prefix, asked) => {
                let (drive, base, to_arg, range_arg, angular, now) = drive_of(&joint, prefix)?;
                match asked {
                    DriveEdit::Drive(true) => {
                        set(
                            self,
                            ctx,
                            serde_json::json!({to_arg: now.unwrap_or(0.0) as f32}),
                        );
                    }
                    DriveEdit::Drive(false) => {
                        self.clear_formula(ctx, id, &format!("{base}/to"));
                        self.tab.playing = None;
                        set(self, ctx, serde_json::json!({to_arg: false}));
                    }
                    DriveEdit::To(value) => set(self, ctx, serde_json::json!({to_arg: value})),
                    DriveEdit::Limits(true) => {
                        let at = now.unwrap_or(0.0) as f32;
                        let range = if angular {
                            [(at - 45.0).max(-180.0), (at + 45.0).min(180.0)]
                        } else {
                            [at - 10.0, at + 10.0]
                        };
                        set(self, ctx, serde_json::json!({range_arg: range}));
                    }
                    DriveEdit::Limits(false) => {
                        for end in 0..2 {
                            self.clear_formula(ctx, id, &format!("{base}/limits/{end}"));
                        }
                        set(self, ctx, serde_json::json!({range_arg: false}));
                    }
                    DriveEdit::End(end, value) => {
                        let mut range = drive.limits?;
                        range[end] = value;
                        let [mut low, mut high] = range;
                        if low > high {
                            std::mem::swap(&mut low, &mut high);
                        }
                        set(self, ctx, serde_json::json!({range_arg: [low, high]}));
                    }
                    DriveEdit::Record => {
                        let to = drive.to?;
                        let mine = self.tab.playing.filter(|p| p.joint == id);
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
                    DriveEdit::Play => {
                        let to = drive.to?;
                        let back = match self.tab.playing.filter(|p| p.joint == id) {
                            Some(p) => {
                                self.tab.playing = None;
                                p.start
                            }
                            None => {
                                let (low, high) = sweep_range(drive.limits, angular, f64::from(to));
                                let span = (high - low).max(1e-6);
                                let from = ((f64::from(to) - low) / span).clamp(0.0, 1.0);
                                self.tab.playing = Some(crate::Play {
                                    joint: id,
                                    start: to,
                                    phase: (1.0 - 2.0 * from).acos(),
                                });
                                to
                            }
                        };
                        set(self, ctx, serde_json::json!({to_arg: back}));
                    }
                }
            }
            JointEdit::Kind(index) => {
                let current = JointTool::of_kind(&joint.kind)?;
                let tool = *JointTool::ALL.get(index).filter(|t| **t != current)?;
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
            JointEdit::Repick => {
                let current = JointTool::of_kind(&joint.kind)?;
                if created {
                    crate::commands::record_joint(ctx, id, None, placements);
                }
                self.start_repick(id, current);
                return Some(TaskOutcome::Open);
            }
            JointEdit::Number(key, value) => set(self, ctx, serde_json::json!({key: value})),
            JointEdit::Flip(on) => set(self, ctx, serde_json::json!({"flip": on})),
            JointEdit::Shift(k, value) => {
                // A fixed joint's shift, which `asm.set` does not take.
                let JointKind::Fixed { shift, .. } = &mut joint.kind else {
                    return None;
                };
                shift[k] = f64::from(value);
                if let Err(why) = ctx.document.update_feature_data(id, joint.to_json()) {
                    ctx.log_warn(why.to_string());
                }
                ctx.document.clear_feature_dirty(id);
                self.solve_and_apply(ctx);
            }
            JointEdit::CheckSweep => {
                if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = joint.kind
                {
                    let hinge = matches!(joint.kind, JointKind::Hinge { .. });
                    let range = motion_range(drive.limits, hinge, now);
                    self.check_sweep(ctx, id, range);
                }
            }
            JointEdit::StopSweep => self.tab.sweeping = None,
            JointEdit::SelectBody(body) => {
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            JointEdit::TurnBy(value) => self.tab.turn_by = Some(value),
            JointEdit::Turn => {
                let by = f64::from(self.tab.turn_by.unwrap_or(90.0));
                self.turn_in_task(ctx, id, created, placements, by, false);
            }
            JointEdit::TurnOver => self.turn_in_task(ctx, id, created, placements, 0.0, true),
            JointEdit::Turn90 => {
                let args = serde_json::json!({"joint": id.0.to_string()});
                self.run_in_task(ctx, id, created, placements, "asm.turn90", args);
            }
            JointEdit::SelectJoint(joint) => {
                ctx.request(core_document::HostRequest::SelectFeatures(vec![joint]));
            }
            JointEdit::Delete => {
                ctx.document.remove_feature(id).ok()?;
                self.tab.playing = None;
                // A joint the task made has nothing to undo in a recording.
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
                    label: "Delete joint".to_string(),
                });
            }
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
        crate::keep_recorded(ctx, created);
        self.tab.task = Some(crate::Task::Joint {
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
        crate::keep_recorded(ctx, created);
        self.tab.task = Some(crate::Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: crate::all_placements(ctx),
        });
    }

    /// A coupling's settings: its two joints, what ties them, the ratio.
    /// Each change is made as `asm.set` makes it and solves at once.
    fn coupling_turn(
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
    fn coupling_walk(
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
    fn coupling_edit(
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

    fn move_turn(
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
    fn move_walk(
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
    fn move_edit(
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

#[cfg(all(test, feature = "egui"))]
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

/// A figure to about four significant digits: plainly while it reads so,
/// in powers of ten when it is very large or small.
fn figure(v: f64) -> String {
    if v == 0.0 || (0.01..1e6).contains(&v.abs()) {
        format!("{v:.2}")
    } else {
        format!("{v:.3e}")
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
