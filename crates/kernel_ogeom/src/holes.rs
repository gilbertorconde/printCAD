//! Round holes found in a solid: the faces of a full bore, what closes each
//! of its ends, and the hole they make, for a feature to drill again.
//!
//! A bore is every concave cylindrical face on one axis at one radius,
//! together a whole turn. Each end is an opening where a flat face around
//! it faces out along the axis, a flat bottom where one faces back into
//! the bore, or a drill point where a cone on the axis closes it. A bore
//! with any other end (a counterbore's step, a countersink, a slot) is
//! counted and left alone.

use kernel_api::{FaceSurface, KernelError, KernelResult, RecognizedHole};
use ogeom::algo::face_normal;
use ogeom::geom::SurfaceGeometry;
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::topo::{Model, NodeData, SameKey, Shape, ShapeType, explore_unique};

use crate::tess;

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn scale(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn norm(a: V) -> f64 {
    dot(a, a).sqrt()
}
fn f64s(p: [f32; 3]) -> V {
    p.map(f64::from)
}

/// A face as the recognition reads it.
struct Face {
    shape: Shape,
    surface: FaceSurface,
    /// A point on it and its outward normal there.
    at: V,
    normal: V,
    /// Its triangulation's corners, and its area.
    points: Vec<V>,
    area: f64,
}

/// A concave cylinder face of a bore.
struct BoreFace {
    face: usize,
    origin: V,
    axis: V,
    radius: f64,
}

pub fn recognize_holes(brep: &[u8]) -> KernelResult<(Vec<RecognizedHole>, usize)> {
    let tol = tess::tolerances();
    let (model, root) = tess::read_blob(brep)?;
    let other = |e: ogeom::core::OgeomError| KernelError::Other(anyhow::anyhow!("{e}"));
    let shapes = explore_unique(&model, &root, ShapeType::Face).map_err(other)?;
    let mut faces = Vec::with_capacity(shapes.len());
    for shape in shapes {
        let Ok((at, normal)) = face_normal(&model, &shape, tol) else {
            continue;
        };
        let (at, normal) = ([at.x, at.y, at.z], [normal.x, normal.y, normal.z]);
        let length = norm(normal).max(1e-12);
        let normal = scale(normal, 1.0 / length);
        let mesh = triangulate_face(&model, &shape, Deflection::default(), tol).map_err(other)?;
        let points: Vec<V> = mesh.positions.iter().map(|p| [p.x, p.y, p.z]).collect();
        let area = mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| points[i as usize]);
                let (u, v) = (sub(b, a), sub(c, a));
                norm([
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ]) * 0.5
            })
            .sum();
        let facing = normal.map(|v| v as f32);
        let surface = tess::face_surface(&model, &shape, facing);
        faces.push(Face {
            shape,
            surface,
            at,
            normal,
            points,
            area,
        });
    }

    // Concave cylinders: the face's normal points at its axis.
    let bores: Vec<BoreFace> = faces
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let FaceSurface::Cylinder {
                origin,
                axis,
                radius,
            } = f.surface
            else {
                return None;
            };
            let (origin, axis) = (f64s(origin), f64s(axis));
            let axis = scale(axis, 1.0 / norm(axis).max(1e-12));
            let off = sub(f.at, origin);
            let radial = sub(off, scale(axis, dot(off, axis)));
            (dot(f.normal, radial) < 0.0).then_some(BoreFace {
                face: i,
                origin,
                axis,
                radius: f64::from(radius),
            })
        })
        .collect();

    // One bore: faces on one axis at one radius.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, b) in bores.iter().enumerate() {
        let same = |g: &Vec<usize>| {
            let o = &bores[g[0]];
            let off = sub(b.origin, o.origin);
            let across = sub(off, scale(o.axis, dot(off, o.axis)));
            (b.radius - o.radius).abs() < 1e-3
                && dot(b.axis, o.axis).abs() > 1.0 - 1e-6
                && norm(across) < 1e-3
        };
        match groups.iter_mut().find(|g| same(g)) {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }

    let edge_faces = edge_face_map(&model, &faces).map_err(other)?;

    let mut holes = Vec::new();
    let mut unknown = 0;
    for group in groups {
        match describe(&model, &faces, &bores, &group, &edge_faces) {
            Found::Hole(hole) => holes.push(*hole),
            Found::Bore => unknown += 1,
            // A round's or a slot end's arc is no bore.
            Found::Nothing => {}
        }
    }
    Ok((holes, unknown))
}

/// For each face, the faces it shares an edge with.
fn edge_face_map(model: &Model, faces: &[Face]) -> ogeom::core::OgeomResult<Vec<Vec<usize>>> {
    let mut by_edge: Vec<(SameKey, Vec<usize>)> = Vec::new();
    for (i, face) in faces.iter().enumerate() {
        for edge in explore_unique(model, &face.shape, ShapeType::Edge)? {
            let key = SameKey(edge);
            match by_edge.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(i),
                None => by_edge.push((key, vec![i])),
            }
        }
    }
    let mut out = vec![Vec::new(); faces.len()];
    for (_, list) in by_edge {
        for &a in &list {
            for &b in &list {
                if a != b && !out[a].contains(&b) {
                    out[a].push(b);
                }
            }
        }
    }
    Ok(out)
}

/// What a group of concave cylinder faces turned out to be.
enum Found {
    Hole(Box<RecognizedHole>),
    /// A whole bore, ended in a way no Hole feature makes.
    Bore,
    /// Less than a whole turn: a round, a slot's end.
    Nothing,
}

