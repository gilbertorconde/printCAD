//! A bench's flat regions (`Frame::regions`): the plane they lie on, the
//! profile the kernel fills, and their hatching.

use glam::{DVec2, DVec3};
use kernel_api::{Profile, ProfilePlane, ProfileSegment, ProfileWire};

/// The most hatch lines a region draws, however fine its spacing.
const HATCH_LIMIT: usize = 2000;

/// A region's plane: its first point, the way its outline turns, and a
/// first axis along its first side.
fn plane(region: &bench_api::Region) -> Option<ProfilePlane> {
    let points: Vec<DVec3> = region
        .outline
        .iter()
        .map(|p| DVec3::from_array(p.map(f64::from)))
        .collect();
    let origin = *points.first()?;
    // Newell's normal: the outline's area vector, sound for any simple
    // outline however its corners turn.
    let mut normal = DVec3::ZERO;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        normal += a.cross(*b);
    }
    let normal = normal.try_normalize()?;
    let x_axis = points
        .iter()
        .map(|p| *p - origin)
        .map(|v| v - normal * v.dot(normal))
        .find_map(DVec3::try_normalize)?;
    Some(ProfilePlane {
        origin: origin.to_array(),
        x_axis: x_axis.to_array(),
        y_axis: normal.cross(x_axis).to_array(),
        normal: normal.to_array(),
    })
}

fn to_plane(plane: &ProfilePlane, p: [f32; 3]) -> DVec2 {
    let d = DVec3::from_array(p.map(f64::from)) - DVec3::from_array(plane.origin);
    DVec2::new(
        d.dot(DVec3::from_array(plane.x_axis)),
        d.dot(DVec3::from_array(plane.y_axis)),
    )
}

fn from_plane(plane: &ProfilePlane, p: DVec2) -> [f32; 3] {
    (DVec3::from_array(plane.origin)
        + DVec3::from_array(plane.x_axis) * p.x
        + DVec3::from_array(plane.y_axis) * p.y)
        .as_vec3()
        .to_array()
}

/// The region's loops in its plane: the outline, then each hole.
fn loops(region: &bench_api::Region, plane: &ProfilePlane) -> Vec<Vec<DVec2>> {
    std::iter::once(&region.outline)
        .chain(&region.holes)
        .map(|ring| ring.iter().map(|p| to_plane(plane, *p)).collect::<Vec<_>>())
        .filter(|ring| ring.len() >= 3)
        .collect()
}

/// The region as a profile for the kernel to fill.
pub(crate) fn profile(region: &bench_api::Region) -> Option<Profile> {
    let plane = plane(region)?;
    let wires = loops(region, &plane)
        .into_iter()
        .map(|ring| {
            let segments = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .filter(|(a, b)| a.distance(**b) > 1e-9)
                .map(|(a, b)| ProfileSegment::Line {
                    start: a.to_array(),
                    end: b.to_array(),
                })
                .collect();
            ProfileWire::new(segments)
        })
        .collect();
    Some(Profile { plane, wires })
}

/// The hatch lines inside the region, each its two ends: lines
/// `hatch` apart at `hatch_angle` in its plane, cut where they cross its
/// loops and kept inside (even-odd, holes left out).
pub(crate) fn hatch(region: &bench_api::Region) -> Vec<([f32; 3], [f32; 3])> {
    let Some(spacing) = region.hatch.map(f64::from).filter(|s| *s > 0.0) else {
        return Vec::new();
    };
    let Some(plane) = plane(region) else {
        return Vec::new();
    };
    let loops = loops(region, &plane);
    let angle = f64::from(region.hatch_angle).to_radians();
    let along = DVec2::new(angle.cos(), angle.sin());
    let across = DVec2::new(-along.y, along.x);
    let offsets = loops.iter().flatten().map(|p| p.dot(across));
    let (lo, hi) = offsets.fold((f64::MAX, f64::MIN), |(lo, hi), o| (lo.min(o), hi.max(o)));
    if lo > hi {
        return Vec::new();
    }
    let first = (lo / spacing).ceil() as i64;
    let last = (hi / spacing).floor() as i64;
    let mut out = Vec::new();
    for k in first..=last {
        if out.len() >= HATCH_LIMIT {
            break;
        }
        let offset = k as f64 * spacing;
        // Where the line `p · across = offset` crosses each side, as a
        // distance along it.
        let mut crossings: Vec<f64> = loops
            .iter()
            .flat_map(|ring| ring.iter().zip(ring.iter().cycle().skip(1)))
            .filter_map(|(a, b)| {
                let (da, db) = (a.dot(across) - offset, b.dot(across) - offset);
                // Half-open, so a line through a corner counts it once.
                ((da > 0.0) != (db > 0.0)).then(|| {
                    let t = da / (da - db);
                    (*a + (*b - *a) * t).dot(along)
                })
            })
            .collect();
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.as_chunks::<2>().0 {
            let point = |s: f64| from_plane(&plane, across * offset + along * s);
            out.push((point(pair[0]), point(pair[1])));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square with a square hole, hatched across: every line inside the
    /// square and none across the hole.
    #[test]
    fn a_hatch_stays_inside_the_outline_and_out_of_its_holes() {
        let square =
            |lo: f32, hi: f32| vec![[lo, lo, 2.0], [hi, lo, 2.0], [hi, hi, 2.0], [lo, hi, 2.0]];
        let region = bench_api::Region {
            outline: square(0.0, 10.0),
            holes: vec![square(4.0, 6.0)],
            hatch: Some(1.0),
            hatch_angle: 90.0,
            ..Default::default()
        };
        let lines = hatch(&region);
        assert!(!lines.is_empty());
        let inside =
            |p: [f32; 3]| (-1e-4..=10.0001).contains(&p[0]) && (-1e-4..=10.0001).contains(&p[1]);
        for (a, b) in &lines {
            assert!(inside(*a) && inside(*b) && a[2] == 2.0, "{a:?} {b:?}");
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let in_hole = (4.0..6.0).contains(&mid[0]) && (4.0..6.0).contains(&mid[1]);
            assert!(!in_hole, "{a:?} {b:?}");
        }
        // A line through the hole splits in two around it.
        let through_hole = lines
            .iter()
            .filter(|(a, _)| (a[0] - 5.0).abs() < 1e-3)
            .count();
        assert_eq!(through_hole, 2);
        let profile = profile(&region).unwrap();
        assert_eq!(profile.wires.len(), 2);
        assert!((profile.plane.normal[2].abs() - 1.0).abs() < 1e-9);
    }
}
