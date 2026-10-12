//! Sketch tool state machine.
//!
//! Tools accumulate *intent* (snapped targets) and only materialize
//! geometry when a shape completes, so cancelling mid-shape never leaves
//! orphan points behind. Endpoint snapping reuses existing point ids, which
//! is what makes consecutive segments and closed profiles share vertices.
//!
//! Handlers live in submodules: `draw` (new geometry), `modify`
//! (fillet/chamfer/trim/extend/split/offset), `transform`
//! (translate/rotate/scale/mirror/array over the current selection),
//! `join` (a chain of curves merged into one B-spline).

mod draw;
mod join;
mod modify;
mod transform;

use std::collections::HashSet;

use uuid::Uuid;

pub use draw::{
    arc_slot_shape, bspline_preview, frame_inner_corners, polygon_vertices, rect_center3_corners,
    rect3_corners, slot_corners,
};
pub use join::{JOIN_TOLERANCE, join};
pub use modify::{next_stroke_crossing, trim_preview};
pub use transform::{
    Similarity, array, copy_constraints, copy_from, copy_mapped, remove_axis_alignment,
};
pub(crate) use transform::{full_turn_angles, pattern_multiples};

use crate::sketch::{ConicKind, ConstraintKind, GeometryElement, Point, Sketch, Vec2D};
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
    /// Three-corner rectangle: first corner picked.
    Rect3A { a: SnapTarget },
    /// Three-corner rectangle: one edge picked, waiting for the width.
    Rect3B { a: SnapTarget, b: SnapTarget },
    /// Rectangle from its centre and two corners: centre picked.
    RectCenter3At { center: SnapTarget },
    /// Rectangle from its centre and two corners: centre and a corner
    /// picked, waiting for the direction of the next corner.
    RectCenter3Corner {
        center: SnapTarget,
        corner: SnapTarget,
    },
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
    /// Three-point ellipse: one end of the major axis picked.
    Ellipse3A { a: Vec2D },
    /// Three-point ellipse: both ends of the major axis picked, waiting
    /// for a point on the rim.
    Ellipse3B { a: Vec2D, b: Vec2D },
    /// Arc of ellipse: center picked.
    EllipseArcCenter { center: SnapTarget },
    /// Arc of ellipse: center and major vertex picked, waiting for the rim
    /// point that sets the minor radius and starts the arc.
    EllipseArcMajor {
        center: SnapTarget,
        major_pos: Vec2D,
    },
    /// Arc of ellipse: the ellipse and the arc's start are set, waiting
    /// for its end (counter-clockwise).
    EllipseArcStart {
        center: SnapTarget,
        major: Vec2D,
        ratio: f32,
        start: Vec2D,
    },
    /// Arc of parabola or hyperbola: the parabola's vertex or the
    /// hyperbola's centre picked.
    ConicAt { kind: ConicKind, center: SnapTarget },
    /// Arc of parabola or hyperbola: the axis picked (the focus, or the
    /// branch's vertex), waiting for the arc's start, which on a hyperbola
    /// also sets how wide it opens.
    ConicAxis {
        kind: ConicKind,
        center: SnapTarget,
        axis: Vec2D,
    },
    /// Arc of parabola or hyperbola: the curve and the arc's start set,
    /// waiting for its end.
    ConicStart {
        kind: ConicKind,
        center: SnapTarget,
        axis: Vec2D,
        minor: f32,
        start: Vec2D,
    },
    /// Polyline: waiting for the next segment's end. `first` is the chain's
    /// first point (a click back on it closes the shape), `heading` the
    /// direction the last segment left `from` in, `prev` that segment, and
    /// `segment` what the next segments are (M cycles).
    PolylineFrom {
        from: SnapTarget,
        first: Option<Uuid>,
        heading: Option<Vec2D>,
        prev: Option<Uuid>,
        segment: PolySegment,
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
    /// Fillet or chamfer: the first of two curves picked, where.
    CornerFirst {
        curve: Uuid,
        pick: Vec2D,
        bevel: bool,
    },
}

