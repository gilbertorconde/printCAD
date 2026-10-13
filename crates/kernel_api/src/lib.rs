//! printCAD's geometry interface: meshes, profiles and solid operations
//! as plain data, with no geometry code. The app's kernel adapter answers
//! it, and workbench packages build their solids as [`SolidOp`] plans
//! through `printcad-bench-api`.
//!
//! Versions follow `printcad-bench-api`'s: 0.1.x speaks the workbench
//! contract `printcad:workbench@0.1`, and a patch release only adds.
//! [`FaceSurface`] and [`ProfileSegment`], which the app sends a package,
//! are `#[non_exhaustive]` unless the `exhaustive` feature (printCAD's
//! own) is on; [`SolidOp`] and the enums inside it keep their variants'
//! fields within a minor version.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

pub mod naming;
pub use naming::TopoName;

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
    /// Defaults to 30°.
    #[serde(default = "default_weld_angle_threshold_deg")]
    pub weld_angle_threshold_deg: f32,
    /// When true, import keeps each body's shape snapshot in `brep_blob`
    /// beside its mesh. Meshing happens during the import either way;
    /// serializing the snapshots is most of the work of reading a large
    /// assembly, and turning this off skips it. The STEP asset remains
    /// available in the document regardless.
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
    /// Empty when the source has no faces (a sketch, a datum, a mesh saved
    /// before faces were recorded), and a consumer falls back to geometry.
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
    /// The name of each kernel face, indexed like [`Self::faces`]: what a
    /// pick keeps so a rebuild finds the face again. Zero where a face has
    /// none; empty when the source names none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_names: Vec<TopoName>,
    /// The names of the two faces each kernel edge runs between, indexed
    /// like [`Self::edge_ids`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_faces: Vec<[TopoName; 2]>,
}

/// The kind and placement of a kernel face's surface, in the mesh's frame.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
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
/// The kernel returns one entry per placed body in the source file, meshed.
/// [`Self::brep_blob`] is empty when no snapshot was kept:
/// [`TessellationSettings::persist_brep_snapshot`] off, a full-mesh import,
/// or a mesh file, which has no shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedBody {
    /// Optional name extracted from the source file (e.g. STEP product label).
    pub name: Option<String>,
    /// Tessellated mesh ready for the viewport.
    #[serde(default)]
    pub mesh: TriMesh,
    /// Serialized shape snapshot for this body, when one was kept: ogeom
    /// native-format text bytes (`ogeom::io::native`, one root shape per
    /// blob).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brep_blob: Vec<u8>,
    /// Per-face linear RGB albedo in face-exploration order
    /// (`ogeom::topo::explore` with a Face filter) over the matching shape
    /// snapshot.
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
    /// The shape is a mesh converted as it is: its flat stretches are
    /// faces and its curved ones still the mesh's facets, which a refine
    /// can rebuild on the surfaces they approximate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub faceted: bool,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// One hierarchy node from the imported file's product structure.
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
    /// Human-readable name from the file's product labels when available.
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
    /// Optional assembly/object tree reconstructed from the file's product
    /// structure.
    #[serde(default)]
    pub nodes: Vec<ImportedNode>,
    /// Length unit declared by the source file, when detectable. Geometry in
    /// `bodies` is *always* expressed in millimetres regardless; this field
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
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
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
    /// A B-spline of any degree over its own knots, rational when it has
    /// weights: an exact conic arc, a spline drawn through points. An open
    /// one's knots are clamped (`control points + degree + 1` of them), so
    /// it runs first → last control point; a periodic one is a uniform ring
    /// over its control points closing smoothly on itself, and its knots
    /// are not used.
    Nurbs {
        degree: u32,
        knots: Vec<f64>,
        control_points: Vec<[f64; 2]>,
        /// One per control point; empty when every weight is one.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        weights: Vec<f64>,
        #[serde(default)]
        periodic: bool,
    },
}

/// What a [`SolidOp::PipeThrough`] runs along.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PipePath {
    /// A sketch's wire.
    Profile(Profile),
    /// Edges of the running solid, joined end to end.
    Edges(Vec<EdgeProbe>),
    /// Edges of another solid (a native-format snapshot, moved by
    /// `transform` into the body's frame), joined end to end; the probes
    /// are in the body's frame.
    EdgesOf {
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        edges: Vec<EdgeProbe>,
    },
}

/// One section of a [`SolidOp::LoftThrough`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LoftSection {
    Profile(Profile),
    /// The flat face of the running solid the probe finds, by its outer
    /// and inner boundaries.
    Face(FaceProbe),
    /// A point the loft closes to, in the body's frame.
    Point([f64; 3]),
}

/// A closed loop of profile segments. Consecutive segments share endpoints;
/// a single `Circle` segment is a wire by itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfileWire {
    pub segments: Vec<ProfileSegment>,
    /// The name of each segment, one per segment, by what it was drawn from
    /// (a sketch element): the faces a feature sweeps from it are named by
    /// it, so they keep their names while dimensions change. Empty, or a
    /// zero, where a segment has no such name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<TopoName>,
}

impl ProfileWire {
    /// A wire of `segments` with no names.
    pub fn new(segments: Vec<ProfileSegment>) -> Self {
        Self {
            segments,
            names: Vec::new(),
        }
    }

