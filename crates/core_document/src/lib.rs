pub mod asset;
pub mod command;
pub mod datum;
pub mod evaluate;
pub mod expr;
pub mod feature;
pub mod history;
pub mod op;
pub mod palette;
pub mod placement;
pub mod rebuild;
pub mod registration;
pub mod runtime;
pub mod server;
pub mod service;
pub mod shortcut;
pub mod undo;
pub mod units;
pub mod variables;
pub mod workbench;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tar::{Archive, Builder, Header};
use thiserror::Error;
use uuid::Uuid;

pub use asset::{AssetReference, AssetType};
pub use command::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind, ParamSpec, Recorded,
};
pub use datum::{
    AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumFrame, DatumShape,
    datums_of_body,
};
pub use evaluate::{Evaluation, Parameter, SlotValue};
pub use feature::{
    BodyId, FeatureError, FeatureId, FeatureNode, FeatureTree, WorkbenchFeature, data_revision,
    node_revision,
};
pub use kernel_api::TriMesh;
pub use palette::SketchPalette;
pub use placement::BodyPlacement;
pub use rebuild::{BuildError, BuildPlan, RebuildJob};
pub use runtime::{
    CameraOrientRequest, EdgeRef, FaceRef, HookOutcome, HostRequest, InputResult, KeyCode,
    LogEntry, LogLevel, MouseButton, SketchAttachRequest, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
pub use service::DocumentService;
pub use shortcut::{ActionDescriptor, Chord};
pub use units::{Unit, format_area_mm2, format_length_mm, format_volume_mm3};
pub use variables::{VARIABLES_KIND, Variable, VariableSet};
pub use workbench::{
    FeatureInfo, MarkKind, MenuItem, MenuScope, OvpRow, OvpWidget, PassiveGeometry, PropertyHints,
    ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, StatusItems, TaskInfo, TaskOutcome,
    TaskRequest, ToolBehavior, ToolDescriptor, ToolHint, ToolVariant, ViewportHud, ViewportPick,
    Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchId, base_tool_id, tool_variant,
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

impl WorkbenchStorage {
    pub fn new(workbench_id: WorkbenchId, data: serde_json::Value) -> Self {
        Self { workbench_id, data }
    }
}

/// Primary data structure persisted by the application.
///
/// The document is saved as a `.prtcad` file, which is a tar archive
/// (optionally gzip- or zstd-compressed) containing:
/// - `thumbnail.png` - A preview of the model, first when present
/// - `document.json` - This document structure (serialized)
/// - `assets/` - External files (STEP, STL, etc.) referenced by the document
/// - `brep/` - Per-body shape snapshots (ogeom native text) and face-color sidecars
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    metadata: DocumentMetadata,
    feature_tree: FeatureTree,
    bodies: Vec<Body>,
    /// Workbench-specific data storage (type-erased).
    workbench_storage: HashMap<String, WorkbenchStorage>,
    /// References to external files stored in the .prtcad archive.
    assets: HashMap<Uuid, AssetReference>,
    /// Tessellated meshes for imported geometry, keyed by body id.
    /// Stored alongside the document so reload doesn't require re-tessellation.
    #[serde(default)]
    imported_meshes: HashMap<BodyId, ImportedGeometry>,
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
    history: Vec<DocumentRevision>,
    /// Raw asset bytes (STEP/STL files, etc.) kept in memory between import
    /// and save. Populated either on import or after `load_from_file`. Skipped
    /// from JSON because the bytes live as separate entries in the tar archive.
    #[serde(skip)]
    asset_blobs: HashMap<Uuid, std::sync::Arc<Vec<u8>>>,
    /// Frozen BRep binaries for deferred STEP tessellation / fast re-open (not in JSON).
    #[serde(skip)]
    imported_brep_blobs: HashMap<BodyId, std::sync::Arc<Vec<u8>>>,
    /// Per-face RGB snapshot parallel to [`Self::imported_brep_blobs`] face order.
    #[serde(skip)]
    imported_brep_face_colors: HashMap<BodyId, Vec<[f32; 3]>>,
    /// The meshes of placed bodies in their own frame, beside the placed
    /// copies in `imported_meshes`, with their bounds. Derived: rebuilt from
    /// the placed copy on load.
    #[serde(skip)]
    local_meshes: HashMap<BodyId, LocalGeometry>,
    /// A PNG preview of the model, written as the container's first entry
    /// (`thumbnail.png`) so a file browser reads it without unpacking the
    /// rest. Derived state: set by the saving client, never an op.
    #[serde(skip)]
    thumbnail: Option<std::sync::Arc<Vec<u8>>>,
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
    /// (op, inverse) pairs since the last journal boundary — the raw
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
}

/// The document's formulas, worked out.
#[derive(Debug, Clone, Default)]
struct Evaluated {
    evaluation: evaluate::Evaluation,
    /// The `mutation_seq` it was worked out at.
    at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub created_at: i64,
    /// Feature exposed as the body's shape. `None` means the last feature in
    /// the history; setting an earlier feature previews that history state
    /// (features after the tip are excluded from the build).
    #[serde(default)]
    pub tip: Option<FeatureId>,
    /// How the body is drawn when the user chose, instead of the material
    /// colour it came with.
    #[serde(default)]
    pub display: Option<BodyDisplay>,
    /// The user asked for the kernel's repair on this body's imported
    /// shape. The repaired shape is derived from it, like the rest of an
    /// import's geometry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repair_requested: bool,
    /// The user asked for this mesh body to become a B-rep solid. The
    /// solid is derived from it, like the rest of an import's geometry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solid_requested: bool,
    /// Kept out of the scene: not drawn, picked or framed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Where the body's own geometry sits in the document. Its features,
    /// sketches and kernel shape stay in the body's frame; the mesh the
    /// scene draws and picks is placed.
    #[serde(default, skip_serializing_if = "BodyPlacement::is_identity")]
    pub placement: BodyPlacement,
}

/// A user-chosen look for a body: its colour and how much of it shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BodyDisplay {
    pub color: [f32; 3],
    /// 1 is solid; less lets what is behind show through.
    pub opacity: f32,
}

/// The archive entry holding a document's PNG preview.
const THUMBNAIL_ENTRY: &str = "thumbnail.png";

/// A body's mesh in its own frame, and that mesh's bounds.
pub type LocalGeometry = (Arc<TriMesh>, Option<([f32; 3], [f32; 3])>);

impl Default for BodyDisplay {
    fn default() -> Self {
        Self {
            color: [0.78, 0.78, 0.82],
            opacity: 1.0,
        }
    }
}

