//! Per-user undo over the op stream.
//!
//! No snapshots: every entry is the (op, inverse) pairs of one user
//! gesture. Undo applies the inverses in reverse order; redo re-applies the
//! forward ops. Both go through [`Document::apply_history_op`], so a
//! history jump IS ordinary ops: it marks the affected features dirty here
//! and flows to the document server (and any peers) like any other edit.
//! That is what makes this undo safe in a shared document: it never
//! replaces state wholesale, it edits forward, and it only edits what THIS
//! user touched.
//!
//! Non-invertible ops (imports, asset registration) are barriers: the
//! journal clears rather than store an inverse that would lie.
//!
//! A script's run and the person at the window can both edit while the
//! run is open (a task opened during it, an edit between its calls). The
//! journal keeps whose each edit is, and each takes back only its own: a
//! failed run its calls' edits, a cancelled task the task's. One is taken
//! back by its inverses beneath the other's later edits only where those
//! touch none of what it touched ([`touches`]); where they do, the later
//! edits were made on top of it, and it is kept with them instead.
//!
//! Entries keep feature payloads as compressed JSON text ([`packed`]),
//! written on a thread of its own after the step closes, the tree built
//! again when a step is applied.

mod packed;
mod touches;

use crate::Document;
use crate::op::DocumentOp;
use packed::{Kept, Packer};
use touches::Touches;

/// Names one closed gesture of a journal: every op it took, however they
/// coalesced, under one identity that undo and redo carry with the entry.
/// No other gesture, in this journal or another tab's, has the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JournalEntryId(uuid::Uuid);

/// One undoable user gesture.
#[derive(Debug, Clone)]
struct Entry {
    id: JournalEntryId,
    label: String,
    forward: Vec<Kept>,
    /// Stored in capture order; applied back-to-front on undo.
    inverse: Vec<Kept>,
}

/// A move of a journal's stacks while a run is open, kept so a run that
/// fails can be taken back exactly.
#[derive(Debug)]
enum RunMove {
    /// A call's step closed onto the undo stack.
    Closed(JournalEntryId),
    /// A step of the person's closed onto the undo stack: edits made
    /// between the run's calls, or a task accepted while the run was open.
    Person(JournalEntryId),
    /// The newest step went from undo to redo.
    Undone,
    /// The newest undone step went back from redo to undo.
    Redone,
    /// A step closed and cleared these from redo.
    RedoCleared(Vec<Entry>),
}

/// A script's run: its calls close steps of their own, so it can step
/// back through them, and become one step when it ends.
#[derive(Debug, Default)]
struct Run {
    /// The step the whole run becomes.
    label: Option<String>,
    /// The steps its calls closed.
    made: Vec<JournalEntryId>,
    /// How its stacks moved, oldest first.
    moves: Vec<RunMove>,
    /// A non-invertible op crossed: nothing before it can be taken back.
    barrier: bool,
    /// It opened inside a task, and is part of it until the task ends:
    /// the task's Cancel takes back the run's edits made in it, its OK
    /// keeps them in its step.
    in_task: bool,
}

/// What [`OpJournal::roll_back`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RolledBack {
    /// Edits taken back.
    pub undone: usize,
    /// A non-invertible op (an import) crossed: only what followed it was
    /// taken back, and the history cleared.
    pub barrier: bool,
    /// The run's edits were kept, nothing taken back: the person edited
    /// something they touched after them, so taking them back would take
    /// the person's edits too, or apply beneath them. They stay with what
    /// the person is doing (an open task) or as steps.
    pub kept: bool,
}

/// Which way two kinds of edits must not meet for one of them to move
/// past or be taken back beneath the other.
#[derive(Debug, Clone, Copy)]
enum Meet {
    /// A person's edit after a run's that touches what it touched.
    PersonAfterRun,
    /// A run's edit after a person's that touches what it touched.
    RunAfterPerson,
    /// Either.
    Either,
}

/// The op journal: bounded per-user undo/redo stacks.
#[derive(Debug, Default)]
pub struct OpJournal {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    limit: usize,
    /// Label for the next boundary, set by explicit commits ("Create
    /// body"); gestures without one get a generic label.
    pending_label: Option<String>,
    /// A script's run is open: boundaries wait, its calls close steps of
    /// their own, and it ends as one step.
    run: Option<Run>,
    /// A task is open: its edits wait for it to end, as one step when it
    /// is accepted and taken back when it is cancelled.
    task: bool,
    packer: Packer,
}

