//! Cursor snapping: endpoint reuse, curves (the sketch's origin and axes
//! among them), lines' extensions, and alignment with the axes or with
//! other lines.
//!
//! Snapping to an existing point *reuses* that point id instead of creating
//! a coincident twin, so shared endpoints produce naturally connected
//! profiles (the simple, robust alternative to auto-coincident constraints).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;

use core_document::mesh_query::nearest_on_segment;
use glam::DVec2;
use uuid::Uuid;

use crate::curves::Curve;
use crate::geom2d::{Prim, raw_hits, within};
use crate::sketch::{
    GeometryElement, IdHasher, IdMap, ORIGIN_ID, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID,
};

/// What the cursor resolved to after snapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapTarget {
    /// Reuse an existing point element.
    Existing(Uuid),
    /// Create a new point at this position.
    New(Vec2D),
}

impl SnapTarget {
    pub fn position(&self, sketch: &Sketch) -> Option<Vec2D> {
        match self {
            SnapTarget::Existing(id) => sketch.point_position(*id),
            SnapTarget::New(pos) => Some(*pos),
        }
    }
}

/// What a snap landed on: each draws its own marker, and each leaves the
/// new point held there by its own constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    /// An existing point: a line's end, a vertex. Reused, never doubled.
    Endpoint,
    /// An existing point that is a circle's, an arc's or an ellipse's
    /// centre. Reused.
    Center,
    /// The sketch's origin.
    Origin,
    /// Where two curves (or a curve and an axis) cross.
    Intersection,
    /// The middle of a line.
    Midpoint,
    /// On a line, square to it from the point being drawn from.
    Perpendicular,
    /// On a circle or an arc, touching it from the point being drawn from.
    Tangent,
    /// Somewhere on a curve.
    OnCurve,
    /// Somewhere on one of the sketch's axes.
    OnAxis,
    /// Level with the point being drawn from.
    Horizontal,
    /// Plumb with the point being drawn from.
    Vertical,
    /// The segment being drawn runs parallel to another line.
    Parallel,
    /// The segment being drawn runs square to another line.
    PerpendicularTo,
    /// On a line's extension past one of its ends, or on an arc's circle
    /// past its ends.
    OnExtension,
}

impl SnapKind {
    /// Its name, beside the marker.
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "Endpoint",
            SnapKind::Center => "Center",
            SnapKind::Origin => "Origin",
            SnapKind::Intersection => "Intersection",
            SnapKind::Midpoint => "Midpoint",
            SnapKind::Perpendicular => "Perpendicular",
            SnapKind::Tangent => "Tangent",
            SnapKind::OnCurve => "On curve",
            SnapKind::OnAxis => "On axis",
            SnapKind::Horizontal => "Horizontal",
            SnapKind::Vertical => "Vertical",
            SnapKind::Parallel => "Parallel",
            SnapKind::PerpendicularTo => "Perpendicular to",
            SnapKind::OnExtension => "On extension",
        }
    }
}

/// Where a click lands: the point to use and what it landed on (`None` for
/// the bare cursor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub target: SnapTarget,
    pub pos: Vec2D,
    pub kind: Option<SnapKind>,
    /// The curve a perpendicular or tangent snap landed on (the new
    /// segment is held square to it, or touching it), the line a parallel
    /// or square direction follows, or the line an extension continues.
    pub curve: Option<Uuid>,
    /// A direction snap (level, plumb, parallel, square) that also lands
    /// on a curve or an axis: where the direction crosses it, the click
    /// held by both. The kind of the curve it lands on.
    pub on: Option<SnapKind>,
}

/// What a snap knows of the drawing in progress: the point a segment is
/// drawn from (for horizontal and vertical alignment), points it must
/// not land on (that same point), and whether arcs' circles past their
/// ends take a point.
#[derive(Debug, Clone, Default)]
pub struct SnapContext {
    pub from: Option<Vec2D>,
    pub exclude: Vec<Uuid>,
    pub arcs: bool,
}

/// Where `cursor` snaps, within `tol` (sketch units). The kinds are tried
/// in this order, the first that finds anything winning:
///
/// 1. an existing point, reused (`Endpoint`, `Center`): keeping its id is
///    what joins the new curve to it, so it wins even over a nearer origin
///    or crossing;
/// 2. the origin;
/// 3. a crossing of two curves or a curve and an axis;
/// 4. a line's middle;
/// 5. from `cx.from`, the foot square to a line or a point touching a
///    circle or an arc;
/// 6. a curve, then an axis at the same distance, or where a direction
///    from `cx.from` crosses it;
/// 7. level or plumb with `cx.from`;
/// 8. parallel or square to another line from `cx.from`;
/// 9. a line's extension, or an arc's circle past its ends (`cx.arcs`).
///
/// Within a kind the nearest wins, and candidates as near are settled by
/// what they are ([`Rank`]), so the answer does not depend on the order
/// the elements are looked at in. Elements whose points are missing or whose numbers are not
/// finite take no snap. The one function every drawing click and the cue
/// before it go through, so what is shown is where the click lands.
pub fn resolve(sketch: &Sketch, cursor: Vec2D, tol: f64, cx: &SnapContext) -> Snap {
    let bare = Snap {
        target: SnapTarget::New(cursor),
        pos: cursor,
        kind: None,
        curve: None,
        on: None,
    };
    let c = cursor.to_glam();
    if tol.is_nan() || tol <= 0.0 || !c.is_finite() {
        return bare;
    }
    let at = |target: SnapTarget, pos: Vec2D, kind: SnapKind| Snap {
        target,
        pos,
        kind: Some(kind),
        curve: None,
        on: None,
    };
    let scene = Scene::of(sketch);
    let near = scene.near(c, tol);
    if let Some((id, pos)) = nearest_point(&near, &cx.exclude) {
        let kind = if scene.centres.contains(&id) {
            SnapKind::Center
        } else {
            SnapKind::Endpoint
        };
        return at(SnapTarget::Existing(id), Vec2D::from_glam(pos), kind);
    }
    let origin = Vec2D::new(0.0, 0.0);
    if !cx.exclude.contains(&ORIGIN_ID) && c.length() <= tol {
        return at(SnapTarget::New(origin), origin, SnapKind::Origin);
    }
    if let Some(cross) = crossing_near(&near, c, tol) {
        let cross = Vec2D::from_glam(cross);
        return at(SnapTarget::New(cross), cross, SnapKind::Intersection);
    }
    let mut middle = Best::new();
    for item in &scene.items {
        if let Shape::Prim(Prim::Seg { a, b }) = item.shape {
            let m = (a + b) * 0.5;
            let d = (m - c).length();
            if d <= tol {
                middle.offer(Rank::new(d, 0, m).of(item, c), m);
            }
        }
    }
    if let Some(m) = middle.get() {
        let m = Vec2D::from_glam(m);
        return at(SnapTarget::New(m), m, SnapKind::Midpoint);
    }
    if let Some(from) = cx.from
        && let Some((id, pos, kind)) = square_or_touching(&near, from.to_glam(), c, tol)
    {
        return Snap {
            curve: Some(id),
            ..at(SnapTarget::New(pos), pos, kind)
        };
    }
    if let Some((id, proj)) = curve_near(&scene, c, tol, &cx.exclude) {
        let kind = if id == X_AXIS_ID || id == Y_AXIS_ID {
            SnapKind::OnAxis
        } else {
            SnapKind::OnCurve
        };
        // Lying along a direction from the point drawn from too: where
        // the direction crosses the curve, so the click keeps both.
        if let Some(from) = cx.from
            && let Some(both) = direction_onto(&scene, from, cursor, tol, &cx.exclude, id)
        {
            return Snap {
                on: Some(kind),
                ..both
            };
        }
        return at(SnapTarget::New(proj), proj, kind);
    }
    if let Some(from) = cx.from {
        match snap_axis(from, cursor, tol) {
            (pos, Some(AxisSnap::Horizontal)) => {
                return at(SnapTarget::New(pos), pos, SnapKind::Horizontal);
            }
            (pos, Some(AxisSnap::Vertical)) => {
                return at(SnapTarget::New(pos), pos, SnapKind::Vertical);
            }
            _ => {}
        }
        if let Some((id, pos, kind)) = along_a_line(&scene, from, cursor, tol, &cx.exclude) {
            return Snap {
                curve: Some(id),
                ..at(SnapTarget::New(pos), pos, kind)
            };
        }
    }
    let mut extension = Best::new();
    for item in &scene.items {
        let takes = match (&item.shape, item.ends) {
            // A line through the point drawn from would only be continued.
            (Shape::Prim(Prim::Seg { .. }), Some((a, b))) => {
                !cx.exclude.contains(&a) && !cx.exclude.contains(&b)
            }
            (Shape::Prim(Prim::Arc { .. }), _) => cx.arcs && !cx.exclude.contains(&item.id),
            _ => false,
        };
        if takes
            && let Some((foot, d)) = item.shape.extension_foot(c)
            && d <= tol
        {
            extension.offer(Rank::new(d, 0, foot).of(item, c), (item.id, foot));
        }
    }
    if let Some((id, foot)) = extension.get() {
        let pos = Vec2D::from_glam(foot);
        return Snap {
            curve: Some(id),
            ..at(SnapTarget::New(pos), pos, SnapKind::OnExtension)
        };
    }
    bare
}

