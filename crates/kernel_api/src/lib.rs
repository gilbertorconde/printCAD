use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Convenience alias for kernel fallible operations.
pub type KernelResult<T> = Result<T, KernelError>;

/// Handle to a body managed by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BodyHandle(pub u64);

/// Request describing which features or bodies must be recomputed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildRequest {
    /// Feature identifiers that triggered the rebuild.
    pub dirty_features: Vec<String>,
    /// Whether dependent features should be recomputed automatically.
    pub propagate: bool,
}

/// Response returned for every rebuild invocation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildResponse {
    /// Bodies that were modified or regenerated.
    pub updated_bodies: Vec<BodyHandle>,
    /// Kernel provided diagnostics or warnings.
    pub diagnostics: Vec<String>,
}

/// How linear (chord) deflection is chosen for OCCT meshing.
///
/// The default is **bbox-scaled** deflection: absolute chord height is derived
/// from the sum of the shape's bounding-box extents and a dimensionless
/// multiplier ([`TessellationSettings::mesh_deviation`], typically `0.2`).
/// This produces visually consistent tessellation across small and large
/// parts without per-model tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LinearDeflectionMode {
    /// Bounding-box scaled: linear deflection = `(dx + dy + dz) / 300 ×`
    /// [`TessellationSettings::mesh_deviation`].
    #[default]
    BboxScaled,
    /// Fixed absolute linear deflection in model units (millimetres for STEP geometry).
    AbsoluteMm,
}

/// Parameters controlling tessellation quality for viewport rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TessellationSettings {
    #[serde(default)]
    pub linear_deflection_mode: LinearDeflectionMode,
    /// Dimensionless multiplier applied to the bbox-derived chord height when
    /// [`Self::linear_deflection_mode`] is [`LinearDeflectionMode::BboxScaled`].
    /// Typical range `0.05..=1.0`; smaller values yield finer triangulation.
    #[serde(default = "default_mesh_deviation")]
    pub mesh_deviation: f32,
    /// Absolute chord height in model units when using [`LinearDeflectionMode::AbsoluteMm`].
    pub chord_tolerance: f32,
    pub angular_tolerance_deg: f32,
    /// When true, the kernel collapses vertices that share a position across
    /// multiple faces *and* whose face normals are within
    /// [`Self::weld_angle_threshold_deg`] of each other. This typically
    /// shrinks vertex counts by 4–6× on dense CAD models without flattening
    /// genuine sharp edges, since dissimilar face normals stay separate.
    #[serde(default = "default_weld_cross_face")]
    pub weld_cross_face: bool,
    /// Maximum angle between two face normals at a shared position for them
    /// to be merged into a single welded vertex. Above this angle the kernel
    /// keeps them separate so hard CAD edges stay crisp under shading.
    /// Defaults to 30° (common cross-face weld preset).
    #[serde(default = "default_weld_angle_threshold_deg")]
    pub weld_angle_threshold_deg: f32,
    /// When true, import serializes each body with `BRepTools::Write` into
    /// `brep_blob` (in memory) and leaves mesh fields empty until a follow-up
    /// tessellation job runs on the kernel thread. **This is the recommended
    /// default for large STEP files:** work is split across read/serialize vs
    /// meshing, the UI can show 0 triangles then a tessellation log line, and
    /// you avoid a single multi‑minute inline mesh+transfer FFI call.
    /// When false, the importer tessellates **during** the import call (no BRep
    /// blob); small parts can feel simpler, but huge models block one long step
    /// (`inline_mesh_ms` in `[printcad_import_brep_cpp]` on stderr).
    /// Serialization can still take minutes on massive assemblies; the STEP
    /// asset remains available in the document regardless.
    #[serde(default = "default_persist_brep_snapshot")]
    pub persist_brep_snapshot: bool,
    /// When true, compute mesh outline / edge segments for the viewport (face
    /// boundaries). Skipping saves CPU on huge imports when outlines are not needed.
    #[serde(default = "default_generate_boundary_edges")]
    pub generate_boundary_edges: bool,
}

fn default_weld_cross_face() -> bool {
    false
}

fn default_weld_angle_threshold_deg() -> f32 {
    30.0
}

fn default_mesh_deviation() -> f32 {
    0.2
}

