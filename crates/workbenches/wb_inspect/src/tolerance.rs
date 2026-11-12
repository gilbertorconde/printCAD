//! Form and orientation tolerances: how far an element strays from its
//! ideal shape, or from the attitude a datum element sets, against a
//! limit. A plane is its measured surface (its points), a cylinder, cone
//! or line its axis over its fitted length, a sphere or circle its centre.

use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchId};
use glam::DVec3;
use kernel_api::{FitKind, FittedShape, TriMesh};
use metrology::orientation::{self, Datum};
use serde::{Deserialize, Serialize};

use crate::element::{Element, Span};

/// The feature kind of every tolerance.
pub const KIND: &str = "wb.inspect.tolerance";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Characteristic {
    #[default]
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
    Position,
    SurfaceProfile,
    LineProfile,
    CircularRunout,
    TotalRunout,
    Symmetry,
}

impl Characteristic {
    pub const ALL: [Characteristic; 16] = [
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
        Characteristic::Position,
        Characteristic::SurfaceProfile,
        Characteristic::LineProfile,
        Characteristic::CircularRunout,
        Characteristic::TotalRunout,
        Characteristic::Symmetry,
    ];

    pub fn key(self) -> String {
        self.word().to_lowercase().replace(' ', "_")
    }

    pub fn is_profile(self) -> bool {
        matches!(self, Self::SurfaceProfile | Self::LineProfile)
    }

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
            Characteristic::Position => "Position",
            Characteristic::SurfaceProfile => "Surface profile",
            Characteristic::LineProfile => "Line profile",
            Characteristic::CircularRunout => "Circular runout",
            Characteristic::TotalRunout => "Total runout",
            Characteristic::Symmetry => "Symmetry",
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
                | Characteristic::SurfaceProfile
                | Characteristic::LineProfile
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

/// A material boundary modifier. Its size is stated separately in mm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    Mmc,
    Lmc,
}

impl Material {
    pub fn word(self) -> &'static str {
        match self {
            Self::Mmc => "maximum_material_requirement",
            Self::Lmc => "least_material_requirement",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Mmc => "Ⓜ",
            Self::Lmc => "Ⓛ",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatumRef {
    pub element: FeatureId,
    #[serde(default)]
    pub modifier: Option<Material>,
    /// Explicit material boundary; otherwise read from its size dimension.
    #[serde(default)]
    pub size: Option<f64>,
}

/// One tolerance of an inspection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub datums: Vec<DatumRef>,
    #[serde(default)]
    pub basic: Option<[f64; 3]>,
    /// Zone axis or plane normal in the datum frame; Z when omitted.
    #[serde(default)]
    pub basic_direction: Option<[f64; 3]>,
    #[serde(default)]
    pub material: Option<Material>,
    #[serde(default)]
    pub material_size: Option<f64>,
    /// Offset of the profile zone's centre from the nominal surface.
    #[serde(default)]
    pub unequal: Option<f64>,
    #[serde(default)]
    pub section: Option<FeatureId>,
    #[serde(default)]
    pub comparison: Option<FeatureId>,
    /// Face index of the comparison's reference supplying the nominal.
    #[serde(default)]
    pub reference_face: Option<u32>,
    /// The other face of a slot or opposed-plane symmetry tolerance.
    #[serde(default)]
    pub opposed: Option<FeatureId>,
    /// Axial/radial station width for circular runout, mm.
    #[serde(default)]
    pub section_width: Option<f64>,
}

impl WorkbenchFeature for Tolerance {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        let mut ids: Vec<_> = std::iter::once(self.element)
            .chain(self.references().iter().map(|d| d.element))
            .chain(self.section)
            .chain(self.comparison)
            .chain(self.opposed)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// An element as a tolerance reads it.
pub(super) enum Part {
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
    pub bonus: f64,
    pub datum_shift: Vec<f64>,
    pub datums: Vec<String>,
}

impl Tolerance {
    /// The legacy datum remains primary; an explicit entry for it carries
    /// its modifier without creating a duplicate reference.
    pub fn references(&self) -> Vec<DatumRef> {
        let mut refs = self.datums.clone();
        if let Some(id) = self.datum {
            if let Some(i) = refs.iter().position(|d| d.element == id) {
                let primary = refs.remove(i);
                refs.insert(0, primary);
            } else {
                refs.insert(
                    0,
                    DatumRef {
                        element: id,
                        modifier: None,
                        size: None,
                    },
                );
            }
        }
        refs
    }

    pub fn accepts(&self, e: &Evaluation) -> bool {
        e.value <= self.limit + e.bonus + 1e-12
    }
    pub fn passes(&self, value: f64) -> bool {
        value <= self.limit + 1e-12
    }

