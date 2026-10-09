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
        mesh_path: None,
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

/// A linked copy takes its source's shape where it sits itself, follows
/// every change to the source's shape, shares its snapshot, and loses it
/// with the source; undoing its making takes it away.
#[test]
fn a_linked_copy_follows_its_source() {
    let sized = |x: f32| {
        geometry(Arc::new(TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [x, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        }))
    };
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(10);
    let source = doc.create_body(Some("Bracket".into()));
    doc.set_imported_geometry(source, sized(1.0));
    doc.set_imported_brep_data(source, b"ogeom bracket".to_vec(), Vec::new());
    journal.note(&mut doc);
    let copy = doc.create_linked_copy(source, None).expect("a copy");
    journal.note(&mut doc);
    assert_eq!(doc.copy_source(copy), Some(source));
    assert!(
        doc.bodies()
            .iter()
            .any(|b| b.id == copy && b.name == "Bracket_1"),
        "{:?}",
        doc.bodies().iter().map(|b| &b.name).collect::<Vec<_>>()
    );
    assert_eq!(doc.imported_brep_blob(copy), Some(&b"ogeom bracket"[..]));
    doc.set_body_placement(
        copy,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(50.0, 0.0, 0.0)),
    );
    let x = |doc: &Document, b| doc.imported_geometry(b).unwrap().mesh.positions[1][0];
    assert!((x(&doc, copy) - 51.0).abs() < 1e-5, "placed on its own");
    doc.set_imported_geometry(source, sized(3.0));
    assert!(
        (x(&doc, copy) - 53.0).abs() < 1e-5,
        "follows the source's shape"
    );
    assert!(
        doc.body_solid_is_imported(copy),
        "no history of its own to build"
    );
    let bytes = doc.save_to_bytes(Compression::None).unwrap();
    let loaded = Document::load_from_bytes(bytes).unwrap();
    assert_eq!(loaded.copy_source(copy), Some(source));
    assert_eq!(loaded.imported_brep_blob(copy), Some(&b"ogeom bracket"[..]));
    assert!(
        (x(&loaded, copy) - 53.0).abs() < 1e-5,
        "derived again on load"
    );
    journal.undo(&mut doc);
    journal.undo(&mut doc);
    assert!(doc.bodies().iter().all(|b| b.id != copy), "undone");
}

/// A mirrored copy draws the source mirrored, wound to face out, and
/// waits for the kernel's mirror of the source's snapshot, which it keeps
/// only while the source still has the snapshot it was made from.
#[test]
fn a_mirrored_copy_waits_for_its_snapshot() {
    use core_document::MirrorPlane;
    let mut doc = Document::new("t");
    let source = doc.create_body(Some("Left".into()));
    doc.set_imported_geometry(source, geometry(triangle()));
    doc.set_imported_brep_data(source, b"ogeom left".to_vec(), Vec::new());
    let plane = MirrorPlane {
        point: [5.0, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    };
    let copy = doc.create_mirrored_copy(source, plane, None).unwrap();
    let mesh = &doc.imported_geometry(copy).unwrap().mesh;
    assert!(
        close(mesh.positions[1], [9.0, 0.0, 0.0]),
        "{:?}",
        mesh.positions
    );
    assert_eq!(mesh.indices, [0, 2, 1], "wound the other way");
    assert!(doc.imported_brep_blob(copy).is_none());
    let waiting = doc.copies_awaiting_shape();
    assert_eq!(waiting.len(), 1);
    let (body, from, local) = &waiting[0];
    assert_eq!(*body, copy);
    assert_eq!(*local, plane);
    assert!(doc.set_mirrored_shape(copy, from, b"ogeom right".to_vec()));
    assert_eq!(doc.imported_brep_blob(copy), Some(&b"ogeom right"[..]));
    assert!(doc.copies_awaiting_shape().is_empty());
    // The source changes: the mirror is made again, and an answer from the
    // old snapshot is not kept.
    let stale = from.clone();
    doc.set_imported_brep_data(source, b"ogeom left 2".to_vec(), Vec::new());
    assert!(doc.imported_brep_blob(copy).is_none());
    assert!(!doc.set_mirrored_shape(copy, &stale, b"ogeom old".to_vec()));
    assert!(
        doc.create_mirrored_copy(copy, plane, None).is_none(),
        "no mirror of a mirror"
    );
}

/// A part linked from another file keeps its link through a save, waits
/// for its shape after a load, takes it from the file without the
/// document counting as edited, and asks again when reloaded.
#[test]
fn a_linked_part_is_read_from_its_file() {
    use core_document::FileLink;
    let mut doc = Document::new("t");
    let link = FileLink {
        path: "/tmp/part.prtcad".into(),
        body: core_document::BodyId::new(),
        stamp: 7,
    };
    let part = doc.create_linked_body("Bracket".into(), link.clone());
    assert_eq!(doc.links_awaiting_geometry(), [(part, link.clone())]);
    doc.mark_clean();
    doc.set_linked_geometry(
        part,
        geometry(triangle()),
        Some(Arc::new(b"ogeom part".to_vec())),
        None,
    );
    assert!(!doc.metadata().dirty(), "reading a link is no edit");
    assert!(doc.links_awaiting_geometry().is_empty());
    assert!(doc.body_solid_is_imported(part));

    let bytes = doc.save_to_bytes(Compression::None).unwrap();
    let loaded = Document::load_from_bytes(bytes).unwrap();
    assert!(
        loaded.imported_brep_blob(part).is_none(),
        "its shape is its file's, read again"
    );
    assert_eq!(
        loaded.bodies().iter().find(|b| b.id == part).unwrap().link,
        Some(link)
    );

    let mut journal = OpJournal::new(10);
    doc.set_link_stale(part, true);
    assert!(doc.link_stale(part));
    assert!(doc.reload_link(part, 9));
    journal.note(&mut doc);
    assert!(!doc.link_stale(part));
    assert_eq!(doc.links_awaiting_geometry()[0].1.stamp, 9);
}

/// A pick a hair off a flat face lands on the face's exact plane.
#[test]
fn a_pick_on_a_flat_face_is_carried_onto_its_plane() {
    let pick = core_document::FaceRef {
        point: [3.0, 4.0, 8.499],
        normal: [0.0, 0.0, 1.0],
        surface: Some(kernel_api::FaceSurface::Plane {
            origin: [0.0, 0.0, 8.5],
            normal: [0.0, 0.0, -1.0],
        }),
        name: 0,
    };
    let on = pick.on_its_plane();
    assert_eq!(on.point, [3.0, 4.0, 8.5]);
    assert_eq!(on.normal, [0.0, 0.0, 1.0], "facing as the pick did");
    let curved = core_document::FaceRef {
        surface: None,
        ..pick
    };
    assert_eq!(curved.on_its_plane().point, pick.point, "no plane to go to");
}