/// A candidate's standing among others of one kind, compared field by
/// field: nearer first; then the kind's own order of variants (`tier`);
/// then the element nearer the cursor (of two parallel lines a direction
/// follows, the one in view); then the position; then the element's own
/// points. Only elements lying exactly on one another are left, and those
/// are taken by their place in the sketch, the earlier drawn first: ids
/// would not do, since a replayed recording draws the same elements under
/// new ones, while it keeps their order, as saving and loading do.
#[derive(Debug, Clone, Copy)]
struct Rank {
    dist: f64,
    tier: u8,
    reach: f64,
    at: DVec2,
    shape: [f64; 6],
    index: usize,
}

impl Rank {
    /// A candidate `dist` from the cursor at `at`, of no element.
    fn new(dist: f64, tier: u8, at: DVec2) -> Self {
        Self {
            dist,
            tier,
            reach: 0.0,
            at,
            shape: [0.0; 6],
            index: 0,
        }
    }

    /// The candidate made from `item`, with the cursor at `cursor`.
    fn of(self, item: &Item, cursor: DVec2) -> Self {
        Self {
            reach: item.shape.distance(cursor),
            shape: item.shape.key(),
            index: item.index,
            ..self
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        let shapes = self.shape.iter().zip(&other.shape);
        self.dist
            .total_cmp(&other.dist)
            .then(self.tier.cmp(&other.tier))
            .then(self.reach.total_cmp(&other.reach))
            .then(self.at.x.total_cmp(&other.at.x))
            .then(self.at.y.total_cmp(&other.at.y))
            .then_with(|| shapes.fold(Ordering::Equal, |o, (a, b)| o.then(a.total_cmp(b))))
            .then(self.index.cmp(&other.index))
    }
}

/// The best of the candidates offered, by [`Rank`].
struct Best<T>(Option<(Rank, T)>);

impl<T> Best<T> {
    fn new() -> Self {
        Self(None)
    }

    fn offer(&mut self, rank: Rank, value: T) {
        if self
            .0
            .as_ref()
            .is_none_or(|(best, _)| rank.cmp(best).is_lt())
        {
            self.0 = Some((rank, value));
        }
    }

    fn get(self) -> Option<T> {
        self.0.map(|(_, value)| value)
    }
}

/// One element where it stands.
#[derive(Debug, Clone)]
enum Shape {
    Point(DVec2),
    /// A line, a circle or an arc.
    Prim(Prim),
    /// An ellipse, a parabola, a hyperbola or a spline.
    Curve(Curve),
}

impl Shape {
    /// `geom` with its points looked up by `point`: `None` where one is
    /// missing, a number is not finite or a circle's radius is negative.
    fn of(point: &dyn Fn(Uuid) -> Option<Vec2D>, geom: &GeometryElement) -> Option<Self> {
        let finite = |id| point(id).filter(|p| p.to_glam().is_finite());
        let at = |id| finite(id).map(|p| p.to_glam());
        let shape = match geom {
            GeometryElement::Point(p) => Shape::Point(p.position.to_glam()),
            GeometryElement::Line(l) => Shape::Prim(Prim::Seg {
                a: at(l.start)?,
                b: at(l.end)?,
            }),
            GeometryElement::Circle(circle) => Shape::Prim(Prim::Circle {
                c: at(circle.center)?,
                r: circle.radius,
            }),
            GeometryElement::Arc(arc) => {
                let (c, s) = (at(arc.center)?, at(arc.start)?);
                Shape::Prim(Prim::Arc {
                    c,
                    r: (s - c).length(),
                    s,
                    e: at(arc.end)?,
                })
            }
            GeometryElement::Ellipse(_)
            | GeometryElement::BSpline(_)
            | GeometryElement::Conic(_) => Shape::Curve(Curve::of_with(&finite, geom)?),
        };
        shape.is_sound().then_some(shape)
    }

    /// Every number finite and a radius not negative.
    fn is_sound(&self) -> bool {
        match self {
            Shape::Point(p) => p.is_finite(),
            Shape::Prim(Prim::Seg { a, b }) => a.is_finite() && b.is_finite(),
            Shape::Prim(Prim::Circle { c, r }) => c.is_finite() && r.is_finite() && *r >= 0.0,
            Shape::Prim(Prim::Arc { c, r, s, e }) => {
                c.is_finite() && s.is_finite() && e.is_finite() && r.is_finite()
            }
            Shape::Curve(curve) => {
                let (t0, t1) = curve.span;
                t0.is_finite()
                    && t1.is_finite()
                    && curve.at(t0).is_finite()
                    && curve.at(t1).is_finite()
            }
        }
    }

