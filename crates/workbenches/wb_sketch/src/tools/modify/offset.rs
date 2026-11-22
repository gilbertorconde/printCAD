//! Offset: the chains the selection forms, each copied to one side by a
//! distance, joined at the corners or sampled where no exact offset exists.

use super::*;
use kernel_api::KernelQueries;

/// One curve of the selection, oriented along the chain walk.
struct ChainLink {
    id: Uuid,
    entry: Uuid,
    exit: Uuid,
}

/// Endpoint ids of a chainable curve (lines and arcs).
pub(super) fn chain_ends(geom: &GeometryElement) -> Option<(Uuid, Uuid)> {
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
    Arc { c: DVec2, new_r: f64 },
}

/// How the offset tool makes its copies.
#[derive(Clone, Copy)]
pub(crate) struct OffsetOptions {
    pub distance: f64,
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

/// Which side of a curve its copy goes, before `flip` turns it over.
#[derive(Clone, Copy)]
enum Side {
    /// The side the click is on.
    Toward(Vec2D),
    /// Out of the closed curve (`true`) or into it.
    Outward(bool),
}

/// One piece of the selection the offset copies on its own: a circle, a
/// curve sampled into a spline, or a chain of lines and arcs end to end.
enum OffsetGroup {
    Circle(crate::sketch::Circle),
    Sampled(GeometryElement),
    Chain(Vec<GeometryElement>),
}

impl OffsetGroup {
    fn elements(&self) -> Vec<GeometryElement> {
        match self {
            OffsetGroup::Circle(c) => vec![GeometryElement::Circle(c.clone())],
            OffsetGroup::Sampled(g) => vec![g.clone()],
            OffsetGroup::Chain(links) => links.clone(),
        }
    }

    fn offset(
        &self,
        sketch: &mut Sketch,
        side: Side,
        distance: f64,
        flip: bool,
        round: bool,
        kernel: Option<&dyn KernelQueries>,
    ) -> Option<OffsetMade> {
        match self {
            OffsetGroup::Circle(c) => offset_circle(sketch, c, side, distance, flip),
            OffsetGroup::Sampled(g) => offset_sampled(sketch, g.id(), side, distance, flip),
            OffsetGroup::Chain(links) => {
                offset_chain(sketch, links, side, distance, flip, round, kernel)
            }
        }
    }

