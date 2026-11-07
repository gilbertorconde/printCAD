//! Fillet, chamfer, draft, thickness and face edits (offset, move, remove)
//! on the running solid, with geometric (point-based) edge and face
//! selection.
//!
//! A fillet or a chamfer goes to the kernel as one chain, every selected
//! edge at once, resolved on the solid as it stands, so blends and bevels
//! that meet at a vertex close their corner between them.

use kernel_api::{ChamferSpec, EdgeSelection, ThicknessJoin};
use ogeom::algo::distance_between_shapes;
use ogeom::fillet::{Chamfer, chamfer_edges_with, fillet_edges};
use ogeom::geom::Curve3d as _;
use ogeom::math::{Direction, Plane, Point, Vector};
use ogeom::offset::{
    Join, apply_draft, make_thick_solid_with, move_faces, offset_faces, offset_shape,
};
use ogeom::topo::{EdgeRepr, Model, NodeData, Shape, ShapeType, ancestors_of, explore_unique};

use super::tol;

fn point3(p: [f64; 3]) -> Point {
    Point::new(p[0], p[1], p[2])
}

/// The sub-shape of `root` nearest to `probe`, of the wanted type.
pub fn nearest_of(
    model: &mut Model,
    root: &Shape,
    want: ShapeType,
    probe: Point,
) -> Result<Shape, String> {
    nearest_with_distance(model, root, want, probe).map(|(shape, _)| shape)
}

/// [`nearest_of`] and how far the probe is from it.
pub(crate) fn nearest_with_distance(
    model: &mut Model,
    root: &Shape,
    want: ShapeType,
    probe: Point,
) -> Result<(Shape, f64), String> {
    let vertex = model.add_vertex(ogeom::topo::VertexData::new(probe));
    let candidates = explore_unique(model, root, want)
        .map_err(|e| format!("exploring the solid failed: {e}"))?;
    let mut best: Option<(f64, Shape)> = None;
    for candidate in candidates {
        let Ok(d) = distance_between_shapes(
            model,
            &vertex,
            &candidate,
            ogeom::intersect::ExtremaOptions::default(),
            tol(),
        ) else {
            continue;
        };
        if best.as_ref().is_none_or(|(bd, _)| d.distance < *bd) {
            best = Some((d.distance, candidate));
        }
    }
    best.map(|(d, s)| (s, d))
        .ok_or_else(|| format!("no {want:?} found near the selection point"))
}

/// The face of `root` named `name`, else the one nearest `point`.
fn face_named_or_nearest(
    model: &mut Model,
    root: &Shape,
    name: Option<kernel_api::TopoName>,
    point: [f64; 3],
) -> Result<Shape, String> {
    match name.and_then(|n| crate::naming::find_face(model, root, n, point3(point))) {
        Some(face) => Ok(face),
        None => nearest_of(model, root, ShapeType::Face, point3(point)),
    }
}

/// How far from an edge a picked point may lie and still name it: a tenth
/// of the solid's diagonal, room for the edge to move with an upstream
/// edit while a point in the middle of a face names nothing.
pub(crate) fn pick_reach(model: &Model, solid: &Shape) -> f64 {
    crate::tess::robust_bounds(model, solid)
        .map(|(lo, hi)| (hi - lo).magnitude() * 0.1)
        .unwrap_or(f64::INFINITY)
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
            return Ok((points.iter().map(|p| Probe::at(point3(*p))).collect(), true));
        }
        EdgeSelection::Picked(picks) => {
            let probes = picks
                .iter()
                .map(|pick| {
                    let [x, y, z] = pick.direction;
                    let length = (x * x + y * y + z * z).sqrt();
                    Probe {
                        point: point3(pick.point),
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

pub fn fillet(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    edges: &EdgeSelection,
    follow_tangent: bool,
) -> Result<Shape, String> {
    let mut chain = selected_edges(model, solid, edges)?;
    if chain.is_empty() {
        return Err("fillet selection matches no edges".into());
    }
    if follow_tangent {
        chain = tangent_chain(model, solid, chain)?;
    }
    fillet_edges(model, solid, &chain, radius, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("fillet failed: {e}"))
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
        pick_reach(model, solid)
    } else {
        f64::INFINITY
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
        let Some((nearest, _)) = near.first() else {
            return Err("the solid has no edges".into());
        };
        if *nearest > reach {
            return Err(format!(
                "no edge near the pick at ({:.3}, {:.3}, {:.3}): the nearest is {nearest:.3} mm away",
                p.x, p.y, p.z
            ));
        }
        let found = near
            .into_iter()
            .take_while(|(d, _)| *d <= reach)
            .find(|(d, edge)| match probe.along {
                Some(along) => runs_along(model, edge, p, along, *d),
                None => true,
            });
        let Some((_, edge)) = found else {
            let a = probe.along.unwrap_or(Vector::new(0.0, 0.0, 0.0));
            return Err(format!(
                "no edge near the pick at ({:.3}, {:.3}, {:.3}) runs along ({:.3}, {:.3}, {:.3})",
                p.x, p.y, p.z, a.x, a.y, a.z
            ));
        };
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
/// corner.
fn tangent_chain(model: &Model, solid: &Shape, chain: Vec<Shape>) -> Result<Vec<Shape>, String> {
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
    let mut result = chain;
    while let Some(i) = queue.pop() {
        let Some(mine) = ends[i] else { continue };
        for (j, theirs) in ends.iter().enumerate() {
            let Some(theirs) = theirs else { continue };
            if taken[j] {
                continue;
            }
            let meets = mine.iter().any(|(p, t)| {
                theirs
                    .iter()
                    .any(|(q, s)| p.distance(*q) <= END_REACH && parallel(*t, *s))
            });
            if meets {
                taken[j] = true;
                queue.push(j);
                result.push(edges[j].clone());
            }
        }
    }
    Ok(result)
}

pub fn chamfer(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
    follow_tangent: bool,
) -> Result<Shape, String> {
    let mut chain = selected_edges(model, solid, edges)?;
    if chain.is_empty() {
        return Err("chamfer selection matches no edges".into());
    }
    if follow_tangent {
        chain = tangent_chain(model, solid, chain)?;
    }
    let faces = match spec {
        ChamferSpec::EqualDistance { .. } => Vec::new(),
        _ => reference_faces(model, solid, &chain, flip)?,
    };
    let mut specs = Vec::with_capacity(chain.len());
    for (i, edge) in chain.into_iter().enumerate() {
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
    chamfer_edges_with(model, solid, &specs, tol())
        .map(|b| {
            crate::naming::record(&b.history);
            b.shape
        })
        .map_err(|e| format!("chamfer failed: {e}"))
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
    let normal = Direction::new(
        Vector::new(neutral_normal[0], neutral_normal[1], neutral_normal[2]),
        tol(),
    )
    .map_err(|_| "draft neutral normal is (near) zero".to_string())?;
    let neutral = Plane::through(point3(neutral_point), normal);
    let pull = match pull_dir {
        Some(d) => Direction::new(Vector::new(d[0], d[1], d[2]), tol())
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
