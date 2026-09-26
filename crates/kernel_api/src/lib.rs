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

/// How linear (chord) deflection is chosen for kernel meshing.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// When true, import serializes each body's shape snapshot into
    /// `brep_blob` (in memory) and leaves mesh fields empty until a follow-up
    /// tessellation job runs on the kernel thread. **This is the recommended
    /// default for large STEP files:** work is split across read/serialize vs
    /// meshing, the UI can show 0 triangles then a tessellation log line, and
    /// you avoid a single multi‑minute inline mesh+transfer FFI call.
    /// When false, the importer tessellates **during** the import call (no
    /// shape blob); small parts can feel simpler, but huge models block one
    /// long step.
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
    /// Which kernel face each triangle was cut from: one entry per triangle
    /// (`indices.len() / 3`), an index into the body's faces in the kernel's
    /// own order. A curved face is many triangles with many normals, and
    /// this is what lets a click on one of them select the whole face.
    /// Empty when the source has no faces — a sketch, a datum, a mesh saved
    /// before faces were recorded — and a consumer falls back to geometry.
    #[serde(default)]
    pub faces: Vec<u32>,
    /// Which kernel edge each outline segment belongs to: one entry per
    /// pair in [`Self::edges`], an index into the body's edges in the
    /// kernel's own order. A curved edge is many segments, and this is what
    /// lets a click on one of them select the whole edge. Empty when the
    /// outline came from triangle boundaries rather than kernel edges.
    #[serde(default)]
    pub edge_ids: Vec<u32>,
    /// The exact surface of each kernel face, indexed like [`Self::faces`]:
    /// what a picked face is, beyond the triangles drawn for it. Empty when
    /// the source has no faces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_surfaces: Vec<FaceSurface>,
}

/// The kind and placement of a kernel face's surface, in the mesh's frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum FaceSurface {
    /// A plane through `origin`, with the face's outward `normal`.
    Plane { origin: [f32; 3], normal: [f32; 3] },
    /// A circular cylinder about the line through `origin` along `axis`.
    Cylinder {
        origin: [f32; 3],
        axis: [f32; 3],
        radius: f32,
    },
    /// A circular cone about the line through `apex` along `axis`.
    Cone { apex: [f32; 3], axis: [f32; 3] },
    /// A sphere.
    Sphere { center: [f32; 3], radius: f32 },
    /// A torus about the line through `center` along `axis`.
    Torus { center: [f32; 3], axis: [f32; 3] },
    /// Any other surface: a spline, a sweep.
    #[default]
    Other,
}

impl FaceSurface {
    /// The axis a turned face turns about, as a point on it and its
    /// direction: a cylinder's, a cone's or a torus's.
    pub fn axis(&self) -> Option<([f32; 3], [f32; 3])> {
        match *self {
            FaceSurface::Cylinder { origin, axis, .. } => Some((origin, axis)),
            FaceSurface::Cone { apex, axis } => Some((apex, axis)),
            FaceSurface::Torus { center, axis } => Some((center, axis)),
            _ => None,
        }
    }

    /// The same surface moved: `point` maps points, `direction` maps
    /// directions (a rigid motion, so lengths stay).
    pub fn moved(
        &self,
        point: impl Fn([f32; 3]) -> [f32; 3],
        direction: impl Fn([f32; 3]) -> [f32; 3],
    ) -> Self {
        match *self {
            FaceSurface::Plane { origin, normal } => FaceSurface::Plane {
                origin: point(origin),
                normal: direction(normal),
            },
            FaceSurface::Cylinder {
                origin,
                axis,
                radius,
            } => FaceSurface::Cylinder {
                origin: point(origin),
                axis: direction(axis),
                radius,
            },
            FaceSurface::Cone { apex, axis } => FaceSurface::Cone {
                apex: point(apex),
                axis: direction(axis),
            },
            FaceSurface::Sphere { center, radius } => FaceSurface::Sphere {
                center: point(center),
                radius,
            },
            FaceSurface::Torus { center, axis } => FaceSurface::Torus {
                center: point(center),
                axis: direction(axis),
            },
            FaceSurface::Other => FaceSurface::Other,
        }
    }
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
    /// Serialized shape snapshot for this body when the fast STEP path was
    /// used: ogeom native-format text bytes (`ogeom::io::native`, one root
    /// shape per blob).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brep_blob: Vec<u8>,
    /// Per-face linear RGB albedo in face-exploration order
    /// (`ogeom::topo::explore` with a Face filter) over the matching shape
    /// snapshot (used for deferred tessellation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_colors: Vec<[f32; 3]>,
    /// Axis-aligned bounds in millimetres from the raw BRep (before tessellation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
    /// What the kernel's checker found in the body's shape as read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<ShapeHealth>,
    /// The names of the layers the file puts the body on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
}

