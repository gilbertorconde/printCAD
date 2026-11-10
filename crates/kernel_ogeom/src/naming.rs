//! The names of the faces of the solid a chain builds, carried from op to
//! op (see `kernel_api::naming`).
//!
//! A feature's tool names its faces by what made them: the wall a profile
//! segment sweeps by the segment's name, the ends by where they stand
//! along the profile's normal. Any other face a feature adds is named by
//! its kind of surface and which way it faces, ties broken by where it
//! is, so a box keeps its names when it grows. After an op, each face of
//! the result takes the names of every face it came from: the faces of the
//! solid before and of the tool that it shares ground with (a piece a
//! boolean cut from a face lies on that face; a face refine merged covers
//! the faces it merged).
//!
//! While an op runs, the chain sets the names of the solid it works on
//! ([`set_current`]); looking up a face or an edge by a reference's names
//! goes through [`find_face`] and [`find_edge`], which fall back to the
//! reference's point when the names find nothing.

use std::cell::{OnceCell, RefCell};

use kernel_api::{Profile, ProfileSegment, TopoName, naming};
use ogeom::algo::History;
use ogeom::math::{Point, Vector};
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::topo::{Model, NodeData, SameKey, Shape, ShapeType, explore_unique};

use crate::convert::array;
use crate::tess;

/// How fine a face is drawn to tell which faces lie on which: this part of
/// its size, within [`FINEST`, `COARSEST`] mm. A flat face is drawn exactly
/// whatever the chord; a curved one strays from its surface by the chord.
const CHORD_PART: f64 = 2e-3;
const FINEST: f64 = 0.005;
const COARSEST: f64 = 0.25;
/// Slack on top of two faces' chords for a point to lie on a face.
const SLACK: f64 = 1e-4;

/// One face with its names, and what telling it apart takes, worked out
/// when first asked: most faces take their names from the kernel's history
/// and never need either.
#[derive(Debug, Clone)]
pub(crate) struct NamedFace {
    pub face: Shape,
    pub names: Vec<TopoName>,
    /// The tag of the op that made the face: a face a later op splits,
    /// trims or leaves alone keeps its maker; zero until given one.
    pub maker: TopoName,
    drawing: OnceCell<Drawing>,
    print: OnceCell<Print>,
}

/// A face drawn finely enough to tell which points lie on it.
#[derive(Debug, Clone)]
struct Drawing {
    /// A point well inside the face, and its normal there.
    sample: Option<(Point, Vector)>,
    triangles: Vec<[Point; 3]>,
    min: Point,
    max: Point,
    /// The chord it was drawn to.
    chord: f64,
}

/// A face at a glance: its kind of surface, how many vertices and edges
/// bound it, and where its vertices lie. An op that leaves a face as it
/// was, whatever it calls it, leaves its print as it was; one that trims
/// the face changes its edges or its vertices.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Print {
    kind: u8,
    vertices: usize,
    edges: usize,
    /// The bounds of the vertices, in micrometres.
    bounds: [i64; 6],
}

impl Print {
    fn of(model: &Model, face: &Shape) -> Self {
        let vertices = explore_unique(model, face, ShapeType::Vertex).unwrap_or_default();
        let edges = explore_unique(model, face, ShapeType::Edge)
            .map(|e| e.len())
            .unwrap_or(0);
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for vertex in &vertices {
            let Some(NodeData::Vertex(data)) = model.node(vertex).map(|n| n.data()) else {
                continue;
            };
            let Ok(placement) = vertex.transform(model.datums()) else {
                continue;
            };
            let p = placement.apply(data.point);
            for (i, c) in array(p).into_iter().enumerate() {
                lo[i] = lo[i].min(c);
                hi[i] = hi[i].max(c);
            }
        }
        let um = |c: f64| (c * 1000.0).round() as i64;
        Self {
            kind: surface_kind(model, face),
            vertices: vertices.len(),
            edges,
            bounds: [
                um(lo[0]),
                um(lo[1]),
                um(lo[2]),
                um(hi[0]),
                um(hi[1]),
                um(hi[2]),
            ],
        }
    }
}

