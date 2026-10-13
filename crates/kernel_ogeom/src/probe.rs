//! Answers to [`ShapeProbe`]s: what a reference standing on a solid finds
//! on it, asked of the solid as it stands (a snapshot, or the running solid
//! part way through a chain).

use kernel_api::{FaceProbe, ProbeAnswer, ProbedCircle, ShapeProbe};
use ogeom::algo::{
    distance_between_shapes, project_on_curve, project_on_surface, volume_properties,
};
use ogeom::geom::{Curve, Curve3d as _, Surface as _, Transformable as _};
use ogeom::math::{Point, Vector};
use ogeom::mesh::Deflection;
use ogeom::topo::{EdgeRepr, Model, NodeData, Orientation, Shape, ShapeType, explore_unique};

use crate::tess;

/// Samples a projection brackets its foot with before refining it.
const PROJECTION_SAMPLES: usize = 24;

fn point3(p: [f64; 3]) -> Point {
    Point::new(p[0], p[1], p[2])
}

fn array(p: Point) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn unit(v: Vector) -> [f64; 3] {
    let length = v.magnitude().max(1e-300);
    [v.x / length, v.y / length, v.z / length]
}

/// What `root` answers to `probe`.
pub fn answer(model: &mut Model, root: &Shape, probe: &ShapeProbe) -> Result<ProbeAnswer, String> {
    match *probe {
        ShapeProbe::Face {
            point,
            normal,
            name,
        } => face(
            model,
            root,
            &FaceProbe {
                point,
                normal,
                name,
            },
        ),
        ShapeProbe::Edge {
            point,
            direction,
            faces,
            along,
        } => edge(model, root, point, direction, faces, along),
        ShapeProbe::Mass => mass(model, root),
    }
}

fn face(model: &mut Model, root: &Shape, probe: &FaceProbe) -> Result<ProbeAnswer, String> {
    let face = crate::queries::face_named(model, root, probe).map_err(|e| e.to_string())?;
    let (on_face, outward) = outward_at(model, &face, point3(probe.point))?;
    let outward = unit(outward);
    let facing = outward.map(|c| c as f32);
    Ok(ProbeAnswer::Face {
        point: array(on_face),
        normal: outward,
        surface: tess::face_surface(model, &face, facing),
    })
}

/// The point of `face` nearest `near`, and the face's outward normal there
/// (its surface's normal, turned where the face is reversed).
pub(crate) fn outward_at(
    model: &mut Model,
    face: &Shape,
    near: Point,
) -> Result<(Point, Vector), String> {
    let tol = tess::tolerances();
    let vertex = model.add_vertex(ogeom::topo::VertexData::new(near));
    let nearest = distance_between_shapes(
        model,
        &vertex,
        face,
        ogeom::intersect::ExtremaOptions::default(),
        tol,
    )
    .map_err(|e| format!("measuring to the face failed: {e}"))?;
    let on_face = nearest
        .pairs
        .first()
        .map(|pair| pair.point_b)
        .ok_or("the face has no nearest point")?;
    let Some(NodeData::Face(data)) = model.node(face).map(|n| n.data()) else {
        return Err("the face holds no face data".into());
    };
    let surface = model
        .geometry()
        .surface(data.surface)
        .ok_or("the face's surface is missing")?;
    let placement = face.transform(model.datums()).map_err(|e| e.to_string())?;
    let local = placement
        .inverse()
        .map_err(|e| e.to_string())?
        .apply(on_face);
    let foot = project_on_surface(surface, local, PROJECTION_SAMPLES, tol)
        .map_err(|e| format!("finding the point on the face failed: {e}"))?;
    let (u, v) = foot.parameters;
    let normal = surface
        .normal_at(u, v, tol)
        .map_err(|e| format!("the face has no normal there: {e}"))?;
    let placed = placement.apply_vector(normal.vector());
    let outward = if face.orientation() == Orientation::Reversed {
        -placed
    } else {
        placed
    };
    Ok((on_face, outward))
}

/// Least cosine between a probe's direction and an edge's tangent where the
/// edge passes the probe's point nearest, for the edge to run that way.
const RUNS_ALONG: f64 = 0.866;

fn edge(
    model: &mut Model,
    root: &Shape,
    point: [f64; 3],
    direction: [f64; 3],
    faces: [kernel_api::TopoName; 2],
    along: Option<f64>,
) -> Result<ProbeAnswer, String> {
    let found = find_edge(model, root, point, direction, faces)?;
    describe_edge(model, &found, point, along)
}

