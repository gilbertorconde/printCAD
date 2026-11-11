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

/// The op journal: bounded per-user undo/redo stacks.
#[derive(Debug, Default)]
pub struct OpJournal {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    limit: usize,
    /// Label for the next boundary, set by explicit commits ("Create
    /// body"); gestures without one get a generic label.
    pending_label: Option<String>,
    /// Boundaries wait while this is set: a script's edits are one gesture
    /// whatever the commands it calls do.
    held: bool,
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

    /// Hold boundaries until released, so everything done meanwhile is one
    /// gesture; releasing does not close it, the next boundary does.
    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    /// Close the current gesture: drain captured pairs into one entry.
    /// Called at gesture boundaries: every frame with no mouse button held,
    /// and immediately after discrete commands. A barrier clears history.
    /// Nothing closes while the journal is held. Answers the entry it
    /// closed, if any.
    pub fn note(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        if self.held {
            return None;
        }
        self.close(document)
    }

    fn close(&mut self, document: &mut Document) -> Option<JournalEntryId> {
        let (pairs, barrier) = document.take_journal_pairs();
        if barrier {
            self.undo.clear();
            self.redo.clear();
            self.pending_label = None;
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
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        // A fresh edit forks history; the old future is unreachable.
        self.redo.clear();
        Some(id)
    }

    /// Take back everything done since the last boundary and leave no
    /// step for it: a script that stopped with an error puts the document
    /// back as it found it. Answers how many edits were taken back, and
    /// whether a non-invertible op (an import) crossed, in which case only
    /// what followed it is taken back and the history clears as it would
    /// have on closing.
    pub fn roll_back(&mut self, document: &mut Document) -> (usize, bool) {
        let (pairs, barrier) = document.take_journal_pairs();
        document.without_journal(|doc| {
            for (_, inverse) in pairs.iter().rev() {
                doc.apply_history_op(inverse);
            }
        });
        self.pending_label = None;
        if barrier {
            self.undo.clear();
            self.redo.clear();
        }
        (pairs.len(), barrier)
    }

    /// Undo the newest gesture. Returns its label.
    pub fn undo(&mut self, document: &mut Document) -> Option<String> {
        // Fold any uncommitted edits first so they are what gets undone,
        // held or not.
        self.close(document);
        let entry = self.undo.pop()?;
        document.without_journal(|doc| {
            for op in entry.inverse.iter().rev() {
                doc.apply_history_op(op);
            }
        });
        let label = entry.label.clone();
        self.redo.push(entry);
        Some(label)
    }

    /// Re-apply the most recently undone gesture. Returns its label.
    pub fn redo(&mut self, document: &mut Document) -> Option<String> {
        let entry = self.redo.pop()?;
        document.without_journal(|doc| {
            for op in &entry.forward {
                doc.apply_history_op(op);
            }
        });
        let label = entry.label.clone();
        self.undo.push(entry);
        Some(label)
    }

    /// Forget everything: the journal goes on with a different document.
    pub fn reset(&mut self, document: &mut Document) {
        let _ = document.take_journal_pairs();
        self.undo.clear();
        self.redo.clear();
        self.pending_label = None;
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
    /// changes while the journal is held. Answers the label undone.
    pub fn undo_entry(&mut self, document: &mut Document, id: JournalEntryId) -> Option<String> {
        if self.held {
            return None;
        }
        self.close(document);
        if self.undo_entry_id() != Some(id) {
            return None;
        }
        self.undo(document)
    }
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
    fn a_held_journal_makes_everything_until_the_next_boundary_one_step() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        journal.label_next("Run script");
        journal.hold(true);
        let body = doc.create_body(Some("A".into()));
        journal.note(&mut doc);
        doc.rename_body(body, "B");
        journal.note(&mut doc);
        doc.create_body(Some("C".into()));
        journal.hold(false);
        journal.note(&mut doc);

        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Run script"));
        assert!(doc.bodies().is_empty(), "the whole run went back at once");
        assert!(!journal.can_undo());
    }

    /// A run that fails is taken back whole and leaves no step; what was
    /// undoable before it still is.
    #[test]
    fn a_held_run_rolled_back_leaves_the_document_and_history_as_before() {
        let mut doc = Document::new("Script");
        let mut journal = OpJournal::new(16);
        let kept = doc.create_body(Some("Kept".into()));
        journal.label_next("Create body");
        journal.note(&mut doc);

        journal.label_next("Run script");
        journal.hold(true);
        doc.rename_body(kept, "Renamed");
        let body = doc.create_body(Some("A".into()));
        doc.add_feature_in_body(datum(), "D".into(), Some(body))
            .unwrap();
        journal.note(&mut doc);
        journal.hold(false);
        let (undone, barrier) = journal.roll_back(&mut doc);
        assert!(undone >= 3 && !barrier, "{undone} {barrier}");
        journal.note(&mut doc);

        assert_eq!(doc.bodies().len(), 1);
        assert_eq!(doc.bodies()[0].name, "Kept");
        assert_eq!(doc.feature_tree().all_nodes().count(), 0);
        assert_eq!(journal.undo(&mut doc).as_deref(), Some("Create body"));
        assert!(!journal.can_undo(), "the run left no step");
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

        // Older, held, or overtaken by an open edit: refused, nothing moves.
        assert_eq!(journal.undo_entry(&mut doc, made), None);
        journal.hold(true);
        assert_eq!(journal.undo_entry(&mut doc, renamed), None);
        journal.hold(false);
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
