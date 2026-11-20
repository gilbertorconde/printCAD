//! Surface textures in the running app: the pressed preview kept up to
//! the body while its task is open (`doc_export::textures` holds the
//! pressing).

use std::sync::Arc;

use core_document::BodyId;
use doc_export::textures::*;
use surface_texture::Detail;

impl crate::PrintCadApp {
    /// Keep each visible textured body's preview up to its textures and
    /// its shape: take in what the threads made, and start one for each
    /// key wanted that is neither made nor being made.
    pub(crate) fn drive_texture_previews(&mut self) {
        self.session.drawn.texture_meshes.take_in();
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
            .drawn
            .textured
            .retain(|body, _| bodies.contains(body));
        for body in bodies {
            let document = &self.session.document;
            let Some(pressing) =
                Pressing::of(document, body, &mut self.session.drawn.texture_pictures)
            else {
                continue;
            };
            let Some(key) = preview_key(document, body, &pressing) else {
                continue;
            };
            let pressed = &mut self.session.drawn.texture_meshes;
            let preview = self.session.drawn.textured.entry(body).or_default();
            preview.wanted = key;
            if let Some(mesh) = pressed.made.get(&key) {
                preview.made = Some((key, Arc::clone(mesh)));
                continue;
            }
            if pressed.making.contains_key(&key) || pressed.failed.contains(&key) {
                continue;
            }
            let Some((local, _)) = document.derived().local_geometry(body) else {
                continue;
            };
            let (tx, rx) = std::sync::mpsc::channel();
            pressed.making.insert(key, Some(rx));
            let spawned = app_platform::spawn("printcad-texture", move || {
                let _ = tx.send(Arc::new(pressing.apply(&local, Detail::PREVIEW)));
            });
            if let Err(err) = spawned {
                app_log::warn(format!("The texture preview could not start: {err}"));
                self.session.drawn.texture_meshes.making.remove(&key);
            }
        }
        // A mesh no body wants or draws any more is let go.
        let kept: std::collections::HashSet<u64> = self
            .session
            .drawn
            .textured
            .values()
            .map(|p| p.wanted)
            .collect();
        let pressed = &mut self.session.drawn.texture_meshes;
        pressed.made.retain(|key, _| kept.contains(key));
        pressed.failed.retain(|key| kept.contains(key));
    }

    /// Whether a texture preview is being made: frames keep coming until
    /// it lands.
    pub(crate) fn texture_previews_pending(&self) -> bool {
        self.session.drawn.texture_meshes.pending()
    }
}
