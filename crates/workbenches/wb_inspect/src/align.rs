//! Alignment: moving a scan (its body placement, never the reference) so it
//! sits where it should be measured. Each alignment is kept as a feature
//! saying how it was made and the placement it set; everything measured on
//! the scan is in the scan's own frame, so it comes along.

use core_document::{
    BodyId, BodyPlacement, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use glam::{DMat3, DQuat, DVec3};
use kernel_api::{KernelQueries, RegisterRequest, Registration};
use serde::{Deserialize, Serialize};

use crate::compare::Inputs;

/// The feature kind of every alignment.
pub const KIND: &str = "wb.inspect.alignment";

/// One alignment of a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    pub name: String,
    pub scan: BodyId,
    pub how: How,
    /// Where the scan sat before, and where the alignment put it.
    pub before: BodyPlacement,
    pub after: BodyPlacement,
}

/// How an alignment was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Fitted onto a reference body's surface.
    BestFit {
        reference: BodyId,
        /// Whether every principal-axis matching was tried as well.
        search: bool,
        rms: f64,
        inliers: usize,
        sampled: usize,
        /// How many motions the fit could not pin down.
        free: usize,
        /// Whether its rounds settled before their limit.
        #[serde(default = "settled")]
        converged: bool,
    },
    /// Moved and turned by numbers: X, then Y, then Z about the origin,
    /// then the move.
    Numbers {
        translate: [f64; 3],
        rotate_deg: [f64; 3],
    },
    /// Back where the scanner put it.
    Reset,
}

impl WorkbenchFeature for Alignment {
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

fn settled() -> bool {
    true
}

impl How {
    pub fn word(&self) -> &'static str {
        match self {
            How::BestFit { .. } => "Best fit",
            How::Numbers { .. } => "Moved by numbers",
            How::Reset => "Reset alignment",
        }
    }

    /// One line saying what it came to.
    pub fn summary(&self) -> String {
        match self {
            How::BestFit {
                rms,
                inliers,
                free,
                converged,
                ..
            } => {
                let mut s = format!("RMS {rms:.4} mm over {inliers} points");
                if !converged {
                    s.push_str(" · still moving when its rounds ran out");
                }
                if *free > 0 {
                    s.push_str(&format!(" · {free} motion(s) left free: it may slide"));
                }
                s
            }
            How::Numbers {
                translate,
                rotate_deg,
            } => format!(
                "turned {:.2}°, {:.2}°, {:.2}°, moved {:.3}, {:.3}, {:.3} mm",
                rotate_deg[0],
                rotate_deg[1],
                rotate_deg[2],
                translate[0],
                translate[1],
                translate[2]
            ),
            How::Reset => "the scanner's frame".to_string(),
        }
    }
}

/// The scan fitted onto its reference: where the scan goes, and the fit.
pub fn best_fit(
    kernel: &dyn KernelQueries,
    inputs: &Inputs,
    reference: BodyPlacement,
    search: bool,
) -> Result<(BodyPlacement, Registration), String> {
    let (points, normals) = inputs.points();
    let request = RegisterRequest {
        points,
        normals,
        search,
        max_distance: None,
        facing_limit_deg: None,
    };
    let fit = kernel
        .register(inputs.target(), &request)
        .map_err(|e| e.to_string())?;
    let motion = placement_of(&fit.transform);
    // The scan's frame goes to the reference's, through the fit, and back
    // out where the reference sits.
    let after = reference.after(&motion.after(&inputs.scan_to_reference()));
    Ok((after, fit))
}

/// The placement turned X, then Y, then Z degrees about the origin, then
/// moved.
pub fn by_numbers(translate: [f64; 3], rotate_deg: [f64; 3]) -> BodyPlacement {
    let [x, y, z] = rotate_deg.map(f64::to_radians);
    let turn = DQuat::from_rotation_z(z) * DQuat::from_rotation_y(y) * DQuat::from_rotation_x(x);
    BodyPlacement::new(turn.as_quat(), DVec3::from_array(translate).as_vec3())
}

/// A rigid motion given as three row-major rows.
fn placement_of(rows: &[[f64; 4]; 3]) -> BodyPlacement {
    let m = DMat3::from_cols_array_2d(&[
        [rows[0][0], rows[1][0], rows[2][0]],
        [rows[0][1], rows[1][1], rows[2][1]],
        [rows[0][2], rows[1][2], rows[2][2]],
    ]);
    BodyPlacement::new(
        DQuat::from_mat3(&m).as_quat(),
        DVec3::new(rows[0][3], rows[1][3], rows[2][3]).as_vec3(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A motion read from rows is the one written there, and turning by
    /// numbers goes X, then Y, then Z.
    #[test]
    fn placements_read_and_turn_as_written() {
        let p = by_numbers([1.0, 2.0, 3.0], [0.0, 0.0, 90.0]);
        let moved = p.point([1.0, 0.0, 0.0]);
        assert!((moved[0] - 1.0).abs() < 1e-6 && (moved[1] - 3.0).abs() < 1e-6);
        let r = p.rows();
        let back = placement_of(&[r[0], r[1], r[2]]);
        let q = back.point([0.3, -0.7, 2.0]);
        let w = p.point([0.3, -0.7, 2.0]);
        assert!((0..3).all(|i| (q[i] - w[i]).abs() < 1e-5));
        let xy = by_numbers([0.0; 3], [90.0, 0.0, 90.0]).point([0.0, 1.0, 0.0]);
        assert!(
            (xy[0] + 0.0).abs() < 1e-6 && (xy[1] - 0.0).abs() < 1e-6 && (xy[2] - 1.0).abs() < 1e-6,
            "{xy:?}"
        );
    }
}
