//! The line and polyline tools' runtime: a click resolves into where the
//! segment ends ([`resolve`]), that into the operation it proposes
//! ([`propose`]), which the host applies ([`apply`]) before the gesture
//! moves on ([`advance`]). The preview draws the same proposal the click
//! applies, so what is shown is what lands.
//!
//! The runtime reads the sketch and never writes it: [`apply`] is the one
//! function here that does, called by the click the viewport and the
//! `sketch.draw` command share. It checks every point and curve the
//! proposal names before its first write, so a proposal that has gone
//! stale is refused whole. Solving, storing and undo stay with the host;
//! [`Links`] keeps the undo step each committed segment closed, so taking
//! one back puts the gesture where it was before it.

use core_document::JournalEntryId;
use uuid::Uuid;

use crate::geom2d;
use crate::sketch::{Arc, ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::snap::{self, AxisSnap, Snap, SnapTarget};
use crate::tools::{self, PolySegment, ToolParams, ToolState};

/// Where a click of the line or polyline tool lands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Resolved {
    /// The first click: where the chain starts.
    Start(SnapTarget),
    /// A segment's end, from the point drawn from.
    End(End),
    /// The point the segment is drawn from is gone (an undo, a script).
    StartGone,
}

/// A segment's end as a click resolves it: an existing point other than
/// the start (its id reused), else a point on the nearest curve, else the
/// cursor kept level or plumb with the start where it nearly is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct End {
    pub from: SnapTarget,
    pub from_pos: Vec2D,
    pub end: SnapTarget,
    pub end_pos: Vec2D,
    /// The direction the segment keeps.
    pub axis: Option<AxisSnap>,
    /// What the click snapped to there, which the relations read.
    pub snapped: Snap,
    /// The reach the end and its holds are found again within.
    pub tol: f64,
}

/// The curve a proposal makes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Curve {
    Line,
    /// An arc leaving the start along the polyline's heading; a right turn
    /// (`ccw` false) is stored end to start.
    Arc {
        center: Vec2D,
        radius: f64,
        ccw: bool,
    },
}

/// A constraint a proposal adds to the curve it makes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Relation {
    Horizontal,
    Vertical,
    /// Square to a line the end landed on or follows.
    Perpendicular(Uuid),
    Parallel(Uuid),
    /// Touching a circle or arc the end landed on.
    Tangent(Uuid),
    /// The arc leaves the curve before it along it, or back along it.
    TangentToPrevious(Uuid),
    /// The arc leaves the curve before it square: its angle there,
    /// measured once it is made.
    AngleToPrevious(Uuid),
}

impl Relation {
    /// The other curve the relation needs.
    fn source(self) -> Option<Uuid> {
        match self {
            Relation::Horizontal | Relation::Vertical => None,
            Relation::Perpendicular(id)
            | Relation::Parallel(id)
            | Relation::Tangent(id)
            | Relation::TangentToPrevious(id)
            | Relation::AngleToPrevious(id) => Some(id),
        }
    }

    fn log(self) -> &'static str {
        match self {
            Relation::Horizontal => " [auto: horizontal]",
            Relation::Vertical => " [auto: vertical]",
            Relation::Perpendicular(_) => " [auto: perpendicular]",
            Relation::Parallel(_) => " [auto: parallel]",
            Relation::Tangent(_) => " [auto: tangent]",
            Relation::TangentToPrevious(_) | Relation::AngleToPrevious(_) => "",
        }
    }
}

/// The edit one segment proposes: what the host applies.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proposal {
    pub from: SnapTarget,
    pub end: SnapTarget,
    pub from_pos: Vec2D,
    pub end_pos: Vec2D,
    pub curve: Curve,
    /// In the order they are added, which the redundancy check follows.
    pub relations: Vec<Relation>,
    pub tol: f64,
    /// The segment ends on the polyline's first point and closes it.
    pub closes: bool,
}

/// Why a click proposes nothing, or a proposal was not applied. Either
/// leaves the sketch and the gesture as they were.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Rejection {
    /// The point drawn from is gone.
    StartGone,
    /// The end is where the start is.
    TooShort,
    /// A point or curve the proposal names is gone.
    SourceGone(Uuid),
}

