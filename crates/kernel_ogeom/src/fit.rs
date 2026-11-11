//! Shapes fitted to measured points: the kernel's least-squares fits
//! (`ogeom::algo::fit`), asked in the contract's terms and answered in
//! them, and regions grown over a mesh from a clicked point.

use kernel_api::{
    FitAnswer, FitKind, FitMethod, FitRequest, FittedShape, GrowAnswer, GrowRequest, KernelError,
    KernelResult, TriMesh,
};
use ogeom::algo::Cutoff;
use ogeom::algo::fit::{self, AnyFit, Fit, FitFixed, FitKind as GrowKind, FitOptions, GrowOptions};
use ogeom::math::{Axis, Circle, Cone, Cylinder, Plane, Point, Sphere, Torus, Vector};
use ogeom::mesh::Bvh;

use crate::convert::{array, point, vector};
use crate::mesh_intake::{Degenerate, Weld};
use crate::tess;

/// Fit `request.kind` to its points.
pub(crate) fn fit(request: &FitRequest) -> KernelResult<FitAnswer> {
    let tol = tess::tolerances();
    let points: Vec<Point> = request.points.iter().map(point).collect();
    let normals: Vec<Vector> = request.normals.iter().map(vector).collect();
    let normals = (normals.len() == points.len()).then_some(normals.as_slice());
    let cutoff = request.sigma_cutoff.map_or(Cutoff::All, Cutoff::Sigma);
    let fixed = FitFixed {
        radius: request.fixed_radius,
        ..FitFixed::default()
    };
    macro_rules! options {
        () => {
            FitOptions {
                normals,
                cutoff,
                robust_start: true,
                fixed: fixed.clone(),
                ..FitOptions::default()
            }
        };
    }
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    Ok(match request.kind {
        FitKind::Plane => answer(
            fit::plane(&points, &options!(), tol).map_err(failed)?,
            plane_shape,
        ),
        FitKind::Sphere => answer(
            fit::sphere(&points, &options!(), tol).map_err(failed)?,
            sphere_shape,
        ),
        FitKind::Cylinder => answer(
            fit::cylinder(&points, &options!(), tol).map_err(failed)?,
            cylinder_shape,
        ),
        FitKind::Cone => answer(
            fit::cone(&points, &options!(), tol).map_err(failed)?,
            cone_shape,
        ),
        FitKind::Torus => answer(
            fit::torus(&points, &options!(), tol).map_err(failed)?,
            torus_shape,
        ),
        FitKind::Circle => answer(
            fit::circle3(&points, &options!(), tol).map_err(failed)?,
            circle_shape,
        ),
        FitKind::Line => answer(
            fit::line3(&points, &options!(), tol).map_err(failed)?,
            line_shape,
        ),
    })
}

