//! Camera-independent transform handles. Geometry is shared by drawing and picking.
//! Hold the camera fixed during a gesture. Apply every delta to the caller's press
//! snapshot, commit on Finished, and restore that snapshot on Cancelled.
//!
//! Space is the host's, in `f64`: the gizmo never sees a floating origin, only the
//! host's [`GizmoView`] that projects and casts rays in it.
//!
//! Arrows end in cones, planar squares translate in a plane, and during a drag the
//! gizmo shows what changes: a ghost of the press position with a dashed travel
//! line and the axis guide for translation, the swept sector for rotation, and the
//! value on a plate next to it. Modes stay separate: one gizmo shows either arrows
//! or rings.
//!
//! Handles come from a list ([`HandleSet`]): each [`HandleSpec`] is one scalar
//! freedom, a slide along or a turn about an arbitrary unit direction, such as one
//! joint of a kinematic chain ([`crate::hand`] turns the delta into joint
//! increments). Translate shows the listed slides (arrows,
//! plus a planar handle for every non-collinear pair), Rotate the listed turns
//! (rings); a freedom that is not listed has no handle and cannot be grabbed.
//! [`HandleSet::placement_axes`] is the classic three-axis gizmo.
//!
//! A frame of a host: [`Gizmo::layout`], [`Layout::hit`] under the pointer, then
//! [`Gizmo::press`] on a press over a handle, or [`Gizmo::drag`] and
//! [`Gizmo::release`] / [`Gizmo::cancel`] while one is held, and
//! [`Gizmo::shapes`] of that layout to draw.
use std::fmt;

use emath::{Pos2, Vec2};
use glam::{DQuat, DVec3};

use crate::axis::Axis;
use crate::hand::SlidePlane;
use crate::paint::{Dash, Ink, Paint, Shape, Stroke};
use crate::screen::{convex_hull, segment_distance};
use crate::view::GizmoView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Translate,
    Rotate,
    Scale,
}

/// A grabbable part, named by the keys of the specs it comes from. The motion
/// follows from the handle: `Ring` rotates; `Axis` and `View` translate, or scale
/// in `Mode::Scale`; `Plane` translates.
///
/// The default key is the axis letter: the classic gizmo
/// ([`HandleSet::placement_axes`]) names its handles `Axis(Axis::X)`,
/// `Ring(Axis::Z)`, `Plane(Axis::X, Axis::Y)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle<K = Axis> {
    /// Arrow with a cone (translation) or axis with a cube (scale) of a `Slide` spec.
    Axis(K),
    /// Rotation ring of a `Turn` spec.
    Ring(K),
    /// Planar handle of two `Slide` specs, in list order.
    Plane(K, K),
    /// Centre disc (Translate, Scale) or outer ring (Rotate): free motion in the
    /// screen plane, present only with [`HandleSet::screen`].
    View,
}

/// What a listed freedom does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    /// Translation along `direction`: an arrow in Translate (a cube axis in Scale).
    Slide,
    /// Rotation about `direction` through the pivot: a ring in Rotate.
    Turn,
}

/// One handle: a scalar freedom along an arbitrary unit direction in host space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandleSpec<K> {
    /// The host's name for the freedom; unique per kind within a set.
    pub key: K,
    pub kind: JointKind,
    /// Letter and colour of the handle (X red, Y green, Z blue): the letter of the
    /// axis in the frame the freedom is written in, not of the world axis it
    /// happens to point along.
    pub axis: Axis,
    /// The freedom is written in axes other than the world's (a datum's, the
    /// object's own): its shaft, ring and plane outline draw dashed, so a world
    /// axis and a datum axis differ to the eye.
    pub dashed: bool,
    /// Finite unit vector in host space: for a chain joint, its direction taken at
    /// the press.
    pub direction: DVec3,
}

/// Everything the gizmo shows in one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct HandleSet<K> {
    /// Pivot of every handle: rings turn about it. The orientation frames the
    /// screen handle (Shift snap and plate in its axes) and `Delta::Scale`, and
    /// orients ring segmentation; it does not orient listed handles.
    pub placement: Placement,
    pub handles: Vec<HandleSpec<K>>,
    /// Add the screen-plane handle ([`Handle::View`]). It moves freely, not along
    /// a listed freedom, so a host whose freedoms are joints leaves it off.
    pub screen: bool,
}

impl HandleSet<Axis> {
    /// The classic gizmo: slides and turns along the three placement axes plus the
    /// screen handle. Identity orientation is world axes (solid), any other is
    /// local (dashed).
    pub fn placement_axes(placement: Placement) -> Self {
        let dashed = placement.orientation != DQuat::IDENTITY;
        let handles = [JointKind::Slide, JointKind::Turn]
            .into_iter()
            .flat_map(|kind| {
                Axis::ALL.map(|axis| HandleSpec {
                    key: axis,
                    kind,
                    axis,
                    dashed,
                    direction: placement.axis(axis),
                })
            })
            .collect();
        Self {
            placement,
            handles,
            screen: true,
        }
    }
}

impl<K: Copy + PartialEq + fmt::Debug> HandleSet<K> {
    fn validate(&self) {
        self.placement.validate();
        for (index, spec) in self.handles.iter().enumerate() {
            let direction = spec.direction;
            assert!(
                direction.is_finite() && (direction.length_squared() - 1.0).abs() < 1e-9,
                "HandleSpec {:?}: direction must be a finite unit vector, got {direction:?}",
                spec.key
            );
            assert!(
                !self.handles[..index]
                    .iter()
                    .any(|other| other.key == spec.key && other.kind == spec.kind),
                "HandleSet: duplicate {:?} key {:?}",
                spec.kind,
                spec.key
            );
        }
    }

