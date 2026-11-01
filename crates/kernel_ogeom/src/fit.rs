//! Shapes fitted to measured points: the kernel's least-squares fits
//! (`ogeom::algo::fit`), asked in the contract's terms and answered in
//! them, and regions grown over a mesh from a clicked point.

use kernel_api::{
    FitAnswer, FitKind, FitRequest, FittedShape, GrowAnswer, GrowRequest, KernelError,
    KernelResult, TriMesh,
};
use ogeom::algo::Cutoff;
use ogeom::algo::fit::{self, AnyFit, Fit, FitFixed, FitKind as GrowKind, FitOptions, GrowOptions};
use ogeom::math::{Cone, Cylinder, Plane, Point, Sphere, Torus, Vector};
use ogeom::mesh::Bvh;
use ogeom::topo::Triangulation;

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
            |s| FittedShape::Circle {
                centre: xyz(s.frame().origin()),
                normal: s.frame().z().to_array(),
                radius: s.radius(),
            },
        ),
        FitKind::Line => answer(
            fit::line3(&points, &options!(), tol).map_err(failed)?,
            |s| FittedShape::Line {
                point: xyz(s.location),
                direction: s.direction.to_array(),
            },
        ),
    })
}

/// Grow a region of `kind` over `mesh` from the seeds and fit it.
pub(crate) fn grow(mesh: &TriMesh, request: &GrowRequest) -> KernelResult<GrowAnswer> {
    let kind = match request.kind {
        FitKind::Plane => GrowKind::Plane,
        FitKind::Sphere => GrowKind::Sphere,
        FitKind::Cylinder => GrowKind::Cylinder,
        FitKind::Cone => GrowKind::Cone,
        FitKind::Torus => GrowKind::Torus,
        FitKind::Circle | FitKind::Line => {
            return Err(KernelError::Unsupported(
                "growing a circle or a line along a rim".into(),
            ));
        }
    };
    let tri = Triangulation {
        positions: mesh
            .positions
            .iter()
            .map(|p| Point::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect(),
        normals: mesh
            .normals
            .iter()
            .map(|n| Vector::new(f64::from(n[0]), f64::from(n[1]), f64::from(n[2])))
            .collect(),
        parameters: Vec::new(),
        triangles: mesh.indices.as_chunks::<3>().0.to_vec(),
        deflection_met: true,
    };
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let index = Bvh::build(&tri).map_err(failed)?;
    let seeds: Vec<Point> = request
        .seeds
        .iter()
        .map(|p| Point::new(p[0], p[1], p[2]))
        .collect();
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
    };
    Ok(GrowAnswer {
        fit,
        triangles: grown.triangles,
        vertices: grown.vertices,
        seeds_in_region: grown.seeds.iter().map(|s| s.in_region).collect(),
    })
}

fn xyz(p: Point) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn plane_shape(s: &Plane) -> FittedShape {
    FittedShape::Plane {
        origin: xyz(s.origin()),
        normal: s.normal().to_array(),
    }
}

fn sphere_shape(s: &Sphere) -> FittedShape {
    FittedShape::Sphere {
        centre: xyz(s.frame().origin()),
        radius: s.radius(),
    }
}

fn cylinder_shape(s: &Cylinder) -> FittedShape {
    FittedShape::Cylinder {
        point: xyz(s.axis().location),
        axis: s.axis().direction.to_array(),
        radius: s.radius(),
    }
}

fn cone_shape(s: &Cone) -> FittedShape {
    FittedShape::Cone {
        apex: xyz(s.apex()),
        axis: s.axis().direction.to_array(),
        half_angle: s.half_angle(),
    }
}

fn torus_shape(s: &Torus) -> FittedShape {
    FittedShape::Torus {
        centre: xyz(s.frame().origin()),
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
