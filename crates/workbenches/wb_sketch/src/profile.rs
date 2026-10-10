//! Extraction of closed profile wires from sketch geometry, for
//! consumption by solid-modeling features (pad/pocket).
//!
//! Curves are stitched into loops through their *shared point ids*: the
//! sketcher's endpoint snapping reuses point elements, so a visually closed
//! profile is topologically closed here with no coincidence tolerance.

use std::collections::HashMap;

use kernel_api::{ProfilePlane, ProfileSegment, ProfileWire};
use uuid::Uuid;

use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use crate::snap::arc_angles;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// The sketch has no closed geometry to extrude.
    Empty,
    /// A curve endpoint is used by only one curve: the loop never closes.
    OpenAt(Uuid),
    /// More than two curves meet at one point; the loop is ambiguous.
    BranchingAt(Uuid),
    /// A curve references a missing point element.
    MissingPoint(Uuid),
    /// The sketch's constraints are not met: its curves are not where
    /// they say.
    Unsolved,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Empty => write!(f, "sketch contains no closed profile"),
            ProfileError::OpenAt(_) => write!(f, "profile is not closed (open endpoint)"),
            ProfileError::BranchingAt(_) => {
                write!(f, "profile branches (more than two curves share a point)")
            }
            ProfileError::MissingPoint(_) => write!(f, "curve references a missing point"),
            ProfileError::Unsolved => {
                write!(f, "the sketch does not solve: its constraints conflict")
            }
        }
    }
}

fn v2(p: Vec2D) -> [f64; 2] {
    [p.x as f64, p.y as f64]
}

/// Mid-point of the CCW arc from `start` to `end` around `center`.
fn arc_midpoint(center: Vec2D, start: Vec2D, end: Vec2D) -> Vec2D {
    let sv = (start - center).to_glam();
    let ev = (end - center).to_glam();
    let radius = sv.length();
    let (start_angle, sweep) = arc_angles(sv, ev);
    let mid_angle = start_angle + sweep * 0.5;
    Vec2D::new(
        center.x + radius * mid_angle.cos(),
        center.y + radius * mid_angle.sin(),
    )
}

/// A curve edge in the endpoint graph.
struct EdgeCurve {
    /// Endpoint point ids (start, end).
    ends: (Uuid, Uuid),
    segment: ProfileSegment,
    /// The name of the element it was drawn from.
    name: kernel_api::TopoName,
}

/// The name a segment drawn from element `id` carries: the faces a feature
/// sweeps from it are named by it, whatever its dimensions.
fn element_name(id: Uuid) -> kernel_api::TopoName {
    kernel_api::naming::name_of_id(id.as_bytes())
}

/// Convert the world-space sketch plane into the kernel's profile plane.
pub fn plane_of(plane: &SketchPlane) -> ProfilePlane {
    ProfilePlane {
        origin: plane.origin.map(f64::from),
        x_axis: plane.x_axis.map(f64::from),
        y_axis: plane.y_axis.map(f64::from),
        normal: plane.normal.map(f64::from),
    }
}

/// How near two projected ends are one end, in sketch units (mm): a
/// solid's edges meet at a vertex only within its tolerance, and a sketch
/// keeps points in single precision, a few millionths of a millimetre
/// apart at part sizes.
const PROJECTED_JOIN: f64 = 1e-4;

/// A sketch feature's closed loops on its plane, in its body's frame, as
/// its formulas leave it; `None` for a feature that is not a sketch or a
/// sketch that closes nothing. What workbench packages read as a
/// feature's profile.
pub fn closed_profile(
    document: &core_document::Document,
    id: core_document::FeatureId,
) -> Option<kernel_api::Profile> {
    use core_document::WorkbenchFeature;
    let node = document.get_feature_meta(id)?;
    if node.workbench_id.as_str() != "wb.sketch" {
        return None;
    }
    let data = document.feature_values(id).unwrap_or(&node.data);
    let feature = crate::SketchFeature::from_json(data).ok()?;
    let wires = extract_wires(&feature.sketch).ok()?;
    Some(kernel_api::Profile {
        plane: plane_of(&feature.plane),
        wires,
    })
}

