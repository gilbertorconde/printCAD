//! Signed distances from measured points to a reference surface, and the
//! rigid motion that fits them onto it: the surface a mesh's
//! own triangles, or a solid meshed finely enough that the meshing is far
//! below what is measured.

use kernel_api::{
    DistanceRequest, DistanceTarget, FreeMotion, KernelError, KernelResult, LinearDeflectionMode,
    RegisterRequest, Registration, TessellationSettings, ThicknessRequest, TriMesh,
};
use ogeom::algo::distance::{self, DistanceOptions};
use ogeom::algo::register::{self, Icp, Starts};
use ogeom::algo::thickness::{self, Method, Thickness};
use ogeom::math::{Point, Transform, Vector};
use ogeom::mesh::Bvh;
use ogeom::topo::Triangulation;

use crate::fit::weld;
use crate::tess;

/// A solid is meshed to a chord of this share of its bounding box's
/// summed extents (0.01 mm on a 100 mm cube).
const SHAPE_CHORD_SHARE: f32 = 0.01;

/// The signed distance of each point to `target`, `NaN` where nothing lies
/// within the request's reach.
pub(crate) fn signed_distance(
    target: DistanceTarget<'_>,
    request: &DistanceRequest,
) -> KernelResult<Vec<f64>> {
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let index = target_index(target)?;
    let points = points_of(&request.points);
    let normals = normals_of(&request.normals);
    let normals = (normals.len() == points.len()).then_some(normals.as_slice());
    let options = DistanceOptions {
        max: request.max.unwrap_or(f64::INFINITY),
        facing_limit: request
            .facing_limit_deg
            .filter(|_| normals.is_some())
            .map(f64::to_radians),
        signed: true,
    };
    let found = distance::to_mesh(&index, &points, normals, &options).map_err(failed)?;
    Ok(found
        .into_iter()
        .map(|c| c.map_or(f64::NAN, |c| c.distance))
        .collect())
}

/// Points as many samples of a scan as the coarse search starting from
/// every principal-axis matching works on.
const SEARCH_SAMPLES: usize = 2_000;
/// And as the fit refined from its answer works on.
const FIT_SAMPLES: usize = 60_000;

/// The rigid motion bringing the request's points best onto `target`.
pub(crate) fn register(
    target: DistanceTarget<'_>,
    request: &RegisterRequest,
) -> KernelResult<Registration> {
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let index = target_index(target)?;
    let points = points_of(&request.points);
    let normals = normals_of(&request.normals);
    let normals = (normals.len() == points.len()).then_some(normals.as_slice());
    let base = Icp {
        max_distance: request.max_distance,
        facing_limit: request
            .facing_limit_deg
            .filter(|_| normals.is_some())
            .map(f64::to_radians),
        ..Icp::default()
    };
    let run = |starts: Starts, sample: usize| {
        register::icp(
            &points,
            normals,
            &index,
            &Icp {
                starts,
                sample,
                ..base.clone()
            },
        )
        .map_err(failed)
    };
    let start = if request.search {
        let here = run(Starts::Identity, SEARCH_SAMPLES)?;
        let turned = run(Starts::PrincipalAxes24, SEARCH_SAMPLES)?;
        if turned.score < here.score {
            turned.transform
        } else {
            here.transform
        }
    } else {
        Transform::IDENTITY
    };
    let fit = run(Starts::Given(vec![start]), FIT_SAMPLES)?;
    let g = fit.transform.to_general();
    let r = g.linear.rows;
    let t = g.translation;
    Ok(Registration {
        transform: [
            [r[0][0], r[0][1], r[0][2], t.x],
            [r[1][0], r[1][1], r[1][2], t.y],
            [r[2][0], r[2][1], r[2][2], t.z],
        ],
        rms: fit.rms,
        inliers: fit.inliers,
        sampled: fit.sampled,
        converged: fit.converged,
        free: fit
            .free_directions
            .iter()
            .map(|m| FreeMotion {
                about: [m.about.x, m.about.y, m.about.z],
                rotation: m.rotation.to_array(),
                translation: m.translation.to_array(),
            })
            .collect(),
    })
}

/// The wall thickness of `target` at each of the request's points.
pub(crate) fn thickness(
    target: DistanceTarget<'_>,
    request: &ThicknessRequest,
) -> KernelResult<Vec<f64>> {
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let points = points_of(&request.points);
    let normals = normals_of(&request.normals);
    let options = Thickness {
        method: if request.sphere {
            Method::Sphere
        } else {
            Method::Ray {
                cone: request.cone_deg.to_radians(),
                rays: request.rays.max(1),
            }
        },
        facing_limit: request.facing_limit_deg.to_radians(),
        max: request.max.unwrap_or(f64::INFINITY),
        ..Thickness::default()
    };
    let found = match target {
        DistanceTarget::Shape(blob) => {
            let (model, shape) = tess::read_blob(blob)?;
            thickness::thickness_of_shape(&model, &shape, &points, &normals, &options)
                .map_err(failed)?
        }
        DistanceTarget::Mesh(_) => {
            let index = target_index(target)?;
            thickness::thickness(&index, &points, &normals, &options).map_err(failed)?
        }
    };
    Ok(found.into_iter().map(|t| t.unwrap_or(f64::NAN)).collect())
}

fn points_of(points: &[[f64; 3]]) -> Vec<Point> {
    points
        .iter()
        .map(|p| Point::new(p[0], p[1], p[2]))
        .collect()
}

fn normals_of(normals: &[[f64; 3]]) -> Vec<Vector> {
    normals
        .iter()
        .map(|n| Vector::new(n[0], n[1], n[2]))
        .collect()
}

/// `target`'s triangles, a solid meshed finely first, indexed for
/// nearest-point queries.
fn target_index(target: DistanceTarget<'_>) -> KernelResult<Bvh> {
    let meshed;
    let mesh = match target {
        DistanceTarget::Mesh(mesh) => mesh,
        DistanceTarget::Shape(blob) => {
            let detail = TessellationSettings {
                linear_deflection_mode: LinearDeflectionMode::BboxScaled,
                mesh_deviation: SHAPE_CHORD_SHARE,
                ..TessellationSettings::default()
            };
            meshed = tess::tessellate_blob(blob, &[], &detail, tess::Faces::Wide)?;
            &meshed
        }
    };
    Bvh::build(&welded(mesh)).map_err(|e| KernelError::InvalidInput(e.to_string()))
}

/// `mesh` with the vertices at one spot made one, so its signs are read
/// across the creases a drawn mesh is split at.
fn welded(mesh: &TriMesh) -> Triangulation {
    let (welded, first) = weld(mesh);
    Triangulation {
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
    }
}
