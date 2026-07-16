//! Cursor snapping: endpoint reuse and axis alignment.
//!
//! Snapping to an existing point *reuses* that point id instead of creating
//! a coincident twin, so shared endpoints produce naturally connected
//! profiles (the simple, robust alternative to auto-coincident constraints).

use uuid::Uuid;

use crate::sketch::{GeometryElement, Sketch, Vec2D};

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

/// Axis alignment detected while drawing a line segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSnap {
    Horizontal,
    Vertical,
}

/// Snap `cursor` to the nearest existing sketch point within `tol`
/// (sketch units). Points in `exclude` are ignored (e.g. the chain's own
/// previous point).
pub fn snap_to_point(sketch: &Sketch, cursor: Vec2D, tol: f32, exclude: &[Uuid]) -> SnapTarget {
    let mut best: Option<(Uuid, f32)> = None;
    for geom in &sketch.geometry {
        if let GeometryElement::Point(p) = geom {
            if exclude.contains(&p.id) {
                continue;
            }
            let d = (p.position - cursor).to_glam().length();
            if d <= tol && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((p.id, d));
            }
        }
    }
    match best {
        Some((id, _)) => SnapTarget::Existing(id),
        None => SnapTarget::New(cursor),
    }
}

/// While drawing from `from`, snap `cursor` onto the horizontal/vertical
/// axis through `from` when it is within `tol` of it. Returns the adjusted
/// position and which axis was snapped (used to auto-add the matching
/// constraint, mirroring FreeCAD's auto-constraints).
pub fn snap_axis(from: Vec2D, cursor: Vec2D, tol: f32) -> (Vec2D, Option<AxisSnap>) {
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

/// Distance from `pos` to the closest bit of `geom` (sketch units), used
/// for hit-testing in select mode. `None` for unresolvable references.
pub fn distance_to_element(sketch: &Sketch, geom: &GeometryElement, pos: Vec2D) -> Option<f32> {
    let p = pos.to_glam();
    match geom {
        GeometryElement::Point(pt) => Some((pt.position.to_glam() - p).length()),
        GeometryElement::Line(line) => {
            let a = sketch.point_position(line.start)?.to_glam();
            let b = sketch.point_position(line.end)?.to_glam();
            let ab = b - a;
            let len_sq = ab.length_squared();
            if len_sq < 1e-12 {
                return Some((p - a).length());
            }
            let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
            Some((p - (a + ab * t)).length())
        }
        GeometryElement::Circle(circle) => {
            let c = sketch.point_position(circle.center)?.to_glam();
            Some(((p - c).length() - circle.radius).abs())
        }
        GeometryElement::Arc(arc) => {
            let c = sketch.point_position(arc.center)?.to_glam();
            let s = sketch.point_position(arc.start)?.to_glam();
            let e = sketch.point_position(arc.end)?.to_glam();
            let radius = (s - c).length();
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let cursor_angle = (p - c).y.atan2((p - c).x);
            let mut rel = cursor_angle - start_angle;
            while rel < 0.0 {
                rel += std::f32::consts::TAU;
            }
            if rel <= sweep {
                Some(((p - c).length() - radius).abs())
            } else {
                // Off the arc's angular range: distance to nearest endpoint.
                Some((p - s).length().min((p - e).length()))
            }
        }
    }
}

/// CCW start angle and sweep (0..TAU) for an arc from `start_vec` to
/// `end_vec` (both relative to the center).
pub fn arc_angles(start_vec: glam::Vec2, end_vec: glam::Vec2) -> (f32, f32) {
    let start_angle = start_vec.y.atan2(start_vec.x);
    let end_angle = end_vec.y.atan2(end_vec.x);
    let mut sweep = end_angle - start_angle;
    while sweep <= 0.0 {
        sweep += std::f32::consts::TAU;
    }
    (start_angle, sweep)
}

/// Distance from `pos` to the nearest CURVE of the sketch (lines, arcs,
/// circles — points excluded, they're too small to be a click target for
/// feature-level selection). `None` for a sketch with no curves.
pub fn nearest_curve_distance(sketch: &Sketch, pos: Vec2D) -> Option<f32> {
    sketch
        .geometry
        .iter()
        .filter(|g| !matches!(g, GeometryElement::Point(_)))
        .filter_map(|g| distance_to_element(sketch, g, pos))
        .min_by(|a, b| a.total_cmp(b))
}

/// Topmost element within `tol` of `pos`, preferring points over curves so
/// endpoints stay clickable on top of their lines.
pub fn hit_test(sketch: &Sketch, pos: Vec2D, tol: f32) -> Option<Uuid> {
    let mut best_point: Option<(Uuid, f32)> = None;
    let mut best_curve: Option<(Uuid, f32)> = None;
    for geom in &sketch.geometry {
        let Some(d) = distance_to_element(sketch, geom, pos) else {
            continue;
        };
        if d > tol {
            continue;
        }
        let slot = match geom {
            GeometryElement::Point(_) => &mut best_point,
            _ => &mut best_curve,
        };
        if slot.map(|(_, bd)| d < bd).unwrap_or(true) {
            *slot = Some((geom.id(), d));
        }
    }
    best_point.or(best_curve).map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Circle, Line, Point, Sketch};

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
    }

    #[test]
    fn snap_excludes_requested_ids() {
        let (sketch, a, _, _) = sketch_with_line();
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
    }

    #[test]
    fn axis_snap_ignores_degenerate_click() {
        let (pos, axis) = snap_axis(Vec2D::new(0.0, 0.0), Vec2D::new(0.1, 0.1), 0.5);
        assert_eq!(axis, None);
        assert_eq!(pos.x, 0.1);
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
    fn nearest_curve_distance_ignores_points() {
        let (sketch, _, _, _) = sketch_with_line();
        // Mid-span, 2 units off the line.
        let d = nearest_curve_distance(&sketch, Vec2D::new(5.0, 2.0)).unwrap();
        assert!((d - 2.0).abs() < 1e-5);
        let empty = Sketch::new("e");
        assert!(nearest_curve_distance(&empty, Vec2D::new(0.0, 0.0)).is_none());
    }

    #[test]
    fn circle_hit_is_on_rim_not_center() {
        let mut sketch = Sketch::new("t");
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 5.0)));
        assert_eq!(hit_test(&sketch, Vec2D::new(5.1, 0.0), 0.5), Some(circle));
        // Near the middle of the circle nothing is hit (center point wins
        // only within tolerance of the center itself).
        assert_eq!(hit_test(&sketch, Vec2D::new(2.5, 0.0), 0.5), None);
    }
}
