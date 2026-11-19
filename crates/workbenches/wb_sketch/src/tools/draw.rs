//! Drawing tools: each handler advances the state machine one click and
//! materializes geometry only when the shape completes.

use uuid::Uuid;

use super::ToolParams;
use super::{ToolEffect, ToolState, held_point, materialize, materialize_on_curve, short};
use crate::geom2d;
use crate::sketch::{
    Arc, BSpline, Circle, Conic, ConicKind, ConstraintKind, Ellipse, GeometryElement, Line, Point,
    Sketch, Vec2D,
};
use crate::snap::{self, SnapTarget, arc_angles};
use crate::spline;

/// Point snap first (its id reused, never a coincident duplicate); otherwise a
/// curve within tolerance captures the click, projecting the position onto
/// it so `materialize_on_curve` records the on-curve constraint.
pub(crate) fn snap_point_or_curve(sketch: &Sketch, cursor: Vec2D, tol: f64) -> SnapTarget {
    match snap::snap_to_point(sketch, cursor, tol, &[]) {
        SnapTarget::Existing(id) => SnapTarget::Existing(id),
        SnapTarget::New(pos) => match snap::snap_to_curve(sketch, cursor, tol, &[]) {
            Some((_, proj)) => SnapTarget::New(proj),
            None => SnapTarget::New(pos),
        },
    }
}

/// Whether two of `targets` are one existing point: a shape through them
/// would join that point to itself.
fn joins_itself<'a>(targets: impl IntoIterator<Item = &'a SnapTarget>) -> bool {
    let mut seen = std::collections::HashSet::new();
    targets
        .into_iter()
        .any(|t| matches!(t, SnapTarget::Existing(id) if !seen.insert(*id)))
}

pub(super) fn point(sketch: &mut Sketch, cursor: Vec2D, snap_tol: f64) -> ToolEffect {
    // A snap onto an existing point would only double it.
    if let SnapTarget::Existing(_) =
        snap::snap_to_point(sketch, cursor, super::curve_attach_eps(snap_tol), &[])
    {
        return ToolEffect::log("A point is already there");
    }
    let id = materialize_on_curve(sketch, SnapTarget::New(cursor), snap_tol);
    ToolEffect::changed(format!(
        "Point at ({:.2}, {:.2}) [{}]",
        cursor.x,
        cursor.y,
        short(id)
    ))
}

pub(super) fn circle(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    match *state {
        ToolState::CircleFrom { center } => {
            let Some(center_pos) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let radius = (cursor - center_pos).to_glam().length();
            if radius < 1e-6 {
                return ToolEffect::none();
            }
            let center_id = materialize_on_curve(sketch, center, snap_tol);
            // The rim click, where a snap put it, stays on the circle.
            let rim = snap_point_or_curve(sketch, cursor, super::curve_attach_eps(snap_tol));
            let rim = held_point(sketch, rim, snap_tol);
            let circle =
                sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, radius)));
            hold_on(sketch, rim, circle);
            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Circle r={radius:.2} at ({:.2}, {:.2})",
                center_pos.x, center_pos.y
            ))
        }
        _ => {
            let center = snap_point_or_curve(sketch, cursor, snap_tol);
            *state = ToolState::CircleFrom { center };
            ToolEffect::none()
        }
    }
}

/// The points the clicks at `at` stand for where a snap put them on a
/// point or a curve, each held there ([`held_point`]); `None` for a click
/// on nothing. Taken before the shape through them is added, so none is
/// found on the shape itself.
fn held_clicks<const N: usize>(
    sketch: &mut Sketch,
    at: [Vec2D; N],
    snap_tol: f64,
) -> [Option<Uuid>; N] {
    let eps = super::curve_attach_eps(snap_tol);
    at.map(|p| {
        let target = snap_point_or_curve(sketch, p, eps);
        held_point(sketch, target, snap_tol)
    })
}

/// Keep `point`, when there is one, on `curve`: a circle's or an arc's
/// circle, or an ellipse.
fn hold_on(sketch: &mut Sketch, point: Option<Uuid>, curve: Uuid) {
    let Some(point) = point else {
        return;
    };
    let kind = match sketch.get_geometry(curve) {
        Some(GeometryElement::Ellipse(_)) => ConstraintKind::PointOnEllipse {
            point,
            ellipse: curve,
        },
        _ => ConstraintKind::PointOnCircle {
            point,
            circle: curve,
        },
    };
    sketch.add_constraint(kind);
}

/// Cursor snapped to an existing point's *position* (nothing materialized).
fn snapped_pos(sketch: &Sketch, cursor: Vec2D, snap_tol: f64) -> Vec2D {
    snap::snap_to_point(sketch, cursor, snap_tol, &[])
        .position(sketch)
        .unwrap_or(cursor)
}

