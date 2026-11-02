//! What the tree's menus set on a body: frozen, selectable, material,
//! face colours and shading, each one undoable op.

use core_document::{Document, FaceColor, Material, TriMesh, mesh_with_face_colors};

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
    let entry = doc.bodies().iter().find(|b| b.id == body).unwrap();
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
            name: 0,
            index: 1,
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
