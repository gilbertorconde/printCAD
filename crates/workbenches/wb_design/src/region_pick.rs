//! Choosing a sketch's regions in the view while a pad, pocket, revolution
//! or groove task is open: a click on the sketch's plane takes the region
//! under it alone, Ctrl adds it or takes it away. The regions left out
//! show shaded on the plane, so there is something to click.

use std::cell::RefCell;

use core_document::{
    BodyId, Document, FeatureId, OverlayMesh, WorkbenchFeature, WorkbenchRuntimeContext,
};
use glam::Vec3;
use wb_sketch::SketchFeature;
use wb_sketch::profile;

use crate::feature::DesignFeature;

/// How see-through the regions left out are drawn.
const LEFT_OUT_OPACITY: f32 = 0.25;
/// How far the shading stands off the sketch plane, in mm, so the face of
/// the solid it lies on does not fight it.
const LIFT_MM: f64 = 0.02;

/// Feature `id`, its body and its sketch, when it takes regions of a
/// sketch that closes more than one.
fn choosing(
    document: &Document,
    id: FeatureId,
) -> Option<(DesignFeature, BodyId, SketchFeature, FeatureId)> {
    let node = document.get_feature_meta(id)?;
    let body = node.body?;
    let feature = DesignFeature::from_json(&node.data).ok()?;
    let sketch = match &feature {
        DesignFeature::Pad {
            sketch: Some(sketch),
            profile_face: None,
            ..
        }
        | DesignFeature::Pocket {
            sketch: Some(sketch),
            profile_face: None,
            ..
        } => *sketch,
        DesignFeature::Revolution { sketch, .. } | DesignFeature::Groove { sketch, .. } => *sketch,
        _ => return None,
    };
    if crate::build::sketch_regions(document, sketch).len() < 2 {
        return None;
    }
    let drawn = crate::build::load_sketch(document, sketch).ok()?;
    Some((feature, body, drawn, sketch))
}

/// Where the ray from `viewport_pos` meets the sketch's plane, in the
/// plane's own coordinates.
fn on_plane(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    sketch: &SketchFeature,
    viewport_pos: (f32, f32),
) -> Option<[f64; 2]> {
    let (origin, dir) = ctx.viewport_to_ray(viewport_pos)?;
    // The sketch keeps its plane in its body's frame.
    let into = ctx.document.body_placement(body).inverse();
    let (origin, dir) = (
        Vec3::from_array(into.point(origin)),
        Vec3::from_array(into.direction(dir)),
    );
    let plane = &sketch.plane;
    let (o, n) = (
        Vec3::from_array(plane.origin),
        Vec3::from_array(plane.normal),
    );
    let facing = dir.dot(n);
    if facing.abs() < 1e-6 {
        return None;
    }
    let t = (o - origin).dot(n) / facing;
    if t < 0.0 {
        return None;
    }
    let d = origin + dir * t - o;
    Some([
        f64::from(d.dot(Vec3::from_array(plane.x_axis))),
        f64::from(d.dot(Vec3::from_array(plane.y_axis))),
    ])
}

/// Even-odd: whether `p` lies inside the polygons taken together.
fn inside(polygons: &[Vec<[f64; 2]>], p: [f64; 2]) -> bool {
    let mut odd = false;
    for polygon in polygons {
        let n = polygon.len();
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1])
                && a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) > p[0]
            {
                odd = !odd;
            }
        }
    }
    odd
}

/// The regions taken once region `name` is clicked: it alone, or with
/// `adding` it joins those taken or leaves them. Empty means the regions
/// the whole profile takes; `None` when the click would take nothing.
pub(crate) fn clicked(
    regions: &[wb_sketch::regions::Region],
    chosen: &[String],
    name: &str,
    adding: bool,
) -> Option<Vec<String>> {
    let taken: Vec<bool> = regions
        .iter()
        .map(|r| {
            if chosen.is_empty() {
                r.default
            } else {
                chosen.contains(&r.name)
            }
        })
        .collect();
    let now: Vec<bool> = regions
        .iter()
        .zip(&taken)
        .map(|(r, &on)| match (r.name == name, adding) {
            (true, true) => !on,
            (true, false) => true,
            (false, true) => on,
            (false, false) => false,
        })
        .collect();
    if !now.contains(&true) {
        return None;
    }
    let defaults: Vec<bool> = regions.iter().map(|r| r.default).collect();
    Some(if now == defaults {
        Vec::new()
    } else {
        regions
            .iter()
            .zip(&now)
            .filter(|(_, on)| **on)
            .map(|(r, _)| r.name.clone())
            .collect()
    })
}

