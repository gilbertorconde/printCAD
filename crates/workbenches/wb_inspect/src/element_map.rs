//! A scan measured against one of its own elements: the distance of every
//! scan point within the element's drawn extent from the element's ideal
//! shape, positive on the side the material faces out to. "Is this face
//! flat, is this bore round?" needs no reference body and no alignment.

use core_document::{
    BodyId, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use kernel_api::{FittedShape, TriMesh};
use serde::{Deserialize, Serialize};

use crate::element::{Element, Span};

/// The feature kind of every map against an element.
pub const KIND: &str = "wb.inspect.element_map";

/// How far past the element's drawn extent a point still counts, as a
/// share of that extent.
const MARGIN: f64 = 0.02;

/// One map of a scan against an element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementMap {
    pub name: String,
    pub scan: BodyId,
    pub element: FeatureId,
    /// Points whose surface turns more than this many degrees from the
    /// shape's are left unmeasured.
    #[serde(default = "default_facing")]
    pub facing_limit_deg: f64,
    /// The side counted positive turned over.
    #[serde(default)]
    pub flip: bool,
    #[serde(default)]
    pub tolerance: Option<f64>,
}

fn default_facing() -> f64 {
    60.0
}

impl WorkbenchFeature for ElementMap {
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
        vec![self.element]
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// The signed distance and outward direction of `p` from `shape`, or
/// `None` where the shape has no surface to measure to.
fn from_shape(shape: &FittedShape, p: [f64; 3]) -> Option<(f64, [f64; 3])> {
    match *shape {
        FittedShape::Plane { origin, normal } => {
            let n = unit(normal)?;
            Some((dot(sub(p, origin), n), n))
        }
        FittedShape::Sphere { centre, radius } if radius > 0.0 => {
            let out = sub(p, centre);
            let d = length(out);
            Some((d - radius, unit(out)?))
        }
        FittedShape::Cylinder {
            point,
            axis,
            radius,
        } => {
            let a = unit(axis)?;
            let off = sub(p, point);
            let radial = sub(off, scale(a, dot(off, a)));
            Some((length(radial) - radius, unit(radial)?))
        }
        FittedShape::Cone {
            apex,
            axis,
            half_angle,
        } => {
            let a = unit(axis)?;
            let off = sub(p, apex);
            let along = dot(off, a);
            let radial = sub(off, scale(a, along));
            let r = length(radial);
            let (s, c) = half_angle.sin_cos();
            // Distance to the cone's line in the plane through the axis.
            let d = r * c - along * s;
            let out = unit(sub(scale(unit(radial)?, c), scale(a, s)))?;
            Some((d, out))
        }
        _ => None,
    }
}

/// Whether `p` lies within the element's drawn extent.
fn within(e: &Element, p: [f64; 3]) -> bool {
    match (&e.result.shape, e.result.span) {
        (&FittedShape::Plane { origin, normal }, Span::Patch { u, across }) => {
            let v = crate::element::cross(normal, u);
            let (a, b) = (dot(sub(p, origin), u), dot(sub(p, origin), v));
            let inside = |t: f64, [lo, hi]: [f64; 2]| {
                let m = (hi - lo) * MARGIN;
                t >= lo - m && t <= hi + m
            };
            inside(a, across[0]) && inside(b, across[1])
        }
        (&FittedShape::Cylinder { point, axis, .. }, Span::Axis(t))
        | (
            &FittedShape::Line {
                point,
                direction: axis,
            },
            Span::Axis(t),
        )
        | (
            &FittedShape::Cone {
                apex: point, axis, ..
            },
            Span::Axis(t),
        ) => {
            let Some(a) = unit(axis) else {
                return false;
            };
            let at = dot(sub(p, point), a);
            let m = (t[1] - t[0]) * MARGIN;
            at >= t[0] - m && at <= t[1] + m
        }
        _ => true,
    }
}

/// The distance of every vertex of `scan` from `e`'s shape (`NaN` outside
/// its extent or facing away), positive on the side the scan's own normals
/// say the material faces out to.
pub fn measure(scan: &TriMesh, e: &Element, m: &ElementMap) -> Vec<f32> {
    let cos_limit = m.facing_limit_deg.to_radians().cos();
    let normal = |v: usize| scan.normals.get(v).map(|n| n.map(f64::from));
    let mut sides = 0.0;
    let mut raw: Vec<Option<f64>> = Vec::with_capacity(scan.positions.len());
    for (v, p) in scan.positions.iter().enumerate() {
        let p = p.map(f64::from);
        let reading = within(e, p)
            .then(|| from_shape(&e.result.shape, p))
            .flatten()
            .and_then(|(d, out)| {
                let Some(n) = normal(v).and_then(unit) else {
                    return Some(d);
                };
                let facing = dot(n, out);
                (facing.abs() >= cos_limit).then(|| {
                    sides += facing.signum();
                    d
                })
            });
        raw.push(reading);
    }
    // The scan's normals say which side of the shape is outside.
    let sign = if (sides < 0.0) != m.flip { -1.0 } else { 1.0 };
    raw.into_iter()
        .map(|d| d.map_or(f32::NAN, |d| (d * sign) as f32))
        .collect()
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

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = length(a);
    (l > 1e-12).then(|| scale(a, 1.0 / l))
}

impl crate::maps::Mapped for ElementMap {
    const KIND: &'static str = KIND;
    const WORD: &'static str = "Map";
    /// The scan's mesh, the element, and the key.
    type Inputs = (std::sync::Arc<TriMesh>, Element, u64);

