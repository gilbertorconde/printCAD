//! Sketch feature implementation for the document feature tree.

use core_document::{DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};
use serde_json;

use crate::sketch::{Sketch, SketchPlane};

/// A sketch feature that can be stored in the document's feature tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchFeature {
    /// The sketch data.
    pub sketch: Sketch,
    /// The reference plane for the sketch.
    pub plane: SketchPlane,
}

impl SketchFeature {
    pub fn new(sketch: Sketch, plane: SketchPlane) -> Self {
        Self { sketch, plane }
    }

    pub fn from_sketch(sketch: Sketch) -> Self {
        Self {
            sketch,
            plane: SketchPlane::default(),
        }
    }
}

impl WorkbenchFeature for SketchFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.sketch")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SketchFeature should always serialize")
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        // Sketches have no dependencies (they are root features)
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.sketch.name
    }
}
