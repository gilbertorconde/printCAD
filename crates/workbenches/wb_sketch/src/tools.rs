//! Sketch tool state machine.
//!
//! Tools accumulate *intent* (snapped targets) and only materialize
//! geometry when a shape completes, so cancelling mid-shape never leaves
//! orphan points behind. Endpoint snapping reuses existing point ids, which
//! is what makes consecutive segments and closed profiles share vertices.

use uuid::Uuid;

use crate::sketch::{Arc, Circle, Constraint, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::snap::{self, AxisSnap, SnapTarget};

/// In-progress state of the active drawing tool.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ToolState {
    #[default]
    Idle,
    /// Line tool: waiting for the segment's end point. `chain` is true once
    /// at least one segment was committed (right-click/Escape ends chains).
    LineFrom { from: SnapTarget, chain: bool },
    /// Rectangle tool: first corner picked.
    RectFrom { corner: SnapTarget },
    /// Circle tool: center picked, waiting for a rim point.
    CircleFrom { center: SnapTarget },
    /// Arc tool: center picked, waiting for the start point.
    ArcCenter { center: SnapTarget },
    /// Arc tool: center + start picked, waiting for the end point.
    ArcStart {
        center: SnapTarget,
        start: SnapTarget,
    },
    /// Polygon tool: center picked, waiting for a vertex.
    PolygonCenter { center: SnapTarget },
    /// Slot tool: first centerline endpoint picked.
    SlotFrom { from: SnapTarget },
}

/// Panel-editable parameters consumed by the drawing tools.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolParams {
    /// Number of sides for the regular polygon tool (3..=12).
    pub polygon_sides: u32,
    /// Full slot width (distance between the two parallel edges), mm.
    pub slot_width: f32,
    /// Corner fillet radius for the sketch fillet tool, mm.
    pub fillet_radius: f32,
}

impl Default for ToolParams {
    fn default() -> Self {
        Self {
            polygon_sides: 6,
            slot_width: 4.0,
            fillet_radius: 2.0,
        }
    }
}

impl ToolState {
    pub fn is_idle(&self) -> bool {
        matches!(self, ToolState::Idle)
    }

    /// One-line status for the UI ("click end point…").
    pub fn status(&self) -> Option<&'static str> {
        match self {
            ToolState::Idle => None,
            ToolState::LineFrom { chain: false, .. } => Some("Line: click the end point"),
            ToolState::LineFrom { chain: true, .. } => {
                Some("Line: click to chain, right-click or Esc to finish")
            }
            ToolState::RectFrom { .. } => Some("Rectangle: click the opposite corner"),
            ToolState::CircleFrom { .. } => Some("Circle: click a point on the rim"),
            ToolState::ArcCenter { .. } => Some("Arc: click the start point"),
            ToolState::ArcStart { .. } => Some("Arc: click the end point (counter-clockwise)"),
            ToolState::PolygonCenter { .. } => Some("Polygon: click a vertex"),
            ToolState::SlotFrom { .. } => Some("Slot: click the other end of the centerline"),
        }
    }
}

/// What a completed tool click produced (used for logging + dirty marking).
pub struct ToolEffect {
    pub changed: bool,
    pub log: Option<String>,
}

impl ToolEffect {
    fn none() -> Self {
        Self {
            changed: false,
            log: None,
        }
    }
    fn changed(log: impl Into<String>) -> Self {
        Self {
            changed: true,
            log: Some(log.into()),
        }
    }
}

/// Resolve a snap target into a concrete point id, creating the point when
/// needed.
fn materialize(sketch: &mut Sketch, target: SnapTarget) -> Uuid {
    match target {
        SnapTarget::Existing(id) => id,
        SnapTarget::New(pos) => sketch.add_geometry(GeometryElement::Point(Point::new(pos))),
    }
}

