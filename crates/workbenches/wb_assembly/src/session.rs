//! What the bench holds for the document on screen: its task, picking,
//! held body and handles, and the checks running on it. The host keeps
//! it with the tab on a switch and gives it back when the tab returns, so
//! a Cancel puts back only its own document's bodies.

use core_document::{BodyId, BodyPlacement, Document, FeatureId, WorkbenchRuntimeContext};

use crate::interference::Interference;
use crate::job::Job;
use crate::mass::MassReport;
use crate::sweep_check::MotionClash;
use crate::{AssemblyWorkbench, Grab, Picking, Task, handles};

#[derive(Default)]
pub(crate) struct Session {
    picking: Option<Picking>,
    seen: Option<(uuid::Uuid, [u32; 3])>,
    task: Option<Task>,
    left: Vec<(BodyId, BodyPlacement)>,
    verdict: Option<Result<String, String>>,
    grab: Option<Grab>,
    handles: handles::Handles,
    group_seen: Option<uuid::Uuid>,
    checking: Option<Job<Interference>>,
    measuring: Option<Job<MassReport>>,
    sweeping: Option<(FeatureId, Job<Vec<MotionClash>>)>,
    motion_clashes: Option<(FeatureId, Result<Vec<MotionClash>, String>)>,
    #[cfg(feature = "egui")]
    playing: Option<crate::Play>,
}

impl AssemblyWorkbench {
    /// The document's state taken out, leaving the bench as if none
    /// were open.
    pub(crate) fn take_session(&mut self) -> Session {
        use std::mem::take;
        Session {
            picking: take(&mut self.picking),
            seen: take(&mut self.seen),
            task: take(&mut self.task),
            left: take(&mut self.left),
            verdict: take(&mut self.verdict),
            grab: take(&mut self.grab),
            handles: take(&mut self.handles),
            group_seen: take(&mut self.group_seen),
            checking: take(&mut self.checking),
            measuring: take(&mut self.measuring),
            sweeping: take(&mut self.sweeping),
            motion_clashes: take(&mut self.motion_clashes),
            #[cfg(feature = "egui")]
            playing: take(&mut self.playing),
        }
    }

    /// A document's state put back. What was worked out by the edit
    /// count of the document before is forgotten: another document's
    /// counts say nothing of this one.
    pub(crate) fn put_session(&mut self, session: Session) {
        self.picking = session.picking;
        self.seen = session.seen;
        self.task = session.task;
        self.left = session.left;
        self.verdict = session.verdict;
        self.grab = session.grab;
        self.handles = session.handles;
        self.group_seen = session.group_seen;
        self.checking = session.checking;
        self.measuring = session.measuring;
        self.sweeping = session.sweeping;
        self.motion_clashes = session.motion_clashes;
        #[cfg(feature = "egui")]
        {
            self.playing = session.playing;
        }
        for cache in [&self.redundant, &self.unmet] {
            *cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        *self.freedom.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
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
        if self.grab.as_ref().is_some_and(|g| body_gone(g.body)) {
            self.grab = None;
        }
        if let Some(picking) = &mut self.picking {
            if picking.repick.is_some_and(feature_gone) {
                self.picking = None;
            } else if picking.first.as_ref().is_some_and(|f| body_gone(f.0)) {
                picking.first = None;
                picking.tab = None;
                picking.slot = None;
            } else if picking.slot.as_ref().is_some_and(|s| body_gone(s.0)) {
                picking.slot = None;
            }
        }
        let gone = match &self.task {
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
        match self.task.take() {
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
        self.left.clear();
        self.handles.reset();
        ctx.log_warn("The task's body or joint is gone; the task is closed");
    }

    /// Where the open task leaves the bodies it keeps start placements
    /// for, as one of the bench's hooks returns; nothing without such a
    /// task.
    pub(crate) fn note_left(&mut self, document: &Document) {
        self.left.clear();
        if let Some(
            Task::Move { placements, .. }
            | Task::Joint { placements, .. }
            | Task::Coupling { placements, .. },
        ) = &self.task
        {
            self.left.extend(
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