impl Rejection {
    pub fn message(self) -> &'static str {
        match self {
            Rejection::StartGone => "The point the segment was drawn from is gone",
            Rejection::TooShort => "",
            Rejection::SourceGone(_) => "What the segment was drawn to is gone; nothing was added",
        }
    }
}

/// What [`apply`] made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Applied {
    pub curve: Uuid,
    pub start: Uuid,
    pub end: Uuid,
}

/// Whether `state` is a line or polyline gesture under way.
pub(crate) fn is_chain(state: &ToolState) -> bool {
    matches!(
        state,
        ToolState::LineFrom { .. } | ToolState::PolylineFrom { .. }
    )
}

/// Where a click at `cursor` lands in `state`, `tol` the reach it snaps
/// within: what the click applies and the preview shows.
pub(crate) fn resolve(
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    tol: f64,
    params: &ToolParams,
) -> Resolved {
    let (from, arc) = match *state {
        ToolState::LineFrom { from, .. } => (from, false),
        ToolState::PolylineFrom { from, segment, .. } => (from, segment != PolySegment::Line),
        _ => return Resolved::Start(tools::snap_point_or_curve(sketch, cursor, tol)),
    };
    let Some(from_pos) = from.position(sketch) else {
        return Resolved::StartGone;
    };
    let snapped = tools::snap_at(state, sketch, cursor, tol, params);
    let (axis_pos, axis) = if arc {
        (cursor, None)
    } else {
        snap::snap_axis(from_pos, cursor, tol)
    };
    let exclude = match from {
        SnapTarget::Existing(id) => vec![id],
        SnapTarget::New(_) => vec![],
    };
    let (end, axis) = match snap::snap_to_point(sketch, cursor, tol, &exclude) {
        SnapTarget::Existing(id) => (SnapTarget::Existing(id), axis),
        SnapTarget::New(_) => match snap::snap_to_curve(sketch, cursor, tol, &[]) {
            // On a curve the point is held there; it stays level or plumb
            // only where the snap carried that direction onto the curve.
            Some((_, proj)) => (
                SnapTarget::New(proj),
                axis.filter(|_| carries_axis(&snapped)),
            ),
            None => (SnapTarget::New(axis_pos), axis),
        },
    };
    Resolved::End(End {
        from,
        from_pos,
        end_pos: end.position(sketch).unwrap_or(axis_pos),
        end,
        axis,
        snapped,
        tol,
    })
}

/// Whether `snapped` is a level or plumb direction carried onto a curve:
/// the segment keeps that direction as well as landing on the curve.
fn carries_axis(snapped: &Snap) -> bool {
    snapped.on.is_some()
        && matches!(
            snapped.kind,
            Some(snap::SnapKind::Horizontal | snap::SnapKind::Vertical)
        )
}

/// The segment `end` proposes in `state`: its curve, the relations auto
/// constraints infer, and whether it closes the polyline.
pub(crate) fn propose(
    state: &ToolState,
    sketch: &Sketch,
    end: &End,
    params: &ToolParams,
) -> Result<Proposal, Rejection> {
    if (end.end_pos - end.from_pos).to_glam().length() < 1e-6 {
        return Err(Rejection::TooShort);
    }
    let auto = params.auto_constraints;
    let axis = end
        .axis
        .filter(|_| auto && matches!(end.end, SnapTarget::New(_)))
        .map(|axis| match axis {
            AxisSnap::Horizontal => Relation::Horizontal,
            AxisSnap::Vertical => Relation::Vertical,
        });
    let touching = auto
        .then(|| square_or_touching(sketch, &end.snapped))
        .flatten();
    let mut proposal = Proposal {
        from: end.from,
        end: end.end,
        from_pos: end.from_pos,
        end_pos: end.end_pos,
        curve: Curve::Line,
        relations: Vec::new(),
        tol: end.tol,
        closes: false,
    };
    match *state {
        ToolState::PolylineFrom {
            first,
            heading,
            prev,
            segment,
            ..
        } => {
            proposal.closes = matches!(end.end, SnapTarget::Existing(id) if Some(id) == first);
            let tangent = heading
                .and_then(|h| segment.arc_heading(h, end.end_pos - end.from_pos))
                .and_then(|h| geom2d::tangent_arc(end.from_pos, h, end.end_pos));
            match tangent {
                Some((center, radius, ccw)) => {
                    proposal.curve = Curve::Arc {
                        center,
                        radius,
                        ccw,
                    };
                    proposal.relations.extend(prev.map(|prev| {
                        if segment == PolySegment::Perpendicular {
                            Relation::AngleToPrevious(prev)
                        } else {
                            Relation::TangentToPrevious(prev)
                        }
                    }));
                }
                None => proposal.relations.extend(touching.into_iter().chain(axis)),
            }
        }
        _ => proposal.relations.extend(axis.into_iter().chain(touching)),
    }
    Ok(proposal)
}

