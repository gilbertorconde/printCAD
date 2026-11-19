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

use crate::Document;
use crate::op::DocumentOp;

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
    forward: Vec<DocumentOp>,
    /// Stored in capture order; applied back-to-front on undo.
    inverse: Vec<DocumentOp>,
}

/// A move of a journal's stacks while a run is open, kept so a run that
/// fails can be taken back exactly.
#[derive(Debug)]
enum RunMove {
    /// A call's step closed onto the undo stack.
    Closed,
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
}

impl OpJournal {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            ..Self::default()
        }
    }

    /// Name the gesture the next boundary closes.
    pub fn label_next(&mut self, label: impl Into<String>) {
        self.pending_label = Some(label.into());
    }

    /// Open a script's run, named by the label given last: boundaries
    /// wait until [`end_run`](Self::end_run), each call closes a step of
    /// its own ([`note_call`](Self::note_call)) that undo and redo step
    /// through as any other, and the run ends as one step.
    pub fn begin_run(&mut self) {
        if self.run.is_none() {
            self.run = Some(Run {
                label: self.pending_label.take(),
                ..Run::default()
            });
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
        if self.task {
            return None;
        }
        self.pending_label = Some(label.into());
        let closed = self.close(document);
        self.pending_label = None;
        closed
    }

    /// End the open run: what is still open closes (unless a task holds
    /// it), and the newest steps its calls closed become one step, named
    /// as the run is. Steps it undid or redid stay where it left them.
    /// Answers the run's step.
    pub fn end_run(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        self.run.as_ref()?;
        if !self.task {
            self.close(document);
        }
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

    fn close(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        let (pairs, barrier) = document.take_journal_pairs();
        if barrier {
            self.cross_barrier();
            return None;
        }
        if pairs.is_empty() {
            return None;
        }
        // Moving a body's tip alone is moving through its history, as
        // selecting a feature does: not an edit to undo. A tip moved along
        // with an edit (a feature inserted at it) goes with the edit.
        if pairs
            .iter()
            .all(|(op, _)| matches!(op, DocumentOp::SetBodyTip { .. }))
        {
            self.pending_label = None;
            return None;
        }
        let label = self
            .pending_label
            .take()
            .unwrap_or_else(|| "Edit".to_string());
        let (forward, inverse): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
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
            run.made.push(id);
            if !cleared.is_empty() {
                run.moves.push(RunMove::RedoCleared(cleared));
            }
            run.moves.push(RunMove::Closed);
        }
        self.trim();
        Some(id)
    }

    /// Take back everything done since the last boundary and leave no
    /// step for it: a script that stopped with an error puts the document
    /// back as it found it. With a run open that is everything the run
    /// did, its undos and redos too, and the run ends leaving no step.
    /// Answers how many edits were taken back, and whether a
    /// non-invertible op (an import) crossed, in which case only what
    /// followed it is taken back and the history clears as it would have
    /// on closing.
    pub fn roll_back(&mut self, document: &mut Document) -> (usize, bool) {
        let (mut undone, mut barrier) = self.roll_back_pending(document);
        let Some(run) = self.run.take() else {
            return (undone, barrier);
        };
        barrier |= run.barrier;
        for step in run.moves.into_iter().rev() {
            match step {
                RunMove::Closed => {
                    if let Some(entry) = self.undo.pop() {
                        undone += apply_back(document, &entry);
                    }
                }
                RunMove::Undone => {
                    if let Some(entry) = self.redo.pop() {
                        undone += apply_forward(document, &entry);
                        self.undo.push(entry);
                    }
                }
                RunMove::Redone => {
                    if let Some(entry) = self.undo.pop() {
                        undone += apply_back(document, &entry);
                        self.redo.push(entry);
                    }
                }
                RunMove::RedoCleared(entries) => self.redo = entries,
            }
        }
        self.trim();
        (undone, barrier)
    }

    /// Take back the edits not yet closed, leaving no step for them.
    fn roll_back_pending(&mut self, document: &mut Document) -> (usize, bool) {
        let (pairs, barrier) = document.take_journal_pairs();
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
    /// given, closed unless a script's run is open.
    pub fn accept_task(
        &mut self,
        document: &mut Document,
        label: Option<String>,
    ) -> Option<JournalEntryId> {
        self.task = false;
        if let Some(label) = label {
            self.label_next(label);
        }
        self.note(document)
    }

    /// End the open task taking back everything done while it was open,
    /// leaving no step: the document is as the task found it. Answers as
    /// [`roll_back`](Self::roll_back) does: across a non-invertible op only
    /// what followed it is taken back.
    pub fn cancel_task(&mut self, document: &mut Document) -> (usize, bool) {
        self.task = false;
        self.roll_back_pending(document)
    }

    /// Undo the newest gesture. Returns its label.
    pub fn undo(&mut self, document: &mut Document) -> Option<String> {
        // Fold any uncommitted edits first so they are what gets undone,
        // in a run or not.
        self.close(document);
        let entry = self.undo.pop()?;
        apply_back(document, &entry);
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
        apply_forward(document, &entry);
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

/// Take `entry` back: its inverses, newest first. Answers how many.
fn apply_back(document: &mut Document, entry: &Entry) -> usize {
    document.without_journal(|doc| {
        for op in entry.inverse.iter().rev() {
            doc.apply_history_op(op);
        }
    });
    entry.inverse.len()
}

/// Do `entry` again: its ops, oldest first. Answers how many.
fn apply_forward(document: &mut Document, entry: &Entry) -> usize {
    document.without_journal(|doc| {
        for op in &entry.forward {
            doc.apply_history_op(op);
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
        assert_eq!(node.data, serde_json::json!({"v": 1}));
        assert_eq!(node.package_source.as_deref(), Some("acme/cam"));
    }

    #[test]
    fn a_run_ends_as_one_step_whatever_its_calls_closed() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        journal.label_next("Run script");
        journal.begin_run();
        let body = doc.create_body(Some("A".into()));
        journal.note(&mut doc);
        doc.rename_body(body, "B");
        journal.note_call(&mut doc, "doc.rename");
        doc.create_body(Some("C".into()));
        journal.end_run(&mut doc);
        journal.note(&mut doc);

        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert!(doc.bodies().is_empty(), "the whole run went back at once");
        assert!(!journal.can_undo());
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
        journal.begin_run();
        doc.rename_body(kept, "Renamed");
        journal.note_call(&mut doc, "doc.rename");
        let body = doc.create_body(Some("A".into()));
        doc.add_feature_in_body(datum(), "D".into(), Some(body))
            .unwrap();
        journal.note(&mut doc);
        let (undone, barrier) = journal.roll_back(&mut doc);
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
        journal.begin_run();
        doc.rename_body(body, "B");
        journal.note_call(&mut doc, "doc.rename");
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
        journal.begin_run();
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        assert!(doc.bodies().is_empty());
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
        journal.begin_run();
        journal.undo(&mut doc);
        assert_eq!(journal.end_run(&mut doc), None);
        assert!(!journal.can_undo());
        assert_eq!(journal.redo(&mut doc).as_deref(), Some("Create body"));
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
        journal.begin_run();
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
