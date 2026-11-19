//! The solver's verdict on the edited sketch: what it says to the panel,
//! HUD and status bar, the elements nothing left free can move, and the
//! task's status pill.

use std::collections::HashSet;

use core_document::SketchPalette;
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
    /// again only when `sketch` differs from the one the kept set was
    /// worked out on other than in where its labels sit (an edit, an undo,
    /// a formula's new value, a card's draft); none while the sketch is
    /// fully constrained, when everything draws in that colour instead.
    pub(crate) fn locked_elements(&self, sketch: &Sketch) -> std::sync::Arc<HashSet<Uuid>> {
        if self.tab.active_sketch_id.is_none() || sketch.is_fully_constrained {
            return Default::default();
        }
        if let Some((at, set)) = self.tab.locked.borrow().as_ref()
            && at.eq_but_labels(sketch)
        {
            return set.clone();
        }
        let set = std::sync::Arc::new(solver::locked_elements(sketch));
        *self.tab.locked.borrow_mut() = Some((sketch.clone(), set.clone()));
        set
    }

    /// [`solver::dof_estimate`] of `sketch`, worked out again only when
    /// `sketch` differs from the one the kept estimate was worked out on:
    /// every reader of a frame, and every frame the sketch stays, share
    /// one estimate, as does a label's drag, while any other change to it
    /// (an edit, an undo, a peer's op, a formula's new value, a card's
    /// draft) is a different sketch.
    fn freedom(&self, sketch: &Sketch) -> i32 {
        if let Some((at, dof)) = self.tab.freedom.borrow().as_ref()
            && at.eq_but_labels(sketch)
        {
            return *dof;
        }
        let dof = solver::dof_estimate(sketch);
        *self.tab.freedom.borrow_mut() = Some((sketch.clone(), dof));
        dof
    }

    /// [`solver::diagnose`] of `sketch`; on a sketch too large to analyse,
    /// whose diagnosis is its freedom alone, the kept estimate.
    pub(crate) fn diagnose(&self, sketch: &Sketch) -> solver::Diagnosis {
        if solver::diagnosable(sketch) {
            return solver::diagnose(sketch);
        }
        solver::Diagnosis {
            dof: self.freedom(sketch),
            analyzed: false,
            ..Default::default()
        }
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
            None => (self.freedom(sketch), true, Vec::new(), Vec::new()),
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

#[cfg(test)]
mod tests {
    use core_document::{
        Document, Workbench, WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
    };

    use crate::sketch::{ConstraintKind, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};
    use crate::{SketchFeature, SketchWorkbench, solver, stored_sketch};

    /// A hover over an unchanged sketch reads the kept freedom through
    /// every frame and every reader; the document changing under it, as a
    /// peer's op or an undo changes it, works it out again.
    #[test]
    fn the_freedom_is_estimated_once_per_sketch() {
        let mut sketch = Sketch::new("s");
        let [a, b] = [0.0, 5.0]
            .map(|x| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 1.0)))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let mut doc = Document::new("t");
        let feature = SketchFeature::new(sketch, SketchPlane::xy());
        let id = doc.add_feature(feature, "s".into()).unwrap();
        let mut wb = SketchWorkbench::default();
        let frame = |doc: &mut Document, wb: &mut SketchWorkbench, x: f32| {
            let mut ctx =
                WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
            ctx.active_document_object = Some(id);
            let viewport_pos = (x, 300.0);
            let event = WorkbenchInputEvent::MouseMove { viewport_pos };
            wb.on_input(&event, Some("sketch.line"), &mut ctx);
            wb.on_frame(0.016, &mut ctx);
            wb.viewport_hud(&ctx);
            wb.status_items(&ctx);
            wb.solver_verdict(&wb.frame_displayed(&ctx).unwrap().sketch)
                .dof
        };
        frame(&mut doc, &mut wb, 400.0);
        // A planted value read back was not worked out again.
        wb.tab.freedom.borrow_mut().as_mut().unwrap().1 = -7;
        assert_eq!(frame(&mut doc, &mut wb, 420.0), -7);
        assert_eq!(frame(&mut doc, &mut wb, 440.0), -7);
        // A label held elsewhere, as its drag holds it, is the same sketch.
        let mut held = stored_sketch(&doc, id).unwrap().sketch;
        held.constraints[0].label_offset = Some(Vec2D::new(3.0, 4.0));
        assert_eq!(wb.solver_verdict(&held).dof, -7);
        let mut edited = stored_sketch(&doc, id).unwrap();
        let free = Point::new(Vec2D::new(2.0, 3.0));
        edited.sketch.add_geometry(GeometryElement::Point(free));
        doc.update_feature_data(id, edited.to_json()).unwrap();
        let dof = frame(&mut doc, &mut wb, 460.0);
        assert_eq!(dof, solver::dof_estimate(&edited.sketch));
        assert!(dof > 0);
        // A solve's own measure is kept for the sketch it stores.
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        let mut feature = wb.get_active_sketch(&ctx).unwrap();
        feature
            .sketch
            .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(4.0, 3.0))));
        wb.solve_now(&mut ctx, &mut feature);
        wb.store_sketch(&mut ctx, feature);
        let kept = wb.tab.freedom.borrow().clone().unwrap();
        assert_eq!(
            kept.1,
            solver::dof_estimate(&stored_sketch(&doc, id).unwrap().sketch)
        );
        wb.tab.freedom.borrow_mut().as_mut().unwrap().1 = -7;
        assert_eq!(frame(&mut doc, &mut wb, 480.0), -7);
    }

    /// A card's draft is a sketch the document does not hold: what it
    /// fixes is drawn locked though no document edit came with it.
    #[test]
    fn the_locked_elements_follow_a_draft() {
        let mut sketch = Sketch::new("s");
        let [a, b, _] = [0.0, 5.0, 9.0]
            .map(|x| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 1.0)))));
        let fix = |sketch: &mut Sketch, point, x| {
            let position = Vec2D::new(x, 1.0);
            sketch.add_constraint(ConstraintKind::FixedPoint { point, position });
        };
        fix(&mut sketch, a, 0.0);
        let mut doc = Document::new("t");
        let feature = SketchFeature::new(sketch.clone(), SketchPlane::xy());
        let id = doc.add_feature(feature, "s".into()).unwrap();
        let mut wb = SketchWorkbench::default();
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(id);
        wb.on_frame(0.016, &mut ctx);
        assert_eq!(*wb.locked_elements(&sketch), [a].into());
        let mut draft = sketch;
        fix(&mut draft, b, 5.0);
        assert_eq!(*wb.locked_elements(&draft), [a, b].into());
    }

    /// A sketch too large to analyse is diagnosed by the kept estimate.
    #[test]
    fn a_large_sketch_is_diagnosed_by_its_kept_freedom() {
        let mut sketch = Sketch::new("s");
        for k in 0..61 {
            let [a, b] = [0.0, 5.0].map(|x| {
                let at = Vec2D::new(x, k as f64);
                sketch.add_geometry(GeometryElement::Point(Point::new(at)))
            });
            let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
            sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        }
        let wb = SketchWorkbench::default();
        assert_eq!(wb.diagnose(&sketch).dof, solver::dof_estimate(&sketch));
        wb.tab.freedom.borrow_mut().as_mut().unwrap().1 = -7;
        let diagnosis = wb.diagnose(&sketch);
        assert_eq!((diagnosis.dof, diagnosis.analyzed), (-7, false));
    }
}
