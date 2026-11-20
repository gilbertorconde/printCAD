//! Trim, extend and split of lines, arcs and circles: where other curves
//! cross the one under the cursor, and the piece each tool keeps.

use super::*;

/// Intersections of `target` with every other curve in the sketch, and
/// with the sketch's two axes when `axes` is on, and where the ends of
/// other curves lie on it.
/// `bounded_target` restricts hits to the target's own extent (trim);
/// extend wants the unbounded carrier.
fn hits_with_others(
    sketch: &Sketch,
    target_id: Uuid,
    target: &Prim,
    bounded_target: bool,
    axes: bool,
) -> Vec<DVec2> {
    let mut out = Vec::new();
    if axes {
        // Each axis as the unbounded line through the origin.
        for along in [DVec2::X, DVec2::Y] {
            let axis = Prim::Seg {
                a: DVec2::ZERO,
                b: along,
            };
            out.extend(
                raw_hits(target, &axis)
                    .into_iter()
                    .filter(|p| !bounded_target || within(target, *p)),
            );
        }
    }
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
    if let Some(curve) = sketch
        .get_geometry(target_id)
        .and_then(|g| Curve::of(sketch, g))
    {
        let span = if bounded_target {
            curve.span
        } else {
            curve.reach()
        };
        out.extend(
            ends_on(sketch, target_id, &curve, span)
                .into_iter()
                .map(|t| curve.at(t)),
        );
    }
    out
}

/// Where the ends of the sketch's other curves lie on `target` over
/// `span`, within [`END_SLACK`], as parameters on it: an end touching a
/// curve meets it as a crossing does, as the regions join it.
pub(super) fn ends_on(
    sketch: &Sketch,
    target_id: Uuid,
    target: &Curve,
    span: (f64, f64),
) -> Vec<f64> {
    let bound = if span == target.span {
        target.bound()
    } else {
        None
    };
    sketch
        .geometry
        .iter()
        .filter(|g| g.id() != target_id)
        .filter_map(|g| Curve::of(sketch, g))
        .filter(|c| !c.closed)
        .flat_map(|c| [c.at(c.span.0), c.at(c.span.1)])
        .filter(|e| bound.is_none_or(|(centre, r)| (*e - centre).length() <= r + END_SLACK))
        .filter_map(|e| {
            let t = target.nearest(e, span);
            ((target.at(t) - e).length() <= END_SLACK).then_some(t)
        })
        .collect()
}

