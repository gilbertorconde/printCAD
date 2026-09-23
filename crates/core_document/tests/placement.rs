//! A body's placement moves what the scene draws and picks, keeps its own
//! geometry as it was, undoes like any edit, and survives a save.

use std::sync::Arc;

use core_document::history::OpJournal;
use core_document::{BodyPlacement, Compression, Document, ImportedGeometry, TriMesh};
use glam::{Quat, Vec3};

fn triangle() -> Arc<TriMesh> {
    Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
        ..TriMesh::default()
    })
}

fn geometry(mesh: Arc<TriMesh>) -> ImportedGeometry {
    ImportedGeometry {
        bounds_mm: mesh.bounds(),
        mesh,
        source_asset: None,
        revision: 0,
        brep_blob_path: None,
        face_colors_path: None,
        health: None,
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
}

fn shifted_and_turned() -> BodyPlacement {
    BodyPlacement::new(
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        Vec3::new(10.0, 0.0, 0.0),
    )
}

#[test]
fn a_placed_body_draws_moved_and_keeps_its_own_mesh() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    doc.set_imported_geometry(body, geometry(triangle()));
    doc.set_body_placement(body, shifted_and_turned());

    let placed = doc.imported_geometry(body).unwrap();
    assert!(close(placed.mesh.positions[1], [10.0, 1.0, 0.0]));
    let (lo, hi) = placed.bounds_mm.unwrap();
    assert!(close(lo, [9.0, 0.0, 0.0]) && close(hi, [10.0, 1.0, 0.0]));
    let (local, _) = doc.local_geometry(body).unwrap();
    assert!(close(local.positions[1], [1.0, 0.0, 0.0]));

    // New geometry from the kernel is in the body's frame, and is placed.
    doc.set_imported_geometry(body, geometry(triangle()));
    assert!(close(
        doc.imported_geometry(body).unwrap().mesh.positions[1],
        [10.0, 1.0, 0.0]
    ));

    // Back to where it started: the drawn mesh is the body's own again.
    doc.set_body_placement(body, BodyPlacement::IDENTITY);
    assert!(close(
        doc.imported_geometry(body).unwrap().mesh.positions[1],
        [1.0, 0.0, 0.0]
    ));
}

#[test]
fn moving_a_body_undoes_like_any_edit() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(16);
    let body = doc.create_body(None);
    doc.set_imported_geometry(body, geometry(triangle()));
    journal.note(&mut doc);
    doc.set_body_placement(body, shifted_and_turned());
    journal.note(&mut doc);
    journal.undo(&mut doc).expect("the move undoes");
    assert!(doc.body_placement(body).is_identity());
    assert!(close(
        doc.imported_geometry(body).unwrap().mesh.positions[1],
        [1.0, 0.0, 0.0]
    ));
    journal.redo(&mut doc).expect("and redoes");
    assert_eq!(doc.body_placement(body), shifted_and_turned());
}

#[test]
fn a_placement_survives_a_save_with_the_body_s_own_mesh() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    doc.set_imported_geometry(body, geometry(triangle()));
    doc.set_body_placement(body, shifted_and_turned());
    let bytes = doc.save_to_bytes(Compression::None).unwrap();
    let loaded = Document::load_from_bytes(bytes).unwrap();
    assert_eq!(loaded.body_placement(body), shifted_and_turned());
    assert!(close(
        loaded.imported_geometry(body).unwrap().mesh.positions[1],
        [10.0, 1.0, 0.0]
    ));
    let (local, _) = loaded.local_geometry(body).unwrap();
    assert!(close(local.positions[1], [1.0, 0.0, 0.0]));
}
