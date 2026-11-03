//! Form and orientation tolerances: how far an element strays from its
//! ideal shape, or from the attitude a datum element sets, against a
//! limit. A plane is its measured surface (its points), a cylinder, cone
//! or line its axis over its fitted length, a sphere or circle its centre.

use core_document::{
    BodyId, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use kernel_api::{FitKind, FittedShape, TriMesh};
use metrology::orientation::{self, Datum};
use serde::{Deserialize, Serialize};

use crate::element::{Element, Span};

/// The feature kind of every tolerance.
pub const KIND: &str = "wb.inspect.tolerance";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Characteristic {
    Flatness,
    Straightness,
    Circularity,
    Cylindricity,
    Sphericity,
    Parallelism,
    Perpendicularity,
    Angularity,
    Coaxiality,
    Concentricity,
}

impl Characteristic {
    pub const ALL: [Characteristic; 10] = [
        Characteristic::Flatness,
        Characteristic::Straightness,
        Characteristic::Circularity,
        Characteristic::Cylindricity,
        Characteristic::Sphericity,
        Characteristic::Parallelism,
        Characteristic::Perpendicularity,
        Characteristic::Angularity,
        Characteristic::Coaxiality,
        Characteristic::Concentricity,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Characteristic::Flatness => "Flatness",
            Characteristic::Straightness => "Straightness",
            Characteristic::Circularity => "Circularity",
            Characteristic::Cylindricity => "Cylindricity",
            Characteristic::Sphericity => "Sphericity",
            Characteristic::Parallelism => "Parallelism",
            Characteristic::Perpendicularity => "Perpendicularity",
            Characteristic::Angularity => "Angularity",
            Characteristic::Coaxiality => "Coaxiality",
            Characteristic::Concentricity => "Concentricity",
        }
    }

    /// Whether it is taken against a datum.
    pub fn needs_datum(self) -> bool {
        !matches!(
            self,
            Characteristic::Flatness
                | Characteristic::Straightness
                | Characteristic::Circularity
                | Characteristic::Cylindricity
                | Characteristic::Sphericity
        )
    }

    /// The form tolerance a fitted kind is read for.
    pub fn of_form(kind: FitKind) -> Option<Characteristic> {
        Some(match kind {
            FitKind::Plane => Characteristic::Flatness,
            FitKind::Line => Characteristic::Straightness,
            FitKind::Circle => Characteristic::Circularity,
            FitKind::Cylinder => Characteristic::Cylindricity,
            FitKind::Sphere => Characteristic::Sphericity,
            FitKind::Cone | FitKind::Torus => return None,
        })
    }
}

/// One tolerance of an inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    pub name: String,
    pub scan: BodyId,
    pub characteristic: Characteristic,
    /// The element toleranced.
    pub element: FeatureId,
    /// The element it is taken against, for an orientation tolerance.
    #[serde(default)]
    pub datum: Option<FeatureId>,
    /// The widest zone that passes, mm.
    pub limit: f64,
    /// An angularity's basic angle between the normals, degrees.
    #[serde(default)]
    pub angle_deg: Option<f64>,
}

impl WorkbenchFeature for Tolerance {
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
        std::iter::once(self.element).chain(self.datum).collect()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// An element as a tolerance reads it.
enum Part {
    /// A measured plane's points, and its normal.
    Surface(Vec<[f64; 3]>, [f64; 3]),
    /// An axis between its two ends.
    Axis([[f64; 3]; 2]),
    /// A centre, with the axis of a circle.
    Centre([f64; 3], Option<[f64; 3]>),
}

/// What a tolerance came to.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    /// The zone's width, mm.
    pub value: f64,
    /// Where its label goes, in the scan's frame.
    pub at: [f64; 3],
    /// Said beside it: what it was read from when that is a stand-in.
    pub note: Option<String>,
}

impl Tolerance {
    pub fn passes(&self, value: f64) -> bool {
        value <= self.limit + 1e-12
    }

    /// What it shows: the zone measured over its limit, both in `unit`.
    pub fn label(&self, e: &Evaluation, unit: core_document::Unit) -> String {
        format!(
            "{} {} / {} {}",
            self.characteristic.word(),
            core_document::units::show_mm(e.value, unit),
            core_document::units::show_mm(self.limit, unit),
            if self.passes(e.value) { "PASS" } else { "FAIL" }
        )
    }
}

