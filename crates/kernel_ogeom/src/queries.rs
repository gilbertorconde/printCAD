//! The kernel's answers to the questions a workbench asks while it runs
//! (`kernel_api::KernelQueries`).

use kernel_api::{KernelError, KernelQueries, KernelResult, ProfilePlane, ProjectedEdge};
use ogeom::algo::{ProjectedCurve, project_edge_onto_plane};
use ogeom::geom::Curve2d as _;
use ogeom::math::{Direction, Frame, Plane, Point, Vector};
use ogeom::topo::ShapeType;

use crate::ops::dressup::nearest_of;
use crate::tess;

/// The ogeom kernel's answers. It holds nothing, so one serves every caller.
pub struct OgeomQueries;

/// The instance the application hands its workbenches.
pub static QUERIES: OgeomQueries = OgeomQueries;

/// Points taken along a curve with no closed form in a sketch.
const POLYLINE_POINTS: usize = 64;

fn other(message: impl std::fmt::Display) -> KernelError {
    KernelError::Other(anyhow::anyhow!("{message}"))
}

impl KernelQueries for OgeomQueries {
    fn project_edge(
        &self,
        brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<ProjectedEdge> {
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let edge = nearest_of(
            &mut model,
            &root,
            ShapeType::Edge,
            Point::new(near[0], near[1], near[2]),
        )
        .map_err(other)?;
        let direction = |v: [f64; 3]| Direction::new(Vector::new(v[0], v[1], v[2]), tol);
        let frame = Frame::from_axes(
            Point::new(plane.origin[0], plane.origin[1], plane.origin[2]),
            direction(plane.x_axis).map_err(other)?,
            direction(plane.y_axis).map_err(other)?,
            direction(plane.normal).map_err(other)?,
            tol,
        )
        .map_err(other)?;
        let projected =
            project_edge_onto_plane(&model, &edge, &Plane::new(frame), tol).map_err(other)?;
        Ok(match projected {
            ProjectedCurve::Point(p) => ProjectedEdge::Point([p.x, p.y]),
            ProjectedCurve::Line { start, end } => ProjectedEdge::Line {
                start: [start.x, start.y],
                end: [end.x, end.y],
            },
            ProjectedCurve::Circle {
                centre,
                radius,
                range,
            } => ProjectedEdge::Circle {
                centre: [centre.x, centre.y],
                radius,
                range,
            },
            ProjectedCurve::Ellipse {
                centre,
                major,
                ratio,
                range,
            } => ProjectedEdge::Ellipse {
                centre: [centre.x, centre.y],
                major: [major.x, major.y],
                ratio,
                range,
            },
            ProjectedCurve::BSpline { curve, .. } => {
                let (a, b) = curve.domain();
                let points = (0..=POLYLINE_POINTS)
                    .map(|i| {
                        let t = a + (b - a) * i as f64 / POLYLINE_POINTS as f64;
                        curve.point_at(t, tol).map(|p| [p.x, p.y]).map_err(other)
                    })
                    .collect::<KernelResult<Vec<_>>>()?;
                ProjectedEdge::Polyline(points)
            }
        })
    }
}