    /// The name of segment `index`, when it has one.
    pub fn name_of(&self, index: usize) -> Option<TopoName> {
        self.names.get(index).copied().filter(|n| *n != 0)
    }
}

/// The plane a profile lives on, in world coordinates (millimetres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfilePlane {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

/// The XY plane through the origin.
impl Default for ProfilePlane {
    fn default() -> Self {
        Self {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        }
    }
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
    /// Keep only what the existing solid and the tool share.
    Common,
}

/// A set of closed wires on one plane. The largest-area wire is the outer
/// boundary, the rest become holes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub plane: ProfilePlane,
    pub wires: Vec<ProfileWire>,
}

/// Where a one-directional extrusion stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Stop on the base solid's face named `name`, else the one nearest
    /// `point` (a picked face, where it was picked), exactly on its surface
    /// whatever its shape, pushed `offset` out along its outward normal.
    /// With no base solid, or no face of it there, the plane through
    /// `point` square to `normal`.
    UpToFace {
        point: [f64; 3],
        normal: [f64; 3],
        offset: f64,
        #[serde(default)]
        name: TopoName,
    },
    /// Stop at the first face of the base solid hit along the extrusion
    /// direction, on its surface.
    ToFirst,
    /// Stop at the last face of the base solid hit along the extrusion
    /// direction, on its surface.
    ToLast,
    /// Stop on a set of the base solid's faces: every line of the sweep
    /// ends at the first of them it meets, moved `offset` further along the
    /// sweep (a negative offset stops short).
    UpToShape { faces: Vec<FaceProbe>, offset: f64 },
    /// Stop on a face of another shape: the face of `shape` (a native-format
    /// snapshot, moved by `transform`, a rigid row-major 4×4 matrix, into
    /// the body's frame) nearest `point`, exactly on its surface whatever
    /// its shape, pushed `offset` out along its outward normal.
    UpToFaceOf {
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        point: [f64; 3],
        offset: f64,
    },
    /// Stop on a set of faces as `UpToShape` does, some of them the base
    /// solid's (`faces`) and some of other shapes (`others`, faces other
    /// bodies lend this one). Needs no base solid when every face is
    /// another shape's.
    UpToShapeOf {
        faces: Vec<FaceProbe>,
        others: Vec<FacesOf>,
        offset: f64,
    },
}

/// Faces of another shape: the faces of `shape` (a native-format snapshot,
/// moved by `transform`, a rigid row-major 4×4 matrix, into the body's
/// frame) nearest each of `points`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FacesOf {
    pub shape: Vec<u8>,
    pub transform: Option<Box<[[f64; 4]; 4]>>,
    pub points: Vec<[f64; 3]>,
}

/// Where a revolution stops turning.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum RevolveTermination {
    /// Through the sweep's own angles.
    #[default]
    Angle,
    /// On the first face of the base solid the profile's centre meets as it
    /// turns.
    ToFirst,
    /// On the last face of the base solid the profile's centre meets as it
    /// turns.
    ToLast,
    /// On the base solid's face nearest the probe.
    UpToFace(FaceProbe),
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
        /// Where the turn stops; anything but `Angle` takes the angle from
        /// what the profile meets and leaves the angles above unused.
        #[serde(default)]
        termination: RevolveTermination,
    },
    /// Sweep the profile along a helix whose axis lies in the sketch plane.
    /// A cone angle moves every point away from the axis by tan(angle) of
    /// every pitch it climbs.
    Helix {
        axis_origin: [f64; 2],
        axis_dir: [f64; 2],
        pitch: f64,
        height: f64,
        left_handed: bool,
        cone_angle_deg: f64,
        reversed: bool,
        /// How many turns, when given: the pitch is then `height / turns`
        /// and `pitch` is not read. A height of 0 with turns and growth is
        /// a flat spiral.
        #[serde(default)]
        turns: Option<f64>,
        /// How far every point moves away from the axis per turn, when
        /// given, in place of `cone_angle_deg`.
        #[serde(default)]
        growth: Option<f64>,
    },
    /// Sweep the profile along a helix whose axis runs square to the sketch
    /// plane through `axis_origin` (sketch 2D coordinates): every point of
    /// the profile climbs its own helix about it, the profile staying
    /// level. The rest reads as for `Helix`.
    HelixNormal {
        axis_origin: [f64; 2],
        pitch: f64,
        height: f64,
        left_handed: bool,
        cone_angle_deg: f64,
        reversed: bool,
        #[serde(default)]
        turns: Option<f64>,
        #[serde(default)]
        growth: Option<f64>,
    },
}

/// How a pipe's section turns as it runs down its path.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum PipeFrame {
    /// Rotation-minimizing: the section neither twists nor kinks where the
    /// path bends.
    #[default]
    RotationMinimizing,
    /// The path's own curvature frame, turning with it.
    Frenet,
    /// The section turns about the path so that one direction of it keeps
    /// pointing at a second path running beside the first.
    Auxiliary { path: Profile },
    /// The section keeps a fixed binormal: the world-space direction square
    /// to the path that the section's frame holds all the way.
    Binormal { direction: [f64; 3] },
    /// The section keeps its orientation in space: carried along the path
    /// without ever turning, as it is drawn.
    Fixed,
}

