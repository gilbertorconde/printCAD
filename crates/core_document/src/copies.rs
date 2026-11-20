//! Linked copies of bodies and parts linked from other files.

use std::path::{Path, PathBuf};

use super::*;

/// Where a linked part's shape comes from: a body of another printCAD
/// file, as that file stood when it was last read (`stamp`, its modified
/// time in seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileLink {
    /// The file's absolute path.
    pub path: String,
    pub body: BodyId,
    pub stamp: u64,
    /// The file's path from the folder of the document holding the link,
    /// `/`-separated; tried before `path`, so a project folder moved whole
    /// keeps its links. Written whenever the document has a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<String>,
}

impl FileLink {
    /// A link to `body` of the file at `path`, read at `stamp`, from a
    /// document in `folder` (none while it has no file).
    pub fn new(path: &Path, body: BodyId, stamp: u64, folder: Option<&Path>) -> Self {
        let path = normal(path);
        Self {
            relative: folder.and_then(|f| relative_path(&path, f)),
            path: path.to_string_lossy().into_owned(),
            body,
            stamp,
        }
    }

    /// Where the linked file is for a document in `folder`: beside it by
    /// the relative path, else at the absolute one; `None` when neither
    /// holds a file.
    pub fn resolve(&self, folder: Option<&Path>) -> Option<PathBuf> {
        let beside = self
            .relative
            .as_deref()
            .zip(folder)
            .map(|(relative, folder)| normal(&folder.join(relative)));
        beside
            .into_iter()
            .chain(std::iter::once(PathBuf::from(&self.path)))
            .find(|p| p.is_file())
    }

    /// The path to read for a document in `folder`: where the file is
    /// found, else the absolute path it was linked from.
    pub fn located(&self, folder: Option<&Path>) -> PathBuf {
        self.resolve(folder)
            .unwrap_or_else(|| PathBuf::from(&self.path))
    }

    /// This link as a document in `from` writes it into a file in `to`:
    /// the file where it is found now, by its absolute path and from `to`.
    pub fn for_folder(&self, from: Option<&Path>, to: &Path) -> Self {
        Self::new(&self.located(from), self.body, self.stamp, Some(to))
    }
}

/// `path` with `.` and `..` taken out, without asking the file system.
fn normal(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `target` from `folder`, both absolute, `/`-separated; `None` when they
/// share no root (another drive).
fn relative_path(target: &Path, folder: &Path) -> Option<String> {
    use std::path::Component;
    let folder = normal(folder);
    let target: Vec<Component> = target.components().collect();
    let folder: Vec<Component> = folder.components().collect();
    fn root<'a>(c: &'a [Component<'a>]) -> &'a [Component<'a>] {
        let n = c
            .iter()
            .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
            .count();
        &c[..n]
    }
    if root(&target).is_empty() || root(&target) != root(&folder) {
        return None;
    }
    let shared = target
        .iter()
        .zip(&folder)
        .take_while(|(a, b)| a == b)
        .count();
    let up = std::iter::repeat_n("..".to_string(), folder.len() - shared);
    let down = target[shared..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    let parts: Vec<String> = up.chain(down).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
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

    /// Link a part to another file, or to its own file found elsewhere:
    /// one edit, its shape read again from `link`. False when `body` is not
    /// a linked part.
    pub fn relink(&mut self, body: BodyId, link: FileLink) -> bool {
        if self.body(body).is_none_or(|b| b.link.is_none()) {
            return false;
        }
        self.link_missing.remove(&body);
        self.record_and_apply(op::DocumentOp::SetBodyLink { id: body, link });
        true
    }

    /// Write every link for a file in `to`, the document having been in
    /// `from`: the file each finds now, by its absolute path and from `to`.
    /// For the copy a save writes; the links mean the same files, so it is
    /// no edit and records nothing.
    pub fn place_links_for_file(&mut self, from: Option<&Path>, to: &Path) {
        for body in &mut self.bodies {
            if let Some(link) = &body.link {
                body.link = Some(link.for_folder(from, to));
            }
        }
    }

    /// Mark a linked part whose file is found nowhere. Derived state: no op.
    pub fn set_link_missing(&mut self, body: BodyId, missing: bool) {
        if missing {
            self.link_missing.insert(body);
        } else {
            self.link_missing.remove(&body);
        }
    }

    /// Whether a linked part's file is found nowhere, as last looked.
    pub fn link_missing(&self, body: BodyId) -> bool {
        self.link_missing.contains(&body)
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
            .max(self.derived.next_geometry_revision);
        self.derived.next_geometry_revision = next_revision.saturating_add(1);
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
