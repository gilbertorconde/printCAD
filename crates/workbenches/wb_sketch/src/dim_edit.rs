//! The dimension being edited in the view (`DimEdit`), and setting a
//! dimension's value, formula or variable.

use core_document::{FeatureId, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::overlay::SketchProjector;
use crate::sketch::Constraint;
use crate::{SketchWorkbench, ids_json};
use crate::{commands, glyphs, sketch};

impl SketchWorkbench {
    /// The pending in-viewport dimension edit, if one is open.
    pub fn pending_dim_edit(&self) -> Option<&DimEdit> {
        self.dim_edit.as_ref()
    }

    /// Mutable access to the pending dimension edit (text/driving fields).
    pub fn pending_dim_edit_mut(&mut self) -> Option<&mut DimEdit> {
        self.dim_edit.as_mut()
    }

    /// Set a dimension from typed text through its task: a value (with a
    /// unit if typed, `1 in`) that stands as it is, or a formula that reads
    /// other values and sets it from now on.
    #[cfg(test)]
    pub(crate) fn set_dimension(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
        driving: bool,
    ) {
        self.open_dimension_task(ctx, constraint, text);
        if let Some(edit) = self.dim_edit.as_mut() {
            edit.driving = driving;
        }
        self.commit_dim_edit(ctx);
    }

    /// Put what `text` comes to in a variable (`save`) and bind dimension
    /// `constraint` to it, through its task.
    #[cfg(test)]
    pub(crate) fn save_dimension_as(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
        save: &SaveAs,
    ) {
        self.open_dimension_task(ctx, constraint, text);
        if let Some(edit) = self.dim_edit.as_mut() {
            edit.save_as = Some(save.clone());
        }
        self.commit_dim_edit(ctx);
    }

    /// Replace the constraint at `idx` with `constraint`, then re-solve and
    /// persist. Backs the panel's inline dimension editing: the constraint
    /// is edited in place, no extra state is kept.
    pub fn update_constraint(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        idx: usize,
        constraint: Constraint,
    ) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(slot) = feature.sketch.constraints.get_mut(idx) else {
            return;
        };
        // A dimension's value or driving flag, as a recording says it; the
        // panel's other edits (a name, a label's place) are not modelling.
        let value = sketch::dimension_value(&constraint.kind);
        if (value != sketch::dimension_value(&slot.kind) || constraint.driving != slot.driving)
            && let (Some(value), Some(sketch_id)) = (value, self.active_sketch_id)
        {
            ctx.record(
                "sketch.set_value",
                commands::args(serde_json::json!({
                    "sketch": sketch_id.0.to_string(),
                    "constraint": constraint.id.to_string(),
                    "value": value,
                    "driving": constraint.driving,
                })),
                serde_json::Value::Null,
            );
        }
        // Its active and parked flags, as `sketch.set_constraint` sets them.
        if (constraint.active != slot.active || constraint.parked != slot.parked)
            && let Some(sketch_id) = self.active_sketch_id
        {
            let mut args = serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "items": ids_json(&[constraint.id]),
            });
            if constraint.active != slot.active {
                args["active"] = serde_json::json!(constraint.active);
            }
            if constraint.parked != slot.parked {
                args["parked"] = serde_json::json!(constraint.parked);
            }
            ctx.record(
                "sketch.set_constraint",
                commands::args(args),
                serde_json::Value::Null,
            );
        }
        // Formulas that read it by its old name read it by the new one.
        if let (Some(old), Some(new), Some(sketch_id)) = (
            slot.name.clone(),
            constraint.name.clone(),
            self.active_sketch_id,
        ) && old != new
            && !new.trim().is_empty()
            && let Some(sketch_name) = ctx
                .document
                .get_feature_meta(sketch_id)
                .map(|n| n.name.clone())
        {
            ctx.document.rewrite_formulas(|text| {
                core_document::expr::rename_property(text, &sketch_name, &old, &new)
            });
        }
        *slot = constraint;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
    }

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
        let proj = SketchProjector::new(ctx, feature.plane);
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
            .map(glyphs::fmt_num)
            .unwrap_or_default();
        let mut edit = DimEdit::new(id, pos, text, c.driving);
        edit.new = true;
        self.dim_edit = Some(edit);
        self.dim_draft = None;
    }
}

