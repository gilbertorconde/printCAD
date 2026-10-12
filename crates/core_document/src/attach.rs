//! Attaching a datum-like frame to what is picked: the modes an attachment
//! can take, the picks read as references on a body, the attachment each
//! mode makes of them, and the first look the body's solid gives them. A
//! datum's task and a sketch's attachment both build on it.

use crate::{
    BasePlane, BodyId, DatumAttachment, DatumFeature, DatumFrame, EdgeAnchor, EdgeRef, EdgeSpot,
    FaceAnchor, FaceRef, FeatureId, FrameAnchor, LineAnchor, PlaneAnchor, PointAnchor,
    WorkbenchRuntimeContext,
};

/// The modes the selector offers after the base planes, with what each
/// needs picked first.
pub const PICK_MODES: &[(&str, &str)] = &[
    ("face", "Click a face first"),
    ("three_points", ""),
    ("normal_to_edge", "Click an edge first"),
    ("along_edge", "Click an edge first"),
    ("two_points", ""),
    ("plane_intersection", ""),
    ("curve_centre", "Click a circular edge first"),
    ("inertia", "Needs the body's solid"),
    (
        "on_datum",
        "Needs a datum plane or coordinate system made before",
    ),
    ("other_body", "Needs another body"),
    ("face_normal", "Click a face first"),
    ("tangent_to_edge", "Click an edge first"),
    ("line_and_point", "Click an edge first"),
    ("line_meets_plane", "Click an edge first"),
    ("two_lines", "Click two edges first"),
    ("midplane", "Click two faces first"),
];

/// What is picked in the viewport, as references in the datum's body.
pub struct Picked {
    /// The face picked last.
    pub face: Option<FaceAnchor>,
    /// Every picked face, in the order picked.
    pub faces: Vec<FaceAnchor>,
    pub edges: Vec<EdgeAnchor>,
    /// Datums a datum may stand on: their id, frame and whether they are a
    /// coordinate system. Empty unless the caller says which.
    pub datums: Vec<(FeatureId, DatumFrame, bool)>,
}

