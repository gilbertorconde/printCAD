//! The largest size a fillet or chamfer builds at, which a drag of its
//! size stops at: worked out by the kernel beside the task's preview build
//! (`SolidBuildResult::size_limit`), kept by the document per feature, and
//! read here while the inputs it was found for still hold. Until it is
//! known a drag goes where it is taken; a typed size is never held to it.

use core_document::{Document, FeatureId, WorkbenchFeature};

use crate::feature::{ChamferMode, DesignFeature, FilletMode};

/// The largest values a drag sets for a fillet's or chamfer's sizes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DragLimits {
    /// A fillet's radius, a chamfer's size.
    pub first: Option<f32>,
    /// A fillet's or chamfer's second size, with two distances.
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
        DesignFeature::Fillet {
            radius,
            mode,
            radius2,
            ..
        } => (
            *radius,
            (*mode == FilletMode::TwoDistances).then_some(*radius2),
        ),
        DesignFeature::Chamfer {
            size, mode, size2, ..
        } => (
            *size,
            (*mode == ChamferMode::TwoDistances).then_some(*size2),
        ),
        _ => return None,
    };
    let key = crate::build::edge_op(&part).ok()??.size_limit_key()?;
    let limit = document
        .size_limit(feature)
        .filter(|limit| limit.key == key);
    // A fillet's two distances each stop where their own face runs out.
    if let DesignFeature::Fillet {
        mode: FilletMode::TwoDistances,
        ..
    } = part
        && let Some(sides) = limit.and_then(|limit| limit.sides)
    {
        let [first, second] = sides.sides.map(|side| {
            side.largest
                .map(|most| hundredths_below(most as f32))
                .filter(|most| *most > 0.0)
        });
        return (first.is_some() || second.is_some()).then_some(DragLimits { first, second });
    }
    let largest = match limit {
        Some(limit) => limit.largest? as f32,
        None => crate::failing::largest_size(document, feature)?,
    };
    let first = hundredths_below(largest);
    (first > 0.0).then(|| DragLimits {
        first: Some(first),
        second: size2.map(|s2| hundredths_below(s2 * largest / size)),
    })
}

/// The largest a drag sets the number at `pointer` of `feature` to, when
/// it is a fillet's or chamfer's size with a known limit.
pub(crate) fn drag_most(document: &Document, feature: FeatureId, pointer: &str) -> Option<f32> {
    let limits = drag_limits(document, feature)?;
    match pointer {
        "/Fillet/radius" | "/Chamfer/size" => limits.first,
        "/Fillet/radius2" | "/Chamfer/size2" => limits.second,
        _ => None,
    }
}

/// `size` rounded down to a hundredth of a millimetre, as fields show it:
/// never past what was found to build.
fn hundredths_below(size: f32) -> f32 {
    ((f64::from(size) * 100.0 + 1e-6).floor() / 100.0) as f32
}
