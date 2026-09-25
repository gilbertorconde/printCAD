//! Sketch feature implementation for the document feature tree.

use core_document::{DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

use crate::sketch::{Sketch, SketchPlane};

/// A sketch feature that can be stored in the document's feature tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchFeature {
    /// The sketch data.
    pub sketch: Sketch,
    /// The reference plane for the sketch.
    pub plane: SketchPlane,
    /// The datum the sketch was drawn on, which its plane follows: moved,
    /// turned or flipped, the datum takes the sketch with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<DatumSupport>,
}

/// A sketch's place on a datum: a datum plane, or one of a coordinate
/// system's three planes, pushed `offset` along its normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatumSupport {
    pub datum: FeatureId,
    /// A coordinate system's plane: `XY`, `XZ` or `YZ`. A datum plane has
    /// only its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<String>,
    /// Millimetres along the plane's normal.
    #[serde(default)]
    pub offset: f32,
}

impl DatumSupport {
    /// The plane this support puts a sketch on, from the datum's `data`;
    /// `None` when it is not a datum plane or coordinate system.
    pub fn plane_from(&self, data: &serde_json::Value) -> Option<SketchPlane> {
        use core_document::{DatumFeature, DatumShape};
        let datum = DatumFeature::from_json(data).ok()?;
        let frame = match datum.shape {
            DatumShape::Plane { .. } => datum.frame(),
            DatumShape::CoordinateSystem { .. } => {
                let which = self.plane.as_deref().unwrap_or("XY");
                datum
                    .frame()
                    .planes()
                    .into_iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(which))?
                    .1
            }
            _ => return None,
        };
        let mut plane = SketchPlane {
            origin: frame.origin,
            normal: frame.normal,
            x_axis: frame.x_axis,
            y_axis: frame.y_axis(),
        };
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * self.offset;
        }
        Some(plane)
    }
}

impl SketchFeature {
    pub fn new(sketch: Sketch, plane: SketchPlane) -> Self {
        Self {
            sketch,
            plane,
            support: None,
        }
    }

    pub fn from_sketch(sketch: Sketch) -> Self {
        Self {
            sketch,
            plane: SketchPlane::default(),
            support: None,
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
        self.support.iter().map(|s| s.datum).collect()
    }

    fn name(&self) -> &str {
        &self.sketch.name
    }
}
