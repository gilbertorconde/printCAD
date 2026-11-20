//! Renderer-independent rays and scene intersection primitives.

use glam::Vec3;

/// Normalized world-space ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        assert!(origin.is_finite(), "ray origin must be finite: {origin:?}");
        assert!(
            direction.is_finite(),
            "ray direction must be finite: {direction:?}"
        );
        let direction = direction.normalize();
        assert!(!direction.is_nan(), "ray direction must be non-zero");
        Self { origin, direction }
    }

    pub fn point_at(self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }

    /// Where the ray meets the plane through `plane_origin` square to
    /// `plane_normal`; `None` when it runs along the plane (within 1e-6 of
    /// square to the normal) or the plane lies behind it.
    pub fn plane_hit(self, plane_origin: Vec3, plane_normal: Vec3) -> Option<Vec3> {
        let normal = plane_normal.normalize();
        let denom = self.direction.dot(normal);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (plane_origin - self.origin).dot(normal) / denom;
        if t < 0.0 {
            return None;
        }
        Some(self.origin + self.direction * t)
    }
}

/// Closest path point to a ray, ranked by angular rather than metric miss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayApproach {
    pub index: usize,
    pub along: f32,
    pub offset: f32,
}

pub fn nearest_point_to_ray(points: &[Vec3], ray: Ray) -> Option<RayApproach> {
    points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let from_origin = *point - ray.origin;
            let along = from_origin.dot(ray.direction);
            if along <= 0.0 || !along.is_finite() {
                return None;
            }
            let offset = (from_origin.length_squared() - along * along)
                .clamp(0.0, f32::INFINITY)
                .sqrt();
            offset.is_finite().then_some(RayApproach {
                index,
                along,
                offset,
            })
        })
        .min_by(|a, b| (a.offset / a.along).total_cmp(&(b.offset / b.along)))
}

/// Closest point on a polyline axis, ranked by angular miss. Unlike the
/// station-only query this also picks the middle of a long segment. The host
/// applies its screen-space snap threshold; this helper owns only ray math.
pub fn nearest_polyline_point(points: &[Vec3], ray: Ray) -> Option<Vec3> {
    if points.len() == 1 {
        return nearest_point_to_ray(points, ray).map(|hit| points[hit.index]);
    }
    points
        .windows(2)
        .filter_map(|segment| {
            let approach = closest_ray_segment(ray, segment[0], segment[1])?;
            (approach.along > 0.0).then(|| {
                (
                    approach.separation / approach.along,
                    segment[0].lerp(segment[1], approach.segment_t),
                )
            })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, point)| point)
}

#[cfg(test)]
mod polyline_anchor_tests {
    use super::*;
    #[test]
    fn picks_segment_middle_and_rejects_empty_or_behind() {
        let ray = Ray::new(Vec3::ZERO, Vec3::Z);
        let points = [Vec3::new(-100.0, 0.0, 10.0), Vec3::new(100.0, 0.0, 10.0)];
        assert_eq!(nearest_polyline_point(&points, ray), Some(Vec3::Z * 10.0));
        assert_eq!(nearest_polyline_point(&[], ray), None);
        assert_eq!(
            nearest_polyline_point(&[Vec3::NEG_Z, Vec3::NEG_Z * 2.0], ray),
            None
        );
        assert_eq!(nearest_polyline_point(&[Vec3::Z], ray), Some(Vec3::Z));
    }
}

/// Intersection of a ray with a variable-radius polyline tube.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolylineHit {
    /// Closest source point; stable key for MD, diameter, and Frenet arrays.
    pub point_index: usize,
    /// Source segment containing the closest point on the path.
    pub segment_index: usize,
    /// First approximate surface contact along the ray.
    pub distance: f32,
    pub position: Vec3,
    /// Interpolation coordinate on `[segment_index, segment_index + 1]`.
    pub segment_t: f32,
}

/// Nearest points between a forward ray and a finite line segment.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RaySegmentApproach {
    /// Distance from the ray origin to the nearest point on the ray.
    along: f32,
    /// Interpolation coordinate of the nearest point on the segment.
    segment_t: f32,
    /// Euclidean distance between the two nearest points.
    separation: f32,
}

