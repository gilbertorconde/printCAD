//! Animations a bench records (a joint's motion swept through its range):
//! the scene drawn once per frame, as the camera sees it, into an animated
//! PNG. Frames are drawn on the CPU, on a thread of their own, all framed
//! alike so the view holds still while the model moves.

use std::path::PathBuf;
use std::sync::Arc;

use core_document::{BodyId, BodyPlacement};
use glam::Vec3;

use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::thumbnail::Shape;

/// Frame size, in pixels.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;

/// Everything a recording needs, taken from the document when it is asked
/// for: each frame's shapes and the view.
pub(crate) struct Animation {
    pub name: String,
    frames: Vec<Vec<Shape>>,
    forward: Vec3,
    up: Vec3,
    frame_ms: u32,
}

impl PrintCadApp {
    /// The visible bodies once per frame, those `frames` name at that
    /// frame's placement, the rest where they sit.
    pub(crate) fn animation(
        &self,
        name: String,
        frames: &[Vec<(BodyId, BodyPlacement)>],
        frame_ms: u32,
    ) -> Animation {
        let document = &self.session.document;
        let bodies: Vec<BodyId> = document
            .bodies()
            .iter()
            .map(|b| b.id)
            .filter(|b| document.imported_body_effective_visible(*b))
            .collect();
        let frames = frames
            .iter()
            .map(|placements| {
                bodies
                    .iter()
                    .filter_map(|body| {
                        let mesh = match placements.iter().find(|(b, _)| b == body) {
                            Some((_, placement)) => {
                                let (local, _) = document.local_geometry(*body)?;
                                Arc::new(placement.mesh(&local))
                            }
                            None => Arc::clone(&document.imported_geometry(*body)?.mesh),
                        };
                        Some(crate::app::doc_io::preview_shape(document, *body, mesh))
                    })
                    .collect()
            })
            .collect();
        let (forward, up) = self.session.camera.view_basis();
        Animation {
            name,
            frames,
            forward,
            up,
            frame_ms,
        }
    }
}

/// Draw `animation` and write it to `path`, away from the window; the log
/// says when it is done.
pub(crate) fn write_in_background(animation: Animation, path: PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("printcad-animation".into())
        .spawn(move || match encode(&animation) {
            Ok(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => app_log::info(format!(
                    "Saved a {}-frame animation to {}",
                    animation.frames.len(),
                    path.display()
                )),
                Err(err) => app_log::error(format!("Could not save {}: {err}", path.display())),
            },
            Err(why) => app_log::error(format!("Could not record the animation: {why}")),
        });
    if let Err(err) = spawned {
        app_log::error(format!("Could not start recording the animation: {err}"));
    }
}

/// The frames drawn and put together as an animated PNG, played on a loop.
fn encode(animation: &Animation) -> Result<Vec<u8>, String> {
    if animation.frames.is_empty() {
        return Err("there are no frames".into());
    }
    let framing: Vec<Shape> = animation
        .frames
        .iter()
        .flatten()
        .map(|s| Shape {
            mesh: Arc::clone(&s.mesh),
            color: s.color,
            vertex_colours: s.vertex_colours,
        })
        .collect();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .set_animated(animation.frames.len() as u32, 0)
            .map_err(|e| e.to_string())?;
        encoder
            .set_frame_delay(animation.frame_ms.min(u32::from(u16::MAX)) as u16, 1000)
            .map_err(|e| e.to_string())?;
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        for frame in &animation.frames {
            let rgba = crate::thumbnail::rasterize_framed(
                frame,
                &framing,
                animation.forward,
                animation.up,
                WIDTH,
                HEIGHT,
            )
            .ok_or("a frame had nothing to draw")?;
            writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_api::TriMesh;

    fn triangle(x: f32) -> Shape {
        Shape {
            mesh: Arc::new(TriMesh {
                positions: vec![[x, 0.0, 0.0], [x + 10.0, 0.0, 0.0], [x, 10.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                indices: vec![0, 1, 2],
                ..TriMesh::default()
            }),
            color: [0.8, 0.5, 0.2],
            vertex_colours: false,
        }
    }

    #[test]
    fn frames_become_an_animated_png_that_plays_them_in_turn() {
        let animation = Animation {
            name: "slide".into(),
            frames: (0..4).map(|i| vec![triangle(i as f32 * 5.0)]).collect(),
            forward: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            frame_ms: 40,
        };
        let bytes = encode(&animation).expect("encodes");
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let reader = decoder.read_info().expect("reads");
        let info = reader.info();
        assert_eq!((info.width, info.height), (WIDTH, HEIGHT));
        let control = info.animation_control().expect("animated");
        assert_eq!(control.num_frames, 4);
        assert_eq!(control.num_plays, 0, "on a loop");
    }
}
