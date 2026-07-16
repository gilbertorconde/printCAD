//! Part Design feature payloads stored in the document feature tree.

use core_document::{DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

/// Which in-plane sketch axis a revolution spins about (through the sketch
/// origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RevolveAxis {
    /// The sketch's vertical (y) axis (the default).
    #[default]
    SketchY,
    /// The sketch's horizontal (x) axis.
    SketchX,
}

impl RevolveAxis {
    pub fn label(&self) -> &'static str {
        match self {
            RevolveAxis::SketchY => "Sketch Y axis",
            RevolveAxis::SketchX => "Sketch X axis",
        }
    }

    /// Direction in sketch 2D coordinates.
    pub fn dir_2d(&self) -> [f64; 2] {
        match self {
            RevolveAxis::SketchY => [0.0, 1.0],
            RevolveAxis::SketchX => [1.0, 0.0],
        }
    }
}

/// A solid-modeling feature. Each one consumes a sketch profile and either
/// adds or removes material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PartFeature {
    /// Extrude the sketch profile, adding material (fused with whatever the
    /// body already has; the body's first additive feature starts its solid).
    Pad {
        sketch: FeatureId,
        length: f32,
        /// Extrude along -normal instead of +normal.
        reversed: bool,
        /// Extrude half the length to each side of the sketch plane.
        #[serde(default)]
        symmetric: bool,
    },
    /// Extrude the sketch profile and subtract it from the body's solid.
    Pocket {
        sketch: FeatureId,
        depth: f32,
        /// Cut along -normal instead of +normal.
        reversed: bool,
        /// Cut through the entire body regardless of depth.
        #[serde(default)]
        through_all: bool,
    },
    /// Revolve the sketch profile about an in-plane axis, adding material.
    Revolution {
        sketch: FeatureId,
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        /// Revolve the opposite way around the axis.
        #[serde(default)]
        reversed: bool,
    },
    /// Revolve the sketch profile and subtract it.
    Groove {
        sketch: FeatureId,
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        #[serde(default)]
        reversed: bool,
    },
}

impl PartFeature {
    pub fn sketch(&self) -> FeatureId {
        match self {
            PartFeature::Pad { sketch, .. }
            | PartFeature::Pocket { sketch, .. }
            | PartFeature::Revolution { sketch, .. }
            | PartFeature::Groove { sketch, .. } => *sketch,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            PartFeature::Pad { .. } => "Pad",
            PartFeature::Pocket { .. } => "Pocket",
            PartFeature::Revolution { .. } => "Revolution",
            PartFeature::Groove { .. } => "Groove",
        }
    }

    /// True when this feature removes material (must not be a body's first).
    pub fn is_subtractive(&self) -> bool {
        matches!(
            self,
            PartFeature::Pocket { .. } | PartFeature::Groove { .. }
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_pad_json_without_new_fields_still_deserializes() {
        // A Pad serialized before `symmetric` existed.
        let old = serde_json::json!({
            "Pad": { "sketch": FeatureId::new(), "length": 5.0, "reversed": false }
        });
        let feature = PartFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            PartFeature::Pad {
                symmetric: false,
                ..
            }
        ));
    }
}
