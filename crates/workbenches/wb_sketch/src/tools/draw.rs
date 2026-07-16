//! Drawing tools: each handler advances the state machine one click and
//! materializes geometry only when the shape completes.

use uuid::Uuid;

use super::{materialize, materialize_on_curve, short, ToolEffect, ToolState};
use crate::geom2d;
use crate::sketch::{
    Arc, BSpline, Circle, ConstraintKind, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D,
};
use crate::snap::{self, arc_angles, AxisSnap, SnapTarget};

/// Point snap first (id reuse — never a coincident duplicate); otherwise a
/// curve within tolerance captures the click, projecting the position onto
/// it so `materialize_on_curve` records the on-curve constraint.
fn snap_point_or_curve(sketch: &Sketch, cursor: Vec2D, tol: f32) -> SnapTarget {
    match snap::snap_to_point(sketch, cursor, tol, &[]) {
        SnapTarget::Existing(id) => SnapTarget::Existing(id),
        SnapTarget::New(pos) => match snap::snap_to_curve(sketch, cursor, tol, &[]) {
            Some((_, proj)) => SnapTarget::New(proj),
            None => SnapTarget::New(pos),
        },
    }
}

pub(super) fn point(sketch: &mut Sketch, cursor: Vec2D) -> ToolEffect {
    // Deliberately no point-snap: clicking near an existing point
    // would otherwise be a silent no-op.
    let id = sketch.add_geometry(GeometryElement::Point(Point::new(cursor)));
    ToolEffect::changed(format!(
        "Point at ({:.2}, {:.2}) [{}]",
        cursor.x,
        cursor.y,
        short(id)
    ))
}

pub(super) fn line(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
) -> ToolEffect {
    match *state {
        ToolState::LineFrom { from, .. } => {
            let Some(from_pos) = from.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            // Snap priority: existing point (id reuse) > curve (projected,
            // auto on-curve constraint) > axis alignment.
            let (axis_pos, mut axis) = snap::snap_axis(from_pos, cursor, snap_tol);
            let exclude = match from {
                SnapTarget::Existing(id) => vec![id],
                SnapTarget::New(_) => vec![],
            };
            let end = match snap::snap_to_point(sketch, cursor, snap_tol, &exclude) {
                SnapTarget::Existing(id) => SnapTarget::Existing(id),
                SnapTarget::New(_) => match snap::snap_to_curve(sketch, cursor, snap_tol, &[]) {
                    Some((_, proj)) => {
                        axis = None; // the projected point isn't axis-aligned
                        SnapTarget::New(proj)
                    }
                    None => SnapTarget::New(axis_pos),
                },
            };
            let end_pos = end.position(sketch).unwrap_or(axis_pos);
            if (end_pos - from_pos).to_glam().length() < 1e-6 {
                return ToolEffect::none(); // zero-length click, ignore
            }

            let start_id = materialize_on_curve(sketch, from, snap_tol);
            let end_id = materialize_on_curve(sketch, end, snap_tol);
            let line_id = sketch.add_geometry(GeometryElement::Line(Line::new(start_id, end_id)));

            // Auto-constraint on axis-snapped segments,
            // only when the end point wasn't itself snapped to existing
            // geometry (which takes priority over the axis).
            let mut log = format!(
                "Line ({:.2}, {:.2}) → ({:.2}, {:.2})",
                from_pos.x, from_pos.y, end_pos.x, end_pos.y
            );
            if matches!(end, SnapTarget::New(_)) {
                match axis {
                    Some(AxisSnap::Horizontal) => {
                        sketch.add_constraint(ConstraintKind::Horizontal { element: line_id });
                        log.push_str(" [auto: horizontal]");
                    }
                    Some(AxisSnap::Vertical) => {
                        sketch.add_constraint(ConstraintKind::Vertical { element: line_id });
                        log.push_str(" [auto: vertical]");
                    }
                    None => {}
                }
            }

            // Chain: continue from the end point.
            *state = ToolState::LineFrom {
                from: SnapTarget::Existing(end_id),
                chain: true,
            };
            ToolEffect::changed(log)
        }
        _ => {
            let from = snap_point_or_curve(sketch, cursor, snap_tol);
            *state = ToolState::LineFrom { from, chain: false };
            ToolEffect::none()
        }
    }
}

/// Four counter-clockwise corner positions → 4 shared-vertex lines plus the
/// H/V constraints that make the shape stay a rectangle under later edits.
fn close_rectangle(sketch: &mut Sketch, pa: Uuid, pb: Uuid, pc: Uuid, pd: Uuid) {
    let bottom = sketch.add_geometry(GeometryElement::Line(Line::new(pa, pb)));
    let right = sketch.add_geometry(GeometryElement::Line(Line::new(pb, pc)));
    let top = sketch.add_geometry(GeometryElement::Line(Line::new(pc, pd)));
    let left = sketch.add_geometry(GeometryElement::Line(Line::new(pd, pa)));
    sketch.add_constraint(ConstraintKind::Horizontal { element: bottom });
    sketch.add_constraint(ConstraintKind::Horizontal { element: top });
    sketch.add_constraint(ConstraintKind::Vertical { element: right });
    sketch.add_constraint(ConstraintKind::Vertical { element: left });
}