impl PipeFrame {
    /// The frame an older `frenet` flag named.
    pub fn from_frenet(frenet: bool) -> Self {
        if frenet {
            PipeFrame::Frenet
        } else {
            PipeFrame::RotationMinimizing
        }
    }
}

/// Reads a [`PipeFrame`] or the `frenet` flag it stands in for.
pub fn deserialize_pipe_frame<'de, D>(deserializer: D) -> Result<PipeFrame, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Flag(bool),
        Frame(PipeFrame),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Flag(frenet) => PipeFrame::from_frenet(frenet),
        Stored::Frame(frame) => frame,
    })
}

/// How a pipe's section turns a sharp corner of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PipeCorner {
    /// The section is carried into the corner and sheared onto the plane
    /// bisecting it: the two legs meet on that mitre.
    #[default]
    Transformed,
    /// Each leg runs on straight past the corner and the two are fused,
    /// leaving the outside of the corner square.
    Right,
    /// The section turns about the corner, the outside of it rounded.
    Round,
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
    /// An ellipsoid, cut as a sphere is: between two latitudes about its
    /// third axis and through a sweep round it.
    Ellipsoid {
        radius1: f64,
        radius2: f64,
        radius3: f64,
        #[serde(default = "south_pole")]
        angle1_deg: f64,
        #[serde(default = "north_pole")]
        angle2_deg: f64,
        #[serde(default = "full_turn")]
        angle3_deg: f64,
    },
    /// A regular prism, its top shifted by the skew angles: leaning that
    /// far from its axis toward its x and its y.
    Prism {
        sides: u32,
        circumradius: f64,
        height: f64,
        #[serde(default)]
        skew_x_deg: f64,
        #[serde(default)]
        skew_y_deg: f64,
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

fn south_pole() -> f64 {
    -90.0
}

fn north_pole() -> f64 {
    90.0
}

fn full_turn() -> f64 {
    360.0
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
    /// The edges bordering the picked faces, each found by its name when
    /// it has one.
    OfPickedFaces(Vec<FaceProbe>),
    Near(Vec<[f64; 3]>),
    /// Edges picked one by one: each the nearest edge to its point that
    /// runs along its direction there.
    Picked(Vec<EdgeProbe>),
}

/// A picked edge, as a point beside it and the way it runs there, and the
/// names of the two faces it runs between. A rebuild finds the edge
/// between faces of those names, the point choosing among several; without
/// names, the nearest edge. A zero direction leaves the way open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeProbe {
    pub point: [f64; 3],
    pub direction: [f64; 3],
    #[serde(default)]
    pub faces: [TopoName; 2],
}

/// How a thickness's walls meet across an edge of the solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThicknessJoin {
    /// The walls run on until they meet: sharp corners.
    #[default]
    Intersection,
    /// The walls round about the edge, the wall thickness its radius.
    Arc,
}

impl ThicknessJoin {
    pub const ALL: [ThicknessJoin; 2] = [ThicknessJoin::Intersection, ThicknessJoin::Arc];