pub(super) fn circle3(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::Circle3One { a } => {
            if (pos - a).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::Circle3Two { a, b: pos };
            ToolEffect::none()
        }
        ToolState::Circle3Two { a, b } => {
            let Some(c) = geom2d::circumcenter(a.to_glam(), b.to_glam(), pos.to_glam()) else {
                return ToolEffect::none(); // collinear rim points
            };
            let radius = (a.to_glam() - c).length();
            let rim = held_clicks(sketch, [a, b, pos], snap_tol);
            let center_id =
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(c))));
            let circle =
                sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, radius)));
            for point in rim {
                hold_on(sketch, point, circle);
            }
            *state = ToolState::Idle;
            ToolEffect::changed(format!("Circle r={radius:.2} through 3 points"))
        }
        _ => {
            *state = ToolState::Circle3One { a: pos };
            ToolEffect::none()
        }
    }
}

pub(super) fn arc(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    match *state {
        ToolState::ArcCenter { center } => {
            let start = snap_point_or_curve(sketch, cursor, snap_tol);
            let (Some(c), Some(s)) = (center.position(sketch), start.position(sketch)) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            if (s - c).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::ArcStart { center, start };
            ToolEffect::none()
        }
        ToolState::ArcStart { center, start } => {
            let (Some(center_pos), Some(start_pos)) =
                (center.position(sketch), start.position(sketch))
            else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let radius = (start_pos - center_pos).to_glam().length();
            // The end lies on the arc's circle: a landed click is already
            // there, on what it snapped to, where the end is held
            // (`snap_context`'s guide); a typed one is projected onto it.
            let dir = (cursor - center_pos).to_glam();
            if dir.length() < 1e-6 {
                return ToolEffect::none();
            }
            let end_pos = Vec2D::from_glam(center_pos.to_glam() + dir.normalize() * radius);
            let end_snap = snap::snap_to_point(sketch, end_pos, snap_tol, &[]);
            // An end snapped onto the start would close the arc on one
            // point, no sweep the solver can hold: wait for another end.
            if end_snap
                .position(sketch)
                .is_none_or(|e| (e - start_pos).to_glam().length() < 1e-6)
            {
                return ToolEffect::none();
            }

            let center_id = materialize_on_curve(sketch, center, snap_tol);
            let start_id = materialize_on_curve(sketch, start, snap_tol);
            let end_id = materialize_on_curve(sketch, end_snap, snap_tol);
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id, start_id, end_id, radius,
            )));
            *state = ToolState::Idle;
            ToolEffect::changed(format!("Arc r={radius:.2}"))
        }
        _ => {
            let center = snap_point_or_curve(sketch, cursor, snap_tol);
            *state = ToolState::ArcCenter { center };
            ToolEffect::none()
        }
    }
}

pub(super) fn arc3(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    match *state {
        ToolState::Arc3Start { start } => {
            let end = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            let (Some(s), Some(e)) = (start.position(sketch), end.position(sketch)) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            if (e - s).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::Arc3End { start, end };
            ToolEffect::none()
        }
        ToolState::Arc3End { start, end } => {
            let (Some(s), Some(e)) = (start.position(sketch), end.position(sketch)) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let Some(c) = geom2d::circumcenter(s.to_glam(), e.to_glam(), cursor.to_glam()) else {
                return ToolEffect::none(); // collinear: no arc through here
            };
            let radius = (s.to_glam() - c).length();
            // Arcs are stored CCW: swap the endpoints when the clicked rim
            // point lies on the clockwise side.
            let (a, b) = if geom2d::point_on_arc(c, s.to_glam(), e.to_glam(), cursor.to_glam()) {
                (start, end)
            } else {
                (end, start)
            };
            let center_id =
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(c))));
            let start_id = materialize_on_curve(sketch, a, snap_tol);
            let end_id = materialize_on_curve(sketch, b, snap_tol);
            let [rim] = held_clicks(sketch, [cursor], snap_tol);
            let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id, start_id, end_id, radius,
            )));
            hold_on(sketch, rim, arc);
            *state = ToolState::Idle;
            ToolEffect::changed(format!("Arc r={radius:.2} through 3 points"))
        }
        _ => {
            let start = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::Arc3Start { start };
            ToolEffect::none()
        }
    }
}

pub(super) fn ellipse(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    match *state {
        ToolState::EllipseCenter { center } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            if (cursor - c).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::EllipseMajor {
                center,
                major_pos: cursor,
            };
            ToolEffect::none()
        }
        ToolState::EllipseMajor { center, major_pos } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let major = (major_pos - c).to_glam();
            let major_len = major.length();
            // Minor radius = perpendicular distance of the click from the
            // major axis, expressed as a ratio of the major radius.
            let minor = (major / major_len).perp_dot((cursor - c).to_glam()).abs();
            if minor < 1e-6 {
                return ToolEffect::none(); // click on the axis: degenerate
            }
            // A second radius longer than the first makes it the major.
            let (major, ratio) = crate::geom2d::ellipse_axes(major, minor);
            let major_len = major.length();
            let center_id = materialize_on_curve(sketch, center, snap_tol);
            sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
                center_id,
                Vec2D::from_glam(major),
                ratio,
            )));
            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Ellipse {major_len:.2} × {:.2} at ({:.2}, {:.2})",
                major_len * ratio,
                c.x,
                c.y
            ))
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::EllipseCenter { center };
            ToolEffect::none()
        }
    }
}

