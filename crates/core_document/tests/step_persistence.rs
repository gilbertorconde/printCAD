//! Verifies that imported STEP geometry survives a `.prtcad` round trip:
//!   * mesh data is restored from `document.json`,
//!   * raw STEP bytes are restored from `assets/...` archive entries.

use core_document::{
    AssetReference, AssetType, Compression, Document, ImportedGeometry, TriMesh,
};
use serde_json::json;
use std::sync::Arc;

fn fake_mesh() -> Arc<TriMesh> {
    Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
        edges: Vec::new(),
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