/// Extract every closed wire from the sketch. Standalone points are
/// ignored; circles are closed wires by themselves; lines/arcs must form
/// closed loops via shared endpoints, and those that do not are left out.
pub fn extract_wires(sketch: &Sketch) -> Result<Vec<ProfileWire>, ProfileError> {
    let (mut wires, edges) = curves(sketch)?;
    let touching = endpoint_graph(&edges);
    // What does not close is left out: a curve with a loose end goes, and
    // then whatever that leaves loose, until only loops remain. A stray
    // line or a spur off a loop does not stop the loops being a profile.
    let mut used = vec![false; edges.len()];
    let mut loose: Option<Uuid> = None;
    loop {
        let dangling: Vec<(Uuid, usize)> = touching
            .iter()
            .filter_map(|(point, list)| {
                let live: Vec<usize> = list.iter().copied().filter(|&i| !used[i]).collect();
                (live.len() == 1).then(|| (*point, live[0]))
            })
            .collect();
        if dangling.is_empty() {
            break;
        }
        for (point, edge) in dangling {
            loose.get_or_insert(point);
            used[edge] = true;
        }
    }
    for (point, list) in &touching {
        if list.iter().filter(|&&i| !used[i]).count() > 2 {
            return Err(ProfileError::BranchingAt(*point));
        }
    }
    if used.iter().all(|&u| u) && wires.is_empty() {
        return Err(loose.map_or(ProfileError::Empty, ProfileError::OpenAt));
    }

    // Walk loops: every vertex left has degree exactly 2, so each unvisited
    // edge starts a unique cycle.
    for start_idx in 0..edges.len() {
        if used[start_idx] {
            continue;
        }
        let mut segments = Vec::new();
        let mut names = Vec::new();
        let start_point = edges[start_idx].ends.0;
        let mut current_point = start_point;
        let mut current_idx = start_idx;
        loop {
            used[current_idx] = true;
            let edge = &edges[current_idx];
            // Each segment runs the way the walk goes: a curve drawn from
            // its other end (an arc turning clockwise round the loop) is
            // turned about.
            segments.push(if edge.ends.0 == current_point {
                edge.segment.clone()
            } else {
                reversed(&edge.segment)
            });
            names.push(edge.name);
            current_point = if edge.ends.0 == current_point {
                edge.ends.1
            } else {
                edge.ends.0
            };
            if current_point == start_point {
                break;
            }
            // Exactly one other unused edge touches this point (degree 2).
            let next = touching[&current_point].iter().copied().find(|&i| !used[i]);
            match next {
                Some(i) => current_idx = i,
                // Degree checks above make this unreachable, but never trust
                // an invariant with a panic in production code.
                None => return Err(ProfileError::OpenAt(current_point)),
            }
        }
        // A loop that encloses nothing (a line and its copy, a line of no
        // length) is no region to build.
        if !encloses_nothing(&segments) {
            wires.push(ProfileWire { segments, names });
        }
    }

    if wires.is_empty() {
        return Err(loose.map_or(ProfileError::Empty, ProfileError::OpenAt));
    }
    Ok(wires)
}

/// Every chain of the sketch's curves, open or closed, for surfaces: an
/// open chain runs from one loose end to the other, a closed one round its
/// loop; a self-closing curve is a chain by itself. A point where three
/// curves or more meet is refused, as no single chain passes through it.
pub fn extract_chains(sketch: &Sketch) -> Result<Vec<ProfileWire>, ProfileError> {
    let (mut chains, edges) = curves(sketch)?;
    let touching = endpoint_graph(&edges);
    if let Some((point, _)) = touching.iter().find(|(_, list)| list.len() > 2) {
        return Err(ProfileError::BranchingAt(*point));
    }
    let mut used = vec![false; edges.len()];
    // Open chains first, each from a loose end, in the order the sketch
    // holds its curves; what is left are loops.
    let mut starts: Vec<(usize, Uuid)> = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        for end in [edge.ends.0, edge.ends.1] {
            if touching[&end].len() == 1 {
                starts.push((index, end));
            }
        }
    }
    let loops = (0..edges.len()).map(|i| (i, edges[i].ends.0));
    for (start_idx, start_point) in starts.into_iter().chain(loops) {
        if used[start_idx] {
            continue;
        }
        let mut segments = Vec::new();
        let mut names = Vec::new();
        let mut current_point = start_point;
        let mut current_idx = start_idx;
        loop {
            used[current_idx] = true;
            let edge = &edges[current_idx];
            let forward = edge.ends.0 == current_point;
            segments.push(if forward {
                edge.segment.clone()
            } else {
                reversed(&edge.segment)
            });
            names.push(edge.name);
            current_point = if forward { edge.ends.1 } else { edge.ends.0 };
            match touching[&current_point].iter().copied().find(|&i| !used[i]) {
                Some(next) => current_idx = next,
                None => break,
            }
        }
        chains.push(ProfileWire { segments, names });
    }
    Ok(chains)
}

