//! The largest size a fillet or chamfer builds at, which a drag of its
//! size stops at: worked out by the kernel beside the task's preview build
//! (`SolidBuildResult::size_limit`), kept by the document per feature, and
//! read here while the inputs it was found for still hold. Until it is
//! known a drag goes where it is taken; a typed size is never held to it.

use core_document::{Document, FeatureId, WorkbenchFeature};

use crate::feature::{ChamferMode, DesignFeature};

/// The largest values a drag sets for a fillet's or chamfer's sizes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DragLimits {
    /// A fillet's radius, a chamfer's size.
    pub first: f32,
    /// A chamfer's second size, with two distances.
    pub second: Option<f32>,
}

/// `feature`'s drag limits, when its last preview build found them for the
/// edges and kind of size it has now; else, while it stands refused, the
/// largest size that fits all its edges.
pub(crate) fn drag_limits(document: &Document, feature: FeatureId) -> Option<DragLimits> {
    let node = document.get_feature_meta(feature)?;
    let part =
        DesignFeature::from_json(document.feature_values(feature).unwrap_or(&node.data)).ok()?;
    let (size, size2) = match &part {
        DesignFeature::Fillet { radius, .. } => (*radius, None),
        DesignFeature::Chamfer {
            size, mode, size2, ..
        } => (
            *size,
            (*mode == ChamferMode::TwoDistances).then_some(*size2),
        ),
        _ => return None,
    };
    let key = crate::build::edge_op(&part).ok()??.size_limit_key()?;
    let largest = match document.size_limit(feature) {
        Some(limit) if limit.key == key => limit.largest? as f32,
        _ => crate::failing::largest_size(document, feature)?,
    };
    let first = hundredths_below(largest);
    (first > 0.0).then(|| DragLimits {
        first,
        second: size2.map(|s2| hundredths_below(s2 * largest / size)),
    })
}

/// The largest a drag sets the number at `pointer` of `feature` to, when
/// it is a fillet's or chamfer's size with a known limit.
pub(crate) fn drag_most(document: &Document, feature: FeatureId, pointer: &str) -> Option<f32> {
    let limits = drag_limits(document, feature)?;
    match pointer {
        "/Fillet/radius" | "/Chamfer/size" => Some(limits.first),
        "/Chamfer/size2" => limits.second,
        _ => None,
    }
}

/// `size` rounded down to a hundredth of a millimetre, as fields show it:
/// never past what was found to build.
fn hundredths_below(size: f32) -> f32 {
    ((f64::from(size) * 100.0 + 1e-6).floor() / 100.0) as f32
}
