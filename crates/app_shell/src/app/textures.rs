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
const PICTURE_PX: u32 = 1024;

/// Pictures read so far, by asset; `None` for one that would not read.
pub(crate) type Pictures = HashMap<Uuid, Option<Arc<HeightMap>>>;

/// One texture with its faces and heights.
#[derive(Debug, Clone)]
struct Pressed {
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
pub(crate) fn textures_of(
    document: &Document,
    body: BodyId,
) -> Option<(&[FaceTexture], Option<MirrorPlane>)> {
    let entry = document.bodies().iter().find(|b| b.id == body)?;
    if !entry.textures.is_empty() {
        return Some((&entry.textures, None));
    }
    let source = entry.copy_of?;
    let source = document.bodies().iter().find(|b| b.id == source)?;
    (!source.textures.is_empty()).then_some((source.textures.as_slice(), entry.mirror))
}

/// Every texture of a body, ready to press into a mesh of it.
#[derive(Debug, Clone)]
pub(crate) struct Pressing {
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
    pub(crate) fn of(document: &Document, body: BodyId, pictures: &mut Pictures) -> Option<Self> {
        let (textures, mirror) = textures_of(document, body)?;
        let mesh = &document.imported_geometry(body)?.mesh;
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
                                    crate::log_panel::warn(format!(
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
    pub(crate) fn apply(&self, mesh: &TriMesh, detail: Detail) -> TriMesh {
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
pub(crate) struct Preview {
    /// The key of what is wanted now.
    pub wanted: u64,
    /// The last one made, with its key.
    pub made: Option<(u64, Arc<TriMesh>)>,
}

/// The pressed meshes previews are drawn with, by what they were pressed
/// from: a source and the copies that share its mesh and textures want
/// the same key, so they are pressed once and drawn from one mesh.
#[derive(Default)]
pub(crate) struct PressedMeshes {
    /// Keys being made on a thread.
    making: std::collections::HashSet<u64>,
    /// Keys made, with their mesh.
    made: HashMap<u64, Arc<TriMesh>>,
    tx: Option<std::sync::mpsc::Sender<Made>>,
    rx: Option<std::sync::mpsc::Receiver<Made>>,
}

/// A preview made on its thread.
pub(crate) struct Made {
    key: u64,
    mesh: Arc<TriMesh>,
}

/// What `body`'s preview is pressed from, with its textures `pressing`:
/// its own mesh, named by the body it shares that mesh with (an unmirrored
/// copy shares its source's) and that body's geometry revision, which the
/// document never gives twice.
fn preview_key(document: &Document, body: BodyId, pressing: &Pressing) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let (local, _) = document.local_geometry(body)?;
    let owner = document
        .copy_source(body)
        .filter(|source| {
            document
                .local_geometry(*source)
                .is_some_and(|(theirs, _)| Arc::ptr_eq(&local, &theirs))
        })
        .unwrap_or(body);
    let revision = document.imported_geometry(owner)?.revision;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (pressing.key, owner, revision).hash(&mut hasher);
    Some(hasher.finish())
}

impl crate::PrintCadApp {
    /// Keep each visible textured body's preview up to its textures and
    /// its shape: take in what the threads made, and start one for each
    /// key wanted that is neither made nor being made.
    pub(crate) fn drive_texture_previews(&mut self) {
        let pressed = &mut self.session.texture_meshes;
        if let Some(rx) = &pressed.rx {
            while let Ok(made) = rx.try_recv() {
                pressed.making.remove(&made.key);
                pressed.made.insert(made.key, made.mesh);
            }
        }
        let document = &self.session.document;
        let bodies: Vec<BodyId> = document
            .bodies()
            .iter()
            .filter(|b| {
                document.imported_body_effective_visible(b.id)
                    && textures_of(document, b.id).is_some()
            })
            .map(|b| b.id)
            .collect();
        self.session
            .textured
            .retain(|body, _| bodies.contains(body));
        for body in bodies {
            let document = &self.session.document;
            let Some(pressing) = Pressing::of(document, body, &mut self.session.texture_pictures)
            else {
                continue;
            };
            let Some(key) = preview_key(document, body, &pressing) else {
                continue;
            };
            let pressed = &mut self.session.texture_meshes;
            let preview = self.session.textured.entry(body).or_default();
            preview.wanted = key;
            if let Some(mesh) = pressed.made.get(&key) {
                preview.made = Some((key, Arc::clone(mesh)));
                continue;
            }
            if pressed.making.contains(&key) {
                continue;
            }
            let Some((local, _)) = document.local_geometry(body) else {
                continue;
            };
            pressed.making.insert(key);
            let tx = pressed
                .tx
                .get_or_insert_with(|| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    pressed.rx = Some(rx);
                    tx
                })
                .clone();
            let spawned = crate::platform::spawn("printcad-texture", move || {
                let mesh = Arc::new(pressing.apply(&local, Detail::PREVIEW));
                let _ = tx.send(Made { key, mesh });
            });
            if let Err(err) = spawned {
                crate::log_panel::warn(format!("The texture preview could not start: {err}"));
                self.session.texture_meshes.making.remove(&key);
            }
        }
        // A mesh no body wants or draws any more is let go.
        let kept: std::collections::HashSet<u64> =
            self.session.textured.values().map(|p| p.wanted).collect();
        self.session
            .texture_meshes
            .made
            .retain(|key, _| kept.contains(key));
    }

    /// Whether a texture preview is being made: frames keep coming until
    /// it lands.
    pub(crate) fn texture_previews_pending(&self) -> bool {
        self.session.texture_meshes.pending()
    }
}

impl PressedMeshes {
    /// Whether one is being made.
    pub(crate) fn pending(&self) -> bool {
        !self.making.is_empty()
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
        doc.set_imported_geometry(body, crate::test_support::geometry(mesh));
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