    /// How far `p` is from the nearest bit of the shape. Curves without a
    /// closed-form distance answer the nearest point of the curve itself,
    /// so a point on it is at no distance whatever the tolerance (a
    /// tessellation's chords sag off a bent span by hundredths of a
    /// millimetre, past a scripted click's tolerance).
    fn distance(&self, p: DVec2) -> f64 {
        match *self {
            Shape::Point(q) => (q - p).length(),
            Shape::Prim(Prim::Seg { a, b }) => (p - nearest_on_segment(p, a, b)).length(),
            Shape::Prim(Prim::Circle { c, r }) => ((p - c).length() - r).abs(),
            Shape::Prim(Prim::Arc { c, r, s, e }) => {
                if on_sweep(c, s, e, p) {
                    ((p - c).length() - r).abs()
                } else {
                    (p - s).length().min((p - e).length())
                }
            }
            Shape::Curve(ref curve) => (curve.at(curve.nearest(p, curve.span)) - p).length(),
        }
    }

    /// No further from `p` than the shape is: zero, or for a curve the
    /// distance to a circle it lies within, so most curves away from the
    /// cursor are passed over without working out their nearest point.
    fn distance_at_least(&self, p: DVec2) -> f64 {
        match self {
            Shape::Curve(curve) => curve
                .bound()
                .map_or(0.0, |(c, r)| ((p - c).length() - r).max(0.0)),
            _ => 0.0,
        }
    }

    /// The numbers that place the shape, for settling ties between
    /// shapes alike in everything else.
    fn key(&self) -> [f64; 6] {
        match self {
            Shape::Point(p) => [p.x, p.y, 0.0, 0.0, 0.0, 0.0],
            Shape::Prim(Prim::Seg { a, b }) => [a.x, a.y, b.x, b.y, 0.0, 0.0],
            Shape::Prim(Prim::Circle { c, r }) => [c.x, c.y, *r, 0.0, 0.0, 0.0],
            Shape::Prim(Prim::Arc { c, s, e, .. }) => [c.x, c.y, s.x, s.y, e.x, e.y],
            Shape::Curve(curve) => {
                let (t0, t1) = curve.span;
                let (a, m, b) = (curve.at(t0), curve.at((t0 + t1) * 0.5), curve.at(t1));
                [a.x, a.y, m.x, m.y, b.x, b.y]
            }
        }
    }

    /// Where `p` drops onto a line, a circle or an arc: `None` for a line
    /// with no length, at a circle's centre, off an arc's sweep and for
    /// every other shape.
    fn project(&self, p: DVec2) -> Option<DVec2> {
        match *self {
            Shape::Prim(Prim::Seg { a, b }) => {
                ((b - a).length_squared() >= 1e-12).then(|| nearest_on_segment(p, a, b))
            }
            Shape::Prim(Prim::Circle { c, r }) => {
                let dir = p - c;
                (dir.length() >= 1e-6).then(|| c + dir.normalize() * r)
            }
            Shape::Prim(Prim::Arc { c, r, s, e }) => {
                let dir = p - c;
                (dir.length() >= 1e-6 && on_sweep(c, s, e, p)).then(|| c + dir.normalize() * r)
            }
            _ => None,
        }
    }

    /// Where `p` falls on a line's extension past one of its ends, no
    /// further out than half the line's length, or on an arc's circle past
    /// its ends: the foot and how far `p` is from it.
    fn extension_foot(&self, p: DVec2) -> Option<(DVec2, f64)> {
        match *self {
            Shape::Prim(Prim::Arc { c, r, s, e }) => {
                let dir = p - c;
                if dir.length() < 1e-9 || r < 1e-9 || on_sweep(c, s, e, p) {
                    return None;
                }
                Some((c + dir.normalize() * r, (dir.length() - r).abs()))
            }
            Shape::Prim(Prim::Seg { a, b }) => {
                let ab = b - a;
                let len_sq = ab.length_squared();
                if len_sq < 1e-12 {
                    return None;
                }
                let t = (p - a).dot(ab) / len_sq;
                if (0.0..=1.0).contains(&t) || !(-0.5..=1.5).contains(&t) {
                    return None;
                }
                let foot = a + ab * t;
                Some((foot, (p - foot).length()))
            }
            _ => None,
        }
    }
}

/// Whether `p` lies within the angles the arc about `c` sweeps from `s` to
/// `e`, counter-clockwise.
fn on_sweep(c: DVec2, s: DVec2, e: DVec2, p: DVec2) -> bool {
    let (start, sweep) = arc_angles(s - c, e - c);
    let d = p - c;
    (d.y.atan2(d.x) - start).rem_euclid(std::f64::consts::TAU) <= sweep
}

/// One element of a [`Scene`].
#[derive(Debug, Clone)]
struct Item {
    id: Uuid,
    shape: Shape,
    /// A line's two points.
    ends: Option<(Uuid, Uuid)>,
    construction: bool,
    /// Its place in the sketch's list, the latest drawn last.
    index: usize,
}

/// A sketch's elements where they stand, for one pointer query, every
/// curve's points looked up once. Elements [`Shape::of`] refuses are left
/// out: stale references and numbers out of range take no snap and no
/// click.
struct Scene {
    items: Vec<Item>,
    /// Points that are a circle's, an arc's or an ellipse's centre.
    centres: HashSet<Uuid, BuildHasherDefault<IdHasher>>,
}

impl Scene {
    /// The sketch's elements a tool snaps to and constrains: everything
    /// but the outlines made from text, which are the text's alone.
    fn of(sketch: &Sketch) -> Self {
        Self::with(sketch, false)
    }

    /// [`Self::of`], with the text outlines' curves too (a click on one
    /// takes its text), `text` given.
    fn with(sketch: &Sketch, text: bool) -> Self {
        let mut points: IdMap<Vec2D> =
            HashMap::with_capacity_and_hasher(sketch.geometry.len(), Default::default());
        for geom in &sketch.geometry {
            if let GeometryElement::Point(p) = geom {
                // The first of a repeated id, as `Sketch::point_position`.
                points.entry(p.id).or_insert(p.position);
            }
        }
        let point = |id: Uuid| {
            if id == ORIGIN_ID {
                Some(Vec2D::new(0.0, 0.0))
            } else {
                points.get(&id).copied()
            }
        };
        let mut centres = HashSet::default();
        let mut items = Vec::with_capacity(sketch.geometry.len());
        for (index, geom) in sketch.geometry.iter().enumerate() {
            if sketch.derived.contains(&geom.id())
                && (!text || matches!(geom, GeometryElement::Point(_)))
            {
                continue;
            }
            let centre = match geom {
                GeometryElement::Circle(c) => Some(c.center),
                GeometryElement::Arc(a) => Some(a.center),
                GeometryElement::Ellipse(e) => Some(e.center),
                _ => None,
            };
            centres.extend(centre);
            let ends = match geom {
                GeometryElement::Line(l) => Some((l.start, l.end)),
                _ => None,
            };
            if let Some(shape) = Shape::of(&point, geom) {
                items.push(Item {
                    id: geom.id(),
                    shape,
                    ends,
                    construction: sketch.is_construction(geom.id()),
                    index,
                });
            }
        }
        Self { items, centres }
    }

