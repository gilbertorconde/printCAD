//! A fill pattern: the places of a flat lattice over a region of a plane,
//! and its copies, each original's tool built once, moved to every place
//! and put in by one boolean.
//!
//! The region is laid out on its plane as the segments of its edge; a
//! place is kept where its copy's outline lies inside, the margin clear of
//! the edge (or, trimming at the edge, where some of the outline does).
//! Copies the edge passes through are trimmed to the region's solid, the
//! region swept along its normal through them, less the margin.

use kernel_api::{FillLattice, FillLayout, FillRegion, MAX_FILL_COPIES, ProfilePlane};
use ogeom::algo::{copied, make_face, make_prism, transformed};
use ogeom::geom::{PlaneSurface, SurfaceGeometry};
use ogeom::math::{Transform, Vector};
use ogeom::mesh::Deflection;
use ogeom::topo::{Filter, Model, Shape, ShapeType, explore};

use super::pattern::{PatternTool, ToolInstance};
use super::{bool_once, combine_solids, common_pieces, fuse_all, solids_of, tol};
use crate::tess;

/// The most lattice places a fill looks at: past it the spacing is tiny
/// beside the region.
const MAX_LOOKED_AT: usize = 400_000;

type V2 = [f64; 2];

fn sub(a: V2, b: V2) -> V2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot2(a: V2, b: V2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross2(a: V2, b: V2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit3(a: [f64; 3]) -> Option<[f64; 3]> {
    let len = dot3(a, a).sqrt();
    (len > 1e-12).then(|| a.map(|c| c / len))
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A region laid out on its plane.
pub(crate) struct Area {
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    normal: [f64; 3],
    /// Its edge, as segments in the plane's (u, v): closed loops, in no
    /// order.
    edge: Vec<(V2, V2)>,
    /// The outer wires of the area and the wires of its holes, for
    /// trimming copies to it.
    outers: Vec<Shape>,
    holes: Vec<Shape>,
}

impl Area {
    fn to_plane(&self, p: [f64; 3]) -> V2 {
        let d: [f64; 3] = std::array::from_fn(|k| p[k] - self.origin[k]);
        [dot3(d, self.u), dot3(d, self.v)]
    }

    fn to_space(&self, p: V2) -> [f64; 3] {
        std::array::from_fn(|k| self.u[k] * p[0] + self.v[k] * p[1])
    }

    /// Whether `p` lies inside: a ray along u crosses the edge an odd
    /// number of times.
    fn inside(&self, p: V2) -> bool {
        let mut inside = false;
        for (a, b) in &self.edge {
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// How far `p` lies from the edge.
    fn clearance(&self, p: V2) -> f64 {
        self.edge
            .iter()
            .map(|(a, b)| point_segment(p, *a, *b))
            .fold(f64::INFINITY, f64::min)
    }

    /// Whether `p` lies inside, at least `margin` from the edge.
    fn holds(&self, p: V2, margin: f64) -> bool {
        self.inside(p) && (margin <= 0.0 || self.clearance(p) >= margin)
    }

    /// Whether the outline `hull` (a convex polygon) lies inside, at
    /// least `margin` from the edge, the edge crossing none of it.
    fn holds_whole(&self, hull: &[V2], margin: f64) -> bool {
        if !hull.iter().all(|p| self.inside(*p)) {
            return false;
        }
        let sides = hull.len();
        for (a, b) in &self.edge {
            if sides > 2 && (inside_convex(hull, *a) || inside_convex(hull, *b)) {
                return false;
            }
            for k in 0..sides {
                let (c, d) = (hull[k], hull[(k + 1) % sides]);
                if segment_segment(*a, *b, c, d) < margin.max(1e-9) {
                    return false;
                }
            }
        }
        true
    }

    /// The area's bounds in the plane.
    fn bounds(&self) -> Option<(V2, V2)> {
        let mut points = self.edge.iter().flat_map(|(a, b)| [*a, *b]);
        let first = points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        }))
    }
}

fn point_segment(p: V2, a: V2, b: V2) -> f64 {
    let ab = sub(b, a);
    let len2 = dot2(ab, ab);
    let t = if len2 > 0.0 {
        (dot2(sub(p, a), ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    let d = sub(p, q);
    dot2(d, d).sqrt()
}

/// How far apart segments `ab` and `cd` come: zero where they cross.
fn segment_segment(a: V2, b: V2, c: V2, d: V2) -> f64 {
    let (d1, d2) = (cross2(sub(b, a), sub(c, a)), cross2(sub(b, a), sub(d, a)));
    let (d3, d4) = (cross2(sub(d, c), sub(a, c)), cross2(sub(d, c), sub(b, c)));
    if (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0) && d1 != 0.0 && d3 != 0.0 {
        return 0.0;
    }
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

/// Whether `p` lies inside the convex polygon `hull`, walked either way.
fn inside_convex(hull: &[V2], p: V2) -> bool {
    let n = hull.len();
    let mut sign = 0.0;
    for k in 0..n {
        let c = cross2(sub(hull[(k + 1) % n], hull[k]), sub(p, hull[k]));
        if c.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    sign != 0.0
}

/// The convex hull of `points`, anticlockwise.
fn hull_of(mut points: Vec<V2>) -> Vec<V2> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    if points.len() < 3 {
        return points;
    }
    let mut lower: Vec<V2> = Vec::new();
    for p in &points {
        while lower.len() >= 2
            && cross2(
                sub(lower[lower.len() - 1], lower[lower.len() - 2]),
                sub(*p, lower[lower.len() - 2]),
            ) <= 0.0
        {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<V2> = Vec::new();
    for p in points.iter().rev() {
        while upper.len() >= 2
            && cross2(
                sub(upper[upper.len() - 1], upper[upper.len() - 2]),
                sub(*p, upper[upper.len() - 2]),
            ) <= 0.0
        {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The segments of `wire`'s edges, laid on the area's plane by `flat`.
fn wire_segments(
    model: &Model,
    wire: &Shape,
    flat: impl Fn([f64; 3]) -> V2,
) -> Result<Vec<(V2, V2)>, String> {
    let tol = tess::tolerances();
    let mut out = Vec::new();
    for edge in explore(model, wire, Filter::OfType(ShapeType::Edge)).map_err(|e| e.to_string())? {
        let points = crate::queries::edge_points(model, &edge, Deflection::default(), tol)
            .map_err(|e| e.to_string())?;
        out.extend(points.windows(2).map(|w| (flat(w[0]), flat(w[1]))));
    }
    Ok(out)
}

/// The region `layout` fills, laid out on its plane: a sketch's area, or
/// the face of `solid` its probe finds.
pub(crate) fn area(
    model: &mut Model,
    solid: Option<&Shape>,
    region: &FillRegion,
) -> Result<Area, String> {
    match region {
        FillRegion::Profile(profile) => {
            let plane = profile.plane;
            let (Some(u), Some(normal)) = (unit3(plane.x_axis), unit3(plane.normal)) else {
                return Err("the fill's sketch plane has no direction".into());
            };
            let v = cross3(normal, u);
            let built = crate::profile::build_profile(model, profile)
                .map_err(|e| format!("the fill's region: {e}"))?;
            let mut area = Area {
                origin: plane.origin,
                u,
                v,
                normal,
                edge: Vec::new(),
                outers: Vec::new(),
                holes: Vec::new(),
            };
            for group in &built.groups {
                area.outers.push(group.outer.wire.clone());
                area.holes
                    .extend(group.holes.iter().map(|hole| hole.wire.clone()));
            }
            let wires: Vec<Shape> = area.outers.iter().chain(&area.holes).cloned().collect();
            for wire in &wires {
                let segments = wire_segments(model, wire, |p| area.to_plane(p))?;
                area.edge.extend(segments);
            }
            Ok(area)
        }
        FillRegion::Face(probe) => {
            let solid = solid.ok_or("a fill over a face needs a solid to find it on")?;
            let face = crate::probe::face_for(model, solid, probe, crate::probe::Reach::Pick)
                .map_err(|e| format!("the fill's face: {e}"))?;
            let facing = probe.normal.map(|c| c as f32);
            let kernel_api::FaceSurface::Plane { origin, normal } =
                tess::face_surface(model, &face, facing)
            else {
                return Err("the fill's face is not flat: fill a flat face or a sketch".into());
            };
            let normal = unit3(normal.map(f64::from)).ok_or("the fill's face has no normal")?;
            // The chain's x laid on the face, its y where x stands square
            // to it.
            let lay = |axis: [f64; 3]| {
                let along = dot3(axis, normal);
                unit3(std::array::from_fn(|k| axis[k] - normal[k] * along))
            };
            let u = lay([1.0, 0.0, 0.0])
                .filter(|_| dot3([1.0, 0.0, 0.0], normal).abs() < 0.999)
                .or_else(|| lay([0.0, 1.0, 0.0]))
                .ok_or("the fill's face has no direction")?;
            let v = cross3(normal, u);
            let outer = model
                .outer_wire(&face)
                .map_err(|e| e.to_string())?
                .ok_or("the fill's face has no edge")?;
            let mut area = Area {
                origin: origin.map(f64::from),
                u,
                v,
                normal,
                edge: Vec::new(),
                outers: vec![outer.clone()],
                holes: Vec::new(),
            };
            area.edge = wire_segments(model, &outer, |p| area.to_plane(p))?;
            Ok(area)
        }
    }
}

/// Where a fill's copies stand: each copy's move from the original, the
/// original's own place left out, in the order occurrences count them,
/// and whether the area's edge passes through it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Places {
    pub moves: Vec<[f64; 3]>,
    pub crossing: Vec<bool>,
}

/// The places of `layout`'s lattice over `area`.
pub(crate) fn places(area: &Area, layout: &FillLayout) -> Result<Places, String> {
    let (step_a, step_b, stagger) = match layout.lattice {
        FillLattice::Grid { x, y } => (x, y, false),
        FillLattice::Honeycomb { pitch } => (pitch, pitch * 3f64.sqrt() * 0.5, true),
    };
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !positive(step_a) || !positive(step_b) {
        return Err("the fill's spacing must be more than zero".into());
    }
    let margin = layout.margin;
    if !(margin.is_finite() && margin >= 0.0) {
        return Err("the fill's margin cannot be negative".into());
    }
    let anchor = area.to_plane(layout.anchor);
    let hull: Vec<V2> = hull_of(
        layout
            .outline
            .iter()
            .map(|p| sub(area.to_plane(*p), anchor))
            .collect(),
    );
    let reach = hull.iter().map(|p| dot2(*p, *p).sqrt()).fold(0.0, f64::max);
    let (sin, cos) = layout.angle_deg.to_radians().sin_cos();
    let (a, b) = ([cos, sin], [-sin, cos]);
    let (lo, hi) = area.bounds().ok_or("the fill's region has no edge")?;
    // The lattice's rows over the area's bounds, from the anchor.
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]].map(|c| sub(c, anchor));
    let span = |axis: V2| {
        corners
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), c| {
                let t = dot2(*c, axis);
                (l.min(t), h.max(t))
            })
    };
    let ((a_lo, a_hi), (b_lo, b_hi)) = (span(a), span(b));
    let row = |lo: f64, hi: f64, step: f64| {
        (
            ((lo - reach) / step).floor() as i64 - 1,
            ((hi + reach) / step).ceil() as i64 + 1,
        )
    };
    let (j_lo, j_hi) = row(b_lo, b_hi, step_b);
    let (i_lo, i_hi) = row(a_lo, a_hi, step_a);
    let looked_at = (j_hi - j_lo + 1) as f64 * (i_hi - i_lo + 1) as f64;
    if looked_at > MAX_LOOKED_AT as f64 {
        return Err(format!(
            "the fill's spacing is too small for its region: its lattice has some {} places \
             over it; widen the spacing",
            looked_at.round()
        ));
    }
    let mut out = Places::default();
    for j in j_lo..=j_hi {
        let shift = if stagger && j.rem_euclid(2) == 1 {
            0.5
        } else {
            0.0
        };
        for i in i_lo..=i_hi {
            if i == 0 && j == 0 {
                continue;
            }
            let along = (i as f64 + shift) * step_a;
            let across = j as f64 * step_b;
            let offset = [a[0] * along + b[0] * across, a[1] * along + b[1] * across];
            let at = [anchor[0] + offset[0], anchor[1] + offset[1]];
            let moved: Vec<V2> = hull.iter().map(|p| [p[0] + at[0], p[1] + at[1]]).collect();
            let whole = if area.holds(at, margin + reach) {
                true
            } else if moved.is_empty() {
                false
            } else {
                area.holds_whole(&moved, margin)
            };
            let kept = whole
                || (layout.clip
                    && std::iter::once(at)
                        .chain(moved.iter().copied())
                        .chain((0..moved.len()).map(|k| {
                            let (p, q) = (moved[k], moved[(k + 1) % moved.len()]);
                            [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5]
                        }))
                        .any(|p| area.holds(p, margin)));
            if !kept {
                continue;
            }
            if out.moves.len() == MAX_FILL_COPIES {
                return Err(format!(
                    "the fill makes more than {MAX_FILL_COPIES} copies, the most one fill \
                     makes: widen its spacing or fill a smaller region"
                ));
            }
            out.moves.push(area.to_space(offset));
            out.crossing.push(!whole);
        }
    }
    Ok(out)
}

/// Every occurrence's move as a row-major transform, the original's (the
/// identity) first.
pub(crate) fn transforms(
    model: &mut Model,
    solid: Option<&Shape>,
    layout: &FillLayout,
) -> Result<Vec<[[f64; 4]; 4]>, String> {
    let area = area(model, solid, &layout.region)?;
    let places = places(&area, layout)?;
    Ok(places.moves.iter().map(|m| translation(*m)).collect())
}

fn translation(m: [f64; 3]) -> [[f64; 4]; 4] {
    [
        [1.0, 0.0, 0.0, m[0]],
        [0.0, 1.0, 0.0, m[1]],
        [0.0, 0.0, 1.0, m[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// The copies of a fill put into `base`: for each original's tool, built
/// once where the original stands, a copy at every place not `skipped`
/// (occurrence 0 the original, copy `k` occurrence `k + 1`), those the
/// edge passes through trimmed to the area when the fill trims, and all
/// of them fused or cut in one boolean. `keep` is handed each tool so
/// put in.
pub fn apply(
    model: &mut Model,
    base: Shape,
    tools: &[ToolInstance],
    layout: &FillLayout,
    skipped: &[u32],
    mut keep: impl FnMut(&Shape, bool),
) -> Result<Shape, String> {
    let area = area(model, Some(&base), &layout.region)?;
    let places = places(&area, layout)?;
    let kept: Vec<([f64; 3], bool)> = places
        .moves
        .iter()
        .zip(&places.crossing)
        .zip(1u32..)
        .filter(|(_, index)| !skipped.contains(index))
        .map(|((m, crossing), _)| (*m, *crossing && layout.clip))
        .collect();
    if kept.is_empty() {
        return Ok(base);
    }
    let mut acc = base;
    for instance in tools {
        let tool = match &instance.tool {
            PatternTool::Solid(shape) => shape.clone(),
            PatternTool::Op(op) => super::pattern::build_tool_op(model, Some(&acc), op)?,
        };
        let mut copies = Vec::with_capacity(kept.len());
        for (m, _) in &kept {
            let fresh = copied(model, &tool)
                .map_err(|e| format!("copying the fill's tool failed: {e}"))?
                .shape;
            let moved = transformed(
                model,
                &fresh,
                Transform::translation(Vector::new(m[0], m[1], m[2])),
            )
            .map_err(|e| format!("moving a copy of the fill's tool failed: {e}"))?
            .shape;
            copies.push(solids_of(model, &moved));
        }
        // A boolean takes a compound of lumps standing apart: copies that
        // meet are fused into one lump first.
        let hull = tool_hull(model, &tool, &area)?;
        let moves: Vec<V2> = kept
            .iter()
            .map(|(m, _)| [dot3(*m, area.u), dot3(*m, area.v)])
            .collect();
        let mut whole = Vec::new();
        let mut cut_through = Vec::new();
        for group in meeting(&hull, &moves) {
            let crossing = group.iter().any(|&k| kept[k].1);
            let mut pieces: Vec<Shape> = group
                .iter()
                .flat_map(|&k| std::mem::take(&mut copies[k]))
                .collect();
            if group.len() > 1 {
                let fused = fuse_all(model, pieces)?;
                pieces = solids_of(model, &fused);
            }
            if crossing {
                cut_through.extend(pieces);
            } else {
                whole.extend(pieces);
            }
        }
        if !cut_through.is_empty() {
            let region = region_solid(model, &area, layout.margin, &cut_through)?;
            let crossing = model
                .add_compound(&cut_through)
                .map_err(|e| format!("gathering the fill's copies failed: {e}"))?;
            whole.extend(common_pieces(model, &crossing, &region)?);
        }
        if whole.is_empty() {
            continue;
        }
        let copies = if whole.len() == 1 {
            whole.remove(0)
        } else {
            model
                .add_compound(&whole)
                .map_err(|e| format!("gathering the fill's copies failed: {e}"))?
        };
        keep(&copies, instance.subtractive);
        let kind = if instance.subtractive {
            kernel_api::BoolKind::Cut
        } else {
            kernel_api::BoolKind::Fuse
        };
        acc = if solids_of(model, &acc).len() == 1 {
            bool_once(model, &acc, &copies, kind)
        } else {
            combine_solids(model, &acc, &copies, kind)
        }
        .map_err(|e| format!("the fill's boolean failed: {e}"))?;
    }
    Ok(acc)
}

/// `tool` seen along the area's normal: the convex hull of its edges laid
/// on the area's plane.
fn tool_hull(model: &Model, tool: &Shape, area: &Area) -> Result<Vec<V2>, String> {
    let tol = tess::tolerances();
    let mut points = Vec::new();
    for edge in explore(model, tool, Filter::OfType(ShapeType::Edge)).map_err(|e| e.to_string())? {
        let along = crate::queries::edge_points(model, &edge, Deflection::default(), tol)
            .map_err(|e| e.to_string())?;
        points.extend(along.into_iter().map(|p| area.to_plane(p)));
    }
    Ok(hull_of(points))
}

/// The copies, `hull` moved by each of `moves`, in groups that meet: a
/// copy alone where it stands clear of every other. Hulls closer than a
/// hundredth of their size meet, since an edge's points lie a little
/// inside a curved side.
fn meeting(hull: &[V2], moves: &[V2]) -> Vec<Vec<usize>> {
    let size = hull.iter().map(|p| dot2(*p, *p).sqrt()).fold(0.0, f64::max);
    let centre = hull.first().copied().unwrap_or([0.0, 0.0]);
    let reach = hull
        .iter()
        .map(|p| dot2(sub(*p, centre), sub(*p, centre)).sqrt())
        .fold(0.0, f64::max);
    let gap = 0.01 * size.max(reach) + 1e-6;
    let mut parent: Vec<usize> = (0..moves.len()).collect();
    fn root(parent: &mut [usize], mut k: usize) -> usize {
        while parent[k] != k {
            parent[k] = parent[parent[k]];
            k = parent[k];
        }
        k
    }
    let moved = |m: V2| -> Vec<V2> { hull.iter().map(|p| [p[0] + m[0], p[1] + m[1]]).collect() };
    for i in 0..moves.len() {
        for j in i + 1..moves.len() {
            let d = sub(moves[j], moves[i]);
            if dot2(d, d).sqrt() > 2.0 * reach + gap {
                continue;
            }
            if hulls_apart(&moved(moves[i]), &moved(moves[j])) <= gap {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slot: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for k in 0..moves.len() {
        let r = root(&mut parent, k);
        let at = *slot.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[at].push(k);
    }
    groups
}

/// How far apart two convex polygons stand: zero where they overlap.
fn hulls_apart(a: &[V2], b: &[V2]) -> f64 {
    if a.len() > 2 && b.iter().any(|p| inside_convex(a, *p))
        || b.len() > 2 && a.iter().any(|p| inside_convex(b, *p))
    {
        return 0.0;
    }
    let edges = |h: &[V2]| -> Vec<(V2, V2)> {
        (0..h.len()).map(|k| (h[k], h[(k + 1) % h.len()])).collect()
    };
    let mut nearest = f64::INFINITY;
    for (p, q) in edges(a) {
        for (r, s) in edges(b) {
            nearest = nearest.min(segment_segment(p, q, r, s));
        }
    }
    nearest
}

/// The area, less `margin` round its edge, swept along its normal
/// through every one of `copies`.
fn region_solid(
    model: &mut Model,
    area: &Area,
    margin: f64,
    copies: &[Shape],
) -> Result<Shape, String> {
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for copy in copies {
        let Some((lo, hi)) = tess::guaranteed_bounds(model, copy) else {
            continue;
        };
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let d: [f64; 3] = std::array::from_fn(|k| [x, y, z][k] - area.origin[k]);
                    let h = dot3(d, area.normal);
                    low = low.min(h);
                    high = high.max(h);
                }
            }
        }
    }
    if !(low.is_finite() && high.is_finite()) {
        return Err("the fill's copies have no bounds".into());
    }
    let (low, high) = (low - 1.0, high + 1.0);
    let plane = crate::convert::plane_of(&ProfilePlane {
        origin: area.origin,
        x_axis: area.u,
        y_axis: area.v,
        normal: area.normal,
    })
    .map_err(|e| format!("the fill's plane: {e}"))?;
    let (lo, hi) = area.bounds().ok_or("the fill's region has no edge")?;
    let room = (hi[0] - lo[0]).max(hi[1] - lo[1]) + margin + 10.0;
    let surface = || {
        PlaneSurface::over(
            plane,
            (lo[0] - room, hi[0] + room),
            (lo[1] - room, hi[1] + room),
        )
        .map(SurfaceGeometry::Plane)
        .map_err(|e| format!("the fill's plane: {e}"))
    };
    let shift = Vector::new(
        area.normal[0] * low,
        area.normal[1] * low,
        area.normal[2] * low,
    );
    let sweep = Vector::new(
        area.normal[0] * (high - low),
        area.normal[1] * (high - low),
        area.normal[2] * (high - low),
    );
    // Each wire moved in by the margin (a hole's out), as a solid through
    // the copies.
    let mut prisms = |wires: &[Shape], grow: f64| -> Result<Vec<Shape>, String> {
        let mut out = Vec::new();
        for wire in wires {
            let moved: Vec<Shape> = if margin > 0.0 {
                let offset = ogeom::offset::offset_wire(
                    model,
                    wire,
                    grow * margin,
                    ogeom::offset::Join::Arc,
                    tol(),
                )
                .map_err(|e| {
                    format!(
                        "the fill's region cannot be moved in by its margin ({e}): set the \
                         margin to 0, or keep only whole copies"
                    )
                })?
                .shape;
                explore(model, &offset, Filter::OfType(ShapeType::Wire))
                    .map_err(|e| e.to_string())?
            } else {
                vec![
                    copied(model, wire)
                        .map_err(|e| format!("copying the fill's edge failed: {e}"))?
                        .shape,
                ]
            };
            for ring in moved {
                let face = make_face(model, surface()?, &[ring], tol())
                    .map_err(|e| format!("the fill's region face: {e}"))?
                    .shape;
                let face = transformed(model, &face, Transform::translation(shift))
                    .map_err(|e| format!("the fill's region face: {e}"))?
                    .shape;
                out.push(
                    make_prism(model, &face, sweep, tol())
                        .map_err(|e| format!("the fill's region solid: {e}"))?
                        .shape,
                );
            }
        }
        Ok(out)
    };
    let outers = prisms(&area.outers, -1.0)?;
    let holes = prisms(&area.holes, 1.0)?;
    let mut region = fuse_all(model, outers)?;
    for hole in holes {
        region = combine_solids(model, &region, &hole, kernel_api::BoolKind::Cut)?;
    }
    Ok(region)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_api::{Profile, ProfileSegment, ProfileWire};

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> ProfileWire {
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        ProfileWire::new(
            (0..4)
                .map(|k| ProfileSegment::Line {
                    start: corners[k],
                    end: corners[(k + 1) % 4],
                })
                .collect(),
        )
    }

    fn layout(wires: Vec<ProfileWire>, lattice: FillLattice) -> FillLayout {
        FillLayout {
            region: FillRegion::Profile(Profile {
                plane: ProfilePlane::default(),
                wires,
            }),
            lattice,
            angle_deg: 0.0,
            margin: 0.0,
            clip: false,
            anchor: [5.0, 5.0, 0.0],
            outline: Vec::new(),
        }
    }

    fn places_of(layout: &FillLayout) -> Result<Places, String> {
        let mut model = Model::new();
        let area = area(&mut model, None, &layout.region)?;
        places(&area, layout)
    }

    /// A 2 mm wide outline about the anchor.
    fn outline(half: f64) -> Vec<[f64; 3]> {
        [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
            .iter()
            .map(|[x, y]| [5.0 + x * half, 5.0 + y * half, 0.0])
            .collect()
    }

    /// A 40 × 30 square filled 10 apart from (5, 5): the places at 5, 15,
    /// 25, 35 along and 5, 15, 25 across, the original's left out, row by
    /// row.
    #[test]
    fn a_grid_fills_the_region_row_by_row_from_the_original() {
        let fill = layout(
            vec![square(0.0, 0.0, 40.0, 30.0)],
            FillLattice::Grid { x: 10.0, y: 10.0 },
        );
        let places = places_of(&fill).unwrap();
        assert_eq!(places.moves.len(), 11);
        assert_eq!(places.moves[0], [10.0, 0.0, 0.0]);
        assert_eq!(places.moves[3], [0.0, 10.0, 0.0]);
        assert!(places.crossing.iter().all(|c| !c));
    }

    /// A margin and an outline keep copies off the edge: a 2 mm outline
    /// with a 1 mm margin needs 2 mm clear of its centre's own side, so
    /// along a 40 mm side only centres from 2 to 38 stand.
    #[test]
    fn whole_copies_keep_their_outline_and_margin_inside() {
        let mut fill = layout(
            vec![square(0.0, 0.0, 40.0, 10.0)],
            FillLattice::Grid { x: 1.5, y: 100.0 },
        );
        fill.outline = outline(1.0);
        fill.margin = 1.0;
        let places = places_of(&fill).unwrap();
        let xs: Vec<f64> = places.moves.iter().map(|m| 5.0 + m[0]).collect();
        assert!(
            xs.iter().all(|x| (2.0 - 1e-9..=38.0 + 1e-9).contains(x)),
            "{xs:?}"
        );
        assert_eq!(places.moves.len() + 1, 25, "2 to 38 by 1.5: {xs:?}");
        // Trimmed at the edge, the copies reaching into it stand too.
        fill.clip = true;
        let clipped = places_of(&fill).unwrap();
        assert!(clipped.moves.len() > places.moves.len());
        assert!(clipped.crossing.iter().any(|c| *c));
    }

    /// A hole in the region keeps copies out of it.
    #[test]
    fn a_hole_in_the_region_keeps_copies_out() {
        let fill = layout(
            vec![square(0.0, 0.0, 40.0, 40.0), square(10.0, 10.0, 30.0, 30.0)],
            FillLattice::Grid { x: 10.0, y: 10.0 },
        );
        let places = places_of(&fill).unwrap();
        let at = |m: &[f64; 3]| [5.0 + m[0], 5.0 + m[1]];
        assert!(
            places
                .moves
                .iter()
                .map(at)
                .all(|[x, y]| !((10.0..30.0).contains(&x) && (10.0..30.0).contains(&y))),
        );
        assert_eq!(
            places.moves.len() + 1,
            12,
            "the ring of 16 less the 4 inside"
        );
    }

    /// A honeycomb's rows stand pitch × √3 / 2 apart, every other one
    /// moved half a pitch; turned, the lattice turns about the original.
    #[test]
    fn a_honeycomb_staggers_its_rows_and_turns_about_the_original() {
        let mut fill = layout(
            vec![square(0.0, 0.0, 40.0, 40.0)],
            FillLattice::Honeycomb { pitch: 4.0 },
        );
        let places = places_of(&fill).unwrap();
        let row = 4.0 * 3f64.sqrt() / 2.0;
        assert!(
            places
                .moves
                .iter()
                .any(|m| (m[0] - 2.0).abs() < 1e-9 && (m[1] - row).abs() < 1e-9),
            "the next row half a pitch along"
        );
        fill.angle_deg = 90.0;
        let turned = places_of(&fill).unwrap();
        assert!(
            turned
                .moves
                .iter()
                .any(|m| m[0].abs() < 1e-9 && (m[1] - 4.0).abs() < 1e-9),
            "along the lattice is up the plane"
        );
    }

    /// Too many copies, or too fine a spacing, is refused by name.
    #[test]
    fn too_many_copies_are_refused() {
        let many = layout(
            vec![square(0.0, 0.0, 1000.0, 1000.0)],
            FillLattice::Grid { x: 10.0, y: 10.0 },
        );
        let err = places_of(&many).unwrap_err();
        assert!(err.contains("2000"), "{err}");
        let fine = layout(
            vec![square(0.0, 0.0, 1000.0, 1000.0)],
            FillLattice::Grid { x: 0.01, y: 0.01 },
        );
        assert!(places_of(&fine).unwrap_err().contains("too small"));
        let zero = layout(
            vec![square(0.0, 0.0, 10.0, 10.0)],
            FillLattice::Honeycomb { pitch: 0.0 },
        );
        assert!(places_of(&zero).is_err());
    }
}
