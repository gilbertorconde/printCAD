//! A task's edits in the journal: from the frame a task opens, or a tool
//! makes what it will open on, until it closes, nothing closes an undo
//! step. OK closes them as one step named by the task; its Cancel takes
//! them back through the journal and leaves no step, so every task,
//! a bench's or the application's, cancels the same way. A task that
//! goes without closing (its tab or bench left, its subject gone) keeps
//! what it did as one step.

use core_document::TaskOutcome;

use crate::PrintCadApp;
use crate::log_panel as app_log;

impl PrintCadApp {
    /// Whether a task that holds its edits as one step is open, or due to
    /// open on the next frame: the panel drew one, or the active bench
    /// answers one (a tool that made a feature this frame, after the
    /// panel was drawn).
    fn task_in_progress(&mut self) -> bool {
        if self.session.task_open {
            return true;
        }
        let wb_id = self.session.active_workbench.0.clone();
        let params = self.interaction_ctx_params();
        self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.task(ctx))
            .and_then(|(task, _)| task)
            .is_some_and(|task| !task.stepwise)
    }

    /// At the frame's end: a task under way opens its step in the journal
    /// with the edits not yet closed (a tool's feature made this frame),
    /// one gone without closing keeps its edits as a step. Answers whether
    /// a task is under way, when no other boundary may close a step.
    pub(crate) fn follow_task_step(&mut self) -> bool {
        let under_way = self.task_in_progress();
        match (under_way, self.session.journal.in_task()) {
            (true, false) => self.session.journal.begin_task(),
            (false, true) => {
                self.settle_formulas();
                self.session
                    .journal
                    .accept_task(&mut self.session.document, None);
            }
            _ => {}
        }
        under_way
    }

    /// What the open task did so far becomes a step named `label`, and
    /// the task goes on from there (`HostRequest::KeepTaskEdits`).
    pub(crate) fn keep_task_edits(&mut self, label: String) {
        if !self.session.journal.in_task() {
            self.session.journal.label_next(label);
            self.close_gesture();
            return;
        }
        self.settle_formulas();
        self.session
            .journal
            .accept_task(&mut self.session.document, Some(label));
        self.session.journal.begin_task();
    }

    /// A task closed: accepted, its edits are one step named by it;
    /// closed by its Cancel, they are taken back and leave no step; closed
    /// otherwise (Done on a panel with nothing to cancel, its subject gone)
    /// they are kept.
    pub(crate) fn end_task_step(&mut self, outcome: &TaskOutcome, take_back: bool) {
        if !self.session.journal.in_task() {
            self.session.journal.begin_task();
        }
        if take_back {
            let (undone, barrier) = self.session.journal.cancel_task(&mut self.session.document);
            if undone > 0 {
                self.after_history_jump();
            }
            if barrier {
                app_log::warn(
                    "Cancel took back what followed the file the task read in; \
                     what came before it stays, and undo history was cleared",
                );
            }
            return;
        }
        let label = match outcome {
            TaskOutcome::Accepted { label } => {
                // The faces and edges the task took are used: the next
                // picks start afresh. A cancelled task leaves them picked.
                let body = self.session.selection.body;
                self.clear_view_selection();
                self.session.selection.body = body;
                Some(label.clone())
            }
            TaskOutcome::Cancelled | TaskOutcome::Open => None,
        };
        self.settle_formulas();
        self.session
            .journal
            .accept_task(&mut self.session.document, label);
    }
}
