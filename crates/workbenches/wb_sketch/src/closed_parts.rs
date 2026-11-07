//! A closed sketch's parts as the view shows and picks them with no task
//! open: every region its curves close, shaded on its own (the regions a
//! plain profile takes a little stronger than the rest), and its drawn
//! curves.

use std::collections::HashMap;
use std::sync::Arc;

use core_document::{FeatureId, FeaturePart, PartPick, PassiveRegion, ViewportPick};
use kernel_api::ProfileWire;

use crate::SketchFeature;
use crate::boundary::{self, narrow, narrow3};
use crate::regions::Region;
use crate::sketch::{GeometryElement, Vec2D};
use crate::{profile, snap};

/// How strongly a region a plain profile leaves out (a hole, a region
/// between crossing curves) is shaded, as a share of the regions it takes.
const LEFT_OUT_SHARE: f32 = 0.5;

/// Each closed sketch's regions with their outlines, by the data revision
/// they were found at.
#[derive(Default)]
pub(crate) struct Kept(std::cell::RefCell<HashMap<FeatureId, (u64, Arc<Shaded>)>>);

/// A sketch's regions and each one's own outline.
#[derive(Default)]
struct Shaded {
    regions: Vec<Region>,
    wires: Vec<Vec<ProfileWire>>,
}

impl Kept {
    /// The regions of sketch `id`, drawn as `feature` at `revision`.
    fn of(&self, id: FeatureId, revision: u64, feature: &SketchFeature) -> Arc<Shaded> {
        let mut kept = self.0.borrow_mut();
        if let Some((at, shaded)) = kept.get(&id)
            && *at == revision
        {
            return Arc::clone(shaded);
        }
        let (regions, wires) = profile::region_wires(&feature.sketch)
            .unwrap_or_default()
            .into_iter()
            .unzip();
        let shaded = Arc::new(Shaded { regions, wires });
        kept.insert(id, (revision, Arc::clone(&shaded)));
        shaded
    }

    /// Every region of sketch `id` as the view shades it, each lifted
    /// `lift` off the plane, the regions a plain profile takes at
    /// `opacity` and the others lighter.
    pub(crate) fn passive_regions(
        &self,
        id: FeatureId,
        revision: u64,
        feature: &SketchFeature,
        lift: f64,
        opacity: f32,
    ) -> Vec<PassiveRegion> {
        let mut plane = profile::plane_of(&feature.plane);
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * lift;
        }
        let shaded = self.of(id, revision, feature);
        shaded
            .regions
            .iter()
            .zip(&shaded.wires)
            .map(|(region, wires)| PassiveRegion {
                profile: kernel_api::Profile {
                    plane,
                    wires: wires.clone(),
                },
                opacity: if region.default {
                    opacity
                } else {
                    opacity * LEFT_OUT_SHARE
                },
                part: Some(FeaturePart::Region(region.name.clone())),
            })
            .collect()
    }

    /// The part of sketch `id` under the cursor: a drawn curve within
    /// `tolerance_px`, else, with `regions` on, the region the cursor is
    /// in.
    pub(crate) fn pick(
        &self,
        id: FeatureId,
        revision: u64,
        feature: &SketchFeature,
        pick: &ViewportPick,
        tolerance_px: f32,
        regions: bool,
    ) -> Option<PartPick> {
        let (world, pos, px_per_unit) = cursor_in_sketch(feature, pick)?;
        let sketch = &feature.sketch;
        let curve = sketch
            .geometry
            .iter()
            .filter(|g| !matches!(g, GeometryElement::Point(_)) && !sketch.is_construction(g.id()))
            .filter_map(|g| Some((g.id(), snap::distance_to_element(sketch, g, pos)?)))
            .filter(|(_, d)| narrow(*d) * px_per_unit <= tolerance_px)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let part = match curve {
            Some((curve, _)) => FeaturePart::Edge(curve.to_string()),
            None if regions => {
                let shaded = self.of(id, revision, feature);
                let region = crate::regions::region_at(&shaded.regions, [pos.x, pos.y])?;
                FeaturePart::Region(region.name.clone())
            }
            None => return None,
        };
        Some(PartPick {
            part,
            point: narrow3(world),
        })
    }
}

/// Where the cursor's ray meets the sketch's plane: in the frame the view
/// is given in, in the sketch's own coordinates, and how many pixels a
/// sketch unit spans there.
pub(crate) fn cursor_in_sketch(
    feature: &SketchFeature,
    pick: &ViewportPick,
) -> Option<([f64; 3], Vec2D, f32)> {
    use core_document::runtime::world_to_viewport;
    let plane = feature.plane;
    let world = boundary::cursor_on_plane(
        pick.view_proj,
        pick.viewport,
        pick.cursor,
        plane.origin,
        plane.normal,
    )?;
    let origin = glam::DVec3::from_array(plane.origin);
    let rel = glam::DVec3::from_array(world) - origin;
    let pos = Vec2D::new(
        rel.dot(glam::DVec3::from_array(plane.x_axis)),
        rel.dot(glam::DVec3::from_array(plane.y_axis)),
    );
    let o_px = world_to_viewport(pick.view_proj, pick.viewport, narrow3(plane.origin))?;
    let x_px = world_to_viewport(
        pick.view_proj,
        pick.viewport,
        narrow3((origin + glam::DVec3::from_array(plane.x_axis)).to_array()),
    )?;
    let px_per_unit = ((x_px.0 - o_px.0).powi(2) + (x_px.1 - o_px.1).powi(2)).sqrt();
    (px_per_unit >= 1e-6).then_some((world, pos, px_per_unit))
}