    /// The elements within `tol` of `p`, with how far each is.
    fn near(&self, p: DVec2, tol: f64) -> Vec<(&Item, f64)> {
        self.items
            .iter()
            .filter_map(|item| {
                if item.shape.distance_at_least(p) > tol {
                    return None;
                }
                let d = item.shape.distance(p);
                (d <= tol).then_some((item, d))
            })
            .collect()
    }

    fn get(&self, id: Uuid) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }
}

/// The nearest point among `near` not in `exclude`, with its position.
fn nearest_point(near: &[(&Item, f64)], exclude: &[Uuid]) -> Option<(Uuid, DVec2)> {
    // Points on one spot differ only by their place in the sketch.
    let mut best = Best::new();
    for &(item, d) in near {
        if let Shape::Point(p) = item.shape
            && !exclude.contains(&item.id)
        {
            best.offer(Rank::new(d, 0, p).of(item, p), (item.id, p));
        }
    }
    best.get()
}

/// The nearest curve (a line, a circle or an arc's rim) within `tol`, then
/// the X axis, then the Y axis at the same distance: its id and where
/// `cursor` drops onto it.
fn curve_near(scene: &Scene, cursor: DVec2, tol: f64, exclude: &[Uuid]) -> Option<(Uuid, Vec2D)> {
    let mut best = Best::new();
    for item in &scene.items {
        if exclude.contains(&item.id) {
            continue;
        }
        if let Some(proj) = item.shape.project(cursor) {
            let d = (proj - cursor).length();
            if d <= tol {
                best.offer(Rank::new(d, 0, proj).of(item, cursor), (item.id, proj));
            }
        }
    }
    for (tier, id, proj) in [
        (1, X_AXIS_ID, DVec2::new(cursor.x, 0.0)),
        (2, Y_AXIS_ID, DVec2::new(0.0, cursor.y)),
    ] {
        let d = (proj - cursor).length();
        if !exclude.contains(&id) && d <= tol {
            best.offer(Rank::new(d, tier, proj), (id, proj));
        }
    }
    best.get().map(|(id, proj)| (id, Vec2D::from_glam(proj)))
}

/// From `from`, a direction the cursor lies along (level, plumb, or
/// parallel or square to another line) carried to where it crosses curve
/// `onto`, when that crossing is near the cursor: the direction's snap at
/// the crossing.
fn direction_onto(
    scene: &Scene,
    from: Vec2D,
    cursor: Vec2D,
    tol: f64,
    exclude: &[Uuid],
    onto: Uuid,
) -> Option<Snap> {
    let (dir, kind, curve) = match snap_axis(from, cursor, tol) {
        (_, Some(AxisSnap::Horizontal)) => (DVec2::X, SnapKind::Horizontal, None),
        (_, Some(AxisSnap::Vertical)) => (DVec2::Y, SnapKind::Vertical, None),
        _ => {
            let (id, pos, kind) = along_a_line(scene, from, cursor, tol, exclude)?;
            let dir = (pos - from).to_glam().try_normalize()?;
            (dir, kind, Some(id))
        }
    };
    // The line other than the one followed: a direction meets its own line
    // only along it.
    if curve == Some(onto) {
        return None;
    }
    let pos = ray_meets(scene, onto, from.to_glam(), dir, cursor.to_glam())?;
    let pos = Vec2D::from_glam(pos);
    ((pos - cursor).to_glam().length() <= 2.0 * tol).then_some(Snap {
        target: SnapTarget::New(pos),
        pos,
        kind: Some(kind),
        curve,
        on: None,
    })
}

/// Where the line through `f` along `dir` crosses curve `id` (an axis, a
/// line, a circle or an arc), the crossing nearest `near`.
fn ray_meets(scene: &Scene, id: Uuid, f: DVec2, dir: DVec2, near: DVec2) -> Option<DVec2> {
    let across = |a: DVec2, along: DVec2| -> Option<DVec2> {
        let denom = dir.perp_dot(along);
        (denom.abs() > 1e-12).then(|| f + dir * ((a - f).perp_dot(along) / denom))
    };
    let mut best = Best::new();
    let mut offer = |p: DVec2| best.offer(Rank::new((p - near).length(), 0, p), p);
    if id == X_AXIS_ID {
        across(DVec2::ZERO, DVec2::X).into_iter().for_each(offer);
    } else if id == Y_AXIS_ID {
        across(DVec2::ZERO, DVec2::Y).into_iter().for_each(offer);
    } else {
        let shape = &scene.get(id)?.shape;
        let found = match *shape {
            Shape::Prim(Prim::Seg { a, b }) => across(a, b - a).into_iter().collect(),
            Shape::Prim(Prim::Circle { c, r }) | Shape::Prim(Prim::Arc { c, r, .. }) => {
                circle_meets(f, dir, c, r)
            }
            _ => Vec::new(),
        };
        // On the curve itself, not only on its line or its circle.
        for p in found {
            if shape.distance(p) <= 1e-6 {
                offer(p);
            }
        }
    }
    best.get()
}

/// Where the line through `f` along unit `dir` crosses the circle at `c`
/// of radius `r`.
fn circle_meets(f: DVec2, dir: DVec2, c: DVec2, r: f64) -> Vec<DVec2> {
    let oc = f - c;
    let b = oc.dot(dir);
    let disc = b * b - (oc.length_squared() - r * r);
    if disc.is_nan() || disc < 0.0 {
        return Vec::new();
    }
    let root = disc.sqrt();
    vec![f + dir * (-b - root), f + dir * (-b + root)]
}

