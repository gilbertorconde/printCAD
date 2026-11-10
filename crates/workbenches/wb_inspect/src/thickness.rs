//! Wall thickness: how thick a part is at every point of its mesh, read
//! inward from the point by rays or by the largest ball that fits, on a
//! scan or on a solid (exactly, from its own surfaces). It needs no
//! reference and no alignment.

use std::sync::Arc;

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature, WorkbenchId};
use kernel_api::{DistanceTarget, KernelQueries, ThicknessRequest, TriMesh};
use metrology::stats::{self, Histogram, Summary};
use serde::{Deserialize, Serialize};

/// The feature kind of every thickness map.
pub const KIND: &str = "wb.inspect.thickness";

/// Surfaces within this many degrees of edge-on to a ray are passed over.
const FACING_LIMIT_DEG: f64 = 60.0;

/// One thickness map of a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WallThickness {
    pub name: String,
    pub body: BodyId,
    /// The largest ball that fits, in place of rays.
    #[serde(default)]
    pub sphere: bool,
    /// Walls thinner than this are counted, and draw past the scale's
    /// hot end.
    #[serde(default)]
    pub min_wall: Option<f64>,
    /// Walls thicker than this read unmeasured; a fifth of the body's
    /// diagonal when `None`.
    #[serde(default)]
    pub max: Option<f64>,
}

impl WorkbenchFeature for WallThickness {
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

/// What a thickness map is read from, out of the document.
#[derive(Clone)]
pub struct Inputs {
    mesh: Arc<TriMesh>,
    shape: Option<Arc<Vec<u8>>>,
    pub key: u64,
}

/// What `t` is read from in `document` now.
pub fn inputs(document: &Document, t: &WallThickness) -> Result<Inputs, String> {
    let (mesh, _) = document
        .local_geometry(t.body)
        .ok_or("the body has no geometry")?;
    if mesh.normals.len() != mesh.positions.len() {
        return Err("the body's mesh has no normals to read inward along".into());
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        document
            .imported_geometry(t.body)
            .map(|g| g.revision)
            .hash(&mut h);
        format!("{}{:?}", t.sphere, t.max).hash(&mut h);
        h.finish()
    };
    Ok(Inputs {
        shape: document.imported_brep_blob_arc(t.body),
        mesh,
        key,
    })
}

/// The thickness at every vertex of the body's mesh (`NaN` unmeasured).
pub fn measure(
    kernel: &dyn KernelQueries,
    inputs: &Inputs,
    t: &WallThickness,
) -> Result<Arc<[f32]>, String> {
    let mesh = &inputs.mesh;
    let max = t.max.unwrap_or_else(|| {
        mesh.bounds()
            .map(|(lo, hi)| {
                let d: f32 = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum();
                f64::from(d.sqrt()) / 5.0
            })
            .unwrap_or(f64::INFINITY)
    });
    let request = ThicknessRequest {
        points: mesh.positions.iter().map(|p| p.map(f64::from)).collect(),
        normals: mesh.normals.iter().map(|n| n.map(f64::from)).collect(),
        sphere: t.sphere,
        cone_deg: 0.0,
        rays: 1,
        facing_limit_deg: FACING_LIMIT_DEG,
        max: Some(max),
    };
    let target = match &inputs.shape {
        Some(blob) => DistanceTarget::Shape(blob),
        None => DistanceTarget::Mesh(mesh),
    };
    let values = kernel
        .thickness(target, &request)
        .map_err(|e| e.to_string())?;
    Ok(values.into_iter().map(|v| v as f32).collect())
}

/// What a set of thicknesses says.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub summary: Option<Summary>,
    /// The scale's ends: the 2nd and 95th percentiles.
    pub range: [f64; 2],
    pub histogram: Histogram,
    /// The share thinner than the least wall.
    pub thinner: Option<f64>,
}

