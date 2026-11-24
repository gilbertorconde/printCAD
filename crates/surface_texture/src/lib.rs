//! Surface textures: a height pattern pressed into chosen faces of a
//! mesh, for printing. The exact solid is untouched; the texture lives in
//! the mesh the view draws and the file for the slicer is written from.
//!
//! [`apply`] welds the mesh, splits the chosen faces' triangles (and their
//! neighbours along shared edges, so no crack opens) until their edges are
//! short enough for the pattern, then moves each point of those faces
//! along the surface's normal by the pattern's height there. A texture's
//! rim does not move, so a textured face meets its neighbours exactly.

mod pattern;

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use kernel_api::TriMesh;

pub use core_document::texture::{Axis, Projection, Texture};
pub use pattern::{HeightMap, Pattern, height};

fn axis_vector(axis: Axis) -> Vec3 {
    match axis {
        Axis::X => Vec3::X,
        Axis::Y => Vec3::Y,
        Axis::Z => Vec3::Z,
    }
}

/// Two directions square to the axis and to each other: a plane's.
fn axis_plane(axis: Axis) -> (Vec3, Vec3) {
    match axis {
        Axis::X => (Vec3::Y, Vec3::Z),
        Axis::Y => (Vec3::Z, Vec3::X),
        Axis::Z => (Vec3::X, Vec3::Y),
    }
}

/// One texture laid on some faces of the mesh.
#[derive(Debug, Clone, Copy)]
pub struct Job<'a> {
    pub texture: &'a Texture,
    /// The faces it covers, by their index in the mesh; empty for all.
    pub faces: &'a [u32],
    /// A picture pattern's heights.
    pub image: Option<&'a HeightMap>,
}

/// How fine the textured faces are made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detail {
    /// Edges to a tile's side: more follows the pattern closer.
    pub edges_per_tile: f32,
    /// The shortest edge made, mm.
    pub min_edge_mm: f32,
    /// The most triangles made: past it, edges are left longer.
    pub max_triangles: usize,
}

impl Detail {
    /// For the view: quick, the pattern's shape clear.
    pub const PREVIEW: Detail = Detail {
        edges_per_tile: 8.0,
        min_edge_mm: 0.15,
        max_triangles: 600_000,
    };
    /// For the slicer: close to the pattern, within what a slicer takes.
    pub const EXPORT: Detail = Detail {
        edges_per_tile: 20.0,
        min_edge_mm: 0.08,
        max_triangles: 6_000_000,
    };
}

/// The mesh with every job's texture pressed into its faces.
pub fn apply(mesh: &TriMesh, jobs: &[Job], detail: Detail) -> TriMesh {
    if jobs.is_empty() || mesh.indices.is_empty() {
        return mesh.clone();
    }
    let mut work = Work::weld(mesh);
    work.assign(jobs);
    if work.job.iter().all(Option::is_none) {
        return mesh.clone();
    }
    let targets: Vec<f32> = jobs
        .iter()
        .map(|job| (job.texture.tile_mm / detail.edges_per_tile).max(detail.min_edge_mm))
        .collect();
    work.refine(&targets, detail.max_triangles);
    let heights = work.displace(jobs);
    work.into_mesh(mesh, &heights)
}

/// Points a ten-thousandth of a millimetre apart are one point to a press.
/// A mirrored copy's mesh comes back into its source's frame with a few
/// `f32` steps of rounding, about 1e-5 mm on a part 100 mm across, and a
/// point left double there would open a crack once the press moves it;
/// no pattern, tenths of a millimetre deep on millimetre tiles, draws
/// points this close apart.
const PRESSED_POINT: kernel_api::PointMatch = kernel_api::PointMatch::Grid { per_mm: 1.0e4 };

/// The mesh being worked: welded points, triangles with their face, their
/// job and the way out of the solid.
struct Work {
    points: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    face: Vec<u32>,
    /// Which job textures the triangle; `None` for none (or kept flat).
    job: Vec<Option<usize>>,
}

