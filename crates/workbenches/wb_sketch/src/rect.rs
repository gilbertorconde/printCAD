//! The rectangle tools' runtime: the click that finishes a rectangle
//! proposes the whole shape ([`propose`]): where each corner lands and
//! which point it reuses, the centre a centred one is held about, a
//! rounded one's radius, a frame's inner outline. The preview draws that
//! proposal ([`outline`]) and the click applies it ([`apply`]), so a
//! rectangle the preview shows is the one that lands, and one it shows
//! refused (dashed) is refused by the click too.
//!
//! The clicks before the last only pick corners or a centre and change
//! nothing. [`apply`] is the one function here that writes to the sketch;
//! it checks every point the proposal reuses before its first write, so a
//! proposal gone stale is refused whole.

use uuid::Uuid;

use crate::sketch::{ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::snap::{self, SnapTarget};
use crate::tools::{self, ToolEffect, ToolParams, ToolState};

/// Which rectangle a tool draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// From two opposite corners.
    Corners,
    /// From its centre and a corner.
    Center,
    /// From two opposite corners, the corners rounded.
    Rounded,
    /// From two opposite corners, with a second outline a wall inside.
    Frame,
    /// At any angle, from three corners.
    Three,
    /// At any angle, from its centre and two neighbouring corners.
    CenterThree,
}

impl Kind {
    pub fn of(tool: &str) -> Option<Kind> {
        Some(match tool {
            "sketch.rect" => Kind::Corners,
            "sketch.rect_center" => Kind::Center,
            "sketch.rect_rounded" => Kind::Rounded,
            "sketch.rect_frame" => Kind::Frame,
            "sketch.rect3" => Kind::Three,
            "sketch.rect_center3" => Kind::CenterThree,
            _ => return None,
        })
    }

    /// Whether a click in `state` finishes this kind's rectangle.
    fn finishes(self, state: &ToolState) -> bool {
        matches!(
            (self, state),
            (
                Kind::Corners | Kind::Rounded | Kind::Frame,
                ToolState::RectFrom { .. }
            ) | (Kind::Center, ToolState::RectCenterAt { .. })
                | (Kind::Three, ToolState::Rect3B { .. })
                | (Kind::CenterThree, ToolState::RectCenter3Corner { .. })
        )
    }

    /// Edges held level and plumb, rather than square to each other.
    fn level(self) -> bool {
        !matches!(self, Kind::Three | Kind::CenterThree)
    }
}

/// A corner (or centre) of a proposal: where it lands and the point it
/// is, reused or new. A clicked one is held on the curve a snap put it
/// on; one worked out from the clicks only reuses a point exactly there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Corner {
    pub at: Vec2D,
    pub target: SnapTarget,
    pub held: bool,
}

/// The rectangle a finishing click proposes: what [`apply`] writes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proposal {
    pub kind: Kind,
    /// In order round the rectangle; level ones counter-clockwise from
    /// the first corner clicked, or the lower left for a centred one.
    pub corners: [Corner; 4],
    /// The centre a centred rectangle is held symmetric about.
    pub centre: Option<Corner>,
    /// A rounded rectangle's corner radius.
    pub radius: f64,
    /// A frame's wall and its inner outline, corners in the outer's order.
    pub inner: Option<(f64, [Vec2D; 4])>,
    pub tol: f64,
}

impl Proposal {
    /// The rectangle's two side lengths, the first edge's first.
    pub fn size(&self) -> (f64, f64) {
        let c = self.corners.map(|c| c.at.to_glam());
        ((c[1] - c[0]).length(), (c[2] - c[1]).length())
    }
}

/// Why a click finishes no rectangle. Each leaves the sketch as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Rejection {
    /// The click only picks a corner or the centre.
    Waiting,
    /// A corner or centre picked before is gone (an undo, a script).
    StartGone,
    /// The rectangle would have no area.
    Degenerate,
    /// No fillet radius or wall is set.
    NoSize(Kind),
    /// The corners' radius or the wall does not fit; the outline it
    /// would have had.
    NoRoom {
        kind: Kind,
        size: f64,
        outline: [Vec2D; 4],
    },
    /// A point the proposal reuses is gone.
    SourceGone(Uuid),
}