fn default_persist_brep_snapshot() -> bool {
    true
}

fn default_generate_boundary_edges() -> bool {
    true
}

impl Default for TessellationSettings {
    fn default() -> Self {
        Self {
            linear_deflection_mode: LinearDeflectionMode::default(),
            mesh_deviation: default_mesh_deviation(),
            chord_tolerance: 0.1,
            angular_tolerance_deg: 28.65,
            weld_cross_face: default_weld_cross_face(),
            weld_angle_threshold_deg: default_weld_angle_threshold_deg(),
            persist_brep_snapshot: default_persist_brep_snapshot(),
            generate_boundary_edges: default_generate_boundary_edges(),
        }
    }
}

/// Triangular mesh generated from kernel bodies for viewports and export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Optional line-list of feature edges that should be drawn as outlines on
    /// top of the shaded surface (e.g. face boundaries from a STEP import).
    /// Stored as flat pairs of indices into [`Self::positions`]; the slice
    /// length is therefore always a multiple of two. Empty when the source has
    /// no edge information available.
    #[serde(default)]
    pub edges: Vec<u32>,
    /// Optional per-vertex linear RGB albedo in 0..1 (same length as [`Self::positions`] when set).
    /// Empty means the renderer uses the body tint from push constants only.
    #[serde(default)]
    pub colors: Vec<[f32; 3]>,
}

impl TriMesh {
    /// Compute axis-aligned bounding box `(min, max)` of the mesh, if non-empty.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        if self.positions.is_empty() {
            return None;
        }
        let mut min = self.positions[0];
        let mut max = self.positions[0];
        for &[x, y, z] in &self.positions {
            if x < min[0] {
                min[0] = x;
            }
            if y < min[1] {
                min[1] = y;
            }
            if z < min[2] {
                min[2] = z;
            }
            if x > max[0] {
                max[0] = x;
            }
            if y > max[1] {
                max[1] = y;
            }
            if z > max[2] {
                max[2] = z;
            }
        }
        Some((min, max))
    }
}

/// A single body produced by an external import (e.g. STEP).
///
/// The kernel returns one entry per top-level solid/shell encountered in the
/// source file. With deferred tessellation, [`Self::mesh`] may be empty until
/// background tessellation finishes; [`Self::brep_blob`] / [`Self::face_colors`]
/// are populated by the fast STEP read path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedBody {
    /// Optional name extracted from the source file (e.g. STEP product label).
    pub name: Option<String>,
    /// Tessellated mesh ready for the viewport (empty while tessellation is pending).
    #[serde(default)]
    pub mesh: TriMesh,
    /// Serialized BRep (e.g. `BRepTools::Write`) for this body when the fast
    /// STEP path was used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brep_blob: Vec<u8>,
    /// Per-face linear RGB albedo in the same order as `TopExp_Explorer` over
    /// faces on the matching BRep (used when serializing BRep without an XCAF
    /// document for deferred tessellation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_colors: Vec<[f32; 3]>,
    /// Axis-aligned bounds in millimetres from the raw BRep (before tessellation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
}

/// Node type emitted by STEP import hierarchy reconstruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImportedNodeKind {
    #[default]
    Assembly,
    Part,
    Instance,
}

/// One hierarchy node from the imported STEP/XCAF structure.
///
/// Nodes can either be pure containers (`body_index = None`) or reference a
/// renderable payload in [`ImportedModel::bodies`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedNode {
    /// Stable id scoped to one import result.
    pub id: u64,
    /// Parent node id; None means root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u64>,
    /// Human-readable name from STEP/XCAF labels when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Structural type of this node.
    #[serde(default)]
    pub kind: ImportedNodeKind,
    /// Initial visibility from source file metadata (if available).
    #[serde(default = "default_imported_node_visible")]
    pub visible: bool,
    /// Index into [`ImportedModel::bodies`] when this node owns geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_index: Option<usize>,
    /// Local transform matrix (row-major) relative to parent, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_transform: Option<[[f32; 4]; 4]>,
}

fn default_imported_node_visible() -> bool {
    true
}

/// Length unit declared by an imported source file.
///
/// This is a thin mirror of `core_document::Unit` that lives in `kernel_api`
/// to avoid pulling document-side types into the kernel crate. App code is
/// responsible for translating it into the document's display unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
}

