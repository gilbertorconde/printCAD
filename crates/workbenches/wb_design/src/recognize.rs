//! Recognize holes: the round holes of a body's solid made Hole features
//! again. Their faces are deleted from the solid, and each set of alike
//! holes (one opening plane, one size) is drilled again from a sketch of
//! their centres, so their sizes become numbers to change.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use kernel_api::RecognizedHole;
use wb_sketch::sketch::{Circle, GeometryElement, Point, Sketch, SketchPlane, Vec2D};

use crate::feature::{DrillPoint, FacePick, HoleCut, HoleFit};
use crate::{DesignFeature, DesignWorkbench};

/// What recognizing the holes of a body made.
#[derive(Debug, Default)]
pub(crate) struct Recognized {
    /// The holes found and drilled again.
    pub holes: usize,
    /// The features added: the deletion, then each sketch and its hole.
    pub features: Vec<FeatureId>,
    /// Bores that are not holes a Hole feature makes (a counterbore, a
    /// slot), left as they are.
    pub left: usize,
}

/// Find the round holes of `body`'s solid as it is built now, and make
/// them features: an imported solid takes a base shape first.
pub(crate) fn recognize_holes(
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<Recognized, String> {
    let mut features = Vec::new();
    if ctx.document.body_solid_is_imported(body) {
        features.push(
            crate::take_base(ctx, body)
                .ok_or("this body takes its shape from elsewhere and takes no features")?,
        );
    }
    let blob = ctx
        .document
        .imported_brep_blob(body)
        .ok_or("the body has no solid yet; build it first")?
        .to_vec();
    let kernel = ctx
        .kernel
        .ok_or("there is no kernel to read the solid with")?;
    let (holes, left) = kernel.recognize_holes(&blob).map_err(|e| e.to_string())?;
    if holes.is_empty() {
        return Ok(Recognized {
            holes: 0,
            features,
            left,
        });
    }

    let faces: Vec<FacePick> = holes
        .iter()
        .flat_map(|h| h.faces.iter())
        .map(|(point, normal)| FacePick {
            point: point.map(|v| v as f32),
            normal: normal.map(|v| v as f32),
            name: 0,
        })
        .collect();
    let name = DesignWorkbench::next_feature_name(ctx, "DeleteFaces");
    let deleted = ctx
        .document
        .add_feature_in_body(DesignFeature::DeleteFaces { faces }, name, Some(body))
        .map_err(|e| e.to_string())?;
    ctx.document.mark_feature_dirty(deleted);
    features.push(deleted);

    for set in alike(&holes) {
        let (sketch, hole) = drill(ctx, body, &set)?;
        features.push(sketch);
        features.push(hole);
    }
    Ok(Recognized {
        holes: holes.len(),
        features,
        left,
    })
}

/// The holes in sets one Hole feature drills: one opening plane, one
/// direction, one size and end.
fn alike(holes: &[RecognizedHole]) -> Vec<Vec<&RecognizedHole>> {
    let key = |h: &RecognizedHole| {
        let round = |v: f64| (v * 1000.0).round() as i64;
        let offset: f64 = (0..3).map(|i| h.entry[i] * h.direction[i]).sum();
        (
            h.direction.map(round),
            round(offset),
            round(h.diameter),
            round(h.depth),
            h.through,
            h.drill_point_deg.map(round),
        )
    };
    let mut sets: Vec<(_, Vec<&RecognizedHole>)> = Vec::new();
    for hole in holes {
        let k = key(hole);
        match sets.iter_mut().find(|(key, _)| *key == k) {
            Some((_, set)) => set.push(hole),
            None => sets.push((k, vec![hole])),
        }
    }
    sets.into_iter().map(|(_, set)| set).collect()
}

/// A sketch of the set's centres on its opening plane, its normal out of
/// the material, and the Hole feature drilling them.
fn drill(
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    set: &[&RecognizedHole],
) -> Result<(FeatureId, FeatureId), String> {
    let first = set[0];
    let normal = first.direction.map(|v| -v);
    // Any direction square to the normal for the sketch's x: the world
    // axis least along it, made square.
    let least = (0..3)
        .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .unwrap_or(0);
    let mut x = [0.0; 3];
    x[least] = 1.0;
    let along: f64 = (0..3).map(|i| x[i] * normal[i]).sum();
    let x: [f64; 3] = std::array::from_fn(|i| x[i] - normal[i] * along);
    let plane = SketchPlane::from_frame(
        first.entry.map(|v| v as f32),
        normal.map(|v| v as f32),
        x.map(|v| v as f32),
    );
    let name = DesignWorkbench::next_feature_name(ctx, "HoleSketch");
    let mut sketch = Sketch::new(name.clone());
    sketch.plane = plane;
    for hole in set {
        let d: [f64; 3] = std::array::from_fn(|i| hole.entry[i] - first.entry[i]);
        let local = |axis: [f32; 3]| (0..3).map(|i| d[i] * f64::from(axis[i])).sum::<f64>() as f32;
        let centre = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
            local(plane.x_axis),
            local(plane.y_axis),
        ))));
        sketch.add_geometry(GeometryElement::Circle(Circle::new(
            centre,
            (hole.diameter / 2.0) as f32,
        )));
    }
    let sketch_id = ctx
        .document
        .add_feature_in_body(
            wb_sketch::SketchFeature::new(sketch, plane),
            name,
            Some(body),
        )
        .map_err(|e| e.to_string())?;
    ctx.document.set_feature_visible(sketch_id, false);

    let hole = DesignFeature::Hole {
        clearance: None,
        thread_length: Default::default(),
        refine: false,
        boolean_tolerance: None,
        sketch: sketch_id,
        diameter: first.diameter as f32,
        depth: first.depth as f32,
        through_all: first.through,
        cut: HoleCut::None,
        thread: None,
        threaded: false,
        modeled_thread: false,
        thread_depth: 0.0,
        fit: HoleFit::Normal,
        drill_point: match first.drill_point_deg {
            Some(angle) => DrillPoint::Angled {
                angle_deg: angle as f32,
            },
            None => DrillPoint::Flat,
        },
        point_in_depth: false,
        taper_deg: 0.0,
        nut_trap: None,
        reversed: false,
    };
    let name = DesignWorkbench::next_feature_name(ctx, "Hole");
    let hole_id = ctx
        .document
        .add_feature_in_body(hole, name, Some(body))
        .map_err(|e| e.to_string())?;
    ctx.document.mark_feature_dirty(hole_id);
    Ok((sketch_id, hole_id))
}
