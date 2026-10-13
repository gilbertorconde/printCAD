//! The measure tool's picks and what they measure: a point, an edge (its
//! length, a circle's radius), a face (its area, a round face's radius, a
//! cone's angle, a torus's radii) or a datum (a plane, a line, a point).
//! Picks run on in a chain: each new one is measured against the one
//! before it (the distance and, where both have a direction, the angle),
//! and the chain's totals follow (its path, its edges' length, its faces'
//! area).

use core_document::{DatumFeature, DatumShape, EdgeCircle, Unit, format_area_mm2};
use glam::Vec3;
use kernel_api::{FaceSurface, TriMesh};

use crate::kernel_worker::MeasuredPart;

/// One thing the measure tool picked, in world space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MeasurePick {
    Point([f32; 3]),
    Edge {
        point: [f32; 3],
        direction: [f32; 3],
        length: f32,
        circle: Option<EdgeCircle>,
        /// The body it is an edge of.
        body: Option<core_document::BodyId>,
    },
    Face {
        point: [f32; 3],
        normal: [f32; 3],
        surface: Option<FaceSurface>,
        area: f64,
        /// The body it is a face of.
        body: Option<core_document::BodyId>,
    },
    /// A datum, by name, and where on it the click landed.
    Datum {
        name: String,
        datum: DatumPick,
        point: [f32; 3],
    },
}

/// What a picked datum stands for, placed in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DatumPick {
    Plane {
        origin: [f32; 3],
        normal: [f32; 3],
    },
    Line {
        origin: [f32; 3],
        direction: [f32; 3],
    },
    /// A datum point, or a coordinate system's origin.
    Point,
}

impl DatumPick {
    /// The datum `feature` (in its body's frame) placed by `placement`, and
    /// where a ray from `eye` along `ray` meets it: a plane where the ray
    /// crosses it, a line where it passes nearest, else its origin.
    pub fn of(
        feature: &DatumFeature,
        placement: core_document::BodyPlacement,
        eye: Vec3,
        ray: Vec3,
    ) -> (Self, [f32; 3]) {
        let frame = feature.frame();
        let origin = Vec3::from_array(placement.point(frame.origin));
        match feature.shape {
            DatumShape::Plane { .. } => {
                let normal =
                    Vec3::from_array(placement.direction(frame.normal)).normalize_or_zero();
                let across = ray.dot(normal);
                let point = if across.abs() > 1e-6 {
                    eye + ray * ((origin - eye).dot(normal) / across)
                } else {
                    origin
                };
                (
                    DatumPick::Plane {
                        origin: origin.to_array(),
                        normal: normal.to_array(),
                    },
                    point.to_array(),
                )
            }
            DatumShape::Line { .. } => {
                let direction =
                    Vec3::from_array(placement.direction(frame.x_axis)).normalize_or_zero();
                let point = nearest_on_line(origin, direction, eye, ray);
                (
                    DatumPick::Line {
                        origin: origin.to_array(),
                        direction: direction.to_array(),
                    },
                    point.to_array(),
                )
            }
            DatumShape::Point | DatumShape::CoordinateSystem { .. } => {
                (DatumPick::Point, origin.to_array())
            }
        }
    }
}

/// The point of the line through `origin` along unit `direction` nearest
/// the line through `eye` along `ray`; its origin where they run parallel.
fn nearest_on_line(origin: Vec3, direction: Vec3, eye: Vec3, ray: Vec3) -> Vec3 {
    let ray = ray.normalize_or_zero();
    let b = direction.dot(ray);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return origin;
    }
    let w = origin - eye;
    let t = (b * w.dot(ray) - w.dot(direction)) / denom;
    origin + direction * t
}

/// The kernel's least distance between the measure tool's last two picks
/// of solids (faces or edges): the request it answers, where the first
/// pick's body sat when asked, and once it is in, the distance and the
/// closest points in that body's own frame.
#[derive(Debug, Clone)]
pub(crate) struct MeasureGap {
    pub token: u64,
    pub first_placement: core_document::BodyPlacement,
    pub found: Option<Result<crate::kernel_worker::LeastDistance, String>>,
}

