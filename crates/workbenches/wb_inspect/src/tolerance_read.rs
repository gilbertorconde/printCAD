//! Tolerance inputs detached from the document, with profile queries cached
//! and run away from the window. Every reading is keyed by its inputs.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use core_document::jobs::Keyed;
use core_document::{Document, FeatureId, WorkbenchFeature};
use glam::DVec3;
use kernel_api::{DistanceRequest, FittedShape, KernelQueries, TriMesh};
use metrology::gdt::{self, Datum, Frame};

use crate::element::{Element, vertices};
use crate::tolerance::{self, Characteristic as C, Evaluation, Material, Part, Tolerance};

#[derive(Clone)]
struct Inputs {
    tolerance: Tolerance,
    element: Element,
    datums: Vec<Element>,
    scan: Option<Arc<TriMesh>>,
    opposed: Option<Element>,
    comparison: Option<crate::compare::Inputs>,
    section: Option<crate::section::Section>,
    bonus: f64,
    shifts: Vec<f64>,
    nominal: Option<([f64; 3], [f64; 3])>,
    reference_frame: Frame,
    key: u64,
}

type Reading = Result<Evaluation, String>;

/// Each tolerance's latest reading, by the key of its inputs.
#[derive(Clone, Default)]
pub(crate) struct Readings(Keyed<FeatureId, Reading>);

impl Readings {
    pub fn busy(&self) -> bool {
        self.0.busy()
    }

    /// `t`'s reading from what `document` holds now. A profile is measured
    /// on a thread unless `wait`, and reads as being measured until it is
    /// in; a measuring for older inputs is superseded.
    pub fn read(
        &self,
        document: &Document,
        id: FeatureId,
        t: &Tolerance,
        kernel: Option<&'static dyn KernelQueries>,
        wait: bool,
    ) -> Reading {
        let inputs = Inputs::new(document, t)?;
        let key = inputs.key;
        if let Some(reading) = self.0.get(id, key) {
            return reading;
        }
        if wait || !t.characteristic.is_profile() {
            let answer = inputs.evaluate(kernel.map(|k| k as &dyn KernelQueries));
            self.0.store(id, key, answer.clone());
            return answer;
        }
        let kernel = kernel.ok_or("no kernel to measure a profile")?;
        self.0
            .ensure(id, key, move || Ok(inputs.evaluate(Some(kernel))))
            .unwrap_or_else(|| Err("measuring profile".into()))
    }
}

pub(crate) fn evaluate(
    document: &Document,
    t: &Tolerance,
    kernel: Option<&dyn KernelQueries>,
) -> Reading {
    Inputs::new(document, t)?.evaluate(kernel)
}

