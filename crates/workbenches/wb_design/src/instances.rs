//! The occurrences of the pattern under edit, drawn as dots in the view:
//! one where the original stands and one where each copy does, a skipped
//! one crossed out. A click on a dot leaves that occurrence out of the
//! pattern, or brings it back.

use core_document::{
    Document, FeatureId, ScreenSpaceLabel, ScreenSpaceMark, WorkbenchFeature,
    WorkbenchRuntimeContext,
};
use glam::Vec3;

use crate::build::{occurrence_transforms, transform_point};
use crate::feature::{DesignFeature, EdgeSel};

/// How close to a dot, in pixels, a click takes it.
const REACH_PX: f32 = 9.0;

/// One occurrence of a pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Occurrence {
    /// 0 the original, then each copy in turn.
    pub index: u32,
    /// Where it stands, in world space.
    pub at: Vec3,
    pub skipped: bool,
}

/// What the panel and the view call occurrence `index`: the original, or
/// the copy's number.
pub(crate) fn occurrence_name(index: u32) -> String {
    if index == 0 {
        "Original".into()
    } else {
        format!("Copy {index}")
    }
}

/// Every occurrence of `feature`, a pattern or mirror, at where its
/// originals stand moved as the occurrence moves them: none for another
/// feature, or one whose originals stand nowhere a dot can show.
pub(crate) fn occurrences_of(document: &Document, feature: FeatureId) -> Vec<Occurrence> {
    let Some(node) = document.get_feature_meta(feature) else {
        return Vec::new();
    };
    let Some(body) = node.body else {
        return Vec::new();
    };
    let data = document.feature_values(feature).unwrap_or(&node.data);
    let Ok(part) = DesignFeature::from_json(data) else {
        return Vec::new();
    };
    let (Some(originals), Some(skipped)) = (part.originals(), part.skipped()) else {
        return Vec::new();
    };
    let Some(anchor) = anchor_of(document, originals) else {
        return Vec::new();
    };
    let Ok(transforms) = occurrence_transforms(document, &part) else {
        return Vec::new();
    };
    let placement = document.body_placement(body);
    transforms
        .iter()
        .zip(0u32..)
        .map(|(transform, index)| {
            let local = transform_point(transform, anchor).map(|v| v as f32);
            Occurrence {
                index,
                at: Vec3::from_array(placement.point(local)),
                skipped: skipped.contains(&index),
            }
        })
        .collect()
}

/// Where the originals stand together, in their body's own frame: the
/// middle of the places each stands.
fn anchor_of(document: &Document, originals: &[FeatureId]) -> Option<[f64; 3]> {
    let places: Vec<[f32; 3]> = originals
        .iter()
        .filter_map(|id| place_of(document, *id))
        .collect();
    if places.is_empty() {
        return None;
    }
    let n = places.len() as f64;
    let sum = places.iter().fold([0.0f64; 3], |acc, p| {
        [
            acc[0] + f64::from(p[0]),
            acc[1] + f64::from(p[1]),
            acc[2] + f64::from(p[2]),
        ]
    });
    Some(sum.map(|v| v / n))
}

/// Where one original stands: the middle of its sketch's drawing, a
/// primitive's origin, a dress-up's first edge.
fn place_of(document: &Document, original: FeatureId) -> Option<[f32; 3]> {
    let data = document
        .feature_values(original)
        .or_else(|| document.get_feature_data(original))?;
    let part = DesignFeature::from_json(data).ok()?;
    if let Some(sketch) = part.sketch() {
        return crate::handles::sketch_centre(document, sketch).map(|(centre, _)| centre);
    }
    match &part {
        DesignFeature::Primitive { placement, .. } => Some(placement.origin.map(|v| v as f32)),
        DesignFeature::Fillet { edges, .. } | DesignFeature::Chamfer { edges, .. } => match edges {
            EdgeSel::Edges(picks) => picks.first().map(|pick| pick.point),
            _ => None,
        },
        _ => None,
    }
}

/// The occurrence whose dot `pos` (viewport pixels) falls on, the nearest
/// when dots crowd.
pub(crate) fn hit(
    ctx: &WorkbenchRuntimeContext,
    occurrences: &[Occurrence],
    pos: (f32, f32),
) -> Option<u32> {
    occurrences
        .iter()
        .filter_map(|o| {
            let (x, y) = ctx.world_to_viewport(o.at.to_array())?;
            let distance = (x - pos.0).hypot(y - pos.1);
            (distance <= REACH_PX).then_some((distance, o.index))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, index)| index)
}

