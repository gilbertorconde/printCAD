//! Editing tools that operate on existing geometry: corner fillet/chamfer,
//! trim, extend, split and offset.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use uuid::Uuid;

use super::ToolEffect;
use crate::geom2d;
use crate::sketch::{Arc, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::snap::{self, arc_angles};

/// Relative slack in curve parameter space: intersections this close to a
/// span end don't count as cut points (shared endpoints intersect exactly
/// at the ends).
const SPAN_EPS: f32 = 1e-3;

// ---------------------------------------------------------------- corners

struct CornerCtx {
    corner_id: Uuid,
    corner: Vec2D,
    l1: Uuid,
    l2: Uuid,
    /// Unit directions corner → far endpoint of each line.
    u1: Vec2,
    u2: Vec2,
    len1: f32,
    len2: f32,
}

/// The corner point under the cursor, when shared by exactly two
/// non-collinear lines.
fn corner_under_cursor(sketch: &Sketch, cursor: Vec2D, snap_tol: f32) -> Option<CornerCtx> {
    let snap::SnapTarget::Existing(corner_id) = snap::snap_to_point(sketch, cursor, snap_tol, &[])
    else {
        return None; // no point under the cursor
    };
    let corner = sketch.point_position(corner_id)?;

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
        return None;
    };
    let far_of = |l: &Line| {
        let far = if l.start == corner_id { l.end } else { l.start };
        sketch.point_position(far)
    };
    let (far1, far2) = (far_of(l1)?, far_of(l2)?);

    let v1 = (far1 - corner).to_glam();
    let v2 = (far2 - corner).to_glam();
    let (len1, len2) = (v1.length(), v2.length());
    if len1 < 1e-6 || len2 < 1e-6 {
        return None;
    }
    let (u1, u2) = (v1 / len1, v2 / len2);
    if u1.perp_dot(u2).abs() < 1e-6 {
        return None; // collinear segments: no corner to cut
    }
    Some(CornerCtx {
        corner_id,
        corner,
        l1: l1.id,
        l2: l2.id,
        u1,
        u2,
        len1,
        len2,
    })
}

/// Shorten both corner lines to their new endpoints and remove the corner
/// point together with every constraint that referenced it.
fn replace_corner(sketch: &mut Sketch, ctx: &CornerCtx, t1_id: Uuid, t2_id: Uuid) {
    for (line_id, new_end) in [(ctx.l1, t1_id), (ctx.l2, t2_id)] {
        if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(line_id) {
            if l.start == ctx.corner_id {
                l.start = new_end;
            } else {
                l.end = new_end;
            }
        }
    }
    sketch.geometry.retain(|g| g.id() != ctx.corner_id);
    sketch
        .constraints
        .retain(|c| !crate::sketch::constraint_refs(&c.kind).contains(&ctx.corner_id));
    sketch.construction.remove(&ctx.corner_id);
}

/// Fillet the corner point under the cursor: the point must be shared by
/// exactly two non-collinear lines, both long enough for the tangent
/// offset. The corner point is replaced by two tangent points joined by a
/// CCW arc; constraints referencing the removed corner are dropped.
pub(super) fn fillet(sketch: &mut Sketch, cursor: Vec2D, snap_tol: f32, radius: f32) -> ToolEffect {
    if radius < 1e-6 {
        return ToolEffect::none();
    }
    let Some(ctx) = corner_under_cursor(sketch, cursor, snap_tol) else {
        return ToolEffect::none();
    };
    let theta = ctx.u1.dot(ctx.u2).clamp(-1.0, 1.0).acos(); // corner opening angle
    let d = radius / (theta * 0.5).tan(); // corner → tangent point distance
    if d >= ctx.len1 - 1e-6 || d >= ctx.len2 - 1e-6 {
        return ToolEffect::none(); // radius too large for these segments
    }

    // Tangent points along each line; arc center on the angle bisector.
    let t1 = Vec2D::from_glam(ctx.corner.to_glam() + ctx.u1 * d);
    let t2 = Vec2D::from_glam(ctx.corner.to_glam() + ctx.u2 * d);
    let bisector = (ctx.u1 + ctx.u2).normalize();
    let center = Vec2D::from_glam(ctx.corner.to_glam() + bisector * (radius / (theta * 0.5).sin()));

    let t1_id = sketch.add_geometry(GeometryElement::Point(Point::new(t1)));
    let t2_id = sketch.add_geometry(GeometryElement::Point(Point::new(t2)));
    let center_id = sketch.add_geometry(GeometryElement::Point(Point::new(center)));

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
    replace_corner(sketch, &ctx, t1_id, t2_id);

    ToolEffect::changed(format!("Fillet r={radius:.2} at corner"))
}

