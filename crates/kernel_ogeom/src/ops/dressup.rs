//! Fillet, chamfer, draft, thickness and face edits (offset, move, remove)
//! on the running solid, with geometric (point-based) edge and face
//! selection.
//!
//! A fillet or a chamfer goes to the kernel as one chain, every selected
//! edge at once, resolved on the solid as it stands, so blends and bevels
//! that meet at a vertex close their corner between them.

use kernel_api::{
    ChamferCorner, ChamferSpec, EdgeSelection, FilletCorner, Overflow, RoundDistances,
    RoundProfile, ThicknessJoin,
};
use ogeom::algo::distance_between_shapes;
use ogeom::fillet::{
    Blend, BlendRefusal, BlendSizing, Chamfer, LawSize, LawStation, blend_edges_overflow,
    blend_limit, chamfer_edges_checked, chamfer_edges_overflow, fillet_edge_distances_overflow,
    fillet_edges_corner_checked, fillet_edges_corner_overflow, fillet_edges_variable_overflow,
    largest_chamfer_distance_all,
};
use ogeom::geom::Curve3d as _;
use ogeom::math::{Direction, Plane, Point, Vector};
use ogeom::offset::{
    Join, apply_draft, make_thick_solid_with, move_faces, offset_faces, offset_shape,
};
use ogeom::topo::{EdgeRepr, Model, NodeData, Shape, ShapeType, ancestors_of, explore_unique};

use super::tol;
use crate::convert::{array, point, vector};

/// The sub-shape of `root` nearest to `probe`, of the wanted type.
pub fn nearest_of(
    model: &mut Model,
    root: &Shape,
    want: ShapeType,
    probe: Point,
) -> Result<Shape, String> {
    crate::probe::by_distance(model, root, want, probe)?
        .into_iter()
        .next()
        .map(|(_, shape)| shape)
        .ok_or_else(|| format!("no {want:?} found near the selection point"))
}

/// The face of `root` named `name`, else the one nearest `point`.
fn face_named_or_nearest(
    model: &mut Model,
    root: &Shape,
    name: Option<kernel_api::TopoName>,
    point: [f64; 3],
) -> Result<Shape, String> {
    let probe = crate::probe::at_point(crate::convert::point(point), name.unwrap_or(0));
    crate::probe::face_for(model, root, &probe, crate::probe::Reach::Pick)
}

/// What names an edge: the names of the faces it runs between, when the
/// pick kept them; else the edge nearest `point`, and, when `along` is
/// set, running that way there.
struct Probe {
    point: Point,
    along: Option<Vector>,
    faces: [kernel_api::TopoName; 2],
}

impl Probe {
    fn at(point: Point) -> Self {
        Self {
            point,
            along: None,
            faces: [0, 0],
        }
    }
}

/// The edges a selection names on `solid`, each once. Every edge, or the
/// edges of picked faces, are taken as they are, seams left out; edges
/// picked one by one are found from their picks.
fn selected_edges(
    model: &mut Model,
    solid: &Shape,
    edges: &EdgeSelection,
) -> Result<Vec<Shape>, String> {
    let picks: Vec<([f64; 3], Option<kernel_api::TopoName>)> = match edges {
        EdgeSelection::All => {
            let seams = seam_edges(model, solid)?;
            return Ok(explore_unique(model, solid, ShapeType::Edge)
                .map_err(|e| format!("exploring edges failed: {e}"))?
                .into_iter()
                .filter(|e| !seams.iter().any(|s| s.is_same(e)))
                .collect());
        }
        EdgeSelection::OfPickedFaces(probes) => {
            probes.iter().map(|p| (p.point, Some(p.name))).collect()
        }
        EdgeSelection::OfFaces(points) => points.iter().map(|p| (*p, None)).collect(),
        EdgeSelection::Picked(_) | EdgeSelection::Near(_) => {
            let probes = selection_probes(edges)?;
            return chain_of(model, solid, probes);
        }
    };
    // A seam is where a closed face meets itself, not an edge to round.
    let seams = seam_edges(model, solid)?;
    let mut chain: Vec<Shape> = Vec::new();
    for (p, name) in picks {
        let face = face_named_or_nearest(model, solid, name, p)?;
        let face_edges = explore_unique(model, &face, ShapeType::Edge)
            .map_err(|e| format!("exploring face edges failed: {e}"))?;
        for edge in face_edges {
            if !seams.iter().any(|s| s.is_same(&edge)) && !chain.iter().any(|s| s.is_same(&edge)) {
                chain.push(edge);
            }
        }
    }
    Ok(chain)
}

/// Probes for the edges a selection picks one by one, which must lie
/// within reach of their edge.
fn selection_probes(edges: &EdgeSelection) -> Result<(Vec<Probe>, bool), String> {
    let probes = match edges {
        EdgeSelection::Near(points) => {
            return Ok((points.iter().map(|p| Probe::at(point(*p))).collect(), true));
        }
        EdgeSelection::Picked(picks) => {
            let probes = picks
                .iter()
                .map(|pick| {
                    let [x, y, z] = pick.direction;
                    let length = (x * x + y * y + z * z).sqrt();
                    Probe {
                        point: point(pick.point),
                        along: (length > 1e-9)
                            .then(|| Vector::new(x / length, y / length, z / length)),
                        faces: pick.faces,
                    }
                })
                .collect();
            return Ok((probes, true));
        }
        EdgeSelection::All | EdgeSelection::OfFaces(_) | EdgeSelection::OfPickedFaces(_) => {
            Vec::new()
        }
    };
    Ok((probes, false))
}

/// The edges of `solid` that border one face only: a closed face's seam,
/// where the face meets itself (a cylinder's, a sphere's). No blend
/// rounds them; a selection of every edge, or of a face's, leaves them out.
fn seam_edges(model: &Model, solid: &Shape) -> Result<Vec<Shape>, String> {
    let faces = explore_unique(model, solid, ShapeType::Face)
        .map_err(|e| format!("exploring faces failed: {e}"))?;
    let mut counted: Vec<(Shape, usize)> = Vec::new();
    for face in &faces {
        let edges = explore_unique(model, face, ShapeType::Edge)
            .map_err(|e| format!("exploring face edges failed: {e}"))?;
        for edge in edges {
            match counted.iter_mut().find(|(e, _)| e.is_same(&edge)) {
                Some((_, n)) => *n += 1,
                None => counted.push((edge, 1)),
            }
        }
    }
    Ok(counted
        .into_iter()
        .filter(|(_, n)| *n == 1)
        .map(|(e, _)| e)
        .collect())
}

/// How far `point` is from `shape`.
fn distance_to(model: &mut Model, point: Point, shape: &Shape) -> Option<f64> {
    let vertex = model.add_vertex(ogeom::topo::VertexData::new(point));
    distance_between_shapes(
        model,
        &vertex,
        shape,
        ogeom::intersect::ExtremaOptions::default(),
        tol(),
    )
    .ok()
    .map(|d| d.distance)
}

/// Whether `edge`, `distance` from `point`, runs along `along` where it
/// passes the point: a step that way, forward or back, leaves the
/// distance to it nearly as it was (within 15 degrees), where a step
/// across it changes the distance by most of the step. The step is long
/// next to the distance, so a step past the edge's side does not pass for
/// one along it.
fn runs_along(model: &mut Model, edge: &Shape, point: Point, along: Vector, distance: f64) -> bool {
    let step = (distance * 4.0).max(0.05);
    let level = step * 15f64.to_radians().sin();
    [1.0, -1.0].into_iter().any(|sign| {
        distance_to(model, point + along * (step * sign), edge)
            .is_some_and(|d| (d - distance).abs() <= level)
    })
}

/// The edges of `solid` picked one by one, each found as a fillet's
/// picks are.
pub(crate) fn picked_edges(
    model: &mut Model,
    solid: &Shape,
    picks: &[kernel_api::EdgeProbe],
) -> Result<Vec<Shape>, String> {
    let probes = selection_probes(&EdgeSelection::Picked(picks.to_vec()))?;
    chain_of(model, solid, probes)
}

/// The edges of another body's `shape` the picks name. The names the
/// chain knows are its own body's, so they are found by where they were
/// picked, as edges whose names were never kept are.
pub(crate) fn picked_edges_elsewhere(
    model: &mut Model,
    shape: &Shape,
    picks: &[kernel_api::EdgeProbe],
) -> Result<Vec<Shape>, String> {
    let picks: Vec<kernel_api::EdgeProbe> = picks
        .iter()
        .map(|p| kernel_api::EdgeProbe {
            faces: [0, 0],
            ..*p
        })
        .collect();
    picked_edges(model, shape, &picks)
}

/// A fillet or chamfer the kernel refused: its message, and the edges
/// found to fail when the op's edges are tried alone and in the groups
/// meeting at a vertex.
#[derive(Debug, Default)]
pub struct Refusal {
    pub message: String,
    pub edges: kernel_api::EdgeRefusal,
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self {
            message,
            edges: Default::default(),
        }
    }
}

/// The most edges a refusal is tried edge by edge for.
const DIAGNOSED_EDGES: usize = 64;

