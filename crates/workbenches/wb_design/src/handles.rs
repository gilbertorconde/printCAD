//! Drag handles on the feature under edit: a dot at a pad's or pocket's
//! end, or beside a fillet's or chamfer's first edge, that drags the
//! number it stands for along its axis.

use core_document::{
    Document, FeatureId, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, WorkbenchFeature,
    WorkbenchRuntimeContext,
};
use glam::Vec3;

use crate::feature::{DesignFeature, EdgeSel, ExtrudeDirection, ExtrudeMode};

/// How close to a handle, in pixels, a press takes hold of it.
const REACH_PX: f32 = 10.0;
/// The smallest number a drag sets.
const LEAST: f32 = 0.01;

/// A handle: the number `value` stands at `origin + dir * value * scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handle {
    pub feature: FeatureId,
    /// World space.
    pub origin: Vec3,
    /// World space, of unit length.
    pub dir: Vec3,
    pub scale: f32,
    pub value: f32,
}

impl Handle {
    pub fn position(&self) -> Vec3 {
        self.origin + self.dir * self.value * self.scale
    }

    /// The number with the handle at the point of its axis nearest the
    /// ray, rounded to a tenth.
    pub fn value_at(&self, ray_origin: Vec3, ray_dir: Vec3) -> Option<f32> {
        let d = ray_dir.normalize_or_zero();
        let w = self.origin - ray_origin;
        let b = self.dir.dot(d);
        let denom = 1.0 - b * b;
        if denom < 1e-6 {
            return None;
        }
        let t = (b * d.dot(w) - self.dir.dot(w)) / denom;
        let value = (t / self.scale * 10.0).round() / 10.0;
        Some(value.max(LEAST))
    }
}

/// The handle `feature` offers, looked at from `eye`: none for a number a
/// formula sets or a feature whose shape the handle cannot follow.
pub(crate) fn handle_of(document: &Document, feature: FeatureId, eye: Vec3) -> Option<Handle> {
    let node = document.get_feature_meta(feature)?;
    let body = node.body?;
    let part = DesignFeature::from_json(&node.data).ok()?;
    let placement = document.body_placement(body);
    let world = |p: [f32; 3]| Vec3::from_array(placement.point(p));
    let world_dir = |d: [f32; 3]| Vec3::from_array(placement.direction(d)).normalize_or_zero();
    let free = |key: &str| !node.formulas.contains_key(key);
    let extrude = |sketch: Option<FeatureId>,
                   value: f32,
                   reversed: bool,
                   symmetric: bool,
                   mode: &ExtrudeMode,
                   direction: &ExtrudeDirection,
                   against: bool| {
        if *mode != ExtrudeMode::Dimension || *direction != ExtrudeDirection::Normal {
            return None;
        }
        let (centre, normal) = sketch_centre(document, sketch?)?;
        let sign = if reversed != against { -1.0 } else { 1.0 };
        Some(Handle {
            feature,
            origin: world(centre),
            dir: world_dir(normal) * sign,
            scale: if symmetric { 0.5 } else { 1.0 },
            value,
        })
    };
    let edge_handle = |edges: &EdgeSel, value: f32| {
        let EdgeSel::Edges(picks) = edges else {
            return None;
        };
        let pick = picks.first()?;
        let origin = world(pick.point);
        let along = world_dir(pick.direction);
        let to_eye = eye - origin;
        let dir = (to_eye - along * to_eye.dot(along)).normalize_or_zero();
        (dir != Vec3::ZERO).then_some(Handle {
            feature,
            origin,
            dir,
            scale: 1.0,
            value,
        })
    };
    match &part {
        DesignFeature::Pad {
            sketch,
            length,
            reversed,
            symmetric,
            mode,
            direction,
            ..
        } if free("length") => extrude(
            *sketch, *length, *reversed, *symmetric, mode, direction, false,
        ),
        DesignFeature::Pocket {
            sketch,
            depth,
            reversed,
            symmetric,
            mode,
            direction,
            ..
        } if free("depth") => extrude(
            *sketch, *depth, *reversed, *symmetric, mode, direction, true,
        ),
        DesignFeature::Fillet { radius, edges, .. } if free("radius") => {
            edge_handle(edges, *radius)
        }
        DesignFeature::Chamfer { size, edges, .. } if free("size") => edge_handle(edges, *size),
        _ => None,
    }
}

