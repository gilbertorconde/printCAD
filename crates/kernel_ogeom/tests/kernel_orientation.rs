//! A sweep's shell must face out whichever way its profile was walked.

use ogeom::algo::{
    face_normal, make_edge_between, make_face_with_pcurves, make_prism, volume_properties,
};
use ogeom::core::{OgeomResult, Tolerances};
use ogeom::geom::{Curve, Curve3d, LineCurve, PlaneSurface, SurfaceGeometry};
use ogeom::math::{Direction, Frame, Plane, Point, Vector};
use ogeom::mesh::Deflection;
use ogeom::topo::{Filter, Model, ShapeType, VertexData, explore};

/// A box 20 × 20 × 10 swept from a square on the XY plane walked in the
/// given sense about +Z, as (volume, faces facing away from the centre,
/// faces in all).
fn prism_from_square(clockwise: bool) -> OgeomResult<(f64, usize, usize)> {
    let tol = Tolerances::millimetres();
    let mut model = Model::new();
    let mut corners = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
    if clockwise {
        corners.reverse();
    }
    let points: Vec<Point> = corners
        .iter()
        .map(|(x, y)| Point::new(*x, *y, 0.0))
        .collect();
    let vertices: Vec<_> = points
        .iter()
        .map(|p| model.add_vertex(VertexData::new(*p)))
        .collect();
    let mut edges = Vec::new();
    for i in 0..4 {
        let j = (i + 1) % 4;
        let curve = LineCurve::segment(points[i], points[j], tol).unwrap();
        let range = curve.domain();
        let edge = make_edge_between(
            &mut model,
            Curve::Line(curve),
            range,
            &vertices[i],
            &vertices[j],
            tol,
        )
        .unwrap()
        .shape;
        edges.push(edge);
    }
    let plane = Plane::new(
        Frame::new(
            Point::new(0.0, 0.0, 0.0),
            Direction::new(Vector::new(0.0, 0.0, 1.0), tol).unwrap(),
            Direction::new(Vector::new(1.0, 0.0, 0.0), tol).unwrap(),
            tol,
        )
        .unwrap(),
    );
    let surface = PlaneSurface::over(plane, (-1.0, 21.0), (-1.0, 21.0)).unwrap();
    let face = make_face_with_pcurves(&mut model, SurfaceGeometry::Plane(surface), &[edges], tol)
        .unwrap()
        .shape;
    let solid = make_prism(&mut model, &face, Vector::new(0.0, 0.0, 10.0), tol)?.shape;

    let volume = volume_properties(&model, &solid, Deflection::default(), tol)?.mass;
    let centre = Point::new(10.0, 10.0, 5.0);
    let faces = explore(&model, &solid, Filter::OfType(ShapeType::Face))?;
    let mut outward = 0;
    for face in &faces {
        let (at, normal) = face_normal(&model, face, tol)?;
        if normal.dot(at - centre) > 0.0 {
            outward += 1;
        }
    }
    Ok((volume, outward, faces.len()))
}

#[test]
fn a_prism_from_a_counter_clockwise_square_faces_out_everywhere() {
    let (volume, outward, faces) = prism_from_square(false).expect("a box");
    assert_eq!(faces, 6);
    assert_eq!(outward, 6, "every face presents away from the centre");
    assert!((volume - 4000.0).abs() < 1e-3, "volume {volume}");
}

/// The same square walked the other way is the same region of the plane,
/// so the same box must come out of the sweep.
#[test]
#[ignore = "kernel: make_prism from a clockwise-walked planar face builds an inside-out shell, which volume_properties refuses as wound inward (ogeom-rs#42)"]
fn a_prism_from_a_clockwise_square_faces_out_everywhere() {
    let (volume, outward, faces) =
        prism_from_square(true).expect("the same box, whichever way its profile was walked");
    assert_eq!(faces, 6);
    assert_eq!(outward, 6, "every face presents away from the centre");
    assert!((volume - 4000.0).abs() < 1e-3, "volume {volume}");
}
