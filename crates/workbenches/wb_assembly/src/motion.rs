//! Motion over time: several joints driven at once, each by a formula of
//! the time `t` in seconds, from a start to an end in steps. The frames
//! are solved on a copy of the document; the document itself is not moved.

use core_document::expr::Dim;
use core_document::{Document, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

use crate::joint::{JointFeature, JointKind};

/// The feature kind motion studies are stored as.
pub const MOTION_KIND: &str = "wb.assembly.motion";

/// A joint driven by a formula of `t`: a hinge's angle in degrees, a
/// slider's position in millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedDrive {
    pub joint: FeatureId,
    pub formula: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionStudy {
    pub start: f32,
    pub end: f32,
    pub step: f32,
    pub drives: Vec<TimedDrive>,
}

impl Default for MotionStudy {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: 2.0,
            step: 0.05,
            drives: Vec::new(),
        }
    }
}

impl WorkbenchFeature for MotionStudy {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(MOTION_KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Motion"
    }
}

/// `formula` with every bare `t` read as `t` seconds.
fn with_time(formula: &str, t: f64) -> String {
    let chars: Vec<char> = formula.chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '`' || c == '.';
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1).copied();
        if *c == 't' && !before.is_some_and(word) && !after.is_some_and(word) {
            out.push_str(&format!("({t})"));
        } else {
            out.push(*c);
        }
    }
    out
}

/// The value `formula` gives at `t` seconds for a drive that takes
/// `want`: degrees for an angle, millimetres for a length. A plain number
/// (`30 * sin(t * 90°)` too) takes the drive's unit; any other kind is
/// refused.
pub fn value_at(formula: &str, t: f64, want: Dim) -> Result<f64, String> {
    let ctx = core_document::expr::Context {
        length_unit: core_document::units::Unit::Mm,
        resolve: &core_document::expr::NoReferences,
    };
    let q = core_document::expr::evaluate(&with_time(formula, t), &ctx)
        .map_err(|e| format!("{formula}: {e}"))?;
    if q.dim != want && q.dim != Dim::NUMBER {
        return Err(format!(
            "{formula}: this gives {} where {} is needed",
            q.dim.describe(),
            want.describe()
        ));
    }
    Ok(q.value)
}

/// What a drive of `joint` takes: an angle for a hinge, a length for a
/// slider.
pub fn drive_dim(document: &Document, joint: FeatureId) -> Result<Dim, String> {
    let feature = document
        .get_feature_meta(joint)
        .filter(|n| n.workbench_id.as_str() == crate::JOINT_KIND)
        .and_then(|n| JointFeature::from_json(&n.data).ok())
        .ok_or("a driven joint is gone")?;
    match feature.kind {
        JointKind::Hinge { .. } => Ok(Dim::ANGLE),
        JointKind::Slider { .. } => Ok(Dim::LENGTH),
        _ => Err("only hinges and sliders are driven".into()),
    }
}

impl MotionStudy {
    /// The times of its frames, start to end.
    pub fn times(&self) -> Vec<f32> {
        let step = self.step.max(1e-3);
        let count = (((self.end - self.start) / step).floor().max(0.0) as usize).min(10_000);
        (0..=count).map(|i| self.start + step * i as f32).collect()
    }

    /// Every body's placement at each frame, each drive held where its
    /// formula says, solved on a copy of `document`.
    pub fn frames(&self, document: &Document) -> Result<crate::Frames, String> {
        let mut copy = document.clone();
        let mut frames = Vec::new();
        for t in self.times() {
            for drive in &self.drives {
                let want = drive_dim(&copy, drive.joint)?;
                let value = value_at(&drive.formula, f64::from(t), want)? as f32;
                let mut feature = copy
                    .get_feature_data(drive.joint)
                    .and_then(|d| JointFeature::from_json(d).ok())
                    .ok_or("a driven joint is gone")?;
                match &mut feature.kind {
                    JointKind::Hinge { drive: d, .. } | JointKind::Slider { drive: d, .. } => {
                        d.to = Some(value);
                    }
                    _ => return Err("only hinges and sliders are driven".into()),
                }
                copy.update_feature_data(drive.joint, feature.to_json())
                    .map_err(|e| e.to_string())?;
            }
            if let Ok(moves) = crate::solve(&copy) {
                crate::place_bodies(&mut copy, &moves);
            }
            frames.push((
                t,
                copy.bodies().iter().map(|b| (b.id, b.placement)).collect(),
            ));
        }
        Ok(frames)
    }
}

/// A point of `body` (in its own frame) followed through `frames`: where
/// it is at each, and how fast it goes there (mm/s, by the frames either
/// side).
pub fn trace(
    frames: &crate::Frames,
    body: core_document::BodyId,
    point: [f32; 3],
) -> Vec<(f32, [f32; 3], f32)> {
    let at: Vec<(f32, glam::Vec3)> = frames
        .iter()
        .filter_map(|(t, placements)| {
            let (_, p) = placements.iter().find(|(b, _)| *b == body)?;
            Some((*t, glam::Vec3::from_array(p.point(point))))
        })
        .collect();
    (0..at.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(1), (i + 1).min(at.len() - 1));
            let dt = at[b].0 - at[a].0;
            let speed = if dt > 0.0 {
                at[b].1.distance(at[a].1) / dt
            } else {
                0.0
            };
            (at[i].0, at[i].1.to_array(), speed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_formula_reads_the_time() {
        assert_eq!(value_at("90 * t", 0.5, Dim::ANGLE).unwrap(), 45.0);
        assert!((value_at("30 * sin(t * 90°)", 1.0, Dim::LENGTH).unwrap() - 30.0).abs() < 1e-9);
        assert!((value_at("1 in * t", 2.0, Dim::LENGTH).unwrap() - 50.8).abs() < 1e-9);
        assert_eq!(with_time("tan(t) + t2 + a.t", 2.0), "tan((2)) + t2 + a.t");
        let study = MotionStudy {
            start: 0.0,
            end: 1.0,
            step: 0.25,
            drives: Vec::new(),
        };
        assert_eq!(study.times(), [0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn a_formula_of_the_wrong_kind_is_refused() {
        // A hinge turns by angles, a slider moves by lengths.
        assert!(value_at("1 in * t", 1.0, Dim::ANGLE).is_err());
        assert!(value_at("90° * t", 1.0, Dim::LENGTH).is_err());
        assert_eq!(
            value_at("1 rad * t", 1.0, Dim::ANGLE).unwrap().round(),
            57.0
        );
    }

    #[test]
    fn a_traced_point_has_its_path_and_speed() {
        use core_document::{BodyId, BodyPlacement};
        let body = BodyId::new();
        // Turning about Z at 90° a second, a point 10 out.
        let frames: crate::Frames = (0..=4)
            .map(|i| {
                let t = i as f32 * 0.25;
                let turn = glam::Quat::from_rotation_z((90.0 * t).to_radians());
                (t, vec![(body, BodyPlacement::new(turn, glam::Vec3::ZERO))])
            })
            .collect();
        let path = trace(&frames, body, [10.0, 0.0, 0.0]);
        assert_eq!(path.len(), 5);
        assert!((path[4].1[1] - 10.0).abs() < 1e-4);
        let expected = 10.0 * std::f32::consts::FRAC_PI_2;
        // By the chord across the frames either side: a little under the
        // arc at this coarse step.
        assert!(
            (path[2].2 - expected).abs() / expected < 0.05,
            "{}",
            path[2].2
        );
    }
}