/// Nearest line or arc within `tol` of `pos`, circles too with
/// `include_circles` (points and other element kinds excluded).
fn curve_under_cursor(
    sketch: &Sketch,
    pos: Vec2D,
    tol: f64,
    include_circles: bool,
) -> Option<Uuid> {
    sketch
        .geometry
        .iter()
        .filter(|g| {
            matches!(g, GeometryElement::Line(_) | GeometryElement::Arc(_))
                || (include_circles && matches!(g, GeometryElement::Circle(_)))
        })
        .filter_map(|g| crate::shape::distance_to_element(sketch, g, pos).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Remove `point_id` (and constraints referencing it) when no remaining
/// curve uses it: the end a cut left behind.
pub(super) fn drop_if_orphan(sketch: &mut Sketch, point_id: Uuid) {
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
        lo: Option<f64>,
        hi: Option<f64>,
    },
    /// Removed arc span as CCW angles relative to the arc start.
    ArcSpan {
        id: Uuid,
        lo: Option<f64>,
        hi: Option<f64>,
    },
    /// Removed circle span between two absolute angles (CCW lo → hi).
    CircleSpan { id: Uuid, lo: f64, hi: f64 },
}

fn plan_trim(sketch: &Sketch, cursor: Vec2D, tol: f64, axes: bool) -> Option<TrimPlan> {
    let id = curve_under_cursor(sketch, cursor, tol, true)?;
    let prim = prim_of(sketch, sketch.get_geometry(id)?)?;
    let hits = hits_with_others(sketch, id, &prim, true, axes);
    let p = cursor.to_glam();
    match prim {
        Prim::Seg { a, b } => {
            let ab = b - a;
            let len_sq = ab.length_squared();
            let t_of = |q: DVec2| (q - a).dot(ab) / len_sq;
            let ts: Vec<f64> = hits
                .iter()
                .map(|q| t_of(*q))
                .filter(|t| (SPAN_EPS..=1.0 - SPAN_EPS).contains(t))
                .collect();
            let tc = t_of(p).clamp(0.0, 1.0);
            let lo = ts.iter().copied().filter(|t| *t < tc).reduce(f64::max);
            let hi = ts.iter().copied().filter(|t| *t > tc).reduce(f64::min);
            if lo.is_none() && hi.is_none() {
                Some(TrimPlan::RemoveWhole { id })
            } else {
                Some(TrimPlan::LineSpan { id, lo, hi })
            }
        }
        Prim::Arc { c, s, e, .. } => {
            let (start_angle, sweep) = arc_angles(s - c, e - c);
            let eps = (sweep * SPAN_EPS).max(1e-4);
            let rel_of = |q: DVec2| geom2d::wrap_positive((q - c).y.atan2((q - c).x) - start_angle);
            let rels: Vec<f64> = hits
                .iter()
                .map(|q| rel_of(*q))
                .filter(|r| (eps..=sweep - eps).contains(r))
                .collect();
            let rc = rel_of(p).min(sweep);
            let lo = rels.iter().copied().filter(|r| *r < rc).reduce(f64::max);
            let hi = rels.iter().copied().filter(|r| *r > rc).reduce(f64::min);
            if lo.is_none() && hi.is_none() {
                Some(TrimPlan::RemoveWhole { id })
            } else {
                Some(TrimPlan::ArcSpan { id, lo, hi })
            }
        }
        Prim::Circle { c, .. } => {
            let click_ang = (p - c).y.atan2((p - c).x);
            // Neighbors of the click going CW (lo) and CCW (hi).
            let mut lo: Option<(f64, DVec2)> = None; // max rel
            let mut hi: Option<(f64, DVec2)> = None; // min rel
            for q in &hits {
                let ang = (*q - c).y.atan2((*q - c).x);
                let rel = geom2d::wrap_positive(ang - click_ang);
                if !(1e-4..=std::f64::consts::TAU - 1e-4).contains(&rel) {
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
                    let ang = |q: DVec2| (q - c).y.atan2((q - c).x);
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
fn general_stroke_crossings(sketch: &Sketch, a: DVec2, b: DVec2) -> Vec<DVec2> {
    let stroke = Curve::segment(a, b);
    let external = sketch.external_ids();
    sketch
        .geometry
        .iter()
        .filter(|g| !external.contains(&g.id()) && prim_of(sketch, g).is_none())
        .filter_map(|g| Curve::of(sketch, g))
        .flat_map(|curve| {
            crate::curves::crossings(&stroke, stroke.span, &curve, curve.span)
                .into_iter()
                .map(|(t, _)| stroke.at(t))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Highlight polyline for the span a trim click at `cursor` would remove
/// (overlay hover preview). `None` when nothing is trimmable there.
pub fn trim_preview(sketch: &Sketch, cursor: Vec2D, tol: f64, axes: bool) -> Option<Vec<Vec2D>> {
    const SAMPLES: usize = 24;
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return Some(general_preview(sketch, id, &curve, cursor));
    }
    let plan = plan_trim(sketch, cursor, tol, axes)?;
    let sample_arc = |c: DVec2, r: f64, from: f64, sweep: f64| -> Vec<Vec2D> {
        (0..=SAMPLES)
            .map(|i| {
                let a = from + sweep * (i as f64 / SAMPLES as f64);
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
            Prim::Circle { c, r } => Some(sample_arc(c, r, 0.0, std::f64::consts::TAU)),
        },
        TrimPlan::LineSpan { id, lo, hi } => {
            let Prim::Seg { a, b } = prim_of(sketch, sketch.get_geometry(id)?)? else {
                return None;
            };
            let pos = |t: f64| Vec2D::from_glam(a + (b - a) * t);
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

/// Trim at the click, the sketch's axes cutting too when `axes` is on:
/// an end the trim leaves on an axis is held there.
pub(crate) fn trim(sketch: &mut Sketch, cursor: Vec2D, tol: f64, axes: bool) -> ToolEffect {
    if !axes {
        return trim_at(sketch, cursor, tol, false);
    }
    let before: HashSet<Uuid> = sketch.geometry.iter().map(GeometryElement::id).collect();
    let effect = trim_at(sketch, cursor, tol, true);
    let made: Vec<Uuid> = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !before.contains(id))
        .collect();
    hold_on_axes(sketch, &made);
    effect
}

fn trim_at(sketch: &mut Sketch, cursor: Vec2D, tol: f64, axes: bool) -> ToolEffect {
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return trim_general(sketch, id, &curve, cursor);
    }
    let Some(plan) = plan_trim(sketch, cursor, tol, axes) else {
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
            let pos = |t: f64| Vec2D::from_glam(a.to_glam() + (b.to_glam() - a.to_glam()) * t);
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
            let pos = |rel: f64| {
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
            let pos = |a: f64| Vec2D::new(c.x + radius * a.cos(), c.y + radius * a.sin());
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

/// Extend the end nearest the click to the next curve it meets, or the
/// next axis when `axes` is on: an end that lands on an axis is held
/// there.
pub(crate) fn extend(sketch: &mut Sketch, cursor: Vec2D, tol: f64, axes: bool) -> ToolEffect {
    let curve = axes
        .then(|| curve_under_cursor(sketch, cursor, tol, false))
        .flatten();
    let effect = extend_to(sketch, cursor, tol, axes);
    if let Some(curve) = curve
        && let Some(ends) = sketch.get_geometry(curve).map(Sketch::curve_point_ids)
    {
        hold_on_axes(sketch, &ends);
    }
    effect
}

/// Hold each of `points` that lies on a sketch axis, and is not held to
/// it already, on that axis. Ids that are not points are passed over.
fn hold_on_axes(sketch: &mut Sketch, points: &[Uuid]) {
    const ON_AXIS: f64 = 1e-4;
    for &point in points {
        let Some(GeometryElement::Point(p)) = sketch.get_geometry(point) else {
            continue;
        };
        let at = p.position;
        for (line, off) in [(X_AXIS_ID, at.y), (Y_AXIS_ID, at.x)] {
            let held = sketch.constraints.iter().any(|c| {
                matches!(c.kind, ConstraintKind::PointOnLine { point: q, line: l } if q == point && l == line)
            });
            if off.abs() <= ON_AXIS && !held {
                sketch.add_constraint(ConstraintKind::PointOnLine { point, line });
            }
        }
    }
}

fn extend_to(sketch: &mut Sketch, cursor: Vec2D, tol: f64, axes: bool) -> ToolEffect {
    if let Some((id, curve)) = general_under_cursor(sketch, cursor, tol) {
        return extend_general(sketch, id, &curve, cursor);
    }
    let Some(id) = curve_under_cursor(sketch, cursor, tol, false) else {
        return ToolEffect::log("Click near the end of a line or an arc to extend");
    };
    let Some(prim) = prim_of(sketch, sketch.get_geometry(id).unwrap()) else {
        return ToolEffect::none();
    };
    let hits = hits_with_others(sketch, id, &prim, false, axes);
    let p = cursor.to_glam();
    match prim {
        Prim::Seg { a, b } => {
            let Some(GeometryElement::Line(l)) = sketch.get_geometry(id) else {
                return ToolEffect::none();
            };
            let (start_pid, end_pid) = (l.start, l.end);
            let ab = b - a;
            let t_of = |q: DVec2| (q - a).dot(ab) / ab.length_squared();
            // The clicked half decides which endpoint grows.
            let target = if t_of(p) >= 0.5 {
                let t = hits
                    .iter()
                    .map(|q| t_of(*q))
                    .filter(|t| *t > 1.0 + SPAN_EPS)
                    .reduce(f64::min);
                t.map(|t| (end_pid, a + ab * t))
            } else {
                let t = hits
                    .iter()
                    .map(|q| t_of(*q))
                    .filter(|t| *t < -SPAN_EPS)
                    .reduce(f64::max);
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
            let rel_of = |q: DVec2| geom2d::wrap_positive((q - c).y.atan2((q - c).x) - start_angle);
            let outside: Vec<f64> = hits
                .iter()
                .map(|q| rel_of(*q))
                .filter(|rel| *rel > sweep + 1e-3 && *rel < std::f64::consts::TAU - 1e-3)
                .collect();
            let rel_click = rel_of(p).min(sweep);
            // Nearer endpoint half; the end grows CCW, the start grows CW.
            let target = if rel_click >= sweep * 0.5 {
                outside
                    .iter()
                    .copied()
                    .reduce(f64::min)
                    .map(|rel| (end_pid, rel))
            } else {
                outside
                    .iter()
                    .copied()
                    .reduce(f64::max)
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

pub(crate) fn split(sketch: &mut Sketch, cursor: Vec2D, tol: f64) -> ToolEffect {
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
            let t = core_document::mesh_query::segment_param(p, a, b);
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
fn split_conic(sketch: &mut Sketch, cursor: Vec2D, tol: f64) -> Option<ToolEffect> {
    let distance = |g: &GeometryElement| crate::shape::distance_to_element(sketch, g, cursor);
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
    let t = shape.param([cursor.x, cursor.y]);
    let (lo, hi) = (t0.min(t1), t0.max(t1));
    let eps = (hi - lo) * SPAN_EPS;
    if !(lo + eps..=hi - eps).contains(&t) {
        return Some(ToolEffect::log("Too close to an end to split"));
    }
    let [x, y] = shape.point(t);
    let middle = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
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
