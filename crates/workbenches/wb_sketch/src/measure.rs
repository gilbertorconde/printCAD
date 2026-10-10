//! What the constraints that reach past points and lines measure: an arc's
//! length, the gap between two items and where it lies, the tangent a
//! curve has at a point on it, the angle two curves make there, and the
//! ratio of sines a refraction holds. The solver states the same relations
//! over its own variables; these read them off the sketch as it is.

use glam::Vec2;
use uuid::Uuid;

use crate::sketch::{GeometryElement, Reference, Sketch};
use sketch_solver::contact::circles_nest;

/// An item as the measurements see it: a point, the infinite line through
/// a segment, or a whole circle (an arc counts as its circle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item {
    Point(Vec2),
    Line(Vec2, Vec2),
    Circle(Vec2, f32),
}

/// `id` as an [`Item`]: points and the origin, lines and the axes,
/// circles and arcs. `None` for anything else.
pub fn item(sketch: &Sketch, id: Uuid) -> Option<Item> {
    if let Some(reference) = Reference::of(id) {
        return Some(match reference.direction() {
            None => Item::Point(Vec2::ZERO),
            Some(d) => Item::Line(Vec2::ZERO, d.to_glam()),
        });
    }
    let pos = |id: Uuid| sketch.point_position(id).map(|p| p.to_glam());
    Some(match sketch.get_geometry(id)? {
        GeometryElement::Point(p) => Item::Point(p.position.to_glam()),
        GeometryElement::Line(l) => Item::Line(pos(l.start)?, pos(l.end)?),
        GeometryElement::Circle(c) => Item::Circle(pos(c.center)?, c.radius),
        GeometryElement::Arc(a) => Item::Circle(pos(a.center)?, a.radius),
        _ => return None,
    })
}

/// Length along an arc, start to end counter-clockwise.
pub fn arc_length(sketch: &Sketch, arc: Uuid) -> Option<f32> {
    let GeometryElement::Arc(a) = sketch.get_geometry(arc)? else {
        return None;
    };
    let c = sketch.point_position(a.center)?.to_glam();
    let s = sketch.point_position(a.start)?.to_glam();
    let e = sketch.point_position(a.end)?.to_glam();
    let (_, sweep) = crate::snap::arc_angles(s - c, e - c);
    Some(a.radius * sweep)
}

/// The gap between two items: its length and the nearest point on each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub distance: f32,
    /// On the first item.
    pub a: Vec2,
    /// On the second item.
    pub b: Vec2,
}

/// The shortest distance between two items (see [`Item`]), for the pairs a
/// gap constraint takes: a point, line or circle against a circle, a point
/// against a line, and two lines, read as parallel (the second's midpoint
/// from the first's line). A line crossing a circle, or two circles
/// crossing, are a gap of zero.
pub fn gap(sketch: &Sketch, id1: Uuid, id2: Uuid) -> Option<Gap> {
    let (i1, i2) = (item(sketch, id1)?, item(sketch, id2)?);
    match gap_ordered(i1, i2) {
        Some(g) => Some(g),
        None => gap_ordered(i2, i1).map(|g| Gap {
            distance: g.distance,
            a: g.b,
            b: g.a,
        }),
    }
}

fn unit_or_x(v: Vec2) -> Vec2 {
    v.try_normalize().unwrap_or(Vec2::X)
}

fn foot_on_line(p: Vec2, s: Vec2, e: Vec2) -> Vec2 {
    let d = e - s;
    let len_sq = d.length_squared();
    if len_sq < 1e-12 {
        return s;
    }
    s + d * ((p - s).dot(d) / len_sq)
}

