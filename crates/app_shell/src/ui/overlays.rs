//! Painters for the viewport annotations workbenches emit: constant-width
//! lines, point markers and icon glyphs, text labels, and the orbit pivot.
//! Coordinates arrive in physical pixels relative to the viewport origin.

use egui::{Color32, Context};

pub fn rgb(color: [f32; 3], alpha: f32) -> Color32 {
    Color32::from_rgb(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
    )
    .gamma_multiply(alpha)
}

fn viewport_painter(ctx: &Context, viewport_rect: egui::Rect, id: &'static str) -> egui::Painter {
    // Background order draws beneath UI panels and on top of the 3D scene,
    // which is composited separately; clip to the viewport area.
    let layer_id = egui::LayerId::new(egui::Order::Background, egui::Id::new(id));
    ctx.layer_painter(layer_id).with_clip_rect(viewport_rect)
}

/// How far a line's dark rim reaches past it on each side, in pixels.
const RIM_PX: f32 = 1.0;
/// How dark the rim is at a line's full strength.
const RIM_ALPHA: f32 = 0.55;
/// The strength from which a line gets a rim: fainter lines (the grid,
/// guides) are background, and stay unrimmed.
const RIMMED_FROM: f32 = 0.6;

/// Where `overlay` starts and ends on screen.
fn ends(
    viewport_rect: egui::Rect,
    overlay: &core_document::ScreenSpaceOverlay,
    ppp: f32,
) -> (egui::Pos2, egui::Pos2) {
    (
        egui::pos2(
            viewport_rect.min.x + overlay.start[0] / ppp,
            viewport_rect.min.y + overlay.start[1] / ppp,
        ),
        egui::pos2(
            viewport_rect.min.x + overlay.end[0] / ppp,
            viewport_rect.min.y + overlay.end[1] / ppp,
        ),
    )
}