impl MeasureGap {
    /// The distance and the two closest points in the world, once found.
    pub fn in_world(&self) -> Option<(f32, Vec3, Vec3)> {
        let Some(Ok((distance, a, b))) = &self.found else {
            return None;
        };
        let world =
            |p: &[f64; 3]| Vec3::from_array(self.first_placement.point(p.map(|v| v as f32)));
        Some((*distance as f32, world(a), world(b)))
    }
}

/// The status bar's quick measure: the selection's picks, and the least
/// distance between the last two when the kernel can measure it.
#[derive(Debug, Clone)]
pub(crate) struct QuickMeasure {
    pub picks: Vec<MeasurePick>,
    pub gap: Option<MeasureGap>,
}

impl QuickMeasure {
    /// What the selection reads, one line each, as the measure tool reads
    /// the same picks; where the least distance is asked, it stands in for
    /// the distance between the points the picks were made at.
    pub fn lines(&self, unit: Unit) -> Vec<String> {
        let mut lines = readout(&self.picks, unit);
        let Some(gap) = &self.gap else {
            return lines;
        };
        if let Some(i) = lines.iter().position(|l| l.starts_with("Δx")) {
            lines.drain(i.saturating_sub(1)..=i);
        }
        let least = match (&gap.found, gap.in_world()) {
            (_, Some((distance, _, _))) => format!("Least distance {}", length(distance, unit)),
            (Some(Err(why)), _) => format!("Least distance unknown: {why}"),
            _ => "Least distance …".to_string(),
        };
        lines.insert(0, least);
        lines
    }
}

/// What the kernel measures the least distance between: the last two
/// picks, when each is a face or an edge of a body, as the body, which of
/// its parts and the picked point in the world.
pub(crate) fn gap_operands(
    picks: &[MeasurePick],
) -> Option<[(core_document::BodyId, MeasuredPart, [f32; 3]); 2]> {
    let operand = |pick: &MeasurePick| match pick {
        MeasurePick::Face {
            point,
            body: Some(body),
            ..
        } => Some((*body, MeasuredPart::Face, *point)),
        MeasurePick::Edge {
            point,
            body: Some(body),
            ..
        } => Some((*body, MeasuredPart::Edge, *point)),
        _ => None,
    };
    let [.., a, b] = picks else {
        return None;
    };
    Some([operand(a)?, operand(b)?])
}

/// How many decimals a length reads with in `unit`: about a hundredth of
/// a millimetre in each.
pub(crate) fn decimals(unit: Unit) -> usize {
    match unit {
        Unit::Mm => 2,
        Unit::Cm | Unit::In => 3,
        Unit::M | Unit::Ft => 5,
    }
}

/// A length in `unit`, read to [`decimals`].
pub(crate) fn length(value_mm: f32, unit: Unit) -> String {
    core_document::format_length_mm(value_mm, unit, decimals(unit))
}

/// An area in `unit`'s square.
fn area(value_mm2: f64, unit: Unit) -> String {
    format_area_mm2(value_mm2, unit, decimals(unit))
}

/// The least distance between two lines, each a point and a direction:
/// square across where they run parallel.
pub(crate) fn line_distance(pa: Vec3, da: Vec3, pb: Vec3, db: Vec3) -> f32 {
    let (da, db) = (da.normalize_or_zero(), db.normalize_or_zero());
    let across = da.cross(db);
    let between = pb - pa;
    if across.length() < 1e-6 {
        return (between - da * between.dot(da)).length();
    }
    between.dot(across.normalize()).abs()
}

impl MeasurePick {
    /// Where it was picked.
    pub fn point(&self) -> [f32; 3] {
        match self {
            MeasurePick::Point(p)
            | MeasurePick::Edge { point: p, .. }
            | MeasurePick::Face { point: p, .. }
            | MeasurePick::Datum { point: p, .. } => *p,
        }
    }

