mod apply;
mod archive;
pub mod arrays;
pub mod asset;
pub mod attach;
mod body;
pub mod command;
pub mod components;
pub mod configurations;
mod copies;
pub mod datum;
pub mod derived;
pub mod describe;
pub mod eigen;
pub mod evaluate;
pub mod expr;
pub mod feature;
pub mod grid;
pub mod handle_arrow;
pub mod history;
mod imported;
pub mod inertia;
pub mod jobs;
mod mesh_entry;
pub mod mesh_query;
pub mod notes;
pub mod op;
pub mod palette;
pub mod patch;
pub mod placement;
pub mod rebuild;
pub mod registration;
pub mod renamed;
pub mod runtime;
pub mod sections;
pub mod server;
pub mod service;
mod shapes;
pub mod shortcut;
pub mod stamp;
pub mod steps;
#[cfg(feature = "test-support")]
pub mod testing;
pub mod texture;
pub mod threads;
pub mod units;
pub mod variables;
pub mod walk;
pub mod workbench;

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use arrays::{
    ArrayError, ArrayId, ArrayInstance, ArrayMember, ArrayPattern, CopyArray, PathSource,
    PathStation, path_stations,
};
pub use asset::{AssetReference, AssetType};
pub use body::{
    Body, BodyDisplay, FaceColor, FaceKey, FaceTexture, Material, MirrorPlane, PrintRest, ScanInfo,
    Shading, StandIn, mesh_with_face_colors,
};
pub use command::{
    AgentAccess, Args, BenchFn, BenchFnById, CommandArgs, CommandError, CommandFn, CommandFnById,
    CommandProse, CommandResult, CommandSpec, CommandTable, Example, ParamKind, ParamSpec,
    Recorded,
};
pub use components::{Component, ComponentId};
pub use configurations::{CONFIGURATIONS_KIND, Configuration, Configurations};
pub use copies::FileLink;
pub use datum::{
    AnchorCircle, AttachmentOffset, BasePlane, DATUM_KIND, DatumAttachment, DatumFeature,
    DatumFrame, DatumShape, EdgeAnchor, EdgeSpot, FaceAnchor, FrameAnchor, LineAnchor, PlacedDatum,
    PlaneAnchor, PointAnchor, body_plane_in, datums_of_body,
};
pub use derived::{BuildStatus, Derived, DerivedMut};
pub use evaluate::{Evaluation, Parameter, SlotValue};
pub use feature::{
    BodyId, DataRevision, FeatureError, FeatureId, FeatureNode, FeatureOrigin, FeatureTree,
    WorkbenchFeature, data_revision, node_revision,
};
pub use grid::PlaneGrid;
pub use handle_arrow::handle_arrow;
pub use history::JournalEntryId;
use imported::MeshesApart;
pub use imported::{Annotation, ImportedGeometry, ImportedObjectNode, LocalGeometry};
pub use kernel_api::{PmiDatum, PmiDimension, PmiTolerance, SemanticPmi, TriMesh};
pub use notes::{
    EdgeKey, Note, NoteAuthor, NoteId, NotePlaced, NotePlane, NoteWant, SketchAnchor,
    SketchNotePick, SketchNoteSpot, sketch_label_for,
};
pub use palette::SketchPalette;
pub use placement::BodyPlacement;
pub use rebuild::{
    BuildError, BuildPlan, MeshPlan, PlanProbe, PlanProjection, ProbedReferences,
    ProjectedReferences, RebuildJob,
};
pub use runtime::{
    CameraOrientRequest, EdgeCircle, EdgeRef, FaceOrigin, FaceRef, HookOutcome, HostRequest,
    InputResult, KeyCode, LogEntry, LogLevel, MarkedSurface, MouseButton, SidePaneView,
    SketchAttachRequest, ToolHistoryReply, VertexRef, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
pub use sections::{SavedSection, SavedWedge, SectionAim};
pub use service::DocumentService;
pub use shortcut::{ActionDescriptor, Chord};
pub use stamp::{CONTAINER_FORMAT, FileAge, FileStamp, file_age, set_writer_version};
pub use threads::ThreadCallout;
pub use units::{Unit, format_area_mm2, format_length_mm, format_volume_mm3};
pub use variables::{Numbering, VARIABLES_KIND, Variable, VariableChoice, VariableSet};
pub use workbench::{
    BenchBuild, BenchCommands, BenchFeatures, BenchOutput, BenchPanels, BenchParameters,
    BenchReport, BenchTools, FeatureInfo, FeaturePart, FeaturePaths, FeatureReference,
    FeatureState, FeatureStatus, FileImport, INSPECT_CATEGORY, MapLegend, MarkKind, MenuItem,
    MenuScope, OverlayMesh, PartPick, PassiveGeometry, PassiveRegion, PassiveTint, PrintPart,
    PropertyHints, RADIAL_SLOTS, ReferenceChoice, ReportTable, ScalarMap, ScreenSpaceImage,
    ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, ScreenSpacePolygon, StatusItems,
    StatusTone, TabSlot, TaskInfo, TaskOutcome, TaskRequest, TaskStatus, ToolBehavior,
    ToolDescriptor, ToolHint, ToolVariant, ViewportHud, ViewportOutput, ViewportPick, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchId, base_tool_id, fresh_revision,
    mesh_revision, numbered_pins, tool_variant,
};

/// Result type for document operations.
pub type DocumentResult<T> = std::result::Result<T, DocumentError>;

/// Type-erased storage for workbench-specific data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchStorage {
    /// Workbench ID this storage belongs to.
    pub workbench_id: WorkbenchId,
    /// Arbitrary JSON data (workbench-specific).
    pub data: serde_json::Value,
}