    pub fn label(&self) -> &'static str {
        match self {
            ThicknessJoin::Intersection => "Intersection",
            ThicknessJoin::Arc => "Arc",
        }
    }
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
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SolidOp {
    /// Start the chain from a snapshot of another solid, in the native
    /// format `SolidBuildResult::brep_blob` carries. Only ever the first op.
    Shape { brep: Vec<u8> },
    Sweep {
        profile: Profile,
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Extrude a flat face of the running solid: its outer and inner
    /// boundaries are the profile and its outward normal the sketch
    /// normal. `kind` is an `Extrude`.
    SweepFace {
        face: FaceProbe,
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Extrude a flat face of another shape (native-format `shape`, moved by
    /// `transform` into this body's frame): the face of it at `point`, a
    /// face another body lends this one.
    SweepFaceOf {
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        point: [f64; 3],
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Revolve a profile until it meets a face of another shape
    /// (native-format `shape`, moved by `transform` into this body's frame):
    /// the face of it nearest `point`, a face another body lends this one.
    /// `kind` is a `Revolve`, whose own termination is not read.
    RevolveToFaceOf {
        profile: Profile,
        kind: SweepKind,
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        point: [f64; 3],
        op: BooleanOp,
    },
    /// Skin through two or more sections (in order), each a profile, a
    /// flat face of the running solid (its boundary) or a point, which
    /// only the first or last may be.
    LoftThrough {
        sections: Vec<LoftSection>,
        ruled: bool,
        closed: bool,
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
    /// sketch, the section turning as `frame` says and turning its corners
    /// as `corner` says. With `sections`, the section changes shape down
    /// the path, passing through each of them where it crosses the path,
    /// in order, the profile first.
    Pipe {
        profile: Profile,
        spine: Profile,
        #[serde(default, alias = "frenet", deserialize_with = "deserialize_pipe_frame")]
        frame: PipeFrame,
        #[serde(default)]
        corner: PipeCorner,
        #[serde(default)]
        sections: Vec<Profile>,
        op: BooleanOp,
    },
    /// A pipe whose profile may be a flat face of the running solid, whose
    /// path may be edges of a solid, and whose last section may be a point
    /// it closes to; the rest reads as for `Pipe`.
    PipeThrough {
        profile: LoftSection,
        path: PipePath,
        #[serde(default)]
        frame: PipeFrame,
        #[serde(default)]
        corner: PipeCorner,
        #[serde(default)]
        sections: Vec<LoftSection>,
        op: BooleanOp,
    },
    Primitive {
        kind: PrimitiveKind,
        placement: Placement,
        op: BooleanOp,
    },
    /// Round the selected edges; with `follow_tangent`, every edge meeting
    /// one of them tangentially joins the selection, and so on along the
    /// chain.
    Fillet {
        radius: f64,
        edges: EdgeSelection,
        #[serde(default)]
        follow_tangent: bool,
    },
    /// Bevel the selected edges, taking tangent chains as a fillet does.
    Chamfer {
        spec: ChamferSpec,
        flip: bool,
        edges: EdgeSelection,
        #[serde(default)]
        follow_tangent: bool,
    },
    /// Tilt the selected faces by `angle_deg` about their intersection with
    /// the neutral plane.
    Draft {
        angle_deg: f64,
        neutral_point: [f64; 3],
        neutral_normal: [f64; 3],
        pull_dir: Option<[f64; 3]>,
        faces: Vec<[f64; 3]>,
        /// The name of each face, one per point, found by name first; empty
        /// or zero where a face has none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
    },
    /// Merge adjacent faces of the solid that lie on one plane: the split
    /// a fuse or cut leaves where two pieces meet flush.
    Refine,
    /// Offset faces of the running solid along their outward normals by
    /// `distance` (negative into the material), the faces around them
    /// following on their own surfaces.
    OffsetFaces {
        faces: Vec<[f64; 3]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
        distance: f64,
    },
    /// Move faces of the running solid rigidly by `transform` (row-major,
    /// a translation or a rotation), the faces around them following.
    MoveFaces {
        faces: Vec<[f64; 3]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
        transform: [[f64; 4]; 4],
    },
    /// Remove faces of the running solid and close the openings from the
    /// neighbouring faces' own geometry: a hole, a boss or a fillet taken
    /// away. Faces as points on them, named where they have names.
    RemoveFaces {
        faces: Vec<[f64; 3]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
    },
    /// Hollow the solid, removing the faces sampled by `open_faces`.
    Thickness {
        value: f64,
        open_faces: Vec<[f64; 3]>,
        /// The name of each open face, one per point; empty or zero where
        /// a face has none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        open_face_names: Vec<TopoName>,
        inward: bool,
        /// How the walls meet where the solid's faces meet.
        #[serde(default)]
        join: ThicknessJoin,
        /// The walls stand on both sides of the solid's faces, `value` each
        /// way, rather than on the one `inward` says.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        both_sides: bool,
    },
    /// Re-apply earlier steps' tool solids (or the whole current solid when
    /// `originals` is empty) under each transform, fusing additive tools and
    /// cutting subtractive ones. `originals` holds chain indices of earlier
    /// shape-producing steps.
    Transform {
        transforms: Vec<[[f64; 4]; 4]>,
        originals: Vec<usize>,
    },
    /// [`SolidOp::Transform`] with the originals themselves left out: the
    /// steps `originals` names build their tools without adding or cutting
    /// them, so only the transformed copies go in. With `originals` empty
    /// the running solid gives way to its copies.
    TransformCopies {
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
    /// A surface step: a sheet added to the body, or the body's sheets
    /// joined, trimmed or mirrored.
    Surface(SurfaceOp),
    /// `op` with its booleans run at a stated tolerance: geometry within
    /// `tolerance` mm of touching counts as touching, so faces that nearly
    /// meet are joined rather than left with a gap or a sliver between
    /// them. The op keeps its place in the chain, as a pattern's
    /// `originals` count it.
    WithTolerance { tolerance: f64, op: Box<SolidOp> },
    /// A sketch carried along its plane's normal onto faces of the running
    /// solid, each face split along the curves where they land; `mode`
    /// says whether the regions the curves enclose then stand out of the
    /// faces or sink into them by `height`.
    Project {
        profile: Profile,
        faces: Vec<FaceProbe>,
        mode: ProjectMode,
        height: f64,
    },
}

/// What a [`SolidOp::Project`] makes of the faces a sketch lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ProjectMode {
    /// The faces split along the sketch's curves, the solid's shape
    /// unchanged: lines later features and face colours can use.
    #[default]
    Split,
    /// The regions the curves enclose raised out of the faces.
    Emboss,
    /// The regions the curves enclose sunk into the faces.
    Deboss,
}

impl ProjectMode {
    pub const ALL: [ProjectMode; 3] =
        [ProjectMode::Split, ProjectMode::Emboss, ProjectMode::Deboss];

    pub fn label(&self) -> &'static str {
        match self {
            ProjectMode::Split => "Split lines",
            ProjectMode::Emboss => "Emboss",
            ProjectMode::Deboss => "Deboss",
        }
    }
}

/// A curve a surface is built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CurveSource {
    /// A chain of a sketch's curves, open or closed, on its plane.
    Sketch {
        plane: ProfilePlane,
        wire: ProfileWire,
    },
    /// An edge of the body's own shape as it stands where the step comes,
    /// found as a fillet's picked edges are.
    Edge(EdgeProbe),
    /// An edge of another body's shape: `brep` is that body's snapshot,
    /// `edge` finds the edge in its own frame, and `transform` (a rigid
    /// row-major 4×4 matrix) carries it to where it sits in this chain's
    /// frame, when the two bodies are placed differently.
    BodyEdge {
        brep: Vec<u8>,
        edge: EdgeProbe,
        #[serde(default)]
        transform: Option<Box<[[f64; 4]; 4]>>,
    },
}

/// How a surface meets the faces beside it across a shared edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Continuity {
    /// Touching: no gap, a crease allowed.
    #[default]
    G0,
    /// Tangent: no crease.
    G1,
    /// Curvature continuous: no change in how sharply it bends.
    G2,
}

/// The steps of surface modelling. A constructive step (every one but
/// `Sew`, `TrimByPlane` and `Mirror`) adds its sheet to the body beside
/// what is there; the rest act on all of the body's shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceOp {
    /// Each curve swept straight along `direction` by `length`, both ways
    /// by half of it when `symmetric`.
    Extrude {
        curves: Vec<CurveSource>,
        direction: [f64; 3],
        length: f64,
        #[serde(default)]
        symmetric: bool,
    },
    /// Each curve turned by `angle_deg` about the axis through `origin`
    /// along `axis`.
    Revolve {
        curves: Vec<CurveSource>,
        origin: [f64; 3],
        axis: [f64; 3],
        angle_deg: f64,
    },
    /// The flat face closed planar loops bound, a loop inside another a
    /// hole in it.
    PlanarFill { curves: Vec<CurveSource> },
    /// The surface curves meeting end to end bound, meeting the faces the
    /// boundary edges belong to with `continuity`.
    Fill {
        boundary: Vec<CurveSource>,
        #[serde(default)]
        continuity: Continuity,
    },
    /// The surface of straight lines between two curves, end to end.
    Ruled {
        first: CurveSource,
        second: CurveSource,
    },
    /// A surface through section curves in order, closing back to the first
    /// when `closed`.
    Loft {
        sections: Vec<CurveSource>,
        #[serde(default)]
        closed: bool,
    },
    /// A profile curve swept along a path.
    Sweep {
        profile: Vec<CurveSource>,
        path: Vec<CurveSource>,
        #[serde(default)]
        frame: PipeFrame,
    },
    /// A surface through section curves in order that also follows guide
    /// curves, each crossing every section once: one face.
    GuidedLoft {
        sections: Vec<CurveSource>,
        guides: Vec<CurveSource>,
    },
    /// A profile running from the first rail's start to the second's,
    /// swept along both: at each point along them it is turned and scaled
    /// so its ends ride the rails.
    SweepTwoRails {
        profile: Vec<CurveSource>,
        first_rail: Vec<CurveSource>,
        second_rail: Vec<CurveSource>,
    },
    /// Faces carried off along their normals by `distance`, as new faces.
    Offset {
        faces: Vec<FaceProbe>,
        distance: f64,
    },
    /// Faces carried past picked edges of their boundary by `length`.
    Extend {
        edges: Vec<EdgeProbe>,
        length: f64,
        #[serde(default)]
        continuity: Continuity,
    },
    /// A surface bridging two edges, meeting each edge's face with
    /// `continuity`.
    Blend {
        first: EdgeProbe,
        second: EdgeProbe,
        #[serde(default)]
        continuity: Continuity,
    },
    /// Picked faces split along curves projected onto them.
    Split {
        faces: Vec<FaceProbe>,
        curves: Vec<CurveSource>,
    },
    /// The body's faces joined where their edges meet, or come within
    /// `gap` of each other (0 for exactly); a shell that closes becomes a
    /// solid.
    Sew {
        #[serde(default)]
        gap: f64,
    },
    /// A round of `radius` along picked edges where two faces of a sheet
    /// meet.
    Fillet { edges: Vec<EdgeProbe>, radius: f64 },
    /// A round of `radius` between two faces of the body that need share
    /// no edge, such as two separate surfaces, each cut back to where it
    /// meets the round. The ball rolls on the side each face's normal
    /// points to, on the other where `flip` says so.
    FilletFaces {
        first: FaceProbe,
        second: FaceProbe,
        radius: f64,
        #[serde(default)]
        flip: [bool; 2],
    },
    /// The body's sheets given `thickness` along their normals (both ways
    /// by half when `both_sides`): a solid.
    Thicken {
        thickness: f64,
        #[serde(default)]
        both_sides: bool,
    },
    /// The body's sheets cut by the plane through `origin`, the side
    /// `normal` points to kept.
    TrimByPlane { origin: [f64; 3], normal: [f64; 3] },
    /// The body's shape and its reflection in the plane through `origin`
    /// square to `normal`.
    Mirror { origin: [f64; 3], normal: [f64; 3] },
}

impl SurfaceOp {
    /// Whether the step makes a new sheet (and so may begin a chain).
    pub fn constructs(&self) -> bool {
        !matches!(
            self,
            SurfaceOp::Sew { .. }
                | SurfaceOp::Fillet { .. }
                | SurfaceOp::FilletFaces { .. }
                | SurfaceOp::Thicken { .. }
                | SurfaceOp::TrimByPlane { .. }
                | SurfaceOp::Mirror { .. }
                | SurfaceOp::Split { .. }
                | SurfaceOp::Extend { .. }
        )
    }
}

impl SolidOp {
    /// The op itself, out of any [`SolidOp::WithTolerance`] around it.
    pub fn inner(&self) -> &SolidOp {
        match self {
            SolidOp::WithTolerance { op, .. } => op.inner(),
            op => op,
        }
    }

    /// The tolerance the op's booleans run at, when one is stated: the
    /// outermost [`SolidOp::WithTolerance`]'s.
    pub fn boolean_tolerance(&self) -> Option<f64> {
        match self {
            SolidOp::WithTolerance { tolerance, .. } => Some(*tolerance),
            _ => None,
        }
    }

    /// The op with its booleans run at `tolerance`, or as it is with none.
    pub fn with_boolean_tolerance(self, tolerance: Option<f64>) -> SolidOp {
        match tolerance {
            Some(tolerance) => SolidOp::WithTolerance {
                tolerance,
                op: Box::new(self),
            },
            None => self,
        }
    }

    /// Whether the step makes a shape of its own, so a chain may begin with
    /// it: a new solid, or a surface step that adds a sheet.
    pub fn starts_shape(&self) -> bool {
        match self.inner() {
            SolidOp::Surface(op) => op.constructs(),
            _ => self.boolean_op() == Some(BooleanOp::NewSolid),
        }
    }

    /// The boolean role of a shape-producing step; `None` for modifiers.
    pub fn boolean_op(&self) -> Option<BooleanOp> {
        match self.inner() {
            SolidOp::Sweep { op, .. }
            | SolidOp::SweepFace { op, .. }
            | SolidOp::SweepFaceOf { op, .. }
            | SolidOp::RevolveToFaceOf { op, .. }
            | SolidOp::Loft { op, .. }
            | SolidOp::LoftThrough { op, .. }
            | SolidOp::PipeThrough { op, .. }
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// The answers to the chain's probes, in the order they were asked.
    pub probes: Vec<Result<ProbeAnswer, String>>,
}

/// What a feature being edited does to its body, beside the body without
/// it: the view while it is edited.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// A piece of a parabola, `vertex + axis s² / (4 focal) + side s`,
    /// `axis` a unit vector the way it opens and `side` it turned a quarter
    /// counter-clockwise, from `s = range.0` to `range.1`.
    Parabola {
        vertex: [f64; 2],
        axis: [f64; 2],
        focal: f64,
        range: (f64, f64),
    },
    /// A piece of one branch of a hyperbola, `centre + major cosh s +
    /// minor sinh s`, `major` (centre to the branch's vertex) square to
    /// `minor`, from `s = range.0` to `range.1`.
    Hyperbola {
        centre: [f64; 2],
        major: [f64; 2],
        minor: [f64; 2],
        range: (f64, f64),
    },
    /// A B-spline, as the kernel projects it: `degree`, its clamped
    /// `knots` (`control_points + degree + 1` of them), and a weight per
    /// control point when rational (empty when every weight is one).
    Spline {
        degree: u32,
        knots: Vec<f64>,
        control_points: Vec<[f64; 2]>,
        weights: Vec<f64>,
    },
    /// Any other curve, as points along it: exact at each point, straight
    /// between.
    Polyline(Vec<[f64; 2]>),
}

/// One curve where a plane crosses a shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionCurve {
    /// The exact curve, in the plane's own 2D coordinates.
    pub curve: ProjectedEdge,
    /// Points along it, in the shape's own frame, in the order its run
    /// goes.
    pub points: Vec<[f64; 3]>,
    /// The run goes against the way `curve` runs.
    #[serde(default)]
    pub reversed: bool,
}

/// Curves where a plane crosses a shape, end to end: a closed loop round
/// a solid's cut, or an open run where a sheet's edge stops it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionRun {
    pub curves: Vec<SectionCurve>,
    pub closed: bool,
}