/// What a polyline's next segment is: a line, or an arc leaving the last
/// segment's end along it, square to it, or back the way it came.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PolySegment {
    #[default]
    Line,
    Tangent,
    Perpendicular,
    Reverse,
}

impl PolySegment {
    /// The mode after this one, as the switch key cycles them.
    pub fn next(self) -> Self {
        match self {
            PolySegment::Line => PolySegment::Tangent,
            PolySegment::Tangent => PolySegment::Perpendicular,
            PolySegment::Perpendicular => PolySegment::Reverse,
            PolySegment::Reverse => PolySegment::Line,
        }
    }

    /// Its word in a recording.
    pub fn word(self) -> &'static str {
        match self {
            PolySegment::Line => "line",
            PolySegment::Tangent => "arc",
            PolySegment::Perpendicular => "perpendicular_arc",
            PolySegment::Reverse => "reverse_arc",
        }
    }

    pub fn of_word(word: &str) -> Option<Self> {
        [
            PolySegment::Line,
            PolySegment::Tangent,
            PolySegment::Perpendicular,
            PolySegment::Reverse,
        ]
        .into_iter()
        .find(|s| s.word() == word)
    }

    /// The direction an arc of this kind leaves the end in, the last
    /// segment having left along `heading` and the cursor at `toward` from
    /// the end; `None` for a line.
    pub fn arc_heading(self, heading: Vec2D, toward: Vec2D) -> Option<Vec2D> {
        let h = heading.to_glam();
        Some(Vec2D::from_glam(match self {
            PolySegment::Line => return None,
            PolySegment::Tangent => h,
            PolySegment::Reverse => -h,
            // Square to the last segment, on the side the cursor is.
            PolySegment::Perpendicular => {
                let n = h.perp();
                if n.dot(toward.to_glam()) >= 0.0 {
                    n
                } else {
                    -n
                }
            }
        }))
    }
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
    /// A fillet or chamfer keeps the corner as a construction point on
    /// both curves, with what held it.
    pub corner_keep: bool,
    /// Offset distance for the offset tool, mm.
    pub offset_distance: f32,
    /// The offset rounds the copies' convex corners with arcs.
    pub offset_round: bool,
    /// The offset copies to both sides.
    pub offset_both: bool,
    /// The offset replaces the originals with their copies.
    pub offset_delete: bool,
    /// One offset dimension holds the copies to the originals.
    pub offset_linked: bool,
    /// Copy count for translate/rotate (0 = move the originals).
    pub copies: u32,
    /// Copies and arrays stay the originals' size, spaced by one pitch.
    pub copies_linked: bool,
    /// Translate and rotate count every element, the original included,
    /// and the clicked shift or turn spans the whole pattern.
    pub copies_total: bool,
    /// Translate and rotate repeat the copies on the other side of the
    /// original as well.
    pub copies_symmetric: bool,
    /// Rotate spreads the elements evenly around a full turn, taking only
    /// the centre's click.
    pub rotate_full_turn: bool,
    /// Whether new B-splines close on themselves.
    pub bspline_periodic: bool,
    /// The degree of new B-splines (2 to 5).
    pub bspline_degree: u32,
    /// New B-splines pass through the clicks rather than use them as
    /// control points.
    pub bspline_interpolate: bool,
    /// Whether the line tool adds horizontal/vertical constraints to
    /// axis-snapped segments.
    pub auto_constraints: bool,
    /// The rectangular array: rows and columns, and the step between them.
    pub array_rows: u32,
    pub array_cols: u32,
    pub array_dx: f32,
    pub array_dy: f32,
    /// The mirror keeps the original beside its image; off, it moves it.
    pub mirror_keep: bool,
    /// The image holds to the original with symmetric constraints, so it
    /// follows edits.
    pub mirror_linked: bool,
    /// The mirror reflects through a point rather than about a line.
    pub mirror_center: bool,
    /// Trim and Extend cut against the sketch's two axes as well.
    pub cut_at_axes: bool,
}

