//! What drawn geometry was probably designed at: each line's length and
//! each round's radius next to its probable design value, and the
//! relations the geometry nearly keeps (level, upright, parallel,
//! perpendicular, equal, concentric), each with how far it is from
//! keeping it. Evidence for the user, never applied on its own: geometry
//! fitted to a scan reads 19.97 where the part was drawn at 20.

use uuid::Uuid;

use crate::sketch::{ConstraintKind, GeometryElement, Sketch, Vec2D};

/// Relations off by more than this, degrees, are not offered.
const ANGLE_DEG: f64 = 2.0;
/// Equal sizes and concentric centres off by more than this share of the
/// size, or this many mm when more, are not offered.
const SHARE: f64 = 0.01;
const LEAST_MM: f64 = 0.05;

/// A size and the value it was probably designed at.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Size {
    pub element: Uuid,
    /// `length` of a line, `radius` of a circle or arc.
    pub kind: &'static str,
    pub measured: f64,
    pub design: f64,
}

/// A relation the geometry nearly keeps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Relation {
    pub kind: &'static str,
    pub elements: Vec<Uuid>,
    /// How far from keeping it: degrees for directions, mm otherwise.
    pub offset: f64,
}

/// A line's ends, an arc's or circle's centre and radius.
enum Shape {
    Line(Vec2D, Vec2D),
    Round(Vec2D, f64),
}

fn shapes(sketch: &Sketch) -> Vec<(Uuid, Shape)> {
    let at = |id| sketch.point_position(id);
    sketch
        .geometry
        .iter()
        .filter(|g| !sketch.is_construction(g.id()) && !sketch.external.contains_key(&g.id()))
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some((l.id, Shape::Line(at(l.start)?, at(l.end)?))),
            GeometryElement::Arc(a) => Some((a.id, Shape::Round(at(a.center)?, a.radius))),
            GeometryElement::Circle(c) => Some((c.id, Shape::Round(at(c.center)?, c.radius))),
            _ => None,
        })
        .collect()
}

/// Whether a constraint already sets the size of `element`.
fn sized(sketch: &Sketch, element: Uuid) -> bool {
    sketch.constraints.iter().any(|c| match c.kind {
        ConstraintKind::Length { line, .. } => line == element,
        ConstraintKind::Radius { circle, .. } | ConstraintKind::Diameter { circle, .. } => {
            circle == element
        }
        _ => false,
    })
}

/// Whether a constraint already ties these elements in this way.
fn related(sketch: &Sketch, kind: &str, a: Uuid, b: Option<Uuid>) -> bool {
    let pair =
        |x: Uuid, y: Uuid| Some(x) == Some(a) && Some(y) == b || Some(y) == Some(a) && Some(x) == b;
    sketch.constraints.iter().any(|c| match (kind, &c.kind) {
        ("horizontal", ConstraintKind::Horizontal { element }) => *element == a,
        ("vertical", ConstraintKind::Vertical { element }) => *element == a,
        ("parallel", ConstraintKind::Parallel { line1, line2 }) => pair(*line1, *line2),
        ("perpendicular", ConstraintKind::Perpendicular { line1, line2 }) => pair(*line1, *line2),
        ("equal", ConstraintKind::EqualLength { line1, line2 }) => pair(*line1, *line2),
        ("equal", ConstraintKind::EqualRadius { circle1, circle2 }) => pair(*circle1, *circle2),
        _ => false,
    })
}

/// Every unsized line's length and round's radius, with the value it was
/// probably designed at.
pub(crate) fn sizes(sketch: &Sketch) -> Vec<Size> {
    shapes(sketch)
        .into_iter()
        .filter(|(id, _)| !sized(sketch, *id))
        .map(|(element, shape)| match shape {
            Shape::Line(a, b) => {
                let measured = (b.x - a.x).hypot(b.y - a.y);
                Size {
                    element,
                    kind: "length",
                    measured,
                    design: metrology::design::length_mm(measured),
                }
            }
            Shape::Round(_, r) => Size {
                element,
                kind: "radius",
                measured: r,
                design: metrology::design::radius_mm(r),
            },
        })
        .collect()
}