/// One edge of a face's boundary loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopEdge {
    /// Points along it, in the shape's own frame, in the order the loop
    /// walks it.
    pub points: Vec<[f64; 3]>,
    /// The edge a round face meets itself along: it turns up twice in
    /// its loop, once each way.
    #[serde(default)]
    pub seam: bool,
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

/// The solid two shapes share: its volume, its centre and its mesh, in
/// the first shape's frame.
#[derive(Debug, Clone)]
pub struct Overlap {
    pub volume_mm3: f64,
    pub centre_mm: [f64; 3],
    pub mesh: TriMesh,
}

/// How near two shapes come: the distance between their boundaries and a
/// nearest point on each, in the first shape's frame. Zero where the
/// boundaries touch or cross.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub distance_mm: f64,
    pub on_a: [f64; 3],
    pub on_b: [f64; 3],
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

/// A picked face: its name, and a point on it with its outward normal.
/// A rebuild finds the face by its name, the point choosing among the
/// pieces of a face that split; without a name (or when no face bears it
/// any more), the face nearest the point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct FaceProbe {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    #[serde(default)]
    pub name: TopoName,
}

/// A question about a solid a reference stands on, named geometrically so
/// it can be asked again of the solid a rebuild makes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ShapeProbe {
    /// The face named `name`, else the face nearest `point` (among faces
    /// as near, the one whose outward normal agrees best with `normal`),
    /// and where on it `point` falls.
    Face {
        point: [f64; 3],
        normal: [f64; 3],
        #[serde(default)]
        name: TopoName,
    },
    /// The edge between faces named `faces`, else the edge nearest `point`
    /// that runs along `direction` there (a zero direction takes the
    /// nearest edge whichever way it runs).
    Edge {
        point: [f64; 3],
        direction: [f64; 3],
        #[serde(default)]
        faces: [TopoName; 2],
        /// A share along the edge from its start (0 to 1) whose point and
        /// tangent the answer also gives.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        along: Option<f64>,
    },
    /// The solid's centre of mass and principal axes of inertia.
    Mass,
}

