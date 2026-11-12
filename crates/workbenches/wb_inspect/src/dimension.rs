//! A dimension between elements: a distance, an angle or a diameter read
//! from the elements' fitted shapes whenever it is asked, so a refitted
//! element moves every dimension on it. A dimension keeps a nominal and a
//! tolerance band; with a band it passes or fails.

use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchId};
use kernel_api::{FittedShape, TriMesh};
use serde::{Deserialize, Serialize};

use glam::DVec3;

use crate::element::{Element, Span};
use crate::vec::{basis, unit};

/// The feature kind of every Inspection dimension.
pub const KIND: &str = "wb.inspect.dimension";

/// Planes or axes further apart than this, in degrees, get no distance.
pub const PARALLEL_DEG: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DimKind {
    Distance,
    Angle,
    Diameter,
}

impl DimKind {
    pub const ALL: [DimKind; 3] = [DimKind::Distance, DimKind::Angle, DimKind::Diameter];

    pub fn word(self) -> &'static str {
        match self {
            DimKind::Distance => "Distance",
            DimKind::Angle => "Angle",
            DimKind::Diameter => "Diameter",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            DimKind::Distance => "constraint-distance",
            DimKind::Angle => "constraint-angle",
            DimKind::Diameter => "constraint-diameter",
        }
    }

    /// How many elements it is measured between.
    pub fn operands(self) -> usize {
        match self {
            DimKind::Diameter => 1,
            _ => 2,
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            DimKind::Angle => "°",
            _ => "mm",
        }
    }
}

/// One dimension of an inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub name: String,
    /// The scan its elements were fitted on.
    pub scan: BodyId,
    pub kind: DimKind,
    /// The elements it is measured between, in order: a distance is
    /// measured from the middle of the first.
    pub elements: Vec<FeatureId>,
    /// What it should read: the design value nearest the first reading
    /// until set.
    pub nominal: f64,
    /// How far above and below the nominal it may read; with both zero it
    /// has no band and neither passes nor fails.
    #[serde(default)]
    pub plus: f64,
    #[serde(default)]
    pub minus: f64,
}

impl WorkbenchFeature for Dimension {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        self.elements.clone()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// What an element stands for in a dimension.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    /// A sphere's, circle's or torus's centre.
    Point(DVec3),
    /// An axis through `at` along unit `dir`, measured over `along`.
    Axis {
        at: DVec3,
        dir: DVec3,
        along: [f64; 2],
    },
    /// A plane through the middle of its measured patch, with the band
    /// its points fill.
    Plane {
        middle: DVec3,
        normal: DVec3,
        form: (f64, f64),
    },
}

/// What `e` stands for; an axis or plane with no direction stands for
/// nothing.
fn role(e: &Element) -> Result<Role, String> {
    let r = &e.result;
    let along = match r.span {
        Span::Axis(t) => t,
        _ => [0.0, 0.0],
    };
    let unit = |v: [f64; 3]| unit(v.into()).ok_or_else(|| no_direction(e));
    Ok(match r.shape {
        FittedShape::Sphere { centre, .. }
        | FittedShape::Circle { centre, .. }
        | FittedShape::Torus { centre, .. } => Role::Point(centre.into()),
        FittedShape::Cylinder { point, axis, .. } => Role::Axis {
            at: point.into(),
            dir: unit(axis)?,
            along,
        },
        FittedShape::Cone { apex, axis, .. } => Role::Axis {
            at: apex.into(),
            dir: unit(axis)?,
            along,
        },
        FittedShape::Line { point, direction } => Role::Axis {
            at: point.into(),
            dir: unit(direction)?,
            along,
        },
        FittedShape::Plane { origin, normal } => {
            let (origin, normal) = (DVec3::from(origin), unit(normal)?);
            let middle = match r.span {
                Span::Patch { u, across } => {
                    let u = DVec3::from(u);
                    let v = normal.cross(u);
                    let mid = |[a, b]: [f64; 2]| (a + b) / 2.0;
                    origin + (u * mid(across[0]) + v * mid(across[1]))
                }
                _ => origin,
            };
            Role::Plane {
                middle,
                normal,
                form: r.form,
            }
        }
    })
}

fn no_direction(e: &Element) -> String {
    format!("{} has no direction", e.name)
}

/// The direction an element gives an angle: an axis, a plane's normal, a
/// circle's or torus's axis.
fn direction(e: &Element) -> Result<(DVec3, bool), String> {
    let (v, plane) = match e.result.shape {
        FittedShape::Plane { normal, .. } => (normal, true),
        FittedShape::Cylinder { axis, .. }
        | FittedShape::Cone { axis, .. }
        | FittedShape::Torus { axis, .. }
        | FittedShape::Line {
            direction: axis, ..
        } => (axis, false),
        FittedShape::Circle { normal, .. } => (normal, false),
        FittedShape::Sphere { .. } => return Err(no_direction(e)),
    };
    Ok((unit(v.into()).ok_or_else(|| no_direction(e))?, plane))
}

