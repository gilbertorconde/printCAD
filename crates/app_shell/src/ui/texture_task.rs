//! The Surface texture task: the patterns pressed into a body's faces for
//! printing. Faces clicked in the view join the texture being edited; the
//! pattern comes from a gallery or a picture; edits show in the view as
//! they are made, OK keeps them as one undo step, Cancel puts back the
//! textures the task opened on.

use core_document::{BodyId, Document, FaceKey, FaceTexture, Recorded};
use egui::{RichText, Ui, Vec2};
use serde_json::json;
use surface_texture::{Pattern, Projection, Texture};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    QtyField, check_row, secondary_button, select_field, small_secondary_button,
};

use super::UiCommand;
use super::host_tasks::HostTaskEnd;

/// The side of a pattern's picture in the gallery, points.
const THUMB: f32 = 52.0;

#[derive(Debug, Clone)]
pub struct TextureTask {
    body: BodyId,
    /// The body's textures when the task opened, for Cancel.
    start: Vec<FaceTexture>,
    /// Which of the body's textures is being edited.
    current: usize,
    /// The faces last picked in the view, so each pick counts once.
    last_picks: Vec<u32>,
    /// The face the task was opened on, which a body with no texture
    /// gets its first on; taken on the first frame.
    opened_on: Option<Option<u32>>,
    /// A linked copy with no textures of its own shows its source's: the
    /// task edits from them, and they become the copy's own only once
    /// they differ.
    inherited: Vec<FaceTexture>,
}

impl TextureTask {
    /// A task on `body`'s textures; a body with none gets one to edit on
    /// its first frame, on the face the menu was opened on or else the
    /// whole body. A copy showing its source's textures edits from
    /// them instead.
    pub fn open(document: &Document, body: BodyId, face: Option<u32>) -> Self {
        let start = textures_of(document, body);
        let inherited = match crate::app::textures::textures_of(document, body) {
            Some((textures, _)) if start.is_empty() => textures.to_vec(),
            _ => Vec::new(),
        };
        Self {
            body,
            start,
            current: 0,
            last_picks: face.into_iter().collect(),
            opened_on: Some(face),
            inherited,
        }
    }

    /// `textures` as the body keeps them: none while they are the ones a
    /// copy shows of its source, so it goes on following the source.
    fn kept(&self, textures: Vec<FaceTexture>) -> Vec<FaceTexture> {
        if !self.inherited.is_empty() && textures == self.inherited {
            Vec::new()
        } else {
            textures
        }
    }