    /// Whether it stands for a point alone: a picked point or a datum point,
    /// which distances are measured from exactly.
    fn is_point(&self) -> bool {
        matches!(
            self,
            MeasurePick::Point(_)
                | MeasurePick::Datum {
                    datum: DatumPick::Point,
                    ..
                }
        )
    }

    /// The way it runs or faces: a straight edge's direction, a circle's
    /// or a round face's axis, a flat face's normal, a datum plane's normal
    /// or a datum line's direction.
    fn direction(&self) -> Option<Vec3> {
        let d = match self {
            MeasurePick::Point(_) => return None,
            MeasurePick::Edge {
                circle: Some(c), ..
            } => c.normal,
            MeasurePick::Edge { direction, .. } => *direction,
            MeasurePick::Face {
                surface, normal, ..
            } => match surface {
                Some(FaceSurface::Plane { normal, .. }) => *normal,
                Some(FaceSurface::Other) | None => *normal,
                Some(turned) => turned.axis()?.1,
            },
            MeasurePick::Datum { datum, .. } => match datum {
                DatumPick::Plane { normal, .. } => *normal,
                DatumPick::Line { direction, .. } => *direction,
                DatumPick::Point => return None,
            },
        };
        let d = Vec3::from_array(d).normalize_or_zero();
        (d != Vec3::ZERO).then_some(d)
    }

    /// The line it turns about or runs along: a round face's axis, a
    /// circular edge's, a datum line.
    fn axis(&self) -> Option<(Vec3, Vec3)> {
        let (point, direction) = match self {
            MeasurePick::Edge {
                circle: Some(c), ..
            } => (c.center, c.normal),
            MeasurePick::Face {
                surface: Some(surface),
                ..
            } => surface.axis()?,
            MeasurePick::Datum {
                datum: DatumPick::Line { origin, direction },
                ..
            } => (*origin, *direction),
            _ => return None,
        };
        Some((Vec3::from_array(point), Vec3::from_array(direction)))
    }

    /// A flat face's plane or a datum plane, as a point on it and its unit
    /// normal.
    fn plane(&self) -> Option<(Vec3, Vec3)> {
        let (origin, normal) = match self {
            MeasurePick::Face {
                surface: Some(FaceSurface::Plane { origin, normal }),
                ..
            }
            | MeasurePick::Datum {
                datum: DatumPick::Plane { origin, normal },
                ..
            } => (origin, normal),
            _ => return None,
        };
        Some((
            Vec3::from_array(*origin),
            Vec3::from_array(*normal).normalize_or_zero(),
        ))
    }

    /// What it measures on its own, one line each.
    pub fn describe(&self, unit: Unit) -> Vec<String> {
        let len = |v: f32| length(v, unit);
        match self {
            MeasurePick::Point(p) => {
                vec![format!("Point {}, {}, {}", len(p[0]), len(p[1]), len(p[2]))]
            }
            MeasurePick::Edge {
                length,
                circle: Some(c),
                ..
            } => vec![
                format!("Radius {}", len(c.radius)),
                format!("Diameter {}", len(2.0 * c.radius)),
                format!("Length {}", len(*length)),
            ],
            MeasurePick::Edge { length, .. } => vec![format!("Length {}", len(*length))],
            MeasurePick::Face {
                surface, area: a, ..
            } => {
                let mut lines = vec![format!("Area {}", area(*a, unit))];
                match surface {
                    Some(FaceSurface::Cylinder { radius, .. })
                    | Some(FaceSurface::Sphere { radius, .. }) => {
                        lines.push(format!("Radius {}", len(*radius)));
                        lines.push(format!("Diameter {}", len(2.0 * radius)));
                    }
                    Some(FaceSurface::Cone { half_angle, .. }) if *half_angle > 0.0 => {
                        let half = half_angle.to_degrees();
                        lines.push(format!("Half angle {half:.2}°"));
                        lines.push(format!("Cone angle {:.2}°", 2.0 * half));
                    }
                    Some(FaceSurface::Torus {
                        major_radius,
                        minor_radius,
                        ..
                    }) if *major_radius > 0.0 => {
                        lines.push(format!("Major radius {}", len(*major_radius)));
                        lines.push(format!("Minor radius {}", len(*minor_radius)));
                        lines.push(format!("Tube diameter {}", len(2.0 * minor_radius)));
                    }
                    _ => {}
                }
                lines
            }
            MeasurePick::Datum {
                name,
                datum: DatumPick::Point,
                point: p,
            } => vec![format!(
                "{name} {}, {}, {}",
                len(p[0]),
                len(p[1]),
                len(p[2])
            )],
            MeasurePick::Datum { name, .. } => vec![name.clone()],
        }
    }
}

