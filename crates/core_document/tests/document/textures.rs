//! A body's surface textures: one undoable op, replayed on a peer, kept
//! in the file, and each finding its faces by name.

use core_document::history::OpJournal;
use core_document::texture::{Pattern, Texture};
use core_document::{Document, FaceKey, FaceTexture, TriMesh};

use crate::support::{peer_with, reloaded};

fn knurl_on(faces: Vec<FaceKey>) -> Vec<FaceTexture> {
    vec![FaceTexture {
        texture: Texture {
            pattern: Pattern::Hex,
            depth_mm: 0.6,
            ..Texture::default()
        },
        faces,
    }]
}

#[test]
fn textures_are_one_undo_step_replay_and_save() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let mut journal = OpJournal::new(16);
    journal.note(&mut doc);
    let _ = doc.take_pending_ops();

    let textures = knurl_on(vec![FaceKey { name: 42, index: 3 }]);
    doc.set_body_textures(body, textures.clone());
    let ops = doc.take_pending_ops();
    journal.note(&mut doc);
    assert_eq!(ops.len(), 1);
    let textures_of = |doc: &Document| doc.body(body).unwrap().textures.clone();
    assert_eq!(textures_of(&doc), textures);

    let mut replica = peer_with(body);
    for op in &ops {
        replica.apply_remote_op(op);
    }
    assert_eq!(textures_of(&replica), textures);

    let back = reloaded(&mut doc);
    assert_eq!(textures_of(&back), textures);

    assert!(journal.undo(&mut doc).is_some());
    assert!(textures_of(&doc).is_empty());
    assert!(journal.redo(&mut doc).is_some());
    assert_eq!(textures_of(&doc), textures);
}

/// A face is found by its name where the mesh names it, whatever its
/// index has become; else by its index.
#[test]
fn a_texture_finds_its_faces_by_name() {
    let mesh = TriMesh {
        faces: vec![0, 1, 2, 2],
        face_names: vec![10, 20, 30],
        ..Default::default()
    };
    let named = FaceTexture {
        texture: Texture::default(),
        faces: vec![FaceKey { name: 30, index: 0 }],
    };
    assert_eq!(named.face_indices(&mesh), [2]);
    let unnamed = TriMesh {
        faces: vec![0, 1],
        ..Default::default()
    };
    let by_index = FaceTexture {
        texture: Texture::default(),
        faces: vec![FaceKey { name: 0, index: 1 }],
    };
    assert_eq!(by_index.face_indices(&unnamed), [1]);
    assert!(
        knurl_on(Vec::new())[0].face_indices(&mesh).is_empty(),
        "all faces"
    );
}

/// A texture as a saved file holds it reads back to the same value and
/// writes back to the same JSON, so files written earlier keep their
/// textures.
#[test]
fn a_saved_texture_reads_and_writes_the_same_json() {
    use core_document::texture::{Axis, Projection};
    let saved = serde_json::json!({
        "texture": {
            "pattern": {"Image": {"asset": "6f1c1f3e-8d5a-4b55-9a3e-2b1f0c9d7e41"}},
            "projection": {"Cylindrical": "Y"},
            "tile_mm": 2.5,
            "rotation_deg": 15.0,
            "depth_mm": 0.25,
            "inward": true,
            "keep_flat_deg": 10.0
        },
        "faces": [{"name": 7, "index": 2}]
    });
    let read: FaceTexture = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(
        read.texture.pattern,
        Pattern::Image {
            asset: "6f1c1f3e-8d5a-4b55-9a3e-2b1f0c9d7e41".parse().unwrap()
        }
    );
    assert_eq!(read.texture.projection, Projection::Cylindrical(Axis::Y));
    assert_eq!(read.faces, [FaceKey { name: 7, index: 2 }]);
    assert_eq!(serde_json::to_value(&read).unwrap(), saved);

    let oldest = serde_json::json!({
        "texture": {"pattern": "Knurl", "projection": "Triplanar", "tile_mm": 3.0, "depth_mm": 0.4}
    });
    let read: FaceTexture = serde_json::from_value(oldest).unwrap();
    assert_eq!(read.texture, Texture::default());
    assert!(read.faces.is_empty());
}