impl Default for ToolParams {
    fn default() -> Self {
        Self {
            polygon_sides: 6,
            slot_width: 4.0,
            fillet_radius: 2.0,
            chamfer_length: 2.0,
            corner_keep: false,
            offset_distance: 2.0,
            offset_round: false,
            offset_both: false,
            offset_delete: false,
            offset_linked: false,
            copies: 0,
            copies_linked: false,
            copies_total: false,
            copies_symmetric: false,
            rotate_full_turn: false,
            bspline_periodic: false,
            bspline_degree: 3,
            bspline_interpolate: false,
            auto_constraints: true,
            array_rows: 2,
            array_cols: 2,
            array_dx: 20.0,
            array_dy: 20.0,
            mirror_keep: true,
            mirror_linked: false,
            mirror_center: false,
            cut_at_axes: false,
        }
    }
}

impl ToolState {
    pub fn is_idle(&self) -> bool {
        matches!(self, ToolState::Idle)
    }

    /// The viewport hint while a shape is in progress: the tool's name and
    /// what the next click does.
    pub fn hint(&self) -> Option<(&'static str, &'static str)> {
        self.status().and_then(|s| s.split_once(": "))
    }

    /// One-line status for the UI ("click end point…").
    pub fn status(&self) -> Option<&'static str> {
        match self {
            ToolState::Idle => None,
            ToolState::LineFrom { chain: false, .. } => Some("Line: click the end point"),
            ToolState::LineFrom { chain: true, .. } => Some(
                "Line: click to chain, drag from the end for a tangent arc, right-click or Esc to finish",
            ),
            ToolState::RectFrom { .. } => Some("Rectangle: click the opposite corner"),
            ToolState::RectCenterAt { .. } => Some("Rectangle: click a corner"),
            ToolState::Rect3A { .. } => Some("Rectangle: click the second corner"),
            ToolState::Rect3B { .. } => Some("Rectangle: click to set the width"),
            ToolState::RectCenter3At { .. } => Some("Rectangle: click a corner"),
            ToolState::RectCenter3Corner { .. } => Some("Rectangle: click toward the next corner"),
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
            ToolState::Ellipse3A { .. } => Some("Ellipse: click the other end of the major axis"),
            ToolState::Ellipse3B { .. } => Some("Ellipse: click a point on the rim"),
            ToolState::EllipseArcCenter { .. } => {
                Some("Arc of ellipse: click the major-axis vertex")
            }
            ToolState::EllipseArcMajor { .. } => {
                Some("Arc of ellipse: click the arc's start on the rim (sets the minor radius)")
            }
            ToolState::EllipseArcStart { .. } => {
                Some("Arc of ellipse: click the arc's end (counter-clockwise)")
            }
            ToolState::ConicAt {
                kind: ConicKind::Parabola,
                ..
            } => Some("Arc of parabola: click the focus"),
            ToolState::ConicAt {
                kind: ConicKind::Hyperbola,
                ..
            } => Some("Arc of hyperbola: click the vertex of the branch"),
            ToolState::ConicAxis {
                kind: ConicKind::Parabola,
                ..
            } => Some("Arc of parabola: click the arc's start"),
            ToolState::ConicAxis {
                kind: ConicKind::Hyperbola,
                ..
            } => Some("Arc of hyperbola: click the arc's start beyond the vertex"),
            ToolState::ConicStart {
                kind: ConicKind::Parabola,
                ..
            } => Some("Arc of parabola: click the arc's end"),
            ToolState::ConicStart {
                kind: ConicKind::Hyperbola,
                ..
            } => Some("Arc of hyperbola: click the arc's end"),
            ToolState::PolylineFrom {
                segment: PolySegment::Line,
                ..
            } => Some("Polyline: click the next point; M for arcs, right-click or Esc to finish"),
            ToolState::PolylineFrom {
                segment: PolySegment::Tangent,
                ..
            } => Some("Polyline: click where the tangent arc ends; M for a square arc"),
            ToolState::PolylineFrom {
                segment: PolySegment::Perpendicular,
                ..
            } => Some("Polyline: click where the square arc ends; M for a reversed arc"),
            ToolState::PolylineFrom {
                segment: PolySegment::Reverse,
                ..
            } => Some("Polyline: click where the reversed arc ends; M for lines"),
            ToolState::BSplineDraw { .. } => Some(
                "Spline: click its points; Enter/right-click finishes (degree, closed: tool settings)",
            ),
            ToolState::TranslateFrom { .. } => Some("Move: click the destination"),
            ToolState::RotateCenter { .. } => Some("Rotate: click the angle reference"),
            ToolState::RotateRef { .. } => Some("Rotate: click the target angle"),
            ToolState::ScaleBase { .. } => Some("Scale: click the reference point"),
            ToolState::ScaleRef { .. } => Some("Scale: click the target point"),
            ToolState::MirrorAxisFrom { .. } => Some("Mirror: click the second axis point"),
            ToolState::CornerFirst { bevel: false, .. } => Some("Fillet: click the second curve"),
            ToolState::CornerFirst { bevel: true, .. } => Some("Chamfer: click the second curve"),
        }
    }
}