#[allow(clippy::too_many_arguments, reason = "the fillet op's own fields")]
pub fn fillet(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    edges: &EdgeSelection,
    follow_tangent: bool,
    corner: FilletCorner,
    distances: Option<RoundDistances>,
    auto: bool,
    overflow: Overflow,
) -> Result<Shape, Refusal> {
    if let Some(distances) = distances {
        let chain = edge_chain(model, solid, edges, follow_tangent, "fillet")?;
        let faces = reference_faces(model, solid, &chain, distances.flip)?;
        let rounds: Vec<(Shape, Blend)> = chain
            .into_iter()
            .zip(faces)
            .map(|(edge, face)| {
                let round = Blend::RoundDistances {
                    face,
                    on_face: radius,
                    on_other: distances.other,
                    profile: profile(distances.profile),
                };
                (edge, round)
            })
            .collect();
        // Rounds alone: the chamfer corner the call takes is never used.
        let built = blend_edges_overflow(
            model,
            solid,
            &rounds,
            ogeom::fillet::ChamferCorner::Facet,
            kernel_overflow(overflow),
            tol(),
        )
        .map_err(|e| format!("fillet with two distances failed: {e}"))?;
        crate::naming::record(&built.history);
        return Ok(built.shape);
    }
    let corner = kernel_corner(corner);
    let chain = edge_chain(model, solid, edges, follow_tangent, "fillet")?;
    let pick =
        |picked: &[usize]| -> Vec<Shape> { picked.iter().map(|&i| chain[i].clone()).collect() };
    // The checked call names the edges a refusal concerns, under every
    // overflow behaviour; other options go through the call taking them,
    // whose refusal the edges are tried apart for.
    let asked = if overflow.is_default() {
        fillet_edges_corner_checked(model, solid, &chain, radius, corner, tol())
    } else {
        round(model, solid, &chain, radius, corner, overflow).map_err(|error| BlendRefusal {
            edges: Vec::new(),
            error,
        })
    };
    match asked {
        Ok(built) => {
            crate::naming::record(&built.history);
            Ok(built.shape)
        }
        // An automatic round refused for running off a face stops its
        // contact there; one that fits both faces is the round built.
        Err(_)
            if auto
                && let Some(shape) =
                    auto_round(model, solid, radius, edges, follow_tangent, overflow)? =>
        {
            Ok(shape)
        }
        Err(refused) => Err(refusal(
            "fillet",
            radius,
            model,
            solid,
            &chain,
            refused,
            |m, picked| {
                fillet_reading(m, solid, &pick(picked), corner, overflow, radius)
                    .map(|r| r.per(radius))
            },
            |m, picked, share| {
                round(m, solid, &pick(picked), radius * share, corner, overflow).map(|_| ())
            },
        )?),
    }
}

/// A round whose radius runs along the selected edges, taken as one run
/// end to end: `start` at the run's start, `variable.end` at its end and
/// the radii between at their shares of its length. The run starts at the
/// free end of the first edge found at one (the first edge on a loop), or
/// at the other end with `variable.reversed`.
pub fn variable_fillet(
    model: &mut Model,
    solid: &Shape,
    start: f64,
    variable: &kernel_api::VariableRadius,
    edges: &EdgeSelection,
    follow_tangent: bool,
    overflow: Overflow,
) -> Result<Shape, String> {
    let run = variable_run(model, solid, variable, edges, follow_tangent)?;
    let law = radius_law(start, variable);
    let built =
        fillet_edges_variable_overflow(model, solid, &run, &law, kernel_overflow(overflow), tol());
    match built {
        Ok(built) => {
            crate::naming::record(&built.history);
            Ok(built.shape)
        }
        Err(ogeom::core::OgeomError::Cancelled) => Err("variable fillet failed: cancelled".into()),
        Err(e) => {
            let mut message = format!("variable fillet failed: {e}");
            // The largest scale of every radius that builds, read by the
            // kernel's own builds of the law under every overflow
            // behaviour, so under other options told only where the law so
            // scaled builds under them.
            let scale = ogeom::fillet::largest_law_scale(model, solid, &run, &law, 1.0, tol());
            if let Ok(Some(scale)) = scale
                && scale < 1.0
                && (overflow.is_default()
                    || law_builds(model, solid, &run, &law_scaled(&law, scale), overflow))
            {
                let percent = (scale * 1000.0).floor() / 10.0;
                message.push_str(&format!("; its radii fit at {percent} % of their size"));
            }
            Err(message)
        }
    }
}

/// A variable fillet's edges as the run its law goes along.
fn variable_run(
    model: &mut Model,
    solid: &Shape,
    variable: &kernel_api::VariableRadius,
    edges: &EdgeSelection,
    follow_tangent: bool,
) -> Result<Vec<Shape>, String> {
    let chain = edge_chain(model, solid, edges, follow_tangent, "fillet")?;
    let run = edge_run(model, &chain)?;
    Ok(if variable.reversed {
        run.iter().rev().map(Shape::reversed).collect()
    } else {
        run
    })
}

/// The kernel's law for a variable fillet starting at `start`: its points
/// the start, the radii between in the order of their places, the end.
fn radius_law(start: f64, variable: &kernel_api::VariableRadius) -> ogeom::fillet::RadiusLaw {
    if variable.between.is_empty() {
        ogeom::fillet::RadiusLaw::Linear {
            start,
            end: variable.end,
        }
    } else {
        let mut points = vec![(0.0, start)];
        points.extend(variable.between.iter().map(|[at, radius]| (*at, *radius)));
        points.push((1.0, variable.end));
        ogeom::fillet::RadiusLaw::Points(points)
    }
}

/// `law` with every radius times `scale`.
fn law_scaled(law: &ogeom::fillet::RadiusLaw, scale: f64) -> ogeom::fillet::RadiusLaw {
    use ogeom::fillet::RadiusLaw;
    match law {
        RadiusLaw::Linear { start, end } => RadiusLaw::Linear {
            start: start * scale,
            end: end * scale,
        },
        RadiusLaw::Points(points) => {
            RadiusLaw::Points(points.iter().map(|(s, r)| (*s, r * scale)).collect())
        }
    }
}

/// `law` with its point `at` (a linear law's start 0 and end 1) at
/// `radius`; on a `closed` run its first and last points are one, and
/// either sets both, as the kernel's reading frees them.
fn law_with(
    law: &ogeom::fillet::RadiusLaw,
    at: usize,
    radius: f64,
    closed: bool,
) -> ogeom::fillet::RadiusLaw {
    use ogeom::fillet::RadiusLaw;
    let last = match law {
        RadiusLaw::Linear { .. } => 1,
        RadiusLaw::Points(points) => points.len().saturating_sub(1),
    };
    let ends = closed && (at == 0 || at == last);
    let set = |i: usize| i == at || (ends && (i == 0 || i == last));
    match law {
        RadiusLaw::Linear { start, end } => RadiusLaw::Linear {
            start: if set(0) { radius } else { *start },
            end: if set(1) { radius } else { *end },
        },
        RadiusLaw::Points(points) => RadiusLaw::Points(
            points
                .iter()
                .enumerate()
                .map(|(i, (s, r))| (*s, if set(i) { radius } else { *r }))
                .collect(),
        ),
    }
}

/// Whether `run`, its edges end to end, closes on itself: no vertex of it
/// ends one edge alone.
fn closed_run(model: &Model, run: &[Shape]) -> bool {
    let mut ends: Vec<(Shape, usize)> = Vec::new();
    for edge in run {
        let Ok(Some((a, b))) = ogeom::algo::edge_vertices(model, edge) else {
            return false;
        };
        for vertex in [a, b] {
            match ends.iter_mut().find(|(v, _)| v.is_same(&vertex)) {
                Some((_, count)) => *count += 1,
                None => ends.push((vertex, 1)),
            }
        }
    }
    !ends.is_empty() && ends.iter().all(|(_, count)| *count % 2 == 0)
}

/// Whether the round of `run` under `law` builds on `solid` reaching other
/// geometry as `overflow` says, on a copy of `model`.
fn law_builds(
    model: &Model,
    solid: &Shape,
    run: &[Shape],
    law: &ogeom::fillet::RadiusLaw,
    overflow: Overflow,
) -> bool {
    let overflow = kernel_overflow(overflow);
    fillet_edges_variable_overflow(&mut model.clone(), solid, run, law, overflow, tol()).is_ok()
}

/// The largest value the radius `at` of a variable fillet `op` builds at
/// on `solid`, the others held, counted as [`kernel_api::SolidOp::law_radii`]
/// counts them: the kernel's reading, confirmed by its builds of the whole
/// law under every overflow behaviour. Under other options it holds where
/// the law builds at it under them, and is `None` where it does not, the
/// kernel reading no law under them. `None` too where every value short of
/// the solid's diagonal builds, where none does, or for any other op.
pub fn law_limit(model: &Model, solid: &Shape, op: &kernel_api::SolidOp, at: usize) -> Option<f64> {
    let kernel_api::SolidOp::Fillet {
        radius,
        edges,
        follow_tangent,
        variable: Some(variable),
        overflow,
        ..
    } = op
    else {
        return None;
    };
    let span = crate::tess::robust_bounds(model, solid).map(|(lo, hi)| (hi - lo).magnitude())?;
    let run = variable_run(&mut model.clone(), solid, variable, edges, *follow_tangent).ok()?;
    let law = radius_law(*radius, variable);
    let sizing = BlendSizing::RoundLaw {
        chain: &run,
        law: &law,
        size: LawSize::At(LawStation::Point(at)),
    };
    let limit = blend_limit(model, solid, &sizing, span, tol()).ok()?;
    // A bound that builds leaves the radius free.
    limit.refused?;
    let size = limit.size?;
    (overflow.is_default() || {
        let law = law_with(&law, at, size, closed_run(model, &run));
        law_builds(model, solid, &run, &law, *overflow)
    })
    .then_some(size)
}

