//! Upright frames and mirror planes read from a scan's welded triangles.

use kernel_api::{KernelError, KernelResult, Orientation, SymmetryPlane, TriMesh};
use ogeom::algo::{orient, register};
use ogeom::math::{Plane, Point, Vector};
use ogeom::mesh::Bvh;
use ogeom::topo::Triangulation;

fn failed(e: ogeom::core::OgeomError) -> KernelError {
    KernelError::InvalidInput(e.to_string())
}

fn mesh_of(mesh: &TriMesh) -> KernelResult<Triangulation> {
    if !mesh.indices.len().is_multiple_of(3)
        || mesh
            .indices
            .iter()
            .any(|i| *i as usize >= mesh.positions.len())
        || mesh.positions.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(KernelError::InvalidInput("invalid scan triangles".into()));
    }
    Ok(crate::deviation::welded(mesh))
}

pub(crate) fn orient(mesh: &TriMesh) -> KernelResult<Orientation> {
    let mesh = mesh_of(mesh)?;
    let tol = crate::tess::tolerances();
    let frame = orient::principal_frame(&mesh, tol).map_err(failed)?;
    let options = orient::Directions::default();
    let faces = orient::face_directions(&mesh, &options, tol).map_err(failed)?;
    let revolution = orient::axis_of_revolution(&mesh, &options, tol).map_err(failed)?;
    let rims = orient::open_rims(&mesh).map_err(failed)?;
    let mut support = faces
        .first()
        .map(|f| (f.direction.vector(), f.area, "largest flat face"));
    for rim in rims.iter().filter(|r| r.closed && r.flatness <= 0.5) {
        let area = rim.vector_area.magnitude();
        if area > 0.0 && support.is_none_or(|(_, a, _)| area > a) {
            support = Some((rim.vector_area / area, area, "planar open rim"));
        }
    }
    let round = revolution.filter(|r| {
        r.share > 0.5
            || (r.share > 0.05
                && faces.iter().all(|f| {
                    f.direction.vector().dot(r.axis.direction.vector()).abs()
                        > options.resolution.cos()
                }))
    });
    let (z, said) = if let Some(axis) = round {
        (axis.axis.direction.vector(), "axis of revolution")
    } else if let Some((normal, _, said)) = support {
        (-normal, said)
    } else {
        (
            frame.z().vector(),
            "principal axes (no dominant flat face or round walls)",
        )
    };
    // The longest principal direction square to the standing direction fixes X.
    let mut x = Vector::ZERO;
    for axis in [frame.x(), frame.y(), frame.z()] {
        let d = axis.vector();
        let projected = d - z * d.dot(z);
        if projected.magnitude() > 0.5 {
            x = projected / projected.magnitude();
            break;
        }
    }
    let y = z.cross(x);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &mesh.positions {
        let p = *p - Point::ORIGIN;
        for (i, axis) in [x, y, z].iter().enumerate() {
            lo[i] = lo[i].min(p.dot(*axis));
            hi[i] = hi[i].max(p.dot(*axis));
        }
    }
    let origin = x * ((lo[0] + hi[0]) * 0.5) + y * ((lo[1] + hi[1]) * 0.5) + z * lo[2];
    Ok(Orientation {
        origin: origin.to_array(),
        x_axis: x.to_array(),
        z_axis: z.to_array(),
        said: said.into(),
    })
}

pub(crate) fn symmetry_plane(mesh: &TriMesh) -> KernelResult<SymmetryPlane> {
    let mesh = mesh_of(mesh)?;
    let frame = orient::principal_frame(&mesh, crate::tess::tolerances()).map_err(failed)?;
    let seeds = [frame.x(), frame.y(), frame.z()].map(|n| Plane::through(frame.origin(), n));
    let index = Bvh::build(&mesh).map_err(failed)?;
    let normals = vec![Vector::ZERO; mesh.positions.len()];
    let fit = register::symmetry_plane(
        &mesh.positions,
        &normals,
        &index,
        &seeds,
        &register::Icp {
            sample: 2_000,
            ..Default::default()
        },
    )
    .map_err(failed)?;
    let plane = fit.plane.frame();
    let p = plane.origin();
    Ok(SymmetryPlane {
        point: [p.x, p.y, p.z],
        normal: plane.z().vector().to_array(),
        rms: fit.rms,
    })
}
