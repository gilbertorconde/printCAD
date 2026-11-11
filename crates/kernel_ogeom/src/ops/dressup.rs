//! Fillet, chamfer, draft, thickness and face edits (offset, move, remove)
//! on the running solid, with geometric (point-based) edge and face
//! selection.
//!
//! A fillet or a chamfer goes to the kernel as one chain, every selected
//! edge at once, resolved on the solid as it stands, so blends and bevels
//! that meet at a vertex close their corner between them.

use kernel_api::{ChamferSpec, EdgeSelection, ThicknessJoin};
use ogeom::algo::distance_between_shapes;
use ogeom::fillet::{
    BlendRefusal, Chamfer, chamfer_edges_checked, chamfer_edges_with, fillet_edges,
    fillet_edges_checked, largest_chamfer_distance, largest_fillet_radius_all,
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

pub fn fillet(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    edges: &EdgeSelection,
    follow_tangent: bool,
) -> Result<Shape, Refusal> {
    let mut chain = selected_edges(model, solid, edges)?;
    if chain.is_empty() {
        return Err("fillet selection matches no edges".to_string().into());
    }
    if follow_tangent {
        chain = tangent_chain(model, solid, chain)?;
    }
    let pick =
        |picked: &[usize]| -> Vec<Shape> { picked.iter().map(|&i| chain[i].clone()).collect() };
    match fillet_edges_checked(model, solid, &chain, radius, tol()) {
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
                fillet_edges(m, solid, &pick(picked), radius * share, tol()).map(|_| ())
            },
        )?),
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
) -> Result<Shape, Refusal> {
    let asked = match spec {
        ChamferSpec::EqualDistance { distance } | ChamferSpec::DistanceAngle { distance, .. } => {
            *distance
        }
        ChamferSpec::TwoDistances { distance1, .. } => *distance1,
    };
    let mut chain = selected_edges(model, solid, edges)?;
    if chain.is_empty() {
        return Err("chamfer selection matches no edges".to_string().into());
    }
    if follow_tangent {
        chain = tangent_chain(model, solid, chain)?;
    }
    let faces = match spec {
        ChamferSpec::EqualDistance { .. } => Vec::new(),
        _ => reference_faces(model, solid, &chain, flip)?,
    };
    let mut specs = Vec::with_capacity(chain.len());
    for (i, edge) in chain.iter().cloned().enumerate() {
        let spec = match spec {
            ChamferSpec::EqualDistance { distance } => Chamfer::Symmetric(*distance),
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
    match chamfer_edges_checked(model, solid, &specs, tol()) {
        Ok(built) => {
            crate::naming::record(&built.history);
            Ok(built.shape)
        }
        Err(refused) => {
            let edges: Vec<Shape> = specs.iter().map(|(edge, _)| edge.clone()).collect();
            Err(refusal(
                "chamfer",
                asked,
                model,
                solid,
                &edges,
                refused,
                |m, picked| largest_chamfer_share(m, solid, &specs, picked, asked),
                |m, picked, share| {
                    let picked: Vec<(Shape, Chamfer)> = picked
                        .iter()
                        .map(|&i| (specs[i].0.clone(), scaled(&specs[i].1, share)))
                        .collect();
                    chamfer_edges_with(m, solid, &picked, tol()).map(|_| ())
                },
            )?)
        }
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