/// Ellipse from the two ends of its major axis and a point on its rim.
pub(super) fn ellipse3(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::Ellipse3A { a } => {
            if (pos - a).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::Ellipse3B { a, b: pos };
            ToolEffect::none()
        }
        ToolState::Ellipse3B { a, b } => {
            let center = Vec2D::from_glam((a.to_glam() + b.to_glam()) * 0.5);
            let Some((major, ratio)) = geom2d::ellipse_through(center, b, pos) else {
                return ToolEffect::none(); // the rim point is on the axis line
            };
            let rim = held_clicks(sketch, [a, b, pos], snap_tol);
            let center_id = sketch.add_geometry(GeometryElement::Point(Point::new(center)));
            let ellipse = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
                center_id, major, ratio,
            )));
            for point in rim {
                hold_on(sketch, point, ellipse);
            }
            *state = ToolState::Idle;
            let len = major.to_glam().length();
            ToolEffect::changed(format!(
                "Ellipse {len:.2} × {:.2} through three points",
                len * ratio
            ))
        }
        _ => {
            *state = ToolState::Ellipse3A { a: pos };
            ToolEffect::none()
        }
    }
}

/// Arc of an ellipse: its center, a major-axis vertex, the start on the rim
/// (which sets the minor radius), then the end, counter-clockwise, which
/// lands on the ellipse (`snap_context`'s guide). The arc's endpoints are
/// points held on the ellipse, so it closes profiles with the curves it
/// meets there, and on what their clicks snapped to.
pub(super) fn ellipse_arc(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    match *state {
        ToolState::EllipseArcCenter { center } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            if (cursor - c).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::EllipseArcMajor {
                center,
                major_pos: cursor,
            };
            ToolEffect::none()
        }
        ToolState::EllipseArcMajor { center, major_pos } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let Some((major, ratio)) = geom2d::ellipse_through(c, major_pos, cursor) else {
                return ToolEffect::none();
            };
            *state = ToolState::EllipseArcStart {
                center,
                major,
                ratio,
                start: cursor,
            };
            ToolEffect::none()
        }
        ToolState::EllipseArcStart {
            center,
            major,
            ratio,
            start,
        } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let guide = snap::Guide::Ellipse {
                centre: c,
                major,
                ratio,
            };
            let (Some(start), Some(end)) = (guide.foot(start), guide.foot(cursor)) else {
                return ToolEffect::none();
            };
            let probe = Ellipse::new(Uuid::nil(), major, ratio);
            if (probe.param_at(c, end) - probe.param_at(c, start)).abs() < 1e-4 {
                return ToolEffect::none();
            }
            let Some([start_id, end_id]) = guided_ends(sketch, &guide, [start, end], snap_tol, &[])
            else {
                return ToolEffect::none();
            };
            let center_id = materialize_on_curve(sketch, center, snap_tol);
            let ellipse = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new_arc(
                center_id, major, ratio, start_id, end_id,
            )));
            for point in [start_id, end_id] {
                sketch.add_constraint(ConstraintKind::PointOnEllipse { point, ellipse });
            }
            *state = ToolState::Idle;
            ToolEffect::changed("Arc of ellipse")
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::EllipseArcCenter { center };
            ToolEffect::none()
        }
    }
}

pub(super) fn bspline(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    let target = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
    match state {
        ToolState::BSplineDraw { points } => {
            let prev = points.last().and_then(|t| t.position(sketch));
            let new = target.position(sketch);
            if let (Some(prev), Some(new)) = (prev, new)
                && (new - prev).to_glam().length() < 1e-6
            {
                return ToolEffect::none(); // double-click on the same spot
            }
            points.push(target);
        }
        _ => {
            *state = ToolState::BSplineDraw {
                points: vec![target],
            };
        }
    }
    ToolEffect::none()
}