/// What the kernel's checker found in a shape.
///
/// The checker sorts its findings in two: *broken*, where an algorithm
/// reading the shape gets a wrong answer rather than an error, and
/// *suspect*, out of order but harmless to every operation. Only broken
/// findings ask for a repair.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeHealth {
    /// Findings that make an algorithm reading the shape answer wrongly.
    pub broken: usize,
    /// Findings that are out of order but harmless.
    pub suspect: usize,
    /// The first findings, a sentence each, broken first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// The kernel's repair has run on the shape these findings describe.
    #[serde(default)]
    pub repaired: bool,
}

impl ShapeHealth {
    /// How many findings a health record keeps as sentences.
    pub const KEPT_FINDINGS: usize = 8;

    /// Whether the shape has anything a repair is for.
    pub fn is_broken(&self) -> bool {
        self.broken > 0
    }

    /// One line for a tooltip: the counts, then the findings kept.
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();
        let counts = match (self.broken, self.suspect) {
            (0, 0) => "The shape checks clean".to_string(),
            (b, 0) => format!("{b} shape defect(s) that make operations answer wrongly"),
            (0, s) => format!("{s} shape irregularity(ies), harmless to operations"),
            (b, s) => format!(
                "{b} shape defect(s) that make operations answer wrongly, \
                 and {s} harmless irregularity(ies)"
            ),
        };
        lines.push(counts);
        if self.repaired && self.is_broken() {
            lines.push("Repaired; these remain, beyond what the repair mends".to_string());
        }
        lines.extend(self.findings.iter().cloned());
        let shown = self.findings.len();
        let total = self.broken + self.suspect;
        if total > shown {
            lines.push(format!("… and {} more", total - shown));
        }
        lines.join("\n")
    }
}

/// A shape run through the kernel's repair: the mended snapshot, its mesh,
/// what the repair did, and what the checker finds afterwards.
#[derive(Debug, Clone, Default)]
pub struct RepairResult {
    /// Native-format snapshot of the mended shape.
    pub brep_blob: Vec<u8>,
    /// Per-face colours for the mended shape, in its face order; empty when
    /// the repair changed the faces too much to carry them over.
    pub face_colors: Vec<[f32; 3]>,
    /// Render mesh of the mended shape.
    pub mesh: TriMesh,
    /// Axis-aligned bounds in millimetres.
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
    /// The checker's findings on the mended shape, marked repaired.
    pub health: ShapeHealth,
    /// What was mended, a phrase each ("12 tolerances tightened").
    pub mended: Vec<String>,
}

/// A mesh body turned into a B-rep by the kernel: the shape, its mesh, the
/// checker's verdict, and what the conversion found.
#[derive(Debug, Clone, Default)]
pub struct MeshSolidResult {
    /// Native-format snapshot of the shape.
    pub brep_blob: Vec<u8>,
    /// Per-face colours in the shape's face order: the mesh's colour on
    /// every face when it had one colour, else empty.
    pub face_colors: Vec<[f32; 3]>,
    /// Render mesh of the shape, with its kernel faces and edges.
    pub mesh: TriMesh,
    /// Axis-aligned bounds in millimetres.
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
    /// The checker's findings on the shape.
    pub health: ShapeHealth,
    /// Every piece of the mesh closed and became a solid; otherwise the
    /// shape is the open shell(s) the mesh makes.
    pub closed: bool,
    /// What the conversion did and where the mesh does not close, a phrase
    /// each ("12 triangles into 6 faces", "3 hole edges").
    pub summary: Vec<String>,
}