/// Tessellated geometry produced by an external import (STEP, STL, ...).
///
/// `mesh` is wrapped in `Arc` so the renderer can hold on to it across
/// frames without forcing a triangle-data clone every frame, and a `revision`
/// counter lets the GPU mesh cache cheaply detect when the geometry has been
/// reassigned without inspecting triangle data. The counter is bumped by
/// [`Document::set_imported_geometry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedGeometry {
    /// Triangulated representation ready for the viewport.
    pub mesh: std::sync::Arc<TriMesh>,
    /// Optional reference back to the source asset (e.g. STEP file).
    #[serde(default)]
    pub source_asset: Option<Uuid>,
    /// Monotonic counter bumped every time [`Document::set_imported_geometry`]
    /// replaces the mesh for this body. Renderers compare against their cached
    /// revision to know when GPU buffers need to be re-uploaded.
    #[serde(default)]
    pub revision: u64,
    /// Axis-aligned bounds in millimetres (e.g. from raw BRep before tessellation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
    /// Archive path to BRep binary (`brep/<uuid>.bin`) when present on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brep_blob_path: Option<String>,
    /// Archive path to packed per-face colours (`brep/<uuid>.colors`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_colors_path: Option<String>,
    /// What the kernel's checker found in the shape this geometry was
    /// drawn from; `None` for a shape that was never checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<kernel_api::ShapeHealth>,
}

/// Persistent imported object node (assembly/part/instance) shown in the model tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedObjectNode {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub children: Vec<Uuid>,
    pub kind: kernel_api::ImportedNodeKind,
    pub name: String,
    #[serde(default = "default_imported_object_visible")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_id: Option<BodyId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_transform: Option<[[f32; 4]; 4]>,
}