/// Read `values` as `t` asks.
pub fn read(values: &[f32], t: &WallThickness) -> Reading {
    let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let low = stats::percentile(&wide, 0.02).unwrap_or(0.0);
    let high = stats::percentile(&wide, 0.95).unwrap_or(1.0);
    let range = if high > low {
        [low, high]
    } else {
        [low, low + 1.0]
    };
    Reading {
        summary: stats::summary(&wide),
        histogram: Histogram::of(&wide, range[0], range[1], crate::compare::BINS),
        range,
        thinner: t
            .min_wall
            .and_then(|m| stats::share_within(&wide, f64::NEG_INFINITY, m)),
    }
}

impl crate::maps::Mapped for WallThickness {
    const KIND: &'static str = KIND;
    const WORD: &'static str = "Thickness";
    type Inputs = Inputs;

    fn body(&self) -> BodyId {
        self.body
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn inputs(&self, document: &Document) -> Result<Inputs, String> {
        inputs(document, self)
    }

    fn key(inputs: &Inputs) -> u64 {
        inputs.key
    }

    fn measure(
        &self,
        inputs: &Inputs,
        kernel: Option<&'static dyn KernelQueries>,
    ) -> Result<Arc<[f32]>, String> {
        measure(kernel.ok_or("no kernel to measure with")?, inputs, self)
    }

    /// Thin walls hot: the ramp runs red at the thin end.
    fn paint(
        &self,
        id: FeatureId,
        name: &str,
        inputs: &Inputs,
        values: Arc<[f32]>,
        _maps: &crate::maps::Maps,
        ctx: &core_document::WorkbenchRuntimeContext,
    ) -> Option<core_document::ScalarMap> {
        let r = read(&values, self);
        let ramp = metrology::ramp::Ramp::Jet;
        let (thick_cap, thin_cap) = ramp.caps();
        let mut stats = Vec::new();
        if let Some(s) = r.summary {
            for (name, v) in [("Mean", s.mean), ("Thinnest", s.min), ("Thickest", s.max)] {
                stats.push((
                    name.to_string(),
                    core_document::units::show_mm(v, ctx.document.display_unit()),
                ));
            }
            if s.unmeasured > 0 {
                stats.push(("Unmeasured".to_string(), s.unmeasured.to_string()));
            }
        }
        if let (Some(m), Some(share)) = (self.min_wall, r.thinner) {
            stats.push((
                format!("Thinner than {m}"),
                format!("{:.1} %", share * 100.0),
            ));
        }
        Some(core_document::ScalarMap {
            body: self.body,
            revision: inputs.key ^ id.0.as_u64_pair().1,
            values,
            range: [r.range[0] as f32, r.range[1] as f32],
            bands: 0,
            ramp: (0..32)
                .map(|i| ramp.at(1.0 - f64::from(i) / 31.0))
                .collect(),
            under: thin_cap,
            over: thick_cap,
            none: ctx.sketch_palette.inactive,
            legend: core_document::MapLegend {
                title: format!("{name} · wall thickness"),
                unit: "mm".to_string(),
                stats,
                histogram: r.histogram.counts.iter().map(|n| *n as u32).collect(),
                tolerance: self.min_wall.map(|m| (r.range[0] as f32, m as f32)),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scale spans the walls between their 2nd and 95th percentiles,
    /// and the share under the least wall leaves the unmeasured out.
    #[test]
    fn a_reading_spans_its_walls_and_counts_the_thin() {
        let t = WallThickness {
            name: "Thickness 1".into(),
            body: BodyId(uuid::Uuid::nil()),
            sphere: false,
            min_wall: Some(1.0),
            max: None,
        };
        let mut values: Vec<f32> = (1..=100).map(|i| i as f32 * 0.1).collect();
        values.push(f32::NAN);
        let r = read(&values, &t);
        assert!(r.range[0] < 0.5 && r.range[1] > 9.0, "{:?}", r.range);
        assert!((r.thinner.unwrap() - 0.10).abs() < 1e-9, "{:?}", r.thinner);
        assert_eq!(r.summary.map(|s| s.unmeasured), Some(1));
    }
}
