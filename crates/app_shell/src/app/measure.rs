//! The measure tool's picks and what they measure: a point, an edge (its
//! length, a circle's radius) or a face (its area, a round face's radius);
//! two picks give the distance between them and, where both have a
//! direction, the angle.

use core_document::{EdgeCircle, Unit, format_area_mm2, format_length_mm};
use glam::Vec3;
use kernel_api::{FaceSurface, TriMesh};

/// One thing the measure tool picked, in world space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MeasurePick {
    Point([f32; 3]),
    Edge {
        point: [f32; 3],
        direction: [f32; 3],
        length: f32,
        circle: Option<EdgeCircle>,
    },
    Face {
        point: [f32; 3],
        normal: [f32; 3],
        surface: Option<FaceSurface>,
        area: f64,
    },
}

impl MeasurePick {
    /// Where it was picked.
    pub fn point(&self) -> [f32; 3] {
        match self {
            MeasurePick::Point(p)
            | MeasurePick::Edge { point: p, .. }
            | MeasurePick::Face { point: p, .. } => *p,
        }
    }

    /// The way it runs or faces: a straight edge's direction, a circle's
    /// or a round face's axis, a flat face's normal.
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
        };
        let d = Vec3::from_array(d).normalize_or_zero();
        (d != Vec3::ZERO).then_some(d)
    }

    /// A flat face's plane, as a point on it and its unit normal.
    fn plane(&self) -> Option<(Vec3, Vec3)> {
        match self {
            MeasurePick::Face {
                surface: Some(FaceSurface::Plane { origin, normal }),
                ..
            } => Some((
                Vec3::from_array(*origin),
                Vec3::from_array(*normal).normalize_or_zero(),
            )),
            _ => None,
        }
    }

    /// What it measures on its own, one line each.
    pub fn describe(&self, unit: Unit) -> Vec<String> {
        let len = |v: f32| format_length_mm(v, unit, 2);
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
            MeasurePick::Face { surface, area, .. } => {
                let mut lines = vec![format!("Area {}", format_area_mm2(*area, unit, 2))];
                match surface {
                    Some(FaceSurface::Cylinder { radius, .. })
                    | Some(FaceSurface::Sphere { radius, .. }) => {
                        lines.push(format!("Radius {}", len(*radius)));
                        lines.push(format!("Diameter {}", len(2.0 * radius)));
                    }
                    _ => {}
                }
                lines
            }
        }
    }
}

/// What two picks measure between them, one line each: the distance
/// (between two parallel flat faces, square across), and the angle when
/// both have a direction.
pub(crate) fn describe_pair(a: &MeasurePick, b: &MeasurePick, unit: Unit) -> Vec<String> {
    let len = |v: f32| format_length_mm(v, unit, 2);
    let (pa, pb) = (Vec3::from_array(a.point()), Vec3::from_array(b.point()));
    let mut lines = Vec::new();
    match (a.plane(), b.plane()) {
        (Some((oa, na)), Some((ob, nb))) if na.cross(nb).length() < 1e-4 => {
            lines.push(format!("Distance {}", len((ob - oa).dot(na).abs())));
        }
        _ => {
            let d = pb - pa;
            lines.push(format!("Distance {}", len(d.length())));
            lines.push(format!(
                "Δx {}  Δy {}  Δz {}",
                len(d.x.abs()),
                len(d.y.abs()),
                len(d.z.abs())
            ));
        }
    }
    if let (Some(da), Some(db)) = (a.direction(), b.direction()) {
        let angle = da.dot(db).clamp(-1.0, 1.0).acos().to_degrees();
        lines.push(format!("Angle {angle:.2}°"));
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
        let sub = crate::app::input::face_submesh(mesh, face.point, face.normal)?;
        Some(MeasurePick::Face {
            point: face.point,
            normal: face.normal,
            surface: face.surface,
            area: mesh_area(&sub),
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
        }
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
        let edge = MeasurePick::Edge {
            point: [1.0, 0.0, 1.0],
            direction: [1.0, 0.0, 0.0],
            length: 2.0,
            circle: None,
        };
        let picked = pick_at(Some(edge.clone()), Some(&mesh), Some([1.0, 0.0, 1.0]));
        assert_eq!(picked, Some(edge));
        assert_eq!(
            pick_at(None, None, Some([9.0, 9.0, 9.0])),
            Some(MeasurePick::Point([9.0, 9.0, 9.0]))
        );
        assert_eq!(pick_at(None, Some(&mesh), None), None);
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
        let edge = |direction| MeasurePick::Edge {
            point: [0.0; 3],
            direction,
            length: 10.0,
            circle: None,
        };
        let lines = describe_pair(&edge([1.0, 0.0, 0.0]), &edge([1.0, 1.0, 0.0]), Unit::Mm);
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
        };
        assert_eq!(circle.describe(Unit::Mm)[0], "Radius 5.00 mm");
        let bore = MeasurePick::Face {
            point: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
            surface: Some(FaceSurface::Cylinder {
                origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius: 3.0,
            }),
            area: 12.0,
        };
        let lines = bore.describe(Unit::Mm);
        assert!(lines.iter().any(|l| l == "Diameter 6.00 mm"), "{lines:?}");
        // The circle's axis and the bore's run the same way.
        let pair = describe_pair(&circle, &bore, Unit::Mm);
        assert!(pair.iter().any(|l| l == "Angle 0.00°"), "{pair:?}");
    }

    #[test]
    fn a_square_of_two_triangles_has_its_area() {
        let mesh = TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 3.0, 0.0],
                [0.0, 3.0, 0.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..TriMesh::default()
        };
        assert!((mesh_area(&mesh) - 6.0).abs() < 1e-9);
    }
}
