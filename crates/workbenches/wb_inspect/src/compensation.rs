//! What a scanned print says beyond each axis's stretch: which scan
//! points stand on the model's round holes and bosses (whose radius error
//! the stretch fit reads with it), the first layer's flare and the
//! bottom's warp. Everything is in the model's own frame, the model
//! standing as it was printed, +Z up.

use glam::DVec3;
use kernel_api::{FaceSurface, TriMesh};
use metrology::Vec3;
use metrology::shrink::{Role, Shrink};
use metrology::stats;

use crate::vec::unit;

/// The first layers' height, mm: the band of side wall a flare is read
/// over.
pub const FIRST_LAYERS: f64 = 0.6;

/// How far above the model's bottom, mm, a corner may lift and still be
/// read as the bottom face's.
const LIFT_REACH: f64 = 3.0;

/// A round face of the model: its axis, radius and extent along it, and
/// whether it is a hole.
struct Round {
    origin: DVec3,
    axis: DVec3,
    radius: f64,
    along: [f64; 2],
    hole: bool,
}

/// `p`'s distance along the round's axis, its radial direction (none on
/// the axis itself) and its distance from the axis.
fn radial(r: &Round, p: Vec3) -> (f64, Option<DVec3>, f64) {
    let v = DVec3::from(p) - r.origin;
    let t = v.dot(r.axis);
    let out = v - r.axis * t;
    (t, unit(out), out.length())
}

/// The model's cylindrical faces, from its mesh.
fn rounds(model: &TriMesh) -> Vec<Round> {
    let mut found: Vec<(u32, Round, f64)> = Vec::new();
    for (t, tri) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        let Some(&face) = model.faces.get(t) else {
            break;
        };
        let Some(FaceSurface::Cylinder {
            origin,
            axis,
            radius,
        }) = model.face_surfaces.get(face as usize)
        else {
            continue;
        };
        let Some(axis) = unit(DVec3::from(axis.map(f64::from))) else {
            continue;
        };
        let at = match found.iter().position(|(f, ..)| *f == face) {
            Some(at) => at,
            None => {
                found.push((
                    face,
                    Round {
                        origin: DVec3::from(origin.map(f64::from)),
                        axis,
                        radius: f64::from(*radius),
                        along: [f64::MAX, f64::MIN],
                        hole: false,
                    },
                    0.0,
                ));
                found.len() - 1
            }
        };
        let (_, round, facing) = &mut found[at];
        for v in tri {
            let p = model.positions[*v as usize].map(f64::from);
            let (t, out, _) = radial(round, p);
            round.along = [round.along[0].min(t), round.along[1].max(t)];
            if let (Some(n), Some(out)) = (model.normals.get(*v as usize), out) {
                *facing += DVec3::from(n.map(f64::from)).dot(out);
            }
        }
    }
    found
        .into_iter()
        .map(|(_, mut r, facing)| {
            r.hole = facing < 0.0;
            r
        })
        .collect()
}

/// What each scan point stands on: a round hole's or boss's wall (well
/// inside its ends, facing across the axis), the first layers' side wall
/// (set aside for the flare), or anything else.
pub fn roles(model: &TriMesh, points: &[Vec3], normals: &[Vec3], bottom: f64) -> Vec<Role> {
    let rounds = rounds(model);
    points
        .iter()
        .zip(normals)
        .map(|(p, m)| {
            if p[2] - bottom < FIRST_LAYERS && m[2].abs() < 0.5 {
                return Role::Aside;
            }
            for r in &rounds {
                let (t, out, distance) = radial(r, *p);
                let band = (0.05 * r.radius).max(0.3);
                let inside = t > r.along[0] + 0.5 && t < r.along[1] - 0.5;
                let facing = out.map_or(0.0, |out| DVec3::from(*m).dot(out))
                    * if r.hole { -1.0 } else { 1.0 };
                if inside && (distance - r.radius).abs() < band && facing > 0.8 {
                    return if r.hole { Role::Hole } else { Role::Boss };
                }
            }
            Role::Plain
        })
        .collect()
}