/// Advance the tool state machine with a left click at `cursor` (sketch
/// coords). `snap_tol` is in sketch units; `params` carries the
/// panel-editable tool settings.
pub fn handle_click(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    params: &ToolParams,
) -> ToolEffect {
    match tool {
        "sketch.point" => {
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
        "sketch.line" => handle_line_click(state, sketch, cursor, snap_tol),
        "sketch.rect" => handle_rect_click(state, sketch, cursor, snap_tol),
        "sketch.circle" => handle_circle_click(state, sketch, cursor, snap_tol),
        "sketch.arc" => handle_arc_click(state, sketch, cursor, snap_tol),
        "sketch.polygon" => {
            handle_polygon_click(state, sketch, cursor, snap_tol, params.polygon_sides)
        }
        "sketch.slot" => handle_slot_click(state, sketch, cursor, snap_tol, params.slot_width),
        "sketch.fillet" => handle_fillet_click(sketch, cursor, snap_tol, params.fillet_radius),
        _ => ToolEffect::none(),
    }
}

fn handle_line_click(
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
            // Axis snap first (it adjusts the position), then point snap
            // (an exact existing point beats axis alignment).
            let (axis_pos, axis) = snap::snap_axis(from_pos, cursor, snap_tol);
            let exclude = match from {
                SnapTarget::Existing(id) => vec![id],
                SnapTarget::New(_) => vec![],
            };
            let end = match snap::snap_to_point(sketch, cursor, snap_tol, &exclude) {
                SnapTarget::Existing(id) => SnapTarget::Existing(id),
                SnapTarget::New(_) => SnapTarget::New(axis_pos),
            };
            let end_pos = end.position(sketch).unwrap_or(axis_pos);
            if (end_pos - from_pos).to_glam().length() < 1e-6 {
                return ToolEffect::none(); // zero-length click, ignore
            }

            let start_id = materialize(sketch, from);
            let end_id = materialize(sketch, end);
            let line_id = sketch.add_geometry(GeometryElement::Line(Line::new(start_id, end_id)));

            // Auto-constraint on axis-snapped segments (FreeCAD behaviour),
            // only when the end point wasn't itself snapped to existing
            // geometry (which takes priority over the axis).
            let mut log = format!(
                "Line ({:.2}, {:.2}) → ({:.2}, {:.2})",
                from_pos.x, from_pos.y, end_pos.x, end_pos.y
            );
            if matches!(end, SnapTarget::New(_)) {
                match axis {
                    Some(AxisSnap::Horizontal) => {
                        sketch
                            .constraints
                            .push(Constraint::Horizontal { element: line_id });
                        log.push_str(" [auto: horizontal]");
                    }
                    Some(AxisSnap::Vertical) => {
                        sketch
                            .constraints
                            .push(Constraint::Vertical { element: line_id });
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
            let from = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::LineFrom { from, chain: false };
            ToolEffect::none()
        }
    }
}

fn handle_rect_click(
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

            let pa = materialize(sketch, corner);
            let pb = sketch.add_geometry(GeometryElement::Point(Point::new(b)));
            let pc = sketch.add_geometry(GeometryElement::Point(Point::new(c)));
            let pd = sketch.add_geometry(GeometryElement::Point(Point::new(d)));

            let bottom = sketch.add_geometry(GeometryElement::Line(Line::new(pa, pb)));
            let right = sketch.add_geometry(GeometryElement::Line(Line::new(pb, pc)));
            let top = sketch.add_geometry(GeometryElement::Line(Line::new(pc, pd)));
            let left = sketch.add_geometry(GeometryElement::Line(Line::new(pd, pa)));

            // The H/V constraints are what make it stay a rectangle under
            // later edits — the shape without them is just four lines.
            sketch
                .constraints
                .push(Constraint::Horizontal { element: bottom });
            sketch
                .constraints
                .push(Constraint::Horizontal { element: top });
            sketch
                .constraints
                .push(Constraint::Vertical { element: right });
            sketch
                .constraints
                .push(Constraint::Vertical { element: left });

            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Rectangle {:.2} × {:.2}",
                (c.x - a.x).abs(),
                (c.y - a.y).abs()
            ))
        }
        _ => {
            let corner = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::RectFrom { corner };
            ToolEffect::none()
        }
    }
}

fn handle_circle_click(
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
            let center_id = materialize(sketch, center);
            sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, radius)));
            *state = ToolState::Idle;
            ToolEffect::changed(format!(
                "Circle r={radius:.2} at ({:.2}, {:.2})",
                center_pos.x, center_pos.y
            ))
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::CircleFrom { center };
            ToolEffect::none()
        }
    }
}