    fn spec(&self, key: K, kind: JointKind) -> &HandleSpec<K> {
        self.handles
            .iter()
            .find(|spec| spec.key == key && spec.kind == kind)
            .unwrap_or_else(|| panic!("HandleSet: no {kind:?} handle {key:?}"))
    }

    /// Geometry of `handle` in this set, resolved once at the press.
    fn grip(&self, handle: Handle<K>) -> Grip {
        match handle {
            Handle::Axis(key) => {
                let spec = self.spec(key, JointKind::Slide);
                Grip::Arrow {
                    direction: spec.direction,
                    axis: spec.axis,
                }
            }
            Handle::Ring(key) => Grip::Ring {
                direction: self.spec(key, JointKind::Turn).direction,
            },
            Handle::Plane(first, second) => {
                let first = self.spec(first, JointKind::Slide);
                let second = self.spec(second, JointKind::Slide);
                Grip::Plane {
                    plane: SlidePlane::new(first.direction, second.direction)
                        .expect("a planar handle exists only for a non-collinear pair"),
                    first: (first.direction, first.axis),
                    second: (second.direction, second.axis),
                }
            }
            Handle::View => Grip::View,
        }
    }
}

/// Resolved geometry of a grabbed handle: directions, letters, Gram solver.
#[derive(Clone, Copy, Debug)]
enum Grip {
    Arrow {
        direction: DVec3,
        axis: Axis,
    },
    Plane {
        plane: SlidePlane,
        first: (DVec3, Axis),
        second: (DVec3, Axis),
    },
    Ring {
        direction: DVec3,
    },
    View,
}

/// Two unit vectors completing `direction` to a right-handed orthonormal frame
/// `(direction, u, v)`: ring plane, cone base, cube faces. `u` is the orientation's
/// next axis after the handle letter (Y for X, Z for Y, X for Z) with its component
/// along `direction` removed, so for a placement axis it is the next placement
/// axis; if that axis is (nearly) along `direction`, the one after it.
fn across(direction: DVec3, axis: Axis, orientation: DQuat) -> (DVec3, DVec3) {
    let reject = |candidate: Axis| {
        let along = orientation * candidate.unit();
        along - direction * direction.dot(along)
    };
    let mut u = reject(axis.next());
    if u.length_squared() < 1e-6 {
        u = reject(axis.next().next());
    }
    let u = u.normalize();
    (u, direction.cross(u))
}

/// What a drag does; derived from the handle and the mode in one place
/// ([`Motion::of`]) so drawing, the press and sampling never disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Translate,
    Rotate,
    Scale,
}

impl Motion {
    fn of<K>(mode: Mode, handle: Handle<K>) -> Self {
        match handle {
            Handle::Ring(_) => Self::Rotate,
            // In Rotate the view handle is the outer screen-plane ring.
            Handle::View if mode == Mode::Rotate => Self::Rotate,
            Handle::Axis(_) | Handle::View if mode == Mode::Scale => Self::Scale,
            Handle::Axis(_) | Handle::View | Handle::Plane(..) => Self::Translate,
        }
    }
}

// Handle geometry as fractions of `Style::radius` (the arrow length in pixels).
/// The shaft starts off the centre so the view handle stays clear.
const SHAFT_START: f64 = 0.12;
/// Cone base; the cone tip is at 1.0, so the cone is a fifth of the arrow.
const CONE_BASE: f64 = 0.78;
const CONE_RADIUS: f64 = 0.075;
/// Scale cube: centre along the axis and half edge.
const CUBE_CENTER: f64 = 0.92;
const CUBE_HALF: f64 = 0.065;
/// Planar squares span `[PLANE_MIN, PLANE_MAX]` on both of their axes.
const PLANE_MIN: f64 = 0.28;
const PLANE_MAX: f64 = 0.52;
/// Axis letter sits past the cone tip.
const LETTER_AT: f64 = 1.16;
/// Radius of the central view handle (a disc), px.
const VIEW_HALF: f32 = 8.0;
/// Outer screen-plane ring in Rotate, × radius.
const VIEW_RING: f64 = 1.18;
/// Central axis triad in Rotate, × radius: tells which ring turns about which axis.
const TRIAD: f64 = 0.24;
/// A ring point counts as the near half when its offset from the pivot points
/// toward the camera: `offset · forward < FRONT_SLACK`. The slack keeps a ring
/// that faces the camera (all points at ≈0) whole instead of flickering.
const FRONT_SLACK: f64 = 0.02;
const RING_SEGMENTS: usize = 96;
/// `|normal · forward|` below this: a ring or a planar handle is seen edge-on. The
/// ray/plane hit is ill-conditioned there; a ring switches to [`EDGE_ON_RADIAN_PX`],
/// a planar handle is drawn but not picked.
const EDGE_ON: f64 = 0.08;
/// Edge-on ring: pointer travel along the ring's screen tangent per radian, px
/// (the default ring radius, so a quarter radius ≈ 14°).
const EDGE_ON_RADIAN_PX: f64 = 120.0;
/// An arrow shorter than this on screen points (nearly) at the camera: drawn but
/// not picked, its screen direction is no use for a drag.
const END_ON_PX: f32 = 18.0;
/// Shift snaps translation to a 1-2-5 step no shorter than this many pixels at the
/// press, so the step follows zoom.
const SNAP_PX: f64 = 10.0;
/// Rotation snap with Shift: 15°.
const ROTATION_STEP: f64 = std::f64::consts::PI / 12.0;
/// Half length of the axis guide line during an axis drag, px.
const GUIDE_PX: f64 = 4000.0;
/// Dash and gap of `Entry`/`Local` shafts and rings, px.
const DASHED: Dash = Dash {
    dash: 9.0,
    gap: 5.0,
};
/// Dash and gap of the travel line, px.
const TRAVEL: Dash = Dash {
    dash: 6.0,
    gap: 4.0,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub pivot: DVec3,
    /// Local axes; identity selects world axes. Scale factors use these axes.
    pub orientation: DQuat,
}