    pub fn body(&self) -> BodyId {
        self.body
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        picked_faces: &[(BodyId, u32)],
        accept: bool,
        cancel: bool,
        commands: &mut Vec<UiCommand>,
    ) -> Option<HostTaskEnd> {
        let body = self.body;
        if cancel {
            document.set_body_textures(body, self.start.clone());
            return Some(HostTaskEnd::Cancelled);
        }
        let mut textures = textures_of(document, body);
        if textures.is_empty() {
            textures = self.inherited.clone();
        }
        if let Some(face) = self.opened_on.take()
            && textures.is_empty()
        {
            let faces = match (face, document.imported_geometry(body)) {
                (Some(face), Some(geometry)) => vec![FaceKey::of(&geometry.mesh, face)],
                _ => Vec::new(),
            };
            textures.push(FaceTexture {
                texture: Texture::default(),
                faces,
            });
        }
        if accept {
            let textures = self.kept(textures);
            let calls = if textures == self.start {
                Vec::new()
            } else {
                vec![set_textures_call(body, &textures)]
            };
            return Some(HostTaskEnd::Accepted("Surface texture".into(), calls));
        }
        let mesh = document
            .imported_geometry(body)
            .map(|g| std::sync::Arc::clone(&g.mesh));

        // A label for each of the body's textures, picking the one edited,
        // and Add for another.
        ui.horizontal_wrapped(|ui| {
            for n in 0..textures.len() {
                if ui
                    .selectable_label(n == self.current, format!("Texture {}", n + 1))
                    .clicked()
                {
                    self.current = n;
                }
            }
            if small_secondary_button(ui, "Add")
                .on_hover_text("Another texture, on other faces")
                .clicked()
            {
                textures.push(FaceTexture {
                    texture: Texture::default(),
                    faces: Vec::new(),
                });
                self.current = textures.len() - 1;
                self.last_picks.clear();
            }
        });
        if !self.inherited.is_empty() {
            note(
                ui,
                "A copy without textures of its own shows its original's. Changing them here \
                 gives this copy its own.",
            );
        }
        if textures.is_empty() {
            note(
                ui,
                "No texture. Add one to press a pattern into the body's faces.",
            );
            document.set_body_textures(body, textures);
            return None;
        }
        self.current = self.current.min(textures.len() - 1);
        let mut remove = false;
        {
            let edited = &mut textures[self.current];

            // The faces picked in the view join the texture, each once.
            heading(ui, "Faces");
            let picks: Vec<u32> = picked_faces
                .iter()
                .filter(|(b, _)| *b == body)
                .map(|(_, index)| *index)
                .collect();
            if !picks.is_empty() && picks != self.last_picks {
                if let Some(mesh) = &mesh {
                    for &index in picks.iter().filter(|i| !self.last_picks.contains(i)) {
                        if !edited.faces.iter().any(|k| k.is_face(mesh, index)) {
                            edited.faces.push(FaceKey::of(mesh, index));
                        }
                    }
                }
                self.last_picks = picks;
            }
            if edited.faces.is_empty() {
                note(
                    ui,
                    "The whole body. Click faces in the view to texture only them.",
                );
            } else {
                let mut drop = None;
                ui.horizontal_wrapped(|ui| {
                    for (n, key) in edited.faces.iter().enumerate() {
                        if small_secondary_button(ui, &format!("Face {}  ×", key.index + 1))
                            .on_hover_text("Leave this face smooth")
                            .clicked()
                        {
                            drop = Some(n);
                        }
                    }
                });
                if let Some(n) = drop {
                    edited.faces.remove(n);
                }
                if secondary_button(ui, "Whole body").clicked() {
                    edited.faces.clear();
                    self.last_picks.clear();
                }
            }

            heading(ui, "Pattern");
            if let Some(pattern) = gallery(ui, edited.texture.pattern) {
                edited.texture.pattern = pattern;
            }
            ui.horizontal(|ui| {
                if secondary_button(ui, "Picture…")
                    .on_hover_text("A greyscale PNG or JPEG, white high")
                    .clicked()
                {
                    commands.push(UiCommand::PickTexturePicture {
                        body,
                        index: self.current,
                    });
                }
                if matches!(edited.texture.pattern, Pattern::Image { .. }) {
                    ui.label(
                        RichText::new("A picture is in use")
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                    );
                }
            });

            heading(ui, "How it lies");
            let t = &mut edited.texture;
            egui::Grid::new("texture_grid")
                .num_columns(2)
                .spacing([SPACE_2, SPACE_1])
                .show(ui, |ui| {
                    ui_kit::widgets::field_label(ui, "Projection");
                    let options: Vec<(Projection, String)> = Projection::all()
                        .into_iter()
                        .map(|p| (p, p.label()))
                        .collect();
                    let options: Vec<(Projection, &str)> =
                        options.iter().map(|(p, l)| (*p, l.as_str())).collect();
                    select_field(ui, "texture_projection", &mut t.projection, &options, 170.0);
                    ui.end_row();
                    ui_kit::widgets::field_label(ui, "Tile");
                    QtyField::mm(&mut t.tile_mm).range(0.2..=500.0).show(ui);
                    ui.end_row();
                    ui_kit::widgets::field_label(ui, "Turn");
                    QtyField::degrees(&mut t.rotation_deg).show(ui);
                    ui.end_row();
                    ui_kit::widgets::field_label(ui, "Depth");
                    QtyField::mm(&mut t.depth_mm).range(0.01..=20.0).show(ui);
                    ui.end_row();
                    ui_kit::widgets::field_label(ui, "Keep flat");
                    QtyField::degrees(&mut t.keep_flat_deg)
                        .range(0.0..=89.0)
                        .show(ui);
                    ui.end_row();
                });
            check_row(ui, &mut t.inward, "Into the surface");
            note(
                ui,
                "Keep flat leaves faces within that angle of facing up or down smooth: a print's \
                 top and the side it stands on. 0 textures them too.",
            );
            ui.add_space(SPACE_2);
            if secondary_button(ui, "Remove this texture").clicked() {
                remove = true;
            }
        }
        if remove {
            textures.remove(self.current);
            self.current = self.current.saturating_sub(1);
        }
        document.set_body_textures(body, self.kept(textures));
        None
    }
}

/// The body's textures as they stand.
fn textures_of(document: &Document, body: BodyId) -> Vec<FaceTexture> {
    document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .map(|b| b.textures.clone())
        .unwrap_or_default()
}

/// What a recording keeps of the textures set.
pub fn set_textures_call(body: BodyId, textures: &[FaceTexture]) -> Recorded {
    let args = json!({"body": body.0.to_string(), "textures": textures});
    Recorded {
        id: "doc.set_textures".to_string(),
        args: match args {
            serde_json::Value::Object(map) => map,
            _ => Default::default(),
        },
        result: serde_json::Value::Null,
    }
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT3));
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(SPACE_3);
    ui_kit::widgets::overline(ui, text);
    ui.add_space(SPACE_1);
}

