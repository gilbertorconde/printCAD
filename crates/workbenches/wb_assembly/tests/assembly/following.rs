//! A joint picked on named faces follows them: a body rebuilt with the
//! face moved is searched for it by name, and what is joined to it moves
//! with the face as the host settles the document.

use std::sync::Arc;

use core_document::{BodyId, Document, DocumentService, ImportedGeometry, WorkbenchRuntimeContext};
use kernel_api::{FaceSurface, TriMesh};
use wb_assembly::{Anchor, AssemblyWorkbench, JOINT_KIND};

/// What the host does before closing an undo step.
fn settle(registry: &mut DocumentService, doc: &mut Document) {
    registry.evaluate(doc);
    let moved = doc.take_moved_values();
    if moved.is_empty() {
        return;
    }
    for id in registry.ids().to_vec() {
        let wb = registry.workbench_mut(&id).unwrap();
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        wb.values_moved(&mut ctx, &moved);
    }
}

/// A slab whose top face, named 77, stands at `top`.
fn slab(top: f32) -> ImportedGeometry {
    ImportedGeometry {
        mesh: Arc::new(TriMesh {
            positions: vec![[0.0, 0.0, top], [10.0, 0.0, top], [0.0, 10.0, top]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            faces: vec![0],
            face_surfaces: vec![FaceSurface::Plane {
                origin: [0.0, 0.0, top],
                normal: [0.0, 0.0, 1.0],
            }],
            face_names: vec![77],
            ..TriMesh::default()
        }),
        source_asset: None,
        revision: 0,
        bounds_mm: None,
        brep_blob_path: None,
        mesh_path: None,
        face_colors_path: None,
        health: None,
    }
}

fn height(doc: &Document, body: BodyId) -> f32 {
    doc.body_placement(body).point([0.0, 0.0, 0.0])[2]
}

#[test]
fn a_part_follows_the_face_it_is_mated_to() {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(AssemblyWorkbench::default()))
        .unwrap();
    let mut doc = Document::new("t");
    let base = doc.create_body(Some("Base".into()));
    let part = doc.create_body(Some("Part".into()));
    doc.set_imported_geometry(base, slab(10.0));
    let args = serde_json::json!({
        "body": part.0.to_string(),
        "face": {"point": [0, 0, 0], "normal": [0, 0, -1]},
        "other": base.0.to_string(),
        "other_face": {"point": [1, 1, 10], "normal": [0, 0, 1], "name": 77},
    });
    {
        let wb = registry
            .workbench_mut(&core_document::WorkbenchId::from(JOINT_KIND))
            .unwrap();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        wb.run_command("asm.mate", args.as_object().unwrap(), &mut ctx)
            .unwrap();
    }
    settle(&mut registry, &mut doc);
    assert!((height(&doc, part) - 10.0).abs() < 1e-3);

    // The base rebuilt taller: its top, still face 77, is 4 higher.
    doc.set_imported_geometry(base, slab(14.0));
    settle(&mut registry, &mut doc);
    assert!(
        (height(&doc, part) - 14.0).abs() < 1e-3,
        "{}",
        height(&doc, part)
    );
    let joint = wb_assembly::joints(&doc)
        .into_iter()
        .find(|j| j.body == part)
        .unwrap();
    let Anchor::Plane { point, .. } = joint.feature.fixed else {
        panic!()
    };
    assert!(
        (point[2] - 14.0).abs() < 1e-4,
        "the working end is on the face"
    );
}
