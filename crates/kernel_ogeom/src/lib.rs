//! ogeom-backed implementation of the [`Kernel`] trait.
//!
//! Pure-Rust kernel adapter: STEP, IGES and mesh import, deferred
//! tessellation of persisted shape snapshots, and solid-op chain execution
//! all go through the `ogeom` kernel. Persisted shape blobs are ogeom
//! native-format text bytes (`ogeom::io::native`).

mod annotations;
mod chain;
mod convert;
mod deviation;
pub mod dxf;
pub mod export;
mod fit;
mod health;
mod holes;
mod import;
mod orient;
pub use chain::ChainCache;
pub use import::is_iges;
mod mesh;
mod mesh_edit;
mod mesh_intake;
mod mesh_solids;
mod naming;
pub use mesh::is_mesh_file;
mod ops;
mod probe;
mod profile;
mod queries;
mod reuse;
mod rounds;
pub use queries::{OgeomQueries, QUERIES};
pub mod progress;
mod tess;

pub use progress::CONTEXT_PREFIX;
// The host installs a progress watch around each job; re-exported here so
// app code never depends on `ogeom` directly.
pub use ogeom::core::progress::{Canceller, Stage, Watch, watched};

/// Announce a stage as the kernel would, for host-side tests of a sink.
#[doc(hidden)]
pub fn stage_for_test(name: &str) {
    ogeom::core::progress::stage(name);
}

use std::path::Path;

use kernel_api::{
    ChainError, ImportedModel, Kernel, KernelError, KernelResult, SolidBuildResult, SolidOp,
    TessellationSettings, TriMesh,
};

/// ogeom-backed kernel. It holds no state: every call reads what it is
/// given, so it needs no setting up.
#[derive(Debug, Default, Clone, Copy)]
pub struct OgeomKernel;

impl OgeomKernel {
    pub fn new() -> Self {
        Self
    }

    /// Tessellates one imported body from its native-format snapshot using a
    /// pre-snapshotted per-face RGB table (same order as `ogeom::topo::explore`
    /// over the deserialized root shape).
    pub fn tessellate_step_brep(
        &self,
        brep_blob: &[u8],
        face_colors: &[[f32; 3]],
        detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        tess::tessellate_blob(brep_blob, face_colors, detail, tess::Faces::Wide)
    }

    /// Tessellate many snapshots at once, spreading the *bodies* across
    /// threads and keeping each body's faces on one.
    ///
    /// An assembly is usually many small bodies, where per-body overhead
    /// (parsing the snapshot above all) dominates the face work, so this is
    /// the pass that matters for large imports. Results come back in input
    /// order, one per job, so a failure is attributed to its own body.
    pub fn tessellate_step_breps(
        &self,
        jobs: &[(&[u8], &[[f32; 3]])],
        detail: &TessellationSettings,
    ) -> KernelResult<Vec<KernelResult<TriMesh>>> {
        Ok(ogeom::core::parallel::map_ordered(
            jobs,
            |_, (blob, face_colors)| {
                tess::tessellate_blob(blob, face_colors, detail, tess::Faces::Inline)
            },
        ))
    }

    /// Run the kernel's repair on a body's snapshot: the mended snapshot,
    /// its mesh, and the checker's verdict on it.
    pub fn repair_brep(
        &mut self,
        brep_blob: &[u8],
        face_colors: &[[f32; 3]],
        detail: &TessellationSettings,
    ) -> KernelResult<kernel_api::RepairResult> {
        health::repair_blob(brep_blob, face_colors, detail)
    }

    /// Build a B-rep from a mesh body's render mesh: coplanar triangles
    /// merged into planar faces, curved stretches kept as facets for a
    /// later refine, open shells where the mesh does not close.
    pub fn mesh_to_solid(
        &mut self,
        mesh: &TriMesh,
        detail: &TessellationSettings,
    ) -> KernelResult<kernel_api::MeshSolidResult> {
        mesh::solid_of_mesh(mesh, detail)
    }

    /// Rebuild a converted solid's faceted stretches on the surfaces they
    /// approximate; refused where the result would not close.
    pub fn refine_brep(
        &mut self,
        brep_blob: &[u8],
        face_colors: &[[f32; 3]],
        detail: &TessellationSettings,
    ) -> KernelResult<kernel_api::MeshSolidResult> {
        mesh::refine_blob(brep_blob, face_colors, detail)
    }

    /// Volume, surface area and centre of mass of a body's snapshot.
    pub fn physical_properties(
        &mut self,
        brep_blob: &[u8],
    ) -> KernelResult<kernel_api::PhysicalProperties> {
        health::measure_blob(brep_blob)
    }

