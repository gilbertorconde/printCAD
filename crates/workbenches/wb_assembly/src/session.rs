//! What the bench holds for the document on screen: its task, picking,
//! held body and handles, the checks running on it, its view's toggles
//! and the values its panels hold as typed. The trait's default
//! `suspend_session`/`resume_session` keep it with the tab on a switch
//! and give it back when the tab returns, so a Cancel puts back only its
//! own document's bodies.
use core_document::{BodyId, BodyPlacement, Document, FeatureId, WorkbenchRuntimeContext};

use crate::interference::Interference;
use crate::mass::MassReport;
use crate::sweep_check::MotionClash;
use crate::{AssemblyWorkbench, Freedom, Grab, Named, Picking, Task, handles};
use core_document::jobs::Job;

#[derive(Default)]
pub(crate) struct Tab {
    pub(crate) picking: Option<Picking>,
    /// The selection a pick was last taken from, so a pick is a change.
    pub(crate) seen: Option<(uuid::Uuid, [u32; 3])>,
    pub(crate) task: Option<Task>,
    /// Where the open task last left the bodies it keeps start
    /// placements for, noted as each of the bench's hooks returns: a body
    /// found elsewhere was moved by someone else in between.
    pub(crate) left: Vec<(BodyId, BodyPlacement)>,
    /// What the last solve said, for the task panel.
    pub(crate) verdict: Option<Result<String, String>>,
    /// What each jointed body may still do, and at which edit of the
    /// document that was worked out: the status bar asks every frame.
    /// Kept with the tab, since another document's edit counts say
    /// nothing of this one.
    pub(crate) freedom: std::sync::Mutex<Option<(u64, Freedom)>>,
    /// The joints that hold nothing the others do not, at an edit of the
    /// document, for the status bar and the joint panel.
    pub(crate) redundant: std::sync::Mutex<Option<(u64, Named)>>,
    /// The joints and couplings that do not hold where the bodies sit, at
    /// an edit of the document, for the joint panel's messages.
    pub(crate) unmet: std::sync::Mutex<Option<(u64, Named)>>,
    /// A body held by the mouse, dragged with its joints holding.
    pub(crate) grab: Option<Grab>,
    /// Drags go through other bodies rather than stopping at them.
    pub(crate) collisions_off: bool,
    /// An interference check running on its own thread.
    pub(crate) checking: Option<Job<Interference>>,
    /// The bodies being measured for their mass, on their own thread.
    pub(crate) measuring: Option<Job<MassReport>>,
    /// A joint's motion being checked for collisions, on its own thread.
    pub(crate) sweeping: Option<(FeatureId, Job<Vec<MotionClash>>)>,
    /// What the last check of a joint's motion found, for that joint.
    pub(crate) motion_clashes: Option<(FeatureId, Result<Vec<MotionClash>, String>)>,
    /// The clearance the interference panel checks for, mm, as last typed.
    pub(crate) clearance_mm: Option<f32>,
    /// How far a joint's Turn turns its body, degrees, as last typed.
    pub(crate) turn_by: Option<f32>,
    /// The body last clicked while a group's bodies are picked, so a click
    /// counts once.
    pub(crate) group_seen: Option<uuid::Uuid>,
    /// The Move task's handles: whether they move or turn, and the one held.
    pub(crate) handles: handles::Handles,
    /// A driven hinge or slider swept through its range to show it move.
    #[cfg(feature = "egui")]
    pub(crate) playing: Option<crate::Play>,
    /// The name of a column to add to the parts list, as typed.
    #[cfg(feature = "egui")]
    pub(crate) parts_column: String,
}

impl AssemblyWorkbench {
    /// What the bench holds, checked against the document, which may have
    /// changed underneath it (an undo, another client's ops): a held body
    /// that is gone is let go, nothing put back or recorded; picks on a
    /// gone body or joint are dropped; a task whose body or joint is gone
    /// closes, putting back each body it moved that is still where the
    /// task left it. A body moved by someone else since keeps that place,
    /// where Cancel would put every body back.
    pub(crate) fn settle_held(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let document = &*ctx.document;
        let body_gone = |body: BodyId| body != crate::WORLD && document.body(body).is_none();
        let feature_gone = |id: FeatureId| document.get_feature_meta(id).is_none();
        if self.tab.grab.as_ref().is_some_and(|g| body_gone(g.body)) {
            self.tab.grab = None;
        }
        if let Some(picking) = &mut self.tab.picking {
            if picking.repick.is_some_and(feature_gone) {
                self.tab.picking = None;
            } else if picking.first.as_ref().is_some_and(|f| body_gone(f.0)) {
                picking.first = None;
                picking.tab = None;
                picking.slot = None;
            } else if picking.slot.as_ref().is_some_and(|s| body_gone(s.0)) {
                picking.slot = None;
            }
        }
        let gone = match &self.tab.task {
            Some(Task::Move { body, .. } | Task::Copies { body, .. }) => body_gone(*body),
            Some(Task::Replace { old, .. }) => body_gone(*old),
            Some(Task::Joint { id, .. } | Task::Coupling { id, .. }) => feature_gone(*id),
            Some(Task::Group {
                editing: Some(id), ..
            }) => feature_gone(*id),
            _ => false,
        };
        if !gone {
            return;
        }
        match self.tab.task.take() {
            Some(Task::Move {
                placements, turns, ..
            }) => {
                self.put_back_left(ctx.document, &placements);
                crate::coupling::set_turn_counts(ctx.document, &turns);
            }
            Some(Task::Joint { placements, .. } | Task::Coupling { placements, .. }) => {
                self.put_back_left(ctx.document, &placements)
            }
            _ => {}
        }
        self.tab.left.clear();
        self.tab.handles.reset();
        ctx.log_warn("The task's body or joint is gone; the task is closed");
    }

    /// Where the open task leaves the bodies it keeps start placements
    /// for, as one of the bench's hooks returns; nothing without such a
    /// task.
    pub(crate) fn note_left(&mut self, document: &Document) {
        self.tab.left.clear();
        if let Some(
            Task::Move { placements, .. }
            | Task::Joint { placements, .. }
            | Task::Coupling { placements, .. },
        ) = &self.tab.task
        {
            self.tab.left.extend(
                placements
                    .iter()
                    .filter(|(body, _)| document.body(*body).is_some())
                    .map(|(body, _)| (*body, document.body_placement(*body))),
            );
        }
    }

    /// Each body back at its start placement where it still is where the
    /// task left it (at its start, when nothing was noted for it).
    fn put_back_left(&self, document: &mut Document, starts: &[(BodyId, BodyPlacement)]) {
        for (body, start) in starts {
            let left = self
                .tab
                .left
                .iter()
                .find(|(b, _)| b == body)
                .map_or(*start, |(_, at)| *at);
            if document.body(*body).is_some() && document.body_placement(*body) == left {
                document.set_body_placement(*body, *start);
            }
        }
    }
}
