//! Imported geometry, the imported object graph and its visibility.

use super::*;

/// A body's mesh in its own frame, and that mesh's bounds.
pub type LocalGeometry = (Arc<TriMesh>, Option<([f32; 3], [f32; 3])>);

/// Tessellated geometry produced by an external import (STEP, STL, ...).
///
/// `mesh` is wrapped in `Arc` so the renderer can hold on to it across
/// frames without forcing a triangle-data clone every frame, and a `revision`
/// counter lets the GPU mesh cache cheaply detect when the geometry has been
/// reassigned without inspecting triangle data. The counter is bumped by
/// [`Document::set_imported_geometry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedGeometry {
    /// Triangulated representation ready for the viewport. An archive keeps
    /// it as an entry of its own ([`Self::mesh_path`]), the document's JSON
    /// an empty mesh in its place.
    #[serde(default, serialize_with = "serialize_mesh")]
    pub mesh: std::sync::Arc<TriMesh>,
    /// Archive path to the mesh (`mesh/<uuid>.bin`) when it is kept apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_path: Option<String>,
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

impl ImportedGeometry {
    /// The geometry's box `(min, max)`: the shape's own bounds when kept,
    /// else the mesh's; `None` for an empty mesh with no bounds kept.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        self.bounds_mm.or_else(|| self.mesh.bounds())
    }
}

thread_local! {
    /// Set while an archive's JSON is written: its meshes are entries of
    /// their own, which a document's JSON would hold as text many times
    /// their size (past what one buffer may hold on a 32-bit target).
    static MESHES_APART: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn serialize_mesh<S: serde::Serializer>(
    mesh: &std::sync::Arc<TriMesh>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if MESHES_APART.with(std::cell::Cell::get) {
        // An empty mesh, which a reader without `mesh_path` still takes.
        TriMesh::default().serialize(serializer)
    } else {
        mesh.serialize(serializer)
    }
}

/// Keeps meshes out of the JSON written while it lives.
pub(crate) struct MeshesApart;

impl MeshesApart {
    pub(crate) fn start() -> Self {
        MESHES_APART.with(|apart| apart.set(true));
        MeshesApart
    }
}

impl Drop for MeshesApart {
    fn drop(&mut self) {
        MESHES_APART.with(|apart| apart.set(false));
    }
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
    /// What an annotation node says and draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<Annotation>,
    /// The layers the file puts this node's body on, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
}

/// An annotation an imported file carries: a dimension, a tolerance, a
/// datum or a note, with the lines it draws and where its label goes.
///
/// The geometry is in the frame of the body it describes, which for an
/// imported body is where the import put it; it is drawn where the body
/// is placed, and hidden with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub kind: kernel_api::AnnotationKind,
    /// What its label shows.
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub polylines: Vec<Vec<[f32; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f32; 3]>,
    /// The body it describes, where the file says which.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyId>,
}

fn default_imported_object_visible() -> bool {
    true
}

impl Document {
    /// Apply one file import as a single atomic op: the asset with its
    /// source bytes, the bodies it created (identities resolved by the
    /// caller), the object hierarchy, and, on a fresh document, the
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

    /// Store a body's geometry, given in the body's own frame; the scene's
    /// copy is placed where the body sits.
    ///
    /// The `revision` field on the supplied `ImportedGeometry` is overwritten
    /// with the next revision the document hands out, so renderers can
    /// distinguish "this is the same mesh as last frame" from "this body's
    /// mesh has been replaced" with a cheap u64 comparison.
    pub fn set_imported_geometry(&mut self, body: BodyId, mut geometry: ImportedGeometry) {
        let next_revision = self
            .imported_meshes
            .get(&body)
            .map(|prev| prev.revision.saturating_add(1))
            .unwrap_or(0)
            .max(self.next_geometry_revision);
        self.next_geometry_revision = next_revision.saturating_add(1);
        geometry.revision = next_revision;
        self.local_meshes.remove(&body);
        self.imported_meshes.insert(body, geometry);
        if !self.body_placement(body).is_identity() {
            self.place_geometry(body);
        }
        self.refresh_copies_of(body);
        self.mark_dirty();
    }

