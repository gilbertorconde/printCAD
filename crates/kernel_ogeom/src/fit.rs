//! Shapes fitted to measured points: the kernel's least-squares fits
//! (`ogeom::algo::fit`), asked in the contract's terms and answered in
//! them.

use kernel_api::{FitAnswer, FitKind, FitRequest, FittedShape, KernelError, KernelResult};
use ogeom::algo::Cutoff;
use ogeom::algo::fit::{self, Fit, FitFixed, FitOptions};
use ogeom::math::{Point, Vector};

use crate::tess;

/// Fit `request.kind` to its points.
pub(crate) fn fit(request: &FitRequest) -> KernelResult<FitAnswer> {
    let tol = tess::tolerances();
    let points: Vec<Point> = request
        .points
        .iter()
        .map(|p| Point::new(p[0], p[1], p[2]))
        .collect();
    let normals: Vec<Vector> = request
        .normals
        .iter()
        .map(|n| Vector::new(n[0], n[1], n[2]))
        .collect();
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
    let p = |p: Point| [p.x, p.y, p.z];
    Ok(match request.kind {
        FitKind::Plane => answer(
            fit::plane(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Plane {
                origin: p(s.origin()),
                normal: s.normal().to_array(),
            },
        ),
        FitKind::Sphere => answer(
            fit::sphere(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Sphere {
                centre: p(s.frame().origin()),
                radius: s.radius(),
            },
        ),
        FitKind::Cylinder => answer(
            fit::cylinder(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Cylinder {
                point: p(s.axis().location),
                axis: s.axis().direction.to_array(),
                radius: s.radius(),
            },
        ),
        FitKind::Cone => answer(fit::cone(&points, &options!(), tol).map_err(failed)?, |s| {
            FittedShape::Cone {
                apex: p(s.apex()),
                axis: s.axis().direction.to_array(),
                half_angle: s.half_angle(),
            }
        }),
        FitKind::Torus => answer(
            fit::torus(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Torus {
                centre: p(s.frame().origin()),
                axis: s.frame().z().to_array(),
                major: s.major_radius(),
                minor: s.minor_radius(),
            },
        ),
        FitKind::Circle => answer(
            fit::circle3(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Circle {
                centre: p(s.frame().origin()),
                normal: s.frame().z().to_array(),
                radius: s.radius(),
            },
        ),
        FitKind::Line => answer(
            fit::line3(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Line {
                point: p(s.location),
                direction: s.direction.to_array(),
            },
        ),
    })
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
