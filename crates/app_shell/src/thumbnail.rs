//! Pictures of bodies drawn on the CPU: the preview saved with a document,
//! the frames of an assembly's animation and the pictures an agent asks
//! for (`proof.rs`). Orthographic, flat shaded, framed to what is asked
//! with a margin. Nothing here touches the GPU or the window, so it runs
//! in the save worker, headless and in tests, and the same input draws the
//! same pixels every time.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use kernel_api::TriMesh;

/// The preview's size, in pixels, wide as the start page's cards.
pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 180;
/// Drawn this many times larger and averaged down, for smooth edges.
const SUPERSAMPLE: u32 = 2;
/// Above this many output pixels a picture is drawn once per pixel: a
/// supersampled 2048 square would hold a few hundred megabytes.
const SUPERSAMPLED_PIXELS: u32 = 2_100_000;
/// Above this many triangles, every n-th is drawn: a thumbnail of an
/// assembly needs its silhouette, not its fillets.
const TRIANGLE_BUDGET: usize = 2_000_000;
/// The share of the picture left clear on each side of the framed model.
pub const MARGIN: f32 = 0.08;
/// How much one see-through layer covers what lies behind it.
const XRAY_ALPHA: f32 = 0.28;
/// The section's fill: the back faces seen through a cut, darkened as the
/// viewport's shader darkens them.
const SECTION_SHADE: f32 = 0.55;

/// One body to draw: its mesh and the colour it shows in.
pub struct Shape {
    pub mesh: Arc<TriMesh>,
    pub color: [f32; 3],
    /// The mesh's own per-vertex colours multiply `color`, as they do in
    /// the viewport for a body without a colour of its own.
    pub vertex_colours: bool,
}

/// How a picture draws beyond its shaded faces. The default is the plain
/// preview: no outlines, no cut, a clear background.
#[derive(Default, Clone)]
pub struct Look {
    /// Face outlines (each mesh's `edges`) over the faces, in this colour.
    pub edges: Option<[f32; 3]>,
    /// Faces drawn in a colour of their own, by (shape index, face index).
    pub face_paint: HashMap<(usize, u32), [f32; 3]>,
    /// Edges drawn wide in a colour of their own, by (shape index, the
    /// kernel edge `edge_ids` names), whether or not outlines are on.
    pub edge_paint: HashMap<(usize, u32), [f32; 3]>,
    /// A cut: a point on the plane and its normal; whatever lies on the
    /// side the normal points to is cut away.
    pub section: Option<(Vec3, Vec3)>,
    /// Bodies see-through; painted faces stay solid.
    pub xray: bool,
    /// An opaque background, top and bottom of a vertical gradient.
    pub background: Option<([f32; 3], [f32; 3])>,
    /// Points the framing takes in beside the shapes (markers).
    pub frame_points: Vec<Vec3>,
}

/// Where the world lands in a drawn picture.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
    centre: Vec3,
    /// Output pixels per model unit.
    pub scale: f32,
    pub width: u32,
    pub height: u32,
}

impl Projection {
    /// A world point's place in the picture, in output pixels from the top
    /// left corner.
    pub fn to_pixel(self, p: Vec3) -> (f32, f32) {
        let q = Vec3::new(p.dot(self.right), p.dot(self.up), p.dot(self.forward));
        (
            self.width as f32 * 0.5 + (q.x - self.centre.x) * self.scale,
            self.height as f32 * 0.5 - (q.y - self.centre.y) * self.scale,
        )
    }
}

/// A drawn picture: straight RGBA and how it was projected.
pub struct Drawn {
    pub rgba: Vec<u8>,
    pub projection: Projection,
}

/// The PNG, or `None` when there is nothing to draw.
pub fn render(shapes: &[Shape], forward: Vec3, up: Vec3) -> Option<Vec<u8>> {
    render_at(shapes, forward, up, WIDTH, HEIGHT)
}