impl Rejection {
    pub fn message(self) -> String {
        match self {
            Rejection::Waiting | Rejection::Degenerate => String::new(),
            Rejection::StartGone => "The corner the rectangle was drawn from is gone".into(),
            Rejection::NoSize(Kind::Frame) => "Set a wall thickness first".into(),
            Rejection::NoSize(_) => "Set a fillet radius first".into(),
            Rejection::NoRoom {
                kind,
                size,
                outline: o,
            } => {
                let (w, h) = ((o[2].x - o[0].x).abs(), (o[2].y - o[0].y).abs());
                if kind == Kind::Frame {
                    format!("A frame {w:.2} × {h:.2} has no room for a {size:.2} wall")
                } else {
                    format!(
                        "A rectangle {w:.2} × {h:.2} has no room for corners of radius {size:.2}"
                    )
                }
            }
            Rejection::SourceGone(_) => {
                "A point the rectangle was drawn to is gone; nothing was added".into()
            }
        }
    }
}

/// What [`apply`] made: the outer outline's corners and edges, in the
/// proposal's order (a rounded one's corners are gone, cut by its arcs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Applied {
    pub corners: [Uuid; 4],
    pub edges: [Uuid; 4],
}

/// The point at `pos` a click put there: an existing one exactly there,
/// else one held on the curve it lies exactly on.
fn clicked(sketch: &Sketch, pos: Vec2D, tol: f64) -> Corner {
    let eps = tools::curve_attach_eps(tol);
    Corner {
        at: pos,
        target: tools::snap_point_or_curve(sketch, pos, eps),
        held: true,
    }
}

/// The point at `pos` worked out from the clicks: an existing one exactly
/// there, else a new, free one. Nothing was snapped there, so nothing
/// holds it there.
fn derived(sketch: &Sketch, pos: Vec2D, tol: f64) -> Corner {
    let eps = tools::curve_attach_eps(tol);
    Corner {
        at: pos,
        target: snap::snap_to_point(sketch, pos, eps, &[]),
        held: false,
    }
}

/// A corner picked by an earlier click, where it is now.
fn picked(sketch: &Sketch, target: SnapTarget) -> Result<Corner, Rejection> {
    Ok(Corner {
        at: target.position(sketch).ok_or(Rejection::StartGone)?,
        target,
        held: true,
    })
}

