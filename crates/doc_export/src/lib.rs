//! Writing a document's bodies out, apart from the window: the export
//! dialog's options and what a script's arguments make of them, the
//! bodies taken for the writer thread with their colours, notes, PMI and
//! pressed textures, a file per body, the print layout's copies, and the
//! slicer's command line.

mod args;
mod export;
pub mod textures;

pub use args::*;
pub use export::*;

#[cfg(test)]
mod test_support {
    use std::sync::Arc;

    use core_document::ImportedGeometry;
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
}
