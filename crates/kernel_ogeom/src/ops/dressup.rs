//! Fillet / chamfer / draft / thickness on the running solid, with geometric
//! (point-based) edge and face selection.
//!
//! A fillet or a chamfer goes to the kernel as one chain, every selected
//! edge at once, resolved on the solid as it stands, so blends and bevels
//! that meet at a vertex close their corner between them.

use kernel_api::{ChamferSpec, EdgeSelection};
use ogeom::algo::distance_between_shapes;
use ogeom::fillet::{Chamfer, chamfer_edges_with, fillet_edges};
use ogeom::math::{Direction, Plane, Point, Vector};
use ogeom::offset::{apply_draft, make_thick_solid};
use ogeom::topo::{Model, NodeData, Shape, ShapeType, ancestors_of, explore_unique};

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
fn nearest_with_distance(
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

/// How far from an edge a picked point may lie and still name it: a tenth
/// of the solid's diagonal, room for the edge to move with an upstream
/// edit while a point in the middle of a face names nothing.
fn pick_reach(model: &Model, solid: &Shape) -> f64 {
    crate::tess::robust_bounds(model, solid)
        .map(|(lo, hi)| (hi - lo).magnitude() * 0.1)
        .unwrap_or(f64::INFINITY)
}

/// A point on (or representative of) an edge, for later re-resolution.
fn edge_probe(model: &Model, edge: &Shape) -> Option<Point> {
    let vertices = explore_unique(model, edge, ShapeType::Vertex).ok()?;
    let mut acc = Vector::new(0.0, 0.0, 0.0);
    let mut n = 0.0;
    for v in &vertices {
        let node = model.node(v)?;
        if let NodeData::Vertex(data) = node.data() {
            let placed = v.transform(model.datums()).ok()?.apply(data.point);
            acc += placed - Point::new(0.0, 0.0, 0.0);
            n += 1.0;
        }
    }
    if n == 0.0 {
        return None;
    }
    Some(Point::new(acc.x / n, acc.y / n, acc.z / n))
}

/// A point that names an edge: the edge nearest it, and, when `along` is
/// set, running that way there.
struct Probe {
    point: Point,
    along: Option<Vector>,
}

impl Probe {
    fn at(point: Point) -> Self {
        Self { point, along: None }
    }
}

/// Probes for every edge a selection names, resolved on `solid`, and
/// whether they are picks, which must lie within reach of their edge (the
/// others are read off the solid's own edges).
fn selection_probes(
    model: &mut Model,
    solid: &Shape,
    edges: &EdgeSelection,
) -> Result<(Vec<Probe>, bool), String> {
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
                    }
                })
                .collect();
            return Ok((probes, true));
        }
        EdgeSelection::All => {
            let all = explore_unique(model, solid, ShapeType::Edge)
                .map_err(|e| format!("exploring edges failed: {e}"))?;
            all.iter()
                .filter_map(|e| edge_probe(model, e))
                .map(Probe::at)
                .collect()
        }
        EdgeSelection::OfFaces(points) => {
            let mut probes = Vec::new();
            let mut seen: Vec<Shape> = Vec::new();
            for p in points {
                let face = nearest_of(model, solid, ShapeType::Face, point3(*p))?;
                let face_edges = explore_unique(model, &face, ShapeType::Edge)
                    .map_err(|e| format!("exploring face edges failed: {e}"))?;
                for edge in face_edges {
                    if seen.iter().any(|s| s.is_same(&edge)) {
                        continue;
                    }
                    if let Some(probe) = edge_probe(model, &edge) {
                        probes.push(Probe::at(probe));
                    }
                    seen.push(edge);
                }
            }
            probes
        }
    };
    Ok((probes, false))
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

pub fn fillet(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    edges: &EdgeSelection,
) -> Result<Shape, String> {
    let probes = selection_probes(model, solid, edges)?;
    if probes.0.is_empty() {
        return Err("fillet selection matches no edges".into());
    }
    let chain = chain_of(model, solid, probes)?;
    fillet_edges(model, solid, &chain, radius, tol())
        .map(|b| b.shape)
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

pub fn chamfer(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
) -> Result<Shape, String> {
    let probes = selection_probes(model, solid, edges)?;
    if probes.0.is_empty() {
        return Err("chamfer selection matches no edges".into());
    }
    let chain = chain_of(model, solid, probes)?;
    let mut specs = Vec::with_capacity(chain.len());
    for edge in chain {
        let spec = match spec {
            ChamferSpec::EqualDistance { distance } => Chamfer::Symmetric(*distance),
            ChamferSpec::TwoDistances {
                distance1,
                distance2,
            } => Chamfer::Distances {
                face: adjacent_face(model, solid, &edge, flip)?,
                on_face: *distance1,
                on_other: *distance2,
            },
            ChamferSpec::DistanceAngle {
                distance,
                angle_deg,
            } => Chamfer::Angle {
                face: adjacent_face(model, solid, &edge, flip)?,
                distance: *distance,
                angle: angle_deg.to_radians(),
            },
        };
        specs.push((edge, spec));
    }
    chamfer_edges_with(model, solid, &specs, tol())
        .map(|b| b.shape)
        .map_err(|e| format!("chamfer failed: {e}"))
}

/// One of the two faces sharing the edge; `flip` selects the other.
fn adjacent_face(model: &Model, solid: &Shape, edge: &Shape, flip: bool) -> Result<Shape, String> {
    let mut faces = ancestors_of(model, solid, edge, ShapeType::Face)
        .map_err(|e| format!("finding the edge's faces failed: {e}"))?;
    // ancestors_of yields per route; dedupe.
    let mut unique: Vec<Shape> = Vec::new();
    for f in faces.drain(..) {
        if !unique.iter().any(|u| u.is_same(&f)) {
            unique.push(f);
        }
    }
    let idx = usize::from(flip && unique.len() > 1);
    unique
        .into_iter()
        .nth(idx)
        .ok_or_else(|| "the edge borders no face of the solid".to_string())
}

pub fn draft(
    model: &mut Model,
    solid: &Shape,
    angle_deg: f64,
    neutral_point: [f64; 3],
    neutral_normal: [f64; 3],
    pull_dir: Option<[f64; 3]>,
    face_points: &[[f64; 3]],
) -> Result<Shape, String> {
    if face_points.is_empty() {
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
    let mut faces = Vec::with_capacity(face_points.len());
    for p in face_points {
        faces.push(nearest_of(model, solid, ShapeType::Face, point3(*p))?);
    }
    apply_draft(
        model,
        solid,
        &faces,
        neutral,
        pull,
        angle_deg.to_radians(),
        tol(),
    )
    .map(|b| b.shape)
    .map_err(|e| format!("draft failed: {e}"))
}

pub fn thickness(
    model: &mut Model,
    solid: &Shape,
    value: f64,
    open_face_points: &[[f64; 3]],
    inward: bool,
) -> Result<Shape, String> {
    let mut removed = Vec::with_capacity(open_face_points.len());
    for p in open_face_points {
        removed.push(nearest_of(model, solid, ShapeType::Face, point3(*p))?);
    }
    // Positive thickness hollows inward; negative builds the walls outward
    // around the solid.
    let signed = if inward { value } else { -value };
    make_thick_solid(model, solid, &removed, signed, tol())
        .map(|b| b.shape)
        .map_err(|e| format!("thickness failed: {e}"))
}