/// The PNG at `width` × `height`, or `None` when there is nothing to draw.
pub fn render_at(
    shapes: &[Shape],
    forward: Vec3,
    up: Vec3,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let rgba = rasterize_at(shapes, forward, up, width, height)?;
    pixmap(&rgba, width, height)?.encode_png().ok()
}

/// Straight RGBA as a tiny-skia pixmap, which holds premultiplied colour.
pub fn pixmap(rgba: &[u8], width: u32, height: u32) -> Option<tiny_skia::Pixmap> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    for (dst, src) in pixmap
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_chunks::<4>().0)
    {
        let a = u32::from(src[3]);
        for c in 0..3 {
            dst[c] = ((u32::from(src[c]) * a + 127) / 255) as u8;
        }
        dst[3] = src[3];
    }
    Some(pixmap)
}

/// Straight RGBA, `WIDTH` × `HEIGHT`, the model fitted with a margin.
#[cfg(test)]
pub fn rasterize(shapes: &[Shape], forward: Vec3, up: Vec3) -> Option<Vec<u8>> {
    rasterize_at(shapes, forward, up, WIDTH, HEIGHT)
}

/// Straight RGBA, `width` × `height`, the model fitted with a margin.
pub fn rasterize_at(
    shapes: &[Shape],
    forward: Vec3,
    up: Vec3,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    rasterize_framed(shapes, shapes, forward, up, width, height)
}

/// Straight RGBA, `width` × `height`, framed to fit `framing` with a
/// margin rather than `shapes` themselves: the frames of an animation
/// share one framing, so the view holds still while the model moves.
pub fn rasterize_framed(
    shapes: &[Shape],
    framing: &[Shape],
    forward: Vec3,
    up: Vec3,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    draw(
        shapes,
        framing,
        forward,
        up,
        width,
        height,
        &Look::default(),
    )
    .map(|d| d.rgba)
}

/// One sample of the supersampled picture.
#[derive(Clone, Copy, Default)]
struct Sample {
    rgb: [f32; 3],
    alpha: f32,
}

/// The supersampled buffers a picture is drawn into.
struct Canvas {
    w: u32,
    h: u32,
    depth: Vec<f32>,
    colour: Vec<Sample>,
}

/// What a triangle's pixels do with the buffers.
#[derive(Clone, Copy, PartialEq)]
enum Pass {
    /// Nearest wins and writes its depth.
    Opaque,
    /// In front of the opaque depth, adds one see-through layer.
    Layer,
}

/// The see-through layers in front of each sample: their colours summed
/// and their count, so the result does not depend on drawing order.
struct Layers {
    sum: Vec<[f32; 3]>,
    count: Vec<u16>,
}