/// A segment drawn to a perpendicular or tangent snap is held square to or
/// touching the curve it landed on; one drawn along a parallel or square
/// direction, parallel or square to the line it follows.
fn square_or_touching(sketch: &Sketch, snapped: &Snap) -> Option<Relation> {
    let curve = snapped.curve?;
    Some(match (snapped.kind?, sketch.get_geometry(curve)?) {
        (
            snap::SnapKind::Perpendicular | snap::SnapKind::PerpendicularTo,
            GeometryElement::Line(_),
        ) => Relation::Perpendicular(curve),
        (snap::SnapKind::Parallel, GeometryElement::Line(_)) => Relation::Parallel(curve),
        (snap::SnapKind::Tangent, GeometryElement::Circle(_) | GeometryElement::Arc(_)) => {
            Relation::Tangent(curve)
        }
        _ => return None,
    })
}

/// The points along the curve `proposal` makes, for the preview.
pub(crate) fn outline(proposal: &Proposal, segments: usize) -> Vec<Vec2D> {
    let (a, b) = (proposal.from_pos, proposal.end_pos);
    let Curve::Arc {
        center: c,
        radius,
        ccw,
    } = proposal.curve
    else {
        return vec![a, b];
    };
    let angle = |p: Vec2D| (p.y - c.y).atan2(p.x - c.x);
    let (t0, mut t1) = (angle(a), angle(b));
    if ccw {
        while t1 <= t0 {
            t1 += std::f64::consts::TAU;
        }
    } else {
        while t1 >= t0 {
            t1 -= std::f64::consts::TAU;
        }
    }
    (0..=segments)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / segments as f64;
            Vec2D::new(c.x + radius * t.cos(), c.y + radius * t.sin())
        })
        .collect()
}

/// Write `proposal` into `sketch`: its points (reused, or new and held
/// where they were snapped), its curve and its relations. Every point and
/// curve it names is checked first, so a refusal writes nothing.
pub(crate) fn apply(sketch: &mut Sketch, proposal: &Proposal) -> Result<Applied, Rejection> {
    let point = |target: SnapTarget| match target {
        SnapTarget::Existing(id) if sketch.point_position(id).is_none() => {
            Err(Rejection::SourceGone(id))
        }
        _ => Ok(()),
    };
    point(proposal.from)?;
    point(proposal.end)?;
    for id in proposal.relations.iter().filter_map(|r| r.source()) {
        if sketch.get_geometry(id).is_none() {
            return Err(Rejection::SourceGone(id));
        }
    }
    let start = tools::materialize_on_curve(sketch, proposal.from, proposal.tol);
    let end = tools::materialize_on_curve(sketch, proposal.end, proposal.tol);
    let curve = match proposal.curve {
        Curve::Line => sketch.add_geometry(GeometryElement::Line(Line::new(start, end))),
        Curve::Arc {
            center,
            radius,
            ccw,
        } => {
            let center = sketch.add_geometry(GeometryElement::Point(Point::new(center)));
            let (s, e) = if ccw { (start, end) } else { (end, start) };
            sketch.add_geometry(GeometryElement::Arc(Arc::new(center, s, e, radius)))
        }
    };
    for relation in &proposal.relations {
        let kind = match *relation {
            Relation::Horizontal => ConstraintKind::Horizontal { element: curve },
            Relation::Vertical => ConstraintKind::Vertical { element: curve },
            Relation::Perpendicular(other) => ConstraintKind::Perpendicular {
                line1: curve,
                line2: other,
            },
            Relation::Parallel(other) => ConstraintKind::Parallel {
                line1: curve,
                line2: other,
            },
            Relation::Tangent(other) => ConstraintKind::Tangent {
                line_or_circle1: curve,
                item2: other,
            },
            Relation::TangentToPrevious(prev) => ConstraintKind::Tangent {
                line_or_circle1: prev,
                item2: curve,
            },
            Relation::AngleToPrevious(prev) => {
                let probe = ConstraintKind::AngleAtPoint {
                    curve1: prev,
                    curve2: curve,
                    point: start,
                    angle_rad: 0.0,
                };
                let Some(degrees) = crate::sketch::measured_value(sketch, &probe) else {
                    continue;
                };
                crate::sketch::with_dimension_value(&probe, degrees)
            }
        };
        sketch.add_constraint(kind);
    }
    Ok(Applied { curve, start, end })
}