fn gap_ordered(i1: Item, i2: Item) -> Option<Gap> {
    Some(match (i1, i2) {
        (Item::Point(p), Item::Line(s, e)) => {
            let foot = foot_on_line(p, s, e);
            Gap {
                distance: (p - foot).length(),
                a: p,
                b: foot,
            }
        }
        (Item::Point(p), Item::Circle(c, r)) => {
            let d = (p - c).length();
            Gap {
                distance: (d - r).abs(),
                a: p,
                b: c + unit_or_x(p - c) * r,
            }
        }
        (Item::Line(s1, e1), Item::Line(s2, e2)) => {
            let middle = (s2 + e2) * 0.5;
            let foot = foot_on_line(middle, s1, e1);
            Gap {
                distance: (middle - foot).length(),
                a: foot,
                b: middle,
            }
        }
        (Item::Line(s, e), Item::Circle(c, r)) => {
            let foot = foot_on_line(c, s, e);
            let toward = (foot - c)
                .try_normalize()
                .unwrap_or_else(|| unit_or_x(e - s).perp());
            Gap {
                distance: ((foot - c).length() - r).max(0.0),
                a: foot,
                b: c + toward * r,
            }
        }
        (Item::Circle(c1, r1), Item::Circle(c2, r2)) => {
            let d = (c2 - c1).length();
            if circles_nest(f64::from(d), f64::from(r1), f64::from(r2)) {
                // Along the line from the larger circle's center through
                // the smaller's: where the smaller comes nearest the rim.
                let (big_c, big_r, small_c, small_r) = if r1 >= r2 {
                    (c1, r1, c2, r2)
                } else {
                    (c2, r2, c1, r1)
                };
                let u = unit_or_x(small_c - big_c);
                let (on_big, on_small) = (big_c + u * big_r, small_c + u * small_r);
                let (a, b) = if r1 >= r2 {
                    (on_big, on_small)
                } else {
                    (on_small, on_big)
                };
                Gap {
                    distance: (r1 - r2).abs() - d,
                    a,
                    b,
                }
            } else {
                let u = unit_or_x(c2 - c1);
                Gap {
                    distance: (d - r1 - r2).max(0.0),
                    a: c1 + u * r1,
                    b: c2 - u * r2,
                }
            }
        }
        _ => return None,
    })
}

/// The points along a spline, or along an arc of a parabola or hyperbola
/// between its ends: fine enough that their polyline's length is the
/// curve's.
pub fn curve_samples(sketch: &Sketch, curve: Uuid) -> Option<Vec<Vec2>> {
    use crate::sketch::{ConicKind, GeometryElement};
    const STEPS: usize = 256;
    let pos = |id: Uuid| sketch.point_position(id).map(|p| p.to_glam());
    match sketch.get_geometry(curve)? {
        GeometryElement::BSpline(b) => {
            let basis = crate::spline::basis_of(b)?;
            let control = b
                .control_points
                .iter()
                .map(|id| pos(*id).map(|p| [f64::from(p.x), f64::from(p.y)]))
                .collect::<Option<Vec<_>>>()?;
            Some(
                crate::spline::sample_basis(&basis, &control, STEPS)
                    .into_iter()
                    .map(|p| p.to_glam())
                    .collect(),
            )
        }
        GeometryElement::Conic(k) => {
            let c = pos(k.center)?;
            let a = k.axis.to_glam().length().max(1e-9);
            let u = k.axis.to_glam() / a;
            let w = u.perp();
            // The parameter of a point on the curve, in its own frame.
            let param = |p: Vec2| {
                let across = (p - c).dot(w);
                match k.kind {
                    ConicKind::Hyperbola => (across / k.minor.max(1e-9)).asinh(),
                    ConicKind::Parabola => across,
                }
            };
            let at = |t: f32| match k.kind {
                ConicKind::Hyperbola => c + u * (a * t.cosh()) + w * (k.minor * t.sinh()),
                ConicKind::Parabola => c + u * (t * t / (4.0 * a)) + w * t,
            };
            let (t0, t1) = (param(pos(k.start)?), param(pos(k.end)?));
            Some(
                (0..=STEPS)
                    .map(|i| at(t0 + (t1 - t0) * i as f32 / STEPS as f32))
                    .collect(),
            )
        }
        _ => None,
    }
}