impl OpJournal {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            ..Self::default()
        }
    }

    /// Wait until every step closed so far holds its payloads packed,
    /// as it does a moment after closing: what a test or a bench
    /// measures the history's memory after.
    #[doc(hidden)]
    pub fn settle(&self) {
        packed::settle();
    }

    /// Pack the payloads of steps as they close, on this thread, rather
    /// than on the packing thread: what a bench compares against.
    #[doc(hidden)]
    pub fn pack_on_this_thread(&mut self, here: bool) {
        self.packer.set_background(!here);
    }

    /// Name the gesture the next boundary closes.
    pub fn label_next(&mut self, label: impl Into<String>) {
        self.pending_label = Some(label.into());
    }

    /// Open a script's run, named by the label given last: boundaries
    /// wait until [`end_run`](Self::end_run), each call closes a step of
    /// its own ([`note_call`](Self::note_call)) that undo and redo step
    /// through as any other, and the run ends as one step. What each call
    /// records, from [`begin_call`](Self::begin_call) to its `note_call`,
    /// is the run's; the rest (an open task's edits, edits in the window
    /// between calls) is the person's: a run rolled back takes back only
    /// its own.
    pub fn begin_run(&mut self, document: &mut Document) {
        if self.run.is_none() {
            document.journal_pending_mut().begin_run();
            self.run = Some(Run {
                label: self.pending_label.take(),
                in_task: self.task,
                ..Run::default()
            });
        }
    }

    /// One of the open run's calls begins: what is recorded until
    /// [`note_call`](Self::note_call) is the run's, not the person's.
    pub fn begin_call(&mut self, document: &mut Document) {
        if self.run.is_some() {
            document.journal_pending_mut().in_call(true);
        }
    }

    /// Whether the open run crossed a non-invertible op (an import), so
    /// no undo takes it back to where it began.
    pub fn run_crossed_barrier(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.barrier)
    }

    /// Close what a script's call did as a step named `label`, in a run
    /// or not; nothing closes while a task is open. Answers the entry it
    /// closed, if any.
    pub fn note_call(
        &mut self,
        document: &mut Document,
        label: impl Into<String>,
    ) -> Option<JournalEntryId> {
        let closed = match self.task {
            true => None,
            false => {
                self.pending_label = Some(label.into());
                let closed = self.close(document);
                self.pending_label = None;
                closed
            }
        };
        document.journal_pending_mut().in_call(false);
        closed
    }

    /// End the open run: what is still open closes (unless a task holds
    /// it), and the newest steps its calls closed become one step, named
    /// as the run is. Steps it undid or redid stay where it left them.
    /// The run's edits a task opened during it holds join the run's step
    /// when they and the task's touch nothing in common, and stay the
    /// task's otherwise; a run opened inside a task leaves them all to
    /// it. Answers the run's step.
    pub fn end_run(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        let in_task = self.run.as_ref()?.in_task;
        if !self.task {
            self.close(document);
        } else if !in_task && !self.pending_meet(document, Meet::Either) {
            let (pairs, barrier) = document.journal_pending_mut().take_by(true);
            if barrier {
                self.cross_barrier();
            } else {
                self.push_step(pairs, "Edit".to_string(), true);
            }
        }
        document.journal_pending_mut().end_run();
        let run = self.run.take()?;
        let ours = self
            .undo
            .iter()
            .rev()
            .take_while(|entry| run.made.contains(&entry.id))
            .count();
        let steps = self.undo.split_off(self.undo.len() - ours);
        let mut id = None;
        if !steps.is_empty() {
            let mut forward = Vec::new();
            let mut inverse = Vec::new();
            for entry in steps {
                forward.extend(entry.forward);
                inverse.extend(entry.inverse);
            }
            let made = JournalEntryId(uuid::Uuid::new_v4());
            self.undo.push(Entry {
                id: made,
                label: run.label.unwrap_or_else(|| "Edit".to_string()),
                forward,
                inverse,
            });
            id = Some(made);
        }
        self.trim();
        id
    }

    /// Close the current gesture: drain captured pairs into one entry.
    /// Called at gesture boundaries: every frame with no mouse button held,
    /// and immediately after discrete commands. A barrier clears history.
    /// Nothing closes while a run or a task is open. Answers the entry it
    /// closed, if any.
    pub fn note(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        if self.run.is_some() || self.task {
            return None;
        }
        self.close(document)
    }

    /// Drop the oldest steps past the limit, unless a run is open and may
    /// still take its moves back.
    fn trim(&mut self) {
        if self.run.is_none() && self.undo.len() > self.limit {
            let over = self.undo.len() - self.limit;
            self.undo.drain(..over);
        }
    }

    /// Everything before a non-invertible op is out of reach: the history
    /// clears, and an open run can take back only what follows.
    fn cross_barrier(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending_label = None;
        if let Some(run) = &mut self.run {
            run.made.clear();
            run.moves.clear();
            run.barrier = true;
        }
    }

    /// Close what is open as a step. With a run open, the person's edits
    /// among it (made between the run's calls) close first as a step of
    /// their own, unless they touch what the run's before them touched:
    /// then they were made on the run's, and all of it is the person's.
    fn close(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        if self.run.is_none() {
            let (pairs, barrier) = document.take_journal_pairs();
            if barrier {
                self.cross_barrier();
                return None;
            }
            if pairs.is_empty() {
                return None;
            }
            let label = self.take_label();
            return self.push_step(pairs, label, false);
        }
        if self.pending_meet(document, Meet::PersonAfterRun) {
            document.journal_pending_mut().hand_over();
        }
        let (person, barrier) = document.journal_pending_mut().take_by(false);
        let (run, crossed) = document.journal_pending_mut().take_by(true);
        if barrier || crossed {
            self.cross_barrier();
            return None;
        }
        if person.is_empty() && run.is_empty() {
            return None;
        }
        let label = self.take_label();
        if run.is_empty() {
            return self.push_step(person, label, false);
        }
        self.push_step(person, "Edit".to_string(), false);
        self.push_step(run, label, true)
    }

    /// The label given for the next step, or the generic one.
    fn take_label(&mut self) -> String {
        self.pending_label
            .take()
            .unwrap_or_else(|| "Edit".to_string())
    }

    /// Push `pairs` as a step named `label`, the open run's (`by_run`) or
    /// the person's.
    fn push_step(
        &mut self,
        pairs: Vec<(DocumentOp, DocumentOp)>,
        label: String,
        by_run: bool,
    ) -> Option<JournalEntryId> {
        // Moving a body's tip alone is moving through its history, as
        // selecting a feature does: not an edit to undo. A tip moved along
        // with an edit (a feature inserted at it) goes with the edit.
        if pairs
            .iter()
            .all(|(op, _)| matches!(op, DocumentOp::SetBodyTip { .. }))
        {
            return None;
        }
        let (forward, inverse): (Vec<_>, Vec<_>) = pairs
            .into_iter()
            .map(|(op, inverse)| (self.packer.pack(op), self.packer.pack(inverse)))
            .unzip();
        let id = JournalEntryId(uuid::Uuid::new_v4());
        self.undo.push(Entry {
            id,
            label,
            forward,
            inverse,
        });
        // A fresh edit forks history; the old future is unreachable.
        let cleared = std::mem::take(&mut self.redo);
        if let Some(run) = &mut self.run {
            if by_run {
                run.made.push(id);
                if !cleared.is_empty() {
                    run.moves.push(RunMove::RedoCleared(cleared));
                }
                run.moves.push(RunMove::Closed(id));
            } else {
                // The person's step forks the history for good: a run
                // rolled back past it leaves redo empty.
                run.moves.push(RunMove::Person(id));
            }
        }
        self.trim();
        Some(id)
    }

    /// Whether the edits not yet closed hold a run's and a person's that
    /// meet as `meet` says.
    fn pending_meet(&self, document: &Document, meet: Meet) -> bool {
        let mut run = Touches::default();
        let mut person = Touches::default();
        for (op, inverse, by_run) in document.journal_pending().tagged() {
            let touches = touches::of_pair(op, inverse);
            let (earlier, mine) = match by_run {
                true => (&person, &mut run),
                false => (&run, &mut person),
            };
            let checked = match meet {
                Meet::PersonAfterRun => !by_run,
                Meet::RunAfterPerson => by_run,
                Meet::Either => true,
            };
            if checked && touches.meets(earlier) {
                return true;
            }
            mine.absorb(touches);
        }
        if let Meet::Either = meet {
            return run.meets(&person);
        }
        false
    }

    /// Whether the person edited, after the open run's edits, something
    /// they touched: its closed steps in the order they closed, then the
    /// edits not yet closed. A run that undid or redid a step counts as
    /// having touched everything.
    fn run_tangled(&mut self, document: &Document) -> bool {
        let Some(run) = &self.run else {
            return false;
        };
        let mut ours = Touches::default();
        for step in &run.moves {
            match step {
                RunMove::Closed(id) => match self.undo.iter().find(|e| e.id == *id) {
                    Some(entry) => ours.absorb(entry_touches(&mut self.packer, entry)),
                    None => ours.add_all(),
                },
                RunMove::Person(id) => {
                    let Some(entry) = self.undo.iter().find(|e| e.id == *id) else {
                        return true;
                    };
                    if entry_touches(&mut self.packer, entry).meets(&ours) {
                        return true;
                    }
                }
                RunMove::Undone | RunMove::Redone => ours.add_all(),
                RunMove::RedoCleared(_) => {}
            }
        }
        for (op, inverse, by_run) in document.journal_pending().tagged() {
            let touches = touches::of_pair(op, inverse);
            if by_run {
                ours.absorb(touches);
            } else if touches.meets(&ours) {
                return true;
            }
        }
        false
    }

    /// Take back everything done since the last boundary and leave no
    /// step for it: a script that stopped with an error puts the document
    /// back as it found it. With a run open that is everything the run
    /// did, its undos and redos too, and the run ends leaving no step;
    /// the person's edits made meanwhile (an open task's, steps between
    /// its calls) stay, the run's taken back beneath them. When the person
    /// edited, after the run, something it touched, nothing is taken back
    /// and the run stays open for [`end_run`](Self::end_run), its edits
    /// handed to the person's ([`RolledBack::kept`]). Across a
    /// non-invertible op (an import) only what followed it is taken back
    /// and the history clears as it would have on closing.
    pub fn roll_back(&mut self, document: &mut Document) -> RolledBack {
        if self.run.is_none() {
            let (undone, barrier) = self.roll_back_pending(document);
            return RolledBack {
                undone,
                barrier,
                kept: false,
            };
        }
        if self.run_tangled(document) {
            document.journal_pending_mut().hand_over();
            return RolledBack {
                undone: 0,
                barrier: self.run_crossed_barrier(),
                kept: true,
            };
        }
        let taken = document.journal_pending_mut().take_by(true);
        document.journal_pending_mut().end_run();
        let (mut undone, mut barrier) = self.take_back(document, taken);
        let Some(run) = self.run.take() else {
            return RolledBack {
                undone,
                barrier,
                kept: false,
            };
        };
        barrier |= run.barrier;
        // The person's steps stay on top, the run's beneath them taken back.
        let mut held = Vec::new();
        for step in run.moves.into_iter().rev() {
            match step {
                RunMove::Closed(_) => {
                    if let Some(entry) = self.undo.pop() {
                        undone += apply_back(document, &mut self.packer, &entry);
                    }
                }
                RunMove::Person(_) => held.extend(self.undo.pop()),
                RunMove::Undone => {
                    if let Some(entry) = self.redo.pop() {
                        undone += apply_forward(document, &mut self.packer, &entry);
                        self.undo.push(entry);
                    }
                }
                RunMove::Redone => {
                    if let Some(entry) = self.undo.pop() {
                        undone += apply_back(document, &mut self.packer, &entry);
                        self.redo.push(entry);
                    }
                }
                RunMove::RedoCleared(entries) => self.redo = entries,
            }
        }
        if !held.is_empty() {
            self.redo.clear();
            self.undo.extend(held.into_iter().rev());
        }
        self.trim();
        RolledBack {
            undone,
            barrier,
            kept: false,
        }
    }

    /// Take back the edits not yet closed, leaving no step for them.
    fn roll_back_pending(&mut self, document: &mut Document) -> (usize, bool) {
        let taken = document.take_journal_pairs();
        self.take_back(document, taken)
    }

    fn take_back(
        &mut self,
        document: &mut Document,
        (pairs, barrier): (Vec<(DocumentOp, DocumentOp)>, bool),
    ) -> (usize, bool) {
        document.without_journal(|doc| {
            for (_, inverse) in pairs.iter().rev() {
                doc.apply_history_op(inverse);
            }
        });
        self.pending_label = None;
        if barrier {
            self.cross_barrier();
        }
        (pairs.len(), barrier)
    }

    /// Open a task: what is done from here on, the edits not yet closed
    /// with it, waits for the task to end. No boundary closes it
    /// meanwhile, and no tool step is undone inside it.
    pub fn begin_task(&mut self) {
        self.task = true;
    }

    /// Whether a task is open.
    pub fn in_task(&self) -> bool {
        self.task
    }

    /// End the open task keeping its edits: one step, named `label` when
    /// given. With a script's run open the step is the person's, not the
    /// run's: the run's edits made while the task was open stay the run's,
    /// unless the task's touch what they touched after them (or the run
    /// opened inside the task), in which case they go in the task's step.
    pub fn accept_task(
        &mut self,
        document: &mut Document,
        label: Option<String>,
    ) -> Option<JournalEntryId> {
        self.task = false;
        if let Some(label) = label {
            self.label_next(label);
        }
        let Some(run) = &mut self.run else {
            return self.note(document);
        };
        if std::mem::take(&mut run.in_task) || self.pending_meet(document, Meet::PersonAfterRun) {
            document.journal_pending_mut().hand_over();
        }
        let (pairs, barrier) = document.journal_pending_mut().take_by(false);
        if barrier {
            self.cross_barrier();
            return None;
        }
        let label = self.take_label();
        self.push_step(pairs, label, false)
    }

    /// End the open task taking back everything done while it was open,
    /// leaving no step: the document is as the task found it. With a
    /// script's run open, the run's edits made meanwhile stay, unless
    /// they touch what the task's touched before them (or the run opened
    /// inside the task): then they were made on the task's and go back
    /// with them. Answers as [`roll_back`](Self::roll_back) does: across a
    /// non-invertible op only what followed it is taken back.
    pub fn cancel_task(&mut self, document: &mut Document) -> (usize, bool) {
        self.task = false;
        let Some(run) = &mut self.run else {
            return self.roll_back_pending(document);
        };
        if std::mem::take(&mut run.in_task) || self.pending_meet(document, Meet::RunAfterPerson) {
            return self.roll_back_pending(document);
        }
        let taken = document.journal_pending_mut().take_by(false);
        self.take_back(document, taken)
    }

    /// Undo the newest gesture. Returns its label.
    pub fn undo(&mut self, document: &mut Document) -> Option<String> {
        // Fold any uncommitted edits first so they are what gets undone,
        // in a run or not.
        self.close(document);
        let entry = self.undo.pop()?;
        apply_back(document, &mut self.packer, &entry);
        let label = entry.label.clone();
        self.redo.push(entry);
        if let Some(run) = &mut self.run {
            run.moves.push(RunMove::Undone);
        }
        Some(label)
    }

    /// Re-apply the most recently undone gesture. Returns its label.
    pub fn redo(&mut self, document: &mut Document) -> Option<String> {
        let entry = self.redo.pop()?;
        apply_forward(document, &mut self.packer, &entry);
        let label = entry.label.clone();
        self.undo.push(entry);
        if let Some(run) = &mut self.run {
            run.moves.push(RunMove::Redone);
        }
        Some(label)
    }

    /// Forget everything: the journal goes on with a different document.
    pub fn reset(&mut self, document: &mut Document) {
        let _ = document.take_journal_pairs();
        document.journal_pending_mut().end_run();
        self.undo.clear();
        self.redo.clear();
        self.pending_label = None;
        self.task = false;
        self.run = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The newest closed gesture, the one undo would take back; edits not
    /// yet closed are not part of it.
    pub fn undo_entry_id(&self) -> Option<JournalEntryId> {
        self.undo.last().map(|entry| entry.id)
    }

    /// Undo the gesture `id` names, only while it is the newest: edits not
    /// yet closed close first, so they are newer and refuse it. Nothing
    /// changes while a run or a task is open. Answers the label undone.
    pub fn undo_entry(&mut self, document: &mut Document, id: JournalEntryId) -> Option<String> {
        if self.run.is_some() || self.task {
            return None;
        }
        self.close(document);
        if self.undo_entry_id() != Some(id) {
            return None;
        }
        self.undo(document)
    }
}

