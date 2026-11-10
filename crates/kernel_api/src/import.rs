//! What an import reads: bodies, their health and repair, physical
//! properties, the assembly tree and annotations.

use serde::{Deserialize, Serialize};

use crate::*;

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
    /// The inertia of the volume at unit density, when there is one.
    #[serde(default)]
    pub inertia: Option<Inertia>,
}

/// A solid's inertia at unit density, in mm⁵ (times a density in mass per
/// mm³ gives mass times mm²), in the solid's frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Inertia {
    /// The inertia tensor about the centre of mass, row by row.
    pub about_centre: [[f64; 3]; 3],
    /// The principal moments, smallest first, each with its axis through
    /// the centre of mass (unit length; right-handed in that order).
    pub principal: [(f64, [f64; 3]); 3],
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
