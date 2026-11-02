//! A solid's round holes and bosses drawn larger or smaller in radius,
//! as a printer that prints them off is made up for: every cylindrical
//! face offset along its normal, the neighbours following, holes and
//! bosses each by their own amount.

use ogeom::algo::face_normal;
use ogeom::offset::offset_faces;
use ogeom::topo::{Model, Shape, ShapeType, explore_unique};

use kernel_api::FaceSurface;

use crate::tess;

/// The cylindrical faces of `solid`, holes (their normal toward the axis)
/// or bosses as `holes` asks.
fn rounds(model: &Model, solid: &Shape, holes: bool) -> Result<Vec<Shape>, String> {
    let tol = tess::tolerances();
    let faces = explore_unique(model, solid, ShapeType::Face).map_err(|e| e.to_string())?;
    Ok(faces
        .into_iter()
        .filter(|face| {
            let Ok((at, normal)) = face_normal(model, face, tol) else {
                return false;
            };
            let facing = [normal.x as f32, normal.y as f32, normal.z as f32];
            let FaceSurface::Cylinder { origin, axis, .. } =
                tess::face_surface(model, face, facing)
            else {
                return false;
            };
            let (o, a) = (origin.map(f64::from), axis.map(f64::from));
            let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
            let a = a.map(|c| c / len);
            let off = [at.x - o[0], at.y - o[1], at.z - o[2]];
            let t = off[0] * a[0] + off[1] * a[1] + off[2] * a[2];
            let radial = [off[0] - t * a[0], off[1] - t * a[1], off[2] - t * a[2]];
            let out = normal.x * radial[0] + normal.y * radial[1] + normal.z * radial[2];
            (out < 0.0) == holes
        })
        .collect())
}

/// `solid` with its holes' radii changed by `hole` and its bosses' by
/// `boss`, mm (positive larger).
pub(crate) fn offset_rounds(
    model: &mut Model,
    solid: &Shape,
    hole: f64,
    boss: f64,
) -> Result<Shape, String> {
    let mut shape = solid.clone();
    // A hole grows as its wall moves against its outward normal.
    for (holes, distance) in [(true, -hole), (false, boss)] {
        if distance == 0.0 {
            continue;
        }
        let faces = rounds(model, &shape, holes)?;
        if faces.is_empty() {
            continue;
        }
        shape = offset_faces(model, &shape, &faces, distance, tess::tolerances())
            .map(|b| b.shape)
            .map_err(|e| {
                format!(
                    "offsetting the round {} by {distance} mm failed: {e}",
                    if holes { "holes" } else { "bosses" }
                )
            })?;
    }
    Ok(shape)
}