/// What `entry`'s ops and their inverses touch.
fn entry_touches(packer: &mut Packer, entry: &Entry) -> Touches {
    let mut touches = Touches::default();
    for kept in entry.forward.iter().chain(&entry.inverse) {
        touches.add(&packer.unpack(kept));
    }
    touches
}

/// Take `entry` back: its inverses, newest first. Answers how many.
fn apply_back(document: &mut Document, packer: &mut Packer, entry: &Entry) -> usize {
    document.without_journal(|doc| {
        for kept in entry.inverse.iter().rev() {
            doc.apply_history_op(&packer.unpack(kept));
        }
    });
    entry.inverse.len()
}

/// Do `entry` again: its ops, oldest first. Answers how many.
fn apply_forward(document: &mut Document, packer: &mut Packer, entry: &Entry) -> usize {
    document.without_journal(|doc| {
        for kept in &entry.forward {
            doc.apply_history_op(&packer.unpack(kept));
        }
    });
    entry.forward.len()
}

/// Run `task` as the host runs one that ends by its Cancel: what was
/// done before closes as a step, the task opens in a journal, `task`
/// edits and cancels, and the journal takes back what it did. Panics
/// unless the task changed the document, its Cancel left it as it was
/// (by the replicated projection) and no undo step was added. The
/// benches' and the app's task tests share it.
#[doc(hidden)]
pub fn assert_cancel_takes_back(document: &mut Document, task: impl FnOnce(&mut Document)) {
    let mut journal = OpJournal::new(16);
    journal.note(document);
    let before = document.replicated_projection();
    let newest = journal.undo_entry_id();
    journal.begin_task();
    task(document);
    assert_ne!(
        document.replicated_projection(),
        before,
        "the task changed nothing to take back"
    );
    let (undone, barrier) = journal.cancel_task(document);
    assert!(
        undone > 0 && !barrier,
        "{undone} taken back, barrier {barrier}"
    );
    assert_eq!(
        document.replicated_projection(),
        before,
        "Cancel left the document as the task found it"
    );
    journal.note(document);
    assert_eq!(journal.undo_entry_id(), newest, "Cancel left no step");
    assert!(!journal.can_redo(), "nor anything to redo");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BasePlane;
    use crate::datum::{DatumAttachment, DatumFeature, DatumShape};

    fn datum() -> DatumFeature {
        DatumFeature {
            shape: DatumShape::Plane { size: 10.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: Default::default(),
        }
    }

    #[test]
    fn undo_and_redo_walk_the_op_stream_both_ways() {
        let mut doc = Document::new("Journal");
        let mut journal = OpJournal::new(16);

        let body = doc.create_body(Some("Base".into()));
        journal.label_next("Create body");
        journal.note(&mut doc);

        doc.rename_body(body, "Renamed");
        journal.note(&mut doc);
        assert_eq!(doc.bodies()[0].name, "Renamed");

        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Edit"));
        assert_eq!(doc.bodies()[0].name, "Base", "rename undone");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        assert!(doc.bodies().is_empty(), "creation undone");

        assert_eq!(journal.redo(&mut doc).as_deref(), Some("Create body"));
        assert_eq!(doc.bodies().len(), 1);
        assert_eq!(journal.redo(&mut doc).as_deref(), Some("Edit"));
        assert_eq!(doc.bodies()[0].name, "Renamed");
    }

    #[test]
    fn a_package_writing_a_feature_again_is_undone_with_its_edit() {
        let mut doc = Document::new("Package");
        let mut journal = OpJournal::new(16);
        let thing = doc.add_feature_of_kind(
            crate::WorkbenchId::new("acme.cam.op"),
            "Op".into(),
            None,
            Vec::new(),
            serde_json::json!({"v": 1}),
            crate::FeatureOrigin::new("acme.cam 0.1.0", Some("acme/cam".into())),
        );
        journal.note(&mut doc);

        doc.update_feature_data(thing, serde_json::json!({"v": 2}))
            .unwrap();
        doc.set_feature_origin(
            thing,
            crate::FeatureOrigin::new("acme.cam 0.2.0", Some("acme/cam".into())),
        )
        .unwrap();
        journal.note(&mut doc);
        let node = doc.get_feature_meta(thing).unwrap();
        assert_eq!(node.made_by.as_deref(), Some("acme.cam 0.2.0"));

        journal.undo(&mut doc);
        let node = doc.get_feature_meta(thing).unwrap();
        assert_eq!(
            node.made_by.as_deref(),
            Some("acme.cam 0.1.0"),
            "the older writer is back"
        );
        assert_eq!(*node.data, serde_json::json!({"v": 1}));
        assert_eq!(node.package_source.as_deref(), Some("acme/cam"));
    }

    #[test]
    fn a_run_ends_as_one_step_whatever_its_calls_closed() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        let body = doc.create_body(Some("A".into()));
        journal.note(&mut doc);
        doc.rename_body(body, "B");
        journal.note_call(&mut doc, "doc.rename");
        journal.begin_call(&mut doc);
        doc.create_body(Some("C".into()));
        journal.end_run(&mut doc);
        journal.note(&mut doc);

        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert!(doc.bodies().is_empty(), "the whole run went back at once");
        assert!(!journal.can_undo());
    }

    /// A run that fails while a task is open takes back only what it did:
    /// the feature the task made, and the task's edit of it, stay, and
    /// accepting the task makes them one step. A run's write to the same
    /// feature does not join the task's.
    #[test]
    fn a_run_rolled_back_in_an_open_task_leaves_the_task_s_edits() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(Some("A".into()));
        journal.label_next("Create body");
        journal.note(&mut doc);

        journal.begin_task();
        let made = doc
            .add_feature_in_body(datum(), "D".into(), Some(body))
            .unwrap();
        doc.update_feature_data(made, serde_json::json!({"task": 1}))
            .unwrap();
        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.update_feature_data(made, serde_json::json!({"run": 2}))
            .unwrap();
        doc.rename_body(body, "B");
        journal.note_call(&mut doc, "doc.rename");
        let RolledBack {
            undone, barrier, ..
        } = journal.roll_back(&mut doc);
        assert!(undone == 2 && !barrier, "{undone} {barrier}");
        journal.end_run(&mut doc);

        let node = doc
            .get_feature_meta(made)
            .expect("the task's feature stays");
        assert_eq!(*node.data, serde_json::json!({"task": 1}));
        assert_eq!(doc.bodies()[0].name, "A");
        journal.accept_task(&mut doc, Some("Datum".into()));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Datum"));
        assert!(doc.get_feature_meta(made).is_none(), "the task's one step");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
    }

    /// A run that ends well inside a task is part of it: Cancel takes it
    /// back with the rest.
    #[test]
    fn cancelling_a_task_takes_back_a_run_inside_it() {
        let mut doc = Document::new("Script");
        let body = doc.create_body(Some("A".into()));
        assert_cancel_takes_back(&mut doc, |doc| {
            let mut journal = OpJournal::new(16);
            journal.begin_task();
            doc.add_feature_in_body(datum(), "D".into(), Some(body))
                .unwrap();
            journal.begin_run(doc);
            journal.begin_call(doc);
            doc.rename_body(body, "B");
            journal.note_call(doc, "doc.rename");
            journal.end_run(doc);
        });
    }

    /// A document with a body for a task and one for a run, made as one
    /// step, and a journal holding it.
    fn task_and_run_bodies() -> (Document, OpJournal, crate::BodyId, crate::BodyId) {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        let task_body = doc.create_body(Some("A".into()));
        let run_body = doc.create_body(Some("K".into()));
        journal.label_next("Create bodies");
        journal.note(&mut doc);
        (doc, journal, task_body, run_body)
    }

    fn names(doc: &Document) -> Vec<String> {
        doc.bodies().iter().map(|b| b.name.clone()).collect()
    }

    fn data(doc: &Document, id: crate::FeatureId) -> Option<serde_json::Value> {
        doc.get_feature_meta(id).map(|node| (*node.data).clone())
    }

    /// A run that began before a task opened and fails while it is open
    /// takes back its own calls, the one closed before the task and the
    /// one the task held, and leaves the task's feature and its edits
    /// made before and after the run's call; the task still ends as one
    /// step and the run leaves none.
    #[test]
    fn a_failed_run_begun_before_a_task_leaves_the_task_s_edits() {
        let (mut doc, mut journal, task_body, run_body) = task_and_run_bodies();
        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K1");
        journal.note_call(&mut doc, "doc.rename");

        journal.begin_task();
        let made = doc
            .add_feature_in_body(datum(), "D".into(), Some(task_body))
            .unwrap();
        doc.update_feature_data(made, serde_json::json!({"task": 1}))
            .unwrap();
        journal.begin_call(&mut doc);
        doc.create_body(Some("R".into()));
        assert_eq!(journal.note_call(&mut doc, "doc.new_body"), None);
        doc.update_feature_data(made, serde_json::json!({"task": 2}))
            .unwrap();

        let rolled = journal.roll_back(&mut doc);
        assert_eq!(
            rolled,
            RolledBack {
                undone: 2,
                barrier: false,
                kept: false
            }
        );
        assert_eq!(journal.end_run(&mut doc), None, "the run left no step");
        assert_eq!(names(&doc), ["A", "K"], "both of the run's calls went");
        assert_eq!(data(&doc, made), Some(serde_json::json!({"task": 2})));

        journal.accept_task(&mut doc, Some("Datum".into()));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Datum"));
        assert_eq!(data(&doc, made), None, "the task's one step");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create bodies"));
        assert!(!journal.can_undo());
    }

    /// A task opened during a run and cancelled takes back only its own
    /// edits: the run's calls before and while it was open stay, and the
    /// run still ends as one step.
    #[test]
    fn cancelling_a_task_opened_during_a_run_takes_back_only_its_edits() {
        let (mut doc, mut journal, task_body, run_body) = task_and_run_bodies();
        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K1");
        journal.note_call(&mut doc, "doc.rename");

        journal.begin_task();
        let made = doc
            .add_feature_in_body(datum(), "D".into(), Some(task_body))
            .unwrap();
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K2");
        journal.note_call(&mut doc, "doc.rename");
        let (undone, barrier) = journal.cancel_task(&mut doc);
        assert!(undone > 0 && !barrier, "{undone} {barrier}");
        assert_eq!(data(&doc, made), None, "the task's feature went");
        assert_eq!(names(&doc), ["A", "K2"], "the run's calls stay");

        journal.end_run(&mut doc);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(names(&doc), ["A", "K"]);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create bodies"));
    }

    /// A run that ends well while a task opened during it is still open
    /// is one step, its call in the task with it; the task accepted is
    /// another, above it, holding only the task's edits.
    #[test]
    fn a_run_and_a_task_opened_during_it_are_a_step_each() {
        let (mut doc, mut journal, task_body, run_body) = task_and_run_bodies();
        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K1");
        journal.note_call(&mut doc, "doc.rename");

        journal.begin_task();
        let made = doc
            .add_feature_in_body(datum(), "D".into(), Some(task_body))
            .unwrap();
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K2");
        journal.note_call(&mut doc, "doc.rename");
        assert!(journal.end_run(&mut doc).is_some());
        journal.accept_task(&mut doc, Some("Datum".into()));

        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Datum"));
        assert_eq!(data(&doc, made), None);
        assert_eq!(names(&doc), ["A", "K2"], "the task's step is its own");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(names(&doc), ["A", "K"], "the run's is one, both calls");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create bodies"));
    }

    /// When the task edited what the failed run made, taking the run back
    /// would take the task's work with it: nothing goes back, the run's
    /// closed call stays a step, and its call the task held is the task's
    /// from then on, so Cancel takes it back with the task's edit.
    #[test]
    fn a_failed_run_keeps_its_edits_when_the_task_built_on_them() {
        let (mut doc, mut journal, task_body, run_body) = task_and_run_bodies();
        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        let made = doc
            .add_feature_in_body(datum(), "D".into(), Some(task_body))
            .unwrap();
        journal.note_call(&mut doc, "design.datum");

        journal.begin_task();
        doc.update_feature_data(made, serde_json::json!({"task": 1}))
            .unwrap();
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K1");
        journal.note_call(&mut doc, "doc.rename");

        let rolled = journal.roll_back(&mut doc);
        assert_eq!(
            rolled,
            RolledBack {
                undone: 0,
                barrier: false,
                kept: true
            }
        );
        journal.end_run(&mut doc);
        assert_eq!(data(&doc, made), Some(serde_json::json!({"task": 1})));
        assert_eq!(names(&doc), ["A", "K1"]);

        journal.cancel_task(&mut doc);
        assert_ne!(data(&doc, made), Some(serde_json::json!({"task": 1})));
        assert_eq!(names(&doc), ["A", "K"], "the handed-over call went too");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(data(&doc, made), None);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create bodies"));
    }

    /// A run's call that edits what the task made, after it, was made on
    /// the task's work: Cancel takes it back with the task.
    #[test]
    fn cancelling_a_task_takes_back_the_run_s_edits_made_on_it() {
        let (mut doc, mut journal, task_body, run_body) = task_and_run_bodies();
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K1");
        journal.note_call(&mut doc, "doc.rename");

        journal.begin_task();
        let made = doc
            .add_feature_in_body(datum(), "D".into(), Some(task_body))
            .unwrap();
        journal.begin_call(&mut doc);
        doc.update_feature_data(made, serde_json::json!({"run": 1}))
            .unwrap();
        journal.note_call(&mut doc, "design.set");
        journal.cancel_task(&mut doc);
        assert_eq!(data(&doc, made), None);
        assert_eq!(names(&doc), ["A", "K1"], "the run's earlier call stays");
        journal.end_run(&mut doc);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Edit"));
        assert_eq!(names(&doc), ["A", "K"]);
    }

    /// The person's edits between a run's calls, with no task open, are a
    /// step of their own: a run that fails takes back its calls beneath it
    /// and leaves it.
    #[test]
    fn a_failed_run_leaves_the_person_s_edits_between_its_calls() {
        let (mut doc, mut journal, task_body, run_body) = task_and_run_bodies();
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(run_body, "K1");
        journal.note_call(&mut doc, "doc.rename");
        doc.rename_body(task_body, "A1");
        assert_eq!(journal.note(&mut doc), None, "a run holds boundaries");
        journal.begin_call(&mut doc);
        doc.create_body(Some("R".into()));
        journal.note_call(&mut doc, "doc.new_body");

        let rolled = journal.roll_back(&mut doc);
        assert!(!rolled.kept && rolled.undone == 2, "{rolled:?}");
        journal.end_run(&mut doc);
        assert_eq!(names(&doc), ["A1", "K"]);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Edit"));
        assert_eq!(names(&doc), ["A", "K"]);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create bodies"));
    }

    /// A run that fails is taken back whole and leaves no step; what was
    /// undoable before it still is.
    #[test]
    fn a_run_rolled_back_leaves_the_document_and_history_as_before() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        let kept = doc.create_body(Some("Kept".into()));
        journal.label_next("Create body");
        journal.note(&mut doc);

        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(kept, "Renamed");
        journal.note_call(&mut doc, "doc.rename");
        journal.begin_call(&mut doc);
        let body = doc.create_body(Some("A".into()));
        doc.add_feature_in_body(datum(), "D".into(), Some(body))
            .unwrap();
        journal.note(&mut doc);
        let RolledBack {
            undone, barrier, ..
        } = journal.roll_back(&mut doc);
        assert!(undone >= 3 && !barrier, "{undone} {barrier}");
        journal.note(&mut doc);

        assert_eq!(doc.bodies().len(), 1);
        assert_eq!(doc.bodies()[0].name, "Kept");
        assert_eq!(doc.feature_tree().all_nodes().count(), 0);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        assert!(!journal.can_undo(), "the run left no step");
    }

    /// In a run, undo steps back through the run's own calls one at a
    /// time and on into what came before, as the UI's Undo does; the
    /// calls left become the run's one step, and a run that fails puts
    /// back what it undid too.
    #[test]
    fn a_run_s_calls_are_steps_it_can_undo_and_redo() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(Some("A".into()));
        journal.label_next("Create body");
        journal.note(&mut doc);
        let name = |doc: &Document| doc.bodies().first().map(|b| b.name.clone());

        journal.label_next("Run script");
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        doc.rename_body(body, "B");
        journal.note_call(&mut doc, "doc.rename");
        journal.begin_call(&mut doc);
        doc.rename_body(body, "C");
        journal.note_call(&mut doc, "doc.rename");
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("doc.rename"));
        assert_eq!(name(&doc).as_deref(), Some("B"), "only the last call");
        assert_eq!(journal.redo(&mut doc).as_deref(), Some("doc.rename"));
        assert_eq!(name(&doc).as_deref(), Some("C"));
        journal.undo(&mut doc);
        journal.end_run(&mut doc);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(name(&doc).as_deref(), Some("A"));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        journal.redo(&mut doc);
        journal.redo(&mut doc);
        assert_eq!(name(&doc).as_deref(), Some("B"));

        // A run that undoes what came before it, then fails.
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        assert!(doc.bodies().is_empty());
        journal.begin_call(&mut doc);
        doc.create_body(Some("Z".into()));
        journal.note_call(&mut doc, "doc.new_body");
        assert!(!journal.can_redo(), "a call forks the history");
        journal.roll_back(&mut doc);
        assert_eq!(name(&doc).as_deref(), Some("B"));
        assert_eq!(doc.bodies().len(), 1);
        assert!(
            journal.can_redo(),
            "what the call cleared from redo is back"
        );
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        assert!(!journal.can_undo());

        // A run that only undoes leaves no step of its own: what it undid
        // redoes.
        journal.redo(&mut doc);
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        journal.undo(&mut doc);
        assert_eq!(journal.end_run(&mut doc), None);
        assert!(!journal.can_undo());
        assert_eq!(journal.redo(&mut doc).as_deref(), Some("Create body"));
    }

    /// A payload as a large sketch's is: many elements, each with ids,
    /// numbers no shorter text writes exactly, and text.
    fn payload(elements: usize, tag: f64) -> serde_json::Value {
        let items: Vec<_> = (0..elements)
            .map(|i| {
                serde_json::json!({
                    "id": uuid::Uuid::new_v4(),
                    "at": [i as f64 * 0.1 + tag, -0.0, 1e-300, f64::MAX / 3.0],
                    "count": u64::MAX - i as u64,
                    "offset": i64::MIN + i as i64,
                    "name": format!("élément {i} \"quoted\" \u{1F4D0}"),
                    "construction": i % 2 == 0,
                    "label": null,
                })
            })
            .collect();
        serde_json::json!({"geometry": items, "tag": tag})
    }

    /// Undo and redo put back the very payloads written, to the last bit
    /// of every double, however the journal keeps them; a removed
    /// feature's payload comes back with it.
    #[test]
    fn undo_and_redo_restore_payloads_bit_for_bit() {
        let mut doc = Document::new("Exact");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(None);
        let d = doc
            .add_feature_in_body(datum(), "D".into(), Some(body))
            .expect("add");
        let states: Vec<_> = (0..3).map(|k| payload(50, k as f64 + 0.3)).collect();
        for state in &states {
            doc.update_feature_data(d, state.clone()).expect("update");
            journal.note(&mut doc);
        }
        let text = |doc: &Document| doc.get_feature_data(d).expect("data").to_string();
        for state in states.iter().rev().skip(1) {
            journal.undo(&mut doc);
            assert_eq!(*doc.get_feature_data(d).unwrap(), *state);
            assert_eq!(text(&doc), state.to_string(), "every double as written");
        }
        for state in states.iter().skip(1) {
            journal.redo(&mut doc);
            assert_eq!(text(&doc), state.to_string());
        }

        doc.remove_feature(d).expect("remove");
        journal.note(&mut doc);
        journal.undo(&mut doc);
        assert_eq!(text(&doc), states[2].to_string(), "the removed payload");
        journal.redo(&mut doc);
        assert!(doc.get_feature_data(d).is_none());
    }

    /// The history keeps no payload tree of its own: each one an edit
    /// replaced is gone once its step is packed, though undo still
    /// reaches it, and one undo put back goes once the next edit replaces
    /// it.
    #[test]
    fn the_history_holds_no_payload_tree() {
        let mut doc = Document::new("Compact");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(None);
        let d = doc
            .add_feature_in_body(datum(), "D".into(), Some(body))
            .expect("add");
        doc.update_feature_data(d, payload(2000, 0.0))
            .expect("update");
        journal.note(&mut doc);
        let held = |doc: &Document| {
            std::sync::Arc::downgrade(&doc.get_feature_meta(d).expect("node").data)
        };
        let mut replaced = Vec::new();
        for k in 1..=4 {
            replaced.push(held(&doc));
            doc.update_feature_data(d, payload(2000, f64::from(k)))
                .expect("update");
            journal.note(&mut doc);
            let _ = doc.take_pending_ops();
            doc.confirm_sent_ops();
        }
        journal.settle();
        assert!(
            replaced.iter().all(|tree| tree.strong_count() == 0),
            "every replaced payload dropped"
        );

        journal.undo(&mut doc);
        assert_eq!(doc.get_feature_data(d).unwrap()["tag"], 3.0);
        let restored = held(&doc);
        doc.update_feature_data(d, payload(10, 9.0))
            .expect("update");
        journal.note(&mut doc);
        let _ = doc.take_pending_ops();
        doc.confirm_sent_ops();
        journal.settle();
        assert_eq!(restored.strong_count(), 0, "the undone-to payload too");
        journal.undo(&mut doc);
        assert_eq!(doc.get_feature_data(d).unwrap()["tag"], 3.0);
    }

    #[test]
    fn a_drag_burst_undoes_to_where_it_started() {
        let mut doc = Document::new("Drag");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(None);
        let d = doc
            .add_feature_in_body(datum(), "D".into(), Some(body))
            .expect("add");
        journal.note(&mut doc);

        for i in 0..50 {
            doc.update_feature_data(d, serde_json::json!({"x": i}))
                .expect("update");
        }
        journal.note(&mut doc);

        journal.undo(&mut doc);
        assert_eq!(
            doc.get_feature_data(d).expect("data")["x"],
            serde_json::Value::Null,
            "the drag undid to the pre-drag payload, not frame 49"
        );
    }

    /// A step's identity names the whole coalesced gesture, survives undo
    /// and redo, and undoes it only while it is the newest step.
    #[test]
    fn a_named_step_undoes_only_while_it_is_the_newest() {
        let mut doc = Document::new("Named");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(Some("A".into()));
        let made = journal.note(&mut doc).expect("a step closed");
        for name in ["B", "C", "D"] {
            doc.rename_body(body, name);
        }
        let renamed = journal.note(&mut doc).expect("one step for the burst");
        assert_ne!(made, renamed);
        assert_eq!(journal.undo_entry_id(), Some(renamed));
        assert_eq!(journal.note(&mut doc), None, "nothing new to close");

        // Older, in a run, or overtaken by an open edit: refused, nothing moves.
        assert_eq!(journal.undo_entry(&mut doc, made), None);
        journal.begin_run(&mut doc);
        journal.begin_call(&mut doc);
        assert_eq!(journal.undo_entry(&mut doc, renamed), None);
        journal.end_run(&mut doc);
        assert_eq!(doc.bodies()[0].name, "D");

        assert!(journal.undo_entry(&mut doc, renamed).is_some());
        assert_eq!(doc.bodies()[0].name, "A", "the burst went back whole");
        assert_eq!(journal.undo_entry_id(), Some(made));
        journal.redo(&mut doc);
        assert_eq!(journal.undo_entry_id(), Some(renamed), "redo keeps it");

        doc.rename_body(body, "E");
        assert_eq!(journal.undo_entry(&mut doc, renamed), None);
        assert_eq!(doc.bodies()[0].name, "E");
        assert_ne!(journal.undo_entry_id(), Some(renamed));
        assert!(!journal.can_redo(), "the open edit closed as a step");
    }

    #[test]
    fn removing_a_feature_undoes_back_with_identity_and_order_intact() {
        let mut doc = Document::new("Remove");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(None);
        let d1 = doc
            .add_feature_in_body(datum(), "D1".into(), Some(body))
            .expect("add");
        let d2 = doc
            .add_feature_in_body(datum(), "D2".into(), Some(body))
            .expect("add");
        journal.note(&mut doc);
        let seq_before = doc.feature_tree().get_node(d1).expect("d1").seq;

        doc.remove_feature(d1).expect("remove");
        journal.note(&mut doc);
        assert!(doc.feature_tree().get_node(d1).is_none());

        journal.undo(&mut doc);
        let node = doc.feature_tree().get_node(d1).expect("restored");
        assert_eq!(node.seq, seq_before, "same place in history");
        assert_eq!(node.name, "D1");
        assert!(doc.feature_tree().get_node(d2).is_some());
    }

    #[test]
    fn an_import_is_a_barrier_that_clears_history() {
        let mut doc = Document::new("Barrier");
        let mut journal = OpJournal::new(16);
        doc.create_body(None);
        journal.note(&mut doc);
        assert!(journal.can_undo());

        doc.apply_import(
            crate::AssetReference::new(
                "assets/x.step".to_string(),
                crate::AssetType::Step,
                serde_json::json!({}),
            ),
            Vec::new(),
            kernel_api::TessellationSettings::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        journal.note(&mut doc);
        assert!(!journal.can_undo(), "imports clear undo history");
    }

    /// A task holds what a tool made before it opened and every edit
    /// after: OK closes them as one step whatever boundary came between,
    /// no tool step is undone inside it, and Cancel takes everything back
    /// with no step; across an import only what followed it.
    #[test]
    fn a_task_is_one_step_on_ok_and_none_on_cancel() {
        let mut doc = Document::new("Task");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(Some("A".into()));
        journal.note(&mut doc);
        let made = journal.undo_entry_id().unwrap();

        let point = doc.add_feature_in_body(datum(), "D".into(), Some(body));
        journal.begin_task();
        doc.rename_body(body, "B");
        assert_eq!(journal.note(&mut doc), None, "no boundary inside a task");
        assert_eq!(journal.undo_entry(&mut doc, made), None);
        doc.rename_body(body, "C");
        journal.accept_task(&mut doc, Some("Create D".into()));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create D"));
        assert_eq!(doc.bodies()[0].name, "A");
        assert!(doc.get_feature_meta(point.unwrap()).is_none());

        assert_cancel_takes_back(&mut doc, |doc| {
            doc.add_feature_in_body(datum(), "D".into(), Some(body))
                .unwrap();
            doc.rename_body(body, "B");
        });

        journal.begin_task();
        doc.rename_body(body, "Before");
        doc.apply_import(
            crate::AssetReference::new(
                "assets/x.step".to_string(),
                crate::AssetType::Step,
                serde_json::json!({}),
            ),
            Vec::new(),
            kernel_api::TessellationSettings::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        doc.rename_body(body, "After");
        assert_eq!(journal.cancel_task(&mut doc), (1, true));
        assert_eq!(doc.bodies()[0].name, "Before", "what followed the import");
        assert!(!journal.in_task() && !journal.can_undo() && !journal.can_redo());
    }

    #[test]
    fn history_jumps_flow_to_the_outbox_like_ordinary_edits() {
        let mut doc = Document::new("Relay");
        let mut journal = OpJournal::new(16);
        let body = doc.create_body(Some("A".into()));
        doc.rename_body(body, "B");
        journal.note(&mut doc);
        let _ = doc.take_pending_ops();

        journal.undo(&mut doc);
        let ops = doc.take_pending_ops();
        assert!(
            !ops.is_empty(),
            "peers must hear the undo as ordinary ops, got none"
        );
    }
}
