//! Cursor snapping: endpoint reuse, curves (the sketch's origin and axes
//! among them) and axis alignment.
//!
//! Snapping to an existing point *reuses* that point id instead of creating
//! a coincident twin, so shared endpoints produce naturally connected
//! profiles (the simple, robust alternative to auto-coincident constraints).

use uuid::Uuid;

use crate::sketch::{GeometryElement, ORIGIN_ID, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID};

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
    /// Somewhere on a curve.
    OnCurve,
    /// Somewhere on one of the sketch's axes.
    OnAxis,
    /// Level with the point being drawn from.
    Horizontal,
    /// Plumb with the point being drawn from.
    Vertical,
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
            SnapKind::OnCurve => "On curve",
            SnapKind::OnAxis => "On axis",
            SnapKind::Horizontal => "Horizontal",
            SnapKind::Vertical => "Vertical",
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
}

/// What a snap knows of the drawing in progress: the point a segment is
/// drawn from (for horizontal and vertical alignment) and points it must
/// not land on (that same point).
#[derive(Debug, Clone, Default)]
pub struct SnapContext {
    pub from: Option<Vec2D>,
    pub exclude: Vec<Uuid>,
}

/// Where `cursor` snaps, within `tol` (sketch units), in this order:
/// an existing point (reused), the origin, a crossing of two curves, a
/// line's middle, a curve or an axis, then alignment with `from`. The one
/// function every drawing click and the cue before it go through, so what
/// is shown is where the click lands.
pub fn resolve(sketch: &Sketch, cursor: Vec2D, tol: f32, cx: &SnapContext) -> Snap {
    let bare = Snap {
        target: SnapTarget::New(cursor),
        pos: cursor,
        kind: None,
    };
    if tol <= 0.0 {
        return bare;
    }
    let at = |target: SnapTarget, pos: Vec2D, kind: SnapKind| Snap {
        target,
        pos,
        kind: Some(kind),
    };
    if let SnapTarget::Existing(id) = snap_to_point(sketch, cursor, tol, &cx.exclude)
        && let Some(pos) = sketch.point_position(id)
    {
        let center = sketch.geometry.iter().any(|g| match g {
            GeometryElement::Circle(c) => c.center == id,
            GeometryElement::Arc(a) => a.center == id,
            GeometryElement::Ellipse(e) => e.center == id,
            _ => false,
        });
        let kind = if center {
            SnapKind::Center
        } else {
            SnapKind::Endpoint
        };
        return at(SnapTarget::Existing(id), pos, kind);
    }
    let near = |p: Vec2D| (p - cursor).to_glam().length();
    let origin = Vec2D::new(0.0, 0.0);
    if !cx.exclude.contains(&ORIGIN_ID) && near(origin) <= tol {
        return at(SnapTarget::New(origin), origin, SnapKind::Origin);
    }
    if let Some(cross) = crossing_near(sketch, cursor, tol) {
        return at(SnapTarget::New(cross), cross, SnapKind::Intersection);
    }
    let middle = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                let a = sketch.point_position(l.start)?;
                let b = sketch.point_position(l.end)?;
                Some(Vec2D::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5))
            }
            _ => None,
        })
        .filter(|m| near(*m) <= tol)
        .min_by(|a, b| near(*a).total_cmp(&near(*b)));
    if let Some(m) = middle {
        return at(SnapTarget::New(m), m, SnapKind::Midpoint);
    }
    if let Some((id, proj)) = snap_to_curve(sketch, cursor, tol, &cx.exclude) {
        let kind = if id == X_AXIS_ID || id == Y_AXIS_ID {
            SnapKind::OnAxis
        } else if id == ORIGIN_ID {
            SnapKind::Origin
        } else {
            SnapKind::OnCurve
        };
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
    }
    bare
}

/// How far the axes reach when crossed with other curves.
const AXIS_REACH: f32 = 1.0e6;

/// The crossing of two curves (the sketch's axes among them) nearest
/// `cursor`, within `tol`.
fn crossing_near(sketch: &Sketch, cursor: Vec2D, tol: f32) -> Option<Vec2D> {
    use crate::geom2d::{Prim, prim_of, raw_hits, within};
    let c = cursor.to_glam();
    let mut prims: Vec<Prim> = sketch
        .geometry
        .iter()
        .filter(|g| !matches!(g, GeometryElement::Point(_)))
        .filter(|g| distance_to_element(sketch, g, cursor).is_some_and(|d| d <= tol))
        .filter_map(|g| prim_of(sketch, g))
        .collect();
    if cursor.y.abs() <= tol {
        prims.push(Prim::Seg {
            a: glam::Vec2::new(-AXIS_REACH, 0.0),
            b: glam::Vec2::new(AXIS_REACH, 0.0),
        });
    }
    if cursor.x.abs() <= tol {
        prims.push(Prim::Seg {
            a: glam::Vec2::new(0.0, -AXIS_REACH),
            b: glam::Vec2::new(0.0, AXIS_REACH),
        });
    }
    let mut best: Option<(glam::Vec2, f32)> = None;
    for i in 0..prims.len() {
        for j in i + 1..prims.len() {
            for p in raw_hits(&prims[i], &prims[j]) {
                let d = (p - c).length();
                if d <= tol
                    && within(&prims[i], p)
                    && within(&prims[j], p)
                    && best.is_none_or(|(_, bd)| d < bd)
                {
                    best = Some((p, d));
                }
            }
        }
    }
    best.map(|(p, _)| Vec2D::from_glam(p))
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
/// constraint).
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
        // Sampled curves: distance to the tessellated polyline is accurate
        // to well under any click tolerance.
        GeometryElement::Ellipse(e) => polyline_distance(&e.points(sketch, 48)?, p),
        GeometryElement::BSpline(b) => {
            let ctrl: Option<Vec<Vec2D>> = b
                .control_points
                .iter()
                .map(|id| sketch.point_position(*id))
                .collect();
            polyline_distance(&crate::geom2d::bspline_points(&ctrl?, b.periodic, 64), p)
        }
    }
}