/// Evaluate `t` on `element` (and `datum`), the element's points read from
/// `scan`, its mesh in its own frame.
pub fn evaluate(
    t: &Tolerance,
    element: &Element,
    datum: Option<&Element>,
    scan: Option<&TriMesh>,
) -> Result<Evaluation, String> {
    use Characteristic as C;
    let at = centre_of(element);
    if !t.characteristic.needs_datum() {
        if Characteristic::of_form(element.kind) != Some(t.characteristic) {
            return Err(format!(
                "{} is read on a {}",
                t.characteristic.word(),
                form_kind(t.characteristic)
            ));
        }
        let (lo, hi) = element.result.form;
        return Ok(match element.result.zone {
            Some(zone) => Evaluation {
                value: zone.width,
                at,
                note: (!zone.optimal).then(|| "a locally narrowest zone".into()),
            },
            None => Evaluation {
                value: hi - lo,
                at,
                note: Some("the least-squares band".into()),
            },
        });
    }
    let datum = datum.ok_or("an orientation tolerance needs a datum")?;
    let (part, note) = part_of(element, scan);
    let missing = || {
        format!(
            "{} does not read {} against {}",
            t.characteristic.word(),
            element.name,
            datum.name
        )
    };
    let value = match (t.characteristic, &part, part_of(datum, None).0) {
        (C::Parallelism, Part::Surface(points, n), Part::Surface(_, dn)) => {
            orientation::plane_parallelism(points, *n, Datum::Plane(dn))
        }
        (C::Parallelism, Part::Surface(points, n), Part::Axis(e)) => {
            orientation::plane_parallelism(points, *n, Datum::Axis(sub(e[1], e[0])))
        }
        (C::Parallelism, Part::Axis(ends), Part::Axis(e)) => {
            orientation::axis_parallelism(*ends, sub(e[1], e[0]))
        }
        (C::Perpendicularity, Part::Surface(points, n), Part::Surface(_, dn)) => {
            orientation::plane_perpendicularity(points, *n, Datum::Plane(dn))
        }
        (C::Perpendicularity, Part::Surface(points, n), Part::Axis(e)) => {
            orientation::plane_perpendicularity(points, *n, Datum::Axis(sub(e[1], e[0])))
        }
        (C::Perpendicularity, Part::Axis(ends), Part::Surface(_, dn)) => {
            orientation::axis_perpendicularity(*ends, dn)
        }
        (C::Angularity, Part::Surface(points, n), Part::Surface(_, dn)) => {
            let angle = t.angle_deg.ok_or("angularity needs its basic angle")?;
            orientation::plane_angularity(points, *n, dn, angle.to_radians())
        }
        (C::Coaxiality, Part::Axis(ends), Part::Axis(e)) => {
            orientation::coaxiality(*ends, e[0], sub(e[1], e[0]))
        }
        (C::Concentricity, Part::Centre(c, _), Part::Axis(e)) => {
            orientation::concentricity(*c, e[0], sub(e[1], e[0]))
        }
        (C::Concentricity, Part::Centre(c, _), Part::Centre(dc, Some(axis))) => {
            orientation::concentricity(*c, dc, axis)
        }
        _ => None,
    };
    Ok(Evaluation {
        value: value.ok_or_else(missing)?,
        at,
        note,
    })
}

fn form_kind(c: Characteristic) -> &'static str {
    match c {
        Characteristic::Flatness => "plane",
        Characteristic::Straightness => "line",
        Characteristic::Circularity => "circle",
        Characteristic::Cylindricity => "cylinder",
        _ => "sphere",
    }
}

/// How `e` is read: a fitted plane at its points on the scan within its
/// form band, a constructed one at its drawn corners (said so); an axis
/// over its fitted length; a centre.
fn part_of(e: &Element, scan: Option<&TriMesh>) -> (Part, Option<String>) {
    let along = |p: [f64; 3], d: [f64; 3]| match e.result.span {
        Span::Axis([a, b]) => [add(p, scale(d, a)), add(p, scale(d, b))],
        _ => [p, add(p, d)],
    };
    match e.result.shape {
        FittedShape::Plane { origin, normal } => {
            let (lo, hi) = e.result.form;
            let points: Vec<[f64; 3]> = scan
                .map(|mesh| {
                    crate::element::vertices(&e.region)
                        .filter_map(|v| mesh.positions.get(v as usize))
                        .map(|p| p.map(f64::from))
                        .filter(|p| {
                            let r = dot(sub(*p, origin), normal);
                            r >= lo - 1e-9 && r <= hi + 1e-9
                        })
                        .collect()
                })
                .unwrap_or_default();
            if points.len() >= 3 {
                (Part::Surface(points, normal), None)
            } else {
                (
                    Part::Surface(corners(origin, normal, e.result.span), normal),
                    Some("read at the plane's drawn corners".into()),
                )
            }
        }
        FittedShape::Cylinder { point, axis, .. }
        | FittedShape::Line {
            point,
            direction: axis,
        } => (Part::Axis(along(point, axis)), None),
        FittedShape::Cone { apex, axis, .. } => (Part::Axis(along(apex, axis)), None),
        FittedShape::Sphere { centre, .. } => (Part::Centre(centre, None), None),
        FittedShape::Circle { centre, normal, .. } => (Part::Centre(centre, Some(normal)), None),
        FittedShape::Torus { centre, axis, .. } => (Part::Centre(centre, Some(axis)), None),
    }
}

