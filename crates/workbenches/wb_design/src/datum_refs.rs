//! A datum's references to what it stands on: made from picks or from a
//! command's arguments, filled in from the body's solid where the kernel is
//! at hand, and written back out as the arguments that make them again.

use core_document::{
    Args, BasePlane, BodyId, CommandError, DatumAttachment, DatumFeature, DatumShape, Document,
    EdgeAnchor, EdgeSpot, FaceAnchor, FeatureId, LineAnchor, PlaneAnchor, PointAnchor,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Map, Value, json};

use crate::commands::vector3;

pub use core_document::attach::{edge_anchor, face_anchor, settle};

/// The modes a datum attaches by, as `design.datum` names them, with what
/// each is.
pub const MODES: &[(&str, &str)] = &[
    ("base_plane", "on a base plane (plane)"),
    (
        "face",
        "on a face, tangent to it where curved (face_point, face_normal)",
    ),
    ("three_points", "through three points (points)"),
    (
        "normal_to_edge",
        "square to an edge (edge_point, edge_direction, spot)",
    ),
    (
        "along_edge",
        "along an edge, or a circle's axis (edge_point, edge_direction)",
    ),
    ("two_points", "through two points (points)"),
    ("plane_intersection", "where two planes meet (planes)"),
    (
        "curve_centre",
        "at a circular edge's centre (edge_point, edge_direction)",
    ),
    (
        "inertia",
        "at the body's centre of mass, on its axes of inertia",
    ),
    (
        "on_datum",
        "on a datum plane, or a coordinate system's plane (datum, plane)",
    ),
    (
        "other_body",
        "on one of another body's origin planes (of_body, plane)",
    ),
    ("face_normal", "square to a face (face_point, face_normal)"),
    (
        "tangent_to_edge",
        "tangent to an edge (edge_point, edge_direction, spot)",
    ),
    ("line_and_point", "through a line and a point (line, point)"),
    (
        "line_meets_plane",
        "where a line meets a plane (line, plane)",
    ),
    ("two_lines", "where two lines cross (lines)"),
    (
        "midplane",
        "midway between two planes, or bisecting them (planes)",
    ),
];

/// Where a sketch's point or line ends stand, in its body's frame.
pub fn sketch_points(
    document: &Document,
    sketch: FeatureId,
    element: uuid::Uuid,
) -> Option<Vec<[f32; 3]>> {
    let feature = wb_sketch::SketchFeature::from_json(document.feature_values(sketch)?).ok()?;
    sketch_points_of(&feature, element)
}

/// [`sketch_points`] of a sketch feature at hand.
pub fn sketch_points_of(
    feature: &wb_sketch::SketchFeature,
    element: uuid::Uuid,
) -> Option<Vec<[f32; 3]>> {
    use wb_sketch::sketch::GeometryElement;
    let plane = feature.plane;
    let world = |id: uuid::Uuid| {
        let p = feature.sketch.point_position(id)?;
        Some(std::array::from_fn(|i| {
            (plane.origin[i] + plane.x_axis[i] * p.x + plane.y_axis[i] * p.y) as f32
        }))
    };
    match feature.sketch.get_geometry(element)? {
        GeometryElement::Point(_) => Some(vec![world(element)?]),
        GeometryElement::Line(l) => Some(vec![world(l.start)?, world(l.end)?]),
        _ => None,
    }
}

fn id_of(value: Option<&Value>, name: &str) -> Result<uuid::Uuid, CommandError> {
    value
        .and_then(Value::as_str)
        .and_then(|t| uuid::Uuid::parse_str(t).ok())
        .ok_or_else(|| CommandError::bad(name, "must be an id"))
}