/// A dimension read off its elements.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub value: f64,
    /// Between two points: the second less the first, along X, Y and Z of
    /// the scan's frame.
    pub deltas: Option<[f64; 3]>,
    /// Between planes, and a diameter: the smallest and largest the
    /// surfaces themselves give, form included.
    pub range: Option<(f64, f64)>,
    /// Where its line runs, in the scan's frame.
    pub from: [f64; 3],
    pub to: [f64; 3],
}

/// Read `kind` off `elements`.
pub fn read(kind: DimKind, elements: &[Element]) -> Result<Reading, String> {
    if elements.len() != kind.operands() {
        return Err(format!(
            "a {} takes {} element{}",
            kind.word().to_lowercase(),
            kind.operands(),
            if kind.operands() == 1 { "" } else { "s" }
        ));
    }
    match kind {
        DimKind::Diameter => diameter(&elements[0]),
        DimKind::Angle => angle(&elements[0], &elements[1]),
        DimKind::Distance => distance(&elements[0], &elements[1]),
    }
}

fn diameter(e: &Element) -> Result<Reading, String> {
    let (lo, hi) = e
        .result
        .diameters()
        .ok_or_else(|| format!("{} has no diameter", e.name))?;
    let (centre, normal, radius) = match e.result.shape {
        FittedShape::Sphere { centre, radius } => (centre.into(), DVec3::Z, radius),
        FittedShape::Circle {
            centre,
            normal,
            radius,
        } => (centre.into(), normal.into(), radius),
        FittedShape::Cylinder {
            point,
            axis,
            radius,
        } => {
            let t = match e.result.span {
                Span::Axis([a, b]) => (a + b) / 2.0,
                _ => 0.0,
            };
            let axis = DVec3::from(axis);
            (DVec3::from(point) + axis * t, axis, radius)
        }
        _ => return Err(format!("{} has no diameter", e.name)),
    };
    let (u, _) = basis(normal).ok_or_else(|| no_direction(e))?;
    Ok(Reading {
        value: 2.0 * radius,
        deltas: None,
        range: Some((lo, hi)),
        from: (centre + u * -radius).to_array(),
        to: (centre + u * radius).to_array(),
    })
}

fn angle(a: &Element, b: &Element) -> Result<Reading, String> {
    let ((da, plane_a), (db, plane_b)) = (direction(a)?, direction(b)?);
    let c = da.dot(db).clamp(-1.0, 1.0);
    let value = match (plane_a, plane_b) {
        (true, true) => c.acos().to_degrees(),
        (false, false) => c.abs().acos().to_degrees(),
        _ => 90.0 - c.abs().acos().to_degrees(),
    };
    let (from, to) = (centre_of(a)?, centre_of(b)?);
    Ok(Reading {
        value,
        deltas: None,
        range: None,
        from: from.to_array(),
        to: to.to_array(),
    })
}