/// A body's measure: its volume, surface area and centre of mass, exact
/// where the kernel has a closed form for its faces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PhysicalProperties {
    /// Enclosed volume in mm³; `None` when the shape encloses none (an open
    /// shell, a sheet) or its shell is wound inside out.
    pub volume_mm3: Option<f64>,
    /// Total surface area in mm².
    pub area_mm2: f64,
    /// Centre of mass at uniform density, in mm; the centre of the surface
    /// when there is no volume.
    pub centre_mm: [f64; 3],
    /// Some face had no closed form, so the figures were integrated over a
    /// tessellation and are close rather than exact (within a fraction of a
    /// percent on curved walls).
    #[serde(default)]
    pub approximate: bool,
}

/// Node type emitted by STEP import hierarchy reconstruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImportedNodeKind {
    #[default]
    Assembly,
    Part,
    Instance,
    /// The group holding an import's annotations.
    Annotations,
    /// One annotation the file carries: a dimension, a tolerance, a datum
    /// or a note.
    Annotation,
}

/// What an imported annotation states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    /// A size or a location, with its value.
    Dimension,
    /// A geometric tolerance: flatness, position, profile and their kin.
    Tolerance,
    /// A datum letter, or a target a datum is contacted at.
    Datum,
    /// Text with nothing measured behind it.
    Note,
    /// A drawn annotation the file does not say more about.
    #[default]
    Other,
}

impl AnnotationKind {
    /// The kind as a word for the interface.
    pub fn label(self) -> &'static str {
        match self {
            AnnotationKind::Dimension => "Dimension",
            AnnotationKind::Tolerance => "Tolerance",
            AnnotationKind::Datum => "Datum",
            AnnotationKind::Note => "Note",
            AnnotationKind::Other => "Annotation",
        }
    }
}

/// One annotation an imported file carries (a dimension, a tolerance, a
/// datum or a note), with what it draws.
///
/// The polylines and the anchor are in document millimetres, placed where
/// the file's bodies are placed, so an annotation drawn on an assembly's
/// part sits on that part.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportedAnnotation {
    /// The name the file gives it (`Linear Size.3`, `Flatness.1`), or the
    /// text it shows where the file names it by that.
    pub name: String,
    pub kind: AnnotationKind,
    /// What a label shows: `Ø 35 ±0.2`, `Flatness 0.2`, `A`.
    pub text: String,
    /// The drawn geometry: leaders, frames, witness lines and whatever
    /// strokes the file draws its text with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub polylines: Vec<Vec<[f32; 3]>>,
    /// Where the label goes; `None` for an annotation that draws nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f32; 3]>,
    /// The body the annotation describes, as an index into
    /// [`ImportedModel::bodies`], where the file says which.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_index: Option<usize>,
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
    /// Everything the reader had to say about the file, kept whole so it
    /// can be handed to whoever maintains the kernel rather than scrolled
    /// past in a terminal.
    #[serde(default)]
    pub report: ImportReport,
    /// Optional assembly/object tree reconstructed from STEP/XCAF labels.
    #[serde(default)]
    pub nodes: Vec<ImportedNode>,
    /// Length unit declared by the source file, when detectable. Geometry in
    /// `bodies` is *always* expressed in millimetres regardless — this field
    /// is purely informational and used by the UI to pick a display unit.
    #[serde(default)]
    pub source_unit: Option<LengthUnit>,
    /// The file's annotations (dimensions, tolerances, datums and notes)
    /// in file order, the drawn ones first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<ImportedAnnotation>,
}

/// What the reader had to say about a file.
///
/// A community STEP file routinely produces a thousand lines of "this edge
/// misses its vertex by a micron"; each one is true and none is actionable
/// on its own. The kernel counts them by kind, and the whole prose stays
/// here for the report a user can send along with the file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    /// The kernel that read the file, by name and version.
    pub kernel: String,
    /// One entry per kind of imperfection, largest count first.
    pub summary: Vec<ImportWarningKind>,
    /// Every warning, one line each, in the order the reader met them.
    pub warnings: Vec<String>,
    /// STEP entity ids of faces that read without a complete trim, so they
    /// draw with gaps unless healed.
    pub untrimmed_faces: Vec<u64>,
    /// Entity keywords the reader never visited, with counts. Presentation
    /// and annotation land here by design; geometry landing here is a gap.
    pub skipped: Vec<(String, usize)>,
}

impl ImportReport {
    /// Whether there is anything worth writing down.
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty() && self.untrimmed_faces.is_empty()
    }
}

