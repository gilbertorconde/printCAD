//! Notes pinned to a body: each edit one undoable op, replayed on a peer,
//! kept in the file, absent from a file that never had them.

use core_document::history::OpJournal;
use core_document::op::DocumentOp;
use core_document::{Document, Note};

use crate::support::{peer_with, reloaded};

fn texts(doc: &Document, body: core_document::BodyId) -> Vec<String> {
    doc.body_notes(body)
        .iter()
        .map(|n| n.text.clone())
        .collect()
}

#[test]
fn adding_changing_and_removing_a_note_are_one_undo_step_each() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let mut journal = OpJournal::new(16);
    journal.note(&mut doc);
    let _ = doc.take_pending_ops();

    let note = Note::pinned("Datum".into(), [0.0, 0.0, 5.0], [0.0, 0.0, 9.0], None);
    let id = doc.add_note(body, note).expect("added");
    journal.note(&mut doc);
    let mut changed = doc.note(id).unwrap().1.clone();
    changed.text = "Datum A".into();
    changed.hidden = true;
    assert!(doc.set_note(changed));
    journal.note(&mut doc);
    let ops = doc.take_pending_ops();
    assert_eq!(ops.len(), 2, "one op per edit");
    assert!(
        ops.iter()
            .all(|op| matches!(op, DocumentOp::SetBodyNotes { .. }))
    );
    assert!(
        !doc.note_effective_visible(id),
        "a hidden note does not show"
    );

    let mut replica = peer_with(body);
    for op in &ops {
        replica.apply_remote_op(op);
    }
    assert_eq!(replica.body_notes(body), doc.body_notes(body));

    let back = reloaded(&mut doc);
    assert_eq!(back.body_notes(body), doc.body_notes(body));

    assert!(journal.undo(&mut doc).is_some());
    assert_eq!(texts(&doc, body), ["Datum"]);
    assert!(journal.undo(&mut doc).is_some());
    assert!(doc.body_notes(body).is_empty());
    assert!(journal.redo(&mut doc).is_some());
    assert!(journal.redo(&mut doc).is_some());
    assert_eq!(texts(&doc, body), ["Datum A"]);

    assert!(doc.remove_note(id));
    journal.note(&mut doc);
    assert!(doc.body_notes(body).is_empty());
    assert!(!doc.remove_note(id), "a note goes once");
    assert!(journal.undo(&mut doc).is_some());
    assert_eq!(texts(&doc, body), ["Datum A"]);
}

/// A body saved before notes existed reads with none, and a body with
/// none writes no `notes` key.
#[test]
fn a_body_without_notes_reads_and_writes_none() {
    let old = serde_json::json!({
        "id": uuid::Uuid::new_v4(),
        "name": "Old",
        "created_at": 0,
    });
    let body: core_document::Body = serde_json::from_value(old).expect("reads");
    assert!(body.notes.is_empty());
    let written = serde_json::to_value(&body).unwrap();
    assert!(written.get("notes").is_none());
}

#[test]
fn a_formula_note_shows_what_it_comes_to() {
    let doc = Document::new("t");
    let mut note = Note::pinned("=2 mm * 3".into(), [0.0; 3], [0.0, 0.0, 1.0], None);
    assert_eq!(doc.note_text(&note), "6 mm");
    note.text = "Plain".into();
    assert_eq!(doc.note_text(&note), "Plain");
    note.text = "=Nothing.here".into();
    assert!(doc.note_text(&note).starts_with("=Nothing.here ("));
}
