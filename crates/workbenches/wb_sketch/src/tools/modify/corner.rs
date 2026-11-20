//! Fillet and chamfer: the corner under the cursor, the two curves that
//! meet there, and the arc or line put between them.

use super::*;

/// What a corner tool puts between two curves: an arc of a radius, or a
/// line set back a length from where they meet.
#[derive(Clone, Copy)]
pub(crate) enum CornerCut {
    Round(f64),
    Bevel(f64),
}

impl CornerCut {
    fn name(self) -> &'static str {
        match self {
            CornerCut::Round(_) => "Fillet",
            CornerCut::Bevel(_) => "Chamfer",
        }
    }
}

/// Which end of a line or an arc.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Start,
    End,
}

/// A point that ends exactly two lines or arcs.
struct Corner {
    point: Uuid,
    at: DVec2,
    curves: [Uuid; 2],
}

/// The corner point under the cursor, when exactly two lines or arcs end
/// there.
fn corner_under_cursor(sketch: &Sketch, cursor: Vec2D, snap_tol: f64) -> Option<Corner> {
    let snap::SnapTarget::Existing(point) = snap::snap_to_point(sketch, cursor, snap_tol, &[])
    else {
        return None;
    };
    let at = sketch.point_position(point)?.to_glam();
    let touching: Vec<Uuid> = sketch
        .geometry
        .iter()
        .filter(|g| chain_ends(g).is_some_and(|(s, e)| s == point || e == point))
        .map(|g| g.id())
        .collect();
    let [a, b] = touching.as_slice() else {
        return None;
    };
    Some(Corner {
        point,
        at,
        curves: [*a, *b],
    })
}