/// The path tool's click: a node at `cursor`, or the path closed when it
/// lands on the first of two or more nodes, or finished open when it lands
/// again on the last.
pub(super) fn path(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> ToolEffect {
    let target = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
    let node = super::PathNode {
        at: target,
        out: None,
    };
    let ToolState::PathDraw { nodes } = state else {
        *state = ToolState::PathDraw { nodes: vec![node] };
        return ToolEffect::none();
    };
    let near = |t: &SnapTarget| {
        t.position(sketch)
            .is_some_and(|p| (p - cursor).to_glam().length() <= snap_tol)
    };
    if nodes.len() >= 2 && near(&nodes[0].at) {
        return path_finish(state, sketch, true);
    }
    if nodes.last().is_some_and(|last| near(&last.at)) {
        return if nodes.len() >= 2 {
            path_finish(state, sketch, false)
        } else {
            ToolEffect::none()
        };
    }
    nodes.push(node);
    ToolEffect::none()
}

/// The control points of the path through `nodes` (where each stands and
/// the handle dragged from it), closed back to the first when `closed`:
/// node, handle out, handle in, node, … A dragged node's handle toward the
/// start mirrors the one dragged out; an undragged node's handles stand a
/// third of the way along the pieces either side, so pieces between two
/// such nodes are straight.
fn path_control(at: &[Vec2D], out: &[Option<Vec2D>], closed: bool) -> Vec<Vec2D> {
    let n = at.len();
    let pieces = if closed { n } else { n - 1 };
    let mut control = vec![at[0]];
    for i in 0..pieces {
        let (a, b) = (i, (i + 1) % n);
        let third = (at[b] - at[a]).to_glam() / 3.0;
        let after = out[a].unwrap_or(at[a] + Vec2D::from_glam(third));
        let before = match out[b] {
            Some(o) => at[b] + (at[b] - o),
            None => at[b] - Vec2D::from_glam(third),
        };
        control.extend([after, before, at[b]]);
    }
    control
}

/// The path the tool's nodes so far make, with the next node at `next`
/// when the pointer is out: the curve sampled, and the last node's handle
/// lines while it has them.
pub fn path_preview(
    sketch: &Sketch,
    nodes: &[super::PathNode],
    next: Option<Vec2D>,
) -> (Vec<Vec2D>, Vec<[Vec2D; 2]>) {
    let mut at: Vec<Vec2D> = nodes.iter().filter_map(|n| n.at.position(sketch)).collect();
    let mut out: Vec<Option<Vec2D>> = nodes.iter().map(|n| n.out).collect();
    if at.len() != nodes.len() {
        return (Vec::new(), Vec::new());
    }
    let mut handles = Vec::new();
    if let (Some(node), Some(Some(o))) = (at.last().copied(), out.last().copied()) {
        handles.push([node, o]);
        handles.push([node, node + (node - o)]);
    }
    if let Some(next) = next {
        at.push(next);
        out.push(None);
    }
    if at.len() < 2 {
        return (Vec::new(), handles);
    }
    let control = path_control(&at, &out, false);
    let curve = bezier_samples(&control, 24);
    (curve, handles)
}

/// The chain of cubic pieces over `control` (node, handle, handle, node,
/// …), `steps` samples a piece.
fn bezier_samples(control: &[Vec2D], steps: usize) -> Vec<Vec2D> {
    let mut points = vec![control[0]];
    for piece in control.windows(4).step_by(3) {
        let [p0, p1, p2, p3] = [piece[0], piece[1], piece[2], piece[3]].map(Vec2D::to_glam);
        for s in 1..=steps {
            let t = s as f64 / steps as f64;
            let u = 1.0 - t;
            let p = p0 * (u * u * u)
                + p1 * (3.0 * u * u * t)
                + p2 * (3.0 * u * t * t)
                + p3 * (t * t * t);
            points.push(Vec2D::from_glam(p));
        }
    }
    points
}

/// Make the path the tool's nodes place, closed back to the first when
/// `closed`: a cubic spline in node form, every node a point of the sketch
/// (one a click snapped to kept), its handles new points, each node a drag
/// pulled out held smooth. Fewer than two nodes make nothing.
pub(super) fn path_finish(state: &mut ToolState, sketch: &mut Sketch, closed: bool) -> ToolEffect {
    let ToolState::PathDraw { nodes } = std::mem::take(state) else {
        return ToolEffect::none();
    };
    if nodes.len() < 2 {
        return ToolEffect::none();
    }
    let Some(at) = nodes
        .iter()
        .map(|n| n.at.position(sketch))
        .collect::<Option<Vec<Vec2D>>>()
    else {
        return ToolEffect::none();
    };
    let out: Vec<Option<Vec2D>> = nodes.iter().map(|n| n.out).collect();
    let control = path_control(&at, &out, closed);
    let node_ids: Vec<Uuid> = nodes.iter().map(|n| materialize(sketch, n.at)).collect();
    let n = node_ids.len();
    let ids: Vec<Uuid> = control
        .iter()
        .enumerate()
        .map(|(i, p)| match (i % 3, i / 3) {
            (0, k) => node_ids[k % n],
            _ => sketch.add_geometry(GeometryElement::Point(Point::new(*p))),
        })
        .collect();
    let pieces = (ids.len() - 1) / 3;
    let spline = sketch.add_geometry(GeometryElement::BSpline(BSpline {
        knots: crate::nodes::path_knots(pieces),
        ..BSpline::new(ids, false)
    }));
    if let Some(path) = crate::nodes::nodes_of(sketch, spline) {
        for (node, placed) in path.nodes.iter().zip(&nodes) {
            if placed.out.is_some() {
                crate::nodes::set_kind(sketch, node, crate::nodes::NodeKind::Smooth);
            }
        }
    }
    let shape = if closed { "Closed path" } else { "Path" };
    ToolEffect::changed(format!("{shape} of {n} nodes"))
}

/// Arc of a parabola or a hyperbola: the vertex (parabola) or centre
/// (hyperbola), then the focus or the branch's vertex, which set the axis;
/// then the arc's start, which on a hyperbola also sets how wide it opens,
/// and its end, each landing on the curve (`snap_context`'s guide). The
/// end points are held on the curve by the conic itself, and on what
/// their clicks snapped to.
pub(super) fn conic(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
    kind: ConicKind,
) -> ToolEffect {
    use crate::conic::{Shape, hyperbola_minor};
    let on_curve = |shape: &Shape, p: Vec2D| {
        let t = shape.param([p.x, p.y]);
        let [x, y] = shape.point(t);
        (t, Vec2D::new(x, y))
    };
    match *state {
        ToolState::ConicAt { kind, center } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let axis = cursor - c;
            if axis.to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::ConicAxis { kind, center, axis };
            ToolEffect::none()
        }
        ToolState::ConicAxis { kind, center, axis } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let minor = match kind {
                ConicKind::Parabola => 0.0,
                ConicKind::Hyperbola => match hyperbola_minor(c, c + axis, cursor) {
                    Some(b) => b,
                    None => return ToolEffect::none(), // not beyond the vertex
                },
            };
            let Some(shape) = Shape::new(kind, c, axis, minor) else {
                return ToolEffect::none();
            };
            let (_, start) = on_curve(&shape, cursor);
            *state = ToolState::ConicStart {
                kind,
                center,
                axis,
                minor,
                start,
            };
            ToolEffect::none()
        }
        ToolState::ConicStart {
            kind,
            center,
            axis,
            minor,
            start,
        } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let Some(shape) = Shape::new(kind, c, axis, minor) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let (t0, start) = on_curve(&shape, start);
            let (t1, end) = on_curve(&shape, cursor);
            if (t1 - t0).abs() < 1e-4 {
                return ToolEffect::none();
            }
            let guide = snap::Guide::Conic(shape);
            // A parabola's vertex lies on it but makes no end.
            let vertex = match center {
                SnapTarget::Existing(id) => vec![id],
                SnapTarget::New(_) => Vec::new(),
            };
            let Some([start_id, end_id]) =
                guided_ends(sketch, &guide, [start, end], snap_tol, &vertex)
            else {
                return ToolEffect::none();
            };
            let center_id = materialize_on_curve(sketch, center, snap_tol);
            sketch.add_geometry(GeometryElement::Conic(Conic::new(
                kind, center_id, axis, minor, start_id, end_id,
            )));
            *state = ToolState::Idle;
            ToolEffect::changed(match kind {
                ConicKind::Parabola => format!(
                    "Arc of parabola, focal distance {:.2}",
                    axis.to_glam().length()
                ),
                ConicKind::Hyperbola => format!(
                    "Arc of hyperbola, semi-axes {:.2} × {minor:.2}",
                    axis.to_glam().length()
                ),
            })
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::ConicAt { kind, center };
            ToolEffect::none()
        }
    }
}

