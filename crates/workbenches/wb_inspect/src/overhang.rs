//! How far a body's surface overhangs as it would print: at each point of
//! its mesh facing down, the angle past vertical it leans out, in degrees
//! (0 a wall, 90 a ceiling); surface facing up, and the face it stands on
//! the bed with, read unmeasured. Above the printer's limit (45° by
//! default) a print needs support there.

use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchId};
use kernel_api::TriMesh;
use serde::{Deserialize, Serialize};

/// The feature kind of every overhang map.
pub const KIND: &str = "wb.inspect.overhang";

/// How near its lowest point, mm, a face facing down stands on the bed.
pub const BED: f64 = 0.05;

/// One overhang map of a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overhang {
    pub name: String,
    pub body: BodyId,
    /// The steepest overhang the printer prints without support, degrees
    /// past vertical.
    #[serde(default = "forty_five")]
    pub limit_deg: f64,
}

fn forty_five() -> f64 {
    45.0
}

impl WorkbenchFeature for Overhang {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Each vertex's overhang past vertical, degrees, the mesh standing as it
/// is with +Z up, 0 on a wall; `NaN` facing up or on the bed (within `bed` of its
/// lowest point, facing straight down).
pub fn measure(mesh: &TriMesh, bed: f64) -> Vec<f32> {
    let lowest = mesh
        .positions
        .iter()
        .map(|p| f64::from(p[2]))
        .fold(f64::MAX, f64::min);
    mesh.positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let Some(n) = mesh.normals.get(i) else {
                return f32::NAN;
            };
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len <= 0.0 {
                return f32::NAN;
            }
            let down = -n[2] / len;
            let on_bed = f64::from(p[2]) - lowest <= bed && down > 0.99;
            if down < -1e-6 || on_bed {
                f32::NAN
            } else {
                down.clamp(0.0, 1.0).asin().to_degrees()
            }
        })
        .collect()
}

/// The share of the mesh's area that overhangs past `limit_deg`, and
/// the steepest overhang, from the values `measure` gave.
pub fn read(mesh: &TriMesh, values: &[f32], limit_deg: f64) -> (f64, f64) {
    let (mut over, mut total, mut steepest) = (0.0, 0.0, 0.0f64);
    for t in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = t.map(|v| glam::DVec3::from(mesh.positions[v as usize].map(f64::from)));
        let area = (b - a).cross(c - a).length() / 2.0;
        total += area;
        let worst = t
            .iter()
            .filter_map(|v| values.get(*v as usize).copied())
            .filter(|v| !v.is_nan())
            .fold(f32::NAN, f32::max);
        if !worst.is_nan() {
            steepest = steepest.max(f64::from(worst));
            if f64::from(worst) > limit_deg
                && t.iter()
                    .all(|v| values.get(*v as usize).is_some_and(|x| !x.is_nan()))
            {
                over += area;
            }
        }
    }
    (if total > 0.0 { over / total } else { 0.0 }, steepest)
}

/// The map of `o` on `mesh` from the `values` `measure` gave: walls blue, ceilings red, the limit marked
/// on the legend with the share of surface past it.
pub fn map(
    mesh: &TriMesh,
    revision: u64,
    values: std::sync::Arc<[f32]>,
    o: &Overhang,
    name: &str,
    none: [f32; 3],
) -> core_document::ScalarMap {
    let (share, steepest) = read(mesh, &values, o.limit_deg);
    let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let histogram = metrology::stats::Histogram::of(&wide, 0.0, 90.0, 18);
    let ramp = metrology::ramp::Ramp::Jet;
    let (low, high) = ramp.caps();
    core_document::ScalarMap {
        body: o.body,
        revision,
        values,
        range: [0.0, 90.0],
        bands: 0,
        ramp: (0..32).map(|i| ramp.at(f64::from(i) / 31.0)).collect(),
        under: low,
        over: high,
        none,
        legend: core_document::MapLegend {
            title: format!("{name} · overhangs"),
            unit: "°".to_string(),
            stats: vec![
                (
                    format!("Past {}°", o.limit_deg),
                    format!("{:.1} %", share * 100.0),
                ),
                ("Steepest".to_string(), format!("{steepest:.0}°")),
            ],
            histogram: histogram.counts.iter().map(|n| *n as u32).collect(),
            tolerance: Some((0.0, o.limit_deg as f32)),
        },
    }
}

impl crate::maps::Mapped for Overhang {
    const KIND: &'static str = KIND;
    const WORD: &'static str = "Overhang";
    /// The body's mesh as it stands, and the key.
    type Inputs = (std::sync::Arc<TriMesh>, u64);

    fn body(&self) -> BodyId {
        self.body
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn inputs(&self, document: &core_document::Document) -> Result<Self::Inputs, String> {
        let g = document
            .imported_geometry(self.body)
            .ok_or("the body has no geometry to read")?;
        Ok((g.mesh.clone(), g.revision ^ self.limit_deg.to_bits()))
    }

    fn key((_, key): &Self::Inputs) -> u64 {
        *key
    }

    fn measure(
        &self,
        (mesh, _): &Self::Inputs,
        _kernel: Option<&'static dyn kernel_api::KernelQueries>,
    ) -> Result<std::sync::Arc<[f32]>, String> {
        Ok(measure(mesh, BED).into())
    }

    fn paint(
        &self,
        id: FeatureId,
        name: &str,
        (mesh, key): &Self::Inputs,
        values: std::sync::Arc<[f32]>,
        _maps: &crate::maps::Maps,
        ctx: &core_document::WorkbenchRuntimeContext,
    ) -> Option<core_document::ScalarMap> {
        Some(map(
            mesh,
            key ^ id.0.as_u64_pair().1,
            values,
            self,
            name,
            ctx.sketch_palette.inactive,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall reads 0°, a face leaning 30° out 30°, a ceiling 90°,
    /// the face on the bed and the top unmeasured.
    #[test]
    fn overhangs_read_their_lean_past_vertical() {
        let (s, c) = 30f32.to_radians().sin_cos();
        let mesh = TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 5.0],
                [0.0, 0.0, 5.0],
                [0.0, 0.0, 9.0],
                [0.0, 0.0, 9.0],
            ],
            normals: vec![
                [0.0, 0.0, -1.0],
                [1.0, 0.0, 0.0],
                [c, 0.0, -s],
                [0.0, 0.0, -1.0],
                [0.0, 0.0, 1.0],
            ],
            ..TriMesh::default()
        };
        let v = measure(&mesh, 0.05);
        assert!(v[0].is_nan() && v[4].is_nan(), "{v:?}");
        assert!(
            v[1].abs() < 1e-4 && (v[2] - 30.0).abs() < 1e-3 && (v[3] - 90.0).abs() < 1e-3,
            "{v:?}"
        );
    }
}