/// Draw the pictures a bench lays over the viewport, each a textured grid
/// between its corners laid out by [`picture_mesh`]; a picture not shown
/// any more lets its texture go.
pub fn draw_screen_space_images(
    ctx: &Context,
    viewport_rect: egui::Rect,
    images: &[core_document::ScreenSpaceImage],
    textures: &mut std::collections::HashMap<u64, egui::TextureHandle>,
) {
    textures.retain(|key, _| images.iter().any(|i| i.key == *key));
    if images.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_images");
    for image in images {
        let texture = textures.entry(image.key).or_insert_with(|| {
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [image.size[0] as usize, image.size[1] as usize],
                &image.rgba,
            );
            ctx.load_texture(
                format!("screen_image::{}", image.key),
                pixels,
                egui::TextureOptions::LINEAR,
            )
        });
        let tint = Color32::from_white_alpha((image.opacity.clamp(0.0, 1.0) * 255.0) as u8);
        let (points, indices) = picture_mesh(image.corners, PICTURE_TOLERANCE_PX);
        let mut mesh = egui::Mesh::with_texture(texture.id());
        mesh.vertices = points
            .into_iter()
            .map(|(pos, uv)| egui::epaint::Vertex {
                pos: egui::pos2(
                    viewport_rect.min.x + pos[0] / ppp,
                    viewport_rect.min.y + pos[1] / ppp,
                ),
                uv: egui::pos2(uv[0], uv[1]),
                color: tint,
            })
            .collect();
        mesh.indices = indices;
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// How far, in physical pixels, a picture's texels may stray from where
/// the perspective puts them.
const PICTURE_TOLERANCE_PX: f64 = 0.5;
/// The most cells a picture's grid takes along each side.
const PICTURE_MAX_CELLS: usize = 128;

/// A picture's grid point: where it lands on screen, in physical pixels,
/// and the texture coordinate it carries.
type PictureVertex = ([f32; 2], [f32; 2]);

/// The vertices (screen position in physical pixels, texture coordinate)
/// and triangles that lay a picture between its four screen corners (top
/// left, top right, bottom right, bottom left) as a flat rectangle seen in
/// perspective would. A triangle's texture coordinates run linearly across
/// the screen while a tilted plane's do not, so the picture is cut into a
/// grid of cells, each placed by the projective map the four corners
/// determine, fine enough that no texel lands farther than `tolerance`
/// pixels from its place (up to [`PICTURE_MAX_CELLS`] a side).
fn picture_mesh(corners: [[f32; 2]; 4], tolerance: f64) -> (Vec<PictureVertex>, Vec<u32>) {
    let map = SquareToQuad::new(corners);
    let cells = map.as_ref().map_or(1, |map| map.cells_within(tolerance));
    let at = |u: f64, v: f64| -> [f32; 2] {
        match &map {
            Some(map) => {
                let [x, y] = map.apply(u, v);
                [x as f32, y as f32]
            }
            // Not a picture seen in perspective: its corners joined as
            // they stand.
            None => {
                let lerp = |a: [f32; 2], b: [f32; 2], t: f32| {
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                };
                let (u, v) = (u as f32, v as f32);
                lerp(
                    lerp(corners[0], corners[1], u),
                    lerp(corners[3], corners[2], u),
                    v,
                )
            }
        }
    };
    let side = cells + 1;
    let mut points = Vec::with_capacity(side * side);
    for j in 0..side {
        let v = j as f64 / cells as f64;
        for i in 0..side {
            let u = i as f64 / cells as f64;
            points.push((at(u, v), [u as f32, v as f32]));
        }
    }
    let mut indices = Vec::with_capacity(cells * cells * 6);
    for j in 0..cells {
        for i in 0..cells {
            let a = (j * side + i) as u32;
            let b = a + 1;
            let d = a + side as u32;
            let c = d + 1;
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    (points, indices)
}

/// The projective map taking the unit square's corners (0,0), (1,0),
/// (1,1), (0,1) to a quad's four corners: how a flat rectangle's own
/// coordinates land on screen under a perspective camera.
struct SquareToQuad {
    /// `x = (a·u + b·v + c) / w`, `y = (d·u + e·v + f) / w`,
    /// `w = g·u + h·v + 1`.
    m: [f64; 8],
}

impl SquareToQuad {
    /// `None` for corners no rectangle in front of the camera projects to
    /// (three in a line, or folded over).
    fn new(corners: [[f32; 2]; 4]) -> Option<Self> {
        let [p0, p1, p2, p3] = corners.map(|c| [f64::from(c[0]), f64::from(c[1])]);
        let (dx1, dy1) = (p1[0] - p2[0], p1[1] - p2[1]);
        let (dx2, dy2) = (p3[0] - p2[0], p3[1] - p2[1]);
        let (dx3, dy3) = (p0[0] - p1[0] + p2[0] - p3[0], p0[1] - p1[1] + p2[1] - p3[1]);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-12 {
            return None;
        }
        let g = (dx3 * dy2 - dx2 * dy3) / den;
        let h = (dx1 * dy3 - dx3 * dy1) / den;
        let m = [
            p1[0] - p0[0] + g * p1[0],
            p3[0] - p0[0] + h * p3[0],
            p0[0],
            p1[1] - p0[1] + g * p1[1],
            p3[1] - p0[1] + h * p3[1],
            p0[1],
            g,
            h,
        ];
        // The denominator is affine in (u, v): positive at the corners
        // keeps it positive over the whole picture.
        let ws = [1.0, 1.0 + g, 1.0 + g + h, 1.0 + h];
        (ws.iter().all(|w| *w > 1e-9) && m.iter().all(|x| x.is_finite())).then_some(Self { m })
    }

    fn apply(&self, u: f64, v: f64) -> [f64; 2] {
        let [a, b, c, d, e, f, g, h] = self.m;
        let w = g * u + h * v + 1.0;
        [(a * u + b * v + c) / w, (d * u + e * v + f) / w]
    }

    /// The fewest cells a side (doubling from one) for which drawing each
    /// cell's two triangles linearly strays no more than `tolerance`
    /// pixels from the map anywhere. A triangle strays most at its edge
    /// midpoints and centre, and a cell the more the smaller `w` is there,
    /// which, `w` being affine, is smallest in a corner: the four corner
    /// cells are the ones checked.
    fn cells_within(&self, tolerance: f64) -> usize {
        let mut cells = 1;
        while cells < PICTURE_MAX_CELLS && self.worst_stray(cells) > tolerance {
            cells *= 2;
        }
        cells
    }

    fn worst_stray(&self, cells: usize) -> f64 {
        let step = 1.0 / cells as f64;
        let stray = |corners: &[(f64, f64)], weights: &[f64]| {
            let (mut u, mut v, mut x, mut y) = (0.0, 0.0, 0.0, 0.0);
            for (&(cu, cv), &k) in corners.iter().zip(weights) {
                let [px, py] = self.apply(cu, cv);
                u += k * cu;
                v += k * cv;
                x += k * px;
                y += k * py;
            }
            let [tx, ty] = self.apply(u, v);
            (x - tx).hypot(y - ty)
        };
        let half = [0.5, 0.5];
        let third = [1.0 / 3.0; 3];
        let last = cells - 1;
        let mut worst: f64 = 0.0;
        for j in [0, last] {
            for i in [0, last] {
                let (u0, v0) = (i as f64 * step, j as f64 * step);
                let (u1, v1) = (u0 + step, v0 + step);
                for s in [
                    stray(&[(u0, v0), (u1, v0)], &half),
                    stray(&[(u1, v0), (u1, v1)], &half),
                    stray(&[(u0, v0), (u1, v1)], &half),
                    stray(&[(u0, v1), (u1, v1)], &half),
                    stray(&[(u0, v0), (u0, v1)], &half),
                    stray(&[(u0, v0), (u1, v0), (u1, v1)], &third),
                    stray(&[(u0, v0), (u1, v1), (u0, v1)], &third),
                ] {
                    worst = worst.max(s);
                }
            }
        }
        worst
    }
}

/// Draw filled polygons, beneath the lines: each a fan of triangles from
/// its first point.
pub fn draw_screen_space_polygons(
    ctx: &Context,
    viewport_rect: egui::Rect,
    polygons: &[core_document::ScreenSpacePolygon],
) {
    if polygons.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_polygons");
    for polygon in polygons {
        let color = rgb(polygon.color, polygon.alpha);
        let mut mesh = egui::Mesh::default();
        for point in &polygon.points {
            mesh.colored_vertex(
                egui::pos2(
                    viewport_rect.min.x + point[0] / ppp,
                    viewport_rect.min.y + point[1] / ppp,
                ),
                color,
            );
        }
        for i in 1..polygon.points.len().saturating_sub(1) as u32 {
            mesh.add_triangle(0, i, i + 1);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Draw constant-thickness lines in the viewport area.
pub fn draw_screen_space_overlays(
    ctx: &Context,
    viewport_rect: egui::Rect,
    overlays: &[core_document::ScreenSpaceOverlay],
) {
    if overlays.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_overlays");
    // A dark rim under every strong line first, so a light line reads on a
    // light face and a coloured one on a face of its colour; the rims all
    // go down before any line, so none covers a neighbour.
    for overlay in overlays.iter().filter(|o| o.alpha >= RIMMED_FROM) {
        let (start, end) = ends(viewport_rect, overlay, ppp);
        let rim = egui::Stroke::new(
            (overlay.thickness + 2.0 * RIM_PX) / ppp,
            Color32::from_black_alpha((RIM_ALPHA * overlay.alpha * 255.0) as u8),
        );
        match overlay.dash {
            Some((dash, gap)) => {
                painter.add(egui::Shape::dashed_line(
                    &[start, end],
                    rim,
                    dash / ppp,
                    gap / ppp,
                ));
            }
            None => {
                painter.line_segment([start, end], rim);
            }
        }
    }
    for overlay in overlays {
        let start = egui::pos2(
            viewport_rect.min.x + overlay.start[0] / ppp,
            viewport_rect.min.y + overlay.start[1] / ppp,
        );
        let end = egui::pos2(
            viewport_rect.min.x + overlay.end[0] / ppp,
            viewport_rect.min.y + overlay.end[1] / ppp,
        );
        let stroke = egui::Stroke::new(overlay.thickness / ppp, rgb(overlay.color, overlay.alpha));
        match overlay.dash {
            Some((dash, gap)) => {
                painter.add(egui::Shape::dashed_line(
                    &[start, end],
                    stroke,
                    dash / ppp,
                    gap / ppp,
                ));
            }
            None => {
                painter.line_segment([start, end], stroke);
            }
        }
    }
}

/// Draw point markers and icon glyphs, above the lines and beneath the
/// labels.
pub fn draw_screen_space_marks(
    ctx: &Context,
    viewport_rect: egui::Rect,
    marks: &[core_document::ScreenSpaceMark],
) {
    if marks.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_marks");
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    for mark in marks {
        let pos = egui::pos2(
            viewport_rect.min.x + mark.pos[0] / ppp,
            viewport_rect.min.y + mark.pos[1] / ppp,
        );
        let color = rgb(mark.color, mark.alpha);
        match mark.kind {
            core_document::MarkKind::Dot { radius } => {
                painter.circle_filled(pos, radius / ppp, color);
            }
            core_document::MarkKind::Crosshair { size } => {
                let h = size / ppp / 2.0;
                let stroke = egui::Stroke::new(1.0, color);
                painter.line_segment(
                    [egui::pos2(pos.x - h, pos.y), egui::pos2(pos.x + h, pos.y)],
                    stroke,
                );
                painter.line_segment(
                    [egui::pos2(pos.x, pos.y - h), egui::pos2(pos.x, pos.y + h)],
                    stroke,
                );
            }
            core_document::MarkKind::Icon { name, size } => {
                if let Some(tex) = ui_kit::icon::texture(ctx, name) {
                    let rect = egui::Rect::from_center_size(pos, egui::Vec2::splat(size / ppp));
                    painter.image(tex.id(), rect, uv, color);
                }
            }
        }
    }
}

/// Draw text labels: dimension values, constraint suffixes, readouts.
pub fn draw_screen_space_labels(
    ctx: &Context,
    viewport_rect: egui::Rect,
    labels: &[core_document::ScreenSpaceLabel],
) {
    if labels.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_labels");
    for label in labels {
        let pos = egui::pos2(
            viewport_rect.min.x + label.pos[0] / ppp,
            viewport_rect.min.y + label.pos[1] / ppp,
        );
        let color = rgb(label.color, 1.0);
        let font = if label.mono {
            ui_kit::mono(label.size / ppp)
        } else {
            ui_kit::sans(label.size / ppp)
        };
        let galley = painter.layout_no_wrap(label.text.clone(), font, color);
        let rect = egui::Rect::from_center_size(pos, galley.size());
        if label.background {
            painter.rect_filled(
                rect.expand2(egui::Vec2::from(ui_kit::tokens::PILL_PAD) / ppp),
                ui_kit::tokens::RADIUS_SM,
                ui_kit::tokens::BG0,
            );
        }
        painter.galley(rect.min, galley, color);
    }
}

/// The orbit pivot marker: a small ringed dot where the next orbit turns.
pub fn draw_pivot_indicator(ctx: &Context, x: f32, y: f32) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        super::hud::VIEW_LAYER,
        egui::Id::new("pivot_indicator"),
    ));
    let ppp = ctx.pixels_per_point();
    let pos = egui::pos2(x / ppp, y / ppp);
    let accent = ui_kit::tokens::ACCENT;
    painter.circle(
        pos,
        8.0,
        ui_kit::tokens::ACCENT_DIM,
        egui::Stroke::new(1.5, accent),
    );
    let cross = 4.0;
    let stroke = egui::Stroke::new(1.5, ui_kit::tokens::TEXT1);
    painter.line_segment(
        [
            egui::pos2(pos.x - cross, pos.y),
            egui::pos2(pos.x + cross, pos.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(pos.x, pos.y - cross),
            egui::pos2(pos.x, pos.y + cross),
        ],
        stroke,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200 × 160 picture on a plane leaning steeply away from a pinhole
    /// camera at the origin looking along +z: its far edge ten times as
    /// deep as its near one.
    fn world(u: f64, v: f64) -> [f64; 3] {
        let (o, a, b) = (
            [-100.0, -80.0, 60.0],
            [180.0, 40.0, 30.0],
            [-20.0, 160.0, 540.0],
        );
        [0, 1, 2].map(|k| o[k] + u * a[k] + v * b[k])
    }

    fn screen(u: f64, v: f64) -> [f64; 2] {
        let p = world(u, v);
        [960.0 + 700.0 * p[0] / p[2], 540.0 + 700.0 * p[1] / p[2]]
    }

    fn corners() -> [[f32; 2]; 4] {
        [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| {
            let [x, y] = screen(u, v);
            [x as f32, y as f32]
        })
    }

    /// How far, in pixels, the texel a mesh shows at the screen point of
    /// each interior picture point lies from where the perspective puts
    /// it: the texture coordinate is interpolated linearly across the
    /// screen triangle holding the point, as the GPU does.
    fn worst_texel_stray(points: &[PictureVertex], indices: &[u32]) -> f64 {
        let p = |q: [f32; 2]| [f64::from(q[0]), f64::from(q[1])];
        let mut worst: f64 = 0.0;
        for j in 1..20 {
            for i in 1..20 {
                let s = screen(f64::from(i) / 20.0, f64::from(j) / 20.0);
                let uv = indices
                    .chunks(3)
                    .find_map(|t| {
                        let [a, b, c] = [0, 1, 2].map(|k| points[t[k] as usize]);
                        let (pa, pb, pc) = (p(a.0), p(b.0), p(c.0));
                        let det =
                            (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
                        let l1 = ((s[0] - pa[0]) * (pc[1] - pa[1])
                            - (pc[0] - pa[0]) * (s[1] - pa[1]))
                            / det;
                        let l2 = ((pb[0] - pa[0]) * (s[1] - pa[1])
                            - (s[0] - pa[0]) * (pb[1] - pa[1]))
                            / det;
                        let l0 = 1.0 - l1 - l2;
                        (l0 >= -1e-9 && l1 >= -1e-9 && l2 >= -1e-9).then(|| {
                            let (ua, ub, uc) = (p(a.1), p(b.1), p(c.1));
                            [0, 1].map(|k| l0 * ua[k] + l1 * ub[k] + l2 * uc[k])
                        })
                    })
                    .expect("the picture covers its own points");
                let shown = screen(uv[0], uv[1]);
                worst = worst.max((shown[0] - s[0]).hypot(shown[1] - s[1]));
            }
        }
        worst
    }

    #[test]
    fn a_tilted_picture_follows_the_perspective_within_a_pixel() {
        let (points, indices) = picture_mesh(corners(), PICTURE_TOLERANCE_PX);
        let stray = worst_texel_stray(&points, &indices);
        assert!(stray < 1.0, "texels stray {stray:.2} px");
    }

    #[test]
    fn two_triangles_bend_a_tilted_picture() {
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let points: Vec<_> = corners().into_iter().zip(uv).collect();
        let stray = worst_texel_stray(&points, &[0, 1, 2, 0, 2, 3]);
        assert!(stray > 20.0, "texels stray only {stray:.2} px");
    }

    #[test]
    fn a_picture_seen_square_on_is_two_triangles() {
        let (points, indices) = picture_mesh(
            [[10.0, 20.0], [310.0, 40.0], [290.0, 240.0], [-10.0, 220.0]],
            PICTURE_TOLERANCE_PX,
        );
        assert_eq!((points.len(), indices.len()), (4, 6));
    }
}
