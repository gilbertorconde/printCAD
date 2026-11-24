//! Notes on a sketch: where one stands as the sketch is now and what a
//! click with the note tool lands on. A note on an element keeps to the
//! element's point nearest where it was put, one on a constraint to the
//! constraint's anchor (a dimension's label), one on a point of the plane
//! to that point; the label moves as the point does. All of it is in the
//! sketch's plane, which carries it wherever the sketch is placed.

use core_document::{NotePlane, SketchNotePick, SketchNoteSpot, ViewportPick};
use glam::DVec2;
use uuid::Uuid;

use crate::SketchFeature;
use crate::boundary::{narrow, narrow3};
use crate::glyphs::{dim_anchor, element_anchor};
use crate::shape::Shape;
use crate::sketch::{GeometryElement, Sketch, Vec2D, constraint_refs};

/// Where a note on `feature` stands: `at` and `label` in its plane, taken
/// to `item` as the sketch is now; lost, as stored, when `item` is gone.
pub(crate) fn spot(
    feature: &SketchFeature,
    item: Option<Uuid>,
    at: [f32; 3],
    label: [f32; 3],
) -> SketchNoteSpot {
    let plane = note_plane(feature);
    let stored = SketchNoteSpot {
        at: [at[0], at[1], 0.0],
        label: [label[0], label[1], 0.0],
        lost: false,
        plane,
        size: sketch_size(&feature.sketch),
    };
    let Some(item) = item else {
        return stored;
    };
    let from = DVec2::new(f64::from(at[0]), f64::from(at[1]));
    match item_point(&feature.sketch, item, from) {
        Some(point) => {
            let shift = point - from;
            SketchNoteSpot {
                at: [narrow(point.x), narrow(point.y), 0.0],
                label: [
                    narrow(f64::from(label[0]) + shift.x),
                    narrow(f64::from(label[1]) + shift.y),
                    0.0,
                ],
                ..stored
            }
        }
        None => SketchNoteSpot {
            lost: true,
            ..stored
        },
    }
}

/// The point of element or constraint `item` a note near `from` keeps to:
/// an element's nearest point, a constraint's anchor. `None` when the
/// sketch has no such item, or it cannot be placed.
pub(crate) fn item_point(sketch: &Sketch, item: Uuid, from: DVec2) -> Option<DVec2> {
    if let Some(geom) = sketch.get_geometry(item) {
        let shape = Shape::of(&|id| sketch.point_position(id), geom)?;
        return Some(shape.nearest(from));
    }
    let constraint = sketch.constraints.iter().find(|c| c.id == item)?;
    constraint_anchor(sketch, constraint).map(Vec2D::to_glam)
}

/// Where a constraint stands in the sketch: a dimension at its label, any
/// other at the middle of what it holds.
fn constraint_anchor(sketch: &Sketch, constraint: &crate::sketch::Constraint) -> Option<Vec2D> {
    if let Some(anchor) = dim_anchor(sketch, &constraint.kind) {
        let offset = constraint.label_offset.unwrap_or(Vec2D::new(0.0, 0.0));
        return Some(Vec2D::new(anchor.x + offset.x, anchor.y + offset.y));
    }
    let anchors: Vec<Vec2D> = constraint_refs(&constraint.kind)
        .into_iter()
        .filter_map(|id| element_anchor(sketch, id))
        .collect();
    let n = anchors.len() as f64;
    (n > 0.0).then(|| {
        Vec2D::new(
            anchors.iter().map(|p| p.x).sum::<f64>() / n,
            anchors.iter().map(|p| p.y).sum::<f64>() / n,
        )
    })
}

/// How far across the sketch's points span.
fn sketch_size(sketch: &Sketch) -> f32 {
    let (lo, hi) = sketch
        .geometry
        .iter()
        .flat_map(Sketch::curve_point_ids)
        .chain(sketch.geometry.iter().map(GeometryElement::id))
        .filter_map(|id| sketch.point_position(id))
        .map(Vec2D::to_glam)
        .filter(|p| p.is_finite())
        .fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(p), hi.max(p)),
        );
    if lo.x > hi.x {
        return 0.0;
    }
    narrow((hi - lo).length())
}

