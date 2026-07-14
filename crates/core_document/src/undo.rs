//! Snapshot-based undo/redo for [`Document`].
//!
//! A full in-memory clone of the document is kept per undo step. This is
//! cheap in practice: meshes and import blobs are `Arc`-shared (a clone is
//! refcount bumps plus the feature-tree JSON), and — unlike a serde
//! round-trip — a memory clone preserves the `#[serde(skip)]` sidecar maps
//! (asset bytes, BRep blobs) that a JSON snapshot would silently drop.
//!
//! Change detection rides on [`Document::mutation_seq`], which every
//! mutation path bumps via `mark_dirty`. The host calls [`UndoHistory::note`]
//! once per frame **while no mouse button is held**, so an entire drag
//! interaction coalesces into a single undo step and idle frames cost one
//! integer comparison.

use crate::Document;

struct UndoEntry {
    label: String,
    snapshot: Document,
}

/// Undo/redo stacks plus a `baseline` clone of the last committed state.
pub struct UndoHistory {
    baseline: Document,
    baseline_seq: u64,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    limit: usize,
    pending_label: Option<String>,
}

impl UndoHistory {
    /// `limit` bounds the undo depth (oldest entries are discarded).
    pub fn new(doc: &Document, limit: usize) -> Self {
        Self {
            baseline: doc.clone(),
            baseline_seq: doc.mutation_seq(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            limit: limit.max(1),
            pending_label: None,
        }
    }

    /// Label to use for the next committed step (e.g. the active tool name).
    /// Consumed by the next `note`/`commit` that actually records a step.
    pub fn set_pending_label(&mut self, label: impl Into<String>) {
        self.pending_label = Some(label.into());
    }

    /// Cut an undo boundary if the document changed since the baseline.
    /// Call once per frame at a moment when it is safe to snapshot (no
    /// drag in progress). No-op when nothing changed.
    pub fn note(&mut self, doc: &Document) {
        self.record(doc, None);
    }

    /// Explicit boundary for discrete operations (import, create body) so
    /// they never merge with surrounding edits. Also flushes any edits that
    /// preceded the operation in the same frame.
    pub fn commit(&mut self, doc: &Document, label: impl Into<String>) {
        self.record(doc, Some(label.into()));
    }

    fn record(&mut self, doc: &Document, label: Option<String>) {
        if doc.mutation_seq() == self.baseline_seq {
            return;
        }
        let label = label
            .or_else(|| self.pending_label.take())
            .unwrap_or_else(|| "Edit".to_string());
        let snapshot = std::mem::replace(&mut self.baseline, doc.clone());
        self.baseline_seq = doc.mutation_seq();
        self.undo_stack.push(UndoEntry { label, snapshot });
        if self.undo_stack.len() > self.limit {
            self.undo_stack.remove(0);
        }
        // New edits invalidate the redo branch.
        self.redo_stack.clear();
    }

    /// Restore the previous step. Returns its label, or `None` when there
    /// is nothing to undo. Uncommitted edits are folded into a step first,
    /// so they are what gets undone.
    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        self.note(doc);
        let entry = self.undo_stack.pop()?;
        let redo_snapshot = std::mem::replace(&mut self.baseline, entry.snapshot);
        self.redo_stack.push(UndoEntry {
            label: entry.label.clone(),
            snapshot: redo_snapshot,
        });
        *doc = self.baseline.clone();
        doc.mark_dirty();
        self.baseline_seq = doc.mutation_seq();
        Some(entry.label)
    }

