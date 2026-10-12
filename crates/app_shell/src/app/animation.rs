//! Animations a bench records (a joint's motion swept through its range):
//! the scene drawn once per frame, as the camera sees it, into an animated
//! PNG, a GIF, a folder of numbered PNG frames, or with `ffmpeg` on the
//! system a WebM or MP4 video, by the ending of the name saved to. Frames
//! are drawn on the CPU, on a thread of their own, all framed alike so the
//! view holds still while the model moves.

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

/// The extension that saves a folder of frames: the folder takes the
/// file's name without it.
pub(crate) const FRAMES_EXTENSION: &str = "frames";

/// The kinds of file an animation is saved as, each named by the
/// extension that ends the file's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    AnimatedPng,
    Gif,
    Frames,
    Webm,
    Mp4,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::AnimatedPng,
        Kind::Gif,
        Kind::Frames,
        Kind::Webm,
        Kind::Mp4,
    ];

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Kind::AnimatedPng => "png",
            Kind::Gif => "gif",
            Kind::Frames => FRAMES_EXTENSION,
            Kind::Webm => "webm",
            Kind::Mp4 => "mp4",
        }
    }

    /// What `path` is saved as, and where: the kind its extension names,
    /// in any case; a name ending otherwise, or not at all, is an animated
    /// PNG and is given `.png`.
    pub(crate) fn of(path: &std::path::Path) -> (Kind, PathBuf) {
        let named = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .and_then(|e| Kind::ALL.into_iter().find(|k| k.extension() == e));
        match named {
            Some(kind) => (kind, path.to_path_buf()),
            None => {
                let mut name = path.as_os_str().to_owned();
                name.push(".png");
                (Kind::AnimatedPng, PathBuf::from(name))
            }
        }
    }
}

/// The save dialog's title, its one file type and that type's extensions,
/// the videos only with ffmpeg. The dialog reports only the path chosen,
/// never the type picked, so the name's ending decides and the dialog
/// offers no choice of type that could disagree with it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_dialog(videos: bool) -> (String, String, Vec<&'static str>) {
    let extensions: Vec<&'static str> = Kind::ALL
        .into_iter()
        .filter(|k| videos || !matches!(k, Kind::Webm | Kind::Mp4))
        .map(Kind::extension)
        .collect();
    let mut said = vec!["no ending or .png an animated PNG", ".gif a GIF"];
    if videos {
        said.push(".webm or .mp4 a video");
    }
    said.push(".frames a folder of PNG frames");
    let title = format!("Record the motion ({})", said.join(", "));
    let label = format!(
        "Animation ({})",
        extensions
            .iter()
            .map(|e| format!(".{e}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    (title, label, extensions)
}

/// Draw `animation` and write it to `path`, away from the window, in the
/// kind the path's extension names ([`Kind::of`]). The log says when it
/// is done.
pub(crate) fn write_in_background(animation: Animation, path: PathBuf) {
    let spawned = crate::platform::spawn("printcad-animation", move || {
        match write(&animation, &path) {
            Ok(written) => app_log::info(format!(
                "Saved a {}-frame animation to {}",
                animation.frames.len(),
                written.display()
            )),
            Err(why) => app_log::error(format!("Could not record the animation: {why}")),
        }
    });
    if let Err(err) = spawned {
        app_log::error(format!("Could not start recording the animation: {err}"));
    }
}

/// Write the animation where `path` says, in the kind [`Kind::of`] makes
/// of it; where it went.
fn write(animation: &Animation, path: &std::path::Path) -> Result<PathBuf, String> {
    let (kind, path) = Kind::of(path);
    let save = |target: &std::path::Path, bytes: Vec<u8>| {
        crate::platform::write(target, &bytes)
            .map_err(|err| format!("could not save {}: {err}", target.display()))
    };
    match kind {
        Kind::Webm | Kind::Mp4 => {
            encode_video(animation, &path, kind)?;
            Ok(path)
        }
        Kind::Gif => {
            save(&path, encode_gif(animation)?)?;
            Ok(path)
        }
        Kind::Frames => {
            let folder = path.with_extension("");
            std::fs::create_dir_all(&folder)
                .map_err(|err| format!("could not make {}: {err}", folder.display()))?;
            let stem = folder
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "frame".into());
            for (i, rgba) in frames(animation)?.into_iter().enumerate() {
                save(
                    &folder.join(format!("{stem}-{:04}.png", i + 1)),
                    encode_png(&rgba)?,
                )?;
            }
            Ok(folder)
        }
        Kind::AnimatedPng => {
            save(&path, encode(animation)?)?;
            Ok(path)
        }
    }
}

/// Every frame drawn, RGBA, framed alike.
fn frames(animation: &Animation) -> Result<Vec<Vec<u8>>, String> {
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
    animation
        .frames
        .iter()
        .map(|frame| {
            crate::thumbnail::rasterize_framed(
                frame,
                &framing,
                animation.forward,
                animation.up,
                WIDTH,
                HEIGHT,
            )
            .ok_or_else(|| "a frame had nothing to draw".to_string())
        })
        .collect()
}

/// Where `ffmpeg` is on this system, when it is: what a video is made
/// with.
pub(crate) fn ffmpeg() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let name = local_ipc::program_name("ffmpeg");
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
}