/// `kind` fitted to every one of `points` by `method`.
pub(crate) fn zone_fit(
    kind: FitKind,
    method: FitMethod,
    points: &[[f64; 3]],
) -> KernelResult<FitAnswer> {
    use ogeom::algo::fit::zone;
    let tol = tess::tolerances();
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let pts: Vec<Point> = points.iter().map(point).collect();
    let refused = || {
        KernelError::InvalidInput(format!(
            "a {kind:?} is not fitted {}",
            match method {
                FitMethod::LeastSquares => "by least squares here",
                FitMethod::MinimumZone => "to a minimum zone",
                FitMethod::Inscribed => "inscribed",
                FitMethod::Circumscribed => "circumscribed",
            }
        ))
    };
    let (shape, optimal) = match (kind, method) {
        (_, FitMethod::LeastSquares) => {
            let request = FitRequest {
                kind,
                points: points.to_vec(),
                normals: Vec::new(),
                sigma_cutoff: None,
                fixed_radius: None,
            };
            return fit(&request);
        }
        (FitKind::Plane, FitMethod::MinimumZone) => {
            let z = zone::flatness(&pts, tol).map_err(failed)?;
            (plane_shape(&z.shape), z.optimal)
        }
        (FitKind::Line, FitMethod::MinimumZone) => {
            let z = zone::straightness(&pts, tol).map_err(failed)?;
            (line_shape(&z.shape), z.optimal)
        }
        (FitKind::Sphere, FitMethod::MinimumZone) => {
            let z = zone::sphericity(&pts, tol).map_err(failed)?;
            (sphere_shape(&z.shape), z.optimal)
        }
        (FitKind::Cylinder, m) => {
            let z = match m {
                FitMethod::MinimumZone => zone::cylindricity(&pts, tol),
                FitMethod::Inscribed => zone::inscribed_cylinder(&pts, tol),
                _ => zone::circumscribed_cylinder(&pts, tol),
            }
            .map_err(failed)?;
            (cylinder_shape(&z.shape), z.optimal)
        }
        (FitKind::Circle, m) => {
            // In the plane of the least-squares circle.
            let mean = fit::circle3(&pts, &FitOptions::default(), tol).map_err(failed)?;
            let frame = mean.shape.frame();
            let (o, x, y, n) = (frame.origin(), frame.x(), frame.y(), frame.z());
            let flat: Vec<ogeom::math::Point2> = pts
                .iter()
                .map(|p| {
                    let d = *p - o;
                    ogeom::math::Point2 {
                        x: d.dot(x.vector()),
                        y: d.dot(y.vector()),
                    }
                })
                .collect();
            let z = match m {
                FitMethod::MinimumZone => zone::circularity(&flat, tol),
                FitMethod::Inscribed => zone::inscribed_circle(&flat, tol),
                _ => zone::circumscribed_circle(&flat, tol),
            }
            .map_err(failed)?;
            let c = z.shape.centre();
            let centre = o + x.vector() * c.x + y.vector() * c.y;
            (
                FittedShape::Circle {
                    centre: array(centre),
                    normal: n.to_array(),
                    radius: z.shape.radius(),
                },
                z.optimal,
            )
        }
        _ => return Err(refused()),
    };
    let residuals: Vec<f64> = points.iter().map(|p| residual(&shape, *p)).collect();
    let lo = residuals.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = residuals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let form = match shape {
        // A line's zone is the cylinder round it: its diameter.
        FittedShape::Line { .. } => (-hi, hi),
        _ => (lo, hi),
    };
    let sigma =
        (residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len().max(1) as f64).sqrt();
    Ok(FitAnswer {
        shape,
        sigma,
        form,
        used: vec![true; points.len()],
        residuals,
        converged: optimal,
    })
}

/// `p`'s signed distance from `shape`, out of a round shape positive; a
/// line's the distance from it.
fn residual(shape: &FittedShape, p: [f64; 3]) -> f64 {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let len = |a: [f64; 3]| dot(a, a).sqrt();
    let off_axis = |q: [f64; 3], d: [f64; 3]| {
        let along = dot(q, d);
        len([
            q[0] - d[0] * along,
            q[1] - d[1] * along,
            q[2] - d[2] * along,
        ])
    };
    match *shape {
        FittedShape::Plane { origin, normal } => dot(sub(p, origin), normal),
        FittedShape::Sphere { centre, radius } => len(sub(p, centre)) - radius,
        FittedShape::Cylinder {
            point,
            axis,
            radius,
        } => off_axis(sub(p, point), axis) - radius,
        FittedShape::Line { point, direction } => off_axis(sub(p, point), direction),
        FittedShape::Circle {
            centre,
            normal,
            radius,
        } => off_axis(sub(p, centre), normal) - radius,
        FittedShape::Cone { .. } | FittedShape::Torus { .. } => f64::NAN,
    }
}

