//! Bodies shown where an exploded view or a motion study puts them, and
//! the frames of a drive swept on a copy of the document.

use crate::{
    AssemblyWorkbench, JointFeature, JointKind, Task, exploded, place_bodies, restore_placements,
    solve,
};
use core_document::{BodyId, BodyPlacement, FeatureId, WorkbenchFeature, WorkbenchRuntimeContext};

impl AssemblyWorkbench {
    /// The open exploded view's bodies where its steps have them at the
    /// progress set.
    pub(crate) fn show_steps(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(Task::Explode {
            placements, steps, ..
        }) = &self.tab.task
        else {
            return;
        };
        let Some(view) = steps
            .view
            .and_then(|id| exploded::view_of(ctx.document, id))
        else {
            return;
        };
        for (body, placement) in view.placed_at(ctx.document, placements, steps.at) {
            ctx.document.set_body_placement(body, placement);
        }
    }

    /// A motion study open: the bodies back where they were.
    pub(crate) fn put_back_motion(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(Task::Motion(studying)) = &self.tab.task {
            restore_placements(ctx, &studying.placements);
            self.tab.task = None;
        }
    }

    /// The motion study's bodies where frame `frame` has them.
    pub(crate) fn show_frame(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(Task::Motion(studying)) = &self.tab.task else {
            return;
        };
        let Some(frames) = &studying.frames else {
            return;
        };
        if let Some((_, placements)) = frames.get(studying.frame) {
            restore_placements(ctx, placements);
        }
    }

    /// An exploded view open: the bodies back where they were.
    pub(crate) fn put_back_explosion(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(Task::Explode { placements, .. }) = &self.tab.task {
            restore_placements(ctx, placements);
            self.tab.task = None;
        }
    }
}

/// Every visible body moved out from the middle of the assembly by
/// `spread` times its own distance from it, from where `placements` put
/// them.
pub(crate) fn explode(
    ctx: &mut WorkbenchRuntimeContext,
    placements: &[(BodyId, BodyPlacement)],
    spread: f32,
) {
    let centres: Vec<(BodyId, BodyPlacement, glam::Vec3)> = placements
        .iter()
        .filter(|(body, _)| ctx.document.imported_body_effective_visible(*body))
        .filter_map(|(body, placement)| {
            let (mesh, bounds) = ctx.document.local_geometry(*body)?;
            let (lo, hi) = bounds.or_else(|| mesh.bounds())?;
            let middle = (glam::Vec3::from_array(lo) + glam::Vec3::from_array(hi)) * 0.5;
            let centre = glam::Vec3::from_array(placement.point(middle.to_array()));
            Some((*body, *placement, centre))
        })
        .collect();
    if centres.is_empty() {
        return;
    }
    let middle = centres.iter().map(|(_, _, c)| *c).sum::<glam::Vec3>() / centres.len() as f32;
    for (body, placement, centre) in centres {
        let out = (centre - middle) * spread;
        ctx.document.set_body_placement(
            body,
            BodyPlacement::new(placement.quat(), placement.offset() + out),
        );
    }
}

/// A hinge's or a slider's drive swept from `low` to `high` and back in
/// `count` frames, on a copy of the document: each frame, every body the
/// joints moved and where. Nothing changes in `document`.
pub fn sweep_frames(
    document: &core_document::Document,
    joint: FeatureId,
    low: f32,
    high: f32,
    count: usize,
) -> Vec<Vec<(BodyId, BodyPlacement)>> {
    let values: Vec<f32> = (0..count)
        .map(|i| {
            let t = i as f64 / count as f64;
            (f64::from(low)
                + f64::from(high - low) * (0.5 - 0.5 * (std::f64::consts::TAU * t).cos()))
                as f32
        })
        .collect();
    let frames = sweep_values(document, joint, &values);
    let start: Vec<(BodyId, BodyPlacement)> = document
        .bodies()
        .iter()
        .map(|b| (b.id, b.placement))
        .collect();
    let moved = frames.iter().any(|f| *f != start);
    if moved { frames } else { Vec::new() }
}

/// A hinge's or a slider's drive held at each of `values` in turn, on a
/// copy of the document: every body's placement at each. Empty for a
/// joint that is not a hinge or a slider.
pub fn sweep_values(
    document: &core_document::Document,
    joint: FeatureId,
    values: &[f32],
) -> Vec<Vec<(BodyId, BodyPlacement)>> {
    let Some(mut feature) = document
        .get_feature_data(joint)
        .and_then(|d| JointFeature::from_json(d).ok())
    else {
        return Vec::new();
    };
    let mut copy = document.clone();
    let mut frames = Vec::with_capacity(values.len());
    for value in values {
        match &mut feature.kind {
            JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } => {
                drive.to = Some(*value);
            }
            _ => return Vec::new(),
        }
        if copy.update_feature_data(joint, feature.to_json()).is_err() {
            return Vec::new();
        }
        if let Ok(moves) = solve(&copy) {
            place_bodies(&mut copy, &moves);
        }
        frames.push(copy.bodies().iter().map(|b| (b.id, b.placement)).collect());
    }
    frames
}