/// How far the first layers' side walls stand out past what the stretch
/// explains, mm: the median over the points set aside.
pub fn flare(
    fit: &Shrink,
    points: &[Vec3],
    normals: &[Vec3],
    distance: &[f64],
    roles: &[Role],
) -> Option<f64> {
    let off: Vec<f64> = (0..roles.len())
        .filter(|i| roles[*i] == Role::Aside && distance[*i].is_finite())
        .map(|i| distance[i] - fit.predict(points[i], normals[i]))
        .collect();
    (off.len() >= 6)
        .then(|| stats::percentile(&off, 0.5))
        .flatten()
}

/// The bottom face as printed: how far from flat it is and how high its
/// highest part (a lifted corner) stands over the rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Warp {
    /// The bottom's highest point over its lowest, the lowest hundredth
    /// of points left out as noise, mm.
    pub flatness: f64,
    /// The highest part over the bottom's middle height, mm.
    pub lift: f64,
    /// Where it lifts most.
    pub at: Vec3,
}

/// The warp of the scan's down-facing points near the model's bottom.
pub fn warp(points: &[Vec3], normals: &[Vec3], bottom: f64) -> Option<Warp> {
    let under: Vec<Vec3> = points
        .iter()
        .zip(normals)
        .filter(|(p, m)| m[2] < -0.9 && p[2] - bottom < LIFT_REACH)
        .map(|(p, _)| *p)
        .collect();
    if under.len() < 12 {
        return None;
    }
    let heights: Vec<f64> = under.iter().map(|p| p[2]).collect();
    let low = stats::percentile(&heights, 0.01)?;
    let middle = stats::percentile(&heights, 0.5)?;
    let at = under.iter().max_by(|a, b| a[2].total_cmp(&b[2])).copied()?;
    let high = at[2];
    Some(Warp {
        flatness: high - low,
        lift: high - middle,
        at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plate with a hole: points on the hole's wall read as the hole's,
    /// points at its ends and on the plate's first layer do not; a bottom
    /// with one corner lifted reads its lift.
    #[test]
    fn scan_points_find_the_hole_they_stand_on_and_the_bottom_its_lift() {
        // One cylinder face, a hole of radius 3 about Z from z 0 to 5.
        let mut model = TriMesh::default();
        for k in 0..12 {
            let t = f64::from(k) * std::f64::consts::TAU / 12.0;
            for z in [0.0, 5.0] {
                model
                    .positions
                    .push([3.0 * t.cos() as f32, 3.0 * t.sin() as f32, z]);
                model.normals.push([-t.cos() as f32, -t.sin() as f32, 0.0]);
            }
        }
        model.indices = (0..12u32)
            .flat_map(|k| {
                let (a, b) = (2 * k, 2 * ((k + 1) % 12));
                [a, b, a + 1, b, b + 1, a + 1]
            })
            .collect();
        model.faces = vec![0; 24];
        model.face_surfaces = vec![FaceSurface::Cylinder {
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 3.0,
        }];
        let points = [
            [2.9, 0.0, 2.5],
            [2.9, 0.0, 0.2],
            [0.0, 3.05, 2.5],
            [9.0, 0.0, 0.3],
            [9.0, 0.0, 2.5],
        ];
        let normals = [
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let roles = roles(&model, &points, &normals, 0.0);
        assert_eq!(
            roles,
            [
                Role::Hole,
                Role::Aside,
                Role::Hole,
                Role::Aside,
                Role::Plain
            ]
        );

        let mut bottom = Vec::new();
        for x in 0..10 {
            for y in 0..10 {
                let lift = if x + y > 15 {
                    0.1 * f64::from(x + y - 15)
                } else {
                    0.0
                };
                bottom.push([f64::from(x), f64::from(y), lift]);
            }
        }
        let down = vec![[0.0, 0.0, -1.0]; bottom.len()];
        let w = warp(&bottom, &down, 0.0).unwrap();
        assert!((w.lift - 0.3).abs() < 0.05 && w.flatness > 0.25, "{w:?}");
        assert!(w.at[0] + w.at[1] > 16.0);
    }
}
