//! Sketch tool state machine.
//!
//! Tools accumulate *intent* (snapped targets) and only materialize
//! geometry when a shape completes, so cancelling mid-shape never leaves
//! orphan points behind. Endpoint snapping reuses existing point ids, which
//! is what makes consecutive segments and closed profiles share vertices.
//!
//! Handlers live in submodules: `draw` (new geometry), `modify`
//! (fillet/chamfer/trim/extend/split/offset), `transform`
//! (translate/rotate/scale/mirror over the current selection).

mod draw;
mod modify;
mod transform;

use std::collections::HashSet;

use uuid::Uuid;

pub use draw::{arc_slot_shape, polygon_vertices, slot_corners};
pub use modify::trim_preview;
pub use transform::Similarity;

use crate::sketch::{ConstraintKind, GeometryElement, Point, Sketch, Vec2D};
use crate::snap::{self, SnapTarget};

/// In-progress state of the active drawing tool.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ToolState {
    #[default]
    Idle,
    /// Line tool: waiting for the segment's end point. `chain` is true once
    /// at least one segment was committed (right-click/Escape ends chains).
    LineFrom { from: SnapTarget, chain: bool },
    /// Rectangle tool: first corner picked.
    RectFrom { corner: SnapTarget },
    /// Centered-rectangle tool: center picked.
    RectCenterAt { center: Vec2D },
    /// Circle tool: center picked, waiting for a rim point.
    CircleFrom { center: SnapTarget },
    /// 3-point circle: first rim point picked.
    Circle3One { a: Vec2D },
    /// 3-point circle: two rim points picked.
    Circle3Two { a: Vec2D, b: Vec2D },
    /// Arc tool: center picked, waiting for the start point.
    ArcCenter { center: SnapTarget },
    /// Arc tool: center + start picked, waiting for the end point.
    ArcStart {
        center: SnapTarget,
        start: SnapTarget,
    },
    /// 3-point arc: start endpoint picked.
    Arc3Start { start: SnapTarget },
    /// 3-point arc: both endpoints picked, waiting for a rim point.
    Arc3End { start: SnapTarget, end: SnapTarget },
    /// Polygon tool: center picked, waiting for a vertex.
    PolygonCenter { center: SnapTarget },
    /// Slot tool: first centerline endpoint picked.
    SlotFrom { from: SnapTarget },
    /// Arc-slot tool: arc center picked.
    ArcSlotCenter { center: SnapTarget },
    /// Arc-slot tool: center + centerline start picked.
    ArcSlotStart {
        center: SnapTarget,
        start: SnapTarget,
    },
    /// Ellipse tool: center picked.
    EllipseCenter { center: SnapTarget },
    /// Ellipse tool: center + major vertex picked, waiting for minor extent.
    EllipseMajor {
        center: SnapTarget,
        major_pos: Vec2D,
    },
    /// B-spline tool: control points accumulated so far.
    BSplineDraw { points: Vec<SnapTarget> },
    /// Translate tool: base point picked.
    TranslateFrom { base: Vec2D },
    /// Rotate tool: pivot picked.
    RotateCenter { center: Vec2D },
    /// Rotate tool: pivot + angle reference picked.
    RotateRef { center: Vec2D, reference: Vec2D },
    /// Scale tool: base point picked.
    ScaleBase { base: Vec2D },
    /// Scale tool: base + reference picked.
    ScaleRef { base: Vec2D, reference: Vec2D },
    /// Mirror tool: first axis point picked (no line was clicked).
    MirrorAxisFrom { a: Vec2D },
}

/// Panel-editable parameters consumed by the drawing tools.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolParams {
    /// Number of sides for the regular polygon tool (3..=12).
    pub polygon_sides: u32,
    /// Full slot width (distance between the two parallel edges), mm.
    pub slot_width: f32,
    /// Corner fillet radius for the sketch fillet tool, mm.
    pub fillet_radius: f32,
    /// Corner setback for the sketch chamfer tool, mm.
    pub chamfer_length: f32,
    /// Offset distance for the offset tool, mm.
    pub offset_distance: f32,
    /// Copy count for translate/rotate (0 = move the originals).
    pub copies: u32,
    /// Whether new B-splines close on themselves.
    pub bspline_periodic: bool,
}

