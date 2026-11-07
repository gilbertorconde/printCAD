//! A profile that nearly closes, closed when a feature is made from it:
//! a Pad, Pocket, Revolution or Groove on a sketch whose curves stop short
//! of each other has its near-miss ends joined (the sketcher's repair), in
//! the same step that makes the feature, and says what it joined; the
//! task's notice puts the sketch back. A sketch the repair cannot close is
//! left as it is, and the feature's error names its loose ends.

use core_document::{Document, FeatureId, WorkbenchFeature};
use wb_sketch::SketchFeature;

use crate::feature::DesignFeature;

/// How far apart, as a share of the sketch's size, two ends may be and
/// still be joined.
const REACH: f64 = 0.005;
/// The reach's bounds, mm.
const LEAST_REACH: f64 = 1e-3;
const MOST_REACH: f64 = 0.5;

/// A sketch closed for a feature: what it was, and what the repair did.
#[derive(Debug, Clone)]
pub(crate) struct SketchRepair {
    pub sketch: FeatureId,
    pub before: serde_json::Value,
    pub done: wb_sketch::repair::Repaired,
}

impl SketchRepair {
    /// "Joined 2 ends of Sketch001 to close its profile".
    pub fn describe(&self, document: &Document) -> String {
        let name = document
            .get_feature_meta(self.sketch)
            .map_or_else(|| "the sketch".to_string(), |n| n.name.clone());
        format!("Closed the profile of {name}: {}", self.done.describe())
    }

    /// Put the sketch back as it was before the repair.
    pub fn undo(&self, document: &mut Document) -> Result<(), String> {
        document
            .update_feature_data(self.sketch, self.before.clone())
            .map_err(|e| e.to_string())?;
        document.mark_feature_dirty(self.sketch);
        Ok(())
    }
}

/// Close the profile `feature` is made from, when it is a sweep of a
/// sketch whose curves leave loose ends that a join within reach closes.
/// `None` when the profile closes already, the feature sweeps no sketch,
/// or the repair would leave it open.
pub(crate) fn close_profile(
    document: &mut Document,
    feature: &DesignFeature,
) -> Option<SketchRepair> {
    if !matches!(
        feature,
        DesignFeature::Pad { .. }
            | DesignFeature::Pocket { .. }
            | DesignFeature::Revolution { .. }
            | DesignFeature::Groove { .. }
    ) {
        return None;
    }
    let sketch = feature.sketch()?;
    let before = document.get_feature_data(sketch)?.clone();
    let mut drawn = SketchFeature::from_json(&before).ok()?;
    let loose = wb_sketch::profile::loose_ends(&drawn.sketch).len();
    if loose == 0 {
        return None;
    }
    let reach = (size_of(&drawn.sketch) * REACH).clamp(LEAST_REACH, MOST_REACH);
    let done = wb_sketch::repair::repair(&mut drawn.sketch, reach);
    if done.joined + done.closed == 0 || !wb_sketch::profile::loose_ends(&drawn.sketch).is_empty() {
        return None;
    }
    document.update_feature_data(sketch, drawn.to_json()).ok()?;
    document.mark_feature_dirty(sketch);
    Some(SketchRepair {
        sketch,
        before,
        done,
    })
}

/// The diagonal of the box round a sketch's points, mm.
fn size_of(sketch: &wb_sketch::sketch::Sketch) -> f64 {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for g in &sketch.geometry {
        if let wb_sketch::sketch::GeometryElement::Point(p) = g {
            for (k, c) in [p.position.x, p.position.y].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
    }
    if lo[0] > hi[0] {
        return 0.0;
    }
    (hi[0] - lo[0]).hypot(hi[1] - lo[1])
}
