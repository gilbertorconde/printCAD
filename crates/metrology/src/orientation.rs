//! Orientation and location tolerances: the size of the narrowest zone,
//! set by the datum, that holds the feature. A plane is evaluated at its
//! measured points, an axis at its two ends over its fitted length, a
//! centre as a point.

use crate::{Vec3, cross, dot, norm, sub, unit};

/// A plane's parallelism to a datum plane (its `normal`) or axis (its
/// `direction`, which the plane runs along): the distance between the two
/// planes square to the datum's normal that hold every point. For an
/// axis datum the zone's normal is the plane's own, square to the axis.
pub fn plane_parallelism(points: &[Vec3], plane_normal: Vec3, datum: Datum) -> Option<f64> {
    let across = match datum {
        Datum::Plane(normal) => unit(normal)?,
        Datum::Axis(direction) => square_to(plane_normal, direction)?,
    };
    width_along(points, across)
}

/// A plane's perpendicularity to a datum: the zone is square to the datum
/// plane (or holds the datum axis direction) and faces as the plane does.
pub fn plane_perpendicularity(points: &[Vec3], plane_normal: Vec3, datum: Datum) -> Option<f64> {
    let across = match datum {
        Datum::Plane(normal) => square_to(plane_normal, normal)?,
        Datum::Axis(direction) => unit(direction)?,
    };
    width_along(points, across)
}

/// A plane's angularity to a datum plane at `angle` (radians) between
/// their normals: the zone's normal is the datum's turned by the angle
/// towards the plane's own normal.
pub fn plane_angularity(
    points: &[Vec3],
    plane_normal: Vec3,
    datum_normal: Vec3,
    angle: f64,
) -> Option<f64> {
    let d = unit(datum_normal)?;
    let toward = square_to(plane_normal, d)?;
    let across = add(d.map(|c| c * angle.cos()), toward.map(|c| c * angle.sin()));
    width_along(points, across)
}

/// An axis's parallelism to a datum direction: the diameter of the
/// smallest cylinder along the datum holding the axis between its `ends`.
pub fn axis_parallelism(ends: [Vec3; 2], datum_direction: Vec3) -> Option<f64> {
    let d = unit(datum_direction)?;
    let run = sub(ends[1], ends[0]);
    Some(norm(sub(run, d.map(|c| c * dot(run, d)))))
}

/// An axis's perpendicularity to a datum plane: its parallelism to the
/// plane's normal.
pub fn axis_perpendicularity(ends: [Vec3; 2], datum_normal: Vec3) -> Option<f64> {
    axis_parallelism(ends, datum_normal)
}

/// Coaxiality: twice the farthest the axis's `ends` stray from the datum
/// axis (through `datum_point` along `datum_direction`).
pub fn coaxiality(ends: [Vec3; 2], datum_point: Vec3, datum_direction: Vec3) -> Option<f64> {
    let a = distance_to_line(ends[0], datum_point, datum_direction)?;
    let b = distance_to_line(ends[1], datum_point, datum_direction)?;
    Some(2.0 * a.max(b))
}

/// Concentricity: twice the distance of a centre from the datum axis.
pub fn concentricity(centre: Vec3, datum_point: Vec3, datum_direction: Vec3) -> Option<f64> {
    Some(2.0 * distance_to_line(centre, datum_point, datum_direction)?)
}

/// What an orientation tolerance is taken against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Datum {
    /// A plane, by its normal.
    Plane(Vec3),
    /// An axis, by its direction.
    Axis(Vec3),
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `v` with its component along `along` taken out, made unit.
fn square_to(v: Vec3, along: Vec3) -> Option<Vec3> {
    let a = unit(along)?;
    unit(sub(v, a.map(|c| c * dot(v, a))))
}

fn width_along(points: &[Vec3], across: Vec3) -> Option<f64> {
    let heights: Vec<f64> = points.iter().map(|p| dot(*p, across)).collect();
    crate::form::peak_to_peak(&heights)
}

fn distance_to_line(p: Vec3, on: Vec3, direction: Vec3) -> Option<f64> {
    let d = unit(direction)?;
    Some(norm(cross(sub(p, on), d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square patch tilted by `rise` over 10 mm in X, its normal near Z.
    fn tilted(rise: f64) -> Vec<Vec3> {
        [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], [10.0, 10.0]]
            .map(|[x, y]| [x, y, rise * x / 10.0])
            .to_vec()
    }

    #[test]
    fn zones_read_their_closed_forms() {
        let z = [0.0, 0.0, 1.0];
        let x = [1.0, 0.0, 0.0];
        let close = |a: Option<f64>, b: f64| (a.unwrap() - b).abs() < 1e-12;
        // A face rising 0.1 over 10 mm is 0.1 out of parallel with XY.
        assert!(close(
            plane_parallelism(&tilted(0.1), z, Datum::Plane(z)),
            0.1
        ));
        // A wall facing X, leaning 0.2 in X over its 10 mm height, is 0.2
        // out of square with XY; the rising face is 0.1 out of square with
        // the Z axis.
        let wall: Vec<Vec3> = [[0.0, 0.0], [0.0, 10.0], [10.0, 0.0], [10.0, 10.0]]
            .map(|[y, h]| [0.2 * h / 10.0, y, h])
            .to_vec();
        assert!(close(
            plane_perpendicularity(&wall, x, Datum::Plane(z)),
            0.2
        ));
        assert!(close(
            plane_perpendicularity(&tilted(0.1), z, Datum::Axis(z)),
            0.1
        ));
        // A face at 45° to XY, exact, has no angularity error.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let slope: Vec<Vec3> = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]]
            .map(|[a, y]| [a, y, a])
            .to_vec();
        let angle = std::f64::consts::FRAC_PI_4;
        assert!(close(plane_angularity(&slope, [-s, 0.0, s], z, angle), 0.0));
        // An axis 20 long leaning 0.1 sideways: parallelism 0.1; offset
        // 0.05 from the datum at one end and 0.15 at the other: coaxiality 0.3.
        let ends = [[0.05, 0.0, 0.0], [0.15, 0.0, 20.0]];
        assert!(close(axis_parallelism(ends, z), 0.1));
        assert!(close(axis_perpendicularity(ends, z), 0.1));
        assert!(close(coaxiality(ends, [0.0; 3], z), 0.3));
        assert!(close(concentricity([0.0, 0.02, 7.0], [0.0; 3], z), 0.04));
        assert_eq!(axis_parallelism(ends, [0.0; 3]), None);
    }
}