impl Placement {
    fn axis(self, axis: Axis) -> DVec3 {
        self.orientation * axis.unit()
    }
    fn validate(self) {
        assert!(self.pivot.is_finite());
        assert!(
            self.orientation.is_finite() && (self.orientation.length_squared() - 1.0).abs() < 1e-10
        );
    }
}

/// What a gesture has done since the press, in host space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delta {
    /// Offset of the target.
    Translation(DVec3),
    /// Turn about the unit `axis` through the press pivot by `angle` radians, right
    /// handed; more than a full turn is kept (450° is not 90°).
    Rotation { axis: DVec3, angle: f64 },
    /// Positive factors along the press placement's axes, about its pivot.
    Scale(DVec3),
}

impl Delta {
    /// Rotation and scaling preserve the gesture's pivot. Callers can use this
    /// for previews without introducing transforms into their domain model.
    pub fn apply(self, point: DVec3, placement: Placement) -> DVec3 {
        placement.validate();
        let moved = match self {
            Self::Translation(offset) => point + offset,
            Self::Rotation { axis, angle } => {
                placement.pivot + DQuat::from_axis_angle(axis, angle) * (point - placement.pivot)
            }
            Self::Scale(factors) => {
                placement.pivot
                    + placement.orientation
                        * (placement.orientation.conjugate() * (point - placement.pivot) * factors)
            }
        };
        assert!(
            moved.is_finite(),
            "Delta::apply: the moved point must be finite"
        );
        moved
    }
}

/// `Updated`/`Finished` carry the world delta from the press. For a listed slide it
/// lies along the spec direction (`hand::slide_delta` gives the joint increment), for a planar handle it lies in the pair's plane
/// (`SlidePlane::split`), for a ring it turns about the spec direction (the angle
/// is the joint increment).
#[derive(Clone, Copy, Debug)]
pub enum Event<K = Axis> {
    Started(Handle<K>),
    Updated(Delta),
    Finished(Delta),
    Cancelled,
}

/// Sizes of the gizmo on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Arrow length and ring radius, px.
    pub radius: f32,
    /// How near the pointer must come to a handle to take it, px.
    pub hit_radius: f32,
    /// Shaft and ring width, px.
    pub stroke: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            radius: 120.0,
            hit_radius: 9.0,
            stroke: 3.0,
        }
    }
}

impl Style {
    fn validate(&self) {
        assert!(self.radius.is_finite() && self.radius > 20.0);
        assert!(self.hit_radius.is_finite() && self.hit_radius > 0.0);
        assert!(self.stroke.is_finite() && self.stroke > 0.0);
    }
}

/// The gizmo's state between frames: the gesture in progress, if any.
pub struct Gizmo<K = Axis> {
    drag: Option<Drag<K>>,
    pub style: Style,
}

impl<K> Default for Gizmo<K> {
    fn default() -> Self {
        Self {
            drag: None,
            style: Style::default(),
        }
    }
}

struct Drag<K> {
    mode: Mode,
    motion: Motion,
    handle: Handle<K>,
    /// The set at the press: the gesture and its drawing use it, whatever the host
    /// passes later (it may move earlier joints, the handle axes must not float).
    set: HandleSet<K>,
    grip: Grip,
    start: Pos2,
    normal: DVec3,
    anchor: DVec3,
    screen_axis: Vec2,
    /// Screen-right direction in the view handle's press plane, for snapping.
    view_right: DVec3,
    /// Ring seen edge-on at the press: the angle is the pointer travel along
    /// `screen_axis` (the nearest ring point's tangent), not a plane-hit angle.
    edge_on: bool,
    /// Host units per pixel at the press; the camera holds still during a gesture.
    unit: f64,
    /// Translation snap step, fixed at the press (see [`SNAP_PX`]).
    step: f64,
    last_angle: f64,
    turns: f64,
    delta: Delta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// Polyline: shaft, ring.
    Line,
    /// Opaque convex polygon: cone, cube, view handle.
    Solid,
    /// See-through convex polygon: planar square.
    Translucent,
}

struct Part<K> {
    handle: Handle<K>,
    points: Vec<Pos2>,
    fill: Fill,
    ink: Ink,
    /// Axis letter drawn next to this part.
    letter: Option<(Pos2, Axis)>,
    /// `Entry`/`Local` freedom: lines and outlines drawn dashed. Picking ignores it.
    dashed: bool,
    pick: Pick,
}

/// How a drawn part takes part in the hit-test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Pick {
    Handle,
    /// An edge-on ring: its segment ends lie on the other rings, so it is picked
    /// only where no regular handle is in reach.
    Fallback,
    /// An end-on arrow or an edge-on plane: the freedom stays visible from any
    /// view, but there is no drag direction to grab it by.
    Never,
}

/// Drawn but never picked: far halves of rings, the sphere outline, the triad.
enum Decor {
    Line(Vec<Pos2>, Stroke),
    Dot(Pos2, f32, Paint),
}

impl Decor {
    fn shape(&self) -> Shape {
        match self {
            Self::Line(points, stroke) => Shape::Line {
                points: points.clone(),
                stroke: *stroke,
                dash: None,
            },
            Self::Dot(at, radius, paint) => Shape::Dot {
                at: *at,
                radius: *radius,
                paint: *paint,
            },
        }
    }
}

/// Everything the gizmo draws in one frame; its parts are also the hit-test.
pub struct Layout<K> {
    parts: Vec<Part<K>>,
    decor: Vec<Decor>,
}

