//! Where a sketch element stands as a shape the cursor can be measured
//! against: its points looked up, its numbers checked, and the distance
//! from a point to its nearest bit.

use core_document::mesh_query::nearest_on_segment;
use glam::DVec2;
use uuid::Uuid;

use crate::curves::Curve;
use crate::geom2d::{Prim, arc_angles};
use crate::sketch::{GeometryElement, Sketch, Vec2D};

/// Distance from `pos` to the closest bit of `geom` (sketch units).
/// `None` for unresolvable references and numbers out of range.
pub fn distance_to_element(sketch: &Sketch, geom: &GeometryElement, pos: Vec2D) -> Option<f64> {
    Shape::of(&|id| sketch.point_position(id), geom).map(|shape| shape.distance(pos.to_glam()))
}

/// [`distance_to_element`] when it is at most `reach`: a curve whose
/// bound keeps it further away is passed over without its nearest point.
pub fn distance_within(
    sketch: &Sketch,
    geom: &GeometryElement,
    pos: Vec2D,
    reach: f64,
) -> Option<f64> {
    let shape = Shape::of(&|id| sketch.point_position(id), geom)?;
    let p = pos.to_glam();
    if shape.distance_at_least(p) > reach {
        return None;
    }
    Some(shape.distance(p)).filter(|d| *d <= reach)
}

/// One element where it stands.
#[derive(Debug, Clone)]
pub(crate) enum Shape {
    Point(DVec2),
    /// A line, a circle or an arc.
    Prim(Prim),
    /// An ellipse, a parabola, a hyperbola or a spline.
    Curve(Curve),
}

impl Shape {
    /// `geom` with its points looked up by `point`: `None` where one is
    /// missing, a number is not finite or a circle's radius is negative.
    pub(crate) fn of(
        point: &dyn Fn(Uuid) -> Option<Vec2D>,
        geom: &GeometryElement,
    ) -> Option<Self> {
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
    pub(crate) fn is_sound(&self) -> bool {
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
    pub(crate) fn distance(&self, p: DVec2) -> f64 {
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
    pub(crate) fn distance_at_least(&self, p: DVec2) -> f64 {
        match self {
            Shape::Curve(curve) => curve
                .bound()
                .map_or(0.0, |(c, r)| ((p - c).length() - r).max(0.0)),
            _ => 0.0,
        }
    }

    /// The numbers that place the shape, for settling ties between
    /// shapes alike in everything else.
    pub(crate) fn key(&self) -> [f64; 6] {
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
    pub(crate) fn project(&self, p: DVec2) -> Option<DVec2> {
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
    pub(crate) fn extension_foot(&self, p: DVec2) -> Option<(DVec2, f64)> {
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
pub(crate) fn on_sweep(c: DVec2, s: DVec2, e: DVec2, p: DVec2) -> bool {
    let (start, sweep) = arc_angles(s - c, e - c);
    let d = p - c;
    (d.y.atan2(d.x) - start).rem_euclid(std::f64::consts::TAU) <= sweep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{BSpline, Point};

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
        let spline = GeometryElement::BSpline(BSpline::new(poles, false));
        sketch.add_geometry(spline.clone());
        let curve = Curve::of(&sketch, &spline).unwrap();
        for i in 1..40 {
            let t = curve.span.0 + (curve.span.1 - curve.span.0) * i as f64 / 40.0;
            let on = curve.at(t);
            let d = distance_to_element(&sketch, &spline, Vec2D::new(on.x, on.y)).unwrap();
            // A scripted click takes curves within a micrometre (`sketch.draw`).
            assert!(d < 1e-6, "t = {t}: {d} off a point of the curve");
        }
    }
}