impl Picked {
    pub fn of(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Self {
        let on_body = ctx.selected_body_id == Some(body.0);
        let face = ctx
            .selected_face_in(body)
            .map(|face| face_anchor(&face, on_body));
        let faces = ctx
            .selected_faces_in(body)
            .iter()
            .map(|face| face_anchor(face, on_body))
            .collect();
        let edges = ctx
            .selected_edges_in(body)
            .iter()
            .map(|edge| edge_anchor(edge, edge.body == body.0))
            .collect();
        Self {
            face,
            faces,
            edges,
            datums: Vec::new(),
        }
    }

    /// With the datum planes and coordinate systems of `body` it may stand
    /// on: those `before` lets through.
    pub fn with_datums(
        mut self,
        ctx: &WorkbenchRuntimeContext,
        body: BodyId,
        before: impl Fn(FeatureId) -> bool,
    ) -> Self {
        self.datums = crate::datums_of_body(ctx.document, body)
            .into_iter()
            .filter(|(id, _, datum)| {
                before(*id)
                    && matches!(
                        datum.shape,
                        crate::DatumShape::Plane { .. }
                            | crate::DatumShape::CoordinateSystem { .. }
                    )
            })
            .map(|(id, _, datum)| {
                let system = matches!(datum.shape, crate::DatumShape::CoordinateSystem { .. });
                (id, datum.frame(), system)
            })
            .collect();
        self
    }

    /// The picked points: each edge where it was picked, then the face.
    pub fn points(&self) -> Vec<PointAnchor> {
        self.edges
            .iter()
            .map(|edge| PointAnchor::Edge {
                edge: *edge,
                spot: EdgeSpot::Picked,
            })
            .chain(self.face.map(|face| PointAnchor::Face { face }))
            .collect()
    }
}

fn add(a: [f32; 3], b: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

/// The edge the datum stands on now, when its mode takes one.
pub fn current_edge(attachment: &DatumAttachment) -> Option<EdgeAnchor> {
    match attachment {
        DatumAttachment::NormalToEdge { edge, .. }
        | DatumAttachment::AlongEdge { edge }
        | DatumAttachment::CurveCentre { edge }
        | DatumAttachment::TangentToEdge { edge, .. } => Some(*edge),
        DatumAttachment::LineAndPoint {
            line: LineAnchor::Edge { edge },
            ..
        }
        | DatumAttachment::LineMeetsPlane {
            line: LineAnchor::Edge { edge },
            ..
        } => Some(*edge),
        _ => None,
    }
}

/// The attachment `mode` makes from what is picked, `frame` (where the
/// datum or sketch is now) filling in the points not picked and `standing`
/// the edge it stands on now; `None` when the mode needs a pick there is
/// not.
pub fn candidate(
    mode: &str,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    frame: DatumFrame,
    standing: Option<EdgeAnchor>,
    picked: &Picked,
) -> Option<DatumAttachment> {
    let edge = picked.edges.first().copied().or(standing);
    let points = |n: usize| -> Vec<PointAnchor> {
        let spare = [
            frame.origin,
            add(frame.origin, frame.x_axis, 10.0),
            add(frame.origin, frame.y_axis(), 10.0),
        ];
        let mut points = picked.points();
        points.truncate(n);
        while points.len() < n {
            points.push(PointAnchor::At {
                point: spare[points.len()],
            });
        }
        points
    };
    Some(match mode {
        "face" => DatumAttachment::Face { face: picked.face? },
        "three_points" => DatumAttachment::ThreePoints {
            points: points(3).try_into().ok()?,
        },
        "two_points" => DatumAttachment::TwoPoints {
            points: points(2).try_into().ok()?,
        },
        "normal_to_edge" => DatumAttachment::NormalToEdge {
            edge: edge?,
            spot: EdgeSpot::Picked,
            along: None,
        },
        "along_edge" => DatumAttachment::AlongEdge { edge: edge? },
        "on_datum" => {
            let (datum, found, system) = picked.datums.first().copied()?;
            let plane = system.then_some(BasePlane::XY);
            DatumAttachment::OnDatum {
                datum,
                plane,
                frame: found.into(),
            }
        }
        "other_body" => {
            let other = ctx
                .document
                .bodies()
                .iter()
                .map(|b| b.id)
                .find(|b| *b != body)?;
            let found = crate::body_plane_in(ctx.document, body, other, BasePlane::XY)?;
            DatumAttachment::OtherBody {
                body: other,
                plane: BasePlane::XY,
                frame: found.into(),
            }
        }
        "face_normal" => DatumAttachment::FaceNormal { face: picked.face? },
        "tangent_to_edge" => DatumAttachment::TangentToEdge {
            edge: edge?,
            spot: EdgeSpot::Picked,
        },
        "line_and_point" => {
            let line = LineAnchor::Edge { edge: edge? };
            // The point: a second edge where picked, else the face, else
            // a spare point off the line.
            let point = picked
                .edges
                .get(1)
                .map(|edge| PointAnchor::Edge {
                    edge: *edge,
                    spot: EdgeSpot::Picked,
                })
                .or(picked.face.map(|face| PointAnchor::Face { face }))
                .unwrap_or(PointAnchor::At {
                    point: add(frame.origin, frame.y_axis(), 10.0),
                });
            DatumAttachment::LineAndPoint { line, point }
        }
        "line_meets_plane" => DatumAttachment::LineMeetsPlane {
            line: LineAnchor::Edge { edge: edge? },
            plane: match picked.face {
                Some(face) => PlaneAnchor::Face { face },
                None => PlaneAnchor::Base(BasePlane::XY),
            },
        },
        "two_lines" => DatumAttachment::TwoLines {
            lines: [
                LineAnchor::Edge {
                    edge: *picked.edges.first()?,
                },
                LineAnchor::Edge {
                    edge: *picked.edges.get(1)?,
                },
            ],
        },
        "curve_centre" => DatumAttachment::CurveCentre {
            edge: edge.filter(|e| e.circle.is_some())?,
        },
        "midplane" => DatumAttachment::Midplane {
            planes: [
                PlaneAnchor::Face {
                    face: *picked.faces.first()?,
                },
                PlaneAnchor::Face {
                    face: *picked.faces.get(1)?,
                },
            ],
        },
        "plane_intersection" => {
            let first = match picked.face {
                Some(face) => PlaneAnchor::Face { face },
                None => PlaneAnchor::Base(BasePlane::XY),
            };
            let (_, normal) = first.plane();
            // The base plane least like the first.
            let second = BasePlane::ALL
                .into_iter()
                .min_by(|a, b| {
                    let along = |p: &BasePlane| {
                        let (_, n, _) = p.frame();
                        (n[0] * normal[0] + n[1] * normal[1] + n[2] * normal[2]).abs()
                    };
                    along(a).total_cmp(&along(b))
                })
                .map(PlaneAnchor::Base)?;
            DatumAttachment::PlaneIntersection {
                planes: [first, second],
            }
        }
        "inertia" => {
            ctx.kernel?;
            ctx.document.imported_brep_blob(body)?;
            DatumAttachment::Inertia {
                centre: [0.0; 3],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            }
        }
        _ => return None,
    })
}

pub fn mode_label(mode: &str) -> &'static str {
    let face = FaceAnchor {
        name: 0,
        point: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
        surface: None,
        follows: false,
    };
    let edge = EdgeAnchor {
        faces: [0, 0],
        point: [0.0; 3],
        direction: [1.0, 0.0, 0.0],
        ends: None,
        middle: None,
        circle: None,
        follows: false,
        along: None,
    };
    let point = PointAnchor::At { point: [0.0; 3] };
    let plane = PlaneAnchor::Base(BasePlane::XY);
    let frame = FrameAnchor {
        origin: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
        x_axis: [1.0, 0.0, 0.0],
    };
    let line = LineAnchor::Edge { edge };
    match mode {
        "face" => DatumAttachment::Face { face },
        "three_points" => DatumAttachment::ThreePoints { points: [point; 3] },
        "normal_to_edge" => DatumAttachment::NormalToEdge {
            edge,
            spot: EdgeSpot::Picked,
            along: None,
        },
        "on_datum" => DatumAttachment::OnDatum {
            datum: FeatureId(uuid::Uuid::nil()),
            plane: None,
            frame,
        },
        "other_body" => DatumAttachment::OtherBody {
            body: BodyId(uuid::Uuid::nil()),
            plane: BasePlane::XY,
            frame,
        },
        "face_normal" => DatumAttachment::FaceNormal { face },
        "tangent_to_edge" => DatumAttachment::TangentToEdge {
            edge,
            spot: EdgeSpot::Picked,
        },
        "line_and_point" => DatumAttachment::LineAndPoint { line, point },
        "line_meets_plane" => DatumAttachment::LineMeetsPlane { line, plane },
        "two_lines" => DatumAttachment::TwoLines { lines: [line; 2] },
        "along_edge" => DatumAttachment::AlongEdge { edge },
        "two_points" => DatumAttachment::TwoPoints { points: [point; 2] },
        "plane_intersection" => DatumAttachment::PlaneIntersection { planes: [plane; 2] },
        "midplane" => DatumAttachment::Midplane { planes: [plane; 2] },
        "curve_centre" => DatumAttachment::CurveCentre { edge },
        _ => DatumAttachment::Inertia {
            centre: [0.0; 3],
            axes: [[0.0; 3]; 3],
        },
    }
    .label()
}

/// A picked face as a datum keeps it; `follows` when it is on the datum's
/// own body.
pub fn face_anchor(face: &FaceRef, follows: bool) -> FaceAnchor {
    FaceAnchor {
        name: face.name,
        point: face.point,
        normal: face.normal,
        surface: face.surface,
        follows,
    }
}

/// A picked edge as a datum keeps it; `follows` when it is on the datum's
/// own body.
pub fn edge_anchor(edge: &EdgeRef, follows: bool) -> EdgeAnchor {
    EdgeAnchor {
        faces: edge.faces,
        point: edge.point,
        direction: edge.direction,
        ends: None,
        middle: None,
        circle: edge.circle.map(|c| crate::AnchorCircle {
            center: c.center,
            normal: c.normal,
            radius: c.radius,
        }),
        follows,
        along: None,
    }
}

/// Fill in what the body's solid says of the datum's references (a face's
/// surface, an edge's ends, the centre of mass) and check they make a
/// frame. Without a kernel or a solid the references stay as given, and a
/// datum at the centre of mass cannot be made.
pub fn settle(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    datum: &mut DatumFeature,
) -> Result<(), String> {
    let probes = datum.probes();
    if !probes.is_empty() {
        let solid = ctx
            .kernel
            .zip(ctx.document.imported_brep_blob(body))
            .filter(|_| !ctx.document.is_mesh_body(body));
        match solid {
            Some((kernel, blob)) => {
                let answers: Vec<Result<kernel_api::ProbeAnswer, String>> = probes
                    .iter()
                    .map(|probe| kernel.probe(blob, probe).map_err(|e| e.to_string()))
                    .collect();
                if let Some(Err(e)) = answers.iter().find(|a| a.is_err()) {
                    return Err(format!("the body's solid does not answer: {e}"));
                }
                datum.take_answers(&answers);
            }
            None if matches!(datum.attachment, DatumAttachment::Inertia { .. }) => {
                return Err("the centre of mass needs the body's solid".into());
            }
            None => {}
        }
    }
    match datum.attachment.problem() {
        Some(problem) => Err(problem.to_string()),
        None => Ok(()),
    }
}
