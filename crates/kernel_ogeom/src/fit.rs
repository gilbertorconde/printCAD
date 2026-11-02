//! Shapes fitted to measured points: the kernel's least-squares fits
//! (`ogeom::algo::fit`), asked in the contract's terms and answered in
//! them, and regions grown over a mesh from a clicked point.

use kernel_api::{
    FitAnswer, FitKind, FitRequest, FittedShape, GrowAnswer, GrowRequest, KernelError,
    KernelResult, TriMesh,
};
use ogeom::algo::Cutoff;
use ogeom::algo::fit::{self, AnyFit, Fit, FitFixed, FitKind as GrowKind, FitOptions, GrowOptions};
use ogeom::math::{Axis, Circle, Cone, Cylinder, Plane, Point, Sphere, Torus, Vector};
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
            circle_shape,
        ),
        FitKind::Line => answer(
            fit::line3(&points, &options!(), tol).map_err(failed)?,
            line_shape,
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
        FitKind::Circle => GrowKind::Circle3,
        FitKind::Line => GrowKind::Line3,
    };
    // A drawn mesh is split at its creases for shading: the kernel follows
    // creases across shared edges, so vertices at one spot become one.
    let (welded, first) = weld(mesh);
    let tri = Triangulation {
        positions: first
            .iter()
            .map(|v| mesh.positions[*v as usize])
            .map(|p| Point::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect(),
        normals: Vec::new(),
        parameters: Vec::new(),
        triangles: mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|v| welded[v as usize]))
            .collect(),
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

/// Each vertex's index among the distinct positions, and the first vertex
/// at each distinct position.
pub(crate) fn weld(mesh: &TriMesh) -> (Vec<u32>, Vec<u32>) {
    let mut at = std::collections::HashMap::new();
    let mut first = Vec::new();
    let welded = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(v, p)| {
            *at.entry(p.map(f32::to_bits)).or_insert_with(|| {
                first.push(v as u32);
                first.len() as u32 - 1
            })
        })
        .collect();
    (welded, first)
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

fn circle_shape(s: &Circle) -> FittedShape {
    FittedShape::Circle {
        centre: xyz(s.frame().origin()),
        normal: s.frame().z().to_array(),
        radius: s.radius(),
    }
}

fn line_shape(s: &Axis) -> FittedShape {
    FittedShape::Line {
        point: xyz(s.location),
        direction: s.direction.to_array(),
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