impl Work {
    /// Weld points at one spot into one and wind every triangle outward,
    /// by the mesh's normals.
    fn weld(mesh: &TriMesh) -> Self {
        let mut at: HashMap<[i64; 3], u32> = HashMap::new();
        let mut points = Vec::new();
        let mut index = Vec::with_capacity(mesh.positions.len());
        for p in &mesh.positions {
            let key = PRESSED_POINT.key(*p);
            let p = Vec3::from_array(*p);
            let id = *at.entry(key).or_insert_with(|| {
                points.push(p);
                points.len() as u32 - 1
            });
            index.push(id);
        }
        let mut triangles = Vec::with_capacity(mesh.indices.len() / 3);
        let mut face = Vec::with_capacity(mesh.indices.len() / 3);
        for (n, t) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| index[i as usize]);
            if a == b || b == c || a == c {
                continue;
            }
            let out: Vec3 = [t[0], t[1], t[2]]
                .iter()
                .filter_map(|i| mesh.normals.get(*i as usize))
                .map(|n| Vec3::from_array(*n))
                .sum();
            let (pa, pb, pc) = (points[a as usize], points[b as usize], points[c as usize]);
            let turned = (pb - pa).cross(pc - pa).dot(out) < 0.0;
            triangles.push(if turned { [a, c, b] } else { [a, b, c] });
            face.push(mesh.faces.get(n).copied().unwrap_or(0));
        }
        let job = vec![None; triangles.len()];
        Self {
            points,
            triangles,
            face,
            job,
        }
    }

    /// Mark each triangle with the job whose faces hold it, but for those
    /// a job keeps flat.
    fn assign(&mut self, jobs: &[Job]) {
        for (t, tri) in self.triangles.iter().enumerate() {
            let normal = self.normal_of(tri);
            self.job[t] = jobs.iter().position(|job| {
                (job.faces.is_empty() || job.faces.contains(&self.face[t]))
                    && !kept_flat(job.texture, normal)
            });
        }
    }

    fn normal_of(&self, tri: &[u32; 3]) -> Vec3 {
        let [a, b, c] = tri.map(|i| self.points[i as usize]);
        (b - a).cross(c - a).normalize_or_zero()
    }

    /// Split textured triangles' edges longer than their job's target, and
    /// every triangle sharing a split edge, until none is left or the cap
    /// is reached.
    fn refine(&mut self, targets: &[f32], max_triangles: usize) {
        loop {
            let mut long: HashMap<(u32, u32), u32> = HashMap::new();
            for (t, tri) in self.triangles.iter().enumerate() {
                let Some(job) = self.job[t] else {
                    continue;
                };
                for k in 0..3 {
                    let (a, b) = (tri[k], tri[(k + 1) % 3]);
                    let length = self.points[a as usize].distance(self.points[b as usize]);
                    if length > targets[job] {
                        long.insert(edge(a, b), u32::MAX);
                    }
                }
            }
            // Each split edge adds about two triangles; stop short of the
            // cap rather than cross it.
            if long.is_empty() || self.triangles.len() + 2 * long.len() > max_triangles {
                return;
            }
            for (&(a, b), mid) in long.iter_mut() {
                let m = (self.points[a as usize] + self.points[b as usize]) * 0.5;
                self.points.push(m);
                *mid = self.points.len() as u32 - 1;
            }
            let mut triangles = Vec::with_capacity(self.triangles.len() * 2);
            let mut face = Vec::with_capacity(self.triangles.len() * 2);
            let mut job = Vec::with_capacity(self.triangles.len() * 2);
            for (t, tri) in self.triangles.iter().enumerate() {
                for child in split(*tri, &long) {
                    triangles.push(child);
                    face.push(self.face[t]);
                    job.push(self.job[t]);
                }
            }
            self.triangles = triangles;
            self.face = face;
            self.job = job;
        }
    }

    /// Move each textured point along its normal by its job's pattern, and
    /// say how far each point moved.
    fn displace(&mut self, jobs: &[Job]) -> Vec<f32> {
        let n = self.points.len();
        let mut normal = vec![Vec3::ZERO; n];
        let mut job_of: Vec<Option<usize>> = vec![None; n];
        // A point any untextured triangle touches is a rim: it stays.
        let mut rim = vec![false; n];
        for (t, tri) in self.triangles.iter().enumerate() {
            let [a, b, c] = tri.map(|i| self.points[i as usize]);
            let weighted = (b - a).cross(c - a);
            for &i in tri {
                match self.job[t] {
                    Some(j) => {
                        normal[i as usize] += weighted;
                        job_of[i as usize].get_or_insert(j);
                    }
                    None => rim[i as usize] = true,
                }
            }
        }
        let frames: Vec<Frame> = jobs
            .iter()
            .enumerate()
            .map(|(j, job)| Frame::of(job.texture, &self.points, &job_of, j))
            .collect();
        let mut moved = vec![0.0; n];
        for i in 0..n {
            let Some(j) = job_of[i] else {
                continue;
            };
            if rim[i] {
                continue;
            }
            let dir = normal[i].normalize_or_zero();
            let job = &jobs[j];
            let h = frames[j].height(job, self.points[i], dir);
            let depth = job.texture.depth_mm * h;
            let depth = if job.texture.inward { -depth } else { depth };
            self.points[i] += dir * depth;
            moved[i] = depth;
        }
        moved
    }

    /// The finished mesh: points unwelded per face, so each face's normals
    /// are its own and an untextured corner stays sharp; outlines between
    /// faces wherever nothing moved.
    fn into_mesh(self, source: &TriMesh, moved: &[f32]) -> TriMesh {
        let mut out = TriMesh {
            face_names: source.face_names.clone(),
            face_other_names: source.face_other_names.clone(),
            face_makers: source.face_makers.clone(),
            face_surfaces: source.face_surfaces.clone(),
            ..TriMesh::default()
        };
        let mut slot: HashMap<(u32, u32), u32> = HashMap::new();
        let mut sums: Vec<Vec3> = Vec::new();
        for (t, tri) in self.triangles.iter().enumerate() {
            let face = self.face[t];
            let [a, b, c] = tri.map(|i| self.points[i as usize]);
            let weighted = (b - a).cross(c - a);
            for &i in tri {
                let id = *slot.entry((i, face)).or_insert_with(|| {
                    out.positions.push(self.points[i as usize].to_array());
                    sums.push(Vec3::ZERO);
                    out.positions.len() as u32 - 1
                });
                sums[id as usize] += weighted;
                out.indices.push(id);
            }
            out.faces.push(face);
        }
        out.normals = sums
            .iter()
            .map(|n| n.normalize_or_zero().to_array())
            .collect();
        // An edge between two faces is an outline where neither end moved.
        let mut sides: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for (t, tri) in self.triangles.iter().enumerate() {
            for k in 0..3 {
                sides
                    .entry(edge(tri[k], tri[(k + 1) % 3]))
                    .or_default()
                    .push(self.face[t]);
            }
        }
        for ((a, b), faces) in sides {
            let across = faces.windows(2).any(|w| w[0] != w[1]) || faces.len() == 1;
            if across && moved[a as usize] == 0.0 && moved[b as usize] == 0.0 {
                let face = faces[0];
                if let (Some(&ia), Some(&ib)) = (slot.get(&(a, face)), slot.get(&(b, face))) {
                    out.edges.extend_from_slice(&[ia, ib]);
                }
            }
        }
        out
    }
}

