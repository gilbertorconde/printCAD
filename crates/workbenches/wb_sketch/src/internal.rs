//! Internal geometry: construction lines and points that show a curve's
//! own frame, each held to the curve by an internal-alignment constraint.
//! An ellipse shows its axes and foci, a parabola its axis and focus, a
//! hyperbola its axes and its branch's focus, a B-spline its control
//! polygon. Shown, they are geometry like any other: a dimension on an axis
//! sizes the curve, a dragged focus reshapes it. Hidden, the pieces nothing
//! else holds on to go; the ones a constraint or a curve of the user's
//! reaches stay.

use std::collections::HashSet;

use uuid::Uuid;

use crate::conic::Shape;
use crate::sketch::{
    ConicKind, ConstraintKind, GeometryElement, InternalRole, Line, Point, Sketch, Vec2D,
};

/// Where a piece of internal geometry sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Point(Vec2D),
    Line(Vec2D, Vec2D),
}

fn v(x: f64, y: f64) -> Vec2D {
    Vec2D::new(x, y)
}

/// The roles `curve` shows internal geometry for, in the order they are
/// made; empty for a curve that has none.
pub fn roles(sketch: &Sketch, curve: Uuid) -> Vec<InternalRole> {
    match sketch.get_geometry(curve) {
        Some(GeometryElement::Ellipse(_)) => vec![
            InternalRole::MajorAxis,
            InternalRole::MinorAxis,
            InternalRole::Focus1,
            InternalRole::Focus2,
        ],
        Some(GeometryElement::Conic(c)) => match c.kind {
            // The axis runs to the focus, so the focus comes first.
            ConicKind::Parabola => vec![InternalRole::Focus1, InternalRole::MajorAxis],
            ConicKind::Hyperbola => vec![
                InternalRole::MajorAxis,
                InternalRole::MinorAxis,
                InternalRole::Focus1,
            ],
        },
        Some(GeometryElement::BSpline(b)) => {
            let n = b.control_points.len();
            let sides = if b.periodic { n } else { n.saturating_sub(1) };
            (0..sides as u32).map(InternalRole::ControlEdge).collect()
        }
        _ => Vec::new(),
    }
}

/// Where `role` sits on `curve` as the curve is now.
pub fn place(sketch: &Sketch, curve: Uuid, role: InternalRole) -> Option<Place> {
    match sketch.get_geometry(curve)? {
        GeometryElement::Ellipse(e) => {
            let c = sketch.point_position(e.center)?;
            let major = e.major.to_glam();
            let a = major.length();
            if a <= 1e-9 {
                return None;
            }
            let u = major / a;
            let b = a * e.ratio.min(1.0);
            let minor = u.perp() * b;
            let f = u * (a * a - b * b).max(0.0).sqrt();
            let at = |d: glam::DVec2| Vec2D::from_glam(c.to_glam() + d);
            Some(match role {
                InternalRole::MajorAxis => Place::Line(at(-major), at(major)),
                InternalRole::MinorAxis => Place::Line(at(-minor), at(minor)),
                InternalRole::Focus1 => Place::Point(at(f)),
                InternalRole::Focus2 => Place::Point(at(-f)),
                InternalRole::ControlEdge(_) => return None,
            })
        }
        GeometryElement::Conic(conic) => {
            let shape = Shape::of(conic, sketch)?;
            let at = |along: f64, across: f64| {
                let [ox, oy] = shape.origin;
                let (u, w) = (shape.u, [-shape.u[1], shape.u[0]]);
                v(
                    ox + u[0] * along + w[0] * across,
                    oy + u[1] * along + w[1] * across,
                )
            };
            let focus = match conic.kind {
                ConicKind::Parabola => shape.a,
                ConicKind::Hyperbola => shape.a.hypot(shape.b),
            };
            Some(match (role, conic.kind) {
                (InternalRole::MajorAxis, _) => Place::Line(at(0.0, 0.0), at(shape.a, 0.0)),
                (InternalRole::MinorAxis, ConicKind::Hyperbola) => {
                    Place::Line(at(0.0, -shape.b), at(0.0, shape.b))
                }
                (InternalRole::Focus1, _) => Place::Point(at(focus, 0.0)),
                _ => return None,
            })
        }
        GeometryElement::BSpline(b) => {
            let InternalRole::ControlEdge(n) = role else {
                return None;
            };
            let (from, to) = control_edge(&b.control_points, b.periodic, n)?;
            Some(Place::Line(
                sketch.point_position(from)?,
                sketch.point_position(to)?,
            ))
        }
        _ => None,
    }
}