/// What a completed tool click produced (used for logging + dirty marking).
pub struct ToolEffect {
    pub changed: bool,
    pub log: Option<String>,
}

impl ToolEffect {
    pub(crate) fn none() -> Self {
        Self {
            changed: false,
            log: None,
        }
    }
    pub(crate) fn changed(log: impl Into<String>) -> Self {
        Self {
            changed: true,
            log: Some(log.into()),
        }
    }
    /// Nothing changed, and why.
    pub(crate) fn log(log: impl Into<String>) -> Self {
        Self {
            changed: false,
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

/// Like `materialize`, but a NEW point placed where a snap put it stays
/// there: at a line's middle by a midpoint constraint, on the origin by a
/// coincidence, on each curve it lies on (two at a crossing) by an on-curve
/// constraint, found again within `curve_attach_eps`. Existing points are
/// reused untouched: shared ids already imply coincidence.
fn materialize_on_curve(sketch: &mut Sketch, target: SnapTarget, snap_tol: f32) -> Uuid {
    let SnapTarget::New(pos) = target else {
        return materialize(sketch, target);
    };
    let eps = curve_attach_eps(snap_tol);
    let point = sketch.add_geometry(GeometryElement::Point(Point::new(pos)));
    if pos.to_glam().length() <= eps {
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: point,
            point2: crate::sketch::ORIGIN_ID,
        });
        return point;
    }
    let mut on: Vec<ConstraintKind> = Vec::new();
    for (axis, off) in [
        (crate::sketch::X_AXIS_ID, pos.y),
        (crate::sketch::Y_AXIS_ID, pos.x),
    ] {
        if off.abs() <= eps {
            on.push(ConstraintKind::PointOnLine { point, line: axis });
        }
    }
    for geom in &sketch.geometry {
        if geom.id() == point
            || matches!(geom, GeometryElement::Point(_))
            || !snap::distance_to_element(sketch, geom, pos).is_some_and(|d| d <= eps)
        {
            continue;
        }
        let kind = match geom {
            GeometryElement::Line(l) => {
                let middle = sketch
                    .point_position(l.start)
                    .zip(sketch.point_position(l.end))
                    .map(|(a, b)| Vec2D::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5));
                if middle.is_some_and(|m| (m - pos).to_glam().length() <= eps) {
                    ConstraintKind::Midpoint { point, line: l.id }
                } else {
                    ConstraintKind::PointOnLine { point, line: l.id }
                }
            }
            GeometryElement::Circle(_) | GeometryElement::Arc(_) => ConstraintKind::PointOnCircle {
                point,
                circle: geom.id(),
            },
            GeometryElement::Ellipse(_) => ConstraintKind::PointOnEllipse {
                point,
                ellipse: geom.id(),
            },
            _ => continue,
        };
        on.push(kind);
    }
    // Two are a crossing; a third would only over-constrain the point.
    for kind in on.into_iter().take(2) {
        sketch.add_constraint(kind);
    }
    point
}

/// What a snap made in tool `state` knows of the drawing in
/// progress: a straight segment's start, for alignment, which it must not
/// land back on.
pub fn snap_context(state: &ToolState, sketch: &Sketch) -> snap::SnapContext {
    let from = match state {
        ToolState::LineFrom { from, .. }
        | ToolState::PolylineFrom {
            from,
            segment: PolySegment::Line,
            ..
        } => Some(*from),
        _ => None,
    };
    snap::SnapContext {
        from: from.and_then(|f| f.position(sketch)),
        exclude: match from {
            Some(SnapTarget::Existing(id)) => vec![id],
            _ => Vec::new(),
        },
    }
}