/// The relations the geometry nearly keeps and no constraint states yet,
/// nearest first within each kind.
pub(crate) fn relations(sketch: &Sketch) -> Vec<Relation> {
    let all = shapes(sketch);
    let mut out = Vec::new();
    let direction = |a: Vec2D, b: Vec2D| (b.y - a.y).atan2(b.x - a.x).to_degrees();
    // Off a direction by an angle folded into [0, 90].
    let off = |angle: f64, from: f64| {
        let d = (angle - from).rem_euclid(180.0);
        d.min(180.0 - d)
    };
    let mut push = |kind, elements: Vec<Uuid>, offset: f64| {
        out.push(Relation {
            kind,
            elements,
            offset,
        })
    };
    for (i, (a, sa)) in all.iter().enumerate() {
        if let Shape::Line(p, q) = sa {
            let t = direction(*p, *q);
            for (kind, axis) in [("horizontal", 0.0), ("vertical", 90.0)] {
                let o = off(t, axis);
                if o <= ANGLE_DEG && !related(sketch, kind, *a, None) {
                    push(kind, vec![*a], o);
                }
            }
        }
        for (b, sb) in &all[i + 1..] {
            match (sa, sb) {
                (Shape::Line(p, q), Shape::Line(r, s)) => {
                    let (t, u) = (direction(*p, *q), direction(*r, *s));
                    let level = |t: f64| off(t, 0.0) <= ANGLE_DEG || off(t, 90.0) <= ANGLE_DEG;
                    // Two lines on the axes are related through them.
                    if !(level(t) && level(u)) {
                        for (kind, by) in [("parallel", 0.0), ("perpendicular", 90.0)] {
                            let o = off(t - u, by);
                            if o <= ANGLE_DEG && !related(sketch, kind, *a, Some(*b)) {
                                push(kind, vec![*a, *b], o);
                            }
                        }
                    }
                    let (la, lb) = ((q.x - p.x).hypot(q.y - p.y), (s.x - r.x).hypot(s.y - r.y));
                    let o = (la - lb).abs();
                    if o <= (la.max(lb) * SHARE).max(LEAST_MM)
                        && !related(sketch, "equal", *a, Some(*b))
                    {
                        push("equal", vec![*a, *b], o);
                    }
                }
                (Shape::Round(c, r), Shape::Round(d, s)) => {
                    let o = (r - s).abs();
                    if o <= (r.max(*s) * SHARE).max(LEAST_MM)
                        && !related(sketch, "equal", *a, Some(*b))
                    {
                        push("equal", vec![*a, *b], o);
                    }
                    let o = (c.x - d.x).hypot(c.y - d.y);
                    if o > 0.0 && o <= (r.max(*s) * SHARE).max(LEAST_MM) {
                        push("concentric", vec![*a, *b], o);
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| a.kind.cmp(b.kind).then(a.offset.total_cmp(&b.offset)));
    out
}

/// Each of `elements` (every sized one when empty) held at its design
/// value by a driving dimension. The dimensions added.
pub(crate) fn hold_at_design(sketch: &mut Sketch, elements: &[Uuid]) -> Vec<Uuid> {
    let picked: Vec<Size> = sizes(sketch)
        .into_iter()
        .filter(|s| elements.is_empty() || elements.contains(&s.element))
        .collect();
    picked
        .into_iter()
        .map(|s| {
            sketch.add_constraint(match s.kind {
                "length" => ConstraintKind::Length {
                    line: s.element,
                    length: s.design,
                },
                _ => ConstraintKind::Radius {
                    circle: s.element,
                    radius: s.design,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Circle, Line, Point};

    /// A nearly level 19.97 mm line and a nearly upright one beside it
    /// read as level, upright and perpendicular-through-the-axes (not
    /// offered twice), the length's design value 20; two circles of
    /// nearly one radius about nearly one centre read equal and
    /// concentric. Holding at design values sets the line to 20.
    #[test]
    fn fitted_geometry_reads_its_design() {
        let mut s = Sketch::new("S");
        let mut point =
            |x: f64, y: f64| s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
        let (a, b, c) = (point(0.0, 0.0), point(19.97, 0.2), point(19.97, 10.0));
        let (o1, o2) = (point(40.0, 0.0), point(40.02, 0.0));
        let base = s.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let side = s.add_geometry(GeometryElement::Line(Line::new(b, c)));
        let inner = s.add_geometry(GeometryElement::Circle(Circle::new(o1, 4.98)));
        let outer = s.add_geometry(GeometryElement::Circle(Circle::new(o2, 5.01)));

        let found = sizes(&s);
        let base_size = found.iter().find(|z| z.element == base).unwrap();
        assert_eq!((base_size.kind, base_size.design), ("length", 20.0));
        assert_eq!(
            found.iter().find(|z| z.element == inner).unwrap().design,
            5.0
        );

        let kinds: Vec<(&str, Vec<Uuid>)> = relations(&s)
            .into_iter()
            .map(|r| (r.kind, r.elements))
            .collect();
        assert!(kinds.contains(&("horizontal", vec![base])));
        assert!(kinds.contains(&("vertical", vec![side])));
        assert!(
            !kinds.iter().any(|(k, _)| *k == "perpendicular"),
            "{kinds:?}"
        );
        assert!(kinds.contains(&("equal", vec![inner, outer])));
        assert!(kinds.contains(&("concentric", vec![inner, outer])));

        assert_eq!(hold_at_design(&mut s, &[base]).len(), 1);
        assert!(sizes(&s).iter().all(|z| z.element != base), "sized now");
        assert!(!relations(&s).is_empty());
    }
}