/// One kind of imperfect import, counted rather than repeated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportWarningKind {
    /// Stable across runs: `vertex-miss`, `boundary-slop`, `fit-short`,
    /// `untrimmed`.
    pub kind: String,
    pub count: usize,
    /// The worst measured value among them, in millimetres, for the kinds
    /// that measure one; zero otherwise.
    pub worst: f64,
    /// One entity id to look at first.
    pub exemplar: u64,
}

/// One segment of a closed 2D profile wire, in sketch-plane coordinates
/// (millimetres). Arcs are encoded as three on-curve points so consumers
/// never have to agree on a winding convention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Full ellipse. `major` is the vector from the center to a major-axis
    /// vertex; the minor radius is `|major| * ratio` with `ratio` in (0, 1].
    Ellipse {
        center: [f64; 2],
        major: [f64; 2],
        ratio: f64,
    },
    /// Elliptical arc between two parameter angles (radians, counter-clockwise
    /// in the ellipse frame where the major axis is at parameter 0).
    EllipseArc {
        center: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        start_param: f64,
        end_param: f64,
    },
    /// Cubic B-spline through the given control points. A periodic spline
    /// closes smoothly on itself; an open one runs first → last control point.
    BSpline {
        control_points: Vec<[f64; 2]>,
        periodic: bool,
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

/// A set of closed wires on one plane. The largest-area wire is the outer
/// boundary, the rest become holes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub plane: ProfilePlane,
    pub wires: Vec<ProfileWire>,
}

/// Where a one-directional extrusion stops.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExtrudeTermination {
    /// Fixed length in millimetres.
    Blind { distance: f64 },
    /// Extend well past the base solid's bounding box in this direction.
    ThroughAll,
    /// Stop on the given world-space plane, shifted by `offset` along its
    /// normal (a picked planar face plus an offset-to-face value).
    UpToPlane {
        point: [f64; 3],
        normal: [f64; 3],
        offset: f64,
    },
    /// Stop on the base solid's face nearest `point` (a picked face, where
    /// it was picked), exactly on its surface whatever its shape, pushed
    /// `offset` out along its outward normal. With no base solid, or no
    /// face of it there, the plane through `point` square to `normal`.
    UpToFace {
        point: [f64; 3],
        normal: [f64; 3],
        offset: f64,
    },
    /// Stop at the first face of the base solid hit along the extrusion
    /// direction, on its surface.
    ToFirst,
    /// Stop at the last face of the base solid hit along the extrusion
    /// direction, on its surface.
    ToLast,
}

/// How a profile is swept into a solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SweepKind {
    /// Linear extrusion along `plane.normal` (or `direction` when set).
    Extrude {
        termination: ExtrudeTermination,
        /// Independent termination for the opposite side of the sketch plane
        /// (two-sided extrusion). `None` extrudes one side only.
        second_side: Option<ExtrudeTermination>,
        /// Extrude half the blind distance to each side of the sketch plane.
        symmetric: bool,
        /// Swap which side of the sketch plane is "forward".
        reversed: bool,
        /// Draft angle applied along the sweep; positive widens the far end.
        taper_deg: f64,
        /// Custom world-space sweep direction; `None` uses the plane normal.
        direction: Option<[f64; 3]>,
    },
    /// Revolution about an axis lying IN the sketch plane, given in sketch
    /// 2D coordinates (point + direction). `angle_deg` in (0, 360].
    Revolve {
        axis_origin: [f64; 2],
        axis_dir: [f64; 2],
        angle_deg: f64,
        /// Independent sweep angle for the opposite rotation direction.
        second_angle_deg: Option<f64>,
        /// Center the sweep on the sketch plane (half the angle each way).
        midplane: bool,
        reversed: bool,
    },
    /// Sweep the profile along a helix whose axis lies in the sketch plane.
    /// `height == 0` produces a flat spiral driven by `cone_angle_deg`.
    Helix {
        axis_origin: [f64; 2],
        axis_dir: [f64; 2],
        pitch: f64,
        height: f64,
        left_handed: bool,
        cone_angle_deg: f64,
        reversed: bool,
    },
}