/// Result of importing an external CAD file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedModel {
    /// One entry per top-level body in the source file.
    pub bodies: Vec<ImportedBody>,
    /// Optional assembly/object tree reconstructed from STEP/XCAF labels.
    #[serde(default)]
    pub nodes: Vec<ImportedNode>,
    /// Length unit declared by the source file, when detectable. Geometry in
    /// `bodies` is *always* expressed in millimetres regardless — this field
    /// is purely informational and used by the UI to pick a display unit.
    #[serde(default)]
    pub source_unit: Option<LengthUnit>,
}

/// One segment of a closed 2D profile wire, in sketch-plane coordinates
/// (millimetres). Arcs are encoded as three on-curve points so consumers
/// never have to agree on a winding convention.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProfileSegment {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    /// Circular arc through three points (start → mid → end).
    Arc {
        start: [f64; 2],
        mid: [f64; 2],
        end: [f64; 2],
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
}

/// A closed loop of profile segments. Consecutive segments share endpoints;
/// a single `Circle` segment is a wire by itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileWire {
    pub segments: Vec<ProfileSegment>,
}

/// The plane a profile lives on, in world coordinates (millimetres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfilePlane {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

/// How an extrusion combines with the body's existing solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BooleanOp {
    /// Replace / start the body's solid (first feature).
    NewSolid,
    /// Union with the existing solid (Pad on an existing body).
    Fuse,
    /// Subtract from the existing solid (Pocket).
    Cut,
}

/// How a profile is swept into a solid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SweepKind {
    /// Linear extrusion along `plane.normal`; negative extrudes backwards.
    Extrude {
        distance: f64,
        /// Extrude half the distance to each side of the sketch plane.
        symmetric: bool,
    },
    /// Revolution about an axis lying IN the sketch plane, given in sketch
    /// 2D coordinates (point + direction). `angle_deg` in (0, 360].
    Revolve {
        axis_origin: [f64; 2],
        axis_dir: [f64; 2],
        angle_deg: f64,
    },
}

/// A single sketch-profile solid step in a body's build history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidOp {
    pub plane: ProfilePlane,
    /// Closed wires; the largest-area wire is the outer boundary, the rest
    /// become holes.
    pub wires: Vec<ProfileWire>,
    pub kind: SweepKind,
    pub op: BooleanOp,
}

/// Result of executing a body's solid-op chain.
#[derive(Debug, Clone, Default)]
pub struct SolidBuildResult {
    /// OCCT BRep snapshot of the final solid (for later re-tessellation,
    /// persistence, and downstream booleans).
    pub brep_blob: Vec<u8>,
    /// Render mesh of the final solid.
    pub mesh: TriMesh,
    /// Axis-aligned bounds in millimetres.
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
}

/// Trait implemented by any geometry kernel that can serve the application.
pub trait Kernel: Send {
    /// Human-friendly identifier for logging purposes.
    fn name(&self) -> &str;

    /// Called once before any geometry work happens.
    fn initialize(&mut self) -> KernelResult<()>;

    /// Recompute dirty features/bodies and return the affected handles.
    fn rebuild(&mut self, request: &RebuildRequest) -> KernelResult<RebuildResponse>;

    /// Produce a triangular mesh for the provided body handle.
    fn tessellate(&self, body: BodyHandle, detail: &TessellationSettings) -> KernelResult<TriMesh>;

    /// Read a STEP/STP file from disk and return tessellated bodies.
    ///
    /// Implementations that do not support STEP import should return
    /// [`KernelError::Unsupported`].
    fn import_step(
        &mut self,
        _path: &Path,
        _detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        Err(KernelError::Unsupported(
            "STEP import is not implemented by this kernel".into(),
        ))
    }
}

/// Standardized error type for kernel interactions.
#[derive(Debug, Error)]
pub enum KernelError {
    #[error("kernel initialization failed: {0}")]
    Initialization(String),
    #[error("kernel not initialized")]
    NotInitialized,
    #[error("operation unsupported: {0}")]
    Unsupported(String),
    #[error("invalid kernel input: {0}")]
    InvalidInput(String),
    #[error("import failed: {0}")]
    Import(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