/// The edge of `root` between faces named `faces`, else the nearest to
/// `point` that runs along `direction` where it passes nearest (a zero
/// direction taking the nearest whichever way it runs).
pub(crate) fn find_edge(
    model: &mut Model,
    root: &Shape,
    point: [f64; 3],
    direction: [f64; 3],
    faces: [kernel_api::TopoName; 2],
) -> Result<Shape, String> {
    if let Some(named) = crate::naming::find_edge(model, root, faces, point3(point)) {
        return Ok(named);
    }
    let tol = tess::tolerances();
    let edges = explore_unique(model, root, ShapeType::Edge).map_err(|e| e.to_string())?;
    let mut near: Vec<(f64, Shape)> = Vec::with_capacity(edges.len());
    for edge in edges {
        let vertex = model.add_vertex(ogeom::topo::VertexData::new(point3(point)));
        if let Ok(d) = distance_between_shapes(
            model,
            &vertex,
            &edge,
            ogeom::intersect::ExtremaOptions::default(),
            tol,
        ) {
            near.push((d.distance, edge));
        }
    }
    if near.is_empty() {
        return Err("the solid has no edges".into());
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let length = direction.iter().map(|c| c * c).sum::<f64>().sqrt();
    // The nearest edge that runs the probe's way where it passes nearest,
    // so a rim found again after its circle grew or moved is still the
    // rim, not the straight edge beside it.
    for (_, edge) in near {
        let Ok(ProbeAnswer::Edge { direction: d, .. }) = describe_edge(model, &edge, point, None)
        else {
            continue;
        };
        let along = (0..3).map(|i| d[i] * direction[i]).sum::<f64>().abs();
        if length < 1e-9 || along >= RUNS_ALONG * length {
            return Ok(edge);
        }
    }
    let [x, y, z] = point;
    let [a, b, c] = direction;
    Err(format!(
        "no edge near the pick at ({x:.3}, {y:.3}, {z:.3}) runs along ({a:.3}, {b:.3}, {c:.3})"
    ))
}

/// What `edge` is where it passes `point` nearest, and, asked `along`, where
/// it is that share of its parameter from its start.
fn describe_edge(
    model: &Model,
    edge: &Shape,
    point: [f64; 3],
    along: Option<f64>,
) -> Result<ProbeAnswer, String> {
    let tol = tess::tolerances();
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return Err("the edge holds no edge data".into());
    };
    let Some(EdgeRepr::Curve3d { curve, range, .. }) = data.curve3d() else {
        return Err("the edge has no curve".into());
    };
    let range = *range;
    let placement = edge.transform(model.datums()).map_err(|e| e.to_string())?;
    let curve: Curve = model
        .geometry()
        .curve(*curve)
        .ok_or("the edge's curve is missing")?
        .transformed(&placement, tol)
        .map_err(|e| e.to_string())?;
    let trimmed = ogeom::geom::TrimmedCurve::new(curve.clone(), range.0, range.1, tol)
        .map_err(|e| format!("trimming the edge's curve failed: {e}"))?;
    let foot = project_on_curve(
        &Curve::Trimmed(Box::new(trimmed)),
        point3(point),
        PROJECTION_SAMPLES,
        tol,
    )
    .map_err(|e| format!("finding the point on the edge failed: {e}"))?;
    let at = |t: f64| curve.point_at(t, tol).map_err(|e| e.to_string());
    let reversed = edge.orientation() == Orientation::Reversed;
    let (mut start, mut end) = (at(range.0)?, at(range.1)?);
    let mut tangent = curve
        .d1_at(foot.parameter, tol)
        .map_err(|e| e.to_string())?;
    if reversed {
        std::mem::swap(&mut start, &mut end);
        tangent = -tangent;
    }
    let circle = match &curve {
        Curve::Circle(c) => {
            let circle = c.circle();
            Some(ProbedCircle {
                centre: array(circle.centre()),
                normal: unit(circle.frame().z().vector()),
                radius: circle.radius(),
            })
        }
        _ => None,
    };
    let along = match along {
        Some(share) => {
            let share = share.clamp(0.0, 1.0);
            let t = if reversed {
                range.1 - share * (range.1 - range.0)
            } else {
                range.0 + share * (range.1 - range.0)
            };
            let mut d = curve.d1_at(t, tol).map_err(|e| e.to_string())?;
            if reversed {
                d = -d;
            }
            Some((array(at(t)?), unit(d)))
        }
        None => None,
    };
    Ok(ProbeAnswer::Edge {
        point: array(foot.point),
        direction: unit(tangent),
        start: array(start),
        end: array(end),
        middle: array(at(f64::midpoint(range.0, range.1))?),
        circle,
        along,
    })
}

fn mass(model: &mut Model, root: &Shape) -> Result<ProbeAnswer, String> {
    let tol = tess::tolerances();
    let solids = explore_unique(model, root, ShapeType::Solid).map_err(|e| e.to_string())?;
    if solids.is_empty() {
        return Err("the shape holds no solid to weigh".into());
    }
    let props = volume_properties(model, root, Deflection::default(), tol)
        .map_err(|e| format!("measuring the solid failed: {e}"))?;
    let axes = props
        .principal_axes(tol)
        .map_err(|e| format!("the solid's axes of inertia failed: {e}"))?;
    let [a, b, _] = axes.map(|(_, axis)| unit(axis.vector()));
    // Right-handed: the third axis is the first two crossed.
    let c = unit(Vector::new(a[0], a[1], a[2]).cross(Vector::new(b[0], b[1], b[2])));
    Ok(ProbeAnswer::Mass {
        centre: array(props.centre),
        axes: [a, b, c],
    })
}
