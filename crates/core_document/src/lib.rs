pub mod asset;
pub mod datum;
pub mod feature;
pub mod registration;
pub mod runtime;
pub mod service;
pub mod undo;
pub mod units;
pub mod workbench;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tar::{Archive, Builder, Header};
use thiserror::Error;
use uuid::Uuid;

pub use asset::{AssetReference, AssetType};
pub use datum::{
    datums_of_body, AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumFrame,
    DatumShape,
};
pub use feature::{BodyId, FeatureError, FeatureId, FeatureNode, FeatureTree, WorkbenchFeature};
pub use kernel_api::TriMesh;
pub use runtime::{
    CameraOrientRequest, FaceRef, InputResult, KeyCode, LogEntry, LogLevel, MouseButton,
    SketchAttachRequest, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
pub use service::DocumentService;
pub use units::{format_length_mm, Unit};
pub use workbench::{
    CommandDescriptor, ScreenSpaceOverlay, ToolBehavior, ToolDescriptor, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchId,
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
/// - `document.json` - This document structure (serialized)
/// - `assets/` - External files (STEP, STL, etc.) referenced by the document
/// - `brep/` - Per-body OCCT B-Rep snapshots and face-color sidecars
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
    /// Reverse index for fast body->imported-object visibility checks.
    #[serde(skip)]
    imported_body_to_object: HashMap<BodyId, Uuid>,
    /// Monotonic edit counter bumped by [`Self::mark_dirty`]. Cheap change
    /// detection for the undo system: equal values mean "no edits since".
    /// Not persisted; only compared for equality within one process.
    #[serde(skip)]
    mutation_seq: u64,
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
            imported_body_to_object: HashMap::new(),
            mutation_seq: 0,
        }
    }

    /// Currently selected display unit for this document (mm by default).
    pub fn display_unit(&self) -> Unit {
        self.display_unit
    }

    /// Override the display unit. Marks the document dirty so the choice is
    /// persisted on the next save.
    pub fn set_display_unit(&mut self, unit: Unit) {
        if self.display_unit != unit {
            self.display_unit = unit;
            self.mark_dirty();
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
            self.metadata.name = name;
            self.mark_dirty();
        }
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
        self.mutation_seq
    }

    pub fn mark_clean(&mut self) {
        self.metadata.dirty = false;
    }

    pub fn push_revision(&mut self, revision: DocumentRevision) {
        self.history.push(revision);
        self.metadata.revision += 1;
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
        let id = FeatureId::new();
        let deps = feature.dependencies();
        let seq = self.feature_tree.next_seq();

        let node = FeatureNode {
            id,
            workbench_id: F::workbench_id(),
            name,
            body,
            visible: true,
            suppressed: false,
            dirty: false,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            seq,
            error: None,
            data: feature.to_json(),
        };

        self.feature_tree.add_node(node);

        // Add dependencies
        for dep in deps {
            self.feature_tree.add_dependency(id, dep);
        }

        self.mark_dirty();
        Ok(id)
    }

    /// Get feature data (returns JSON, workbench must deserialize).
    pub fn get_feature_data(&self, id: FeatureId) -> Option<&serde_json::Value> {
        self.feature_tree.get_node(id).map(|n| &n.data)
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
        if let Some(node) = self.feature_tree.get_node_mut(id) {
            node.data = data;
            self.mark_dirty();
            Ok(())
        } else {
            Err(DocumentError::FeatureNotFound(id))
        }
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
        if let Some(node) = self.feature_tree.get_node_mut(feature_id) {
            if node.visible != visible {
                node.visible = visible;
                self.mark_dirty();
            }
        }
    }

    /// Rewire a feature's dependencies (marks it dirty for recompute).
    pub fn set_feature_dependencies(&mut self, feature_id: FeatureId, deps: Vec<FeatureId>) {
        self.feature_tree.set_dependencies(feature_id, deps);
        self.mark_feature_dirty(feature_id);
    }

    /// Rename a feature (user-facing name in the tree and panels).
    pub fn rename_feature(&mut self, feature_id: FeatureId, name: impl Into<String>) {
        if let Some(node) = self.feature_tree.get_node_mut(feature_id) {
            let name = name.into();
            if node.name != name && !name.trim().is_empty() {
                node.name = name;
                self.mark_dirty();
            }
        }
    }

    /// Rename a body.
    pub fn rename_body(&mut self, body: BodyId, name: impl Into<String>) {
        if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == body) {
            let name = name.into();
            if entry.name != name && !name.trim().is_empty() {
                entry.name = name;
                self.mark_dirty();
            }
        }
    }

    /// Suppress/unsuppress a feature (excluded from builds while suppressed).
    pub fn set_feature_suppressed(&mut self, feature_id: FeatureId, suppressed: bool) {
        if let Some(node) = self.feature_tree.get_node_mut(feature_id) {
            if node.suppressed != suppressed {
                node.suppressed = suppressed;
                self.mark_dirty();
            }
        }
    }

    /// Set (or clear) the feature exposed as a body's shape. Features after
    /// the tip are excluded from the build until the tip moves back.
    pub fn set_body_tip(&mut self, body: BodyId, tip: Option<FeatureId>) {
        if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == body) {
            if entry.tip != tip {
                entry.tip = tip;
                self.mark_dirty();
            }
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
        peers.sort_by_key(|(s, _)| *s);
        let position = peers.iter().position(|(s, _)| *s == seq).unwrap_or(0);
        let neighbour_pos = if up {
            position.checked_sub(1)
        } else {
            (position + 1 < peers.len()).then_some(position + 1)
        };
        let Some(neighbour_pos) = neighbour_pos else {
            return false;
        };
        let (neighbour_seq, neighbour_id) = peers[neighbour_pos];

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

        if let Some(n) = self.feature_tree.get_node_mut(feature_id) {
            n.seq = neighbour_seq;
        }
        if let Some(n) = self.feature_tree.get_node_mut(neighbour_id) {
            n.seq = seq;
        }
        // Order changes results: rebuild the whole history.
        self.feature_tree.mark_dirty(feature_id);
        self.feature_tree.mark_dirty(neighbour_id);
        self.mark_dirty();
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
        let dependents = self.feature_tree.dependents(feature_id);
        for dep in &dependents {
            self.feature_tree.mark_dirty(*dep);
        }
        if self.feature_tree.remove_node(feature_id) {
            self.mark_dirty();
            Ok(())
        } else {
            Err(DocumentError::FeatureNotFound(feature_id))
        }
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

    /// Get mutable workbench storage.
    pub fn get_workbench_storage_mut(
        &mut self,
        wb_id: &WorkbenchId,
    ) -> Option<&mut WorkbenchStorage> {
        self.workbench_storage.get_mut(wb_id.as_str())
    }

    /// Set workbench storage.
    pub fn set_workbench_storage(&mut self, wb_id: WorkbenchId, data: serde_json::Value) {
        self.workbench_storage.insert(
            wb_id.as_str().to_string(),
            WorkbenchStorage::new(wb_id, data),
        );
        self.mark_dirty();
    }

    /// Get the feature tree.
    pub fn feature_tree(&self) -> &FeatureTree {
        &self.feature_tree
    }

    /// Get mutable feature tree.
    pub fn feature_tree_mut(&mut self) -> &mut FeatureTree {
        &mut self.feature_tree
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
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let body_name = match name {
            Some(explicit) => explicit,
            None => next_indexed_name("body", self.bodies.iter().map(|b| b.name.as_str())),
        };
        let body = Body {
            id,
            name: body_name,
            created_at,
            tip: None,
        };
        self.bodies.push(body);
        self.mark_dirty();
        id
    }

    /// Add an asset reference to the document.
    pub fn add_asset(&mut self, asset: AssetReference) -> Uuid {
        let id = asset.id;
        self.assets.insert(id, asset);
        self.mark_dirty();
        id
    }

    /// Add an asset reference together with its raw bytes. The bytes are
    /// preserved in memory until the next `save_to_file` call writes them into
    /// the archive.
    pub fn add_asset_with_data(&mut self, asset: AssetReference, data: Vec<u8>) -> Uuid {
        let id = asset.id;
        self.assets.insert(id, asset);
        self.asset_blobs.insert(id, std::sync::Arc::new(data));
        self.mark_dirty();
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
    pub fn set_imported_geometry(&mut self, body: BodyId, mut geometry: ImportedGeometry) {
        let next_revision = self
            .imported_meshes
            .get(&body)
            .map(|prev| prev.revision.saturating_add(1))
            .unwrap_or(0);
        geometry.revision = next_revision;
        self.imported_meshes.insert(body, geometry);
        self.mark_dirty();
    }

    /// Drop a body's computed/imported geometry (mesh, BRep snapshot,
    /// face colours). Used when a body's last solid feature is deleted.
    pub fn remove_imported_geometry(&mut self, body: BodyId) {
        let removed = self.imported_meshes.remove(&body).is_some();
        self.imported_brep_blobs.remove(&body);
        self.imported_brep_face_colors.remove(&body);
        if removed {
            self.mark_dirty();
        }
    }

    /// Store BRep binary + face colour snapshot for a body (in-memory until save).
    pub fn set_imported_brep_data(
        &mut self,
        body: BodyId,
        brep_blob: Vec<u8>,
        face_colors: Vec<[f32; 3]>,
    ) {
        self.imported_brep_blobs
            .insert(body, std::sync::Arc::new(brep_blob));
        self.imported_brep_face_colors.insert(body, face_colors);
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
        self.imported_object_roots = roots;
        self.imported_objects = nodes;
        self.rebuild_imported_body_index();
        self.mark_dirty();
    }

    /// Append imported hierarchy nodes (used when importing multiple STEP files).
    pub fn append_imported_object_graph(
        &mut self,
        roots: Vec<Uuid>,
        nodes: HashMap<Uuid, ImportedObjectNode>,
    ) {
        self.imported_object_roots.extend(roots);
        for (id, node) in nodes {
            self.imported_objects.insert(id, node);
        }
        self.rebuild_imported_body_index();
        self.mark_dirty();
    }

    /// Remove imported hierarchy metadata.
    pub fn clear_imported_object_graph(&mut self) {
        self.imported_object_roots.clear();
        self.imported_objects.clear();
        self.imported_body_to_object.clear();
        self.mark_dirty();
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

    pub fn set_imported_object_visibility(&mut self, id: Uuid, visible: bool) -> bool {
        let mut changed = false;
        if let Some(node) = self.imported_objects.get_mut(&id) {
            if node.visible != visible {
                node.visible = visible;
                changed = true;
            }
        }
        if changed {
            self.mark_dirty();
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
        match self.imported_body_to_object.get(&body).copied() {
            Some(id) => self.imported_object_effective_visible(id),
            None => true,
        }
    }

    /// Save document to a .prtcad file (tar archive, optionally compressed).
    pub fn save_to_file(&mut self, path: &Path, compression: Compression) -> DocumentResult<()> {
        Self::sync_brep_paths_for_archive(self);
        let file = File::create(path)?;

        match compression {
            Compression::None => {
                let mut builder = Builder::new(file);
                Self::write_archive(&mut builder, self)?;
                builder.finish()?;
            }
            Compression::Gzip => {
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                let mut builder = Builder::new(encoder);
                Self::write_archive(&mut builder, self)?;
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
                    Self::write_archive(&mut builder, self)?;
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

        let mut archive: Archive<Box<dyn Read>> = match compression {
            Compression::None => Archive::new(Box::new(file)),
            Compression::Gzip => {
                let decoder = flate2::read::GzDecoder::new(file);
                Archive::new(Box::new(decoder))
            }
            Compression::Zstd => {
                let decoder = zstd::Decoder::new(file)
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                Archive::new(Box::new(decoder))
            }
        };

        // First pass: collect document.json plus any asset entries by archive
        // path. We can't seek inside a streaming archive, so this happens in a
        // single traversal.
        let mut doc_json: Option<Vec<u8>> = None;
        let mut blobs_by_path: HashMap<String, Vec<u8>> = HashMap::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let entry_path = entry.path()?.to_path_buf();
            let entry_path_str = entry_path.to_string_lossy().to_string();
            if entry_path == Path::new("document.json") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                doc_json = Some(buf);
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

        // Resolve any asset blobs that match an `AssetReference::path`.
        for (asset_id, asset) in &doc.assets {
            if let Some(bytes) = blobs_by_path.remove(&asset.path) {
                doc.asset_blobs
                    .insert(*asset_id, std::sync::Arc::new(bytes));
            }
        }

        // Restore BRep sidecars for imported geometry.
        for (body_id, geom) in &doc.imported_meshes {
            if let Some(ref brep_path) = geom.brep_blob_path {
                if let Some(bytes) = blobs_by_path.remove(brep_path) {
                    doc.imported_brep_blobs
                        .insert(*body_id, std::sync::Arc::new(bytes));
                }
            }
            if let Some(ref col_path) = geom.face_colors_path {
                if let Some(bytes) = blobs_by_path.remove(col_path) {
                    if let Some(parsed) = decode_face_colors_blob(&bytes) {
                        doc.imported_brep_face_colors.insert(*body_id, parsed);
                    }
                }
            }
        }

        doc.rebuild_imported_body_index();
        Ok(doc)
    }

    fn write_archive<W: Write>(builder: &mut Builder<W>, doc: &Document) -> DocumentResult<()> {
        let json = serde_json::to_vec_pretty(doc)?;
        let mut header = Header::new_gnu();
        header.set_path("document.json")?;
        header.set_size(json.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &json[..])?;

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
        }
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

fn next_indexed_name<'a>(base: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let mut max_suffix: Option<u32> = None;

    for name in existing {
        if name.eq_ignore_ascii_case(base) {
            max_suffix = Some(max_suffix.map_or(0, |m| m));
        } else if let Some(rest) = name
            .to_ascii_lowercase()
            .strip_prefix(&(base.to_ascii_lowercase() + "_"))
        {
            if let Ok(n) = rest.parse::<u32>() {
                max_suffix = Some(max_suffix.map_or(n, |m| m.max(n)));
            }
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

#[derive(Debug, Clone, Copy)]
pub enum Compression {
    None,
    Gzip,
    Zstd,
}
