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
                    crate::commands::record_task(self, ctx, &task);
                    TaskOutcome::Accepted {
                        label: "Edit feature".to_string(),
                    }
                }
                None => TaskOutcome::Open,
            };
        };
        // Selecting another feature accepts the open task implicitly.
        if self.task.as_ref().is_some_and(|t| t.feature != target_id) {
            let label = self.task_label(ctx);
            if let Some(task) = self.task.take() {
                crate::commands::record_task(self, ctx, &task);
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
            let label = self.task_label(ctx);
            if let Some(task) = self.task.take() {
                crate::commands::record_task(self, ctx, &task);
            }
            ctx.active_document_object = None;
            // With the live preview off, the accepted edit is what
            // rebuilds.
            if !self.options.update_while_editing {
                ctx.document.mark_feature_dirty(target_id);
            }
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

    fn name_row(
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        feature_id: FeatureId,
        node_name: &str,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_2;
            editors::label_cell(ui, "Name");
            let mut edited = node_name.to_owned();
            let resp = ui.add(
                egui::TextEdit::singleline(&mut edited)
                    .desired_width(160.0)
                    .font(sans(FONT_SM)),
            );
            if resp.lost_focus() && edited != node_name && !edited.trim().is_empty() {
                ctx.document.rename_feature(feature_id, edited);
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
        Self::name_row(ui, ctx, feature_id, &node.name);
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
            let deps_after = feature.dependencies();
            if ctx
                .document
                .update_feature_data(feature_id, feature.to_json())
                .is_ok()
            {
                if deps_before != deps_after {
                    ctx.document
                        .set_feature_dependencies(feature_id, deps_after);
                }
                if self.options.update_while_editing {
                    ctx.document.mark_feature_dirty(feature_id);
                }
            }
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
                    "Preview updates live. Press Enter to accept, Esc to cancel.",
                );
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
        Self::name_row(ui, ctx, datum_id, &node.name);
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
            let _ = ctx.document.update_feature_data(datum_id, datum.to_json());
            // Sketches attached to this datum re-derive their plane from it
            // on their next edit; solids are unaffected.
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