fn handle_arc_click(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
) -> ToolEffect {
    match *state {
        ToolState::ArcCenter { center } => {
            let start = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
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

            let center_id = materialize(sketch, center);
            let start_id = materialize(sketch, start);
            let end_id = materialize(sketch, end_snap);
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_id, start_id, end_id, radius,
            )));
            *state = ToolState::Idle;
            ToolEffect::changed(format!("Arc r={radius:.2}"))
        }
        _ => {
            let center = snap::snap_to_point(sketch, cursor, snap_tol, &[]);
            *state = ToolState::ArcCenter { center };
            ToolEffect::none()
        }
    }
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

fn handle_polygon_click(
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

fn handle_slot_click(
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

/// Fillet the corner point under the cursor: the point must be shared by
/// exactly two non-collinear lines, both long enough for the tangent
/// offset. The corner point is replaced by two tangent points joined by a
/// CCW arc; constraints referencing the removed corner are dropped.
fn handle_fillet_click(
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    radius: f32,
) -> ToolEffect {
    if radius < 1e-6 {
        return ToolEffect::none();
    }
    let snap::SnapTarget::Existing(corner_id) = snap::snap_to_point(sketch, cursor, snap_tol, &[])
    else {
        return ToolEffect::none(); // no point under the cursor
    };
    let Some(corner) = sketch.point_position(corner_id) else {
        return ToolEffect::none();
    };

    // Exactly two lines must meet at the corner.
    let touching: Vec<Line> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) if l.start == corner_id || l.end == corner_id => {
                Some(l.clone())
            }
            _ => None,
        })
        .collect();
    let [l1, l2] = touching.as_slice() else {
        return ToolEffect::none();
    };
    let far_of = |l: &Line| {
        let far = if l.start == corner_id { l.end } else { l.start };
        sketch.point_position(far)
    };
    let (Some(far1), Some(far2)) = (far_of(l1), far_of(l2)) else {
        return ToolEffect::none();
    };

    let v1 = (far1 - corner).to_glam();
    let v2 = (far2 - corner).to_glam();
    let (len1, len2) = (v1.length(), v2.length());
    if len1 < 1e-6 || len2 < 1e-6 {
        return ToolEffect::none();
    }
    let (u1, u2) = (v1 / len1, v2 / len2);
    if u1.perp_dot(u2).abs() < 1e-6 {
        return ToolEffect::none(); // collinear segments: no corner to round
    }
    let theta = u1.dot(u2).clamp(-1.0, 1.0).acos(); // corner opening angle
    let d = radius / (theta * 0.5).tan(); // corner → tangent point distance
    if d >= len1 - 1e-6 || d >= len2 - 1e-6 {
        return ToolEffect::none(); // radius too large for these segments
    }

    // Tangent points along each line; arc center on the angle bisector.
    let t1 = Vec2D::from_glam(corner.to_glam() + u1 * d);
    let t2 = Vec2D::from_glam(corner.to_glam() + u2 * d);
    let bisector = (u1 + u2).normalize();
    let center = Vec2D::from_glam(corner.to_glam() + bisector * (radius / (theta * 0.5).sin()));

    let (l1_id, l2_id) = (l1.id, l2.id);
    let t1_id = sketch.add_geometry(GeometryElement::Point(Point::new(t1)));
    let t2_id = sketch.add_geometry(GeometryElement::Point(Point::new(t2)));
    let center_id = sketch.add_geometry(GeometryElement::Point(Point::new(center)));

    // Shorten both lines to their tangent points.
    for (line_id, tangent_id) in [(l1_id, t1_id), (l2_id, t2_id)] {
        if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(line_id) {
            if l.start == corner_id {
                l.start = tangent_id;
            } else {
                l.end = tangent_id;
            }
        }
    }

    // Bridge the tangent points with the SHORT arc (the one on the corner
    // side): CCW from the endpoint whose radius vector reaches the other by
    // a positive sweep below π.
    let w1 = (t1 - center).to_glam();
    let w2 = (t2 - center).to_glam();
    let (start_id, end_id) = if w1.perp_dot(w2) > 0.0 {
        (t1_id, t2_id)
    } else {
        (t2_id, t1_id)
    };
    sketch.add_geometry(GeometryElement::Arc(Arc::new(
        center_id, start_id, end_id, radius,
    )));

    // The corner point is gone; drop it and every constraint touching it.
    sketch.geometry.retain(|g| g.id() != corner_id);
    sketch
        .constraints
        .retain(|c| !crate::sketch::constraint_refs(c).contains(&corner_id));
    sketch.construction.remove(&corner_id);

    ToolEffect::changed(format!("Fillet r={radius:.2} at corner"))
}