/// The line, arc or circle under the cursor that a corner tool may cut,
/// other than `except`. External geometry is the solid's and stays whole.
fn joinable_under_cursor(
    sketch: &Sketch,
    cursor: Vec2D,
    tol: f64,
    except: Option<Uuid>,
) -> Option<Uuid> {
    sketch
        .geometry
        .iter()
        .filter(|g| {
            matches!(
                g,
                GeometryElement::Line(_) | GeometryElement::Arc(_) | GeometryElement::Circle(_)
            ) && Some(g.id()) != except
                && !sketch.is_external(g.id())
        })
        .filter_map(|g| crate::shape::distance_to_element(sketch, g, cursor).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The curve's direction at `p` on it (a circle's counter-clockwise).
fn direction_at(prim: &Prim, p: DVec2) -> DVec2 {
    match *prim {
        Prim::Seg { a, b } => (b - a).normalize_or_zero(),
        Prim::Arc { c, .. } | Prim::Circle { c, .. } => (p - c).normalize_or_zero().perp(),
    }
}

/// The curves `radius` away from `prim`, on either side.
fn offsets(prim: &Prim, radius: f64) -> Vec<Prim> {
    match *prim {
        Prim::Seg { a, b } => {
            let n = (b - a).normalize_or_zero().perp() * radius;
            if n == DVec2::ZERO {
                return Vec::new();
            }
            vec![
                Prim::Seg { a: a + n, b: b + n },
                Prim::Seg { a: a - n, b: b - n },
            ]
        }
        Prim::Arc { c, r, .. } | Prim::Circle { c, r } => {
            let mut out = vec![Prim::Circle { c, r: r + radius }];
            if (r - radius).abs() > 1e-6 {
                out.push(Prim::Circle {
                    c,
                    r: (r - radius).abs(),
                });
            }
            out
        }
    }
}

/// Where a circle of `radius` centred at `centre` touches `prim`.
fn touch(prim: &Prim, centre: DVec2, radius: f64) -> DVec2 {
    match *prim {
        Prim::Seg { a, b } => {
            let d = b - a;
            a + d * ((centre - a).dot(d) / d.length_squared())
        }
        Prim::Arc { c, r, .. } | Prim::Circle { c, r } => {
            let dir = (centre - c).normalize_or_zero();
            let (near, far) = (c + dir * r, c - dir * r);
            let miss = |q: DVec2| ((q - centre).length() - radius).abs();
            if miss(near) <= miss(far) { near } else { far }
        }
    }
}

/// Every circle of `radius` touching both curves' carriers: its centre and
/// where it touches each.
fn fillet_circles(a: &Prim, b: &Prim, radius: f64) -> Vec<(DVec2, DVec2, DVec2)> {
    let mut out = Vec::new();
    for oa in offsets(a, radius) {
        for ob in offsets(b, radius) {
            for c in raw_hits(&oa, &ob) {
                out.push((c, touch(a, c, radius), touch(b, c, radius)));
            }
        }
    }
    out
}

/// Which side of the curve's carrier `p` is on: left of a line, outside a
/// circle.
fn side(prim: &Prim, p: DVec2) -> bool {
    match *prim {
        Prim::Seg { a, b } => (b - a).perp_dot(p - a) > 0.0,
        Prim::Arc { c, r, .. } | Prim::Circle { c, r } => (p - c).length() > r,
    }
}

/// The end of a line or an arc that moves to `to` (on its carrier) so the
/// part holding `pick` stays: trimmed back when `to` is on it, extended
/// when beyond it. `None` for a circle, which stays whole.
fn end_toward(prim: &Prim, pick: DVec2, to: DVec2) -> Option<End> {
    match *prim {
        Prim::Seg { a, b } => {
            let ab = b - a;
            let t = |q: DVec2| (q - a).dot(ab);
            Some(if t(to) > t(pick) {
                End::End
            } else {
                End::Start
            })
        }
        Prim::Arc { c, s, e, .. } => {
            let (start, sweep) = arc_angles(s - c, e - c);
            let rel = |q: DVec2| geom2d::wrap_positive((q - c).y.atan2((q - c).x) - start);
            let (at, picked) = (rel(to), rel(pick).min(sweep));
            Some(if at <= sweep {
                if at > picked { End::End } else { End::Start }
            } else if at - sweep < std::f64::consts::TAU - at {
                End::End
            } else {
                End::Start
            })
        }
        Prim::Circle { .. } => None,
    }
}

/// Point `id`'s `end` at `point`.
fn set_end(sketch: &mut Sketch, id: Uuid, end: End, point: Uuid) {
    match sketch.get_geometry_mut(id) {
        Some(GeometryElement::Line(l)) => match end {
            End::Start => l.start = point,
            End::End => l.end = point,
        },
        Some(GeometryElement::Arc(a)) => match end {
            End::Start => a.start = point,
            End::End => a.end = point,
        },
        _ => {}
    }
}

/// The end of `id` that is `point`.
fn end_at(sketch: &Sketch, id: Uuid, point: Uuid) -> Option<End> {
    let (s, e) = chain_ends(sketch.get_geometry(id)?)?;
    if s == point {
        Some(End::Start)
    } else if e == point {
        Some(End::End)
    } else {
        None
    }
}

/// Hold `point` on each curve's carrier: the corner a cut keeps as
/// construction geometry.
fn hold_on(sketch: &mut Sketch, point: Uuid, curves: [Uuid; 2]) {
    for curve in curves {
        let kind = match sketch.get_geometry(curve) {
            Some(GeometryElement::Line(_)) => ConstraintKind::PointOnLine { point, line: curve },
            Some(GeometryElement::Arc(_) | GeometryElement::Circle(_)) => {
                ConstraintKind::PointOnCircle {
                    point,
                    circle: curve,
                }
            }
            _ => continue,
        };
        sketch.add_constraint(kind);
    }
    sketch.set_construction(point, true);
}

/// A line or arc an edit made longer or shorter: a dimension of its length
/// or sweep (or an equality of it with another) would pull it back, so it
/// goes.
pub(super) fn curve_resized(sketch: &mut Sketch, curve: Uuid) {
    sketch.constraints.retain(|c| match c.kind {
        ConstraintKind::Length { line, .. } => line != curve,
        ConstraintKind::EqualLength { line1, line2 } => line1 != curve && line2 != curve,
        ConstraintKind::ArcLength { arc, .. } | ConstraintKind::ArcAngle { arc, .. } => {
            arc != curve
        }
        ConstraintKind::CurveLength { curve: c, .. } => c != curve,
        _ => true,
    });
}

/// `second` is the rest of `first`, cut from it, held on the same line.
/// `joined` when they share an end (a split): held parallel, or level or
/// plumb as `first` is when it is, the shared end doing the rest. Else
/// (the far part of a trimmed middle) held collinear with `first`, which
/// keeps it level or plumb with it too.
pub(super) fn continues(sketch: &mut Sketch, first: Uuid, second: Uuid, joined: bool) {
    if !joined {
        sketch.add_constraint(ConstraintKind::Collinear {
            line1: first,
            line2: second,
        });
        return;
    }
    let axis: Vec<ConstraintKind> = sketch
        .constraints
        .iter()
        .filter_map(|c| match c.kind {
            ConstraintKind::Horizontal { element } if element == first => {
                Some(ConstraintKind::Horizontal { element: second })
            }
            ConstraintKind::Vertical { element } if element == first => {
                Some(ConstraintKind::Vertical { element: second })
            }
            _ => None,
        })
        .collect();
    if axis.is_empty() {
        sketch.add_constraint(ConstraintKind::Parallel {
            line1: second,
            line2: first,
        });
    } else {
        for kind in axis {
            sketch.add_constraint(kind);
        }
    }
}

/// The shorter arc of `radius` about `centre` from `t1` to `t2`, made
/// tangent to both curves.
fn add_fillet_arc(
    sketch: &mut Sketch,
    centre: DVec2,
    radius: f64,
    (t1, t2): (Uuid, Uuid),
    curves: [Uuid; 2],
) {
    let (Some(p1), Some(p2)) = (sketch.point_position(t1), sketch.point_position(t2)) else {
        return;
    };
    let centre_id =
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(centre))));
    let (w1, w2) = (p1.to_glam() - centre, p2.to_glam() - centre);
    let (start, end) = if w1.perp_dot(w2) > 0.0 {
        (t1, t2)
    } else {
        (t2, t1)
    };
    let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(
        centre_id, start, end, radius,
    )));
    // Tangent to both curves, so it stays a fillet when they move.
    for curve in curves {
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: curve,
            item2: arc,
        });
    }
}

