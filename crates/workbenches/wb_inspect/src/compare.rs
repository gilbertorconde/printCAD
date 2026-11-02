//! A scan compared with a reference: the signed distance of every scan
//! vertex to the reference's surface, positive where the scan stands
//! outside it, and what those distances say. The comparison keeps only
//! its settings; the distances are measured again whenever the scan, the
//! reference or where either sits changes.

use std::sync::Arc;

use core_document::{
    BodyId, BodyPlacement, Document, DocumentError, DocumentResult, FeatureId, WorkbenchFeature,
    WorkbenchId,
};
use kernel_api::{DistanceRequest, DistanceTarget, KernelQueries, TriMesh};
use metrology::ramp::Ramp;
use metrology::stats::{self, Histogram, Summary};
use serde::{Deserialize, Serialize};

/// The feature kind of every comparison.
pub const KIND: &str = "wb.inspect.comparison";

/// The widest range a map starts at, in millimetres.
const MOST_AUTO_RANGE: f64 = 1.0;

/// The legend's histogram bins.
pub const BINS: usize = 24;

/// One comparison of a scan with a reference body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub name: String,
    pub scan: BodyId,
    pub reference: BodyId,
    /// The half-width of the colour scale, in mm; worked out from the
    /// distances when `None`.
    #[serde(default)]
    pub range: Option<f64>,
    /// Steps of flat colour; 0 is continuous.
    #[serde(default)]
    pub bands: u32,
    /// The ramp by name (`jet`, `viridis`, …).
    #[serde(default = "default_ramp")]
    pub ramp: String,
    /// Distances within ± this count as in tolerance.
    #[serde(default)]
    pub tolerance: Option<f64>,
    /// Points farther than this from the reference are left unmeasured.
    #[serde(default)]
    pub max: Option<f64>,
    /// Only reference surface turned within this many degrees of a scan
    /// point's own normal is measured to.
    #[serde(default = "default_facing")]
    pub facing_limit_deg: Option<f64>,
}

fn default_ramp() -> String {
    Ramp::Jet.name().to_string()
}

fn default_facing() -> Option<f64> {
    Some(90.0)
}

impl WorkbenchFeature for Comparison {
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
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Comparison {
    pub fn ramp(&self) -> Ramp {
        Ramp::ALL
            .into_iter()
            .find(|r| r.name().eq_ignore_ascii_case(&self.ramp))
            .unwrap_or_default()
    }
}

/// What a comparison is measured from, taken out of the document so the
/// measuring can run away from it.
#[derive(Clone)]
pub struct Inputs {
    scan: Arc<TriMesh>,
    /// The scan's own frame to the reference's.
    scan_to_reference: BodyPlacement,
    reference: Reference,
    /// Changes whenever anything above does.
    pub key: u64,
}

#[derive(Clone)]
enum Reference {
    Shape(Arc<Vec<u8>>),
    Mesh(Arc<TriMesh>),
}

impl Inputs {
    /// The scan's points and normals in the reference's frame.
    pub fn points(&self) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
        let mesh = &self.scan;
        let to = &self.scan_to_reference;
        let points = mesh
            .positions
            .iter()
            .map(|p| to.point(*p).map(f64::from))
            .collect();
        let normals = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals
                .iter()
                .map(|n| to.direction(*n).map(f64::from))
                .collect()
        } else {
            Vec::new()
        };
        (points, normals)
    }

    /// The reference, as the kernel measures to it.
    pub fn target(&self) -> DistanceTarget<'_> {
        match &self.reference {
            Reference::Shape(blob) => DistanceTarget::Shape(blob),
            Reference::Mesh(mesh) => DistanceTarget::Mesh(mesh),
        }
    }

    /// The scan's own frame to the reference's.
    pub fn scan_to_reference(&self) -> BodyPlacement {
        self.scan_to_reference
    }
}

/// What `c` is measured from in `document` now.
pub fn inputs(document: &Document, c: &Comparison) -> Result<Inputs, String> {
    let mut inputs = inputs_for(document, c.scan, c.reference)?;
    inputs.key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        inputs.key.hash(&mut h);
        format!("{:?}{:?}", c.max, c.facing_limit_deg).hash(&mut h);
        h.finish()
    };
    Ok(inputs)
}

