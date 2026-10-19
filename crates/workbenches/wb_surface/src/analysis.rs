//! How a body's surfaces bend, painted over it: a curvature map (the
//! Gaussian, mean, largest or smallest curvature at each point, from the
//! kernel's exact surfaces) or zebra stripes (bands of the angle the
//! surface's normal turns about an axis, from the mesh's normals). A
//! stripe kinks where faces meet with a crease and jumps in width where
//! their curvature does, which is how G1 and G2 joins read by eye.

use std::sync::{Arc, Mutex};

use core_document::{BodyId, Document, OverlayMesh, SketchPalette};
use kernel_api::{KernelQueries, TriMesh};
use serde::{Deserialize, Serialize};

/// Which curvature a map shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Measure {
    /// The product of the two principal curvatures: positive on a dome,
    /// negative on a saddle, zero on a plane and on what unrolls flat.
    #[default]
    Gaussian,
    /// Their mean.
    Mean,
    /// The largest.
    Max,
    /// The smallest.
    Min,
}

impl Measure {
    pub const ALL: [Measure; 4] = [Measure::Gaussian, Measure::Mean, Measure::Max, Measure::Min];

    pub fn label(self) -> &'static str {
        match self {
            Measure::Gaussian => "Gaussian",
            Measure::Mean => "Mean",
            Measure::Max => "Largest",
            Measure::Min => "Smallest",
        }
    }

    /// The command's name for it.
    pub fn name(self) -> &'static str {
        match self {
            Measure::Gaussian => "gaussian",
            Measure::Mean => "mean",
            Measure::Max => "max",
            Measure::Min => "min",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// Its unit, as the legend writes it.
    pub fn unit(self) -> &'static str {
        match self {
            Measure::Gaussian => "1/mm²",
            _ => "1/mm",
        }
    }

    fn of(self, [max, min]: [f64; 2]) -> f64 {
        match self {
            Measure::Gaussian => max * min,
            Measure::Mean => 0.5 * (max + min),
            Measure::Max => max,
            Measure::Min => min,
        }
    }
}

/// The axis zebra stripes run along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StripeAxis {
    X,
    Y,
    #[default]
    Z,
}

impl StripeAxis {
    pub const ALL: [StripeAxis; 3] = [StripeAxis::X, StripeAxis::Y, StripeAxis::Z];

    pub fn label(self) -> &'static str {
        match self {
            StripeAxis::X => "X",
            StripeAxis::Y => "Y",
            StripeAxis::Z => "Z",
        }
    }

    /// The axis and two directions square to it and to each other.
    fn frame(self) -> [[f64; 3]; 3] {
        match self {
            StripeAxis::X => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            StripeAxis::Y => [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            StripeAxis::Z => [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        }
    }
}

/// How many dark stripes a zebra display starts with, per half turn.
pub const DEFAULT_STRIPES: u32 = 6;

/// What is painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    /// A curvature map, its colours running from `-limit` to `limit`; a
    /// limit of zero takes it from the body.
    Curvature { measure: Measure, limit: f64 },
    /// Zebra stripes: `stripes` dark bands per half turn of the normal.
    Zebra { stripes: u32, axis: StripeAxis },
}

/// A body painted with an analysis.
pub struct Analysis {
    pub body: BodyId,
    pub name: String,
    pub display: Display,
    /// The paint for the body's mesh as it was last read, and what it was
    /// made from.
    painted: Mutex<Option<Painted>>,
}

/// A painted mesh and what it was made from.
struct Painted {
    key: (u64, usize, DisplayKey),
    mesh: Arc<TriMesh>,
    /// The measure's range over the body, for a curvature map.
    range: Option<(f64, f64)>,
}

type DisplayKey = (u8, u64, u32, u8);

fn key_of(display: Display) -> DisplayKey {
    match display {
        Display::Curvature { measure, limit } => (0, limit.to_bits(), 0, measure as u8),
        Display::Zebra { stripes, axis } => (1, 0, stripes, axis as u8),
    }
}

impl Analysis {
    pub fn new(body: BodyId, name: String, display: Display) -> Self {
        Self {
            body,
            name,
            display,
            painted: Mutex::new(None),
        }
    }

    /// The body painted as the analysis shows it, made again when the body
    /// or the display changed; `None` when the body has no mesh, or the
    /// map no kernel to read curvature from.
    pub fn overlay(
        &self,
        document: &Document,
        kernel: Option<&dyn KernelQueries>,
        palette: &SketchPalette,
    ) -> Option<OverlayMesh> {
        let painted = self.paint(document, kernel, palette)?;
        Some(OverlayMesh {
            mesh: painted,
            revision: None,
            color: [1.0, 1.0, 1.0],
            wireframe: false,
            opacity: 1.0,
            on_top: false,
        })
    }