impl Drawing {
    fn of(model: &Model, face: &Shape) -> Self {
        let tol = tess::tolerances();
        let chord = tess::robust_bounds(model, face)
            .map_or(COARSEST, |(lo, hi)| (hi - lo).magnitude() * CHORD_PART)
            .clamp(FINEST, COARSEST);
        let triangles: Vec<[Point; 3]> = Deflection::with_chord(chord)
            .ok()
            .and_then(|d| triangulate_face(model, face, d, tol).ok())
            .map(|mesh| {
                mesh.triangles
                    .iter()
                    .map(|t| t.map(|i| mesh.positions[i as usize]))
                    .collect()
            })
            .unwrap_or_default();
        let sample = triangles
            .iter()
            .map(|[a, b, c]| (((*b - *a).cross(*c - *a)).magnitude(), [*a, *b, *c]))
            .max_by(|x, y| x.0.total_cmp(&y.0))
            .filter(|(area, _)| *area > 0.0)
            .map(|(_, [a, b, c])| {
                let centre = Point::new(
                    (a.x + b.x + c.x) / 3.0,
                    (a.y + b.y + c.y) / 3.0,
                    (a.z + b.z + c.z) / 3.0,
                );
                let n = (b - a).cross(c - a);
                (centre, n * (1.0 / n.magnitude()))
            });
        let mut min = Point::new(f64::MAX, f64::MAX, f64::MAX);
        let mut max = Point::new(f64::MIN, f64::MIN, f64::MIN);
        for p in triangles.iter().flatten() {
            min = Point::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
            max = Point::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
        }
        Self {
            sample,
            triangles,
            min,
            max,
            chord,
        }
    }

    /// Whether `p`, a point drawn to `chord`, lies on the face.
    fn holds(&self, p: Point, chord: f64) -> bool {
        let reach = 2.0 * (self.chord + chord) + SLACK;
        if p.x < self.min.x - reach
            || p.y < self.min.y - reach
            || p.z < self.min.z - reach
            || p.x > self.max.x + reach
            || p.y > self.max.y + reach
            || p.z > self.max.z + reach
        {
            return false;
        }
        self.triangles
            .iter()
            .any(|t| point_triangle_distance(p, t) <= reach)
    }
}

impl NamedFace {
    fn new(face: Shape) -> Self {
        Self {
            face,
            names: Vec::new(),
            maker: 0,
            drawing: OnceCell::new(),
            print: OnceCell::new(),
        }
    }

    fn drawing(&self, model: &Model) -> &Drawing {
        self.drawing.get_or_init(|| Drawing::of(model, &self.face))
    }

    fn print(&self, model: &Model) -> &Print {
        self.print.get_or_init(|| Print::of(model, &self.face))
    }

    /// `face`, the same face as this one under another shape: its maker,
    /// drawing and print as they are, no names yet.
    fn as_face(&self, face: Shape) -> Self {
        Self {
            face,
            names: Vec::new(),
            maker: self.maker,
            drawing: self.drawing.clone(),
            print: self.print.clone(),
        }
    }
}

/// The names of the faces of one solid.
#[derive(Debug, Clone, Default)]
pub(crate) struct NameMap {
    faces: Vec<NamedFace>,
}

impl NameMap {
    /// The names of `face`, when it is one of this solid's.
    pub fn names_of(&self, face: &Shape) -> &[TopoName] {
        let key = SameKey(face.clone());
        self.faces
            .iter()
            .find(|f| SameKey(f.face.clone()) == key)
            .map_or(&[], |f| f.names.as_slice())
    }