/// Primary data structure persisted by the application.
///
/// The document is saved as a `.prtcad` file, which is a tar archive
/// (optionally gzip- or zstd-compressed) containing:
/// - `thumbnail.png` - A preview of the model, first when present
/// - `meta.json` - What wrote the file ([`FileStamp`]), before the document
/// - `document.json` - This document structure (serialized)
/// - `assets/` - External files (STEP, STL, etc.) referenced by the document
/// - `brep/` - Per-body shape snapshots (ogeom native text) and face-color sidecars
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    metadata: DocumentMetadata,
    feature_tree: FeatureTree,
    bodies: Vec<Body>,
    /// Bodies grouped into components, nested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    components: Vec<Component>,
    /// Bodies repeated as linked copies, each array owning its copies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    copy_arrays: Vec<arrays::CopyArray>,
    /// Unused, and still written so builds that require it read the file.
    #[serde(default)]
    workbench_storage: HashMap<String, WorkbenchStorage>,
    /// References to external files stored in the .prtcad archive.
    assets: HashMap<Uuid, AssetReference>,
    /// What the builds found (`derived.rs`): derived on each replica, never
    /// an op. Its bodies' meshes are written under `imported_meshes`, so a
    /// reload needs no re-tessellation; the rest of it is not saved.
    #[serde(rename = "imported_meshes", default, with = "derived::saved_meshes")]
    derived: Derived,
    /// Imported STEP hierarchy (assemblies/parts/instances).
    #[serde(default)]
    imported_objects: HashMap<Uuid, ImportedObjectNode>,
    /// Ordered roots for the imported hierarchy tree.
    #[serde(default)]
    imported_object_roots: Vec<Uuid>,
    /// Per-document display unit. All numeric storage stays in millimetres;
    /// this only controls how lengths are surfaced to the user.
    #[serde(default)]
    display_unit: Unit,
    /// The unit the measure tool reads in, when the user chose one over
    /// the display unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measure_unit: Option<Unit>,
    /// What an AI agent working on this document is told to follow, beside
    /// the rules the user keeps for every document. Travels with the file.
    #[serde(default)]
    agent_rules: String,
    /// Named clipping planes the view switches between (`sections.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    saved_sections: Vec<sections::SavedSection>,
    /// The highest number a note of this document was given: a new note
    /// takes the next, so a note taken away leaves its number unused.
    #[serde(default, skip_serializing_if = "is_zero")]
    notes_numbered: u32,
    /// Unused, and still written so builds that require it read the file.
    #[serde(default)]
    history: Vec<DocumentRevision>,
    /// Raw asset bytes (STEP/STL files, etc.) kept in memory between import
    /// and save. Populated either on import or after `load_from_file`. Skipped
    /// from JSON because the bytes live as separate entries in the tar archive.
    #[serde(skip)]
    asset_blobs: HashMap<Uuid, std::sync::Arc<Vec<u8>>>,
    /// Bodies' base solids: the shape a body's feature history starts
    /// from, kept from what an import, a mesh conversion or a repair made
    /// of the body, apart from what its features build and draw. Set and
    /// cleared by the `SetBodyBase` op.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    base_solids: HashMap<BodyId, ImportedGeometry>,
    /// Mesh bodies' starts: the mesh as imported, which a mesh history
    /// builds from, apart from the edited mesh the body draws. Set and
    /// cleared by the `SetMeshStart` op; saved as `mesh/<body>.start.bin`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    mesh_starts: HashMap<BodyId, ImportedGeometry>,
    /// The base solids' snapshots, saved as `brep/<body>.base.bin`.
    #[serde(skip)]
    base_brep_blobs: HashMap<BodyId, std::sync::Arc<Vec<u8>>>,
    /// The base solids' face colours, saved as `brep/<body>.base.colors`.
    #[serde(skip)]
    base_face_colors: HashMap<BodyId, Vec<[f32; 3]>>,
    /// A PNG preview of the model, written as the container's first entry
    /// (`thumbnail.png`) so a file browser reads it without unpacking the
    /// rest. Derived state: set by the saving client, never an op.
    #[serde(skip)]
    thumbnail: Option<std::sync::Arc<Vec<u8>>>,
    /// What wrote the container this document was read from; `None` for a
    /// container from before stamps, and for a document never read from
    /// one. Derived state: a save writes [`FileStamp::current`].
    #[serde(skip)]
    written_by: Option<FileStamp>,
    /// Reverse index for fast body->imported-object visibility checks.
    #[serde(skip)]
    imported_body_to_object: HashMap<BodyId, Uuid>,
    /// Monotonic edit counter bumped by [`Self::mark_dirty`]. Cheap change
    /// detection for the undo system: equal values mean "no edits since".
    /// Not persisted; only compared for equality within one process.
    #[serde(skip)]
    mutation_seq: u64,
    /// Captured-but-undrained user-edit ops (the outbox). Skipped by serde
    /// and cleared by `Clone`: snapshots carry state, never the outbox.
    #[serde(skip)]
    pending_ops: op::OpBuffer,
    /// (op, inverse) pairs since the last journal boundary: the raw
    /// material of per-user undo. Same clone-empty rule as the outbox.
    #[serde(skip)]
    journal_pending: op::JournalBuffer,
    /// True while history traversal or remote application drives the
    /// document: those must not journal themselves.
    #[serde(skip)]
    history_suppressed: bool,
    /// What every formula comes to (`evaluate`), and at which edit it was
    /// worked out. Derived on each replica, never an op.
    #[serde(skip)]
    evaluated: Evaluated,
    /// Which document value this is: every new, loaded or cloned one is
    /// another (`content_stamp`).
    #[serde(skip)]
    lineage: Lineage,
    /// True while a [`DerivedMut`] handle is out: recording an op then is
    /// a bug, which debug builds assert.
    #[serde(skip)]
    deriving: bool,
    /// The bodies the configuration in effect leaves out, kept as the
    /// table changes.
    #[serde(skip)]
    left_out: Vec<BodyId>,
    /// Linked parts whose file changed since it was read.
    #[serde(skip)]
    link_stale: std::collections::HashSet<BodyId>,
    /// Linked parts whose file is found neither beside the document nor
    /// where it was linked from.
    #[serde(skip)]
    link_missing: std::collections::HashSet<BodyId>,
    /// The bodies the view is isolated to, when it is: the rest draw and
    /// pick as if hidden while each body's own visibility stays as saved.
    /// View state of this replica, never an op and never saved.
    #[serde(skip)]
    isolated: Option<Vec<BodyId>>,
}

/// Why a feature cannot move a step in its body's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRefused {
    /// No such feature.
    NotFound,
    /// It is already first, or last.
    AtEnd,
    /// The step would put it past `neighbour`, which one of the two is
    /// built from.
    Dependency { neighbour: FeatureId },
}

/// A number no other document value in the process has: a clone takes a
/// new one, since the two go on to be edited apart.
#[derive(Debug)]
struct Lineage(u64);

