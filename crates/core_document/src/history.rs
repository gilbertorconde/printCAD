//! Per-user undo over the op stream.
//!
//! No snapshots: every entry is the (op, inverse) pairs of one user
//! gesture. Undo applies the inverses in reverse order; redo re-applies the
//! forward ops. Both go through [`Document::apply_history_op`], so a
//! history jump IS ordinary ops — it marks the affected features dirty here
//! and flows to the document server (and any peers) like any other edit.
//! That is what makes this undo safe in a shared document: it never
//! replaces state wholesale, it edits forward, and it only edits what THIS
//! user touched.
//!
//! Non-invertible ops (imports, asset registration) are barriers: the
//! journal clears rather than store an inverse that would lie.

use crate::op::DocumentOp;
use crate::Document;

/// One undoable user gesture.
#[derive(Debug, Clone)]
struct Entry {
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

    /// Close the current gesture: drain captured pairs into one entry.
    /// Called at gesture boundaries — every frame with no mouse button held,
    /// and immediately after discrete commands. A barrier clears history.
    pub fn note(&mut self, document: &mut Document) {
        let (pairs, barrier) = document.take_journal_pairs();
        if barrier {
            self.undo.clear();
            self.redo.clear();
            self.pending_label = None;
            return;
        }
        if pairs.is_empty() {
            return;
        }
        let label = self
            .pending_label
            .take()
            .unwrap_or_else(|| "Edit".to_string());
        let (forward, inverse): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        self.undo.push(Entry {
            label,
            forward,
            inverse,
        });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        // A fresh edit forks history; the old future is unreachable.
        self.redo.clear();
    }

    /// Undo the newest gesture. Returns its label.
    pub fn undo(&mut self, document: &mut Document) -> Option<String> {
        // Fold any uncommitted edits first so they are what gets undone.
        self.note(document);
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

    /// Forget everything — a different document is in front of us now.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datum::{DatumAttachment, DatumFeature, DatumShape};
    use crate::BasePlane;

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