/// Grow a region of `kind` over `mesh` from the seeds and fit it.
pub(crate) fn grow(mesh: &TriMesh, request: &GrowRequest) -> KernelResult<GrowAnswer> {
    let kind = match request.kind {
        FitKind::Plane => GrowKind::Plane,
        FitKind::Sphere => GrowKind::Sphere,
        FitKind::Cylinder => GrowKind::Cylinder,
        FitKind::Cone => GrowKind::Cone,
        FitKind::Torus => GrowKind::Torus,
        FitKind::Circle => GrowKind::Circle3,
        FitKind::Line => GrowKind::Line3,
    };
    // A drawn mesh is split at its creases for shading: the kernel follows
    // creases across shared edges, so vertices at one spot become one.
    let crate::mesh_intake::Intake { tri, first, .. } = crate::mesh_intake::intake(
        mesh,
        Weld::Compact(crate::mesh_intake::SAME_POINT),
        Degenerate::Keep,
    )?;
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let index = Bvh::build(&tri).map_err(failed)?;
    let seeds: Vec<Point> = request.seeds.iter().map(point).collect();
    let defaults = GrowOptions::default();
    let options = GrowOptions {
        cutoff: request.sigma_cutoff.map_or(defaults.cutoff, Cutoff::Sigma),
        normal_limit: request
            .normal_limit_deg
            .map_or(defaults.normal_limit, f64::to_radians),
        peel: request.peel,
        max_triangles: request.max_triangles,
        ..defaults
    };
    let grown =
        fit::grow(&tri, &index, &seeds, kind, &options, tess::tolerances()).map_err(failed)?;
    let fit = match grown.fit {
        AnyFit::Plane(f) => answer(f, plane_shape),
        AnyFit::Sphere(f) => answer(f, sphere_shape),
        AnyFit::Cylinder(f) => answer(f, cylinder_shape),
        AnyFit::Cone(f) => answer(f, cone_shape),
        AnyFit::Torus(f) => answer(f, torus_shape),
        AnyFit::Circle(f) => answer(f, circle_shape),
        AnyFit::Line(f) => answer(f, line_shape),
        _ => return Err(KernelError::Unsupported("a grown fit of this kind".into())),
    };
    Ok(GrowAnswer {
        fit,
        triangles: grown.triangles,
        vertices: grown.vertices.iter().map(|v| first[*v as usize]).collect(),
        seeds_in_region: grown.seeds.iter().map(|s| s.in_region).collect(),
    })
}

fn plane_shape(s: &Plane) -> FittedShape {
    FittedShape::Plane {
        origin: array(s.origin()),
        normal: s.normal().to_array(),
    }
}

fn sphere_shape(s: &Sphere) -> FittedShape {
    FittedShape::Sphere {
        centre: array(s.frame().origin()),
        radius: s.radius(),
    }
}

fn cylinder_shape(s: &Cylinder) -> FittedShape {
    FittedShape::Cylinder {
        point: array(s.axis().location),
        axis: s.axis().direction.to_array(),
        radius: s.radius(),
    }
}

fn cone_shape(s: &Cone) -> FittedShape {
    FittedShape::Cone {
        apex: array(s.apex()),
        axis: s.axis().direction.to_array(),
        half_angle: s.half_angle(),
    }
}

fn circle_shape(s: &Circle) -> FittedShape {
    FittedShape::Circle {
        centre: array(s.frame().origin()),
        normal: s.frame().z().to_array(),
        radius: s.radius(),
    }
}

fn line_shape(s: &Axis) -> FittedShape {
    FittedShape::Line {
        point: array(s.location),
        direction: s.direction.to_array(),
    }
}

fn torus_shape(s: &Torus) -> FittedShape {
    FittedShape::Torus {
        centre: array(s.frame().origin()),
        axis: s.frame().z().to_array(),
        major: s.major_radius(),
        minor: s.minor_radius(),
    }
}

fn answer<S>(fitted: Fit<S>, shape: impl Fn(&S) -> FittedShape) -> FitAnswer {
    FitAnswer {
        shape: shape(&fitted.shape),
        sigma: fitted.sigma,
        form: fitted.form,
        used: fitted.used,
        residuals: fitted.residuals,
        converged: fitted.converged,
    }
}
