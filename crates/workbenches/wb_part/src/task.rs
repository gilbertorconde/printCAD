//! The Part Design task: one feature (or datum) open for editing in the
//! task panel. Edits apply live; Cancel restores the payload captured when
//! the task opened, or deletes a feature the tool itself just created.

use core_document::{
    FeatureId, TaskInfo, TaskOutcome, TaskRequest, WorkbenchFeature, WorkbenchRuntimeContext,
};
use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, Note, destructive_button, note_card};
use ui_kit::{mono, sans, sans_semibold};

use crate::build::sketch_plane_description;
use crate::feature::PartFeature;
use crate::{PartDesignWorkbench, editors};

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
    /// The feature payload when the task opened; Cancel writes it back.
    pub snapshot: serde_json::Value,
    /// The tool created this feature just now: Cancel deletes it.
    pub created_by_tool: bool,
    /// Sketches the tool hid; shown again when the feature is cancelled.
    pub hidden_sketches: Vec<FeatureId>,
    /// The tool that made it and the body it was made for, when a tool
    /// did: OK records the command that makes it.
    pub made_by: Option<(String, core_document::BodyId)>,
    /// The feature's name and formulas when the task opened, which Cancel
    /// puts back with the data.
    pub name: String,
    pub formulas: std::collections::BTreeMap<String, String>,
    /// Sketches the task showed or hid as the profile changed, with what
    /// each was before: Cancel puts them back.
    pub visibility: Vec<(FeatureId, bool)>,
    /// The name as typed in the Name field and not yet applied: it goes
    /// in when the field is left or the task accepted, and Esc drops it.
    pub name_draft: Option<String>,
}

/// The Name field of the task for `feature`.
fn name_field_id(feature: FeatureId) -> egui::Id {
    egui::Id::new(("part_task_name", feature))
}