/// The sketch's plane in its body's frame.
fn note_plane(feature: &SketchFeature) -> NotePlane {
    NotePlane {
        origin: narrow3(feature.plane.origin),
        x_axis: narrow3(feature.plane.x_axis),
        y_axis: narrow3(feature.plane.y_axis),
    }
}

/// What a click with the note tool lands on in sketch `id`: `hovered` (the
/// element or constraint the sketcher shows under the cursor while it is
/// edited) or else the element nearest the cursor within `tolerance_px`,
/// at its point nearest the cursor; else, when `plane_points` allows, the
/// plane's point under the cursor.
pub(crate) fn pick(
    feature: &SketchFeature,
    pick: &ViewportPick,
    tolerance_px: f32,
    plane_points: bool,
    hovered: Option<Uuid>,
) -> Option<SketchNotePick> {
    let (_, pos, px_per_unit) = crate::closed_parts::cursor_in_sketch(feature, pick)?;
    let sketch = &feature.sketch;
    let cursor = pos.to_glam();
    let reach = f64::from(tolerance_px / px_per_unit.max(f32::MIN_POSITIVE));
    let hit = hovered
        .and_then(|item| Some((item, item_point(sketch, item, cursor)?)))
        .or_else(|| {
            sketch
                .geometry
                .iter()
                .filter_map(|g| {
                    let shape = Shape::of(&|id| sketch.point_position(id), g)?;
                    let point = shape.nearest(cursor);
                    let d = (point - cursor).length();
                    (d <= reach).then_some((g.id(), point, d))
                })
                // A point over the curve it ends wins at the same distance.
                .min_by(|a, b| {
                    let is_point =
                        |id| matches!(sketch.get_geometry(id), Some(GeometryElement::Point(_)));
                    a.2.total_cmp(&b.2)
                        .then_with(|| is_point(b.0).cmp(&is_point(a.0)))
                })
                .map(|(id, point, _)| (id, point))
        });
    let (item, point) = match hit {
        Some((item, point)) => (Some(item), point),
        None if plane_points => (None, cursor),
        None => return None,
    };
    // The label up and to the right on screen, whichever way the plane's
    // axes face the view.
    let reach = f64::from(LABEL_PX / px_per_unit.max(f32::MIN_POSITIVE));
    let up_right = screen_up_right(feature, pick, point).unwrap_or(DVec2::ONE.normalize());
    let label = point + up_right * reach;
    Some(SketchNotePick {
        item,
        at: [narrow(point.x), narrow(point.y), 0.0],
        label: [narrow(label.x), narrow(label.y), 0.0],
    })
}

/// How far a new note's label stands from its point on screen, in pixels.
const LABEL_PX: f32 = 48.0;

/// The direction in the sketch's plane that runs up and to the right on
/// screen from `at`.
fn screen_up_right(feature: &SketchFeature, pick: &ViewportPick, at: DVec2) -> Option<DVec2> {
    use core_document::runtime::world_to_viewport;
    let plane = feature.plane;
    let world = |p: DVec2| {
        let w = glam::DVec3::from_array(plane.origin)
            + glam::DVec3::from_array(plane.x_axis) * p.x
            + glam::DVec3::from_array(plane.y_axis) * p.y;
        world_to_viewport(pick.view_proj, pick.viewport, narrow3(w.to_array()))
    };
    let o = world(at)?;
    let x = world(at + DVec2::X)?;
    let y = world(at + DVec2::Y)?;
    // Screen pixels per plane unit along each axis; screen y runs down.
    let m = glam::DMat2::from_cols(
        DVec2::new(f64::from(x.0 - o.0), f64::from(x.1 - o.1)),
        DVec2::new(f64::from(y.0 - o.0), f64::from(y.1 - o.1)),
    );
    if m.determinant().abs() < 1e-9 {
        return None;
    }
    let dir = m.inverse() * DVec2::new(1.0, -1.0);
    (dir.length() > 1e-12).then(|| dir.normalize())
}