/// A click on the view while feature `id`'s task is open: the region
/// under it chosen. False when the click is not on one of its sketch's
/// regions, so it goes on to pick as any click does.
pub(crate) fn click(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    viewport_pos: (f32, f32),
) -> bool {
    let Some((mut feature, body, sketch, sketch_id)) = choosing(ctx.document, id) else {
        return false;
    };
    let Some(at) = on_plane(ctx, body, &sketch, viewport_pos) else {
        return false;
    };
    let regions = crate::build::sketch_regions(ctx.document, sketch_id);
    let Some(hit) = regions.iter().find(|r| inside(&r.outline, at)) else {
        return false;
    };
    let Some(chosen) = (match &mut feature {
        DesignFeature::Pad {
            regions: chosen, ..
        }
        | DesignFeature::Pocket {
            regions: chosen, ..
        }
        | DesignFeature::Revolution {
            regions: chosen, ..
        }
        | DesignFeature::Groove {
            regions: chosen, ..
        } => Some(chosen),
        _ => None,
    }) else {
        return false;
    };
    let Some(now) = clicked(&regions, chosen, &hit.name, ctx.ctrl_down) else {
        return true;
    };
    if *chosen != now {
        *chosen = now;
        if ctx
            .document
            .update_feature_data(id, feature.to_json())
            .is_ok()
        {
            ctx.document.mark_feature_dirty(id);
        }
    }
    true
}

type Shading = Option<(FeatureId, u64, Vec<String>, kernel_api::TriMesh)>;

thread_local! {
    static SHADED: RefCell<Shading> = const { RefCell::new(None) };
}

/// The regions feature `id` leaves out of its sketch, shaded on the
/// sketch's plane where its body sits.
pub(crate) fn overlay(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Option<OverlayMesh> {
    let (feature, body, sketch, sketch_id) = choosing(ctx.document, id)?;
    let kernel = ctx.kernel?;
    let regions = crate::build::sketch_regions(ctx.document, sketch_id);
    let chosen = feature.regions();
    let left_out: Vec<String> = regions
        .iter()
        .filter(|r| {
            if chosen.is_empty() {
                !r.default
            } else {
                !chosen.contains(&r.name)
            }
        })
        .map(|r| r.name.clone())
        .collect();
    if left_out.is_empty() {
        return None;
    }
    let seq = ctx.document.mutation_seq();
    let mesh = SHADED.with(|shaded| {
        if let Some((at, when, names, mesh)) = &*shaded.borrow()
            && (*at, *when) == (id, seq)
            && *names == left_out
        {
            return Some(mesh.clone());
        }
        let wires = profile::extract_regions(&sketch.sketch, &left_out).ok()?;
        let mut plane = profile::plane_of(&sketch.plane);
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * LIFT_MM;
        }
        let mesh = kernel
            .profile_mesh(&kernel_api::Profile { plane, wires })
            .ok()?;
        *shaded.borrow_mut() = Some((id, seq, left_out, mesh.clone()));
        Some(mesh)
    })?;
    if mesh.indices.is_empty() {
        return None;
    }
    Some(OverlayMesh {
        mesh: ctx.document.body_placement(body).mesh(&mesh),
        color: ctx.sketch_palette.geometry,
        wireframe: false,
        opacity: LEFT_OUT_OPACITY,
        on_top: false,
    })
}

#[cfg(test)]
mod tests {
    use super::clicked;
    use wb_sketch::regions::Region;

    fn region(name: &str, default: bool) -> Region {
        Region {
            name: name.into(),
            area: 1.0,
            inside: [0.0, 0.0],
            outline: Vec::new(),
            default,
        }
    }

    #[test]
    fn a_click_takes_one_region_and_ctrl_adds_or_takes_away() {
        let regions = [region("a", true), region("b", true), region("c", false)];
        assert_eq!(clicked(&regions, &[], "b", false), Some(vec!["b".into()]));
        // Ctrl on a region the whole profile takes leaves it out.
        assert_eq!(
            clicked(&regions, &[], "a", true),
            Some(vec!["b".to_string()])
        );
        // Adding back what the whole profile takes is the whole profile.
        assert_eq!(clicked(&regions, &["b".into()], "a", true), Some(vec![]));
        assert_eq!(
            clicked(&regions, &["b".into()], "c", true),
            Some(vec!["b".to_string(), "c".to_string()])
        );
        assert_eq!(
            clicked(&regions, &["b".into()], "b", true),
            None,
            "never nothing"
        );
    }
}
