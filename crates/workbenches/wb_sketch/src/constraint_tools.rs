//! The constraint tools on the selection: which constraints a toolbar
//! action adds, and the flags (driving, active) the toolbar flips.

use std::collections::HashSet;

use crate::{
    ConstraintFlag, SketchFeature, SketchWorkbench, commands, constrain, dim_edit, ids_json, sketch,
};
use core_document::{InputResult, WorkbenchRuntimeContext};
use uuid::Uuid;

impl SketchWorkbench {
    /// A constraint tool on the current selection: every kind it maps to
    /// is added, dimensional ones at their measured value. A selection the
    /// tool does not take yet, but could with more picks, arms the tool
    /// (`model_picks`).
    pub(crate) fn apply_constraint_tool(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        which: &str,
    ) -> InputResult {
        self.tab.armed = None;
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let shape = constrain::SelectionShape::picked_in(
            &feature.sketch,
            &self.tab.selected,
            self.tab.selected.in_order(),
        );
        // "dimension" is whichever dimensional tool the selection takes.
        let resolved = if which == "dimension" {
            constrain::dimension_in(&shape, &feature.sketch)
        } else {
            Some(which)
        };
        let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
        items.sort();
        let own_ends = constrain::joins_own_ends(&feature.sketch, &shape.points_picked());
        let done = resolved.is_some_and(|resolved| {
            self.constrain_items(ctx, feature, resolved, &items, Vec::new())
        });
        if !done && which == "coincident" && own_ends {
            ctx.log_warn(
                "The two ends of one line or arc cannot meet: draw a circle for a closed round",
            );
        } else if !done && !self.arm_constraint(ctx, tool, which, &shape) {
            ctx.log_warn(if which == "dimension" {
                "Select a line, circles, or two items to dimension".to_string()
            } else {
                format!("The {which} constraint does not fit the current selection")
            });
        }
        self.tab.selection_shape = if done { Default::default() } else { shape };
        if done {
            self.continue_constraint(ctx, tool, which);
        }
        InputResult::consumed()
    }

    /// Add the constraints the tool `which` makes of `items` to `feature`'s
    /// sketch and store it, recording `first` (the calls that made what the
    /// items name) and then the constraint. Whether the items fit.
    pub(crate) fn constrain_items(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        mut feature: SketchFeature,
        which: &str,
        items: &[Uuid],
        first: Vec<(&'static str, core_document::CommandArgs, serde_json::Value)>,
    ) -> bool {
        // A dimension the sketch holds already is not made twice: the one
        // it holds is selected instead, its value there to change.
        if let Some(held) = commands::held_dimension(&feature.sketch, which, items) {
            let label = feature
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == held)
                .map(|c| sketch::constraint_label(&c.kind))
                .unwrap_or_default();
            ctx.log_warn(format!(
                "The sketch already holds this dimension ({label}); change its value instead"
            ));
            self.tab.selected.clear();
            self.tab.selected_constraints = HashSet::from([held]);
            return true;
        }
        let Ok(made) = commands::constrain(&mut feature.sketch, which, items, None) else {
            return false;
        };
        let remove_redundant = self.options.auto_remove_redundant;
        let removed = if remove_redundant {
            commands::remove_superseded(&mut feature.sketch, &made)
        } else {
            0
        };
        let mut calls = Vec::new();
        if let Some(sketch_id) = self.tab.active_sketch_id {
            calls = first;
            let mut args = serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "kind": which,
                "items": ids_json(items),
            });
            if remove_redundant {
                args["remove_redundant"] = serde_json::json!(true);
            }
            calls.push((
                "sketch.constrain",
                commands::object(args),
                serde_json::json!(made),
            ));
        }
        if removed > 0 {
            ctx.log_info(format!(
                "Removed {removed} constraint(s) the new one made redundant"
            ));
            self.tab
                .selected_constraints
                .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
        }
        let mut dimension = None;
        for id in &made {
            if let Some(c) = feature
                .sketch
                .constraints
                .iter()
                .find(|c| c.id.to_string() == *id)
            {
                ctx.log_info(format!(
                    "Added constraint: {}",
                    sketch::constraint_label(&c.kind)
                ));
                if c.kind.is_dimensional() && dimension.is_none() {
                    dimension = Some(c.id);
                }
            }
        }
        self.solve(ctx, &mut feature);
        // What the constraint took is done with: the next picks start afresh.
        let picked = self.tab.selected.in_order().to_vec();
        self.tab.selected.clear();
        // A new dimension asks its value in a card, and waits there with
        // the sketch and the calls that make it until OK.
        if let Some(id) = dimension.filter(|_| self.options.ask_dimension_value) {
            self.open_new_dimension(ctx, &feature, id);
            if let Some(edit) = self.tab.dim_edit.as_mut() {
                edit.restore = picked;
                edit.made = Some(dim_edit::MadeDimension { feature, calls });
            }
            return true;
        }
        self.tab.pending_focus = dimension;
        for (id, args, result) in calls {
            ctx.record(id, args, result);
        }
        self.store_sketch(ctx, feature);
        true
    }

    /// Flip one flag of the selected constraints, each for itself, through
    /// `sketch.set_constraint`'s code.
    pub(crate) fn toggle_constraint_flag(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        which: ConstraintFlag,
    ) -> InputResult {
        let (Some(mut feature), Some(id)) =
            (self.get_active_sketch(ctx), self.tab.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        // The selected constraints, split by the flag each ends with.
        let mut to: [Vec<Uuid>; 2] = [Vec::new(), Vec::new()];
        for c in &feature.sketch.constraints {
            if self.tab.selected_constraints.contains(&c.id) {
                let now = match which {
                    ConstraintFlag::Driving => c.driving,
                    ConstraintFlag::Active => c.active,
                    ConstraintFlag::Parked => c.parked,
                };
                to[usize::from(!now)].push(c.id);
            }
        }
        if to.iter().all(Vec::is_empty) {
            return InputResult::consumed();
        }
        for (flag, mut items) in [(false, to[0].clone()), (true, to[1].clone())] {
            if items.is_empty() {
                continue;
            }
            items.sort();
            let mut flags = commands::ConstraintFlags::default();
            let name = match which {
                ConstraintFlag::Driving => {
                    flags.driving = Some(flag);
                    "driving"
                }
                ConstraintFlag::Active => {
                    flags.active = Some(flag);
                    "active"
                }
                ConstraintFlag::Parked => {
                    flags.parked = Some(flag);
                    "parked"
                }
            };
            commands::set_constraints(&mut feature.sketch, &items, flags);
            let mut args = serde_json::json!({
                "sketch": id.0.to_string(),
                "items": ids_json(&items),
            });
            args[name] = serde_json::json!(flag);
            ctx.record(
                "sketch.set_constraint",
                commands::object(args),
                serde_json::Value::Null,
            );
        }
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }
}
