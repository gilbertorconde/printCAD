//! Parametric primitives. Full solids of revolution go through the exact
//! primitive builders; partial-angle forms fall back to synthetic
//! profile-and-revolve constructions, mirroring the previous kernel's
//! three-angle parameterizations.

use kernel_api::{Placement, PrimitiveKind, Profile, ProfilePlane, ProfileSegment, ProfileWire};
use ogeom::algo::{
    general_transformed_shape, make_box, make_cone, make_cylinder, make_face, make_polygon,
    make_prism, make_sphere, make_torus, transformed,
};
use ogeom::geom::{PlaneSurface, SurfaceGeometry};
use ogeom::math::{Direction, Frame, GeneralTransform, Matrix3, Plane, Point, Transform, Vector};
use ogeom::offset::make_loft;
use ogeom::topo::{Model, Shape, VertexData};

use super::tol;
use crate::ops::sweep::build_tool as sweep_tool;

const FULL_EPS: f64 = 1e-3;

pub fn build_tool(
    model: &mut Model,
    kind: &PrimitiveKind,
    placement: &Placement,
) -> Result<Shape, String> {
    let frame = placement_frame(placement)?;
    match kind {
        PrimitiveKind::Box {
            length,
            width,
            height,
        } => make_box(model, frame, (*length, *width, *height), tol())
            .map(|b| b.shape)
            .map_err(|e| format!("box: {e}")),

        PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg,
        } => {
            if is_full(*angle_deg) {
                make_cylinder(model, frame, *radius, *height, tol())
                    .map(|b| b.shape)
                    .map_err(|e| format!("cylinder: {e}"))
            } else {
                // Rectangle radius × height beside the axis, revolved.
                let wire = vec![
                    ProfileSegment::Line {
                        start: [0.0, 0.0],
                        end: [*radius, 0.0],
                    },
                    ProfileSegment::Line {
                        start: [*radius, 0.0],
                        end: [*radius, *height],
                    },
                    ProfileSegment::Line {
                        start: [*radius, *height],
                        end: [0.0, *height],
                    },
                    ProfileSegment::Line {
                        start: [0.0, *height],
                        end: [0.0, 0.0],
                    },
                ];
                revolve_synthetic(model, placement, wire, *angle_deg)
            }
        }

        PrimitiveKind::Cone {
            radius1,
            radius2,
            height,
            angle_deg,
        } => {
            if is_full(*angle_deg) {
                if (radius1 - radius2).abs() <= 1e-9 {
                    make_cylinder(model, frame, *radius1, *height, tol())
                        .map(|b| b.shape)
                        .map_err(|e| format!("cone (equal radii): {e}"))
                } else {
                    make_cone(model, frame, *radius1, *radius2, *height, tol())
                        .map(|b| b.shape)
                        .map_err(|e| format!("cone: {e}"))
                }
            } else {
                let mut pts: Vec<[f64; 2]> = vec![[0.0, 0.0]];
                if *radius1 > 1e-9 {
                    pts.push([*radius1, 0.0]);
                }
                if *radius2 > 1e-9 {
                    pts.push([*radius2, *height]);
                }
                pts.push([0.0, *height]);
                let wire: Vec<ProfileSegment> = pts
                    .iter()
                    .zip(pts.iter().cycle().skip(1))
                    .map(|(a, b)| ProfileSegment::Line { start: *a, end: *b })
                    .collect();
                revolve_synthetic(model, placement, wire, *angle_deg)
            }
        }

        PrimitiveKind::Sphere {
            radius,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            let full_lat = *angle1_deg <= -90.0 + FULL_EPS && *angle2_deg >= 90.0 - FULL_EPS;
            if full_lat && is_full(*angle3_deg) {
                make_sphere(model, frame, *radius, tol())
                    .map(|b| b.shape)
                    .map_err(|e| format!("sphere: {e}"))
            } else {
                let (a1, a2) = (angle1_deg.to_radians(), angle2_deg.to_radians());
                if a2 <= a1 {
                    return Err("sphere latitude range is empty".into());
                }
                let p = |a: f64| [radius * a.cos(), radius * a.sin()];
                let mut wire = Vec::new();
                let (p1, p2) = (p(a1), p(a2));
                let on_axis = |pt: [f64; 2]| pt[0].abs() <= 1e-9;
                if !on_axis(p1) {
                    wire.push(ProfileSegment::Line {
                        start: [0.0, 0.0],
                        end: p1,
                    });
                }
                wire.push(ProfileSegment::Arc {
                    start: p1,
                    mid: p((a1 + a2) * 0.5),
                    end: p2,
                });
                if !on_axis(p2) {
                    wire.push(ProfileSegment::Line {
                        start: p2,
                        end: [0.0, 0.0],
                    });
                }
                if on_axis(p1) && on_axis(p2) {
                    // Full meridian: close pole to pole along the axis.
                    wire.push(ProfileSegment::Line { start: p2, end: p1 });
                }
                revolve_synthetic(model, placement, wire, angle_deg3_or_full(*angle3_deg))
            }
        }

        PrimitiveKind::Torus {
            radius1,
            radius2,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            let full_tube = *angle2_deg - *angle1_deg >= 360.0 - FULL_EPS
                || (*angle1_deg == 0.0 && *angle2_deg == 0.0);
            if full_tube && is_full(*angle3_deg) {
                make_torus(model, frame, *radius1, *radius2, tol())
                    .map(|b| b.shape)
                    .map_err(|e| format!("torus: {e}"))
            } else {
                let center = [*radius1, 0.0];
                let mut wire = Vec::new();
                if full_tube {
                    wire.push(ProfileSegment::Circle {
                        center,
                        radius: *radius2,
                    });
                } else {
                    let (a1, a2) = (angle1_deg.to_radians(), angle2_deg.to_radians());
                    if a2 <= a1 {
                        return Err("torus tube angle range is empty".into());
                    }
                    let p = |a: f64| [center[0] + radius2 * a.cos(), center[1] + radius2 * a.sin()];
                    wire.push(ProfileSegment::Line {
                        start: center,
                        end: p(a1),
                    });
                    wire.push(ProfileSegment::Arc {
                        start: p(a1),
                        mid: p((a1 + a2) * 0.5),
                        end: p(a2),
                    });
                    wire.push(ProfileSegment::Line {
                        start: p(a2),
                        end: center,
                    });
                }
                revolve_synthetic(model, placement, wire, angle_deg3_or_full(*angle3_deg))
            }
        }

        PrimitiveKind::Ellipsoid {
            radius1,
            radius2,
            radius3,
        } => {
            let unit = make_sphere(model, Frame::WORLD, 1.0, tol())
                .map_err(|e| format!("ellipsoid sphere: {e}"))?
                .shape;
            let scale = GeneralTransform {
                linear: Matrix3::from_columns(
                    Vector::new(*radius1, 0.0, 0.0),
                    Vector::new(0.0, *radius2, 0.0),
                    Vector::new(0.0, 0.0, *radius3),
                ),
                translation: Vector::new(0.0, 0.0, 0.0),
            };
            let scaled = general_transformed_shape(model, &unit, &scale, tol())
                .map_err(|e| format!("ellipsoid scaling: {e}"))?
                .shape;
            let place = Transform::from_frame(&frame);
            transformed(model, &scaled, place)
                .map(|b| b.shape)
                .map_err(|e| format!("ellipsoid placement: {e}"))
        }

        PrimitiveKind::Prism {
            sides,
            circumradius,
            height,
        } => {
            if *sides < 3 {
                return Err("prism needs at least 3 sides".into());
            }
            let n = *sides as usize;
            let pts: Vec<Point> = (0..n)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    frame.origin()
                        + frame.x().vector() * (circumradius * a.cos())
                        + frame.y().vector() * (circumradius * a.sin())
                })
                .collect();
            let wire = make_polygon(model, &pts, true, tol())
                .map_err(|e| format!("prism polygon: {e}"))?
                .shape;
            let extent = circumradius + height + 10.0;
            let surface =
                PlaneSurface::over(Plane::new(frame), (-extent, extent), (-extent, extent))
                    .map_err(|e| format!("prism plane: {e}"))?;
            let face = make_face(
                model,
                SurfaceGeometry::Plane(surface),
                std::slice::from_ref(&wire),
                tol(),
            )
            .map_err(|e| format!("prism face: {e}"))?
            .shape;
            make_prism(model, &face, frame.z().vector() * *height, tol())
                .map(|b| b.shape)
                .map_err(|e| format!("prism extrusion: {e}"))
        }

        PrimitiveKind::Wedge {
            xmin,
            xmax,
            ymin,
            ymax,
            zmin,
            zmax,
            x2min,
            x2max,
            z2min,
            z2max,
        } => {
            let at = |x: f64, y: f64, z: f64| {
                frame.origin()
                    + frame.x().vector() * x
                    + frame.y().vector() * y
                    + frame.z().vector() * z
            };
            let bottom = [
                at(*xmin, *ymin, *zmin),
                at(*xmax, *ymin, *zmin),
                at(*xmax, *ymin, *zmax),
                at(*xmin, *ymin, *zmax),
            ];
            let bottom_wire = make_polygon(model, &bottom, true, tol())
                .map_err(|e| format!("wedge base polygon: {e}"))?
                .shape;
            let top_degenerate = (x2max - x2min).abs() <= 1e-9 && (z2max - z2min).abs() <= 1e-9;
            if top_degenerate {
                let apex = at((x2min + x2max) * 0.5, *ymax, (z2min + z2max) * 0.5);
                let vertex = model.add_vertex(VertexData::new(apex));
                make_loft(model, &bottom_wire, &vertex, tol())
                    .map(|b| b.shape)
                    .map_err(|e| format!("wedge apex loft: {e}"))
            } else {
                let top = [
                    at(*x2min, *ymax, *z2min),
                    at(*x2max, *ymax, *z2min),
                    at(*x2max, *ymax, *z2max),
                    at(*x2min, *ymax, *z2max),
                ];
                let top_wire = make_polygon(model, &top, true, tol())
                    .map_err(|e| format!("wedge top polygon: {e}"))?
                    .shape;
                make_loft(model, &bottom_wire, &top_wire, tol())
                    .map(|b| b.shape)
                    .map_err(|e| format!("wedge loft: {e}"))
            }
        }
    }
}