    /// The curvature map's range over the body, low to high.
    pub fn range(
        &self,
        document: &Document,
        kernel: Option<&dyn KernelQueries>,
        palette: &SketchPalette,
    ) -> Option<(f64, f64)> {
        self.paint(document, kernel, palette)?;
        self.painted.lock().ok()?.as_ref()?.range
    }

    fn paint(
        &self,
        document: &Document,
        kernel: Option<&dyn KernelQueries>,
        palette: &SketchPalette,
    ) -> Option<Arc<TriMesh>> {
        let placed = document.imported_geometry(self.body)?;
        let key = (
            placed.revision,
            Arc::as_ptr(&placed.mesh) as usize,
            key_of(self.display),
        );
        let mut slot = self.painted.lock().ok()?;
        if let Some(p) = slot.as_ref()
            && p.key == key
        {
            return Some(Arc::clone(&p.mesh));
        }
        let world = &placed.mesh;
        let (mesh, range) = match self.display {
            Display::Zebra { stripes, axis } => (zebra(world, stripes, axis, palette), None),
            Display::Curvature { measure, limit } => {
                let (local, _) = document.local_geometry(self.body)?;
                let brep = document.imported_brep_blob(self.body)?;
                let values = curvature_values(&local, brep, kernel?, measure)?;
                let range = spread(&values);
                let limit = if limit > 0.0 {
                    limit
                } else {
                    auto_limit(&values)
                };
                (curvature_map(world, &values, limit, palette), range)
            }
        };
        let mesh = Arc::new(mesh);
        *slot = Some(Painted {
            key,
            mesh: Arc::clone(&mesh),
            range,
        });
        Some(mesh)
    }
}