    /// The tag of the op that made `face`; zero when it is none of this
    /// solid's.
    pub fn maker_of(&self, face: &Shape) -> TopoName {
        let key = SameKey(face.clone());
        self.faces
            .iter()
            .find(|f| SameKey(f.face.clone()) == key)
            .map_or(0, |f| f.maker)
    }

    /// The one name a pick keeps of `face`: the least of its names.
    pub fn primary(&self, face: &Shape) -> TopoName {
        self.names_of(face).iter().copied().min().unwrap_or(0)
    }

    /// The faces of `root`, each named by `name`; faces left without a
    /// name are named afresh under `fresh`.
    pub fn assign(
        model: &Model,
        root: &Shape,
        fresh: TopoName,
        name: impl FnMut(&mut NamedFace) -> Vec<TopoName>,
    ) -> Self {
        Self::assign_reusing(model, root, fresh, &[], name)
    }

    /// [`Self::assign`], a face that is one of `known`'s keeping that
    /// face's names and drawing as they are.
    fn assign_reusing(
        model: &Model,
        root: &Shape,
        fresh: TopoName,
        known: &[&NameMap],
        mut name: impl FnMut(&mut NamedFace) -> Vec<TopoName>,
    ) -> Self {
        let faces = explore_unique(model, root, ShapeType::Face).unwrap_or_default();
        let mut map = NameMap {
            faces: Vec::with_capacity(faces.len()),
        };
        for face in faces {
            let key = SameKey(face.clone());
            let kept = known
                .iter()
                .flat_map(|m| m.faces.iter())
                .find(|f| SameKey(f.face.clone()) == key);
            let mut named = match kept {
                Some(f) => {
                    let mut same = f.as_face(face);
                    same.names = f.names.clone();
                    same
                }
                None => NamedFace::new(face),
            };
            if named.names.is_empty() {
                named.names = name(&mut named);
            }
            map.faces.push(named);
        }
        map.name_the_rest(model, fresh);
        map.give_makers(known, fresh);
        map
    }

    /// Each face without a maker takes the maker of the face of `sources`
    /// it came from, the one whose names it carries most of (all of them);
    /// a face that came from none was made by the op tagged `fresh`.
    fn give_makers(&mut self, sources: &[&NameMap], fresh: TopoName) {
        for f in self.faces.iter_mut().filter(|f| f.maker == 0) {
            f.maker = sources
                .iter()
                .flat_map(|m| m.faces.iter())
                .filter(|g| g.maker != 0 && !g.names.is_empty())
                .filter(|g| g.names.iter().all(|n| f.names.contains(n)))
                .max_by_key(|g| g.names.len())
                .map_or(fresh, |g| g.maker);
        }
    }