/// The points an arc's ends at `at`, on `guide`, stand for: an existing
/// point on the guide a click landed on, reused, or a new one held on
/// what it lies on, as a snapped click is held; points in `exclude` are
/// never reused. `None` when both landed on one point.
fn guided_ends(
    sketch: &mut Sketch,
    guide: &snap::Guide,
    at: [Vec2D; 2],
    snap_tol: f64,
    exclude: &[Uuid],
) -> Option<[Uuid; 2]> {
    let reused = at.map(
        |p| match snap::snap_to_point(sketch, p, snap_tol, exclude) {
            SnapTarget::Existing(id) => sketch
                .point_position(id)
                .filter(|q| guide.holds(*q))
                .map(|_| id),
            SnapTarget::New(_) => None,
        },
    );
    if reused[0].is_some() && reused[0] == reused[1] {
        return None;
    }
    let mut held = |p: Vec2D, id: Option<Uuid>| {
        id.unwrap_or_else(|| materialize_on_curve(sketch, SnapTarget::New(p), snap_tol))
    };
    Some([held(at[0], reused[0]), held(at[1], reused[1])])
}

/// The spline the tool's clicks so far make with `params`: over them as
/// control points, or through them.
pub fn bspline_preview(clicks: &[Vec2D], params: &ToolParams) -> Option<Vec<Vec2D>> {
    let points: Vec<[f64; 2]> = clicks.iter().map(|p| [p.x, p.y]).collect();
    let periodic = params.bspline_periodic;
    let (degree, knots, control) = if params.bspline_interpolate {
        let fit = sketch_solver::spline::interpolate(&points, params.bspline_degree, periodic)?;
        (fit.degree, fit.knots, fit.control)
    } else {
        (params.bspline_degree, Vec::new(), points)
    };
    let basis = sketch_solver::spline::Basis::new(degree, control.len(), &knots, periodic)?;
    Some(spline::sample_basis(&basis, &control, 48))
}

