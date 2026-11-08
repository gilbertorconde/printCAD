//! Dimension values edited against an isolated sketch, including its solved geometry.

use crate::operations::SolveOutcome;
use crate::sketch::{self, ConstraintKind};
use crate::sketch_draft::{SketchDraft, SketchSession};
use crate::{DimEdit, SaveAs, SketchFeature, SketchWorkbench};
use core_document::{Recorded, WorkbenchFeature, WorkbenchRuntimeContext};
use uuid::Uuid;

pub(crate) struct DimensionSession {
    pub draft: SketchDraft,
    records: Vec<Recorded>,
    original: crate::sketch::Constraint,
    initial_text: String,
    input: Option<(String, bool, Option<SaveAs>)>,
}

impl SketchWorkbench {
    /// Open the value editor on dimension `id`, just added: by its label,
    /// showing what it measured.
    pub(crate) fn open_new_dimension(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        id: Uuid,
    ) {
        let Some(c) = feature.sketch.constraints.iter().find(|c| c.id == id) else {
            return;
        };
        let proj = crate::overlay::SketchProjector::new(ctx, feature.plane);
        let pos = self
            .glyphs(ctx, &feature.sketch, &proj)
            .iter()
            .find(|g| g.constraint == id)
            .map(|g| g.pos)
            .unwrap_or_else(|| {
                let (_, _, w, h) = ctx.viewport;
                [w as f32 / 2.0, h as f32 / 2.0]
            });
        let text = sketch::dimension_value(&c.kind)
            .map(crate::glyphs::fmt_num)
            .unwrap_or_default();
        let mut edit = DimEdit::new(id, pos, text, c.driving);
        edit.new = true;
        self.dim_edit = Some(edit);
    }

    /// The pending dimension task, if one is open.
    pub fn pending_dim_edit(&self) -> Option<&DimEdit> {
        self.dim_edit.as_ref()
    }

    /// Mutable access to the pending dimension edit (text/driving fields).
    pub fn pending_dim_edit_mut(&mut self) -> Option<&mut DimEdit> {
        self.dim_edit.as_mut()
    }

    /// Close the pending dimension edit without applying it.
    pub fn cancel_dim_edit(&mut self) {
        if let Some(edit) = self.dim_edit.take() {
            self.selected_constraints.remove(&edit.constraint);
            self.selected.extend(edit.restore);
        }
        if self.session.dimension().is_some() {
            self.session = SketchSession::None;
        }
        self.last_diagnosis = None;
    }

    /// Accept the pending dimension task's solved candidate.
    pub fn commit_dim_edit(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.accept_dimension(ctx);
    }