/// The rectangle a click of `kind` at `cursor` in `state` finishes, `tol`
/// the reach it finds points within: what the click applies and the
/// preview shows.
pub(crate) fn propose(
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    tol: f64,
    kind: Kind,
    params: &ToolParams,
) -> Result<Proposal, Rejection> {
    if !kind.finishes(state) {
        return Err(Rejection::Waiting);
    }
    let mut proposal = Proposal {
        kind,
        corners: [derived(sketch, cursor, tol); 4],
        centre: None,
        radius: params.fillet_radius,
        inner: None,
        tol,
    };
    match *state {
        ToolState::RectFrom { corner } => {
            let a = picked(sketch, corner)?;
            let c = cursor;
            if (c.x - a.at.x).abs() < 1e-6 || (c.y - a.at.y).abs() < 1e-6 {
                return Err(Rejection::Degenerate);
            }
            let (b, d) = (Vec2D::new(c.x, a.at.y), Vec2D::new(a.at.x, c.y));
            proposal.corners = [
                a,
                derived(sketch, b, tol),
                clicked(sketch, c, tol),
                derived(sketch, d, tol),
            ];
            let outline = [a.at, b, c, d];
            match kind {
                Kind::Rounded => {
                    let r = params.fillet_radius;
                    if !(r >= 1e-6 && r.is_finite()) {
                        return Err(Rejection::NoSize(kind));
                    }
                    if 2.0 * r >= (c.x - a.at.x).abs().min((c.y - a.at.y).abs()) - 1e-6 {
                        return Err(Rejection::NoRoom {
                            kind,
                            size: r,
                            outline,
                        });
                    }
                    // Each corner is cut away by its arc: a new point each,
                    // never one shared with what is drawn there already.
                    for (corner, at) in proposal.corners.iter_mut().zip(outline) {
                        *corner = Corner {
                            at,
                            target: SnapTarget::New(at),
                            held: false,
                        };
                    }
                }
                Kind::Frame => {
                    let wall = params.offset_distance;
                    if !(wall >= 1e-6 && wall.is_finite()) {
                        return Err(Rejection::NoSize(kind));
                    }
                    let inner = frame_inner_corners(a.at, c, wall).ok_or(Rejection::NoRoom {
                        kind,
                        size: wall,
                        outline,
                    })?;
                    proposal.inner = Some((wall, inner));
                }
                _ => {}
            }
        }
        ToolState::RectCenterAt { center } => {
            let k = cursor;
            if (k.x - center.x).abs() < 1e-6 || (k.y - center.y).abs() < 1e-6 {
                return Err(Rejection::Degenerate);
            }
            // The opposite corner mirrored through the centre.
            let o = Vec2D::new(2.0 * center.x - k.x, 2.0 * center.y - k.y);
            let (x0, x1) = (o.x.min(k.x), o.x.max(k.x));
            let (y0, y1) = (o.y.min(k.y), o.y.max(k.y));
            proposal.corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| derived(sketch, Vec2D::new(x, y), tol));
            proposal.centre = Some(clicked(sketch, center, tol));
        }
        ToolState::Rect3B { a, b } => {
            let (a, b) = (picked(sketch, a)?, picked(sketch, b)?);
            let (c, d) = rect3_corners(a.at, b.at, cursor).ok_or(Rejection::Degenerate)?;
            proposal.corners = [a, b, clicked(sketch, c, tol), derived(sketch, d, tol)];
        }
        ToolState::RectCenter3Corner { center, corner } => {
            let (centre, k) = (picked(sketch, center)?, picked(sketch, corner)?);
            let [_, p2, p3, p4] =
                rect_center3_corners(centre.at, k.at, cursor).ok_or(Rejection::Degenerate)?;
            proposal.corners = [
                k,
                derived(sketch, p2, tol),
                derived(sketch, p3, tol),
                derived(sketch, p4, tol),
            ];
            proposal.centre = Some(centre);
        }
        _ => return Err(Rejection::Waiting),
    }
    Ok(proposal)
}

/// The lines the preview draws for `proposal`: the outline, a rounded
/// one's arcs at its corners, and a frame's inner outline.
pub(crate) fn outline(proposal: &Proposal) -> Vec<Vec<Vec2D>> {
    let corners = proposal.corners.map(|c| c.at);
    let closed = |c: [Vec2D; 4]| c.into_iter().chain([c[0]]).collect::<Vec<_>>();
    match (proposal.kind, proposal.inner) {
        (Kind::Rounded, _) => vec![rounded(corners[0], corners[2], proposal.radius)],
        (_, Some((_, inner))) => vec![closed(corners), closed(inner)],
        _ => vec![closed(corners)],
    }
}

/// The outline of the level rectangle from `a` to `c` with its corners
/// rounded at `r`, counter-clockwise from the lower right corner's arc.
fn rounded(a: Vec2D, c: Vec2D, r: f64) -> Vec<Vec2D> {
    let (x0, x1) = (a.x.min(c.x), a.x.max(c.x));
    let (y0, y1) = (a.y.min(c.y), a.y.max(c.y));
    let mut out = Vec::new();
    for (cx, cy, start) in [
        (x1 - r, y0 + r, -90.0f64),
        (x1 - r, y1 - r, 0.0),
        (x0 + r, y1 - r, 90.0),
        (x0 + r, y0 + r, 180.0),
    ] {
        for step in 0..=8 {
            let t = (start + step as f64 * 90.0 / 8.0).to_radians();
            out.push(Vec2D::new(cx + r * t.cos(), cy + r * t.sin()));
        }
    }
    out.push(out[0]);
    out
}

/// Four corner points in order → four shared-vertex lines.
fn edges(sketch: &mut Sketch, corners: [Uuid; 4]) -> [Uuid; 4] {
    std::array::from_fn(|i| {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )))
    })
}

