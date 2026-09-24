//! The preview saved with a document: the visible bodies drawn on the CPU
//! from the direction the view looks, flat shaded on a clear background,
//! as a PNG. It runs in the save worker, so it never holds the window, and
//! needs nothing from the GPU.

use std::sync::Arc;

use glam::Vec3;
use kernel_api::TriMesh;

/// The preview's size, in pixels, wide as the start page's cards.
pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 180;
/// Drawn this many times larger and averaged down, for smooth edges.
const SUPERSAMPLE: u32 = 2;
/// Above this many triangles, every n-th is drawn: a thumbnail of an
/// assembly needs its silhouette, not its fillets.
const TRIANGLE_BUDGET: usize = 2_000_000;

/// One body to draw: its mesh and the colour it shows in.
pub struct Shape {
    pub mesh: Arc<TriMesh>,
    pub color: [f32; 3],
    /// The mesh's own per-vertex colours multiply `color`, as they do in
    /// the viewport for a body without a colour of its own.
    pub vertex_colours: bool,
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
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    // tiny-skia holds premultiplied colour.
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
    pixmap.encode_png().ok()
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
    let forward = forward.try_normalize()?;
    let right = forward.cross(up).try_normalize()?;
    let up = right.cross(forward);
    let project = |p: [f32; 3]| {
        let p = Vec3::from_array(p);
        Vec3::new(p.dot(right), p.dot(up), p.dot(forward))
    };

    // The model's extent on screen.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut triangles = 0usize;
    for shape in shapes {
        triangles += shape.mesh.indices.len() / 3;
    }
    for shape in framing {
        if shape.mesh.indices.len() < 3 {
            continue;
        }
        for p in &shape.mesh.positions {
            let q = project(*p);
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    if triangles == 0 || lo.x > hi.x {
        return None;
    }
    let (w, h) = (width * SUPERSAMPLE, height * SUPERSAMPLE);
    let margin = 0.08;
    let span_x = (hi.x - lo.x).max(1e-6);
    let span_y = (hi.y - lo.y).max(1e-6);
    let scale = ((w as f32 * (1.0 - 2.0 * margin)) / span_x)
        .min((h as f32 * (1.0 - 2.0 * margin)) / span_y);
    let centre = (lo + hi) * 0.5;
    let to_pixel = |q: Vec3| {
        (
            w as f32 * 0.5 + (q.x - centre.x) * scale,
            // Image rows run downward.
            h as f32 * 0.5 - (q.y - centre.y) * scale,
            q.z,
        )
    };

    let mut depth = vec![f32::INFINITY; (w * h) as usize];
    let mut colour = vec![[0.0f32; 4]; (w * h) as usize];
    let light = (-forward * 0.8 + up * 0.5 - right * 0.3).normalize();
    let stride = triangles.div_ceil(TRIANGLE_BUDGET).max(1);
    for shape in shapes {
        let mesh = &*shape.mesh;
        let vertex_colours = shape.vertex_colours && mesh.colors.len() == mesh.positions.len();
        for tri in mesh.indices.as_chunks::<3>().0.iter().step_by(stride) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| mesh.positions[i as usize]);
            let n = (Vec3::from_array(b) - Vec3::from_array(a))
                .cross(Vec3::from_array(c) - Vec3::from_array(a))
                .normalize_or_zero();
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
            let rgb = base.map(|c| (c * shade).clamp(0.0, 1.0));
            let [p0, p1, p2] = [a, b, c].map(|p| to_pixel(project(p)));
            fill(&mut depth, &mut colour, w, h, [p0, p1, p2], rgb);
        }
    }

    // Average each block of samples into one pixel.
    let mut out = vec![0u8; (width * height * 4) as usize];
    let n = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let i = ((y * SUPERSAMPLE + sy) * w + x * SUPERSAMPLE + sx) as usize;
                    // Premultiplied while averaging, so the edge against the
                    // clear background does not darken.
                    let [r, g, b, a] = colour[i];
                    sum[0] += r * a;
                    sum[1] += g * a;
                    sum[2] += b * a;
                    sum[3] += a;
                }
            }
            let a = sum[3] / n;
            let o = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                let straight = if sum[3] > 0.0 { sum[c] / sum[3] } else { 0.0 };
                out[o + c] = (straight * 255.0).round() as u8;
            }
            out[o + 3] = (a * 255.0).round() as u8;
        }
    }
    Some(out)
}

/// Fill one triangle into the depth and colour buffers, nearest wins.
fn fill(
    depth: &mut [f32],
    colour: &mut [[f32; 4]],
    w: u32,
    h: u32,
    [p0, p1, p2]: [(f32, f32, f32); 3],
    rgb: [f32; 3],
) {
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
            let z = w0 * p0.2 + w1 * p1.2 + w2 * p2.2;
            let i = (y * w + x) as usize;
            if z < depth[i] {
                depth[i] = z;
                colour[i] = [rgb[0], rgb[1], rgb[2], 1.0];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube, twelve triangles.
    fn cube(offset: f32) -> Shape {
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
        Shape {
            mesh: Arc::new(TriMesh {
                normals: vec![[0.0, 0.0, 1.0]; positions.len()],
                positions,
                indices,
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
    fn an_empty_scene_has_no_preview() {
        assert!(render(&[], Vec3::Y, Vec3::Z).is_none());
    }

    #[test]
    fn the_preview_is_a_png() {
        let png = render(&[cube(0.0)], Vec3::Y, Vec3::Z).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let decoded = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (WIDTH, HEIGHT));
    }
}