impl<K: Copy> Layout<K> {
    /// The handle under `point`: within `hit_radius` px of a pickable part, a
    /// regular handle before an edge-on ring, then the nearest.
    pub fn hit(&self, point: Pos2, hit_radius: f32) -> Option<Handle<K>> {
        self.parts
            .iter()
            .filter(|part| part.pick != Pick::Never)
            .filter_map(|part| {
                let distance = part.distance(point);
                (distance <= hit_radius).then_some((part.handle, part.pick, distance))
            })
            .min_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)))
            .map(|hit| hit.0)
    }
}

/// Indices of the longest run of `flags[i] == want` on a closed loop, in order.
/// A great circle cut by a plane through its centre has one near and one far arc,
/// so each run is the whole arc. All equal → the full loop, closed.
fn arc(flags: &[bool], want: bool) -> Vec<usize> {
    let n = flags.len();
    if flags.iter().all(|f| *f == want) {
        return (0..=n).map(|i| i % n).collect();
    }
    let Some(start) = (0..n).find(|&i| flags[i] == want && flags[(i + n - 1) % n] != want) else {
        return Vec::new();
    };
    (0..n)
        .map(|k| (start + k) % n)
        .take_while(|&i| flags[i] == want)
        .collect()
}

/// Screen circle around `center`, closed.
fn screen_circle(center: Pos2, radius: f32) -> Vec<Pos2> {
    (0..=RING_SEGMENTS)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / RING_SEGMENTS as f32;
            center + emath::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

fn plane_hit(view: &impl GizmoView, point: Pos2, pivot: DVec3, normal: DVec3) -> Option<DVec3> {
    let (origin, direction) = view.ray(point)?;
    let denominator = normal.dot(direction);
    if denominator.abs() < 1e-5 {
        return None;
    }
    let distance = normal.dot(pivot - origin) / denominator;
    (distance >= 0.0).then(|| origin + direction * distance - pivot)
}

/// 1-2-5 step: the smallest of `{1, 2, 5} × 10^k` not shorter than `raw`.
fn nice_step(raw: f64) -> f64 {
    assert!(raw.is_finite() && raw > 0.0, "nice_step: {raw}");
    let power = 10_f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|step| *step >= raw * (1.0 - 1e-12))
        .expect("10 × 10^floor(log10 raw) ≥ raw")
}

/// Solid or dashed polyline.
fn line(points: Vec<Pos2>, stroke: Stroke, dashed: bool) -> Shape {
    Shape::Line {
        points,
        stroke,
        dash: dashed.then_some(DASHED),
    }
}

impl<K> Part<K> {
    fn distance(&self, point: Pos2) -> f32 {
        let closed = self.fill != Fill::Line;
        if closed {
            let mut positive = false;
            let mut negative = false;
            for (&a, &b) in self.points.iter().zip(self.points.iter().cycle().skip(1)) {
                let cross = (b - a).rot90().dot(point - a);
                positive |= cross > 0.0;
                negative |= cross < 0.0;
            }
            if !(positive && negative) {
                return 0.0;
            }
        }
        let mut distance = f32::INFINITY;
        for pair in self.points.windows(2) {
            distance = distance.min(segment_distance(point, pair[0], pair[1]));
        }
        if closed {
            distance = distance.min(segment_distance(
                point,
                *self.points.last().unwrap(),
                self.points[0],
            ));
        }
        distance
    }

    fn shapes(&self, style: &Style, highlighted: bool, out: &mut Vec<Shape>) {
        let paint = Paint::of(if highlighted {
            Ink::Highlight
        } else {
            self.ink
        });
        let fill = match self.fill {
            Fill::Line => None,
            Fill::Solid => Some(paint),
            Fill::Translucent => Some(paint.faded(if highlighted { 0.35 } else { 0.18 })),
        };
        if let Some(fill) = fill {
            // Explicit triangles keep edge-on polygons bounded; no miter joins.
            out.push(Shape::Fan {
                points: self.points.clone(),
                paint: fill,
            });
            let mut outline = self.points.clone();
            outline.push(self.points[0]);
            let edge = if self.dashed {
                Stroke::new(1.5, paint)
            } else if matches!(self.fill, Fill::Translucent) {
                Stroke::new(1.0, Paint::of(self.ink))
            } else {
                Stroke::new(1.0, Paint::of(Ink::Black(120)))
            };
            out.push(line(outline, edge, self.dashed));
        } else {
            let shifted: Vec<Pos2> = self
                .points
                .iter()
                .map(|p| *p + emath::vec2(0.0, 1.0))
                .collect();
            out.push(line(
                shifted,
                Stroke::new(style.stroke + 2.0, Paint::of(Ink::Black(100))),
                self.dashed,
            ));
            out.push(line(
                self.points.clone(),
                Stroke::new(style.stroke, paint),
                self.dashed,
            ));
        }
        if let Some((at, axis)) = self.letter {
            out.push(Shape::Letter {
                at,
                text: axis.letter(),
                paint,
            });
        }
    }
}

