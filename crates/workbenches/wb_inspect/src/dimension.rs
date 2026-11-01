//! A dimension between elements: a distance, an angle or a diameter read
//! from the elements' fitted shapes whenever it is asked, so a refitted
//! element moves every dimension on it. A dimension keeps a nominal and a
//! tolerance band; with a band it passes or fails.

use core_document::{
    BodyId, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use kernel_api::{FittedShape, TriMesh};
use serde::{Deserialize, Serialize};

use crate::element::{Element, Span, add, cross, dot, scale, sub};

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

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(DocumentError::Serialization)
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
    Point([f64; 3]),
    /// An axis through `at` along unit `dir`, measured over `along`.
    Axis {
        at: [f64; 3],
        dir: [f64; 3],
        along: [f64; 2],
    },
    /// A plane through the middle of its measured patch, with the band
    /// its points fill.
    Plane {
        middle: [f64; 3],
        normal: [f64; 3],
        form: (f64, f64),
    },
}

fn role(e: &Element) -> Role {
    let r = &e.result;
    let along = match r.span {
        Span::Axis(t) => t,
        _ => [0.0, 0.0],
    };
    match r.shape {
        FittedShape::Sphere { centre, .. }
        | FittedShape::Circle { centre, .. }
        | FittedShape::Torus { centre, .. } => Role::Point(centre),
        FittedShape::Cylinder { point, axis, .. } => Role::Axis {
            at: point,
            dir: unit(axis),
            along,
        },
        FittedShape::Cone { apex, axis, .. } => Role::Axis {
            at: apex,
            dir: unit(axis),
            along,
        },
        FittedShape::Line { point, direction } => Role::Axis {
            at: point,
            dir: unit(direction),
            along,
        },
        FittedShape::Plane { origin, normal } => {
            let normal = unit(normal);
            let middle = match r.span {
                Span::Patch { u, across } => {
                    let v = cross(normal, u);
                    let mid = |[a, b]: [f64; 2]| (a + b) / 2.0;
                    add(
                        origin,
                        add(scale(u, mid(across[0])), scale(v, mid(across[1]))),
                    )
                }
                _ => origin,
            };
            Role::Plane {
                middle,
                normal,
                form: r.form,
            }
        }
    }
}

/// The direction an element gives an angle: an axis, a plane's normal, a
/// circle's or torus's axis.
fn direction(e: &Element) -> Option<([f64; 3], bool)> {
    match e.result.shape {
        FittedShape::Plane { normal, .. } => Some((unit(normal), true)),
        FittedShape::Cylinder { axis, .. }
        | FittedShape::Cone { axis, .. }
        | FittedShape::Torus { axis, .. }
        | FittedShape::Line {
            direction: axis, ..
        } => Some((unit(axis), false)),
        FittedShape::Circle { normal, .. } => Some((unit(normal), false)),
        FittedShape::Sphere { .. } => None,
    }
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
        FittedShape::Sphere { centre, radius } => (centre, [0.0, 0.0, 1.0], radius),
        FittedShape::Circle {
            centre,
            normal,
            radius,
        } => (centre, normal, radius),
        FittedShape::Cylinder {
            point,
            axis,
            radius,
        } => {
            let t = match e.result.span {
                Span::Axis([a, b]) => (a + b) / 2.0,
                _ => 0.0,
            };
            (add(point, scale(axis, t)), axis, radius)
        }
        _ => return Err(format!("{} has no diameter", e.name)),
    };
    let (u, _) = crate::element::basis(normal);
    Ok(Reading {
        value: 2.0 * radius,
        deltas: None,
        range: Some((lo, hi)),
        from: add(centre, scale(u, -radius)),
        to: add(centre, scale(u, radius)),
    })
}

fn angle(a: &Element, b: &Element) -> Result<Reading, String> {
    let ((da, plane_a), (db, plane_b)) = (
        direction(a).ok_or_else(|| format!("{} has no direction", a.name))?,
        direction(b).ok_or_else(|| format!("{} has no direction", b.name))?,
    );
    let c = dot(da, db).clamp(-1.0, 1.0);
    let value = match (plane_a, plane_b) {
        (true, true) => c.acos().to_degrees(),
        (false, false) => c.abs().acos().to_degrees(),
        _ => 90.0 - c.abs().acos().to_degrees(),
    };
    let (from, to) = (centre_of(a), centre_of(b));
    Ok(Reading {
        value,
        deltas: None,
        range: None,
        from,
        to,
    })
}