/// What a solid answers to a [`ShapeProbe`], in the solid's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProbeAnswer {
    /// The point of the face nearest the probe's, the face's outward normal
    /// there, and the face's surface.
    Face {
        point: [f64; 3],
        normal: [f64; 3],
        surface: FaceSurface,
    },
    /// The point of the edge nearest the probe's and the way the edge runs
    /// there, its two ends and its middle (halfway along its parameter,
    /// which is halfway along an arc), and the circle it runs round when
    /// it is a circle or an arc of one.
    Edge {
        point: [f64; 3],
        direction: [f64; 3],
        start: [f64; 3],
        end: [f64; 3],
        middle: [f64; 3],
        circle: Option<ProbedCircle>,
        /// The point and tangent the probe's share along gives, when it
        /// asked for one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        along: Option<([f64; 3], [f64; 3])>,
    },
    /// The centre of mass and the principal axes of inertia, the axis of
    /// the smallest moment first.
    Mass {
        centre: [f64; 3],
        axes: [[f64; 3]; 3],
    },
}

/// The circle an edge runs round.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProbedCircle {
    pub centre: [f64; 3],
    pub normal: [f64; 3],
    pub radius: f64,
}

/// A probe asked part way through a solid-op chain: of the solid the first
/// `after_op` ops make.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChainProbe {
    pub after_op: usize,
    pub probe: ShapeProbe,
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