fn layout<K: Copy>(
    view: &impl GizmoView,
    set: &HandleSet<K>,
    mode: Mode,
    style: &Style,
) -> Layout<K> {
    let placement = set.placement;
    let mut parts = Vec::new();
    let mut decor = Vec::new();
    let Some(center) = view.project(placement.pivot) else {
        return Layout { parts, decor };
    };
    let unit = view.world_per_pixel(placement.pivot);
    assert!(unit.is_finite() && unit > 0.0);
    let length = unit * f64::from(style.radius);
    let forward = view.forward();
    let project = |offset: DVec3| view.project(placement.pivot + offset * length);
    // Translate and Scale show the slides, Rotate the turns; the rest have no handle.
    let shown = if mode == Mode::Rotate {
        JointKind::Turn
    } else {
        JointKind::Slide
    };
    let specs: Vec<&HandleSpec<K>> = set
        .handles
        .iter()
        .filter(|spec| spec.kind == shown)
        .collect();
    if mode == Mode::Rotate && (set.screen || !specs.is_empty()) {
        // Sphere outline: the rings read as great circles of one ball.
        decor.push(Decor::Line(
            screen_circle(center, style.radius),
            Stroke::new(1.0, Paint::of(Ink::White(40))),
        ));
    }
    for spec in &specs {
        let direction = spec.direction;
        let axis = spec.axis;
        let dashed = spec.dashed;
        let (u, v) = across(direction, axis, placement.orientation);
        if mode == Mode::Rotate {
            if let Some(tip) = project(direction * TRIAD) {
                // Facing the camera, the triad leg shrinks to a dot: still the
                // colour of its ring, drawn over the centre.
                decor.push(Decor::Line(
                    vec![center, tip],
                    Stroke::new(2.5, Paint::of(Ink::Axis(axis))),
                ));
                decor.push(Decor::Dot(tip, 3.0, Paint::of(Ink::Axis(axis))));
            }
            // An edge-on ring is drawn and picked as a segment; its drag is
            // linear (`Drag::begin`).
            let edge_on = direction.dot(forward).abs() < EDGE_ON;
            let offsets: Vec<DVec3> = (0..RING_SEGMENTS)
                .map(|i| {
                    let angle = i as f64 * std::f64::consts::TAU / RING_SEGMENTS as f64;
                    u * angle.cos() + v * angle.sin()
                })
                .collect();
            let Some(screen) = offsets
                .iter()
                .map(|o| project(*o))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            // Near half is the handle; the far half is a dim hint and never picks,
            // so crossings behind the ball cannot steal the grab.
            let near: Vec<bool> = offsets
                .iter()
                .map(|o| o.dot(forward) < FRONT_SLACK)
                .collect();
            let far = arc(&near, false);
            if far.len() > 1 {
                decor.push(Decor::Line(
                    far.iter().map(|&i| screen[i]).collect(),
                    Stroke::new(1.5, Paint::of(Ink::Axis(axis)).faded(0.35)),
                ));
            }
            let front = arc(&near, true);
            // Letter at the nearest point; a ring facing the camera has no nearest
            // point, there the top of the screen wins. The tie-break weight moves
            // the letter of a tilted ring by under 0.1° (≪ one 3.75° segment).
            // Edge-on the nearest point projects onto the centre, under the
            // triad: the letter goes to the segment end instead.
            let radius_px = f64::from(style.radius);
            let score = |i: usize| {
                let primary = if edge_on {
                    f64::from(screen[i].distance(center)) / radius_px
                } else {
                    -offsets[i].dot(forward)
                };
                primary - 1e-3 * f64::from(screen[i].y - center.y) / radius_px
            };
            let letter = front
                .iter()
                .copied()
                .max_by(|a, b| score(*a).total_cmp(&score(*b)))
                .map(|i| (screen[i] + (screen[i] - center).normalized() * 14.0, axis));
            if front.len() > 1 {
                parts.push(Part {
                    handle: Handle::Ring(spec.key),
                    points: front.iter().map(|&i| screen[i]).collect(),
                    fill: Fill::Line,
                    ink: Ink::Axis(axis),
                    letter,
                    dashed,
                    pick: if edge_on {
                        Pick::Fallback
                    } else {
                        Pick::Handle
                    },
                });
            }
            continue;
        }
        let (Some(start), Some(tip)) = (project(direction * SHAFT_START), project(direction))
        else {
            continue;
        };
        // Nearly end-on arrows give no usable screen direction; the cone still
        // shows as a disc of the axis colour.
        let pick = if center.distance(tip) >= END_ON_PX {
            Pick::Handle
        } else {
            Pick::Never
        };
        let handle = Handle::Axis(spec.key);
        let (shaft_end, head) = if mode == Mode::Scale {
            let corners = (0..8).map(|i| {
                let sign = |bit: usize| if i & bit == 0 { -CUBE_HALF } else { CUBE_HALF };
                project(direction * (CUBE_CENTER + sign(1)) + u * sign(2) + v * sign(4))
            });
            (CUBE_CENTER - CUBE_HALF, corners.collect::<Option<Vec<_>>>())
        } else {
            let base = (0..16).map(|i| {
                let angle = f64::from(i) * std::f64::consts::TAU / 16.0;
                project(direction * CONE_BASE + (u * angle.cos() + v * angle.sin()) * CONE_RADIUS)
            });
            let tip = std::iter::once(Some(tip));
            (CONE_BASE, base.chain(tip).collect::<Option<Vec<_>>>())
        };
        let Some(shaft_end) = project(direction * shaft_end) else {
            continue;
        };
        parts.push(Part {
            handle,
            points: vec![start, shaft_end],
            fill: Fill::Line,
            ink: Ink::Axis(axis),
            letter: None,
            dashed,
            pick,
        });
        if let Some(head) = head {
            parts.push(Part {
                handle,
                points: convex_hull(head),
                fill: Fill::Solid,
                ink: Ink::Axis(axis),
                letter: project(direction * LETTER_AT).map(|at| (at, axis)),
                dashed: false,
                pick,
            });
        }
    }
    if mode == Mode::Translate {
        // A planar handle for every non-collinear pair of slides, in list order.
        // Non-orthogonal axes give a parallelogram: its sides run along the two
        // freedoms, so what is drawn is exactly where the pair can go.
        for (index, first) in specs.iter().enumerate() {
            for second in &specs[index + 1..] {
                if SlidePlane::new(first.direction, second.direction).is_none() {
                    continue;
                }
                let u = first.direction;
                let v = second.direction;
                let pick = if u.cross(v).normalize().dot(forward).abs() >= EDGE_ON {
                    Pick::Handle
                } else {
                    Pick::Never
                };
                let points: Option<Vec<_>> = [
                    (PLANE_MIN, PLANE_MIN),
                    (PLANE_MAX, PLANE_MIN),
                    (PLANE_MAX, PLANE_MAX),
                    (PLANE_MIN, PLANE_MAX),
                ]
                .map(|(x, y)| project(u * x + v * y))
                .into_iter()
                .collect();
                let ink = if first.axis == second.axis {
                    Ink::SameLetterPlane
                } else {
                    Ink::Axis(Axis::ALL[3 - first.axis.index() - second.axis.index()])
                };
                if let Some(points) = points {
                    parts.push(Part {
                        handle: Handle::Plane(first.key, second.key),
                        points,
                        fill: Fill::Translucent,
                        ink,
                        letter: None,
                        dashed: first.dashed || second.dashed,
                        pick,
                    });
                }
            }
        }
    }
    if !set.screen {
        return Layout { parts, decor };
    }
    if mode == Mode::Rotate {
        decor.push(Decor::Dot(center, 3.5, Paint::of(Ink::View)));
        parts.push(Part {
            handle: Handle::View,
            points: screen_circle(center, style.radius * VIEW_RING as f32),
            fill: Fill::Line,
            ink: Ink::View,
            letter: None,
            dashed: false,
            pick: Pick::Handle,
        });
    } else {
        let mut disc = screen_circle(center, VIEW_HALF);
        disc.pop();
        parts.push(Part {
            handle: Handle::View,
            points: disc,
            fill: Fill::Solid,
            ink: Ink::View,
            letter: None,
            dashed: false,
            pick: Pick::Handle,
        });
    }
    Layout { parts, decor }
}

