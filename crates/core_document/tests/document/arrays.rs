//! Arrays of linked copies: each occurrence a copy the array owns, placed
//! by the pattern from where the source sits; an edit is one op and one
//! undo step, whatever copies it makes or takes away; the ops replay.

use std::sync::Arc;

use core_document::history::OpJournal;
use core_document::{
    ArrayPattern, BodyId, BodyPlacement, Compression, Document, ImportedGeometry, TriMesh,
};
use glam::{Quat, Vec3};

fn geometry() -> ImportedGeometry {
    let mesh = Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
        ..TriMesh::default()
    });
    ImportedGeometry {
        bounds_mm: mesh.bounds(),
        mesh,
        source_asset: None,
        revision: 0,
        brep_blob_path: None,
        mesh_path: None,
        face_colors_path: None,
        health: None,
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
}

fn row(spacing: f32) -> ArrayPattern {
    ArrayPattern::Linear {
        direction: [1.0, 0.0, 0.0],
        spacing,
    }
}

/// A body with a shape and a snapshot.
fn part(doc: &mut Document) -> BodyId {
    let body = doc.create_body(Some("Peg".into()));
    doc.set_imported_geometry(body, geometry());
    doc.set_imported_brep_data(body, b"ogeom peg".to_vec(), Vec::new());
    body
}

/// Where each copy's first corner is drawn, in occurrence order.
fn corners(doc: &Document, array: core_document::ArrayId) -> Vec<[f32; 3]> {
    doc.copy_array(array)
        .unwrap()
        .instances
        .iter()
        .map(|i| doc.imported_geometry(i.body).unwrap().mesh.positions[0])
        .collect()
}

#[test]
fn a_row_places_its_copies_along_the_direction_and_follows_the_source() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(20.0), 4).unwrap();
    let a = doc.copy_array(array).unwrap().clone();
    assert_eq!(a.instances.len(), 3, "the source and three copies");
    assert_eq!(
        a.instances.iter().map(|i| i.index).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    for i in &a.instances {
        assert_eq!(doc.copy_source(i.body), Some(source));
        assert_eq!(
            doc.imported_brep_blob(i.body),
            Some(&b"ogeom peg"[..]),
            "one shape, which the parts list counts as one part"
        );
    }
    let at = corners(&doc, array);
    assert!(close(at[0], [20.0, 0.0, 0.0]) && close(at[2], [60.0, 0.0, 0.0]));

    doc.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(0.0, 5.0, 0.0)),
    );
    assert!(
        close(corners(&doc, array)[1], [40.0, 5.0, 0.0]),
        "moving the source carries the row"
    );
}

#[test]
fn a_ring_turns_its_copies_about_the_axis() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    doc.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0)),
    );
    let ring = ArrayPattern::Polar {
        center: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        angle: 90.0,
    };
    let array = doc.create_copy_array(source, ring, 4).unwrap();
    let at = corners(&doc, array);
    assert!(close(at[0], [0.0, 10.0, 0.0]), "{at:?}");
    assert!(close(at[1], [-10.0, 0.0, 0.0]), "{at:?}");
    assert!(close(at[2], [0.0, -10.0, 0.0]), "{at:?}");
    let turned = doc.body_placement(doc.copy_array(array).unwrap().instances[0].body);
    assert!(
        close(turned.direction([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]),
        "each copy turned with its place"
    );
}

#[test]
fn a_new_count_is_one_op_and_one_undo_step() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(16);
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(10.0), 3).unwrap();
    journal.note(&mut doc);
    let first = doc.copy_array(array).unwrap().clone();
    let _ = doc.take_pending_ops();

    doc.set_copy_array(array, row(12.0), 6, Vec::new()).unwrap();
    assert_eq!(doc.take_pending_ops().len(), 1, "one op for the whole edit");
    journal.note(&mut doc);
    assert_eq!(doc.bodies().len(), 6);
    let grown = doc.copy_array(array).unwrap().clone();
    assert_eq!(
        &grown.instances[..2],
        &first.instances[..],
        "the copies it had are kept"
    );
    assert!(close(corners(&doc, array)[4], [60.0, 0.0, 0.0]));

    journal.undo(&mut doc).expect("undoes");
    assert_eq!(doc.bodies().len(), 3, "the three copies it made are gone");
    assert_eq!(doc.copy_array(array), Some(&first));
    assert!(
        close(corners(&doc, array)[1], [20.0, 0.0, 0.0]),
        "placed back"
    );

    journal.redo(&mut doc).expect("redoes");
    assert_eq!(doc.copy_array(array), Some(&grown), "the same copies again");

    doc.set_copy_array(array, row(12.0), 2, Vec::new()).unwrap();
    journal.note(&mut doc);
    assert_eq!(doc.bodies().len(), 2);
    journal.undo(&mut doc).expect("undoes");
    assert_eq!(doc.bodies().len(), 6, "fewer, then as many again");
    assert!(
        doc.copy_array(array)
            .unwrap()
            .instances
            .iter()
            .all(|i| doc.imported_geometry(i.body).is_some()),
        "each copy made again takes the source's shape"
    );
}

#[test]
fn deleting_a_copy_skips_it_and_undoes() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(16);
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(10.0), 4).unwrap();
    journal.note(&mut doc);
    let middle = doc.copy_array(array).unwrap().instances[1].clone();
    assert!(doc.remove_body(middle.body));
    journal.note(&mut doc);
    let a = doc.copy_array(array).unwrap();
    assert_eq!(a.skipped, [2]);
    assert_eq!(a.count, 4, "still four occurrences, one left out");
    assert!(doc.bodies().iter().all(|b| b.id != middle.body));
    journal.undo(&mut doc).expect("undoes");
    assert!(doc.bodies().iter().any(|b| b.id == middle.body), "back");
    assert!(doc.copy_array(array).unwrap().skipped.is_empty());

    assert!(doc.remove_copy_array(array));
    assert_eq!(doc.bodies().len(), 1, "the source stays alone");
}

#[test]
fn array_edits_replay_and_survive_a_save() {
    let mut live = Document::new("t");
    let source = part(&mut live);
    let baseline = live.clone();
    let _ = live.take_pending_ops();

    let array = live.create_copy_array(source, row(10.0), 3).unwrap();
    live.set_copy_array(array, row(15.0), 5, vec![3]).unwrap();
    live.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, 2.0)),
    );
    let ops = live.take_pending_ops();
    let json = serde_json::to_string(&ops).unwrap();
    let ops: Vec<core_document::op::DocumentOp> = serde_json::from_str(&json).unwrap();
    let mut replica = baseline;
    for op in &ops {
        replica.apply_op(op);
    }
    assert_eq!(
        live.replicated_projection(),
        replica.replicated_projection()
    );

    let bytes = live.save_to_bytes(Compression::None).unwrap();
    let loaded = Document::load_from_bytes(bytes).unwrap();
    assert_eq!(loaded.copy_array(array), live.copy_array(array));
    assert!(close(corners(&loaded, array)[2], [60.0, 0.0, 2.0]));
}

#[test]
fn an_array_s_copy_is_not_a_source_and_counts_are_bounded() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(10.0), 2).unwrap();
    let copy = doc.copy_array(array).unwrap().instances[0].body;
    assert!(doc.create_copy_array(copy, row(10.0), 2).is_err());
    assert!(doc.set_copy_array(array, row(10.0), 0, Vec::new()).is_err());
    let flat = ArrayPattern::Linear {
        direction: [0.0; 3],
        spacing: 5.0,
    };
    assert!(doc.set_copy_array(array, flat, 3, Vec::new()).is_err());
}
