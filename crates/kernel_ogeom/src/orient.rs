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

/// The sections of a scan by a stack of parallel planes.
pub(crate) fn sections(
    mesh: &TriMesh,
    origin: [f64; 3],
    normal: [f64; 3],
    offsets: &[f64],
) -> KernelResult<Vec<Vec<kernel_api::SectionChain>>> {
    let mesh = mesh_of(mesh)?;
    let tol = crate::tess::tolerances();
    let normal = ogeom::math::Direction::new(Vector::new(normal[0], normal[1], normal[2]), tol)
        .map_err(failed)?;
    let plane = Plane::through(Point::new(origin[0], origin[1], origin[2]), normal);
    let cut = ogeom::mesh::sections(&mesh, &plane, offsets).map_err(failed)?;
    Ok(cut
        .into_iter()
        .map(|s| {
            s.chains
                .into_iter()
                .map(|c| kernel_api::SectionChain {
                    points: c.points.iter().map(|p| [p.x, p.y, p.z]).collect(),
                    outward: c.outward.iter().map(|v| [v.x, v.y, v.z]).collect(),
                    closed: c.closed,
                })
                .collect()
        })
        .collect())
}

/// The shadow of a scan on a plane and the outline fitted around it.
pub(crate) fn shadow(
    mesh: &TriMesh,
    request: &kernel_api::ShadowRequest,
) -> KernelResult<kernel_api::ShadowAnswer> {
    use kernel_api::{ProfileSegment, ProfileWire};
    use ogeom::algo::{Outline, Piece2, fit_outline};
    if request.tolerance.is_nan() || request.tolerance <= 0.0 || !request.play.is_finite() {
        return Err(KernelError::InvalidInput(
            "an outline needs a tolerance above zero and a finite play".into(),
        ));
    }
    let mesh = mesh_of(mesh)?;
    let plane = crate::profile::plane_of(&request.plane).map_err(failed)?;
    let loops = ogeom::mesh::shadow(&mesh, &plane).map_err(failed)?;
    let wires = fit_outline(
        &loops,
        &Outline {
            play: request.play,
            tolerance: request.tolerance,
            round_below: request.round_below_deg.to_radians(),
        },
    )
    .map_err(failed)?;
    let xy = |p: ogeom::math::Point2| [p.x, p.y];
    let segment = |piece: &Piece2| match *piece {
        Piece2::Line { from, to } => ProfileSegment::Line {
            start: xy(from),
            end: xy(to),
        },
        Piece2::Arc { .. } => ProfileSegment::Arc {
            start: xy(piece.point_at(0.0)),
            mid: xy(piece.point_at(0.5)),
            end: xy(piece.point_at(1.0)),
        },
        Piece2::Circle { centre, radius, .. } => ProfileSegment::Circle {
            center: xy(centre),
            radius,
        },
    };
    Ok(kernel_api::ShadowAnswer {
        silhouette: loops
            .iter()
            .map(|l| l.points.iter().map(|p| xy(*p)).collect())
            .collect(),
        outline: wires
            .iter()
            .map(|w| ProfileWire::new(w.pieces.iter().map(segment).collect()))
            .collect(),
    })
}
