//! What the bench holds for the document on screen: its task, picking,
//! held body and handles, and the checks running on it. The host keeps
//! it with the tab on a switch and gives it back when the tab returns, so
//! a Cancel puts back only its own document's bodies.

use core_document::FeatureId;

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
