//! The dimension card: a dimension's value, formula or new variable edited
//! in a card by its label, the value taking the keys as it opens, the view
//! showing the sketch solved at it, until OK writes it as one undo step or
//! Cancel and Escape drop it. A dimension a tool has just made waits in
//! the card too, stored only on OK.
//!
//! The draft is worked out on a copy of the document: the dimension set,
//! its formula or variable in place, every formula worked out again
//! (`evaluate_with_retained_parameters`), so a loop or an unknown name
//! is refused before Apply and what the change moves in the sketch's
//! other dimensions shows with it.

use bench_api::{ButtonStyle, FloatingCard, PanelEvent, Tone, panel};
use core_document::expr::{self, Quantity};
use core_document::{HostRequest, WorkbenchFeature};
use core_document::{WorkbenchRuntimeContext, evaluate};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::operations::SolveOutcome;
use crate::sketch::{self, ConstraintKind};
use crate::{DimEdit, SaveAs, SketchWorkbench, commands, glyphs, params, solver};

/// How many variables the task offers to read, one click each.
const MAX_CHOICES: usize = 12;

/// What the open dimension task works from.
pub(crate) struct DimensionDraft {
    constraint: Uuid,
    /// The open sketch as the task found it, with the task's switches
    /// between radius and diameter, or arc length and sweep, made.
    base: SketchFeature,
    /// The document's edit count when it opened: an edit from elsewhere
    /// puts the task away.
    seq: u64,
    /// What the field held as the task opened (the dimension's formula,
    /// or its value as labels write it): left so, the value stays exact.
    initial: String,
    /// The dimension's formula as the task found it, through its switches.
    formula: Option<String>,
    /// The switches made, as `sketch.set_constraint` takes them in `as`.
    switches: Vec<&'static str>,
    /// What the result was worked out from.
    input: Option<(String, bool, Option<SaveAs>)>,
    /// The sketch solved at the value, or why it cannot be.
    working: Result<SketchFeature, String>,
}

impl SketchWorkbench {
    /// Open dimension `constraint`'s task with `text` in its field (a
    /// value typed into its row, or a formula).
    pub(crate) fn open_dimension_task(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
    ) {
        let Some(c) = self.get_active_sketch(ctx).and_then(|f| {
            f.sketch
                .constraints
                .into_iter()
                .find(|c| c.id == constraint)
        }) else {
            return;
        };
        self.dim_edit = Some(DimEdit::new(
            constraint,
            [0.0; 2],
            text.to_string(),
            c.driving,
        ));
        self.dim_draft = None;
    }