/// The control points side `n` of a control polygon runs between.
fn control_edge(points: &[Uuid], periodic: bool, n: u32) -> Option<(Uuid, Uuid)> {
    let n = n as usize;
    let from = *points.get(n)?;
    let to = match points.get(n + 1) {
        Some(to) => *to,
        None if periodic && points.len() > 2 => points[0],
        None => return None,
    };
    Some((from, to))
}

/// The points a line in `role` is drawn through when it shares them: a
/// conic's axis starts at its centre (and a parabola's ends at its focus),
/// a control polygon's sides run between the control points.
fn shared_ends(sketch: &Sketch, curve: Uuid, role: InternalRole) -> (Option<Uuid>, Option<Uuid>) {
    match (sketch.get_geometry(curve), role) {
        (Some(GeometryElement::Conic(c)), InternalRole::MajorAxis) => {
            let focus = (c.kind == ConicKind::Parabola)
                .then(|| {
                    shown(sketch, curve)
                        .into_iter()
                        .find(|s| s.2 == InternalRole::Focus1)
                        .map(|s| s.1)
                })
                .flatten()
                .filter(|id| matches!(sketch.get_geometry(*id), Some(GeometryElement::Point(_))));
            (Some(c.center), focus)
        }
        (Some(GeometryElement::BSpline(b)), InternalRole::ControlEdge(n)) => {
            match control_edge(&b.control_points, b.periodic, n) {
                Some((from, to)) => (Some(from), Some(to)),
                None => (None, None),
            }
        }
        _ => (None, None),
    }
}

/// The internal geometry `curve` shows: `(constraint, element, role)`.
pub fn shown(sketch: &Sketch, curve: Uuid) -> Vec<(Uuid, Uuid, InternalRole)> {
    sketch
        .constraints
        .iter()
        .filter_map(|c| match c.kind {
            ConstraintKind::InternalAlignment {
                element,
                curve: of,
                role,
            } if of == curve => Some((c.id, element, role)),
            _ => None,
        })
        .collect()
}

/// The curve `element` is internal geometry of, if it is any's.
pub fn curve_of(sketch: &Sketch, element: Uuid) -> Option<Uuid> {
    sketch.constraints.iter().find_map(|c| match c.kind {
        ConstraintKind::InternalAlignment {
            element: e, curve, ..
        } if e == element => Some(curve),
        _ => None,
    })
}

/// The curves `items` name that take internal geometry: each curve itself,
/// or the curve a piece of internal geometry belongs to. In sketch order,
/// each once.
pub fn curves_of(sketch: &Sketch, items: &[Uuid]) -> Vec<Uuid> {
    let wanted: HashSet<Uuid> = items
        .iter()
        .filter_map(|id| {
            if !roles(sketch, *id).is_empty() {
                Some(*id)
            } else {
                curve_of(sketch, *id)
            }
        })
        .collect();
    sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| wanted.contains(id))
        .collect()
}

/// Whether some piece of `curve`'s internal geometry is not shown.
pub fn missing_any(sketch: &Sketch, curve: Uuid) -> bool {
    let have: HashSet<InternalRole> = shown(sketch, curve).iter().map(|s| s.2).collect();
    roles(sketch, curve).iter().any(|r| !have.contains(r))
}

/// Make every piece of `curve`'s internal geometry it does not show yet,
/// as construction where the curve is now. The ids of what was made.
pub fn show(sketch: &mut Sketch, curve: Uuid) -> Vec<Uuid> {
    let have: HashSet<InternalRole> = shown(sketch, curve).iter().map(|s| s.2).collect();
    let mut made = Vec::new();
    for role in roles(sketch, curve) {
        if have.contains(&role) {
            continue;
        }
        let Some(place) = place(sketch, curve, role) else {
            continue;
        };
        let mut point = |sketch: &mut Sketch, at: Vec2D| {
            let id = sketch.add_geometry(GeometryElement::Point(Point::new(at)));
            sketch.set_construction(id, true);
            made.push(id);
            id
        };
        let element = match place {
            Place::Point(at) => point(sketch, at),
            Place::Line(from, to) => {
                let (shared_from, shared_to) = shared_ends(sketch, curve, role);
                let start = shared_from.unwrap_or_else(|| point(sketch, from));
                let end = shared_to.unwrap_or_else(|| point(sketch, to));
                let line = sketch.add_geometry(GeometryElement::Line(Line::new(start, end)));
                sketch.set_construction(line, true);
                made.push(line);
                line
            }
        };
        sketch.add_constraint(ConstraintKind::InternalAlignment {
            element,
            curve,
            role,
        });
    }
    made
}

