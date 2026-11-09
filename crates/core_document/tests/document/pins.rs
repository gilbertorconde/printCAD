//! Pins on a body: each edit one undoable op, replayed on a peer, kept in
//! the file, numbered across the document.

use core_document::history::OpJournal;
use core_document::op::DocumentOp;
use core_document::{Document, PinAuthor, PinWant};

use crate::support::{peer_with, reloaded};

#[test]
fn pins_are_one_undo_step_each_and_never_share_a_number() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let other = doc.create_body(None);
    let mut journal = OpJournal::new(16);
    journal.note(&mut doc);
    let _ = doc.take_pending_ops();

    let new = |doc: &Document, body, note: &str| {
        doc.new_pin(body, note.into(), PinAuthor::User, None, PinWant::Face)
    };
    let first = doc.add_pin(body, new(&doc, body, "Wall")).expect("added");
    journal.note(&mut doc);
    let second = doc.add_pin(other, new(&doc, other, "Hole")).expect("added");
    journal.note(&mut doc);
    assert_eq!((first, second), (1, 2), "numbered across the document");
    let mut done = doc.pin(first).unwrap().1.clone();
    done.done = true;
    done.hidden = true;
    assert!(doc.set_pin(done));
    journal.note(&mut doc);
    let ops = doc.take_pending_ops();
    assert_eq!(ops.len(), 3, "one op per edit");
    assert!(
        ops.iter()
            .all(|op| matches!(op, DocumentOp::SetBodyPins { .. }))
    );
    assert!(
        !doc.pin_effective_visible(first),
        "a hidden pin does not show"
    );

    let mut replica = peer_with(body);
    replica.apply_remote_op(&DocumentOp::CreateBody {
        id: other,
        name: "Body".into(),
        created_at: 0,
    });
    for op in &ops {
        replica.apply_remote_op(op);
    }
    assert_eq!(replica.body_pins(body), doc.body_pins(body));
    let mut back = reloaded(&mut doc);
    assert_eq!(back.body_pins(other), doc.body_pins(other));

    assert!(journal.undo(&mut doc).is_some());
    assert!(!doc.pin(first).unwrap().1.done);
    assert!(doc.remove_pin(second));
    assert!(!doc.remove_pin(second), "a pin goes once");
    // A pin taken away leaves its number unused, in this document and the
    // file it was saved to.
    assert_eq!(doc.add_pin(body, new(&doc, body, "Next")), Some(3));
    assert_eq!(back.add_pin(body, new(&back, body, "Next")), Some(3));
}