    /// What it shows: the zone measured over its limit, both in `unit`.
    pub fn label(&self, e: &Evaluation, unit: core_document::Unit) -> String {
        let mut label = format!(
            "{} {} / {} {}",
            self.characteristic.word(),
            core_document::units::show_mm(e.value, unit),
            core_document::units::show_mm(self.limit + e.bonus, unit),
            if self.accepts(e) { "PASS" } else { "FAIL" }
        );
        let mm = |v: f64| core_document::units::show_mm(v, unit);
        if let Some(material) = self.material {
            label.push_str(&format!(" · bonus {}{}", mm(e.bonus), material.symbol()));
        }
        if !e.datums.is_empty() {
            label.push_str(&format!(" | {}", e.datums.join(" | ")));
        }
        if let Some(offset) = self.unequal {
            label.push_str(&format!(" · zone offset {}", mm(offset)));
        }
        let shifts: Vec<String> = e
            .datums
            .iter()
            .zip(&e.datum_shift)
            .filter(|(_, s)| **s > 0.0)
            .map(|(d, s)| format!("{d} {}", mm(*s)))
            .collect();
        if !shifts.is_empty() {
            label.push_str(&format!(
                " · datum shift {} (not applied)",
                shifts.join(", ")
            ));
        }
        label
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
                bonus: 0.0,
                datum_shift: Vec::new(),
                datums: Vec::new(),
                note: (!zone.optimal).then(|| "a locally narrowest zone".into()),
            },
            None => Evaluation {
                value: hi - lo,
                at,
                bonus: 0.0,
                datum_shift: Vec::new(),
                datums: Vec::new(),
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
    // An axis's direction, from its first end to its second.
    let run = |e: [[f64; 3]; 2]| (DVec3::from(e[1]) - DVec3::from(e[0])).to_array();
    let value = match (t.characteristic, &part, part_of(datum, None).0) {
        (C::Parallelism, Part::Surface(points, n), Part::Surface(_, dn)) => {
            orientation::plane_parallelism(points, *n, Datum::Plane(dn))
        }
        (C::Parallelism, Part::Surface(points, n), Part::Axis(e)) => {
            orientation::plane_parallelism(points, *n, Datum::Axis(run(e)))
        }
        (C::Parallelism, Part::Axis(ends), Part::Axis(e)) => {
            orientation::axis_parallelism(*ends, run(e))
        }
        (C::Perpendicularity, Part::Surface(points, n), Part::Surface(_, dn)) => {
            orientation::plane_perpendicularity(points, *n, Datum::Plane(dn))
        }
        (C::Perpendicularity, Part::Surface(points, n), Part::Axis(e)) => {
            orientation::plane_perpendicularity(points, *n, Datum::Axis(run(e)))
        }
        (C::Perpendicularity, Part::Axis(ends), Part::Surface(_, dn)) => {
            orientation::axis_perpendicularity(*ends, dn)
        }
        (C::Angularity, Part::Surface(points, n), Part::Surface(_, dn)) => {
            let angle = t.angle_deg.ok_or("angularity needs its basic angle")?;
            orientation::plane_angularity(points, *n, dn, angle.to_radians())
        }
        (C::Coaxiality, Part::Axis(ends), Part::Axis(e)) => {
            orientation::coaxiality(*ends, e[0], run(e))
        }
        (C::Concentricity, Part::Centre(c, _), Part::Axis(e)) => {
            orientation::concentricity(*c, e[0], run(e))
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
        bonus: 0.0,
        datum_shift: Vec::new(),
        datums: vec![datum.name.clone()],
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
pub(super) fn part_of(e: &Element, scan: Option<&TriMesh>) -> (Part, Option<String>) {
    let along = |p: [f64; 3], d: [f64; 3]| {
        let (p, d) = (DVec3::from(p), DVec3::from(d));
        match e.result.span {
            Span::Axis([a, b]) => [(p + d * a).to_array(), (p + d * b).to_array()],
            _ => [p.to_array(), (p + d).to_array()],
        }
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
                            let r = (DVec3::from(*p) - DVec3::from(origin)).dot(normal.into());
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
            let (origin, u) = (DVec3::from(origin), DVec3::from(u));
            let v = DVec3::from(normal).cross(u);
            let mut out = Vec::new();
            for a in across[0] {
                for b in across[1] {
                    out.push((origin + (u * a + v * b)).to_array());
                }
            }
            out
        }
        _ => vec![origin],
    }
}

pub(super) fn centre_of(e: &Element) -> [f64; 3] {
    match e.result.shape {
        FittedShape::Plane { origin, normal } => match e.result.span {
            Span::Patch { u, across } => {
                let u = DVec3::from(u);
                let v = DVec3::from(normal).cross(u);
                let mid = |[a, b]: [f64; 2]| (a + b) / 2.0;
                (DVec3::from(origin) + (u * mid(across[0]) + v * mid(across[1]))).to_array()
            }
            _ => origin,
        },
        FittedShape::Cylinder { point, axis, .. }
        | FittedShape::Line {
            point,
            direction: axis,
        } => match e.result.span {
            Span::Axis([a, b]) => {
                (DVec3::from(point) + DVec3::from(axis) * ((a + b) / 2.0)).to_array()
            }
            _ => point,
        },
        FittedShape::Cone { apex, .. } => apex,
        FittedShape::Sphere { centre, .. }
        | FittedShape::Circle { centre, .. }
        | FittedShape::Torus { centre, .. } => centre,
    }
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
            ..Default::default()
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
