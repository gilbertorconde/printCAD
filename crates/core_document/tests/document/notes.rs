//! Notes on a body: each edit one undoable op, replayed on a peer, kept in
//! the file, numbered across the document, absent from a file that never
//! had them.

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

    let mut note = Note::pinned("Datum".into(), [0.0, 0.0, 5.0], [0.0, 0.0, 9.0], None);
    note.asks = true;
    note.resolved = Some("The bottom face".into());
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
    assert!(
        back.body_notes(body).iter().all(|n| n.from_file)
            && doc.body_notes(body).iter().all(|n| !n.from_file),
        "a note read from the file says so; one written here does not"
    );

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
    // A note taken away leaves its number unused, here and in the file.
    let next = Note::pinned("Next".into(), [0.0; 3], [0.0; 3], None);
    let next = doc.add_note(body, next.clone()).unwrap();
    assert_eq!(doc.note(next).unwrap().1.number, 2);
    let mut back = back;
    let again = back.add_note(body, Note::pinned("Next".into(), [0.0; 3], [0.0; 3], None));
    assert_eq!(back.note(again.unwrap()).unwrap().1.number, 2);
    assert!(journal.undo(&mut doc).is_some());
    assert!(journal.undo(&mut doc).is_some());
    assert_eq!(texts(&doc, body), ["Datum A"]);
    assert_eq!(
        doc.note(id).unwrap().1.number,
        1,
        "its number comes back with it"
    );
}

/// Notes in a file written before notes had numbers are numbered on
/// reading, after any number already given.
#[test]
fn notes_from_before_numbers_are_numbered_on_reading() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    for text in ["A", "B"] {
        doc.add_note(body, Note::pinned(text.into(), [0.0; 3], [0.0; 3], None));
    }
    let mut old = serde_json::to_value(&doc).unwrap();
    for note in old["bodies"][0]["notes"].as_array_mut().unwrap() {
        note.as_object_mut().unwrap().remove("number");
    }
    old.as_object_mut().unwrap().remove("notes_numbered");
    let mut doc: Document = serde_json::from_value(old).unwrap();
    let back = reloaded(&mut doc);
    let numbers: Vec<u32> = back.body_notes(body).iter().map(|n| n.number).collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!(back.next_note_number(), 3);
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

/// A feature drawn in a plane `z` above its body's origin, holding one
/// item unless `gone`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Plane {
    z: f32,
    gone: bool,
}

impl core_document::WorkbenchFeature for Plane {
    fn workbench_id() -> core_document::WorkbenchId {
        core_document::WorkbenchId::from("kind.plane")
    }
    fn dependencies(&self) -> Vec<core_document::FeatureId> {
        Vec::new()
    }
    fn name(&self) -> &str {
        "plane"
    }
}

/// The bench of `Plane`: a note on its item keeps one unit right of where
/// it was put, one on its plane where it was put.
struct PlaneBench;

impl core_document::Workbench for PlaneBench {
    fn descriptor(&self) -> core_document::WorkbenchDescriptor {
        core_document::WorkbenchDescriptor::new("wb.planes", "Planes", "")
            .feature_kinds(["kind.plane"])
    }
    fn configure(&self, _context: &mut core_document::WorkbenchContext) {}
}

impl core_document::BenchFeatures for PlaneBench {
    fn note_spot(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        item: Option<uuid::Uuid>,
        at: [f32; 3],
        label: [f32; 3],
    ) -> Option<core_document::SketchNoteSpot> {
        let plane: Plane = serde_json::from_value(document.get_feature_data(id)?.clone()).ok()?;
        let shift = if item.is_some() && !plane.gone {
            1.0
        } else {
            0.0
        };
        Some(core_document::SketchNoteSpot {
            at: [at[0] + shift, at[1], 0.0],
            label: [label[0] + shift, label[1], 0.0],
            lost: item.is_some() && plane.gone,
            plane: core_document::NotePlane {
                origin: [0.0, 0.0, plane.z],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
            },
            size: 10.0,
        })
    }
}
impl core_document::BenchTools for PlaneBench {}
impl core_document::BenchPanels for PlaneBench {}
impl core_document::BenchBuild for PlaneBench {}
impl core_document::BenchParameters for PlaneBench {}
impl core_document::BenchOutput for PlaneBench {}
impl core_document::BenchCommands for PlaneBench {}