/// Ring radius of a rotation handle, × `Style::radius`.
fn ring_scale(grip: Grip) -> f64 {
    if matches!(grip, Grip::View) {
        VIEW_RING
    } else {
        1.0
    }
}

/// The value a drag has reached, as the plate next to the feedback shows it, in
/// the units of the freedom: a slide along its own direction, a planar pair as its
/// two joint increments (Gram split: for a skewed pair not the projections),
/// rotation in degrees with its sign, scale as factors. The screen handle reads in
/// the placement axes.
fn feedback_text(grip: Grip, delta: Delta, placement: Placement) -> String {
    match (delta, grip) {
        (Delta::Translation(t), Grip::Arrow { direction, axis }) => {
            format!("{} {:+.2}", axis.letter(), t.dot(direction))
        }
        (
            Delta::Translation(t),
            Grip::Plane {
                plane,
                first,
                second,
            },
        ) => {
            let (a, b) = plane.split(t);
            format!(
                "{} {:+.2}  {} {:+.2}",
                first.1.letter(),
                a,
                second.1.letter(),
                b
            )
        }
        (Delta::Translation(t), _) => {
            let [x, y, z] = Axis::ALL.map(|a| t.dot(placement.axis(a)));
            format!("Δ {x:+.2} {y:+.2} {z:+.2}  |{:.2}|", t.length())
        }
        (Delta::Rotation { angle, .. }, _) => format!("{:+.1}°", angle.to_degrees()),
        (Delta::Scale(factors), Grip::Arrow { axis, .. }) => {
            format!("{} ×{:.3}", axis.letter(), factors[axis.index()])
        }
        (Delta::Scale(factors), _) => format!("×{:.3}", factors.x),
    }
}

/// `value` rounded to the nearest multiple of `step`.
fn snapped(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}

impl<K: Copy + PartialEq + fmt::Debug> Drag<K> {
    fn begin(
        view: &impl GizmoView,
        set: &HandleSet<K>,
        mode: Mode,
        handle: Handle<K>,
        start: Pos2,
    ) -> Option<Self> {
        let placement = set.placement;
        let motion = Motion::of(mode, handle);
        let grip = set.grip(handle);
        let forward = view.forward();
        let normal = match grip {
            Grip::Plane { first, second, .. } => first.0.cross(second.0).normalize(),
            Grip::Ring { direction } => direction,
            Grip::Arrow { direction, .. } => {
                let normal = forward - direction * forward.dot(direction);
                if normal.length_squared() < 1e-8 {
                    return None;
                }
                normal.normalize()
            }
            Grip::View => forward,
        };
        let unit = view.world_per_pixel(placement.pivot);
        // Screen direction of a world `direction` at the pivot.
        let on_screen = |direction: DVec3| -> Option<Vec2> {
            let center = view.project(placement.pivot)?;
            let end = view.project(placement.pivot + direction * unit * 80.0)?;
            (center.distance(end) >= 1.0).then(|| (end - center).normalized())
        };
        // The edge-on ring plane holds the line of sight, its plane hit
        // degenerates. The grabbed point is the ring point nearest to the camera:
        // +angle moves it along `normal × near`, so the ring follows the pointer.
        let edge_on = matches!(grip, Grip::Ring { .. }) && normal.dot(forward).abs() < EDGE_ON;
        let (anchor, screen_axis) = if edge_on {
            let near = (normal * normal.dot(forward) - forward).normalize();
            (near, on_screen(normal.cross(near))?)
        } else {
            let anchor = plane_hit(view, start, placement.pivot, normal)?;
            if motion == Motion::Rotate && anchor.length_squared() < 1e-12 {
                return None;
            }
            let screen_axis = match grip {
                Grip::Arrow { direction, .. } => on_screen(direction)?,
                _ => emath::vec2(1.0, -1.0).normalized(),
            };
            (anchor, screen_axis)
        };
        let view_right = if matches!(grip, Grip::View) {
            plane_hit(view, start + emath::vec2(1.0, 0.0), placement.pivot, normal)
                .map(|hit| hit - anchor)
                .map(|right| right - normal * right.dot(normal))
                .and_then(DVec3::try_normalize)
                .unwrap_or_else(|| normal.any_orthonormal_vector())
        } else {
            DVec3::ZERO
        };
        let delta = match motion {
            Motion::Translate => Delta::Translation(DVec3::ZERO),
            Motion::Rotate => Delta::Rotation {
                axis: normal,
                angle: 0.0,
            },
            Motion::Scale => Delta::Scale(DVec3::ONE),
        };
        Some(Self {
            mode,
            motion,
            handle,
            set: set.clone(),
            grip,
            start,
            normal,
            anchor,
            screen_axis,
            view_right,
            edge_on,
            unit,
            step: nice_step(unit * SNAP_PX),
            last_angle: 0.0,
            turns: 0.0,
            delta,
        })
    }

