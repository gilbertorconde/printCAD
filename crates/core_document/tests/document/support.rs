//! What the document's tests build and check their documents with.

use std::sync::Arc;

use core_document::op::DocumentOp;
use core_document::{BodyId, Compression, Document, ImportedGeometry, TriMesh};

/// One triangle on the XY plane, its corner at the origin.
pub fn triangle() -> Arc<TriMesh> {
    Arc::new(TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
        ..TriMesh::default()
    })
}

/// `mesh` as a body's geometry, bounded by it and from no file.
pub fn geometry(mesh: Arc<TriMesh>) -> ImportedGeometry {
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

/// `doc` saved to an uncompressed container and read back.
pub fn reloaded(doc: &mut Document) -> Document {
    let bytes = doc.save_to_bytes(Compression::None).expect("saves");
    Document::load_from_bytes(bytes).expect("loads")
}

/// A peer holding only `body`, as its creation reached it.
pub fn peer_with(body: BodyId) -> Document {
    let mut peer = Document::new("peer");
    peer.apply_remote_op(&DocumentOp::CreateBody {
        id: body,
        name: "Body".into(),
        created_at: 0,
    });
    peer
}

/// `ops`, through the JSON the wire and the op log carry them as, applied
/// to `baseline` land on `live`'s replicated state.
pub fn assert_replays(live: &Document, baseline: Document, ops: &[DocumentOp]) {
    let json = serde_json::to_string(ops).expect("ops serialize");
    let decoded: Vec<DocumentOp> = serde_json::from_str(&json).expect("ops deserialize");
    let mut replica = baseline;
    for op in &decoded {
        replica.apply_op(op);
    }
    assert_eq!(
        live.replicated_projection(),
        replica.replicated_projection(),
        "replay must converge on the live document's replicated state"
    );
}

/// A container of `entries`, in order.
pub fn pack(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).expect("path");
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, data.as_slice()).expect("append");
    }
    builder.into_inner().expect("finish")
}
