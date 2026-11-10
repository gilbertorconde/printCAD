//! Linked copies of bodies and parts linked from other files.

use super::*;

/// Where a linked part's shape comes from: a body of another printCAD
/// file, as that file stood when it was last read (`stamp`, its modified
/// time in seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileLink {
    pub path: String,
    pub body: BodyId,
    pub stamp: u64,
}

impl Document {
    /// A linked copy of `source` (of its source, when it is a copy itself):
    /// a body of the same shape, which follows every change to it, placed
    /// where `source` sits until it is moved.
    pub fn create_linked_copy(&mut self, source: BodyId, name: Option<String>) -> Option<BodyId> {
        self.create_copy(source, name, None)
    }

    /// A linked copy of `source` that is its mirror image across the plane
    /// `mirror` (in the world), placed where `source` sits. Its snapshot
    /// is the kernel's mirror of the source's, which the host derives
    /// (`copies_awaiting_shape`). `None` for a mirror of a mirror.
    pub fn create_mirrored_copy(
        &mut self,
        source: BodyId,
        mirror: MirrorPlane,
        name: Option<String>,
    ) -> Option<BodyId> {
        let entry = self.body(source)?;
        let to_local = entry.placement.inverse();
        let local = MirrorPlane {
            point: to_local.point(mirror.point),
            normal: to_local.direction(mirror.normal),
        };
        self.create_copy(source, name, Some(local))
    }

    fn create_copy(
        &mut self,
        source: BodyId,
        name: Option<String>,
        mirror: Option<MirrorPlane>,
    ) -> Option<BodyId> {
        let entry = self.body(source)?;
        if entry.mirror.is_some() && mirror.is_some() {
            return None;
        }
        // A copy of a mirrored copy is mirrored alike.
        let mirror = mirror.or(entry.mirror);
        let root = entry.copy_of.unwrap_or(source);
        let placement = entry.placement;
        let base = self.body(root).map(|b| b.name.clone())?;
        let id = BodyId::new();
        let name = match name {
            Some(explicit) => explicit,
            None => next_indexed_name(&base, self.bodies.iter().map(|b| b.name.as_str())),
        };
        self.record_and_apply(op::DocumentOp::CreateLinkedCopy {
            id,
            name,
            created_at: epoch_ms_now(),
            source: root,
            mirror,
        });
        self.set_body_placement(id, placement);
        Some(id)
    }

    /// A part linked from `link`'s file, placed at the origin.
    pub fn create_linked_body(&mut self, name: String, link: FileLink) -> BodyId {
        let id = BodyId::new();
        self.record_and_apply(op::DocumentOp::CreateLinkedBody {
            id,
            name,
            created_at: epoch_ms_now(),
            link,
        });
        id
    }

    /// Read a linked part again from its file, now at `stamp`.
    pub fn reload_link(&mut self, body: BodyId, stamp: u64) -> bool {
        let Some(link) = self.body(body).and_then(|b| b.link.clone()) else {
            return false;
        };
        self.record_and_apply(op::DocumentOp::SetBodyLink {
            id: body,
            link: FileLink { stamp, ..link },
        });
        true
    }

    /// Linked parts with no shape yet: each with its link.
    pub fn links_awaiting_geometry(&self) -> Vec<(BodyId, FileLink)> {
        self.bodies
            .iter()
            .filter(|b| !self.imported_meshes.contains_key(&b.id))
            .filter_map(|b| Some((b.id, b.link.clone()?)))
            .collect()
    }

    /// Give a linked part its shape as read from its file: the mesh in its
    /// own frame, the snapshot and its face colours. Derived state: no op.
    pub fn set_linked_geometry(
        &mut self,
        body: BodyId,
        geometry: ImportedGeometry,
        blob: Option<std::sync::Arc<Vec<u8>>>,
        colors: Option<Vec<[f32; 3]>>,
    ) {
        match blob {
            Some(blob) => {
                self.imported_brep_blobs.insert(body, blob);
            }
            None => {
                self.imported_brep_blobs.remove(&body);
            }
        }
        match colors {
            Some(colors) => {
                self.imported_brep_face_colors.insert(body, colors);
            }
            None => {
                self.imported_brep_face_colors.remove(&body);
            }
        }
        let dirty = self.metadata.dirty;
        self.set_imported_geometry(body, geometry);
        // Reading a link is not an edit.
        self.metadata.dirty = dirty;
    }

    /// Mark a linked part whose file changed since it was read. Derived
    /// state: no op.
    pub fn set_link_stale(&mut self, body: BodyId, stale: bool) {
        if stale {
            self.link_stale.insert(body);
        } else {
            self.link_stale.remove(&body);
        }
    }

