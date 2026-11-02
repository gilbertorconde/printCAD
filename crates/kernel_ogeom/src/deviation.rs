//! Signed distances from measured points to a reference surface: a mesh's
//! own triangles, or a solid meshed finely enough that the meshing is far
//! below what is measured.

use kernel_api::{
    DistanceRequest, DistanceTarget, KernelError, KernelResult, LinearDeflectionMode,
    TessellationSettings, TriMesh,
};
use ogeom::algo::distance::{self, DistanceOptions};
use ogeom::math::{Point, Vector};
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
    let failed = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    let index = Bvh::build(&welded(mesh)).map_err(failed)?;
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
