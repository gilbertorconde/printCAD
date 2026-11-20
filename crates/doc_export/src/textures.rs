//! A body's surface textures, ready to press into a mesh of it: each with
//! the faces it covers, found in the body's mesh as it stands, and a
//! picture's heights, read from its document asset. A linked copy without
//! textures of its own is pressed with its source's, mirrored with it when
//! it is a mirror image.

use std::collections::HashMap;
use std::sync::Arc;

use core_document::{BodyId, Document, FaceTexture, MirrorPlane};
use kernel_api::TriMesh;
use surface_texture::{Detail, HeightMap, Job, Pattern, Texture};
use uuid::Uuid;

/// The widest a picture's heights are read, pixels.
pub const PICTURE_PX: u32 = 1024;

/// Pictures read so far, by asset; `None` for one that would not read.
pub type Pictures = HashMap<Uuid, Option<Arc<HeightMap>>>;

/// One texture with its faces and heights.
#[derive(Debug, Clone)]
pub struct Pressed {
    texture: Texture,
    faces: Vec<u32>,
    image: Option<Arc<HeightMap>>,
}

/// The textures `body` is pressed with: its own when it has any, else,
/// for a linked copy, its source's, with the plane (in the source's frame)
/// the copy mirrors them across when it is a mirror image. `None` when
/// there are none.
///
/// A copy's own textures decide: they are a choice made on that body, as
/// its own face colours are, and a copy without any looks like its source.
pub fn textures_of(
    document: &Document,
    body: BodyId,
) -> Option<(&[FaceTexture], Option<MirrorPlane>)> {
    let entry = document.body(body)?;
    if !entry.textures.is_empty() {
        return Some((&entry.textures, None));
    }
    let source = entry.copy_of?;
    let source = document.body(source)?;
    (!source.textures.is_empty()).then_some((source.textures.as_slice(), entry.mirror))
}

/// Every texture of a body, ready to press into a mesh of it.
#[derive(Debug, Clone)]
pub struct Pressing {
    textures: Vec<Pressed>,
    /// The plane a mirrored copy's inherited textures are mirrored across:
    /// they are pressed into its mesh mirrored back to the source's shape,
    /// and the result mirrored again.
    mirror: Option<MirrorPlane>,
    /// What they are, for knowing a preview still stands for them.
    pub key: u64,
}

impl Pressing {
    /// The textures `body` is pressed with (`textures_of`), their faces
    /// found in its mesh; `None` when there are none. A copy's mesh keeps
    /// its source's faces and their names, mirrored or not, so the
    /// source's textures find them there.
    pub fn of(document: &Document, body: BodyId, pictures: &mut Pictures) -> Option<Self> {
        let (textures, mirror) = textures_of(document, body)?;
        let mesh = &document.derived().imported_geometry(body)?.mesh;
        let textures: Vec<Pressed> = textures
            .iter()
            .map(|t| Pressed {
                texture: t.texture,
                faces: t.face_indices(mesh),
                image: match t.texture.pattern {
                    Pattern::Image { asset } => pictures
                        .entry(asset)
                        .or_insert_with(|| {
                            let bytes = document.asset_bytes(asset)?;
                            // Read once, so a picture that does not read is
                            // said once and its texture lies flat.
                            match HeightMap::from_image(bytes, PICTURE_PX) {
                                Ok(map) => Some(Arc::new(map)),
                                Err(err) => {
                                    app_log::warn(format!(
                                        "A texture's picture does not read, so it presses \
                                         nothing: {err}"
                                    ));
                                    None
                                }
                            }
                        })
                        .clone(),
                    _ => None,
                },
            })
            .collect();
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for t in &textures {
                format!("{:?}", t.texture).hash(&mut hasher);
                t.faces.hash(&mut hasher);
                t.image
                    .as_ref()
                    .map(|i| (i.width, i.height))
                    .hash(&mut hasher);
            }
            format!("{mirror:?}").hash(&mut hasher);
            hasher.finish()
        };
        Some(Self {
            textures,
            mirror,
            key,
        })
    }

    /// `mesh` with the textures pressed in, as fine as `detail` asks; for
    /// a mirrored copy, the mirror image of what its source's mesh gets.
    pub fn apply(&self, mesh: &TriMesh, detail: Detail) -> TriMesh {
        match &self.mirror {
            Some(plane) => plane.mesh(&self.press(&plane.mesh(mesh), detail)),
            None => self.press(mesh, detail),
        }
    }

    fn press(&self, mesh: &TriMesh, detail: Detail) -> TriMesh {
        let jobs: Vec<Job<'_>> = self
            .textures
            .iter()
            .map(|t| Job {
                texture: &t.texture,
                faces: &t.faces,
                image: t.image.as_deref(),
            })
            .collect();
        surface_texture::apply(mesh, &jobs, detail)
    }
}

/// A textured body as drawn: the textures pressed into its own mesh, in
/// its own frame, drawn where it sits. While a newer one is being made,
/// the last stays on screen.
#[derive(Debug, Clone, Default)]
pub struct Preview {
    /// The key of what is wanted now.
    pub wanted: u64,
    /// The last one made, with its key.
    pub made: Option<(u64, Arc<TriMesh>)>,
}