/// A fillet or chamfer tool's click. On a point where two lines or arcs
/// end, the corner is cut there. On a curve, the curve is remembered and
/// the next click names the second one: the two need not meet, and each is
/// trimmed or extended to the cut, keeping the part that was clicked (a
/// circle stays whole). With `keep_corner`, the corner stays as a
/// construction point on both curves, keeping what holds it.
pub(crate) fn corner(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f64,
    cut: CornerCut,
    keep_corner: bool,
) -> ToolEffect {
    let size = match cut {
        CornerCut::Round(r) => r,
        CornerCut::Bevel(l) => l,
    };
    // `!(>=)` so a size that is not a number is refused too.
    if !(size >= 1e-6 && size.is_finite()) {
        *state = ToolState::Idle;
        return ToolEffect::log(match cut {
            CornerCut::Round(_) => "Set a fillet radius first",
            CornerCut::Bevel(_) => "Set a chamfer length first",
        });
    }
    if let ToolState::CornerFirst { curve, pick, .. } = *state {
        let Some(second) = joinable_under_cursor(sketch, cursor, snap_tol, Some(curve)) else {
            return ToolEffect::log("Click the second curve");
        };
        *state = ToolState::Idle;
        return between(
            sketch,
            [(curve, pick.to_glam()), (second, cursor.to_glam())],
            cut,
            keep_corner,
        );
    }
    if let Some(corner) = corner_under_cursor(sketch, cursor, snap_tol) {
        return at_corner(sketch, &corner, cut, keep_corner);
    }
    if let Some(curve) = joinable_under_cursor(sketch, cursor, snap_tol, None) {
        *state = ToolState::CornerFirst {
            curve,
            pick: cursor,
            bevel: matches!(cut, CornerCut::Bevel(_)),
        };
        return ToolEffect::log(format!("{}: click the second curve", cut.name()));
    }
    ToolEffect::log("Click a corner where two curves meet, or a curve")
}