/// The sketch's curves that count: self-closing ones (circles, ellipses,
/// periodic splines) as wires of their own, the rest as curves between two
/// end points, projected ends at one spot joined.
fn curves(sketch: &Sketch) -> Result<(Vec<ProfileWire>, Vec<EdgeCurve>), ProfileError> {
    if sketch.unsolved {
        return Err(ProfileError::Unsolved);
    }
    let mut wires = Vec::new();
    let mut edges: Vec<EdgeCurve> = Vec::new();

    for geom in &sketch.geometry {
        // Construction geometry is a guide, never part of the profile, and
        // so is external geometry unless it is marked as defining it.
        let guide = sketch
            .external
            .get(&geom.id())
            .is_some_and(|source| !source.defining);
        if sketch.is_construction(geom.id()) || guide {
            continue;
        }
        let name = element_name(geom.id());
        match geom {
            GeometryElement::Point(_) => {}
            // A circle of no size encloses nothing.
            GeometryElement::Circle(c) if c.radius <= f32::EPSILON => {}
            GeometryElement::Circle(c) => {
                let center = sketch
                    .point_position(c.center)
                    .ok_or(ProfileError::MissingPoint(c.center))?;
                wires.push(ProfileWire {
                    names: vec![name],
                    segments: vec![ProfileSegment::Circle {
                        center: v2(center),
                        radius: c.radius as f64,
                    }],
                });
            }
            GeometryElement::Line(l) => {
                let a = sketch
                    .point_position(l.start)
                    .ok_or(ProfileError::MissingPoint(l.start))?;
                let b = sketch
                    .point_position(l.end)
                    .ok_or(ProfileError::MissingPoint(l.end))?;
                edges.push(EdgeCurve {
                    name,
                    ends: (l.start, l.end),
                    segment: ProfileSegment::Line {
                        start: v2(a),
                        end: v2(b),
                    },
                });
            }
            GeometryElement::Arc(arc) => {
                let c = sketch
                    .point_position(arc.center)
                    .ok_or(ProfileError::MissingPoint(arc.center))?;
                let s = sketch
                    .point_position(arc.start)
                    .ok_or(ProfileError::MissingPoint(arc.start))?;
                let e = sketch
                    .point_position(arc.end)
                    .ok_or(ProfileError::MissingPoint(arc.end))?;
                edges.push(EdgeCurve {
                    name,
                    ends: (arc.start, arc.end),
                    segment: ProfileSegment::Arc {
                        start: v2(s),
                        mid: v2(arc_midpoint(c, s, e)),
                        end: v2(e),
                    },
                });
            }
            // A full ellipse is a closed wire by itself, like a circle; an
            // arc of one joins the curves at its endpoints, as a circular
            // arc does.
            GeometryElement::Ellipse(e) => {
                let center = sketch
                    .point_position(e.center)
                    .ok_or(ProfileError::MissingPoint(e.center))?;
                let major = [f64::from(e.major.x), f64::from(e.major.y)];
                match e.arc {
                    None => wires.push(ProfileWire {
                        names: vec![name],
                        segments: vec![ProfileSegment::Ellipse {
                            center: v2(center),
                            major,
                            ratio: f64::from(e.ratio),
                        }],
                    }),
                    Some(arc) => {
                        // The span in double precision, so the arc's ends
                        // land as close as can be to the points it names.
                        let (t0, t1) = ellipse_arc_span(sketch, e, center)?;
                        edges.push(EdgeCurve {
                            name,
                            ends: (arc.start, arc.end),
                            segment: ProfileSegment::EllipseArc {
                                center: v2(center),
                                major,
                                ratio: f64::from(e.ratio),
                                start_param: t0,
                                end_param: t1,
                            },
                        });
                    }
                }
            }
            // An arc of a parabola or a hyperbola joins the curves at its
            // ends, as a circular arc does, drawn exactly as the rational
            // quadratic it is.
            GeometryElement::Conic(c) => {
                let at = |id: Uuid| {
                    sketch
                        .point_position(id)
                        .map(v2)
                        .ok_or(ProfileError::MissingPoint(id))
                };
                let (start, end) = (at(c.start)?, at(c.end)?);
                let (shape, t0, t1) = c.params(sketch).ok_or(ProfileError::MissingPoint(c.id))?;
                let ([_, middle, _], weight) = shape.quadratic(t0, t1);
                let weights = match c.kind {
                    crate::sketch::ConicKind::Parabola => Vec::new(),
                    crate::sketch::ConicKind::Hyperbola => vec![1.0, weight, 1.0],
                };
                edges.push(EdgeCurve {
                    name,
                    ends: (c.start, c.end),
                    segment: ProfileSegment::Nurbs {
                        degree: 2,
                        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                        control_points: vec![start, middle, end],
                        weights,
                        periodic: false,
                    },
                });
            }
            // Periodic B-splines close on themselves; open ones connect via
            // their first/last control point like any other curve.
            GeometryElement::BSpline(b) => {
                if b.control_points.len() < 2 {
                    continue; // degenerate: nothing to contribute
                }
                let control_points = b
                    .control_points
                    .iter()
                    .map(|id| {
                        sketch
                            .point_position(*id)
                            .map(v2)
                            .ok_or(ProfileError::MissingPoint(*id))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let segment = spline_segment(b, control_points);
                if b.periodic {
                    wires.push(ProfileWire {
                        names: vec![name],
                        segments: vec![segment],
                    });
                } else {
                    edges.push(EdgeCurve {
                        name,
                        ends: (
                            b.control_points[0],
                            *b.control_points.last().expect("len >= 2"),
                        ),
                        segment,
                    });
                }
            }
        }
    }

    if edges.is_empty() && wires.is_empty() {
        return Err(ProfileError::Empty);
    }

    // Projected curves each bring their own end points: where two meet at
    // one spot they join there, as drawn curves sharing a point do.
    let mut same: Vec<(Uuid, [f64; 2])> = Vec::new();
    let mut joined: HashMap<Uuid, Uuid> = HashMap::new();
    for edge in &edges {
        for end in [edge.ends.0, edge.ends.1] {
            if !sketch.is_external(end) || joined.contains_key(&end) {
                continue;
            }
            let Some(at) = sketch.point_position(end).map(v2) else {
                continue;
            };
            let found = same
                .iter()
                .find(|(_, p)| (p[0] - at[0]).hypot(p[1] - at[1]) < PROJECTED_JOIN)
                .map(|(id, _)| *id);
            match found {
                Some(first) => {
                    joined.insert(end, first);
                }
                None => {
                    same.push((end, at));
                    joined.insert(end, end);
                }
            }
        }
    }
    for edge in &mut edges {
        let canon = |id: Uuid| joined.get(&id).copied().unwrap_or(id);
        edge.ends = (canon(edge.ends.0), canon(edge.ends.1));
    }
    Ok((wires, edges))
}

/// Point id -> indices of the curves ending there.
fn endpoint_graph(edges: &[EdgeCurve]) -> HashMap<Uuid, Vec<usize>> {
    let mut touching: HashMap<Uuid, Vec<usize>> = HashMap::new();
    for (idx, edge) in edges.iter().enumerate() {
        touching.entry(edge.ends.0).or_default().push(idx);
        touching.entry(edge.ends.1).or_default().push(idx);
    }
    touching
}

/// Whether a loop of lines and arcs has no area: its corners and arcs'
/// middles, as a polygon, enclose next to none. Loops with other curves
/// are taken as they come.
fn encloses_nothing(segments: &[ProfileSegment]) -> bool {
    let mut corners = Vec::with_capacity(segments.len() * 2);
    for segment in segments {
        match segment {
            ProfileSegment::Line { start, .. } => corners.push(*start),
            ProfileSegment::Arc { start, mid, .. } => {
                corners.push(*start);
                corners.push(*mid);
            }
            _ => return false,
        }
    }
    let twice: f64 = (0..corners.len())
        .map(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    twice.abs() * 0.5 < 1e-9
}

/// The loose ends of the curves that count in the profile: points only one
/// of them reaches, which the profile leaves out with their curves.
pub fn loose_ends(sketch: &Sketch) -> Vec<Uuid> {
    let mut reach: HashMap<Uuid, usize> = HashMap::new();
    for geom in &sketch.geometry {
        let guide = sketch
            .external
            .get(&geom.id())
            .is_some_and(|source| !source.defining);
        if sketch.is_construction(geom.id()) || guide {
            continue;
        }
        let ends = match geom {
            GeometryElement::Line(l) => [l.start, l.end],
            GeometryElement::Arc(a) => [a.start, a.end],
            GeometryElement::Conic(c) => [c.start, c.end],
            GeometryElement::Ellipse(e) => match e.arc {
                Some(arc) => [arc.start, arc.end],
                None => continue,
            },
            GeometryElement::BSpline(b) if !b.periodic => {
                match (b.control_points.first(), b.control_points.last()) {
                    (Some(&a), Some(&z)) => [a, z],
                    _ => continue,
                }
            }
            _ => continue,
        };
        for end in ends {
            // Projected ends at one spot are one end, as the profile joins
            // them.
            let end = if sketch.is_external(end) {
                let at = sketch.point_position(end).map(v2);
                reach
                    .keys()
                    .copied()
                    .find(|other| {
                        sketch.is_external(*other)
                            && at.zip(sketch.point_position(*other).map(v2)).is_some_and(
                                |(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) < PROJECTED_JOIN,
                            )
                    })
                    .unwrap_or(end)
            } else {
                end
            };
            *reach.entry(end).or_default() += 1;
        }
    }
    reach
        .into_iter()
        .filter_map(|(point, n)| (n == 1).then_some(point))
        .collect()
}

/// The profile segment of spline `b` over its control point positions: the
/// cubic over even knots as plain control points, anything else with its
/// degree and knots spelt out.
fn spline_segment(b: &crate::sketch::BSpline, control_points: Vec<[f64; 2]>) -> ProfileSegment {
    if b.is_default_cubic() {
        return ProfileSegment::BSpline {
            control_points,
            periodic: b.periodic,
        };
    }
    match crate::spline::basis_of(b) {
        Some(basis) => ProfileSegment::Nurbs {
            degree: basis.degree() as u32,
            knots: if b.periodic {
                Vec::new()
            } else {
                basis.knots().to_vec()
            },
            control_points,
            weights: basis.weights().to_vec(),
            periodic: b.periodic,
        },
        None => ProfileSegment::BSpline {
            control_points,
            periodic: b.periodic,
        },
    }
}

/// `segment` run from its end to its start. An arc of an ellipse only runs
/// counter-clockwise, so it stays as it is.
fn reversed(segment: &ProfileSegment) -> ProfileSegment {
    match segment {
        ProfileSegment::Line { start, end } => ProfileSegment::Line {
            start: *end,
            end: *start,
        },
        ProfileSegment::Arc { start, mid, end } => ProfileSegment::Arc {
            start: *end,
            mid: *mid,
            end: *start,
        },
        ProfileSegment::BSpline {
            control_points,
            periodic,
        } => ProfileSegment::BSpline {
            control_points: control_points.iter().rev().copied().collect(),
            periodic: *periodic,
        },
        // A spline turned about: its control points and weights in the
        // other order, its knots mirrored across its domain.
        ProfileSegment::Nurbs {
            degree,
            knots,
            control_points,
            weights,
            periodic,
        } => {
            let (first, last) = (
                knots.first().copied().unwrap_or(0.0),
                knots.last().copied().unwrap_or(0.0),
            );
            ProfileSegment::Nurbs {
                degree: *degree,
                knots: knots.iter().rev().map(|k| first + last - k).collect(),
                control_points: control_points.iter().rev().copied().collect(),
                weights: weights.iter().rev().copied().collect(),
                periodic: *periodic,
            }
        }
        other => other.clone(),
    }
}

/// An arc of an ellipse's parameter span, `(t0, t1)` with `t1 > t0`, worked
/// in double precision from its endpoints.
fn ellipse_arc_span(
    sketch: &Sketch,
    e: &crate::sketch::Ellipse,
    center: Vec2D,
) -> Result<(f64, f64), ProfileError> {
    let arc = e.arc.ok_or(ProfileError::MissingPoint(e.id))?;
    let at = |id: Uuid| {
        sketch
            .point_position(id)
            .ok_or(ProfileError::MissingPoint(id))
    };
    let (major_x, major_y) = (f64::from(e.major.x), f64::from(e.major.y));
    let a = major_x.hypot(major_y);
    let b = a * f64::from(e.ratio);
    let (ux, uy) = (major_x / a, major_y / a);
    let param = |p: Vec2D| {
        let (dx, dy) = (
            f64::from(p.x) - f64::from(center.x),
            f64::from(p.y) - f64::from(center.y),
        );
        ((dx * -uy + dy * ux) / b).atan2((dx * ux + dy * uy) / a)
    };
    let t0 = param(at(arc.start)?);
    let mut t1 = param(at(arc.end)?);
    while t1 <= t0 + 1e-12 {
        t1 += std::f64::consts::TAU;
    }
    Ok((t0, t1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Line, Point};

    fn pt(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }
    fn line(sketch: &mut Sketch, a: Uuid, b: Uuid) -> Uuid {
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)))
    }

    fn rectangle(sketch: &mut Sketch) -> [Uuid; 4] {
        let a = pt(sketch, 0.0, 0.0);
        let b = pt(sketch, 10.0, 0.0);
        let c = pt(sketch, 10.0, 5.0);
        let d = pt(sketch, 0.0, 5.0);
        line(sketch, a, b);
        line(sketch, b, c);
        line(sketch, c, d);
        line(sketch, d, a);
        [a, b, c, d]
    }

    #[test]
    fn chains_take_open_curves_and_loops_alike() {
        let mut sketch = Sketch::new("t");
        rectangle(&mut sketch);
        // An open L away from the rectangle, drawn from its middle out.
        let a = pt(&mut sketch, 20.0, 0.0);
        let b = pt(&mut sketch, 30.0, 0.0);
        let c = pt(&mut sketch, 30.0, 8.0);
        line(&mut sketch, b, c);
        line(&mut sketch, a, b);
        let chains = extract_chains(&sketch).unwrap();
        assert_eq!(chains.len(), 2);
        let open = chains
            .iter()
            .find(|w| w.segments.len() == 2)
            .expect("the L");
        let ends: Vec<_> = open
            .segments
            .iter()
            .map(|s| match s {
                ProfileSegment::Line { start, end } => (*start, *end),
                _ => panic!("lines"),
            })
            .collect();
        assert_eq!(ends[0].1, ends[1].0, "the chain runs end to end");
        assert!(
            ends[0].0 == [20.0, 0.0] || ends[0].0 == [30.0, 8.0],
            "it starts at a loose end: {ends:?}"
        );
        assert!(chains.iter().any(|w| w.segments.len() == 4), "the loop");
    }

    #[test]
    fn a_branch_is_no_chain() {
        let mut sketch = Sketch::new("t");
        let o = pt(&mut sketch, 0.0, 0.0);
        for (x, y) in [(10.0, 0.0), (0.0, 10.0), (-10.0, 0.0)] {
            let p = pt(&mut sketch, x, y);
            line(&mut sketch, o, p);
        }
        assert!(matches!(
            extract_chains(&sketch),
            Err(ProfileError::BranchingAt(_))
        ));
    }

    #[test]
    fn rectangle_extracts_one_closed_wire() {
        let mut sketch = Sketch::new("t");
        rectangle(&mut sketch);
        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn circle_is_its_own_wire() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 3.0, 3.0);
        sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 2.0)));
        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert!(matches!(
            wires[0].segments[0],
            ProfileSegment::Circle { radius, .. } if (radius - 2.0).abs() < 1e-9
        ));
    }

    #[test]
    fn rectangle_with_hole_gives_two_wires() {
        let mut sketch = Sketch::new("t");
        rectangle(&mut sketch);
        let c = pt(&mut sketch, 5.0, 2.5);
        sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 1.0)));
        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 2);
    }

    #[test]
    fn open_chain_alone_is_rejected() {
        let mut sketch = Sketch::new("t");
        let a = pt(&mut sketch, 0.0, 0.0);
        let b = pt(&mut sketch, 10.0, 0.0);
        let c = pt(&mut sketch, 10.0, 5.0);
        line(&mut sketch, a, b);
        line(&mut sketch, b, c);
        assert!(matches!(
            extract_wires(&sketch),
            Err(ProfileError::OpenAt(_))
        ));
    }

    #[test]
    fn branching_is_rejected() {
        let mut sketch = Sketch::new("t");
        let [a, ..] = rectangle(&mut sketch);
        let e = pt(&mut sketch, -5.0, -5.0);
        let e2 = pt(&mut sketch, -5.0, 5.0);
        // Two extra edges through corner `a` (degree 4) forming a closed-ish
        // detour: every vertex except `a` has degree 2, so the failure is
        // unambiguously the branch at `a`.
        line(&mut sketch, a, e);
        line(&mut sketch, e, e2);
        line(&mut sketch, e2, a);
        assert!(matches!(
            extract_wires(&sketch),
            Err(ProfileError::BranchingAt(_))
        ));
    }

    /// A loop with loose curves around it is still a profile: the spur
    /// off one of its corners and a stray line elsewhere are left out.
    #[test]
    fn loose_curves_are_left_out_of_a_closed_loop() {
        let mut sketch = Sketch::new("t");
        let [a, ..] = rectangle(&mut sketch);
        let e = pt(&mut sketch, -5.0, -5.0);
        let f = pt(&mut sketch, -9.0, -5.0);
        line(&mut sketch, a, e); // a spur off the rectangle
        line(&mut sketch, e, f); // and on from it
        let g = pt(&mut sketch, 30.0, 0.0);
        let h = pt(&mut sketch, 40.0, 0.0);
        line(&mut sketch, g, h); // a stray line
        let wires = extract_wires(&sketch).expect("the rectangle closes");
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4, "the rectangle alone");
    }

    #[test]
    fn empty_sketch_is_rejected_but_points_are_ignored() {
        let mut sketch = Sketch::new("t");
        assert_eq!(extract_wires(&sketch), Err(ProfileError::Empty));
        pt(&mut sketch, 1.0, 1.0);
        assert_eq!(extract_wires(&sketch), Err(ProfileError::Empty));
    }

    #[test]
    fn construction_curves_are_ignored_by_profile() {
        let mut sketch = Sketch::new("t");
        let [a, _, c, _] = rectangle(&mut sketch);
        // A construction diagonal through two rectangle corners would make
        // the endpoint graph branch if it were considered part of the wire.
        let diagonal = line(&mut sketch, a, c);
        sketch.set_construction(diagonal, true);
        // A construction circle must not become its own wire either.
        let center = pt(&mut sketch, 5.0, 2.5);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
        sketch.set_construction(circle, true);

        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1, "only the rectangle remains");
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn all_construction_geometry_yields_empty_error() {
        let mut sketch = Sketch::new("t");
        let center = pt(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.set_construction(circle, true);
        assert_eq!(extract_wires(&sketch), Err(ProfileError::Empty));
    }

    #[test]
    fn arc_capped_slot_closes_and_mid_is_on_arc() {
        let mut sketch = Sketch::new("t");
        // A "slot": two horizontal lines capped by two semicircle arcs.
        let a = pt(&mut sketch, 0.0, 0.0);
        let b = pt(&mut sketch, 10.0, 0.0);
        let c = pt(&mut sketch, 10.0, 4.0);
        let d = pt(&mut sketch, 0.0, 4.0);
        let right_center = pt(&mut sketch, 10.0, 2.0);
        let left_center = pt(&mut sketch, 0.0, 2.0);
        line(&mut sketch, a, b);
        // CCW arc from b (10,0) to c (10,4) around (10,2) bulges right.
        sketch.add_geometry(GeometryElement::Arc(Arc::new(right_center, b, c, 2.0)));
        line(&mut sketch, c, d);
        sketch.add_geometry(GeometryElement::Arc(Arc::new(left_center, d, a, 2.0)));
        let wires = extract_wires(&sketch).unwrap();
        // The two arc centers are standalone-ish points but referenced by
        // arcs; the wire itself is the 4 curve segments.
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
        let mid = wires[0]
            .segments
            .iter()
            .find_map(|s| match s {
                ProfileSegment::Arc { mid, .. } => Some(*mid),
                _ => None,
            })
            .unwrap();
        // Right cap bulge: mid should be at x = 12 (10 + r), y = 2.
        let on_right = (mid[0] - 12.0).abs() < 1e-4 && (mid[1] - 2.0).abs() < 1e-4;
        let on_left = (mid[0] + 2.0).abs() < 1e-4 && (mid[1] - 2.0).abs() < 1e-4;
        assert!(on_right || on_left, "arc mid off-curve: {mid:?}");
    }

    #[test]
    fn every_segment_runs_on_from_where_the_last_one_ended() {
        // A notch cut into a square by a clockwise arc, and one side drawn
        // backwards: the wire still runs one way round.
        let mut sketch = Sketch::new("t");
        let a = pt(&mut sketch, 0.0, 0.0);
        let b = pt(&mut sketch, 4.0, 0.0);
        let c = pt(&mut sketch, 6.0, 0.0);
        let d = pt(&mut sketch, 10.0, 0.0);
        let e = pt(&mut sketch, 10.0, 10.0);
        let f = pt(&mut sketch, 0.0, 10.0);
        let notch = pt(&mut sketch, 5.0, 0.0);
        line(&mut sketch, a, b);
        // Counter-clockwise from c to b about (5, 0): over the top, into
        // the square.
        sketch.add_geometry(GeometryElement::Arc(Arc::new(notch, c, b, 1.0)));
        line(&mut sketch, c, d);
        line(&mut sketch, e, d);
        line(&mut sketch, e, f);
        line(&mut sketch, f, a);
        let wires = extract_wires(&sketch).unwrap();
        let ends = |s: &ProfileSegment| match s {
            ProfileSegment::Line { start, end } | ProfileSegment::Arc { start, end, .. } => {
                (*start, *end)
            }
            _ => unreachable!(),
        };
        let segments = &wires[0].segments;
        for (i, segment) in segments.iter().enumerate() {
            let next = &segments[(i + 1) % segments.len()];
            assert_eq!(ends(segment).1, ends(next).0, "segment {i}");
        }
    }

    #[test]
    fn a_spline_or_a_conic_walked_backwards_is_turned_about() {
        use crate::sketch::{BSpline, Conic, ConicKind};
        // A parabola's arc from (4, 2) back to (-4, 2), a quadratic spline
        // drawn from (-4, -3) up to (-4, 2), and lines: the walk meets the
        // spline end first.
        let mut sketch = Sketch::new("t");
        let vertex = pt(&mut sketch, 0.0, 0.0);
        let right = pt(&mut sketch, 4.0, 2.0);
        let left = pt(&mut sketch, -4.0, 2.0);
        let low_left = pt(&mut sketch, -4.0, -3.0);
        let bulge = pt(&mut sketch, -6.0, -0.5);
        let low_right = pt(&mut sketch, 4.0, -3.0);
        sketch.add_geometry(GeometryElement::Conic(Conic::new(
            ConicKind::Parabola,
            vertex,
            Vec2D::new(0.0, 2.0),
            0.0,
            right,
            left,
        )));
        sketch.add_geometry(GeometryElement::BSpline(BSpline {
            degree: 2,
            ..BSpline::new(vec![low_left, bulge, left], false)
        }));
        line(&mut sketch, low_left, low_right);
        line(&mut sketch, right, low_right);
        let wires = extract_wires(&sketch).unwrap();
        let ends = |s: &ProfileSegment| match s {
            ProfileSegment::Line { start, end } => (*start, *end),
            ProfileSegment::Nurbs { control_points, .. } => (
                control_points[0],
                *control_points.last().expect("control points"),
            ),
            _ => unreachable!(),
        };
        let segments = &wires[0].segments;
        assert_eq!(segments.len(), 4);
        for (i, segment) in segments.iter().enumerate() {
            let next = &segments[(i + 1) % segments.len()];
            let (at, from) = (ends(segment).1, ends(next).0);
            assert!(
                (at[0] - from[0]).hypot(at[1] - from[1]) < 1e-9,
                "segment {i} ends at {at:?}, the next starts at {from:?}"
            );
        }
        // The knots of a turned spline still run upward from where they did.
        for segment in segments {
            if let ProfileSegment::Nurbs { knots, .. } = segment {
                assert!(knots.windows(2).all(|w| w[0] <= w[1]), "{knots:?}");
            }
        }
    }
}

