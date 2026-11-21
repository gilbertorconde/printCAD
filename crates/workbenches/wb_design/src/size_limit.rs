//! The largest size a fillet or chamfer builds at, which a drag of its
//! size stops at: worked out by the kernel for the task's preview build,
//! in it or after it (`SolidBuildResult::size_limit`, or a reading the app
//! lands later), kept by the document per feature, and
//! read here while the inputs it was found for still hold. Until it is
//! known a drag goes where it is taken; a typed size is never held to it.

use core_document::{Document, FeatureId, WorkbenchFeature};

use crate::feature::{ChamferMode, DesignFeature, FilletMode};

/// The largest values a drag sets for a fillet's or chamfer's sizes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragLimits {
    /// A fillet's radius (a variable fillet's start radius), a chamfer's
    /// size.
    pub first: Option<f32>,
    /// A fillet's or chamfer's second size, with two distances; a variable
    /// fillet's end radius.
    pub second: Option<f32>,
    /// A variable fillet's radii between, in the order of its points.
    pub points: Vec<Option<f32>>,
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
    let op = crate::build::edge_op(&part).ok()??;
    let key = op.size_limit_key()?;
    let limit = document
        .derived()
        .size_limit(feature)
        .filter(|limit| limit.key == key);
    if let DesignFeature::Fillet {
        mode: FilletMode::Variable,
        points,
        ..
    } = &part
    {
        return law_limits(&op, limit?, points);
    }
    // Two distances each stop where their own face runs out.
    let two = matches!(
        part,
        DesignFeature::Fillet {
            mode: FilletMode::TwoDistances,
            ..
        } | DesignFeature::Chamfer {
            mode: ChamferMode::TwoDistances,
            ..
        }
    );
    if two && let Some(sides) = limit.and_then(|limit| limit.sides) {
        let [first, second] = sides.sides.map(|side| below(side.largest));
        return (first.is_some() || second.is_some()).then_some(DragLimits {
            first,
            second,
            points: Vec::new(),
        });
    }
    let largest = match limit {
        Some(limit) => limit.largest? as f32,
        None => crate::failing::largest_size(document, feature)?,
    };
    let first = hundredths_below(largest);
    (first > 0.0).then(|| DragLimits {
        first: Some(first),
        second: size2.map(|s2| hundredths_below(s2 * largest / size)),
        points: Vec::new(),
    })
}

/// A variable fillet's limits: each radius's own, found with the others
/// as they are now, by the key of the op with that radius left out.
fn law_limits(
    op: &kernel_api::SolidOp,
    limit: &kernel_api::SizeLimit,
    points: &[crate::feature::RadiusPoint],
) -> Option<DragLimits> {
    let most = |at: usize| {
        let key = op.law_limit_key(at)?;
        let found = limit.law.iter().find(|law| law.key == key)?;
        below(found.largest)
    };
    // The op holds the radii between in the order of their places.
    let mut by_place: Vec<usize> = (0..points.len()).collect();
    by_place.sort_by(|a, b| points[*a].at.total_cmp(&points[*b].at));
    let mut between = vec![None; points.len()];
    for (rank, &i) in by_place.iter().enumerate() {
        between[i] = most(rank + 1);
    }
    Some(DragLimits {
        first: most(0),
        second: most(points.len() + 1),
        points: between,
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

/// A limit in mm as a drag stops at it, `None` where there is none.
fn below(largest: Option<f64>) -> Option<f32> {
    largest
        .map(|most| hundredths_below(most as f32))
        .filter(|most| *most > 0.0)
}

/// `size` rounded down to a hundredth of a millimetre, as fields show it:
/// never past what was found to build.
fn hundredths_below(size: f32) -> f32 {
    ((f64::from(size) * 100.0 + 1e-6).floor() / 100.0) as f32
}
