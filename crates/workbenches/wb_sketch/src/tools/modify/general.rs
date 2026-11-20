//! Trim, extend and split of the general curves (ellipses, conics and
//! splines), cut where they are sampled.

use super::*;

/// The curve nearest `pos` within `tol`, of any kind.
fn nearest_curve(sketch: &Sketch, pos: Vec2D, tol: f64) -> Option<Uuid> {
    sketch
        .geometry
        .iter()
        .filter(|g| !matches!(g, GeometryElement::Point(_)))
        .filter_map(|g| crate::shape::distance_to_element(sketch, g, pos).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The curve under the cursor when it is one only the general path cuts:
/// an ellipse or arc of one, a parabola or hyperbola, a spline.
pub(super) fn general_under_cursor(sketch: &Sketch, pos: Vec2D, tol: f64) -> Option<(Uuid, Curve)> {
    let id = nearest_curve(sketch, pos, tol)?;
    let geom = sketch.get_geometry(id)?;
    if prim_of(sketch, geom).is_some() {
        return None;
    }
    Some((id, Curve::of(sketch, geom)?))
}

/// Where `target` (over `span`) crosses every other curve of the sketch,
/// or another's end lies on it, as parameters on it.
fn general_hits(sketch: &Sketch, id: Uuid, target: &Curve, span: (f64, f64)) -> Vec<f64> {
    let mut hits: Vec<f64> = sketch
        .geometry
        .iter()
        .filter(|g| g.id() != id)
        .filter_map(|g| Curve::of(sketch, g))
        .flat_map(|other| {
            crate::curves::crossings(target, span, &other, other.span)
                .into_iter()
                .map(|(t, _)| t)
        })
        .collect();
    hits.extend(ends_on(sketch, id, target, span));
    // A spline's loop bounds its own trim: the crossing cuts both passes.
    if matches!(sketch.get_geometry(id), Some(GeometryElement::BSpline(_))) {
        for (a, b) in crate::curves::self_crossings(target, span) {
            hits.extend([a, b]);
        }
    }
    hits
}

/// A general curve's crossings as sketch points, for the line, arc and
/// circle paths that meet one.
pub(super) fn general_points(sketch: &Sketch, target_id: Uuid, bounded: bool) -> Vec<DVec2> {
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
                .map(|(t, _)| target.at(t))
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
    let p = glam::DVec2::new(cursor.x, cursor.y);
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
    let eps = (s1 - s0) * SPAN_EPS;
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
            Vec2D::new(p.x, p.y)
        })
        .collect()
}

pub(super) fn general_preview(
    sketch: &Sketch,
    id: Uuid,
    curve: &Curve,
    cursor: Vec2D,
) -> Vec<Vec2D> {
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
    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p.x, p.y))))
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
        .map(|p| sketch.point_position(*p).map(|v| [v.x, v.y]))
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
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(q[0], q[1]))))
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

pub(super) fn trim_general(
    sketch: &mut Sketch,
    id: Uuid,
    curve: &Curve,
    cursor: Vec2D,
) -> ToolEffect {
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

pub(super) fn extend_general(
    sketch: &mut Sketch,
    id: Uuid,
    curve: &Curve,
    cursor: Vec2D,
) -> ToolEffect {
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
    let p = glam::DVec2::new(cursor.x, cursor.y);
    let tc = curve.nearest(p, curve.span);
    let at_high = tc >= (s0 + s1) * 0.5;
    let reach = curve.reach();
    let hits = general_hits(sketch, id, curve, reach);
    let eps = (s1 - s0) * SPAN_EPS;
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
        pt.position = Vec2D::new(q.x, q.y);
    }
    curve_resized(sketch, id);
    ToolEffect::changed("Extended arc to intersection")
}

pub(super) fn split_general(
    sketch: &mut Sketch,
    id: Uuid,
    curve: &Curve,
    cursor: Vec2D,
) -> ToolEffect {
    let p = glam::DVec2::new(cursor.x, cursor.y);
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
    let eps = (s1 - s0) * SPAN_EPS;
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