impl Inputs {
    fn new(document: &Document, t: &Tolerance) -> Result<Self, String> {
        validate(t)?;
        let e = crate::element_at(document, t.element).ok_or("the element is gone")?;
        let scan = document.local_geometry(e.scan);
        let get = |id| {
            let other = crate::element_at(document, id).ok_or("a referenced element is gone")?;
            if other.scan != e.scan {
                return Err("tolerance elements must belong to the same scan".into());
            }
            Ok(other)
        };
        let refs = t.references();
        let datums = refs
            .iter()
            .map(|d| get(d.element))
            .collect::<Result<Vec<_>, String>>()?;
        let opposed = t.opposed.map(get).transpose()?;
        let bonus = if let Some(material) = t.material {
            let actual = size(&e, opposed.as_ref())
                .ok_or("material modifier needs a size element or slot")?;
            let boundary = material_boundary(
                document,
                t.element,
                material,
                t.material_size,
                &e,
                scan.as_ref().map(|m| &*m.0),
            )?;
            gdt::bonus(actual, boundary).ok_or("material sizes must be positive and finite")?
        } else {
            0.0
        };
        let shifts = refs
            .iter()
            .zip(&datums)
            .map(|(r, d)| {
                r.modifier.map_or(Ok(0.0), |material| {
                    let actual =
                        size(d, None).ok_or("datum material modifier needs a size element")?;
                    let boundary = material_boundary(
                        document,
                        r.element,
                        material,
                        r.size,
                        d,
                        scan.as_ref().map(|m| &*m.0),
                    )?;
                    gdt::bonus(actual, boundary).ok_or_else(|| "invalid datum material size".into())
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let comparison = t
            .comparison
            .map(|id| {
                let c = crate::compare::Comparison::from_json(
                    document.feature_values(id).ok_or("comparison is gone")?,
                )
                .map_err(|e| e.to_string())?;
                if c.scan != e.scan {
                    return Err("comparison belongs to a different scan".into());
                }
                crate::compare::inputs(document, &c)
            })
            .transpose()?;
        let section = t
            .section
            .map(|id| {
                let s = crate::section::Section::from_json(
                    document.feature_values(id).ok_or("section is gone")?,
                )
                .map_err(|e| e.to_string())?;
                if s.scan != e.scan {
                    return Err("section belongs to a different scan".to_string());
                }
                Ok(s)
            })
            .transpose()?;
        let reference_frame = if let Some(id) = t.comparison.filter(|_| !datums.is_empty()) {
            nominal_frame(document, id, &datums)?
        } else {
            Frame::identity()
        };
        let nominal = t
            .reference_face
            .map(|face| {
                let id = t.comparison.ok_or("a nominal face needs a comparison")?;
                let c = crate::compare::Comparison::from_json(
                    document.feature_values(id).ok_or("comparison is gone")?,
                )
                .map_err(|e| e.to_string())?;
                let mesh = document
                    .local_geometry(c.reference)
                    .ok_or("reference has no geometry")?
                    .0;
                match mesh.face_surfaces.get(face as usize) {
                    Some(kernel_api::FaceSurface::Plane { origin, normal }) => {
                        Ok((origin.map(f64::from), normal.map(f64::from)))
                    }
                    Some(kernel_api::FaceSurface::Cylinder { origin, axis, .. }) => {
                        Ok((origin.map(f64::from), axis.map(f64::from)))
                    }
                    Some(kernel_api::FaceSurface::Sphere { center, .. }) => {
                        Ok((center.map(f64::from), [0.0, 0.0, 1.0]))
                    }
                    _ => Err("the nominal face must be a plane, cylinder or sphere".to_string()),
                }
            })
            .transpose()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{t:?}{e:?}{datums:?}{opposed:?}{section:?}{bonus:?}{shifts:?}{nominal:?}{reference_frame:?}").hash(&mut h);
        document
            .imported_geometry(e.scan)
            .map(|g| g.revision)
            .hash(&mut h);
        scan.as_ref()
            .map(|s| Arc::as_ptr(&s.0) as usize)
            .hash(&mut h);
        comparison.as_ref().map(|c| c.key).hash(&mut h);
        Ok(Self {
            tolerance: t.clone(),
            element: e,
            datums,
            scan: scan.map(|s| s.0),
            opposed,
            comparison,
            section,
            bonus,
            shifts,
            nominal,
            reference_frame,
            key: h.finish(),
        })
    }

    fn evaluate(&self, kernel: Option<&dyn KernelQueries>) -> Reading {
        let t = &self.tolerance;
        let e = &self.element;
        let scan = self.scan.as_deref();
        let at = tolerance::centre_of(e);
        let mut note = None;
        let value = match t.characteristic {
            C::Position => {
                let frame = frame(&self.datums)?;
                let basic = t
                    .basic
                    .or(self.nominal.map(|n| self.reference_frame.local(n.0)))
                    .ok_or("position needs basic offsets or a reference_face on its comparison")?;
                let dir = t
                    .basic_direction
                    .or(self
                        .nominal
                        .map(|n| self.reference_frame.axes.map(|a| DVec3::from(a).dot(n.1.into()))))
                    .unwrap_or([0.0, 0.0, 1.0]);
                let (part, n) = tolerance::part_of(e, scan);
                note = n;
                if frame.free_translations + frame.free_rotations > 0 {
                    note = Some(format!("frame retains {} translation and {} rotation freedoms in the coordinate system", frame.free_translations, frame.free_rotations));
                }
                match part {
                    Part::Surface(points, _) if self.opposed.is_some() => {
                        let other = self.opposed.as_ref().unwrap();
                        let planes = [plane(e)?, plane(other)?];
                        let normal = DVec3::from(frame.world(dir)) - DVec3::from(frame.origin);
                        let points = points
                            .iter()
                            .map(|p| {
                                let p = DVec3::from(*p);
                                let h = planes.map(|(o, n)| {
                                    let n = DVec3::from(n);
                                    (DVec3::from(o) - p).dot(n) / normal.dot(n)
                                });
                                frame.local((p + normal * ((h[0] + h[1]) / 2.0)).to_array())
                            })
                            .collect::<Vec<_>>();
                        gdt::plane_position(&points, basic, dir)
                    }
                    Part::Surface(points, _) => gdt::plane_position(
                        &points.iter().map(|p| frame.local(*p)).collect::<Vec<_>>(),
                        basic,
                        dir,
                    ),
                    Part::Axis(ends) => gdt::position(&ends.map(|p| frame.local(p)), basic, dir),
                    Part::Centre(c, _) => gdt::position(&[frame.local(c)], basic, dir),
                }
            }
            C::SurfaceProfile | C::LineProfile => {
                let kernel = kernel.ok_or("no kernel to measure a profile")?;
                let target = self
                    .comparison
                    .as_ref()
                    .ok_or("profile needs a comparison")?;
                let points = if t.characteristic == C::LineProfile {
                    let s = self
                        .section
                        .as_ref()
                        .ok_or("line profile needs a section")?;
                    crate::section::cut(kernel, scan.ok_or("scan mesh is gone")?, s)?
                        .into_iter()
                        .flat_map(|c| c.chains)
                        .flat_map(|c| c.points)
                        .collect()
                } else {
                    region(e, scan)?
                };
                let frame = frame(&self.datums)?;
                let points = points
                    .into_iter()
                    .map(|p| {
                        if self.datums.is_empty() {
                            target
                                .scan_to_reference()
                                .point(p.map(|x| x as f32))
                                .map(f64::from)
                        } else {
                            self.reference_frame.world(frame.local(p))
                        }
                    })
                    .collect::<Vec<_>>();
                if points.is_empty() {
                    return Err("profile has no measured points".into());
                }
                let request = DistanceRequest {
                    points,
                    normals: Vec::new(),
                    max: None,
                    facing_limit_deg: None,
                };
                let distances = kernel
                    .signed_distance(target.target(), &request)
                    .map_err(|e| e.to_string())?;
                if distances.len() != request.points.len() {
                    return Err("profile distance count differs from its points".into());
                }
                gdt::profile(&distances, t.unequal.unwrap_or(0.0))
            }
            C::CircularRunout | C::TotalRunout => {
                let first = self.datums.first().ok_or("runout needs a datum axis")?;
                let (mut on, mut direction) = axis(first)?;
                if let Some(second) = self.datums.get(1) {
                    let (b, db) = axis(second)?;
                    (on, direction) = gdt::common_axis(on, direction, b, db)
                        .ok_or("common datum needs separated, aligned axes")?;
                }
                let kind = match e.result.shape {
                    FittedShape::Plane { .. } => gdt::Runout::Axial,
                    FittedShape::Cylinder { .. } => gdt::Runout::Radial { slope: 0.0 },
                    FittedShape::Cone {
                        axis, half_angle, ..
                    } => gdt::Runout::Radial {
                        slope: half_angle.tan() * DVec3::from(axis).dot(direction.into()).signum(),
                    },
                    _ => return Err("runout needs a cylinder, cone or plane".into()),
                };
                let width = if t.characteristic == C::CircularRunout {
                    let w = t
                        .section_width
                        .ok_or("circular runout needs section_width in mm")?;
                    note = Some(format!("sampled section bands {w:.4} mm wide"));
                    Some(w)
                } else {
                    None
                };
                gdt::runout(&region(e, scan)?, on, direction, kind, width)
            }
            C::Symmetry => {
                let other = self
                    .opposed
                    .as_ref()
                    .ok_or("symmetry needs the opposed plane")?;
                let datum = self.datums.first().ok_or("symmetry needs a datum plane")?;
                let (on, normal) = plane(datum)?;
                let planes = [plane(e)?, plane(other)?];
                let (Part::Surface(points, _), n) = tolerance::part_of(e, scan) else {
                    return Err("symmetry needs opposed planes".into());
                };
                note = n;
                gdt::symmetry(&points, planes, on, normal)
            }
            _ => {
                let mut reading = tolerance::evaluate(t, e, self.datums.first(), scan)?;
                reading.datum_shift = self.shifts.clone();
                reading.datums = self.labels();
                return Ok(reading);
            }
        }
        .ok_or("the tolerance has no finite zone (check coverage, directions and section width)")?;
        Ok(Evaluation {
            value,
            at,
            note,
            bonus: self.bonus,
            datum_shift: self.shifts.clone(),
            datums: self.labels(),
        })
    }

    fn labels(&self) -> Vec<String> {
        self.datums
            .iter()
            .zip(self.tolerance.references())
            .map(|(d, r)| format!("{}{}", d.name, r.modifier.map_or("", Material::symbol)))
            .collect()
    }
}

fn validate(t: &Tolerance) -> Result<(), String> {
    if !t.limit.is_finite() || t.limit < 0.0 {
        return Err("limit must be finite and nonnegative".into());
    }
    let refs = t.references();
    if refs.len() > 3 || refs.iter().map(|r| r.element).collect::<HashSet<_>>().len() != refs.len()
    {
        return Err("a datum frame has up to three distinct elements".into());
    }
    if t.characteristic.needs_datum() && refs.is_empty() {
        return Err("this characteristic needs a datum".into());
    }
    if t.material.is_some() && t.characteristic != C::Position {
        return Err("material bonus applies to position only".into());
    }
    if t.unequal.is_some() && !t.characteristic.is_profile() {
        return Err("unequal disposition applies to profiles only".into());
    }
    if matches!(t.characteristic, C::CircularRunout | C::TotalRunout) && refs.len() > 2 {
        return Err("runout takes one datum axis or a common pair".into());
    }
    Ok(())
}

/// Match datum geometry to exact nominal surfaces in the placed reference.
/// Distinct datums cannot consume the same nominal face.
fn nominal_frame(document: &Document, id: FeatureId, datums: &[Element]) -> Result<Frame, String> {
    let c = crate::compare::Comparison::from_json(
        document.feature_values(id).ok_or("comparison is gone")?,
    )
    .map_err(|e| e.to_string())?;
    let input = crate::compare::inputs(document, &c)?;
    let mesh = document
        .local_geometry(c.reference)
        .ok_or("reference has no geometry")?
        .0;
    let to = input.scan_to_reference();
    let mut used = HashSet::new();
    let mut nominal = Vec::new();
    for e in datums {
        let anchor = to
            .point(tolerance::centre_of(e).map(|v| v as f32))
            .map(f64::from);
        let normal = match e.result.shape {
            FittedShape::Plane { normal, .. } => normal,
            _ => axis(e)?.1,
        };
        let direction = to.direction(normal.map(|v| v as f32)).map(f64::from);
        let candidates = mesh
            .face_surfaces
            .iter()
            .enumerate()
            .filter(|(i, _)| !used.contains(i))
            .filter_map(|(i, s)| {
                let (d, datum, distance) = match *s {
                    kernel_api::FaceSurface::Plane { origin, normal }
                        if matches!(e.result.shape, FittedShape::Plane { .. }) =>
                    {
                        let (p, n) = (origin.map(f64::from), normal.map(f64::from));
                        (
                            n,
                            Datum::Plane {
                                point: p,
                                normal: n,
                            },
                            (DVec3::from(anchor) - DVec3::from(p)).dot(n.into()).abs(),
                        )
                    }
                    kernel_api::FaceSurface::Cylinder { origin, axis, .. }
                        if !matches!(e.result.shape, FittedShape::Plane { .. }) =>
                    {
                        let (p, d) = (origin.map(f64::from), axis.map(f64::from));
                        let (v, dv) = (DVec3::from(anchor) - DVec3::from(p), DVec3::from(d));
                        let perpendicular = v - dv * v.dot(dv);
                        (
                            d,
                            Datum::Axis {
                                point: p,
                                direction: d,
                            },
                            perpendicular.length(),
                        )
                    }
                    _ => return None,
                };
                let agreement = DVec3::from(d).dot(direction.into()).abs();
                (agreement > 0.9).then_some((i, datum, distance + (1.0 - agreement).abs()))
            });
        let (i, mut datum, _) = candidates
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .ok_or("a datum has no matching nominal plane or axis on the reference")?;
        match &mut datum {
            Datum::Plane { normal, .. } => {
                if DVec3::from(*normal).dot(direction.into()) < 0.0 {
                    *normal = normal.map(|c| -c);
                }
            }
            Datum::Axis { direction: d, .. } => {
                if DVec3::from(*d).dot(direction.into()) < 0.0 {
                    *d = d.map(|c| -c);
                }
            }
        }
        used.insert(i);
        nominal.push(datum);
    }
    Frame::from_datums(&nominal).ok_or_else(|| "degenerate nominal datum frame".into())
}

fn frame(elements: &[Element]) -> Result<Frame, String> {
    let datums = elements
        .iter()
        .map(|e| match e.result.shape {
            FittedShape::Plane { origin, normal } => Ok(Datum::Plane {
                point: origin,
                normal,
            }),
            _ => axis(e).map(|(point, direction)| Datum::Axis { point, direction }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Frame::from_datums(&datums).ok_or_else(|| "degenerate datum frame".into())
}
fn plane(e: &Element) -> Result<([f64; 3], [f64; 3]), String> {
    match e.result.shape {
        FittedShape::Plane { origin, normal } => Ok((origin, normal)),
        _ => Err("a plane element is required".into()),
    }
}
fn axis(e: &Element) -> Result<([f64; 3], [f64; 3]), String> {
    match e.result.shape {
        FittedShape::Cylinder { axis, .. } | FittedShape::Cone { axis, .. } => {
            Ok((tolerance::centre_of(e), axis))
        }
        FittedShape::Line { direction, .. } => Ok((tolerance::centre_of(e), direction)),
        FittedShape::Circle { centre, normal, .. } => Ok((centre, normal)),
        _ => Err("an axial element is required".into()),
    }
}
fn region(e: &Element, scan: Option<&TriMesh>) -> Result<Vec<[f64; 3]>, String> {
    let mesh = scan.ok_or("scan mesh is gone")?;
    let points = vertices(&e.region)
        .map(|i| {
            mesh.positions
                .get(i as usize)
                .map(|p| p.map(f64::from))
                .ok_or_else(|| "the element's region is outside its scan".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if points.is_empty() {
        return Err("a measured scan region is required".into());
    }
    Ok(points)
}
fn size(e: &Element, opposed: Option<&Element>) -> Option<f64> {
    match e.result.shape {
        FittedShape::Cylinder { radius, .. } | FittedShape::Sphere { radius, .. } => {
            Some(2.0 * radius)
        }
        FittedShape::Plane { origin, normal } => {
            let (other, n) = plane(opposed?).ok()?;
            let normal = DVec3::from(normal);
            (normal.dot(n.into()).abs() > 0.999)
                .then(|| (DVec3::from(other) - DVec3::from(origin)).dot(normal).abs())
        }
        _ => None,
    }
}
fn material_boundary(
    document: &Document,
    id: FeatureId,
    material: Material,
    explicit: Option<f64>,
    e: &Element,
    mesh: Option<&TriMesh>,
) -> Result<f64, String> {
    if let Some(size) = explicit {
        return Ok(size);
    }
    let dimensions = document
        .features_of::<crate::dimension::Dimension>()
        .into_iter()
        .map(|(_, d)| d)
        .filter(|d| d.elements == [id] && d.kind == crate::dimension::DimKind::Diameter)
        .collect::<Vec<_>>();
    if dimensions.len() != 1 {
        return Err(
            "state material_size (or give the element exactly one diameter dimension)".into(),
        );
    }
    let d = &dimensions[0];
    let internal = mesh
        .and_then(|m| {
            vertices(&e.region).find_map(|i| {
                let p = m.positions.get(i as usize)?.map(f64::from);
                let n = m.normals.get(i as usize)?.map(f64::from);
                let centre = tolerance::centre_of(e);
                Some((DVec3::from(p) - DVec3::from(centre)).dot(n.into()) < 0.0)
            })
        })
        .ok_or("state material_size when the material side has no scan normals")?;
    Ok(if internal == (material == Material::Mmc) {
        d.nominal - d.minus
    } else {
        d.nominal + d.plus
    })
}