    /// The outline it closes, in the order it runs, or `None` when it
    /// does not close.
    fn outline(&self, sketch: &Sketch) -> Option<Vec<DVec2>> {
        let samples = |geom: &GeometryElement| {
            crate::external_ref::element_samples(sketch, geom)
                .map(|s| s.into_iter().map(Vec2D::to_glam).collect())
        };
        match self {
            OffsetGroup::Circle(c) => samples(&GeometryElement::Circle(c.clone())),
            OffsetGroup::Sampled(geom) => match geom {
                GeometryElement::Ellipse(e) if e.arc.is_none() => samples(geom),
                GeometryElement::BSpline(b) if b.periodic => samples(geom),
                _ => None,
            },
            OffsetGroup::Chain(links) => {
                let ends: Vec<(Uuid, (Uuid, Uuid))> = links
                    .iter()
                    .filter_map(|g| chain_ends(g).map(|e| (g.id(), e)))
                    .collect();
                match order_chain(&ends)? {
                    (order, true) => chain_outline(sketch, links, &order),
                    _ => None,
                }
            }
        }
    }
}

/// The points a chain passes through, in the order it runs: each link's
/// entry, then an arc's points between its ends.
fn chain_outline(
    sketch: &Sketch,
    curves: &[GeometryElement],
    links: &[ChainLink],
) -> Option<Vec<DVec2>> {
    let mut points = Vec::new();
    for link in links {
        points.push(sketch.point_position(link.entry)?.to_glam());
        let geom = curves.iter().find(|g| g.id() == link.id)?;
        if let GeometryElement::Arc(arc) = geom {
            let mut along: Vec<DVec2> = crate::external_ref::element_samples(sketch, geom)?
                .into_iter()
                .map(Vec2D::to_glam)
                .collect();
            if link.entry != arc.start {
                along.reverse();
            }
            let inner = along.len().saturating_sub(1);
            points.extend(along.into_iter().take(inner).skip(1));
        }
    }
    Some(points)
}

/// Twice the area a closed outline encloses, positive when it runs
/// counter-clockwise.
fn signed_area(outline: &[DVec2]) -> f64 {
    let n = outline.len();
    (0..n)
        .map(|i| outline[i].perp_dot(outline[(i + 1) % n]))
        .sum()
}

/// Whether `p` lies inside a closed outline (even-odd crossings).
fn inside(outline: &[DVec2], p: DVec2) -> bool {
    let n = outline.len();
    let mut within = false;
    for i in 0..n {
        let (a, b) = (outline[i], outline[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            within = !within;
        }
    }
    within
}

/// Whether the left of a closed outline, as it runs, is out of it when
/// `outward`, or into it when not.
fn left_is(outline: &[DVec2], outward: bool) -> bool {
    (signed_area(outline) < 0.0) == outward
}

/// The selection split into what is copied on its own: each circle and
/// sampled curve, and each run of lines and arcs sharing ends.
fn offset_groups(curves: &[GeometryElement]) -> Vec<OffsetGroup> {
    let mut groups = Vec::new();
    let mut chained: Vec<&GeometryElement> = Vec::new();
    for g in curves {
        match g {
            GeometryElement::Circle(c) => groups.push(OffsetGroup::Circle(c.clone())),
            GeometryElement::Line(_) | GeometryElement::Arc(_) => chained.push(g),
            other => groups.push(OffsetGroup::Sampled(other.clone())),
        }
    }
    let mut by_point: HashMap<Uuid, Vec<usize>> = HashMap::new();
    for (i, g) in chained.iter().enumerate() {
        if let Some((a, b)) = chain_ends(g) {
            by_point.entry(a).or_default().push(i);
            by_point.entry(b).or_default().push(i);
        }
    }
    let mut seen = vec![false; chained.len()];
    for first in 0..chained.len() {
        if seen[first] {
            continue;
        }
        seen[first] = true;
        let mut chain = vec![first];
        let mut next = 0;
        while let Some(&i) = chain.get(next) {
            next += 1;
            let Some((a, b)) = chain_ends(chained[i]) else {
                continue;
            };
            for &j in [a, b].iter().flat_map(|p| &by_point[p]) {
                if !seen[j] {
                    seen[j] = true;
                    chain.push(j);
                }
            }
        }
        chain.sort_unstable();
        groups.push(OffsetGroup::Chain(
            chain.into_iter().map(|i| chained[i].clone()).collect(),
        ));
    }
    groups
}

/// Copy each selected curve or chain `options.distance` off it. One goes
/// to the side of the click; of several, the closed ones all grow or all
/// shrink as the one nearest the click does, and one that does not close
/// takes the click's side. A closed chain is offset by `kernel` when it
/// can, which splits a copy that would cross itself into the loops that
/// stand the whole distance off.
pub(crate) fn offset(
    sketch: &mut Sketch,
    cursor: Vec2D,
    selected: &HashSet<Uuid>,
    options: OffsetOptions,
    kernel: Option<&dyn KernelQueries>,
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
    let groups = offset_groups(&curves);
    let outlines: Vec<Option<Vec<DVec2>>> = groups.iter().map(|g| g.outline(sketch)).collect();
    let nearest_closed = groups
        .iter()
        .zip(&outlines)
        .filter_map(|(g, outline)| {
            let near = g
                .elements()
                .iter()
                .filter_map(|e| crate::shape::distance_to_element(sketch, e, cursor))
                .min_by(f64::total_cmp)?;
            Some((near, outline.as_ref()?))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, outline)| outline);
    let sides: Vec<Side> = outlines
        .iter()
        .map(|outline| match nearest_closed {
            Some(nearest) if groups.len() > 1 && outline.is_some() => {
                Side::Outward(!inside(nearest, cursor.to_glam()))
            }
            _ => Side::Toward(cursor),
        })
        .collect();
    let flips: &[bool] = if options.both {
        &[false, true]
    } else {
        &[false]
    };
    let mut made: Vec<OffsetMade> = Vec::new();
    let mut left_out = 0;
    for &flip in flips {
        let mut pairs = Vec::new();
        let mut any = false;
        for (group, side) in groups.iter().zip(&sides) {
            match group.offset(sketch, *side, distance, flip, options.round, kernel) {
                Some(copied) => {
                    any = true;
                    pairs.extend(copied.pairs);
                }
                None => left_out += 1,
            }
        }
        if any {
            made.push(OffsetMade { pairs });
        }
    }
    if made.is_empty() {
        return ToolEffect::log(
            "Select circles, ellipses, splines or conics, or chains of lines and arcs",
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
    let mut said = format!("Offset {count} element(s) by {distance:.2}");
    if left_out > 0 {
        said.push_str(&format!(
            "; {left_out} left out (branching, or too small to shrink)"
        ));
    }
    ToolEffect::changed(said)
}

/// A concentric copy of a circle on its `side` (the other with `flip`).
fn offset_circle(
    sketch: &mut Sketch,
    circle: &crate::sketch::Circle,
    side: Side,
    distance: f64,
    flip: bool,
) -> Option<OffsetMade> {
    let c = sketch.point_position(circle.center)?;
    let outside = match side {
        Side::Toward(cursor) => (cursor - c).to_glam().length() > circle.radius,
        Side::Outward(out) => out,
    } != flip;
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
    side: Side,
    distance: f64,
    flip: bool,
) -> Option<OffsetMade> {
    let geom = sketch.get_geometry(id)?.clone();
    let closed = match &geom {
        GeometryElement::Ellipse(e) => e.arc.is_none(),
        GeometryElement::BSpline(b) => b.periodic,
        _ => false,
    };
    let mut samples: Vec<DVec2> = crate::external_ref::element_samples(sketch, &geom)?
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
    // Central differences inside; at an open curve's ends the one-sided
    // difference of second order (−3p₀ + 4p₁ − p₂), as accurate as the
    // central one, so the copy's ends stand square off the curve's.
    let tangent = |i: usize| {
        let v = if closed {
            samples[(i + 1) % n] - samples[(i + n - 1) % n]
        } else if i == 0 {
            -3.0 * samples[0] + 4.0 * samples[1] - samples[2]
        } else if i == n - 1 {
            3.0 * samples[n - 1] - 4.0 * samples[n - 2] + samples[n - 3]
        } else {
            samples[i + 1] - samples[i - 1]
        };
        v.normalize_or_zero()
    };
    let left = match side {
        Side::Toward(cursor) => {
            let p = cursor.to_glam();
            let nearest = (0..n).min_by(|a, b| {
                (samples[*a] - p)
                    .length()
                    .total_cmp(&(samples[*b] - p).length())
            })?;
            tangent(nearest).perp().dot(p - samples[nearest]) > 0.0
        }
        Side::Outward(out) => left_is(&samples, out),
    } != flip;
    let d = if left { distance } else { -distance };
    // A whole step keeps the picks as even in the curve's parameter as the
    // samples are. A closed fit sets its points at even parameters, and
    // uneven picks (a step of 1.5 takes gaps of one and two samples in turn)
    // would make the copy wave about the true parallel.
    let step = (n / OFFSET_SAMPLES).max(1);
    let mut picks: Vec<usize> = (0..n).step_by(step).collect();
    if !closed && picks.last() != Some(&(n - 1)) {
        picks.push(n - 1);
    }
    let points: Vec<[f64; 2]> = picks
        .iter()
        .map(|&i| samples[i] + tangent(i).perp() * d)
        .map(|q| [q.x, q.y])
        .collect();
    let fit = sketch_solver::spline::interpolate(&points, 3, closed)?;
    let control: Vec<Uuid> = fit
        .control
        .iter()
        .map(|q| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(q[0], q[1])))))
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
) -> Option<DVec2> {
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

/// Whether the click lies left of a chain as it runs, at the link
/// nearest it.
fn click_left_of(
    sketch: &Sketch,
    curves: &[GeometryElement],
    links: &[ChainLink],
    cursor: Vec2D,
) -> Option<bool> {
    let elem_of = |id: Uuid| curves.iter().find(|g| g.id() == id).unwrap();
    let pos_of = |sketch: &Sketch, pid: Uuid| sketch.point_position(pid).map(|p| p.to_glam());
    let near = links
        .iter()
        .filter_map(|l| {
            crate::shape::distance_to_element(sketch, elem_of(l.id), cursor).map(|d| (l, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(l, _)| l)?;
    Some(match elem_of(near.id) {
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
    })
}

/// Copies of a chain of lines and arcs, joined where they meet: a closed
/// chain's by `kernel` when it offsets it, the rest here.
fn offset_chain(
    sketch: &mut Sketch,
    curves: &[GeometryElement],
    side: Side,
    distance: f64,
    flip: bool,
    round: bool,
    kernel: Option<&dyn KernelQueries>,
) -> Option<OffsetMade> {
    let ends: Vec<(Uuid, (Uuid, Uuid))> = curves
        .iter()
        .filter_map(|g| chain_ends(g).map(|e| (g.id(), e)))
        .collect();
    let (links, closed) = order_chain(&ends)?;
    let elem_of = |id: Uuid| curves.iter().find(|g| g.id() == id).unwrap();
    let pos_of = |sketch: &Sketch, pid: Uuid| sketch.point_position(pid).map(|p| p.to_glam());

    let left = match side {
        Side::Toward(cursor) => click_left_of(sketch, curves, &links, cursor)?,
        Side::Outward(out) => left_is(&chain_outline(sketch, curves, &links)?, out),
    };
    let d_left = if left != flip { distance } else { -distance };
    if closed
        && let Some(kernel) = kernel
        && let Some(made) = kernel_chain(sketch, curves, &links, d_left, round, kernel)
    {
        return Some(made);
    }

    // Raw offset endpoints + carriers per link.
    let mut prims: Vec<(OffsetPrim, DVec2, DVec2)> = Vec::with_capacity(links.len());
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
                let proj = |q: DVec2| c + (q - c).normalize() * new_r;
                prims.push((OffsetPrim::Arc { c, new_r }, proj(a), proj(b)));
            }
            _ => return None,
        }
    }

    // Join consecutive offsets at their carrier intersection (nearest
    // candidate to the raw corner; the raw midpoint as a fallback).
    let join = |ap: &(OffsetPrim, DVec2, DVec2), bp: &(OffsetPrim, DVec2, DVec2)| -> DVec2 {
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
    let new_point = |sketch: &mut Sketch, p: DVec2| {
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

/// A closed chain's copy as the kernel offsets it, `d_left` to the left of
/// the way it runs: each piece a line or arc between points the pieces
/// either side share, a copy's arc about its original's centre and a
/// rounded corner's about the corner, tangent to the copies beside it.
/// `None` when the kernel refuses it.
fn kernel_chain(
    sketch: &mut Sketch,
    curves: &[GeometryElement],
    links: &[ChainLink],
    d_left: f64,
    round: bool,
    kernel: &dyn KernelQueries,
) -> Option<OffsetMade> {
    use kernel_api::{OffsetFrom, ProfileSegment, ProfileWire};
    let elem_of = |id: Uuid| curves.iter().find(|g| g.id() == id);
    let pos = |sketch: &Sketch, id: Uuid| sketch.point_position(id).map(Vec2D::to_glam);
    let xy = |p: DVec2| [p.x, p.y];
    let mut segments = Vec::with_capacity(links.len());
    for link in links {
        let (a, b) = (pos(sketch, link.entry)?, pos(sketch, link.exit)?);
        segments.push(match elem_of(link.id)? {
            GeometryElement::Line(_) => ProfileSegment::Line {
                start: xy(a),
                end: xy(b),
            },
            GeometryElement::Arc(arc) => {
                let c = pos(sketch, arc.center)?;
                let (from, sweep) =
                    geom2d::arc_angles(pos(sketch, arc.start)? - c, pos(sketch, arc.end)? - c);
                let half = from + sweep * 0.5;
                ProfileSegment::Arc {
                    start: xy(a),
                    mid: xy(c + DVec2::new(half.cos(), half.sin()) * arc.radius),
                    end: xy(b),
                }
            }
            _ => return None,
        });
    }
    // The kernel moves a positive distance out of the loop.
    let left_out = left_is(&chain_outline(sketch, curves, links)?, true);
    let outward = if left_out { d_left } else { -d_left };
    let loops = kernel
        .offset_wire_2d(&ProfileWire::new(segments), outward, round)
        .ok()?;
    let glam = |p: [f64; 2]| DVec2::new(p[0], p[1]);
    let starts = |ring: &[kernel_api::OffsetPiece]| -> Option<Vec<DVec2>> {
        ring.iter()
            .map(|piece| match piece.segment {
                ProfileSegment::Line { start, .. } | ProfileSegment::Arc { start, .. } => {
                    Some(glam(start))
                }
                _ => None,
            })
            .collect()
    };
    let rings: Vec<Vec<DVec2>> = loops.iter().map(|r| starts(r)).collect::<Option<_>>()?;
    let mut pairs = Vec::new();
    for (ring, at) in loops.iter().zip(&rings) {
        let ends: Vec<Uuid> = at
            .iter()
            .map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(*p)))))
            .collect();
        let n = ring.len();
        let mut made: Vec<(Uuid, OffsetFrom)> = Vec::with_capacity(n);
        for (k, piece) in ring.iter().enumerate() {
            let (a, b) = (ends[k], ends[(k + 1) % n]);
            let id = match (&piece.segment, piece.from) {
                (ProfileSegment::Line { .. }, OffsetFrom::Segment(_)) => {
                    sketch.add_geometry(GeometryElement::Line(Line::new(a, b)))
                }
                (ProfileSegment::Arc { start, mid, end }, from) => {
                    let centre = match from {
                        OffsetFrom::Segment(i) => match elem_of(links.get(i)?.id)? {
                            GeometryElement::Arc(arc) => arc.center,
                            _ => return None,
                        },
                        OffsetFrom::Corner(i) => links.get(i)?.exit,
                    };
                    let (s, m, e) = (glam(*start), glam(*mid), glam(*end));
                    let radius = (s - pos(sketch, centre)?).length();
                    let (start, end) = if (m - s).perp_dot(e - m) > 0.0 {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, start, end, radius)))
                }
                _ => return None,
            };
            if let OffsetFrom::Segment(i) = piece.from {
                let original = links.get(i)?.id;
                let flag = sketch.is_construction(original);
                sketch.set_construction(id, flag);
                pairs.push([original, id]);
            }
            made.push((id, piece.from));
        }
        for (k, &(arc, from)) in made.iter().enumerate() {
            if !matches!(from, OffsetFrom::Corner(_)) {
                continue;
            }
            for &(copy, beside) in [made[(k + n - 1) % n], made[(k + 1) % n]].iter() {
                if matches!(beside, OffsetFrom::Segment(_)) {
                    sketch.add_constraint(ConstraintKind::Tangent {
                        line_or_circle1: copy,
                        item2: arc,
                    });
                }
            }
        }
    }
    Some(OffsetMade { pairs })
}