/// Parametric primitive shapes (dimensions in millimetres, angles degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PrimitiveKind {
    Box {
        length: f64,
        width: f64,
        height: f64,
    },
    Cylinder {
        radius: f64,
        height: f64,
        angle_deg: f64,
    },
    Sphere {
        radius: f64,
        /// Latitude range and longitude sweep (three-angle parameterization).
        angle1_deg: f64,
        angle2_deg: f64,
        angle3_deg: f64,
    },
    Cone {
        radius1: f64,
        radius2: f64,
        height: f64,
        angle_deg: f64,
    },
    Torus {
        radius1: f64,
        radius2: f64,
        angle1_deg: f64,
        angle2_deg: f64,
        angle3_deg: f64,
    },
    Ellipsoid {
        radius1: f64,
        radius2: f64,
        radius3: f64,
    },
    Prism {
        sides: u32,
        circumradius: f64,
        height: f64,
    },
    /// Box with an independently sized top rectangle (zero top spans give a
    /// pyramid). Spans are along local X (`x2*`) and Z (`z2*`) at height Y.
    Wedge {
        xmin: f64,
        xmax: f64,
        ymin: f64,
        ymax: f64,
        zmin: f64,
        zmax: f64,
        x2min: f64,
        x2max: f64,
        z2min: f64,
        z2max: f64,
    },
}

/// A right-handed placement frame in world coordinates (millimetres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub z_axis: [f64; 3],
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            z_axis: [0.0, 0.0, 1.0],
        }
    }
}

/// Which edges of the current solid a dress-up applies to. Edges are
/// identified geometrically: by a face they border (sample point on the
/// face) or by a point near the edge itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdgeSelection {
    All,
    OfFaces(Vec<[f64; 3]>),
    Near(Vec<[f64; 3]>),
    /// Edges picked one by one: each the nearest edge to its point that
    /// runs along its direction there.
    Picked(Vec<EdgeProbe>),
}

/// A picked edge, as a point beside it and the way it runs there. A zero
/// direction leaves the way open.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeProbe {
    pub point: [f64; 3],
    pub direction: [f64; 3],
}

/// Chamfer sizing, mirroring the three standard input styles.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChamferSpec {
    EqualDistance { distance: f64 },
    TwoDistances { distance1: f64, distance2: f64 },
    DistanceAngle { distance: f64, angle_deg: f64 },
}

/// Boolean between the running solid and an external tool solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoolKind {
    Fuse,
    Cut,
    Common,
}

/// One step in a body's build history. Shape-producing steps (`Sweep`,
/// `Loft`, `Pipe`, `Primitive`) carry a [`BooleanOp`]; the remaining steps
/// modify or combine the running solid directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SolidOp {
    /// Start the chain from a snapshot of another solid, in the native
    /// format `SolidBuildResult::brep_blob` carries. Only ever the first op.
    Shape {
        brep: Vec<u8>,
    },
    Sweep {
        profile: Profile,
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Skin through two or more section profiles (in order).
    Loft {
        sections: Vec<Profile>,
        ruled: bool,
        closed: bool,
        op: BooleanOp,
    },
    /// Sweep a profile along an open or closed spine wire from another
    /// sketch. `frenet` uses a Frenet frame instead of the corrected frame.
    Pipe {
        profile: Profile,
        spine: Profile,
        frenet: bool,
        op: BooleanOp,
    },
    Primitive {
        kind: PrimitiveKind,
        placement: Placement,
        op: BooleanOp,
    },
    Fillet {
        radius: f64,
        edges: EdgeSelection,
    },
    Chamfer {
        spec: ChamferSpec,
        flip: bool,
        edges: EdgeSelection,
    },
    /// Tilt the selected faces by `angle_deg` about their intersection with
    /// the neutral plane.
    Draft {
        angle_deg: f64,
        neutral_point: [f64; 3],
        neutral_normal: [f64; 3],
        pull_dir: Option<[f64; 3]>,
        faces: Vec<[f64; 3]>,
    },
    /// Merge adjacent faces of the solid that lie on one plane — the split
    /// a fuse or cut leaves where two pieces meet flush.
    Refine,
    /// Hollow the solid, removing the faces sampled by `open_faces`.
    Thickness {
        value: f64,
        open_faces: Vec<[f64; 3]>,
        inward: bool,
    },
    /// Re-apply earlier steps' tool solids (or the whole current solid when
    /// `originals` is empty) under each transform, fusing additive tools and
    /// cutting subtractive ones. `originals` holds chain indices of earlier
    /// shape-producing steps.
    Transform {
        transforms: Vec<[[f64; 4]; 4]>,
        originals: Vec<usize>,
    },
    /// Boolean against an external solid (another body's shape snapshot,
    /// ogeom native-format text bytes).
    Boolean {
        tool_brep: Vec<u8>,
        kind: BoolKind,
        /// Where the tool sits in this chain's frame, when the two bodies
        /// are placed differently: a rigid row-major 4×4 matrix.
        #[serde(default)]
        tool_transform: Option<[[f64; 4]; 4]>,
    },
}