/// From `from`, `cursor` turned onto the direction of another line or the
/// direction square to it, when it lies within `tol` of that direction:
/// the line, the turned point and which of the two it follows, the
/// nearest direction first, along before square. A line through `from`
/// itself only offers its square: along it, the segment would only
/// continue it.
fn along_a_line(
    scene: &Scene,
    from: Vec2D,
    cursor: Vec2D,
    tol: f64,
    exclude: &[Uuid],
) -> Option<(Uuid, Vec2D, SnapKind)> {
    let (f, d) = (from.to_glam(), (cursor - from).to_glam());
    let mut best = Best::new();
    for item in &scene.items {
        let (Shape::Prim(Prim::Seg { a, b }), Some((start, end))) = (&item.shape, item.ends) else {
            continue;
        };
        let Some(dir) = (*b - *a).try_normalize() else {
            continue;
        };
        let through_from = exclude.contains(&start) || exclude.contains(&end);
        for (tier, u, kind) in [
            (0, dir, SnapKind::Parallel),
            (1, dir.perp(), SnapKind::PerpendicularTo),
        ] {
            if through_from && kind == SnapKind::Parallel {
                continue;
            }
            let along = d.dot(u);
            let off = d.dot(u.perp()).abs();
            // Some reach along the direction, as level and plumb ask, so a
            // click on `from` itself is not a direction.
            if along.abs() > tol && off <= tol {
                let p = f + u * along;
                best.offer(
                    Rank::new(off, tier, p).of(item, cursor.to_glam()),
                    (item.id, p, kind),
                );
            }
        }
    }
    best.get()
        .map(|(id, p, kind)| (id, Vec2D::from_glam(p), kind))
}

/// Where `pos` falls on line `geom`'s extension past one of its ends, no
/// further out than half the line's length, or on arc `geom`'s circle
/// past its ends: the foot and how far `pos` is from it. `None` for any
/// other element, along the line or the arc itself, and for an element
/// whose points are missing or out of range.
pub fn extension_foot(sketch: &Sketch, geom: &GeometryElement, pos: Vec2D) -> Option<(Vec2D, f64)> {
    let shape = Shape::of(&|id| sketch.point_position(id), geom)?;
    let (foot, d) = shape.extension_foot(pos.to_glam())?;
    Some((Vec2D::from_glam(foot), d))
}

/// From `from`, the foot of the perpendicular on a line, or a point where
/// a line would touch a circle or an arc, nearest `cursor` within `tol`,
/// among the elements `near` it: feet before touching points as near.
fn square_or_touching(
    near: &[(&Item, f64)],
    f: DVec2,
    c: DVec2,
    tol: f64,
) -> Option<(Uuid, Vec2D, SnapKind)> {
    let mut best = Best::new();
    let mut offer = |item: &Item, p: DVec2, kind: SnapKind| {
        let d = (p - c).length();
        // A foot or a touch at the start itself draws nothing.
        if d <= tol && (p - f).length() > tol {
            let tier = u8::from(kind == SnapKind::Tangent);
            best.offer(Rank::new(d, tier, p).of(item, c), (item.id, p, kind));
        }
    };
    for &(item, _) in near {
        let Shape::Prim(prim) = item.shape else {
            continue;
        };
        match prim {
            Prim::Seg { a, b } => {
                let along = b - a;
                if along.length_squared() > 0.0 {
                    let t = (f - a).dot(along) / along.length_squared();
                    let foot = a + along * t;
                    if within(&prim, foot) {
                        offer(item, foot, SnapKind::Perpendicular);
                    }
                }
            }
            Prim::Arc { c: centre, r, .. } | Prim::Circle { c: centre, r } => {
                let away = f - centre;
                let d = away.length();
                if d > r {
                    // The touching points, where the radius is square to
                    // the line from `from`.
                    let angle = (r / d).acos();
                    let base = away.y.atan2(away.x);
                    for side in [-1.0f64, 1.0] {
                        let t = base + side * angle;
                        let p = centre + DVec2::new(t.cos(), t.sin()) * r;
                        if within(&prim, p) {
                            offer(item, p, SnapKind::Tangent);
                        }
                    }
                }
            }
        }
    }
    best.get()
        .map(|(id, p, kind)| (id, Vec2D::from_glam(p), kind))
}

/// How far the axes reach when crossed with other curves.
const AXIS_REACH: f64 = 1.0e6;

/// The crossing of two curves (the sketch's axes among them) nearest
/// `cursor`, within `tol`, among the elements `near` it.
fn crossing_near(near: &[(&Item, f64)], c: DVec2, tol: f64) -> Option<DVec2> {
    let mut prims: Vec<Prim> = near
        .iter()
        .filter_map(|(item, _)| match item.shape {
            Shape::Prim(prim) => Some(prim),
            _ => None,
        })
        .collect();
    if c.y.abs() <= tol {
        prims.push(Prim::Seg {
            a: DVec2::new(-AXIS_REACH, 0.0),
            b: DVec2::new(AXIS_REACH, 0.0),
        });
    }
    if c.x.abs() <= tol {
        prims.push(Prim::Seg {
            a: DVec2::new(0.0, -AXIS_REACH),
            b: DVec2::new(0.0, AXIS_REACH),
        });
    }
    let mut best = Best::new();
    for i in 0..prims.len() {
        for j in i + 1..prims.len() {
            for p in raw_hits(&prims[i], &prims[j]) {
                let d = (p - c).length();
                if d <= tol && within(&prims[i], p) && within(&prims[j], p) {
                    best.offer(Rank::new(d, 0, p), p);
                }
            }
        }
    }
    best.get()
}

/// Axis alignment detected while drawing a line segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSnap {
    Horizontal,
    Vertical,
}

/// `cursor` turned about `from` to the nearest direction that is a whole
/// number of `step_deg` from the x axis, at the cursor's distance projected
/// onto it.
pub fn angle_step(from: Vec2D, cursor: Vec2D, step_deg: f64) -> Vec2D {
    let d = (cursor - from).to_glam();
    if d.length_squared() < 1e-12 || step_deg <= 0.0 {
        return cursor;
    }
    let step = step_deg.to_radians();
    let angle = (d.y.atan2(d.x) / step).round() * step;
    let dir = DVec2::from_angle(angle);
    let reach = d.dot(dir).max(0.0);
    Vec2D::from_glam(from.to_glam() + dir * reach)
}

/// Snap `cursor` to the nearest existing sketch point within `tol`
/// (sketch units), of points on one spot the one drawn first. Points in
/// `exclude` are ignored (e.g. the chain's own previous point).
pub fn snap_to_point(sketch: &Sketch, cursor: Vec2D, tol: f64, exclude: &[Uuid]) -> SnapTarget {
    let scene = Scene::of(sketch);
    match nearest_point(&scene.near(cursor.to_glam(), tol), exclude) {
        Some((id, _)) => SnapTarget::Existing(id),
        None => SnapTarget::New(cursor),
    }
}

/// While drawing from `from`, snap `cursor` onto the horizontal/vertical
/// axis through `from` when it is within `tol` of it. Returns the adjusted
/// position and which axis was snapped (used to auto-add the matching
/// constraint).
pub fn snap_axis(from: Vec2D, cursor: Vec2D, tol: f64) -> (Vec2D, Option<AxisSnap>) {
    let dx = (cursor.x - from.x).abs();
    let dy = (cursor.y - from.y).abs();
    // Require some real extent along the snapped axis so a click right on
    // top of `from` doesn't produce a degenerate "snapped" segment.
    if dy <= tol && dx > tol {
        (Vec2D::new(cursor.x, from.y), Some(AxisSnap::Horizontal))
    } else if dx <= tol && dy > tol {
        (Vec2D::new(from.x, cursor.y), Some(AxisSnap::Vertical))
    } else {
        (cursor, None)
    }
}

