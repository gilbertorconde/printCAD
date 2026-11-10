//! Readings of a map at a point: under the cursor while hovering, and kept
//! as pins on the part. A pin holds its map and where it sits on the scan;
//! its value is read again from the map whenever the map changes.

use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchId};
use kernel_api::TriMesh;
use serde::{Deserialize, Serialize};

/// The feature kind of every pinned reading.
pub const KIND: &str = "wb.inspect.pin";

/// A reading kept on the part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub name: String,
    pub scan: BodyId,
    /// The map it reads.
    pub map: FeatureId,
    /// Where it sits, in the scan's frame.
    pub at: [f64; 3],
}

impl WorkbenchFeature for Pin {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        vec![self.map]
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// The map's value at `at` (in the mesh's frame): the value of the nearest
/// vertex within `reach`, `None` past it or where unmeasured.
pub fn read(mesh: &TriMesh, values: &[f32], at: [f64; 3], reach: f64) -> Option<f64> {
    let mut best = (reach * reach, None);
    for (p, v) in mesh.positions.iter().zip(values) {
        let d: f64 = (0..3).map(|i| (f64::from(p[i]) - at[i]).powi(2)).sum();
        if d <= best.0 {
            best = (d, Some(*v));
        }
    }
    best.1.filter(|v| !v.is_nan()).map(f64::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reading takes the nearest vertex's value, and none past its reach
    /// or where the map is unmeasured.
    #[test]
    fn a_reading_takes_the_nearest_value() {
        let mesh = TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            ..TriMesh::default()
        };
        let values = [0.1, -0.2, f32::NAN];
        assert_eq!(
            read(&mesh, &values, [0.9, 0.1, 0.0], 0.5),
            Some(-0.2f32 as f64)
        );
        assert_eq!(read(&mesh, &values, [5.0, 0.0, 0.0], 0.5), None);
        assert_eq!(read(&mesh, &values, [0.0, 0.95, 0.0], 0.5), None);
    }
}
