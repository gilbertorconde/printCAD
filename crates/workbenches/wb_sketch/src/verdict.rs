//! The solver's verdict on the edited sketch: what it says to the panel,
//! HUD and status bar, the elements nothing left free can move, and the
//! task's status pill.

use std::collections::HashSet;

use core_document::{SketchPalette, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::sketch::Sketch;
use crate::{SketchWorkbench, SolveOutcome, solver};

/// The solver's verdict on the edited sketch, as the panel, HUD and status
/// bar show it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SolverVerdict {
    pub kind: SolverKind,
    pub title: String,
    pub body: String,
    pub dof: i32,
    /// Constraints whose geometry a click on the message selects.
    pub offenders: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SolverKind {
    Empty,
    /// Edited with automatic update off: not solved yet.
    Unsolved,
    Unanalyzed,
    Conflicting,
    Redundant,
    Fully,
    Under,
}

impl SolverKind {
    pub(crate) fn color(self, pal: &SketchPalette) -> [f32; 3] {
        match self {
            SolverKind::Fully => pal.fully_constrained,
            SolverKind::Conflicting => pal.trim,
            SolverKind::Redundant | SolverKind::Unsolved => pal.constraint,
            SolverKind::Empty | SolverKind::Unanalyzed | SolverKind::Under => pal.reference,
        }
    }
}

impl SketchWorkbench {
    /// The open sketch's elements nothing left free can move, worked out
    /// once per document edit; none while the sketch is fully constrained,
    /// when everything draws in that colour instead.
    pub(crate) fn locked_elements(
        &self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) -> std::sync::Arc<HashSet<Uuid>> {
        let Some(id) = self.tab.active_sketch_id else {
            return Default::default();
        };
        if sketch.is_fully_constrained {
            return Default::default();
        }
        let key = (id, ctx.document.mutation_seq());
        if let Some((at, set)) = self.tab.locked.borrow().as_ref()
            && *at == key
        {
            return set.clone();
        }
        let set = std::sync::Arc::new(solver::locked_elements(sketch));
        *self.tab.locked.borrow_mut() = Some((key, set.clone()));
        set
    }

    /// The solver's verdict from the cached diagnosis, or a cheap estimate
    /// of the freedom left when none is cached yet.
    pub(crate) fn solver_verdict(&self, sketch: &Sketch) -> SolverVerdict {
        let not_converged = matches!(self.tab.last_solve, Some(SolveOutcome::NotConverged { .. }));
        let (dof, analyzed, conflicting, redundant) = match &self.tab.last_diagnosis {
            Some(d) => (
                d.dof,
                d.analyzed,
                d.conflicting.clone(),
                d.redundant.clone(),
            ),
            None => (solver::dof_estimate(sketch), true, Vec::new(), Vec::new()),
        };
        let verdict = |kind, title: &str, body: String, offenders: Vec<Uuid>| SolverVerdict {
            kind,
            title: title.to_string(),
            body,
            dof,
            offenders,
        };
        if sketch.geometry.is_empty() {
            verdict(
                SolverKind::Empty,
                "Empty sketch",
                "Draw geometry with the tools above.".into(),
                Vec::new(),
            )
        } else if self.tab.unsolved_edits {
            // Before any verdict on freedom or conflicts: those describe
            // the solved sketch, and the geometry shown is not it.
            verdict(
                SolverKind::Unsolved,
                "Not solved",
                "Automatic update is off: the geometry waits for Solve now.".into(),
                Vec::new(),
            )
        } else if !analyzed {
            verdict(
                SolverKind::Unanalyzed,
                "Not analyzed",
                "Too many constraints to check for conflicts.".into(),
                Vec::new(),
            )
        } else if not_converged || !conflicting.is_empty() {
            verdict(
                SolverKind::Conflicting,
                "Conflicting constraints",
                "Constraints contradict each other. Click to select their geometry.".into(),
                conflicting,
            )
        } else if !redundant.is_empty() {
            verdict(
                SolverKind::Redundant,
                "Redundant constraints",
                "Some constraints add nothing the others do not already enforce.".into(),
                redundant,
            )
        } else if dof == 0 && !sketch.constraints.is_empty() {
            verdict(
                SolverKind::Fully,
                "Fully constrained",
                "Sketch has 0 degrees of freedom.".into(),
                Vec::new(),
            )
        } else {
            verdict(
                SolverKind::Under,
                "Under-constrained",
                format!("{dof} degrees of freedom remain. Add dimensions or constraints."),
                Vec::new(),
            )
        }
    }

    /// The solver's verdict as the panel's header shows it: a few words
    /// and a tone, the reason on hover.
    pub(crate) fn task_status(&self, sketch: &Sketch) -> core_document::TaskStatus {
        use core_document::StatusTone;
        let verdict = self.solver_verdict(sketch);
        let (text, tone) = match verdict.kind {
            SolverKind::Fully => ("Fully constrained".to_string(), StatusTone::Good),
            SolverKind::Under => (
                match verdict.dof {
                    1 => "1 degree of freedom left".to_string(),
                    n => format!("{n} degrees of freedom left"),
                },
                StatusTone::Warning,
            ),
            SolverKind::Conflicting => (verdict.title.clone(), StatusTone::Bad),
            SolverKind::Redundant | SolverKind::Unsolved => {
                (verdict.title.clone(), StatusTone::Warning)
            }
            _ => (verdict.title.clone(), StatusTone::Info),
        };
        core_document::TaskStatus {
            text,
            detail: verdict.body,
            tone,
        }
    }
}