/// Take away `curve`'s internal geometry that nothing else holds: a piece
/// no other constraint names, and no other curve is drawn through, nor
/// (for a line) through an end of its own. The ids of what went.
pub fn hide(sketch: &mut Sketch, curve: Uuid) -> Vec<Uuid> {
    let own: HashSet<Uuid> = sketch
        .get_geometry(curve)
        .map(Sketch::curve_point_ids)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let internal: HashSet<Uuid> = shown(sketch, curve).iter().map(|s| s.0).collect();
    let held = |sketch: &Sketch, id: Uuid, by_line: Option<Uuid>| {
        sketch.constraints.iter().any(|c| {
            !internal.contains(&c.id) && crate::sketch::constraint_refs(&c.kind).contains(&id)
        }) || sketch.geometry.iter().any(|g| {
            g.id() != curve
                && Some(g.id()) != by_line
                && !internal_element(sketch, curve, g.id())
                && Sketch::curve_point_ids(g).contains(&id)
        })
    };
    let mut doomed = Vec::new();
    for (_, element, _) in shown(sketch, curve) {
        let ends = match sketch.get_geometry(element) {
            Some(GeometryElement::Line(l)) => vec![l.start, l.end],
            _ => Vec::new(),
        };
        let kept = held(sketch, element, None)
            || ends
                .iter()
                .any(|end| !own.contains(end) && held(sketch, *end, Some(element)));
        if !kept {
            doomed.push(element);
        }
    }
    if doomed.is_empty() {
        return doomed;
    }
    sketch.remove_geometry_cascade(&doomed)
}

/// Whether `id` is a piece of `curve`'s internal geometry.
fn internal_element(sketch: &Sketch, curve: Uuid, id: Uuid) -> bool {
    sketch.constraints.iter().any(|c| {
        matches!(c.kind, ConstraintKind::InternalAlignment { element, curve: of, .. }
            if element == id && of == curve)
    })
}