/// The measure at every vertex of `mesh` (the body's own, in the
/// snapshot's frame), from the kernel's exact surfaces; `None` where the
/// kernel read none.
pub fn curvature_values(
    mesh: &TriMesh,
    brep: &[u8],
    kernel: &dyn KernelQueries,
    measure: Measure,
) -> Option<Vec<Option<f64>>> {
    if mesh.faces.len() * 3 != mesh.indices.len() {
        return None;
    }
    // Each vertex asked of the first face whose triangles use it, each
    // face named by its first triangle's middle.
    let mut owner: Vec<Option<usize>> = vec![None; mesh.positions.len()];
    let mut groups: Vec<([f64; 3], Vec<usize>)> = Vec::new();
    let mut group_of: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let face = mesh.faces[t];
        let group = *group_of.entry(face).or_insert_with(|| {
            let p = [tri[0], tri[1], tri[2]].map(|i| mesh.positions[i as usize]);
            let mid = [0, 1, 2].map(|k| f64::from(p[0][k] + p[1][k] + p[2][k]) / 3.0);
            groups.push((mid, Vec::new()));
            groups.len() - 1
        });
        for &i in tri {
            let slot = &mut owner[i as usize];
            if slot.is_none() {
                *slot = Some(group);
                groups[group].1.push(i as usize);
            }
        }
    }
    let asked: Vec<kernel_api::FacePoints> = groups
        .iter()
        .map(|(mid, vertices)| {
            (
                *mid,
                vertices
                    .iter()
                    .map(|&i| {
                        (
                            mesh.positions[i].map(f64::from),
                            mesh.normals[i].map(f64::from),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let answers = kernel.curvature(brep, &asked).ok()?;
    let mut values = vec![None; mesh.positions.len()];
    for ((_, vertices), read) in groups.iter().zip(answers) {
        for (&i, k) in vertices.iter().zip(read) {
            values[i] = k.map(|k| measure.of(k));
        }
    }
    Some(values)
}

/// The lowest and highest of the values read.
fn spread(values: &[Option<f64>]) -> Option<(f64, f64)> {
    values.iter().flatten().fold(None, |acc, &v| match acc {
        None => Some((v, v)),
        Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
    })
}

/// A limit for the colours that a few sharp corners do not wash out: the
/// size most values stay within.
fn auto_limit(values: &[Option<f64>]) -> f64 {
    let mut sizes: Vec<f64> = values.iter().flatten().map(|v| v.abs()).collect();
    if sizes.is_empty() {
        return 1.0;
    }
    sizes.sort_by(f64::total_cmp);
    let at = ((sizes.len() - 1) as f64 * 0.95).round() as usize;
    let limit = sizes[at];
    if limit > 1e-12 { limit } else { 1.0 }
}

/// The colour a value takes: the low colour at `-limit`, the middle one at
/// zero, the high one at `limit`.
pub fn ramp(value: f64, limit: f64, palette: &SketchPalette) -> [f32; 3] {
    let t = (value / limit).clamp(-1.0, 1.0) as f32;
    let (from, to, f) = if t < 0.0 {
        (palette.analysis_mid, palette.analysis_low, -t)
    } else {
        (palette.analysis_mid, palette.analysis_high, t)
    };
    [0, 1, 2].map(|k| from[k] * (1.0 - f) + to[k] * f)
}

/// How far the paint stands off the surface, so it draws over the body
/// rather than through it: a millionth of the body's size each way.
fn standoff(mesh: &TriMesh) -> f32 {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let size = (0..3).map(|k| (hi[k] - lo[k]).max(0.0)).fold(0.0, f32::max);
    (size * 1e-4).max(1e-4)
}

/// `mesh` coloured vertex by vertex, drawn on both sides of the surface.
fn curvature_map(
    mesh: &TriMesh,
    values: &[Option<f64>],
    limit: f64,
    palette: &SketchPalette,
) -> TriMesh {
    let colors: Vec<[f32; 3]> = values
        .iter()
        .map(|v| match v {
            Some(v) => ramp(*v, limit, palette),
            None => palette.inactive,
        })
        .collect();
    let painted = TriMesh {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        indices: mesh.indices.clone(),
        colors,
        ..TriMesh::default()
    };
    both_sides(&painted, standoff(mesh))
}

/// The mesh lifted `by` along its normals, and again the other way, so a
/// sheet shows its paint from either side.
fn both_sides(mesh: &TriMesh, by: f32) -> TriMesh {
    let mut out = TriMesh::default();
    for sign in [1.0f32, -1.0] {
        let base = out.positions.len() as u32;
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            out.positions
                .push([0, 1, 2].map(|k| p[k] + n[k] * by * sign));
            out.normals.push(n.map(|c| c * sign));
        }
        out.colors.extend_from_slice(&mesh.colors);
        for tri in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i + base);
            if sign > 0.0 {
                out.indices.extend([a, b, c]);
            } else {
                out.indices.extend([a, c, b]);
            }
        }
    }
    out
}

/// The angle the normal `n` turns about the axis, in half turns.
fn turn(n: [f32; 3], frame: &[[f64; 3]; 3]) -> f64 {
    let n = n.map(f64::from);
    let dot = |d: [f64; 3]| n[0] * d[0] + n[1] * d[1] + n[2] * d[2];
    dot(frame[2]).atan2(dot(frame[1])) / std::f64::consts::PI
}

/// `mesh` cut into zebra stripes: bands of the angle its normal turns
/// about `axis`, `stripes` dark and as many light per half turn, each
/// triangle split along the bands' borders so the stripes keep sharp
/// edges however coarse the mesh.
pub fn zebra(mesh: &TriMesh, stripes: u32, axis: StripeAxis, palette: &SketchPalette) -> TriMesh {
    let frame = axis.frame();
    let bands = 2.0 * f64::from(stripes.max(1));
    let mut out = TriMesh::default();
    for tri in mesh.indices.as_chunks::<3>().0 {
        let corners = [tri[0], tri[1], tri[2]].map(|i| i as usize);
        let mut t = corners.map(|i| turn(mesh.normals[i], &frame) * bands);
        // Across the turn's wrap, the corners read on one side of it.
        let whole = 2.0 * bands;
        let top = t.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        for v in &mut t {
            if top - *v > bands {
                *v += whole;
            }
        }
        let lo = t.iter().copied().fold(f64::INFINITY, f64::min).floor() as i64;
        let hi = t.iter().copied().fold(f64::NEG_INFINITY, f64::max).floor() as i64;
        for band in lo..=hi {
            let piece = clip_band(&t, band as f64);
            if piece.len() < 3 {
                continue;
            }
            let color = if band.rem_euclid(2) == 0 {
                palette.stripe_dark
            } else {
                palette.stripe_light
            };
            let base = out.positions.len() as u32;
            for w in &piece {
                let mut p = [0.0f32; 3];
                let mut n = [0.0f32; 3];
                for (corner, weight) in corners.iter().zip(w) {
                    for k in 0..3 {
                        p[k] += mesh.positions[*corner][k] * *weight as f32;
                        n[k] += mesh.normals[*corner][k] * *weight as f32;
                    }
                }
                out.positions.push(p);
                out.normals.push(normalized(n));
                out.colors.push(color);
            }
            for k in 1..piece.len() as u32 - 1 {
                out.indices.extend([base, base + k, base + k + 1]);
            }
        }
    }
    both_sides(&out, standoff(mesh))
}

fn normalized(n: [f32; 3]) -> [f32; 3] {
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length > 1e-12 {
        n.map(|c| c / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// The part of a triangle whose corners carry `t` that lies in
/// `band ≤ t ≤ band + 1`, as barycentric weights of its corners, in order
/// round it.
fn clip_band(t: &[f64; 3], band: f64) -> Vec<[f64; 3]> {
    let corners = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let poly: Vec<([f64; 3], f64)> = corners.into_iter().zip(*t).collect();
    let poly = clip(&poly, |v| v - band);
    let poly = clip(&poly, |v| band + 1.0 - v);
    poly.into_iter().map(|(w, _)| w).collect()
}

/// A polygon cut to where `side` is not negative.
fn clip(poly: &[([f64; 3], f64)], side: impl Fn(f64) -> f64) -> Vec<([f64; 3], f64)> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for (i, &(w, v)) in poly.iter().enumerate() {
        let (nw, nv) = poly[(i + 1) % poly.len()];
        let (a, b) = (side(v), side(nv));
        if a >= 0.0 {
            out.push((w, v));
        }
        if (a >= 0.0) != (b >= 0.0) {
            let f = a / (a - b);
            out.push((
                [0, 1, 2].map(|k| w[k] + (nw[k] - w[k]) * f),
                v + (nv - v) * f,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Half a cylinder of radius 5 about Z, as a fan of quads.
    fn half_tube(segments: usize) -> TriMesh {
        let mut mesh = TriMesh::default();
        for i in 0..=segments {
            let a = std::f32::consts::PI * i as f32 / segments as f32;
            let n = [a.cos(), a.sin(), 0.0];
            for z in [0.0, 10.0] {
                mesh.positions.push([5.0 * n[0], 5.0 * n[1], z]);
                mesh.normals.push(n);
            }
        }
        for i in 0..segments as u32 {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            mesh.indices.extend([a, c, b, b, c, d]);
        }
        mesh
    }

    fn area(mesh: &TriMesh, color: [f32; 3]) -> f32 {
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| mesh.colors[t[0] as usize] == color)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[t[k] as usize]);
                let (u, v) = (
                    [0, 1, 2].map(|i| b[i] - a[i]),
                    [0, 1, 2].map(|i| c[i] - a[i]),
                );
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
            })
            .sum()
    }

    /// Half a turn of a round surface about the stripes' axis crosses
    /// each stripe once: as many dark as light, of even width, however
    /// coarse the mesh, and both sides painted.
    #[test]
    fn stripes_split_a_round_surface_evenly() {
        let palette = SketchPalette::default();
        for segments in [7, 48] {
            let mesh = half_tube(segments);
            let painted = zebra(&mesh, 4, StripeAxis::Z, &palette);
            let (dark, light) = (
                area(&painted, palette.stripe_dark),
                area(&painted, palette.stripe_light),
            );
            // The fan is a polygon round the half circle, both sides.
            let whole = 2.0
                * 10.0
                * 2.0
                * 5.0
                * (std::f32::consts::PI / (2.0 * segments as f32)).sin()
                * segments as f32;
            assert!(
                ((dark + light) - whole).abs() < 1e-3 * whole,
                "{dark} {light} {whole}"
            );
            assert!(
                (dark - light).abs() < 0.02 * whole,
                "{segments}: {dark} {light}"
            );
        }
    }

    /// A flat face takes one stripe.
    #[test]
    fn a_flat_face_is_one_stripe() {
        let palette = SketchPalette::default();
        let mesh = TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
            normals: vec![[1.0, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        let painted = zebra(&mesh, 4, StripeAxis::Z, &palette);
        assert_eq!(painted.indices.len(), 6, "the triangle, once each side");
    }

    #[test]
    fn the_ramp_runs_low_through_flat_to_high() {
        let palette = SketchPalette::default();
        assert_eq!(ramp(-2.0, 1.0, &palette), palette.analysis_low);
        assert_eq!(ramp(0.0, 1.0, &palette), palette.analysis_mid);
        assert_eq!(ramp(1.0, 1.0, &palette), palette.analysis_high);
    }

    #[test]
    fn a_few_sharp_corners_do_not_set_the_limit() {
        let mut values: Vec<Option<f64>> = (0..100).map(|_| Some(0.2)).collect();
        values[0] = Some(50.0);
        assert!((auto_limit(&values) - 0.2).abs() < 1e-12);
    }
}