/// Distance from `p` to a sampled polyline. `None` for fewer than 2 samples.
fn polyline_distance(pts: &[Vec2D], p: glam::Vec2) -> Option<f32> {
    pts.windows(2)
        .map(|w| {
            let a = w[0].to_glam();
            let ab = w[1].to_glam() - a;
            let len_sq = ab.length_squared();
            if len_sq < 1e-12 {
                (p - a).length()
            } else {
                let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
                (p - (a + ab * t)).length()
            }
        })
        .min_by(f32::total_cmp)
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

/// Projection of `pos` onto a curve element (`None` off the curve's range
/// or for kinds without an on-curve constraint: points, ellipses, splines).
fn project_to_curve(sketch: &Sketch, geom: &GeometryElement, pos: Vec2D) -> Option<Vec2D> {
    let p = pos.to_glam();
    match geom {
        GeometryElement::Line(line) => {
            let a = sketch.point_position(line.start)?.to_glam();
            let b = sketch.point_position(line.end)?.to_glam();
            let ab = b - a;
            let len_sq = ab.length_squared();
            if len_sq < 1e-12 {
                return None;
            }
            let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
            Some(Vec2D::from_glam(a + ab * t))
        }
        GeometryElement::Circle(circle) => {
            let c = sketch.point_position(circle.center)?.to_glam();
            let dir = p - c;
            if dir.length() < 1e-6 {
                return None;
            }
            Some(Vec2D::from_glam(c + dir.normalize() * circle.radius))
        }
        GeometryElement::Arc(arc) => {
            let c = sketch.point_position(arc.center)?.to_glam();
            let s = sketch.point_position(arc.start)?.to_glam();
            let e = sketch.point_position(arc.end)?.to_glam();
            let dir = p - c;
            if dir.length() < 1e-6 {
                return None;
            }
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let mut rel = dir.y.atan2(dir.x) - start_angle;
            while rel < 0.0 {
                rel += std::f32::consts::TAU;
            }
            (rel <= sweep).then(|| Vec2D::from_glam(c + dir.normalize() * (s - c).length()))
        }
        _ => None,
    }
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
    tol: f32,
    exclude: &[Uuid],
) -> Option<(Uuid, Vec2D)> {
    let mut best: Option<(Uuid, Vec2D, f32)> = None;
    for geom in &sketch.geometry {
        if exclude.contains(&geom.id()) {
            continue;
        }
        let Some(proj) = project_to_curve(sketch, geom, cursor) else {
            continue;
        };
        let d = (proj - cursor).to_glam().length();
        if d <= tol && best.as_ref().map(|(_, _, bd)| d < *bd).unwrap_or(true) {
            best = Some((geom.id(), proj, d));
        }
    }
    let origin = Vec2D::new(0.0, 0.0);
    if !exclude.contains(&ORIGIN_ID) && cursor.to_glam().length() <= tol {
        return Some((ORIGIN_ID, origin));
    }
    for (id, proj) in [
        (X_AXIS_ID, Vec2D::new(cursor.x, 0.0)),
        (Y_AXIS_ID, Vec2D::new(0.0, cursor.y)),
    ] {
        if exclude.contains(&id) {
            continue;
        }
        let d = (proj - cursor).to_glam().length();
        if d <= tol && best.as_ref().map(|(_, _, bd)| d < *bd).unwrap_or(true) {
            best = Some((id, proj, d));
        }
    }
    best.map(|(id, proj, _)| (id, proj))
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
    if let Some((id, _)) = best_point.or(best_curve) {
        return Some(id);
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
        // A line drawn along the X axis takes the snap at the same distance.
        let (sketch, _, _, l) = sketch_with_line();
        assert_eq!(
            snap_to_curve(&sketch, Vec2D::new(5.0, 0.2), 0.5, &[]).map(|(id, _)| id),
            Some(l)
        );
        assert_eq!(
            snap_to_curve(&sketch, Vec2D::new(5.0, 0.2), 0.5, &[l]).map(|(id, _)| id),
            Some(X_AXIS_ID)
        );
    }

    #[test]
    fn drawn_geometry_wins_over_the_reference_under_it() {
        let (sketch, a, _, l) = sketch_with_line();
        // The line runs along the X axis from the origin: it answers for
        // both its mid-span and its endpoint.
        assert_eq!(hit_test(&sketch, Vec2D::new(5.0, 0.1), 0.5), Some(l));
        assert_eq!(hit_test(&sketch, Vec2D::new(0.05, 0.0), 0.5), Some(a));
    }
}