/// An edge by its points, the lower first.
fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// A triangle with some of its edges split at the points `mids` names:
/// two, three or four triangles wound as it was.
fn split(tri: [u32; 3], mids: &HashMap<(u32, u32), u32>) -> Vec<[u32; 3]> {
    let mid = |k: usize| mids.get(&edge(tri[k], tri[(k + 1) % 3])).copied();
    let cut: Vec<usize> = (0..3).filter(|k| mid(*k).is_some()).collect();
    // Turned so the first split edge runs from `a` to `b`.
    let turn = |k: usize| [tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]];
    match cut.as_slice() {
        [] => vec![tri],
        [k] => {
            let [a, b, c] = turn(*k);
            let m = mid(*k).unwrap_or(a);
            vec![[a, m, c], [m, b, c]]
        }
        [k0, k1] => {
            // The unsplit edge is the one neither names; turn so it is
            // the third (c to a), the split ones a-b and b-c.
            let unsplit = 3 - k0 - k1;
            let start = (unsplit + 1) % 3;
            let [a, b, c] = turn(start);
            let (mab, mbc) = (mid(start).unwrap_or(a), mid((start + 1) % 3).unwrap_or(b));
            vec![[mab, b, mbc], [a, mab, mbc], [a, mbc, c]]
        }
        _ => {
            let [a, b, c] = tri;
            let (mab, mbc, mca) = (
                mid(0).unwrap_or(a),
                mid(1).unwrap_or(b),
                mid(2).unwrap_or(c),
            );
            vec![[a, mab, mca], [mab, b, mbc], [mca, mbc, c], [mab, mbc, mca]]
        }
    }
}