/// Complete the in-progress B-spline (right-click/Enter). Fewer than 3
/// points cancels without creating geometry. Through points, the clicks
/// become the points it passes through and its control points are worked
/// out from them.
pub(super) fn bspline_finish(
    state: &mut ToolState,
    sketch: &mut Sketch,
    params: &ToolParams,
) -> ToolEffect {
    let ToolState::BSplineDraw { points } = std::mem::take(state) else {
        return ToolEffect::none();
    };
    if points.len() < 3 {
        return ToolEffect::none();
    }
    let periodic = params.bspline_periodic;
    let kind = if periodic { "Periodic" } else { "Open" };
    let n = points.len();
    if params.bspline_interpolate {
        let Some(at) = points
            .iter()
            .map(|t| t.position(sketch))
            .collect::<Option<Vec<Vec2D>>>()
        else {
            return ToolEffect::none();
        };
        let at: Vec<[f64; 2]> = at.iter().map(|p| [p.x, p.y]).collect();
        let Some(fit) = sketch_solver::spline::interpolate(&at, params.bspline_degree, periodic)
        else {
            return ToolEffect::log("No spline goes through the same point twice in a row");
        };
        let fit_points: Vec<Uuid> = points.iter().map(|t| materialize(sketch, *t)).collect();
        // An open spline starts and ends on its first and last points: they
        // are its end control points too, one point each, so the curves
        // drawn to them close profiles through them.
        let last = fit.control.len() - 1;
        let control: Vec<Uuid> = fit
            .control
            .iter()
            .enumerate()
            .map(|(i, p)| match i {
                0 if !periodic => fit_points[0],
                i if i == last && !periodic => fit_points[n - 1],
                _ => {
                    let at = Vec2D::new(p[0], p[1]);
                    sketch.add_geometry(GeometryElement::Point(Point::new(at)))
                }
            })
            .collect();
        let degree = fit.degree;
        sketch.add_geometry(GeometryElement::BSpline(BSpline {
            degree,
            knots: fit.knots,
            fit_points,
            fit_params: fit.params,
            ..BSpline::new(control, periodic)
        }));
        return ToolEffect::changed(format!(
            "{kind} B-spline of degree {degree} through {n} points"
        ));
    }
    let ids: Vec<Uuid> = points.iter().map(|t| materialize(sketch, *t)).collect();
    // The degree asked for; with too few control points for it the curve
    // takes the highest they allow.
    let degree = params.bspline_degree.clamp(1, spline::MAX_DEGREE);
    sketch.add_geometry(GeometryElement::BSpline(BSpline {
        degree,
        ..BSpline::new(ids, periodic)
    }));
    ToolEffect::changed(format!(
        "{kind} B-spline of degree {degree} with {n} control points"
    ))
}

/// Vertex positions of a regular N-gon inscribed in the circle through
/// `vertex` around `center` (first vertex exactly at `vertex`). Also used
/// by the overlay preview so the preview matches the committed shape.
pub fn polygon_vertices(center: Vec2D, vertex: Vec2D, sides: u32) -> Vec<Vec2D> {
    let c = center.to_glam();
    let v = vertex.to_glam() - c;
    let n = sides.max(3);
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * (i as f64) / (n as f64);
            let (sin, cos) = a.sin_cos();
            Vec2D::new(c.x + v.x * cos - v.y * sin, c.y + v.x * sin + v.y * cos)
        })
        .collect()
}