    /// Make the draft for the open task and work it out again when its
    /// input changed; put the task away when its dimension went or the
    /// document changed under it.
    pub(crate) fn refresh_dimension_draft(&mut self, ctx: &WorkbenchRuntimeContext) {
        let Some(edit) = &self.dim_edit else {
            self.dim_draft = None;
            return;
        };
        let stale = self.dim_draft.as_ref().is_some_and(|d| {
            d.constraint != edit.constraint || d.seq != ctx.document.mutation_seq()
        });
        if stale {
            self.dim_edit = None;
            self.dim_draft = None;
            return;
        }
        if self.dim_draft.is_none() {
            let base = match &edit.made {
                Some(made) => Some(made.feature.clone()),
                None => self.get_active_sketch(ctx),
            };
            let (Some(base), Some(id)) = (base, self.active_sketch_id) else {
                self.dim_edit = None;
                return;
            };
            let Some(c) = base
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == edit.constraint)
            else {
                // The dimension went (an undo, a delete): so does its task.
                self.dim_edit = None;
                return;
            };
            let formula = ctx
                .document
                .feature_formula(id, &edit.constraint.to_string())
                .map(str::to_string);
            let initial = formula.clone().unwrap_or_else(|| {
                sketch::dimension_value(&c.kind)
                    .map(glyphs::fmt_num)
                    .unwrap_or_default()
            });
            self.dim_draft = Some(DimensionDraft {
                constraint: edit.constraint,
                base,
                seq: ctx.document.mutation_seq(),
                initial,
                formula,
                switches: Vec::new(),
                input: None,
                working: Err(String::new()),
            });
        }
        let input = (edit.text.clone(), edit.driving, edit.save_as.clone());
        if self.dim_draft.as_ref().and_then(|d| d.input.as_ref()) == Some(&input) {
            return;
        }
        let working = self.calculate_dimension(ctx);
        if let Some(draft) = self.dim_draft.as_mut() {
            draft.input = Some(input);
            draft.working = working;
        }
        self.last_diagnosis = None;
    }

    /// The draft's sketch solved, while it is current, else what it was
    /// worked out from (a new dimension at what it measured).
    pub(crate) fn dimension_preview(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        let edit = self.dim_edit.as_ref()?;
        match &self.dim_draft {
            Some(d) if d.seq == ctx.document.mutation_seq() => {
                Some(d.working.as_ref().unwrap_or(&d.base).clone())
            }
            Some(_) => None,
            None => edit.made.as_ref().map(|m| m.feature.clone()),
        }
    }

    /// The formula dimension `constraint` takes from the task's text: the
    /// variable's reference when saved as one, the text when it is a
    /// formula, none for a plain value.
    fn task_formula(
        &self,
        document: &core_document::Document,
        edit: &DimEdit,
    ) -> Result<Option<String>, String> {
        let text = edit.text.trim();
        let Some(save) = &edit.save_as else {
            return Ok((!expr::is_constant(text)).then(|| text.to_string()));
        };
        if !expr::is_valid_name(save.name.trim()) {
            return Err("Give the variable a valid name".into());
        }
        let set_name = match save.set {
            Some(set) => match document.get_feature_meta(set) {
                Some(node) => node.name.clone(),
                None => return Err("The variable set is no longer there".into()),
            },
            None => document.unused_object_name("Variables"),
        };
        Ok(Some(format!(
            "{}.{}",
            expr::quote_name(&set_name),
            expr::quote_name(save.name.trim())
        )))
    }

    /// The draft's sketch at the task's text, or why it cannot be.
    fn calculate_dimension(&self, ctx: &WorkbenchRuntimeContext) -> Result<SketchFeature, String> {
        let (Some(edit), Some(draft), Some(id)) =
            (&self.dim_edit, &self.dim_draft, self.active_sketch_id)
        else {
            return Err(String::new());
        };
        let text = edit.text.trim();
        if text.is_empty() {
            return Err("Type a value or a formula".into());
        }
        if !expr::is_constant(text) {
            expr::check_syntax(text).map_err(|e| e.message)?;
        }
        let mut feature = draft.base.clone();
        let c = feature
            .sketch
            .constraints
            .iter_mut()
            .find(|c| c.id == edit.constraint)
            .ok_or("The dimension is no longer there")?;
        let dim = params::formula_dim(&c.kind);
        let formula = self.task_formula(ctx.document, edit)?;
        let value = match &formula {
            // Left as it opened, the value is the one stored, not the
            // rounded one the label writes.
            None if edit.text == draft.initial => sketch::dimension_value(&c.kind),
            None => Some(ctx.document.evaluate_formula(text, Some(dim))?.value),
            // A formula's value comes from the evaluation below.
            Some(_) => sketch::dimension_value(&c.kind),
        }
        .ok_or("This constraint has no value")?;
        c.kind = sketch::with_dimension_value(&c.kind, value);
        c.driving = edit.driving || formula.is_some();
        let key = edit.constraint.to_string();
        // What else the change moves, worked out on a copy as the document
        // would: needed when a formula or variable is involved anywhere.
        let formulas_anywhere = formula.is_some()
            || draft.formula.is_some()
            || ctx
                .document
                .feature_tree()
                .all_nodes()
                .any(|(_, n)| !n.formulas.is_empty());
        if formulas_anywhere {
            let mut scratch = ctx.document.clone();
            scratch
                .update_feature_data(id, feature.to_json())
                .map_err(|e| e.to_string())?;
            if let Some(save) = &edit.save_as {
                let set = match save.set {
                    Some(set) => set,
                    None => scratch.add_variable_set(&scratch.unused_object_name("Variables"))?,
                };
                scratch.set_variable(
                    set,
                    save.name.trim(),
                    &variable_formula(ctx, text, dim)?,
                    None,
                )?;
            }
            scratch
                .set_feature_formula(id, key.clone(), formula.clone())
                .map_err(|e| e.to_string())?;
            let evaluation = evaluate::evaluate_with_retained_parameters(&scratch, &|node| {
                (node.workbench_id == SketchFeature::workbench_id())
                    .then(|| params::parameters(node))
            });
            if let Some(slot) = evaluation
                .slots
                .get(&id)
                .and_then(|slots| slots.iter().find(|s| s.key == key))
                && let Err(why) = &slot.result
            {
                return Err(format!("{text}: {why}"));
            }
            if let Some(data) = evaluation.data.get(&id) {
                let evaluated = SketchFeature::from_json(data).map_err(|e| e.to_string())?;
                feature.sketch.constraints = evaluated.sketch.constraints;
            }
        }
        let c = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)
            .ok_or("The dimension is no longer there")?;
        let value = sketch::dimension_value(&c.kind).unwrap_or(f64::NAN);
        if !value.is_finite() {
            return Err(format!("{text} is not a number the sketch can take"));
        }
        if value <= 0.0 && sized(&c.kind) {
            return Err(format!(
                "A {} must be greater than zero",
                sketch::constraint_label(&c.kind).to_lowercase()
            ));
        }
        if self.options.auto_update
            && matches!(
                solver::solve(&mut feature.sketch),
                SolveOutcome::NotConverged { .. }
            )
        {
            return Err("The constraints cannot be solved at this value".into());
        }
        Ok(feature)
    }

    /// Apply the task: the variable made, the formula set or cleared, the
    /// value and the sketch solved stored, recorded as the commands that
    /// make them, and closed as one undo step. Nothing happens while the
    /// draft is refused.
    pub fn commit_dim_edit(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.refresh_dimension_draft(ctx);
        let (Some(edit), Some(draft), Some(id)) = (
            self.dim_edit.clone(),
            self.dim_draft.take(),
            self.active_sketch_id,
        ) else {
            return;
        };
        let Ok(feature) = draft.working.clone() else {
            self.dim_draft = Some(draft);
            return;
        };
        self.dim_edit = None;
        let sketch_id = id.0.to_string();
        let constraint = edit.constraint.to_string();
        // A new dimension is made first, as the calls that make it.
        if let Some(made) = edit.made.clone() {
            for (call, args, result) in made.calls {
                ctx.record(call, args, result);
            }
            self.store_sketch(ctx, made.feature);
        }
        for switch in &draft.switches {
            ctx.record(
                "sketch.set_constraint",
                commands::args(serde_json::json!({
                    "sketch": sketch_id, "items": [constraint], "as": switch,
                })),
                serde_json::Value::Null,
            );
        }
        let text = edit.text.trim();
        let formula = match &edit.save_as {
            Some(save) => match self.save_variable(ctx, &feature, &edit, save) {
                Ok(reference) => Some(reference),
                Err(why) => {
                    ctx.log_warn(why);
                    return;
                }
            },
            None => (!expr::is_constant(text)).then(|| text.to_string()),
        };
        if ctx.document.feature_formula(id, &constraint) != formula.as_deref() {
            let _ = ctx
                .document
                .set_feature_formula(id, constraint.clone(), formula.clone());
            ctx.record(
                "doc.set_formula",
                commands::args(serde_json::json!({
                    "id": sketch_id, "parameter": constraint, "formula": formula,
                })),
                serde_json::Value::Null,
            );
        }
        let c = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint);
        // Recorded when it changes what the dimension held as the card
        // opened (through its switches).
        let held = draft
            .base
            .sketch
            .constraints
            .iter()
            .find(|b| b.id == edit.constraint)
            .map(|b| (sketch::dimension_value(&b.kind), b.driving));
        if let (None, Some(c)) = (&formula, c)
            && held != Some((sketch::dimension_value(&c.kind), c.driving))
        {
            ctx.record(
                "sketch.set_value",
                commands::args(serde_json::json!({
                    "sketch": sketch_id,
                    "constraint": constraint,
                    "value": sketch::dimension_value(&c.kind),
                    "driving": c.driving,
                })),
                serde_json::Value::Null,
            );
        }
        self.unsolved_edits = !self.options.auto_update && !solver::holds(&feature.sketch);
        self.last_diagnosis = None;
        self.store_sketch(ctx, feature);
        // The host closes the card's undo step once OK is handled.
        ctx.request(HostRequest::JournalLabel("Dimension".into()));
    }

    /// Put the task's value in its variable, making the set when it is new,
    /// recorded as `var.new` and `var.set`. The reference formulas read it
    /// by.
    fn save_variable(
        &self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        edit: &DimEdit,
        save: &SaveAs,
    ) -> Result<String, String> {
        let reference = self
            .task_formula(ctx.document, edit)?
            .ok_or("Give the variable a valid name")?;
        let kind = &feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)
            .ok_or("The dimension is no longer there")?
            .kind;
        let value = variable_formula(ctx, edit.text.trim(), params::formula_dim(kind))?;
        let name = save.name.trim();
        let (set, set_name) = match save.set {
            Some(set) => (
                set,
                ctx.document
                    .get_feature_meta(set)
                    .map(|n| n.name.clone())
                    .ok_or("The variable set is no longer there")?,
            ),
            None => {
                let set_name = ctx.document.unused_object_name("Variables");
                let set = ctx.document.add_variable_set(&set_name)?;
                ctx.record(
                    "var.new",
                    commands::args(serde_json::json!({ "name": set_name })),
                    serde_json::json!(set.0.to_string()),
                );
                (set, set_name)
            }
        };
        ctx.document.set_variable(set, name, &value, None)?;
        ctx.record(
            "var.set",
            commands::args(serde_json::json!({
                "set": set_name, "name": name, "formula": value,
            })),
            serde_json::Value::Null,
        );
        ctx.log_info(format!("{set_name}.{name} = {value}"));
        Ok(reference)
    }

    /// Close the card without applying it, writing nothing: a dimension a
    /// tool has just made stays unmade, and what was selected before it is
    /// selected again.
    pub fn cancel_dim_edit(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.dim_draft = None;
        if let Some(edit) = self.dim_edit.take()
            && edit.made.is_some()
        {
            self.selected.extend(edit.restore);
        }
    }

    /// The card by the dimension's label: the value or formula (with the
    /// keys as it opens, its text selected), what it comes to, the
    /// variables to read, Save as variable, Driving and the switch between
    /// radius and diameter or arc length and sweep; OK held back with the
    /// reason while the value cannot be taken.
    pub(crate) fn dimension_card(&mut self, ctx: &WorkbenchRuntimeContext) -> Option<FloatingCard> {
        self.refresh_dimension_draft(ctx);
        let (edit, draft) = (self.dim_edit.as_ref()?, self.dim_draft.as_ref()?);
        let c = draft
            .base
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == edit.constraint)?;
        let document = &*ctx.document;
        let kind_label = sketch::constraint_label(&c.kind);
        let title = match c.name.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(name) => name.to_string(),
            None if edit.new => format!("New {}", kind_label.to_lowercase()),
            None => kind_label.clone(),
        };
        let dim = params::formula_dim(&c.kind);
        let mut children = Vec::new();
        let switch = match c.kind {
            ConstraintKind::Radius { .. } => Some(("dim.circle", ["Radius", "Diameter"], 0)),
            ConstraintKind::Diameter { .. } => Some(("dim.circle", ["Radius", "Diameter"], 1)),
            ConstraintKind::ArcLength { .. } => Some(("dim.arc", ["Length", "Sweep"], 0)),
            ConstraintKind::ArcAngle { .. } => Some(("dim.arc", ["Length", "Sweep"], 1)),
            _ => None,
        };
        if let Some((id, options, at)) = switch {
            children.push(panel::segmented(id, "Measures", options, at).into());
        }
        let mut field = panel::entry_with("dim.text", "Value", &edit.text, "A value, or a formula");
        field.live = true;
        field.mono = true;
        field.formulas = true;
        field.large = true;
        children.push(field.into());
        let text = edit.text.trim();
        let constant = expr::is_constant(text) && edit.save_as.is_none();
        if let Ok(working) = &draft.working {
            {
                let value = working
                    .sketch
                    .constraints
                    .iter()
                    .find(|w| w.id == c.id)
                    .and_then(|w| sketch::dimension_value(&w.kind))
                    .unwrap_or_default();
                let suffix = match sketch::dimension_unit(&c.kind) {
                    sketch::DimensionUnit::Length => " mm",
                    sketch::DimensionUnit::Angle => "°",
                    sketch::DimensionUnit::Ratio => "",
                };
                let sign = if constant { "=" } else { "ƒ =" };
                let line = format!("{sign} {}{suffix}", fmt_value(value));
                let tone = if constant { Tone::Muted } else { Tone::Accent };
                children.push(panel::caption(&line, tone, true));
            }
        }
        let choices = document.variables_of(dim);
        let sets = document.variable_sets();
        if !choices.is_empty() {
            children.push(panel::overline("Variables"));
            let chips = choices.iter().take(MAX_CHOICES).enumerate().map(|(i, v)| {
                let label = if sets.len() > 1 {
                    &v.reference
                } else {
                    &v.name
                };
                let value = v.value.display(document.display_unit(), 3);
                panel::selectable(
                    &format!("dim.var:{i}"),
                    &format!("{label}  {value}"),
                    text == v.reference,
                )
            });
            children.push(panel::flow(chips));
        }
        let reads_a_variable = choices.iter().any(|v| v.reference == text);
        match &edit.save_as {
            None if !reads_a_variable => children.push(panel::button(
                "dim.save",
                "+ Save as variable",
                ButtonStyle::PlainSmall,
                true,
            )),
            None => {}
            Some(save) => {
                children.push(panel::overline("Save as variable"));
                let mut name = panel::entry_with("dim.var_name", "Name", &save.name, "name");
                name.live = true;
                name.mono = true;
                let mut options: Vec<&str> = sets.iter().map(|(_, n, _)| n.as_str()).collect();
                options.push("New set");
                let at = save
                    .set
                    .and_then(|s| sets.iter().position(|(id, ..)| *id == s))
                    .unwrap_or(sets.len());
                children.push(panel::grid(
                    "dim.save_fields",
                    [
                        name.into(),
                        panel::select("dim.var_set", "Set", options, at).into(),
                    ],
                ));
                let set_name = match save.set {
                    Some(set) => document
                        .get_feature_meta(set)
                        .map(|n| n.name.clone())
                        .unwrap_or_default(),
                    None => document.unused_object_name("Variables"),
                };
                let name = save.name.trim();
                let exists = choices
                    .iter()
                    .any(|v| Some(v.set) == save.set && v.name == name);
                let (note, tone) = if !expr::is_valid_name(name) {
                    ("Give it a name".to_string(), Tone::Bad)
                } else if exists {
                    (
                        format!("{set_name}.{name} exists: it takes this value"),
                        Tone::Warning,
                    )
                } else if save.set.is_none() {
                    (
                        format!("Makes the set {set_name}; formulas read {set_name}.{name}"),
                        Tone::Neutral,
                    )
                } else {
                    (
                        format!("Formulas read it as {set_name}.{name}"),
                        Tone::Neutral,
                    )
                };
                children.push(panel::caption(&note, tone, true));
                children.push(panel::button(
                    "dim.unsave",
                    "Keep the value in the dimension only",
                    ButtonStyle::PlainSmall,
                    true,
                ));
            }
        }
        if constant {
            let mut driving = panel::check("dim.driving", "", "Driving", edit.driving);
            driving.hint = "Off = reference dimension (measured, not enforced)".into();
            children.push(driving.into());
        }
        // By the label as the view draws it, else where it was opened.
        let shown = draft.working.as_ref().unwrap_or(&draft.base);
        let proj = crate::overlay::SketchProjector::new(ctx, shown.plane);
        let screen = self
            .glyphs(ctx, &shown.sketch, &proj)
            .iter()
            .find(|g| g.constraint == edit.constraint)
            .map_or(edit.screen_pos, |g| g.pos);
        Some(FloatingCard {
            // A new card for each dimension, its value taking the keys.
            id: format!("sketch.dimension:{}", edit.constraint),
            title,
            icon: Some(crate::style::constraint_icon(&c.kind).into()),
            screen: Some(screen),
            widgets: children,
            focus: Some("dim.text".into()),
            ok: if edit.save_as.is_some() { "Save" } else { "OK" }.into(),
            refusal: match &draft.working {
                Err(why) if !why.is_empty() => Some(why.clone()),
                _ => None,
            },
            ..Default::default()
        })
    }

    /// A change on the dimension's card. Answers whether it was one.
    pub(crate) fn dimension_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> bool {
        if self.dim_edit.is_none() {
            return false;
        }
        if let PanelEvent::CardClose { id, accept } = event
            && id.starts_with("sketch.dimension:")
        {
            if *accept {
                self.commit_dim_edit(ctx);
            } else {
                self.cancel_dim_edit(ctx);
            }
            return true;
        }
        if !event.id().starts_with("dim.") {
            return false;
        }
        self.refresh_dimension_draft(ctx);
        let document = &*ctx.document;
        match event {
            PanelEvent::Text { id, value } if id == "dim.text" => {
                if let Some(edit) = self.dim_edit.as_mut() {
                    edit.text.clone_from(value);
                }
            }
            PanelEvent::Button { id } if id == "dim.save" => {
                let label = self
                    .dim_draft
                    .as_ref()
                    .and_then(|d| {
                        let c = d
                            .base
                            .sketch
                            .constraints
                            .iter()
                            .find(|c| c.id == d.constraint)?;
                        // Named after the dimension's name, else its kind.
                        let named = c.name.clone().filter(|n| expr::is_valid_name(n));
                        Some(named.unwrap_or_else(|| sketch::constraint_label(&c.kind)))
                    })
                    .unwrap_or_default();
                let set = document.variable_sets().last().map(|(id, ..)| *id);
                let name = document.unused_variable_name(set, &variable_base_name(&label));
                if let Some(edit) = self.dim_edit.as_mut() {
                    edit.save_as = Some(SaveAs { name, set });
                }
            }
            PanelEvent::Button { id } if id == "dim.unsave" => {
                if let Some(edit) = self.dim_edit.as_mut() {
                    edit.save_as = None;
                }
            }
            PanelEvent::Text { id, value } if id == "dim.var_name" => {
                if let Some(save) = self.dim_edit.as_mut().and_then(|e| e.save_as.as_mut()) {
                    save.name.clone_from(value);
                }
            }
            PanelEvent::Choice { id, index } if id == "dim.var_set" => {
                let set = document.variable_sets().get(*index).map(|(id, ..)| *id);
                if let Some(save) = self.dim_edit.as_mut().and_then(|e| e.save_as.as_mut()) {
                    save.set = set;
                }
            }
            PanelEvent::Toggle { id, on } if id == "dim.driving" => {
                if let Some(edit) = self.dim_edit.as_mut() {
                    edit.driving = *on;
                }
            }
            PanelEvent::Choice { id, index } if id == "dim.circle" => {
                self.switch_draft_circle(*index == 1);
            }
            PanelEvent::Choice { id, index } if id == "dim.arc" => {
                self.switch_draft_arc(*index == 1);
            }
            PanelEvent::Button { id } => {
                let chosen = panel::indexed(id, "dim.var:").and_then(|i| {
                    let dim = self.dim_draft.as_ref().and_then(|d| {
                        let c = d
                            .base
                            .sketch
                            .constraints
                            .iter()
                            .find(|c| c.id == d.constraint)?;
                        Some(params::formula_dim(&c.kind))
                    })?;
                    document
                        .variables_of(dim)
                        .get(i)
                        .map(|v| v.reference.clone())
                });
                if let (Some(reference), Some(edit)) = (chosen, self.dim_edit.as_mut()) {
                    edit.text = reference;
                    edit.save_as = None;
                }
            }
            _ => {}
        }
        if self.dim_edit.is_some() {
            self.refresh_dimension_draft(ctx);
        }
        true
    }

    /// The task's dimension made a diameter (or a radius) of its circle in
    /// the draft, what the field holds converted with it; written on Apply.
    fn switch_draft_circle(&mut self, diameter: bool) {
        let (Some(edit), Some(draft)) = (self.dim_edit.as_mut(), self.dim_draft.as_mut()) else {
            return;
        };
        let Some(scale) =
            commands::switch_circle_dimension(&mut draft.base.sketch, edit.constraint, diameter)
        else {
            return;
        };
        if scale == 1.0 {
            return;
        }
        let rescale = |text: &str| match text.trim().parse::<f64>() {
            Ok(value) => glyphs::fmt_num(value * scale),
            Err(_) if text.trim().is_empty() => String::new(),
            Err(_) => commands::scaled_formula(text, scale),
        };
        let unchanged = edit.text == draft.initial;
        edit.text = rescale(&edit.text);
        draft.initial = if unchanged {
            edit.text.clone()
        } else {
            rescale(&draft.initial)
        };
        draft.formula = draft
            .formula
            .as_deref()
            .map(|f| commands::scaled_formula(f, scale));
        draft
            .switches
            .push(if diameter { "diameter" } else { "radius" });
        draft.input = None;
    }

    /// The task's arc dimension made to measure its sweep (or its length)
    /// in the draft, at what the arc measures as the task found it; a
    /// formula on it goes, since it gave the other quantity.
    fn switch_draft_arc(&mut self, angle: bool) {
        let (Some(edit), Some(draft)) = (self.dim_edit.as_mut(), self.dim_draft.as_mut()) else {
            return;
        };
        let sketch = &mut draft.base.sketch;
        let Some(at) = sketch
            .constraints
            .iter()
            .position(|c| c.id == edit.constraint)
        else {
            return;
        };
        let kind = match sketch.constraints[at].kind {
            ConstraintKind::ArcLength { arc, .. } if angle => ConstraintKind::ArcAngle {
                arc,
                angle_rad: crate::measure::arc_sweep(sketch, arc).unwrap_or(0.0),
            },
            ConstraintKind::ArcAngle { arc, .. } if !angle => ConstraintKind::ArcLength {
                arc,
                length: crate::measure::arc_length(sketch, arc).unwrap_or(0.0),
            },
            _ => return,
        };
        sketch.constraints[at].kind = kind;
        let shown = sketch::dimension_value(&sketch.constraints[at].kind)
            .map(glyphs::fmt_num)
            .unwrap_or_default();
        edit.text.clone_from(&shown);
        edit.save_as = None;
        draft.initial = shown;
        draft.formula = None;
        draft.switches.push(if angle { "angle" } else { "length" });
        draft.input = None;
    }
}

/// What a variable saved from the task holds: a typed value with its unit,
/// a formula as typed.
fn variable_formula(
    ctx: &WorkbenchRuntimeContext,
    text: &str,
    dim: expr::Dim,
) -> Result<String, String> {
    if !expr::is_constant(text) {
        return Ok(text.to_string());
    }
    let value = ctx.document.evaluate_formula(text, Some(dim))?.value;
    Ok(Quantity::new(value, dim).display(ctx.document.display_unit(), 6))
}

/// A value as the task writes it: up to four places, no trailing zeros.
fn fmt_value(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// A dimension that sizes something and must stay above zero.
fn sized(kind: &ConstraintKind) -> bool {
    matches!(
        kind,
        ConstraintKind::Length { .. }
            | ConstraintKind::Radius { .. }
            | ConstraintKind::Diameter { .. }
            | ConstraintKind::ArcLength { .. }
            | ConstraintKind::CurveLength { .. }
    )
}

/// What a variable made from a dimension of kind `label` is first called:
/// `Distance X` gives `distance_x`.
fn variable_base_name(label: &str) -> String {
    let name: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let name = name.trim_matches('_').to_string();
    if name.is_empty() {
        "value".to_string()
    } else {
        name
    }
}
