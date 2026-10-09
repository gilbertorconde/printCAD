//! A body's base solid: the shape its feature history starts from, kept
//! from what an import made, put back when it is dropped, undone as one
//! step, and saved with the document.

use std::sync::Arc;

use core_document::history::OpJournal;
use core_document::{BodyId, Document, ImportedGeometry, TriMesh};

const BLOB: &[u8] = b"ogeom native text standing in for a solid";

fn imported_body(doc: &mut Document) -> BodyId {
    let body = doc.create_body(None);
    let mesh = Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
        ..Default::default()
    });
    doc.set_imported_geometry(
        body,
        ImportedGeometry {
            bounds_mm: mesh.bounds(),
            mesh,
            source_asset: Some(uuid::Uuid::new_v4()),
            revision: 0,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    doc.set_imported_brep_data(body, BLOB.to_vec(), vec![[0.5, 0.5, 0.5]]);
    body
}

#[test]
fn a_kept_base_is_the_body_s_start_and_dropping_it_puts_it_back() {
    let mut doc = Document::new("t");
    let body = imported_body(&mut doc);
    assert!(doc.body_solid_is_imported(body));

    assert!(doc.set_body_base(body, true));
    assert!(doc.has_base_solid(body));
    assert!(
        !doc.body_solid_is_imported(body),
        "its history builds it now"
    );
    assert_eq!(doc.base_brep_blob(body), Some(BLOB));
    assert!(!doc.set_body_base(body, true), "kept once");

    // What the history builds replaces what is drawn, not the base.
    doc.set_imported_brep_data(body, b"ogeom built".to_vec(), Vec::new());
    assert_eq!(doc.base_brep_blob(body), Some(BLOB));

    assert!(doc.set_body_base(body, false));
    assert!(!doc.has_base_solid(body));
    assert_eq!(
        doc.imported_brep_blob(body),
        Some(BLOB),
        "the base is its shape again"
    );
    assert!(doc.body_solid_is_imported(body));
}

#[test]
fn keeping_a_base_is_one_undo_step_and_replays_on_a_peer() {
    let mut doc = Document::new("t");
    let body = imported_body(&mut doc);
    let mut journal = OpJournal::new(16);
    journal.note(&mut doc);
    let _ = doc.take_pending_ops();

    doc.set_body_base(body, true);
    let ops = doc.take_pending_ops();
    journal.note(&mut doc);
    assert_eq!(ops.len(), 1);

    // The op names the body; a peer holding the same body keeps its base.
    let mut replica = doc.clone();
    replica.set_body_base(body, false);
    let _ = replica.take_pending_ops();
    for op in &ops {
        replica.apply_remote_op(op);
    }
    assert!(replica.has_base_solid(body));

    assert!(journal.undo(&mut doc).is_some());
    assert!(!doc.has_base_solid(body));
    assert_eq!(doc.imported_brep_blob(body), Some(BLOB));
    assert!(journal.redo(&mut doc).is_some());
    assert!(doc.has_base_solid(body));
}

#[test]
fn a_base_is_saved_and_read_back() {
    let mut doc = Document::new("t");
    let body = imported_body(&mut doc);
    doc.set_body_base(body, true);
    doc.set_imported_brep_data(body, b"ogeom built".to_vec(), Vec::new());
    let dir = std::env::temp_dir().join(format!("printcad-base-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("based.prtcad");
    doc.save_to_file(&path, core_document::Compression::None)
        .unwrap();
    let back = Document::load_from_file(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(back.has_base_solid(body));
    assert_eq!(back.base_brep_blob(body), Some(BLOB));
    assert_eq!(back.base_face_colors(body), Some(&[[0.5, 0.5, 0.5]][..]));
    assert_eq!(back.imported_brep_blob(body), Some(&b"ogeom built"[..]));
}

#[test]
fn a_replaced_shape_waits_on_its_asset_until_derived_on_every_replica() {
    let mut doc = Document::new("t");
    let body = imported_body(&mut doc);
    let _ = doc.take_pending_ops();
    let mut peer = doc.clone();

    assert!(doc.can_replace_shape(body));
    assert!(doc.replace_body_shape(body, "/tmp/new.step", b"ISO-10303-21;".to_vec()));
    let (waiting, asset) = doc.bodies_awaiting_shape()[0];
    assert_eq!(waiting, body);
    let (reference, bytes) = doc.asset_with_bytes(asset).unwrap();
    assert!(reference.path.ends_with(".step"));
    assert_eq!(&bytes[..], b"ISO-10303-21;");

    // A peer given the op waits on the same asset.
    for op in doc.take_pending_ops() {
        peer.apply_remote_op(&op);
    }
    assert_eq!(peer.bodies_awaiting_shape(), [(body, asset)]);

    // Once derived from that asset, nothing waits.
    let mut read = doc.imported_geometry(body).unwrap().clone();
    read.source_asset = Some(asset);
    doc.set_imported_geometry(body, read);
    assert!(doc.bodies_awaiting_shape().is_empty());

    // A based body takes the new shape as its base.
    doc.set_body_base(body, true);
    assert!(doc.replace_body_shape(body, "/tmp/newer.stl", b"solid".to_vec()));
    let (_, newer) = doc.bodies_awaiting_shape()[0];
    let mut read = doc.base_geometry(body).unwrap().clone();
    read.source_asset = Some(newer);
    doc.set_base_solid(body, read, b"ogeom newer".to_vec(), Vec::new());
    assert!(doc.bodies_awaiting_shape().is_empty());
    assert_eq!(doc.base_brep_blob(body), Some(&b"ogeom newer"[..]));
}

#[test]
fn a_shape_built_from_history_alone_is_not_replaced() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    assert!(!doc.can_replace_shape(body));
    assert!(!doc.replace_body_shape(body, "/tmp/x.step", b"x".to_vec()));
}
