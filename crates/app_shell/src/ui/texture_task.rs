//! The Surface texture task: the patterns pressed into a body's faces for
//! printing. Faces clicked in the view join the texture being edited; the
//! pattern comes from a gallery or a picture; edits show in the view as
//! they are made, OK keeps them as one undo step, Cancel puts back the
//! textures the task opened on.

use bench_api::{ButtonStyle, GapSize, PanelEvent, Tone, Widget};
use core_document::{BodyId, Document, FaceKey, FaceTexture, Recorded};
use egui::Ui;
use serde_json::json;
use surface_texture::{Pattern, Projection, Texture};

use super::UiCommand;
use super::host_tasks::HostTaskEnd;
use super::task_blocks::{
    button, degrees, draw, gap, grid, heading, hinted, indexed, length, note, row, secondary,
    select, wrapping,
};

#[derive(Debug, Clone)]
pub struct TextureTask {
    body: BodyId,
    /// The body's textures when the task opened: OK records nothing
    /// while they are the same.
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
        if !textures.is_empty() {
            self.current = self.current.min(textures.len() - 1);
            self.take_picks(document, picked_faces, &mut textures[self.current]);
        }
        let widgets = self.widgets(&textures);
        for event in draw(ui, document, widgets) {
            self.event(&event, &mut textures, commands);
        }
        document.set_body_textures(body, self.kept(textures));
        None
    }

    /// The faces picked in the view join the texture edited, each once.
    fn take_picks(
        &mut self,
        document: &Document,
        picked_faces: &[(BodyId, u32)],
        edited: &mut FaceTexture,
    ) {
        let picks: Vec<u32> = picked_faces
            .iter()
            .filter(|(b, _)| *b == self.body)
            .map(|(_, index)| *index)
            .collect();
        if picks.is_empty() || picks == self.last_picks {
            return;
        }
        if let Some(geometry) = document.imported_geometry(self.body) {
            let mesh = &geometry.mesh;
            for &index in picks.iter().filter(|i| !self.last_picks.contains(i)) {
                if !edited.faces.iter().any(|k| k.is_face(mesh, index)) {
                    edited.faces.push(FaceKey::of(mesh, index));
                }
            }
        }
        self.last_picks = picks;
    }

    fn widgets(&self, textures: &[FaceTexture]) -> Vec<Widget> {
        // A label for each of the body's textures, picking the one edited,
        // and Add for another.
        let mut tabs: Vec<Widget> = (0..textures.len())
            .map(|n| {
                bench_api::Selectable {
                    id: format!("texture:{n}"),
                    text: format!("Texture {}", n + 1),
                    selected: n == self.current,
                }
                .into()
            })
            .collect();
        tabs.push(hinted(
            "Another texture, on other faces",
            button("add", "Add", ButtonStyle::Small, true),
        ));
        let mut w = vec![wrapping(tabs)];
        if !self.inherited.is_empty() {
            w.push(note(
                "A copy without textures of its own shows its original's. Changing them here \
                 gives this copy its own.",
            ));
        }
        let Some(edited) = textures.get(self.current) else {
            w.push(note(
                "No texture. Add one to press a pattern into the body's faces.",
            ));
            return w;
        };
        w.extend(heading("Faces"));
        if edited.faces.is_empty() {
            w.push(note(
                "The whole body. Click faces in the view to texture only them.",
            ));
        } else {
            w.push(wrapping(edited.faces.iter().enumerate().map(|(n, key)| {
                hinted(
                    "Leave this face smooth",
                    button(
                        &format!("drop:{n}"),
                        &format!("Face {}  ×", key.index + 1),
                        ButtonStyle::Small,
                        true,
                    ),
                )
            })));
            w.push(secondary("whole", "Whole body"));
        }

        w.extend(heading("Pattern"));
        let t = &edited.texture;
        w.push(
            bench_api::Gallery {
                id: "pattern".into(),
                tiles: thumbnails().to_vec(),
                selected: Pattern::BUILT_IN.iter().position(|p| *p == t.pattern),
            }
            .into(),
        );
        let mut picture = vec![hinted(
            "A greyscale PNG or JPEG, white high",
            secondary("picture", "Picture…"),
        )];
        if matches!(t.pattern, Pattern::Image { .. }) {
            picture.push(
                bench_api::Caption {
                    text: "A picture is in use".into(),
                    tone: Tone::Muted,
                    small: true,
                    ..Default::default()
                }
                .into(),
            );
        }
        w.push(row(picture));

        w.extend(heading("How it lies"));
        let projections = Projection::all();
        w.push(grid(
            "texture_grid",
            false,
            [
                select(
                    "projection",
                    "Projection",
                    projections.iter().map(|p| p.label()),
                    projections
                        .iter()
                        .position(|p| *p == t.projection)
                        .unwrap_or(usize::MAX),
                    170.0,
                ),
                length("tile", "Tile", t.tile_mm, 0.2, 500.0),
                degrees("turn", "Turn", t.rotation_deg, None),
                length("depth", "Depth", t.depth_mm, 0.01, 20.0),
                degrees("keep_flat", "Keep flat", t.keep_flat_deg, Some((0.0, 89.0))),
            ],
        ));
        w.push(
            bench_api::Check {
                id: "inward".into(),
                text: "Into the surface".into(),
                on: t.inward,
                ..Default::default()
            }
            .into(),
        );
        w.push(note(
            "Keep flat leaves faces within that angle of facing up or down smooth: a print's \
             top and the side it stands on. 0 textures them too.",
        ));
        w.push(gap(GapSize::Medium));
        w.push(secondary("remove", "Remove this texture"));
        w
    }

    fn event(
        &mut self,
        event: &PanelEvent,
        textures: &mut Vec<FaceTexture>,
        commands: &mut Vec<UiCommand>,
    ) {
        if let PanelEvent::Button { id } = event {
            if let Some(n) = indexed(id, "texture:") {
                self.current = n;
                return;
            }
            if id == "add" {
                textures.push(FaceTexture {
                    texture: Texture::default(),
                    faces: Vec::new(),
                });
                self.current = textures.len() - 1;
                self.last_picks.clear();
                return;
            }
        }
        let current = self.current;
        let Some(edited) = textures.get_mut(current) else {
            return;
        };
        let t = &mut edited.texture;
        match event {
            PanelEvent::Button { id } => match id.as_str() {
                "whole" => {
                    edited.faces.clear();
                    self.last_picks.clear();
                }
                "picture" => commands.push(UiCommand::PickTexturePicture {
                    body: self.body,
                    index: current,
                }),
                "remove" => {
                    textures.remove(current);
                    self.current = current.saturating_sub(1);
                }
                id => {
                    if let Some(n) = indexed(id, "drop:")
                        && n < edited.faces.len()
                    {
                        edited.faces.remove(n);
                    }
                }
            },
            PanelEvent::Choice { id, index } if id == "pattern" => {
                if let Some(pattern) = Pattern::BUILT_IN.get(*index) {
                    t.pattern = *pattern;
                }
            }
            PanelEvent::Choice { id, index } if id == "projection" => {
                if let Some(projection) = Projection::all().get(*index) {
                    t.projection = *projection;
                }
            }
            PanelEvent::Number { id, value } => {
                let v = *value as f32;
                match id.as_str() {
                    "tile" => t.tile_mm = v,
                    "turn" => t.rotation_deg = v,
                    "depth" => t.depth_mm = v,
                    "keep_flat" => t.keep_flat_deg = v,
                    _ => {}
                }
            }
            PanelEvent::Toggle { id, on } if id == "inward" => t.inward = *on,
            _ => {}
        }
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

/// The built-in patterns as gallery tiles: each a picture of its heights,
/// two tiles by two in grey, made once.
fn thumbnails() -> &'static [bench_api::Tile] {
    static TILES: std::sync::OnceLock<Vec<bench_api::Tile>> = std::sync::OnceLock::new();
    TILES.get_or_init(|| {
        const PX: usize = 96;
        Pattern::BUILT_IN
            .iter()
            .map(|pattern| {
                let mut rgba = Vec::with_capacity(PX * PX * 4);
                for y in 0..PX {
                    for x in 0..PX {
                        let (u, v) = (x as f32 / PX as f32 * 2.0, 2.0 - y as f32 / PX as f32 * 2.0);
                        let g = (40.0 + pattern.height(u, v, None) * 200.0) as u8;
                        rgba.extend([g, g, g, 255]);
                    }
                }
                bench_api::Tile {
                    label: pattern.label().to_string(),
                    picture: bench_api::Picture {
                        key: format!("texture_thumbnail::{}", pattern.label()),
                        width: PX as u32,
                        height: PX as u32,
                        rgba,
                    },
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body with no texture gets one on the face the task opened on;
    /// Cancel, through the journal, takes it away again.
    #[test]
    fn opening_on_a_face_textures_it_and_cancel_puts_back() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            let mut task = TextureTask::open(doc, body, Some(2));
            let ctx = egui::Context::default();
            ui_kit::apply_theme(&ctx);
            ctx.run_ui(Default::default(), |ui| {
                task.show(ui, doc, &[], false, false, &mut Vec::new());
            })
            .textures_delta
            .clear();
            let textures = textures_of(doc, body);
            assert_eq!(textures.len(), 1);
            // No mesh yet: the whole body.
            assert!(textures[0].faces.is_empty());
            let mut ended = None;
            ctx.run_ui(Default::default(), |ui| {
                ended = task.show(ui, doc, &[], false, true, &mut Vec::new());
            })
            .textures_delta
            .clear();
            assert!(matches!(ended, Some(HostTaskEnd::Cancelled)));
        });
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

    /// Add, the tabs, a pattern tile, the numbers and Remove edit the
    /// body's textures; a pick joins the one edited.
    #[test]
    fn panel_events_edit_the_textures() {
        let doc = Document::new("t");
        let mut task = TextureTask::open(&doc, BodyId(uuid::Uuid::nil()), None);
        let mut textures = vec![FaceTexture {
            texture: Texture::default(),
            faces: Vec::new(),
        }];
        let mut commands = Vec::new();
        let hexes = Pattern::BUILT_IN.len() - 1;
        for event in [
            PanelEvent::Button { id: "add".into() },
            PanelEvent::Choice {
                id: "pattern".into(),
                index: hexes,
            },
            PanelEvent::Number {
                id: "depth".into(),
                value: 0.8,
            },
            PanelEvent::Toggle {
                id: "inward".into(),
                on: true,
            },
            PanelEvent::Button {
                id: "picture".into(),
            },
            PanelEvent::Button {
                id: "texture:0".into(),
            },
            PanelEvent::Button {
                id: "remove".into(),
            },
        ] {
            task.event(&event, &mut textures, &mut commands);
        }
        assert_eq!(textures.len(), 1, "the first removed, the added kept");
        let t = &textures[0].texture;
        assert_eq!(t.pattern, Pattern::BUILT_IN[hexes]);
        assert!(t.inward && (t.depth_mm - 0.8).abs() < 1e-6);
        assert!(matches!(
            commands[..],
            [UiCommand::PickTexturePicture { index: 1, .. }]
        ));
        let json = serde_json::to_string(&task.widgets(&textures)).unwrap();
        assert!(json.contains("texture:0") && !json.contains("texture:1"));
    }
}