impl Default for Lineage {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

impl Clone for Lineage {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// The document's formulas, worked out.
#[derive(Debug, Clone, Default)]
struct Evaluated {
    evaluation: evaluate::Evaluation,
    /// The `mutation_seq` it was worked out at.
    at: Option<u64>,
    /// How many times the formulas were worked out.
    runs: u64,
    /// Features whose values moved since the host last took them.
    moved: Vec<FeatureId>,
}

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: DocumentMetadata::new(name),
            feature_tree: FeatureTree::new(),
            bodies: Vec::new(),
            components: Vec::new(),
            copy_arrays: Vec::new(),
            workbench_storage: HashMap::new(),
            assets: HashMap::new(),
            imported_objects: HashMap::new(),
            imported_object_roots: Vec::new(),
            display_unit: Unit::default(),
            measure_unit: None,
            agent_rules: String::new(),
            saved_sections: Vec::new(),
            notes_numbered: 0,
            history: Vec::new(),
            asset_blobs: HashMap::new(),
            base_solids: HashMap::new(),
            mesh_starts: HashMap::new(),
            base_brep_blobs: HashMap::new(),
            base_face_colors: HashMap::new(),
            thumbnail: None,
            written_by: None,
            imported_body_to_object: HashMap::new(),
            mutation_seq: 0,
            pending_ops: op::OpBuffer::default(),
            journal_pending: op::JournalBuffer::default(),
            history_suppressed: false,
            evaluated: Evaluated::default(),
            lineage: Lineage::default(),
            derived: Derived::default(),
            deriving: false,
            left_out: Vec::new(),
            link_stale: Default::default(),
            link_missing: Default::default(),
            isolated: None,
        }
    }

    /// The PNG preview saved with the document, if it has one.
    pub fn thumbnail(&self) -> Option<&[u8]> {
        self.thumbnail.as_deref().map(Vec::as_slice)
    }

    /// Set the preview the next save writes. Derived state: no op, and the
    /// document keeps its clean or dirty state.
    pub fn set_thumbnail(&mut self, png: Option<Vec<u8>>) {
        self.thumbnail = png.map(std::sync::Arc::new);
    }

    /// What wrote the container this document was read from: `None` for
    /// one from before files were stamped, or a document not read from a
    /// container.
    pub fn written_by(&self) -> Option<&FileStamp> {
        self.written_by.as_ref()
    }

    /// Currently selected display unit for this document (mm by default).
    pub fn display_unit(&self) -> Unit {
        self.display_unit
    }

    /// Override the display unit. Marks the document dirty so the choice is
    /// persisted on the next save.
    pub fn set_display_unit(&mut self, unit: Unit) {
        if self.display_unit != unit {
            self.record_and_apply(op::DocumentOp::SetDisplayUnit { unit });
        }
    }

    /// The unit the measure tool reads in, when one was chosen over the
    /// display unit.
    pub fn measure_unit(&self) -> Option<Unit> {
        self.measure_unit
    }

    /// Choose the unit the measure tool reads in; `None` reads in the
    /// display unit.
    pub fn set_measure_unit(&mut self, unit: Option<Unit>) {
        if self.measure_unit != unit {
            self.record_and_apply(op::DocumentOp::SetMeasureUnit { unit });
        }
    }

    /// What an AI agent working on this document is told to follow.
    pub fn agent_rules(&self) -> &str {
        &self.agent_rules
    }

    /// Set the rules an AI agent working on this document follows.
    pub fn set_agent_rules(&mut self, rules: impl Into<String>) {
        let rules = rules.into();
        if self.agent_rules != rules {
            self.record_and_apply(op::DocumentOp::SetAgentRules { rules });
        }
    }

    pub fn id(&self) -> Uuid {
        self.metadata.id
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        if self.metadata.name != name {
            self.record_and_apply(op::DocumentOp::SetDocumentName { name });
        }
    }

    /// Name the document after the file it is read from or written to.
    /// The name is derived from the file's, not an edit: every copy that
    /// opens the file derives the same, so it records no op.
    pub fn name_after_file(&mut self, name: impl Into<String>) {
        self.metadata.name = name.into();
    }

    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }

    pub fn mark_dirty(&mut self) {
        self.metadata.dirty = true;
        self.mutation_seq = self.mutation_seq.wrapping_add(1);
    }

    /// See the `mutation_seq` field: bumped on every `mark_dirty`.
    pub fn mutation_seq(&self) -> u64 {
        self.mutation_seq.wrapping_add(self.derived.moved)
    }

    pub fn mark_clean(&mut self) {
        self.metadata.dirty = false;
    }

    /// Add a feature to the tree without attaching it to a body.
    /// For body-scoped features, prefer `add_feature_in_body`.
    pub fn add_feature<F: WorkbenchFeature>(
        &mut self,
        feature: F,
        name: String,
    ) -> DocumentResult<FeatureId> {
        self.add_feature_in_body(feature, name, None)
    }

    /// Add a feature to the tree and optionally attach it to a body for hierarchy purposes.
    pub fn add_feature_in_body<F: WorkbenchFeature>(
        &mut self,
        feature: F,
        name: String,
        body: Option<BodyId>,
    ) -> DocumentResult<FeatureId> {
        // Everything is resolved here (id, timestamp, seq), so the op is a
        // pure effect and replays identically on a peer.
        let id = FeatureId::new();
        self.record_and_apply(op::DocumentOp::AddFeature {
            id,
            workbench_id: F::workbench_id(),
            name,
            body,
            deps: feature.dependencies(),
            data: feature.to_json(),
            seq: self.feature_tree.next_seq(),
            created_at: epoch_ms_now(),
            formulas: Default::default(),
            made_by: None,
            package_source: None,
            dependents: Vec::new(),
            hidden: false,
            suppressed: false,
        });
        self.insert_at_tip(id);
        Ok(id)
    }

    /// A feature just added to a body whose tip stands before the end of
    /// its history goes in right after the tip, and the tip moves onto it:
    /// new work lands at the point in history on show, and what came after
    /// the tip builds on it once the tip moves on. A feature that reads
    /// something past the tip stops just after that, and the tip moves onto
    /// it all the same.
    fn insert_at_tip(&mut self, id: FeatureId) {
        let Some(body) = self.feature_tree.get_node(id).and_then(|n| n.body) else {
            return;
        };
        let Some(tip) = self
            .body(body)
            .and_then(|b| b.tip)
            .filter(|tip| self.feature_tree.get_node(*tip).is_some())
        else {
            return;
        };
        loop {
            let mut history: Vec<(u64, FeatureId)> = self
                .feature_tree
                .all_nodes()
                .filter(|(_, n)| n.body == Some(body))
                .map(|(fid, n)| (n.seq, *fid))
                .collect();
            history.sort();
            let Some(at) = history.iter().position(|(_, fid)| *fid == id) else {
                return;
            };
            if at == 0 || history[at - 1].1 == tip {
                break;
            }
            if self.try_move_feature_in_history(id, true).is_err() {
                break;
            }
        }
        self.set_body_tip(body, Some(id));
    }

    /// Add a feature of kind `kind` whose data a workbench package owns,
    /// recording the package and version that made it and where that
    /// package is published.
    pub fn add_feature_of_kind(
        &mut self,
        kind: WorkbenchId,
        name: String,
        body: Option<BodyId>,
        deps: Vec<FeatureId>,
        data: serde_json::Value,
        origin: FeatureOrigin,
    ) -> FeatureId {
        let FeatureOrigin {
            made_by,
            package_source,
        } = origin;
        let id = FeatureId::new();
        self.record_and_apply(op::DocumentOp::AddFeature {
            id,
            workbench_id: kind,
            name,
            body,
            deps,
            data,
            seq: self.feature_tree.next_seq(),
            created_at: epoch_ms_now(),
            formulas: Default::default(),
            made_by,
            package_source,
            dependents: Vec::new(),
            hidden: false,
            suppressed: false,
        });
        self.insert_at_tip(id);
        id
    }

    /// Record that the package `origin` names last wrote the feature, when
    /// that is news.
    pub fn set_feature_origin(
        &mut self,
        id: FeatureId,
        origin: FeatureOrigin,
    ) -> DocumentResult<()> {
        let FeatureOrigin {
            made_by,
            package_source,
        } = origin;
        let node = self
            .feature_tree
            .get_node(id)
            .ok_or(DocumentError::FeatureNotFound(id))?;
        if node.made_by != made_by || node.package_source != package_source {
            self.record_and_apply(op::DocumentOp::SetFeatureOrigin {
                id,
                made_by,
                package_source,
            });
        }
        Ok(())
    }

    /// Get feature data (returns JSON, workbench must deserialize).
    pub fn get_feature_data(&self, id: FeatureId) -> Option<&serde_json::Value> {
        self.feature_tree.get_node(id).map(|n| &*n.data)
    }

    /// The feature's data as it builds: what the user set, with every
    /// formula's current value in. Benches build from this.
    pub fn feature_values(&self, id: FeatureId) -> Option<&serde_json::Value> {
        self.evaluated
            .evaluation
            .data
            .get(&id)
            .or_else(|| self.get_feature_data(id))
    }

    /// Feature `id` read as `F` from [`Self::feature_values`] (its
    /// formulas worked out), when it is of `F`'s kind and reads as one.
    pub fn feature_as<F: WorkbenchFeature>(&self, id: FeatureId) -> Option<F> {
        let node = self.feature_tree.get_node(id)?;
        if node.workbench_id != F::workbench_id() {
            return None;
        }
        F::from_json(self.feature_values(id)?).ok()
    }

    /// [`Self::feature_as`], decoded once per write of the feature's data
    /// when no formula sets its values ([`FeatureNode::read`]): for what
    /// is read every frame.
    pub fn read_feature<F: WorkbenchFeature + Clone + 'static>(&self, id: FeatureId) -> Option<F> {
        let node = self.feature_tree.get_node(id)?;
        if node.workbench_id != F::workbench_id() {
            return None;
        }
        match self.evaluated.evaluation.data.get(&id) {
            Some(values) => F::from_json(values).ok(),
            None => node.read(),
        }
    }

    /// A revision of [`Self::feature_values`]: equal while they are the
    /// same, without reading them through when no formula sets them.
    pub fn values_revision(&self, id: FeatureId) -> Option<u64> {
        match self.evaluated.evaluation.data.get(&id) {
            Some(values) => Some(feature::data_revision(values)),
            None => self
                .feature_tree
                .get_node(id)
                .map(|n| n.data_revision.get()),
        }
    }

    /// Every feature of `F`'s kind that reads as one, from its
    /// [`Self::feature_values`], in build order.
    pub fn features_of<F: WorkbenchFeature>(&self) -> Vec<(FeatureId, F)> {
        let kind = F::workbench_id();
        let mut found: Vec<(u64, FeatureId, F)> = self
            .feature_tree
            .all_nodes()
            .filter(|(_, node)| node.workbench_id == kind)
            .filter_map(|(id, node)| {
                let feature = F::from_json(self.feature_values(*id)?).ok()?;
                Some((node.seq, *id, feature))
            })
            .collect();
        found.sort_by_key(|(seq, id, _)| (*seq, *id));
        found.into_iter().map(|(_, id, f)| (id, f)).collect()
    }

    /// What the feature's formulas and variables come to, in order.
    pub fn evaluated_slots(&self, id: FeatureId) -> &[evaluate::SlotValue] {
        self.evaluated
            .evaluation
            .slots
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The first of the feature's own formulas that fails to work out, as
    /// an error: what keeps it from building, since its data still holds
    /// the number set before the formula.
    pub fn formula_error(&self, id: FeatureId) -> Option<String> {
        self.evaluated_slots(id)
            .iter()
            .find_map(|slot| match (&slot.formula, &slot.result) {
                (Some(formula), Err(why)) => Some(format!(
                    "the formula of {} (`{formula}`) fails: {why}",
                    slot.label
                )),
                _ => None,
            })
    }

    /// Why the feature cannot build for a formula: one of its own fails,
    /// or one of a feature it reads, however far back.
    pub fn build_formula_error(&self, id: FeatureId) -> Option<String> {
        if let Some(why) = self.formula_error(id) {
            return Some(why);
        }
        let mut seen = std::collections::HashSet::from([id]);
        let mut next = self.feature_tree.dependencies(id);
        while let Some(dep) = next.pop() {
            if !seen.insert(dep) {
                continue;
            }
            if let Some(why) = self.formula_error(dep) {
                let name = self
                    .feature_tree
                    .get_node(dep)
                    .map_or_else(String::new, |n| n.name.clone());
                return Some(format!("{name}: {why}"));
            }
            next.extend(self.feature_tree.dependencies(dep));
        }
        None
    }

    /// What the feature's data settled to last time its values were
    /// `unsettled`, taken out of the last evaluation for the next one to
    /// keep (`Evaluation::reused`, which `apply_evaluation` counts as
    /// unchanged).
    pub(crate) fn take_settled_values(
        &mut self,
        id: FeatureId,
        unsettled: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let last = &mut self.evaluated.evaluation;
        if last.unsettled.get(&id) != Some(unsettled) {
            return None;
        }
        last.data.remove(&id)
    }

    /// Equal while nothing a feature reads can have changed: the same
    /// document value, no edit since and the formulas not worked out again.
    /// A cache of what features say keyed on it never goes stale.
    pub fn content_stamp(&self) -> (u64, u64, u64) {
        (self.lineage.0, self.mutation_seq(), self.evaluated.runs)
    }

    /// Whether the document changed since its formulas were worked out.
    pub fn needs_evaluation(&self) -> bool {
        self.evaluated.at != Some(self.mutation_seq())
    }

    /// Take `evaluation` as what the formulas come to now. A feature whose
    /// values changed is marked for rebuilding, and what depends on it;
    /// the document is not marked edited, as nothing the user set moved.
    /// Answers the features marked.
    pub fn apply_evaluation(&mut self, evaluation: evaluate::Evaluation) -> Vec<FeatureId> {
        let mut changed: Vec<FeatureId> = Vec::new();
        let old = &self.evaluated.evaluation.data;
        for id in old.keys().chain(evaluation.data.keys()) {
            if evaluation.reused.contains(id) {
                continue;
            }
            let raw = self.feature_tree.get_node(*id).map(|n| &*n.data);
            let before = old.get(id).or(raw);
            let after = evaluation.data.get(id).or(raw);
            if before != after && !changed.contains(id) {
                changed.push(*id);
            }
        }
        // A formula that starts or stops failing changes what the feature
        // builds as much as a value does.
        let failing = |slots: &HashMap<FeatureId, Vec<evaluate::SlotValue>>| {
            slots
                .iter()
                .filter(|(_, s)| s.iter().any(|s| s.formula.is_some() && s.result.is_err()))
                .map(|(id, _)| *id)
                .collect::<std::collections::HashSet<_>>()
        };
        let (was, is) = (
            failing(&self.evaluated.evaluation.slots),
            failing(&evaluation.slots),
        );
        for id in &changed {
            self.feature_tree.mark_dirty(*id);
        }
        for id in was.symmetric_difference(&is) {
            if self.feature_tree.get_node(*id).is_some() {
                self.feature_tree.mark_dirty(*id);
            }
        }
        let mut moved = std::mem::take(&mut self.evaluated.moved);
        for id in &changed {
            if !moved.contains(id) {
                moved.push(*id);
            }
        }
        self.evaluated = Evaluated {
            evaluation,
            at: Some(self.mutation_seq()),
            runs: self.evaluated.runs.wrapping_add(1),
            moved,
        };
        changed
    }

    /// A value of `id` was set by hand: what follows from it (a joint's
    /// placements) follows as if a formula had moved it.
    pub fn note_value_moved(&mut self, id: FeatureId) {
        if !self.evaluated.moved.contains(&id) {
            self.evaluated.moved.push(id);
        }
    }

    /// Whether feature `id` is in its body's build: not suppressed, and
    /// not past the body's tip.
    pub fn feature_in_build(&self, id: FeatureId) -> bool {
        let Some(node) = self.feature_tree.get_node(id) else {
            return false;
        };
        if node.suppressed {
            return false;
        }
        let tip = node
            .body
            .and_then(|body| self.body(body))
            .and_then(|b| b.tip)
            .and_then(|tip| self.feature_tree.get_node(tip));
        tip.is_none_or(|tip| node.seq <= tip.seq)
    }

    /// The features whose values formulas moved, or whose formulas were
    /// set or taken away, since this was last asked, for
    /// `BenchParameters::values_moved`.
    pub fn take_moved_values(&mut self) -> Vec<FeatureId> {
        std::mem::take(&mut self.evaluated.moved)
    }

    /// Get feature metadata (id, name, dirty, etc.).
    pub fn get_feature_meta(&self, id: FeatureId) -> Option<&FeatureNode> {
        self.feature_tree.get_node(id)
    }

    /// Update feature data (workbench provides serialized JSON).
    pub fn update_feature_data(
        &mut self,
        id: FeatureId,
        data: impl Into<std::sync::Arc<serde_json::Value>>,
    ) -> DocumentResult<()> {
        if self.feature_tree.get_node(id).is_none() {
            return Err(DocumentError::FeatureNotFound(id));
        }
        self.record_and_apply(op::DocumentOp::UpdateFeatureData {
            id,
            data: data.into(),
        });
        Ok(())
    }

    /// Write `patch` into feature `id`'s data in place: what an edit that
    /// changed a few parts of a large payload records instead of the whole
    /// payload ([`op::DocumentOp::PatchFeatureData`]). A patch that does
    /// not fit the data (a path it names is missing) is refused, leaving the
    /// data untouched; an empty one records nothing.
    pub fn patch_feature_data(
        &mut self,
        id: FeatureId,
        patch: patch::DataPatch,
    ) -> DocumentResult<()> {
        if self.feature_tree.get_node(id).is_none() {
            return Err(DocumentError::FeatureNotFound(id));
        }
        if patch.is_empty() {
            return Ok(());
        }
        debug_assert!(!self.deriving, "derived state recorded an op");
        let operation = op::DocumentOp::PatchFeatureData { id, patch };
        let inverse = self
            .invert_and_apply(&operation)
            .map_err(|e| DocumentError::Refused(format!("the feature's data: {e}")))?;
        self.record_applied(operation, inverse);
        Ok(())
    }

    /// Mark feature dirty (triggers recomputation).
    pub fn mark_feature_dirty(&mut self, feature_id: FeatureId) {
        self.feature_tree.mark_dirty(feature_id);
        self.mark_dirty();
    }

    /// Clear a feature's dirty flag (host calls this once its recompute has
    /// been scheduled or applied).
    pub fn clear_feature_dirty(&mut self, feature_id: FeatureId) {
        if let Some(node) = self.feature_tree.get_node_mut(feature_id) {
            node.dirty = false;
        }
    }

    /// Show/hide a feature (e.g. hide a sketch once a pad consumes it).
    pub fn set_feature_visible(&mut self, feature_id: FeatureId, visible: bool) {
        if let Some(node) = self.feature_tree.get_node(feature_id)
            && node.visible != visible
        {
            self.record_and_apply(op::DocumentOp::SetFeatureVisible {
                id: feature_id,
                visible,
            });
        }
    }

    /// Rewire a feature's dependencies (marks it dirty for recompute).
    pub fn set_feature_dependencies(&mut self, feature_id: FeatureId, deps: Vec<FeatureId>) {
        self.record_and_apply(op::DocumentOp::SetFeatureDependencies {
            id: feature_id,
            deps,
        });
    }

    /// Set the formula behind the feature's number `key` (its bench's key,
    /// `BenchParameters::parameters`), or take it away so the number stands as
    /// it is: at the value the formula last gave it, when the formulas are
    /// worked out for the document as it stands.
    pub fn set_feature_formula(
        &mut self,
        feature_id: FeatureId,
        key: impl Into<String>,
        formula: Option<String>,
    ) -> DocumentResult<()> {
        let key = key.into();
        let node = self
            .feature_tree
            .get_node(feature_id)
            .ok_or(DocumentError::FeatureNotFound(feature_id))?;
        if node.formulas.get(&key) != formula.as_ref() {
            // The plain numbers take what the formulas gave them, this
            // one's among them, before it goes.
            let kept = (formula.is_none() && !self.needs_evaluation())
                .then(|| self.evaluated.evaluation.unsettled.get(&feature_id))
                .flatten()
                .filter(|values| **values != *node.data)
                .cloned();
            if let Some(data) = kept {
                self.record_and_apply(op::DocumentOp::UpdateFeatureData {
                    id: feature_id,
                    data: data.into(),
                });
            }
            self.record_and_apply(op::DocumentOp::SetFeatureFormula {
                id: feature_id,
                key,
                formula,
            });
            // What a bench keeps in step with the feature's formulas
            // follows, whether or not the value moves.
            self.note_value_moved(feature_id);
        }
        Ok(())
    }

    /// The formula behind the feature's number `key`, if one sets it.
    pub fn feature_formula(&self, feature_id: FeatureId, key: &str) -> Option<&str> {
        self.feature_tree
            .get_node(feature_id)?
            .formulas
            .get(key)
            .map(String::as_str)
    }

    /// Rename a feature (user-facing name in the tree and panels). A name
    /// that would leave formulas unable to tell two objects apart is
    /// refused ([`Self::name_refusal`]); an empty name, or the one it has,
    /// changes nothing.
    pub fn rename_feature(
        &mut self,
        feature_id: FeatureId,
        name: impl Into<String>,
    ) -> DocumentResult<()> {
        let name = name.into();
        let node = self
            .feature_tree
            .get_node(feature_id)
            .ok_or(DocumentError::FeatureNotFound(feature_id))?;
        if node.name == name || name.trim().is_empty() {
            return Ok(());
        }
        if let Some(why) = self.name_refusal(Some(feature_id), &name) {
            return Err(DocumentError::Refused(why));
        }
        let old = node.name.clone();
        self.record_and_apply(op::DocumentOp::RenameFeature {
            id: feature_id,
            name: name.clone(),
        });
        // Formulas that named it name it still.
        self.rewrite_formulas(|text| expr::rename_object(text, &old, &name));
        Ok(())
    }

    pub fn rename_body(&mut self, body: BodyId, name: impl Into<String>) {
        let name = name.into();
        if let Some(entry) = self.body(body)
            && entry.name != name
            && !name.trim().is_empty()
        {
            self.record_and_apply(op::DocumentOp::RenameBody { id: body, name });
        }
    }

    /// The feature a body's solid is built up to: its tip, else the last
    /// feature of its history.
    pub fn body_tip(&self, body: BodyId) -> Option<FeatureId> {
        let entry = self.body(body)?;
        entry.tip.or_else(|| {
            self.feature_tree
                .all_nodes()
                .filter(|(_, n)| n.body == Some(body))
                .max_by_key(|(id, n)| (n.seq, **id))
                .map(|(id, _)| *id)
        })
    }

    /// The asset `id` as the document keeps it: where it sits in the
    /// archive (its extension names its format) and its bytes.
    pub fn asset_with_bytes(&self, id: Uuid) -> Option<(&AssetReference, std::sync::Arc<Vec<u8>>)> {
        Some((self.assets.get(&id)?, self.asset_blobs.get(&id)?.clone()))
    }

    /// Show or hide a body in the scene.
    pub fn set_body_visible(&mut self, body: BodyId, visible: bool) {
        if let Some(entry) = self.body(body)
            && entry.hidden == visible
        {
            self.record_and_apply(op::DocumentOp::SetBodyVisible { id: body, visible });
        }
    }

    /// Move a body to `placement`.
    pub fn set_body_placement(&mut self, body: BodyId, placement: BodyPlacement) {
        if let Some(entry) = self.body(body)
            && entry.placement != placement
        {
            self.record_and_apply(op::DocumentOp::SetBodyPlacement {
                id: body,
                placement,
            });
        }
    }

    /// Where a body sits; the identity for a body that does not exist.
    pub fn body_placement(&self, body: BodyId) -> BodyPlacement {
        self.body(body).map(|b| b.placement).unwrap_or_default()
    }

    /// After a load: the placed bodies' own meshes, from the placed copies
    /// the file holds.
    fn recover_local_meshes(&mut self) {
        let placed: Vec<(BodyId, BodyPlacement)> = self
            .bodies
            .iter()
            .filter(|b| !b.placement.is_identity())
            .map(|b| (b.id, b.placement))
            .collect();
        for (body, placement) in placed {
            if let Some(geometry) = self.derived.imported_meshes.get(&body) {
                let local = placement.inverse().mesh(&geometry.mesh);
                let bounds = local.bounds();
                self.derived
                    .local_meshes
                    .insert(body, (Arc::new(local), bounds));
            }
        }
    }

    /// Re-derive a body's placed mesh from its own after its placement
    /// changed: the local copy is kept beside it while the body is placed.
    fn place_geometry(&mut self, body: BodyId) {
        let placement = self.body_placement(body);
        let Some((local, local_bounds)) = self.derived().local_geometry(body) else {
            return;
        };
        let Some(geometry) = self.derived.imported_meshes.get_mut(&body) else {
            return;
        };
        if placement.is_identity() {
            geometry.mesh = local;
            geometry.bounds_mm = local_bounds;
            self.derived.local_meshes.remove(&body);
        } else {
            geometry.mesh = Arc::new(placement.mesh(&local));
            geometry.bounds_mm = local_bounds.map(|b| placement.bounds(b));
            self.derived
                .local_meshes
                .insert(body, (local, local_bounds));
        }
        geometry.revision = geometry.revision.saturating_add(1);
    }

    /// Suppress/unsuppress a feature (excluded from builds while suppressed).
    pub fn set_feature_suppressed(&mut self, feature_id: FeatureId, suppressed: bool) {
        if let Some(node) = self.feature_tree.get_node(feature_id)
            && node.suppressed != suppressed
        {
            self.record_and_apply(op::DocumentOp::SetFeatureSuppressed {
                id: feature_id,
                suppressed,
            });
        }
    }

    /// Set (or clear) the feature exposed as a body's shape. Features after
    /// the tip are excluded from the build until the tip moves back.
    pub fn set_body_tip(&mut self, body: BodyId, tip: Option<FeatureId>) {
        if let Some(entry) = self.body(body)
            && entry.tip != tip
        {
            self.record_and_apply(op::DocumentOp::SetBodyTip { id: body, tip });
        }
    }

    /// Move a feature one place in its body's history (earlier when `up`),
    /// swapping it with the feature beside it, whatever kind that is: a
    /// sketch and the feature that uses it keep their places relative to
    /// everything else. Refuses a move that would put a feature before one
    /// it uses (or after one that uses it). Returns whether the order
    /// changed.
    pub fn move_feature_in_history(&mut self, feature_id: FeatureId, up: bool) -> bool {
        self.try_move_feature_in_history(feature_id, up).is_ok()
    }

    /// [`Self::move_feature_in_history`], saying why a move is refused.
    pub fn try_move_feature_in_history(
        &mut self,
        feature_id: FeatureId,
        up: bool,
    ) -> Result<(), MoveRefused> {
        let Some(node) = self.feature_tree.get_node(feature_id) else {
            return Err(MoveRefused::NotFound);
        };
        let (body, seq) = (node.body, node.seq);

        // The body's history, every kind of feature in it, by seq.
        let mut peers: Vec<(u64, FeatureId)> = self
            .feature_tree
            .all_nodes()
            .filter(|(_, n)| n.body == body)
            .map(|(id, n)| (n.seq, *id))
            .collect();
        // Same tie-break as every seq sort: (seq, id) is the total order.
        peers.sort_by_key(|(s, id)| (*s, *id));
        let position = peers.iter().position(|(s, _)| *s == seq).unwrap_or(0);
        let neighbour_pos = if up {
            position.checked_sub(1)
        } else {
            (position + 1 < peers.len()).then_some(position + 1)
        };
        let Some(neighbour_pos) = neighbour_pos else {
            return Err(MoveRefused::AtEnd);
        };
        let (_, neighbour_id) = peers[neighbour_pos];

        // Dependency guard: after the swap every dependency must still come
        // earlier. The swap only reorders these two features, so it suffices
        // to check the pair against each other.
        let deps_of = |id: FeatureId| self.feature_tree.dependencies(id);
        let violates = if up {
            deps_of(feature_id).contains(&neighbour_id)
        } else {
            deps_of(neighbour_id).contains(&feature_id)
        };
        if violates {
            return Err(MoveRefused::Dependency {
                neighbour: neighbour_id,
            });
        }

        // The guard ran above; the op is the resolved swap, pure on replay.
        self.record_and_apply(op::DocumentOp::SwapFeatureSeq {
            a: feature_id,
            b: neighbour_id,
        });
        Ok(())
    }

    /// The features of `feature`'s body, in history order.
    pub fn body_history_of(&self, feature: FeatureId) -> Vec<FeatureId> {
        let Some(body) = self.feature_tree.get_node(feature).map(|n| n.body) else {
            return Vec::new();
        };
        self.feature_tree
            .ordered_nodes(|n| n.body == body)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Move a feature along its body's history, a step at a time as
    /// [`Self::try_move_feature_in_history`] goes, until it sits right
    /// after `after`, or first for `None`. The whole move is checked
    /// first: a refused one changes nothing.
    pub fn move_feature_after(
        &mut self,
        feature: FeatureId,
        after: Option<FeatureId>,
    ) -> Result<(), MoveRefused> {
        let order = self.body_history_of(feature);
        let Some(at) = order.iter().position(|f| *f == feature) else {
            return Err(MoveRefused::NotFound);
        };
        let want = match after {
            None => 0,
            Some(after) => match order.iter().position(|f| *f == after) {
                Some(target) if target < at => target + 1,
                Some(target) => target,
                None => return Err(MoveRefused::NotFound),
            },
        };
        // Every feature it passes, nearest first, must not use it (going
        // later) nor be used by it (going earlier).
        let passed: Vec<FeatureId> = if want < at {
            order[want..at].iter().rev().copied().collect()
        } else {
            order[at + 1..=want].to_vec()
        };
        let uses =
            |user: FeatureId, used: FeatureId| self.feature_tree.dependencies(user).contains(&used);
        if let Some(neighbour) = passed.iter().copied().find(|other| {
            if want < at {
                uses(feature, *other)
            } else {
                uses(*other, feature)
            }
        }) {
            return Err(MoveRefused::Dependency { neighbour });
        }
        for _ in &passed {
            self.try_move_feature_in_history(feature, want < at)?;
        }
        Ok(())
    }

    /// Move a feature into `body`'s history, at its tip, with the features
    /// of its own body that only it uses (its sketch, a datum it alone
    /// reads), in their order. Their data stays as it is, in the body's own
    /// frame, so a body placed elsewhere shows them where it sits. Refuses
    /// a feature something left behind uses, or one that uses something
    /// left behind. Returns the features moved, the given one last.
    pub fn move_feature_to_body(
        &mut self,
        feature_id: FeatureId,
        body: BodyId,
    ) -> Result<Vec<FeatureId>, String> {
        let node = self
            .feature_tree
            .get_node(feature_id)
            .ok_or("no such feature")?;
        let from = node.body.ok_or("the feature belongs to no body")?;
        if from == body {
            return Err("the feature is in that body already".into());
        }
        if self.body(body).is_none() {
            return Err("no such body".into());
        }
        let in_from = |doc: &Self, id: FeatureId| {
            doc.feature_tree
                .get_node(id)
                .is_some_and(|n| n.body == Some(from))
        };
        // The feature and, taken in turn, each input of the moving set that
        // nothing staying behind reads.
        let mut moving = vec![feature_id];
        let mut i = 0;
        while i < moving.len() {
            for dep in self.feature_tree.dependencies(moving[i]) {
                if moving.contains(&dep) || !in_from(self, dep) {
                    continue;
                }
                let shared = self
                    .feature_tree
                    .dependents(dep)
                    .iter()
                    .any(|user| !moving.contains(user) && in_from(self, *user));
                if shared {
                    let name = self
                        .feature_tree
                        .get_node(dep)
                        .map(|n| n.name.clone())
                        .unwrap_or_default();
                    return Err(format!("it uses {name}, which other features use too"));
                }
                moving.push(dep);
            }
            i += 1;
        }
        for id in &moving {
            if let Some(user) = self
                .feature_tree
                .dependents(*id)
                .into_iter()
                .find(|user| !moving.contains(user) && in_from(self, *user))
            {
                let name = self
                    .feature_tree
                    .get_node(user)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                return Err(format!("{name} is built from it"));
            }
        }
        let seq_of = |doc: &Self, id: FeatureId| doc.feature_tree.get_node(id).map(|n| n.seq);
        moving.sort_by_key(|id| (seq_of(self, *id), *id));

        // The source body's tip steps back past what leaves.
        let tip = self.body(from).and_then(|b| b.tip);
        if let Some(tip) = tip.filter(|t| moving.contains(t)) {
            let tip_seq = seq_of(self, tip).unwrap_or(0);
            let before = self
                .feature_tree
                .all_nodes()
                .filter(|(fid, n)| n.body == Some(from) && !moving.contains(fid) && n.seq < tip_seq)
                .max_by_key(|(fid, n)| (n.seq, **fid))
                .map(|(fid, _)| *fid);
            self.set_body_tip(from, before);
        }
        for id in &moving {
            self.record_and_apply(op::DocumentOp::SetFeatureBody {
                id: *id,
                body: Some(body),
                seq: self.feature_tree.next_seq(),
            });
            self.insert_at_tip(*id);
        }
        Ok(moving)
    }

    /// Remove a feature node. Features that depended on it are marked dirty
    /// so their owners can react to the missing input.
    pub fn remove_feature(&mut self, feature_id: FeatureId) -> DocumentResult<()> {
        if self.feature_tree.get_node(feature_id).is_none() {
            return Err(DocumentError::FeatureNotFound(feature_id));
        }
        self.record_and_apply(op::DocumentOp::RemoveFeature { id: feature_id });
        Ok(())
    }

    /// Whether `body` is as `CreateBody` made it: no feature on it, no
    /// geometry, nothing set since. Its removal undoes; any other body's
    /// removal is a history barrier.
    pub fn body_is_bare(&self, body: &Body) -> bool {
        let fresh = Body::new(body.id, body.name.clone(), body.created_at);
        serde_json::to_value(body).ok() == serde_json::to_value(&fresh).ok()
            && !self
                .feature_tree
                .all_nodes()
                .any(|(_, n)| n.body == Some(body.id))
            && !self.derived.imported_meshes.contains_key(&body.id)
            && !self.base_solids.contains_key(&body.id)
    }

    /// Remove a body with every feature attached to it and the geometry it
    /// carried. A bare body's removal undoes; any other has no inverse, so
    /// the entry it records is a history barrier. A copy an array owns is
    /// not removed but skipped by its array.
    pub fn remove_body(&mut self, body: BodyId) -> bool {
        if self.body(body).is_none() {
            return false;
        }
        // A copy an array owns is its occurrence skipped, which undoes.
        if self.array_instance(body).is_some() {
            return self.skip_array_instance(body);
        }
        self.record_and_apply(op::DocumentOp::RemoveBody { id: body });
        true
    }

    /// Take `body` away with its features and the geometry it carried,
    /// its copies left without a shape: what `RemoveBody` does to the body
    /// itself, and to an array's copies the array lets go.
    fn drop_body(&mut self, id: BodyId) {
        self.bodies.retain(|b| b.id != id);
        let orphaned: Vec<FeatureId> = self
            .feature_tree
            .all_nodes()
            .filter(|(_, n)| n.body == Some(id))
            .map(|(fid, _)| *fid)
            .collect();
        for fid in orphaned {
            self.feature_tree.remove_node(fid);
            self.derived.clear_status(fid);
        }
        self.derived.imported_meshes.remove(&id);
        self.derived.local_meshes.remove(&id);
        self.derived.imported_brep_blobs.remove(&id);
        self.derived.imported_brep_face_colors.remove(&id);
        self.base_solids.remove(&id);
        self.mesh_starts.remove(&id);
        self.derived.mesh_results.remove(&id);
        self.base_brep_blobs.remove(&id);
        self.base_face_colors.remove(&id);
        // A mesh asset made for this body alone goes with it.
        let own = serde_json::json!(id.0);
        self.assets
            .retain(|_, asset| asset.metadata.get(shapes::MESH_OF) != Some(&own));
        // Its copies have no shape to take any more.
        self.refresh_copies_of(id);
    }

    pub fn dirty_features(&self) -> Vec<FeatureId> {
        self.feature_tree.dirty_features()
    }

    /// Get recomputation order for dirty features.
    pub fn recompute_order(&self) -> Vec<FeatureId> {
        let dirty = self.dirty_features();
        self.feature_tree.recompute_order(&dirty)
    }

    pub fn feature_tree(&self) -> &FeatureTree {
        &self.feature_tree
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    /// The body `id`, if the document has it.
    pub fn body(&self, id: BodyId) -> Option<&Body> {
        self.bodies.iter().find(|b| b.id == id)
    }

    /// The body `id` to change; only applying an op may.
    fn body_mut(&mut self, id: BodyId) -> Option<&mut Body> {
        self.bodies.iter_mut().find(|b| b.id == id)
    }

    pub fn has_bodies(&self) -> bool {
        !self.bodies.is_empty()
    }

    /// Create a new body entry in the document.
    pub fn create_body(&mut self, name: Option<String>) -> BodyId {
        let id = BodyId::new();
        let name = match name {
            Some(explicit) => explicit,
            None => next_indexed_name("body", self.bodies.iter().map(|b| b.name.as_str())),
        };
        self.record_and_apply(op::DocumentOp::CreateBody {
            id,
            name,
            created_at: epoch_ms_now(),
        });
        id
    }

    /// Show only `bodies`, or every body again with `None`. A view mode:
    /// no op, no edit, each body's visibility left as it is.
    pub fn set_isolation(&mut self, bodies: Option<Vec<BodyId>>) {
        self.isolated = bodies;
    }

    /// The bodies the view is isolated to, while it is.
    pub fn isolation(&self) -> Option<&[BodyId]> {
        self.isolated.as_deref()
    }

    /// Add an asset reference to the document.
    pub fn add_asset(&mut self, asset: AssetReference) -> Uuid {
        let id = asset.id;
        self.record_and_apply(op::DocumentOp::AddAsset { asset, bytes: None });
        id
    }

    /// Add an asset reference together with its raw bytes. The bytes are
    /// preserved in memory until the next `save_to_file` call writes them into
    /// the archive.
    pub fn add_asset_with_data(&mut self, asset: AssetReference, data: Vec<u8>) -> Uuid {
        let id = asset.id;
        self.record_and_apply(op::DocumentOp::AddAsset {
            asset,
            bytes: Some(op::BlobPayload::new(data)),
        });
        id
    }

    /// Get an asset reference by ID.
    pub fn get_asset(&self, asset_id: Uuid) -> Option<&AssetReference> {
        self.assets.get(&asset_id)
    }

    /// Get the raw bytes for an asset, if currently loaded in memory.
    pub fn asset_bytes(&self, asset_id: Uuid) -> Option<&[u8]> {
        self.asset_blobs.get(&asset_id).map(|v| v.as_slice())
    }

    pub fn assets(&self) -> impl Iterator<Item = &AssetReference> {
        self.assets.values()
    }

    /// What the builds found: errors, warnings, answers and the rest of the
    /// derived build state.
    pub fn derived(&self) -> &Derived {
        &self.derived
    }

    /// The handle that writes the derived build state. Nothing done through
    /// it is an edit or records an op.
    pub fn derived_mut(&mut self) -> DerivedMut<'_> {
        DerivedMut::new(self)
    }
}

pub(crate) fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Milliseconds since the epoch, resolved at op-capture time so replay
/// carries the moment rather than re-asking the clock.
fn epoch_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn next_indexed_name<'a>(base: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let mut max_suffix: Option<u32> = None;

    for name in existing {
        if name.eq_ignore_ascii_case(base) {
            max_suffix = Some(max_suffix.unwrap_or(0));
        } else if let Some(rest) = name
            .to_ascii_lowercase()
            .strip_prefix(&(base.to_ascii_lowercase() + "_"))
            && let Ok(n) = rest.parse::<u32>()
        {
            max_suffix = Some(max_suffix.map_or(n, |m| m.max(n)));
        }
    }

    let new_suffix = match max_suffix {
        None => 0,
        Some(m) => m.saturating_add(1),
    };

    if new_suffix == 0 {
        base.to_string()
    } else {
        format!("{base}_{new_suffix}")
    }
}

