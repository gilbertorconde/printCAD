//! The Design task: one feature (or datum) open for editing in the
//! task panel. Edits apply live; Cancel restores the payload captured when
//! the task opened, or deletes a feature the tool itself just created.

use bench_api::{ButtonStyle, Entry, Fields, GapSize, NoteKind, PanelEvent, Stack, Widget};
use core_document::{
    FeatureId, TaskInfo, TaskOutcome, TaskRequest, WorkbenchFeature, WorkbenchRuntimeContext,
};
use ui_kit::tokens::{SPACE_2, SPACE_3};

use crate::feature::DesignFeature;
use crate::form::Form;
use crate::{DesignWorkbench, editors};

/// The task panel's id, keeping its widgets' state.
const PANEL: &str = "design_task";

/// The list of a feature's failing edges.
const FAILING: &str = "failing edges";

/// What the open task edits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TaskKind {
    Part,
    Datum,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskState {
    pub feature: FeatureId,
    pub kind: TaskKind,
    /// The feature payload when the task opened, which OK's recorded
    /// call is told apart from.
    pub snapshot: serde_json::Value,
    /// The tool created this feature just now.
    pub created_by_tool: bool,
    /// The tool that made it and the body it was made for, when a tool
    /// did: OK records the command that makes it.
    pub made_by: Option<(String, core_document::BodyId)>,
    /// The name as typed in the Name field and not yet applied: it goes
    /// in when the field is left or the task accepted, and Esc drops it.
    pub name_draft: Option<String>,
    /// The bodies a split made, its pieces.
    pub pieces: Vec<core_document::BodyId>,
    /// The sketch the tool closed for the feature: the notice that says
    /// so undoes it, as Cancel does.
    pub repaired: Option<crate::profile_repair::SketchRepair>,
}

/// The Name field of the task panel.
#[cfg(test)]
fn name_field_id() -> egui::Id {
    egui::Id::new(PANEL).with(("entry", "Name"))
}