/// What two picks measure between them, one line each: the distance
/// (between two parallel planes, square across; from a point to a plane or
/// a line, square to it), and the angle when both have a direction.
pub(crate) fn describe_pair(a: &MeasurePick, b: &MeasurePick, unit: Unit) -> Vec<String> {
    let len = |v: f32| length(v, unit);
    let (pa, pb) = (Vec3::from_array(a.point()), Vec3::from_array(b.point()));
    let mut lines = Vec::new();
    let point_and = |of: &dyn Fn(&MeasurePick) -> Option<(Vec3, Vec3)>| {
        if a.is_point() {
            of(b).map(|line| (pa, line))
        } else if b.is_point() {
            of(a).map(|line| (pb, line))
        } else {
            None
        }
    };
    let parallel = match (a.plane(), b.plane()) {
        (Some((oa, na)), Some((ob, nb))) if na.cross(nb).length() < 1e-4 => {
            Some((ob - oa).dot(na).abs())
        }
        _ => None,
    };
    if let Some(across) = parallel {
        lines.push(format!("Distance {}", len(across)));
    } else if let Some((p, (o, n))) = point_and(&|p: &MeasurePick| p.plane()) {
        lines.push(format!("Distance to plane {}", len((p - o).dot(n).abs())));
    } else if let Some((p, (o, d))) = point_and(&|p: &MeasurePick| p.axis()) {
        let d = d.normalize_or_zero();
        let off = (p - o) - d * (p - o).dot(d);
        lines.push(format!("Distance to axis {}", len(off.length())));
    } else {
        let d = pb - pa;
        lines.push(format!("Distance {}", len(d.length())));
        lines.push(format!(
            "Δx {}  Δy {}  Δz {}",
            len(d.x.abs()),
            len(d.y.abs()),
            len(d.z.abs())
        ));
    }
    if let (Some((pa, da)), Some((pb, db))) = (a.axis(), b.axis()) {
        lines.push(format!(
            "Axis distance {}",
            len(line_distance(pa, da, pb, db))
        ));
    }
    if let (Some(da), Some(db)) = (a.direction(), b.direction()) {
        let angle = da.dot(db).clamp(-1.0, 1.0).acos().to_degrees();
        lines.push(format!("Angle {angle:.2}°"));
    }
    lines
}

/// The unit a document's measurements read in: the one it keeps for the
/// measure tool, else its display unit.
pub(crate) fn readout_unit(document: &core_document::Document) -> Unit {
    document
        .measure_unit()
        .unwrap_or_else(|| document.display_unit())
}

/// What the chain of picks reads, one line each: a lone pick's own
/// measures; then the last pick against the one before it; and with
/// several, the chain's totals: the path through every pick, every edge's
/// length when all are edges, every face's area when all are faces.
pub(crate) fn readout(picks: &[MeasurePick], unit: Unit) -> Vec<String> {
    let mut lines = match picks {
        [] => return Vec::new(),
        [one] => return one.describe(unit),
        [.., a, b] => describe_pair(a, b, unit),
    };
    if picks.len() > 2 {
        let path: f32 = picks
            .windows(2)
            .map(|w| Vec3::from_array(w[0].point()).distance(Vec3::from_array(w[1].point())))
            .sum();
        lines.push(format!(
            "Path through {} picks {}",
            picks.len(),
            length(path, unit)
        ));
    }
    let edges: Option<f32> = picks
        .iter()
        .map(|p| match p {
            MeasurePick::Edge { length, .. } => Some(*length),
            _ => None,
        })
        .sum();
    if let Some(total) = edges {
        lines.push(format!("Total length {}", length(total, unit)));
    }
    let faces: Option<f64> = picks
        .iter()
        .map(|p| match p {
            MeasurePick::Face { area, .. } => Some(*area),
            _ => None,
        })
        .sum();
    if let Some(total) = faces {
        lines.push(format!("Total area {}", area(total, unit)));
    }
    lines
}