/// The kind of task node `id` is, if it is one this workbench edits.
fn task_kind(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Option<TaskKind> {
    let node = ctx.document.get_feature_meta(id)?;
    match node.workbench_id.as_str() {
        "wb.part" => Some(TaskKind::Part),
        "core.datum" => Some(TaskKind::Datum),
        _ => None,
    }
}

impl PartDesignWorkbench {
    /// The task the active document object calls for.
    pub(crate) fn task_info(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        let id = ctx.active_document_object?;
        let kind = task_kind(ctx, id)?;
        let node = ctx.document.get_feature_meta(id)?;
        let icon = match kind {
            TaskKind::Part => PartFeature::from_json(&node.data)
                .map(|f| f.icon())
                .unwrap_or("tree-feature"),
            TaskKind::Datum => core_document::DatumFeature::from_json(&node.data)
                .map(|d| crate::datum_icon(&d))
                .unwrap_or("datum-plane"),
        };
        Some(TaskInfo {
            title: node.name.clone(),
            icon,
            confirmable: true,
            stepwise: false,
        })
    }

    /// Open the task for `id`, capturing its payload for Cancel.
    fn open_task(&mut self, ctx: &WorkbenchRuntimeContext, id: FeatureId, kind: TaskKind) {
        let Some(node) = ctx.document.get_feature_meta(id) else {
            return;
        };
        let created = self
            .pending_task_from_tool
            .take()
            .filter(|made| made.feature == id);
        self.task = Some(TaskState {
            feature: id,
            kind,
            snapshot: node.data.clone(),
            created_by_tool: created.is_some(),
            hidden_sketches: created
                .as_ref()
                .map(|m| m.hidden.clone())
                .unwrap_or_default(),
            made_by: created.map(|m| (m.tool, m.body)),
            name: node.name.clone(),
            formulas: node.formulas.clone(),
            visibility: Vec::new(),
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
        let target = ctx
            .active_document_object
            .and_then(|id| task_kind(ctx, id).map(|kind| (id, kind)));
        let Some((target_id, target_kind)) = target else {
            // Deselected: the edits so far stay.
            return match self.task.take() {
                Some(task) => {
                    Self::apply_name_draft(ctx, &task);
                    crate::commands::record_task(self, ctx, &task);
                    self.rebuild_accepted(ctx, task.feature);
                    TaskOutcome::Accepted {
                        label: "Edit feature".to_string(),
                    }
                }
                None => TaskOutcome::Open,
            };
        };
        // Selecting another feature accepts the open task implicitly.
        if self.task.as_ref().is_some_and(|t| t.feature != target_id) {
            if let Some(task) = &self.task {
                Self::apply_name_draft(ctx, task);
            }
            let label = self.task_label(ctx);
            if let Some(task) = self.task.take() {
                crate::commands::record_task(self, ctx, &task);
                self.rebuild_accepted(ctx, task.feature);
            }
            self.open_task(ctx, target_id, target_kind.clone());
            return TaskOutcome::Accepted { label };
        }
        if self.task.is_none() {
            self.open_task(ctx, target_id, target_kind);
        }

        if request.cancel {
            return self.cancel_task(ctx);
        }
        if request.accept {
            if let Some(task) = &self.task {
                Self::apply_name_draft(ctx, task);
            }
            let label = self.task_label(ctx);
            if let Some(task) = self.task.take() {
                crate::commands::record_task(self, ctx, &task);
            }
            ctx.active_document_object = None;
            self.rebuild_accepted(ctx, target_id);
            return TaskOutcome::Accepted { label };
        }

        let Some(node) = ctx.document.get_feature_meta(target_id).cloned() else {
            self.task = None;
            return TaskOutcome::Cancelled;
        };
        let kind = self
            .task
            .as_ref()
            .map(|t| t.kind.clone())
            .unwrap_or(TaskKind::Part);
        ui.spacing_mut().item_spacing.y = SPACE_3;
        match kind {
            TaskKind::Part => self.part_card(ui, ctx, target_id, &node),
            TaskKind::Datum => self.datum_card(ui, ctx, target_id, &node),
        }
        TaskOutcome::Open
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
        let Some(task) = &self.task else {
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

    /// Revert the task: delete a tool-created feature, or write the
    /// snapshot back.
    fn cancel_task(&mut self, ctx: &mut WorkbenchRuntimeContext) -> TaskOutcome {
        let Some(task) = self.task.take() else {
            return TaskOutcome::Cancelled;
        };
        for (sketch, was) in &task.visibility {
            ctx.document.set_feature_visible(*sketch, *was);
        }
        let body = ctx
            .document
            .get_feature_meta(task.feature)
            .and_then(|n| n.body);
        if task.created_by_tool {
            if ctx.document.remove_feature(task.feature).is_ok() {
                for sketch in task.hidden_sketches {
                    ctx.document.set_feature_visible(sketch, true);
                }
                if let Some(body) = body {
                    crate::build::invalidate_body(ctx.document, body);
                }
                ctx.log_info("Feature discarded");
            }
        } else {
            // The restored payload's references become the dependencies
            // again.
            let deps = match task.kind {
                TaskKind::Part => PartFeature::from_json(&task.snapshot)
                    .map(|f| f.dependencies())
                    .unwrap_or_default(),
                TaskKind::Datum => Vec::new(),
            };
            if ctx
                .document
                .update_feature_data(task.feature, task.snapshot)
                .is_ok()
            {
                if !deps.is_empty() {
                    ctx.document.set_feature_dependencies(task.feature, deps);
                }
                // The formulas and the name as they were: a formula typed
                // in the task would otherwise write its value back.
                let now = ctx
                    .document
                    .get_feature_meta(task.feature)
                    .map(|n| (n.name.clone(), n.formulas.clone()));
                if let Some((name, formulas)) = now {
                    for key in formulas.keys().filter(|k| !task.formulas.contains_key(*k)) {
                        let _ = ctx
                            .document
                            .set_feature_formula(task.feature, key.clone(), None);
                    }
                    for (key, formula) in &task.formulas {
                        if formulas.get(key) != Some(formula) {
                            let _ = ctx.document.set_feature_formula(
                                task.feature,
                                key.clone(),
                                Some(formula.clone()),
                            );
                        }
                    }
                    if name != task.name {
                        ctx.document.rename_feature(task.feature, task.name.clone());
                    }
                }
                ctx.document.mark_feature_dirty(task.feature);
                ctx.log_info("Edit cancelled");
            }
        }
        ctx.active_document_object = None;
        TaskOutcome::Cancelled
    }

    fn card_header(ui: &mut egui::Ui, icon: &str, title: &str) {
        egui::Frame::new()
            .fill(BG2)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_2;
                    ui_kit::icon::draw(ui, icon, 18.0, ACCENT);
                    ui.label(
                        RichText::new(title)
                            .font(sans_semibold(FONT_MD))
                            .color(TEXT1),
                    );
                });
            });
    }

    /// The Name row. What is typed is held as the task's draft and
    /// applied when the field is left; Esc leaves it with the name as it
    /// was.
    fn name_row(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        node_name: &str,
    ) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_2;
            editors::label_cell(ui, "Name");
            let mut edited = task
                .name_draft
                .clone()
                .unwrap_or_else(|| node_name.to_owned());
            let resp = ui.add(
                egui::TextEdit::singleline(&mut edited)
                    .id(name_field_id(feature_id))
                    .desired_width(160.0)
                    .font(sans(FONT_SM)),
            );
            if resp.changed() {
                task.name_draft = Some(edited);
            }
            if resp.lost_focus() {
                let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if let Some(name) = task.name_draft.take()
                    && !escaped
                    && name != node_name
                    && !name.trim().is_empty()
                {
                    ctx.document.rename_feature(feature_id, name);
                }
            }
        });
    }

    fn part_card(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        node: &core_document::FeatureNode,
    ) {
        let Some(mut feature) = PartFeature::from_json(&node.data).ok() else {
            note_card(
                ui,
                Note::Error,
                Some("Unreadable feature"),
                "The stored payload does not parse.",
            );
            return;
        };
        let Some(body) = node.body else {
            return;
        };
        Self::card_header(
            ui,
            feature.icon(),
            &format!("{} parameters", feature.kind_label()),
        );
        self.name_row(ui, ctx, feature_id, &node.name);
        if let Some(sketch_id) = feature.sketch() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                editors::label_cell(ui, "Plane");
                let description = sketch_plane_description(ctx.document, sketch_id);
                ui.add(
                    egui::Label::new(RichText::new(&description).font(mono(FONT_XS)).color(TEXT3))
                        .truncate(),
                )
                .on_hover_text(description);
            });
        }

        let deps_before = feature.dependencies();
        let sketch_before = feature.sketch();
        let (changed, formula_edits) = {
            let shown: &WorkbenchRuntimeContext = ctx;
            let mut fx = editors::Formulas::of(shown.document, feature_id);
            let changed =
                editors::feature_editor(ui, shown, &mut fx, body, feature_id, &mut feature);
            (changed, std::mem::take(&mut fx.edits))
        };
        for (key, formula) in formula_edits {
            let _ = ctx.document.set_feature_formula(feature_id, key, formula);
        }
        if changed {
            self.apply_part_edit(ctx, feature_id, &deps_before, sketch_before, &feature);
        }

        ui.add_space(SPACE_1);
        match &node.error {
            Some(error) => {
                note_card(ui, Note::Error, Some("Recompute failed"), error);
            }
            None if node.dirty => {
                note_card(ui, Note::Info, None, "Rebuilding…");
            }
            None => {
                note_card(
                    ui,
                    Note::Info,
                    None,
                    if self.options.update_while_editing {
                        "Preview updates live. Press Enter to accept, Esc to cancel."
                    } else {
                        "The solid rebuilds when you accept. Press Enter to accept, Esc to cancel."
                    },
                );
            }
        }
    }

    /// Write an edit the panel made to the feature: its data, the
    /// dependencies it reads, and the sketches it consumes, remembering
    /// each sketch's visibility before the task first touched it.
    fn apply_part_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        deps_before: &[FeatureId],
        sketch_before: Option<FeatureId>,
        feature: &PartFeature,
    ) {
        let deps_after = feature.dependencies();
        if ctx
            .document
            .update_feature_data(feature_id, feature.to_json())
            .is_ok()
        {
            if deps_before != deps_after.as_slice() {
                ctx.document
                    .set_feature_dependencies(feature_id, deps_after);
            }
            // A new profile is consumed like the first: it hides, and
            // the one it replaced shows again.
            if let (Some(old), Some(new)) = (sketch_before, feature.sketch())
                && old != new
            {
                let swapped =
                    crate::build::swap_consumed_sketch(ctx.document, feature_id, old, new);
                if let Some(task) = self.task.as_mut() {
                    for (id, was) in swapped {
                        // The first change of a sketch is what it was.
                        if !task.visibility.iter().any(|(seen, _)| *seen == id) {
                            task.visibility.push((id, was));
                        }
                    }
                }
            }
            if self.options.update_while_editing {
                ctx.document.mark_feature_dirty(feature_id);
            }
        }
    }

    fn datum_card(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        datum_id: FeatureId,
        node: &core_document::FeatureNode,
    ) {
        let Some(mut datum) = core_document::DatumFeature::from_json(&node.data).ok() else {
            note_card(
                ui,
                Note::Error,
                Some("Unreadable datum"),
                "The stored payload does not parse.",
            );
            return;
        };
        let icon = crate::datum_icon(&datum);
        Self::card_header(ui, icon, &format!("{} parameters", datum.shape.label()));
        self.name_row(ui, ctx, datum_id, &node.name);
        let (changed, formula_edits) = {
            let shown: &WorkbenchRuntimeContext = ctx;
            let mut fx = editors::Formulas::of(shown.document, datum_id);
            let changed = editors::datum_editor(ui, shown, &mut fx, datum_id, &mut datum);
            (changed, std::mem::take(&mut fx.edits))
        };
        for (key, formula) in formula_edits {
            let _ = ctx.document.set_feature_formula(datum_id, key, formula);
        }
        if changed {
            // A sketch drawn on this datum follows it (`Workbench::derive`),
            // and what stands on the sketch rebuilds.
            let _ = ctx.document.update_feature_data(datum_id, datum.to_json());
        }
        ui.add_space(SPACE_1);
        Card::new().padding(SPACE_2).show(ui, |ui| {
            if destructive_button(ui, "Delete datum")
                .on_hover_text("Remove this datum")
                .clicked()
                && ctx.document.remove_feature(datum_id).is_ok()
            {
                self.task = None;
                ctx.active_document_object = None;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::ExtrudeMode;
    use core_document::Document;
    use wb_sketch::SketchFeature;
    use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};

    fn rect_sketch() -> SketchFeature {
        let mut sketch = Sketch::new("s");
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)].map(|(x, y)| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        });
        for i in 0..4 {
            sketch.add_geometry(GeometryElement::Line(Line::new(
                corners[i],
                corners[(i + 1) % 4],
            )));
        }
        let plane = sketch.plane;
        SketchFeature::new(sketch, plane)
    }

    fn pad(sketch: FeatureId, length: f32) -> PartFeature {
        PartFeature::Pad {
            refine: false,
            sketch,
            length,
            reversed: false,
            symmetric: false,
            mode: ExtrudeMode::Dimension,
            length2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
        }
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
        wb: &mut PartDesignWorkbench,
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
    fn edit(wb: &mut PartDesignWorkbench, doc: &mut Document, id: FeatureId, to: PartFeature) {
        let before = PartFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.apply_part_edit(&mut ctx, id, &before.dependencies(), before.sketch(), &to);
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
        let mut wb = PartDesignWorkbench::default();
        let panel = panel();
        frame(&mut wb, &panel, &mut doc, Some(id), OPEN, vec![]);

        doc.rename_feature(id, "Renamed");
        doc.set_feature_formula(id, "/Pad/length", None).unwrap();
        doc.set_feature_formula(id, "/Pad/taper_deg", Some("2 deg".into()))
            .unwrap();
        edit(&mut wb, &mut doc, id, pad(second, 7.0));
        assert!(doc.get_feature_meta(first).unwrap().visible);
        assert!(!doc.get_feature_meta(second).unwrap().visible);

        let outcome = frame(&mut wb, &panel, &mut doc, Some(id), CANCEL, vec![]);
        assert_eq!(outcome, TaskOutcome::Cancelled);
        let node = doc.get_feature_meta(id).unwrap();
        assert_eq!(node.name, "Pad");
        assert_eq!(
            node.formulas.clone().into_iter().collect::<Vec<_>>(),
            vec![("/Pad/length".to_string(), "4 mm".to_string())]
        );
        assert_eq!(
            PartFeature::from_json(&node.data).unwrap().sketch(),
            Some(first)
        );
        assert_eq!(doc.feature_tree().dependencies(id), vec![first]);
        assert!(!doc.get_feature_meta(first).unwrap().visible);
        assert!(doc.get_feature_meta(second).unwrap().visible);
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
            let mut wb = PartDesignWorkbench::default();
            wb.options.update_while_editing = false;
            let panel = panel();
            frame(&mut wb, &panel, &mut doc, Some(id), OPEN, vec![]);
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
            let mut wb = PartDesignWorkbench::default();
            let panel = panel();
            frame(&mut wb, &panel, &mut doc, Some(id), OPEN, vec![]);
            panel.memory_mut(|m| m.request_focus(name_field_id(id)));
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
}