pub(super) fn rect(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
) -> ToolEffect {
    match *state {
        ToolState::RectFrom { corner } => {
            let Some(a) = corner.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let c = cursor;
            if (c.x - a.x).abs() < 1e-6 || (c.y - a.y).abs() < 1e-6 {
                return ToolEffect::none(); // degenerate rectangle
            }
            let b = Vec2D::new(c.x, a.y);
            let d = Vec2D::new(a.x, c.y);

            let pa = materialize_on_curve(sketch, corner, snap_tol);
            let pb = sketch.add_geometry(GeometryElement::Point(Point::new(b)));
            let pc = sketch.add_geometry(GeometryElement::Point(Point::new(c)));
            let pd = sketch.add_geometry(GeometryElement::Point(Point::new(d)));
            close_rectangle(sketch, pa, pb, pc, pd);

            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Rectangle {:.2} × {:.2}",
                (c.x - a.x).abs(),
                (c.y - a.y).abs()
            ))
        }
        _ => {
            let corner = snap_point_or_curve(sketch, cursor, snap_tol);
            *state = ToolState::RectFrom { corner };
            ToolEffect::none()
        }
    }
}

pub(super) fn rect_center(state: &mut ToolState, sketch: &mut Sketch, cursor: Vec2D) -> ToolEffect {
    match *state {
        ToolState::RectCenterAt { center } => {
            let k = cursor;
            if (k.x - center.x).abs() < 1e-6 || (k.y - center.y).abs() < 1e-6 {
                return ToolEffect::none(); // degenerate rectangle
            }
            // Opposite corner mirrored through the center.
            let o = Vec2D::new(2.0 * center.x - k.x, 2.0 * center.y - k.y);
            let (x0, x1) = (o.x.min(k.x), o.x.max(k.x));
            let (y0, y1) = (o.y.min(k.y), o.y.max(k.y));
            let pa = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x0, y0))));
            let pb = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x1, y0))));
            let pc = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x1, y1))));
            let pd = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x0, y1))));
            close_rectangle(sketch, pa, pb, pc, pd);

            *state = ToolState::Idle;
            ToolEffect::changed(format!("Rectangle {:.2} × {:.2}", x1 - x0, y1 - y0))
        }
        _ => {
            *state = ToolState::RectCenterAt { center: cursor };
            ToolEffect::none()
        }
    }
}

pub(super) fn circle(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
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
            sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, radius)));
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

/// Cursor snapped to an existing point's *position* (nothing materialized).
fn snapped_pos(sketch: &Sketch, cursor: Vec2D, snap_tol: f32) -> Vec2D {
    snap::snap_to_point(sketch, cursor, snap_tol, &[])
        .position(sketch)
        .unwrap_or(cursor)
}

pub(super) fn circle3(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
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
            let center_id =
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(c))));
            sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, radius)));
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
    snap_tol: f32,
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
            // Project the clicked end point onto the arc's radius so the
            // stored end point actually lies on the arc.
            let dir = (cursor - center_pos).to_glam();
            if dir.length() < 1e-6 {
                return ToolEffect::none();
            }
            let end_pos = Vec2D::from_glam(center_pos.to_glam() + dir.normalize() * radius);
            let end_snap = snap::snap_to_point(sketch, end_pos, snap_tol, &[]);

            let center_id = materialize_on_curve(sketch, center, snap_tol);
            let start_id = materialize_on_curve(sketch, start, snap_tol);
            let end_id = materialize(sketch, end_snap);
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
    snap_tol: f32,
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
            let start_id = materialize(sketch, a);
            let end_id = materialize(sketch, b);
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id, start_id, end_id, radius,
            )));
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
    snap_tol: f32,
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
            let ratio = (minor / major_len).min(1.0);
            let center_id = materialize(sketch, center);
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