/// A note on a sketch shares the document's numbering, stands where the
/// sketch's bench puts it as evaluation leaves it (where the body sits,
/// following the sketch), is lost when its item goes and stays where it
/// last stood when the sketch goes; it shows only while the sketch does,
/// and its anchor travels to a peer and through the file, a note without
/// one writing none.
#[test]
fn a_note_on_a_sketch_is_placed_by_its_bench_and_kept_like_any_note() {
    use core_document::{SketchAnchor, WorkbenchFeature};
    let mut registry = core_document::DocumentService::default();
    registry.register_workbench(Box::new(PlaneBench)).unwrap();
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let plane = Plane {
        z: 2.0,
        gone: false,
    };
    let sketch = doc
        .add_feature_in_body(plane, "Sketch".into(), Some(body))
        .unwrap();
    let _ = doc.take_pending_ops();
    let on_body = doc
        .add_note(body, Note::pinned("Face".into(), [0.0; 3], [0.0; 3], None))
        .unwrap();
    let on_sketch = Note {
        sketch: Some(SketchAnchor {
            feature: sketch,
            item: Some(uuid::Uuid::new_v4()),
        }),
        ..Note::pinned("Line".into(), [3.0, 4.0, 0.0], [3.0, 9.0, 0.0], None)
    };
    let on_sketch = doc.add_note(body, on_sketch).unwrap();
    assert_eq!(doc.note(on_body).unwrap().1.number, 1);
    assert_eq!(doc.note(on_sketch).unwrap().1.number, 2, "one numbering");
    let placed = |doc: &mut Document| {
        registry.evaluate(doc);
        let (body, note) = doc.note(on_sketch).unwrap();
        doc.note_placed(body, note)
    };
    let p = placed(&mut doc);
    assert_eq!(
        (p.at, p.label, p.lost),
        ([4.0, 4.0, 2.0], [4.0, 9.0, 2.0], false)
    );
    doc.set_body_placement(
        body,
        core_document::BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(10.0, 0.0, 0.0)),
    );
    assert_eq!(placed(&mut doc).at, [14.0, 4.0, 2.0], "where the body sits");
    let moved = Plane {
        z: 5.0,
        gone: false,
    };
    doc.update_feature_data(sketch, moved.to_json()).unwrap();
    assert_eq!(placed(&mut doc).at, [14.0, 4.0, 5.0], "with the sketch");
    let (_, note) = doc.note(on_sketch).unwrap();
    assert_eq!(
        doc.note_point_from(body, note, [20.0, 1.0, 5.0]),
        [10.0, 1.0, 0.0],
        "a point given in the world is taken into the plane"
    );

    doc.update_feature_data(sketch, Plane { z: 5.0, gone: true }.to_json())
        .unwrap();
    let p = placed(&mut doc);
    assert!(
        p.lost && p.at == [13.0, 4.0, 5.0],
        "lost where it was put: {p:?}"
    );
    doc.update_feature_data(sketch, moved.to_json()).unwrap();
    let last = placed(&mut doc);

    doc.set_feature_visible(sketch, false);
    assert!(!doc.note_effective_visible(on_sketch) && doc.note_effective_visible(on_body));
    doc.set_feature_visible(sketch, true);
    assert!(doc.note_effective_visible(on_sketch));
    let mut ops = doc.take_pending_ops();
    doc.remove_feature(sketch).unwrap();
    let gone = placed(&mut doc);
    assert!(gone.lost && gone.at == last.at, "where it last stood");
    assert!(doc.note_effective_visible(on_sketch), "a lost note shows");
    ops.extend(doc.take_pending_ops());

    let mut replica = peer_with(body);
    let notes: Vec<_> = ops
        .iter()
        .filter(|op| matches!(op, DocumentOp::SetBodyNotes { .. }))
        .collect();
    for op in notes {
        replica.apply_remote_op(op);
    }
    let back = reloaded(&mut doc);
    for other in [&replica, &back] {
        assert_eq!(other.body_notes(body), doc.body_notes(body));
    }
    let json = serde_json::to_value(doc.body_notes(body)).unwrap();
    assert!(
        json[0].get("sketch").is_none(),
        "a note on a body writes none"
    );
    assert_eq!(json[1]["sketch"]["feature"], sketch.0.to_string());
}