    fn sample(&mut self, view: &impl GizmoView, point: Pos2, snap: bool) {
        if self.motion == Motion::Scale {
            // Exponential scale cannot cross zero. Shift snaps in 10% steps.
            let log = f64::from((point - self.start).dot(self.screen_axis)) / 100.0;
            let factor = if snap {
                (log / 1.1_f64.ln()).round() * 1.1_f64.ln()
            } else {
                log
            }
            .clamp(-10.0, 10.0)
            .exp();
            let mut factors = DVec3::ONE;
            match self.grip {
                Grip::Arrow { axis, .. } => factors[axis.index()] = factor,
                Grip::View => factors = DVec3::splat(factor),
                Grip::Plane { .. } | Grip::Ring { .. } => {
                    unreachable!("Motion::of never scales them")
                }
            }
            self.delta = Delta::Scale(factors);
            return;
        }
        if self.edge_on {
            let angle = f64::from((point - self.start).dot(self.screen_axis)) / EDGE_ON_RADIAN_PX;
            self.delta = self.rotation(angle, snap);
            return;
        }
        let Some(current) = plane_hit(view, point, self.set.placement.pivot, self.normal) else {
            return;
        };
        self.delta = match self.motion {
            Motion::Translate => {
                let offset = current - self.anchor;
                let step = |value: f64| {
                    if snap {
                        snapped(value, self.step)
                    } else {
                        value
                    }
                };
                // Components along the freedoms, rounded to the step with Shift.
                // The plane hit already lies in the handle's plane, so without
                // Shift a planar drag is the raw offset.
                let along = |direction: DVec3| direction * step(offset.dot(direction));
                let offset = match self.grip {
                    Grip::Arrow { direction, .. } => along(direction),
                    Grip::Plane {
                        plane,
                        first,
                        second,
                    } if snap => {
                        // Round the joint increments, not the projections: for a
                        // skewed pair they differ (Gram split).
                        let (a, b) = plane.split(offset);
                        first.0 * step(a) + second.0 * step(b)
                    }
                    Grip::View if snap => {
                        along(self.view_right) + along(self.normal.cross(self.view_right))
                    }
                    Grip::Plane { .. } | Grip::View => offset,
                    Grip::Ring { .. } => unreachable!("rings rotate"),
                };
                Delta::Translation(offset)
            }
            Motion::Rotate => {
                if current.length_squared() < 1e-12 {
                    return;
                }
                let angle = self
                    .normal
                    .dot(self.anchor.cross(current))
                    .atan2(self.anchor.dot(current));
                // Unwrap the cursor phase while evaluating the transform from
                // the immutable press vector, allowing more than one full turn.
                let difference = angle - self.last_angle;
                if difference > std::f64::consts::PI {
                    self.turns -= std::f64::consts::TAU;
                }
                if difference < -std::f64::consts::PI {
                    self.turns += std::f64::consts::TAU;
                }
                self.last_angle = angle;
                self.rotation(angle + self.turns, snap)
            }
            Motion::Scale => unreachable!(),
        };
    }

    /// Rotation by `angle` about the press normal, in `ROTATION_STEP`s with Shift.
    fn rotation(&self, angle: f64, snap: bool) -> Delta {
        assert!(angle.is_finite(), "rotation: the angle must be finite");
        Delta::Rotation {
            axis: self.normal,
            angle: if snap {
                (angle / ROTATION_STEP).round() * ROTATION_STEP
            } else {
                angle
            },
        }
    }

    /// Where the handles are drawn during the drag: translation carries them with
    /// the target; rotation and scale keep them at the press placement.
    fn draw_placement(&self) -> Placement {
        let mut placement = self.set.placement;
        if let Delta::Translation(offset) = self.delta {
            placement.pivot += offset;
        }
        placement
    }

    /// The press set at [`Self::draw_placement`].
    fn draw_set(&self) -> HandleSet<K> {
        HandleSet {
            placement: self.draw_placement(),
            ..self.set.clone()
        }
    }

    /// Screen polygon of the swept rotation: the ring arc from the press direction
    /// through the signed angle (clamped to one full turn for drawing), radius as
    /// the ring. `None` for other motions or off-screen points.
    fn sector(&self, view: &impl GizmoView, style: &Style) -> Option<Vec<Pos2>> {
        let Delta::Rotation { angle, .. } = self.delta else {
            return None;
        };
        // Positive angle turns the press direction toward `normal × press`
        // (the sign of `sample`'s atan2).
        let from = self.anchor.normalize();
        let across = self.normal.cross(from);
        let radius = self.unit * f64::from(style.radius) * ring_scale(self.grip);
        let sweep = angle.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
        let steps = ((sweep.abs() / std::f64::consts::TAU * 96.0).ceil() as usize).max(1);
        (0..=steps)
            .map(|i| {
                let phi = sweep * i as f64 / steps as f64;
                view.project(
                    self.set.placement.pivot + (from * phi.cos() + across * phi.sin()) * radius,
                )
            })
            .collect()
    }