/// What closes one end of a bore.
enum End {
    Open,
    Flat(usize),
    Point(usize, f64),
}

fn describe(
    model: &Model,
    faces: &[Face],
    bores: &[BoreFace],
    group: &[usize],
    neighbours: &[Vec<usize>],
) -> Found {
    let first = &bores[group[0]];
    let (origin, axis, radius) = (first.origin, first.axis, first.radius);
    let along = |p: V| dot(sub(p, origin), axis);
    let members: Vec<usize> = group.iter().map(|&b| bores[b].face).collect();
    let (mut t0, mut t1) = (f64::MAX, f64::MIN);
    for &f in &members {
        for &p in &faces[f].points {
            let t = along(p);
            t0 = t0.min(t);
            t1 = t1.max(t);
        }
    }
    let length = t1 - t0;
    if length <= 1e-6 {
        return Found::Nothing;
    }
    // A whole turn: the faces' area is the full bore's.
    let area: f64 = members.iter().map(|&f| faces[f].area).sum();
    let turn = area / (radius * length);
    if (turn - std::f64::consts::TAU).abs() > 0.1 * std::f64::consts::TAU {
        return Found::Nothing;
    }
    match hole_of(
        model,
        faces,
        &members,
        origin,
        axis,
        radius,
        (t0, t1),
        neighbours,
    ) {
        Some(hole) => Found::Hole(Box::new(hole)),
        None => Found::Bore,
    }
}

/// The hole a whole bore makes, when both its ends are ones a Hole
/// feature makes.
#[allow(clippy::too_many_arguments)]
fn hole_of(
    model: &Model,
    faces: &[Face],
    members: &[usize],
    origin: V,
    axis: V,
    radius: f64,
    (t0, t1): (f64, f64),
    neighbours: &[Vec<usize>],
) -> Option<RecognizedHole> {
    let along = |p: V| dot(sub(p, origin), axis);
    let length = t1 - t0;
    let others: Vec<usize> = members
        .iter()
        .flat_map(|&f| neighbours[f].iter().copied())
        .filter(|n| !members.contains(n))
        .fold(Vec::new(), |mut acc, n| {
            if !acc.contains(&n) {
                acc.push(n);
            }
            acc
        });
    // Each neighbour goes with the end nearer its middle.
    let end_of = |n: usize| {
        let points = &faces[n].points;
        let mid = points.iter().map(|&p| along(p)).sum::<f64>() / points.len().max(1) as f64;
        (mid - t0).abs() > (mid - t1).abs()
    };
    let classify = |high: bool| -> Option<End> {
        let out = if high { axis } else { scale(axis, -1.0) };
        let at_end: Vec<usize> = others
            .iter()
            .copied()
            .filter(|&n| end_of(n) == high)
            .collect();
        if at_end.is_empty() {
            return None;
        }
        let mut open = false;
        let mut flat = None;
        let mut point = None;
        for n in at_end {
            match faces[n].surface {
                FaceSurface::Plane { .. } => {
                    let facing = dot(faces[n].normal, out);
                    if facing > 1.0 - 1e-6 {
                        open = true;
                    } else if facing < -1.0 + 1e-6 {
                        flat = Some(n);
                    } else {
                        return None;
                    }
                }
                FaceSurface::Cone {
                    apex,
                    axis: cone_axis,
                    ..
                } => {
                    let apex = f64s(apex);
                    let off = sub(apex, origin);
                    let across = sub(off, scale(axis, dot(off, axis)));
                    let beyond = if high {
                        along(apex) > t1
                    } else {
                        along(apex) < t0
                    };
                    if norm(across) > 1e-3
                        || dot(f64s(cone_axis), axis).abs() < 1.0 - 1e-6
                        || !beyond
                    {
                        return None;
                    }
                    point = Some((n, cone_half_angle(model, &faces[n].shape)? * 2.0));
                }
                _ => return None,
            }
        }
        match (open, flat, point) {
            (true, None, None) => Some(End::Open),
            (false, Some(n), None) => Some(End::Flat(n)),
            (false, None, Some((n, angle))) => Some(End::Point(n, angle)),
            _ => None,
        }
    };
    let (low, high) = (classify(false)?, classify(true)?);
    let (entry_t, inward, bottom) = match (low, high) {
        (End::Open, End::Open) => (t1, scale(axis, -1.0), None),
        (End::Open, bottom) => (t0, axis, Some(bottom)),
        (bottom, End::Open) => (t1, scale(axis, -1.0), Some(bottom)),
        _ => return None,
    };
    let mut hole_faces: Vec<(V, V)> = members
        .iter()
        .map(|&f| (faces[f].at, faces[f].normal))
        .collect();
    let drill_point_deg = match bottom {
        Some(End::Flat(n)) => {
            hole_faces.push((faces[n].at, faces[n].normal));
            None
        }
        Some(End::Point(n, angle)) => {
            hole_faces.push((faces[n].at, faces[n].normal));
            Some(angle.to_degrees())
        }
        _ => None,
    };
    Some(RecognizedHole {
        entry: add(origin, scale(axis, entry_t)),
        direction: inward,
        diameter: radius * 2.0,
        depth: length,
        through: bottom.is_none(),
        drill_point_deg,
        faces: hole_faces,
    })
}

/// The half angle of a conical face's surface, radians.
fn cone_half_angle(model: &Model, face: &Shape) -> Option<f64> {
    let NodeData::Face(data) = model.node(face)?.data() else {
        return None;
    };
    match model.geometry().surface(data.surface)? {
        SurfaceGeometry::Cone(cone) => Some(cone.cone().half_angle()),
        _ => None,
    }
}