    fn body(&self) -> BodyId {
        self.scan
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn inputs(&self, document: &core_document::Document) -> Result<Self::Inputs, String> {
        crate::element_map_inputs(document, self).ok_or_else(|| "the scan has no mesh".into())
    }

    fn key(inputs: &Self::Inputs) -> u64 {
        inputs.2
    }

    fn measure(
        &self,
        (mesh, e, _): &Self::Inputs,
        _kernel: Option<&'static dyn kernel_api::KernelQueries>,
    ) -> Result<std::sync::Arc<[f32]>, String> {
        Ok(measure(mesh, e, self).into())
    }

    /// Distance from the element on a signed scale, read as a comparison
    /// with no reference body.
    fn paint(
        &self,
        id: FeatureId,
        name: &str,
        (_, e, key): &Self::Inputs,
        values: std::sync::Arc<[f32]>,
        _maps: &crate::maps::Maps,
        ctx: &core_document::WorkbenchRuntimeContext,
    ) -> Option<core_document::ScalarMap> {
        let as_comparison = crate::compare::Comparison {
            name: self.name.clone(),
            scan: self.scan,
            reference: self.scan,
            range: None,
            bands: 0,
            ramp: metrology::ramp::Ramp::Jet.name().to_string(),
            tolerance: self.tolerance,
            max: None,
            facing_limit_deg: None,
            volume_cell: None,
        };
        let r = crate::compare::read(&values, &as_comparison);
        let ramp = metrology::ramp::Ramp::Jet;
        let (under, over) = ramp.caps();
        let range = r.range as f32;
        let mut stats = Vec::new();
        if let Some(s) = r.summary {
            for (name, v) in [
                ("Mean", s.mean),
                ("RMS", s.rms),
                ("Min", s.min),
                ("Max", s.max),
            ] {
                stats.push((
                    name.to_string(),
                    core_document::units::show_mm_signed(v, ctx.document.display_unit()),
                ));
            }
        }
        if let (Some(t), Some(w)) = (self.tolerance, r.within) {
            stats.push((format!("Within ±{t}"), format!("{:.1} %", w * 100.0)));
        }
        Some(core_document::ScalarMap {
            body: self.scan,
            revision: key ^ id.0.as_u64_pair().0,
            values,
            range: [-range, range],
            bands: 0,
            ramp: (0..32).map(|i| ramp.at(f64::from(i) / 31.0)).collect(),
            under,
            over,
            none: ctx.sketch_palette.inactive,
            legend: core_document::MapLegend {
                title: format!("{name} · from {}", e.name),
                unit: "mm".to_string(),
                stats,
                histogram: r.histogram.counts.iter().map(|n| *n as u32).collect(),
                tolerance: self.tolerance.map(|t| (-t as f32, t as f32)),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Fitted;
    use kernel_api::FitKind;

    /// Against a cylinder of radius 10, points at 10.2 and 9.9 read +0.2
    /// and −0.1 where the scan's normals face out, and the other way for a
    /// bore whose normals face in; a point past the cylinder's length or
    /// facing along the axis is left unmeasured.
    #[test]
    fn a_scan_reads_against_one_element() {
        let e = Element {
            method: Default::default(),
            name: "Cylinder 1".into(),
            scan: BodyId(uuid::Uuid::nil()),
            kind: FitKind::Cylinder,
            cutoff: None,
            region: Vec::new(),
            revision: 0,
            result: Fitted {
                zone: None,
                shape: FittedShape::Cylinder {
                    point: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 10.0,
                },
                sigma: 0.0,
                form: (0.0, 0.0),
                used: 0,
                points: 0,
                span: Span::Axis([0.0, 20.0]),
            },
        };
        let mut scan = TriMesh {
            positions: vec![
                [10.2, 0.0, 5.0],
                [0.0, 9.9, 5.0],
                [10.0, 0.0, 40.0],
                [0.0, -10.0, 5.0],
            ],
            normals: vec![
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            ..TriMesh::default()
        };
        let m = ElementMap {
            name: "Map 1".into(),
            scan: e.scan,
            element: FeatureId(uuid::Uuid::nil()),
            facing_limit_deg: 60.0,
            flip: false,
            tolerance: None,
        };
        let d = measure(&scan, &e, &m);
        assert!(
            (d[0] - 0.2).abs() < 1e-5 && (d[1] + 0.1).abs() < 1e-5,
            "{d:?}"
        );
        assert!(d[2].is_nan() && d[3].is_nan());
        for n in scan.normals.iter_mut().take(2) {
            *n = n.map(|c| -c);
        }
        let bore = measure(&scan, &e, &m);
        assert!(
            (bore[0] + 0.2).abs() < 1e-5 && (bore[1] - 0.1).abs() < 1e-5,
            "{bore:?}"
        );
    }
}