/// Distance from `pos` to the closest bit of `geom` (sketch units).
/// `None` for unresolvable references and numbers out of range.
pub fn distance_to_element(sketch: &Sketch, geom: &GeometryElement, pos: Vec2D) -> Option<f64> {
    Shape::of(&|id| sketch.point_position(id), geom).map(|shape| shape.distance(pos.to_glam()))
}

/// CCW start angle and sweep (0..TAU) for an arc from `start_vec` to
/// `end_vec` (both relative to the center).
pub fn arc_angles(start_vec: DVec2, end_vec: DVec2) -> (f64, f64) {
    let start_angle = start_vec.y.atan2(start_vec.x);
    let end_angle = end_vec.y.atan2(end_vec.x);
    let mut sweep = end_angle - start_angle;
    while sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    (start_angle, sweep)
}

/// Snap `cursor` onto the nearest curve (line, circle or arc rim) within
/// `tol`, returning the curve id and the projected position. Callers check
/// `snap_to_point` first: a nearby point always wins (id reuse). The
/// projected position is what makes the matching PointOnLine/PointOnCircle
/// auto-constraint start satisfied.
///
/// The sketch's reference geometry snaps too, answering to its fixed ids:
/// the origin as a point, ahead of any curve within reach, and the two
/// axes as lines, behind drawn geometry at the same distance.
pub fn snap_to_curve(
    sketch: &Sketch,
    cursor: Vec2D,
    tol: f64,
    exclude: &[Uuid],
) -> Option<(Uuid, Vec2D)> {
    if !exclude.contains(&ORIGIN_ID) && cursor.to_glam().length() <= tol {
        return Some((ORIGIN_ID, Vec2D::new(0.0, 0.0)));
    }
    curve_near(&Scene::of(sketch), cursor.to_glam(), tol, exclude)
}

/// Distance from `pos` to the nearest curve of the sketch, points left
/// out: they are too small to be a click target for feature-level
/// selection. `None` for a sketch with no curves.
pub fn nearest_curve_distance(sketch: &Sketch, pos: Vec2D) -> Option<f64> {
    let p = pos.to_glam();
    let mut nearest: Option<f64> = None;
    for item in &Scene::with(sketch, true).items {
        if matches!(item.shape, Shape::Point(_))
            || nearest.is_some_and(|n| item.shape.distance_at_least(p) >= n)
        {
            continue;
        }
        let d = item.shape.distance(p);
        if nearest.is_none_or(|n| d < n) {
            nearest = Some(d);
        }
    }
    nearest
}