impl SolidOp {
    /// The boolean role of a shape-producing step; `None` for modifiers.
    pub fn boolean_op(&self) -> Option<BooleanOp> {
        match self {
            SolidOp::Sweep { op, .. }
            | SolidOp::Loft { op, .. }
            | SolidOp::Pipe { op, .. }
            | SolidOp::Primitive { op, .. } => Some(*op),
            // A snapshot is a solid in itself: it begins a chain.
            SolidOp::Shape { .. } => Some(BooleanOp::NewSolid),
            _ => None,
        }
    }
}

/// Error from executing a body's solid-op chain, attributed to the failing
/// step so the caller can mark the matching feature.
#[derive(Debug, Clone, Error)]
#[error("op {op_index}: {message}")]
pub struct ChainError {
    pub op_index: usize,
    pub message: String,
}

/// Result of executing a body's solid-op chain.
#[derive(Debug, Clone, Default)]
pub struct SolidBuildResult {
    /// Native-format shape snapshot of the final solid (for later
    /// re-tessellation, persistence, and downstream booleans).
    pub brep_blob: Vec<u8>,
    /// Render mesh of the final solid.
    pub mesh: TriMesh,
    /// Axis-aligned bounds in millimetres.
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
    /// What the feature being edited does, when the build was asked for it.
    pub preview: Option<Box<FeaturePreview>>,
}

/// What a feature being edited does to its body, beside the body without
/// it: the view while it is edited.
#[derive(Debug, Clone, Default)]
pub struct FeaturePreview {
    /// The body to show meanwhile: as it stood before a feature that adds,
    /// after one that cuts. None when an adding feature is the body's
    /// first.
    pub shown: Option<Box<SolidBuildResult>>,
    /// The feature's own tool solid, meshed: the material it adds or takes.
    pub tool: TriMesh,
    /// The feature cuts.
    pub cuts: bool,
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

/// An edge of a solid projected onto a plane, in the plane's own 2D
/// coordinates (along its `x_axis` and `y_axis`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectedEdge {
    /// The edge runs along the plane's normal, so it projects to a point.
    Point([f64; 2]),
    /// A segment.
    Line { start: [f64; 2], end: [f64; 2] },
    /// A circle or circular arc, `centre + radius (cos t, sin t)`, turning
    /// counter-clockwise from `range.0` to `range.1`; a full circle when the
    /// range spans a turn.
    Circle {
        centre: [f64; 2],
        radius: f64,
        range: (f64, f64),
    },
    /// An ellipse or elliptical arc, `centre + major cos t + minor sin t`,
    /// `minor` the major axis turned a quarter counter-clockwise and scaled
    /// by `ratio`, from `range.0` to `range.1`.
    Ellipse {
        centre: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        range: (f64, f64),
    },
    /// Any other curve, as points along it: exact at each point, straight
    /// between.
    Polyline(Vec<[f64; 2]>),
}

/// The curves of a 2D drawing file, in the drawing's own coordinates.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Drawing2d {
    /// Millimetres per drawing unit, when the drawing names its unit.
    pub unit_mm: Option<f64>,
    /// Every curve, in the order the file has them.
    pub curves: Vec<DrawingCurve>,
}

/// One curve of a drawing, and whether the drawing marks it hidden
/// (a hidden layer, or a dashed line type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawingCurve {
    pub hidden: bool,
    pub shape: DrawingShape,
}

/// The kinds of curve a drawing carries. Angles are radians, arcs run
/// counter-clockwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawingShape {
    /// Points in order, each segment a line, or an arc where the bulge of
    /// the segment starting at that point (the tangent of a quarter of its
    /// included angle, positive counter-clockwise) is not zero. `bulges`
    /// is as long as `points`, or empty for straight segments only.
    Polyline {
        points: Vec<[f64; 2]>,
        bulges: Vec<f64>,
        closed: bool,
    },
    Arc {
        centre: [f64; 2],
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
    Circle {
        centre: [f64; 2],
        radius: f64,
    },
    /// Points `centre + major cos t + ratio perp(major) sin t` for `t`
    /// from `start_param` to `end_param`; the whole ellipse when those span
    /// a full turn.
    Ellipse {
        centre: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        start_param: f64,
        end_param: f64,
    },
    /// A spline, as points along the exact curve.
    Spline {
        points: Vec<[f64; 2]>,
        closed: bool,
    },
}