/// `scan` and `reference` as they stand in `document`, for measuring one
/// against the other.
pub fn inputs_for(document: &Document, scan: BodyId, reference: BodyId) -> Result<Inputs, String> {
    let (scan_mesh, _) = document
        .local_geometry(scan)
        .ok_or("the scan has no mesh")?;
    let target = match document.imported_brep_blob_arc(reference) {
        Some(blob) => Reference::Shape(blob),
        None => Reference::Mesh(
            document
                .local_geometry(reference)
                .ok_or("the reference has no geometry")?
                .0,
        ),
    };
    let scan_to_reference = document
        .body_placement(reference)
        .inverse()
        .after(&document.body_placement(scan));
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let revision = |b| document.imported_geometry(b).map(|g| g.revision);
        (revision(scan), revision(reference)).hash(&mut h);
        format!("{:?}", scan_to_reference.rows()).hash(&mut h);
        h.finish()
    };
    Ok(Inputs {
        scan: scan_mesh,
        scan_to_reference,
        reference: target,
        key,
    })
}

/// The distances measured, per scan vertex (`NaN` unmeasured).
pub fn measure(
    kernel: &dyn KernelQueries,
    inputs: &Inputs,
    c: &Comparison,
) -> Result<Arc<[f32]>, String> {
    let (points, normals) = inputs.points();
    let request = DistanceRequest {
        points,
        normals,
        max: c.max,
        facing_limit_deg: c.facing_limit_deg,
    };
    let distances = kernel
        .signed_distance(inputs.target(), &request)
        .map_err(|e| e.to_string())?;
    Ok(distances.into_iter().map(|d| d as f32).collect())
}

/// What a set of distances says.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub summary: Option<Summary>,
    /// The scale's half-width the map is drawn at.
    pub range: f64,
    pub histogram: Histogram,
    /// The share within the tolerance, from 0 to 1.
    pub within: Option<f64>,
}

/// Read `values` as `c` asks.
pub fn read(values: &[f32], c: &Comparison) -> Reading {
    let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let range = c.range.filter(|r| *r > 0.0).unwrap_or_else(|| {
        let sizes: Vec<f64> = wide.iter().map(|v| v.abs()).collect();
        auto_range(stats::percentile(&sizes, 95.0).unwrap_or(MOST_AUTO_RANGE))
    });
    Reading {
        summary: stats::summary(&wide),
        range,
        histogram: Histogram::of(&wide, -range, range, BINS),
        within: c.tolerance.and_then(|t| stats::share_within(&wide, -t, t)),
    }
}

/// `size` rounded up to 1, 2 or 5 times a power of ten, no wider than the
/// most a map starts at.
pub fn auto_range(size: f64) -> f64 {
    if size.is_nan() || size <= 0.0 {
        return MOST_AUTO_RANGE;
    }
    let decade = 10f64.powf(size.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * decade)
        .find(|s| *s >= size * (1.0 - 1e-9))
        .unwrap_or(10.0 * decade);
    step.min(MOST_AUTO_RANGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The automatic range rounds up to a round number and stops at 1 mm;
    /// the reading counts what lies within the tolerance and leaves the
    /// unmeasured out.
    #[test]
    fn a_reading_rounds_its_range_and_counts_its_tolerance() {
        for (size, want) in [(0.013, 0.02), (0.2, 0.2), (0.31, 0.5), (4.0, 1.0)] {
            assert!((auto_range(size) - want).abs() < 1e-12, "{size}");
        }
        let c = Comparison {
            name: "Compare 1".into(),
            scan: BodyId(uuid::Uuid::nil()),
            reference: BodyId(uuid::Uuid::nil()),
            range: None,
            bands: 0,
            ramp: default_ramp(),
            tolerance: Some(0.1),
            max: None,
            facing_limit_deg: default_facing(),
        };
        let values = [0.05, -0.05, 0.15, f32::NAN, 0.0];
        let r = read(&values, &c);
        assert_eq!(r.summary.map(|s| (s.count, s.unmeasured)), Some((4, 1)));
        assert!((r.within.unwrap() - 0.75).abs() < 1e-12);
        assert_eq!(r.histogram.counts.iter().sum::<usize>(), 4);
    }
}