/// Where a click in tool `state` at `cursor` lands, and on what:
/// what the click uses and what the cue before it shows.
pub fn snap_at(state: &ToolState, sketch: &Sketch, cursor: Vec2D, tol: f32) -> snap::Snap {
    snap::resolve(sketch, cursor, tol, &snap_context(state, sketch))
}

/// Advance the tool state machine with a left click at `cursor` (sketch
/// coords). `snap_tol` is in sketch units; `params` carries the
/// panel-editable tool settings; `selected` is the current selection (used
/// by the offset and transform tools).
pub fn handle_click(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    params: &ToolParams,
    selected: &HashSet<Uuid>,
) -> ToolEffect {
    // A transform with nothing to act on says so at its first click,
    // rather than walking through its clicks to do nothing.
    if selected.is_empty()
        && state.is_idle()
        && matches!(
            tool,
            "sketch.translate" | "sketch.rotate" | "sketch.scale" | "sketch.mirror"
        )
    {
        return ToolEffect::log("Select the geometry to transform first");
    }
    match tool {
        "sketch.point" => draw::point(sketch, cursor, snap_tol),
        "sketch.line" => draw::line(state, sketch, cursor, snap_tol, params.auto_constraints),
        "sketch.polyline" => {
            draw::polyline(state, sketch, cursor, snap_tol, params.auto_constraints)
        }
        "sketch.rect" => draw::rect(state, sketch, cursor, snap_tol),
        "sketch.rect_rounded" => {
            draw::rect_rounded(state, sketch, cursor, snap_tol, params.fillet_radius)
        }
        "sketch.rect_center" => draw::rect_center(state, sketch, cursor, snap_tol),
        "sketch.rect3" => draw::rect3(state, sketch, cursor, snap_tol),
        "sketch.rect_center3" => draw::rect_center3(state, sketch, cursor, snap_tol),
        "sketch.rect_frame" => {
            draw::rect_frame(state, sketch, cursor, snap_tol, params.offset_distance)
        }
        "sketch.circle" => draw::circle(state, sketch, cursor, snap_tol),
        "sketch.circle3" => draw::circle3(state, sketch, cursor, snap_tol),
        "sketch.arc" => draw::arc(state, sketch, cursor, snap_tol),
        "sketch.arc3" => draw::arc3(state, sketch, cursor, snap_tol),
        "sketch.ellipse" => draw::ellipse(state, sketch, cursor, snap_tol),
        "sketch.ellipse3" => draw::ellipse3(state, sketch, cursor, snap_tol),
        "sketch.ellipse_arc" => draw::ellipse_arc(state, sketch, cursor, snap_tol),
        "sketch.bspline" => draw::bspline(state, sketch, cursor, snap_tol),
        "sketch.parabola" => draw::conic(state, sketch, cursor, snap_tol, ConicKind::Parabola),
        "sketch.hyperbola" => draw::conic(state, sketch, cursor, snap_tol, ConicKind::Hyperbola),
        "sketch.polygon" => draw::polygon(state, sketch, cursor, snap_tol, params.polygon_sides),
        "sketch.slot" => draw::slot(state, sketch, cursor, snap_tol, params.slot_width),
        "sketch.arc_slot" => draw::arc_slot(state, sketch, cursor, snap_tol, params.slot_width),
        "sketch.fillet" => modify::corner(
            state,
            sketch,
            cursor,
            snap_tol,
            modify::CornerCut::Round(params.fillet_radius),
            params.corner_keep,
        ),
        "sketch.chamfer" => modify::corner(
            state,
            sketch,
            cursor,
            snap_tol,
            modify::CornerCut::Bevel(params.chamfer_length),
            params.corner_keep,
        ),
        "sketch.trim" => modify::trim(sketch, cursor, snap_tol, params.cut_at_axes),
        "sketch.bspline_knot" => crate::spline_edit::knot_at_click(sketch, cursor, snap_tol),
        "sketch.extend" => modify::extend(sketch, cursor, snap_tol, params.cut_at_axes),
        "sketch.split" => modify::split(sketch, cursor, snap_tol),
        "sketch.offset" => modify::offset(
            sketch,
            cursor,
            selected,
            modify::OffsetOptions {
                distance: params.offset_distance,
                round: params.offset_round,
                both: params.offset_both,
                delete: params.offset_delete,
                linked: params.offset_linked,
            },
        ),
        "sketch.translate" => {
            transform::translate(state, sketch, cursor, snap_tol, selected, params)
        }
        "sketch.rotate" => transform::rotate(state, sketch, cursor, snap_tol, selected, params),
        "sketch.scale" => {
            transform::scale(state, sketch, cursor, snap_tol, selected, params.copies)
        }
        "sketch.mirror" => transform::mirror(state, sketch, cursor, snap_tol, selected, params),
        _ => ToolEffect::none(),
    }
}