/// A round hole found in a solid: a full bore, open at one end (blind)
/// or both (through), closed by a flat bottom or a drill point.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedHole {
    /// The centre of its opening, on its axis.
    pub entry: [f64; 3],
    /// Along its axis, into the material.
    pub direction: [f64; 3],
    pub diameter: f64,
    /// The bore's length from the opening, not counting a drill point.
    pub depth: f64,
    /// Open at both ends.
    pub through: bool,
    /// The included angle of the cone that ends a blind hole, degrees;
    /// `None` for a flat bottom or a through hole.
    pub drill_point_deg: Option<f64>,
    /// A point on each face the hole is made of (its bore, its bottom),
    /// with that face's outward normal.
    pub faces: Vec<([f64; 3], [f64; 3])>,
}

/// Geometry questions a workbench may ask while it runs, answered by the
/// kernel at once. Shapes arrive as the snapshot bytes the document keeps.
/// How two faces of a shape meet across an edge they share: a point
/// halfway along it, in the shape's own frame, the largest gap between the
/// edge and the faces, the largest angle between the faces' normals along
/// it (zero where they meet tangent), and the largest difference in how
/// sharply they bend square to it (1/mm; zero where they meet curvature
/// continuous, `None` where it could not be read).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeContinuity {
    pub point: [f64; 3],
    pub gap: f64,
    pub angle_deg: f64,
    #[serde(default)]
    pub curvature: Option<f64>,
}

/// A face of a shape, named by a point inside it, and points on it, each
/// with the normal its curvature is signed against.
pub type FacePoints = ([f64; 3], Vec<([f64; 3], [f64; 3])>);

pub trait KernelQueries: Send + Sync {
    /// Every edge two faces of `brep` share, with how they meet there.
    fn continuity(&self, _brep: &[u8]) -> KernelResult<Vec<EdgeContinuity>> {
        Err(KernelError::Unsupported("continuity".into()))
    }

    /// The principal curvatures of `brep`'s faces at points on them. Each
    /// entry names a face by a point inside it (the face nearest that
    /// point) and lists points on it, each with the normal it is signed
    /// against, in the shape's own frame; the answer gives, per point, the
    /// largest and the smallest normal curvature (1/mm): negative where the
    /// face bulges toward that normal, positive where it hollows away.
    /// `None` where the surface has no normal (a pole, an apex).
    fn curvature(
        &self,
        _brep: &[u8],
        _faces: &[FacePoints],
    ) -> KernelResult<Vec<Vec<Option<[f64; 2]>>>> {
        Err(KernelError::Unsupported("curvature".into()))
    }

