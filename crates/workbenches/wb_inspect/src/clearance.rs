//! A fit check: how much room a part has in what it fits into, at every
//! point of the part's mesh, as the two sit now. A gap reads positive,
//! a point inside the other's material negative (they interfere). Either
//! may be a scan or a solid; a solid is measured to exactly.

use std::sync::Arc;

use core_document::{
    BodyId, Document, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use kernel_api::{DistanceRequest, KernelQueries};
use metrology::stats::{self, Histogram, Summary};
use serde::{Deserialize, Serialize};

use crate::compare::{self, BINS};

/// The feature kind of every fit check.
pub const KIND: &str = "wb.inspect.clearance";

/// How far from the housing a point is measured, mm, when not given.
pub const DEFAULT_REACH: f64 = 5.0;

/// One fit check of a part against what it fits into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clearance {
    pub name: String,
    /// The part the map paints.
    pub part: BodyId,
    /// What it fits into.
    pub housing: BodyId,
    /// Gaps narrower than this are counted as too tight.
    #[serde(default)]
    pub min_gap: Option<f64>,
    /// Points farther than this from the housing read unmeasured.
    #[serde(default = "default_reach")]
    pub reach: f64,
}

fn default_reach() -> f64 {
    DEFAULT_REACH
}

impl WorkbenchFeature for Clearance {
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

/// What `c` is measured from in `document` now.
pub fn inputs(document: &Document, c: &Clearance) -> Result<compare::Inputs, String> {
    if c.part == c.housing {
        return Err("the part and what it fits into are the same body".into());
    }
    let mut inputs = compare::inputs_for(document, c.part, c.housing)?;
    inputs.key ^= c.reach.to_bits().rotate_left(17);
    Ok(inputs)
}

/// The room at each of the part's vertices (`NaN` beyond `reach`).
pub fn measure(
    kernel: &dyn KernelQueries,
    inputs: &compare::Inputs,
    c: &Clearance,
) -> Result<Arc<[f32]>, String> {
    let (points, normals) = inputs.points();
    // The housing faces the part across the gap, so its surface turns
    // against the part's: no facing limit.
    let request = DistanceRequest {
        points,
        normals,
        max: Some(c.reach),
        facing_limit_deg: None,
    };
    let distances = kernel
        .signed_distance(inputs.target(), &request)
        .map_err(|e| e.to_string())?;
    Ok(distances.into_iter().map(|d| d as f32).collect())
}

/// What a fit check's distances say.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub summary: Option<Summary>,
    /// The scale's half-width, mm.
    pub range: f64,
    pub histogram: Histogram,
    /// The share of measured points inside the housing's material.
    pub interfering: f64,
    /// The deepest interference, mm (positive), when there is any.
    pub deepest: Option<f64>,
    /// The narrowest gap, mm, when nothing interferes.
    pub narrowest: Option<f64>,
    /// The share of measured points with less room than `min_gap`,
    /// interference included.
    pub too_tight: Option<f64>,
}

/// Read `values` as `c` asks.
pub fn read(values: &[f32], c: &Clearance) -> Reading {
    let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let measured: Vec<f64> = wide.iter().copied().filter(|v| !v.is_nan()).collect();
    let n = measured.len().max(1) as f64;
    let sizes: Vec<f64> = measured.iter().map(|v| v.abs()).collect();
    let range = compare::auto_range(
        stats::percentile(&sizes, 0.5)
            .unwrap_or(1.0)
            .max(c.min_gap.unwrap_or(0.0)),
    );
    let inside: Vec<f64> = measured.iter().copied().filter(|v| *v < 0.0).collect();
    Reading {
        summary: stats::summary(&wide),
        range,
        histogram: Histogram::of(&wide, -range, range, BINS),
        interfering: inside.len() as f64 / n,
        deepest: inside.iter().copied().reduce(f64::min).map(|d| -d),
        narrowest: if inside.is_empty() {
            measured.iter().copied().reduce(f64::min)
        } else {
            None
        },
        too_tight: c
            .min_gap
            .map(|g| measured.iter().filter(|v| **v < g).count() as f64 / n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interference counts the points inside the housing and reads its
    /// depth, and the share too tight leaves the unmeasured out; the
    /// narrowest gap is read only where nothing interferes.
    #[test]
    fn a_fit_reads_its_interference_and_its_narrowest_gap() {
        let c = Clearance {
            name: "Fit 1".into(),
            part: BodyId(uuid::Uuid::nil()),
            housing: BodyId(uuid::Uuid::nil()),
            min_gap: Some(0.2),
            reach: DEFAULT_REACH,
        };
        let r = read(&[-0.1, 0.05, 0.3, 0.5, f32::NAN], &c);
        assert!((r.interfering - 0.25).abs() < 1e-12);
        assert!((r.deepest.unwrap() - 0.1).abs() < 1e-6);
        assert!(r.narrowest.is_none());
        assert!((r.too_tight.unwrap() - 0.5).abs() < 1e-12);
        let clear = read(&[0.05, 0.3, f32::NAN], &c);
        assert!((clear.narrowest.unwrap() - 0.05).abs() < 1e-6 && clear.deepest.is_none());
    }
}