/// Switch a polyline in progress between straight segments and tangent
/// arcs. Returns whether there was a polyline to switch.
pub fn toggle_polyline_arc(state: &mut ToolState) -> bool {
    match state {
        ToolState::PolylineFrom {
            segment, heading, ..
        } => {
            // An arc needs a direction to leave in: the first segment is
            // straight.
            *segment = if heading.is_some() {
                segment.next()
            } else {
                PolySegment::Line
            };
            true
        }
        _ => false,
    }
}

/// The curve that ends at point `at` and the way it runs into it: a line
/// from its other end, an arc as it turns there. The latest such curve
/// when several end there; `None` when none does.
pub fn heading_into(sketch: &Sketch, at: Uuid) -> Option<(Uuid, Vec2D)> {
    let pos = |id: Uuid| sketch.point_position(id);
    let here = pos(at)?;
    sketch.geometry.iter().rev().find_map(|g| {
        let heading = match g {
            GeometryElement::Line(l) if l.end == at => here - pos(l.start)?,
            GeometryElement::Line(l) if l.start == at => here - pos(l.end)?,
            // An arc runs counter-clockwise from its start to its end.
            GeometryElement::Arc(a) if a.end == at => {
                Vec2D::from_glam((here - pos(a.center)?).to_glam().perp())
            }
            GeometryElement::Arc(a) if a.start == at => {
                Vec2D::from_glam(-(here - pos(a.center)?).to_glam().perp())
            }
            _ => return None,
        };
        (heading.to_glam().length() > 1e-9).then(|| (g.id(), heading))
    })
}

/// The line tool's arc: from the point the line tool draws from, turn it
/// into a polyline's tangent-arc step, heading as the curve that ends
/// there runs. Whether there was one to be tangent to.
pub fn line_arc_begin(state: &mut ToolState, sketch: &Sketch) -> bool {
    let from = match *state {
        ToolState::LineFrom {
            from: SnapTarget::Existing(from),
            ..
        } => from,
        ToolState::PolylineFrom {
            segment: PolySegment::Tangent,
            heading: Some(_),
            ..
        } => return true,
        _ => return false,
    };
    let Some((prev, heading)) = heading_into(sketch, from) else {
        return false;
    };
    *state = ToolState::PolylineFrom {
        from: SnapTarget::Existing(from),
        first: None,
        heading: Some(heading),
        prev: Some(prev),
        segment: PolySegment::Tangent,
    };
    true
}

/// Back to the line tool after its arc, drawing on from where the arc ended.
pub fn line_arc_end(state: &mut ToolState) {
    if let ToolState::PolylineFrom { from, .. } = *state {
        *state = ToolState::LineFrom { from, chain: true };
    }
}

/// Put a polyline in progress on segment kind `to` (a recording's switch).
pub fn set_polyline_segment(state: &mut ToolState, to: PolySegment) {
    if let ToolState::PolylineFrom {
        segment, heading, ..
    } = state
    {
        *segment = if heading.is_some() {
            to
        } else {
            PolySegment::Line
        };
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
        ToolState::BSplineDraw { .. } => draw::bspline_finish(state, sketch, params),
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