fn default_imported_object_visible() -> bool {
    true
}

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: DocumentMetadata::new(name),
            feature_tree: FeatureTree::new(),
            bodies: Vec::new(),
            workbench_storage: HashMap::new(),
            assets: HashMap::new(),
            imported_meshes: HashMap::new(),
            imported_objects: HashMap::new(),
            imported_object_roots: Vec::new(),
            display_unit: Unit::default(),
            history: Vec::new(),
            asset_blobs: HashMap::new(),
            imported_brep_blobs: HashMap::new(),
            imported_brep_face_colors: HashMap::new(),
            thumbnail: None,
            local_meshes: HashMap::new(),
            imported_body_to_object: HashMap::new(),
            mutation_seq: 0,
            pending_ops: op::OpBuffer::default(),
            journal_pending: op::JournalBuffer::default(),
            history_suppressed: false,
            evaluated: Evaluated::default(),
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

    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }

    pub fn mark_dirty(&mut self) {
        self.metadata.dirty = true;
        self.mutation_seq = self.mutation_seq.wrapping_add(1);
    }

    /// Record a resolved op into the outbox and apply it. The single path
    /// every user-edit mutator funnels through — replay and live edits run
    /// the same `apply_op` code. The inverse is computed from the state the
    /// op is ABOUT to change, so per-user undo can restore it later without
    /// ever snapshotting the document.
    fn record_and_apply(&mut self, operation: op::DocumentOp) {
        if !self.history_suppressed {
            let inverse = self.invert_op(&operation);
            self.journal_pending.record(&operation, inverse);
        }
        self.apply_op(&operation);
        self.pending_ops.record(operation);
    }

    /// Apply an op produced by history traversal (undo/redo): the effect,
    /// this replica's dirty-marking consequences, and the outbox — peers
    /// hear an undo as ordinary ops — but never the journal, which is being
    /// walked, not written.
    pub(crate) fn apply_history_op(&mut self, operation: &op::DocumentOp) {
        self.apply_remote_op(operation);
        self.pending_ops.record(operation.clone());
    }

    /// The op that would restore the state `operation` is about to change.
    /// `None` marks a history barrier: the op is not invertible (imports,
    /// asset registration) and undo history clears rather than lie.
    fn invert_op(&self, operation: &op::DocumentOp) -> Option<op::DocumentOp> {
        use op::DocumentOp as Op;
        Some(match operation {
            Op::SetDocumentName { .. } => Op::SetDocumentName {
                name: self.metadata.name.clone(),
            },
            Op::SetDisplayUnit { .. } => Op::SetDisplayUnit {
                unit: self.display_unit,
            },
            Op::CreateBody { id, .. } => Op::RemoveBody { id: *id },
            Op::RemoveBody { .. } => return None,
            Op::RenameBody { id, .. } => Op::RenameBody {
                id: *id,
                name: self.bodies.iter().find(|b| b.id == *id)?.name.clone(),
            },
            Op::SetBodyDisplay { id, .. } => Op::SetBodyDisplay {
                id: *id,
                display: self.bodies.iter().find(|b| b.id == *id)?.display,
            },
            Op::SetBodyVisible { id, .. } => Op::SetBodyVisible {
                id: *id,
                visible: !self.bodies.iter().find(|b| b.id == *id)?.hidden,
            },
            Op::SetBodyPlacement { id, .. } => Op::SetBodyPlacement {
                id: *id,
                placement: self.bodies.iter().find(|b| b.id == *id)?.placement,
            },
            Op::SetBodyTip { id, .. } => Op::SetBodyTip {
                id: *id,
                tip: self.bodies.iter().find(|b| b.id == *id)?.tip,
            },
            Op::AddFeature { id, .. } => Op::RemoveFeature { id: *id },
            Op::UpdateFeatureData { id, .. } => Op::UpdateFeatureData {
                id: *id,
                data: self.feature_tree.get_node(*id)?.data.clone(),
            },
            Op::RenameFeature { id, .. } => Op::RenameFeature {
                id: *id,
                name: self.feature_tree.get_node(*id)?.name.clone(),
            },
            Op::SetFeatureFormula { id, key, .. } => Op::SetFeatureFormula {
                id: *id,
                key: key.clone(),
                formula: self.feature_tree.get_node(*id)?.formulas.get(key).cloned(),
            },
            Op::SetFeatureVisible { id, .. } => Op::SetFeatureVisible {
                id: *id,
                visible: self.feature_tree.get_node(*id)?.visible,
            },
            Op::SetFeatureSuppressed { id, .. } => Op::SetFeatureSuppressed {
                id: *id,
                suppressed: self.feature_tree.get_node(*id)?.suppressed,
            },
            Op::SetFeatureDependencies { id, .. } => Op::SetFeatureDependencies {
                id: *id,
                deps: self.feature_tree.dependencies(*id),
            },
            Op::SwapFeatureSeq { a, b } => Op::SwapFeatureSeq { a: *a, b: *b },
            Op::RemoveFeature { id } => {
                let node = self.feature_tree.get_node(*id)?;
                Op::AddFeature {
                    id: node.id,
                    workbench_id: node.workbench_id.clone(),
                    name: node.name.clone(),
                    body: node.body,
                    deps: self.feature_tree.dependencies(*id),
                    data: node.data.clone(),
                    seq: node.seq,
                    created_at: node.created_at,
                    formulas: node.formulas.clone(),
                }
            }
            Op::SetImportedObjectVisibility { id, .. } => Op::SetImportedObjectVisibility {
                id: *id,
                visible: self.imported_objects.get(id)?.visible,
            },
            // History barriers: an import (or raw graph write) is not worth
            // lying about — clearing undo beats a wrong inverse.
            Op::AddAsset { .. }
            | Op::RequestBodyRepair { .. }
            | Op::RequestMeshSolid { .. }
            | Op::ImportModel { .. }
            | Op::AppendImportedObjectGraph { .. }
            | Op::ClearImportedObjectGraph => return None,
        })
    }

    /// Ops captured since the last take. Drained once per frame by the host
    /// and handed to the document server.
    pub fn take_pending_ops(&mut self) -> Vec<op::DocumentOp> {
        self.pending_ops.take()
    }

    /// Journal material since the last boundary: (op, inverse) pairs and
    /// whether a non-invertible op crossed. Consumed by the op journal at
    /// gesture boundaries (mouse-up, explicit commits).
    pub fn take_journal_pairs(&mut self) -> (Vec<(op::DocumentOp, op::DocumentOp)>, bool) {
        self.journal_pending.take()
    }

    /// Run `f` with journal capture off — history traversal and remote
    /// application drive the document without journaling themselves.
    pub(crate) fn without_journal<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let previous = self.history_suppressed;
        self.history_suppressed = true;
        let result = f(self);
        self.history_suppressed = previous;
        result
    }

    /// Apply a resolved op **without recording it** — the path a remote or
    /// replayed op takes. Every effect here must be a pure function of
    /// (current state, op); anything nondeterministic was resolved into the
    /// op at capture. Dirty-marking is apply-side policy: applying an op
    /// that changes build inputs marks the affected features dirty, which is
    /// what triggers this replica's own recompute.
    pub fn apply_op(&mut self, operation: &op::DocumentOp) {
        use op::DocumentOp as Op;
        match operation {
            Op::SetDocumentName { name } => {
                self.metadata.name.clone_from(name);
            }
            Op::SetDisplayUnit { unit } => {
                self.display_unit = *unit;
            }
            Op::CreateBody {
                id,
                name,
                created_at,
            } => {
                self.bodies.push(Body {
                    id: *id,
                    name: name.clone(),
                    created_at: *created_at,
                    tip: None,
                    display: None,
                    repair_requested: false,
                    solid_requested: false,
                    hidden: false,
                    placement: BodyPlacement::IDENTITY,
                });
            }
            Op::RenameBody { id, name } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.name.clone_from(name);
                }
            }
            Op::SetBodyDisplay { id, display } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.display = *display;
                }
            }
            Op::RequestBodyRepair { id } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.repair_requested = true;
                }
            }
            Op::SetBodyVisible { id, visible } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.hidden = !visible;
                }
            }
            Op::SetBodyPlacement { id, placement } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.placement = *placement;
                }
                self.place_geometry(*id);
            }
            Op::RequestMeshSolid { id } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.solid_requested = true;
                }
            }
            Op::RemoveBody { id } => {
                self.bodies.retain(|b| b.id != *id);
                let orphaned: Vec<FeatureId> = self
                    .feature_tree
                    .all_nodes()
                    .filter(|(_, n)| n.body == Some(*id))
                    .map(|(fid, _)| *fid)
                    .collect();
                for fid in orphaned {
                    self.feature_tree.remove_node(fid);
                }
                self.imported_meshes.remove(id);
                self.imported_brep_blobs.remove(id);
                self.imported_brep_face_colors.remove(id);
            }
            Op::SetBodyTip { id, tip } => {
                if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == *id) {
                    entry.tip = *tip;
                }
            }
            Op::AddFeature {
                id,
                workbench_id,
                name,
                body,
                deps,
                data,
                seq,
                created_at,
                formulas,
            } => {
                self.feature_tree.add_node(FeatureNode {
                    id: *id,
                    workbench_id: workbench_id.clone(),
                    name: name.clone(),
                    body: *body,
                    visible: true,
                    suppressed: false,
                    dirty: false,
                    created_at: *created_at,
                    seq: *seq,
                    error: None,
                    data: data.clone(),
                    formulas: formulas.clone(),
                });
                for dep in deps {
                    self.feature_tree.add_dependency(*id, *dep);
                }
            }
            Op::UpdateFeatureData { id, data } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.data = data.clone();
                }
            }
            Op::RenameFeature { id, name } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.name.clone_from(name);
                }
            }
            Op::SetFeatureFormula { id, key, formula } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    match formula {
                        Some(formula) => {
                            node.formulas.insert(key.clone(), formula.clone());
                        }
                        None => {
                            node.formulas.remove(key);
                        }
                    }
                }
            }
            Op::SetFeatureVisible { id, visible } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.visible = *visible;
                }
            }
            Op::SetFeatureSuppressed { id, suppressed } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.suppressed = *suppressed;
                }
            }
            Op::SetFeatureDependencies { id, deps } => {
                self.feature_tree.set_dependencies(*id, deps.clone());
                self.feature_tree.mark_dirty(*id);
            }
            Op::SwapFeatureSeq { a, b } => {
                let (Some(seq_a), Some(seq_b)) = (
                    self.feature_tree.get_node(*a).map(|n| n.seq),
                    self.feature_tree.get_node(*b).map(|n| n.seq),
                ) else {
                    return;
                };
                if let Some(n) = self.feature_tree.get_node_mut(*a) {
                    n.seq = seq_b;
                }
                if let Some(n) = self.feature_tree.get_node_mut(*b) {
                    n.seq = seq_a;
                }
                // Order changes results: rebuild the whole history.
                self.feature_tree.mark_dirty(*a);
                self.feature_tree.mark_dirty(*b);
            }
            Op::RemoveFeature { id } => {
                for dep in self.feature_tree.dependents(*id) {
                    self.feature_tree.mark_dirty(dep);
                }
                self.feature_tree.remove_node(*id);
            }
            Op::AddAsset { asset, bytes } => {
                self.assets.insert(asset.id, asset.clone());
                if let Some(payload) = bytes {
                    self.asset_blobs
                        .insert(asset.id, std::sync::Arc::clone(&payload.0));
                }
            }
            Op::ImportModel {
                asset,
                bytes,
                detail: _,
                bodies,
                roots,
                nodes,
                display_unit,
            } => {
                self.assets.insert(asset.id, asset.clone());
                self.asset_blobs
                    .insert(asset.id, std::sync::Arc::clone(&bytes.0));
                for init in bodies {
                    self.bodies.push(Body {
                        id: init.id,
                        name: init.name.clone(),
                        created_at: init.created_at,
                        tip: None,
                        display: None,
                        repair_requested: false,
                        solid_requested: false,
                        hidden: false,
                        placement: BodyPlacement::IDENTITY,
                    });
                }
                self.imported_object_roots.extend(roots.iter().copied());
                for node in nodes {
                    self.imported_objects.insert(node.id, node.clone());
                }
                self.rebuild_imported_body_index();
                if let Some(unit) = display_unit {
                    self.display_unit = *unit;
                }
            }
            Op::AppendImportedObjectGraph { roots, nodes } => {
                self.imported_object_roots.extend(roots.iter().copied());
                for node in nodes {
                    self.imported_objects.insert(node.id, node.clone());
                }
                self.rebuild_imported_body_index();
            }
            Op::SetImportedObjectVisibility { id, visible } => {
                if let Some(node) = self.imported_objects.get_mut(id) {
                    node.visible = *visible;
                }
            }
            Op::ClearImportedObjectGraph => {
                self.imported_object_roots.clear();
                self.imported_objects.clear();
                self.imported_body_to_object.clear();
            }
        }
        self.mark_dirty();
    }

    /// Apply a peer's op: the effect plus this replica's own consequences.
    ///
    /// `apply_op` is the pure effect; on top of it, a foreign edit that
    /// changes build inputs must mark the affected features dirty HERE,
    /// because this replica is the one that has to re-derive the geometry
    /// the peer's edit invalidated. (The peer marked its own copy dirty at
    /// capture; dirty flags are per-replica, never part of the op.)
    pub fn apply_remote_op(&mut self, operation: &op::DocumentOp) {
        use op::DocumentOp as Op;
        self.apply_op(operation);
        match operation {
            Op::AddFeature { id, .. }
            | Op::UpdateFeatureData { id, .. }
            | Op::SetFeatureSuppressed { id, .. } => {
                self.mark_feature_dirty(*id);
            }
            Op::SetBodyTip { tip: Some(tip), .. } => {
                self.mark_feature_dirty(*tip);
            }
            _ => {}
        }
    }

    /// The state that must converge across replicas: the serialized document
    /// minus per-replica derivations — dirty flags, recompute errors,
    /// revision history, and the imported-geometry sidecars (meshes are
    /// re-derived from asset bytes on each replica). Determinism tests
    /// compare projections, not raw serializations.
    pub fn replicated_projection(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("document serializes");
        fn strip_key_recursively(value: &mut serde_json::Value, key: &str) {
            match value {
                serde_json::Value::Object(map) => {
                    map.remove(key);
                    for child in map.values_mut() {
                        strip_key_recursively(child, key);
                    }
                }
                serde_json::Value::Array(items) => {
                    for child in items.iter_mut() {
                        strip_key_recursively(child, key);
                    }
                }
                _ => {}
            }
        }
        strip_key_recursively(&mut value, "dirty");
        if let Some(map) = value.as_object_mut() {
            map.remove("history");
            map.remove("imported_meshes");
            if let Some(meta) = map.get_mut("metadata").and_then(|m| m.as_object_mut()) {
                meta.remove("revision");
            }
        }
        value
    }

    /// See the `mutation_seq` field: bumped on every `mark_dirty`.
    pub fn mutation_seq(&self) -> u64 {
        self.mutation_seq
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
        // Everything is resolved here — id, timestamp, seq — so the op is a
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
        });
        Ok(id)
    }

    /// Get feature data (returns JSON, workbench must deserialize).
    pub fn get_feature_data(&self, id: FeatureId) -> Option<&serde_json::Value> {
        self.feature_tree.get_node(id).map(|n| &n.data)
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

    /// What the feature's formulas and variables come to, in order.
    pub fn evaluated_slots(&self, id: FeatureId) -> &[evaluate::SlotValue] {
        self.evaluated
            .evaluation
            .slots
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// What the feature's data settled to last time its values were
    /// `unsettled`, if they were.
    pub fn settled_values(
        &self,
        id: FeatureId,
        unsettled: &serde_json::Value,
    ) -> Option<&serde_json::Value> {
        let last = &self.evaluated.evaluation;
        (last.unsettled.get(&id) == Some(unsettled))
            .then(|| last.data.get(&id))
            .flatten()
    }

    /// Whether the document changed since its formulas were worked out.
    pub fn needs_evaluation(&self) -> bool {
        self.evaluated.at != Some(self.mutation_seq)
    }

    /// Take `evaluation` as what the formulas come to now. A feature whose
    /// values changed is marked for rebuilding, and what depends on it;
    /// the document is not marked edited, as nothing the user set moved.
    /// Answers the features marked.
    pub fn apply_evaluation(&mut self, evaluation: evaluate::Evaluation) -> Vec<FeatureId> {
        let mut changed: Vec<FeatureId> = Vec::new();
        let old = &self.evaluated.evaluation.data;
        for id in old.keys().chain(evaluation.data.keys()) {
            let raw = self.feature_tree.get_node(*id).map(|n| &n.data);
            let before = old.get(id).or(raw);
            let after = evaluation.data.get(id).or(raw);
            if before != after && !changed.contains(id) {
                changed.push(*id);
            }
        }
        for id in &changed {
            self.feature_tree.mark_dirty(*id);
        }
        self.evaluated = Evaluated {
            evaluation,
            at: Some(self.mutation_seq),
        };
        changed
    }

    /// Get feature metadata (id, name, dirty, etc.).
    pub fn get_feature_meta(&self, id: FeatureId) -> Option<&FeatureNode> {
        self.feature_tree.get_node(id)
    }

    /// Update feature data (workbench provides serialized JSON).
    pub fn update_feature_data(
        &mut self,
        id: FeatureId,
        data: serde_json::Value,
    ) -> DocumentResult<()> {
        if self.feature_tree.get_node(id).is_none() {
            return Err(DocumentError::FeatureNotFound(id));
        }
        self.record_and_apply(op::DocumentOp::UpdateFeatureData { id, data });
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
    /// `Workbench::parameters`), or take it away so the number stands as
    /// it is.
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
            self.record_and_apply(op::DocumentOp::SetFeatureFormula {
                id: feature_id,
                key,
                formula,
            });
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

    /// Rename a feature (user-facing name in the tree and panels).
    pub fn rename_feature(&mut self, feature_id: FeatureId, name: impl Into<String>) {
        let name = name.into();
        if let Some(node) = self.feature_tree.get_node(feature_id)
            && node.name != name
            && !name.trim().is_empty()
        {
            self.record_and_apply(op::DocumentOp::RenameFeature {
                id: feature_id,
                name,
            });
        }
    }

    /// Rename a body.
    pub fn rename_body(&mut self, body: BodyId, name: impl Into<String>) {
        let name = name.into();
        if let Some(entry) = self.bodies.iter().find(|b| b.id == body)
            && entry.name != name
            && !name.trim().is_empty()
        {
            self.record_and_apply(op::DocumentOp::RenameBody { id: body, name });
        }
    }

    /// Give a body a look of its own, or `None` for the one it came with.
    pub fn set_body_display(&mut self, body: BodyId, display: Option<BodyDisplay>) {
        if let Some(entry) = self.bodies.iter().find(|b| b.id == body)
            && entry.display != display
        {
            self.record_and_apply(op::DocumentOp::SetBodyDisplay { id: body, display });
        }
    }

    /// Ask for the kernel's repair on an imported body's shape. Only a body
    /// whose shape came from an import has one to repair, and only once;
    /// returns whether the request was recorded. Not undoable: the shape
    /// before the repair would have to be derived from the file again.
    pub fn request_body_repair(&mut self, body: BodyId) -> bool {
        let pending = self
            .bodies
            .iter()
            .find(|b| b.id == body)
            .is_some_and(|b| !b.repair_requested);
        if !pending || !self.body_solid_is_imported(body) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::RequestBodyRepair { id: body });
        true
    }

    /// Whether a body is a mesh from a mesh file, not yet a solid: it has
    /// triangles and no shape snapshot.
    pub fn is_mesh_body(&self, body: BodyId) -> bool {
        self.imported_geometry(body)
            .and_then(|g| g.source_asset)
            .and_then(|asset| self.get_asset(asset))
            .is_some_and(|asset| asset.asset_type.is_mesh())
            && self.imported_brep_blob(body).is_none()
    }

    /// Ask for a mesh body to become a B-rep solid; once, and only for a
    /// mesh body. Returns whether the request was recorded. Not undoable:
    /// the mesh it was would have to be derived from the file again.
    pub fn request_mesh_solid(&mut self, body: BodyId) -> bool {
        let pending = self
            .bodies
            .iter()
            .find(|b| b.id == body)
            .is_some_and(|b| !b.solid_requested);
        if !pending || !self.is_mesh_body(body) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::RequestMeshSolid { id: body });
        true
    }

    /// Mesh bodies whose conversion was asked for and has not landed: the
    /// host derives each one.
    pub fn bodies_awaiting_solid(&self) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| b.solid_requested && self.is_mesh_body(b.id))
            .map(|b| b.id)
            .collect()
    }

    /// Bodies whose repair was asked for and whose geometry is not yet the
    /// repaired shape: the host derives each one.
    pub fn bodies_awaiting_repair(&self) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| b.repair_requested)
            .filter(|b| {
                self.imported_geometry(b.id).is_some_and(|g| {
                    g.source_asset.is_some() && !g.health.as_ref().is_some_and(|h| h.repaired)
                })
            })
            .map(|b| b.id)
            .collect()
    }

    /// Show or hide a body in the scene.
    pub fn set_body_visible(&mut self, body: BodyId, visible: bool) {
        if let Some(entry) = self.bodies.iter().find(|b| b.id == body)
            && entry.hidden == visible
        {
            self.record_and_apply(op::DocumentOp::SetBodyVisible { id: body, visible });
        }
    }

    /// Move a body to `placement`.
    pub fn set_body_placement(&mut self, body: BodyId, placement: BodyPlacement) {
        if let Some(entry) = self.bodies.iter().find(|b| b.id == body)
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
        self.bodies
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.placement)
            .unwrap_or_default()
    }

    /// A body's geometry as its kernel shape has it, in the body's own
    /// frame: the mesh and its bounds before the body's placement. The
    /// scene's copy (`imported_geometry`) is placed.
    pub fn local_geometry(&self, body: BodyId) -> Option<LocalGeometry> {
        if let Some(local) = self.local_meshes.get(&body) {
            return Some(local.clone());
        }
        self.imported_meshes
            .get(&body)
            .map(|g| (Arc::clone(&g.mesh), g.bounds_mm))
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
            if let Some(geometry) = self.imported_meshes.get(&body) {
                let local = placement.inverse().mesh(&geometry.mesh);
                let bounds = local.bounds();
                self.local_meshes.insert(body, (Arc::new(local), bounds));
            }
        }
    }

    /// Re-derive a body's placed mesh from its own after its placement
    /// changed: the local copy is kept beside it while the body is placed.
    fn place_geometry(&mut self, body: BodyId) {
        let placement = self.body_placement(body);
        let Some((local, local_bounds)) = self.local_geometry(body) else {
            return;
        };
        let Some(geometry) = self.imported_meshes.get_mut(&body) else {
            return;
        };
        if placement.is_identity() {
            geometry.mesh = local;
            geometry.bounds_mm = local_bounds;
            self.local_meshes.remove(&body);
        } else {
            geometry.mesh = Arc::new(placement.mesh(&local));
            geometry.bounds_mm = local_bounds.map(|b| placement.bounds(b));
            self.local_meshes.insert(body, (local, local_bounds));
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
        if let Some(entry) = self.bodies.iter().find(|b| b.id == body)
            && entry.tip != tip
        {
            self.record_and_apply(op::DocumentOp::SetBodyTip { id: body, tip });
        }
    }

    /// Swap a feature with its history neighbour (previous when `up`, next
    /// otherwise) among same-workbench features of its body. Refuses moves
    /// that would place a feature before one of its dependencies (or after a
    /// dependent). Returns whether the order changed.
    pub fn move_feature_in_history(&mut self, feature_id: FeatureId, up: bool) -> bool {
        let Some(node) = self.feature_tree.get_node(feature_id) else {
            return false;
        };
        let (workbench, body, seq) = (node.workbench_id.clone(), node.body, node.seq);

        // Ordered peers = same body + same workbench, sorted by seq.
        let mut peers: Vec<(u64, FeatureId)> = self
            .feature_tree
            .all_nodes()
            .filter(|(_, n)| n.workbench_id == workbench && n.body == body)
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
            return false;
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
            return false;
        }

        // The guard ran above; the op is the resolved swap, pure on replay.
        self.record_and_apply(op::DocumentOp::SwapFeatureSeq {
            a: feature_id,
            b: neighbour_id,
        });
        true
    }

    /// Record (or clear) a recompute error on a feature. Derived state: no
    /// dirty-marking, the error is display-only and refreshed every rebuild.
    pub fn set_feature_error(&mut self, feature_id: FeatureId, error: Option<String>) {
        if let Some(node) = self.feature_tree.get_node_mut(feature_id) {
            node.error = error;
        }
    }

    /// Clear recompute errors on every feature of a body (a rebuild is
    /// starting; failures will re-flag the culprit).
    pub fn clear_body_feature_errors(&mut self, body: BodyId) {
        let ids: Vec<FeatureId> = self
            .feature_tree
            .all_nodes()
            .filter(|(_, n)| n.body == Some(body))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.set_feature_error(id, None);
        }
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

    /// Remove a body with every feature attached to it and the geometry it
    /// carried. Deleting a body has no inverse, so the entry it records is
    /// a history barrier.
    pub fn remove_body(&mut self, body: BodyId) -> bool {
        if !self.bodies.iter().any(|b| b.id == body) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::RemoveBody { id: body });
        true
    }

    /// Get all dirty features.
    pub fn dirty_features(&self) -> Vec<FeatureId> {
        self.feature_tree.dirty_features()
    }

    /// Get recomputation order for dirty features.
    pub fn recompute_order(&self) -> Vec<FeatureId> {
        let dirty = self.dirty_features();
        self.feature_tree.recompute_order(&dirty)
    }

    /// Get workbench storage.
    pub fn get_workbench_storage(&self, wb_id: &WorkbenchId) -> Option<&WorkbenchStorage> {
        self.workbench_storage.get(wb_id.as_str())
    }

    /// Get the feature tree.
    pub fn feature_tree(&self) -> &FeatureTree {
        &self.feature_tree
    }

    /// All document bodies.
    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    /// Returns true if the document contains at least one body.
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

    /// Apply one STEP import as a single atomic op: the asset with its
    /// source bytes, the bodies it created (identities resolved by the
    /// caller), the object hierarchy, and — on a fresh document — the
    /// file's display unit. Geometry is derived state and is written
    /// separately by the host; a replica re-derives it from the bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_import(
        &mut self,
        asset: AssetReference,
        bytes: Vec<u8>,
        detail: kernel_api::TessellationSettings,
        bodies: Vec<op::ImportedBodyInit>,
        roots: Vec<Uuid>,
        mut nodes: Vec<ImportedObjectNode>,
        display_unit: Option<Unit>,
    ) {
        nodes.sort_by_key(|n| n.id);
        self.record_and_apply(op::DocumentOp::ImportModel {
            asset,
            bytes: op::BlobPayload::new(bytes),
            detail,
            bodies,
            roots,
            nodes,
            display_unit,
        });
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

    /// Get asset path within the archive.
    pub fn get_asset_path(&self, asset_id: Uuid) -> Option<&str> {
        self.assets.get(&asset_id).map(|a| a.path.as_str())
    }

    /// Get the raw bytes for an asset, if currently loaded in memory.
    pub fn asset_bytes(&self, asset_id: Uuid) -> Option<&[u8]> {
        self.asset_blobs.get(&asset_id).map(|v| v.as_slice())
    }

    /// Get all assets.
    pub fn assets(&self) -> impl Iterator<Item = &AssetReference> {
        self.assets.values()
    }

    /// Insert (or replace) the tessellated geometry associated with a body.
    ///
    /// The `revision` field on the supplied `ImportedGeometry` is overwritten
    /// with the next monotonic value for this body so renderers can
    /// distinguish "this is the same mesh as last frame" from "this body's
    /// mesh has been replaced" with a cheap u64 comparison.
    /// Store a body's geometry, given in the body's own frame; the scene's
    /// copy is placed where the body sits.
    pub fn set_imported_geometry(&mut self, body: BodyId, mut geometry: ImportedGeometry) {
        let next_revision = self
            .imported_meshes
            .get(&body)
            .map(|prev| prev.revision.saturating_add(1))
            .unwrap_or(0);
        geometry.revision = next_revision;
        self.local_meshes.remove(&body);
        self.imported_meshes.insert(body, geometry);
        if !self.body_placement(body).is_identity() {
            self.place_geometry(body);
        }
        self.mark_dirty();
    }

    /// Drop a body's computed/imported geometry (mesh, BRep snapshot,
    /// face colours). Used when a body's last solid feature is deleted.
    pub fn remove_imported_geometry(&mut self, body: BodyId) {
        let removed = self.imported_meshes.remove(&body).is_some();
        self.local_meshes.remove(&body);
        self.imported_brep_blobs.remove(&body);
        self.imported_brep_face_colors.remove(&body);
        if removed {
            self.mark_dirty();
        }
    }

    /// Store BRep binary + face colour snapshot for a body (in-memory until save).
    /// An empty snapshot is no shape, as a mesh body has none: it clears the
    /// body's snapshot rather than storing one that describes nothing.
    pub fn set_imported_brep_data(
        &mut self,
        body: BodyId,
        brep_blob: Vec<u8>,
        face_colors: Vec<[f32; 3]>,
    ) {
        if brep_blob.is_empty() {
            self.imported_brep_blobs.remove(&body);
            self.imported_brep_face_colors.remove(&body);
        } else {
            self.imported_brep_blobs
                .insert(body, std::sync::Arc::new(brep_blob));
            self.imported_brep_face_colors.insert(body, face_colors);
        }
        self.mark_dirty();
    }

    pub fn imported_brep_blob(&self, body: BodyId) -> Option<&[u8]> {
        self.imported_brep_blobs.get(&body).map(|v| v.as_slice())
    }

    /// Shared handle to a body's BRep snapshot; cloning is a refcount bump,
    /// so this is the cheap way to hand the blob to a worker thread.
    pub fn imported_brep_blob_arc(&self, body: BodyId) -> Option<std::sync::Arc<Vec<u8>>> {
        self.imported_brep_blobs.get(&body).cloned()
    }

    pub fn imported_brep_face_colors(&self, body: BodyId) -> Option<&[[f32; 3]]> {
        self.imported_brep_face_colors
            .get(&body)
            .map(|v| v.as_slice())
    }

    /// Look up tessellated geometry for a body.
    /// Whether this body's solid came from an import rather than from a
    /// feature history. Only the import path stamps the source asset; a
    /// rebuild's own result leaves it unset.
    pub fn body_solid_is_imported(&self, body: BodyId) -> bool {
        self.imported_geometry(body)
            .is_some_and(|geometry| geometry.source_asset.is_some())
    }

    pub fn imported_geometry(&self, body: BodyId) -> Option<&ImportedGeometry> {
        self.imported_meshes.get(&body)
    }

    /// Iterate over all imported geometries currently stored on the document.
    pub fn imported_geometries(&self) -> impl Iterator<Item = (&BodyId, &ImportedGeometry)> {
        self.imported_meshes.iter()
    }

    /// Replace imported object hierarchy with the supplied nodes.
    pub fn set_imported_object_graph(
        &mut self,
        roots: Vec<Uuid>,
        nodes: HashMap<Uuid, ImportedObjectNode>,
    ) {
        // Replacement = clear + append, so both are expressible as ops.
        self.record_and_apply(op::DocumentOp::ClearImportedObjectGraph);
        self.append_imported_object_graph(roots, nodes);
    }

    /// Append imported hierarchy nodes (used when importing multiple STEP files).
    pub fn append_imported_object_graph(
        &mut self,
        roots: Vec<Uuid>,
        nodes: HashMap<Uuid, ImportedObjectNode>,
    ) {
        // Sorted into a vec so the op serializes in a stable order.
        let mut node_list: Vec<ImportedObjectNode> = nodes.into_values().collect();
        node_list.sort_by_key(|n| n.id);
        self.record_and_apply(op::DocumentOp::AppendImportedObjectGraph {
            roots,
            nodes: node_list,
        });
    }

    /// Remove imported hierarchy metadata.
    pub fn clear_imported_object_graph(&mut self) {
        self.record_and_apply(op::DocumentOp::ClearImportedObjectGraph);
    }

    pub fn imported_object_roots(&self) -> &[Uuid] {
        &self.imported_object_roots
    }

    pub fn imported_object(&self, id: Uuid) -> Option<&ImportedObjectNode> {
        self.imported_objects.get(&id)
    }

    pub fn imported_object_for_body(&self, body: BodyId) -> Option<Uuid> {
        self.imported_body_to_object.get(&body).copied()
    }

    /// The body an imported node stands for: its own, or, for an instance
    /// whose only child is the part it places, that part's. The tree shows
    /// such an instance and its part as one row, so selecting the row
    /// means the part's body.
    pub fn body_of_imported_object(&self, id: Uuid) -> Option<BodyId> {
        let node = self.imported_object(id)?;
        if let Some(body) = node.body_id {
            return Some(body);
        }
        if node.kind == kernel_api::ImportedNodeKind::Instance
            && let [child] = node.children.as_slice()
            && let Some(target) = self.imported_object(*child)
            && target.kind != kernel_api::ImportedNodeKind::Instance
        {
            return target.body_id;
        }
        None
    }

    pub fn set_imported_object_visibility(&mut self, id: Uuid, visible: bool) -> bool {
        let changed = self
            .imported_objects
            .get(&id)
            .is_some_and(|node| node.visible != visible);
        if changed {
            self.record_and_apply(op::DocumentOp::SetImportedObjectVisibility { id, visible });
        }
        changed
    }

    pub fn imported_object_effective_visible(&self, id: Uuid) -> bool {
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            let Some(node) = self.imported_objects.get(&current) else {
                return true;
            };
            if !node.visible {
                return false;
            }
            cursor = node.parent_id;
        }
        true
    }

    pub fn imported_body_effective_visible(&self, body: BodyId) -> bool {
        if self.bodies.iter().any(|b| b.id == body && b.hidden) {
            return false;
        }
        match self.imported_body_to_object.get(&body).copied() {
            Some(id) => self.imported_object_effective_visible(id),
            None => true,
        }
    }

    /// Save document to a .prtcad file (tar archive, optionally compressed).
    pub fn save_to_file(&mut self, path: &Path, compression: Compression) -> DocumentResult<()> {
        let file = File::create(path)?;
        self.save_to_writer(file, compression, None)
    }

    /// Serialize the whole `.prtcad` container into memory — what a client
    /// hands a document server that owns the file but never parses it.
    pub fn save_to_bytes(&mut self, compression: Compression) -> DocumentResult<Vec<u8>> {
        let mut bytes = Vec::new();
        self.save_to_writer(&mut bytes, compression, None)?;
        Ok(bytes)
    }

    /// The same, reporting how much of the archive has been packed.
    ///
    /// The archive carries the document, the file every import came from and
    /// every snapshot blob, so a document with an import takes long enough
    /// that the caller wants to say so.
    pub fn save_to_bytes_watched(
        &mut self,
        compression: Compression,
        progress: ArchiveProgress<'_>,
    ) -> DocumentResult<Vec<u8>> {
        let mut bytes = Vec::with_capacity(self.archive_payload_bytes() as usize);
        self.save_to_writer(&mut bytes, compression, Some(progress))?;
        Ok(bytes)
    }

    /// What the blobs in this document add up to: the part of a save whose
    /// cost grows with the model.
    pub fn archive_payload_bytes(&self) -> u64 {
        let assets: u64 = self
            .assets
            .keys()
            .filter_map(|id| self.asset_blobs.get(id))
            .map(|bytes| bytes.len() as u64)
            .sum();
        let breps: u64 = self
            .imported_brep_blobs
            .values()
            .map(|bytes| bytes.len() as u64)
            .sum();
        let colors: u64 = self
            .imported_brep_face_colors
            .values()
            .map(|colors| colors.len() as u64 * 4)
            .sum();
        assets + breps + colors
    }

    fn save_to_writer<W: Write>(
        &mut self,
        out: W,
        compression: Compression,
        progress: Option<ArchiveProgress<'_>>,
    ) -> DocumentResult<()> {
        Self::sync_brep_paths_for_archive(self);
        let file = out;

        match compression {
            Compression::None => {
                let mut builder = Builder::new(file);
                Self::write_archive(&mut builder, self, progress)?;
                builder.finish()?;
            }
            Compression::Gzip => {
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                let mut builder = Builder::new(encoder);
                Self::write_archive(&mut builder, self, progress)?;
                let encoder = builder.into_inner().map_err(|e| {
                    DocumentError::Compression(format!("gzip encoder finalize failed: {e}"))
                })?;
                encoder.finish()?;
            }
            Compression::Zstd => {
                let mut encoder = zstd::Encoder::new(file, 0)
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                {
                    let mut builder = Builder::new(&mut encoder);
                    Self::write_archive(&mut builder, self, progress)?;
                    builder.finish()?;
                }
                encoder
                    .finish()
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Load document from a .prtcad file (auto-detects compression).
    pub fn load_from_file(path: &Path) -> DocumentResult<Self> {
        let (file, compression) = Self::open_container(path)?;
        Self::load_from_reader(file, compression)
    }

    /// The PNG preview a `.prtcad` file carries, read from the front of the
    /// container without unpacking the document: `None` for a file saved
    /// without one, or one that is not a container.
    pub fn read_thumbnail(path: &Path) -> Option<Vec<u8>> {
        let (file, compression) = Self::open_container(path).ok()?;
        let reader: Box<dyn Read> = match compression {
            Compression::None => Box::new(file),
            Compression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
            Compression::Zstd => Box::new(zstd::Decoder::new(std::io::BufReader::new(file)).ok()?),
        };
        let mut archive = Archive::new(reader);
        // The preview is written first; the document after it means there
        // is none, and the rest of the file is not worth reading to be sure.
        let mut entry = archive.entries().ok()?.next()?.ok()?;
        if entry.path().ok()?.as_ref() != Path::new(THUMBNAIL_ENTRY) {
            return None;
        }
        let mut png = Vec::new();
        entry.read_to_end(&mut png).ok()?;
        Some(png)
    }

    /// A container file, opened, with the compression its name or its
    /// first bytes say.
    fn open_container(path: &Path) -> DocumentResult<(File, Compression)> {
        let mut file = File::open(path)?;

        // Detect compression via extension and magic bytes.
        let mut magic = [0u8; 4];
        let _n = file.read(&mut magic)?;
        file.rewind()?;

        // Decide compression based on file name and magic bytes.
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let compression = if file_name.ends_with(".gz")
            || file_name.ends_with(".prtcad.gz")
            || magic.starts_with(&[0x1f, 0x8b])
        {
            Compression::Gzip
        } else if file_name.ends_with(".zst") || file_name.ends_with(".prtcad.zst") {
            Compression::Zstd
        } else {
            Compression::None
        };
        Ok((file, compression))
    }

    /// Parse a `.prtcad` container from memory — the client side of a
    /// byte-serving document server. Compression is detected from the magic
    /// bytes alone (no filename to consult).
    pub fn load_from_bytes(bytes: Vec<u8>) -> DocumentResult<Self> {
        let compression = if bytes.starts_with(&[0x1f, 0x8b]) {
            Compression::Gzip
        } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            Compression::Zstd
        } else {
            Compression::None
        };
        Self::load_from_reader(std::io::Cursor::new(bytes), compression)
    }

    fn load_from_reader<R: Read + 'static>(
        file: R,
        compression: Compression,
    ) -> DocumentResult<Self> {
        let mut archive: Archive<Box<dyn Read>> = match compression {
            Compression::None => Archive::new(Box::new(file)),
            Compression::Gzip => {
                let decoder = flate2::read::GzDecoder::new(file);
                Archive::new(Box::new(decoder))
            }
            Compression::Zstd => {
                let decoder = zstd::Decoder::new(std::io::BufReader::new(file))
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                Archive::new(Box::new(decoder))
            }
        };

        // First pass: collect document.json plus any asset entries by archive
        // path. We can't seek inside a streaming archive, so this happens in a
        // single traversal.
        let mut doc_json: Option<Vec<u8>> = None;
        let mut thumbnail: Option<Vec<u8>> = None;
        let mut blobs_by_path: HashMap<String, Vec<u8>> = HashMap::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let entry_path = entry.path()?.to_path_buf();
            let entry_path_str = entry_path.to_string_lossy().to_string();
            if entry_path == Path::new("document.json") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                doc_json = Some(buf);
            } else if entry_path == Path::new(THUMBNAIL_ENTRY) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                thumbnail = Some(buf);
            } else if entry_path_str.starts_with("assets/") || entry_path_str.starts_with("brep/") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                blobs_by_path.insert(entry_path_str, buf);
            }
        }

        let json = doc_json.ok_or_else(|| {
            DocumentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "document.json not found in archive",
            ))
        })?;
        let mut doc: Document = serde_json::from_slice(&json)?;
        doc.thumbnail = thumbnail.map(std::sync::Arc::new);
        doc.recover_local_meshes();

        // Resolve any asset blobs that match an `AssetReference::path`.
        for (asset_id, asset) in &doc.assets {
            if let Some(bytes) = blobs_by_path.remove(&asset.path) {
                doc.asset_blobs
                    .insert(*asset_id, std::sync::Arc::new(bytes));
            }
        }

        // Restore shape-snapshot sidecars for imported geometry. Blobs are
        // ogeom native-format text ("ogeom" magic); snapshots written by the
        // previous kernel are unreadable now — drop them with a clear log
        // line rather than failing later with a parse error (the body's mesh
        // still loads; re-import the STEP source to restore the solid).
        for (body_id, geom) in &doc.imported_meshes {
            if let Some(ref brep_path) = geom.brep_blob_path
                && let Some(bytes) = blobs_by_path.remove(brep_path)
            {
                if bytes.starts_with(b"ogeom") {
                    doc.imported_brep_blobs
                        .insert(*body_id, std::sync::Arc::new(bytes));
                } else {
                    tracing::warn!(
                        body = ?body_id,
                        "shape snapshot predates the ogeom kernel; \
                         re-import the STEP source to restore this body's solid"
                    );
                }
            }
            if let Some(ref col_path) = geom.face_colors_path
                && let Some(bytes) = blobs_by_path.remove(col_path)
                && let Some(parsed) = decode_face_colors_blob(&bytes)
            {
                doc.imported_brep_face_colors.insert(*body_id, parsed);
            }
        }

        doc.rebuild_imported_body_index();
        Ok(doc)
    }

    fn write_archive<W: Write>(
        builder: &mut Builder<W>,
        doc: &Document,
        progress: Option<ArchiveProgress<'_>>,
    ) -> DocumentResult<()> {
        let json = serde_json::to_vec_pretty(doc)?;
        // The document's own JSON is only known once it is built, so the
        // total the caller sees settles here and holds for the rest.
        let total = doc.archive_payload_bytes() + json.len() as u64;
        let mut packed = 0u64;
        let report = |packed: u64| {
            if let Some(progress) = progress {
                progress(packed.min(total), total);
            }
        };
        // The preview goes first: a reader after it alone stops at the
        // first entry.
        if let Some(png) = &doc.thumbnail {
            let mut header = Header::new_gnu();
            header.set_path(THUMBNAIL_ENTRY)?;
            header.set_size(png.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, png.as_slice())?;
        }
        let mut header = Header::new_gnu();
        header.set_path("document.json")?;
        header.set_size(json.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &json[..])?;
        packed += json.len() as u64;
        report(packed);

        // Emit asset blobs alongside the document so future reloads can recover
        // the original imported file (e.g. for re-tessellation at a different
        // detail level).
        for (asset_id, asset) in &doc.assets {
            let Some(bytes) = doc.asset_blobs.get(asset_id) else {
                continue;
            };
            let mut header = Header::new_gnu();
            header.set_path(&asset.path)?;
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &bytes[..])?;
            packed += bytes.len() as u64;
            report(packed);
        }

        for (body_id, geom) in &doc.imported_meshes {
            if !doc.imported_brep_blobs.contains_key(body_id) {
                continue;
            }
            let Some(brep_path) = geom.brep_blob_path.as_ref() else {
                continue;
            };
            let Some(colors_path) = geom.face_colors_path.as_ref() else {
                continue;
            };
            let Some(brep_bytes) = doc.imported_brep_blobs.get(body_id) else {
                continue;
            };
            let Some(colors) = doc.imported_brep_face_colors.get(body_id) else {
                continue;
            };
            let colors_bytes = encode_face_colors_blob(colors);

            let mut header = Header::new_gnu();
            header.set_path(brep_path)?;
            header.set_size(brep_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, brep_bytes.as_slice())?;

            let mut header = Header::new_gnu();
            header.set_path(colors_path)?;
            header.set_size(colors_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &colors_bytes[..])?;
            packed += brep_bytes.len() as u64 + colors_bytes.len() as u64;
            report(packed);
        }
        report(total);
        Ok(())
    }

    fn sync_brep_paths_for_archive(doc: &mut Document) {
        for (body_id, geom) in doc.imported_meshes.iter_mut() {
            if doc.imported_brep_blobs.contains_key(body_id) {
                geom.brep_blob_path = Some(format!("brep/{}.bin", body_id.0));
                geom.face_colors_path = Some(format!("brep/{}.colors", body_id.0));
            }
        }
    }

    fn rebuild_imported_body_index(&mut self) {
        self.imported_body_to_object.clear();

        // Build index in tree order (roots -> descendants) and keep the first
        // owner for each body to avoid hash-order instability when multiple
        // metadata nodes reference the same tessellated body.
        let mut stack: Vec<Uuid> = self.imported_object_roots.iter().rev().copied().collect();
        while let Some(node_id) = stack.pop() {
            if let Some(node) = self.imported_objects.get(&node_id) {
                if let Some(body_id) = node.body_id {
                    self.imported_body_to_object
                        .entry(body_id)
                        .or_insert(node_id);
                }
                for child in node.children.iter().rev() {
                    stack.push(*child);
                }
            }
        }

        // Fallback for legacy / detached nodes not reachable from roots.
        for (id, node) in &self.imported_objects {
            if let Some(body_id) = node.body_id {
                self.imported_body_to_object.entry(body_id).or_insert(*id);
            }
        }
    }
}

fn encode_face_colors_blob(colors: &[[f32; 3]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + colors.len() * 12);
    v.extend_from_slice(&(colors.len() as u32).to_le_bytes());
    for c in colors {
        for comp in c {
            v.extend_from_slice(&comp.to_le_bytes());
        }
    }
    v
}

fn decode_face_colors_blob(data: &[u8]) -> Option<Vec<[f32; 3]>> {
    if data.len() < 4 {
        return None;
    }
    let n = u32::from_le_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    let need = 4usize.saturating_add(n.saturating_mul(12));
    if data.len() < need {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = 4 + i * 12;
        out.push([
            f32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?),
            f32::from_le_bytes(data.get(o + 4..o + 8)?.try_into().ok()?),
            f32::from_le_bytes(data.get(o + 8..o + 12)?.try_into().ok()?),
        ]);
    }
    Some(out)
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
    #[error("document serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
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
