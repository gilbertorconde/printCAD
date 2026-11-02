//! A face of the model moved onto a scan: a plane or cylinder fitted to
//! the scan where it is clicked, and the face carried onto it, a flat
//! face along its normal, a round face to the fitted radius.

use core_document::{BodyId, Document};
use kernel_api::{FaceSurface, FitKind, FittedShape, GrowRequest, KernelQueries};

/// A scan surface turned further than this from the face is refused,
/// degrees.
pub const MOST_TURN_DEG: f64 = 20.0;

type V = [f64; 3];

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn unit(a: V) -> V {
    let len = dot(a, a).sqrt().max(1e-300);
    a.map(|c| c / len)
}

/// What snapping a face comes to.
#[derive(Debug, Clone, PartialEq)]
pub enum Snap {
    /// Move the flat face by `distance` along its outward normal.
    Move { distance: f64, angle_deg: f64 },
    /// Offset the round face by `distance` along its outward normal (a
    /// bore's points at its axis), from `radius` to `fitted`.
    Offset {
        distance: f64,
        radius: f64,
        fitted: f64,
        angle_deg: f64,
        axis_offset: f64,
    },
}

/// The surface of `body`'s face at `point` (in the body's frame): the
/// face of the mesh triangle nearest it.
pub fn face_surface(document: &Document, body: BodyId, point: V) -> Option<FaceSurface> {
    let (mesh, _) = document.local_geometry(body)?;
    let mut best = (f64::INFINITY, None);
    for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let c: V = std::array::from_fn(|k| {
            tri.iter()
                .map(|v| f64::from(mesh.positions[*v as usize][k]))
                .sum::<f64>()
                / 3.0
        });
        let d = dot(sub(c, point), sub(c, point));
        if d < best.0 {
            best = (d, mesh.faces.get(t).copied());
        }
    }
    mesh.face_surfaces.get(best.1? as usize).copied()
}

/// The `kind` grown over `scan` from `at` (document frame) and fitted,
/// in `body`'s frame, with its σ.
pub fn fit_on_scan(
    document: &Document,
    kernel: &dyn KernelQueries,
    scan: BodyId,
    at: V,
    kind: FitKind,
    body: BodyId,
) -> Result<(FittedShape, f64), String> {
    if document.body_scan(scan).is_none() {
        return Err("that body is not a scan".into());
    }
    let (mesh, _) = document
        .local_geometry(scan)
        .ok_or("the scan has no mesh")?;
    let scan_at = document.body_placement(scan);
    let seed = scan_at.inverse().point(at.map(|c| c as f32)).map(f64::from);
    let grown = kernel
        .grow(
            &mesh,
            &GrowRequest {
                kind,
                seeds: vec![seed],
                sigma_cutoff: None,
                normal_limit_deg: None,
                peel: false,
                max_triangles: None,
            },
        )
        .map_err(|e| e.to_string())?;
    if grown.seeds_in_region.first() == Some(&false) {
        return Err("the scan there is not that shape".into());
    }
    let to = document.body_placement(body).inverse().after(&scan_at);
    let p = |v: V| to.point(v.map(|c| c as f32)).map(f64::from);
    let d = |v: V| to.direction(v.map(|c| c as f32)).map(f64::from);
    let shape = match grown.fit.shape {
        FittedShape::Plane { origin, normal } => FittedShape::Plane {
            origin: p(origin),
            normal: d(normal),
        },
        FittedShape::Cylinder {
            point,
            axis,
            radius,
        } => FittedShape::Cylinder {
            point: p(point),
            axis: d(axis),
            radius,
        },
        _ => return Err("the fit is not a plane or cylinder".into()),
    };
    Ok((shape, grown.fit.sigma))
}