/// `chain`'s edges as one run end to end, each the way the run goes: from
/// an edge with an end no other edge meets, or round a loop from the first
/// edge. Refused where the edges branch or make more than one run.
fn edge_run(model: &Model, chain: &[Shape]) -> Result<Vec<Shape>, String> {
    let ends: Vec<(Shape, Shape)> = chain
        .iter()
        .map(|edge| {
            ogeom::algo::edge_vertices(model, edge)
                .ok()
                .flatten()
                .ok_or_else(|| "a variable fillet's edge has no ends".to_string())
        })
        .collect::<Result<_, _>>()?;
    let meets = |vertex: &Shape, skip: usize| {
        ends.iter()
            .enumerate()
            .filter(|(i, (a, b))| *i != skip && (a.is_same(vertex) || b.is_same(vertex)))
            .count()
    };
    if (0..chain.len()).any(|i| meets(&ends[i].0, i) > 1 || meets(&ends[i].1, i) > 1) {
        return Err("a variable fillet takes one run of edges end to end, and these branch".into());
    }
    // From a free end where there is one: the first edge with one, turned
    // to start there.
    let first = (0..chain.len())
        .find_map(|i| {
            let (a, b) = &ends[i];
            if meets(a, i) == 0 && !a.is_same(b) {
                Some((i, false))
            } else if meets(b, i) == 0 && !a.is_same(b) {
                Some((i, true))
            } else {
                None
            }
        })
        .unwrap_or((0, false));
    let mut used = vec![false; chain.len()];
    let mut run = Vec::with_capacity(chain.len());
    let (mut at, mut turned) = first;
    loop {
        used[at] = true;
        let (a, b) = &ends[at];
        run.push(if turned {
            chain[at].reversed()
        } else {
            chain[at].clone()
        });
        let head = if turned { a } else { b };
        let Some(next) = (0..chain.len())
            .find(|&k| !used[k] && (ends[k].0.is_same(head) || ends[k].1.is_same(head)))
        else {
            break;
        };
        turned = !ends[next].0.is_same(head);
        at = next;
    }
    if run.len() < chain.len() {
        return Err(format!(
            "a variable fillet takes one run of edges end to end; {} of these {} make one",
            run.len(),
            chain.len()
        ));
    }
    Ok(run)
}

/// The kernel's spelling of how a fillet closes its corners.
fn kernel_corner(corner: FilletCorner) -> ogeom::fillet::FilletCorner {
    match corner {
        FilletCorner::Setback => ogeom::fillet::FilletCorner::Setback,
        FilletCorner::RollingBall => ogeom::fillet::FilletCorner::Rolling,
    }
}

/// The kernel's spelling of what a blend does where it reaches other
/// geometry.
fn kernel_overflow(overflow: Overflow) -> ogeom::fillet::Overflow {
    let mut kernel = ogeom::fillet::Overflow::default();
    kernel.blend_into_round = overflow.blend_into_round;
    kernel.hold_edge = overflow.hold_edge;
    kernel.hold_edge_at_end = overflow.hold_edge_at_end;
    kernel.trim_through = overflow.trim_through;
    kernel.narrow_blend_made = overflow.narrow_blend_made;
    kernel.narrow_whole_face = overflow.narrow_whole_face;
    kernel.narrow_partial_face = overflow.narrow_partial_face;
    kernel
}

/// The rounds of `edges` at `radius`, closed as `corner` says, reaching
/// other geometry as `overflow` says.
fn round(
    model: &mut Model,
    solid: &Shape,
    edges: &[Shape],
    radius: f64,
    corner: ogeom::fillet::FilletCorner,
    overflow: Overflow,
) -> ogeom::core::OgeomResult<ogeom::algo::Built> {
    fillet_edges_corner_overflow(
        model,
        solid,
        edges,
        radius,
        corner,
        kernel_overflow(overflow),
        tol(),
    )
}

/// The bevels `specs`, a vertex of three or more closed as `corner`
/// says, reaching other geometry as `overflow` says.
fn bevel(
    model: &mut Model,
    solid: &Shape,
    specs: &[(Shape, Chamfer)],
    corner: ogeom::fillet::ChamferCorner,
    overflow: Overflow,
) -> ogeom::core::OgeomResult<ogeom::algo::Built> {
    chamfer_edges_overflow(
        model,
        solid,
        specs,
        corner,
        kernel_overflow(overflow),
        tol(),
    )
}

/// `reading` as the request reads under `overflow`: the kernel confirms
/// its readings with every behaviour on, so under other options one only
/// seats the request, a build under them confirming it.
fn under(reading: Reading, overflow: Overflow) -> Reading {
    match reading {
        Reading::Built(read) if !overflow.is_default() => Reading::Seated(read),
        reading => reading,
    }
}

/// An automatic round of one edge too wide for a face beside it: each
/// contact `radius` from the edge, or as far as its face goes, the round
/// elliptical between them. `None` where the round fits both faces or
/// takes more than one edge, for the round of one radius to build.
fn auto_round(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    edges: &EdgeSelection,
    follow_tangent: bool,
    overflow: Overflow,
) -> Result<Option<Shape>, String> {
    let chain = edge_chain(model, solid, edges, follow_tangent, "fillet")?;
    let [edge] = chain.as_slice() else {
        return Ok(None);
    };
    let face = adjacent_face(model, solid, edge, false)?;
    let [on_face, on_other] =
        side_limits(model, solid, edge, &face).map(|most| radius.min(most.unwrap_or(radius)));
    if on_face >= radius && on_other >= radius {
        return Ok(None);
    }
    let built = fillet_edge_distances_overflow(
        model,
        solid,
        edge,
        &face,
        on_face,
        on_other,
        ogeom::fillet::RoundProfile::Elliptical,
        kernel_overflow(overflow),
        tol(),
    )
    .map_err(|e| format!("fillet failed: {e}"))?;
    crate::naming::record(&built.history);
    Ok(Some(built.shape))
}

/// How far a round's contact goes on `face`, then on the edge's other
/// face, before running off it.
fn side_limits(model: &Model, solid: &Shape, edge: &Shape, face: &Shape) -> [Option<f64>; 2] {
    let round = |on_face, on_other| BlendSizing::RoundDistances {
        edge,
        face,
        on_face,
        on_other,
        profile: ogeom::fillet::RoundProfile::Elliptical,
    };
    sides_reach(model, solid, [round(1.0, SLIVER), round(SLIVER, 1.0)])
}

/// How far a bevel's leg goes on `face`, then on the edge's other face,
/// before running off it.
fn chamfer_side_limits(
    model: &Model,
    solid: &Shape,
    edge: &Shape,
    face: &Shape,
) -> [Option<f64>; 2] {
    let legs = |on_face, on_other| Chamfer::Distances {
        face: face.clone(),
        on_face,
        on_other,
    };
    let legs = [legs(1.0, SLIVER), legs(SLIVER, 1.0)];
    sides_reach(
        model,
        solid,
        legs.each_ref().map(|spec| BlendSizing::Chamfer {
            edge,
            spec,
            corner: ogeom::fillet::ChamferCorner::Facet,
        }),
    )
}

/// The other face's distance while one face's own reach is read.
const SLIVER: f64 = 1e-5;

