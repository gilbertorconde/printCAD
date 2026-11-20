//! Fixtures the app's tests share.

use std::sync::Arc;

use core_document::{BodyId, Document, FeatureId, ImportedGeometry};
use kernel_api::TriMesh;

/// `mesh` as a body's geometry, bounded by its points, from no file.
pub(crate) fn geometry(mesh: impl Into<Arc<TriMesh>>) -> ImportedGeometry {
    let mesh = mesh.into();
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

/// Datum points named `names`, in that order in `body`'s history.
pub(crate) fn datum_points(doc: &mut Document, body: BodyId, names: &[&str]) -> Vec<FeatureId> {
    names
        .iter()
        .map(|name| {
            let point = core_document::DatumFeature {
                shape: core_document::DatumShape::Point,
                attachment: core_document::DatumAttachment::BasePlane(core_document::BasePlane::XY),
                offset: Default::default(),
            };
            doc.add_feature_in_body(point, name.to_string(), Some(body))
                .unwrap()
        })
        .collect()
}

/// A box `size` mm with a corner at `at`, its triangles outward.
pub(crate) fn block(size: [f32; 3], at: [f32; 3]) -> TriMesh {
    let corner =
        |i: usize| [0, 1, 2].map(|k| at[k] + if (i >> k) & 1 == 1 { size[k] } else { 0.0 });
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    TriMesh {
        positions: (0..8).map(corner).collect(),
        indices: quads
            .iter()
            .flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]])
            .collect(),
        ..Default::default()
    }
}