/// The gesture after `proposal` was applied as `applied`: on from the
/// segment's end, or at rest once a polyline closes.
pub(crate) fn advance(state: &mut ToolState, proposal: &Proposal, applied: &Applied) {
    let from = SnapTarget::Existing(applied.end);
    *state = match *state {
        ToolState::PolylineFrom { first, segment, .. } => {
            if proposal.closes {
                ToolState::Idle
            } else {
                let heading = match proposal.curve {
                    Curve::Arc { center, ccw, .. } => {
                        let radial = (proposal.end_pos - center)
                            .to_glam()
                            .perp()
                            .normalize_or_zero();
                        if ccw { radial } else { -radial }
                    }
                    Curve::Line => (proposal.end_pos - proposal.from_pos)
                        .to_glam()
                        .normalize_or_zero(),
                };
                ToolState::PolylineFrom {
                    from,
                    first: Some(first.unwrap_or(applied.start)),
                    heading: Some(Vec2D::from_glam(heading)),
                    prev: Some(applied.curve),
                    segment,
                }
            }
        }
        _ => ToolState::LineFrom { from, chain: true },
    };
}

/// One click of the line (`polyline` false) or polyline tool: resolve,
/// propose, apply, advance. A refusal changes nothing; a start that is
/// gone ends the gesture.
pub(crate) fn click(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    tol: f64,
    params: &ToolParams,
    polyline: bool,
) -> tools::ToolEffect {
    let end = match resolve(state, sketch, cursor, tol, params) {
        Resolved::Start(from) => {
            *state = if polyline {
                ToolState::PolylineFrom {
                    from,
                    first: None,
                    heading: None,
                    prev: None,
                    segment: PolySegment::Line,
                }
            } else {
                ToolState::LineFrom { from, chain: false }
            };
            return tools::ToolEffect::none();
        }
        Resolved::StartGone => {
            *state = ToolState::Idle;
            return tools::ToolEffect::log(Rejection::StartGone.message());
        }
        Resolved::End(end) => end,
    };
    let applied = propose(state, sketch, &end, params)
        .and_then(|proposal| Ok((apply(sketch, &proposal)?, proposal)));
    let (applied, proposal) = match applied {
        Ok(done) => done,
        Err(Rejection::TooShort) => return tools::ToolEffect::none(),
        Err(why) => return tools::ToolEffect::log(why.message()),
    };
    advance(state, &proposal, &applied);
    let mut log = match proposal.curve {
        Curve::Arc { radius, .. } => format!("Polyline arc r={radius:.2}"),
        Curve::Line if polyline => "Polyline segment".to_string(),
        Curve::Line => format!(
            "Line ({:.2}, {:.2}) → ({:.2}, {:.2})",
            proposal.from_pos.x, proposal.from_pos.y, proposal.end_pos.x, proposal.end_pos.y
        ),
    };
    if !polyline {
        log.extend(proposal.relations.iter().map(|r| r.log()));
    }
    if proposal.closes {
        log.push_str("; polyline closed");
    }
    tools::ToolEffect::changed(log)
}