#[cfg(test)]
mod external_profile_tests {
    use crate::sketch::{ExternalSource, GeometryElement, Line, Point, Sketch, Vec2D};

    #[test]
    fn a_projected_edge_closes_a_profile_once_it_counts() {
        let mut sketch = Sketch::new("t");
        let mut p = |x: f32, y: f32| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        };
        let (a, b, c) = (p(0.0, 0.0), p(10.0, 0.0), p(5.0, 8.0));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        let edge = sketch.add_geometry(GeometryElement::Line(Line::new(c, a)));
        sketch.external.insert(
            edge,
            ExternalSource {
                body: uuid::Uuid::new_v4(),
                point: [0.0; 3],
                direction: [1.0, 0.0, 0.0],
                section: false,
                defining: false,
                reference: None,
            },
        );
        assert!(
            super::extract_wires(&sketch).is_err(),
            "a guide leaves the outline open"
        );
        crate::commands::set_external_defining(&mut sketch, &[edge], true);
        let wires = super::extract_wires(&sketch).expect("closed by the edge");
        assert_eq!(wires[0].segments.len(), 3);
    }

    /// Projected edges of one solid meet within its tolerance, a few
    /// single-precision steps apart at part sizes: they join there. Ends
    /// a hundredth of a millimetre apart stay apart.
    #[test]
    fn projected_ends_a_rounding_apart_join() {
        let triangle = |gap: f32| {
            let mut sketch = Sketch::new("t");
            let source = ExternalSource {
                body: uuid::Uuid::new_v4(),
                point: [0.0; 3],
                direction: [1.0, 0.0, 0.0],
                section: false,
                defining: true,
                reference: None,
            };
            let corners = [(104.5, 90.46158), (150.0, 90.46158), (120.0, 120.0)];
            for i in 0..3 {
                let (x0, y0) = corners[i];
                let (x1, y1) = corners[(i + 1) % 3];
                let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x0, y0))));
                // Each edge ends a little off where the next one starts.
                let b = sketch
                    .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x1, y1 + gap))));
                let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
                sketch.external.insert(line, source);
            }
            super::extract_wires(&sketch)
        };
        let near = triangle(1.5e-5);
        assert_eq!(near.expect("the ends join")[0].segments.len(), 3);
        assert!(triangle(0.01).is_err(), "a real gap stays open");
    }
}