    /// The first solid of the file at `path`, meshed: a STEP or IGES
    /// file's first body as read, a `.brep` file's shape, or a mesh file's
    /// triangles converted to a solid. What the file held beyond it is
    /// said in `summary`.
    pub fn read_solid(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<kernel_api::MeshSolidResult> {
        if import::is_brep(path) {
            let bytes = std::fs::read(path).map_err(|e| {
                KernelError::Import(format!("failed to read {}: {e}", path.display()))
            })?;
            return self.read_brep(&String::from_utf8_lossy(&bytes), detail);
        }
        let keep_snapshot = TessellationSettings {
            persist_brep_snapshot: true,
            ..detail.clone()
        };
        let model = import::import_step(path, &keep_snapshot, false)?;
        let count = model.bodies.len();
        let Some(body) = model.bodies.into_iter().next() else {
            return Err(KernelError::Other(anyhow::anyhow!(
                "{} holds no body",
                path.display()
            )));
        };
        let mut result = if body.brep_blob.is_empty() {
            self.mesh_to_solid(&body.mesh, detail)?
        } else {
            kernel_api::MeshSolidResult {
                brep_blob: body.brep_blob,
                face_colors: body.face_colors,
                bounds_mm: body.bounds_mm.or_else(|| body.mesh.bounds()),
                mesh: body.mesh,
                health: body.health.unwrap_or_default(),
                closed: true,
                summary: Vec::new(),
            }
        };
        if count > 1 {
            result.summary.push(format!(
                "the file holds {count} bodies; the first was taken"
            ));
        }
        Ok(result)
    }

    /// The shape `.brep` text holds, meshed and checked: its solid where
    /// it holds one.
    pub fn read_brep(
        &mut self,
        text: &str,
        detail: &TessellationSettings,
    ) -> KernelResult<kernel_api::MeshSolidResult> {
        import::read_brep(text, detail)
    }

    /// Read and mesh in one synchronous shot, keeping no shape snapshot:
    /// every body's `brep_blob` is empty.
    pub fn import_step_full_mesh(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        import::import_step(path, detail, true)
    }

    /// Build a body's solid-op chain into its native snapshot and render
    /// mesh (see [`ChainRequest`] for what else a build may be asked).
    pub fn execute_solid_chain(
        &mut self,
        request: ChainRequest<'_>,
    ) -> Result<SolidBuildResult, ChainError> {
        chain::execute(request)
    }
}

/// One build of a body's solid-op chain, and what else it is asked.
pub struct ChainRequest<'a> {
    /// The ops in order; the first must start a shape.
    pub ops: &'a [SolidOp],
    /// The name each op's faces are named under (the feature it builds, as
    /// `kernel_api::naming::name_of_id` of its id); an op without one is
    /// named by where it stands.
    pub tags: &'a [kernel_api::TopoName],
    pub detail: &'a TessellationSettings,
    /// The ops of the feature being edited, whose tool and the body
    /// without it the result previews.
    pub preview: Option<std::ops::Range<usize>>,
    /// Questions asked of the solid part way through the chain.
    pub probes: &'a [kernel_api::ChainProbe],
    /// Edges projected onto sketch planes, asked as probes are.
    pub projections: &'a [kernel_api::ChainProjection],
    /// What this body's last builds kept, resumed from where the history
    /// agrees and updated for the next.
    pub cache: Option<&'a mut ChainCache>,
}

impl<'a> ChainRequest<'a> {
    /// The plain build of `ops`: no names, preview, probes or cache.
    pub fn new(ops: &'a [SolidOp], detail: &'a TessellationSettings) -> Self {
        Self {
            ops,
            tags: &[],
            detail,
            preview: None,
            probes: &[],
            projections: &[],
            cache: None,
        }
    }

    pub fn tags(self, tags: &'a [kernel_api::TopoName]) -> Self {
        Self { tags, ..self }
    }

    pub fn preview(self, preview: Option<std::ops::Range<usize>>) -> Self {
        Self { preview, ..self }
    }

    pub fn probes(self, probes: &'a [kernel_api::ChainProbe]) -> Self {
        Self { probes, ..self }
    }

    pub fn projections(self, projections: &'a [kernel_api::ChainProjection]) -> Self {
        Self {
            projections,
            ..self
        }
    }

    pub fn cache(self, cache: Option<&'a mut ChainCache>) -> Self {
        Self { cache, ..self }
    }
}

impl Kernel for OgeomKernel {
    fn name(&self) -> &str {
        "ogeom"
    }

    fn initialize(&mut self) -> KernelResult<()> {
        Ok(())
    }

    fn import_step(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        import::import_step(path, detail, false)
    }
}