    /// The faces of `result`, each named after every face of `sources` it
    /// came from: as the kernel's `histories` of the op say, else as a
    /// face it looks just like or lies on; faces that came from none are
    /// named afresh under `fresh`.
    pub fn carry(
        model: &Model,
        result: &Shape,
        sources: &[&NameMap],
        fresh: TopoName,
        histories: &[History],
    ) -> Self {
        // Each source edge's two faces' names, for faces a kernel operation
        // generated from an edge (a fillet's blend).
        let edge_names = |edge: &Shape| -> Option<TopoName> {
            let key = SameKey(edge.clone());
            let mut faces: Vec<TopoName> = sources
                .iter()
                .flat_map(|s| s.faces.iter())
                .filter(|g| {
                    explore_unique(model, &g.face, ShapeType::Edge)
                        .unwrap_or_default()
                        .into_iter()
                        .any(|e| SameKey(e) == key)
                })
                .filter_map(|g| g.names.iter().copied().min())
                .collect();
            faces.sort_unstable();
            faces.dedup();
            match faces.as_slice() {
                [a, b, ..] => {
                    let mut bytes = a.to_le_bytes().to_vec();
                    bytes.extend_from_slice(&b.to_le_bytes());
                    Some(naming::child(fresh, &bytes))
                }
                _ => None,
            }
        };
        let histories: Vec<Sources> = histories.iter().map(sources_of).collect();
        let source_faces = || sources.iter().flat_map(|s| s.faces.iter());
        let mut by_history = false;
        let map = Self::assign_reusing(model, result, fresh, sources, |f| {
            // What the kernel says the face came from, first.
            if !histories.is_empty() {
                let came_from = ancestors(&f.face, &histories);
                let mut names: Vec<TopoName> = Vec::new();
                for shape in &came_from[1..] {
                    let key = SameKey(shape.clone());
                    if let Some(g) = source_faces().find(|g| SameKey(g.face.clone()) == key) {
                        names.extend(&g.names);
                    } else if model.kind_of(shape) == Ok(ShapeType::Edge)
                        && let Some(name) = edge_names(shape)
                    {
                        names.push(name);
                    }
                }
                if !names.is_empty() {
                    by_history = true;
                    names.sort_unstable();
                    names.dedup();
                    return names;
                }
            }
            // A face the op left as it was, under another shape.
            let print = f.print(model).clone();
            let mut alike = source_faces().filter(|g| *g.print(model) == print);
            if let (Some(g), None) = (alike.next(), alike.next()) {
                let names = g.names.clone();
                *f = g.as_face(f.face.clone());
                return names;
            }
            // Else where it lies: a piece lies on the face it was cut from.
            let Some((p, _)) = f.drawing(model).sample else {
                return Vec::new();
            };
            let chord = f.drawing(model).chord;
            let mut names: Vec<TopoName> = Vec::new();
            for g in source_faces() {
                if !g.names.is_empty() && g.drawing(model).holds(p, chord) {
                    names.extend(&g.names);
                }
            }
            names
        });
        // Without the kernel's word, a face that merged others covers them.
        if by_history {
            map
        } else {
            map.with_covered(model, sources)
        }
    }

    /// Names every face of `self` also after each source face whose
    /// inside it covers: what a merge of faces keeps.
    fn with_covered(mut self, model: &Model, sources: &[&NameMap]) -> Self {
        for f in &mut self.faces {
            for source in sources {
                for g in &source.faces {
                    if g.names.is_empty() || g.names.iter().all(|n| f.names.contains(n)) {
                        continue;
                    }
                    let drawn = g.drawing(model);
                    if let Some((p, _)) = drawn.sample
                        && f.drawing(model).holds(p, drawn.chord)
                    {
                        f.names.extend(&g.names);
                    }
                }
            }
            f.names.sort_unstable();
            f.names.dedup();
        }
        self
    }