/// The built-in patterns as pictures of their heights; the one clicked.
fn gallery(ui: &mut Ui, current: Pattern) -> Option<Pattern> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(SPACE_1);
        for pattern in Pattern::BUILT_IN {
            let texture = thumbnail(ui.ctx(), pattern);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(THUMB, THUMB + 14.0), egui::Sense::click());
            let image = egui::Rect::from_min_size(rect.min, Vec2::splat(THUMB));
            ui.painter().image(
                texture.id(),
                image,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            let stroke = if pattern == current {
                egui::Stroke::new(2.0, ACCENT)
            } else if response.hovered() {
                egui::Stroke::new(1.0, TEXT2)
            } else {
                egui::Stroke::new(1.0, BORDER)
            };
            ui.painter()
                .rect_stroke(image, RADIUS_SM, stroke, egui::StrokeKind::Inside);
            ui.painter().text(
                egui::pos2(image.center().x, image.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                pattern.label(),
                sans(FONT_XS),
                if pattern == current { TEXT1 } else { TEXT3 },
            );
            if response.clicked() {
                picked = Some(pattern);
            }
        }
    });
    picked
}

/// A pattern's picture: two tiles by two, its heights in grey; made once.
fn thumbnail(ctx: &egui::Context, pattern: Pattern) -> egui::TextureHandle {
    let id = egui::Id::new(("texture_thumbnail", pattern.label()));
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    const PX: usize = 96;
    let mut pixels = Vec::with_capacity(PX * PX);
    for y in 0..PX {
        for x in 0..PX {
            let (u, v) = (x as f32 / PX as f32 * 2.0, 2.0 - y as f32 / PX as f32 * 2.0);
            let h = pattern.height(u, v, None);
            let g = (40.0 + h * 200.0) as u8;
            pixels.push(egui::Color32::from_gray(g));
        }
    }
    let image = egui::ColorImage::new([PX, PX], pixels);
    let handle = ctx.load_texture(
        format!("texture_thumbnail::{}", pattern.label()),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body with no texture gets one on the face the task opened on;
    /// Cancel takes it away again.
    #[test]
    fn opening_on_a_face_textures_it_and_cancel_puts_back() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = TextureTask::open(&doc, body, Some(2));
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        ctx.run_ui(Default::default(), |ui| {
            task.show(ui, &mut doc, &[], false, false, &mut Vec::new());
        })
        .textures_delta
        .clear();
        let textures = textures_of(&doc, body);
        assert_eq!(textures.len(), 1);
        // No mesh yet: the whole body.
        assert!(textures[0].faces.is_empty());
        let mut ended = None;
        ctx.run_ui(Default::default(), |ui| {
            ended = task.show(ui, &mut doc, &[], false, true, &mut Vec::new());
        })
        .textures_delta
        .clear();
        assert!(matches!(ended, Some(HostTaskEnd::Cancelled)));
        assert!(textures_of(&doc, body).is_empty());
    }

    /// On a copy that shows its source's textures, the task edits from
    /// them, and OK with nothing changed leaves the copy following the
    /// source rather than holding textures of its own.
    #[test]
    fn a_copy_edits_from_its_source_textures_and_keeps_following_them() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let knurl = vec![FaceTexture {
            texture: Texture::default(),
            faces: Vec::new(),
        }];
        doc.set_body_textures(body, knurl.clone());
        let copy = doc.create_linked_copy(body, None).unwrap();
        let mut task = TextureTask::open(&doc, copy, Some(2));
        assert_eq!(task.inherited, knurl);
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        ctx.run_ui(Default::default(), |ui| {
            task.show(ui, &mut doc, &[], false, false, &mut Vec::new());
        })
        .textures_delta
        .clear();
        assert!(
            textures_of(&doc, copy).is_empty(),
            "no default texture over the source's"
        );
        let mut ended = None;
        ctx.run_ui(Default::default(), |ui| {
            ended = task.show(ui, &mut doc, &[], true, false, &mut Vec::new());
        })
        .textures_delta
        .clear();
        let Some(HostTaskEnd::Accepted(_, calls)) = ended else {
            panic!("accepted");
        };
        assert!(calls.is_empty());
        assert!(textures_of(&doc, copy).is_empty());
    }

    /// OK records the textures as they stand, in the form the command
    /// reads back.
    #[test]
    fn ok_records_the_textures() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = TextureTask::open(&doc, body, None);
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        ctx.run_ui(Default::default(), |ui| {
            task.show(ui, &mut doc, &[], false, false, &mut Vec::new());
        })
        .textures_delta
        .clear();
        let mut ended = None;
        ctx.run_ui(Default::default(), |ui| {
            ended = task.show(ui, &mut doc, &[], true, false, &mut Vec::new());
        })
        .textures_delta
        .clear();
        let Some(HostTaskEnd::Accepted(_, calls)) = ended else {
            panic!("accepted");
        };
        assert_eq!(calls[0].id, "doc.set_textures");
        let back: Vec<FaceTexture> =
            serde_json::from_value(calls[0].args["textures"].clone()).unwrap();
        assert_eq!(back, textures_of(&doc, body));
    }
}