/// Raycast against capsules around every path segment.
///
/// This is deliberately a path-level primitive rather than triangle picking:
/// complexity is O(path points), independent of radial mesh tessellation, and
/// both the current and a future renderer get identical hover semantics.
pub fn raycast_variable_radius_polyline(
    points: &[Vec3],
    radii: &[f32],
    ray: Ray,
) -> Option<PolylineHit> {
    assert_eq!(points.len(), radii.len(), "points vs radii");
    raycast_variable_radius_polyline_by(points, ray, |index| radii[index])
}

/// Allocation-free variant for radii stored in domain-specific unit types.
pub fn raycast_variable_radius_polyline_by(
    points: &[Vec3],
    ray: Ray,
    mut radius_at: impl FnMut(usize) -> f32,
) -> Option<PolylineHit> {
    if points.is_empty() {
        return None;
    }
    if points.len() == 1 {
        return ray_sphere(ray, points[0], radius_at(0)).map(|distance| PolylineHit {
            point_index: 0,
            segment_index: 0,
            distance,
            position: ray.point_at(distance),
            segment_t: 0.0,
        });
    }

    points
        .windows(2)
        .enumerate()
        .filter_map(|(segment_index, segment)| {
            let approach = closest_ray_segment(ray, segment[0], segment[1])?;
            let start_radius = radius_at(segment_index);
            let radius =
                start_radius + (radius_at(segment_index + 1) - start_radius) * approach.segment_t;
            if !radius.is_finite() || radius < 0.0 || approach.separation > radius {
                return None;
            }
            // Move from the closest approach towards the eye to approximate the
            // first capsule-surface contact. It is exact for the local cylinder.
            let half_chord_sq = radius * radius - approach.separation * approach.separation;
            if !half_chord_sq.is_finite() {
                return None;
            }
            let half_chord = half_chord_sq.clamp(0.0, f32::INFINITY).sqrt();
            let distance = (approach.along - half_chord).max(0.0);
            if !distance.is_finite() {
                return None;
            }
            let position = ray.point_at(distance);
            if !position.is_finite() {
                return None;
            }
            Some(PolylineHit {
                point_index: segment_index + usize::from(approach.segment_t >= 0.5),
                segment_index,
                distance,
                position,
                segment_t: approach.segment_t,
            })
        })
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
}