fn distance(a: &Element, b: &Element) -> Result<Reading, String> {
    let (ra, rb) = (role(a), role(b));
    let parallel = |u: [f64; 3], v: [f64; 3]| dot(u, v).abs() >= PARALLEL_DEG.to_radians().cos();
    let square = |u: [f64; 3], v: [f64; 3]| dot(u, v).abs() <= PARALLEL_DEG.to_radians().sin();
    let not_parallel = || {
        format!(
            "{} and {} are more than {PARALLEL_DEG}° from parallel: measure the angle",
            a.name, b.name
        )
    };
    let plain = |from: [f64; 3], to: [f64; 3], value: f64| Reading {
        value,
        deltas: None,
        range: None,
        from,
        to,
    };
    Ok(match (ra, rb) {
        (Role::Point(p), Role::Point(q)) => {
            let d = sub(q, p);
            Reading {
                deltas: Some(d),
                ..plain(p, q, length(d))
            }
        }
        (Role::Point(p), Role::Axis { at, dir, .. }) => {
            let foot = foot_on_line(p, at, dir);
            plain(p, foot, length(sub(p, foot)))
        }
        (Role::Axis { at, dir, .. }, Role::Point(p)) => {
            let foot = foot_on_line(p, at, dir);
            plain(foot, p, length(sub(p, foot)))
        }
        (Role::Point(p), Role::Plane { middle, normal, .. }) => {
            let d = dot(sub(p, middle), normal);
            plain(p, sub(p, scale(normal, d)), d)
        }
        (Role::Plane { middle, normal, .. }, Role::Point(p)) => {
            let d = dot(sub(p, middle), normal);
            plain(sub(p, scale(normal, d)), p, d)
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
                let mid = add(pa, scale(da, (along[0] + along[1]) / 2.0));
                let foot = foot_on_line(mid, pb, db);
                plain(mid, foot, length(sub(mid, foot)))
            } else {
                let (p, q) = closest_points(pa, da, pb, db);
                plain(p, q, length(sub(q, p)))
            }
        }
        (Role::Axis { at, dir, along }, Role::Plane { middle, normal, .. }) => {
            if !square(dir, normal) {
                return Err(not_parallel());
            }
            let mid = add(at, scale(dir, (along[0] + along[1]) / 2.0));
            let d = dot(sub(mid, middle), normal);
            plain(mid, sub(mid, scale(normal, d)), d.abs())
        }
        (Role::Plane { middle, normal, .. }, Role::Axis { at, dir, along }) => {
            if !square(dir, normal) {
                return Err(not_parallel());
            }
            let mid = add(at, scale(dir, (along[0] + along[1]) / 2.0));
            let d = dot(sub(mid, middle), normal);
            plain(sub(mid, scale(normal, d)), mid, d.abs())
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
            let d = dot(sub(ma, mb), nb);
            let s = dot(na, nb).signum();
            let (a_lo, a_hi) = if s > 0.0 { fa } else { (-fa.1, -fa.0) };
            let (lo, hi) = (d + a_lo - fb.1, d + a_hi - fb.0);
            let foot = sub(ma, scale(nb, d));
            let range = if d >= 0.0 { (lo, hi) } else { (-hi, -lo) };
            Reading {
                range: Some(range),
                ..plain(ma, foot, d.abs())
            }
        }
    })
}

/// Where an element's lines are centred, for an angle's leader.
fn centre_of(e: &Element) -> [f64; 3] {
    match role(e) {
        Role::Point(p) => p,
        Role::Axis { at, dir, along } => add(at, scale(dir, (along[0] + along[1]) / 2.0)),
        Role::Plane { middle, .. } => middle,
    }
}

fn foot_on_line(p: [f64; 3], at: [f64; 3], dir: [f64; 3]) -> [f64; 3] {
    add(at, scale(dir, dot(sub(p, at), dir)))
}

/// The closest points of two lines that are not parallel.
fn closest_points(pa: [f64; 3], da: [f64; 3], pb: [f64; 3], db: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let w = sub(pa, pb);
    let (b, d, e) = (dot(da, db), dot(da, w), dot(db, w));
    let den = (1.0 - b * b).max(1e-12);
    let (s, t) = ((b * e - d) / den, (e - b * d) / den);
    (add(pa, scale(da, s)), add(pb, scale(db, t)))
}

fn length(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    scale(v, 1.0 / length(v).max(1e-12))
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
    /// nominal when it has a band.
    pub fn label(&self, reading: &Reading) -> String {
        let unit = self.kind.unit();
        let prefix = if self.kind == DimKind::Diameter {
            "Ø "
        } else {
            ""
        };
        let mut text = format!("{prefix}{:.3} {unit}", reading.value);
        if let Some(pass) = self.passes(reading.value) {
            text.push_str(&format!(
                "  {:+.3} {}",
                reading.value - self.nominal,
                if pass { "PASS" } else { "FAIL" }
            ));
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
            name: "e".into(),
            scan: BodyId(uuid::Uuid::nil()),
            kind: FitKind::Plane,
            cutoff: None,
            region: Vec::new(),
            revision: 0,
            result: Fitted {
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
    /// get an angle but no distance, and a band passes and fails.
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
        assert_eq!(dim.label(&d), "Ø 8.000 mm  +0.000 PASS");
    }
}