/// Whether a triangle facing `normal` is one `texture` keeps smooth.
fn kept_flat(texture: &Texture, normal: Vec3) -> bool {
    texture.keep_flat_deg > 0.0 && normal.z.abs() >= texture.keep_flat_deg.to_radians().cos()
}

/// Where a job's projection is centred and how many tiles go round.
struct Frame {
    centre: Vec3,
    /// Tiles round, for the wrapping projections.
    round: f32,
}

impl Frame {
    fn of(texture: &Texture, points: &[Vec3], job_of: &[Option<usize>], job: usize) -> Self {
        let mine: Vec<Vec3> = points
            .iter()
            .zip(job_of)
            .filter(|(_, j)| **j == Some(job))
            .map(|(p, _)| *p)
            .collect();
        let centre = if mine.is_empty() {
            Vec3::ZERO
        } else {
            mine.iter().copied().sum::<Vec3>() / mine.len() as f32
        };
        let radius = match texture.projection {
            Projection::Cylindrical(axis) => {
                let along = axis_vector(axis);
                mean(mine.iter().map(|p| {
                    let d = *p - centre;
                    (d - along * d.dot(along)).length()
                }))
            }
            Projection::Spherical => mean(mine.iter().map(|p| p.distance(centre))),
            _ => 0.0,
        };
        let round = (std::f32::consts::TAU * radius / texture.tile_mm.max(1e-3))
            .round()
            .max(1.0);
        Self { centre, round }
    }

    /// The pattern's height at point `p` facing `normal`, 0..=1.
    fn height(&self, job: &Job, p: Vec3, normal: Vec3) -> f32 {
        let texture = job.texture;
        let tile = texture.tile_mm.max(1e-3);
        let sample = |uv: Vec2| {
            let uv = Vec2::from_angle(texture.rotation_deg.to_radians()).rotate(uv);
            height(texture.pattern, uv.x, uv.y, job.image)
        };
        let flat = |axis: Axis| {
            let (a, b) = axis_plane(axis);
            Vec2::new(p.dot(a), p.dot(b)) / tile
        };
        match texture.projection {
            Projection::Planar(axis) => sample(flat(axis)),
            Projection::Triplanar => {
                let w = normal.abs().powf(4.0);
                let total = (w.x + w.y + w.z).max(1e-6);
                (sample(flat(Axis::X)) * w.x
                    + sample(flat(Axis::Y)) * w.y
                    + sample(flat(Axis::Z)) * w.z)
                    / total
            }
            Projection::Cylindrical(axis) => {
                let (a, b) = axis_plane(axis);
                let d = p - self.centre;
                let angle = d.dot(b).atan2(d.dot(a));
                let u = angle / std::f32::consts::TAU * self.round;
                sample(Vec2::new(u, d.dot(axis_vector(axis)) / tile))
            }
            Projection::Spherical => {
                let d = (p - self.centre).normalize_or_zero();
                let u = d.y.atan2(d.x) / std::f32::consts::TAU * self.round;
                let v = d.z.clamp(-1.0, 1.0).asin() / std::f32::consts::TAU * self.round;
                sample(Vec2::new(u, v))
            }
        }
    }
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if count == 0 { 0.0 } else { sum / count as f32 }
}

#[cfg(test)]
mod tests;