pub(super) fn bspline(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
) -> ToolEffect {
    let target = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
    match state {
        ToolState::BSplineDraw { points } => {
            let prev = points.last().and_then(|t| t.position(sketch));
            let new = target.position(sketch);
            if let (Some(prev), Some(new)) = (prev, new) {
                if (new - prev).to_glam().length() < 1e-6 {
                    return ToolEffect::none(); // double-click on the same spot
                }
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

/// Complete the in-progress B-spline (right-click/Enter). Fewer than 3
/// control points cancels without creating geometry.
pub(super) fn bspline_finish(
    state: &mut ToolState,
    sketch: &mut Sketch,
    periodic: bool,
) -> ToolEffect {
    let ToolState::BSplineDraw { points } = std::mem::take(state) else {
        return ToolEffect::none();
    };
    if points.len() < 3 {
        return ToolEffect::none();
    }
    let ids: Vec<Uuid> = points.iter().map(|t| materialize(sketch, *t)).collect();
    let n = ids.len();
    sketch.add_geometry(GeometryElement::BSpline(BSpline::new(ids, periodic)));
    ToolEffect::changed(format!(
        "{} B-spline with {n} control points",
        if periodic { "Periodic" } else { "Open" }
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
            let a = std::f32::consts::TAU * (i as f32) / (n as f32);
            let (sin, cos) = a.sin_cos();
            Vec2D::new(c.x + v.x * cos - v.y * sin, c.y + v.x * sin + v.y * cos)
        })
        .collect()
}

pub(super) fn polygon(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    sides: u32,
) -> ToolEffect {
    match *state {
        ToolState::PolygonCenter { center } => {
            let Some(c) = center.position(sketch) else {
                *state = ToolState::Idle;
                return ToolEffect::none();
            };
            let radius = (cursor - c).to_glam().length();
            if radius < 1e-6 {
                return ToolEffect::none(); // vertex on center: degenerate
            }
            let vertex_ids: Vec<Uuid> = polygon_vertices(c, cursor, sides)
                .into_iter()
                .map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(p))))
                .collect();
            let n = vertex_ids.len();
            for i in 0..n {
                sketch.add_geometry(GeometryElement::Line(Line::new(
                    vertex_ids[i],
                    vertex_ids[(i + 1) % n],
                )));
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
pub fn slot_corners(a: Vec2D, b: Vec2D, width: f32) -> Option<(Vec2D, Vec2D, Vec2D, Vec2D)> {
    let half = width * 0.5;
    let d = (b - a).to_glam();
    if d.length() < 1e-6 || half < 1e-6 {
        return None;
    }
    let dir = d.normalize();
    let n = glam::Vec2::new(-dir.y, dir.x);
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
    snap_tol: f32,
    width: f32,
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

            let center_a = materialize(sketch, from);
            let center_b = sketch.add_geometry(GeometryElement::Point(Point::new(b)));
            let i1 = sketch.add_geometry(GeometryElement::Point(Point::new(p1)));
            let i2 = sketch.add_geometry(GeometryElement::Point(Point::new(p2)));
            let i3 = sketch.add_geometry(GeometryElement::Point(Point::new(p3)));
            let i4 = sketch.add_geometry(GeometryElement::Point(Point::new(p4)));

            sketch.add_geometry(GeometryElement::Line(Line::new(i1, i2)));
            sketch.add_geometry(GeometryElement::Line(Line::new(i3, i4)));
            // CCW semicircle caps bulging away from the slot body:
            // at `b` the CCW sweep p3 → p2 passes through b + dir·half;
            // at `a` the CCW sweep p1 → p4 passes through a − dir·half.
            sketch.add_geometry(GeometryElement::Arc(Arc::new(center_b, i3, i2, half)));
            sketch.add_geometry(GeometryElement::Arc(Arc::new(center_a, i1, i4, half)));

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
    pub outer_r: f32,
    pub inner_r: f32,
    pub cap_r: f32,
}

pub fn arc_slot_shape(
    center: Vec2D,
    start: Vec2D,
    toward_end: Vec2D,
    width: f32,
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
    if !(0.01..std::f32::consts::TAU - 0.01).contains(&sweep) {
        return None; // degenerate or self-overlapping slot
    }
    let e = |ang: f32| glam::Vec2::new(ang.cos(), ang.sin());
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
    snap_tol: f32,
    width: f32,
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
            let center_id = materialize(sketch, center);
            let cap_a_id = materialize(sketch, start);
            let cap_b_id = sketch.add_geometry(GeometryElement::Point(Point::new(shape.cap_b)));
            let outer_a = sketch.add_geometry(GeometryElement::Point(Point::new(shape.outer_a)));
            let outer_b = sketch.add_geometry(GeometryElement::Point(Point::new(shape.outer_b)));
            let inner_a = sketch.add_geometry(GeometryElement::Point(Point::new(shape.inner_a)));
            let inner_b = sketch.add_geometry(GeometryElement::Point(Point::new(shape.inner_b)));

            // Rails run CCW start → end; the caps are CCW semicircles that
            // bulge past the ends (see the straight slot for the same idea).
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id,
                outer_a,
                outer_b,
                shape.outer_r,
            )));
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id,
                inner_a,
                inner_b,
                shape.inner_r,
            )));
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                cap_b_id,
                outer_b,
                inner_b,
                shape.cap_r,
            )));
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                cap_a_id,
                inner_a,
                outer_a,
                shape.cap_r,
            )));

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
