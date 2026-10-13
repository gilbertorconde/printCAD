//! A model for the tests that pick its edges: a 10 mm block's bottom
//! outline, from (20, 0) to (30, 10), and a round boss's rim, radius 3 at
//! (40, 5), all 5 mm under the XY plane. Its kernel projects whichever
//! of those edges is nearest the probe square onto the plane asked.

use core_document::{BodyId, Document, EdgeRef};
use kernel_api::{KernelQueries, KernelResult, ProfilePlane, ProjectedEdge};

/// The edges' depth under the XY plane, mm.
pub const DEPTH: f32 = -5.0;

enum Edge {
    Segment([f64; 3], [f64; 3]),
    Circle([f64; 3], f64),
}

fn edges() -> Vec<Edge> {
    let z = f64::from(DEPTH);
    let corner = |x: f64, y: f64| [x, y, z];
    vec![
        Edge::Segment(corner(20.0, 0.0), corner(30.0, 0.0)),
        Edge::Segment(corner(30.0, 0.0), corner(30.0, 10.0)),
        Edge::Segment(corner(30.0, 10.0), corner(20.0, 10.0)),
        Edge::Segment(corner(20.0, 10.0), corner(20.0, 0.0)),
        Edge::Circle(corner(40.0, 5.0), 3.0),
    ]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn distance(edge: &Edge, p: [f64; 3]) -> f64 {
    match edge {
        Edge::Segment(a, b) => {
            let along = sub(*b, *a);
            let t = (dot(sub(p, *a), along) / dot(along, along)).clamp(0.0, 1.0);
            length(sub(
                p,
                [
                    a[0] + along[0] * t,
                    a[1] + along[1] * t,
                    a[2] + along[2] * t,
                ],
            ))
        }
        Edge::Circle(c, r) => {
            let d = sub(p, *c);
            let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
            ((flat - r).powi(2) + d[2] * d[2]).sqrt()
        }
    }
}

pub struct BlockKernel;

impl KernelQueries for BlockKernel {
    fn project_edge(
        &self,
        _brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<ProjectedEdge> {
        let on = |p: [f64; 3]| {
            let d = sub(p, plane.origin);
            [dot(d, plane.x_axis), dot(d, plane.y_axis)]
        };
        let edge = edges()
            .into_iter()
            .min_by(|a, b| distance(a, near).total_cmp(&distance(b, near)))
            .expect("the block has edges");
        Ok(match edge {
            Edge::Segment(a, b) => ProjectedEdge::Line {
                start: on(a),
                end: on(b),
            },
            Edge::Circle(c, radius) => ProjectedEdge::Circle {
                centre: on(c),
                radius,
                range: (0.0, std::f64::consts::TAU),
            },
        })
    }
}

pub static BLOCK: BlockKernel = BlockKernel;

/// A body standing for the block in `doc`, with a shape to project from.
pub fn add_block(doc: &mut Document) -> BodyId {
    let body = doc.create_body(None);
    doc.set_imported_brep_data(body, b"ogeom block".to_vec(), Vec::new());
    body
}

/// The edge the host picks at `point` (on the edge, in world space),
/// running along `direction` there.
pub fn edge_at(body: BodyId, point: [f32; 3], direction: [f32; 3]) -> EdgeRef {
    EdgeRef {
        point,
        direction,
        length_mm: 10.0,
        body: body.0,
        circle: None,
        faces: [0, 0],
    }
}