impl Default for ToolParams {
    fn default() -> Self {
        Self {
            polygon_sides: 6,
            slot_width: 4.0,
            fillet_radius: 2.0,
            chamfer_length: 2.0,
            offset_distance: 2.0,
            copies: 0,
            bspline_periodic: false,
        }
    }
}

impl ToolState {
    pub fn is_idle(&self) -> bool {
        matches!(self, ToolState::Idle)
    }

    /// One-line status for the UI ("click end point…").
    pub fn status(&self) -> Option<&'static str> {
        match self {
            ToolState::Idle => None,
            ToolState::LineFrom { chain: false, .. } => Some("Line: click the end point"),
            ToolState::LineFrom { chain: true, .. } => {
                Some("Line: click to chain, right-click or Esc to finish")
            }
            ToolState::RectFrom { .. } => Some("Rectangle: click the opposite corner"),
            ToolState::RectCenterAt { .. } => Some("Rectangle: click a corner"),
            ToolState::CircleFrom { .. } => Some("Circle: click a point on the rim"),
            ToolState::Circle3One { .. } => Some("Circle: click a second rim point"),
            ToolState::Circle3Two { .. } => Some("Circle: click a third rim point"),
            ToolState::ArcCenter { .. } => Some("Arc: click the start point"),
            ToolState::ArcStart { .. } => Some("Arc: click the end point (counter-clockwise)"),
            ToolState::Arc3Start { .. } => Some("Arc: click the other endpoint"),
            ToolState::Arc3End { .. } => Some("Arc: click a point on the arc"),
            ToolState::PolygonCenter { .. } => Some("Polygon: click a vertex"),
            ToolState::SlotFrom { .. } => Some("Slot: click the other end of the centerline"),
            ToolState::ArcSlotCenter { .. } => Some("Arc slot: click the centerline start"),
            ToolState::ArcSlotStart { .. } => {
                Some("Arc slot: click the centerline end (counter-clockwise)")
            }
            ToolState::EllipseCenter { .. } => Some("Ellipse: click the major-axis vertex"),
            ToolState::EllipseMajor { .. } => Some("Ellipse: click to set the minor radius"),
            ToolState::BSplineDraw { .. } => {
                Some("Spline: click control points; Enter/right-click finishes (periodic: tool settings)")
            }
            ToolState::TranslateFrom { .. } => Some("Move: click the destination"),
            ToolState::RotateCenter { .. } => Some("Rotate: click the angle reference"),
            ToolState::RotateRef { .. } => Some("Rotate: click the target angle"),
            ToolState::ScaleBase { .. } => Some("Scale: click the reference point"),
            ToolState::ScaleRef { .. } => Some("Scale: click the target point"),
            ToolState::MirrorAxisFrom { .. } => Some("Mirror: click the second axis point"),
        }
    }
}

/// What a completed tool click produced (used for logging + dirty marking).
pub struct ToolEffect {
    pub changed: bool,
    pub log: Option<String>,
}

impl ToolEffect {
    fn none() -> Self {
        Self {
            changed: false,
            log: None,
        }
    }
    fn changed(log: impl Into<String>) -> Self {
        Self {
            changed: true,
            log: Some(log.into()),
        }
    }
}

/// Resolve a snap target into a concrete point id, creating the point when
/// needed.
fn materialize(sketch: &mut Sketch, target: SnapTarget) -> Uuid {
    match target {
        SnapTarget::Existing(id) => id,
        SnapTarget::New(pos) => sketch.add_geometry(GeometryElement::Point(Point::new(pos))),
    }
}

/// Positions produced by curve snapping sit numerically ON the curve;
/// anything else is at least a full snap tolerance away, so a tiny epsilon
/// re-identifies the snapped curve at materialization time.
fn curve_attach_eps(snap_tol: f32) -> f32 {
    (snap_tol * 0.05).max(1e-5)
}