/// Geometry questions a workbench may ask while it runs, answered by the
/// kernel at once. Shapes arrive as the snapshot bytes the document keeps.
/// The solid two shapes share: its volume, its centre and its mesh, in
/// the first shape's frame.
#[derive(Debug, Clone)]
pub struct Overlap {
    pub volume_mm3: f64,
    pub centre_mm: [f64; 3],
    pub mesh: TriMesh,
}

/// A stretch of a region's medial axis, in the profile plane's own 2D
/// coordinates: points along one branch with the clearance at each, the
/// radius of the largest disc centred there that stays inside the region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MedialPath {
    pub points: Vec<[f64; 2]>,
    /// One per point, in millimetres.
    pub clearance: Vec<f64>,
    /// Whether the first and the last point end the axis on the boundary
    /// (a corner, or a rounded end's centre) rather than meet other
    /// branches: towards such an end the clearance falls without the
    /// region getting thinner.
    pub boundary_ends: [bool; 2],
}

/// Where a region is narrowest, and how narrow: the smallest clearance on
/// its medial axis away from the branch ends at its corners (where the
/// clearance runs down to nothing without the region getting any thinner).
/// The wall there is twice the clearance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Narrowest {
    pub at: [f64; 2],
    pub clearance: f64,
}

/// The medial axis of one region of a profile: an outer wire and its holes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MedialRegion {
    /// The profile's wires that bound it, by index, the outer one first.
    pub wires: Vec<usize>,
    pub paths: Vec<MedialPath>,
    /// `None` only when the axis has no place away from the corners.
    pub narrowest: Option<Narrowest>,
}

/// A picked face, named geometrically: a point on it and its outward
/// normal, resolved to the face nearest the point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceProbe {
    pub point: [f64; 3],
    pub normal: [f64; 3],
}

/// The centre line of a pipe-like solid between two of its faces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CentreLine {
    /// Points along it, from the first face to the second, in the shape's
    /// frame.
    pub points: Vec<[f64; 3]>,
    /// Its length, in millimetres.
    pub length: f64,
    /// The largest distance measured from the centroid of a section cut
    /// square to the line to the line itself.
    pub deviation: f64,
    /// It is one straight segment.
    pub straight: bool,
}

pub trait KernelQueries: Send + Sync {
    /// What `a` and `b` share when `b` sits where `b_in_a` (a rigid
    /// row-major 4×4 matrix) puts it in `a`'s frame; `None` when they only
    /// touch or are apart.
    fn overlap(
        &self,
        _a: &[u8],
        _b: &[u8],
        _b_in_a: &[[f64; 4]; 4],
    ) -> KernelResult<Option<Overlap>> {
        Err(KernelError::Unsupported("overlap".into()))
    }

    /// The edge of `brep` nearest `near`, projected orthogonally onto
    /// `plane`. Both are in the shape's own frame.
    fn project_edge(
        &self,
        brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<ProjectedEdge>;

    /// The curves of a DXF drawing, given as its text.
    fn read_dxf(&self, _text: &str) -> KernelResult<Drawing2d> {
        Err(KernelError::Unsupported("reading DXF".into()))
    }

    /// The medial axis of each region of `profile`, held to `tolerance`
    /// (mm), in the profile plane's own 2D coordinates.
    fn medial_axis(&self, _profile: &Profile, _tolerance: f64) -> KernelResult<Vec<MedialRegion>> {
        Err(KernelError::Unsupported("medial axis".into()))
    }

    /// The centre line of the solid in `brep` from the face `from` names to
    /// the face `to` names, held to `tolerance` (mm). Everything is in the
    /// shape's own frame.
    fn centre_line(
        &self,
        _brep: &[u8],
        _from: &FaceProbe,
        _to: &FaceProbe,
        _tolerance: f64,
    ) -> KernelResult<CentreLine> {
        Err(KernelError::Unsupported("centre line".into()))
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