pub(super) fn polygon(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
    sides: u32,
) -> ToolEffect {
    match *state {
        ToolState::PolygonCenter { center } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            // Every vertex is the point exactly where it falls, else a new
            // one; the clicked vertex, where a snap put it, is held there.
            let eps = super::curve_attach_eps(snap_tol);
            let first = snap_point_or_curve(sketch, cursor, eps);
            let Some(v) = first.position(sketch) else {
                return ToolEffect::none();
            };
            let radius = (v - c).to_glam().length();
            if radius < 1e-6 {
                return ToolEffect::none(); // vertex on center: degenerate
            }
            let corners: Vec<SnapTarget> = polygon_vertices(c, v, sides)
                .into_iter()
                .enumerate()
                .map(|(i, p)| match i {
                    0 => first,
                    _ => snap::snap_to_point(sketch, p, eps, &[]),
                })
                .collect();
            if joins_itself(corners.iter().chain([&center])) {
                return ToolEffect::log("The polygon would join a point to itself");
            }
            // Held regular: a construction circle through every vertex,
            // and every side as long as the first. Its centre, size and turn
            // are what is left free.
            // The vertices before the circle, so none is held on it twice.
            let center_id = materialize_on_curve(sketch, center, snap_tol);
            let vertex_ids: Vec<Uuid> = corners
                .into_iter()
                .enumerate()
                .map(|(i, t)| match i {
                    0 => materialize_on_curve(sketch, t, snap_tol),
                    _ => materialize(sketch, t),
                })
                .collect();
            let circle =
                sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, radius)));
            sketch.set_construction(circle, true);
            for vertex in &vertex_ids {
                sketch.add_constraint(ConstraintKind::PointOnCircle {
                    point: *vertex,
                    circle,
                });
            }
            let n = vertex_ids.len();
            let sides: Vec<Uuid> = (0..n)
                .map(|i| {
                    sketch.add_geometry(GeometryElement::Line(Line::new(
                        vertex_ids[i],
                        vertex_ids[(i + 1) % n],
                    )))
                })
                .collect();
            for side in &sides[1..] {
                sketch.add_constraint(ConstraintKind::EqualLength {
                    line1: sides[0],
                    line2: *side,
                });
            }
            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Regular {n}-gon r={radius:.2} at ({:.2}, {:.2})",
                c.x, c.y
            ))
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::PolygonCenter { center };
            ToolEffect::none()
        }
    }
}

/// Junction and center positions of a slot along centerline `a → b` with
/// full width `width`. Returns `(p1, p2, p3, p4)`: `p1/p2` on the +normal
/// side (a/b ends), `p3/p4` on the −normal side (b/a ends). `None` when the
/// centerline or width is degenerate.
pub fn slot_corners(a: Vec2D, b: Vec2D, width: f64) -> Option<(Vec2D, Vec2D, Vec2D, Vec2D)> {
    let half = width * 0.5;
    let d = (b - a).to_glam();
    if d.length() < 1e-6 || half < 1e-6 {
        return None;
    }
    let dir = d.normalize();
    let n = glam::DVec2::new(-dir.y, dir.x);
    let (av, bv) = (a.to_glam(), b.to_glam());
    Some((
        Vec2D::from_glam(av + n * half),
        Vec2D::from_glam(bv + n * half),
        Vec2D::from_glam(bv - n * half),
        Vec2D::from_glam(av - n * half),
    ))
}

pub(super) fn slot(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
    width: f64,
) -> ToolEffect {
    match *state {
        ToolState::SlotFrom { from } => {
            let Some(a) = from.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let b = cursor;
            let Some((p1, p2, p3, p4)) = slot_corners(a, b, width) else {
                return ToolEffect::none(); // degenerate centerline/width
            };
            let half = width * 0.5;
            // Both ends are held where a snap put them; each rail end is the
            // point exactly where it falls, else a new one.
            let eps = super::curve_attach_eps(snap_tol);
            let to = snap_point_or_curve(sketch, b, eps);
            let ends = [p1, p2, p3, p4].map(|p| snap::snap_to_point(sketch, p, eps, &[]));
            if joins_itself([from, to].iter().chain(&ends)) {
                return ToolEffect::log("The slot would join a point to itself");
            }

            let center_a = materialize_on_curve(sketch, from, snap_tol);
            let center_b = materialize_on_curve(sketch, to, snap_tol);
            let [i1, i2, i3, i4] = ends.map(|t| materialize(sketch, t));

            let rail_1 = sketch.add_geometry(GeometryElement::Line(Line::new(i1, i2)));
            let rail_2 = sketch.add_geometry(GeometryElement::Line(Line::new(i3, i4)));
            // CCW semicircle caps bulging away from the slot body:
            // at `b` the CCW sweep p3 → p2 passes through b + dir·half;
            // at `a` the CCW sweep p1 → p4 passes through a − dir·half.
            let cap_b = sketch.add_geometry(GeometryElement::Arc(Arc::new(center_b, i3, i2, half)));
            let cap_a = sketch.add_geometry(GeometryElement::Arc(Arc::new(center_a, i1, i4, half)));
            // Held a slot: each side tangent to both caps, the caps alike.
            for rail in [rail_1, rail_2] {
                for cap in [cap_a, cap_b] {
                    sketch.add_constraint(ConstraintKind::Tangent {
                        line_or_circle1: rail,
                        item2: cap,
                    });
                }
            }
            sketch.add_constraint(ConstraintKind::EqualRadius {
                circle1: cap_a,
                circle2: cap_b,
            });

            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Slot {:.2} long × {width:.2} wide",
                (b - a).to_glam().length()
            ))
        }
        _ => {
            let from = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::SlotFrom { from };
            ToolEffect::none()
        }
    }
}