    /// The edges bounding the face of `brep` nearest `near`: a point on
    /// each, halfway along, and its direction there, in the shape's own
    /// frame. A seam (the edge a round face meets itself along) and an
    /// edge with no curve (a pole) are left out.
    fn face_edges(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<([f64; 3], [f64; 3])>> {
        Err(KernelError::Unsupported("face_edges".into()))
    }

    /// The round holes of `brep`, in its own frame, and how many bores it
    /// has that are not holes it can describe (a counterbore, a slot).
    fn recognize_holes(&self, _brep: &[u8]) -> KernelResult<(Vec<RecognizedHole>, usize)> {
        Err(KernelError::Unsupported("recognize_holes".into()))
    }

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

    /// How near `a` and `b` come when `b` sits where `b_in_a` puts it in
    /// `a`'s frame.
    fn gap(&self, _a: &[u8], _b: &[u8], _b_in_a: &[[f64; 4]; 4]) -> KernelResult<Gap> {
        Err(KernelError::Unsupported("gap".into()))
    }

    /// `brep` mirrored across the plane through `point` with `normal`, in
    /// its own frame, as a snapshot of its own.
    fn mirror(&self, _brep: &[u8], _point: [f64; 3], _normal: [f64; 3]) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported("mirror".into()))
    }

    /// The volume, area and centre of mass of `brep`, in its own frame.
    fn measure(&self, _brep: &[u8]) -> KernelResult<PhysicalProperties> {
        Err(KernelError::Unsupported("measure".into()))
    }

    /// The edge of `brep` nearest `near`, projected orthogonally onto
    /// `plane`. Both are in the shape's own frame.
    fn project_edge(
        &self,
        brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<ProjectedEdge>;

    /// The face of `brep` nearest `near` (in the shape's own frame), as a
    /// native-format snapshot of its own: what a face taken from a solid
    /// keeps once the solid moves on.
    fn face_of(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported(
            "taking a face out of a solid".into(),
        ))
    }

    /// Where the face of `brep` nearest `near` crosses `plane`, as the
    /// curves the plane cuts from it, in the plane's own 2D coordinates:
    /// each the exact line, circle or ellipse it is, or points along any
    /// other. Empty when the face does not reach the plane. Both are in the
    /// shape's own frame.
    fn section_face(
        &self,
        _brep: &[u8],
        _near: [f64; 3],
        _plane: &ProfilePlane,
    ) -> KernelResult<Vec<ProjectedEdge>> {
        Err(KernelError::Unsupported("face section".into()))
    }

    /// Where `plane` crosses `brep`, chained end to end, each curve exact
    /// in the plane's own 2D coordinates and drawn in the shape's own
    /// frame: the whole shape's section, or with `near` only the face
    /// nearest it. Empty when the plane misses.
    fn section(
        &self,
        _brep: &[u8],
        _plane: &ProfilePlane,
        _near: Option<[f64; 3]>,
    ) -> KernelResult<Vec<SectionRun>> {
        Err(KernelError::Unsupported("section".into()))
    }

    /// The boundary loops of the face of `brep` nearest `near`, the outer
    /// one first, each edge in loop order, in the shape's own frame.
    fn face_loops(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<Vec<LoopEdge>>> {
        Err(KernelError::Unsupported("face loops".into()))
    }

    /// The curves of a DXF drawing, given as its text.
    fn read_dxf(&self, _text: &str) -> KernelResult<Drawing2d> {
        Err(KernelError::Unsupported("reading DXF".into()))
    }

    /// The regions `profile` encloses, meshed where they stand in its
    /// plane: what a feature swept from it takes, holes left open.
    fn profile_mesh(&self, _profile: &Profile) -> KernelResult<TriMesh> {
        Err(KernelError::Unsupported("profile mesh".into()))
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

    /// What the shape in `brep` answers to `probe`, in the shape's own
    /// frame.
    fn probe(&self, _brep: &[u8], _probe: &ShapeProbe) -> KernelResult<ProbeAnswer> {
        Err(KernelError::Unsupported("probing a shape".into()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe_json(frame: &str) -> String {
        let plane = r#"{"origin":[0,0,0],"x_axis":[1,0,0],"y_axis":[0,1,0],"normal":[0,0,1]}"#;
        let profile = format!(
            r#"{{"plane":{plane},"wires":[{{"segments":[{{"Circle":{{"center":[0,0],"radius":1}}}}]}}]}}"#
        );
        format!(r#"{{"Pipe":{{"profile":{profile},"spine":{profile},{frame}"op":"NewSolid"}}}}"#)
    }

    fn frame_of(json: &str) -> PipeFrame {
        match serde_json::from_str::<SolidOp>(json).unwrap() {
            SolidOp::Pipe { frame, .. } => frame,
            other => panic!("not a pipe: {other:?}"),
        }
    }

    #[test]
    fn a_pipe_written_with_the_frenet_flag_reads_as_its_frame() {
        assert_eq!(frame_of(&pipe_json(r#""frenet":true,"#)), PipeFrame::Frenet);
        assert_eq!(
            frame_of(&pipe_json(r#""frenet":false,"#)),
            PipeFrame::RotationMinimizing
        );
        assert_eq!(frame_of(&pipe_json("")), PipeFrame::RotationMinimizing);
    }

    #[test]
    fn a_pipe_frame_round_trips() {
        let frame = PipeFrame::Binormal {
            direction: [0.0, 0.0, 1.0],
        };
        let text = serde_json::to_string(&frame).unwrap();
        assert_eq!(frame_of(&pipe_json(&format!(r#""frame":{text},"#))), frame);
    }
}