/// The frames as a `kind` video (WebM, VP9; MP4, H.264) at `path`,
/// through `ffmpeg`, the frames handed to it raw.
fn encode_video(animation: &Animation, path: &std::path::Path, kind: Kind) -> Result<(), String> {
    use std::io::Write as _;
    let ffmpeg = ffmpeg().ok_or("a video needs ffmpeg, which is not on this system")?;
    let frames = frames(animation)?;
    let rate = format!("{:.3}", 1000.0 / f64::from(animation.frame_ms.max(1)));
    let codec: &[&str] = match kind {
        Kind::Mp4 => &["-c:v", "libx264", "-crf", "20"],
        _ => &["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "32"],
    };
    let mut child = std::process::Command::new(ffmpeg)
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
        ])
        .args([
            "-s",
            &format!("{WIDTH}x{HEIGHT}"),
            "-framerate",
            &rate,
            "-i",
            "-",
        ])
        .args(codec)
        .args(["-pix_fmt", "yuv420p"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start ffmpeg: {e}"))?;
    if let Some(mut input) = child.stdin.take() {
        for rgba in &frames {
            input
                .write_all(rgba)
                .map_err(|e| format!("ffmpeg stopped taking frames: {e}"))?;
        }
    }
    let done = child
        .wait_with_output()
        .map_err(|e| format!("ffmpeg did not finish: {e}"))?;
    if !done.status.success() {
        let said = String::from_utf8_lossy(&done.stderr);
        return Err(format!("ffmpeg could not make the video: {}", said.trim()));
    }
    Ok(())
}

/// The frames put together as an animated PNG, played on a loop.
fn encode(animation: &Animation) -> Result<Vec<u8>, String> {
    let frames = frames(animation)?;
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .set_animated(frames.len() as u32, 0)
            .map_err(|e| e.to_string())?;
        encoder
            .set_frame_delay(animation.frame_ms.min(u32::from(u16::MAX)) as u16, 1000)
            .map_err(|e| e.to_string())?;
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        for rgba in &frames {
            writer.write_image_data(rgba).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A `width` × `height` RGBA picture as PNG bytes.
pub(crate) fn png_of(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

fn encode_png(rgba: &[u8]) -> Result<Vec<u8>, String> {
    png_of(WIDTH, HEIGHT, rgba)
}

/// The frames as a GIF played on a loop, each frame's colours chosen for
/// it.
fn encode_gif(animation: &Animation) -> Result<Vec<u8>, String> {
    let frames = frames(animation)?;
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, WIDTH as u16, HEIGHT as u16, &[])
            .map_err(|e| e.to_string())?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        // Hundredths of a second.
        let delay = (animation.frame_ms / 10).clamp(2, u32::from(u16::MAX)) as u16;
        for mut rgba in frames {
            let mut frame = gif::Frame::from_rgba_speed(WIDTH as u16, HEIGHT as u16, &mut rgba, 10);
            frame.delay = delay;
            encoder.write_frame(&frame).map_err(|e| e.to_string())?;
        }
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

    /// The name's ending alone picks the kind, any case; any other ending,
    /// or none, is an animated PNG given `.png`.
    #[test]
    fn the_name_s_ending_picks_the_kind_saved() {
        let of = |name: &str| Kind::of(std::path::Path::new(name));
        assert_eq!(of("/v/hinge.webm"), (Kind::Webm, "/v/hinge.webm".into()));
        assert_eq!(of("/v/HINGE.MP4"), (Kind::Mp4, "/v/HINGE.MP4".into()));
        assert_eq!(of("/v/hinge.gif"), (Kind::Gif, "/v/hinge.gif".into()));
        assert_eq!(
            of("/v/shot.frames"),
            (Kind::Frames, "/v/shot.frames".into())
        );
        assert_eq!(
            of("/v/hinge.png"),
            (Kind::AnimatedPng, "/v/hinge.png".into())
        );
        assert_eq!(
            of("/v/Motion 1"),
            (Kind::AnimatedPng, "/v/Motion 1.png".into())
        );
        assert_eq!(of("/v/arm.v2"), (Kind::AnimatedPng, "/v/arm.v2.png".into()));
    }

    /// The dialog's one file type lists what can be saved, the videos only
    /// with ffmpeg, and its title says how the ending picks.
    #[test]
    fn the_save_dialog_offers_one_type_naming_every_ending() {
        let (title, label, extensions) = save_dialog(false);
        assert_eq!(extensions, ["png", "gif", "frames"]);
        assert_eq!(label, "Animation (.png, .gif, .frames)");
        assert!(!title.contains("webm"), "{title}");
        let (title, label, extensions) = save_dialog(true);
        assert_eq!(extensions, ["png", "gif", "frames", "webm", "mp4"]);
        assert!(label.contains(".webm, .mp4"), "{label}");
        assert!(title.contains(".webm or .mp4 a video"), "{title}");
        for kind in Kind::ALL {
            assert_eq!(
                Kind::of(&PathBuf::from(format!("a.{}", kind.extension()))).0,
                kind
            );
        }
    }

    #[test]
    fn frames_become_a_gif_or_a_folder_of_pngs() {
        let animation = Animation {
            name: "slide".into(),
            frames: (0..3).map(|i| vec![triangle(i as f32 * 5.0)]).collect(),
            forward: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            frame_ms: 50,
        };
        let dir = std::env::temp_dir().join(format!("printcad-anim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gif_path = write(&animation, &dir.join("slide.gif")).unwrap();
        let bytes = std::fs::read(&gif_path).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).unwrap();
        let mut count = 0;
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            assert_eq!(frame.delay, 5);
            count += 1;
        }
        assert_eq!(count, 3);
        let folder = write(&animation, &dir.join(format!("shot.{FRAMES_EXTENSION}"))).unwrap();
        assert_eq!(folder, dir.join("shot"));
        for i in 1..=3 {
            assert!(folder.join(format!("shot-{i:04}.png")).exists());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A path with no extension becomes an animated PNG; with ffmpeg on
    /// the system, a `.webm` is a WebM video (its EBML header first) and an
    /// `.mp4` an MP4 (an `ftyp` box first).
    #[test]
    fn frames_become_a_video_where_ffmpeg_can_make_one() {
        let animation = Animation {
            name: "slide".into(),
            frames: (0..4).map(|i| vec![triangle(i as f32 * 5.0)]).collect(),
            forward: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            frame_ms: 40,
        };
        let dir = std::env::temp_dir().join(format!("printcad-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = write(&animation, &dir.join("plain")).unwrap();
        assert_eq!(plain, dir.join("plain.png"));
        assert!(std::fs::read(&plain).unwrap().starts_with(b"\x89PNG"));
        if ffmpeg().is_some() {
            let webm = write(&animation, &dir.join("slide.webm")).unwrap();
            assert!(
                std::fs::read(webm)
                    .unwrap()
                    .starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
            );
            let mp4 = write(&animation, &dir.join("slide.mp4")).unwrap();
            assert_eq!(&std::fs::read(mp4).unwrap()[4..8], b"ftyp");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
