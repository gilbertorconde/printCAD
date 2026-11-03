//! Datum-referenced location, profile and runout zones, in millimetres.

use crate::{Vec3, cross, dot, norm, sub, unit};

/// A datum's location and orientation, independent of a fitted representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Datum {
    Plane { point: Vec3, normal: Vec3 },
    Axis { point: Vec3, direction: Vec3 },
}

/// A right-handed datum reference frame. Unconstrained freedoms retain
/// the coordinate system's origin and directions, projected onto the datums.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    pub axes: [Vec3; 3],
    pub free_translations: usize,
    pub free_rotations: usize,
}

impl Frame {
    pub fn identity() -> Self {
        Self {
            origin: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            free_translations: 3,
            free_rotations: 3,
        }
    }

    pub fn local(&self, p: Vec3) -> Vec3 {
        self.axes.map(|a| dot(sub(p, self.origin), a))
    }

    pub fn world(&self, p: Vec3) -> Vec3 {
        let mut out = self.origin;
        for (a, t) in self.axes.iter().zip(p) {
            out = add(out, scale(*a, t));
        }
        out
    }

    /// Primary fixes tilt and one plane translation or two axis
    /// translations. Later datums constrain only the remaining freedoms.
    /// The primary direction is Z; a later transverse direction sets X.
    pub fn from_datums(datums: &[Datum]) -> Option<Self> {
        if datums.len() > 3 {
            return None;
        }
        let Some(first) = datums.first() else {
            return Some(Self::identity());
        };
        let direction = |d: &Datum| match d {
            Datum::Plane { normal, .. } => *normal,
            Datum::Axis { direction, .. } => *direction,
        };
        let z = unit(direction(first))?;
        let transverse = datums.iter().skip(1).find_map(|d| {
            let a = unit(direction(d))?;
            unit_checked(sub(a, scale(z, dot(a, z)))).or_else(|| match (first, d) {
                (Datum::Axis { point: origin, .. }, Datum::Axis { point, .. }) => {
                    let offset = sub(*point, *origin);
                    unit_checked(sub(offset, scale(z, dot(offset, z))))
                }
                _ => None,
            })
        });
        let x = transverse.unwrap_or_else(|| perpendicular(z));
        let y = cross(z, x);
        let mut frame = Self {
            axes: [x, y, z],
            free_rotations: usize::from(transverse.is_none()),
            ..Self::identity()
        };
        let mut locked = Vec::<Vec3>::new();
        for datum in datums {
            let (point, normals) = match *datum {
                Datum::Plane { point, normal } => (point, vec![unit(normal)?]),
                Datum::Axis { point, direction } => {
                    let d = unit(direction)?;
                    let u = perpendicular(d);
                    (point, vec![u, cross(d, u)])
                }
            };
            if !point.iter().all(|v| v.is_finite()) {
                return None;
            }
            for n in normals {
                let mut free = n;
                for old in &locked {
                    free = sub(free, scale(*old, dot(free, *old)));
                }
                if let Some(free) = unit_checked(free) {
                    let step = dot(sub(point, frame.origin), n) / dot(free, n);
                    frame.origin = add(frame.origin, scale(free, step));
                    locked.push(free);
                }
            }
        }
        frame.free_translations = 3 - locked.len();
        Some(frame)
    }
}

/// Diameter of a fixed cylindrical zone containing points or axis ends.
pub fn position(points: &[Vec3], true_point: Vec3, direction: Vec3) -> Option<f64> {
    let d = unit(direction)?;
    maximum(
        points
            .iter()
            .map(|p| 2.0 * norm(cross(sub(*p, true_point), d))),
    )
}

/// Width of two planes equally disposed about the true plane.
pub fn plane_position(points: &[Vec3], true_point: Vec3, normal: Vec3) -> Option<f64> {
    let n = unit(normal)?;
    maximum(
        points
            .iter()
            .map(|p| 2.0 * dot(sub(*p, true_point), n).abs()),
    )
}

/// Width of a profile zone centred at `offset` from the nominal surface.
/// Unmeasured points invalidate a tolerance rather than silently passing.
pub fn profile(distances: &[f64], offset: f64) -> Option<f64> {
    maximum(distances.iter().map(|d| 2.0 * (d - offset).abs()))
}

/// Surface readout about an axis. Cone residuals remove the nominal radial
/// slope; axial runout uses height instead of radius.
#[derive(Debug, Clone, Copy)]
pub enum Runout {
    Radial { slope: f64 },
    Axial,
}

/// Runout of measured points. Circular runout groups stations into bands
/// of `section_width` (axial stations, or radial stations for a face),
/// taking the widest band's range; total runout takes the whole range.
/// A circular reading needs at least two points in every occupied band.
pub fn runout(
    points: &[Vec3],
    origin: Vec3,
    axis: Vec3,
    kind: Runout,
    section_width: Option<f64>,
) -> Option<f64> {
    let d = unit(axis)?;
    let mut bands = std::collections::BTreeMap::<i64, Vec<f64>>::new();
    if section_width.is_some_and(|w| !w.is_finite() || w <= 0.0) {
        return None;
    }
    for p in points {
        let v = sub(*p, origin);
        let height = dot(v, d);
        let radius = norm(cross(v, d));
        let (station, value) = match kind {
            Runout::Radial { slope } => (height, radius - slope * height),
            Runout::Axial => (radius, height),
        };
        if !station.is_finite() || !value.is_finite() {
            return None;
        }
        let band = section_width.map_or(0, |w| (station / w).round() as i64);
        bands.entry(band).or_default().push(value);
    }
    if bands.values().any(|v| v.len() < 2) {
        return None;
    }
    maximum(
        bands
            .values()
            .map(|v| crate::form::peak_to_peak(v).unwrap_or(f64::NAN)),
    )
}

