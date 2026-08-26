//! Fillet / chamfer / draft / thickness on the running solid, with geometric
//! (point-based) edge and face selection.
//!
//! ogeom blends one edge per call and each call rebuilds the solid, so
//! multi-edge selections loop: every step re-resolves its probe point against
//! the current solid, the same way picks re-resolve across parametric
//! rebuilds.

use kernel_api::{ChamferSpec, EdgeSelection};
use ogeom::algo::distance_between_shapes;
use ogeom::fillet::{chamfer_edge, chamfer_edge_angle, chamfer_edge_distances, fillet_edge};
use ogeom::math::{Direction, Plane, Point, Vector};
use ogeom::offset::{apply_draft, make_thick_solid};
use ogeom::topo::{ancestors_of, explore_unique, Model, NodeData, Shape, ShapeType};

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
    best.map(|(_, s)| s)
        .ok_or_else(|| format!("no {want:?} found near the selection point"))
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

/// Probe points for every edge a selection names, resolved on `solid`.
fn selection_probes(
    model: &mut Model,
    solid: &Shape,
    edges: &EdgeSelection,
) -> Result<Vec<Point>, String> {
    match edges {
        EdgeSelection::Near(points) => Ok(points.iter().map(|p| point3(*p)).collect()),
        EdgeSelection::All => {
            let all = explore_unique(model, solid, ShapeType::Edge)
                .map_err(|e| format!("exploring edges failed: {e}"))?;
            Ok(all.iter().filter_map(|e| edge_probe(model, e)).collect())
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
                        probes.push(probe);
                    }
                    seen.push(edge);
                }
            }
            Ok(probes)
        }
    }
}

pub fn fillet(
    model: &mut Model,
    solid: &Shape,
    radius: f64,
    edges: &EdgeSelection,
) -> Result<Shape, String> {
    let probes = selection_probes(model, solid, edges)?;
    if probes.is_empty() {
        return Err("fillet selection matches no edges".into());
    }
    let mut current = solid.clone();
    for probe in probes {
        let edge = nearest_of(model, &current, ShapeType::Edge, probe)?;
        current = fillet_edge(model, &current, &edge, radius, tol())
            .map_err(|e| format!("fillet failed: {e}"))?
            .shape;
    }
    Ok(current)
}

pub fn chamfer(
    model: &mut Model,
    solid: &Shape,
    spec: &ChamferSpec,
    flip: bool,
    edges: &EdgeSelection,
) -> Result<Shape, String> {
    let probes = selection_probes(model, solid, edges)?;
    if probes.is_empty() {
        return Err("chamfer selection matches no edges".into());
    }
    let mut current = solid.clone();
    for probe in probes {
        let edge = nearest_of(model, &current, ShapeType::Edge, probe)?;
        current = match spec {
            ChamferSpec::EqualDistance { distance } => {
                chamfer_edge(model, &current, &edge, *distance, tol())
                    .map_err(|e| format!("chamfer failed: {e}"))?
                    .shape
            }
            ChamferSpec::TwoDistances {
                distance1,
                distance2,
            } => {
                let face = adjacent_face(model, &current, &edge, flip)?;
                chamfer_edge_distances(model, &current, &edge, &face, *distance1, *distance2, tol())
                    .map_err(|e| format!("chamfer failed: {e}"))?
                    .shape
            }
            ChamferSpec::DistanceAngle {
                distance,
                angle_deg,
            } => {
                let face = adjacent_face(model, &current, &edge, flip)?;
                chamfer_edge_angle(
                    model,
                    &current,
                    &edge,
                    &face,
                    *distance,
                    angle_deg.to_radians(),
                    tol(),
                )
                .map_err(|e| format!("chamfer failed: {e}"))?
                .shape
            }
        };
    }
    Ok(current)
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