fn is_full(angle_deg: f64) -> bool {
    angle_deg >= 360.0 - FULL_EPS || angle_deg == 0.0
}

fn angle_deg3_or_full(angle_deg: f64) -> f64 {
    if angle_deg == 0.0 {
        360.0
    } else {
        angle_deg
    }
}

fn placement_frame(placement: &Placement) -> Result<Frame, String> {
    let origin = Point::new(
        placement.origin[0],
        placement.origin[1],
        placement.origin[2],
    );
    let z = Direction::new(
        Vector::new(
            placement.z_axis[0],
            placement.z_axis[1],
            placement.z_axis[2],
        ),
        tol(),
    )
    .map_err(|_| "placement z-axis is (near) zero".to_string())?;
    let x = Direction::new(
        Vector::new(
            placement.x_axis[0],
            placement.x_axis[1],
            placement.x_axis[2],
        ),
        tol(),
    )
    .map_err(|_| "placement x-axis is (near) zero".to_string())?;
    Frame::new(origin, z, x, tol()).map_err(|e| format!("placement frame: {e}"))
}

/// Revolve a synthetic 2D profile (in the placement's xz half-plane, v along
/// the placement z-axis) about the placement axis by `angle_deg`, using the
/// shared sweep machinery.
fn revolve_synthetic(
    model: &mut Model,
    placement: &Placement,
    segments: Vec<ProfileSegment>,
    angle_deg: f64,
) -> Result<Shape, String> {
    let frame = placement_frame(placement)?;
    // uv basis: u along frame.x, v along frame.z → normal = u×v = -frame.y.
    let x = frame.x().vector();
    let z = frame.z().vector();
    let n = -frame.y().vector();
    let plane = ProfilePlane {
        origin: [frame.origin().x, frame.origin().y, frame.origin().z],
        x_axis: [x.x, x.y, x.z],
        y_axis: [z.x, z.y, z.z],
        normal: [n.x, n.y, n.z],
    };
    let profile = Profile {
        plane,
        wires: vec![ProfileWire { segments }],
    };
    let kind = kernel_api::SweepKind::Revolve {
        axis_origin: [0.0, 0.0],
        axis_dir: [0.0, 1.0],
        angle_deg: angle_deg.min(360.0),
        second_angle_deg: None,
        midplane: false,
        reversed: false,
    };
    sweep_tool(model, None, &profile, &kind)
}