/// Every element within `tol` of `pos`, the one a click takes first:
/// points before curves, so endpoints stay clickable on top of their lines;
/// then drawn geometry before construction, so a line drawn over a guide is
/// what the click finds; then the nearest, and of ones as near the latest
/// drawn, which draws on top. A click again where an overlapping one is
/// selected takes the next. A text's outlines answer for the text: its
/// point.
pub fn hit_candidates(sketch: &Sketch, pos: Vec2D, tol: f64) -> Vec<Uuid> {
    if tol.is_nan() || tol < 0.0 || !pos.to_glam().is_finite() {
        return Vec::new();
    }
    // Distances within a hair of each other count as the same: two curves
    // lying on one another are equally near wherever the click lands.
    let hair = (tol * 1e-3).max(f64::MIN_POSITIVE);
    let scene = Scene::with(sketch, true);
    let mut hits: Vec<(bool, bool, i64, std::cmp::Reverse<usize>, Uuid)> = scene
        .near(pos.to_glam(), tol)
        .into_iter()
        .map(|(item, d)| {
            (
                !matches!(item.shape, Shape::Point(_)),
                item.construction,
                (d / hair).round() as i64,
                std::cmp::Reverse(item.index),
                item.id,
            )
        })
        .collect();
    hits.sort();
    let mut ids: Vec<Uuid> = Vec::with_capacity(hits.len());
    for (.., id) in hits {
        let id = if sketch.derived.contains(&id) {
            match crate::text::block_of(sketch, id) {
                Some(block) => block.anchor,
                None => continue,
            }
        } else {
            id
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// The element a click at `pos` takes first ([`hit_candidates`]), or the
/// reference geometry where nothing drawn is near.
pub fn hit_test(sketch: &Sketch, pos: Vec2D, tol: f64) -> Option<Uuid> {
    if let Some(id) = hit_candidates(sketch, pos, tol).first() {
        return Some(*id);
    }
    // Reference geometry answers where nothing drawn does: the origin first,
    // then whichever axis runs closer.
    if pos.to_glam().length() <= tol {
        return Some(crate::sketch::ORIGIN_ID);
    }
    let on_x = pos.y.abs() <= tol;
    let on_y = pos.x.abs() <= tol;
    match (on_x, on_y) {
        (true, true) if pos.x.abs() < pos.y.abs() => Some(crate::sketch::Y_AXIS_ID),
        (true, _) => Some(crate::sketch::X_AXIS_ID),
        (_, true) => Some(crate::sketch::Y_AXIS_ID),
        _ => None,
    }
}

/// The cue for drawn element `id` taken by a click at `pos`: the point
/// itself, or the spot on the curve nearest `pos`.
pub fn on_element(sketch: &Sketch, id: Uuid, pos: Vec2D) -> Option<Snap> {
    let geom = sketch.get_geometry(id)?;
    if let GeometryElement::Point(point) = geom {
        return Some(Snap {
            target: SnapTarget::Existing(id),
            pos: point.position,
            kind: Some(SnapKind::Endpoint),
            curve: None,
            on: None,
        });
    }
    let point = |id| sketch.point_position(id);
    let at = Shape::of(&point, geom)
        .and_then(|shape| shape.project(pos.to_glam()))
        .map_or(pos, Vec2D::from_glam);
    Some(Snap {
        target: SnapTarget::New(at),
        pos: at,
        kind: Some(SnapKind::OnCurve),
        curve: Some(id),
        on: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Circle, Line, Point, Sketch};

    /// A line drawn over a construction line is what the click finds; the
    /// guide comes next.
    #[test]
    fn a_line_over_a_guide_is_found_before_it() {
        let (mut sketch, a, b, guide) = sketch_with_line();
        sketch.set_construction(guide, true);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let at = Vec2D::new(5.0, 0.05);
        assert_eq!(hit_candidates(&sketch, at, 0.5), [line, guide]);
        // Drawn before the guide, the line still comes first.
        sketch.set_construction(guide, false);
        sketch.set_construction(line, true);
        assert_eq!(hit_candidates(&sketch, at, 0.5), [guide, line]);
    }

    /// Candidates as near as each other are settled by what they are,
    /// never by the order the sketch lists them in: every order of the
    /// same elements snaps alike. The ties: two lines' feet as far either
    /// side of the cursor (the lower), and two parallel lines a direction
    /// follows equally well (the nearer). Only points on one spot, alike in
    /// everything, go by their place in the list: the first listed.
    #[test]
    fn ties_snap_alike_whatever_order_the_sketch_lists_its_elements_in() {
        let mut sketch = Sketch::new("t");
        let point = |s: &mut Sketch, x, y| {
            s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        };
        let line = |s: &mut Sketch, a: (f64, f64), b: (f64, f64)| {
            let (a, b) = (point(s, a.0, a.1), point(s, b.0, b.1));
            s.add_geometry(GeometryElement::Line(Line::new(a, b)))
        };
        let twins = [point(&mut sketch, 3.0, 10.0), point(&mut sketch, 3.0, 10.0)];
        let low = line(&mut sketch, (0.0, 20.0), (10.0, 20.0));
        line(&mut sketch, (0.0, 20.5), (10.0, 20.5));
        let near = line(&mut sketch, (100.0, 0.0), (103.0, 4.0));
        line(&mut sketch, (200.0, 0.0), (203.0, 4.0));
        let from = |x, y| SnapContext {
            from: Some(Vec2D::new(x, y)),
            ..SnapContext::default()
        };
        let feet = (Vec2D::new(2.0, 20.25), from(2.25, 30.0));
        let along = (Vec2D::new(30.0, -10.2), from(0.0, -50.0));
        let answer = |sketch: &Sketch, (c, cx): &(Vec2D, SnapContext)| {
            let snap = resolve(sketch, *c, 0.5, cx);
            (snap.kind, snap.curve, snap.pos)
        };
        let twin_at = Vec2D::new(3.1, 10.1);
        for turn in 0..sketch.geometry.len() {
            let mut turned = sketch.clone();
            turned.geometry.rotate_left(turn);
            if turn % 2 == 1 {
                turned.geometry.reverse();
            }
            let feet = answer(&turned, &feet);
            assert_eq!((feet.0, feet.1), (Some(SnapKind::Perpendicular), Some(low)));
            assert_eq!(answer(&turned, &along).1, Some(near), "listed from {turn}");
            let listed_first = turned.geometry.iter().find(|g| twins.contains(&g.id()));
            assert_eq!(
                snap_to_point(&turned, twin_at, 0.5, &[]),
                SnapTarget::Existing(listed_first.unwrap().id())
            );
        }
    }

    /// Elements with stale references or numbers out of range take no snap
    /// and no click, and the sound ones beside them still do.
    #[test]
    fn broken_elements_are_passed_over() {
        use crate::sketch::{Arc, BSpline, Ellipse};
        let (mut sketch, a, _, l) = sketch_with_line();
        let gone = Uuid::new_v4();
        let nan = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
            f64::NAN,
            0.0,
        ))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 3.0))));
        for radius in [-2.0, f64::NAN, f64::INFINITY] {
            sketch.add_geometry(GeometryElement::Circle(Circle::new(c, radius)));
        }
        for broken in [
            GeometryElement::Line(Line::new(a, gone)),
            GeometryElement::Line(Line::new(nan, c)),
            GeometryElement::Arc(Arc::new(c, gone, a, 1.0)),
            GeometryElement::Arc(Arc::new(c, nan, a, 1.0)),
            GeometryElement::Ellipse(Ellipse::new(c, Vec2D::new(f64::NAN, 1.0), 0.5)),
            GeometryElement::Ellipse(Ellipse::new_arc(c, Vec2D::new(2.0, 0.0), 0.5, a, gone)),
            GeometryElement::BSpline(BSpline::new(vec![a, gone, c], false)),
            GeometryElement::BSpline(BSpline::new(vec![a, nan, c], false)),
        ] {
            sketch.add_geometry(broken);
        }
        let cx = |from: Option<Vec2D>| SnapContext {
            from,
            arcs: true,
            ..SnapContext::default()
        };
        for x in [-1.0, 0.0, 2.5, 5.0, 7.0, 10.0, f64::NAN] {
            for y in [-1.0, 0.0, 1.0, 3.0, 5.0] {
                let p = Vec2D::new(x, y);
                for from in [
                    None,
                    Some(Vec2D::new(5.0, 3.0)),
                    Some(Vec2D::new(f64::NAN, 1.0)),
                ] {
                    let snap = resolve(&sketch, p, 0.6, &cx(from));
                    assert!(snap.pos.to_glam().is_finite() || x.is_nan(), "{snap:?}");
                }
                hit_candidates(&sketch, p, 0.6);
                nearest_curve_distance(&sketch, p);
            }
        }
        assert_eq!(
            resolve(&sketch, Vec2D::new(5.0, 0.2), 0.5, &cx(None)).kind,
            Some(SnapKind::Midpoint)
        );
        assert_eq!(hit_test(&sketch, Vec2D::new(6.0, 0.2), 0.5), Some(l));
        assert_eq!(
            hit_test(&sketch, Vec2D::new(7.0, 3.0), 0.5),
            None,
            "no broken circle"
        );
    }

    fn sketch_with_line() -> (Sketch, Uuid, Uuid, Uuid) {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let l = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        (sketch, a, b, l)
    }

    #[test]
    fn snaps_to_nearby_point() {
        let (sketch, a, _, _) = sketch_with_line();
        assert_eq!(
            snap_to_point(&sketch, Vec2D::new(0.3, -0.2), 0.5, &[]),
            SnapTarget::Existing(a)
        );
        assert!(matches!(
            snap_to_point(&sketch, Vec2D::new(5.0, 5.0), 0.5, &[]),
            SnapTarget::New(_)
        ));
        // An excluded point is no snap target.
        assert!(matches!(
            snap_to_point(&sketch, Vec2D::new(0.1, 0.0), 0.5, &[a]),
            SnapTarget::New(_)
        ));
    }

    #[test]
    fn axis_snap_levels_nearly_horizontal() {
        let from = Vec2D::new(0.0, 0.0);
        let (pos, axis) = snap_axis(from, Vec2D::new(8.0, 0.2), 0.5);
        assert_eq!(axis, Some(AxisSnap::Horizontal));
        assert_eq!(pos.y, 0.0);
        let (pos, axis) = snap_axis(from, Vec2D::new(-0.3, 6.0), 0.5);
        assert_eq!(axis, Some(AxisSnap::Vertical));
        assert_eq!(pos.x, 0.0);
        let (_, axis) = snap_axis(from, Vec2D::new(5.0, 5.0), 0.5);
        assert_eq!(axis, None);
        // A click on the start has no direction to level.
        let (pos, axis) = snap_axis(from, Vec2D::new(0.1, 0.1), 0.5);
        assert_eq!(axis, None);
        assert_eq!(pos.x, 0.1);
    }

    #[test]
    fn an_angle_step_turns_the_cursor_to_the_nearest_fifteen_degrees() {
        let from = Vec2D::new(1.0, 1.0);
        let at = angle_step(from, Vec2D::new(11.0, 4.0), 15.0);
        let d = (at - from).to_glam();
        assert!((d.y.atan2(d.x).to_degrees() - 15.0).abs() < 1e-4, "{d:?}");
        assert!(
            (d.length()
                - glam::DVec2::new(10.0, 3.0).dot(glam::DVec2::from_angle(15f64.to_radians())))
            .abs()
                < 1e-4
        );
        let straight_up = angle_step(from, Vec2D::new(1.4, 9.0), 15.0);
        assert!((straight_up.x - 1.0).abs() < 1e-5);
        assert_eq!(angle_step(from, from, 15.0), from, "no direction to turn");
    }

    #[test]
    fn hit_test_prefers_points_over_curves() {
        let (sketch, a, _, l) = sketch_with_line();
        // Right on the endpoint: both the point and the line are within
        // tolerance; the point must win.
        assert_eq!(hit_test(&sketch, Vec2D::new(0.05, 0.0), 0.5), Some(a));
        // Mid-span: only the line is close.
        assert_eq!(hit_test(&sketch, Vec2D::new(5.0, 0.1), 0.5), Some(l));
        assert_eq!(hit_test(&sketch, Vec2D::new(5.0, 3.0), 0.5), None);
    }

    #[test]
    fn a_point_of_a_spline_is_on_it() {
        let mut sketch = Sketch::new("s");
        let poles: Vec<Uuid> = [
            (10.0, 10.0),
            (16.0, 52.0),
            (34.0, -39.0),
            (54.0, 45.0),
            (60.0, 10.0),
        ]
        .iter()
        .map(|&(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))))
        .collect();
        let spline = GeometryElement::BSpline(crate::sketch::BSpline::new(poles, false));
        sketch.add_geometry(spline.clone());
        let curve = crate::curves::Curve::of(&sketch, &spline).unwrap();
        for i in 1..40 {
            let t = curve.span.0 + (curve.span.1 - curve.span.0) * i as f64 / 40.0;
            let on = curve.at(t);
            let d = distance_to_element(&sketch, &spline, Vec2D::new(on.x, on.y)).unwrap();
            // A scripted click takes curves within a micrometre (`sketch.draw`).
            assert!(d < 1e-6, "t = {t}: {d} off a point of the curve");
        }
    }

    #[test]
    fn nearest_curve_distance_ignores_points() {
        let (sketch, _, _, _) = sketch_with_line();
        // Mid-span, 2 units off the line.
        let d = nearest_curve_distance(&sketch, Vec2D::new(5.0, 2.0)).unwrap();
        assert!((d - 2.0).abs() < 1e-5);
        let empty = Sketch::new("e");
        assert!(nearest_curve_distance(&empty, Vec2D::new(0.0, 0.0)).is_none());
    }

    #[test]
    fn snap_to_curve_projects_onto_line_and_circle() {
        let (mut sketch, _, _, l) = sketch_with_line();
        // Near the line's mid-span: projected straight down onto it.
        let (id, proj) = snap_to_curve(&sketch, Vec2D::new(5.0, 0.3), 0.5, &[]).unwrap();
        assert_eq!(id, l);
        assert!((proj.x - 5.0).abs() < 1e-5 && proj.y.abs() < 1e-5);
        // Too far: no snap.
        assert!(snap_to_curve(&sketch, Vec2D::new(5.0, 3.0), 0.5, &[]).is_none());
        // Excluded, the X axis under it answers; with that excluded too,
        // nothing does.
        assert_eq!(
            snap_to_curve(&sketch, Vec2D::new(5.0, 0.3), 0.5, &[l]).map(|(id, _)| id),
            Some(crate::sketch::X_AXIS_ID)
        );
        assert!(
            snap_to_curve(
                &sketch,
                Vec2D::new(5.0, 0.3),
                0.5,
                &[l, crate::sketch::X_AXIS_ID]
            )
            .is_none()
        );

        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 20.0))));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 5.0)));
        let (id, proj) = snap_to_curve(&sketch, Vec2D::new(4.8, 20.0), 0.5, &[]).unwrap();
        assert_eq!(id, circle);
        assert!((proj.x - 5.0).abs() < 1e-5 && (proj.y - 20.0).abs() < 1e-5);
    }

    #[test]
    fn circle_hit_is_on_rim_not_center() {
        let mut sketch = Sketch::new("t");
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 5.0)));
        assert_eq!(hit_test(&sketch, Vec2D::new(5.1, 0.0), 0.5), Some(circle));
        // Inside the circle, off both axes, nothing is hit (the center point
        // wins only within tolerance of the center itself).
        assert_eq!(hit_test(&sketch, Vec2D::new(2.5, 2.5), 0.5), None);
    }

    #[test]
    fn the_axes_and_the_origin_answer_where_nothing_is_drawn() {
        let sketch = Sketch::new("t");
        assert_eq!(
            hit_test(&sketch, Vec2D::new(7.0, 0.2), 0.5),
            Some(crate::sketch::X_AXIS_ID)
        );
        assert_eq!(
            hit_test(&sketch, Vec2D::new(-0.1, 9.0), 0.5),
            Some(crate::sketch::Y_AXIS_ID)
        );
        // Their crossing is the origin, which wins over both.
        assert_eq!(
            hit_test(&sketch, Vec2D::new(0.2, 0.1), 0.5),
            Some(crate::sketch::ORIGIN_ID)
        );
        assert_eq!(hit_test(&sketch, Vec2D::new(4.0, 4.0), 0.5), None);
    }

    #[test]
    fn the_origin_and_the_axes_snap_behind_drawn_geometry() {
        use crate::sketch::{ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID};
        let empty = Sketch::new("t");
        assert_eq!(
            snap_to_curve(&empty, Vec2D::new(6.0, 0.3), 0.5, &[]),
            Some((X_AXIS_ID, Vec2D::new(6.0, 0.0)))
        );
        assert_eq!(
            snap_to_curve(&empty, Vec2D::new(-0.2, -3.0), 0.5, &[]),
            Some((Y_AXIS_ID, Vec2D::new(0.0, -3.0)))
        );
        assert_eq!(
            snap_to_curve(&empty, Vec2D::new(0.3, 0.2), 0.5, &[]),
            Some((ORIGIN_ID, Vec2D::new(0.0, 0.0)))
        );
        assert_eq!(snap_to_curve(&empty, Vec2D::new(4.0, 4.0), 0.5, &[]), None);
    }
}