fn short(id: Uuid) -> String {
    id.to_string()[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forward to [`super::handle_click`] with default params (a local item
    /// shadows the glob import, so the existing tests keep their shape).
    fn handle_click(
        state: &mut ToolState,
        tool: &str,
        sketch: &mut Sketch,
        cursor: Vec2D,
        snap_tol: f32,
    ) -> ToolEffect {
        super::handle_click(
            state,
            tool,
            sketch,
            cursor,
            snap_tol,
            &ToolParams::default(),
        )
    }

    fn count_kind(sketch: &Sketch, f: impl Fn(&GeometryElement) -> bool) -> usize {
        sketch.geometry.iter().filter(|g| f(g)).count()
    }
    fn points(s: &Sketch) -> usize {
        count_kind(s, |g| matches!(g, GeometryElement::Point(_)))
    }
    fn lines(s: &Sketch) -> usize {
        count_kind(s, |g| matches!(g, GeometryElement::Line(_)))
    }
    fn arcs(s: &Sketch) -> usize {
        count_kind(s, |g| matches!(g, GeometryElement::Arc(_)))
    }

    /// Number of curve references per point id (2 everywhere = closed loop).
    fn point_use_counts(sketch: &Sketch) -> std::collections::HashMap<Uuid, usize> {
        let mut uses = std::collections::HashMap::new();
        for g in &sketch.geometry {
            for pid in Sketch::curve_point_ids(g) {
                *uses.entry(pid).or_default() += 1;
            }
        }
        uses
    }

    #[test]
    fn line_chain_shares_points() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
        );
        assert_eq!(
            points(&sketch),
            0,
            "no geometry before the segment completes"
        );
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(10.0, 5.0),
            0.5,
        );
        assert_eq!((points(&sketch), lines(&sketch)), (2, 1));
        // Chain continues: third click adds ONE new point + one line.
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(20.0, 0.0),
            0.5,
        );
        assert_eq!((points(&sketch), lines(&sketch)), (3, 2));
        // Shared middle vertex.
        let line_elems: Vec<&Line> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Line(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(line_elems[0].end, line_elems[1].start);
    }

    #[test]
    fn cancelled_first_click_leaves_no_geometry() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
        );
        let _ = ToolState::Idle; // Escape would just drop the state
        assert!(sketch.geometry.is_empty());
    }

    #[test]
    fn line_snaps_end_to_existing_point() {
        let mut sketch = Sketch::new("t");
        let existing =
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
        );
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(10.2, 0.1),
            0.5,
        );
        let line = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l),
                _ => None,
            })
            .unwrap();
        assert_eq!(line.end, existing, "end point reused, not duplicated");
        assert_eq!(points(&sketch), 2);
    }

    #[test]
    fn nearly_horizontal_line_gets_auto_constraint() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
        );
        handle_click(
            &mut state,
            "sketch.line",
            &mut sketch,
            Vec2D::new(12.0, 0.3),
            0.5,
        );
        assert_eq!(sketch.constraints.len(), 1);
        assert!(matches!(
            sketch.constraints[0],
            Constraint::Horizontal { .. }
        ));
        // And the geometry was snapped level.
        let ys: Vec<f32> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) => Some(p.position.y),
                _ => None,
            })
            .collect();
        assert_eq!(ys, vec![0.0, 0.0]);
    }

    #[test]
    fn rectangle_builds_four_lines_with_constraints() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.rect",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
        );
        handle_click(
            &mut state,
            "sketch.rect",
            &mut sketch,
            Vec2D::new(8.0, 5.0),
            0.5,
        );
        assert_eq!((points(&sketch), lines(&sketch)), (4, 4));
        assert_eq!(sketch.constraints.len(), 4);
        assert!(state.is_idle());
        // Corners form a closed loop: every point used exactly twice.
        use std::collections::HashMap;
        let mut uses: HashMap<Uuid, usize> = HashMap::new();
        for g in &sketch.geometry {
            for pid in Sketch::curve_point_ids(g) {
                *uses.entry(pid).or_default() += 1;
            }
        }
        assert!(uses.values().all(|&n| n == 2));
    }

    #[test]
    fn degenerate_rectangle_rejected() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.rect",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
        );
        let fx = handle_click(
            &mut state,
            "sketch.rect",
            &mut sketch,
            Vec2D::new(0.0, 5.0),
            0.5,
        );
        assert!(!fx.changed);
        assert!(sketch.geometry.is_empty());
    }

    #[test]
    fn circle_two_clicks() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.circle",
            &mut sketch,
            Vec2D::new(1.0, 1.0),
            0.5,
        );
        handle_click(
            &mut state,
            "sketch.circle",
            &mut sketch,
            Vec2D::new(4.0, 5.0),
            0.5,
        );
        let circle = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Circle(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert!((circle.radius - 5.0).abs() < 1e-5);
        assert!(state.is_idle());
    }

    #[test]
    fn arc_three_clicks_end_projected_to_radius() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        handle_click(
            &mut state,
            "sketch.arc",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.1,
        );
        handle_click(
            &mut state,
            "sketch.arc",
            &mut sketch,
            Vec2D::new(5.0, 0.0),
            0.1,
        );
        handle_click(
            &mut state,
            "sketch.arc",
            &mut sketch,
            Vec2D::new(0.0, 7.0),
            0.1,
        );
        let arc = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Arc(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap();
        assert!((arc.radius - 5.0).abs() < 1e-5);
        let end = sketch.point_position(arc.end).unwrap();
        assert!(
            (end.to_glam().length() - 5.0).abs() < 1e-4,
            "end lies on the arc"
        );
        assert!(state.is_idle());
    }

    #[test]
    fn polygon_two_clicks_builds_closed_ngon() {
        for sides in [3u32, 6, 12] {
            let mut sketch = Sketch::new("t");
            let mut state = ToolState::default();
            let params = ToolParams {
                polygon_sides: sides,
                ..ToolParams::default()
            };
            super::handle_click(
                &mut state,
                "sketch.polygon",
                &mut sketch,
                Vec2D::new(2.0, 1.0),
                0.5,
                &params,
            );
            assert!(sketch.geometry.is_empty(), "nothing before completion");
            let fx = super::handle_click(
                &mut state,
                "sketch.polygon",
                &mut sketch,
                Vec2D::new(7.0, 1.0),
                0.5,
                &params,
            );
            assert!(fx.changed);
            assert!(state.is_idle());
            let n = sides as usize;
            assert_eq!((points(&sketch), lines(&sketch)), (n, n));
            // Closed loop: every vertex used by exactly two lines.
            let uses = point_use_counts(&sketch);
            assert_eq!(uses.len(), n);
            assert!(uses.values().all(|&c| c == 2), "closed loop for n={n}");
            // All vertices on the circumscribed circle of radius 5.
            for g in &sketch.geometry {
                if let GeometryElement::Point(p) = g {
                    let r = (p.position - Vec2D::new(2.0, 1.0)).to_glam().length();
                    assert!((r - 5.0).abs() < 1e-4, "vertex off circle: r={r}");
                }
            }
        }
    }

    #[test]
    fn polygon_first_vertex_at_click_position() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let params = ToolParams::default();
        super::handle_click(
            &mut state,
            "sketch.polygon",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
            &params,
        );
        super::handle_click(
            &mut state,
            "sketch.polygon",
            &mut sketch,
            Vec2D::new(3.0, 4.0),
            0.5,
            &params,
        );
        let hit = sketch.geometry.iter().any(|g| match g {
            GeometryElement::Point(p) => {
                (p.position - Vec2D::new(3.0, 4.0)).to_glam().length() < 1e-4
            }
            _ => false,
        });
        assert!(hit, "clicked vertex is a polygon vertex");
    }

    #[test]
    fn degenerate_polygon_rejected() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let params = ToolParams::default();
        super::handle_click(
            &mut state,
            "sketch.polygon",
            &mut sketch,
            Vec2D::new(1.0, 1.0),
            0.5,
            &params,
        );
        let fx = super::handle_click(
            &mut state,
            "sketch.polygon",
            &mut sketch,
            Vec2D::new(1.0, 1.0),
            0.5,
            &params,
        );
        assert!(!fx.changed, "vertex == center is degenerate");
        assert!(sketch.geometry.is_empty());
    }

    #[test]
    fn slot_two_clicks_builds_closed_stadium() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let params = ToolParams {
            slot_width: 4.0,
            ..ToolParams::default()
        };
        super::handle_click(
            &mut state,
            "sketch.slot",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
            &params,
        );
        assert!(sketch.geometry.is_empty(), "nothing before completion");
        let fx = super::handle_click(
            &mut state,
            "sketch.slot",
            &mut sketch,
            Vec2D::new(10.0, 0.0),
            0.5,
            &params,
        );
        assert!(fx.changed);
        assert!(state.is_idle());
        // 4 junction points + 2 arc centers, 2 lines, 2 cap arcs.
        assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (6, 2, 2));
        // Junction points shared by exactly one line + one arc; the two arc
        // centers referenced once each.
        let uses = point_use_counts(&sketch);
        let mut counts: Vec<usize> = uses.values().copied().collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![1, 1, 2, 2, 2, 2]);

        // The classic slot must be ONE closed profile wire of 4 segments.
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);

        // CCW-consistent caps bulge OUTWARD: the arc midpoints must sit on
        // the centerline extended past each end (x = -2 and x = 12).
        let mut arc_mid_xs: Vec<f64> = wires[0]
            .segments
            .iter()
            .filter_map(|s| match s {
                kernel_api::ProfileSegment::Arc { mid, .. } => Some(mid[0]),
                _ => None,
            })
            .collect();
        arc_mid_xs.sort_by(f64::total_cmp);
        assert_eq!(arc_mid_xs.len(), 2);
        assert!((arc_mid_xs[0] + 2.0).abs() < 1e-4, "left cap bulges left");
        assert!(
            (arc_mid_xs[1] - 12.0).abs() < 1e-4,
            "right cap bulges right"
        );
    }

    #[test]
    fn slot_works_on_diagonal_centerline() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let params = ToolParams::default();
        super::handle_click(
            &mut state,
            "sketch.slot",
            &mut sketch,
            Vec2D::new(1.0, 1.0),
            0.5,
            &params,
        );
        super::handle_click(
            &mut state,
            "sketch.slot",
            &mut sketch,
            Vec2D::new(7.0, 9.0),
            0.5,
            &params,
        );
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn degenerate_slot_rejected() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let params = ToolParams::default();
        super::handle_click(
            &mut state,
            "sketch.slot",
            &mut sketch,
            Vec2D::new(3.0, 3.0),
            0.5,
            &params,
        );
        let fx = super::handle_click(
            &mut state,
            "sketch.slot",
            &mut sketch,
            Vec2D::new(3.0, 3.0),
            0.5,
            &params,
        );
        assert!(!fx.changed, "zero-length centerline is degenerate");
        assert!(sketch.geometry.is_empty());
    }

    /// Rectangle (0,0)-(w,h) built from shared corner points.
    fn build_rectangle(sketch: &mut Sketch, w: f32, h: f32) -> [Uuid; 4] {
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(w, 0.0))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(w, h))));
        let d = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, h))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        sketch.add_geometry(GeometryElement::Line(Line::new(c, d)));
        sketch.add_geometry(GeometryElement::Line(Line::new(d, a)));
        [a, b, c, d]
    }

    fn fillet_params(radius: f32) -> ToolParams {
        ToolParams {
            fillet_radius: radius,
            ..ToolParams::default()
        }
    }

    #[test]
    fn fillet_rounds_rectangle_corner_tangentially() {
        let mut sketch = Sketch::new("t");
        build_rectangle(&mut sketch, 12.0, 8.0);
        let mut state = ToolState::default();
        let fx = super::handle_click(
            &mut state,
            "sketch.fillet",
            &mut sketch,
            Vec2D::new(12.0, 8.0),
            0.5,
            &fillet_params(2.0),
        );
        assert!(fx.changed);
        // Corner point replaced by 2 tangent points + 1 arc center.
        assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (6, 4, 1));

        let arc = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Arc(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap();
        let center = sketch.point_position(arc.center).unwrap();
        let start = sketch.point_position(arc.start).unwrap();
        let end = sketch.point_position(arc.end).unwrap();
        // 90° corner with R=2: center at (10, 6), tangent points at
        // (12, 6) and (10, 8).
        assert!((center.to_glam() - glam::Vec2::new(10.0, 6.0)).length() < 1e-4);
        assert!((arc.radius - 2.0).abs() < 1e-5);
        assert!(((start.to_glam() - center.to_glam()).length() - 2.0).abs() < 1e-4);
        assert!(((end.to_glam() - center.to_glam()).length() - 2.0).abs() < 1e-4);
        // The arc bridges on the corner side: its midpoint bulges toward
        // the removed corner (12,8), i.e. beyond the chord.
        let (start_angle, sweep) =
            crate::snap::arc_angles((start - center).to_glam(), (end - center).to_glam());
        assert!(
            (sweep - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "quarter-circle sweep, got {sweep}"
        );
        let mid_angle = start_angle + sweep * 0.5;
        let mid = center.to_glam() + 2.0 * glam::Vec2::new(mid_angle.cos(), mid_angle.sin());
        assert!(
            mid.x > 10.5 && mid.y > 6.5,
            "arc bulges toward the corner: {mid:?}"
        );

        // Profile survives: one closed wire of 4 lines + 1 arc.
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 5);
    }

    #[test]
    fn fillet_radius_too_large_rejected() {
        let mut sketch = Sketch::new("t");
        build_rectangle(&mut sketch, 3.0, 3.0);
        let mut state = ToolState::default();
        // 90° corner: tangent offset equals the radius; 4 > 3 cannot fit.
        let fx = super::handle_click(
            &mut state,
            "sketch.fillet",
            &mut sketch,
            Vec2D::new(3.0, 3.0),
            0.5,
            &fillet_params(4.0),
        );
        assert!(!fx.changed);
        assert_eq!(
            (points(&sketch), lines(&sketch), arcs(&sketch)),
            (4, 4, 0),
            "sketch untouched"
        );
    }

    #[test]
    fn fillet_ignores_non_corner_clicks() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let mut state = ToolState::default();
        let params = fillet_params(2.0);

        // Empty space: no point within tolerance.
        let fx = super::handle_click(
            &mut state,
            "sketch.fillet",
            &mut sketch,
            Vec2D::new(5.0, 5.0),
            0.5,
            &params,
        );
        assert!(!fx.changed);
        // Endpoint with only ONE line attached.
        let fx = super::handle_click(
            &mut state,
            "sketch.fillet",
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            0.5,
            &params,
        );
        assert!(!fx.changed);
        assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (2, 1, 0));
    }

    #[test]
    fn fillet_rejects_collinear_segments() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let m = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, m)));
        sketch.add_geometry(GeometryElement::Line(Line::new(m, b)));
        let mut state = ToolState::default();
        let fx = super::handle_click(
            &mut state,
            "sketch.fillet",
            &mut sketch,
            Vec2D::new(5.0, 0.0),
            0.5,
            &fillet_params(1.0),
        );
        assert!(!fx.changed, "collinear corner has no angle to round");
        assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (3, 2, 0));
    }

    #[test]
    fn fillet_drops_constraints_on_removed_corner() {
        let mut sketch = Sketch::new("t");
        let [.., c, _] = build_rectangle(&mut sketch, 12.0, 8.0);
        sketch.constraints.push(Constraint::FixedPoint {
            point: c,
            position: Vec2D::new(12.0, 8.0),
        });
        let mut state = ToolState::default();
        let fx = super::handle_click(
            &mut state,
            "sketch.fillet",
            &mut sketch,
            Vec2D::new(12.0, 8.0),
            0.5,
            &fillet_params(2.0),
        );
        assert!(fx.changed);
        assert!(
            sketch.constraints.is_empty(),
            "constraint on the removed corner dropped"
        );
        assert!(sketch.get_geometry(c).is_none(), "corner point removed");
    }

    #[test]
    fn point_tool_places_point() {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let fx = handle_click(
            &mut state,
            "sketch.point",
            &mut sketch,
            Vec2D::new(2.0, 3.0),
            0.5,
        );
        assert!(fx.changed);
        assert_eq!(points(&sketch), 1);
        assert!(state.is_idle());
    }
}