/// A dot per occurrence: kept ones in the selection colour, skipped ones
/// greyed and crossed, the one under the pointer in the hover colour.
pub(crate) fn marks(
    ctx: &WorkbenchRuntimeContext,
    occurrences: &[Occurrence],
    hovered: Option<u32>,
) -> Vec<ScreenSpaceMark> {
    let palette = &ctx.sketch_palette;
    let mut marks = Vec::new();
    for o in occurrences {
        let Some((x, y)) = ctx.world_to_viewport(o.at.to_array()) else {
            continue;
        };
        let color = if hovered == Some(o.index) {
            palette.preselect
        } else if o.skipped {
            palette.inactive
        } else {
            palette.selected
        };
        if o.skipped {
            marks.push(ScreenSpaceMark::dot([x, y], 4.0, color));
            marks.push(ScreenSpaceMark::crosshair([x, y], 16.0, color));
        } else {
            marks.push(ScreenSpaceMark::dot([x, y], 6.0, color));
        }
    }
    marks
}

/// What a click on the dot under the pointer does.
pub(crate) fn labels(
    ctx: &WorkbenchRuntimeContext,
    occurrences: &[Occurrence],
    hovered: Option<u32>,
) -> Vec<ScreenSpaceLabel> {
    let Some(o) = occurrences.iter().find(|o| Some(o.index) == hovered) else {
        return Vec::new();
    };
    let Some((x, y)) = ctx.world_to_viewport(o.at.to_array()) else {
        return Vec::new();
    };
    let action = if o.skipped { "bring back" } else { "skip" };
    vec![
        ScreenSpaceLabel::new(
            [x, y - 18.0],
            format!("{}: click to {action}", occurrence_name(o.index)),
            ctx.sketch_palette.preselect,
            12.0,
        )
        .pill(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::PatternAxis;

    /// A box at (5, 5, 0) repeated along X 10 apart, its middle copy
    /// skipped, in a body moved 100 along Y.
    fn patterned() -> (Document, FeatureId) {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        doc.set_body_placement(
            body,
            core_document::BodyPlacement {
                translation: [0.0, 100.0, 0.0],
                ..Default::default()
            },
        );
        let block = doc
            .add_feature_in_body(
                DesignFeature::Primitive {
                    keep_inside: false,
                    boolean_tolerance: None,
                    kind: kernel_api::PrimitiveKind::Box {
                        length: 2.0,
                        width: 2.0,
                        height: 2.0,
                    },
                    placement: kernel_api::Placement {
                        origin: [5.0, 5.0, 0.0],
                        ..Default::default()
                    },
                    subtractive: false,
                    refine: false,
                    attached: None,
                },
                "Box".into(),
                Some(body),
            )
            .unwrap();
        let pattern = doc
            .add_feature_in_body(
                DesignFeature::LinearPattern {
                    boolean_tolerance: None,
                    skipped: vec![1],
                    refine: false,
                    originals: vec![block],
                    axis: PatternAxis::X,
                    length: 20.0,
                    occurrences: 3,
                    spacing_mode: false,
                    spacings: Vec::new(),
                    reversed: false,
                },
                "Pattern".into(),
                Some(body),
            )
            .unwrap();
        (doc, pattern)
    }

    #[test]
    fn each_occurrence_stands_where_the_pattern_puts_its_original() {
        let (doc, pattern) = patterned();
        let found = occurrences_of(&doc, pattern);
        let at: Vec<[f32; 3]> = found.iter().map(|o| o.at.to_array()).collect();
        assert_eq!(
            at,
            vec![[5.0, 105.0, 0.0], [15.0, 105.0, 0.0], [25.0, 105.0, 0.0]]
        );
        let skipped: Vec<bool> = found.iter().map(|o| o.skipped).collect();
        assert_eq!(skipped, vec![false, true, false]);
    }

    #[test]
    fn a_click_takes_the_dot_it_falls_on() {
        let (mut doc, pattern) = patterned();
        let found = occurrences_of(&doc, pattern);
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 800));
        ctx.view_proj = Some(
            (glam::Mat4::from_scale(Vec3::new(0.005, 0.005, 0.001))
                * glam::Mat4::from_translation(Vec3::new(-15.0, -105.0, 0.0)))
            .to_cols_array_2d(),
        );
        for o in &found {
            let (x, y) = ctx.world_to_viewport(o.at.to_array()).unwrap();
            assert_eq!(hit(&ctx, &found, (x + 3.0, y - 2.0)), Some(o.index));
        }
        let (x, y) = ctx.world_to_viewport(found[0].at.to_array()).unwrap();
        assert_eq!(hit(&ctx, &found, (x, y + 40.0)), None, "nowhere near a dot");
    }
}
