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

mod copies;
mod coupling;
mod explode;
mod interference;
mod joint;
mod motion;
mod moving;
mod parts;

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

/// The axes in the order the copies panel offers them: X, Y, Z, which
/// are also the normals of the YZ, XZ and XY planes.
const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

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
        let out = bench_panel::show(
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
}

#[cfg(all(test, feature = "egui"))]
#[path = "panel_tests.rs"]
mod tests;
