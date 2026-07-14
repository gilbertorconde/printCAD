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
/// coords). `snap_tol` is in sketch units.
pub fn handle_click(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
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

fn short(id: Uuid) -> String {
    id.to_string()[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_kind(sketch: &Sketch, f: impl Fn(&GeometryElement) -> bool) -> usize {
        sketch.geometry.iter().filter(|g| f(g)).count()
    }
    fn points(s: &Sketch) -> usize {
        count_kind(s, |g| matches!(g, GeometryElement::Point(_)))
    }
    fn lines(s: &Sketch) -> usize {
        count_kind(s, |g| matches!(g, GeometryElement::Line(_)))
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