/// A committed segment: the gesture before it, the curve it made, the
/// undo step its click closed and where the click is in the recording.
#[derive(Debug, Clone)]
pub(crate) struct Link {
    pub before: ToolState,
    pub curve: Uuid,
    pub entry: Option<JournalEntryId>,
    pub event: usize,
}

/// The segments a gesture has committed, newest last, so an undo of one
/// puts the gesture back where it was before it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Links(Vec<Link>);

impl Links {
    pub fn push(&mut self, link: Link) {
        self.0.push(link);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The host's answer to the newest segment's close.
    pub fn closed(&mut self, entry: Option<JournalEntryId>) {
        if let Some(link) = self.0.last_mut() {
            link.entry = entry;
        }
    }

    /// The undo step that takes back the newest segment, while it is the
    /// newest step the host has (`newest`).
    pub fn undoable(&self, newest: Option<JournalEntryId>) -> Option<JournalEntryId> {
        let entry = self.0.last()?.entry?;
        (Some(entry) == newest).then_some(entry)
    }

    /// Bring the gesture in `state` up to `sketch` after a change it did not
    /// make (an undo, a redo, a script): each segment the change took away
    /// puts the gesture back where it was before it; a gesture whose start
    /// or the curves it carries on from are gone with nothing to go back
    /// to ends. Answers where the taken-back clicks were in the recording,
    /// newest first.
    pub fn follow(&mut self, state: &mut ToolState, sketch: &Sketch) -> Vec<usize> {
        let mut taken = Vec::new();
        while let Some(link) = self.0.last()
            && sketch.get_geometry(link.curve).is_none()
        {
            let link = self.0.pop().expect("a newest link");
            taken.push(link.event);
            *state = link.before;
        }
        if !is_chain(state) {
            self.0.clear();
            return taken;
        }
        let live = |id: Uuid| sketch.get_geometry(id).is_some();
        let start_live = match *state {
            ToolState::LineFrom { from, .. } | ToolState::PolylineFrom { from, .. } => {
                from.position(sketch).is_some()
            }
            _ => true,
        };
        if !start_live {
            *state = ToolState::Idle;
            self.0.clear();
        } else if let ToolState::PolylineFrom {
            first,
            heading,
            prev,
            segment,
            ..
        } = state
        {
            if first.is_some_and(|id| !live(id)) {
                *first = None;
            }
            if prev.is_some_and(|id| !live(id)) {
                *prev = None;
                *heading = None;
                *segment = PolySegment::Line;
            }
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    /// A proposal whose start, end or relation's curve is gone writes
    /// nothing; whole, it lands with its relations.
    #[test]
    fn a_stale_proposal_is_refused_whole() {
        let mut sketch = Sketch::new("stale");
        let (a, b) = (point(&mut sketch, 0.0, 0.0), point(&mut sketch, 10.0, 0.0));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let start = point(&mut sketch, 0.0, 5.0);
        let proposal = Proposal {
            from: SnapTarget::Existing(start),
            end: SnapTarget::New(Vec2D::new(10.0, 5.0)),
            from_pos: Vec2D::new(0.0, 5.0),
            end_pos: Vec2D::new(10.0, 5.0),
            curve: Curve::Line,
            relations: vec![Relation::Horizontal, Relation::Parallel(line)],
            tol: 0.1,
            closes: false,
        };
        for gone in [start, line] {
            let mut stale = sketch.clone();
            stale.geometry.retain(|g| g.id() != gone);
            let before = serde_json::to_value(&stale).unwrap();
            assert_eq!(
                apply(&mut stale, &proposal),
                Err(Rejection::SourceGone(gone))
            );
            assert_eq!(serde_json::to_value(&stale).unwrap(), before);
        }
        let made = apply(&mut sketch, &proposal).unwrap();
        assert_eq!(made.start, start);
        let held =
            |want: fn(&ConstraintKind) -> bool| sketch.constraints.iter().any(|c| want(&c.kind));
        assert!(held(|k| matches!(k, ConstraintKind::Horizontal { .. })));
        assert!(held(|k| matches!(k, ConstraintKind::Parallel { .. })));
    }
}