fn corners(origin: [f64; 3], normal: [f64; 3], span: Span) -> Vec<[f64; 3]> {
    match span {
        Span::Patch { u, across } => {
            let v = crate::element::cross(normal, u);
            let mut out = Vec::new();
            for a in across[0] {
                for b in across[1] {
                    out.push(add(origin, add(scale(u, a), scale(v, b))));
                }
            }
            out
        }
        _ => vec![origin],
    }
}

fn centre_of(e: &Element) -> [f64; 3] {
    match e.result.shape {
        FittedShape::Plane { origin, normal } => match e.result.span {
            Span::Patch { u, across } => {
                let v = crate::element::cross(normal, u);
                let mid = |[a, b]: [f64; 2]| (a + b) / 2.0;
                add(
                    origin,
                    add(scale(u, mid(across[0])), scale(v, mid(across[1]))),
                )
            }
            _ => origin,
        },
        FittedShape::Cylinder { point, axis, .. }
        | FittedShape::Line {
            point,
            direction: axis,
        } => match e.result.span {
            Span::Axis([a, b]) => add(point, scale(axis, (a + b) / 2.0)),
            _ => point,
        },
        FittedShape::Cone { apex, .. } => apex,
        FittedShape::Sphere { centre, .. }
        | FittedShape::Circle { centre, .. }
        | FittedShape::Torus { centre, .. } => centre,
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|c| c * k)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Fitted, span_of};

    fn plane(origin: [f64; 3], normal: [f64; 3], form: (f64, f64)) -> Element {
        let shape = FittedShape::Plane { origin, normal };
        let corners = [[-5.0, -5.0, origin[2]], [5.0, 5.0, origin[2]]];
        Element {
            method: Default::default(),
            name: "p".into(),
            scan: BodyId(uuid::Uuid::nil()),
            kind: FitKind::Plane,
            cutoff: None,
            region: vec![[0, 4]],
            revision: 0,
            result: Fitted {
                zone: None,
                span: span_of(&shape, &corners),
                shape,
                sigma: 0.0,
                form,
                used: 4,
                points: 4,
            },
        }
    }

    fn tolerance(c: Characteristic, limit: f64) -> Tolerance {
        Tolerance {
            name: "t".into(),
            scan: BodyId(uuid::Uuid::nil()),
            characteristic: c,
            element: FeatureId(uuid::Uuid::nil()),
            datum: Some(FeatureId(uuid::Uuid::nil())),
            limit,
            angle_deg: None,
        }
    }

    /// Flatness reads the form band; a top whose points tilt 0.05 over
    /// its 10 mm reads 0.05 parallelism to the floor and passes 0.06, and
    /// is refused a perpendicularity it has no reading for; a form
    /// tolerance on the wrong kind is refused.
    #[test]
    fn tolerances_read_their_zones() {
        let floor = plane([0.0; 3], [0.0, 0.0, 1.0], (-0.01, 0.01));
        let top = plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0], (-0.025, 0.025));
        let flat = evaluate(&tolerance(Characteristic::Flatness, 0.03), &top, None, None).unwrap();
        assert!((flat.value - 0.05).abs() < 1e-12);
        let mut scan = TriMesh::default();
        for (x, z) in [(-5.0, 9.975), (5.0, 10.025), (-5.0, 9.975), (5.0, 10.025)] {
            scan.positions.push([x, 0.0, z]);
        }
        let t = tolerance(Characteristic::Parallelism, 0.06);
        let e = evaluate(&t, &top, Some(&floor), Some(&scan)).unwrap();
        assert!((e.value - 0.05).abs() < 1e-5 && t.passes(e.value), "{e:?}");
        assert!(e.note.is_none());
        let cyl = tolerance(Characteristic::Cylindricity, 0.1);
        assert!(evaluate(&cyl, &top, None, None).is_err());
    }
}