/// The pressed meshes previews are drawn with, by what they were pressed
/// from: a source and the copies that share its mesh and textures want
/// the same key, so they are pressed once and drawn from one mesh.
#[derive(Default)]
pub struct PressedMeshes {
    /// Keys being made on a thread, with where each one's mesh comes.
    pub making: HashMap<u64, Option<std::sync::mpsc::Receiver<Arc<TriMesh>>>>,
    /// Keys made, with their mesh.
    pub made: HashMap<u64, Arc<TriMesh>>,
    /// Keys whose thread ended without a mesh: not started again.
    pub failed: std::collections::HashSet<u64>,
}

/// What `body`'s preview is pressed from, with its textures `pressing`:
/// its own mesh, named by the body it shares that mesh with (an unmirrored
/// copy shares its source's) and that body's geometry revision, which the
/// document never gives twice.
pub fn preview_key(document: &Document, body: BodyId, pressing: &Pressing) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let (local, _) = document.derived().local_geometry(body)?;
    let owner = document
        .copy_source(body)
        .filter(|source| {
            document
                .derived()
                .local_geometry(*source)
                .is_some_and(|(theirs, _)| Arc::ptr_eq(&local, &theirs))
        })
        .unwrap_or(body);
    let revision = document.derived().imported_geometry(owner)?.revision;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (pressing.key, owner, revision).hash(&mut hasher);
    Some(hasher.finish())
}

impl PressedMeshes {
    /// Whether one is being made.
    pub fn pending(&self) -> bool {
        !self.making.is_empty()
    }

    /// Take in the meshes the threads made. A thread that ended without
    /// one (it panicked) is said in the log and its key kept as failed,
    /// so it neither keeps frames coming nor starts again.
    pub fn take_in(&mut self) {
        for (key, answer) in &mut self.making {
            match app_platform::take_answer(answer) {
                Some(Ok(mesh)) => {
                    self.made.insert(*key, mesh);
                }
                Some(Err(_)) => {
                    app_log::warn("A texture preview stopped without a mesh.");
                    self.failed.insert(*key);
                }
                None => {}
            }
        }
        self.making.retain(|_, answer| answer.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body with one triangle, textured all over, and two copies of it:
    /// one moved, one mirrored.
    fn textured_with_copies() -> (Document, BodyId, BodyId, BodyId) {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mesh = TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            faces: vec![0],
            ..TriMesh::default()
        };
        doc.derived_mut()
            .set_imported_geometry(body, crate::test_support::geometry(mesh));
        doc.set_body_textures(
            body,
            vec![FaceTexture {
                texture: Texture::default(),
                faces: Vec::new(),
            }],
        );
        let moved = doc.create_linked_copy(body, None).unwrap();
        doc.set_body_placement(
            moved,
            core_document::BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::X * 30.0),
        );
        let plane = MirrorPlane {
            point: [20.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        let mirrored = doc.create_mirrored_copy(body, plane, None).unwrap();
        (doc, body, moved, mirrored)
    }

    fn key(doc: &Document, body: BodyId) -> u64 {
        let pressing = Pressing::of(doc, body, &mut Pictures::new()).unwrap();
        preview_key(doc, body, &pressing).unwrap()
    }

    /// A copy takes its source's textures and shares its pressed mesh
    /// wherever it sits; a mirrored copy has one of its own.
    #[test]
    fn a_copy_shares_its_source_textured_preview() {
        let (doc, body, moved, mirrored) = textured_with_copies();
        assert!(textures_of(&doc, moved).is_some_and(|(_, mirror)| mirror.is_none()));
        assert!(textures_of(&doc, mirrored).is_some_and(|(_, mirror)| mirror.is_some()));
        assert_eq!(key(&doc, moved), key(&doc, body));
        assert_ne!(key(&doc, mirrored), key(&doc, body));
    }

    /// A preview whose thread ends without a mesh is let go and kept as
    /// failed, so nothing is left pending.
    #[test]
    fn a_preview_that_dies_is_let_go() {
        let mut pressed = PressedMeshes::default();
        let (tx, rx) = std::sync::mpsc::channel();
        pressed.making.insert(5, Some(rx));
        pressed.take_in();
        assert!(pressed.pending());
        drop(tx);
        pressed.take_in();
        assert!(!pressed.pending() && pressed.failed.contains(&5));
    }

    /// A copy's own textures decide over its source's.
    #[test]
    fn a_copy_s_own_textures_decide() {
        let (mut doc, body, moved, _) = textured_with_copies();
        let own = vec![FaceTexture {
            texture: Texture {
                depth_mm: 1.5,
                ..Texture::default()
            },
            faces: Vec::new(),
        }];
        doc.set_body_textures(moved, own.clone());
        let (textures, mirror) = textures_of(&doc, moved).unwrap();
        assert_eq!(textures, own.as_slice());
        assert!(mirror.is_none());
        assert_ne!(key(&doc, moved), key(&doc, body));
    }
}