    /// Names the faces still without one: by kind of surface and the way
    /// the face looks, ties broken by where it is.
    fn name_the_rest(&mut self, model: &Model, fresh: TopoName) {
        let mut keyed: Vec<(TopoName, [i64; 3], usize)> = Vec::new();
        for (index, f) in self.faces.iter().enumerate() {
            if !f.names.is_empty() {
                continue;
            }
            let (kind, facing, at) = match f.drawing(model).sample {
                Some((p, n)) => (
                    surface_kind(model, &f.face),
                    array(n).map(|c| (c * 100.0).round() as i64),
                    array(p).map(|c| (c * 1000.0).round() as i64),
                ),
                None => (0, [0; 3], [0; 3]),
            };
            let mut bytes = vec![b'f', kind];
            for c in facing {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            keyed.push((naming::child(fresh, &bytes), at, index));
        }
        keyed.sort();
        let mut previous: Option<TopoName> = None;
        let mut rank = 0u32;
        for (key, _, index) in keyed {
            rank = if previous == Some(key) { rank + 1 } else { 0 };
            previous = Some(key);
            let name = if rank == 0 {
                key
            } else {
                naming::child(key, &rank.to_le_bytes())
            };
            self.faces[index].names.push(name);
        }
    }

    /// The faces of the solid, with their names.
    pub fn faces(&self) -> impl Iterator<Item = (&Shape, &[TopoName])> {
        self.faces.iter().map(|f| (&f.face, f.names.as_slice()))
    }
}

/// A number for the kind of surface a face lies on, as far as naming
/// needs to tell faces apart.
pub(crate) fn surface_kind(model: &Model, face: &Shape) -> u8 {
    let Some(NodeData::Face(data)) = model.node(face).map(|n| n.data()) else {
        return 0;
    };
    let Some(surface) = model.geometry().surface(data.surface) else {
        return 0;
    };
    // The surface's variant, by its name, is kind enough.
    let text = format!("{surface:?}");
    let word: String = text.chars().take_while(|c| c.is_alphanumeric()).collect();
    naming::name_of(word.as_bytes()).to_le_bytes()[0]
}

/// The names of a feature's tool: each wall after the profile segment it
/// was swept from (`segments`: the segments' names and a point inside
/// each, in the tool's frame), the ends after where they stand along
/// `normal` from `origin`, the rest afresh.
pub(crate) fn tool_names(
    model: &Model,
    tool: &Shape,
    feature: TopoName,
    segments: &[(TopoName, Point)],
    origin: Point,
    normal: Vector,
) -> NameMap {
    let mut ends: Vec<(f64, usize)> = Vec::new();
    let mut map = NameMap::assign(model, tool, feature, |_| Vec::new());
    // The walls' and ends' names replace the fresh ones `assign` gave.
    for (index, f) in map.faces.iter_mut().enumerate() {
        // An end lies square to the sweep: a segment's middle is on its
        // rim, not on it.
        let drawn = f.drawing(model);
        let end = drawn
            .sample
            .is_some_and(|(_, n)| n.cross(normal).magnitude() < 1e-3);
        if end {
            if let Some((p, _)) = drawn.sample {
                ends.push(((p - origin).dot(normal), index));
            }
            f.names = Vec::new();
            continue;
        }
        let walls: Vec<TopoName> = segments
            .iter()
            .filter(|(_, p)| drawn.holds(*p, 0.0))
            .map(|(name, _)| naming::child(feature, &name.to_le_bytes()))
            .collect();
        if !walls.is_empty() {
            f.names = walls;
        }
    }
    for (along, index) in ends {
        let end: &[u8] = if along.abs() <= 2.0 * map.faces[index].drawing(model).chord + SLACK {
            b"start"
        } else if along > 0.0 {
            b"end"
        } else {
            b"back"
        };
        map.faces[index].names = vec![naming::child(feature, end)];
    }
    for f in &mut map.faces {
        f.names.retain(|n| *n != 0);
    }
    map
}

/// The name of every segment of `profile` and a point inside it, in the
/// profile's plane in the world; segments without a name are named by
/// where they stand in the profile.
pub(crate) fn profile_segments(profile: &Profile) -> Vec<(TopoName, Point)> {
    let plane = &profile.plane;
    let at = |u: f64, v: f64| {
        Point::new(
            plane.origin[0] + plane.x_axis[0] * u + plane.y_axis[0] * v,
            plane.origin[1] + plane.x_axis[1] * u + plane.y_axis[1] * v,
            plane.origin[2] + plane.x_axis[2] * u + plane.y_axis[2] * v,
        )
    };
    let mut out = Vec::new();
    for (w, wire) in profile.wires.iter().enumerate() {
        for (s, segment) in wire.segments.iter().enumerate() {
            let Some([u, v]) = segment_point(segment) else {
                continue;
            };
            let name = wire.name_of(s).unwrap_or_else(|| {
                let mut bytes = (w as u32).to_le_bytes().to_vec();
                bytes.extend_from_slice(&(s as u32).to_le_bytes());
                naming::name_of(&bytes)
            });
            out.push((name, at(u, v)));
        }
    }
    out
}

/// A point on `segment`, well away from its ends.
fn segment_point(segment: &ProfileSegment) -> Option<[f64; 2]> {
    Some(match segment {
        ProfileSegment::Line { start, end } => {
            [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0]
        }
        ProfileSegment::Arc { mid, .. } => *mid,
        ProfileSegment::Circle { center, radius } => [center[0] + radius, center[1]],
        ProfileSegment::Ellipse { center, major, .. } => {
            [center[0] + major[0], center[1] + major[1]]
        }
        ProfileSegment::EllipseArc {
            center,
            major,
            ratio,
            start_param,
            end_param,
        } => {
            let t = (start_param + end_param) / 2.0;
            let minor = [-major[1] * ratio, major[0] * ratio];
            [
                center[0] + major[0] * t.cos() + minor[0] * t.sin(),
                center[1] + major[1] * t.cos() + minor[1] * t.sin(),
            ]
        }
        ProfileSegment::BSpline { .. } | ProfileSegment::Nurbs { .. } => return None,
    })
}

/// The distance from `p` to triangle `t`.
fn point_triangle_distance(p: Point, t: &[Point; 3]) -> f64 {
    let [a, b, c] = *t;
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.magnitude();
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return bp.magnitude();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (p - (a + ab * v)).magnitude();
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return cp.magnitude();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (p - (a + ac * w)).magnitude();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (p - (b + (c - b) * w)).magnitude();
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (p - (a + ab * v + ac * w)).magnitude()
}

thread_local! {
    /// The names of the solid the op running on this thread works on.
    static CURRENT: RefCell<Option<NameMap>> = const { RefCell::new(None) };
    /// What the kernel said the op running on this thread did, one record
    /// per kernel operation, in order.
    static HISTORIES: RefCell<Vec<History>> = const { RefCell::new(Vec::new()) };
}

/// Keep what a kernel operation of the op running on this thread did:
/// the result's faces are named from it (see [`NameMap::carry`]).
pub(crate) fn record(history: &History) {
    HISTORIES.with(|h| h.borrow_mut().push(history.clone()));
}

/// The names of the solid the ops on this thread work on, while this
/// lives: set by [`set_current`], given back by [`Current::take`], and
/// cleared when dropped (an op that failed).
#[must_use]
pub(crate) struct Current;

impl Current {
    /// The names, given back.
    pub fn take(self) -> NameMap {
        CURRENT.with(|c| c.borrow_mut().take()).unwrap_or_default()
    }