fn distance(a: &Element, b: &Element) -> Result<Reading, String> {
    let (ra, rb) = (role(a)?, role(b)?);
    let parallel = |u: DVec3, v: DVec3| u.dot(v).abs() >= PARALLEL_DEG.to_radians().cos();
    let square = |u: DVec3, v: DVec3| u.dot(v).abs() <= PARALLEL_DEG.to_radians().sin();
    let not_parallel = || {
        format!(
            "{} and {} are more than {PARALLEL_DEG}° from parallel: measure the angle",
            a.name, b.name
        )
    };
    let plain = |from: DVec3, to: DVec3, value: f64| Reading {
        value,
        deltas: None,
        range: None,
        from: from.to_array(),
        to: to.to_array(),
    };
    Ok(match (ra, rb) {
        (Role::Point(p), Role::Point(q)) => {
            let d = q - p;
            Reading {
                deltas: Some(d.to_array()),
                ..plain(p, q, d.length())
            }
        }
        (Role::Point(p), Role::Axis { at, dir, .. }) => {
            let foot = foot_on_line(p, at, dir);
            plain(p, foot, (p - foot).length())
        }
        (Role::Axis { at, dir, .. }, Role::Point(p)) => {
            let foot = foot_on_line(p, at, dir);
            plain(foot, p, (p - foot).length())
        }
        (Role::Point(p), Role::Plane { middle, normal, .. }) => {
            let d = (p - middle).dot(normal);
            plain(p, p - normal * d, d)
        }
        (Role::Plane { middle, normal, .. }, Role::Point(p)) => {
            let d = (p - middle).dot(normal);
            plain(p - normal * d, p, d)
        }
        (
            Role::Axis {
                at: pa,
                dir: da,
                along,
            },
            Role::Axis {
                at: pb, dir: db, ..
            },
        ) => {
            if parallel(da, db) {
                let mid = pa + da * ((along[0] + along[1]) / 2.0);
                let foot = foot_on_line(mid, pb, db);
                plain(mid, foot, (mid - foot).length())
            } else {
                let (p, q) = closest_points(pa, da, pb, db);
                plain(p, q, (q - p).length())
            }
        }
        (Role::Axis { at, dir, along }, Role::Plane { middle, normal, .. }) => {
            if !square(dir, normal) {
                return Err(not_parallel());
            }
            let mid = at + dir * ((along[0] + along[1]) / 2.0);
            let d = (mid - middle).dot(normal);
            plain(mid, mid - normal * d, d.abs())
        }
        (Role::Plane { middle, normal, .. }, Role::Axis { at, dir, along }) => {
            if !square(dir, normal) {
                return Err(not_parallel());
            }
            let mid = at + dir * ((along[0] + along[1]) / 2.0);
            let d = (mid - middle).dot(normal);
            plain(mid - normal * d, mid, d.abs())
        }
        (
            Role::Plane {
                middle: ma,
                normal: na,
                form: fa,
            },
            Role::Plane {
                middle: mb,
                normal: nb,
                form: fb,
            },
        ) => {
            if !parallel(na, nb) {
                return Err(not_parallel());
            }
            // The first plane's middle, from the second, along the second's
            // normal; each surface's points lie their form's band off it.
            let d = (ma - mb).dot(nb);
            let s = na.dot(nb).signum();
            let (a_lo, a_hi) = if s > 0.0 { fa } else { (-fa.1, -fa.0) };
            let (lo, hi) = (d + a_lo - fb.1, d + a_hi - fb.0);
            let foot = ma - nb * d;
            let range = if d >= 0.0 { (lo, hi) } else { (-hi, -lo) };
            Reading {
                range: Some(range),
                ..plain(ma, foot, d.abs())
            }
        }
    })
}

/// Where an element's lines are centred, for an angle's leader.
fn centre_of(e: &Element) -> Result<DVec3, String> {
    Ok(match role(e)? {
        Role::Point(p) => p,
        Role::Axis { at, dir, along } => at + dir * ((along[0] + along[1]) / 2.0),
        Role::Plane { middle, .. } => middle,
    })
}

fn foot_on_line(p: DVec3, at: DVec3, dir: DVec3) -> DVec3 {
    at + dir * (p - at).dot(dir)
}

/// The closest points of two lines that are not parallel.
fn closest_points(pa: DVec3, da: DVec3, pb: DVec3, db: DVec3) -> (DVec3, DVec3) {
    let w = pa - pb;
    let (b, d, e) = (da.dot(db), da.dot(w), db.dot(w));
    let den = (1.0 - b * b).max(1e-12);
    let (s, t) = ((b * e - d) / den, (e - b * d) / den);
    (pa + da * s, pb + db * t)
}

impl Dimension {
    /// Whether it has a band to pass or fail.
    pub fn banded(&self) -> bool {
        self.plus > 0.0 || self.minus > 0.0
    }

    /// Whether `value` lies in its band; `None` without one.
    pub fn passes(&self, value: f64) -> Option<bool> {
        let off = value - self.nominal;
        self.banded()
            .then(|| off <= self.plus + 1e-9 && off >= -self.minus - 1e-9)
    }

    /// The label it shows: the reading, and the deviation from its
    /// nominal when it has a band; a length in `unit`.
    pub fn label(&self, reading: &Reading, unit: core_document::Unit) -> String {
        let prefix = if self.kind == DimKind::Diameter {
            "Ø "
        } else {
            ""
        };
        let off = reading.value - self.nominal;
        let (value, off) = if self.kind == DimKind::Angle {
            (format!("{:.3} °", reading.value), format!("{off:+.3}"))
        } else {
            (
                core_document::units::show_mm(reading.value, unit),
                core_document::units::show_mm_signed(off, unit),
            )
        };
        let mut text = format!("{prefix}{value}");
        if let Some(pass) = self.passes(reading.value) {
            text.push_str(&format!("  {off} {}", if pass { "PASS" } else { "FAIL" }));
        }
        text
    }
}

/// The line a dimension draws, from one element to the other.
pub fn line(reading: &Reading) -> TriMesh {
    let mut mesh = TriMesh::default();
    for p in [reading.from, reading.to] {
        mesh.positions.push(p.map(|c| c as f32));
        mesh.normals.push([0.0, 0.0, 1.0]);
    }
    mesh.edges.extend([0, 1]);
    mesh
}