/// Draw `shapes` from `forward` with `up` up, framed to `framing` and the
/// look's points, as `look` says. `None` when there is nothing to draw.
pub fn draw(
    shapes: &[Shape],
    framing: &[Shape],
    forward: Vec3,
    up: Vec3,
    width: u32,
    height: u32,
    look: &Look,
) -> Option<Drawn> {
    let forward = forward.try_normalize()?;
    let right = forward.cross(up).try_normalize()?;
    let up = right.cross(forward);
    let project = |p: Vec3| Vec3::new(p.dot(right), p.dot(up), p.dot(forward));

    // The model's extent on screen.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let triangles: usize = shapes.iter().map(|s| s.mesh.indices.len() / 3).sum();
    for shape in framing {
        if shape.mesh.indices.len() < 3 {
            continue;
        }
        for p in &shape.mesh.positions {
            let q = project(Vec3::from_array(*p));
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    for p in &look.frame_points {
        let q = project(*p);
        lo = lo.min(q);
        hi = hi.max(q);
    }
    if triangles == 0 || lo.x > hi.x || width == 0 || height == 0 {
        return None;
    }
    let ss = if width * height <= SUPERSAMPLED_PIXELS {
        SUPERSAMPLE
    } else {
        1
    };
    let (w, h) = (width * ss, height * ss);
    let span_x = (hi.x - lo.x).max(1e-6);
    let span_y = (hi.y - lo.y).max(1e-6);
    let scale = ((w as f32 * (1.0 - 2.0 * MARGIN)) / span_x)
        .min((h as f32 * (1.0 - 2.0 * MARGIN)) / span_y);
    let centre = (lo + hi) * 0.5;
    let to_sample = |p: Vec3| {
        let q = project(p);
        (
            w as f32 * 0.5 + (q.x - centre.x) * scale,
            // Image rows run downward.
            h as f32 * 0.5 - (q.y - centre.y) * scale,
            q.z,
        )
    };
    // Outlines pass the depth test this far behind the faces they bound.
    let bias = (hi.z - lo.z).abs() * 0.004 + 1e-4;

    let mut canvas = Canvas {
        w,
        h,
        depth: vec![f32::INFINITY; (w * h) as usize],
        colour: vec![Sample::default(); (w * h) as usize],
    };
    let mut layers = look.xray.then(|| Layers {
        sum: vec![[0.0; 3]; (w * h) as usize],
        count: vec![0; (w * h) as usize],
    });
    let section = look
        .section
        .and_then(|(origin, normal)| Some((origin, normal.try_normalize()?)));
    let past = |p: Vec3| section.map_or(0.0, |(o, n)| (p - o).dot(n));
    let light = (-forward * 0.8 + up * 0.5 - right * 0.3).normalize();
    let stride = triangles.div_ceil(TRIANGLE_BUDGET).max(1);

    // Painted faces always, and every face unless see-through, are solid;
    // see-through faces go on in a second pass against the solid depth.
    let passes: &[Pass] = if look.xray {
        &[Pass::Opaque, Pass::Layer]
    } else {
        &[Pass::Opaque]
    };
    for &pass in passes {
        for (si, shape) in shapes.iter().enumerate() {
            let mesh = &*shape.mesh;
            let vertex_colours = shape.vertex_colours && mesh.colors.len() == mesh.positions.len();
            let faces_known = mesh.faces.len() * 3 == mesh.indices.len();
            for (t, tri) in mesh
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .enumerate()
                .step_by(stride)
            {
                let paint = if look.face_paint.is_empty() || !faces_known {
                    None
                } else {
                    look.face_paint.get(&(si, mesh.faces[t]))
                };
                let solid = paint.is_some() || !look.xray;
                if solid != (pass == Pass::Opaque) {
                    continue;
                }
                let [a, b, c] =
                    [tri[0], tri[1], tri[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
                let n = (b - a).cross(c - a).normalize_or_zero();
                // Both sides lit alike: winding is not to be trusted in every mesh.
                let shade = 0.35 + 0.65 * n.dot(light).abs();
                let base = if vertex_colours {
                    let sum = tri.iter().fold(Vec3::ZERO, |s, i| {
                        s + Vec3::from_array(mesh.colors[*i as usize])
                    });
                    (sum / 3.0 * Vec3::from_array(shape.color)).to_array()
                } else {
                    shape.color
                };
                let rgb = match paint {
                    // Lit lightly, so a painted face reads as its colour.
                    Some(p) => p.map(|c| (c * (0.7 + 0.3 * shade)).clamp(0.0, 1.0)),
                    // Seen through a cut, the inside of a solid is its back
                    // faces: drawn flat and darker, as material.
                    None if section.is_some() && n.dot(forward) > 0.0 => {
                        base.map(|c| (c * SECTION_SHADE).clamp(0.0, 1.0))
                    }
                    None => base.map(|c| (c * shade).clamp(0.0, 1.0)),
                };
                let cut = section.map(|_| [past(a), past(b), past(c)]);
                if cut.is_some_and(|d| d.iter().all(|d| *d > 0.0)) {
                    continue;
                }
                fill(
                    &mut canvas,
                    layers.as_mut(),
                    pass,
                    [to_sample(a), to_sample(b), to_sample(c)],
                    cut,
                    rgb,
                );
            }
        }
    }
    if let Some(layers) = &layers {
        flatten_layers(&mut canvas, layers);
    }

    // Outlines, then the painted edges over them.
    let outline_width = 1.25 * ss as f32;
    let painted_width = 3.0 * ss as f32;
    for painted in [false, true] {
        if !painted && look.edges.is_none() || painted && look.edge_paint.is_empty() {
            continue;
        }
        for (si, shape) in shapes.iter().enumerate() {
            let mesh = &*shape.mesh;
            let ids_known = mesh.edge_ids.len() * 2 == mesh.edges.len();
            for (k, pair) in mesh.edges.as_chunks::<2>().0.iter().enumerate() {
                let paint = ids_known
                    .then(|| look.edge_paint.get(&(si, mesh.edge_ids[k])))
                    .flatten();
                let (rgb, width) = match (painted, paint, look.edges) {
                    (true, Some(p), _) => (*p, painted_width),
                    (false, _, Some(e)) => (e, outline_width),
                    _ => continue,
                };
                let mut a = Vec3::from(mesh.positions[pair[0] as usize]);
                let mut b = Vec3::from(mesh.positions[pair[1] as usize]);
                if section.is_some() {
                    let (da, db) = (past(a), past(b));
                    if da > 0.0 && db > 0.0 {
                        continue;
                    }
                    if da > 0.0 {
                        a = a + (b - a) * (da / (da - db));
                    } else if db > 0.0 {
                        b = b + (a - b) * (db / (db - da));
                    }
                }
                line(&mut canvas, to_sample(a), to_sample(b), width, bias, rgb);
            }
        }
    }

    let rgba = resolve(&canvas, width, height, ss, look.background);
    Some(Drawn {
        rgba,
        projection: Projection {
            right,
            up,
            forward,
            centre,
            scale: scale / ss as f32,
            width,
            height,
        },
    })
}

/// Fill one triangle: nearest wins, or one see-through layer in front of
/// the solid. `cut` holds each corner's distance past the section plane;
/// pixels past it are left out.
fn fill(
    canvas: &mut Canvas,
    mut layers: Option<&mut Layers>,
    pass: Pass,
    [p0, p1, p2]: [(f32, f32, f32); 3],
    cut: Option<[f32; 3]>,
    rgb: [f32; 3],
) {
    let (w, h) = (canvas.w, canvas.h);
    let area = (p1.0 - p0.0) * (p2.1 - p0.1) - (p2.0 - p0.0) * (p1.1 - p0.1);
    if area.abs() < 1e-9 {
        return;
    }
    let min_x = p0.0.min(p1.0).min(p2.0).floor().max(0.0) as u32;
    let max_x = (p0.0.max(p1.0).max(p2.0).ceil() as i64).clamp(0, i64::from(w) - 1) as u32;
    let min_y = p0.1.min(p1.1).min(p2.1).floor().max(0.0) as u32;
    let max_y = (p0.1.max(p1.1).max(p2.1).ceil() as i64).clamp(0, i64::from(h) - 1) as u32;
    if min_x > max_x || min_y > max_y {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p1.0 - px) * (p2.1 - py) - (p2.0 - px) * (p1.1 - py)) / area;
            let w1 = ((p2.0 - px) * (p0.1 - py) - (p0.0 - px) * (p2.1 - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            if let Some([d0, d1, d2]) = cut
                && w0 * d0 + w1 * d1 + w2 * d2 > 0.0
            {
                continue;
            }
            let z = w0 * p0.2 + w1 * p1.2 + w2 * p2.2;
            let i = (y * w + x) as usize;
            if z >= canvas.depth[i] {
                continue;
            }
            match (pass, layers.as_deref_mut()) {
                (Pass::Layer, Some(layers)) => {
                    for (sum, c) in layers.sum[i].iter_mut().zip(rgb) {
                        *sum += c;
                    }
                    layers.count[i] = layers.count[i].saturating_add(1);
                }
                _ => {
                    canvas.depth[i] = z;
                    canvas.colour[i] = Sample { rgb, alpha: 1.0 };
                }
            }
        }
    }
}

/// Lay the see-through layers over the solid picture: their mean colour,
/// covering more the more of them there are.
fn flatten_layers(canvas: &mut Canvas, layers: &Layers) {
    for (i, sample) in canvas.colour.iter_mut().enumerate() {
        let n = layers.count[i];
        if n == 0 {
            continue;
        }
        let mean = layers.sum[i].map(|s| s / f32::from(n));
        let cover = 1.0 - (1.0 - XRAY_ALPHA).powi(i32::from(n));
        let under = sample.alpha;
        let alpha = cover + under * (1.0 - cover);
        let mut rgb = [0.0; 3];
        for c in 0..3 {
            rgb[c] = (mean[c] * cover + sample.rgb[c] * under * (1.0 - cover)) / alpha;
        }
        *sample = Sample { rgb, alpha };
    }
}

/// A line `width` samples wide from `a` to `b`, drawn where it is no more
/// than `bias` behind what is already there.
fn line(
    canvas: &mut Canvas,
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    width: f32,
    bias: f32,
    rgb: [f32; 3],
) {
    let (w, h) = (canvas.w, canvas.h);
    let half = width * 0.5;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let min_x = (a.0.min(b.0) - half).floor().max(0.0) as i64;
    let max_x = ((a.0.max(b.0) + half).ceil() as i64).min(i64::from(w) - 1);
    let min_y = (a.1.min(b.1) - half).floor().max(0.0) as i64;
    let max_y = ((a.1.max(b.1) + half).ceil() as i64).min(i64::from(h) - 1);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if length2 > 1e-12 {
                (((px - a.0) * dx + (py - a.1) * dy) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
            if (px - cx).powi(2) + (py - cy).powi(2) > half * half {
                continue;
            }
            let z = a.2 + t * (b.2 - a.2);
            let i = (y as u32 * w + x as u32) as usize;
            if z <= canvas.depth[i] + bias {
                canvas.colour[i] = Sample { rgb, alpha: 1.0 };
            }
        }
    }
}

/// Average each block of samples into one pixel, over the background when
/// there is one.
fn resolve(
    canvas: &Canvas,
    width: u32,
    height: u32,
    ss: u32,
    background: Option<([f32; 3], [f32; 3])>,
) -> Vec<u8> {
    let mut out = vec![0u8; (width * height * 4) as usize];
    let n = (ss * ss) as f32;
    for y in 0..height {
        let behind = background.map(|(top, bottom)| {
            let t = (y as f32 + 0.5) / height as f32;
            [0, 1, 2].map(|c| top[c] + (bottom[c] - top[c]) * t)
        });
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = ((y * ss + sy) * canvas.w + x * ss + sx) as usize;
                    // Premultiplied while averaging, so the edge against the
                    // clear background does not darken.
                    let Sample { rgb, alpha } = canvas.colour[i];
                    sum[0] += rgb[0] * alpha;
                    sum[1] += rgb[1] * alpha;
                    sum[2] += rgb[2] * alpha;
                    sum[3] += alpha;
                }
            }
            let mut a = sum[3] / n;
            let mut rgb = [0.0f32; 3];
            for c in 0..3 {
                rgb[c] = if sum[3] > 0.0 { sum[c] / sum[3] } else { 0.0 };
            }
            if let Some(bg) = behind {
                for c in 0..3 {
                    rgb[c] = rgb[c] * a + bg[c] * (1.0 - a);
                }
                a = 1.0;
            }
            let o = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                out[o + c] = (rgb[c].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out[o + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A unit cube, twelve triangles wound outward, six faces numbered
    /// bottom, top, front (y = 0), right (x = 1), back, left, and its
    /// twelve edges.
    pub(crate) fn cube(offset: f32) -> Shape {
        let p = |x: f32, y: f32, z: f32| [x + offset, y, z];
        let positions = vec![
            p(0., 0., 0.),
            p(1., 0., 0.),
            p(1., 1., 0.),
            p(0., 1., 0.),
            p(0., 0., 1.),
            p(1., 0., 1.),
            p(1., 1., 1.),
            p(0., 1., 1.),
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ];
        let edges = vec![
            0, 1, 1, 2, 2, 3, 3, 0, 4, 5, 5, 6, 6, 7, 7, 4, 0, 4, 1, 5, 2, 6, 3, 7,
        ];
        Shape {
            mesh: Arc::new(TriMesh {
                normals: vec![[0.0, 0.0, 1.0]; positions.len()],
                positions,
                indices,
                faces: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
                edge_ids: (0..12).collect(),
                edges,
                ..TriMesh::default()
            }),
            color: [0.8, 0.2, 0.2],
            vertex_colours: false,
        }
    }

    fn alpha_at(rgba: &[u8], x: u32, y: u32) -> u8 {
        rgba[((y * WIDTH + x) * 4 + 3) as usize]
    }

    #[test]
    fn the_model_fills_the_middle_and_leaves_the_corners_clear() {
        let rgba = rasterize(&[cube(0.0)], Vec3::new(-1.0, 1.0, -1.0), Vec3::Z).unwrap();
        assert_eq!(alpha_at(&rgba, WIDTH / 2, HEIGHT / 2), 255);
        assert_eq!(alpha_at(&rgba, 0, 0), 0);
        assert_eq!(alpha_at(&rgba, WIDTH - 1, HEIGHT - 1), 0);
        // The cube's red shows through the shading.
        let o = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
        assert!(rgba[o] > rgba[o + 1] && rgba[o] > rgba[o + 2]);
    }

    #[test]
    fn two_bodies_side_by_side_are_both_in_frame() {
        // Seen from the front, looking along +Y with Z up: the second cube
        // sits to the right of the first, with a gap between them.
        let rgba = rasterize(&[cube(0.0), cube(3.0)], Vec3::Y, Vec3::Z).unwrap();
        let row = HEIGHT / 2;
        let covered: Vec<bool> = (0..WIDTH).map(|x| alpha_at(&rgba, x, row) > 0).collect();
        let runs = covered.windows(2).filter(|w| !w[0] && w[1]).count();
        assert_eq!(runs, 2, "two separate bodies across the middle row");
    }

    #[test]
    fn the_preview_is_a_png_and_an_empty_scene_has_none() {
        let png = render(&[cube(0.0)], Vec3::Y, Vec3::Z).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let decoded = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (WIDTH, HEIGHT));
        assert!(render(&[], Vec3::Y, Vec3::Z).is_none());
    }

    #[test]
    fn outlines_draw_over_the_faces_they_bound() {
        let shapes = [cube(0.0)];
        let look = Look {
            edges: Some([0.0, 0.0, 1.0]),
            ..Look::default()
        };
        let drawn = draw(&shapes, &shapes, Vec3::Y, Vec3::Z, 200, 200, &look).unwrap();
        // The front face's top edge runs across the picture at z = 1.
        let (x, y) = drawn.projection.to_pixel(Vec3::new(0.5, 0.0, 1.0));
        let o = ((y as u32 * 200 + x as u32) * 4) as usize;
        let px = &drawn.rgba[o..o + 3];
        assert!(px[2] > px[0] && px[2] > px[1], "the outline's blue: {px:?}");
    }

    #[test]
    fn a_see_through_body_shows_the_painted_face_behind_it() {
        let shapes = [cube(0.0)];
        let mut look = Look {
            xray: true,
            ..Look::default()
        };
        // The back face (y = 1), hidden behind the front one when solid.
        look.face_paint.insert((0, 4), [0.0, 1.0, 0.0]);
        let drawn = draw(&shapes, &shapes, Vec3::Y, Vec3::Z, 200, 200, &look).unwrap();
        let o = ((100 * 200 + 100) * 4) as usize;
        let px = &drawn.rgba[o..o + 3];
        assert!(
            px[1] > px[0] && px[1] > px[2],
            "the back face's green: {px:?}"
        );
    }
}
