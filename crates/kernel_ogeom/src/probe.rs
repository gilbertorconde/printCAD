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

use crate::convert::{array, point, vector};
use crate::tess;

/// Samples a projection brackets its foot with before refining it.
const PROJECTION_SAMPLES: usize = 24;

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
    let face = face_for(model, root, probe, Reach::Any)?;
    let (on_face, outward) = outward_at(model, &face, point(probe.point))?;
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
    let found = edge_near(
        model,
        root,
        crate::convert::point(point),
        direction,
        faces,
        Reach::Any,
    )?;
    describe_edge(model, &found, point, along)
}

/// How far from a probe's point the face or edge found for it may lie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Reach {
    /// A tenth of the solid's diagonal: room for a face or edge to move
    /// with an upstream edit, while a point lost in the middle of another
    /// face names nothing.
    Pick,
    /// Within this many millimetres.
    Within(f64),
    /// However far.
    Any,
}

impl Reach {
    /// The distance in millimetres this reach allows on `root`.
    pub(crate) fn limit(self, model: &Model, root: &Shape) -> f64 {
        match self {
            Reach::Pick => tess::robust_bounds(model, root)
                .map(|(lo, hi)| (hi - lo).magnitude() * 0.1)
                .unwrap_or(f64::INFINITY),
            Reach::Within(mm) => mm,
            Reach::Any => f64::INFINITY,
        }
    }
}

/// A probe of the face at `point`, named `name` (0 for none), preferring no
/// facing.
pub(crate) fn at_point(point: Point, name: kernel_api::TopoName) -> FaceProbe {
    FaceProbe {
        point: array(point),
        normal: [0.0; 3],
        name,
    }
}

/// The sub-shapes of `root` of type `want`, nearest `point` first, each with
/// its distance.
pub(crate) fn by_distance(
    model: &mut Model,
    root: &Shape,
    want: ShapeType,
    point: Point,
) -> Result<Vec<(f64, Shape)>, String> {
    let tol = tess::tolerances();
    let vertex = model.add_vertex(ogeom::topo::VertexData::new(point));
    let shapes = explore_unique(model, root, want)
        .map_err(|e| format!("exploring the solid failed: {e}"))?;
    let mut near: Vec<(f64, Shape)> = shapes
        .into_iter()
        .filter_map(|shape| {
            distance_between_shapes(
                model,
                &vertex,
                &shape,
                ogeom::intersect::ExtremaOptions::default(),
                tol,
            )
            .ok()
            .map(|d| (d.distance, shape))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(near)
}

fn too_far(what: &str, point: Point, distance: f64) -> String {
    format!(
        "no {what} near ({:.3}, {:.3}, {:.3}): the nearest is {distance:.3} mm away",
        point.x, point.y, point.z
    )
}

/// The face of `root` a probe names and how far its point is from it: the
/// face its name finds (at no distance), else the nearest within `reach`,
/// and among faces as near (a point on the edge between them) the one
/// whose outward normal agrees best with the probe's. None when no face
/// lies within reach.
pub(crate) fn face_within(
    model: &mut Model,
    root: &Shape,
    probe: &FaceProbe,
    reach: Reach,
) -> Result<Option<(Shape, f64)>, String> {
    let point = point(probe.point);
    if let Some(face) = crate::naming::find_face(model, root, probe.name, point) {
        return Ok(Some((face, 0.0)));
    }
    let near = by_distance(model, root, ShapeType::Face, point)?;
    let Some(&(best, _)) = near.first() else {
        return Ok(None);
    };
    if best > reach.limit(model, root) {
        return Ok(None);
    }
    let normal = vector(probe.normal);
    let agreement = |model: &Model, face: &Shape| {
        crate::ops::sweep::face_plane(model, face).map_or(0.0, |(_, n)| {
            let outward = if face.orientation() == Orientation::Reversed {
                -n.vector()
            } else {
                n.vector()
            };
            outward.dot(normal)
        })
    };
    // Among equals the first stays, so a probe with no normal takes the
    // nearest.
    let mut chosen: Option<(f64, f64, Shape)> = None;
    for (d, face) in near.into_iter().take_while(|(d, _)| *d <= best + 1e-6) {
        let agrees = agreement(model, &face);
        if chosen.as_ref().is_none_or(|(a, _, _)| agrees > *a) {
            chosen = Some((agrees, d, face));
        }
    }
    Ok(chosen.map(|(_, d, face)| (face, d)))
}

/// [`face_within`], with no face in reach an error.
pub(crate) fn face_for(
    model: &mut Model,
    root: &Shape,
    probe: &FaceProbe,
    reach: Reach,
) -> Result<Shape, String> {
    if let Some((face, _)) = face_within(model, root, probe, reach)? {
        return Ok(face);
    }
    let point = point(probe.point);
    match by_distance(model, root, ShapeType::Face, point)?.first() {
        Some((d, _)) => Err(too_far("face", point, *d)),
        None => Err("the solid has no faces".into()),
    }
}

/// The edge of `root` between faces named `faces`, else the nearest to
/// `point` within `reach` that runs along `direction` where it passes
/// nearest (a zero direction taking the nearest whichever way it runs).
pub(crate) fn edge_near(
    model: &mut Model,
    root: &Shape,
    point: Point,
    direction: [f64; 3],
    faces: [kernel_api::TopoName; 2],
    reach: Reach,
) -> Result<Shape, String> {
    if let Some(named) = crate::naming::find_edge(model, root, faces, point) {
        return Ok(named);
    }
    let near = by_distance(model, root, ShapeType::Edge, point)?;
    let Some(&(nearest, _)) = near.first() else {
        return Err("the solid has no edges".into());
    };
    let limit = reach.limit(model, root);
    if nearest > limit {
        return Err(too_far("edge", point, nearest));
    }
    let length = direction.iter().map(|c| c * c).sum::<f64>().sqrt();
    // The nearest edge that runs the probe's way where it passes nearest,
    // so a rim found again after its circle grew or moved is still the
    // rim, not the straight edge beside it.
    for (_, edge) in near.into_iter().take_while(|(d, _)| *d <= limit) {
        let Ok(ProbeAnswer::Edge { direction: d, .. }) =
            describe_edge(model, &edge, array(point), None)
        else {
            continue;
        };
        let along = (0..3).map(|i| d[i] * direction[i]).sum::<f64>().abs();
        if length < 1e-9 || along >= RUNS_ALONG * length {
            return Ok(edge);
        }
    }
    let [a, b, c] = direction;
    Err(format!(
        "no edge near the pick at ({:.3}, {:.3}, {:.3}) runs along ({a:.3}, {b:.3}, {c:.3})",
        point.x, point.y, point.z
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
        crate::convert::point(point),
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
    let c = unit(vector(a).cross(vector(b)));
    Ok(ProbeAnswer::Mass {
        centre: array(props.centre),
        axes: [a, b, c],
    })
}