/// The length along a spline, or along an arc of a conic between its ends.
pub fn curve_length(sketch: &Sketch, curve: Uuid) -> Option<f32> {
    let pts = curve_samples(sketch, curve)?;
    Some(pts.windows(2).map(|w| (w[1] - w[0]).length()).sum())
}

/// The angle an arc sweeps counter-clockwise from its start to its end, in
/// radians (0 to a whole turn).
pub fn arc_sweep(sketch: &Sketch, arc: Uuid) -> Option<f32> {
    let crate::sketch::GeometryElement::Arc(a) = sketch.get_geometry(arc)? else {
        return None;
    };
    let pos = |id: Uuid| sketch.point_position(id).map(|p| p.to_glam());
    let (c, s, e) = (pos(a.center)?, pos(a.start)?, pos(a.end)?);
    Some(crate::snap::arc_angles(s - c, e - c).1)
}

/// The angle at `vertex` counter-clockwise from the arm to `point1` to the
/// arm to `point2`, in radians (0 to a whole turn).
pub fn angle_three_points(
    sketch: &Sketch,
    point1: Uuid,
    vertex: Uuid,
    point2: Uuid,
) -> Option<f32> {
    let pos = |id: Uuid| sketch.point_position(id).map(|p| p.to_glam());
    let (a, v, b) = (pos(point1)?, pos(vertex)?, pos(point2)?);
    let (d1, d2) = (a - v, b - v);
    if d1.length_squared() < 1e-12 || d2.length_squared() < 1e-12 {
        return None;
    }
    Some(
        (d1.perp_dot(d2))
            .atan2(d1.dot(d2))
            .rem_euclid(std::f32::consts::TAU),
    )
}

/// The unit tangent of a curve at `at`: a line's direction start to end, a
/// circle's or arc's counter-clockwise direction at the point's angle.
pub fn tangent(sketch: &Sketch, curve: Uuid, at: Vec2) -> Option<Vec2> {
    match item(sketch, curve)? {
        Item::Line(s, e) => (e - s).try_normalize(),
        Item::Circle(c, _) => (at - c).try_normalize().map(Vec2::perp),
        Item::Point(_) => None,
    }
}

/// The signed angle (radians) from `curve1`'s tangent to `curve2`'s at
/// `point`.
pub fn angle_at_point(sketch: &Sketch, curve1: Uuid, curve2: Uuid, point: Uuid) -> Option<f32> {
    let at = sketch.point_position(point)?.to_glam();
    let t1 = tangent(sketch, curve1, at)?;
    let t2 = tangent(sketch, curve2, at)?;
    Some(t1.perp_dot(t2).atan2(t1.dot(t2)))
}

/// A ray's two ends, nearer `at` first.
pub fn ray_ends(sketch: &Sketch, ray: Uuid, at: Vec2) -> Option<(Vec2, Vec2)> {
    let Item::Line(s, e) = item(sketch, ray)? else {
        return None;
    };
    Some(if (s - at).length_squared() <= (e - at).length_squared() {
        (s, e)
    } else {
        (e, s)
    })
}