    /// The names, given back, and what the kernel's operations did while
    /// they were set, in order.
    pub fn take_with_histories(self) -> (NameMap, Vec<History>) {
        let histories = HISTORIES.with(|h| std::mem::take(&mut *h.borrow_mut()));
        (self.take(), histories)
    }
}

impl Drop for Current {
    fn drop(&mut self) {
        CURRENT.with(|c| c.borrow_mut().take());
        HISTORIES.with(|h| h.borrow_mut().clear());
    }
}

/// Make `names` the names of the solid the ops on this thread work on, and
/// start keeping what the kernel's operations do.
pub(crate) fn set_current(names: NameMap) -> Current {
    CURRENT.with(|c| *c.borrow_mut() = Some(names));
    HISTORIES.with(|h| h.borrow_mut().clear());
    Current
}

/// The faces of `result` an op made or changed: all of them but those it
/// carried over from `base` untouched, the same face or an exact copy of
/// it through every one of the op's kernel operations (`histories`, in
/// order). What the refine after a feature looks around.
pub(crate) fn touched_faces(
    model: &Model,
    base: &Shape,
    result: &Shape,
    histories: &[History],
) -> Vec<Shape> {
    let mut kept: std::collections::HashSet<SameKey> = explore_unique(model, base, ShapeType::Face)
        .unwrap_or_default()
        .into_iter()
        .map(SameKey)
        .collect();
    for history in histories {
        kept = kept
            .into_iter()
            .filter_map(|SameKey(face)| {
                if history.is_affected(&face) {
                    history.copy_of(&face).cloned().map(SameKey)
                } else {
                    Some(SameKey(face))
                }
            })
            .collect();
    }
    explore_unique(model, result, ShapeType::Face)
        .unwrap_or_default()
        .into_iter()
        .filter(|face| !kept.contains(&SameKey(face.clone())))
        .collect()
}

/// A kernel operation's record turned about: each shape it made, with the
/// shapes it made it from.
type Sources = std::collections::HashMap<SameKey, Vec<Shape>>;

fn sources_of(history: &History) -> Sources {
    let mut out: Sources = std::collections::HashMap::new();
    for input in history.inputs() {
        for made in history
            .modified(&input)
            .iter()
            .chain(history.generated(&input))
        {
            out.entry(SameKey(made.clone()))
                .or_default()
                .push(input.clone());
        }
    }
    out
}

/// Every shape `face` came from through `histories` (the kernel's records
/// of the operations that made it, oldest first, each turned about by
/// [`sources_of`]), `face` included: what each operation modified into it
/// or generated it from, followed back operation by operation.
fn ancestors(face: &Shape, histories: &[Sources]) -> Vec<Shape> {
    let mut all: Vec<Shape> = vec![face.clone()];
    let mut frontier: Vec<Shape> = vec![face.clone()];
    for sources in histories.iter().rev() {
        let mut next: Vec<Shape> = Vec::new();
        for shape in &frontier {
            match sources.get(&SameKey(shape.clone())) {
                Some(inputs) => {
                    for input in inputs {
                        if !all
                            .iter()
                            .any(|a| SameKey(a.clone()) == SameKey(input.clone()))
                        {
                            all.push(input.clone());
                        }
                        next.push(input.clone());
                    }
                }
                // A shape this operation did not make was there before it.
                None => next.push(shape.clone()),
            }
        }
        frontier = next;
    }
    all
}

/// The face of `root` named `name`: the one bearing the name, or, among
/// several (a face a later feature split), the one nearest `near`. `None`
/// when no face of `root` bears it, or no names are set.
pub(crate) fn find_face(model: &Model, root: &Shape, name: TopoName, near: Point) -> Option<Shape> {
    if name == 0 {
        return None;
    }
    let own: Vec<Shape> = explore_unique(model, root, ShapeType::Face).ok()?;
    let candidates: Vec<NamedFace> = CURRENT.with(|c| {
        let current = c.borrow();
        let map = current.as_ref()?;
        Some(
            map.faces
                .iter()
                .filter(|f| f.names.contains(&name))
                .filter(|f| {
                    own.iter()
                        .any(|o| SameKey(o.clone()) == SameKey(f.face.clone()))
                })
                .cloned()
                .collect(),
        )
    })?;
    nearest(model, &candidates, near)
}

/// The edge of `root` between faces named `faces`: among several, the one
/// whose drawn line passes nearest `near`. `None` when no such edge,
/// or no names are set.
pub(crate) fn find_edge(
    model: &Model,
    root: &Shape,
    faces: [TopoName; 2],
    near: Point,
) -> Option<Shape> {
    if faces.contains(&0) {
        return None;
    }
    let own: Vec<Shape> = explore_unique(model, root, ShapeType::Face).ok()?;
    let (a, b): (Vec<Shape>, Vec<Shape>) = CURRENT.with(|c| {
        let current = c.borrow();
        let map = current.as_ref()?;
        let named = |name: TopoName| -> Vec<Shape> {
            map.faces
                .iter()
                .filter(|f| f.names.contains(&name))
                .filter(|f| {
                    own.iter()
                        .any(|o| SameKey(o.clone()) == SameKey(f.face.clone()))
                })
                .map(|f| f.face.clone())
                .collect()
        };
        Some((named(faces[0]), named(faces[1])))
    })?;
    let edges_of = |faces: &[Shape]| -> Vec<Shape> {
        faces
            .iter()
            .flat_map(|f| explore_unique(model, f, ShapeType::Edge).unwrap_or_default())
            .collect()
    };
    let (ea, eb) = (edges_of(&a), edges_of(&b));
    let shared: Vec<Shape> = ea
        .into_iter()
        .filter(|e| eb.iter().any(|o| SameKey(o.clone()) == SameKey(e.clone())))
        .collect();
    match shared.len() {
        0 => None,
        1 => shared.into_iter().next(),
        _ => shared
            .into_iter()
            .map(|e| (edge_distance(model, &e, near), e))
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|(_, e)| e),
    }
}