/// A dimension's value edited in its card (opened by double-clicking a
/// dimensional glyph, by a value typed into its row, or as a tool adds the
/// dimension).
#[derive(Debug, Clone)]
pub struct DimEdit {
    pub constraint: Uuid,
    /// Label position at open time, viewport px: where the card sits when
    /// the label is not drawn.
    pub screen_pos: [f32; 2],
    pub text: String,
    pub driving: bool,
    /// The dimension was just added by a tool: OK keeps it at the value
    /// typed, Cancel leaves it unmade.
    pub new: bool,
    /// The value goes into a variable the dimension then reads.
    pub save_as: Option<SaveAs>,
    /// What was selected when a tool added the dimension, selected again
    /// if Cancel leaves it unmade.
    pub restore: Vec<Uuid>,
    /// A new dimension's sketch and the calls that make it, stored and
    /// recorded only on OK.
    pub(crate) made: Option<MadeDimension>,
}

/// A dimension a tool made, waiting in its card.
#[derive(Debug, Clone)]
pub(crate) struct MadeDimension {
    /// The open sketch with the dimension added and solved.
    pub(crate) feature: SketchFeature,
    /// The calls that make it, recorded on OK.
    pub(crate) calls: Vec<(&'static str, core_document::CommandArgs, serde_json::Value)>,
}

impl DimEdit {
    /// An editor on `constraint` showing `text`.
    pub fn new(constraint: Uuid, screen_pos: [f32; 2], text: String, driving: bool) -> Self {
        Self {
            constraint,
            screen_pos,
            text,
            driving,
            new: false,
            save_as: None,
            restore: Vec::new(),
            made: None,
        }
    }
}

/// Where a dimension's value goes when it is saved as a variable.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveAs {
    /// The variable's name.
    pub name: String,
    /// The set it joins; `None` makes a set for it.
    pub set: Option<FeatureId>,
}

#[cfg(test)]
mod formulas {
    use super::*;
    use crate::sketch::{Constraint, ConstraintKind};
    use crate::test_kinds::all_points;
    use crate::*;
    use core_document::{Document, DocumentService};
    use sketch::{Line, Point};

    /// A horizontal line fixed at the origin, with a length dimension.
    fn scene() -> (Document, SketchWorkbench, FeatureId, Uuid) {
        let mut sketch = Sketch::new("s");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch
            .constraints
            .push(Constraint::new(ConstraintKind::FixedPoint {
                point: a,
                position: Vec2D::new(0.0, 0.0),
            }));
        sketch
            .constraints
            .push(Constraint::new(ConstraintKind::Horizontal {
                element: line,
            }));
        let length = Constraint::new(ConstraintKind::Length { line, length: 10.0 });
        let length_id = length.id;
        sketch.constraints.push(length);
        let plane = sketch.plane;
        let mut doc = Document::new("t");
        let sizes = doc.add_variable_set("Sizes").unwrap();
        doc.set_variable(sizes, "w", "30 mm", None).unwrap();
        let id = doc
            .add_feature(SketchFeature::new(sketch, plane), "Profile".into())
            .unwrap();
        let wb = SketchWorkbench {
            active_sketch_id: Some(id),
            ..Default::default()
        };
        (doc, wb, id, length_id)
    }