/// Lightweight metadata block stored alongside the document payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    id: Uuid,
    name: String,
    revision: u64,
    dirty: bool,
}

impl DocumentMetadata {
    fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            revision: 0,
            dirty: false,
        }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }
}

/// Snapshot representing a committed state of the document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRevision {
    pub message: String,
    pub timestamp_epoch_ms: i64,
}

/// Errors surfaced when interacting with documents or workbench registries.
#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("workbench `{0}` already registered")]
    WorkbenchExists(String),
    #[error("workbench `{0}` is not registered")]
    WorkbenchMissing(String),
    #[error("feature kind `{kind}` is already claimed by workbench `{by}`")]
    FeatureKindClaimed { kind: String, by: String },
    #[error("command `{id}` is already registered by {by}")]
    CommandClaimed { id: String, by: String },
    #[error("a file import runs command `{0}`, which its workbench does not register")]
    ImportWithoutCommand(String),
    #[error("document serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("feature not found: {0:?}")]
    FeatureNotFound(FeatureId),
    #[error("feature error: {0}")]
    Feature(#[from] FeatureError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("compression error: {0}")]
    Compression(String),
}

/// Called as an archive is built, with the bytes packed so far and what the
/// whole archive adds up to. Both climb only forward.
pub type ArchiveProgress<'a> = &'a (dyn Fn(u64, u64) + Send + Sync);

#[derive(Debug, Clone, Copy)]
pub enum Compression {
    None,
    Gzip,
    Zstd,
}