/// Like [`fillet`], but bridges the shortened lines with a straight chamfer
/// line. `length` is the setback from the corner along each line.
pub(super) fn chamfer(
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    length: f32,
) -> ToolEffect {
    if length < 1e-6 {
        return ToolEffect::none();
    }
    let Some(ctx) = corner_under_cursor(sketch, cursor, snap_tol) else {
        return ToolEffect::none();
    };
    if length >= ctx.len1 - 1e-6 || length >= ctx.len2 - 1e-6 {
        return ToolEffect::none(); // setback longer than a segment
    }
    let t1 = Vec2D::from_glam(ctx.corner.to_glam() + ctx.u1 * length);
    let t2 = Vec2D::from_glam(ctx.corner.to_glam() + ctx.u2 * length);
    let t1_id = sketch.add_geometry(GeometryElement::Point(Point::new(t1)));
    let t2_id = sketch.add_geometry(GeometryElement::Point(Point::new(t2)));
    sketch.add_geometry(GeometryElement::Line(Line::new(t1_id, t2_id)));
    replace_corner(sketch, &ctx, t1_id, t2_id);

    ToolEffect::changed(format!("Chamfer {length:.2} at corner"))
}

// ---------------------------------------------------------- intersections

/// A trim/extend-capable curve resolved to positions.
enum Prim {
    Seg { a: Vec2, b: Vec2 },
    Arc { c: Vec2, r: f32, s: Vec2, e: Vec2 },
    Circle { c: Vec2, r: f32 },
}

fn prim_of(sketch: &Sketch, geom: &GeometryElement) -> Option<Prim> {
    match geom {
        GeometryElement::Line(l) => Some(Prim::Seg {
            a: sketch.point_position(l.start)?.to_glam(),
            b: sketch.point_position(l.end)?.to_glam(),
        }),
        GeometryElement::Arc(a) => {
            let c = sketch.point_position(a.center)?.to_glam();
            let s = sketch.point_position(a.start)?.to_glam();
            Some(Prim::Arc {
                c,
                r: (s - c).length(),
                s,
                e: sketch.point_position(a.end)?.to_glam(),
            })
        }
        GeometryElement::Circle(circle) => Some(Prim::Circle {
            c: sketch.point_position(circle.center)?.to_glam(),
            r: circle.radius,
        }),
        _ => None,
    }
}

/// Whether `p` lies within the prim's own extent (segments by parameter,
/// arcs by angular range; circles are unbounded).
fn within(prim: &Prim, p: Vec2) -> bool {
    match *prim {
        Prim::Seg { a, b } => geom2d::on_segment(a, b, p),
        Prim::Arc { c, s, e, .. } => geom2d::point_on_arc(c, s, e, p),
        Prim::Circle { .. } => true,
    }
}

/// Intersections of the *unbounded* carriers of two prims (infinite line /
/// full circle), before any extent filtering.
fn raw_hits(a: &Prim, b: &Prim) -> Vec<Vec2> {
    match (a, b) {
        (Prim::Seg { a: a1, b: a2 }, Prim::Seg { a: b1, b: b2 }) => {
            geom2d::line_line(*a1, *a2, *b1, *b2).into_iter().collect()
        }
        (Prim::Seg { a: a1, b: a2 }, Prim::Arc { c, r, .. })
        | (Prim::Seg { a: a1, b: a2 }, Prim::Circle { c, r })
        | (Prim::Arc { c, r, .. }, Prim::Seg { a: a1, b: a2 })
        | (Prim::Circle { c, r }, Prim::Seg { a: a1, b: a2 }) => {
            geom2d::line_circle(*a1, *a2, *c, *r)
        }
        (
            Prim::Arc { c: c1, r: r1, .. } | Prim::Circle { c: c1, r: r1 },
            Prim::Arc { c: c2, r: r2, .. } | Prim::Circle { c: c2, r: r2 },
        ) => geom2d::circle_circle(*c1, *r1, *c2, *r2),
    }
}

/// Intersections of `target` with every OTHER line/arc/circle in the
/// sketch. `bounded_target` restricts hits to the target's own extent
/// (trim/split); extend wants the unbounded carrier.
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
    out
}