/// The kind of task node `id` is, if it is one this workbench edits.
pub(crate) fn task_kind(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Option<TaskKind> {
    let node = ctx.document.get_feature_meta(id)?;
    match node.workbench_id.as_str() {
        "wb.design" => Some(TaskKind::Part),
        "core.datum" => Some(TaskKind::Datum),
        _ => None,
    }
}

impl DesignWorkbench {
    /// The feature the task panel is for: the open task's, else one the
    /// user asked to edit, else one a tool just made. Selecting a feature
    /// alone opens nothing.
    pub(crate) fn task_target(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(FeatureId, TaskKind)> {
        [
            self.tab.task.as_ref().map(|t| t.feature),
            self.tab.edit_request,
            self.tab.pending_task_from_tool.as_ref().map(|m| m.feature),
        ]
        .into_iter()
        .flatten()
        .find_map(|id| task_kind(ctx, id).map(|kind| (id, kind)))
    }

    /// The task open, or about to open, in the panel.
    pub(crate) fn task_info(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        let (id, kind) = self.task_target(ctx)?;
        let node = ctx.document.get_feature_meta(id)?;
        let (icon, subtitle) = match kind {
            TaskKind::Part => DesignFeature::from_json(&node.data)
                .map(|f| (f.icon(), f.kind_label().to_string()))
                .unwrap_or(("tree-feature", String::new())),
            TaskKind::Datum => core_document::DatumFeature::from_json(&node.data)
                .map(|d| (crate::datum_icon(&d), d.shape.label().to_string()))
                .unwrap_or(("datum-plane", String::new())),
        };
        use core_document::{StatusTone, TaskStatus};
        let status = match (&node.error, node.dirty) {
            (Some(error), _) => TaskStatus {
                text: "Rebuild failed".into(),
                detail: error.clone(),
                tone: StatusTone::Bad,
            },
            (None, true) => TaskStatus {
                text: "Rebuilding…".into(),
                detail: String::new(),
                tone: StatusTone::Info,
            },
            (None, false) if self.options.update_while_editing => TaskStatus {
                text: "Live preview".into(),
                detail: "The view shows the result as you edit. Done keeps it, Cancel \
                         puts the feature back."
                    .into(),
                tone: StatusTone::Good,
            },
            (None, false) => TaskStatus {
                text: "Rebuilds on Done".into(),
                detail: "The solid rebuilds when you press Done.".into(),
                tone: StatusTone::Info,
            },
        };
        Some(TaskInfo {
            title: node.name.clone(),
            subtitle,
            status: Some(status),
            icon,
            confirmable: true,
            stepwise: false,
        })
    }

    /// Open the task for `id`, capturing its payload.
    fn open_task(&mut self, ctx: &WorkbenchRuntimeContext, id: FeatureId, kind: TaskKind) {
        let Some(node) = ctx.document.get_feature_meta(id) else {
            return;
        };
        let created = self
            .tab
            .pending_task_from_tool
            .take()
            .filter(|made| made.feature == id);
        self.tab.task = Some(TaskState {
            feature: id,
            kind,
            snapshot: node.data.clone(),
            created_by_tool: created.is_some(),
            pieces: created
                .as_ref()
                .map(|m| m.pieces.clone())
                .unwrap_or_default(),
            repaired: created.as_ref().and_then(|m| m.repaired.clone()),
            made_by: created.map(|m| (m.tool, m.body)),
            name_draft: None,
        });
    }

    /// Draw the task body and settle accept/cancel.
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        // Selecting something else (or nothing) accepts the open task: the
        // edits so far stay.
        if self
            .tab
            .task
            .as_ref()
            .is_some_and(|t| ctx.active_document_object != Some(t.feature))
        {
            if let Some(task) = &self.tab.task {
                Self::apply_name_draft(ctx, task);
            }
            let label = self.task_label(ctx);
            if let Some(task) = self.tab.task.take() {
                crate::commands::record_task(self, ctx, &task);
                self.rebuild_accepted(ctx, task.feature);
            }
            return TaskOutcome::Accepted { label };
        }
        let Some((target_id, target_kind)) = self.task_target(ctx) else {
            self.tab.edit_request = None;
            return TaskOutcome::Open;
        };
        if self.tab.task.is_none() {
            self.tab.edit_request = None;
            self.open_task(ctx, target_id, target_kind);
            ctx.active_document_object = Some(target_id);
        }

        if request.cancel {
            return self.cancel_task(ctx);
        }
        if request.accept {
            if let Some(task) = &self.tab.task {
                Self::apply_name_draft(ctx, task);
            }
            let label = self.task_label(ctx);
            if let Some(task) = self.tab.task.take() {
                crate::commands::record_task(self, ctx, &task);
            }
            // The feature stays selected, out of its task.
            self.rebuild_accepted(ctx, target_id);
            return TaskOutcome::Accepted { label };
        }

        if ctx.document.get_feature_meta(target_id).is_none() {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        let widgets = self.task_widgets(ctx, target_id);
        let out = bench_panel::show(ui, egui::Id::new(PANEL), &widgets, Some(&*ctx.document));
        self.apply_output(ctx, target_id, out);
        TaskOutcome::Open
    }

    /// What the panel reported: the formulas typed, written to the
    /// feature, then each event through the editor that declared its
    /// widget. A formula's value comes with it and is not written, so the
    /// feature's data keeps the number it had.
    fn apply_output(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        target_id: FeatureId,
        out: bench_panel::PanelOutput,
    ) {
        let by_formula = out.write_formulas(ctx);
        for event in out.events_besides(&by_formula) {
            if self.tab.task.is_some() {
                self.task_event(ctx, target_id, event);
            }
        }
    }

    /// The open task's panel as widgets, after any change the editor
    /// makes on its own (a count of gaps following the occurrences).
    pub(crate) fn task_widgets(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
    ) -> Vec<Widget> {
        let mut f = Form::declare();
        self.task_pass(&mut f, ctx, id);
        let children = f.finish();
        let widgets = vec![
            Stack {
                children: vec![
                    Fields {
                        children,
                        label_width: Some(editors::LABEL_COLUMN),
                    }
                    .into(),
                ],
                spacing: Some(SPACE_3),
                ..Default::default()
            }
            .into(),
        ];
        widgets
    }

    /// What `event` does to the open task's feature, through the editor
    /// that declared the widget it names.
    pub(crate) fn task_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        event: &PanelEvent,
    ) {
        if let PanelEvent::Hover { id: list, index } = event
            && list == FAILING
        {
            self.tab.hovered_failing = *index;
            return;
        }
        let mut f = Form::apply(event);
        self.task_pass(&mut f, ctx, id);
    }

    /// One pass over the task's panel: the editors declare their widgets,
    /// or apply the event, and what they change is written.
    fn task_pass(&mut self, f: &mut Form, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        let Some(node) = ctx.document.get_feature_meta(id).cloned() else {
            return;
        };
        f.freeze_errors = std::mem::take(&mut self.tab.freeze_errors);
        match self.tab.task.as_ref().map(|t| t.kind.clone()) {
            Some(TaskKind::Datum) => self.datum_card(f, ctx, id, &node),
            _ => self.part_card(f, ctx, id, &node),
        }
        self.tab.freeze_errors = std::mem::take(&mut f.freeze_errors);
    }

    /// With the live preview off, the accepted edit is what rebuilds,
    /// however the task was accepted (OK, deselecting, picking another).
    fn rebuild_accepted(&self, ctx: &mut WorkbenchRuntimeContext, feature: FeatureId) {
        if !self.options.update_while_editing {
            ctx.document.mark_feature_dirty(feature);
        }
    }

    /// A name typed and not yet applied goes in with the accepted task.
    fn apply_name_draft(ctx: &mut WorkbenchRuntimeContext, task: &TaskState) {
        if let Some(name) = &task.name_draft
            && !name.trim().is_empty()
        {
            ctx.document.rename_feature(task.feature, name.clone());
        }
    }

    fn task_label(&self, ctx: &WorkbenchRuntimeContext) -> String {
        let Some(task) = &self.tab.task else {
            return "Edit feature".to_string();
        };
        let name = ctx
            .document
            .get_feature_meta(task.feature)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "feature".to_string());
        if task.created_by_tool {
            format!("Create {name}")
        } else {
            format!("Edit {name}")
        }
    }

    /// End the task by its Cancel. The host takes back through the
    /// journal everything done while it was open (the feature a tool made
    /// with its base, own sketch, closed profile and split pieces, or the
    /// edit, its formulas, name and the sketches it showed or hid); here
    /// only what is the bench's own goes. A feature the tool made is let
    /// go of, and the host puts back what was selected before the task;
    /// an edit put back leaves the feature selected.
    fn cancel_task(&mut self, ctx: &mut WorkbenchRuntimeContext) -> TaskOutcome {
        let Some(task) = self.tab.task.take() else {
            return TaskOutcome::Cancelled;
        };
        if task.created_by_tool {
            ctx.active_document_object = None;
            ctx.log_info(if task.pieces.is_empty() {
                "Feature discarded"
            } else {
                "Split discarded"
            });
        }
        TaskOutcome::Cancelled
    }

    /// The Name row. What is typed is held as the task's draft and
    /// applied when the field is left; Esc leaves it with the name as it
    /// was.
    fn name_row(
        &mut self,
        f: &mut Form,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        node_name: &str,
    ) {
        let Some(task) = self.tab.task.as_mut() else {
            return;
        };
        let typed = f.entry(
            "Name",
            Entry {
                label: "Name".into(),
                value: task
                    .name_draft
                    .clone()
                    .unwrap_or_else(|| node_name.to_owned()),
                width: Some(160.0),
                live: true,
                ..Default::default()
            },
        );
        match typed {
            Some(PanelEvent::Text { value, .. }) => task.name_draft = Some(value.clone()),
            Some(PanelEvent::TextLeft { escaped, .. }) => {
                if let Some(name) = task.name_draft.take()
                    && !escaped
                    && name != node_name
                    && !name.trim().is_empty()
                {
                    ctx.document.rename_feature(feature_id, name);
                }
            }
            _ => {}
        }
    }

    fn part_card(
        &mut self,
        f: &mut Form,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        node: &core_document::FeatureNode,
    ) {
        let Some(mut feature) = DesignFeature::from_json(&node.data).ok() else {
            f.note(
                NoteKind::Error,
                Some("Unreadable feature"),
                "The stored payload does not parse.",
            );
            return;
        };
        let Some(body) = node.body else {
            return;
        };
        self.name_row(f, ctx, feature_id, &node.name);
        self.repair_notice(f, ctx);
        let deps_before = feature.dependencies();
        let sketch_before = feature.sketch();
        let (changed, formula_edits) = {
            let shown: &WorkbenchRuntimeContext = ctx;
            let mut fx = editors::Formulas::of(shown.document, feature_id);
            let changed =
                editors::feature_editor(f, shown, &mut fx, body, feature_id, &mut feature);
            (changed, std::mem::take(&mut fx.edits))
        };
        for (key, formula) in formula_edits {
            if let Err(why) = ctx.document.set_feature_formula(feature_id, key, formula) {
                ctx.log_warn(why.to_string());
            }
        }
        if changed {
            self.apply_part_edit(ctx, feature_id, &deps_before, sketch_before, &feature);
        }

        // The header's pill says it is rebuilding or previewing; a failure
        // also gets its whole message here.
        let Some(error) = &node.error else {
            self.tab.hovered_failing = None;
            return;
        };
        f.gap(GapSize::Small);
        f.note(NoteKind::Error, Some("Recompute failed"), error);
        if node.refused_edges.failing.is_empty() {
            self.tab.hovered_failing = None;
        } else {
            f.gap(GapSize::Small);
            let sharp = crate::failing::sharp_count(ctx.document, feature_id);
            let largest = crate::failing::largest_size(ctx.document, feature_id);
            let rows = crate::failing::rows(
                f,
                &node.refused_edges.failing,
                crate::failing::asked_size(&feature),
                largest,
                sharp,
            );
            if rows.leave_sharp
                && let Err(why) = crate::failing::leave_sharp(ctx, feature_id)
            {
                ctx.log_warn(why);
            }
            if rows.use_largest
                && let Err(why) = crate::failing::use_largest(ctx, feature_id)
            {
                ctx.log_warn(why);
            }
        }
        if let Some(hint) = faceted_hint(ctx.document, body, feature_id) {
            f.gap(GapSize::Small);
            f.note(
                NoteKind::Warning,
                Some("The body is still in facets"),
                &hint,
            );
        }
    }

    /// What the tool closed in the sketch for the feature, and the button
    /// that puts the sketch back.
    fn repair_notice(&mut self, f: &mut Form, ctx: &mut WorkbenchRuntimeContext) {
        let Some(repair) = self.tab.task.as_ref().and_then(|t| t.repaired.clone()) else {
            return;
        };
        f.gap(GapSize::Small);
        f.note(
            NoteKind::Info,
            Some("Profile closed"),
            &repair.describe(ctx.document),
        );
        if f.button(
            "Undo the join",
            ButtonStyle::Small,
            true,
            "Puts the sketch back as it was; the feature then names its loose ends",
        ) {
            match repair.undo(ctx.document) {
                Ok(()) => {
                    if let Some(task) = self.tab.task.as_mut() {
                        task.repaired = None;
                    }
                }
                Err(why) => ctx.log_warn(why),
            }
        }
        f.gap(GapSize::Small);
    }

    /// Write an edit the panel made to the feature: its data, the
    /// dependencies it reads, and the sketches it consumes.
    fn apply_part_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        deps_before: &[FeatureId],
        sketch_before: Option<FeatureId>,
        feature: &DesignFeature,
    ) {
        let deps_after = feature.dependencies();
        let before = ctx
            .document
            .get_feature_data(feature_id)
            .and_then(|data| DesignFeature::from_json(data).ok());
        if ctx
            .document
            .update_feature_data(feature_id, feature.to_json())
            .is_ok()
        {
            if let Some(before) = &before {
                crate::split::sync(ctx.document, feature_id, before, feature);
            }
            if deps_before != deps_after.as_slice() {
                ctx.document
                    .set_feature_dependencies(feature_id, deps_after);
            }
            // A new profile is consumed like the first: it hides, and
            // the one it replaced shows again.
            if sketch_before != feature.sketch() {
                crate::references::swap_consumed_sketch(
                    ctx.document,
                    feature_id,
                    sketch_before,
                    feature.sketch(),
                );
            }
            if matches!(feature, DesignFeature::Text { .. })
                && let Err(why) = crate::text_feature::sync(ctx.document, feature_id)
            {
                ctx.log_warn(why);
            }
            if self.options.update_while_editing {
                ctx.document.mark_feature_dirty(feature_id);
            }
        }
    }

    fn datum_card(
        &mut self,
        f: &mut Form,
        ctx: &mut WorkbenchRuntimeContext,
        datum_id: FeatureId,
        node: &core_document::FeatureNode,
    ) {
        let Some(mut datum) = core_document::DatumFeature::from_json(&node.data).ok() else {
            f.note(
                NoteKind::Error,
                Some("Unreadable datum"),
                "The stored payload does not parse.",
            );
            return;
        };
        self.name_row(f, ctx, datum_id, &node.name);
        let (changed, formula_edits) = {
            let shown: &WorkbenchRuntimeContext = ctx;
            let mut fx = editors::Formulas::of(shown.document, datum_id);
            let changed = editors::datum_editor(f, shown, &mut fx, datum_id, &mut datum);
            (changed, std::mem::take(&mut fx.edits))
        };
        for (key, formula) in formula_edits {
            if let Err(why) = ctx.document.set_feature_formula(datum_id, key, formula) {
                ctx.log_warn(why.to_string());
            }
        }
        if changed {
            // A sketch drawn on this datum follows it (`Workbench::derive`),
            // and what stands on the sketch rebuilds.
            match ctx.document.update_feature_data(datum_id, datum.to_json()) {
                Ok(()) => crate::datum_refs::sync_dependencies(ctx, datum_id, &datum),
                Err(why) => ctx.log_warn(why.to_string()),
            }
        }
        f.gap(GapSize::Small);
        let delete = f.card(SPACE_2, |f| {
            f.button(
                "Delete datum",
                ButtonStyle::Destructive,
                true,
                "Remove this datum",
            )
        });
        if delete && ctx.document.remove_feature(datum_id).is_ok() {
            self.tab.task = None;
            ctx.active_document_object = None;
        }
    }
}