/// How the face at `point` with outward `normal` on `surface` moves onto
/// `fitted`.
pub fn snap_of(
    surface: &FaceSurface,
    point: V,
    normal: V,
    fitted: &FittedShape,
) -> Result<Snap, String> {
    let normal = unit(normal);
    match (surface, fitted) {
        (FaceSurface::Plane { .. }, FittedShape::Plane { origin, normal: n }) => {
            let n = unit(*n);
            let cos = dot(normal, n).abs().min(1.0);
            let angle_deg = cos.acos().to_degrees();
            if angle_deg > MOST_TURN_DEG {
                return Err(format!(
                    "the scan there is turned {angle_deg:.0}° from the face, over {MOST_TURN_DEG}°"
                ));
            }
            // Along the face's normal to where it meets the fitted plane.
            let distance = dot(sub(*origin, point), n) / dot(normal, n);
            Ok(Snap::Move {
                distance,
                angle_deg,
            })
        }
        (
            FaceSurface::Cylinder {
                origin,
                axis,
                radius,
            },
            FittedShape::Cylinder {
                point: p,
                axis: a,
                radius: fitted,
            },
        ) => {
            let (o, ax) = (origin.map(f64::from), unit(axis.map(f64::from)));
            let a = unit(*a);
            let angle_deg = dot(ax, a).abs().min(1.0).acos().to_degrees();
            if angle_deg > MOST_TURN_DEG {
                return Err(format!(
                    "the scan's axis there is turned {angle_deg:.0}° from the face's, over {MOST_TURN_DEG}°"
                ));
            }
            let off = sub(*p, o);
            let along = dot(off, ax);
            let across: V = std::array::from_fn(|k| off[k] - along * ax[k]);
            let radius = f64::from(*radius);
            // A bore's outward normal points at its axis.
            let radial: V = {
                let r = sub(point, o);
                let t = dot(r, ax);
                std::array::from_fn(|k| r[k] - t * ax[k])
            };
            let bore = dot(normal, radial) < 0.0;
            let distance = if bore {
                radius - fitted
            } else {
                fitted - radius
            };
            Ok(Snap::Offset {
                distance,
                radius,
                fitted: *fitted,
                angle_deg,
                axis_offset: dot(across, across).sqrt(),
            })
        }
        (FaceSurface::Plane { .. } | FaceSurface::Cylinder { .. }, _) => {
            Err("the scan there is not the face's shape".into())
        }
        _ => Err("only a flat or round face snaps to a scan".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A face 0.4 below a fitted plane tilted 2° moves up 0.4 and
    /// reads the tilt; a bore of radius 3 fitted at 3.2 opens 0.2, a pin
    /// grows; a turn past the limit is refused.
    #[test]
    fn a_face_moves_onto_its_fit() {
        let flat = FaceSurface::Plane {
            origin: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        };
        let t = 2f64.to_radians();
        let tilted = FittedShape::Plane {
            origin: [0.0, 0.0, 0.4],
            normal: [t.sin(), 0.0, t.cos()],
        };
        let Snap::Move {
            distance,
            angle_deg,
        } = snap_of(&flat, [0.0; 3], [0.0, 0.0, 1.0], &tilted).unwrap()
        else {
            panic!()
        };
        assert!((distance - 0.4).abs() < 1e-9 && (angle_deg - 2.0).abs() < 1e-9);
        let steep = FittedShape::Plane {
            origin: [0.0; 3],
            normal: [1.0, 0.0, 1.0],
        };
        assert!(snap_of(&flat, [0.0; 3], [0.0, 0.0, 1.0], &steep).is_err());

        let round = FaceSurface::Cylinder {
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 3.0,
        };
        let fit = FittedShape::Cylinder {
            point: [0.1, 0.0, 5.0],
            axis: [0.0, 0.0, 1.0],
            radius: 3.2,
        };
        // A bore: its outward normal at the axis.
        let Snap::Offset {
            distance,
            axis_offset,
            ..
        } = snap_of(&round, [3.0, 0.0, 1.0], [-1.0, 0.0, 0.0], &fit).unwrap()
        else {
            panic!()
        };
        assert!((distance + 0.2).abs() < 1e-6 && (axis_offset - 0.1).abs() < 1e-9);
        let Snap::Offset { distance, .. } =
            snap_of(&round, [3.0, 0.0, 1.0], [1.0, 0.0, 0.0], &fit).unwrap()
        else {
            panic!()
        };
        assert!((distance - 0.2).abs() < 1e-6, "a pin grows");
    }
}