/// A datum of `body` by its id in `value`, with its frame.
fn datum_of(
    value: Option<&Value>,
    name: &str,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<(FeatureId, DatumFeature), CommandError> {
    let datum = FeatureId(id_of(value, name)?);
    let node = ctx
        .document
        .get_feature_meta(datum)
        .filter(|n| n.workbench_id.as_str() == core_document::DATUM_KIND)
        .ok_or_else(|| CommandError::bad(name, "is not a datum of this document"))?;
    if node.body != Some(body) {
        return Err(CommandError::bad(name, "is a datum of another body"));
    }
    let made = ctx
        .document
        .feature_values(datum)
        .and_then(|v| DatumFeature::from_json(v).ok())
        .ok_or_else(|| CommandError::bad(name, "is a datum that does not read"))?;
    Ok((datum, made))
}

fn line_from(
    value: &Value,
    name: &str,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<LineAnchor, CommandError> {
    if value.is_string() {
        let (datum, made) = datum_of(Some(value), name, ctx, body)?;
        if !matches!(made.shape, DatumShape::Line { .. }) {
            return Err(CommandError::bad(name, "is a datum that is no line"));
        }
        let frame = made.frame();
        return Ok(LineAnchor::Datum {
            datum,
            origin: frame.origin,
            direction: frame.x_axis,
        });
    }
    if value.get("sketch").is_some() {
        let sketch = FeatureId(id_of(value.get("sketch"), &format!("{name}.sketch"))?);
        let element = id_of(value.get("element"), &format!("{name}.element"))?;
        let Some([start, end]) = sketch_points(ctx.document, sketch, element)
            .and_then(|p| <[[f32; 3]; 2]>::try_from(p).ok())
        else {
            return Err(CommandError::bad(name, "names no line of that sketch"));
        };
        return Ok(LineAnchor::Sketch {
            sketch,
            element,
            start,
            end,
        });
    }
    Ok(LineAnchor::Edge {
        edge: edge_from(value, name)?,
    })
}

fn face_from(value: &Value, name: &str) -> Result<FaceAnchor, CommandError> {
    Ok(FaceAnchor {
        name: 0,
        point: vector3(value.get("face_point"), &format!("{name}.face_point"))?,
        normal: vector3(value.get("face_normal"), &format!("{name}.face_normal"))?,
        surface: None,
        follows: true,
    })
}

fn edge_from(value: &Value, name: &str) -> Result<EdgeAnchor, CommandError> {
    Ok(EdgeAnchor {
        along: None,
        faces: [0, 0],
        point: vector3(value.get("edge_point"), &format!("{name}.edge_point"))?,
        direction: match value.get("edge_direction") {
            Some(v) if !v.is_null() => vector3(Some(v), &format!("{name}.edge_direction"))?,
            _ => [0.0; 3],
        },
        ends: None,
        middle: None,
        circle: None,
        follows: true,
    })
}

fn spot_from(value: Option<&Value>, name: &str) -> Result<EdgeSpot, CommandError> {
    match value {
        None | Some(Value::Null) => Ok(EdgeSpot::Picked),
        Some(v) => v
            .as_str()
            .and_then(EdgeSpot::from_key)
            .ok_or_else(|| CommandError::bad(name, "must be picked, start, end, middle or centre")),
    }
}

fn base_plane(name: &str) -> Option<BasePlane> {
    BasePlane::ALL
        .into_iter()
        .find(|p| base_plane_key(*p).eq_ignore_ascii_case(name))
}

fn base_plane_key(plane: BasePlane) -> &'static str {
    match plane {
        BasePlane::XY => "XY",
        BasePlane::XZ => "XZ",
        BasePlane::YZ => "YZ",
    }
}

fn point_from(
    value: &Value,
    name: &str,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<PointAnchor, CommandError> {
    if value.get("datum").is_some() {
        let (datum, made) = datum_of(value.get("datum"), &format!("{name}.datum"), ctx, body)?;
        return Ok(PointAnchor::Datum {
            datum,
            point: made.frame().origin,
        });
    }
    if value.get("sketch").is_some() {
        let sketch = FeatureId(id_of(value.get("sketch"), &format!("{name}.sketch"))?);
        let element = id_of(value.get("element"), &format!("{name}.element"))?;
        let Some([point]) = sketch_points(ctx.document, sketch, element)
            .and_then(|p| <[[f32; 3]; 1]>::try_from(p).ok())
        else {
            return Err(CommandError::bad(name, "names no point of that sketch"));
        };
        return Ok(PointAnchor::Sketch {
            sketch,
            element,
            point,
        });
    }
    if value.get("face_point").is_some() {
        Ok(PointAnchor::Face {
            face: face_from(value, name)?,
        })
    } else if value.get("edge_point").is_some() {
        Ok(PointAnchor::Edge {
            edge: edge_from(value, name)?,
            spot: spot_from(value.get("spot"), &format!("{name}.spot"))?,
        })
    } else {
        Ok(PointAnchor::At {
            point: vector3(Some(value), name)?,
        })
    }
}

fn plane_from(
    value: &Value,
    name: &str,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<PlaneAnchor, CommandError> {
    if let Some(text) = value.as_str() {
        if let Some(plane) = base_plane(text) {
            return Ok(PlaneAnchor::Base(plane));
        }
        let datum = uuid::Uuid::parse_str(text)
            .map(FeatureId)
            .map_err(|_| CommandError::bad(name, "must be XY, XZ, YZ or a datum's id"))?;
        let node = ctx
            .document
            .get_feature_meta(datum)
            .filter(|n| n.workbench_id.as_str() == core_document::DATUM_KIND)
            .ok_or_else(|| CommandError::bad(name, "is not a datum of this document"))?;
        if node.body != Some(body) {
            return Err(CommandError::bad(name, "is a datum of another body"));
        }
        let made = ctx
            .document
            .feature_values(datum)
            .and_then(|v| DatumFeature::from_json(v).ok())
            .ok_or_else(|| CommandError::bad(name, "is a datum that does not read"))?;
        if !has_plane(&made) {
            return Err(CommandError::bad(
                name,
                "is a datum line or point, which has no plane",
            ));
        }
        let frame = made.frame();
        return Ok(PlaneAnchor::Datum {
            datum,
            origin: frame.origin,
            normal: frame.normal,
        });
    }
    Ok(PlaneAnchor::Face {
        face: face_from(value, name)?,
    })
}

/// Whether a datum has a plane another can be made from: a plane, or a
/// coordinate system by its XY plane.
pub fn has_plane(datum: &DatumFeature) -> bool {
    matches!(
        datum.shape,
        core_document::DatumShape::Plane { .. }
            | core_document::DatumShape::CoordinateSystem { .. }
    )
}

fn anchors<const N: usize, T>(
    a: &Args,
    name: &str,
    mut each: impl FnMut(&Value, &str) -> Result<T, CommandError>,
) -> Result<[T; N], CommandError> {
    let list = match a.0.get(name) {
        Some(Value::Array(list)) if list.len() == N => list,
        _ => return Err(CommandError::bad(name, format!("must list {N}"))),
    };
    let made: Vec<T> = list
        .iter()
        .enumerate()
        .map(|(i, v)| each(v, &format!("{name}[{}]", i + 1)))
        .collect::<Result<_, _>>()?;
    made.try_into()
        .map_err(|_| CommandError::bad(name, format!("must list {N}")))
}

/// The attachment `design.datum`'s arguments describe, in `body`'s own
/// frame.
pub fn attachment_from_args(
    a: &Args,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<DatumAttachment, CommandError> {
    let edge = || edge_from(&Value::Object(a.0.clone()), "edge");
    let mode = match a.opt_string("mode")? {
        Some(mode) => mode,
        None if a.has("face_point") => "flat_face",
        None => "base_plane",
    };
    Ok(match mode {
        "base_plane" => {
            let name = a.opt_string("plane")?.unwrap_or("XY");
            DatumAttachment::BasePlane(
                base_plane(name)
                    .ok_or_else(|| CommandError::bad("plane", "must be XY, XZ or YZ"))?,
            )
        }
        "flat_face" => DatumAttachment::FlatFace {
            point: vector3(a.0.get("face_point"), "face_point")?,
            normal: vector3(a.0.get("face_normal"), "face_normal")?,
        },
        "face" => DatumAttachment::Face {
            face: FaceAnchor {
                name: 0,
                point: vector3(a.0.get("face_point"), "face_point")?,
                normal: vector3(a.0.get("face_normal"), "face_normal")?,
                surface: None,
                follows: true,
            },
        },
        "three_points" => DatumAttachment::ThreePoints {
            points: anchors(a, "points", |v, n| point_from(v, n, ctx, body))?,
        },
        "two_points" => DatumAttachment::TwoPoints {
            points: anchors(a, "points", |v, n| point_from(v, n, ctx, body))?,
        },
        "normal_to_edge" => DatumAttachment::NormalToEdge {
            edge: edge()?,
            spot: spot_from(a.0.get("spot"), "spot")?,
            along: match a.opt_number("along")? {
                Some(v) if (0.0..=1.0).contains(&v) => Some(v as f32),
                Some(_) => return Err(CommandError::bad("along", "must be from 0 to 1")),
                None => None,
            },
        },
        "on_datum" => {
            let (datum, made) = datum_of(a.0.get("datum"), "datum", ctx, body)?;
            let plane = match made.shape {
                DatumShape::Plane { .. } => None,
                DatumShape::CoordinateSystem { .. } => Some(
                    base_plane(a.opt_string("plane")?.unwrap_or("XY"))
                        .ok_or_else(|| CommandError::bad("plane", "must be XY, XZ or YZ"))?,
                ),
                _ => return Err(CommandError::bad("datum", "is a datum line or point")),
            };
            let frame = made.frame();
            let frame = match plane {
                Some(which) => frame
                    .planes()
                    .into_iter()
                    .zip(BasePlane::ALL)
                    .find(|(_, p)| *p == which)
                    .map(|((_, f), _)| f)
                    .unwrap_or(frame),
                None => frame,
            };
            DatumAttachment::OnDatum {
                datum,
                plane,
                frame: frame.into(),
            }
        }
        "other_body" => {
            let other = BodyId(id_of(a.0.get("of_body"), "of_body")?);
            let plane = base_plane(a.opt_string("plane")?.unwrap_or("XY"))
                .ok_or_else(|| CommandError::bad("plane", "must be XY, XZ or YZ"))?;
            if other == body {
                return Err(CommandError::bad("of_body", "is the datum's own body"));
            }
            let frame = core_document::body_plane_in(ctx.document, body, other, plane)
                .ok_or_else(|| CommandError::bad("of_body", "is not a body of this document"))?;
            DatumAttachment::OtherBody {
                body: other,
                plane,
                frame: frame.into(),
            }
        }
        "face_normal" => DatumAttachment::FaceNormal {
            face: face_from(&Value::Object(a.0.clone()), "face")?,
        },
        "tangent_to_edge" => DatumAttachment::TangentToEdge {
            edge: edge()?,
            spot: spot_from(a.0.get("spot"), "spot")?,
        },
        "line_and_point" => DatumAttachment::LineAndPoint {
            line: line_from(a.0.get("line").unwrap_or(&Value::Null), "line", ctx, body)?,
            point: point_from(a.0.get("point").unwrap_or(&Value::Null), "point", ctx, body)?,
        },
        "line_meets_plane" => DatumAttachment::LineMeetsPlane {
            line: line_from(a.0.get("line").unwrap_or(&Value::Null), "line", ctx, body)?,
            plane: plane_from(a.0.get("plane").unwrap_or(&Value::Null), "plane", ctx, body)?,
        },
        "two_lines" => DatumAttachment::TwoLines {
            lines: anchors(a, "lines", |v, n| line_from(v, n, ctx, body))?,
        },
        "along_edge" => DatumAttachment::AlongEdge { edge: edge()? },
        "curve_centre" => DatumAttachment::CurveCentre { edge: edge()? },
        "plane_intersection" => DatumAttachment::PlaneIntersection {
            planes: anchors(a, "planes", |v, name| plane_from(v, name, ctx, body))?,
        },
        "midplane" => DatumAttachment::Midplane {
            planes: anchors(a, "planes", |v, name| plane_from(v, name, ctx, body))?,
        },
        "inertia" => DatumAttachment::Inertia {
            centre: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        },
        _ => {
            let modes: Vec<&str> = MODES.iter().map(|(m, _)| *m).collect();
            return Err(CommandError::bad(
                "mode",
                format!("must be one of {}", modes.join(", ")),
            ));
        }
    })
}

fn face_args(face: &FaceAnchor) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("face_point".into(), json!(face.point));
    args.insert("face_normal".into(), json!(face.normal));
    args
}

fn edge_args(edge: &EdgeAnchor) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("edge_point".into(), json!(edge.point));
    args.insert("edge_direction".into(), json!(edge.direction));
    args
}

fn point_args(point: &PointAnchor) -> Value {
    match point {
        PointAnchor::At { point } => json!(point),
        PointAnchor::Face { face } => Value::Object(face_args(face)),
        PointAnchor::Edge { edge, spot } => {
            let mut args = edge_args(edge);
            args.insert("spot".into(), json!(spot.key()));
            Value::Object(args)
        }
        PointAnchor::Datum { datum, .. } => json!({"datum": datum.0.to_string()}),
        PointAnchor::Sketch {
            sketch, element, ..
        } => json!({"sketch": sketch.0.to_string(), "element": element.to_string()}),
    }
}

fn line_args(line: &LineAnchor) -> Value {
    match line {
        LineAnchor::Edge { edge } => Value::Object(edge_args(edge)),
        LineAnchor::Datum { datum, .. } => json!(datum.0.to_string()),
        LineAnchor::Sketch {
            sketch, element, ..
        } => json!({"sketch": sketch.0.to_string(), "element": element.to_string()}),
    }
}

fn plane_args(plane: &PlaneAnchor) -> Value {
    match plane {
        PlaneAnchor::Base(plane) => json!(base_plane_key(*plane)),
        PlaneAnchor::Datum { datum, .. } => json!(datum.0.to_string()),
        PlaneAnchor::Face { face } => Value::Object(face_args(face)),
    }
}

/// The arguments of `design.datum` that make `attachment` again, as the
/// references were picked.
pub fn attachment_args(attachment: &DatumAttachment) -> Map<String, Value> {
    let mut args = match attachment {
        DatumAttachment::BasePlane(plane) => {
            let mut args = Map::new();
            args.insert("plane".into(), json!(base_plane_key(*plane)));
            args
        }
        DatumAttachment::FlatFace { point, normal } => {
            let mut args = Map::new();
            args.insert("face_point".into(), json!(point));
            args.insert("face_normal".into(), json!(normal));
            args
        }
        DatumAttachment::Face { face } => face_args(face),
        DatumAttachment::ThreePoints { points } => {
            let mut args = Map::new();
            args.insert("points".into(), points.iter().map(point_args).collect());
            args
        }
        DatumAttachment::TwoPoints { points } => {
            let mut args = Map::new();
            args.insert("points".into(), points.iter().map(point_args).collect());
            args
        }
        DatumAttachment::NormalToEdge { edge, spot, along } => {
            let mut args = edge_args(edge);
            args.insert("spot".into(), json!(spot.key()));
            if let Some(along) = along {
                args.insert("along".into(), json!(along));
            }
            args
        }
        DatumAttachment::TangentToEdge { edge, spot } => {
            let mut args = edge_args(edge);
            args.insert("spot".into(), json!(spot.key()));
            args
        }
        DatumAttachment::OnDatum { datum, plane, .. } => {
            let mut args = Map::new();
            args.insert("datum".into(), json!(datum.0.to_string()));
            if let Some(plane) = plane {
                args.insert("plane".into(), json!(base_plane_key(*plane)));
            }
            args
        }
        DatumAttachment::OtherBody { body, plane, .. } => {
            let mut args = Map::new();
            args.insert("of_body".into(), json!(body.0.to_string()));
            args.insert("plane".into(), json!(base_plane_key(*plane)));
            args
        }
        DatumAttachment::FaceNormal { face } => face_args(face),
        DatumAttachment::LineAndPoint { line, point } => {
            let mut args = Map::new();
            args.insert("line".into(), line_args(line));
            args.insert("point".into(), point_args(point));
            args
        }
        DatumAttachment::LineMeetsPlane { line, plane } => {
            let mut args = Map::new();
            args.insert("line".into(), line_args(line));
            args.insert("plane".into(), plane_args(plane));
            args
        }
        DatumAttachment::TwoLines { lines } => {
            let mut args = Map::new();
            args.insert("lines".into(), lines.iter().map(line_args).collect());
            args
        }
        DatumAttachment::AlongEdge { edge } | DatumAttachment::CurveCentre { edge } => {
            edge_args(edge)
        }
        DatumAttachment::PlaneIntersection { planes } | DatumAttachment::Midplane { planes } => {
            let mut args = Map::new();
            args.insert("planes".into(), planes.iter().map(plane_args).collect());
            args
        }
        DatumAttachment::Inertia { .. } => Map::new(),
    };
    args.insert("mode".into(), json!(attachment.mode()));
    args
}

/// The datum's dependencies changed with its attachment: the document's
/// record of them follows.
pub fn sync_dependencies(ctx: &mut WorkbenchRuntimeContext, id: FeatureId, datum: &DatumFeature) {
    let wanted = datum.dependencies();
    let current = ctx.document.feature_tree().dependencies(id);
    if current != wanted {
        ctx.document.set_feature_dependencies(id, wanted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, Workbench};
    use kernel_api::{KernelQueries, KernelResult, ProbeAnswer, ProbedCircle, ShapeProbe};

    use crate::DesignWorkbench;

    /// A kernel that knows one solid: a cylinder of radius 5 on the XY
    /// plane, 12 high, its rim at the top, and a straight edge from the
    /// origin to (10, 0, 0).
    struct Cylinder;

    impl KernelQueries for Cylinder {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &kernel_api::ProfilePlane,
        ) -> KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("projecting".into()))
        }

        fn probe(&self, _brep: &[u8], probe: &ShapeProbe) -> KernelResult<ProbeAnswer> {
            Ok(match *probe {
                ShapeProbe::Face { point, .. } => {
                    let r = (point[0] * point[0] + point[1] * point[1]).sqrt();
                    let n = [point[0] / r, point[1] / r, 0.0];
                    ProbeAnswer::Face {
                        point: [n[0] * 5.0, n[1] * 5.0, point[2]],
                        normal: n,
                        surface: kernel_api::FaceSurface::Cylinder {
                            origin: [0.0; 3],
                            axis: [0.0, 0.0, 1.0],
                            radius: 5.0,
                        },
                    }
                }
                ShapeProbe::Edge { point, .. } if point[2] > 6.0 => ProbeAnswer::Edge {
                    along: None,
                    point: [5.0, 0.0, 12.0],
                    direction: [0.0, 1.0, 0.0],
                    start: [5.0, 0.0, 12.0],
                    end: [5.0, 0.0, 12.0],
                    middle: [-5.0, 0.0, 12.0],
                    circle: Some(ProbedCircle {
                        centre: [0.0, 0.0, 12.0],
                        normal: [0.0, 0.0, 1.0],
                        radius: 5.0,
                    }),
                },
                ShapeProbe::Edge { point, .. } => ProbeAnswer::Edge {
                    along: None,
                    point: [point[0], 0.0, 0.0],
                    direction: [1.0, 0.0, 0.0],
                    start: [0.0; 3],
                    end: [10.0, 0.0, 0.0],
                    middle: [5.0, 0.0, 0.0],
                    circle: None,
                },
                ShapeProbe::Mass => ProbeAnswer::Mass {
                    centre: [0.0, 0.0, 6.0],
                    axes: [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                },
            })
        }
    }

    static CYLINDER: Cylinder = Cylinder;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    fn parallel(a: [f32; 3], b: [f32; 3]) -> bool {
        (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).abs() > 1.0 - 1e-5
    }

    /// A document with one body whose solid the stand-in kernel answers
    /// for.
    fn with_solid() -> (Document, BodyId) {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        doc.set_imported_brep_data(body, b"cylinder".to_vec(), Vec::new());
        (doc, body)
    }

    fn datum(
        doc: &mut Document,
        body: BodyId,
        kernel: bool,
        args: Value,
    ) -> Result<(FeatureId, DatumFeature), core_document::CommandError> {
        let mut bench = DesignWorkbench::default();
        let mut args = args.as_object().unwrap().clone();
        args.insert("body".into(), json!(body.0.to_string()));
        let made = {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            if kernel {
                ctx.kernel = Some(&CYLINDER);
            }
            bench.run_command("design.datum", &args, &mut ctx)?
        };
        let id = crate::test_support::made_id(&made);
        let datum = DatumFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        Ok((id, datum))
    }

    #[test]
    fn a_plane_on_a_face_is_tangent_to_the_cylinder_where_picked() {
        let (mut doc, body) = with_solid();
        let (_, made) = datum(
            &mut doc,
            body,
            true,
            json!({"kind": "plane", "mode": "face",
                   "face_point": [0, 5.2, 4], "face_normal": [0, 1, 0]}),
        )
        .unwrap();
        let frame = made.frame();
        assert!(close(frame.origin, [0.0, 5.0, 4.0]), "{frame:?}");
        assert!(close(frame.normal, [0.0, 1.0, 0.0]));
        assert!(parallel(frame.x_axis, [0.0, 0.0, 1.0]), "along the axis");
        assert_eq!(made.probes().len(), 1, "it follows the solid");
    }

    #[test]
    fn three_points_and_two_points_take_given_and_picked_points() {
        let (mut doc, body) = with_solid();
        let (_, plane) = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "plane", "mode": "three_points",
                   "points": [[0, 0, 2], [1, 0, 2], {"x": 0, "y": 1, "z": 2}]}),
        )
        .unwrap();
        assert!(close(plane.frame().normal, [0.0, 0.0, 1.0]));
        assert!(close(plane.frame().origin, [0.0, 0.0, 2.0]));

        let (_, line) = datum(
            &mut doc,
            body,
            true,
            json!({"kind": "line", "mode": "two_points", "points": [
                {"edge_point": [3, 0, 0], "edge_direction": [1, 0, 0], "spot": "end"},
                [10, 0, 8],
            ]}),
        )
        .unwrap();
        let frame = line.frame();
        assert!(close(frame.origin, [10.0, 0.0, 4.0]), "{frame:?}");
        assert!(parallel(frame.x_axis, [0.0, 0.0, 1.0]));
    }

    #[test]
    fn points_in_a_line_make_no_plane() {
        let (mut doc, body) = with_solid();
        let refused = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "plane", "mode": "three_points",
                   "points": [[0, 0, 0], [1, 0, 0], [2, 0, 0]]}),
        );
        assert!(refused.is_err());
    }

    #[test]
    fn edge_modes_stand_on_the_edge_the_solid_names() {
        let (mut doc, body) = with_solid();
        let (_, square) = datum(
            &mut doc,
            body,
            true,
            json!({"kind": "plane", "mode": "normal_to_edge",
                   "edge_point": [3, 0, 0], "edge_direction": [1, 0, 0], "spot": "end"}),
        )
        .unwrap();
        assert!(close(square.frame().origin, [10.0, 0.0, 0.0]));
        assert!(parallel(square.frame().normal, [1.0, 0.0, 0.0]));

        let (_, axis) = datum(
            &mut doc,
            body,
            true,
            json!({"kind": "line", "mode": "along_edge",
                   "edge_point": [5, 0, 12], "edge_direction": [0, 1, 0]}),
        )
        .unwrap();
        assert!(close(axis.frame().origin, [0.0, 0.0, 12.0]));
        assert!(parallel(axis.frame().x_axis, [0.0, 0.0, 1.0]));

        let (_, centre) = datum(
            &mut doc,
            body,
            true,
            json!({"kind": "point", "mode": "curve_centre",
                   "edge_point": [5, 0, 12], "edge_direction": [0, 1, 0]}),
        )
        .unwrap();
        assert!(close(centre.frame().origin, [0.0, 0.0, 12.0]));

        // Without the solid to ask, a straight pick has no centre.
        let refused = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "point", "mode": "curve_centre",
                   "edge_point": [5, 0, 12], "edge_direction": [0, 1, 0]}),
        );
        assert!(refused.is_err());
    }

    #[test]
    fn two_planes_meet_in_a_line_that_follows_a_datum_it_is_made_from() {
        let (mut doc, body) = with_solid();
        let (base, _) = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "plane", "offset": [0, 0, 4]}),
        )
        .unwrap();
        let (line_id, line) = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "line", "mode": "plane_intersection",
                   "planes": [base.0.to_string(), "YZ"]}),
        )
        .unwrap();
        let frame = line.frame();
        assert!(parallel(frame.x_axis, [0.0, 1.0, 0.0]));
        assert!((frame.origin[2] - 4.0).abs() < 1e-4 && frame.origin[0].abs() < 1e-4);
        assert_eq!(doc.feature_tree().dependencies(line_id), vec![base]);

        let other = doc.create_body(None);
        let refused = datum(
            &mut doc,
            other,
            false,
            json!({"kind": "line", "mode": "plane_intersection",
                   "planes": [base.0.to_string(), "YZ"]}),
        );
        assert!(refused.is_err(), "a datum of another body");
        let parallel_planes = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "line", "mode": "plane_intersection", "planes": ["XY", "XY"]}),
        );
        assert!(parallel_planes.is_err());
        let from_a_line = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "line", "mode": "plane_intersection",
                   "planes": [line_id.0.to_string(), "XY"]}),
        );
        assert!(from_a_line.is_err(), "a datum line has no plane");
    }

    #[test]
    fn a_coordinate_system_at_the_centre_of_mass_takes_the_axes_of_inertia() {
        let (mut doc, body) = with_solid();
        let (_, made) = datum(
            &mut doc,
            body,
            true,
            json!({"kind": "coordinate_system", "mode": "inertia"}),
        )
        .unwrap();
        let frame = made.frame();
        assert!(close(frame.origin, [0.0, 0.0, 6.0]));
        assert!(close(frame.x_axis, [0.0, 0.0, 1.0]));
        assert!(close(frame.normal, [0.0, 1.0, 0.0]));

        let refused = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "point", "mode": "inertia"}),
        );
        assert!(refused.is_err(), "no kernel to weigh the solid with");
    }

    /// What a recording writes of a datum makes the same datum again.
    #[test]
    fn every_mode_s_arguments_make_it_again() {
        let (mut doc, body) = with_solid();
        let (base, _) = datum(&mut doc, body, false, json!({"kind": "plane"})).unwrap();
        let (system, _) = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "coordinate_system", "offset": [1, 2, 3]}),
        )
        .unwrap();
        let (dot, _) = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "point", "offset": [4, 4, 4]}),
        )
        .unwrap();
        let (axis, _) = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "line", "mode": "base_plane", "plane": "YZ"}),
        )
        .unwrap();
        let other = doc.create_body(None);
        // A sketch with a line and a point to take them from.
        let (sketch, line, point) = {
            use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};
            let mut drawing = Sketch::new("refs");
            let a = drawing.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 7.0))));
            let b = drawing.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(9.0, 7.0))));
            let line = drawing.add_geometry(GeometryElement::Line(Line::new(a, b)));
            let point =
                drawing.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(3.0, -2.0))));
            let plane = drawing.plane;
            let id = doc
                .add_feature_in_body(
                    wb_sketch::SketchFeature::new(drawing, plane),
                    "refs".into(),
                    Some(body),
                )
                .unwrap();
            (id, line, point)
        };
        let made = [
            json!({"mode": "base_plane", "plane": "XZ"}),
            json!({"face_point": [0, 0, 3], "face_normal": [0, 0, 1]}),
            json!({"mode": "face", "face_point": [0, 5, 4], "face_normal": [0, 1, 0]}),
            json!({"mode": "three_points", "points": [
                [0, 0, 2],
                {"face_point": [1, 0, 2], "face_normal": [0, 0, 1]},
                {"edge_point": [0, 1, 2], "edge_direction": [1, 0, 0], "spot": "middle"},
            ]}),
            json!({"mode": "normal_to_edge", "edge_point": [1, 0, 0],
                   "edge_direction": [1, 0, 0], "spot": "start"}),
            json!({"mode": "along_edge", "edge_point": [1, 0, 0], "edge_direction": [1, 0, 0]}),
            json!({"mode": "two_points", "points": [[0, 0, 0], [0, 0, 1]]}),
            json!({"mode": "plane_intersection", "planes": [
                base.0.to_string(),
                {"face_point": [0, 5, 0], "face_normal": [0, 1, 0]},
            ]}),
            json!({"mode": "midplane", "planes": [
                base.0.to_string(),
                {"face_point": [0, 0, 7], "face_normal": [0, 0, -1]},
            ]}),
            json!({"mode": "normal_to_edge", "edge_point": [1, 0, 0],
                   "edge_direction": [1, 0, 0], "along": 0.25}),
            json!({"mode": "on_datum", "datum": base.0.to_string()}),
            json!({"mode": "on_datum", "datum": system.0.to_string(), "plane": "YZ"}),
            json!({"mode": "other_body", "of_body": other.0.to_string(), "plane": "XZ"}),
            json!({"mode": "face_normal", "face_point": [0, 5, 4], "face_normal": [0, 1, 0]}),
            json!({"mode": "tangent_to_edge", "edge_point": [1, 0, 0],
                   "edge_direction": [1, 0, 0], "spot": "end"}),
            json!({"mode": "line_and_point",
                   "line": {"sketch": sketch.0.to_string(), "element": line.to_string()},
                   "point": {"datum": dot.0.to_string()}}),
            json!({"mode": "line_meets_plane",
                   "line": {"edge_point": [1, 0, 0], "edge_direction": [0, 0, 1]},
                   "plane": "XY"}),
            json!({"mode": "two_lines", "lines": [
                axis.0.to_string(),
                {"sketch": sketch.0.to_string(), "element": line.to_string()},
            ]}),
            json!({"mode": "three_points", "points": [
                [0, 0, 2],
                {"sketch": sketch.0.to_string(), "element": point.to_string()},
                {"datum": dot.0.to_string()},
            ]}),
        ];
        for args in made {
            let mut with_kind = args.as_object().unwrap().clone();
            with_kind.insert("kind".into(), json!("plane"));
            let (_, first) = datum(&mut doc, body, false, Value::Object(with_kind.clone()))
                .unwrap_or_else(|e| panic!("{args}: {e}"));
            let mut again = attachment_args(&first.attachment);
            again.insert("kind".into(), json!("plane"));
            let (_, second) = datum(&mut doc, body, false, Value::Object(again)).unwrap();
            assert_eq!(first.attachment, second.attachment, "{args}");
        }
    }

    #[test]
    fn a_build_asks_each_datum_s_questions_where_it_stands() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut bench = DesignWorkbench::default();
        let early = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "plane", "mode": "face",
                   "face_point": [0, 5, 4], "face_normal": [0, 1, 0]}),
        )
        .unwrap()
        .0;
        {
            let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            let args = json!({"body": body.0.to_string(), "variant": "cylinder"});
            bench
                .run_command("design.primitive", args.as_object().unwrap(), &mut ctx)
                .unwrap();
        }
        let late = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "point", "mode": "inertia_free_face",
                   "face_point": [0, 5, 4], "face_normal": [0, 1, 0]}),
        );
        assert!(late.is_err(), "an unknown mode is refused");
        let late = datum(
            &mut doc,
            body,
            false,
            json!({"kind": "point", "mode": "face",
                   "face_point": [0, 5, 4], "face_normal": [0, 1, 0]}),
        )
        .unwrap()
        .0;

        let jobs = crate::build::rebuild_jobs(&mut doc);
        let plan = jobs[0].plan.as_ref().unwrap();
        let after = |feature: FeatureId| {
            plan.probes
                .iter()
                .find(|p| p.feature == feature)
                .map(|p| p.probe.after_op)
        };
        assert_eq!(after(early), Some(0), "before the cylinder: nothing to ask");
        assert_eq!(
            after(late),
            Some(plan.ops.len()),
            "after all the cylinder's ops"
        );
        assert!(crate::build::rebuild_jobs(&mut doc).is_empty());

        // A datum stood on something else: the body is built again to ask.
        let mut data = doc.get_feature_data(late).unwrap().clone();
        data["attachment"]["Face"]["face"]["point"] = json!([5.0, 0.0, 4.0]);
        doc.update_feature_data(late, data).unwrap();
        assert_eq!(crate::build::rebuild_jobs(&mut doc).len(), 1);
    }
}
