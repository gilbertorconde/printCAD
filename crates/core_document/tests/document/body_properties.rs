//! What the tree's menus set on a body: frozen, selectable, material,
//! face colours and shading, each one undoable op.

use core_document::{Document, FaceColor, FaceKey, Material, TriMesh, mesh_with_face_colors};

#[test]
fn body_properties_are_ops_that_replay() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    doc.set_body_frozen(body, true);
    doc.set_body_selectable(body, false);
    doc.set_body_material(
        body,
        Some(Material {
            name: "PLA".into(),
            density: 1.24,
        }),
    );
    doc.set_face_color(body, 2, 77, Some([1.0, 0.0, 0.0]));
    doc.set_face_color(body, 2, 77, Some([0.0, 1.0, 0.0]));
    let smooth = core_document::Shading::Smooth { crease_deg: 45.0 };
    doc.set_body_shading(body, smooth);
    assert!(doc.body_frozen(body));
    assert!(!doc.body_selectable(body));
    let entry = doc.body(body).unwrap();
    assert_eq!(entry.material.as_ref().unwrap().density, 1.24);
    assert_eq!(entry.face_colors.len(), 1);
    assert_eq!(entry.face_colors[0].color, [0.0, 1.0, 0.0]);
    assert_eq!(entry.shading, smooth);

    let mut replica = Document::new("t");
    for op in doc.take_pending_ops() {
        replica.apply_remote_op(&op);
    }
    assert_eq!(
        replica.replicated_projection()["bodies"],
        doc.replicated_projection()["bodies"]
    );
}

/// A colour written as `{name, index, color}` in a file, and a
/// `SetFaceColor` op whose face carried a colour of its own, still read.
#[test]
fn face_colours_of_the_older_shape_read() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let _ = doc.take_pending_ops();
    let op: core_document::op::DocumentOp = serde_json::from_value(serde_json::json!({
        "SetFaceColor": {
            "id": body,
            "face": {"name": 77, "index": 2, "color": [0.0, 0.0, 0.0]},
            "color": [1.0, 0.0, 0.0]
        }
    }))
    .expect("an op of the older shape reads");
    doc.apply_remote_op(&op);
    let colour = doc.body(body).unwrap().face_colors[0];
    assert_eq!(colour.face, FaceKey { name: 77, index: 2 });
    assert_eq!(colour.color, [1.0, 0.0, 0.0]);

    let kept: FaceColor =
        serde_json::from_str(r#"{"name": 77, "index": 2, "color": [1.0, 0.0, 0.0]}"#).unwrap();
    assert_eq!(kept, colour);
    assert_eq!(
        serde_json::to_value(colour).unwrap(),
        serde_json::json!({"name": 77, "index": 2, "color": [1.0, 0.0, 0.0]})
    );
}

#[test]
fn a_coloured_face_takes_its_colour_to_its_edge() {
    // Two triangles, two faces, sharing an edge.
    let mesh = TriMesh {
        positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 1, 2, 1, 3, 2],
        faces: vec![0, 1],
        ..Default::default()
    };
    let red = [1.0, 0.0, 0.0];
    let grey = [0.5; 3];
    let colored = mesh_with_face_colors(
        &mesh,
        &[FaceColor {
            face: FaceKey { name: 0, index: 1 },
            color: red,
        }],
        grey,
    );
    for (t, face) in colored.faces.iter().enumerate() {
        for k in 0..3 {
            let v = colored.indices[t * 3 + k] as usize;
            let want = if *face == 1 { red } else { grey };
            assert_eq!(colored.colors[v], want, "triangle {t}");
        }
    }
}