fn closest_ray_segment(ray: Ray, start: Vec3, end: Vec3) -> Option<RaySegmentApproach> {
    if !start.is_finite() || !end.is_finite() {
        return None;
    }
    let segment = end - start;
    let segment_len_sq = segment.length_squared();
    if !segment_len_sq.is_finite() {
        return None;
    }
    if segment_len_sq <= f32::EPSILON {
        let from_origin = start - ray.origin;
        let along = from_origin.dot(ray.direction).max(0.0);
        let approach = RaySegmentApproach {
            along,
            segment_t: 0.0,
            separation: ray.point_at(along).distance(start),
        };
        return approach.is_finite().then_some(approach);
    }

    let origin_to_start = ray.origin - start;
    let ray_segment_dot = ray.direction.dot(segment);
    let ray_origin_dot = ray.direction.dot(origin_to_start);
    let segment_origin_dot = segment.dot(origin_to_start);
    let denominator = segment_len_sq - ray_segment_dot * ray_segment_dot;

    let mut segment_t = if denominator > f32::EPSILON {
        (segment_origin_dot - ray_segment_dot * ray_origin_dot) / denominator
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    let mut along = ray_segment_dot * segment_t - ray_origin_dot;
    if along < 0.0 {
        along = 0.0;
        segment_t = (-segment_origin_dot / segment_len_sq).clamp(0.0, 1.0);
    }

    let ray_point = ray.point_at(along);
    let segment_point = start + segment * segment_t;
    let approach = RaySegmentApproach {
        along,
        segment_t,
        separation: ray_point.distance(segment_point),
    };
    approach.is_finite().then_some(approach)
}

impl RaySegmentApproach {
    fn is_finite(self) -> bool {
        self.along.is_finite() && self.segment_t.is_finite() && self.separation.is_finite()
    }
}

fn ray_sphere(ray: Ray, center: Vec3, radius: f32) -> Option<f32> {
    if !radius.is_finite() || radius < 0.0 {
        return None;
    }
    let to_center = center - ray.origin;
    let projected = to_center.dot(ray.direction);
    let perpendicular_sq = to_center.length_squared() - projected * projected;
    let half_chord_sq = radius * radius - perpendicular_sq;
    if half_chord_sq < 0.0 {
        return None;
    }
    let near = projected - half_chord_sq.sqrt();
    let far = projected + half_chord_sq.sqrt();
    (far >= 0.0).then_some(near.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angular_approach_prefers_the_crosshair() {
        let points = [Vec3::new(10.0, 0.0, 100.0), Vec3::new(30.0, 0.0, 1000.0)];
        let approach = nearest_point_to_ray(&points, Ray::new(Vec3::ZERO, Vec3::Z)).unwrap();
        assert_eq!(approach.index, 1);
    }

    #[test]
    fn polyline_hit_uses_visible_radius_and_nearest_depth() {
        let ray = Ray::new(Vec3::ZERO, Vec3::Z);
        let points = [
            Vec3::new(-1.0, 0.0, 10.0),
            Vec3::new(1.0, 0.0, 10.0),
            Vec3::new(1.0, 0.0, 30.0),
        ];
        let hit = raycast_variable_radius_polyline(&points, &[2.0, 2.0, 2.0], ray).unwrap();
        assert!(hit.distance < 10.0);
        assert!(hit.position.z < 10.0);
    }

    #[test]
    fn polyline_miss_and_behind_camera_are_rejected() {
        let ray = Ray::new(Vec3::ZERO, Vec3::Z);
        assert!(
            raycast_variable_radius_polyline(
                &[Vec3::new(10.0, 0.0, 10.0), Vec3::new(10.0, 1.0, 10.0)],
                &[1.0, 1.0],
                ray,
            )
            .is_none()
        );
        assert!(
            raycast_variable_radius_polyline(
                &[Vec3::new(-1.0, 0.0, -10.0), Vec3::new(1.0, 0.0, -10.0)],
                &[1.0, 1.0],
                ray,
            )
            .is_none()
        );
    }

    #[test]
    fn non_finite_segment_geometry_is_ignored_without_affecting_valid_hits() {
        let ray = Ray::new(Vec3::ZERO, Vec3::Z);
        let nan = f32::NAN;
        assert!(
            raycast_variable_radius_polyline(
                &[Vec3::new(-1.0, 0.0, 10.0), Vec3::new(1.0, nan, 10.0)],
                &[100.0, 100.0],
                ray,
            )
            .is_none()
        );

        // Finite inputs can still overflow when squared. That must not turn an
        // infinite half chord into a contact at distance zero.
        assert!(
            raycast_variable_radius_polyline(
                &[Vec3::new(-1.0, 0.0, 10.0), Vec3::new(1.0, 0.0, 10.0)],
                &[f32::MAX, f32::MAX],
                ray,
            )
            .is_none()
        );

        let hit = raycast_variable_radius_polyline(
            &[Vec3::new(-1.0, 0.0, 10.0), Vec3::new(1.0, 0.0, 10.0)],
            &[2.0, 2.0],
            ray,
        )
        .unwrap();
        assert!(hit.distance.is_finite());
        assert_eq!(hit.position, ray.point_at(hit.distance));
    }

    #[test]
    fn callback_radius_controls_the_visible_surface_without_allocating() {
        let points = [Vec3::new(1.5, 0.0, 10.0), Vec3::new(1.5, 1.0, 10.0)];
        let ray = Ray::new(Vec3::ZERO, Vec3::Z);

        assert!(raycast_variable_radius_polyline_by(&points, ray, |_| 1.0).is_none());
        let hit = raycast_variable_radius_polyline_by(&points, ray, |_| 2.0).unwrap();
        assert_eq!(hit.segment_index, 0);
        assert_eq!(hit.point_index, 0);
        assert!((hit.distance - (10.0 - (4.0_f32 - 2.25).sqrt())).abs() < 1e-5);
    }
}