/// A level rectangle's four lines, held level and plumb so it stays a
/// rectangle under later edits.
fn level_edges(sketch: &mut Sketch, corners: [Uuid; 4]) -> [Uuid; 4] {
    let e = edges(sketch, corners);
    sketch.add_constraint(ConstraintKind::Horizontal { element: e[0] });
    sketch.add_constraint(ConstraintKind::Horizontal { element: e[2] });
    sketch.add_constraint(ConstraintKind::Vertical { element: e[1] });
    sketch.add_constraint(ConstraintKind::Vertical { element: e[3] });
    e
}

/// The point `corner` is, reused or made.
fn point_of(sketch: &mut Sketch, corner: &Corner, tol: f64) -> Uuid {
    if corner.held {
        tools::materialize_on_curve(sketch, corner.target, tol)
    } else {
        tools::materialize(sketch, corner.target)
    }
}

/// Write `proposal` into `sketch`. Every point it reuses is checked
/// first, so a refusal writes nothing.
pub(crate) fn apply(sketch: &mut Sketch, proposal: &Proposal) -> Result<Applied, Rejection> {
    for corner in proposal.corners.iter().chain(&proposal.centre) {
        if let SnapTarget::Existing(id) = corner.target
            && sketch.point_position(id).is_none()
        {
            return Err(Rejection::SourceGone(id));
        }
    }
    let tol = proposal.tol;
    // A frame's inner outline first: the outer one's edges are the last
    // lines made, which is where typed width and height look for them.
    let inner = proposal.inner.map(|(_, inner)| {
        let ids = inner.map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(p))));
        level_edges(sketch, ids);
        ids
    });
    let centre_first = proposal.kind == Kind::CenterThree;
    let mut centre = proposal
        .centre
        .filter(|_| centre_first)
        .map(|c| point_of(sketch, &c, tol));
    let corners = proposal.corners.map(|c| point_of(sketch, &c, tol));
    let edges = if proposal.kind.level() {
        level_edges(sketch, corners)
    } else {
        edges(sketch, corners)
    };
    if let Some(c) = proposal.centre.filter(|_| !centre_first) {
        centre = Some(point_of(sketch, &c, tol));
    }
    // A centre picked on nothing is a construction point of its own.
    if let (Some(id), Some(c)) = (centre, proposal.centre)
        && matches!(c.target, SnapTarget::New(_))
    {
        sketch.set_construction(id, true);
    }
    match proposal.kind {
        Kind::Center => {
            if let Some(centre) = centre {
                sketch.add_constraint(ConstraintKind::SymmetricAboutPoint {
                    point1: corners[0],
                    point2: corners[2],
                    center: centre,
                });
            }
        }
        Kind::Three => {
            // Opposite sides parallel and the first corner square: a
            // rectangle free to turn.
            for (line1, line2) in [(edges[0], edges[2]), (edges[1], edges[3])] {
                sketch.add_constraint(ConstraintKind::Parallel { line1, line2 });
            }
            sketch.add_constraint(ConstraintKind::Perpendicular {
                line1: edges[0],
                line2: edges[1],
            });
        }
        Kind::CenterThree => {
            if let Some(centre) = centre {
                for (point1, point2) in [(corners[0], corners[2]), (corners[1], corners[3])] {
                    sketch.add_constraint(ConstraintKind::SymmetricAboutPoint {
                        point1,
                        point2,
                        center: centre,
                    });
                }
            }
            sketch.add_constraint(ConstraintKind::Perpendicular {
                line1: edges[0],
                line2: edges[1],
            });
        }
        Kind::Frame => {
            // The wall is held on every side, measured between the outer
            // and inner corners at the first and third corners.
            if let (Some(inner), Some((wall, _))) = (inner, proposal.inner) {
                for (outer, inner) in [(corners[0], inner[0]), (corners[2], inner[2])] {
                    sketch.add_constraint(ConstraintKind::DistanceX {
                        a: outer,
                        b: Some(inner),
                        value: wall,
                    });
                    sketch.add_constraint(ConstraintKind::DistanceY {
                        a: outer,
                        b: Some(inner),
                        value: wall,
                    });
                }
            }
        }
        Kind::Rounded => {
            // Each corner cut by an arc as the fillet tool cuts it, the
            // curves meeting there in the order they were made, then one
            // radius at every corner.
            let before: std::collections::HashSet<Uuid> =
                sketch.geometry.iter().map(GeometryElement::id).collect();
            let meeting = [
                [edges[0], edges[3]],
                [edges[0], edges[1]],
                [edges[1], edges[2]],
                [edges[2], edges[3]],
            ];
            for (corner, curves) in corners.into_iter().zip(meeting) {
                tools::modify::round_corner(sketch, corner, curves, proposal.radius);
            }
            let arcs: Vec<Uuid> = sketch
                .geometry
                .iter()
                .filter(|g| matches!(g, GeometryElement::Arc(_)) && !before.contains(&g.id()))
                .map(GeometryElement::id)
                .collect();
            for pair in arcs.windows(2) {
                sketch.add_constraint(ConstraintKind::EqualRadius {
                    circle1: pair[0],
                    circle2: pair[1],
                });
            }
        }
        Kind::Corners => {}
    }
    Ok(Applied { corners, edges })
}