/// The two curves a corner tool's click at `cursor` would join, whatever
/// the size: the second curve after a first was picked, else the two that
/// end at the corner point under the cursor. `None` for a click that only
/// picks a first curve, or picks nothing.
pub(crate) fn corner_operands(
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    snap_tol: f64,
) -> Option<[Uuid; 2]> {
    if let ToolState::CornerFirst { curve, .. } = *state {
        return joinable_under_cursor(sketch, cursor, snap_tol, Some(curve))
            .map(|second| [curve, second]);
    }
    corner_under_cursor(sketch, cursor, snap_tol).map(|corner| corner.curves)
}

/// Round the corner `point` where `curves` meet with an arc of `radius`,
/// as a fillet click on it does. Whether it was cut.
pub(crate) fn round_corner(
    sketch: &mut Sketch,
    point: Uuid,
    curves: [Uuid; 2],
    radius: f64,
) -> bool {
    let Some(at) = sketch.point_position(point) else {
        return false;
    };
    let corner = Corner {
        point,
        at: at.to_glam(),
        curves,
    };
    at_corner(sketch, &corner, CornerCut::Round(radius), false).changed
}

/// Cut the corner two lines or arcs share.
fn at_corner(sketch: &mut Sketch, corner: &Corner, cut: CornerCut, keep: bool) -> ToolEffect {
    let prim = |id: Uuid| sketch.get_geometry(id).and_then(|g| prim_of(sketch, g));
    let (Some(a), Some(b)) = (prim(corner.curves[0]), prim(corner.curves[1])) else {
        return ToolEffect::none();
    };
    let q = corner.at;
    if direction_at(&a, q).perp_dot(direction_at(&b, q)).abs() < 1e-4 {
        return ToolEffect::log("The curves run on smoothly there: no corner to cut");
    }
    let away = |p: DVec2| (p - q).length() > 1e-5;
    let (t1, t2, centre) = match cut {
        CornerCut::Round(radius) => {
            let Some((c, t1, t2)) = fillet_circles(&a, &b, radius)
                .into_iter()
                .filter(|(_, t1, t2)| within(&a, *t1) && within(&b, *t2) && away(*t1) && away(*t2))
                .min_by(|x, y| {
                    let d = |t: &(DVec2, DVec2, DVec2)| (t.1 - q).length() + (t.2 - q).length();
                    d(x).total_cmp(&d(y))
                })
            else {
                return ToolEffect::log("The radius is too large for these curves");
            };
            (t1, t2, Some((c, radius)))
        }
        CornerCut::Bevel(length) => {
            let setback = |p: &Prim| {
                raw_hits(p, &Prim::Circle { c: q, r: length })
                    .into_iter()
                    .find(|t| within(p, *t) && away(*t))
            };
            let (Some(t1), Some(t2)) = (setback(&a), setback(&b)) else {
                return ToolEffect::log("The chamfer is longer than these curves");
            };
            (t1, t2, None)
        }
    };
    let new_point = |sketch: &mut Sketch, p: DVec2| {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(p))))
    };
    let ids = (new_point(sketch, t1), new_point(sketch, t2));
    for (curve, point) in [(corner.curves[0], ids.0), (corner.curves[1], ids.1)] {
        let Some(end) = end_at(sketch, curve, corner.point) else {
            continue;
        };
        curve_resized(sketch, curve);
        set_end(sketch, curve, end, point);
    }
    if keep {
        hold_on(sketch, corner.point, corner.curves);
    } else {
        sketch.geometry.retain(|g| g.id() != corner.point);
        sketch
            .constraints
            .retain(|c| !crate::sketch::constraint_refs(&c.kind).contains(&corner.point));
        sketch.construction.remove(&corner.point);
    }
    finish(sketch, cut, ids, centre, corner.curves)
}

