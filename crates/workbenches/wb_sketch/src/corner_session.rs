//! A resolved corner edited and solved independently of the confirmed sketch.

use crate::operations::SolveOutcome;
use crate::sketch_draft::{SketchDraft, SketchSession};
use crate::tools::modify::{CornerCut, CornerTarget};
use crate::{SketchFeature, SketchWorkbench};
use core_document::WorkbenchRuntimeContext;

pub(crate) struct CornerSession {
    pub draft: SketchDraft,
    pub target: CornerTarget,
    pub bevel: bool,
}

/// Applies the retained operands once and solves the complete candidate.
pub(crate) fn calculate(
    base: &crate::sketch::Sketch,
    target: &CornerTarget,
    bevel: bool,
    size: f64,
    keep: bool,
) -> Result<crate::sketch::Sketch, String> {
    if !size.is_finite() || size < 1e-6 {
        return Err("Enter a finite positive corner size".into());
    }
    if !target.valid(base) {
        return Err("The selected corner curves are unavailable or read-only".into());
    }
    let mut working = base.clone();
    let cut = if bevel {
        CornerCut::Bevel(size)
    } else {
        CornerCut::Round(size)
    };
    let effect = crate::tools::modify::apply_corner(&mut working, target, cut, keep);
    if !effect.changed {
        return Err(effect
            .log
            .unwrap_or_else(|| "These curves cannot be joined at this size".into()));
    }
    let temporary = crate::tools::modify::size_corner(&mut working, base, target, cut);
    if matches!(
        crate::solver::solve(&mut working),
        SolveOutcome::NotConverged { .. }
    ) {
        return Err("Constraints cannot be solved for this corner".into());
    }
    if let Some(point) = temporary {
        working.remove_geometry_cascade(&[point]);
        working.is_fully_constrained = crate::solver::dof_estimate(&working) == 0;
    }
    Ok(working)
}

impl SketchWorkbench {
    pub(crate) fn discard_corner(&mut self) {
        if self.session.corner().is_some() {
            self.session = SketchSession::None;
        }
        self.tool_state = crate::tools::ToolState::Idle;
        self.dim_capture.clear_buffers();
        self.dim_capture.sync(&self.tool_state, None);
        self.draw_record = None;
        self.last_diagnosis = None;
    }

    pub(crate) fn invalidate_corner(&mut self, ctx: &WorkbenchRuntimeContext) -> bool {
        if self
            .session
            .corner()
            .is_some_and(|s| !s.draft.is_current(ctx))
        {
            self.discard_corner();
            return true;
        }
        false
    }

    pub(crate) fn pick_corner(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        base: SketchFeature,
        cursor: crate::sketch::Vec2D,
        tolerance: f64,
        bevel: bool,
    ) {
        let (target, effect) = crate::tools::modify::pick_corner(
            &mut self.tool_state,
            &base.sketch,
            cursor,
            tolerance,
            bevel,
        );
        if let Some(message) = effect.log {
            ctx.log_info(message);
        }
        if let Some(target) = target {
            let target = target.with_shared_vertex(&base.sketch);
            let size = if bevel {
                self.tool_params.chamfer_length
            } else {
                self.tool_params.fillet_radius
            };
            let mut draft = SketchDraft::new(base, ctx);
            draft.set_sketch(calculate(
                &draft.base.sketch,
                &target,
                bevel,
                size,
                self.tool_params.corner_keep,
            ));
            self.session = SketchSession::Corner(CornerSession {
                draft,
                target,
                bevel,
            });
            self.last_diagnosis = None;
            self.dim_capture.clear_buffers();
            self.dim_capture.sync(&self.tool_state, None);
        }
    }

    pub(crate) fn recalculate_corner(&mut self) {
        let s = self.session.corner_mut().expect("corner draft");
        let size = if s.bevel {
            self.tool_params.chamfer_length
        } else {
            self.tool_params.fillet_radius
        };
        s.draft.set_sketch(calculate(
            &s.draft.base.sketch,
            &s.target,
            s.bevel,
            size,
            self.tool_params.corner_keep,
        ));
        self.last_diagnosis = None;
    }

    pub(crate) fn accept_corner(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.invalidate_corner(ctx) {
            return;
        }
        if self.session.corner().is_none() {
            return;
        }
        let SketchSession::Corner(session) = std::mem::take(&mut self.session) else {
            unreachable!("corner acceptance requires a corner session")
        };
        let Ok(feature) = &session.draft.working else {
            self.session = SketchSession::Corner(session);
            return;
        };
        if !self.store_sketch(ctx, feature.clone()) {
            self.session = SketchSession::Corner(session);
            return;
        }
        if let Some(record) = &mut self.draw_record {
            record.changed = true;
            record.settings.insert(
                "corner".into(),
                serde_json::to_value(&session.target).unwrap(),
            );
            record.settings.insert(
                "params".into(),
                serde_json::Value::Object(crate::step::params_to_json(&self.tool_params)),
            );
        }
        self.tool_state = crate::tools::ToolState::Idle;
        self.flush_draw_record(ctx);
        self.finish_sketch_task(ctx, if session.bevel { "Chamfer" } else { "Fillet" });
    }
}
