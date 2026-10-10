//! Editing tools that operate on existing geometry: fillet and chamfer,
//! trim, extend, split and offset.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use uuid::Uuid;

use super::{ToolEffect, ToolState};
use crate::curves::Curve;
use crate::geom2d::{self, Prim, prim_of, raw_hits, within};
use crate::sketch::{Arc, ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::snap::{self, arc_angles};

/// Relative slack in curve parameter space: intersections this close to a
/// span end don't count as cut points (shared endpoints intersect exactly
/// at the ends).
const SPAN_EPS: f32 = 1e-3;

/// What a corner tool puts between two curves: an arc of a radius, or a
/// line set back a length from where they meet.
#[derive(Clone, Copy)]
pub(super) enum CornerCut {
    Round(f32),
    Bevel(f32),
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
    at: Vec2,
    curves: [Uuid; 2],
}

/// The corner point under the cursor, when exactly two lines or arcs end
/// there.
fn corner_under_cursor(sketch: &Sketch, cursor: Vec2D, snap_tol: f32) -> Option<Corner> {
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
    tol: f32,
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
        .filter_map(|g| snap::distance_to_element(sketch, g, cursor).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The curve's direction at `p` on it (a circle's counter-clockwise).
fn direction_at(prim: &Prim, p: Vec2) -> Vec2 {
    match *prim {
        Prim::Seg { a, b } => (b - a).normalize_or_zero(),
        Prim::Arc { c, .. } | Prim::Circle { c, .. } => (p - c).normalize_or_zero().perp(),
    }
}

/// The curves `radius` away from `prim`, on either side.
fn offsets(prim: &Prim, radius: f32) -> Vec<Prim> {
    match *prim {
        Prim::Seg { a, b } => {
            let n = (b - a).normalize_or_zero().perp() * radius;
            if n == Vec2::ZERO {
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
fn touch(prim: &Prim, centre: Vec2, radius: f32) -> Vec2 {
    match *prim {
        Prim::Seg { a, b } => {
            let d = b - a;
            a + d * ((centre - a).dot(d) / d.length_squared())
        }
        Prim::Arc { c, r, .. } | Prim::Circle { c, r } => {
            let dir = (centre - c).normalize_or_zero();
            let (near, far) = (c + dir * r, c - dir * r);
            let miss = |q: Vec2| ((q - centre).length() - radius).abs();
            if miss(near) <= miss(far) { near } else { far }
        }
    }
}

/// Every circle of `radius` touching both curves' carriers: its centre and
/// where it touches each.
fn fillet_circles(a: &Prim, b: &Prim, radius: f32) -> Vec<(Vec2, Vec2, Vec2)> {
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
fn side(prim: &Prim, p: Vec2) -> bool {
    match *prim {
        Prim::Seg { a, b } => (b - a).perp_dot(p - a) > 0.0,
        Prim::Arc { c, r, .. } | Prim::Circle { c, r } => (p - c).length() > r,
    }
}

/// The end of a line or an arc that moves to `to` (on its carrier) so the
/// part holding `pick` stays: trimmed back when `to` is on it, extended
/// when beyond it. `None` for a circle, which stays whole.
fn end_toward(prim: &Prim, pick: Vec2, to: Vec2) -> Option<End> {
    match *prim {
        Prim::Seg { a, b } => {
            let ab = b - a;
            let t = |q: Vec2| (q - a).dot(ab);
            Some(if t(to) > t(pick) {
                End::End
            } else {
                End::Start
            })
        }
        Prim::Arc { c, s, e, .. } => {
            let (start, sweep) = arc_angles(s - c, e - c);
            let rel = |q: Vec2| geom2d::wrap_positive((q - c).y.atan2((q - c).x) - start);
            let (at, picked) = (rel(to), rel(pick).min(sweep));
            Some(if at <= sweep {
                if at > picked { End::End } else { End::Start }
            } else if at - sweep < std::f32::consts::TAU - at {
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
fn curve_resized(sketch: &mut Sketch, curve: Uuid) {
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

/// `second` is the rest of `first`, cut from it: held on the same line, as
/// level or plumb as `first` is when it is. `joined` when they share an
/// end (a split), else the second is held on the first's line by its start
/// (the far part of a trimmed middle).
fn continues(sketch: &mut Sketch, first: Uuid, second: Uuid, joined: bool) {
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
    if !joined && let Some(GeometryElement::Line(l)) = sketch.get_geometry(second) {
        let start = l.start;
        sketch.add_constraint(ConstraintKind::PointOnLine {
            point: start,
            line: first,
        });
    }
}

/// The shorter arc of `radius` about `centre` from `t1` to `t2`, made
/// tangent to both curves.
fn add_fillet_arc(
    sketch: &mut Sketch,
    centre: Vec2,
    radius: f32,
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
pub(super) fn corner(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    cut: CornerCut,
    keep_corner: bool,
) -> ToolEffect {
    let size = match cut {
        CornerCut::Round(r) => r,
        CornerCut::Bevel(l) => l,
    };
    if size < 1e-6 {
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
    let away = |p: Vec2| (p - q).length() > 1e-5;
    let (t1, t2, centre) = match cut {
        CornerCut::Round(radius) => {
            let Some((c, t1, t2)) = fillet_circles(&a, &b, radius)
                .into_iter()
                .filter(|(_, t1, t2)| within(&a, *t1) && within(&b, *t2) && away(*t1) && away(*t2))
                .min_by(|x, y| {
                    let d = |t: &(Vec2, Vec2, Vec2)| (t.1 - q).length() + (t.2 - q).length();
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
    let new_point = |sketch: &mut Sketch, p: Vec2| {
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
    picks: [(Uuid, Vec2); 2],
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
        let d = |p: &Vec2| (*p - pa).length() + (*p - pb).length();
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
                let d = |t: &(Vec2, Vec2, Vec2)| (t.1 - pa).length() + (t.2 - pb).length();
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
            let setback = |p: &Prim, pick: Vec2| {
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
    let new_point = |sketch: &mut Sketch, p: Vec2| {
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
    centre: Option<(Vec2, f32)>,
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

/// Intersections of `target` with every other curve in the sketch.
/// `bounded_target` restricts hits to the target's own extent (trim);
/// extend wants the unbounded carrier.
fn hits_with_others(
    sketch: &Sketch,
    target_id: Uuid,
    target: &Prim,
    bounded_target: bool,
) -> Vec<Vec2> {
    let mut out = Vec::new();
    for geom in &sketch.geometry {
        if geom.id() == target_id {
            continue;
        }
        let Some(other) = prim_of(sketch, geom) else {
            continue;
        };
        out.extend(
            raw_hits(target, &other)
                .into_iter()
                .filter(|p| within(&other, *p) && (!bounded_target || within(target, *p))),
        );
    }
    out.extend(general_points(sketch, target_id, bounded_target));
    out
}

/// Nearest line or arc within `tol` of `pos`, circles too with
/// `include_circles` (points and other element kinds excluded).
fn curve_under_cursor(
    sketch: &Sketch,
    pos: Vec2D,
    tol: f32,
    include_circles: bool,
) -> Option<Uuid> {
    sketch
        .geometry
        .iter()
        .filter(|g| {
            matches!(g, GeometryElement::Line(_) | GeometryElement::Arc(_))
                || (include_circles && matches!(g, GeometryElement::Circle(_)))
        })
        .filter_map(|g| snap::distance_to_element(sketch, g, pos).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Remove `point_id` (and constraints referencing it) when no remaining
/// curve uses it: the end a cut left behind.
fn drop_if_orphan(sketch: &mut Sketch, point_id: Uuid) {
    let referenced = sketch
        .geometry
        .iter()
        .any(|g| Sketch::curve_point_ids(g).contains(&point_id));
    if !referenced
        && matches!(
            sketch.get_geometry(point_id),
            Some(GeometryElement::Point(_))
        )
    {
        sketch.remove_geometry_cascade(&[point_id]);
    }
}

/// The span a trim click would remove.
enum TrimPlan {
    /// No intersections: the whole element goes.
    RemoveWhole { id: Uuid },
    /// Removed line span in curve parameters (None = up to that end).
    LineSpan {
        id: Uuid,
        lo: Option<f32>,
        hi: Option<f32>,
    },
    /// Removed arc span as CCW angles relative to the arc start.
    ArcSpan {
        id: Uuid,
        lo: Option<f32>,
        hi: Option<f32>,
    },
    /// Removed circle span between two absolute angles (CCW lo → hi).
    CircleSpan { id: Uuid, lo: f32, hi: f32 },
}

fn plan_trim(sketch: &Sketch, cursor: Vec2D, tol: f32) -> Option<TrimPlan> {
    let id = curve_under_cursor(sketch, cursor, tol, true)?;
    let prim = prim_of(sketch, sketch.get_geometry(id)?)?;
    let hits = hits_with_others(sketch, id, &prim, true);
    let p = cursor.to_glam();
    match prim {
        Prim::Seg { a, b } => {
            let ab = b - a;
            let len_sq = ab.length_squared();
            let t_of = |q: Vec2| (q - a).dot(ab) / len_sq;
            let ts: Vec<f32> = hits
                .iter()
                .map(|q| t_of(*q))
                .filter(|t| (SPAN_EPS..=1.0 - SPAN_EPS).contains(t))
                .collect();
            let tc = t_of(p).clamp(0.0, 1.0);
            let lo = ts.iter().copied().filter(|t| *t < tc).reduce(f32::max);
            let hi = ts.iter().copied().filter(|t| *t > tc).reduce(f32::min);
            if lo.is_none() && hi.is_none() {
                Some(TrimPlan::RemoveWhole { id })
            } else {
                Some(TrimPlan::LineSpan { id, lo, hi })
            }
        }
        Prim::Arc { c, s, e, .. } => {
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let eps = (sweep * SPAN_EPS).max(1e-4);
            let rel_of = |q: Vec2| geom2d::wrap_positive((q - c).y.atan2((q - c).x) - start_angle);
            let rels: Vec<f32> = hits
                .iter()
                .map(|q| rel_of(*q))
                .filter(|r| (eps..=sweep - eps).contains(r))
                .collect();
            let rc = rel_of(p).min(sweep);
            let lo = rels.iter().copied().filter(|r| *r < rc).reduce(f32::max);
            let hi = rels.iter().copied().filter(|r| *r > rc).reduce(f32::min);
            if lo.is_none() && hi.is_none() {
                Some(TrimPlan::RemoveWhole { id })
            } else {
                Some(TrimPlan::ArcSpan { id, lo, hi })
            }
        }
        Prim::Circle { c, .. } => {
            let click_ang = (p - c).y.atan2((p - c).x);
            // Neighbors of the click going CW (lo) and CCW (hi).
            let mut lo: Option<(f32, Vec2)> = None; // max rel
            let mut hi: Option<(f32, Vec2)> = None; // min rel
            for q in &hits {
                let ang = (*q - c).y.atan2((*q - c).x);
                let rel = geom2d::wrap_positive(ang - click_ang);
                if !(1e-4..=std::f32::consts::TAU - 1e-4).contains(&rel) {
                    continue;
                }
                if hi.map(|(r, _)| rel < r).unwrap_or(true) {
                    hi = Some((rel, *q));
                }
                if lo.map(|(r, _)| rel > r).unwrap_or(true) {
                    lo = Some((rel, *q));
                }
            }
            match (lo, hi) {
                (Some((_, lo_p)), Some((_, hi_p))) if (lo_p - hi_p).length() > 1e-4 => {
                    let ang = |q: Vec2| (q - c).y.atan2((q - c).x);
                    Some(TrimPlan::CircleSpan {
                        id,
                        lo: ang(lo_p),
                        hi: ang(hi_p),
                    })
                }
                _ => Some(TrimPlan::RemoveWhole { id }),
            }
        }
    }
}

/// The first place, going from `from` to `to`, where the pointer's path
/// crosses a curve the trim tool cuts (a line, arc or circle), not counting
/// `from` itself: where a trim stroke trims next. External geometry is the
/// solid's, and is never crossed.
pub fn next_stroke_crossing(sketch: &Sketch, from: Vec2D, to: Vec2D) -> Option<Vec2D> {
    let (a, b) = (from.to_glam(), to.to_glam());
    let path = b - a;
    let len_sq = path.length_squared();
    if len_sq < 1e-12 {
        return None;
    }
    let stroke = Prim::Seg { a, b };
    let external = sketch.external_ids();
    // A crossing this close to the start is the one just trimmed.
    let skip = 1e-4 / len_sq.sqrt();
    sketch
        .geometry
        .iter()
        .filter(|g| {
            matches!(
                g,
                GeometryElement::Line(_) | GeometryElement::Arc(_) | GeometryElement::Circle(_)
            ) && !external.contains(&g.id())
        })
        .filter_map(|g| prim_of(sketch, g))
        .flat_map(|prim| {
            raw_hits(&stroke, &prim)
                .into_iter()
                .filter(move |p| geom2d::on_segment(a, b, *p) && within(&prim, *p))
        })
        .chain(general_stroke_crossings(sketch, a, b))
        .map(|p| ((p - a).dot(path) / len_sq, p))
        .filter(|(t, _)| *t > skip)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, p)| Vec2D::from_glam(p))
}

/// Where the stroke `a → b` crosses the ellipses, conics and splines.
fn general_stroke_crossings(sketch: &Sketch, a: Vec2, b: Vec2) -> Vec<Vec2> {
    let stroke = Curve::segment(a.as_dvec2(), b.as_dvec2());
    let external = sketch.external_ids();
    sketch
        .geometry
        .iter()
        .filter(|g| !external.contains(&g.id()) && prim_of(sketch, g).is_none())
        .filter_map(|g| Curve::of(sketch, g))
        .flat_map(|curve| {
            crate::curves::crossings(&stroke, stroke.span, &curve, curve.span)
                .into_iter()
                .map(|(t, _)| stroke.at(t).as_vec2())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Highlight polyline for the span a trim click at `cursor` would remove
/// (overlay hover preview). `None` when nothing is trimmable there.
pub fn trim_preview(sketch: &Sketch, cursor: Vec2D, tol: f32) -> Option<Vec<Vec2D>> {
    const SAMPLES: usize = 24;
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return Some(general_preview(sketch, id, &curve, cursor));
    }
    let plan = plan_trim(sketch, cursor, tol)?;
    let sample_arc = |c: Vec2, r: f32, from: f32, sweep: f32| -> Vec<Vec2D> {
        (0..=SAMPLES)
            .map(|i| {
                let a = from + sweep * (i as f32 / SAMPLES as f32);
                Vec2D::new(c.x + r * a.cos(), c.y + r * a.sin())
            })
            .collect()
    };
    match plan {
        TrimPlan::RemoveWhole { id } => match prim_of(sketch, sketch.get_geometry(id)?)? {
            Prim::Seg { a, b } => Some(vec![Vec2D::from_glam(a), Vec2D::from_glam(b)]),
            Prim::Arc { c, r, s, e } => {
                let (start_angle, sweep) = arc_angles(s - c, e - c);
                Some(sample_arc(c, r, start_angle, sweep))
            }
            Prim::Circle { c, r } => Some(sample_arc(c, r, 0.0, std::f32::consts::TAU)),
        },
        TrimPlan::LineSpan { id, lo, hi } => {
            let Prim::Seg { a, b } = prim_of(sketch, sketch.get_geometry(id)?)? else {
                return None;
            };
            let pos = |t: f32| Vec2D::from_glam(a + (b - a) * t);
            Some(vec![pos(lo.unwrap_or(0.0)), pos(hi.unwrap_or(1.0))])
        }
        TrimPlan::ArcSpan { id, lo, hi } => {
            let Prim::Arc { c, r, s, e } = prim_of(sketch, sketch.get_geometry(id)?)? else {
                return None;
            };
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let (from, to) = (lo.unwrap_or(0.0), hi.unwrap_or(sweep));
            Some(sample_arc(c, r, start_angle + from, to - from))
        }
        TrimPlan::CircleSpan { id, lo, hi } => {
            let Prim::Circle { c, r } = prim_of(sketch, sketch.get_geometry(id)?)? else {
                return None;
            };
            Some(sample_arc(c, r, lo, geom2d::wrap_positive(hi - lo)))
        }
    }
}

pub(super) fn trim(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> ToolEffect {
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return trim_general(sketch, id, &curve, cursor);
    }
    let Some(plan) = plan_trim(sketch, cursor, tol) else {
        return ToolEffect::log("Nothing to trim here: click a curve's part to remove");
    };
    let new_point = |sketch: &mut Sketch, p: Vec2D| -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(p)))
    };
    match plan {
        TrimPlan::RemoveWhole { id } => {
            let pts = sketch
                .get_geometry(id)
                .map(Sketch::curve_point_ids)
                .unwrap_or_default();
            sketch.remove_geometry_cascade(&[id]);
            for p in pts {
                drop_if_orphan(sketch, p);
            }
            ToolEffect::changed("Trimmed away whole element")
        }
        TrimPlan::LineSpan { id, lo, hi } => {
            let Some(GeometryElement::Line(l)) = sketch.get_geometry(id) else {
                return ToolEffect::none();
            };
            let (start_pid, end_pid) = (l.start, l.end);
            let (Some(a), Some(b)) = (
                sketch.point_position(start_pid),
                sketch.point_position(end_pid),
            ) else {
                return ToolEffect::none();
            };
            let pos = |t: f32| Vec2D::from_glam(a.to_glam() + (b.to_glam() - a.to_glam()) * t);
            match (lo, hi) {
                // Middle span: the line splits in two.
                (Some(lo), Some(hi)) => {
                    let p_lo = new_point(sketch, pos(lo));
                    let p_hi = new_point(sketch, pos(hi));
                    curve_resized(sketch, id);
                    if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) {
                        l.end = p_lo;
                    }
                    let rest = sketch.add_geometry(GeometryElement::Line(Line::new(p_hi, end_pid)));
                    continues(sketch, id, rest, false);
                }
                // End-of-line span: shorten and clean up the freed endpoint.
                (Some(lo), None) => {
                    let p_lo = new_point(sketch, pos(lo));
                    curve_resized(sketch, id);
                    if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) {
                        l.end = p_lo;
                    }
                    drop_if_orphan(sketch, end_pid);
                }
                (None, Some(hi)) => {
                    let p_hi = new_point(sketch, pos(hi));
                    curve_resized(sketch, id);
                    if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) {
                        l.start = p_hi;
                    }
                    drop_if_orphan(sketch, start_pid);
                }
                (None, None) => return ToolEffect::none(), // plan never yields this
            }
            ToolEffect::changed("Trimmed line span")
        }
        TrimPlan::ArcSpan { id, lo, hi } => {
            let Some(GeometryElement::Arc(arc)) = sketch.get_geometry(id) else {
                return ToolEffect::none();
            };
            let (center_pid, start_pid, end_pid, radius) =
                (arc.center, arc.start, arc.end, arc.radius);
            let (Some(c), Some(s)) = (
                sketch.point_position(center_pid),
                sketch.point_position(start_pid),
            ) else {
                return ToolEffect::none();
            };
            let sv = (s - c).to_glam();
            let start_angle = sv.y.atan2(sv.x);
            let r = sv.length();
            let pos = |rel: f32| {
                let a = start_angle + rel;
                Vec2D::new(c.x + r * a.cos(), c.y + r * a.sin())
            };
            match (lo, hi) {
                (Some(lo), Some(hi)) => {
                    let p_lo = new_point(sketch, pos(lo));
                    let p_hi = new_point(sketch, pos(hi));
                    if let Some(GeometryElement::Arc(a)) = sketch.get_geometry_mut(id) {
                        a.end = p_lo;
                    }
                    sketch.add_geometry(GeometryElement::Arc(Arc::new(
                        center_pid, p_hi, end_pid, radius,
                    )));
                }
                (Some(lo), None) => {
                    let p_lo = new_point(sketch, pos(lo));
                    if let Some(GeometryElement::Arc(a)) = sketch.get_geometry_mut(id) {
                        a.end = p_lo;
                    }
                    drop_if_orphan(sketch, end_pid);
                }
                (None, Some(hi)) => {
                    let p_hi = new_point(sketch, pos(hi));
                    if let Some(GeometryElement::Arc(a)) = sketch.get_geometry_mut(id) {
                        a.start = p_hi;
                    }
                    drop_if_orphan(sketch, start_pid);
                }
                (None, None) => return ToolEffect::none(),
            }
            ToolEffect::changed("Trimmed arc span")
        }
        TrimPlan::CircleSpan { id, lo, hi } => {
            let Some(GeometryElement::Circle(circle)) = sketch.get_geometry(id) else {
                return ToolEffect::none();
            };
            let (center_pid, radius) = (circle.center, circle.radius);
            let Some(c) = sketch.point_position(center_pid) else {
                return ToolEffect::none();
            };
            let pos = |a: f32| Vec2D::new(c.x + radius * a.cos(), c.y + radius * a.sin());
            // The kept portion runs CCW from hi back around to lo. The arc
            // keeps the circle's id so radius/tangent constraints survive.
            let p_hi = new_point(sketch, pos(hi));
            let p_lo = new_point(sketch, pos(lo));
            if let Some(slot) = sketch.geometry.iter_mut().find(|g| g.id() == id) {
                *slot = GeometryElement::Arc(Arc {
                    id,
                    center: center_pid,
                    start: p_hi,
                    end: p_lo,
                    radius,
                });
            }
            ToolEffect::changed("Trimmed circle to arc")
        }
    }
}

pub(super) fn extend(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> ToolEffect {
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return extend_general(sketch, id, &curve, cursor);
    }
    let Some(id) = curve_under_cursor(sketch, cursor, tol, false) else {
        return ToolEffect::log("Click near the end of a line or an arc to extend");
    };
    let Some(prim) = prim_of(sketch, sketch.get_geometry(id).unwrap()) else {
        return ToolEffect::none();
    };
    let hits = hits_with_others(sketch, id, &prim, false);
    let p = cursor.to_glam();
    match prim {
        Prim::Seg { a, b } => {
            let Some(GeometryElement::Line(l)) = sketch.get_geometry(id) else {
                return ToolEffect::none();
            };
            let (start_pid, end_pid) = (l.start, l.end);
            let ab = b - a;
            let t_of = |q: Vec2| (q - a).dot(ab) / ab.length_squared();
            // The clicked half decides which endpoint grows.
            let target = if t_of(p) >= 0.5 {
                let t = hits
                    .iter()
                    .map(|q| t_of(*q))
                    .filter(|t| *t > 1.0 + SPAN_EPS)
                    .reduce(f32::min);
                t.map(|t| (end_pid, a + ab * t))
            } else {
                let t = hits
                    .iter()
                    .map(|q| t_of(*q))
                    .filter(|t| *t < -SPAN_EPS)
                    .reduce(f32::max);
                t.map(|t| (start_pid, a + ab * t))
            };
            let Some((pid, new_pos)) = target else {
                return ToolEffect::log("Nothing to extend to in that direction");
            };
            if let Some(GeometryElement::Point(pt)) = sketch.get_geometry_mut(pid) {
                pt.position = Vec2D::from_glam(new_pos);
            }
            curve_resized(sketch, id);
            ToolEffect::changed("Extended line to intersection")
        }
        Prim::Arc { c, r, s, e } => {
            let Some(GeometryElement::Arc(arc)) = sketch.get_geometry(id) else {
                return ToolEffect::none();
            };
            let (start_pid, end_pid) = (arc.start, arc.end);
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let rel_of = |q: Vec2| geom2d::wrap_positive((q - c).y.atan2((q - c).x) - start_angle);
            let outside: Vec<f32> = hits
                .iter()
                .map(|q| rel_of(*q))
                .filter(|rel| *rel > sweep + 1e-3 && *rel < std::f32::consts::TAU - 1e-3)
                .collect();
            let rel_click = rel_of(p).min(sweep);
            // Nearer endpoint half; the end grows CCW, the start grows CW.
            let target = if rel_click >= sweep * 0.5 {
                outside
                    .iter()
                    .copied()
                    .reduce(f32::min)
                    .map(|rel| (end_pid, rel))
            } else {
                outside
                    .iter()
                    .copied()
                    .reduce(f32::max)
                    .map(|rel| (start_pid, rel))
            };
            let Some((pid, rel)) = target else {
                return ToolEffect::none();
            };
            let ang = start_angle + rel;
            if let Some(GeometryElement::Point(pt)) = sketch.get_geometry_mut(pid) {
                pt.position = Vec2D::new(c.x + r * ang.cos(), c.y + r * ang.sin());
            }
            ToolEffect::changed("Extended arc to intersection")
        }
        Prim::Circle { .. } => ToolEffect::none(),
    }
}

pub(super) fn split(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> ToolEffect {
    if let Some(effect) = split_conic(sketch, cursor, tol) {
        return effect;
    }
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return split_general(sketch, id, &curve, cursor);
    }
    let Some(id) = curve_under_cursor(sketch, cursor, tol, false) else {
        return ToolEffect::log("Click a line or an arc to split");
    };
    let Some(prim) = prim_of(sketch, sketch.get_geometry(id).unwrap()) else {
        return ToolEffect::none();
    };
    let p = cursor.to_glam();
    match prim {
        Prim::Seg { a, b } => {
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            if !(SPAN_EPS..=1.0 - SPAN_EPS).contains(&t) {
                return ToolEffect::log("Too close to an end to split");
            }
            let m = Vec2D::from_glam(a + ab * t);
            let m_id = sketch.add_geometry(GeometryElement::Point(Point::new(m)));
            let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) else {
                return ToolEffect::none();
            };
            let old_end = l.end;
            l.end = m_id;
            curve_resized(sketch, id);
            let rest = sketch.add_geometry(GeometryElement::Line(Line::new(m_id, old_end)));
            continues(sketch, id, rest, true);
            ToolEffect::changed("Split line")
        }
        Prim::Arc { c, r, s, e } => {
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let rel = geom2d::wrap_positive((p - c).y.atan2((p - c).x) - start_angle);
            let eps = (sweep * SPAN_EPS).max(1e-4);
            if !(eps..=sweep - eps).contains(&rel) {
                return ToolEffect::none();
            }
            let ang = start_angle + rel;
            let m = Vec2D::new(c.x + r * ang.cos(), c.y + r * ang.sin());
            let m_id = sketch.add_geometry(GeometryElement::Point(Point::new(m)));
            let Some(GeometryElement::Arc(arc)) = sketch.get_geometry_mut(id) else {
                return ToolEffect::none();
            };
            let (center_pid, old_end, radius) = (arc.center, arc.end, arc.radius);
            arc.end = m_id;
            let rest = sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_pid, m_id, old_end, radius,
            )));
            // The halves stay one circle: same centre, same radius.
            sketch.add_constraint(ConstraintKind::EqualRadius {
                circle1: id,
                circle2: rest,
            });
            ToolEffect::changed("Split arc")
        }
        Prim::Circle { .. } => ToolEffect::none(),
    }
}

/// Split the arc of a parabola or hyperbola under the cursor, when one is
/// nearer than any line or arc: two arcs of the same curve, sharing its
/// centre and meeting at a new point on it. `None` when no such arc is the
/// nearest curve there.
fn split_conic(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> Option<ToolEffect> {
    let distance = |g: &GeometryElement| snap::distance_to_element(sketch, g, cursor);
    let (id, d) = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Conic(_)))
        .filter(|g| !sketch.is_external(g.id()))
        .filter_map(|g| Some((g.id(), distance(g)?)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let nearer = curve_under_cursor(sketch, cursor, tol, false)
        .and_then(|other| distance(sketch.get_geometry(other)?))
        .is_some_and(|other| other < d);
    if nearer {
        return None;
    }
    let Some(GeometryElement::Conic(conic)) = sketch.get_geometry(id).cloned() else {
        return None;
    };
    let (shape, t0, t1) = conic.params(sketch)?;
    let t = shape.param([f64::from(cursor.x), f64::from(cursor.y)]);
    let (lo, hi) = (t0.min(t1), t0.max(t1));
    let eps = (hi - lo) * f64::from(SPAN_EPS);
    if !(lo + eps..=hi - eps).contains(&t) {
        return Some(ToolEffect::log("Too close to an end to split"));
    }
    let [x, y] = shape.point(t);
    let middle = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
        x as f32, y as f32,
    ))));
    if let Some(GeometryElement::Conic(first)) = sketch.get_geometry_mut(id) {
        first.end = middle;
    }
    sketch.add_geometry(GeometryElement::Conic(crate::sketch::Conic {
        id: Uuid::new_v4(),
        start: middle,
        ..conic
    }));
    Some(ToolEffect::changed("Split arc"))
}

/// The curve nearest `pos` within `tol`, of any kind.
fn nearest_curve(sketch: &Sketch, pos: Vec2D, tol: f32) -> Option<Uuid> {
    sketch
        .geometry
        .iter()
        .filter(|g| !matches!(g, GeometryElement::Point(_)))
        .filter_map(|g| snap::distance_to_element(sketch, g, pos).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The curve under the cursor when it is one only the general path cuts:
/// an ellipse or arc of one, a parabola or hyperbola, a spline.
fn general_under_cursor(sketch: &Sketch, pos: Vec2D, tol: f32) -> Option<(Uuid, Curve)> {
    let id = nearest_curve(sketch, pos, tol)?;
    let geom = sketch.get_geometry(id)?;
    if prim_of(sketch, geom).is_some() {
        return None;
    }
    Some((id, Curve::of(sketch, geom)?))
}

/// Where `target` (over `span`) crosses every other curve of the sketch,
/// as parameters on it.
fn general_hits(sketch: &Sketch, id: Uuid, target: &Curve, span: (f64, f64)) -> Vec<f64> {
    sketch
        .geometry
        .iter()
        .filter(|g| g.id() != id)
        .filter_map(|g| Curve::of(sketch, g))
        .flat_map(|other| {
            crate::curves::crossings(target, span, &other, other.span)
                .into_iter()
                .map(|(t, _)| t)
        })
        .collect()
}

/// A general curve's crossings as sketch points, for the line, arc and
/// circle paths that meet one.
fn general_points(sketch: &Sketch, target_id: Uuid, bounded: bool) -> Vec<Vec2> {
    let Some(target) = sketch
        .get_geometry(target_id)
        .and_then(|g| Curve::of(sketch, g))
    else {
        return Vec::new();
    };
    let span = if bounded { target.span } else { target.reach() };
    sketch
        .geometry
        .iter()
        .filter(|g| g.id() != target_id && prim_of(sketch, g).is_none())
        .filter_map(|g| Curve::of(sketch, g))
        .flat_map(|other| {
            crate::curves::crossings(&target, span, &other, other.span)
                .into_iter()
                .map(|(t, _)| target.at(t).as_vec2())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// What a trim of a general curve removes, in its parameter.
enum GeneralCut {
    /// Between two parameters of an open curve; `None` runs to that end.
    Open {
        lo: Option<f64>,
        hi: Option<f64>,
    },
    /// Around a closed curve from `lo` forward to `hi`.
    Closed {
        lo: f64,
        hi: f64,
    },
    Whole,
}

fn plan_general(sketch: &Sketch, id: Uuid, curve: &Curve, cursor: Vec2D) -> GeneralCut {
    let p = glam::DVec2::new(f64::from(cursor.x), f64::from(cursor.y));
    if curve.closed {
        let reach = (curve.span.0, curve.span.0 + curve.period);
        let tc = curve.nearest(p, reach);
        let rels: Vec<f64> = general_hits(sketch, id, curve, reach)
            .into_iter()
            .map(|t| (t - tc).rem_euclid(curve.period))
            .filter(|r| *r > 1e-9 && *r < curve.period - 1e-9)
            .collect();
        let hi = rels.iter().copied().reduce(f64::min);
        let lo = rels.iter().copied().reduce(f64::max);
        return match (lo, hi) {
            (Some(lo), Some(hi)) if (lo - hi).abs() > 1e-9 => GeneralCut::Closed {
                lo: tc + lo - curve.period,
                hi: tc + hi,
            },
            _ => GeneralCut::Whole,
        };
    }
    let (s0, s1) = curve.span;
    let eps = (s1 - s0) * f64::from(SPAN_EPS);
    let tc = curve.nearest(p, curve.span);
    let ts: Vec<f64> = general_hits(sketch, id, curve, curve.span)
        .into_iter()
        .filter(|t| *t > s0 + eps && *t < s1 - eps)
        .collect();
    let lo = ts.iter().copied().filter(|t| *t < tc).reduce(f64::max);
    let hi = ts.iter().copied().filter(|t| *t > tc).reduce(f64::min);
    if lo.is_none() && hi.is_none() {
        GeneralCut::Whole
    } else {
        GeneralCut::Open { lo, hi }
    }
}

/// Points along `curve` from `t0` to `t1`.
fn general_samples(curve: &Curve, t0: f64, t1: f64) -> Vec<Vec2D> {
    const SAMPLES: usize = 48;
    (0..=SAMPLES)
        .map(|i| {
            let p = curve.at(t0 + (t1 - t0) * i as f64 / SAMPLES as f64);
            Vec2D::new(p.x as f32, p.y as f32)
        })
        .collect()
}

fn general_preview(sketch: &Sketch, id: Uuid, curve: &Curve, cursor: Vec2D) -> Vec<Vec2D> {
    match plan_general(sketch, id, curve, cursor) {
        GeneralCut::Whole if curve.closed => {
            general_samples(curve, curve.span.0, curve.span.0 + curve.period)
        }
        GeneralCut::Whole => general_samples(curve, curve.span.0, curve.span.1),
        GeneralCut::Open { lo, hi } => general_samples(
            curve,
            lo.unwrap_or(curve.span.0),
            hi.unwrap_or(curve.span.1),
        ),
        GeneralCut::Closed { lo, hi } => general_samples(curve, lo, hi),
    }
}

/// A new point on `curve` at `t`.
fn point_at(sketch: &mut Sketch, curve: &Curve, t: f64) -> Uuid {
    let p = curve.at(t);
    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
        p.x as f32, p.y as f32,
    ))))
}

/// Replace the spline `id` with its parts over `spans` (each `(t0, t1)` in
/// its parameter): the first keeps its id. A part keeps the spline's own
/// end point where it starts or ends there, and `shared` names points
/// parts meet at, by parameter.
fn cut_spline(sketch: &mut Sketch, id: Uuid, spans: &[(f64, f64)], shared: &[(f64, Uuid)]) -> bool {
    let Some(GeometryElement::BSpline(spline)) = sketch.get_geometry(id).cloned() else {
        return false;
    };
    let Some(basis) = crate::spline::basis_of(&spline) else {
        return false;
    };
    let Some(control) = spline
        .control_points
        .iter()
        .map(|p| {
            sketch
                .point_position(*p)
                .map(|v| [f64::from(v.x), f64::from(v.y)])
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let (d0, d1) = basis.domain();
    let pieces: Option<Vec<_>> = spans
        .iter()
        .map(|&(a, b)| basis.piece(&control, a, b).map(|piece| (a, b, piece)))
        .collect();
    let Some(pieces) = pieces else {
        return false;
    };
    let near = |x: f64, y: f64| (x - y).abs() < 1e-9 * (d1 - d0).max(1.0);
    let old_points = spline.point_ids();
    let mut replaced = false;
    for (a, b, piece) in pieces {
        let last = piece.control.len() - 1;
        let ids: Vec<Uuid> = piece
            .control
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let at = match i {
                    0 => Some(a),
                    i if i == last => Some(b),
                    _ => None,
                };
                if let Some(t) = at {
                    if !spline.periodic && near(t, d0) {
                        return spline.control_points[0];
                    }
                    if !spline.periodic && near(t, d1) {
                        return *spline.control_points.last().unwrap();
                    }
                    if let Some((_, pid)) = shared.iter().find(|(s, _)| near(*s, t)) {
                        return *pid;
                    }
                }
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
                    q[0] as f32,
                    q[1] as f32,
                ))))
            })
            .collect();
        let part = crate::sketch::BSpline {
            id: if replaced { Uuid::new_v4() } else { id },
            control_points: ids,
            periodic: false,
            degree: basis.degree() as u32,
            knots: piece.knots,
            fit_points: Vec::new(),
            fit_params: Vec::new(),
            weights: piece.weights,
        };
        if replaced {
            sketch.add_geometry(GeometryElement::BSpline(part));
        } else if let Some(slot) = sketch.geometry.iter_mut().find(|g| g.id() == id) {
            *slot = GeometryElement::BSpline(part);
            replaced = true;
        }
    }
    for p in old_points {
        drop_if_orphan(sketch, p);
    }
    true
}

/// The ends of an ellipse arc or conic arc, the one at the low parameter
/// first.
fn low_high_ends(sketch: &Sketch, id: Uuid) -> Option<(Uuid, Uuid)> {
    match sketch.get_geometry(id)? {
        GeometryElement::Ellipse(e) => e.arc.map(|arc| (arc.start, arc.end)),
        GeometryElement::Conic(k) => {
            let (_, t0, t1) = k.params(sketch)?;
            Some(if t0 <= t1 {
                (k.start, k.end)
            } else {
                (k.end, k.start)
            })
        }
        _ => None,
    }
}

/// Swap the ellipse or conic arc's end `old` for `new`, keeping `new` on
/// the curve where an ellipse's ends are held.
fn replace_end(sketch: &mut Sketch, id: Uuid, old: Uuid, new: Uuid) {
    let on_ellipse = match sketch.get_geometry_mut(id) {
        Some(GeometryElement::Ellipse(e)) => {
            if let Some(arc) = e.arc.as_mut() {
                if arc.start == old {
                    arc.start = new;
                } else if arc.end == old {
                    arc.end = new;
                }
            }
            true
        }
        Some(GeometryElement::Conic(k)) => {
            if k.start == old {
                k.start = new;
            } else if k.end == old {
                k.end = new;
            }
            false
        }
        _ => false,
    };
    if on_ellipse {
        sketch.add_constraint(ConstraintKind::PointOnEllipse {
            point: new,
            ellipse: id,
        });
    }
}

/// A second arc of the same ellipse or conic as `id`, from `low` to `high`
/// as its parameter runs.
fn another_arc(sketch: &mut Sketch, id: Uuid, low: Uuid, high: Uuid) -> Option<Uuid> {
    let geom = sketch.get_geometry(id)?.clone();
    match geom {
        GeometryElement::Ellipse(e) => {
            let new = sketch.add_geometry(GeometryElement::Ellipse(
                crate::sketch::Ellipse::new_arc(e.center, e.major, e.ratio, low, high),
            ));
            for point in [low, high] {
                sketch.add_constraint(ConstraintKind::PointOnEllipse {
                    point,
                    ellipse: new,
                });
            }
            Some(new)
        }
        GeometryElement::Conic(k) => {
            let (_, t0, t1) = k.params(sketch)?;
            let (start, end) = if t0 <= t1 { (low, high) } else { (high, low) };
            Some(
                sketch.add_geometry(GeometryElement::Conic(crate::sketch::Conic {
                    id: Uuid::new_v4(),
                    start,
                    end,
                    ..k
                })),
            )
        }
        _ => None,
    }
}

fn trim_general(sketch: &mut Sketch, id: Uuid, curve: &Curve, cursor: Vec2D) -> ToolEffect {
    let is_spline = matches!(sketch.get_geometry(id), Some(GeometryElement::BSpline(_)));
    match plan_general(sketch, id, curve, cursor) {
        GeneralCut::Whole => {
            let pts = sketch
                .get_geometry(id)
                .map(Sketch::curve_point_ids)
                .unwrap_or_default();
            sketch.remove_geometry_cascade(&[id]);
            for p in pts {
                drop_if_orphan(sketch, p);
            }
            ToolEffect::changed("Trimmed away whole element")
        }
        GeneralCut::Closed { lo, hi } => {
            // What stays runs on from `hi` round to `lo`.
            let kept = (hi, lo + curve.period);
            if is_spline {
                return if cut_spline(sketch, id, &[kept], &[]) {
                    ToolEffect::changed("Trimmed spline")
                } else {
                    ToolEffect::none()
                };
            }
            let (start, end) = (
                point_at(sketch, curve, kept.0),
                point_at(sketch, curve, kept.1),
            );
            if let Some(GeometryElement::Ellipse(e)) = sketch.get_geometry_mut(id) {
                e.arc = Some(crate::sketch::EllipseArcEnds { start, end });
            }
            for point in [start, end] {
                sketch.add_constraint(ConstraintKind::PointOnEllipse { point, ellipse: id });
            }
            ToolEffect::changed("Trimmed ellipse to an arc")
        }
        GeneralCut::Open { lo, hi } => {
            let (s0, s1) = curve.span;
            if is_spline {
                let spans: Vec<(f64, f64)> = [lo.map(|lo| (s0, lo)), hi.map(|hi| (hi, s1))]
                    .into_iter()
                    .flatten()
                    .collect();
                return if cut_spline(sketch, id, &spans, &[]) {
                    ToolEffect::changed("Trimmed spline")
                } else {
                    ToolEffect::none()
                };
            }
            let Some((low, high)) = low_high_ends(sketch, id) else {
                return ToolEffect::none();
            };
            match (lo, hi) {
                (Some(lo), Some(hi)) => {
                    let (a, b) = (point_at(sketch, curve, lo), point_at(sketch, curve, hi));
                    replace_end(sketch, id, high, a);
                    another_arc(sketch, id, b, high);
                }
                (Some(lo), None) => {
                    let a = point_at(sketch, curve, lo);
                    replace_end(sketch, id, high, a);
                    drop_if_orphan(sketch, high);
                }
                (None, Some(hi)) => {
                    let b = point_at(sketch, curve, hi);
                    replace_end(sketch, id, low, b);
                    drop_if_orphan(sketch, low);
                }
                (None, None) => return ToolEffect::none(),
            }
            curve_resized(sketch, id);
            ToolEffect::changed("Trimmed arc span")
        }
    }
}

fn extend_general(sketch: &mut Sketch, id: Uuid, curve: &Curve, cursor: Vec2D) -> ToolEffect {
    if !curve.extends() {
        return ToolEffect::log("A spline stops at its ends: nothing to extend along");
    }
    if curve.closed {
        return ToolEffect::log("A whole ellipse has no end to extend");
    }
    let Some((low, high)) = low_high_ends(sketch, id) else {
        return ToolEffect::none();
    };
    let (s0, s1) = curve.span;
    let p = glam::DVec2::new(f64::from(cursor.x), f64::from(cursor.y));
    let tc = curve.nearest(p, curve.span);
    let at_high = tc >= (s0 + s1) * 0.5;
    let reach = curve.reach();
    let hits = general_hits(sketch, id, curve, reach);
    let eps = (s1 - s0) * f64::from(SPAN_EPS);
    let target = if curve.period > 0.0 {
        // Round the closed carrier from the end, forward or back.
        let outside = |t: f64| curve.wrap(t, s0);
        let beyond: Vec<f64> = hits
            .iter()
            .map(|t| outside(*t))
            .filter(|t| *t > s1 + eps && *t < s0 + curve.period - eps)
            .collect();
        if at_high {
            beyond.iter().copied().reduce(f64::min)
        } else {
            beyond.iter().copied().reduce(f64::max)
        }
    } else if at_high {
        hits.iter()
            .copied()
            .filter(|t| *t > s1 + eps)
            .reduce(f64::min)
    } else {
        hits.iter()
            .copied()
            .filter(|t| *t < s0 - eps)
            .reduce(f64::max)
    };
    let Some(t) = target else {
        return ToolEffect::log("Nothing to extend to in that direction");
    };
    let q = curve.at(t);
    let moving = if at_high { high } else { low };
    if let Some(GeometryElement::Point(pt)) = sketch.get_geometry_mut(moving) {
        pt.position = Vec2D::new(q.x as f32, q.y as f32);
    }
    curve_resized(sketch, id);
    ToolEffect::changed("Extended arc to intersection")
}

fn split_general(sketch: &mut Sketch, id: Uuid, curve: &Curve, cursor: Vec2D) -> ToolEffect {
    let p = glam::DVec2::new(f64::from(cursor.x), f64::from(cursor.y));
    let is_spline = matches!(sketch.get_geometry(id), Some(GeometryElement::BSpline(_)));
    if curve.closed {
        if !is_spline {
            return ToolEffect::log("A whole ellipse cannot be split at one point: trim it");
        }
        // A closed spline opens where it is split, both ends on one point.
        let reach = (curve.span.0, curve.span.0 + curve.period);
        let t = curve.nearest(p, reach);
        let mid = point_at(sketch, curve, t);
        return if cut_spline(
            sketch,
            id,
            &[(t, t + curve.period)],
            &[(t, mid), (t + curve.period, mid)],
        ) {
            ToolEffect::changed("Opened the spline")
        } else {
            ToolEffect::none()
        };
    }
    let (s0, s1) = curve.span;
    let t = curve.nearest(p, curve.span);
    let eps = (s1 - s0) * f64::from(SPAN_EPS);
    if t < s0 + eps || t > s1 - eps {
        return ToolEffect::log("Too close to an end to split");
    }
    let mid = point_at(sketch, curve, t);
    if is_spline {
        return if cut_spline(sketch, id, &[(s0, t), (t, s1)], &[(t, mid)]) {
            ToolEffect::changed("Split spline")
        } else {
            ToolEffect::none()
        };
    }
    let Some((_, high)) = low_high_ends(sketch, id) else {
        return ToolEffect::none();
    };
    replace_end(sketch, id, high, mid);
    another_arc(sketch, id, mid, high);
    curve_resized(sketch, id);
    ToolEffect::changed("Split arc")
}

/// One curve of the selection, oriented along the chain walk.
struct ChainLink {
    id: Uuid,
    entry: Uuid,
    exit: Uuid,
}

/// Endpoint ids of a chainable curve (lines and arcs).
fn chain_ends(geom: &GeometryElement) -> Option<(Uuid, Uuid)> {
    match geom {
        GeometryElement::Line(l) => Some((l.start, l.end)),
        GeometryElement::Arc(a) => Some((a.start, a.end)),
        _ => None,
    }
}

/// Order the selected curves into a single connected path or cycle.
fn order_chain(curves: &[(Uuid, (Uuid, Uuid))]) -> Option<(Vec<ChainLink>, bool)> {
    let mut degree: HashMap<Uuid, Vec<usize>> = HashMap::new();
    for (idx, (_, (a, b))) in curves.iter().enumerate() {
        degree.entry(*a).or_default().push(idx);
        degree.entry(*b).or_default().push(idx);
    }
    if degree.values().any(|v| v.len() > 2) {
        return None; // branching selection
    }
    // Open chains start at a degree-1 endpoint; cycles anywhere.
    let start_point = degree
        .iter()
        .find(|(_, v)| v.len() == 1)
        .map(|(p, _)| *p)
        .unwrap_or(curves[0].1.0);
    let mut used = vec![false; curves.len()];
    let mut links = Vec::new();
    let mut current = start_point;
    while links.len() < curves.len() {
        let Some(idx) = degree
            .get(&current)
            .and_then(|v| v.iter().copied().find(|i| !used[*i]))
        else {
            return None; // disconnected selection
        };
        used[idx] = true;
        let (id, (a, b)) = curves[idx];
        let exit = if a == current { b } else { a };
        links.push(ChainLink {
            id,
            entry: current,
            exit,
        });
        current = exit;
    }
    Some((links, current == start_point && curves.len() > 1))
}

/// The offset carrier of one link.
enum OffsetPrim {
    Line,
    Arc { c: Vec2, new_r: f32 },
}

/// How the offset tool makes its copies.
#[derive(Clone, Copy)]
pub(super) struct OffsetOptions {
    pub distance: f32,
    /// Where two copies part at a convex corner, an arc about the corner
    /// joins them rather than the copies run on to meet.
    pub round: bool,
    /// A copy on each side.
    pub both: bool,
    /// The originals go, the copies take their place.
    pub delete: bool,
    /// One offset dimension holds each copy to its original.
    pub linked: bool,
}

/// What one side's offset made: each original with its copy.
struct OffsetMade {
    pairs: Vec<[Uuid; 2]>,
}

pub(super) fn offset(
    sketch: &mut Sketch,
    cursor: Vec2D,
    selected: &HashSet<Uuid>,
    options: OffsetOptions,
) -> ToolEffect {
    let distance = options.distance;
    if distance < 1e-6 {
        return ToolEffect::none();
    }
    let curves: Vec<GeometryElement> = sketch
        .geometry
        .iter()
        .filter(|g| {
            selected.contains(&g.id())
                && matches!(
                    g,
                    GeometryElement::Line(_)
                        | GeometryElement::Arc(_)
                        | GeometryElement::Circle(_)
                        | GeometryElement::Ellipse(_)
                        | GeometryElement::BSpline(_)
                        | GeometryElement::Conic(_)
                )
        })
        .cloned()
        .collect();
    if curves.is_empty() {
        return ToolEffect::none();
    }
    let sides: &[bool] = if options.both {
        &[false, true]
    } else {
        &[false]
    };
    let mut made = Vec::new();
    for &flip in sides {
        let side = match curves.as_slice() {
            [GeometryElement::Circle(circle)] => {
                offset_circle(sketch, circle, cursor, distance, flip)
            }
            [
                one @ (GeometryElement::Ellipse(_)
                | GeometryElement::BSpline(_)
                | GeometryElement::Conic(_)),
            ] => offset_sampled(sketch, one.id(), cursor, distance, flip),
            _ if curves.iter().all(|g| chain_ends(g).is_some()) => {
                offset_chain(sketch, &curves, cursor, distance, flip, options.round)
            }
            _ => None,
        };
        let Some(side) = side else {
            continue;
        };
        made.push(side);
    }
    if made.is_empty() {
        return ToolEffect::log(
            "Select one circle, ellipse, spline or conic, or a chain of lines and arcs",
        );
    }
    if options.delete {
        let originals: Vec<Uuid> = curves.iter().map(|g| g.id()).collect();
        sketch.remove_geometry_cascade(&originals);
    } else if options.linked {
        for side in made.iter().filter(|m| !m.pairs.is_empty()) {
            sketch.add_constraint(ConstraintKind::Offset {
                pairs: side.pairs.clone(),
                distance,
            });
        }
    }
    let count: usize = made.iter().map(|m| m.pairs.len().max(1)).sum();
    ToolEffect::changed(format!("Offset {count} element(s) by {distance:.2}"))
}

/// A concentric copy of a circle, on the side of the click (the other with
/// `flip`).
fn offset_circle(
    sketch: &mut Sketch,
    circle: &crate::sketch::Circle,
    cursor: Vec2D,
    distance: f32,
    flip: bool,
) -> Option<OffsetMade> {
    let c = sketch.point_position(circle.center)?;
    let outside = ((cursor - c).to_glam().length() > circle.radius) != flip;
    let new_r = if outside {
        circle.radius + distance
    } else {
        circle.radius - distance
    };
    if new_r < 1e-6 {
        return None;
    }
    let flag = sketch.is_construction(circle.id);
    let new_id = sketch.add_geometry(GeometryElement::Circle(crate::sketch::Circle::new(
        circle.center,
        new_r,
    )));
    sketch.set_construction(new_id, flag);
    Some(OffsetMade {
        pairs: vec![[circle.id, new_id]],
    })
}

/// How many points a sampled curve's copy passes through.
const OFFSET_SAMPLES: usize = 32;

/// An ellipse's, spline's or conic's copy, which is none of those: a
/// spline through points set off square to the curve along it.
fn offset_sampled(
    sketch: &mut Sketch,
    id: Uuid,
    cursor: Vec2D,
    distance: f32,
    flip: bool,
) -> Option<OffsetMade> {
    let geom = sketch.get_geometry(id)?.clone();
    let closed = match &geom {
        GeometryElement::Ellipse(e) => e.arc.is_none(),
        GeometryElement::BSpline(b) => b.periodic,
        _ => false,
    };
    let mut samples: Vec<Vec2> = crate::external_ref::element_samples(sketch, &geom)?
        .into_iter()
        .map(Vec2D::to_glam)
        .collect();
    if closed && samples.len() > 2 {
        samples.pop();
    }
    let n = samples.len();
    if n < 3 {
        return None;
    }
    let tangent = |i: usize| {
        let (prev, next) = if closed {
            ((i + n - 1) % n, (i + 1) % n)
        } else {
            (i.saturating_sub(1), (i + 1).min(n - 1))
        };
        (samples[next] - samples[prev]).normalize_or_zero()
    };
    let p = cursor.to_glam();
    let nearest = (0..n).min_by(|a, b| {
        (samples[*a] - p)
            .length()
            .total_cmp(&(samples[*b] - p).length())
    })?;
    let left = (tangent(nearest).perp().dot(p - samples[nearest]) > 0.0) != flip;
    let d = if left { distance } else { -distance };
    let step = (n as f32 / OFFSET_SAMPLES as f32).max(1.0);
    let mut picks: Vec<usize> = (0..)
        .map(|k| (k as f32 * step) as usize)
        .take_while(|i| *i < n)
        .collect();
    if !closed && picks.last() != Some(&(n - 1)) {
        picks.push(n - 1);
    }
    let points: Vec<[f64; 2]> = picks
        .iter()
        .map(|&i| samples[i] + tangent(i).perp() * d)
        .map(|q| [f64::from(q.x), f64::from(q.y)])
        .collect();
    let fit = sketch_solver::spline::interpolate(&points, 3, closed)?;
    let control: Vec<Uuid> = fit
        .control
        .iter()
        .map(|q| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
                q[0] as f32,
                q[1] as f32,
            ))))
        })
        .collect();
    let flag = sketch.is_construction(id);
    let new_id = sketch.add_geometry(GeometryElement::BSpline(crate::sketch::BSpline {
        degree: fit.degree,
        knots: fit.knots,
        ..crate::sketch::BSpline::new(control, closed)
    }));
    sketch.set_construction(new_id, flag);
    // A spline's distance from its original is no dimension the solver
    // holds, so it is not paired.
    Some(OffsetMade { pairs: Vec::new() })
}

/// Tangent of a chain link at its `at` end, the way the chain runs.
fn link_tangent(
    sketch: &Sketch,
    geom: &GeometryElement,
    link: &ChainLink,
    at: Uuid,
) -> Option<Vec2> {
    let pos = |id: Uuid| sketch.point_position(id).map(|p| p.to_glam());
    Some(match geom {
        GeometryElement::Line(_) => (pos(link.exit)? - pos(link.entry)?).normalize_or_zero(),
        GeometryElement::Arc(arc) => {
            let ccw = (pos(at)? - pos(arc.center)?).normalize_or_zero().perp();
            if link.entry == arc.start { ccw } else { -ccw }
        }
        _ => return None,
    })
}

/// Copies of a chain of lines and arcs, joined where they meet.
fn offset_chain(
    sketch: &mut Sketch,
    curves: &[GeometryElement],
    cursor: Vec2D,
    distance: f32,
    flip: bool,
    round: bool,
) -> Option<OffsetMade> {
    let ends: Vec<(Uuid, (Uuid, Uuid))> = curves
        .iter()
        .filter_map(|g| chain_ends(g).map(|e| (g.id(), e)))
        .collect();
    let (links, closed) = order_chain(&ends)?;
    let elem_of = |id: Uuid| curves.iter().find(|g| g.id() == id).unwrap();
    let pos_of = |sketch: &Sketch, pid: Uuid| sketch.point_position(pid).map(|p| p.to_glam());

    // Signed left-offset: positive when the click lies left of the chain
    // direction at the nearest link.
    let near = links
        .iter()
        .filter_map(|l| snap::distance_to_element(sketch, elem_of(l.id), cursor).map(|d| (l, d)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(l, _)| l)?;
    let left = match elem_of(near.id) {
        GeometryElement::Line(_) => {
            let (a, b) = (pos_of(sketch, near.entry)?, pos_of(sketch, near.exit)?);
            (b - a).perp_dot(cursor.to_glam() - a) > 0.0
        }
        GeometryElement::Arc(arc) => {
            let (c, s) = (pos_of(sketch, arc.center)?, pos_of(sketch, arc.start)?);
            let inside = (cursor.to_glam() - c).length() < (s - c).length();
            // Traversed CCW (entry == start) the left side faces the center.
            if near.entry == arc.start {
                inside
            } else {
                !inside
            }
        }
        _ => return None,
    };
    let d_left = if left != flip { distance } else { -distance };

    // Raw offset endpoints + carriers per link.
    let mut prims: Vec<(OffsetPrim, Vec2, Vec2)> = Vec::with_capacity(links.len());
    for link in &links {
        let (a, b) = (pos_of(sketch, link.entry)?, pos_of(sketch, link.exit)?);
        match elem_of(link.id) {
            GeometryElement::Line(_) => {
                let dir = b - a;
                if dir.length() < 1e-6 {
                    return None;
                }
                let shift = dir.normalize().perp() * d_left;
                prims.push((OffsetPrim::Line, a + shift, b + shift));
            }
            GeometryElement::Arc(arc) => {
                let c = pos_of(sketch, arc.center)?;
                let r = (pos_of(sketch, arc.start).unwrap_or(a) - c).length();
                // CCW traversal keeps the center on the left.
                let forward = link.entry == arc.start;
                let new_r = if forward { r - d_left } else { r + d_left };
                if new_r < 1e-6 {
                    return None; // arc would invert
                }
                let proj = |q: Vec2| c + (q - c).normalize() * new_r;
                prims.push((OffsetPrim::Arc { c, new_r }, proj(a), proj(b)));
            }
            _ => return None,
        }
    }

    // Join consecutive offsets at their carrier intersection (nearest
    // candidate to the raw corner; the raw midpoint as a fallback).
    let join = |ap: &(OffsetPrim, Vec2, Vec2), bp: &(OffsetPrim, Vec2, Vec2)| -> Vec2 {
        let raw_mid = (ap.2 + bp.1) * 0.5;
        let candidates = match (&ap.0, &bp.0) {
            (OffsetPrim::Line, OffsetPrim::Line) => geom2d::line_line(ap.1, ap.2, bp.1, bp.2)
                .into_iter()
                .collect::<Vec<_>>(),
            (OffsetPrim::Line, OffsetPrim::Arc { c, new_r }) => {
                geom2d::line_circle(ap.1, ap.2, *c, *new_r)
            }
            (OffsetPrim::Arc { c, new_r }, OffsetPrim::Line) => {
                geom2d::line_circle(bp.1, bp.2, *c, *new_r)
            }
            (OffsetPrim::Arc { c: c1, new_r: r1 }, OffsetPrim::Arc { c: c2, new_r: r2 }) => {
                geom2d::circle_circle(*c1, *r1, *c2, *r2)
            }
        };
        candidates
            .into_iter()
            .min_by(|p, q| (*p - raw_mid).length().total_cmp(&(*q - raw_mid).length()))
            .unwrap_or(raw_mid)
    };
    let new_point = |sketch: &mut Sketch, p: Vec2| {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(p))))
    };
    // Each copy's new (entry, exit) points, and the corners rounded.
    let mut link_ends: Vec<(Option<Uuid>, Option<Uuid>)> = vec![(None, None); links.len()];
    let mut rounded: Vec<(Uuid, Uuid, Uuid)> = Vec::new();
    for i in 0..links.len() {
        let j = (i + 1) % links.len();
        if j == 0 && !closed {
            break;
        }
        let corner = links[i].exit;
        let turn = match (
            link_tangent(sketch, elem_of(links[i].id), &links[i], corner),
            link_tangent(sketch, elem_of(links[j].id), &links[j], corner),
        ) {
            (Some(t_in), Some(t_out)) => t_in.perp_dot(t_out),
            _ => 0.0,
        };
        // Offset to the left of a right turn (or the right of a left one)
        // opens a gap at the corner: a convex corner on the copy's side.
        if round && turn * d_left < -1e-4 {
            let a = new_point(sketch, prims[i].2);
            let b = new_point(sketch, prims[j].1);
            link_ends[i].1 = Some(a);
            link_ends[j].0 = Some(b);
            rounded.push((corner, a, b));
        } else {
            let p = new_point(sketch, join(&prims[i], &prims[j]));
            link_ends[i].1 = Some(p);
            link_ends[j].0 = Some(p);
        }
    }
    if !closed {
        let last = links.len() - 1;
        link_ends[0].0 = Some(new_point(sketch, prims[0].1));
        link_ends[last].1 = Some(new_point(sketch, prims[last].2));
    }

    let mut pairs = Vec::new();
    let mut copies: HashMap<Uuid, Uuid> = HashMap::new();
    for ((link, prim), (entry, exit)) in links.iter().zip(&prims).zip(&link_ends) {
        let (entry, exit) = (entry.unwrap(), exit.unwrap());
        let flag = sketch.is_construction(link.id);
        let new_id = match (elem_of(link.id).clone(), &prim.0) {
            (GeometryElement::Line(_), _) => {
                sketch.add_geometry(GeometryElement::Line(Line::new(entry, exit)))
            }
            (GeometryElement::Arc(arc), OffsetPrim::Arc { new_r, .. }) => {
                // Preserve the stored CCW start/end regardless of traversal.
                let (start, end) = if link.entry == arc.start {
                    (entry, exit)
                } else {
                    (exit, entry)
                };
                sketch.add_geometry(GeometryElement::Arc(Arc::new(
                    arc.center, start, end, *new_r,
                )))
            }
            _ => continue,
        };
        sketch.set_construction(new_id, flag);
        pairs.push([link.id, new_id]);
        copies.insert(link.id, new_id);
    }
    // Each rounded corner: an arc about the original corner from one copy's
    // end to the next one's start, tangent to both.
    for (corner, a, b) in rounded {
        let Some(c) = pos_of(sketch, corner) else {
            continue;
        };
        let (Some(pa), Some(pb)) = (pos_of(sketch, a), pos_of(sketch, b)) else {
            continue;
        };
        let (start, end) = if (pa - c).perp_dot(pb - c) > 0.0 {
            (a, b)
        } else {
            (b, a)
        };
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(corner, start, end, distance)));
        for (link, (entry, exit)) in links.iter().zip(&link_ends) {
            if [*entry, *exit]
                .iter()
                .any(|e| *e == Some(a) || *e == Some(b))
                && let Some(copy) = copies.get(&link.id)
            {
                sketch.add_constraint(ConstraintKind::Tangent {
                    line_or_circle1: *copy,
                    item2: arc,
                });
            }
        }
    }
    Some(OffsetMade { pairs })
}