/// Show or hide the internal geometry of the curves `items` name.
/// `show_it` `None` shows it when some curve lacks a piece, else hides it. Returns
/// whether it showed, and what it made or took away.
pub fn toggle(sketch: &mut Sketch, items: &[Uuid], show_it: Option<bool>) -> (bool, Vec<Uuid>) {
    let curves = curves_of(sketch, items);
    let showing = show_it.unwrap_or_else(|| curves.iter().any(|c| missing_any(sketch, *c)));
    let mut changed = Vec::new();
    for curve in curves {
        if showing {
            changed.extend(show(sketch, curve));
        } else {
            changed.extend(hide(sketch, curve));
        }
    }
    (showing, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{BSpline, Conic, Ellipse};

    fn point(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(v(x, y))))
    }

    fn near(a: Vec2D, b: Vec2D) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    fn ellipse(sketch: &mut Sketch) -> Uuid {
        let c = point(sketch, 1.0, 2.0);
        sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(c, v(5.0, 0.0), 0.6)))
    }

    /// The ellipse of [`ellipse`], its centre fixed where it is.
    fn fixed_ellipse(sketch: &mut Sketch) -> Uuid {
        let e = ellipse(sketch);
        let centre = ellipse_of(sketch, e).center;
        sketch.add_constraint(ConstraintKind::FixedPoint {
            point: centre,
            position: v(1.0, 2.0),
        });
        e
    }

    fn ellipse_of(sketch: &Sketch, id: Uuid) -> Ellipse {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Ellipse(el)) => el.clone(),
            _ => panic!("{id} is no ellipse"),
        }
    }

    #[test]
    fn an_ellipse_shows_its_axes_and_foci_where_they_are() {
        let mut sketch = Sketch::new("t");
        let e = ellipse(&mut sketch);
        let made = show(&mut sketch, e);
        assert_eq!(shown(&sketch, e).len(), 4);
        // Two lines of two points each, and two foci.
        assert_eq!(made.len(), 8);
        assert!(made.iter().all(|id| sketch.is_construction(*id)));
        for (_, element, role) in shown(&sketch, e) {
            match (role, sketch.get_geometry(element)) {
                (InternalRole::MajorAxis, Some(GeometryElement::Line(l))) => {
                    assert!(near(sketch.point_position(l.start).unwrap(), v(-4.0, 2.0)));
                    assert!(near(sketch.point_position(l.end).unwrap(), v(6.0, 2.0)));
                }
                (InternalRole::MinorAxis, Some(GeometryElement::Line(l))) => {
                    assert!(near(sketch.point_position(l.start).unwrap(), v(1.0, -1.0)));
                    assert!(near(sketch.point_position(l.end).unwrap(), v(1.0, 5.0)));
                }
                // a = 5, b = 3: the foci sit 4 from the centre.
                (InternalRole::Focus1, Some(GeometryElement::Point(p))) => {
                    assert!(near(p.position, v(5.0, 2.0)));
                }
                (InternalRole::Focus2, Some(GeometryElement::Point(p))) => {
                    assert!(near(p.position, v(-3.0, 2.0)));
                }
                other => panic!("{other:?}"),
            }
        }
        // Shown again, nothing new comes.
        assert!(show(&mut sketch, e).is_empty());
    }

    #[test]
    fn hiding_keeps_what_the_user_constrained_and_deleting_the_curve_takes_it_all() {
        let mut sketch = Sketch::new("t");
        let e = ellipse(&mut sketch);
        show(&mut sketch, e);
        let major = shown(&sketch, e)
            .into_iter()
            .find(|s| s.2 == InternalRole::MajorAxis)
            .unwrap()
            .1;
        sketch.add_constraint(ConstraintKind::Length {
            line: major,
            length: 10.0,
        });
        let before = sketch.geometry.len();
        let gone = hide(&mut sketch, e);
        // The minor axis (a line, two points) and the two foci.
        assert_eq!(gone.len(), 5);
        assert_eq!(sketch.geometry.len(), before - 5);
        assert_eq!(shown(&sketch, e).len(), 1, "the dimensioned axis stays");
        sketch.remove_geometry_cascade(&[e]);
        assert!(sketch.get_geometry(major).is_none());
        assert!(sketch.constraints.is_empty());
        assert_eq!(sketch.geometry.len(), 0);
    }

    #[test]
    fn conics_and_splines_show_theirs() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 1.0, 2.0);
        let e = point(&mut sketch, 1.0, -2.0);
        let parabola = sketch.add_geometry(GeometryElement::Conic(Conic::new(
            ConicKind::Parabola,
            c,
            v(1.0, 0.0),
            0.0,
            s,
            e,
        )));
        show(&mut sketch, parabola);
        let axis = shown(&sketch, parabola)
            .into_iter()
            .find(|s| s.2 == InternalRole::MajorAxis)
            .unwrap()
            .1;
        let Some(GeometryElement::Line(l)) = sketch.get_geometry(axis) else {
            panic!()
        };
        assert_eq!(l.start, c, "a parabola's axis starts at its vertex");
        assert!(near(sketch.point_position(l.end).unwrap(), v(1.0, 0.0)));
        let focus = shown(&sketch, parabola)
            .into_iter()
            .find(|s| s.2 == InternalRole::Focus1)
            .unwrap()
            .1;
        assert_eq!(l.end, focus, "and ends at its focus");

        let ps: Vec<Uuid> = (0..4)
            .map(|i| point(&mut sketch, i as f64 * 3.0, (i % 2) as f64))
            .collect();
        let spline = sketch.add_geometry(GeometryElement::BSpline(BSpline::new(ps.clone(), true)));
        show(&mut sketch, spline);
        let sides = shown(&sketch, spline);
        assert_eq!(sides.len(), 4, "a periodic polygon closes");
        for (_, line, role) in sides {
            let InternalRole::ControlEdge(n) = role else {
                panic!()
            };
            let Some(GeometryElement::Line(l)) = sketch.get_geometry(line) else {
                panic!()
            };
            assert_eq!((l.start, l.end), (ps[n as usize], ps[(n as usize + 1) % 4]));
        }
        // Hidden, the polygon goes and the control points stay.
        hide(&mut sketch, spline);
        assert!(shown(&sketch, spline).is_empty());
        assert!(ps.iter().all(|p| sketch.get_geometry(*p).is_some()));
    }

    fn line_ends(sketch: &Sketch, line: Uuid) -> (Vec2D, Vec2D) {
        let Some(GeometryElement::Line(l)) = sketch.get_geometry(line) else {
            panic!("not a line")
        };
        (
            sketch.point_position(l.start).unwrap(),
            sketch.point_position(l.end).unwrap(),
        )
    }

    fn role(sketch: &Sketch, curve: Uuid, role: InternalRole) -> Uuid {
        shown(sketch, curve)
            .into_iter()
            .find(|s| s.2 == role)
            .unwrap()
            .1
    }

    /// Every piece of `curve`'s internal geometry sits where the curve, as
    /// it is now, puts it.
    fn aligned(sketch: &Sketch, curve: Uuid) -> bool {
        shown(sketch, curve).iter().all(|&(_, element, r)| {
            match (place(sketch, curve, r), sketch.get_geometry(element)) {
                (Some(Place::Point(at)), Some(GeometryElement::Point(p))) => near(at, p.position),
                (Some(Place::Line(a, b)), Some(GeometryElement::Line(_))) => {
                    let (s, e) = line_ends(sketch, element);
                    near(a, s) && near(b, e)
                }
                _ => false,
            }
        })
    }

    #[test]
    fn a_dimension_on_an_axis_sizes_the_ellipse() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 1.0, 2.0);
        let e = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(c, v(5.0, 1.0), 0.6)));
        show(&mut sketch, e);
        let (major, minor) = (
            role(&sketch, e, InternalRole::MajorAxis),
            role(&sketch, e, InternalRole::MinorAxis),
        );
        sketch.add_constraint(ConstraintKind::Length {
            line: major,
            length: 20.0,
        });
        sketch.add_constraint(ConstraintKind::Length {
            line: minor,
            length: 8.0,
        });
        sketch.add_constraint(ConstraintKind::Horizontal { element: minor });
        let outcome = crate::solver::solve(&mut sketch);
        assert!(
            matches!(outcome, crate::solver::SolveOutcome::Converged { .. }),
            "{outcome:?}"
        );
        let el = ellipse_of(&sketch, e);
        assert!((el.major.to_glam().length() - 10.0).abs() < 1e-3);
        assert!((el.ratio - 0.4).abs() < 1e-4);
        assert!(el.major.x.abs() < 1e-3, "the minor axis lies level");
        assert!(aligned(&sketch, e));
    }

    /// The minor axis's end dragged past the major radius, the centre held
    /// and the foci not shown: the axes trade places (that line holds the
    /// major axis, the other runs the other way as the minor), the
    /// ellipse goes through the dragged end, and solving again changes
    /// nothing.
    #[test]
    fn a_minor_axis_dragged_past_the_major_becomes_the_major() {
        let mut sketch = Sketch::new("t");
        let e = fixed_ellipse(&mut sketch);
        show(&mut sketch, e);
        let foci = [
            role(&sketch, e, InternalRole::Focus1),
            role(&sketch, e, InternalRole::Focus2),
        ];
        sketch.remove_geometry_cascade(&foci);
        let minor = role(&sketch, e, InternalRole::MinorAxis);
        let Some(GeometryElement::Line(line)) = sketch.get_geometry(minor) else {
            panic!()
        };
        // The end above the centre at (1, 2).
        let end = [line.start, line.end]
            .into_iter()
            .max_by(|a, b| {
                let y = |id| sketch.point_position(id).unwrap().y;
                y(*a).total_cmp(&y(*b))
            })
            .unwrap();
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(end) {
            p.position = v(1.0, 10.0);
        }
        crate::solver::solve_holding(&mut sketch, &[end]);
        let shape = |sketch: &Sketch| {
            let el = ellipse_of(sketch, e);
            (el.major, el.ratio)
        };
        let (major, ratio) = shape(&sketch);
        assert!(
            major.x.abs() < 1e-3,
            "the major axis stands upright: {major:?}"
        );
        assert!((major.to_glam().length() - 8.0).abs() < 1e-3, "{major:?}");
        assert!((ratio - 5.0 / 8.0).abs() < 1e-3, "{ratio}");
        assert!(near(sketch.point_position(end).unwrap(), v(1.0, 10.0)));
        assert_eq!(role(&sketch, e, InternalRole::MajorAxis), minor);
        assert!(aligned(&sketch, e));
        crate::solver::solve_holding(&mut sketch, &[end]);
        let (again, ratio_again) = shape(&sketch);
        assert!(near(again, major) && (ratio_again - ratio).abs() < 1e-5);
    }

    /// The minor axis's end dragged past the major radius in small steps,
    /// as a drag goes, with the foci shown: the ellipse passes through a
    /// circle (the foci meeting at the centre) and comes out with the axes
    /// traded, the foci on the new major axis, the first on its vertex's
    /// side and the second opposite.
    #[test]
    fn an_ellipse_with_its_foci_drags_through_a_circle() {
        let mut sketch = Sketch::new("t");
        let e = fixed_ellipse(&mut sketch);
        show(&mut sketch, e);
        let minor = role(&sketch, e, InternalRole::MinorAxis);
        let Some(GeometryElement::Line(line)) = sketch.get_geometry(minor) else {
            panic!()
        };
        let end = line.end;
        assert!(near(sketch.point_position(end).unwrap(), v(1.0, 5.0)));
        // From 3 above the centre to 8, in quarter steps: through 5,
        // where the ellipse is a circle.
        for step in 1..=20 {
            let y = 5.0 + step as f64 * 0.25;
            if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(end) {
                p.position = v(1.0, y);
            }
            crate::solver::solve_holding(&mut sketch, &[end]);
            assert!(
                near(sketch.point_position(end).unwrap(), v(1.0, y)),
                "the end follows the drag at {y}"
            );
            assert!(aligned(&sketch, e), "everything sits on the ellipse at {y}");
        }
        let el = ellipse_of(&sketch, e);
        assert!(el.major.x.abs() < 1e-3, "upright: {:?}", el.major);
        assert!((el.major.to_glam().length() - 8.0).abs() < 1e-3);
        assert!((el.ratio - 5.0 / 8.0).abs() < 1e-3, "{}", el.ratio);
        // a = 8, b = 5: the foci sit √39 from the centre, along the axis.
        let f = 39f64.sqrt();
        let up = el.major.y.signum();
        let (f1, f2) = (
            sketch
                .point_position(role(&sketch, e, InternalRole::Focus1))
                .unwrap(),
            sketch
                .point_position(role(&sketch, e, InternalRole::Focus2))
                .unwrap(),
        );
        assert!(near(f1, v(1.0, 2.0 + up * f)), "{f1:?}");
        assert!(near(f2, v(1.0, 2.0 - up * f)), "{f2:?}");
    }

    /// A minor radius dimensioned past the major: the axes trade places
    /// and the dimensions name the axes they now hold.
    #[test]
    fn a_minor_radius_set_past_the_major_turns_the_dimensions_with_it() {
        let mut sketch = Sketch::new("t");
        let e = ellipse(&mut sketch);
        let long = sketch.add_constraint(ConstraintKind::EllipseRadius {
            ellipse: e,
            major: true,
            radius: 5.0,
        });
        let short = sketch.add_constraint(ConstraintKind::EllipseRadius {
            ellipse: e,
            major: false,
            radius: 8.0,
        });
        crate::solver::solve(&mut sketch);
        let el = ellipse_of(&sketch, e);
        assert!((el.major.to_glam().length() - 8.0).abs() < 1e-3);
        assert!((el.ratio - 5.0 / 8.0).abs() < 1e-3);
        let names = |sketch: &Sketch| {
            [long, short].map(|id| {
                match sketch.constraints.iter().find(|c| c.id == id).unwrap().kind {
                    ConstraintKind::EllipseRadius { major, radius, .. } => (major, radius),
                    _ => panic!(),
                }
            })
        };
        assert_eq!(names(&sketch), [(false, 5.0), (true, 8.0)]);
        crate::solver::solve(&mut sketch);
        assert_eq!(names(&sketch), [(false, 5.0), (true, 8.0)], "settled");
    }

    /// A focus dragged through the centre and out square to the axis, its
    /// partner held by a point it is joined to: the ellipse passes through
    /// a circle, the axes trade, and the two foci stay opposite each other
    /// all the way.
    #[test]
    fn a_focus_dragged_through_the_centre_turns_the_ellipse() {
        let mut sketch = Sketch::new("t");
        let e = fixed_ellipse(&mut sketch);
        show(&mut sketch, e);
        let (focus, other) = (
            role(&sketch, e, InternalRole::Focus1),
            role(&sketch, e, InternalRole::Focus2),
        );
        let joined = point(&mut sketch, -3.0, 2.0);
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: other,
            point2: joined,
        });
        // From (5, 2) to the centre, then up to (1, 5).
        let path = (1..=16)
            .map(|k| v(5.0 - k as f64 * 0.25, 2.0))
            .chain((1..=12).map(|k| v(1.0, 2.0 + k as f64 * 0.25)));
        for at in path {
            if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(focus) {
                p.position = at;
            }
            crate::solver::solve_holding(&mut sketch, &[focus]);
            assert!(near(sketch.point_position(focus).unwrap(), at), "{at:?}");
            let (f, g) = (
                sketch.point_position(focus).unwrap().to_glam(),
                sketch.point_position(other).unwrap().to_glam(),
            );
            assert!(
                ((f + g) * 0.5 - glam::DVec2::new(1.0, 2.0)).length() < 1e-3,
                "opposite through the centre at {at:?}: {f} {g}"
            );
            assert!(aligned(&sketch, e), "on the ellipse at {at:?}");
        }
        let el = ellipse_of(&sketch, e);
        assert!(el.major.x.abs() < 1e-3, "upright: {:?}", el.major);
        assert!(el.ratio < 1.0);
        assert!(near(sketch.point_position(joined).unwrap(), v(1.0, -1.0)));
        assert_eq!(role(&sketch, e, InternalRole::Focus1), focus);
        assert!(
            el.major.y > 0.0,
            "the first focus is on the major vertex's side"
        );
    }

    /// One focus put somewhere new with the centre free: the solver
    /// mirrors it through the centre for the other focus.
    #[test]
    fn a_dragged_focus_reshapes_the_ellipse_and_the_rest_follows() {
        let mut sketch = Sketch::new("t");
        let e = ellipse(&mut sketch);
        show(&mut sketch, e);
        let focus = role(&sketch, e, InternalRole::Focus1);
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(focus) {
            p.position = v(6.0, 2.0);
        }
        crate::solver::solve_holding(&mut sketch, &[focus]);
        assert!(near(sketch.point_position(focus).unwrap(), v(6.0, 2.0)));
        assert!(aligned(&sketch, e));
        let ratio = ellipse_of(&sketch, e).ratio;
        assert!(ratio <= 1.0 && ratio > 0.0);
    }

    #[test]
    fn a_hyperbola_takes_its_minor_axis_from_a_dimension() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let shape = Shape::new(ConicKind::Hyperbola, v(0.0, 0.0), v(2.0, 0.0), 1.0).unwrap();
        let at = |t: f64| {
            let [x, y] = shape.point(t);
            v(x, y)
        };
        let s = point(&mut sketch, at(-1.0).x, at(-1.0).y);
        let end = point(&mut sketch, at(1.0).x, at(1.0).y);
        let h = sketch.add_geometry(GeometryElement::Conic(Conic::new(
            ConicKind::Hyperbola,
            c,
            v(2.0, 0.0),
            1.0,
            s,
            end,
        )));
        show(&mut sketch, h);
        assert!(aligned(&sketch, h));
        let minor = role(&sketch, h, InternalRole::MinorAxis);
        sketch.add_constraint(ConstraintKind::Length {
            line: minor,
            length: 3.0,
        });
        crate::solver::solve(&mut sketch);
        let Some(GeometryElement::Conic(conic)) = sketch.get_geometry(h) else {
            panic!()
        };
        assert!((conic.minor - 1.5).abs() < 1e-3, "{}", conic.minor);
        assert!(aligned(&sketch, h));
        // Its ends are still on it.
        let shape = Shape::of(conic, &sketch).unwrap();
        for id in [conic.start, conic.end] {
            let p = sketch.point_position(id).unwrap();
            assert!(shape.distance([p.x, p.y]).abs() < 1e-3);
        }
    }

    #[test]
    fn the_toggle_shows_what_is_missing_then_hides() {
        let mut sketch = Sketch::new("t");
        let e = ellipse(&mut sketch);
        let (showed, made) = toggle(&mut sketch, &[e], None);
        assert!(showed && !made.is_empty());
        // A piece of it names the curve too.
        let focus = shown(&sketch, e)[2].1;
        let (showed, gone) = toggle(&mut sketch, &[focus], None);
        assert!(!showed);
        assert_eq!(gone.len(), 8);
    }
}