    /// Re-apply the most recently undone step. Returns its label.
    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        // Fresh edits since the last boundary clear the redo branch (via
        // `record`), matching the usual editor convention.
        self.note(doc);
        let entry = self.redo_stack.pop()?;
        let undo_snapshot = std::mem::replace(&mut self.baseline, entry.snapshot);
        self.undo_stack.push(UndoEntry {
            label: entry.label.clone(),
            snapshot: undo_snapshot,
        });
        *doc = self.baseline.clone();
        doc.mark_dirty();
        self.baseline_seq = doc.mutation_seq();
        Some(entry.label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Drop all history and re-baseline on `doc`. Call when the document is
    /// replaced wholesale (File > New / Open) — history must not span
    /// documents.
    pub fn reset(&mut self, doc: &Document) {
        self.baseline = doc.clone();
        self.baseline_seq = doc.mutation_seq();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.pending_label = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetReference, AssetType};

    fn doc_with_body(name: &str) -> Document {
        let mut doc = Document::new(name);
        doc.create_body(Some("Body1".to_string()));
        doc
    }

    #[test]
    fn note_without_changes_records_nothing() {
        let doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 8);
        history.note(&doc);
        assert!(!history.can_undo());
    }

    #[test]
    fn undo_redo_roundtrip_restores_bodies() {
        let mut doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 8);

        doc.create_body(Some("Body1".to_string()));
        history.commit(&doc, "Create body");
        assert!(history.can_undo());

        let label = history.undo(&mut doc).unwrap();
        assert_eq!(label, "Create body");
        assert!(doc.bodies().is_empty());
        assert!(history.can_redo());

        let label = history.redo(&mut doc).unwrap();
        assert_eq!(label, "Create body");
        assert_eq!(doc.bodies().len(), 1);
        assert_eq!(doc.bodies()[0].name, "Body1");
    }

    #[test]
    fn snapshots_preserve_serde_skip_sidecars() {
        let mut doc = doc_with_body("t");
        let body_id = doc.bodies()[0].id;
        let asset = AssetReference::new("assets/x.step", AssetType::Step, serde_json::json!({}));
        let asset_id = doc.add_asset_with_data(asset, vec![1, 2, 3]);
        doc.set_imported_brep_data(body_id, vec![9, 9], vec![[1.0, 0.0, 0.0]]);

        let mut history = UndoHistory::new(&doc, 8);
        doc.create_body(Some("Body2".to_string()));
        history.note(&doc);
        history.undo(&mut doc).unwrap();

        // The `#[serde(skip)]` blob maps must survive the round-trip.
        assert_eq!(doc.asset_bytes(asset_id), Some(&[1u8, 2, 3][..]));
        assert_eq!(doc.imported_brep_blob(body_id), Some(&[9u8, 9][..]));
        assert_eq!(doc.bodies().len(), 1);
    }

    #[test]
    fn new_edit_clears_redo_branch() {
        let mut doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 8);

        doc.create_body(Some("A".to_string()));
        history.note(&doc);
        history.undo(&mut doc).unwrap();
        assert!(history.can_redo());

        doc.create_body(Some("B".to_string()));
        history.note(&doc);
        assert!(!history.can_redo());
    }

    #[test]
    fn limit_discards_oldest() {
        let mut doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 2);
        for i in 0..5 {
            doc.create_body(Some(format!("B{i}")));
            history.note(&doc);
        }
        assert!(history.undo(&mut doc).is_some());
        assert!(history.undo(&mut doc).is_some());
        assert!(history.undo(&mut doc).is_none(), "depth capped at 2");
        assert_eq!(doc.bodies().len(), 3);
    }

    #[test]
    fn undo_folds_uncommitted_edits_first() {
        let mut doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 8);
        doc.create_body(Some("A".to_string()));
        // No note() yet — undo must still see and revert this edit.
        assert!(history.undo(&mut doc).is_some());
        assert!(doc.bodies().is_empty());
    }

    #[test]
    fn reset_drops_history() {
        let mut doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 8);
        doc.create_body(Some("A".to_string()));
        history.note(&doc);
        history.reset(&doc);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn pending_label_is_used_once() {
        let mut doc = Document::new("t");
        let mut history = UndoHistory::new(&doc, 8);
        history.set_pending_label("Add line");
        doc.create_body(Some("A".to_string()));
        history.note(&doc);
        doc.create_body(Some("B".to_string()));
        history.note(&doc);
        assert_eq!(history.undo(&mut doc).as_deref(), Some("Edit"));
        assert_eq!(history.undo(&mut doc).as_deref(), Some("Add line"));
    }
}