/// The common axis connects the centres of two separated datum features.
/// Both feature directions must agree; an ambiguous centre pair is refused.
pub fn common_axis(a: Vec3, da: Vec3, b: Vec3, db: Vec3) -> Option<(Vec3, Vec3)> {
    let (da, db) = (unit(da)?, unit(db)?);
    let direction = unit_checked(sub(b, a))?;
    (dot(da, db).abs() > 0.999 && dot(direction, da).abs() > 0.9).then_some((a, direction))
}

/// Median points between opposed planes, projected along the datum's
/// normal. The samples locate the measured overlap; every pair is read
/// at the same transverse location, not by unrelated vertex indices.
pub fn symmetry(
    samples: &[Vec3],
    planes: [(Vec3, Vec3); 2],
    datum_point: Vec3,
    datum_normal: Vec3,
) -> Option<f64> {
    let d = unit(datum_normal)?;
    let [(a, na), (b, nb)] = planes;
    let (na, nb) = (unit(na)?, unit(nb)?);
    if dot(na, d).abs() < 1e-8 || dot(nb, d).abs() < 1e-8 || dot(na, nb).abs() < 0.9 {
        return None;
    }
    maximum(samples.iter().map(|p| {
        let h1 = dot(sub(a, *p), na) / dot(d, na);
        let h2 = dot(sub(b, *p), nb) / dot(d, nb);
        2.0 * (dot(sub(*p, datum_point), d) + (h1 + h2) / 2.0).abs()
    }))
}

/// Distance from the stated material boundary to the actual mating size.
pub fn bonus(actual: f64, boundary: f64) -> Option<f64> {
    (actual.is_finite() && boundary.is_finite() && actual > 0.0 && boundary > 0.0)
        .then_some((actual - boundary).abs())
}

fn maximum(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut out: Option<f64> = None;
    for v in values {
        if !v.is_finite() {
            return None;
        }
        out = Some(out.map_or(v, |old| old.max(v)));
    }
    out
}

fn unit_checked(v: Vec3) -> Option<Vec3> {
    (norm(v) > 1e-9).then(|| unit(v)).flatten()
}
fn perpendicular(z: Vec3) -> Vec3 {
    let x = if z[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    unit(sub(x, scale(z, dot(x, z)))).unwrap()
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn scale(a: Vec3, t: f64) -> Vec3 {
    a.map(|v| v * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datum_precedence_and_zones_have_hand_checked_values() {
        let (x, y, z) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        let datums = [
            Datum::Plane {
                point: [9.0, 8.0, 3.0],
                normal: z,
            },
            Datum::Plane {
                point: [1.0, 7.0, 9.0],
                normal: x,
            },
            Datum::Plane {
                point: [6.0, 2.0, 8.0],
                normal: y,
            },
        ];
        let f = Frame::from_datums(&datums).unwrap();
        assert_eq!(f.origin, [1.0, 2.0, 3.0]);
        assert_eq!((f.free_translations, f.free_rotations), (0, 0));
        assert_eq!(f.local(f.world([2.0, 3.0, 4.0])), [2.0, 3.0, 4.0]);
        let axis = Datum::Axis {
            point: [1.0, 2.0, 7.0],
            direction: z,
        };
        let f = Frame::from_datums(&[axis, datums[0]]).unwrap();
        assert_eq!(f.origin, [1.0, 2.0, 3.0]);
        assert_eq!((f.free_translations, f.free_rotations), (0, 1));
        let close = |v: Option<f64>, expected: f64| assert!((v.unwrap() - expected).abs() < 1e-10);
        close(
            position(&[[0.1, 0.0, 0.0], [0.2, 0.0, 10.0]], [0.0; 3], z),
            0.4,
        );
        close(plane_position(&[[0.0, 0.0, 0.03]], [0.0; 3], z), 0.06);
        close(profile(&[-0.02, 0.08], 0.03), 0.1);
        assert!(profile(&[f64::NAN], 0.0).is_none());
        close(
            symmetry(
                &[[0.0; 3]],
                [([0.0, 0.0, -2.0], z), ([0.0, 0.0, 2.1], z)],
                [0.0; 3],
                z,
            ),
            0.1,
        );
        close(bonus(10.2, 10.0), 0.2);
        let points = [
            [5.05, 0.0, 0.0],
            [-4.95, 0.0, 0.0],
            [5.15, 0.0, 10.0],
            [-5.05, 0.0, 10.0],
        ];
        close(
            runout(
                &points,
                [0.0; 3],
                z,
                Runout::Radial { slope: 0.0 },
                Some(1.0),
            ),
            0.1,
        );
        close(
            runout(&points, [0.0; 3], z, Runout::Radial { slope: 0.0 }, None),
            0.2,
        );
        close(
            runout(&points, [0.0; 3], z, Runout::Radial { slope: 0.01 }, None),
            0.1,
        );
        assert!(common_axis([0.0; 3], z, [0.0, 0.0, 10.0], z).is_some());
    }
}