    /// Drop a body's computed/imported geometry (mesh, BRep snapshot,
    /// face colours). Used when a body's history builds no solid.
    pub fn remove_imported_geometry(&mut self, body: BodyId) {
        let removed = self.imported_meshes.remove(&body).is_some();
        self.local_meshes.remove(&body);
        self.imported_brep_blobs.remove(&body);
        self.imported_brep_face_colors.remove(&body);
        self.refresh_copies_of(body);
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
        self.refresh_copies_of(body);
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

    /// Whether this body's solid came from an import rather than from a
    /// feature history. Only the import path stamps the source asset; a
    /// rebuild's own result leaves it unset.
    pub fn body_solid_is_imported(&self, body: BodyId) -> bool {
        // A body with a base solid builds its shape from its history, the
        // base first.
        if self.base_solids.contains_key(&body) {
            return false;
        }
        // A stand-in shows in place of a build: the body still builds.
        self.copy_source(body).is_some()
            || self.body(body).is_some_and(|b| b.link.is_some())
            || self.imported_geometry(body).is_some_and(|geometry| {
                geometry
                    .source_asset
                    .is_some_and(|asset| !self.is_stand_in(body, asset))
            })
    }

    /// Look up tessellated geometry for a body.
    pub fn imported_geometry(&self, body: BodyId) -> Option<&ImportedGeometry> {
        self.imported_meshes.get(&body)
    }

    /// The feature of `body` that made its face named `name`: the one whose
    /// tag the mesh records as the face's maker. `None` for a face the
    /// body's features did not make (an imported shape's) or a mesh that
    /// records none.
    pub fn face_maker(&self, body: BodyId, name: kernel_api::TopoName) -> Option<FeatureId> {
        if name == 0 || self.body_solid_is_imported(body) {
            return None;
        }
        let mesh = &self.imported_geometry(body)?.mesh;
        let face = mesh.face_names.iter().position(|n| *n == name)?;
        let maker = *mesh.face_makers.get(face).filter(|m| **m != 0)?;
        self.feature_tree
            .all_nodes()
            .filter(|(_, n)| n.body == Some(body))
            .map(|(id, _)| *id)
            .find(|id| kernel_api::naming::name_of_id(id.0.as_bytes()) == maker)
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

    /// Append imported hierarchy nodes.
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

    /// Every imported annotation, in tree order, with its node.
    pub fn imported_annotations(&self) -> Vec<(&ImportedObjectNode, &Annotation)> {
        let mut out = Vec::new();
        let mut stack: Vec<Uuid> = self.imported_object_roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = self.imported_objects.get(&id) else {
                continue;
            };
            if let Some(annotation) = &node.annotation {
                out.push((node, annotation));
            }
            stack.extend(node.children.iter().rev().copied());
        }
        out
    }

    /// Whether an annotation shows: its row and every row above it shown,
    /// and the body it describes too.
    pub fn imported_annotation_effective_visible(&self, id: Uuid) -> bool {
        let Some(node) = self.imported_objects.get(&id) else {
            return false;
        };
        self.imported_object_effective_visible(id)
            && node
                .annotation
                .as_ref()
                .and_then(|a| a.body)
                .is_none_or(|body| self.imported_body_effective_visible(body))
    }

    pub fn imported_body_effective_visible(&self, body: BodyId) -> bool {
        // Isolated, a kept body shows even when hidden: the view shows the
        // selection, whatever the eyes say. A configuration still leaves
        // out what it leaves out.
        match &self.isolated {
            Some(kept) => kept.contains(&body) && !self.left_out.contains(&body),
            None => self.imported_body_saved_visible(body),
        }
    }

    /// Whether `body` shows by its own visibility and what holds it, the
    /// view's isolation aside: what a file's preview draws.
    pub fn imported_body_saved_visible(&self, body: BodyId) -> bool {
        if self.body(body).is_some_and(|b| b.hidden) || self.left_out.contains(&body) {
            return false;
        }
        match self.imported_body_to_object.get(&body).copied() {
            Some(id) => self.imported_object_effective_visible(id),
            None => true,
        }
    }

    pub(crate) fn rebuild_imported_body_index(&mut self) {
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