    /// Which of `distance_x`, `distance_y` and `dimension` the pending new
    /// dimension of kind `kind` is, when it can be made again as the others:
    /// it is new, and what it was made of (all still in the sketch) is a
    /// line, two lines or two points.
    pub(crate) fn axis_choice(
        &self,
        ctx: &WorkbenchRuntimeContext,
        kind: &ConstraintKind,
    ) -> Option<&'static str> {
        let edit = self.dim_edit.as_ref().filter(|e| e.new)?;
        let feature = self.get_active_sketch(ctx)?;
        if edit.restore.is_empty()
            || edit
                .restore
                .iter()
                .any(|id| feature.sketch.get_geometry(*id).is_none())
        {
            return None;
        }
        let items = edit.restore.iter().copied().collect();
        let shape =
            crate::constrain::SelectionShape::picked_in(&feature.sketch, &items, &edit.restore);
        if !crate::constrain::takes_every_linear_dimension(&shape) {
            return None;
        }
        Some(match kind {
            ConstraintKind::DistanceX { .. } => "distance_x",
            ConstraintKind::DistanceY { .. } => "distance_y",
            _ => "dimension",
        })
    }

    /// Drop the pending new dimension and make the one `tool`
    /// (`distance_x`, `distance_y` or `dimension`) gives on the same items,
    /// as though that tool had been picked on them instead.
    pub fn remake_new_dimension(&mut self, ctx: &mut WorkbenchRuntimeContext, tool: &str) {
        assert!(
            self.dim_edit.as_ref().is_some_and(|e| e.new),
            "only a new dimension is made again"
        );
        self.cancel_dim_edit();
        self.apply_constraint_tool(ctx, &format!("sketch.constrain.{tool}"), tool);
    }

    /// Put what `text` comes to in a variable (`save`) and bind dimension
    /// `constraint` to it: a typed value becomes the variable's value with
    /// its unit, a formula the variable's formula.
    #[cfg(test)]
    pub(crate) fn save_dimension_as(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
        save: &SaveAs,
    ) {
        let feature = self.get_active_sketch(ctx).unwrap();
        let mut edit = DimEdit::new(constraint, [0.; 2], text.into(), true);
        edit.save_as = Some(save.clone());
        self.open_dimension(ctx, feature, edit, Vec::new());
        self.accept_dimension(ctx);
    }

    fn save_dimension_variable(
        ctx: &mut WorkbenchRuntimeContext,
        kind: &sketch::ConstraintKind,
        text: &str,
        save: &SaveAs,
    ) -> Result<(String, f64), String> {
        let dim = crate::params::formula_dim(kind);
        let text = text.trim();
        let value = ctx.document.evaluate_formula(text, Some(dim))?.value;
        let formula = if core_document::expr::is_constant(text) {
            core_document::expr::Quantity::new(value, dim).display(ctx.document.display_unit(), 6)
        } else {
            text.to_string()
        };
        let name = save.name.trim();
        if !core_document::expr::is_valid_name(name) {
            return Err("Give the variable a valid name".into());
        }
        let (set, set_name) = match save.set {
            Some(set) => match ctx.document.get_feature_meta(set) {
                Some(node) => (set, node.name.clone()),
                None => return Err("The variable set is unavailable".into()),
            },
            None => {
                let set_name = ctx.document.unused_object_name("Variables");
                match ctx.document.add_variable_set(&set_name) {
                    Ok(set) => {
                        ctx.record(
                            "var.new",
                            crate::commands::args(serde_json::json!({ "name": set_name })),
                            serde_json::json!(set.0.to_string()),
                        );
                        (set, set_name)
                    }
                    Err(why) => {
                        return Err(why);
                    }
                }
            }
        };
        ctx.document.set_variable(set, name, &formula, None)?;
        ctx.record(
            "var.set",
            crate::commands::args(serde_json::json!({
                "set": set_name,
                "name": name,
                "formula": formula,
            })),
            serde_json::Value::Null,
        );
        ctx.log_info(format!("{set_name}.{name} = {formula}"));
        let reference = format!(
            "{}.{}",
            core_document::expr::quote_name(&set_name),
            core_document::expr::quote_name(name)
        );
        Ok((reference, value))
    }

    /// Set a dimension from typed text: a value (with a unit if typed,
    /// `1 in`) that stands as it is, or a formula that reads other values
    /// and sets it from now on.
    #[cfg(test)]
    pub(crate) fn set_dimension(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
        driving: bool,
    ) {
        let feature = self.get_active_sketch(ctx).unwrap();
        self.open_dimension(
            ctx,
            feature,
            DimEdit::new(constraint, [0.; 2], text.into(), driving),
            Vec::new(),
        );
        self.accept_dimension(ctx);
    }

    pub(crate) fn open_dimension(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        seed: SketchFeature,
        edit: DimEdit,
        records: Vec<Recorded>,
    ) {
        // A task separates the viewport clicks on either side of it.
        self.last_geometry_click = None;
        self.last_empty_click = None;
        self.last_glyph_click = None;
        self.session = SketchSession::Dimension(DimensionSession {
            initial_text: crate::glyphs::fmt_num(
                sketch::dimension_value(
                    &seed
                        .sketch
                        .constraints
                        .iter()
                        .find(|c| c.id == edit.constraint)
                        .unwrap()
                        .kind,
                )
                .unwrap(),
            ),
            original: seed
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == edit.constraint)
                .unwrap()
                .clone(),
            draft: SketchDraft::new(seed, ctx),
            records,
            input: None,
        });
        self.dim_edit = Some(edit);
        self.recalculate_dimension(ctx);
    }

    pub(crate) fn invalidate_dimension(&mut self, ctx: &WorkbenchRuntimeContext) -> bool {
        if self
            .session
            .dimension()
            .is_some_and(|s| !s.draft.is_current(ctx))
        {
            self.cancel_dim_edit();
            return true;
        }
        false
    }

    pub(crate) fn dimension_kind(&self) -> Option<&ConstraintKind> {
        let edit = self.dim_edit.as_ref()?;
        self.session
            .dimension()?
            .draft
            .base
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)
            .map(|c| &c.kind)
    }

    pub(crate) fn recalculate_dimension(&mut self, ctx: &WorkbenchRuntimeContext) {
        if self.invalidate_dimension(ctx) {
            return;
        }
        let (Some(edit), Some(session)) = (&self.dim_edit, self.session.dimension()) else {
            return;
        };
        let input = (edit.text.clone(), edit.driving, edit.save_as.clone());
        if session.input.as_ref() == Some(&input) {
            return;
        }
        let result = self.calculate_dimension(ctx);
        let session = self.session.dimension_mut().unwrap();
        session.input = Some(input);
        session.draft.working = result;
        self.last_diagnosis = None;
    }

    fn calculate_dimension(&self, ctx: &WorkbenchRuntimeContext) -> Result<SketchFeature, String> {
        let edit = self.dim_edit.as_ref().unwrap();
        let mut feature = self.session.dimension().unwrap().draft.base.clone();
        let c = feature
            .sketch
            .constraints
            .iter_mut()
            .find(|c| c.id == edit.constraint)
            .unwrap();
        let dim = crate::params::formula_dim(&c.kind);
        let value = ctx
            .document
            .evaluate_formula(edit.text.trim(), Some(dim))?
            .value;
        // Opening a rounded label must not round the stored measurement.
        let value = if edit.text == self.session.dimension().unwrap().initial_text
            && core_document::expr::is_constant(&edit.text)
        {
            sketch::dimension_value(&c.kind).unwrap()
        } else {
            value
        };
        if !value.is_finite() {
            return Err("Enter a finite dimension value".into());
        }
        c.kind = sketch::with_dimension_value(&c.kind, value);
        c.driving =
            edit.driving || edit.save_as.is_some() || !core_document::expr::is_constant(&edit.text);
        let kind = c.kind.clone();

        // Formula and variable edits use a disposable document. Only its sketch
        // is shown; the confirmed document keeps its metadata and history.
        let mut document = ctx.document.clone();
        let mut scratch = WorkbenchRuntimeContext::new(
            &mut document,
            ctx.camera_position,
            ctx.camera_target,
            ctx.viewport,
        );
        let id = self.active_sketch_id.unwrap();
        scratch
            .document
            .update_feature_data(id, feature.to_json())
            .unwrap();
        let formula = if let Some(save) = &edit.save_as {
            if !core_document::expr::is_valid_name(save.name.trim()) {
                return Err("Give the variable a valid name".into());
            }
            Some(Self::save_dimension_variable(&mut scratch, &kind, &edit.text, save)?.0)
        } else {
            (!core_document::expr::is_constant(&edit.text)).then(|| edit.text.trim().to_string())
        };
        scratch
            .document
            .set_feature_formula(id, edit.constraint.to_string(), formula)
            .map_err(|e| e.to_string())?;
        let evaluation =
            core_document::evaluate::evaluate_with_retained_parameters(scratch.document, &|node| {
                (node.workbench_id == SketchFeature::workbench_id())
                    .then(|| crate::params::parameters(node))
            });
        if let Some(slots) = evaluation.slots.get(&id) {
            for slot in slots {
                slot.result.as_ref().map_err(Clone::clone)?;
            }
        }
        if let Some(data) = evaluation.data.get(&id) {
            let evaluated = SketchFeature::from_json(data).unwrap();
            feature.sketch.constraints = evaluated.sketch.constraints;
        }
        if matches!(
            crate::solver::solve(&mut feature.sketch),
            SolveOutcome::NotConverged { .. }
        ) {
            return Err("Constraints cannot be solved for this dimension".into());
        }
        Ok(feature)
    }

    pub(crate) fn switch_edited_dimension(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        diameter: bool,
    ) {
        if self.invalidate_dimension(ctx) {
            return;
        }
        let edit = self.dim_edit.as_mut().unwrap();
        let session = self.session.dimension_mut().unwrap();
        let unchanged = edit.text == session.initial_text;
        let scale = crate::dimension_ops::switch_circle_dimension(
            &mut session.draft.base.sketch,
            edit.constraint,
            diameter,
        )
        .unwrap();
        edit.text = match edit.text.trim().parse::<f64>() {
            Ok(v) => (v * scale).to_string(),
            Err(_) => crate::dimension_ops::scaled_formula(&edit.text, scale),
        };
        if unchanged {
            session.initial_text = edit.text.clone();
        }
        session.input = None;
        self.recalculate_dimension(ctx);
    }

    pub(crate) fn switch_edited_arc_dimension(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        angle: bool,
    ) {
        if self.invalidate_dimension(ctx) {
            return;
        }
        let session = self.session.dimension_mut().unwrap();
        let Ok(feature) = &session.draft.working else {
            return;
        };
        let mut feature = feature.clone();
        let edit = self.dim_edit.as_mut().unwrap();
        let current = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)
            .unwrap();
        let id = self.active_sketch_id.unwrap();
        // A quantity switch measures the current solved geometry. Keep the
        // accepted recording's solve sequence identical to the shown draft's.
        if sketch::dimension_value(&session.original.kind) != sketch::dimension_value(&current.kind)
            || session.original.driving != current.driving
        {
            session.records.push(Recorded {
                id: "sketch.set_value".into(),
                args: crate::commands::args(serde_json::json!({
                    "sketch": id.0.to_string(), "constraint": edit.constraint.to_string(),
                    "value": sketch::dimension_value(&current.kind).unwrap(), "driving": current.driving,
                })),
                result: serde_json::Value::Null,
            });
        }
        // Conversion measures the shown arc. Formula removal stays inside the
        // disposable document until Apply accepts the new physical quantity.
        let mut document = ctx.document.clone();
        assert!(crate::dimension_ops::switch_arc_dimension(
            &mut document,
            self.active_sketch_id.unwrap(),
            &mut feature.sketch,
            edit.constraint,
            angle,
        ));
        let constraint = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)
            .unwrap();
        session.records.push(Recorded {
            id: "sketch.set_constraint".into(),
            args: crate::commands::args(serde_json::json!({
                "sketch": id.0.to_string(), "items": [edit.constraint.to_string()],
                "as": if angle { "angle" } else { "length" },
            })),
            result: serde_json::Value::Null,
        });
        session.original = constraint.clone();
        edit.text = sketch::dimension_value(&constraint.kind)
            .unwrap()
            .to_string();
        edit.save_as = None;
        session.initial_text = edit.text.clone();
        session.draft.base = feature;
        session.input = None;
        self.recalculate_dimension(ctx);
    }

    pub(crate) fn accept_dimension(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.recalculate_dimension(ctx);
        let Some(session) = self.session.dimension() else {
            return;
        };
        let Ok(feature) = &session.draft.working else {
            return;
        };
        let feature = feature.clone();
        let edit = self.dim_edit.as_ref().unwrap().clone();
        let id = self.active_sketch_id.unwrap();
        let records = session.records.clone();
        let original = session.original.clone();
        for record in records {
            ctx.record(record.id, record.args, record.result);
        }
        let kind = self.dimension_kind().unwrap();
        if std::mem::discriminant(kind) != std::mem::discriminant(&original.kind) {
            ctx.record(
                "sketch.set_constraint",
                crate::commands::args(serde_json::json!({
                    "sketch": id.0.to_string(), "items": [edit.constraint.to_string()],
                    "as": match kind {
                        ConstraintKind::Diameter { .. } => "diameter",
                        ConstraintKind::Radius { .. } => "radius",
                        ConstraintKind::ArcAngle { .. } => "angle",
                        ConstraintKind::ArcLength { .. } => "length",
                        _ => unreachable!("only circle and arc dimensions switch kind"),
                    },
                })),
                serde_json::Value::Null,
            );
        }
        let formula = if let Some(save) = &edit.save_as {
            Some(
                Self::save_dimension_variable(
                    ctx,
                    self.dimension_kind().unwrap(),
                    &edit.text,
                    save,
                )
                .expect("validated variable")
                .0,
            )
        } else {
            (!core_document::expr::is_constant(&edit.text)).then(|| edit.text.trim().to_string())
        };
        let changed_formula = ctx
            .document
            .feature_formula(id, &edit.constraint.to_string())
            != formula.as_deref();
        ctx.document
            .set_feature_formula(id, edit.constraint.to_string(), formula.clone())
            .unwrap();
        // Clearing a binding is recorded too: replay must not keep the old formula.
        if changed_formula {
            ctx.record("doc.set_formula", crate::commands::args(serde_json::json!({
            "id": id.0.to_string(), "parameter": edit.constraint.to_string(), "formula": formula,
        })), serde_json::Value::Null);
        }
        let constraint = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)
            .unwrap();
        if formula.is_some()
            || (sketch::dimension_value(&original.kind)
                != sketch::dimension_value(&constraint.kind)
                || original.driving != constraint.driving)
        {
            ctx.record("sketch.set_value", crate::commands::args(serde_json::json!({
            "sketch": id.0.to_string(), "constraint": edit.constraint.to_string(),
            "value": sketch::dimension_value(&constraint.kind).unwrap(), "driving": constraint.driving,
        })), serde_json::Value::Null);
        }
        assert!(self.store_sketch(ctx, feature));
        self.dim_edit = None;
        self.cancel_dim_edit();
        self.finish_sketch_task(ctx, "Dimension");
    }
}