/// Each face's own reach, the scale of two distances at which a contact
/// first leaves its face (the seat the kernel's limit reading starts
/// from, not the size that builds: a bevel past its face rolls over as a
/// notch and builds), `sizings` the two read with the other face's
/// distance kept at a sliver, so only its own face stops it. `None` where
/// it runs on past the solid's size or the kernel cannot read it. The
/// kernel gives the seat only with the confirmed size, so each reach costs
/// that reading's builds.
fn sides_reach(model: &Model, solid: &Shape, sizings: [BlendSizing<'_>; 2]) -> [Option<f64>; 2] {
    let Some(span) = crate::tess::robust_bounds(model, solid).map(|(lo, hi)| (hi - lo).magnitude())
    else {
        return [None, None];
    };
    sizings.map(|sizing| {
        blend_limit(model, solid, &sizing, span, tol())
            .ok()
            .and_then(|limit| limit.seat)
            .filter(|scale| *scale < span * (1.0 - 1e-9))
    })
}

/// The faces beside a round's edges as its drag handles read them, at
/// its first edge: where on the edge, which way each face runs from it,
/// and how far each contact goes, the least over the round's edges.
/// `flip` puts the edge's other face first, as the round's first distance
/// does. An automatic round reads one edge, the only one it takes, and is
/// `None` for more.
fn round_sides(
    model: &mut Model,
    solid: &Shape,
    edges: &EdgeSelection,
    follow_tangent: bool,
    flip: bool,
    auto: bool,
) -> Option<kernel_api::RoundSides> {
    let chain = edge_chain(model, solid, edges, follow_tangent, "fillet").ok()?;
    if auto && chain.len() != 1 {
        return None;
    }
    let faces = reference_faces(model, solid, &chain, flip).ok()?;
    let limits: Vec<[Option<f64>; 2]> = chain
        .iter()
        .zip(&faces)
        .map(|(edge, face)| side_limits(model, solid, edge, face))
        .collect();
    let least = |i: usize| limits.iter().filter_map(|l| l[i]).reduce(f64::min);
    edge_sides(
        model,
        solid,
        chain.first()?,
        faces.first()?,
        [least(0), least(1)],
    )
}

/// The faces beside `edge`, `face` first, as drag handles read them: a
/// point on the edge, the way each face runs from it, and `limits`.
fn edge_sides(
    model: &mut Model,
    solid: &Shape,
    edge: &Shape,
    face: &Shape,
    limits: [Option<f64>; 2],
) -> Option<kernel_api::RoundSides> {
    let other = faces_of_edge(model, solid, edge)
        .ok()?
        .into_iter()
        .find(|f| !f.is_same(face))?;
    let probe = failing_edge(model, edge, [0, 0], 0, 1, String::new()).probe;
    let (at, along) = (point(probe.point), vector(probe.direction));
    let mut sides = Vec::with_capacity(2);
    for (side, largest) in [(face, limits[0]), (&other, limits[1])] {
        let (_, normal) = crate::probe::outward_at(model, side, at).ok()?;
        let across = normal.cross(along);
        let length = across.magnitude();
        if length < 1e-9 {
            return None;
        }
        let across = across * (1.0 / length);
        // Of the two ways square to the edge, the one staying on the face.
        let mut off = |sign: f64| {
            distance_to(model, at + across * (0.01 * sign), side).unwrap_or(f64::INFINITY)
        };
        let direction = if off(1.0) <= off(-1.0) {
            across
        } else {
            -across
        };
        sides.push(kernel_api::RoundSide {
            direction: array(direction),
            largest,
        });
    }
    Some(kernel_api::RoundSides {
        at: array(at),
        sides: [sides[0], sides[1]],
    })
}

/// A chamfer's sides, for two distances or an automatic size: each face's
/// limit the least over the chamfer's edges, the faces read at its first
/// edge; and its first size at the limit: two distances' first face's,
/// an automatic size's where every face of every edge has run out
/// (`None` where one never does).
fn chamfer_sides(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
    follow_tangent: bool,
    overflow: Overflow,
) -> Option<(Option<f64>, Option<kernel_api::RoundSides>)> {
    let chain = edge_chain(model, solid, edges, follow_tangent, "chamfer").ok()?;
    let faces = match spec {
        ChamferSpec::Auto { .. } => chain
            .iter()
            .map(|edge| adjacent_face(model, solid, edge, false))
            .collect::<Result<Vec<_>, _>>()
            .ok()?,
        _ => reference_faces(model, solid, &chain, flip).ok()?,
    };
    let limits: Vec<[Option<f64>; 2]> = chain
        .iter()
        .zip(&faces)
        .map(|(edge, face)| chamfer_side_limits(model, solid, edge, face))
        .collect();
    let least = |i: usize| limits.iter().filter_map(|l| l[i]).reduce(f64::min);
    let (first, second) = (least(0), least(1));
    let largest = match spec {
        ChamferSpec::Auto { .. } => {
            let every: Vec<Option<f64>> = limits.iter().flatten().copied().collect();
            if every.iter().all(Option::is_none) {
                largest_size(
                    model,
                    solid,
                    &auto_as_equal(spec, flip, edges, follow_tangent, overflow),
                )
            } else if every.iter().any(Option::is_none) {
                None
            } else {
                every.into_iter().flatten().reduce(f64::max)
            }
        }
        _ => first,
    };
    let sides = edge_sides(model, solid, &chain[0], &faces[0], [first, second]);
    Some((largest, sides))
}

/// An automatic chamfer read as the equal-distance one it is while no
/// face runs out.
fn auto_as_equal(
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
    follow_tangent: bool,
    overflow: Overflow,
) -> kernel_api::SolidOp {
    let distance = match spec {
        ChamferSpec::Auto { distance } => *distance,
        _ => 0.0,
    };
    kernel_api::SolidOp::Chamfer {
        spec: ChamferSpec::EqualDistance { distance },
        flip,
        edges: edges.clone(),
        follow_tangent,
        corner: Default::default(),
        overflow,
    }
}

/// The kernel's section for `profile`.
fn profile(profile: RoundProfile) -> ogeom::fillet::RoundProfile {
    match profile {
        RoundProfile::Elliptical => ogeom::fillet::RoundProfile::Elliptical,
        RoundProfile::Conic { rho } => ogeom::fillet::RoundProfile::Conic(rho),
    }
}

/// The refusal of an op over `chain` the kernel refused, naming the edges
/// that fail: the ones the kernel's refusal names, or where it names
/// none, each tried alone with `build` (the edges by index, at a share of
/// the asked size), and where every one builds alone, the groups of them
/// meeting at a vertex, each edge of a failing group named with the
/// group's reason. Then the largest share of the size at which the whole
/// request builds, and each edge failing alone on its own: the share
/// `largest` reads from the kernel, taken as it is where the kernel's own
/// builds confirmed it, else once a build at it holds; found by halving
/// where that build fails or the kernel cannot read it. The whole
/// request's is the least of its failing edges' (alone, or together
/// where none fails alone) where it builds there, and is read for the
/// whole request only where it does not. Every try runs on its own copy
/// of `model`. A long chain is not tried, and a cancelled try cancels.
#[allow(clippy::too_many_arguments)]
fn refusal(
    what: &str,
    size: f64,
    model: &Model,
    solid: &Shape,
    chain: &[Shape],
    refused: BlendRefusal,
    largest: impl Fn(&Model, &[usize]) -> Option<Reading>,
    build: impl Fn(&mut Model, &[usize], f64) -> ogeom::core::OgeomResult<()>,
) -> Result<Refusal, String> {
    let BlendRefusal {
        edges: blamed,
        error,
    } = refused;
    if matches!(error, ogeom::core::OgeomError::Cancelled) {
        return Err(format!("{what} failed: {error}"));
    }
    let diagnosed = chain.len() <= DIAGNOSED_EDGES;
    let attempt = |picked: &[usize], share: f64| -> Result<Option<String>, String> {
        crate::progress::checkpoint()?;
        let mut scratch = model.clone();
        match build(&mut scratch, picked, share) {
            Ok(()) => Ok(None),
            Err(ogeom::core::OgeomError::Cancelled) => Err("cancelled".into()),
            Err(e) => Ok(Some(e.to_string())),
        }
    };
    let mut failing: Vec<(usize, String)> = Vec::new();
    let mut alone: Vec<usize> = Vec::new();
    let blamed: Vec<usize> = blamed.into_iter().filter(|&i| i < chain.len()).collect();
    if !blamed.is_empty() {
        failing = blamed.iter().map(|&i| (i, error.to_string())).collect();
        failing.sort_by_key(|(i, _)| *i);
        if let [only] = blamed[..] {
            alone.push(only);
        }
    } else if chain.len() > 1 && diagnosed {
        let attempt = |picked: &[usize]| attempt(picked, 1.0);
        for i in 0..chain.len() {
            if let Some(reason) = attempt(&[i])? {
                alone.push(i);
                failing.push((i, reason));
            }
        }
        for groups in [vertex_groups(model, chain), face_pairs(model, solid, chain)] {
            if !failing.is_empty() {
                break;
            }
            for group in groups {
                if let Some(reason) = attempt(&group)? {
                    for i in group {
                        if !failing.iter().any(|(j, _)| *j == i) {
                            failing.push((i, reason.clone()));
                        }
                    }
                }
            }
            failing.sort_by_key(|(i, _)| *i);
        }
    } else if chain.len() == 1 {
        failing.push((0, error.to_string()));
        alone.push(0);
    }
    let sized = |picked: &[usize]| -> Result<Option<f64>, String> {
        let builds = |share| Ok(attempt(picked, share)?.is_none());
        match largest(model, picked) {
            Some(Reading::Built(None) | Reading::Seated(None)) => Ok(None),
            Some(Reading::Built(Some(share))) if share < 1.0 => Ok(Some(share)),
            Some(Reading::Seated(Some(share))) if share < 1.0 => {
                if attempt(picked, share)?.is_none() {
                    return Ok(Some(share));
                }
                largest_share(share, builds)
            }
            // A reading that the asked size builds, which it did not.
            _ => largest_share(1.0, builds),
        }
    };
    let every: Vec<usize> = (0..chain.len()).collect();
    let mut fits_alone: Vec<(usize, f64)> = Vec::new();
    if chain.len() > 1 {
        for &i in alone.iter().take(SIZED_ALONE) {
            if let Some(share) = sized(&[i])? {
                fits_alone.push((i, share));
            }
        }
    }
    // The whole request takes no more than an edge failing alone does, or
    // the failing edges together where none fails alone: where it builds
    // at the least of theirs, one build answers what a reading of the
    // whole request would.
    let failing_edges: Vec<usize> = failing.iter().map(|(i, _)| *i).collect();
    let least_alone = match fits_alone.iter().map(|(_, share)| *share).reduce(f64::min) {
        None if diagnosed && !failing_edges.is_empty() && failing_edges.len() < chain.len() => {
            sized(&failing_edges)?
        }
        least => least,
    };
    let fits = match least_alone {
        _ if !diagnosed => None,
        Some(share) if attempt(&every, share)?.is_none() => Some(share),
        _ => sized(&every)?,
    };
    if chain.len() == 1
        && let Some(share) = fits
    {
        fits_alone.push((0, share));
    }
    let of = chain.len();
    let names = crate::naming::current_edge_faces(model, chain);
    let edges: Vec<kernel_api::FailingEdge> = chain
        .iter()
        .zip(&names)
        .enumerate()
        .map(|(index, (edge, faces))| failing_edge(model, edge, *faces, index, of, String::new()))
        .collect();
    let request: Vec<kernel_api::EdgeProbe> = edges.iter().map(|e| e.probe).collect();
    let failing: Vec<kernel_api::FailingEdge> = failing
        .into_iter()
        .map(|(index, reason)| kernel_api::FailingEdge {
            reason,
            fits: fits_alone
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, share)| *share),
            ..edges[index].clone()
        })
        .collect();
    let mut message = if failing.is_empty() {
        format!("{what} failed: {error}")
    } else {
        format!("{what} failed: {}", failing_list(&failing))
    };
    if let Some(share) = fits {
        message.push_str(&format!("; {:.2} mm fits every edge", size * share));
    }
    Ok(Refusal {
        message,
        edges: kernel_api::EdgeRefusal {
            request,
            failing,
            fits,
        },
    })
}

