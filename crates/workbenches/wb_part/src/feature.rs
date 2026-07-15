//! Part Design feature payloads stored in the document feature tree.

use core_document::{DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

/// A solid-modeling feature. Each one consumes a sketch profile and either
/// adds or removes material along the sketch plane's normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PartFeature {
    /// Extrude the sketch profile, adding material (fused with whatever the
    /// body already has; the body's first Pad starts its solid).
    Pad {
        sketch: FeatureId,
        length: f32,
        /// Extrude along -normal instead of +normal.
        reversed: bool,
    },
    /// Extrude the sketch profile and subtract it from the body's solid.
    Pocket {
        sketch: FeatureId,
        depth: f32,
        /// Cut along -normal instead of +normal.
        reversed: bool,
    },
}

impl PartFeature {
    pub fn sketch(&self) -> FeatureId {
        match self {
            PartFeature::Pad { sketch, .. } | PartFeature::Pocket { sketch, .. } => *sketch,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            PartFeature::Pad { .. } => "Pad",
            PartFeature::Pocket { .. } => "Pocket",
        }
    }
}

impl WorkbenchFeature for PartFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.part")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        vec![self.sketch()]
    }

    fn name(&self) -> &str {
        self.kind_label()
    }
}