/// Like `materialize`, but a NEW point whose position lies on an existing
/// curve (arranged by `draw`'s curve snapping projecting the click) also
/// records the matching on-curve auto-constraint. Existing points are
/// reused untouched — shared ids already imply coincidence.
fn materialize_on_curve(sketch: &mut Sketch, target: SnapTarget, snap_tol: f32) -> Uuid {
    let SnapTarget::New(pos) = target else {
        return materialize(sketch, target);
    };
    let curve = snap::snap_to_curve(sketch, pos, curve_attach_eps(snap_tol), &[]);
    let point = sketch.add_geometry(GeometryElement::Point(Point::new(pos)));
    if let Some((curve_id, _)) = curve {
        let kind = match sketch.get_geometry(curve_id) {
            Some(GeometryElement::Line(_)) => ConstraintKind::PointOnLine {
                point,
                line: curve_id,
            },
            Some(GeometryElement::Circle(_) | GeometryElement::Arc(_)) => {
                ConstraintKind::PointOnCircle {
                    point,
                    circle: curve_id,
                }
            }
            _ => return point,
        };
        sketch.add_constraint(kind);
    }
    point
}

/// Advance the tool state machine with a left click at `cursor` (sketch
/// coords). `snap_tol` is in sketch units; `params` carries the
/// panel-editable tool settings; `selected` is the current selection (used
/// by the offset and transform tools).
#[allow(clippy::too_many_arguments)]
pub fn handle_click(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    params: &ToolParams,
    selected: &HashSet<Uuid>,
) -> ToolEffect {
    match tool {
        "sketch.point" => draw::point(sketch, cursor),
        "sketch.line" => draw::line(state, sketch, cursor, snap_tol),
        "sketch.rect" => draw::rect(state, sketch, cursor, snap_tol),
        "sketch.rect_center" => draw::rect_center(state, sketch, cursor),
        "sketch.circle" => draw::circle(state, sketch, cursor, snap_tol),
        "sketch.circle3" => draw::circle3(state, sketch, cursor, snap_tol),
        "sketch.arc" => draw::arc(state, sketch, cursor, snap_tol),
        "sketch.arc3" => draw::arc3(state, sketch, cursor, snap_tol),
        "sketch.ellipse" => draw::ellipse(state, sketch, cursor, snap_tol),
        "sketch.bspline" => draw::bspline(state, sketch, cursor, snap_tol),
        "sketch.polygon" => draw::polygon(state, sketch, cursor, snap_tol, params.polygon_sides),
        "sketch.slot" => draw::slot(state, sketch, cursor, snap_tol, params.slot_width),
        "sketch.arc_slot" => draw::arc_slot(state, sketch, cursor, snap_tol, params.slot_width),
        "sketch.fillet" => modify::fillet(sketch, cursor, snap_tol, params.fillet_radius),
        "sketch.chamfer" => modify::chamfer(sketch, cursor, snap_tol, params.chamfer_length),
        "sketch.trim" => modify::trim(sketch, cursor, snap_tol),
        "sketch.extend" => modify::extend(sketch, cursor, snap_tol),
        "sketch.split" => modify::split(sketch, cursor, snap_tol),
        "sketch.offset" => modify::offset(sketch, cursor, selected, params.offset_distance),
        "sketch.translate" => {
            transform::translate(state, sketch, cursor, snap_tol, selected, params.copies)
        }
        "sketch.rotate" => {
            transform::rotate(state, sketch, cursor, snap_tol, selected, params.copies)
        }
        "sketch.scale" => transform::scale(state, sketch, cursor, snap_tol, selected),
        "sketch.mirror" => transform::mirror(state, sketch, cursor, snap_tol, selected),
        _ => ToolEffect::none(),
    }
}

/// Finish a multi-click sequence (right-click or Enter): completes an
/// in-progress B-spline, drops any other pending state.
pub fn finish_click_sequence(
    state: &mut ToolState,
    sketch: &mut Sketch,
    params: &ToolParams,
) -> ToolEffect {
    match state {
        ToolState::BSplineDraw { .. } => {
            draw::bspline_finish(state, sketch, params.bspline_periodic)
        }
        _ => {
            *state = ToolState::Idle;
            ToolEffect::none()
        }
    }
}

fn short(id: Uuid) -> String {
    id.to_string()[..8].to_string()
}

#[cfg(test)]
mod tests;