/// Cut between two curves picked at `picks`, which need not meet.
fn between(
    sketch: &mut Sketch,
    picks: [(Uuid, DVec2); 2],
    cut: CornerCut,
    keep: bool,
) -> ToolEffect {
    let prim = |id: Uuid| sketch.get_geometry(id).and_then(|g| prim_of(sketch, g));
    let [(ida, pa), (idb, pb)] = picks;
    let (Some(a), Some(b)) = (prim(ida), prim(idb)) else {
        return ToolEffect::none();
    };
    // The picks as they sit on each curve.
    let (pa, pb) = (touch(&a, pa, 0.0), touch(&b, pb, 0.0));
    let meet = raw_hits(&a, &b).into_iter().min_by(|x, y| {
        let d = |p: &DVec2| (*p - pa).length() + (*p - pb).length();
        d(x).total_cmp(&d(y))
    });
    let (t1, t2, centre) = match cut {
        CornerCut::Round(radius) => {
            let circles = fillet_circles(&a, &b, radius);
            let facing: Vec<_> = circles
                .iter()
                .copied()
                .filter(|(c, _, _)| side(&a, *c) == side(&a, pb) && side(&b, *c) == side(&b, pa))
                .collect();
            let pool = if facing.is_empty() { circles } else { facing };
            let Some((c, t1, t2)) = pool.into_iter().min_by(|x, y| {
                let d = |t: &(DVec2, DVec2, DVec2)| (t.1 - pa).length() + (t.2 - pb).length();
                d(x).total_cmp(&d(y))
            }) else {
                return ToolEffect::log("No fillet of that radius touches both curves");
            };
            (t1, t2, Some((c, radius)))
        }
        CornerCut::Bevel(length) => {
            let Some(q) = meet else {
                return ToolEffect::log("These curves never meet: nothing to chamfer");
            };
            let setback = |p: &Prim, pick: DVec2| {
                raw_hits(p, &Prim::Circle { c: q, r: length })
                    .into_iter()
                    .min_by(|x, y| (*x - pick).length().total_cmp(&(*y - pick).length()))
            };
            let (Some(t1), Some(t2)) = (setback(&a, pa), setback(&b, pb)) else {
                return ToolEffect::none();
            };
            (t1, t2, None)
        }
    };
    let new_point = |sketch: &mut Sketch, p: DVec2| {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(p))))
    };
    let ids = (new_point(sketch, t1), new_point(sketch, t2));
    for (id, prim, pick, point, at) in [(ida, &a, pa, ids.0, t1), (idb, &b, pb, ids.1, t2)] {
        let Some(end) = end_toward(prim, pick, at) else {
            continue;
        };
        let old = chain_ends(sketch.get_geometry(id).expect("a picked curve"))
            .map(|(s, e)| if end == End::Start { s } else { e });
        curve_resized(sketch, id);
        set_end(sketch, id, end, point);
        if let Some(old) = old {
            drop_if_orphan(sketch, old);
        }
    }
    if keep && let Some(q) = meet {
        let corner = new_point(sketch, q);
        hold_on(sketch, corner, [ida, idb]);
    }
    finish(sketch, cut, ids, centre, [ida, idb])
}

/// Bridge the cut's two new ends with its arc or line.
fn finish(
    sketch: &mut Sketch,
    cut: CornerCut,
    ends: (Uuid, Uuid),
    centre: Option<(DVec2, f64)>,
    curves: [Uuid; 2],
) -> ToolEffect {
    match (cut, centre) {
        (CornerCut::Round(radius), Some((c, _))) => {
            add_fillet_arc(sketch, c, radius, ends, curves);
            ToolEffect::changed(format!("Fillet r={radius:.2}"))
        }
        (CornerCut::Bevel(length), _) => {
            sketch.add_geometry(GeometryElement::Line(Line::new(ends.0, ends.1)));
            ToolEffect::changed(format!("Chamfer {length:.2}"))
        }
        _ => ToolEffect::none(),
    }
}
