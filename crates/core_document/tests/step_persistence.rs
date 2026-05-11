//! Verifies that imported STEP geometry survives a `.prtcad` round trip:
//!   * mesh data is restored from `document.json`,
//!   * raw STEP bytes are restored from `assets/...` archive entries.

use core_document::{AssetReference, AssetType, Compression, Document, ImportedGeometry, TriMesh};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

fn fake_mesh() -> Arc<TriMesh> {
    Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
        edges: Vec::new(),
        colors: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    })
}

#[test]
fn imported_geometry_roundtrips_through_prtcad() {
    let mut doc = Document::new("PersistenceTest");
    let body_id = doc.create_body(Some("Imported Body".into()));

    let asset = AssetReference::new(
        "assets/test.step".to_string(),
        AssetType::Step,
        json!({"source_path": "/tmp/sample.step"}),
    );
    let raw_bytes = b"ISO-10303-21;\nSTEP;\nfake-content".to_vec();
    let asset_id = doc.add_asset_with_data(asset, raw_bytes.clone());

    doc.set_imported_geometry(
        body_id,
        ImportedGeometry {
            mesh: fake_mesh(),
            source_asset: Some(asset_id),
            revision: 0,
            bounds_mm: None,
            brep_blob_path: None,
            face_colors_path: None,
        },
    );

    let tmp = std::env::temp_dir().join(format!(
        "printcad_step_persistence_{}.prtcad",
        std::process::id()
    ));
    doc.save_to_file(&tmp, Compression::None)
        .expect("save .prtcad");

    let loaded = Document::load_from_file(&tmp).expect("load .prtcad");
    let geometry = loaded
        .imported_geometry(body_id)
        .expect("imported geometry survives save/load");
    assert_eq!(geometry.mesh.positions.len(), 3);
    assert_eq!(geometry.mesh.indices, vec![0, 1, 2]);
    assert_eq!(geometry.mesh.colors.len(), 3);
    assert_eq!(geometry.mesh.colors[0], [1.0, 0.0, 0.0]);
    assert_eq!(geometry.source_asset, Some(asset_id));

    let restored_asset = loaded
        .get_asset(asset_id)
        .expect("asset reference survives save/load");
    assert_eq!(restored_asset.path, "assets/test.step");

    let restored_bytes = loaded
        .asset_bytes(asset_id)
        .expect("asset bytes restored from archive");
    assert_eq!(restored_bytes, raw_bytes.as_slice());

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn brep_sidecars_roundtrip_through_prtcad() {
    let mut doc = Document::new("BrepPersistenceTest");
    let body_id = doc.create_body(Some("BRep Body".into()));

    let brep = vec![0xABu8, 0xCD, 0xEF];
    let colors: Vec<[f32; 3]> = vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    doc.set_imported_brep_data(body_id, brep.clone(), colors.clone());

    let mesh = Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]],
        indices: Vec::new(),
        edges: Vec::new(),
        colors: Vec::new(),
    });
    doc.set_imported_geometry(
        body_id,
        ImportedGeometry {
            mesh,
            source_asset: None,
            revision: 0,
            bounds_mm: Some(([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])),
            brep_blob_path: None,
            face_colors_path: None,
        },
    );

    let tmp = std::env::temp_dir().join(format!(
        "printcad_brep_persistence_{}.prtcad",
        std::process::id()
    ));
    doc.save_to_file(&tmp, Compression::None)
        .expect("save .prtcad");

    let loaded = Document::load_from_file(&tmp).expect("load .prtcad");
    assert_eq!(
        loaded.imported_brep_blob(body_id).expect("brep restored"),
        brep.as_slice()
    );
    assert_eq!(
        loaded
            .imported_brep_face_colors(body_id)
            .expect("colors restored"),
        colors.as_slice()
    );
    let geom = loaded.imported_geometry(body_id).expect("geometry");
    assert_eq!(geom.brep_blob_path, Some(format!("brep/{}.bin", body_id.0)));
    assert_eq!(
        geom.face_colors_path,
        Some(format!("brep/{}.colors", body_id.0))
    );
    assert_eq!(geom.bounds_mm, Some(([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])));

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn imported_object_graph_and_visibility_roundtrip() {
    let mut doc = Document::new("TreePersistenceTest");
    let body_id = doc.create_body(Some("Imported Root Body".into()));
    doc.set_imported_geometry(
        body_id,
        ImportedGeometry {
            mesh: fake_mesh(),
            source_asset: None,
            revision: 0,
            bounds_mm: None,
            brep_blob_path: None,
            face_colors_path: None,
        },
    );

    let root_id = Uuid::new_v4();
    let child_id = Uuid::new_v4();
    let mut nodes = std::collections::HashMap::new();
    nodes.insert(
        root_id,
        core_document::ImportedObjectNode {
            id: root_id,
            parent_id: None,
            children: vec![child_id],
            kind: kernel_api::ImportedNodeKind::Assembly,
            name: "Assembly".into(),
            visible: true,
            body_id: None,
            local_transform: None,
        },
    );
    nodes.insert(
        child_id,
        core_document::ImportedObjectNode {
            id: child_id,
            parent_id: Some(root_id),
            children: Vec::new(),
            kind: kernel_api::ImportedNodeKind::Part,
            name: "Part".into(),
            visible: false,
            body_id: Some(body_id),
            local_transform: Some([
                [1.0, 0.0, 0.0, 10.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
        },
    );
    doc.set_imported_object_graph(vec![root_id], nodes);
    assert!(!doc.imported_body_effective_visible(body_id));

    let tmp = std::env::temp_dir().join(format!(
        "printcad_tree_persistence_{}.prtcad",
        std::process::id()
    ));
    doc.save_to_file(&tmp, Compression::None)
        .expect("save .prtcad");

    let loaded = Document::load_from_file(&tmp).expect("load .prtcad");
    assert_eq!(loaded.imported_object_roots(), &[root_id]);
    let loaded_child = loaded.imported_object(child_id).expect("child node");
    assert_eq!(loaded_child.parent_id, Some(root_id));
    assert!(!loaded_child.visible);
    assert_eq!(loaded_child.body_id, Some(body_id));
    assert!(!loaded.imported_body_effective_visible(body_id));
    assert_eq!(loaded.imported_object_for_body(body_id), Some(child_id));

    let _ = std::fs::remove_file(&tmp);
}