/// The ratio of sines a refraction at `point` shows: the sine of the angle
/// `ray1` arrives at over that of the angle `ray2` leaves at, both from the
/// interface's normal and signed along its tangent. `None` when the second
/// ray runs along the normal (a sine of zero).
pub fn refraction_ratio(
    sketch: &Sketch,
    ray1: Uuid,
    ray2: Uuid,
    interface: Uuid,
    point: Uuid,
) -> Option<f32> {
    let at = sketch.point_position(point)?.to_glam();
    let t = tangent(sketch, interface, at)?;
    let (near1, far1) = ray_ends(sketch, ray1, at)?;
    let (near2, far2) = ray_ends(sketch, ray2, at)?;
    let sin_in = (near1 - far1).try_normalize()?.dot(t);
    let sin_out = (far2 - near2).try_normalize()?.dot(t);
    (sin_out.abs() > 1e-6).then(|| sin_in / sin_out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Line, Point, Vec2D};

    fn point(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    fn circle(sketch: &mut Sketch, x: f32, y: f32, r: f32) -> Uuid {
        let c = point(sketch, x, y);
        sketch.add_geometry(GeometryElement::Circle(Circle::new(c, r)))
    }

    #[test]
    fn a_quarter_arc_is_a_quarter_of_its_circle() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 2.0, 0.0);
        let e = point(&mut sketch, 0.0, 2.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 2.0)));
        let len = arc_length(&sketch, arc).unwrap();
        assert!((len - std::f32::consts::PI).abs() < 1e-5, "{len}");
    }

    #[test]
    fn gaps_between_circles_outside_and_inside() {
        let mut sketch = Sketch::new("t");
        let a = circle(&mut sketch, 0.0, 0.0, 2.0);
        let b = circle(&mut sketch, 10.0, 0.0, 3.0);
        let g = gap(&sketch, a, b).unwrap();
        assert!((g.distance - 5.0).abs() < 1e-5);
        assert!((g.a - Vec2::new(2.0, 0.0)).length() < 1e-5);
        assert!((g.b - Vec2::new(7.0, 0.0)).length() < 1e-5);

        let inner = circle(&mut sketch, 1.0, 0.0, 1.0);
        let big = circle(&mut sketch, 0.0, 0.0, 5.0);
        let g = gap(&sketch, inner, big).unwrap();
        assert!((g.distance - 3.0).abs() < 1e-5, "{g:?}");
        assert!((g.a - Vec2::new(2.0, 0.0)).length() < 1e-5, "on the inner");
        assert!((g.b - Vec2::new(5.0, 0.0)).length() < 1e-5, "on the outer");
    }

    #[test]
    fn a_line_and_a_circle_either_way_round() {
        let mut sketch = Sketch::new("t");
        let s = point(&mut sketch, -5.0, 6.0);
        let e = point(&mut sketch, 5.0, 6.0);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
        let c = circle(&mut sketch, 0.0, 0.0, 2.0);
        let g = gap(&sketch, line, c).unwrap();
        assert!((g.distance - 4.0).abs() < 1e-5);
        assert!((g.a - Vec2::new(0.0, 6.0)).length() < 1e-5);
        assert!((g.b - Vec2::new(0.0, 2.0)).length() < 1e-5);
        let g = gap(&sketch, c, line).unwrap();
        assert!((g.a - Vec2::new(0.0, 2.0)).length() < 1e-5, "order kept");
        let g = gap(&sketch, line, line).unwrap();
        assert!(g.distance.abs() < 1e-5, "a line is no distance from itself");
    }

    #[test]
    fn a_line_leaving_a_circle_along_its_radius_meets_it_square() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 3.0, 0.0);
        let e = point(&mut sketch, 0.0, 3.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 3.0)));
        let far = point(&mut sketch, 8.0, 0.0);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(s, far)));
        let angle = angle_at_point(&sketch, line, arc, s).unwrap();
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "{angle}"
        );
    }

    #[test]
    fn a_straight_ray_refracts_at_a_ratio_of_one() {
        let mut sketch = Sketch::new("t");
        let a = point(&mut sketch, -3.0, 4.0);
        let o = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, 3.0, -4.0);
        let r1 = sketch.add_geometry(GeometryElement::Line(Line::new(a, o)));
        let r2 = sketch.add_geometry(GeometryElement::Line(Line::new(b, o)));
        let l = point(&mut sketch, -10.0, 0.0);
        let r = point(&mut sketch, 10.0, 0.0);
        let interface = sketch.add_geometry(GeometryElement::Line(Line::new(l, r)));
        let ratio = refraction_ratio(&sketch, r1, r2, interface, o).unwrap();
        assert!((ratio - 1.0).abs() < 1e-5, "{ratio}");
    }
}