/// A design value for the first reading, to start the nominal at.
pub fn design_value(kind: DimKind, value: f64) -> f64 {
    match kind {
        DimKind::Angle => metrology::design::angle_deg(value),
        _ => metrology::design::length_mm(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Fitted;
    use kernel_api::FitKind;

    fn element(shape: FittedShape, span: Span, form: (f64, f64)) -> Element {
        Element {
            method: Default::default(),
            name: "e".into(),
            scan: BodyId(uuid::Uuid::nil()),
            kind: FitKind::Plane,
            cutoff: None,
            region: Vec::new(),
            revision: 0,
            result: Fitted {
                zone: None,
                shape,
                sigma: 0.0,
                form,
                used: 0,
                points: 0,
                span,
            },
        }
    }

    fn plane(z: f64, normal: [f64; 3], form: (f64, f64)) -> Element {
        let span = crate::element::span_of(
            &FittedShape::Plane {
                origin: [0.0, 0.0, z],
                normal,
            },
            &[[-1.0, -1.0, z], [1.0, 1.0, z]],
        );
        element(
            FittedShape::Plane {
                origin: [0.0, 0.0, z],
                normal,
            },
            span,
            form,
        )
    }

    /// Each reading at its closed form: planes 10 apart with their
    /// surfaces' range, centres with their deltas, an axis to a plane,
    /// angles in their ranges, a diameter with its band; planes 30° apart
    /// get an angle but no distance, a plane with no normal gets neither,
    /// and a band passes and fails.
    #[test]
    fn dimensions_read_their_closed_forms() {
        let up = [0.0, 0.0, 1.0];
        let tilted = [0.0, 0.5, 0.75f64.sqrt()];
        let (floor, roof) = (plane(0.0, up, (-0.1, 0.2)), plane(10.0, up, (-0.05, 0.05)));
        let ball = |c| {
            element(
                FittedShape::Sphere {
                    centre: c,
                    radius: 2.0,
                },
                Span::None,
                (-0.01, 0.02),
            )
        };
        let bore = element(
            FittedShape::Cylinder {
                point: [3.0, 0.0, 0.0],
                axis: [1.0, 0.0, 0.0],
                radius: 4.0,
            },
            Span::Axis([0.0, 6.0]),
            (-0.1, 0.1),
        );
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let d = read(DimKind::Distance, &[roof.clone(), floor.clone()]).unwrap();
        assert!(close(d.value, 10.0));
        let (lo, hi) = d.range.unwrap();
        assert!(close(lo, 10.0 - 0.05 - 0.2) && close(hi, 10.0 + 0.05 + 0.1));
        let d = read(DimKind::Distance, &[ball([0.0; 3]), ball([3.0, 4.0, 0.0])]).unwrap();
        assert!(close(d.value, 5.0) && d.deltas == Some([3.0, 4.0, 0.0]));
        let d = read(DimKind::Distance, &[bore.clone(), roof.clone()]).unwrap();
        assert!(close(d.value, 10.0) && close(d.from[0], 6.0));
        let angle = |a: &Element, b: &Element| read(DimKind::Angle, &[a.clone(), b.clone()]);
        assert!(close(angle(&bore, &floor).unwrap().value, 0.0));
        let slope = plane(0.0, tilted, (0.0, 0.0));
        assert!(close(angle(&floor, &slope).unwrap().value, 30.0));
        assert!(read(DimKind::Distance, &[floor.clone(), slope]).is_err());
        assert!(angle(&ball([0.0; 3]), &floor).is_err());
        // A plane with no normal is refused, not read as a direction.
        let flat = plane(0.0, [0.0; 3], (0.0, 0.0));
        for kind in [DimKind::Distance, DimKind::Angle] {
            let refused = read(kind, &[roof.clone(), flat.clone()]).unwrap_err();
            assert_eq!(refused, "e has no direction");
        }
        let d = read(DimKind::Diameter, std::slice::from_ref(&bore)).unwrap();
        assert!(close(d.value, 8.0) && d.range == Some((7.8, 8.2)));

        let mut dim = Dimension {
            name: "Diameter 1".into(),
            scan: bore.scan,
            kind: DimKind::Diameter,
            elements: Vec::new(),
            nominal: design_value(DimKind::Diameter, 8.04),
            plus: 0.0,
            minus: 0.0,
        };
        assert_eq!((dim.nominal, dim.passes(8.04)), (8.0, None));
        dim.plus = 0.05;
        assert_eq!(
            (dim.passes(8.04), dim.passes(8.06)),
            (Some(true), Some(false))
        );
        assert_eq!(
            dim.label(&d, core_document::Unit::Mm),
            "Ø 8.000 mm  +0.000 mm PASS"
        );
        assert_eq!(
            dim.label(&d, core_document::Unit::In),
            "Ø 0.3150 in  +0.0000 in PASS"
        );
    }
}
