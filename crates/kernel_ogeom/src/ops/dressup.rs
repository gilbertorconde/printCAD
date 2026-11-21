//! Fillet, chamfer, draft, thickness and face edits (offset, move, remove)
//! on the running solid, with geometric (point-based) edge and face
//! selection.
//!
//! A fillet or a chamfer goes to the kernel as one chain, every selected
//! edge at once, resolved on the solid as it stands, so blends and bevels
//! that meet at a vertex close their corner between them.

use kernel_api::{
    ChamferCorner, ChamferSpec, EdgeSelection, FilletCorner, RoundDistances, RoundProfile,
    ThicknessJoin,
};
use ogeom::algo::distance_between_shapes;
use ogeom::fillet::{
    Blend, BlendRefusal, BlendSizing, Chamfer, blend_edges, blend_limit, chamfer_edges_checked,
    chamfer_edges_cornered, chamfer_edges_with, fillet_edge_distances, fillet_edges_corner,
    fillet_edges_corner_checked, largest_chamfer_distance, largest_fillet_radius_all,
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
        let built = blend_edges(
            model,
            solid,
            &rounds,
            ogeom::fillet::ChamferCorner::Facet,
            tol(),
        )
        .map_err(|e| format!("fillet with two distances failed: {e}"))?;
        crate::naming::record(&built.history);
        return Ok(built.shape);
    }
    if auto && let Some(shape) = auto_round(model, solid, radius, edges, follow_tangent)? {
        return Ok(shape);
    }
    let corner = kernel_corner(corner);
    let chain = edge_chain(model, solid, edges, follow_tangent, "fillet")?;
    let pick =
        |picked: &[usize]| -> Vec<Shape> { picked.iter().map(|&i| chain[i].clone()).collect() };
    match fillet_edges_corner_checked(model, solid, &chain, radius, corner, tol()) {
        Ok(built) => {
            crate::naming::record(&built.history);
            Ok(built.shape)
        }
        Err(refused) => Err(refusal(
            "fillet",
            radius,
            model,
            solid,
            &chain,
            refused,
            |m, picked| {
                largest_fillet_radius_all(m, solid, &pick(picked), radius, tol())
                    .ok()
                    .map(|fits| fits.map(|r| r / radius))
            },
            |m, picked, share| {
                fillet_edges_corner(m, solid, &pick(picked), radius * share, corner, tol())
                    .map(|_| ())
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
) -> Result<Shape, String> {
    let chain = edge_chain(model, solid, edges, follow_tangent, "fillet")?;
    let mut run = edge_run(model, &chain)?;
    if variable.reversed {
        run = run.iter().rev().map(Shape::reversed).collect();
    }
    let law = if variable.between.is_empty() {
        ogeom::fillet::RadiusLaw::Linear {
            start,
            end: variable.end,
        }
    } else {
        let mut points = vec![(0.0, start)];
        points.extend(variable.between.iter().map(|[at, radius]| (*at, *radius)));
        points.push((1.0, variable.end));
        ogeom::fillet::RadiusLaw::Points(points)
    };
    let built = ogeom::fillet::fillet_edges_variable(model, solid, &run, &law, tol())
        .map_err(|e| format!("variable fillet failed: {e}"))?;
    crate::naming::record(&built.history);
    Ok(built.shape)
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
    let built = fillet_edge_distances(
        model,
        solid,
        edge,
        &face,
        on_face,
        on_other,
        ogeom::fillet::RoundProfile::Elliptical,
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
/// it runs on past the solid's size or the kernel cannot read it.
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
                    &auto_as_equal(spec, flip, edges, follow_tangent),
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
/// `largest` reads from the kernel (`Some(None)` where nothing fits) when
/// a build at it holds, else found by halving. Every try runs on its own
/// copy of `model`. A long chain is not tried, and a cancelled try
/// cancels.
#[allow(clippy::too_many_arguments)]
fn refusal(
    what: &str,
    size: f64,
    model: &Model,
    solid: &Shape,
    chain: &[Shape],
    refused: BlendRefusal,
    largest: impl Fn(&Model, &[usize]) -> Option<Option<f64>>,
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
        match largest(model, picked) {
            Some(None) => return Ok(None),
            Some(Some(share)) if share < 1.0 && attempt(picked, share)?.is_none() => {
                return Ok(Some(share));
            }
            _ => {}
        }
        largest_share(|share| Ok(attempt(picked, share)?.is_none()))
    };
    let every: Vec<usize> = (0..chain.len()).collect();
    let fits = if diagnosed { sized(&every)? } else { None };
    let mut fits_alone: Vec<(usize, f64)> = Vec::new();
    if chain.len() > 1 {
        for &i in alone.iter().take(SIZED_ALONE) {
            if let Some(share) = sized(&[i])? {
                fits_alone.push((i, share));
            }
        }
    } else if let Some(share) = fits {
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

/// The fewest share of the asked size a refusal is sized down to: one that
/// fails there is not for its size.
const LEAST_SHARE: f64 = 1.0 / 64.0;

/// How close the largest share that fits is found.
const SHARE_PRECISION: f64 = 0.004;

/// The most failing edges each sized down on their own.
const SIZED_ALONE: usize = 8;

/// The largest share of the asked size, below one, at which `builds`
/// holds, found by halving between [`LEAST_SHARE`] and one; `None` where
/// even the least does not.
fn largest_share(
    mut builds: impl FnMut(f64) -> Result<bool, String>,
) -> Result<Option<f64>, String> {
    if !builds(LEAST_SHARE)? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (LEAST_SHARE, 1.0);
    while hi - lo > SHARE_PRECISION {
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

/// The largest share of the asked chamfer each of `picked` takes as the
/// kernel reads it, the least of them; `Some(None)` where one takes none,
/// `None` where the kernel cannot say. Bevels meeting across a face are
/// not read together, so the share is a guess the caller builds to check.
fn largest_chamfer_share(
    model: &Model,
    solid: &Shape,
    specs: &[(Shape, Chamfer)],
    picked: &[usize],
    asked: f64,
) -> Option<Option<f64>> {
    let mut least: f64 = 1.0;
    for &i in picked {
        let (edge, spec) = &specs[i];
        // A distance is read up to the asked one; two distances as their
        // common scale, up to one.
        let (up_to, per) = match spec {
            Chamfer::Distances { .. } => (1.0, 1.0),
            _ => (asked, asked),
        };
        match largest_chamfer_distance(model, solid, edge, spec, up_to, tol()).ok()? {
            Some(measure) => least = least.min(measure / per),
            None => return Some(None),
        }
    }
    Some(Some(least))
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

pub fn chamfer(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
    follow_tangent: bool,
    corner: ChamferCorner,
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
    let largest =
        |m: &Model, picked: &[usize]| largest_chamfer_share(m, solid, &specs, picked, asked);
    let picked_at = |picked: &[usize], share: f64| -> Vec<(Shape, Chamfer)> {
        picked
            .iter()
            .map(|&i| (specs[i].0.clone(), scaled(&specs[i].1, share)))
            .collect()
    };
    // The kernel's plain chamfer closes a corner of three bevels with a
    // facet; the mitre to one point is asked for by name. That call names
    // no edge when it refuses, so the refusal tries the edges apart.
    if corner == ChamferCorner::Point {
        let point = ogeom::fillet::ChamferCorner::Point;
        return match chamfer_edges_cornered(model, solid, &specs, point, tol()) {
            Ok(built) => {
                crate::naming::record(&built.history);
                Ok(built.shape)
            }
            Err(error) => Err(refusal(
                "chamfer",
                asked,
                model,
                solid,
                &edges,
                BlendRefusal {
                    edges: Vec::new(),
                    error,
                },
                largest,
                |m, picked, share| {
                    chamfer_edges_cornered(m, solid, &picked_at(picked, share), point, tol())
                        .map(|_| ())
                },
            )?),
        };
    }
    match chamfer_edges_checked(model, solid, &specs, tol()) {
        Ok(built) => {
            crate::naming::record(&built.history);
            Ok(built.shape)
        }
        Err(refused) => Err(refusal(
            "chamfer",
            asked,
            model,
            solid,
            &edges,
            refused,
            largest,
            |m, picked, share| {
                chamfer_edges_with(m, solid, &picked_at(picked, share), tol()).map(|_| ())
            },
        )?),
    }
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

/// The largest first size `op`, a fillet or chamfer, builds at on
/// `solid`: a fillet's radius, a chamfer's distance (with two distances,
/// the first at their common scale). The kernel's reading, checked by one
/// build at it and found by halving below it where that build fails.
/// `None` where every size short of the solid's diagonal builds, where
/// none does, or for any other op.
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
        ..
    } = op
        && let Some(read) = chamfer_sides(
            &mut model.clone(),
            solid,
            spec,
            *flip,
            edges,
            *follow_tangent,
        )
    {
        return read;
    }
    (largest_size(model, solid, op), None)
}

/// [`size_limit`]'s first size for an op of one size.
fn largest_size(model: &Model, solid: &Shape, op: &kernel_api::SolidOp) -> Option<f64> {
    let span = crate::tess::robust_bounds(model, solid).map(|(lo, hi)| (hi - lo).magnitude())?;
    let mut model = model.clone();
    let (read, builds): (f64, Box<dyn Fn(f64) -> bool>) = match op {
        kernel_api::SolidOp::Fillet {
            edges,
            follow_tangent,
            corner,
            ..
        } => {
            let chain = edge_chain(&mut model, solid, edges, *follow_tangent, "fillet").ok()?;
            let read = largest_fillet_radius_all(&model, solid, &chain, span, tol()).ok()??;
            let model = model.clone();
            let corner = kernel_corner(*corner);
            (
                read,
                Box::new(move |r| {
                    fillet_edges_corner(&mut model.clone(), solid, &chain, r, corner, tol()).is_ok()
                }),
            )
        }
        kernel_api::SolidOp::Chamfer {
            spec,
            flip,
            edges,
            follow_tangent,
            ..
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
            let mut least = span;
            for (edge, spec) in &specs {
                // Two distances are read as their common scale.
                let per = match spec {
                    Chamfer::Distances { .. } => first,
                    _ => 1.0,
                };
                let measure =
                    largest_chamfer_distance(&model, solid, edge, spec, span / per, tol())
                        .ok()??;
                least = least.min(measure * per);
            }
            let model = model.clone();
            (
                least,
                Box::new(move |size| {
                    let scaled: Vec<(Shape, Chamfer)> = specs
                        .iter()
                        .map(|(edge, spec)| (edge.clone(), scaled(spec, size / first)))
                        .collect();
                    chamfer_edges_with(&mut model.clone(), solid, &scaled, tol()).is_ok()
                }),
            )
        }
        _ => return None,
    };
    if read >= span * (1.0 - 1e-9) {
        return None;
    }
    if builds(read) {
        return Some(read);
    }
    let share = largest_share(|share| Ok(builds(read * share))).ok()??;
    Some(read * share)
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