/// Of `candidates`, the face nearest `near`; the one there is, undrawn.
fn nearest(model: &Model, candidates: &[NamedFace], near: Point) -> Option<Shape> {
    if let [one] = candidates {
        return Some(one.face.clone());
    }
    candidates
        .iter()
        .map(|f| {
            let d = f
                .drawing(model)
                .triangles
                .iter()
                .map(|t| point_triangle_distance(near, t))
                .fold(f64::MAX, f64::min);
            (d, f.face.clone())
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, face)| face)
}

/// How far `near` is from `edge`'s drawn polyline.
fn edge_distance(model: &Model, edge: &Shape, near: Point) -> f64 {
    let tol = tess::tolerances();
    let Ok(deflection) = Deflection::with_chord(FINEST * 4.0) else {
        return f64::MAX;
    };
    let Ok(line) = ogeom::mesh::polyline_of_edge(model, edge, deflection, tol) else {
        return f64::MAX;
    };
    line.windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let ab = b - a;
            let t = ((near - a).dot(ab) / ab.dot(ab).max(1e-300)).clamp(0.0, 1.0);
            (near - (a + ab * t)).magnitude()
        })
        .fold(f64::MAX, f64::min)
}

/// The names of the two faces each of `edges` runs between, in their
/// order: what a pick keeps of an edge.
pub(crate) fn edge_faces(model: &Model, names: &NameMap, edges: &[Shape]) -> Vec<[TopoName; 2]> {
    let mut by_edge: Vec<(SameKey, Vec<TopoName>)> = Vec::new();
    for (face, _) in names.faces() {
        let primary = names.primary(face);
        for edge in explore_unique(model, face, ShapeType::Edge).unwrap_or_default() {
            let key = SameKey(edge);
            match by_edge.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(primary),
                None => by_edge.push((key, vec![primary])),
            }
        }
    }
    edges
        .iter()
        .map(|edge| {
            let key = SameKey(edge.clone());
            let list = by_edge
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, l)| l.as_slice())
                .unwrap_or(&[]);
            match list {
                [a, b, ..] => {
                    let (a, b) = if a <= b { (*a, *b) } else { (*b, *a) };
                    [a, b]
                }
                _ => [0, 0],
            }
        })
        .collect()
}

/// [`edge_faces`] by the names of the solid the op running on this thread
/// works on; none named without them.
pub(crate) fn current_edge_faces(model: &Model, edges: &[Shape]) -> Vec<[TopoName; 2]> {
    CURRENT.with(|c| match c.borrow().as_ref() {
        Some(names) => edge_faces(model, names, edges),
        None => vec![[0, 0]; edges.len()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_on_a_triangle_is_at_no_distance_and_off_it_at_its_height() {
        let t = [
            Point::new(0.0, 0.0, 0.0),
            Point::new(1.0, 0.0, 0.0),
            Point::new(0.0, 1.0, 0.0),
        ];
        assert!(point_triangle_distance(Point::new(0.2, 0.2, 0.0), &t) < 1e-12);
        assert!((point_triangle_distance(Point::new(0.2, 0.2, 0.5), &t) - 0.5).abs() < 1e-12);
        assert!((point_triangle_distance(Point::new(-1.0, 0.0, 0.0), &t) - 1.0).abs() < 1e-12);
    }
}