/// What to do about a feature that failed on a solid converted from a mesh
/// whose curved areas are still facets: refine the body first. `None` for
/// any other body.
fn faceted_hint(
    document: &core_document::Document,
    body: core_document::BodyId,
    feature: FeatureId,
) -> Option<String> {
    if !document.body_health(body).is_some_and(|h| h.faceted) {
        return None;
    }
    let others = crate::build::design_features_of_body(document, body)
        .into_iter()
        .filter(|(id, f)| *id != feature && !matches!(f, DesignFeature::Base { .. }))
        .count();
    let intro = "This body was converted from a mesh and its curved areas are still flat \
                 facets, which features on or along them may fail against.";
    Some(if others == 0 {
        format!(
            "{intro} Cancel, right-click the body › Refine shape to find its round faces, \
             then make this feature again."
        )
    } else {
        format!(
            "{intro} Refine shape is offered while the body has no features: delete them \
             (the Base last), then right-click the body › Refine shape."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{bare_geometry, pad};
    use core_document::Document;
    use wb_sketch::SketchFeature;
    use wb_sketch::sketch::{GeometryElement, Point, Sketch, Vec2D};

    fn rect_sketch() -> SketchFeature {
        crate::test_support::rect_sketch(10.0, 5.0)
    }

    /// A body with two sketches, a pad on each, the first sketch hidden
    /// as its pad's tool leaves it.
    struct Scene {
        doc: Document,
        first: FeatureId,
        second: FeatureId,
        pad: FeatureId,
        other_pad: FeatureId,
    }

    fn scene() -> Scene {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let first = doc
            .add_feature_in_body(rect_sketch(), "Sketch".into(), Some(body))
            .unwrap();
        let second = doc
            .add_feature_in_body(rect_sketch(), "Sketch_1".into(), Some(body))
            .unwrap();
        let pad_id = doc
            .add_feature_in_body(pad(first, 10.0), "Pad".into(), Some(body))
            .unwrap();
        let other_pad = doc
            .add_feature_in_body(pad(second, 3.0), "Pad_1".into(), Some(body))
            .unwrap();
        doc.set_feature_visible(first, false);
        doc.clear_feature_dirty(pad_id);
        doc.clear_feature_dirty(other_pad);
        Scene {
            doc,
            first,
            second,
            pad: pad_id,
            other_pad,
        }
    }

    fn panel() -> egui::Context {
        let panel = egui::Context::default();
        ui_kit::apply_theme(&panel);
        panel
    }

    /// One frame of the task panel with `active` selected.
    fn frame(
        wb: &mut DesignWorkbench,
        panel: &egui::Context,
        doc: &mut Document,
        active: Option<FeatureId>,
        request: TaskRequest,
        events: Vec<egui::Event>,
    ) -> TaskOutcome {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = active;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut outcome = TaskOutcome::Open;
        let mut output = panel.run_ui(input, |ui| {
            outcome = wb.draw_task_panel(ui, &mut ctx, request);
        });
        output.textures_delta.clear();
        outcome
    }

    /// The id of the first declared widget `named` (a field's label, a
    /// check box's or a button's words).
    fn widget_id(widgets: &[Widget], named: &str) -> Option<String> {
        bench_api::walk_widgets(widgets)
            .into_iter()
            .find_map(|w| match w {
                Widget::Quantity(q) if q.label == named => Some(q.id.clone()),
                Widget::Select(c) if c.label == named => Some(c.id.clone()),
                Widget::Check(c) if c.text == named => Some(c.id.clone()),
                Widget::Entry(e) if e.label == named => Some(e.id.clone()),
                Widget::Button { id, label, .. } if label == named => Some(id.clone()),
                _ => None,
            })
    }

    /// Each kind of control the editors declare edits the feature through
    /// the event its widget tells, as one live edit of the open task.
    #[test]
    fn panel_events_edit_the_feature_through_its_editor() {
        let Scene {
            mut doc, pad: id, ..
        } = scene();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        type Event = fn(String) -> PanelEvent;
        let cases: [(&str, Event); 4] = [
            ("Type", |id| PanelEvent::Choice { id, index: 1 }),
            ("Reversed", |id| PanelEvent::Toggle { id, on: true }),
            ("Taper", |id| PanelEvent::Number { id, value: 5.0 }),
            ("Name", |id| PanelEvent::Text {
                id,
                value: "Boss".into(),
            }),
        ];
        for (named, event) in cases {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            ctx.active_document_object = Some(id);
            let widgets = wb.task_widgets(&mut ctx, id);
            let wid = widget_id(&widgets, named).unwrap_or_else(|| panic!("no {named}"));
            if named == "Taper" {
                // Bound to its formula, the taper keeps its range, which a
                // drag stops at.
                let taper = bench_api::walk_widgets(&widgets)
                    .into_iter()
                    .find_map(|w| match w {
                        Widget::Quantity(q) if q.id == wid => {
                            Some((q.bind.is_some(), q.min, q.max))
                        }
                        _ => None,
                    });
                assert_eq!(taper, Some((true, Some(-85.0), Some(85.0))));
            }
            wb.task_event(&mut ctx, id, &event(wid));
        }
        let DesignFeature::Pad {
            mode,
            reversed,
            taper_deg,
            ..
        } = DesignFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap()
        else {
            panic!("a pad");
        };
        assert_ne!(
            mode,
            crate::feature::ExtrudeMode::Dimension,
            "the second end offered"
        );
        assert!(reversed);
        assert_eq!(taper_deg, 5.0);
        assert_eq!(
            wb.tab.task.as_ref().unwrap().name_draft.as_deref(),
            Some("Boss")
        );
        assert_eq!(
            frame(&mut wb, &panel, &mut doc, Some(id), OK, vec![]),
            TaskOutcome::Accepted {
                label: "Edit Boss".into()
            }
        );
        assert_eq!(doc.get_feature_meta(id).unwrap().name, "Boss");
    }

    /// A formula typed into a field goes to the feature and leaves its
    /// number; a number typed over a formula takes the formula away; the
    /// pointer on a failing edge's row lights that edge.
    #[test]
    fn formulas_and_values_from_the_panel_reach_the_feature() {
        let Scene {
            mut doc, pad: id, ..
        } = scene();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        let length =
            |doc: &Document| match DesignFeature::from_json(doc.get_feature_data(id).unwrap()) {
                Ok(DesignFeature::Pad { length, .. }) => length,
                _ => panic!("a pad"),
            };
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(id);
        let widgets = wb.task_widgets(&mut ctx, id);
        let wid = widget_id(&widgets, "Length").unwrap();
        let bind = match bench_api::walk_widgets(&widgets)
            .into_iter()
            .find(|w| w.id() == Some(wid.as_str()))
        {
            Some(Widget::Quantity(q)) => q.bind.clone().expect("the length takes formulas"),
            other => panic!("{other:?}"),
        };
        let key = bind.key.clone();
        let typed = |formula: Option<&str>, value: f64| {
            let mut out = bench_panel::PanelOutput::default();
            out.events = vec![PanelEvent::Number {
                id: wid.clone(),
                value,
            }];
            out.formulas = vec![bench_panel::FormulaSet {
                id: wid.clone(),
                bind: bind.clone(),
                formula: formula.map(String::from),
            }];
            out
        };
        wb.apply_output(&mut ctx, id, typed(Some("6 mm * 2"), 12.0));
        assert_eq!(ctx.document.feature_formula(id, &key), Some("6 mm * 2"));
        assert_eq!(
            length(ctx.document),
            10.0,
            "the formula's value is not written"
        );
        wb.apply_output(&mut ctx, id, typed(None, 7.0));
        assert_eq!(ctx.document.feature_formula(id, &key), None);
        assert_eq!(length(ctx.document), 7.0);
        let hover = PanelEvent::Hover {
            id: FAILING.into(),
            index: Some(1),
        };
        wb.task_event(&mut ctx, id, &hover);
        assert_eq!(wb.tab.hovered_failing, Some(1));
    }

    /// With the largest radius the preview build found, the radius field
    /// and the view's handle stop a drag there, while a typed radius goes
    /// past it (and is refused by the build).
    #[test]
    fn a_fillet_drag_stops_at_the_largest_radius_and_typing_goes_past() {
        let Scene {
            mut doc,
            pad: pad_id,
            ..
        } = scene();
        let body = doc.get_feature_meta(pad_id).unwrap().body;
        let edge = crate::feature::EdgePick {
            point: [5.0, 0.0, 10.0],
            direction: [1.0, 0.0, 0.0],
            faces: [0; 2],
        };
        let fillet = DesignFeature::Fillet {
            radius: 1.0,
            edges: crate::feature::EdgeSel::Edges(vec![edge]),
            follow_tangent: false,
        };
        let key = crate::build::edge_op(&fillet)
            .unwrap()
            .unwrap()
            .size_limit_key()
            .unwrap();
        let id = doc
            .add_feature_in_body(fillet, "Fillet".into(), body)
            .unwrap();
        let built = kernel_api::SolidBuildResult {
            size_limit: Some(kernel_api::SizeLimit {
                op_index: 0,
                key,
                largest: Some(2.4999),
            }),
            ..Default::default()
        };
        doc.store_size_limit(&[id], &built);
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(id);
        let widgets = wb.task_widgets(&mut ctx, id);
        let radius = bench_api::walk_widgets(&widgets)
            .into_iter()
            .find_map(|w| match w {
                Widget::Quantity(q) if q.label == "Radius" => Some(q.clone()),
                _ => None,
            })
            .expect("a radius field");
        assert_eq!(
            radius.drag_max,
            Some(2.49_f32 as f64),
            "down to a hundredth"
        );
        // The handle: a ray far past the edge stops at the limit; the same
        // number typed is kept.
        let eye = glam::Vec3::new(5.0, -50.0, 10.0);
        let handle = crate::handles::handles_of(ctx.document, id, eye)
            .pop()
            .unwrap();
        let dragged = handle
            .value_at(glam::Vec3::new(5.0, -30.0, 40.0), glam::Vec3::NEG_Z, 1.0)
            .unwrap();
        assert_eq!(dragged, 2.49);
        assert_eq!(handle.clamped(9.0), 9.0);
        wb.task_event(
            &mut ctx,
            id,
            &PanelEvent::Number {
                id: radius.id,
                value: 9.0,
            },
        );
        let Ok(DesignFeature::Fillet { radius, .. }) =
            DesignFeature::from_json(ctx.document.get_feature_data(id).unwrap())
        else {
            panic!("a fillet");
        };
        assert_eq!(radius, 9.0, "typed, not held to the limit");
    }

    /// The body menu's Split makes two pieces, hides the body and opens
    /// the first piece's task; Cancel takes the pieces away and shows the
    /// body again, Done keeps them.
    #[test]
    fn a_split_from_the_body_menu_is_taken_back_by_cancel() {
        use core_document::Workbench;
        for accept in [false, true] {
            let mut doc = Document::new("t");
            let body = doc.create_body(Some("Plate".into()));
            doc.set_imported_brep_data(body, b"ogeom".to_vec(), Vec::new());
            let hidden =
                |doc: &Document| doc.bodies().iter().find(|b| b.id == body).unwrap().hidden;
            let split = |doc: &mut Document| {
                let mut wb = DesignWorkbench::default();
                let active = {
                    let mut ctx =
                        WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
                    let scope = core_document::MenuScope::TreeBody(body);
                    assert!(wb.on_command(crate::SPLIT, &scope, &mut ctx));
                    ctx.active_document_object
                };
                assert_eq!((doc.bodies().len(), hidden(doc)), (3, true));
                let panel = panel();
                let ask = |accept, cancel| TaskRequest { accept, cancel };
                frame(&mut wb, &panel, doc, active, ask(false, false), Vec::new());
                assert!(wb.tab.task.as_ref().is_some_and(|t| t.pieces.len() == 2));
                frame(
                    &mut wb,
                    &panel,
                    doc,
                    active,
                    ask(accept, !accept),
                    Vec::new(),
                );
            };
            if accept {
                split(&mut doc);
                assert_eq!((doc.bodies().len(), hidden(&doc)), (3, true));
            } else {
                core_document::history::assert_cancel_takes_back(&mut doc, split);
            }
        }
    }

    /// A double click on `id`'s row: the bench is asked to edit it, and
    /// the panel draws with it selected.
    fn open(
        wb: &mut DesignWorkbench,
        panel: &egui::Context,
        doc: &mut Document,
        id: FeatureId,
    ) -> TaskOutcome {
        {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            ctx.active_document_object = Some(id);
            core_document::Workbench::edit_feature(wb, &mut ctx, id);
        }
        frame(wb, panel, doc, Some(id), OPEN, vec![])
    }

    /// Text's mode and how it lies on the face are rows of icon buttons:
    /// a click sets what it lights, and the lit way clicked again lays
    /// the text flat.
    #[test]
    fn text_on_a_face_sets_its_mode_and_wrap_with_icon_buttons() {
        let Scene { mut doc, first, .. } = scene();
        let body = doc.get_feature_meta(first).and_then(|n| n.body);
        let id = doc
            .add_feature_in_body(
                DesignFeature::Text {
                    sketch: first,
                    faces: Vec::new(),
                    mode: kernel_api::ProjectMode::Emboss,
                    height: 0.6,
                    wrap: true,
                    text: "AB".into(),
                    font: wb_sketch::text::DEFAULT_FONT.into(),
                    size: 5.0,
                    spacing: 0.0,
                    angle: 0.0,
                    across: 0.0,
                    up: 0.0,
                    along: false,
                    full_turn: false,
                    radius: 10.0,
                },
                "Text".to_string(),
                body,
            )
            .unwrap();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        let click = |wb: &mut DesignWorkbench, doc: &mut Document, row: &str, n: usize| {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            ctx.active_document_object = Some(id);
            let widgets = wb.task_widgets(&mut ctx, id);
            let button = bench_api::walk_widgets(&widgets)
                .into_iter()
                .find_map(|w| match w {
                    Widget::Toggles(t) if t.label == row => Some(t.buttons[n].id.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no {row}"));
            wb.task_event(&mut ctx, id, &PanelEvent::Button { id: button });
            match DesignFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap() {
                DesignFeature::Text {
                    mode,
                    wrap,
                    along,
                    full_turn,
                    ..
                } => (mode, wrap, along, full_turn),
                _ => unreachable!(),
            }
        };
        use kernel_api::ProjectMode::{Deboss, Split};
        assert_eq!(click(&mut wb, &mut doc, "Mode", 1).0, Deboss);
        assert_eq!(
            click(&mut wb, &mut doc, "Wrap", 1),
            (Deboss, true, true, false)
        );
        assert_eq!(
            click(&mut wb, &mut doc, "Wrap", 2),
            (Deboss, true, false, true)
        );
        assert_eq!(
            click(&mut wb, &mut doc, "Wrap", 2),
            (Deboss, false, false, false)
        );
        assert_eq!(click(&mut wb, &mut doc, "Mode", 2).0, Split);
    }

    /// Selecting a feature opens nothing; asking to edit it does, and
    /// accepting leaves it selected, out of its task.
    #[test]
    fn a_selected_feature_opens_only_when_asked_to_edit() {
        let Scene {
            mut doc, pad: id, ..
        } = scene();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        {
            let mut ctx = core_document::testing::ctx(&mut doc);
            ctx.active_document_object = Some(id);
            assert!(wb.task_info(&ctx).is_none(), "selected, not edited");
        }
        frame(&mut wb, &panel, &mut doc, Some(id), OPEN, vec![]);
        assert!(wb.tab.task.is_none());

        open(&mut wb, &panel, &mut doc, id);
        assert_eq!(wb.tab.task.as_ref().map(|t| t.feature), Some(id));

        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(id);
        let mut outcome = TaskOutcome::Open;
        let mut output = panel.run_ui(egui::RawInput::default(), |ui| {
            outcome = wb.draw_task_panel(ui, &mut ctx, OK);
        });
        output.textures_delta.clear();
        assert!(matches!(outcome, TaskOutcome::Accepted { .. }));
        assert_eq!(ctx.active_document_object, Some(id), "still selected");
        assert!(wb.tab.task.is_none());
        let again = wb.task_info(&ctx);
        assert!(again.is_none(), "accepting does not reopen it");
    }

    const OPEN: TaskRequest = TaskRequest {
        accept: false,
        cancel: false,
    };
    const OK: TaskRequest = TaskRequest {
        accept: true,
        cancel: false,
    };
    const CANCEL: TaskRequest = TaskRequest {
        accept: false,
        cancel: true,
    };

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// An edit the panel makes, written as its fields write it.
    fn edit(wb: &mut DesignWorkbench, doc: &mut Document, id: FeatureId, to: DesignFeature) {
        let before = DesignFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.apply_part_edit(&mut ctx, id, &before.dependencies(), before.sketch(), &to);
    }

    /// The open pad's end carries a handle; dragging it sets the length.
    #[test]
    fn dragging_the_pad_handle_sets_its_length() {
        use core_document::{MouseButton, Workbench, WorkbenchInputEvent as E};
        let Scene {
            mut doc, pad: id, ..
        } = scene();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);

        let eye = glam::Vec3::new(60.0, -60.0, 30.0);
        let view = glam::camera::rh::view::look_at_mat4(
            eye,
            glam::Vec3::new(5.0, 2.5, 5.0),
            glam::Vec3::Z,
        );
        let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
        let mut ctx = WorkbenchRuntimeContext::new(
            &mut doc,
            eye.to_array(),
            [5.0, 2.5, 5.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some((proj * view).to_cols_array_2d());

        let handle = wb
            .handles(&ctx)
            .into_iter()
            .find(|h| h.label == "Length")
            .expect("the pad offers a handle");
        assert!((handle.position() - glam::Vec3::new(5.0, 2.5, 10.0)).length() < 1e-4);
        let at = ctx.world_to_viewport([5.0, 2.5, 10.0]).unwrap();
        let to = ctx.world_to_viewport([5.0, 2.5, 17.0]).unwrap();
        let far = ctx.world_to_viewport([40.0, 2.5, 10.0]).unwrap();

        let press = |pos| E::MousePress {
            button: MouseButton::Left,
            viewport_pos: pos,
        };
        assert!(
            !wb.on_input(&press(far), None, &mut ctx).consumed,
            "away from it"
        );
        let release = E::MouseRelease {
            button: MouseButton::Left,
            viewport_pos: to,
        };
        let length = |ctx: &WorkbenchRuntimeContext| {
            let data = ctx.document.get_feature_data(id).unwrap();
            let DesignFeature::Pad { length, .. } = DesignFeature::from_json(data).unwrap() else {
                panic!()
            };
            length
        };
        type End = fn(&mut DesignWorkbench, &mut WorkbenchRuntimeContext, &E);
        // Each ending of a drag to 17 mm, and the length it leaves: the
        // release keeps it, Escape and a cancel (focus lost, the tab put
        // away) put back the press's 10 mm.
        let ends: [(&str, End, f32); 3] = [
            (
                "Escape",
                |wb, ctx, _| {
                    let escape = E::KeyPress {
                        key: core_document::KeyCode::Escape,
                    };
                    assert!(wb.on_input(&escape, None, ctx).consumed);
                },
                10.0,
            ),
            ("cancel", |wb, ctx, _| wb.cancel_pointer_gesture(ctx), 10.0),
            (
                "release",
                |wb, ctx, release| assert!(wb.on_input(release, None, ctx).consumed),
                17.0,
            ),
        ];
        for (name, end, left) in ends {
            assert!(wb.on_input(&press(at), None, &mut ctx).consumed, "{name}");
            assert!(
                wb.on_input(&E::MouseMove { viewport_pos: to }, None, &mut ctx)
                    .consumed
            );
            assert!((length(&ctx) - 17.0).abs() < 0.051, "{name}");
            end(&mut wb, &mut ctx, &release);
            // Moves after the end take nothing, and the release is no
            // drag's.
            let past = E::MouseMove { viewport_pos: far };
            assert!(!wb.on_input(&past, None, &mut ctx).consumed, "{name}");
            assert!(!wb.on_input(&release, None, &mut ctx).consumed, "{name}");
            assert!(wb.tab.held.is_none(), "{name}");
            let length = length(&ctx);
            assert!((length - left).abs() < 0.051, "{name}: {length}");
        }
    }

    /// Digits typed while the pad's handle is held set its length on
    /// Enter, and text that is no number sets its formula.
    #[test]
    fn typing_on_a_held_handle_sets_its_number_or_formula() {
        use core_document::{KeyCode, Workbench, WorkbenchInputEvent as E};
        let Scene {
            mut doc, pad: id, ..
        } = scene();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [60.0, -60.0, 30.0], [5.0; 3], (0, 0, 800, 600));
        let length = wb
            .handles(&ctx)
            .into_iter()
            .find(|h| h.label == "Length")
            .unwrap();
        let take = |wb: &mut DesignWorkbench, ctx: &mut WorkbenchRuntimeContext, text: &str| {
            wb.tab.held = Some(length.clone());
            assert!(wb.takes_numeric_input());
            for c in text.chars() {
                wb.on_input(
                    &E::Text {
                        text: c.to_string(),
                    },
                    None,
                    ctx,
                );
            }
            wb.tab.held = None;
            assert!(
                wb.takes_text_input(),
                "the field stays open past the release"
            );
            wb.on_input(
                &E::KeyPress {
                    key: KeyCode::Enter,
                },
                None,
                ctx,
            );
        };
        take(&mut wb, &mut ctx, "12.5");
        let DesignFeature::Pad { length: set, .. } =
            DesignFeature::from_json(ctx.document.get_feature_data(id).unwrap()).unwrap()
        else {
            panic!()
        };
        assert_eq!(set, 12.5);
        take(&mut wb, &mut ctx, "2*4");
        let formulas = &ctx.document.get_feature_meta(id).unwrap().formulas;
        assert_eq!(
            formulas.values().next().map(String::as_str),
            Some("2*4"),
            "{formulas:?}"
        );
    }
    /// The pad's length dragged near a flat face square to its way lands
    /// on the face, lit while it holds; with Alt it drags past.
    #[test]
    fn a_dragged_pad_length_snaps_to_a_parallel_face() {
        use core_document::{MouseButton, Workbench, WorkbenchInputEvent as E};
        let Scene {
            mut doc, pad: id, ..
        } = scene();
        // Another body with a flat top at z = 17.3, off to the side.
        let other = doc.create_body(None);
        let top = kernel_api::TriMesh {
            positions: vec![[30.0, 0.0, 17.3], [40.0, 0.0, 17.3], [30.0, 10.0, 17.3]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            faces: vec![0],
            face_surfaces: vec![kernel_api::FaceSurface::Plane {
                origin: [30.0, 0.0, 17.3],
                normal: [0.0, 0.0, 1.0],
            }],
            ..Default::default()
        };
        doc.set_imported_geometry(
            other,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(top),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        let eye = glam::Vec3::new(60.0, -60.0, 30.0);
        let view = glam::camera::rh::view::look_at_mat4(
            eye,
            glam::Vec3::new(5.0, 2.5, 5.0),
            glam::Vec3::Z,
        );
        let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
        let length_after = |wb: &mut DesignWorkbench, doc: &mut Document, alt: bool| {
            let mut ctx = WorkbenchRuntimeContext::new(
                doc,
                eye.to_array(),
                [5.0, 2.5, 5.0],
                (0, 0, 800, 600),
            );
            ctx.view_proj = Some((proj * view).to_cols_array_2d());
            ctx.alt_down = alt;
            let handle = wb
                .handles(&ctx)
                .into_iter()
                .find(|h| h.label == "Length")
                .unwrap();
            let at = ctx.world_to_viewport(handle.position().to_array()).unwrap();
            let near = ctx.world_to_viewport([5.0, 2.5, 17.1]).unwrap();
            let press = E::MousePress {
                button: MouseButton::Left,
                viewport_pos: at,
            };
            assert!(wb.on_input(&press, None, &mut ctx).consumed);
            wb.on_input(&E::MouseMove { viewport_pos: near }, None, &mut ctx);
            let lit = wb.tab.snapped.is_some();
            let release = E::MouseRelease {
                button: MouseButton::Left,
                viewport_pos: near,
            };
            wb.on_input(&release, None, &mut ctx);
            let DesignFeature::Pad { length, .. } =
                DesignFeature::from_json(ctx.document.get_feature_data(id).unwrap()).unwrap()
            else {
                panic!()
            };
            (length, lit)
        };
        let (snapped, lit) = length_after(&mut wb, &mut doc, false);
        assert!((snapped - 17.3).abs() < 1e-4 && lit, "{snapped}");
        let (free, lit) = length_after(&mut wb, &mut doc, true);
        assert!((free - 17.3).abs() > 0.05 && !lit, "{free}");
    }

    /// With a pad's task open on two crossing circles, a click on the lens
    /// takes it alone and Ctrl+click adds a crescent; a click off the
    /// circles goes on to pick as any click.
    #[test]
    fn clicking_regions_in_the_view_chooses_what_the_pad_takes() {
        use core_document::{MouseButton, Workbench, WorkbenchInputEvent as E};
        use wb_sketch::sketch::Circle;
        let mut sketch = Sketch::new("venn");
        for x in [0.0, 10.0] {
            let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))));
            sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 10.0)));
        }
        let plane = sketch.plane;
        let regions = wb_sketch::profile::regions(&sketch).unwrap();
        let name_at = |x: f64| {
            regions
                .iter()
                .find(|r| (r.inside[0] - x).abs() < 3.0)
                .map(|r| r.name.clone())
                .expect("a region there")
        };
        let (left, lens) = (name_at(-5.0), name_at(5.0));
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch_id = doc
            .add_feature_in_body(
                SketchFeature::new(sketch, plane),
                "Sketch".into(),
                Some(body),
            )
            .unwrap();
        let id = doc
            .add_feature_in_body(pad(sketch_id, 2.0), "Pad".into(), Some(body))
            .unwrap();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);

        let eye = glam::Vec3::new(5.0, -40.0, 80.0);
        let view = glam::camera::rh::view::look_at_mat4(
            eye,
            glam::Vec3::new(5.0, 0.0, 0.0),
            glam::Vec3::Z,
        );
        let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
        let mut ctx = WorkbenchRuntimeContext::new(
            &mut doc,
            eye.to_array(),
            [5.0, 0.0, 0.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some((proj * view).to_cols_array_2d());
        let press = |ctx: &WorkbenchRuntimeContext, at: [f32; 3]| E::MousePress {
            button: MouseButton::Left,
            viewport_pos: ctx.world_to_viewport(at).unwrap(),
        };
        let taken = |ctx: &WorkbenchRuntimeContext| {
            DesignFeature::from_json(ctx.document.get_feature_data(id).unwrap())
                .unwrap()
                .regions()
                .to_vec()
        };

        let event = press(&ctx, [5.0, 6.0, 0.0]);
        assert!(wb.on_input(&event, None, &mut ctx).consumed);
        assert_eq!(taken(&ctx), vec![lens.clone()], "the lens alone");
        ctx.ctrl_down = true;
        let event = press(&ctx, [-6.0, 0.0, 0.0]);
        assert!(wb.on_input(&event, None, &mut ctx).consumed);
        let mut both = taken(&ctx);
        both.sort();
        let mut expected = vec![lens, left];
        expected.sort();
        assert_eq!(both, expected, "the crescent added");
        ctx.ctrl_down = false;
        let event = press(&ctx, [30.0, 0.0, 0.0]);
        assert!(
            !wb.on_input(&event, None, &mut ctx).consumed,
            "off the circles"
        );
    }

    /// Cancel puts back everything the task changed: the data and what it
    /// depends on, the formulas, the name and the sketches it hid or showed.
    #[test]
    fn cancel_restores_formulas_name_and_sketch_visibility() {
        let Scene {
            mut doc,
            first,
            second,
            pad: id,
            ..
        } = scene();
        doc.set_feature_visible(second, true);
        doc.set_feature_formula(id, "/Pad/length", Some("4 mm".into()))
            .unwrap();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            doc.rename_feature(id, "Renamed");
            doc.set_feature_formula(id, "/Pad/length", None).unwrap();
            doc.set_feature_formula(id, "/Pad/taper_deg", Some("2 deg".into()))
                .unwrap();
            edit(&mut wb, doc, id, pad(second, 7.0));
            assert!(doc.get_feature_meta(first).unwrap().visible);
            assert!(!doc.get_feature_meta(second).unwrap().visible);
            let outcome = frame(&mut wb, &panel, doc, Some(id), CANCEL, vec![]);
            assert_eq!(outcome, TaskOutcome::Cancelled);
        });
    }

    /// A whole-body mirror given an original in its task and cancelled
    /// reads nothing again: what it depends on follows its data back.
    #[test]
    fn cancel_leaves_a_whole_body_mirror_reading_nothing() {
        let Scene { mut doc, pad, .. } = scene();
        let body = doc.get_feature_meta(pad).unwrap().body;
        let mirror = |originals| DesignFeature::Mirrored {
            originals,
            plane: crate::feature::MirrorPlane::YZ,
            refine: false,
            boolean_tolerance: None,
            skipped: Vec::new(),
        };
        let id = doc
            .add_feature_in_body(mirror(Vec::new()), "Mirrored".into(), body)
            .unwrap();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        open(&mut wb, &panel, &mut doc, id);
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            edit(&mut wb, doc, id, mirror(vec![pad]));
            assert_eq!(doc.feature_tree().dependencies(id), vec![pad]);
            let outcome = frame(&mut wb, &panel, doc, Some(id), CANCEL, vec![]);
            assert_eq!(outcome, TaskOutcome::Cancelled);
        });
        assert!(doc.feature_tree().dependencies(id).is_empty());
    }

    /// A pad the tool makes from an outline that stops just short closes
    /// it, and says so; Cancel takes the pad away and puts the gap back.
    #[test]
    fn a_tool_made_pad_closes_its_outline_and_cancel_opens_it_again() {
        use wb_sketch::sketch::{GeometryElement, Line, Point, Vec2D};
        let mut gapped = rect_sketch();
        let start = gapped
            .sketch
            .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.05))));
        // The last side, from the top left corner down, ends 0.05 short.
        let corners: Vec<_> = gapped
            .sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) if p.id != start => Some(p.id),
                _ => None,
            })
            .collect();
        gapped.sketch.geometry.retain(|g| !matches!(g, GeometryElement::Line(l) if l.end == corners[0] && l.start == corners[3]));
        gapped
            .sketch
            .add_geometry(GeometryElement::Line(Line::new(corners[3], start)));
        assert!(!wb_sketch::profile::loose_ends(&gapped.sketch).is_empty());

        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch = doc
            .add_feature_in_body(gapped, "Sketch".into(), Some(body))
            .unwrap();
        let mut wb = DesignWorkbench::default();
        let panel = panel();
        let closed = |doc: &Document| {
            let data = doc.get_feature_data(sketch).unwrap();
            wb_sketch::profile::loose_ends(&SketchFeature::from_json(data).unwrap().sketch)
                .is_empty()
        };
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            let made = {
                let mut ctx =
                    WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
                ctx.active_document_object = Some(sketch);
                wb.insert_feature(&mut ctx, "design.pad");
                ctx.active_document_object.unwrap()
            };
            assert!(closed(doc), "the tool closed the outline");
            frame(&mut wb, &panel, doc, Some(made), OPEN, vec![]);
            assert!(
                wb.tab.task.as_ref().unwrap().repaired.is_some(),
                "the task says what it joined"
            );
            let outcome = frame(&mut wb, &panel, doc, Some(made), CANCEL, vec![]);
            assert_eq!(outcome, TaskOutcome::Cancelled);
        });
        assert!(!closed(&doc), "the gap is back");
    }

    /// With the live preview off an edit waits, and however the task is
    /// accepted (OK, deselecting, selecting another feature) it rebuilds.
    #[test]
    fn every_accept_rebuilds_with_the_live_preview_off() {
        for how in ["OK", "deselected", "another selected"] {
            let Scene {
                mut doc,
                first,
                pad: id,
                other_pad,
                ..
            } = scene();
            let mut wb = DesignWorkbench::default();
            wb.options.update_while_editing = false;
            let panel = panel();
            open(&mut wb, &panel, &mut doc, id);
            edit(&mut wb, &mut doc, id, pad(first, 12.0));
            assert!(!doc.get_feature_meta(id).unwrap().dirty, "{how}: waits");

            let (active, request) = match how {
                "OK" => (Some(id), OK),
                "deselected" => (None, OPEN),
                _ => (Some(other_pad), OPEN),
            };
            let outcome = frame(&mut wb, &panel, &mut doc, active, request, vec![]);
            assert!(
                matches!(outcome, TaskOutcome::Accepted { .. }),
                "{how}: {outcome:?}"
            );
            assert!(doc.get_feature_meta(id).unwrap().dirty, "{how}: rebuilds");
        }
    }

    /// The Name field keeps what is typed; leaving it with Enter or
    /// accepting the task applies it, Esc drops it.
    #[test]
    fn a_typed_name_applies_on_enter_or_accept_and_esc_drops_it() {
        let typed = |finish: Vec<egui::Event>, request: TaskRequest| {
            let Scene {
                mut doc, pad: id, ..
            } = scene();
            let mut wb = DesignWorkbench::default();
            let panel = panel();
            open(&mut wb, &panel, &mut doc, id);
            panel.memory_mut(|m| m.request_focus(name_field_id()));
            for text in ["X", "Y"] {
                let events = vec![egui::Event::Text(text.into())];
                frame(&mut wb, &panel, &mut doc, Some(id), OPEN, events);
            }
            assert_eq!(
                doc.get_feature_meta(id).unwrap().name,
                "Pad",
                "nothing applies while typing"
            );
            frame(&mut wb, &panel, &mut doc, Some(id), request, finish);
            doc.get_feature_meta(id).unwrap().name.clone()
        };
        assert_eq!(typed(vec![key(egui::Key::Enter)], OPEN), "PadXY");
        assert_eq!(typed(vec![key(egui::Key::Escape)], OPEN), "Pad");
        assert_eq!(typed(vec![], OK), "PadXY");
    }

    /// A feature failing on a converted solid still in facets says to
    /// refine it: cancel and refine when it is the body's first feature,
    /// clear the features first otherwise; any other body gets no hint.
    #[test]
    fn a_failure_on_a_faceted_body_says_to_refine_it() {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Part".into()));
        let geometry = |faceted: bool| core_document::ImportedGeometry {
            health: Some(kernel_api::ShapeHealth {
                faceted,
                ..Default::default()
            }),
            ..bare_geometry(Some(uuid::Uuid::new_v4()))
        };
        let sketch = doc
            .add_feature_in_body(rect_sketch(), "Sketch".into(), Some(body))
            .unwrap();
        let first = doc
            .add_feature_in_body(pad(sketch, 10.0), "Pad".into(), Some(body))
            .unwrap();

        doc.set_imported_geometry(body, geometry(false));
        assert_eq!(faceted_hint(&doc, body, first), None);

        doc.set_imported_geometry(body, geometry(true));
        let hint = faceted_hint(&doc, body, first).expect("a hint");
        assert!(hint.contains("Cancel"), "{hint}");

        let second = doc
            .add_feature_in_body(pad(sketch, 5.0), "Pad001".into(), Some(body))
            .unwrap();
        let hint = faceted_hint(&doc, body, second).expect("a hint");
        assert!(hint.contains("delete them"), "{hint}");
    }
}