/// One click of a rectangle tool of `kind`: a corner or the centre
/// picked, or the rectangle proposed and applied. A refusal changes
/// nothing; a corner that is gone ends the gesture.
pub(crate) fn click(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    tol: f64,
    params: &ToolParams,
    kind: Kind,
) -> ToolEffect {
    let rejected = |state: &mut ToolState, why: Rejection| {
        if why == Rejection::StartGone {
            *state = ToolState::Idle;
        }
        match why.message() {
            m if m.is_empty() => ToolEffect::none(),
            m => ToolEffect::log(m),
        }
    };
    match propose(state, sketch, cursor, tol, kind, params) {
        Ok(proposal) => {
            if let Err(why) = apply(sketch, &proposal) {
                return rejected(state, why);
            }
            *state = ToolState::Idle;
            return ToolEffect::changed(log(&proposal));
        }
        Err(Rejection::Waiting) => {}
        Err(why) => return rejected(state, why),
    }
    let target = tools::snap_point_or_curve(sketch, cursor, tol);
    // A corner after the first: from the one picked, which must still be
    // there and somewhere else.
    let from = |picked: SnapTarget| match picked.position(sketch) {
        None => Err(Rejection::StartGone),
        Some(at)
            if (target.position(sketch).unwrap_or(cursor) - at)
                .to_glam()
                .length()
                < 1e-6 =>
        {
            Err(Rejection::Degenerate)
        }
        Some(_) => Ok(picked),
    };
    let next = match (kind, state.clone()) {
        (Kind::Center, _) => Ok(ToolState::RectCenterAt { center: cursor }),
        (Kind::Three, ToolState::Rect3A { a }) => {
            from(a).map(|a| ToolState::Rect3B { a, b: target })
        }
        (Kind::CenterThree, ToolState::RectCenter3At { center }) => {
            from(center).map(|center| ToolState::RectCenter3Corner {
                center,
                corner: target,
            })
        }
        (Kind::Three, _) => Ok(ToolState::Rect3A { a: target }),
        (Kind::CenterThree, _) => Ok(ToolState::RectCenter3At { center: target }),
        _ => Ok(ToolState::RectFrom { corner: target }),
    };
    match next {
        Ok(next) => {
            *state = next;
            ToolEffect::none()
        }
        Err(why) => rejected(state, why),
    }
}

/// The log line for an applied `proposal`.
fn log(proposal: &Proposal) -> String {
    let (w, h) = proposal.size();
    match proposal.kind {
        Kind::Corners | Kind::Center => format!("Rectangle {w:.2} × {h:.2}"),
        Kind::Rounded => format!(
            "Rounded rectangle {w:.2} × {h:.2}, r={:.2}",
            proposal.radius
        ),
        Kind::Frame => format!(
            "Frame {w:.2} × {h:.2}, wall {:.2}",
            proposal.inner.map_or(0.0, |(wall, _)| wall)
        ),
        Kind::Three => format!("Rectangle {w:.2} × {h:.2} from three corners"),
        Kind::CenterThree => format!("Rectangle {w:.2} × {h:.2} from its centre and two corners"),
    }
}