/// What a click with the tool armed picks: the edge under the cursor, else
/// the face of the picked body's mesh at the picked point, else the point.
/// The face comes from the body's mesh rather than the hover paint, which is
/// dropped while a button is down and so is gone when the release lands.
pub(crate) fn pick_at(
    edge: Option<MeasurePick>,
    body_mesh: Option<&TriMesh>,
    point: Option<[f32; 3]>,
) -> Option<MeasurePick> {
    if edge.is_some() {
        return edge;
    }
    let point = point?;
    let face = body_mesh.and_then(|mesh| {
        let face = crate::app::input::face_ref_from_mesh(mesh, Vec3::from_array(point))?;
        let (_, sub) = crate::app::input::face_submesh(mesh, 0, None, face.point, face.normal)?;
        Some(MeasurePick::Face {
            point: face.point,
            normal: face.normal,
            surface: face.surface,
            area: mesh_area(&sub),
            body: None,
        })
    });
    Some(face.unwrap_or(MeasurePick::Point(point)))
}

/// The area of a mesh's triangles, mm².
pub(crate) fn mesh_area(mesh: &TriMesh) -> f64 {
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let p = |i: u32| {
                let v = mesh.positions[i as usize];
                glam::DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))
            };
            0.5 * (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).length()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(z: f32, normal: [f32; 3]) -> MeasurePick {
        MeasurePick::Face {
            point: [1.0, 2.0, z],
            normal,
            surface: Some(FaceSurface::Plane {
                origin: [0.0, 0.0, z],
                normal,
            }),
            area: 100.0,
            body: None,
        }
    }

    fn edge(point: [f32; 3], direction: [f32; 3], length: f32) -> MeasurePick {
        MeasurePick::Edge {
            point,
            direction,
            length,
            circle: None,
            body: None,
        }
    }

    /// The pick, as a part of `body`.
    fn on_body(mut pick: MeasurePick, body: core_document::BodyId) -> MeasurePick {
        if let MeasurePick::Edge { body: of, .. } | MeasurePick::Face { body: of, .. } = &mut pick {
            *of = Some(body);
        }
        pick
    }

    /// A 2 x 3 square at z = 1 cut from kernel face 4, beside a unit square
    /// at z = 0 cut from face 5.
    fn two_face_body() -> TriMesh {
        TriMesh {
            positions: vec![
                [0.0, 0.0, 1.0],
                [2.0, 0.0, 1.0],
                [2.0, 3.0, 1.0],
                [0.0, 3.0, 1.0],
                [5.0, 0.0, 0.0],
                [6.0, 0.0, 0.0],
                [6.0, 1.0, 0.0],
                [5.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
            faces: vec![4, 4, 5, 5],
            ..TriMesh::default()
        }
    }

    #[test]
    fn a_click_on_a_body_picks_the_face_there_with_its_area() {
        let mesh = two_face_body();
        let pick = pick_at(None, Some(&mesh), Some([1.0, 1.5, 1.0])).unwrap();
        let MeasurePick::Face { area, normal, .. } = pick else {
            panic!("a face, not {pick:?}");
        };
        assert!((area - 6.0).abs() < 1e-6, "{area}");
        assert!((Vec3::from_array(normal) - Vec3::Z).length() < 1e-5);
        let lines = pick.describe(Unit::Mm);
        assert!(lines[0].starts_with("Area 6.00"), "{lines:?}");
    }

    #[test]
    fn an_edge_takes_the_click_and_empty_space_gives_a_point() {
        let mesh = two_face_body();
        let edge = edge([1.0, 0.0, 1.0], [1.0, 0.0, 0.0], 2.0);
        let picked = pick_at(Some(edge.clone()), Some(&mesh), Some([1.0, 0.0, 1.0]));
        assert_eq!(picked, Some(edge));
        assert_eq!(
            pick_at(None, None, Some([9.0, 9.0, 9.0])),
            Some(MeasurePick::Point([9.0, 9.0, 9.0]))
        );
        assert_eq!(pick_at(None, Some(&mesh), None), None);
    }

    /// The unit chosen in the readout bar is saved with the document, and
    /// a reopened one measures in it.
    #[test]
    fn a_reopened_document_measures_in_the_unit_it_was_saved_with() {
        let mut doc = core_document::Document::new("measured");
        doc.set_display_unit(Unit::Cm);
        assert_eq!(
            readout_unit(&doc),
            Unit::Cm,
            "the display unit until one is chosen"
        );
        doc.set_measure_unit(Some(Unit::In));
        let bytes = doc
            .save_to_bytes(core_document::Compression::None)
            .expect("saves");
        let back = core_document::Document::load_from_bytes(bytes).expect("loads");
        assert_eq!(readout_unit(&back), Unit::In);
        let lines = readout(
            &[edge([0.0; 3], [1.0, 0.0, 0.0], 25.4)],
            readout_unit(&back),
        );
        assert_eq!(lines, ["Length 1.000 in"]);
    }

    #[test]
    fn two_parallel_faces_measure_square_across() {
        let lines = describe_pair(
            &plane(0.0, [0.0, 0.0, 1.0]),
            &plane(7.5, [0.0, 0.0, -1.0]),
            Unit::Mm,
        );
        assert!(lines[0].starts_with("Distance 7.50"), "{lines:?}");
        assert!(lines.iter().any(|l| l == "Angle 180.00°"), "{lines:?}");
    }

    #[test]
    fn two_edges_give_the_angle_between_them() {
        let lines = describe_pair(
            &edge([0.0; 3], [1.0, 0.0, 0.0], 10.0),
            &edge([0.0; 3], [1.0, 1.0, 0.0], 10.0),
            Unit::Mm,
        );
        assert!(lines.iter().any(|l| l == "Angle 45.00°"), "{lines:?}");
    }

    #[test]
    fn a_circle_and_a_bore_give_their_radius() {
        let circle = MeasurePick::Edge {
            point: [0.0; 3],
            direction: [1.0, 0.0, 0.0],
            length: 31.4,
            circle: Some(EdgeCircle {
                center: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                radius: 5.0,
            }),
            body: None,
        };
        assert_eq!(circle.describe(Unit::Mm)[0], "Radius 5.00 mm");
        let bore = face(FaceSurface::Cylinder {
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 3.0,
        });
        let lines = bore.describe(Unit::Mm);
        assert!(lines.iter().any(|l| l == "Diameter 6.00 mm"), "{lines:?}");
        // The circle's axis and the bore's run the same way.
        let pair = describe_pair(&circle, &bore, Unit::Mm);
        assert!(pair.iter().any(|l| l == "Angle 0.00°"), "{pair:?}");
    }

    /// Two bores side by side, or crossing at a slant, measure the least
    /// distance between their axes.
    #[test]
    fn two_bores_measure_between_their_axes() {
        let bore = |origin, axis| {
            face(FaceSurface::Cylinder {
                origin,
                axis,
                radius: 2.0,
            })
        };
        let side_by_side = describe_pair(
            &bore([0.0; 3], [0.0, 0.0, 1.0]),
            &bore([12.0, 5.0, 3.0], [0.0, 0.0, -1.0]),
            Unit::Mm,
        );
        assert!(
            side_by_side.iter().any(|l| l == "Axis distance 13.00 mm"),
            "{side_by_side:?}"
        );
        let crossing = describe_pair(
            &bore([0.0; 3], [1.0, 0.0, 0.0]),
            &bore([4.0, 0.0, 7.0], [0.0, 1.0, 0.0]),
            Unit::Mm,
        );
        assert!(
            crossing.iter().any(|l| l == "Axis distance 7.00 mm"),
            "{crossing:?}"
        );
    }

    /// The kernel's closest points come back in the first body's frame and
    /// are shown where that body sits.
    #[test]
    fn a_least_distance_is_shown_where_the_body_sits() {
        let gap = MeasureGap {
            token: 1,
            first_placement: core_document::BodyPlacement::new(
                glam::Quat::IDENTITY,
                Vec3::new(10.0, 0.0, 0.0),
            ),
            found: Some(Ok((2.5, [0.0; 3], [0.0, 0.0, 2.5]))),
        };
        let (distance, a, b) = gap.in_world().unwrap();
        assert_eq!(distance, 2.5);
        assert_eq!(
            (a, b),
            (Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 2.5))
        );
        let waiting = MeasureGap { found: None, ..gap };
        assert!(waiting.in_world().is_none());
    }

    fn face(surface: FaceSurface) -> MeasurePick {
        MeasurePick::Face {
            point: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
            surface: Some(surface),
            area: 10.0,
            body: None,
        }
    }

    #[test]
    fn a_cone_reads_its_angle_and_a_torus_its_radii() {
        let cone = face(FaceSurface::Cone {
            apex: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            half_angle: 30f32.to_radians(),
        });
        let lines = cone.describe(Unit::Mm);
        assert!(
            lines.contains(&"Half angle 30.00°".to_string()),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"Cone angle 60.00°".to_string()),
            "{lines:?}"
        );
        let torus = face(FaceSurface::Torus {
            center: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            major_radius: 20.0,
            minor_radius: 2.5,
        });
        let lines = torus.describe(Unit::Mm);
        for want in [
            "Major radius 20.00 mm",
            "Minor radius 2.50 mm",
            "Tube diameter 5.00 mm",
        ] {
            assert!(lines.contains(&want.to_string()), "{lines:?}");
        }
    }

    /// A datum plane is picked where the cursor's ray crosses it, and a
    /// point measures square to it; a datum line measures square to its
    /// line.
    #[test]
    fn datums_measure_square_to_their_plane_and_line() {
        let feature = |shape| DatumFeature {
            shape,
            attachment: core_document::DatumAttachment::BasePlane(core_document::BasePlane::XY),
            offset: core_document::AttachmentOffset {
                translation: [0.0, 0.0, 4.0],
                ..Default::default()
            },
        };
        let lifted = core_document::BodyPlacement::new(glam::Quat::IDENTITY, Vec3::Z);
        let (plane, at) = DatumPick::of(
            &feature(DatumShape::Plane { size: 20.0 }),
            lifted,
            Vec3::new(3.0, 2.0, 50.0),
            Vec3::NEG_Z,
        );
        assert_eq!(at, [3.0, 2.0, 5.0]);
        let plane = MeasurePick::Datum {
            name: "Datum Plane".into(),
            datum: plane,
            point: at,
        };
        let lines = describe_pair(&MeasurePick::Point([7.0, 7.0, -2.0]), &plane, Unit::Mm);
        assert_eq!(lines[0], "Distance to plane 7.00 mm", "{lines:?}");

        let (line, _) = DatumPick::of(
            &feature(DatumShape::Line { length: 20.0 }),
            lifted,
            Vec3::new(3.0, 2.0, 50.0),
            Vec3::NEG_Z,
        );
        let line = MeasurePick::Datum {
            name: "Datum Line".into(),
            datum: line,
            point: [0.0, 0.0, 5.0],
        };
        // The line runs along X through (0, 0, 5).
        let lines = describe_pair(&line, &MeasurePick::Point([9.0, 3.0, 9.0]), Unit::Mm);
        assert_eq!(lines[0], "Distance to axis 5.00 mm", "{lines:?}");
    }

    /// Picks run on: the last two are measured against each other and the
    /// chain adds up its path, its edges and its faces.
    #[test]
    fn a_chain_of_picks_adds_up() {
        let edge = |x: f32, length: f32| edge([x, 0.0, 0.0], [0.0, 1.0, 0.0], length);
        let lines = readout(&[edge(0.0, 4.0), edge(3.0, 5.0), edge(7.0, 6.0)], Unit::Mm);
        assert_eq!(lines[0], "Distance 4.00 mm", "the last two: {lines:?}");
        assert!(lines.contains(&"Path through 3 picks 7.00 mm".to_string()));
        assert!(lines.contains(&"Total length 15.00 mm".to_string()));
        assert!(lines.iter().all(|l| !l.starts_with("Total area")));

        let faces = [plane(0.0, [0.0, 0.0, 1.0]), plane(2.0, [0.0, 0.0, 1.0])];
        let lines = readout(&faces, Unit::Cm);
        assert!(
            lines.contains(&"Total area 2.000 cm²".to_string()),
            "{lines:?}"
        );
        assert_eq!(readout(&faces[..1], Unit::Mm), faces[0].describe(Unit::Mm));
        assert!(readout(&[], Unit::Mm).is_empty());
    }

    /// The kernel is asked between the last two picks, when both are faces
    /// or edges of a body.
    #[test]
    fn the_least_distance_is_asked_between_the_last_two_parts() {
        let body = core_document::BodyId(uuid::Uuid::from_u128(7));
        let edge = on_body(edge([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 3.0), body);
        let face = on_body(plane(2.0, [0.0, 0.0, 1.0]), body);
        let point = MeasurePick::Point([0.0; 3]);
        assert_eq!(
            gap_operands(&[point.clone(), edge.clone(), face.clone()]),
            Some([
                (body, MeasuredPart::Edge, [1.0, 0.0, 0.0]),
                (body, MeasuredPart::Face, [1.0, 2.0, 2.0]),
            ])
        );
        assert_eq!(gap_operands(&[edge.clone(), face, point]), None);
        assert_eq!(gap_operands(&[edge]), None);
        let loose = plane(0.0, [0.0, 0.0, 1.0]);
        assert_eq!(gap_operands(&[loose.clone(), loose]), None, "no body");
    }

    /// The quick measure reads one edge's length and one face's area; of
    /// two edges, the kernel's least distance stands in for the distance
    /// between their midpoints, and the angle stays.
    #[test]
    fn the_quick_measure_reads_the_selection() {
        let body = core_document::BodyId(uuid::Uuid::from_u128(7));
        let edge = |x: f32, direction| on_body(edge([x, 0.0, 0.0], direction, 3.0), body);
        let one = QuickMeasure {
            picks: vec![edge(0.0, [0.0, 1.0, 0.0])],
            gap: None,
        };
        assert_eq!(one.lines(Unit::Mm), ["Length 3.00 mm"]);
        let face = QuickMeasure {
            picks: vec![plane(0.0, [0.0, 0.0, 1.0])],
            gap: None,
        };
        assert_eq!(face.lines(Unit::Mm), ["Area 100.00 mm²"]);

        let mut two = QuickMeasure {
            picks: vec![edge(0.0, [0.0, 1.0, 0.0]), edge(4.0, [0.0, 0.0, 1.0])],
            gap: Some(MeasureGap {
                token: 1,
                first_placement: core_document::BodyPlacement::default(),
                found: None,
            }),
        };
        let lines = two.lines(Unit::Mm);
        assert_eq!(lines[0], "Least distance …");
        assert!(
            lines
                .iter()
                .all(|l| !l.starts_with("Distance") && !l.starts_with("Δx")),
            "{lines:?}"
        );
        assert!(lines.contains(&"Angle 90.00°".to_string()), "{lines:?}");
        if let Some(gap) = &mut two.gap {
            gap.found = Some(Ok((4.0, [0.0; 3], [4.0, 0.0, 0.0])));
        }
        assert_eq!(two.lines(Unit::Cm)[0], "Least distance 0.400 cm");
    }
}