/// The fewest share of the bound a halving search goes down to: a request
/// that fails there is not for its size.
const LEAST_SHARE: f64 = 1.0 / 64.0;

/// How close a halving search finds the largest share that fits, as a
/// share of its bound.
const SHARE_PRECISION: f64 = 0.004;

/// The most failing edges each sized down on their own.
const SIZED_ALONE: usize = 8;

/// The largest share below `bound` at which `builds` holds, found by
/// halving between [`LEAST_SHARE`] of the bound and the bound; `None`
/// where even the least does not.
fn largest_share(
    bound: f64,
    mut builds: impl FnMut(f64) -> Result<bool, String>,
) -> Result<Option<f64>, String> {
    let least = LEAST_SHARE * bound;
    if !builds(least)? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (least, bound);
    while hi - lo > SHARE_PRECISION * bound {
        let mid = (lo + hi) / 2.0;
        if builds(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(Some(lo))
}

/// "edge 3 of 12: why", the edges one reason refuses listed together:
/// "edges 1, 2 of 12: why".
fn failing_list(failing: &[kernel_api::FailingEdge]) -> String {
    let mut reasons: Vec<(&str, Vec<usize>, usize)> = Vec::new();
    for f in failing {
        match reasons.iter_mut().find(|(r, _, _)| *r == f.reason) {
            Some((_, edges, _)) => edges.push(f.index + 1),
            None => reasons.push((&f.reason, vec![f.index + 1], f.of)),
        }
    }
    reasons
        .into_iter()
        .map(|(reason, edges, of)| {
            let listed: Vec<String> = edges.iter().map(usize::to_string).collect();
            let noun = if edges.len() == 1 { "edge" } else { "edges" };
            format!("{noun} {} of {of}: {reason}", listed.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The point halfway along a polyline by length, and the way it runs there.
fn halfway(points: &[[f64; 3]]) -> Option<([f64; 3], [f64; 3])> {
    let span = |a: [f64; 3], b: [f64; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let total: f64 = points.windows(2).map(|w| length(span(w[0], w[1]))).sum();
    let mut left = total / 2.0;
    for w in points.windows(2) {
        let d = span(w[0], w[1]);
        let l = length(d);
        if l > 0.0 && left <= l {
            let t = left / l;
            let point = [w[0][0] + d[0] * t, w[0][1] + d[1] * t, w[0][2] + d[2] * t];
            return Some((point, d));
        }
        left -= l;
    }
    None
}

/// The most pairs of edges sharing a face a refusal is tried for.
const DIAGNOSED_PAIRS: usize = 128;

/// The pairs of `chain` (by index) bordering one face, whose bands may
/// overlap across it; none past [`DIAGNOSED_PAIRS`].
fn face_pairs(model: &Model, solid: &Shape, chain: &[Shape]) -> Vec<Vec<usize>> {
    let faces: Vec<Vec<Shape>> = chain
        .iter()
        .map(|edge| ancestors_of(model, solid, edge, ShapeType::Face).unwrap_or_default())
        .collect();
    let mut pairs = Vec::new();
    for i in 0..chain.len() {
        for j in i + 1..chain.len() {
            if faces[i]
                .iter()
                .any(|f| faces[j].iter().any(|g| g.is_same(f)))
            {
                if pairs.len() == DIAGNOSED_PAIRS {
                    return Vec::new();
                }
                pairs.push(vec![i, j]);
            }
        }
    }
    pairs
}

/// The groups of `chain` (by index) meeting at a vertex, two or more each.
fn vertex_groups(model: &Model, chain: &[Shape]) -> Vec<Vec<usize>> {
    let mut groups: Vec<(Shape, Vec<usize>)> = Vec::new();
    for (i, edge) in chain.iter().enumerate() {
        let Ok(Some((a, b))) = ogeom::algo::edge_vertices(model, edge) else {
            continue;
        };
        for vertex in [a, b] {
            match groups.iter_mut().find(|(v, _)| v.is_same(&vertex)) {
                Some((_, members)) if !members.contains(&i) => members.push(i),
                Some(_) => {}
                None => groups.push((vertex, vec![i])),
            }
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (_, members) in groups {
        if members.len() > 1 && !out.contains(&members) {
            out.push(members);
        }
    }
    out
}

/// A failing edge as the app is told of it: where it lies, a pick that
/// names it, and the kernel's reason.
fn failing_edge(
    model: &Model,
    edge: &Shape,
    faces: [kernel_api::TopoName; 2],
    index: usize,
    of: usize,
    reason: String,
) -> kernel_api::FailingEdge {
    let points: Vec<[f64; 3]> = ogeom::mesh::Deflection::with_chord(0.01)
        .ok()
        .and_then(|d| ogeom::mesh::polyline_of_edge(model, edge, d, tol()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(array)
        .collect();
    let probe = halfway(&points)
        .map(|(point, direction)| kernel_api::EdgeProbe {
            point,
            direction,
            faces,
        })
        .unwrap_or_default();
    kernel_api::FailingEdge {
        index,
        of,
        probe,
        points,
        reason,
        fits: None,
    }
}

/// The kernel's reading of the largest size a fillet or chamfer builds at,
/// up to a bound.
#[derive(Debug, Clone, Copy)]
enum Reading {
    /// Confirmed by the kernel's own builds of the request: every size it
    /// probed up to this one builds, and one a ten-thousandth above it is
    /// refused. `None` where none builds.
    Built(Option<f64>),
    /// Read from the seats without building, which a build has to
    /// confirm: bevels on several edges, which the kernel holds clear of
    /// their faces' far sides and of each other but does not build
    /// together, so a corner where they meet goes unseen. `None` where
    /// none fits.
    Seated(Option<f64>),
}

impl Reading {
    /// The reading as a share of `size`.
    fn per(self, size: f64) -> Self {
        match self {
            Reading::Built(read) => Reading::Built(read.map(|r| r / size)),
            Reading::Seated(read) => Reading::Seated(read.map(|r| r / size)),
        }
    }
}

/// The largest radius up to `up_to` at which a round of `edges` closed as
/// `corner` says builds, read by the kernel; `None` where it cannot read
/// it.
fn fillet_reading(
    model: &Model,
    solid: &Shape,
    edges: &[Shape],
    corner: ogeom::fillet::FilletCorner,
    overflow: Overflow,
    up_to: f64,
) -> Option<Reading> {
    let sizing = BlendSizing::Rounds { edges, corner };
    let limit = blend_limit(model, solid, &sizing, up_to, tol()).ok()?;
    Some(under(Reading::Built(limit.size), overflow))
}

/// The largest share up to `up_to` of the bevels `specs` asks, each taken
/// that share of the way, at which they build closed as `corner` says,
/// read by the kernel: one edge's confirmed by its builds; several edges'
/// read from their seats together (overlaps across a face they share
/// included) where one measure scales them all, else the least of each
/// edge's own. `None` where the kernel cannot read it.
fn chamfer_reading(
    model: &Model,
    solid: &Shape,
    specs: &[(Shape, Chamfer)],
    corner: ogeom::fillet::ChamferCorner,
    overflow: Overflow,
    up_to: f64,
) -> Option<Reading> {
    seated_chamfer_reading(model, solid, specs, corner, up_to).map(|r| under(r, overflow))
}

/// [`chamfer_reading`] with every overflow behaviour on.
fn seated_chamfer_reading(
    model: &Model,
    solid: &Shape,
    specs: &[(Shape, Chamfer)],
    corner: ogeom::fillet::ChamferCorner,
    up_to: f64,
) -> Option<Reading> {
    // The measure the kernel reads each bevel by, at a share of one.
    let measure = |spec: &Chamfer| match spec {
        Chamfer::Symmetric(d) => *d,
        Chamfer::Distances { .. } => 1.0,
        Chamfer::Angle { distance, .. } => *distance,
    };
    let alone = |edge: &Shape, spec: &Chamfer| -> Option<Option<f64>> {
        let per = measure(spec);
        let sizing = BlendSizing::Chamfer { edge, spec, corner };
        let limit = blend_limit(model, solid, &sizing, up_to * per, tol()).ok()?;
        Some(limit.size.map(|m| m / per))
    };
    match specs {
        [] => None,
        [(edge, spec)] => Some(Reading::Built(alone(edge, spec)?)),
        [(_, first), ..] => {
            let per = measure(first);
            if specs
                .iter()
                .all(|(_, spec)| (measure(spec) - per).abs() <= per * 1e-12)
            {
                let read = largest_chamfer_distance_all(model, solid, specs, up_to * per, tol());
                return Some(Reading::Seated(read.ok()?.map(|m| m / per)));
            }
            let mut least = up_to;
            for (edge, spec) in specs {
                match alone(edge, spec)? {
                    Some(share) => least = least.min(share),
                    None => return Some(Reading::Seated(None)),
                }
            }
            Some(Reading::Seated(Some(least)))
        }
    }
}

/// Whether the bevels `specs` asks, each taken `share` of the way, build
/// on `solid` closed as `corner` says and reaching other geometry as
/// `overflow` says, on a copy of `model`.
fn chamfer_builds(
    model: &Model,
    solid: &Shape,
    specs: &[(Shape, Chamfer)],
    corner: ogeom::fillet::ChamferCorner,
    overflow: Overflow,
    share: f64,
) -> bool {
    let scaled: Vec<(Shape, Chamfer)> = specs
        .iter()
        .map(|(edge, spec)| (edge.clone(), scaled(spec, share)))
        .collect();
    bevel(&mut model.clone(), solid, &scaled, corner, overflow).is_ok()
}

/// The kernel's spelling of how a chamfer closes its corners.
fn kernel_chamfer_corner(corner: ChamferCorner) -> ogeom::fillet::ChamferCorner {
    match corner {
        ChamferCorner::Point => ogeom::fillet::ChamferCorner::Point,
        ChamferCorner::Facet => ogeom::fillet::ChamferCorner::Facet,
    }
}
/// `chamfer` with its distances taken `share` of the way.
fn scaled(chamfer: &Chamfer, share: f64) -> Chamfer {
    match chamfer {
        Chamfer::Symmetric(d) => Chamfer::Symmetric(d * share),
        Chamfer::Distances {
            face,
            on_face,
            on_other,
        } => Chamfer::Distances {
            face: face.clone(),
            on_face: on_face * share,
            on_other: on_other * share,
        },
        Chamfer::Angle {
            face,
            distance,
            angle,
        } => Chamfer::Angle {
            face: face.clone(),
            distance: distance * share,
            angle: *angle,
        },
    }
}

/// Round between the two faces of `solid` the probes name, which need
/// share no edge.
pub fn blend(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    faces: &[kernel_api::FaceProbe; 2],
) -> Result<Shape, String> {
    let [a, b] = faces;
    let first = face_named_or_nearest(model, solid, Some(a.name).filter(|n| *n != 0), a.point)?;
    let second = face_named_or_nearest(model, solid, Some(b.name).filter(|n| *n != 0), b.point)?;
    if first.is_same(&second) {
        return Err("a fillet between faces needs two different faces".into());
    }
    ogeom::fillet::blend_faces(model, solid, &first, &second, radius, tol())
        .map(|built| {
            crate::naming::record(&built.history);
            built.shape
        })
        .map_err(|e| format!("fillet between faces failed: {e}"))
}

/// The edges of `solid` the probes name, each once. A pick names the
/// nearest edge within reach that runs its way; one with none fails the
/// selection.
fn chain_of(
    model: &mut Model,
    solid: &Shape,
    (probes, picked): (Vec<Probe>, bool),
) -> Result<Vec<Shape>, String> {
    let reach = if picked {
        crate::probe::Reach::Pick
    } else {
        crate::probe::Reach::Any
    };
    let edges = explore_unique(model, solid, ShapeType::Edge)
        .map_err(|e| format!("exploring the solid failed: {e}"))?;
    let mut chain: Vec<Shape> = Vec::with_capacity(probes.len());
    for probe in probes {
        if let Some(edge) = crate::naming::find_edge(model, solid, probe.faces, probe.point) {
            if !chain.iter().any(|e| e.is_same(&edge)) {
                chain.push(edge);
            }
            continue;
        }
        let mut near: Vec<(f64, Shape)> = edges
            .iter()
            .filter_map(|e| distance_to(model, probe.point, e).map(|d| (d, e.clone())))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let p = probe.point;
        let Some(&(nearest, _)) = near.first() else {
            return Err("the solid has no edges".into());
        };
        let lost = crate::probe::names_lost(!probe.faces.contains(&0));
        let reach = crate::probe::position_reach(model, solid, reach, lost);
        if nearest > reach {
            return Err(if lost {
                crate::probe::lost_too_far("edge", p, nearest, reach)
            } else {
                format!(
                    "no edge near the pick at ({:.3}, {:.3}, {:.3}): the nearest is {nearest:.3} mm away",
                    p.x, p.y, p.z
                )
            });
        }
        let found = near
            .into_iter()
            .take_while(|(d, _)| *d <= reach)
            .find(|(d, edge)| match probe.along {
                Some(along) => runs_along(model, edge, p, along, *d),
                None => true,
            });
        let Some((distance, edge)) = found else {
            let a = probe.along.unwrap_or(Vector::new(0.0, 0.0, 0.0));
            return Err(format!(
                "no edge near the pick at ({:.3}, {:.3}, {:.3}) runs along ({:.3}, {:.3}, {:.3})",
                p.x, p.y, p.z, a.x, a.y, a.z
            ));
        };
        if lost {
            crate::probe::note_found_by_position("edge", p, distance);
        }
        if !chain.iter().any(|e| e.is_same(&edge)) {
            chain.push(edge);
        }
    }
    Ok(chain)
}

/// How far apart two edge ends may lie and still meet at one vertex, mm.
const END_REACH: f64 = 1e-4;

/// The sine of the widest angle two edges may turn through where they
/// meet and still run on tangentially (one degree).
const TANGENT_SINE: f64 = 0.017_452;

/// An edge's two ends, placed: each a point and the way the edge's curve
/// runs there. A degenerate edge, or one with no curve, has none.
fn edge_ends(model: &Model, edge: &Shape) -> Option<[(Point, Vector); 2]> {
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return None;
    };
    if data.degenerate {
        return None;
    }
    let Some(EdgeRepr::Curve3d {
        curve,
        location,
        range,
    }) = data.curve3d()
    else {
        return None;
    };
    let geometry = model.geometry().curve(*curve)?;
    let local = location.composed(model.datums()).ok()?;
    let placed = edge.transform(model.datums()).ok()?;
    let end = |u: f64| -> Option<(Point, Vector)> {
        let point = geometry.point_at(u, tol()).ok()?;
        let along = geometry.d1_at(u, tol()).ok()?;
        Some((
            placed.apply(local.apply(point)),
            placed.apply_vector(local.apply_vector(along)),
        ))
    };
    Some([end(range.0)?, end(range.1)?])
}

/// Whether two ways run along one line, either way round.
fn parallel(a: Vector, b: Vector) -> bool {
    let (la, lb) = (a.magnitude(), b.magnitude());
    la > 1e-12 && lb > 1e-12 && a.cross(b).magnitude() <= TANGENT_SINE * la * lb
}

/// `chain` and every edge of `solid` reached from it through ends where
/// two edges meet tangentially: a picked side of a rounded corner brings
/// the round and the side past it, and stops where the outline turns a
/// corner. Where two creases meet at a point both fade at (each one's two
/// faces turning tangent there, as the seam between two equal drums does
/// at its poles), the chain runs on through it too, whatever the angle
/// between the creases.
fn tangent_chain(
    model: &mut Model,
    solid: &Shape,
    chain: Vec<Shape>,
) -> Result<Vec<Shape>, String> {
    let edges = explore_unique(model, solid, ShapeType::Edge)
        .map_err(|e| format!("exploring the solid failed: {e}"))?;
    let ends: Vec<Option<[(Point, Vector); 2]>> =
        edges.iter().map(|edge| edge_ends(model, edge)).collect();
    let index_of = |shape: &Shape| edges.iter().position(|e| e.is_same(shape));
    let mut taken = vec![false; edges.len()];
    let mut queue: Vec<usize> = Vec::new();
    for edge in &chain {
        if let Some(i) = index_of(edge)
            && !taken[i]
        {
            taken[i] = true;
            queue.push(i);
        }
    }
    let mut fades: std::collections::HashMap<(usize, usize), bool> = Default::default();
    let mut fading = |model: &mut Model, i: usize, end: usize, at: Point| -> bool {
        *fades
            .entry((i, end))
            .or_insert_with(|| fades_at(model, solid, &edges[i], at))
    };
    let mut result = chain;
    while let Some(i) = queue.pop() {
        let Some(mine) = ends[i] else { continue };
        for (j, theirs) in ends.iter().enumerate() {
            let Some(theirs) = theirs else { continue };
            if taken[j] {
                continue;
            }
            let mut meets = false;
            for (a, (p, t)) in mine.iter().enumerate() {
                for (b, (q, s)) in theirs.iter().enumerate() {
                    if p.distance(*q) > END_REACH {
                        continue;
                    }
                    if parallel(*t, *s) || (fading(model, i, a, *p) && fading(model, j, b, *q)) {
                        meets = true;
                    }
                }
            }
            if meets {
                taken[j] = true;
                queue.push(j);
                result.push(edges[j].clone());
            }
        }
    }
    Ok(result)
}

/// Whether `edge`'s two faces turn tangent at `at`, an end of it: their
/// outward normals there agree within the tangent angle.
fn fades_at(model: &mut Model, solid: &Shape, edge: &Shape, at: Point) -> bool {
    let Ok(faces) = faces_of_edge(model, solid, edge) else {
        return false;
    };
    let [a, b] = faces.as_slice() else {
        return false;
    };
    let normal = |model: &mut Model, face: &Shape| crate::probe::outward_at(model, face, at).ok();
    match (normal(model, a), normal(model, b)) {
        (Some((_, n)), Some((_, m))) => n.dot(m) > 0.0 && parallel(n, m),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments, reason = "the chamfer op's own fields")]
pub fn chamfer(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
    follow_tangent: bool,
    corner: ChamferCorner,
    overflow: Overflow,
) -> Result<Shape, Refusal> {
    let asked = match spec {
        ChamferSpec::EqualDistance { distance }
        | ChamferSpec::Auto { distance }
        | ChamferSpec::DistanceAngle { distance, .. } => *distance,
        ChamferSpec::TwoDistances { distance1, .. } => *distance1,
    };
    let chain = edge_chain(model, solid, edges, follow_tangent, "chamfer")?;
    let specs = chamfer_specs(model, solid, spec, flip, chain)?;
    let edges: Vec<Shape> = specs.iter().map(|(edge, _)| edge.clone()).collect();
    let kernel_corner = kernel_chamfer_corner(corner);
    let largest = |m: &Model, picked: &[usize]| {
        let picked: Vec<(Shape, Chamfer)> = picked.iter().map(|&i| specs[i].clone()).collect();
        chamfer_reading(m, solid, &picked, kernel_corner, overflow, 1.0)
    };
    let picked_at = |picked: &[usize], share: f64| -> Vec<(Shape, Chamfer)> {
        picked
            .iter()
            .map(|&i| (specs[i].0.clone(), scaled(&specs[i].1, share)))
            .collect()
    };
    let error = match bevel(model, solid, &specs, kernel_corner, overflow) {
        Ok(built) => {
            crate::naming::record(&built.history);
            return Ok(built.shape);
        }
        Err(error) => error,
    };
    // The facet's checked call names the specs a refusal concerns where
    // the engine's own bevels refuse too; else the edges are tried apart.
    let blamed = match corner {
        ChamferCorner::Facet if !matches!(error, ogeom::core::OgeomError::Cancelled) => {
            chamfer_edges_checked(&mut model.clone(), solid, &specs, tol())
                .err()
                .map(|refused| refused.edges)
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Err(refusal(
        "chamfer",
        asked,
        model,
        solid,
        &edges,
        BlendRefusal {
            edges: blamed,
            error,
        },
        largest,
        |m, picked, share| {
            bevel(m, solid, &picked_at(picked, share), kernel_corner, overflow).map(|_| ())
        },
    )?)
}

/// The edges of `solid` a fillet or chamfer (`what`) takes: the selected
/// ones and, with `follow_tangent`, the edges running on from them
/// tangentially.
fn edge_chain(
    model: &mut Model,
    solid: &Shape,
    edges: &EdgeSelection,
    follow_tangent: bool,
    what: &str,
) -> Result<Vec<Shape>, String> {
    let chain = selected_edges(model, solid, edges)?;
    if chain.is_empty() {
        return Err(format!("{what} selection matches no edges"));
    }
    if follow_tangent {
        return tangent_chain(model, solid, chain);
    }
    Ok(chain)
}

/// Each edge of `chain` with the bevel `spec` asks of it, measured from
/// the face `flip` chooses.
fn chamfer_specs(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    chain: Vec<Shape>,
) -> Result<Vec<(Shape, Chamfer)>, String> {
    let faces = match spec {
        ChamferSpec::EqualDistance { .. } | ChamferSpec::Auto { .. } => Vec::new(),
        _ => reference_faces(model, solid, &chain, flip)?,
    };
    let mut specs = Vec::with_capacity(chain.len());
    for (i, edge) in chain.iter().cloned().enumerate() {
        let spec = match spec {
            ChamferSpec::EqualDistance { distance } => Chamfer::Symmetric(*distance),
            ChamferSpec::Auto { distance } => auto_bevel(model, solid, &edge, *distance)?,
            ChamferSpec::TwoDistances {
                distance1,
                distance2,
            } => Chamfer::Distances {
                face: faces[i].clone(),
                on_face: *distance1,
                on_other: *distance2,
            },
            ChamferSpec::DistanceAngle {
                distance,
                angle_deg,
            } => Chamfer::Angle {
                face: faces[i].clone(),
                distance: *distance,
                angle: angle_deg.to_radians(),
            },
        };
        specs.push((edge, spec));
    }
    Ok(specs)
}

/// An automatic bevel of `edge`: `distance` on each face, or as far as a
/// face too narrow for it goes.
fn auto_bevel(
    model: &Model,
    solid: &Shape,
    edge: &Shape,
    distance: f64,
) -> Result<Chamfer, String> {
    let face = adjacent_face(model, solid, edge, false)?;
    let [on_face, on_other] = chamfer_side_limits(model, solid, edge, &face)
        .map(|most| distance.min(most.unwrap_or(distance)));
    Ok(if on_face >= distance && on_other >= distance {
        Chamfer::Symmetric(distance)
    } else {
        Chamfer::Distances {
            face,
            on_face,
            on_other,
        }
    })
}

/// What [`size_limit`] reads: the op's first size at its limit, and the
/// faces beside a round or chamfer of distances at its first edge.
pub type Limits = (Option<f64>, Option<kernel_api::RoundSides>);

pub fn size_limit(model: &Model, solid: &Shape, op: &kernel_api::SolidOp) -> Limits {
    if let kernel_api::SolidOp::Fillet {
        edges,
        follow_tangent,
        distances,
        auto,
        ..
    } = op
        && (distances.is_some() || *auto)
    {
        let flip = distances.is_some_and(|d| d.flip);
        let auto = distances.is_none();
        let sides = round_sides(
            &mut model.clone(),
            solid,
            edges,
            *follow_tangent,
            flip,
            auto,
        );
        if let Some(sides) = sides {
            let [first, second] = sides.sides.map(|side| side.largest);
            // Two distances stop each at its own face; one size grows on
            // until both faces have run out.
            let largest = match (distances.is_some(), first, second) {
                (true, first, _) => first,
                // Neither face runs out: what stops the round is its own.
                (false, None, None) => largest_size(model, solid, op),
                (false, first, second) => first.zip(second).map(|(a, b)| a.max(b)),
            };
            return (largest, Some(sides));
        }
        if distances.is_some() {
            return (None, None);
        }
    }
    if let kernel_api::SolidOp::Chamfer {
        spec: spec @ (ChamferSpec::Auto { .. } | ChamferSpec::TwoDistances { .. }),
        flip,
        edges,
        follow_tangent,
        overflow,
        ..
    } = op
        && let Some(read) = chamfer_sides(
            &mut model.clone(),
            solid,
            spec,
            *flip,
            edges,
            *follow_tangent,
            *overflow,
        )
    {
        return read;
    }
    (largest_size(model, solid, op), None)
}

/// [`size_limit`]'s first size for an op of one size: the largest first
/// size `op`, a fillet or chamfer of one size, builds at on `solid` (a
/// fillet's radius, a chamfer's distance; with two distances, the first at
/// their common scale). A fillet's and a one-edge chamfer's are the
/// kernel's reading, confirmed by its own builds; a chamfer on several
/// edges is read from its seats and checked by one build there, and found
/// by halving below it where that build fails. `None` where every size
/// short of the solid's diagonal builds, where none does, or for any other
/// op.
fn largest_size(model: &Model, solid: &Shape, op: &kernel_api::SolidOp) -> Option<f64> {
    let span = crate::tess::robust_bounds(model, solid).map(|(lo, hi)| (hi - lo).magnitude())?;
    let mut model = model.clone();
    let below_span = |size: f64| (size < span * (1.0 - 1e-9)).then_some(size);
    match op {
        kernel_api::SolidOp::Fillet {
            edges,
            follow_tangent,
            corner,
            variable: None,
            overflow,
            ..
        } => {
            let chain = edge_chain(&mut model, solid, edges, *follow_tangent, "fillet").ok()?;
            let corner = kernel_corner(*corner);
            match fillet_reading(&model, solid, &chain, corner, *overflow, span)? {
                Reading::Built(read) => below_span(read?),
                // Read with every behaviour on: a build under the asked
                // ones confirms it, or the halving finds theirs below.
                Reading::Seated(read) => {
                    let read = read?;
                    let builds = |r| round(&mut model.clone(), solid, &chain, r, corner, *overflow);
                    if builds(read).is_ok() {
                        below_span(read)
                    } else {
                        largest_share(read, |r| Ok(builds(r).is_ok())).ok()?
                    }
                }
            }
        }
        kernel_api::SolidOp::Chamfer {
            spec,
            flip,
            edges,
            follow_tangent,
            corner,
            overflow,
        } => {
            let chain = edge_chain(&mut model, solid, edges, *follow_tangent, "chamfer").ok()?;
            let specs = chamfer_specs(&mut model, solid, spec, *flip, chain).ok()?;
            let first = match spec {
                ChamferSpec::EqualDistance { distance }
                | ChamferSpec::Auto { distance }
                | ChamferSpec::DistanceAngle { distance, .. } => *distance,
                ChamferSpec::TwoDistances { distance1, .. } => *distance1,
            };
            if first <= 0.0 {
                return None;
            }
            let corner = kernel_chamfer_corner(*corner);
            let bound = span / first;
            let share = match chamfer_reading(&model, solid, &specs, corner, *overflow, bound)? {
                Reading::Built(share) => share?,
                Reading::Seated(share) => {
                    let share = share?;
                    let builds =
                        |share| chamfer_builds(&model, solid, &specs, corner, *overflow, share);
                    if share >= bound * (1.0 - 1e-9) || builds(share) {
                        share
                    } else {
                        largest_share(share, |share| Ok(builds(share))).ok()??
                    }
                }
            };
            below_span(share * first)
        }
        _ => None,
    }
}
/// One of the two faces sharing the edge; `flip` selects the other.
fn adjacent_face(model: &Model, solid: &Shape, edge: &Shape, flip: bool) -> Result<Shape, String> {
    let unique = faces_of_edge(model, solid, edge)?;
    let idx = usize::from(flip && unique.len() > 1);
    unique
        .into_iter()
        .nth(idx)
        .ok_or_else(|| "the edge borders no face of the solid".to_string())
}

/// The face each edge of `chain` measures its first distance on: the
/// picked edges' as `flip` says, and along a tangent run the face on the
/// same side as the edge it continues, so the distances keep to their
/// faces where the faces change from edge to edge.
fn reference_faces(
    model: &Model,
    solid: &Shape,
    chain: &[Shape],
    flip: bool,
) -> Result<Vec<Shape>, String> {
    let faces: Vec<Vec<Shape>> = chain
        .iter()
        .map(|edge| faces_of_edge(model, solid, edge))
        .collect::<Result<_, _>>()?;
    let ends: Vec<Option<[(Point, Vector); 2]>> =
        chain.iter().map(|edge| edge_ends(model, edge)).collect();
    let meets = |i: usize, j: usize| match (ends[i], ends[j]) {
        (Some(a), Some(b)) => a.iter().any(|(p, t)| {
            b.iter()
                .any(|(q, s)| p.distance(*q) <= END_REACH && parallel(*t, *s))
        }),
        _ => false,
    };
    let mut chosen: Vec<Option<Shape>> = vec![None; chain.len()];
    for seed in 0..chain.len() {
        if chosen[seed].is_some() {
            continue;
        }
        chosen[seed] = Some(adjacent_face(model, solid, &chain[seed], flip)?);
        let mut queue = vec![seed];
        while let Some(i) = queue.pop() {
            let mine = chosen[i].clone().expect("chosen before queued");
            let other = faces[i].iter().find(|f| !f.is_same(&mine)).cloned();
            for j in 0..chain.len() {
                if chosen[j].is_some() || !meets(i, j) {
                    continue;
                }
                let theirs = &faces[j];
                // The same face, or across from the same other face, or
                // the face that runs on from this one across an edge.
                let pick = theirs
                    .iter()
                    .find(|f| f.is_same(&mine))
                    .or_else(|| {
                        other.as_ref().and_then(|other| {
                            theirs
                                .iter()
                                .any(|f| f.is_same(other))
                                .then(|| theirs.iter().find(|f| !f.is_same(other)))
                                .flatten()
                        })
                    })
                    .or_else(|| theirs.iter().find(|f| share_an_edge(model, f, &mine)))
                    .cloned();
                let Some(pick) = pick else { continue };
                chosen[j] = Some(pick);
                queue.push(j);
            }
        }
    }
    chosen
        .into_iter()
        .zip(chain)
        .map(|(face, edge)| match face {
            Some(face) => Ok(face),
            None => adjacent_face(model, solid, edge, flip),
        })
        .collect()
}

/// The faces of `solid` the edge runs between, each once.
fn faces_of_edge(model: &Model, solid: &Shape, edge: &Shape) -> Result<Vec<Shape>, String> {
    let faces = ancestors_of(model, solid, edge, ShapeType::Face)
        .map_err(|e| format!("finding the edge's faces failed: {e}"))?;
    // ancestors_of yields per route; dedupe.
    let mut unique: Vec<Shape> = Vec::new();
    for f in faces {
        if !unique.iter().any(|u| u.is_same(&f)) {
            unique.push(f);
        }
    }
    Ok(unique)
}

/// Whether two faces meet along an edge.
fn share_an_edge(model: &Model, a: &Shape, b: &Shape) -> bool {
    let (Ok(ea), Ok(eb)) = (
        explore_unique(model, a, ShapeType::Edge),
        explore_unique(model, b, ShapeType::Edge),
    ) else {
        return false;
    };
    ea.iter().any(|x| eb.iter().any(|y| x.is_same(y)))
}

pub fn draft(
    model: &mut Model,
    solid: &Shape,
    angle_deg: f64,
    neutral_point: [f64; 3],
    neutral_normal: [f64; 3],
    pull_dir: Option<[f64; 3]>,
    faces: &[([f64; 3], kernel_api::TopoName)],
) -> Result<Shape, String> {
    if faces.is_empty() {
        return Err("draft has no selected faces".into());
    }
    let normal = Direction::new(vector(neutral_normal), tol())
        .map_err(|_| "draft neutral normal is (near) zero".to_string())?;
    let neutral = Plane::through(point(neutral_point), normal);
    let pull = match pull_dir {
        Some(d) => Direction::new(vector(d), tol())
            .map_err(|_| "draft pull direction is (near) zero".to_string())?,
        None => normal,
    };
    let mut found = Vec::with_capacity(faces.len());
    for (point, name) in faces {
        found.push(face_named_or_nearest(model, solid, Some(*name), *point)?);
    }
    apply_draft(
        model,
        solid,
        &found,
        neutral,
        pull,
        angle_deg.to_radians(),
        tol(),
    )
    .map(|b| {
        crate::naming::record(&b.history);
        b.shape
    })
    .map_err(|e| format!("draft failed: {e}"))
}

/// The faces at `points` of `solid`, by name first.
fn faces_at(
    model: &mut Model,
    solid: &Shape,
    points: &[[f64; 3]],
    names: &[kernel_api::TopoName],
    what: &str,
) -> Result<Vec<Shape>, String> {
    if points.is_empty() {
        return Err(format!("pick at least one face to {what}"));
    }
    let mut faces = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        faces.push(face_named_or_nearest(
            model,
            solid,
            names.get(i).copied(),
            *p,
        )?);
    }
    Ok(faces)
}

/// Offset the faces at `points` of `solid` along their outward normals,
/// their neighbours following.
pub fn offset_faces_of(
    model: &mut Model,
    solid: &Shape,
    points: &[[f64; 3]],
    names: &[kernel_api::TopoName],
    distance: f64,
) -> Result<Shape, String> {
    let faces = faces_at(model, solid, points, names, "offset")?;
    offset_faces(model, solid, &faces, distance, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("offsetting the faces failed: {e}"))
}

/// Offset the whole of `solid` by `distance` along its faces' outward
/// normals: positive grows it, negative shrinks it.
pub fn offset_solid(model: &mut Model, solid: &Shape, distance: f64) -> Result<Shape, String> {
    offset_shape(model, solid, distance, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("offsetting the solid failed: {e}"))
}

/// Move the faces at `points` of `solid` by the rigid row-major
/// `transform`, their neighbours following.
pub fn move_faces_of(
    model: &mut Model,
    solid: &Shape,
    points: &[[f64; 3]],
    names: &[kernel_api::TopoName],
    transform: &[[f64; 4]; 4],
) -> Result<Shape, String> {
    let faces = faces_at(model, solid, points, names, "move")?;
    let motion = crate::ops::pattern::rigid_of(transform)
        .ok_or("faces move by a translation or a rotation")?;
    move_faces(model, solid, &faces, &motion, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("moving the faces failed: {e}"))
}

/// Remove the faces at `points` (by name first) from `solid` and close
/// the openings from the neighbours' own geometry.
pub fn remove_faces(
    model: &mut Model,
    solid: &Shape,
    points: &[[f64; 3]],
    names: &[kernel_api::TopoName],
) -> Result<Shape, String> {
    if points.is_empty() {
        return Err("pick at least one face to delete".into());
    }
    let mut faces = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        faces.push(face_named_or_nearest(
            model,
            solid,
            names.get(i).copied(),
            *p,
        )?);
    }
    ogeom::boolean::remove_faces(model, solid, &faces, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("deleting the faces failed: {e}"))
}

/// Hollow `solid` into walls `value` thick, opened at the faces named,
/// inward or outward (`side`), or both ways at once when `side` is `None`:
/// the inward and outward walls fused along the solid's own faces.
pub fn thickness(
    model: &mut Model,
    solid: &Shape,
    value: f64,
    open_face_points: &[[f64; 3]],
    open_face_names: &[kernel_api::TopoName],
    side: Option<bool>,
    join: ThicknessJoin,
) -> Result<Shape, String> {
    let Some(inward) = side else {
        let inner = thickness(
            model,
            solid,
            value,
            open_face_points,
            open_face_names,
            Some(true),
            join,
        )?;
        let outer = thickness(
            model,
            solid,
            value,
            open_face_points,
            open_face_names,
            Some(false),
            join,
        )?;
        return ogeom::boolean::fuse(model, &inner, &outer, tol())
            .map(|b| {
                crate::naming::record(&b.history);
                b.shape
            })
            .map_err(|e| format!("joining the two sides' walls failed: {e}"));
    };
    let mut removed = Vec::with_capacity(open_face_points.len());
    for (i, p) in open_face_points.iter().enumerate() {
        removed.push(face_named_or_nearest(
            model,
            solid,
            open_face_names.get(i).copied(),
            *p,
        )?);
    }
    // Positive thickness hollows inward; negative builds the walls outward
    // around the solid.
    let signed = if inward { value } else { -value };
    let join = match join {
        ThicknessJoin::Arc => Join::Arc,
        ThicknessJoin::Intersection => Join::Intersection,
    };
    make_thick_solid_with(model, solid, &removed, signed, join, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("thickness failed: {e}"))
}