/// Write `value` into the number the handle stands for.
pub(crate) fn set_value(feature: &mut DesignFeature, value: f32) -> bool {
    match feature {
        DesignFeature::Pad { length, .. } => *length = value,
        DesignFeature::Pocket { depth, .. } => *depth = value,
        DesignFeature::Fillet { radius, .. } => *radius = value,
        DesignFeature::Chamfer { size, .. } => *size = value,
        _ => return false,
    }
    true
}

/// The middle of a sketch's drawing and its plane's normal, in its body's
/// own frame.
pub(crate) fn sketch_centre(
    document: &Document,
    sketch: FeatureId,
) -> Option<([f32; 3], [f32; 3])> {
    let data = document
        .feature_values(sketch)
        .or_else(|| document.get_feature_data(sketch))?;
    let feature = wb_sketch::SketchFeature::from_json(data).ok()?;
    let points: Vec<Vec3> = wb_sketch::render::sketch_polylines(&feature.sketch, &feature.plane)
        .into_iter()
        .flatten()
        .map(Vec3::from_array)
        .collect();
    let (lo, hi) = points.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let centre = if points.is_empty() {
        Vec3::from_array(feature.plane.origin)
    } else {
        (lo + hi) * 0.5
    };
    Some((centre.to_array(), feature.plane.normal))
}

/// Whether the press at `pos` falls on the handle.
pub(crate) fn within_reach(
    ctx: &WorkbenchRuntimeContext,
    handle: &Handle,
    pos: (f32, f32),
) -> bool {
    ctx.world_to_viewport(handle.position().to_array())
        .is_some_and(|(x, y)| (x - pos.0).hypot(y - pos.1) <= REACH_PX)
}

/// The line from the handle's axis origin to it.
pub(crate) fn overlays(ctx: &WorkbenchRuntimeContext, handle: &Handle) -> Vec<ScreenSpaceOverlay> {
    match (
        ctx.world_to_viewport(handle.origin.to_array()),
        ctx.world_to_viewport(handle.position().to_array()),
    ) {
        (Some(a), Some(b)) => vec![
            ScreenSpaceOverlay::new([a.0, a.1], [b.0, b.1], ctx.sketch_palette.selected, 1.5)
                .dashed(4.0, 3.0),
        ],
        _ => Vec::new(),
    }
}

pub(crate) fn marks(
    ctx: &WorkbenchRuntimeContext,
    handle: &Handle,
    held: bool,
) -> Vec<ScreenSpaceMark> {
    let color = if held {
        ctx.sketch_palette.preselect
    } else {
        ctx.sketch_palette.selected
    };
    ctx.world_to_viewport(handle.position().to_array())
        .map(|(x, y)| vec![ScreenSpaceMark::dot([x, y], 6.0, color)])
        .unwrap_or_default()
}

/// The number beside the handle while it is held.
pub(crate) fn labels(ctx: &WorkbenchRuntimeContext, handle: &Handle) -> Vec<ScreenSpaceLabel> {
    ctx.world_to_viewport(handle.position().to_array())
        .map(|(x, y)| {
            vec![
                ScreenSpaceLabel::new(
                    [x, y - 18.0],
                    format!("{:.1} mm", handle.value),
                    ctx.sketch_palette.selected,
                    12.0,
                )
                .pill()
                .mono(),
            ]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_across_the_axis_sets_the_value_where_it_passes() {
        let handle = Handle {
            feature: FeatureId(uuid::Uuid::nil()),
            origin: Vec3::ZERO,
            dir: Vec3::Z,
            scale: 1.0,
            value: 10.0,
        };
        // Looking along -X at height 25.
        let value = handle
            .value_at(Vec3::new(100.0, 0.0, 25.04), Vec3::NEG_X)
            .unwrap();
        assert!((value - 25.0).abs() < 1e-4);
        // Half the length shows on each side of a symmetric one.
        let half = Handle {
            scale: 0.5,
            ..handle
        };
        let value = half
            .value_at(Vec3::new(100.0, 0.0, 6.0), Vec3::NEG_X)
            .unwrap();
        assert!((value - 12.0).abs() < 1e-4);
        // Never below the least, and nothing along the axis itself.
        assert_eq!(
            handle.value_at(Vec3::new(9.0, 0.0, -5.0), Vec3::NEG_X),
            Some(LEAST)
        );
        assert_eq!(
            handle.value_at(Vec3::new(0.0, 0.0, 50.0), Vec3::NEG_Z),
            None
        );
    }
}