/// Nearest line/arc/circle within `tol` of `pos` (points and other element
/// kinds excluded).
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
/// curve uses it — cleanup for endpoints a trim disconnected.
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

// ------------------------------------------------------------------- trim

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

/// Highlight polyline for the span a trim click at `cursor` would remove
/// (overlay hover preview). `None` when nothing is trimmable there.
pub fn trim_preview(sketch: &Sketch, cursor: Vec2D, tol: f32) -> Option<Vec<Vec2D>> {
    const SAMPLES: usize = 24;
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
    let Some(plan) = plan_trim(sketch, cursor, tol) else {
        return ToolEffect::none();
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
                    if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) {
                        l.end = p_lo;
                    }
                    sketch.add_geometry(GeometryElement::Line(Line::new(p_hi, end_pid)));
                }
                // End-of-line span: shorten and clean up the freed endpoint.
                (Some(lo), None) => {
                    let p_lo = new_point(sketch, pos(lo));
                    if let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) {
                        l.end = p_lo;
                    }
                    drop_if_orphan(sketch, end_pid);
                }
                (None, Some(hi)) => {
                    let p_hi = new_point(sketch, pos(hi));
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

// ----------------------------------------------------------------- extend

pub(super) fn extend(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> ToolEffect {
    let Some(id) = curve_under_cursor(sketch, cursor, tol, false) else {
        return ToolEffect::none();
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
                return ToolEffect::none(); // nothing to extend to
            };
            if let Some(GeometryElement::Point(pt)) = sketch.get_geometry_mut(pid) {
                pt.position = Vec2D::from_glam(new_pos);
            }
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

// ------------------------------------------------------------------ split

pub(super) fn split(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> ToolEffect {
    let Some(id) = curve_under_cursor(sketch, cursor, tol, false) else {
        return ToolEffect::none(); // circles have no split point pair
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
                return ToolEffect::none(); // too close to an endpoint
            }
            let m = Vec2D::from_glam(a + ab * t);
            let m_id = sketch.add_geometry(GeometryElement::Point(Point::new(m)));
            let Some(GeometryElement::Line(l)) = sketch.get_geometry_mut(id) else {
                return ToolEffect::none();
            };
            let old_end = l.end;
            l.end = m_id;
            sketch.add_geometry(GeometryElement::Line(Line::new(m_id, old_end)));
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
            sketch.add_geometry(GeometryElement::Arc(Arc::new(
                center_pid, m_id, old_end, radius,
            )));
            ToolEffect::changed("Split arc")
        }
        Prim::Circle { .. } => ToolEffect::none(),
    }
}

// ----------------------------------------------------------------- offset

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
        .unwrap_or(curves[0].1 .0);
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

pub(super) fn offset(
    sketch: &mut Sketch,
    cursor: Vec2D,
    selected: &HashSet<Uuid>,
    distance: f32,
) -> ToolEffect {
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
                    GeometryElement::Line(_) | GeometryElement::Arc(_) | GeometryElement::Circle(_)
                )
        })
        .cloned()
        .collect();
    // A single circle offsets on its own (concentric copy).
    if let [GeometryElement::Circle(circle)] = curves.as_slice() {
        let Some(c) = sketch.point_position(circle.center) else {
            return ToolEffect::none();
        };
        let outside = (cursor - c).to_glam().length() > circle.radius;
        let new_r = if outside {
            circle.radius + distance
        } else {
            circle.radius - distance
        };
        if new_r < 1e-6 {
            return ToolEffect::none();
        }
        let center = circle.center; // concentric: share the center point
        let flag = sketch.is_construction(circle.id);
        let new_id = sketch.add_geometry(GeometryElement::Circle(crate::sketch::Circle::new(
            center, new_r,
        )));
        sketch.set_construction(new_id, flag);
        return ToolEffect::changed(format!("Offset circle to r={new_r:.2}"));
    }
    if curves.is_empty()
        || curves
            .iter()
            .any(|g| matches!(g, GeometryElement::Circle(_)))
    {
        return ToolEffect::none(); // circles only offset alone
    }

    let ends: Vec<(Uuid, (Uuid, Uuid))> = curves
        .iter()
        .filter_map(|g| chain_ends(g).map(|e| (g.id(), e)))
        .collect();
    let Some((links, closed)) = order_chain(&ends) else {
        return ToolEffect::none(); // not a single connected chain
    };
    let elem_of = |id: Uuid| curves.iter().find(|g| g.id() == id).unwrap();
    let pos_of = |sketch: &Sketch, pid: Uuid| sketch.point_position(pid).map(|p| p.to_glam());

    // Signed left-offset: positive when the click lies left of the chain
    // direction at the nearest link.
    let nearest = links
        .iter()
        .filter_map(|l| snap::distance_to_element(sketch, elem_of(l.id), cursor).map(|d| (l, d)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(l, _)| l);
    let Some(near) = nearest else {
        return ToolEffect::none();
    };
    let left = match elem_of(near.id) {
        GeometryElement::Line(_) => {
            let (Some(a), Some(b)) = (pos_of(sketch, near.entry), pos_of(sketch, near.exit)) else {
                return ToolEffect::none();
            };
            (b - a).perp_dot(cursor.to_glam() - a) > 0.0
        }
        GeometryElement::Arc(arc) => {
            let (Some(c), Some(s)) = (pos_of(sketch, arc.center), pos_of(sketch, arc.start)) else {
                return ToolEffect::none();
            };
            let inside = (cursor.to_glam() - c).length() < (s - c).length();
            // Traversed CCW (entry == start) the left side faces the center.
            if near.entry == arc.start {
                inside
            } else {
                !inside
            }
        }
        _ => return ToolEffect::none(),
    };
    let d_left = if left { distance } else { -distance };

    // Raw offset endpoints + carriers per link.
    let mut prims: Vec<(OffsetPrim, Vec2, Vec2)> = Vec::with_capacity(links.len());
    for link in &links {
        let (Some(a), Some(b)) = (pos_of(sketch, link.entry), pos_of(sketch, link.exit)) else {
            return ToolEffect::none();
        };
        match elem_of(link.id) {
            GeometryElement::Line(_) => {
                let dir = b - a;
                if dir.length() < 1e-6 {
                    return ToolEffect::none();
                }
                let shift = dir.normalize().perp() * d_left;
                prims.push((OffsetPrim::Line, a + shift, b + shift));
            }
            GeometryElement::Arc(arc) => {
                let Some(c) = pos_of(sketch, arc.center) else {
                    return ToolEffect::none();
                };
                let r = (pos_of(sketch, arc.start).unwrap_or(a) - c).length();
                // CCW traversal keeps the center on the left.
                let forward = link.entry == arc.start;
                let new_r = if forward { r - d_left } else { r + d_left };
                if new_r < 1e-6 {
                    return ToolEffect::none(); // arc would invert
                }
                let proj = |q: Vec2| c + (q - c).normalize() * new_r;
                prims.push((OffsetPrim::Arc { c, new_r }, proj(a), proj(b)));
            }
            _ => return ToolEffect::none(),
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
    let mut junction: HashMap<Uuid, Vec2> = HashMap::new();
    for i in 0..links.len() {
        let j = (i + 1) % links.len();
        if j == 0 && !closed {
            break;
        }
        junction.insert(links[i].exit, join(&prims[i], &prims[j]));
    }
    if !closed {
        junction.insert(links[0].entry, prims[0].1);
        junction.insert(links[links.len() - 1].exit, prims[links.len() - 1].2);
    }

    // Materialize: one new point per original junction id (internal sharing
    // preserved), then one offset curve per link.
    let mut new_pts: HashMap<Uuid, Uuid> = HashMap::new();
    for (pid, pos) in &junction {
        let id = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(*pos))));
        new_pts.insert(*pid, id);
    }
    let count = links.len();
    for (link, prim) in links.iter().zip(&prims) {
        let flag = sketch.is_construction(link.id);
        let new_id = match (elem_of(link.id).clone(), &prim.0) {
            (GeometryElement::Line(_), _) => sketch.add_geometry(GeometryElement::Line(Line::new(
                new_pts[&link.entry],
                new_pts[&link.exit],
            ))),
            (GeometryElement::Arc(arc), OffsetPrim::Arc { new_r, .. }) => {
                // Preserve the stored CCW start/end regardless of traversal.
                sketch.add_geometry(GeometryElement::Arc(Arc::new(
                    arc.center,
                    new_pts[&arc.start],
                    new_pts[&arc.end],
                    *new_r,
                )))
            }
            _ => continue,
        };
        sketch.set_construction(new_id, flag);
    }
    ToolEffect::changed(format!("Offset {count} element(s) by {distance:.2}"))
}