    fn ctx(doc: &mut Document) -> WorkbenchRuntimeContext<'_> {
        WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600))
    }

    /// The registry with the sketcher in it, as the app evaluates with.
    fn registry() -> DocumentService {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(SketchWorkbench::default()))
            .unwrap();
        registry
    }

    fn line_length(wb: &SketchWorkbench, doc: &mut Document) -> f64 {
        let feature = wb.get_active_sketch(&ctx(doc)).unwrap();
        let xs: Vec<f64> = all_points(&feature.sketch)
            .into_iter()
            .map(|p| p.position.x)
            .collect();
        xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min)
    }

    #[test]
    fn a_dimension_set_by_a_formula_solves_and_follows_its_variable_while_open() {
        let registry = registry();
        let (mut doc, mut wb, id, length) = scene();
        registry.evaluate(&mut doc);
        {
            let mut ctx = ctx(&mut doc);
            wb.set_dimension(&mut ctx, length, "Sizes.w / 2", true);
        }
        assert_eq!(
            doc.feature_formula(id, &length.to_string()),
            Some("Sizes.w / 2")
        );
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 15.0).abs() < 1e-3);

        // The variable changes: the open sketch reads its solved copy.
        let sizes = doc.object_named("Sizes").unwrap();
        doc.set_variable(sizes, "w", "50 mm", None).unwrap();
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 25.0).abs() < 1e-3);

        // A value typed with a unit takes the formula away.
        {
            let mut ctx = ctx(&mut doc);
            wb.set_dimension(&mut ctx, length, "1 in", true);
        }
        assert_eq!(doc.feature_formula(id, &length.to_string()), None);
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 25.4).abs() < 1e-3);
    }

    #[test]
    fn saving_a_value_as_a_variable_makes_it_and_binds_the_dimension() {
        let registry = registry();
        let (mut doc, mut wb, id, length) = scene();
        registry.evaluate(&mut doc);
        // Into a new set: the name is free, the set is made for it.
        {
            let mut ctx = ctx(&mut doc);
            let save = SaveAs {
                name: "wall".into(),
                set: None,
            };
            wb.save_dimension_as(&mut ctx, length, "12", &save);
        }
        let set = doc.object_named("Variables").expect("a set was made");
        let (_, _, data) = doc
            .variable_sets()
            .into_iter()
            .find(|(s, ..)| *s == set)
            .unwrap();
        assert_eq!(data.variable("wall").unwrap().formula, "12 mm");
        assert_eq!(
            doc.feature_formula(id, &length.to_string()),
            Some("Variables.wall")
        );
        // The dimension stands at the value at once, and follows the
        // variable from now on.
        assert!((line_length(&wb, &mut doc) - 12.0).abs() < 1e-3);
        doc.set_variable(set, "wall", "20 mm", None).unwrap();
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 20.0).abs() < 1e-3);

        // Into a set that is there: a formula typed is the variable's.
        let sizes = doc.object_named("Sizes").unwrap();
        {
            let mut ctx = ctx(&mut doc);
            let save = SaveAs {
                name: "half".into(),
                set: Some(sizes),
            };
            wb.save_dimension_as(&mut ctx, length, "Sizes.w / 2", &save);
        }
        registry.evaluate(&mut doc);
        assert_eq!(
            doc.feature_formula(id, &length.to_string()),
            Some("Sizes.half")
        );
        assert!((line_length(&wb, &mut doc) - 15.0).abs() < 1e-3);
        assert_eq!(doc.variable_sets().len(), 2, "no set was made for it");
    }

    #[test]
    fn a_field_is_offered_the_variables_of_its_kind_under_free_names() {
        let (mut doc, ..) = scene();
        let registry = registry();
        let sizes = doc.object_named("Sizes").unwrap();
        doc.set_variable(sizes, "tilt", "30 deg", None).unwrap();
        registry.evaluate(&mut doc);
        let names = |dim| {
            doc.variables_of(dim)
                .into_iter()
                .map(|c| c.reference)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(core_document::expr::Dim::LENGTH), ["Sizes.w"]);
        assert_eq!(names(core_document::expr::Dim::ANGLE), ["Sizes.tilt"]);
        assert_eq!(doc.unused_variable_name(Some(sizes), "w"), "w_2");
        assert_eq!(doc.unused_variable_name(None, "w"), "w");
        assert_eq!(doc.unused_object_name("Sizes"), "Sizes_2");
        assert_eq!(doc.unused_object_name("Variables"), "Variables");
    }
}