    fn feedback(&self, view: &impl GizmoView, style: &Style, out: &mut Vec<Shape>) {
        let pivot = self.set.placement.pivot;
        let Some(origin) = view.project(pivot) else {
            return;
        };
        let line = Stroke::new(1.5, Paint::of(Ink::Feedback));
        let text = feedback_text(self.grip, self.delta, self.set.placement);
        match self.delta {
            Delta::Translation(offset) => {
                if let Grip::Arrow { direction, axis } = self.grip {
                    let reach = direction * self.unit * GUIDE_PX;
                    if let (Some(a), Some(b)) =
                        (view.project(pivot - reach), view.project(pivot + reach))
                    {
                        out.push(Shape::Line {
                            points: vec![a, b],
                            stroke: Stroke::new(1.0, Paint::of(Ink::Axis(axis)).faded(0.7)),
                            dash: None,
                        });
                    }
                }
                let Some(current) = view.project(pivot + offset) else {
                    return;
                };
                out.push(Shape::Line {
                    points: vec![origin, current],
                    stroke: line,
                    dash: Some(TRAVEL),
                });
                out.push(Shape::Circle {
                    at: origin,
                    radius: 5.0,
                    stroke: line,
                });
                out.push(Shape::Plate {
                    at: current + emath::vec2(18.0, -30.0),
                    text,
                });
            }
            Delta::Rotation { .. } => {
                let Some(arc) = self.sector(view, style) else {
                    return;
                };
                // A sector is star-shaped from its centre: the fan covers it for
                // any sweep up to a full turn.
                let mut fan = Vec::with_capacity(arc.len() + 1);
                fan.push(origin);
                fan.extend_from_slice(&arc);
                out.push(Shape::Fan {
                    points: fan,
                    paint: Paint::of(Ink::Feedback).faded(0.3),
                });
                for end in [arc[0], *arc.last().unwrap()] {
                    out.push(Shape::Line {
                        points: vec![origin, end],
                        stroke: line,
                        dash: None,
                    });
                }
                let middle = arc[arc.len() / 2];
                let outward = (middle - origin).normalized() * 22.0;
                out.push(Shape::Line {
                    points: arc,
                    stroke: Stroke::new(2.0, Paint::of(Ink::Feedback)),
                    dash: None,
                });
                out.push(Shape::Plate {
                    at: middle + outward,
                    text,
                });
            }
            Delta::Scale(_) => out.push(Shape::Plate {
                at: origin + emath::vec2(18.0, -30.0),
                text,
            }),
        }
    }
}

impl<K: Copy + PartialEq + fmt::Debug> Gizmo<K> {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The handle held by the gesture in progress.
    pub fn active(&self) -> Option<Handle<K>> {
        self.drag.as_ref().map(|drag| drag.handle)
    }

    /// The mode the gesture in progress started in. A host cancels the gesture
    /// when its mode changes.
    pub fn mode(&self) -> Option<Mode> {
        self.drag.as_ref().map(|drag| drag.mode)
    }

    /// Ends the gesture in progress, if any: Esc, focus loss, a mode change, or the
    /// selected target disappearing, an owning panel closing or the grabbed freedom
    /// leaving the set (the gesture runs on the press snapshot and would not
    /// notice).
    pub fn cancel(&mut self) -> Option<Event<K>> {
        self.drag.take().map(|_| Event::Cancelled)
    }

    /// What this frame shows of `set` in `mode`: only the handles of `set` that
    /// belong to `mode`. While a gesture runs the press set is shown, carried with
    /// a translation.
    pub fn layout(&self, view: &impl GizmoView, set: &HandleSet<K>, mode: Mode) -> Layout<K> {
        set.validate();
        self.style.validate();
        let drawn = self
            .drag
            .as_ref()
            .map_or_else(|| set.clone(), Drag::draw_set);
        layout(view, &drawn, mode, &self.style)
    }

    /// A primary press at `at` over `handle` (from [`Layout::hit`]) with no gesture
    /// in progress: `Started` when the handle gives a drag from this view.
    pub fn press(
        &mut self,
        view: &impl GizmoView,
        set: &HandleSet<K>,
        mode: Mode,
        handle: Handle<K>,
        at: Pos2,
    ) -> Option<Event<K>> {
        assert!(self.drag.is_none(), "Gizmo::press during a gesture");
        self.drag = Drag::begin(view, set, mode, handle, at);
        self.drag.as_ref().map(|_| Event::Started(handle))
    }

    /// A frame of the gesture in progress with the pointer at `at` (`None` keeps
    /// the last value): the delta from the press. `snap` (Shift) rounds it.
    pub fn drag(&mut self, view: &impl GizmoView, at: Option<Pos2>, snap: bool) -> Event<K> {
        let drag = self.drag.as_mut().expect("Gizmo::drag without a gesture");
        if let Some(point) = at {
            drag.sample(view, point, snap);
        }
        Event::Updated(drag.delta)
    }

    /// The primary button released: the gesture ends with its last delta.
    pub fn release(&mut self) -> Event<K> {
        let drag = self.drag.take().expect("Gizmo::release without a gesture");
        Event::Finished(drag.delta)
    }

    /// What to draw for `layout`, back to front: decoration, the feedback of the
    /// gesture in progress (under the handles, so the sector never hides a ring),
    /// then the handles, `highlighted` (the held or hovered one) in
    /// [`Ink::Highlight`].
    pub fn shapes(
        &self,
        view: &impl GizmoView,
        layout: &Layout<K>,
        highlighted: Option<Handle<K>>,
    ) -> Vec<Shape> {
        let mut out: Vec<Shape> = layout.decor.iter().map(Decor::shape).collect();
        if let Some(drag) = &self.drag {
            drag.feedback(view, &self.style, &mut out);
        }
        for part in &layout.parts {
            part.shapes(&self.style, highlighted == Some(part.handle), &mut out);
        }
        out
    }
}

#[cfg(test)]
#[path = "gizmo_tests.rs"]
mod tests;