/// The other two corners of the rectangle with edge `a → b` whose opposite
/// edge passes through `toward`: `(c, d)`, with `c` beyond `b`. `None` when
/// the edge is degenerate or `toward` lies on its line.
pub fn rect3_corners(a: Vec2D, b: Vec2D, toward: Vec2D) -> Option<(Vec2D, Vec2D)> {
    let edge = (b - a).to_glam();
    let len = edge.length();
    if len < 1e-6 {
        return None;
    }
    let n = edge.perp() / len;
    let width = n.dot((toward - a).to_glam());
    if width.abs() < 1e-6 {
        return None;
    }
    let offset = n * width;
    Some((
        Vec2D::from_glam(b.to_glam() + offset),
        Vec2D::from_glam(a.to_glam() + offset),
    ))
}

/// The corners of the rectangle centred at `center` with a corner at
/// `corner` and the next corner in the direction of `toward`: every corner
/// of a rectangle is as far from its centre as the others. Returned in
/// order round the rectangle, `corner` first. `None` when the two corners
/// would coincide or sit opposite each other.
pub fn rect_center3_corners(center: Vec2D, corner: Vec2D, toward: Vec2D) -> Option<[Vec2D; 4]> {
    let c = center.to_glam();
    let r = (corner.to_glam() - c).length();
    let dir = (toward.to_glam() - c).normalize_or_zero();
    if r < 1e-6 || dir == glam::DVec2::ZERO {
        return None;
    }
    let p1 = corner.to_glam();
    let p2 = c + dir * r;
    let (p3, p4) = (2.0 * c - p1, 2.0 * c - p2);
    if (p2 - p1).length() < 1e-4 * r || (p2 - p3).length() < 1e-4 * r {
        return None;
    }
    Some([p1, p2, p3, p4].map(Vec2D::from_glam))
}

/// The inner outline of a frame drawn corner `a` to corner `c`: the
/// rectangle `offset` inside it, corners in the same order as the outer
/// one's (`a`, `(c.x, a.y)`, `c`, `(a.x, c.y)`). `None` when the offset
/// is not a positive size or leaves nothing inside.
pub fn frame_inner_corners(a: Vec2D, c: Vec2D, offset: f64) -> Option<[Vec2D; 4]> {
    let (w, h) = ((c.x - a.x).abs(), (c.y - a.y).abs());
    if !(offset > 1e-6 && 2.0 * offset < w.min(h) - 1e-6) {
        return None;
    }
    let (sx, sy) = ((c.x - a.x).signum(), (c.y - a.y).signum());
    let ia = Vec2D::new(a.x + sx * offset, a.y + sy * offset);
    let ic = Vec2D::new(c.x - sx * offset, c.y - sy * offset);
    Some([ia, Vec2D::new(ic.x, ia.y), ic, Vec2D::new(ia.x, ic.y)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A proposal whose reused point is gone writes nothing; whole, it
    /// lands with its four edges round the corners it named.
    #[test]
    fn a_stale_proposal_is_refused_whole() {
        let mut sketch = Sketch::new("stale");
        let start = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(1.0, 1.0))));
        let state = ToolState::RectFrom {
            corner: SnapTarget::Existing(start),
        };
        let params = ToolParams::default();
        let proposal = propose(
            &state,
            &sketch,
            Vec2D::new(5.0, 4.0),
            0.1,
            Kind::Corners,
            &params,
        )
        .expect("a rectangle");
        let mut stale = sketch.clone();
        stale.geometry.retain(|g| g.id() != start);
        let before = serde_json::to_value(&stale).unwrap();
        assert_eq!(
            apply(&mut stale, &proposal),
            Err(Rejection::SourceGone(start))
        );
        assert_eq!(serde_json::to_value(&stale).unwrap(), before);
        let made = apply(&mut sketch, &proposal).unwrap();
        assert_eq!(made.corners[0], start);
        assert_eq!(sketch.constraints.len(), 4, "held level and plumb");
    }
}