    /// Whether a linked part's file changed since it was read.
    pub fn link_stale(&self, body: BodyId) -> bool {
        self.link_stale.contains(&body)
    }

    /// The body whose shape `body` takes, when it is a linked copy.
    pub fn copy_source(&self, body: BodyId) -> Option<BodyId> {
        self.body(body)?.copy_of
    }

    /// The linked copies of `body`.
    pub fn copies_of(&self, body: BodyId) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| b.copy_of == Some(body))
            .map(|b| b.id)
            .collect()
    }

    /// A linked copy's geometry, derived again from its source's: the same
    /// shape, placed where the copy sits. Derived state: no op.
    pub(crate) fn refresh_copy(&mut self, copy: BodyId) {
        let Some(source) = self.copy_source(copy) else {
            return;
        };
        let mirror = self.body(copy).and_then(|b| b.mirror);
        let Some((mesh, bounds)) = self.local_geometry(source) else {
            self.imported_meshes.remove(&copy);
            self.local_meshes.remove(&copy);
            self.imported_brep_blobs.remove(&copy);
            self.imported_brep_face_colors.remove(&copy);
            return;
        };
        let (mesh, bounds) = match mirror {
            Some(plane) => {
                let mirrored = plane.mesh(&mesh);
                let bounds = mirrored.bounds();
                (Arc::new(mirrored), bounds)
            }
            None => (mesh, bounds),
        };
        let from = self.imported_meshes.get(&source);
        let geometry = ImportedGeometry {
            mesh,
            source_asset: from.and_then(|g| g.source_asset),
            revision: 0,
            bounds_mm: bounds,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: from.and_then(|g| g.health.clone()),
        };
        // A mirror's snapshot is the kernel's, derived by the host; until
        // then it has none.
        match self.imported_brep_blobs.get(&source).cloned() {
            Some(blob) if mirror.is_none() => {
                self.imported_brep_blobs.insert(copy, blob);
            }
            _ => {
                self.imported_brep_blobs.remove(&copy);
            }
        }
        match self.imported_brep_face_colors.get(&source).cloned() {
            Some(colors) => {
                self.imported_brep_face_colors.insert(copy, colors);
            }
            None => {
                self.imported_brep_face_colors.remove(&copy);
            }
        }
        let next_revision = self
            .imported_meshes
            .get(&copy)
            .map(|prev| prev.revision.saturating_add(1))
            .unwrap_or(0)
            .max(self.next_geometry_revision);
        self.next_geometry_revision = next_revision.saturating_add(1);
        self.local_meshes.remove(&copy);
        self.imported_meshes.insert(
            copy,
            ImportedGeometry {
                revision: next_revision,
                ..geometry
            },
        );
        if !self.body_placement(copy).is_identity() {
            self.place_geometry(copy);
        }
    }

    /// Mirrored copies whose snapshot is still to be derived: each with its
    /// source's snapshot and the plane, in the source's frame, to mirror
    /// it across.
    pub fn copies_awaiting_shape(&self) -> Vec<(BodyId, std::sync::Arc<Vec<u8>>, MirrorPlane)> {
        self.bodies
            .iter()
            .filter_map(|b| {
                let (source, plane) = (b.copy_of?, b.mirror?);
                if self.imported_brep_blobs.contains_key(&b.id) {
                    return None;
                }
                Some((b.id, self.imported_brep_blobs.get(&source)?.clone(), plane))
            })
            .collect()
    }

    /// A mirrored copy's snapshot, as the kernel made it from `from` (the
    /// source's snapshot then): kept when the source still has that one.
    /// Derived state: no op.
    pub fn set_mirrored_shape(
        &mut self,
        copy: BodyId,
        from: &std::sync::Arc<Vec<u8>>,
        blob: Vec<u8>,
    ) -> bool {
        let Some(source) = self.copy_source(copy) else {
            return false;
        };
        let current = self.imported_brep_blobs.get(&source);
        if !current.is_some_and(|c| std::sync::Arc::ptr_eq(c, from)) {
            return false;
        }
        let colors = self.imported_brep_face_colors.get(&source).cloned();
        self.imported_brep_blobs
            .insert(copy, std::sync::Arc::new(blob));
        match colors {
            Some(colors) => {
                self.imported_brep_face_colors.insert(copy, colors);
            }
            None => {
                self.imported_brep_face_colors.remove(&copy);
            }
        }
        true
    }

    /// Every linked copy of `body` derived again from it.
    pub(crate) fn refresh_copies_of(&mut self, body: BodyId) {
        for copy in self.copies_of(body) {
            self.refresh_copy(copy);
        }
    }
}