/// Point positions of an arc-ended arc slot: centerline arc around `center`
/// through `start`, ending toward `toward_end`, with full width `width`.
/// Shared by the tool and the overlay preview.
pub struct ArcSlotShape {
    /// Cap centers (on the centerline arc, at the start/end clicks).
    pub cap_a: Vec2D,
    pub cap_b: Vec2D,
    /// Rail endpoints at the start (a) and end (b) angles.
    pub outer_a: Vec2D,
    pub outer_b: Vec2D,
    pub inner_a: Vec2D,
    pub inner_b: Vec2D,
    pub outer_r: f64,
    pub inner_r: f64,
    pub cap_r: f64,
}

pub fn arc_slot_shape(
    center: Vec2D,
    start: Vec2D,
    toward_end: Vec2D,
    width: f64,
) -> Option<ArcSlotShape> {
    let c = center.to_glam();
    let s = start.to_glam();
    let h = width * 0.5;
    let rv = s - c;
    let r = rv.length();
    if h < 1e-6 || r - h < 1e-6 {
        return None; // inner rail would vanish
    }
    let dir_end = toward_end.to_glam() - c;
    if dir_end.length() < 1e-6 {
        return None;
    }
    let (start_angle, sweep) = arc_angles(rv, dir_end);
    if !(0.01..std::f64::consts::TAU - 0.01).contains(&sweep) {
        return None; // degenerate or self-overlapping slot
    }
    let e = |ang: f64| glam::DVec2::new(ang.cos(), ang.sin());
    let end_angle = start_angle + sweep;
    Some(ArcSlotShape {
        cap_a: start,
        cap_b: Vec2D::from_glam(c + e(end_angle) * r),
        outer_a: Vec2D::from_glam(c + e(start_angle) * (r + h)),
        outer_b: Vec2D::from_glam(c + e(end_angle) * (r + h)),
        inner_a: Vec2D::from_glam(c + e(start_angle) * (r - h)),
        inner_b: Vec2D::from_glam(c + e(end_angle) * (r - h)),
        outer_r: r + h,
        inner_r: r - h,
        cap_r: h,
    })
}

pub(super) fn arc_slot(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
    width: f64,
) -> ToolEffect {
    match *state {
        ToolState::ArcSlotCenter { center } => {
            let start = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            let (Some(c), Some(s)) = (center.position(sketch), start.position(sketch)) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            if (s - c).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::ArcSlotStart { center, start };
            ToolEffect::none()
        }
        ToolState::ArcSlotStart { center, start } => {
            let (Some(c), Some(s)) = (center.position(sketch), start.position(sketch)) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let Some(shape) = arc_slot_shape(c, s, cursor, width) else {
                return ToolEffect::none();
            };
            // Each point worked out from the clicks is the point exactly
            // where it falls, else a new one.
            let eps = super::curve_attach_eps(snap_tol);
            let derived = [
                shape.cap_b,
                shape.outer_a,
                shape.outer_b,
                shape.inner_a,
                shape.inner_b,
            ]
            .map(|p| snap::snap_to_point(sketch, p, eps, &[]));
            if joins_itself([center, start].iter().chain(&derived)) {
                return ToolEffect::log("The slot would join a point to itself");
            }
            // Both clicks stay where a snap put them.
            let center_id = materialize_on_curve(sketch, center, snap_tol);
            let cap_a_id = materialize_on_curve(sketch, start, snap_tol);
            let [cap_b_id, outer_a, outer_b, inner_a, inner_b] =
                derived.map(|t| materialize(sketch, t));

            // Rails run CCW start → end; the caps are CCW semicircles that
            // bulge past the ends (see the straight slot for the same idea).
            let outer = sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id,
                outer_a,
                outer_b,
                shape.outer_r,
            )));
            let inner = sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id,
                inner_a,
                inner_b,
                shape.inner_r,
            )));
            let cap_b = sketch.add_geometry(GeometryElement::Arc(Arc::new(
                cap_b_id,
                outer_b,
                inner_b,
                shape.cap_r,
            )));
            let cap_a = sketch.add_geometry(GeometryElement::Arc(Arc::new(
                cap_a_id,
                inner_a,
                outer_a,
                shape.cap_r,
            )));
            // Each cap runs on smoothly into both rails: it stays a slot
            // when dragged. Tangent to both concentric rails at its ends, a
            // cap spans the gap between them, so both caps are half the
            // rails' difference across; an equal-radius constraint on top
            // would say that again and be redundant.
            for cap in [cap_a, cap_b] {
                for rail in [outer, inner] {
                    sketch.add_constraint(ConstraintKind::Tangent {
                        line_or_circle1: rail,
                        item2: cap,
                    });
                }
            }

            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Arc slot r={:.2} × {width:.2} wide",
                (s - c).to_glam().length()
            ))
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::ArcSlotCenter { center };
            ToolEffect::none()
        }
    }
}
