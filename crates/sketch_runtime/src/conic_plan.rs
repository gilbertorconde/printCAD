//! Shared conic construction from a sequence of selected picks.
//!
//! A curve is a function of picks. Preview and application consume the same
//! result, including explicit rejection. Bindings map result point roles to
//! input pick indices only where their positions coincide.
//!
//! The tool owns gesture phases, pixel thresholds and click acceptance.
//! The host owns application, gesture boundaries and undo.
//!

use sketch_geometry::curves::{
    ellipse_angle, ellipse_axes, ellipse_point, ellipse_semi_minor, hyperbola_axes,
    hyperbola_branch, hyperbola_param, hyperbola_point, hyperbola_semi_minor, parabola_frame,
    parabola_param, parabola_point,
};
use sketch_geometry::primitives::{circumcircle, compute_ellipse_focus, ellipse_other_radius};
use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPoint, WorldVec};

/// Construction mode, fixed by the active tool throughout a gesture.
///
/// Every mode uses the same collect-N-picks gesture; only arity, roles and
/// result geometry differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConicKind {
    /// Arc: start, end, then a point on the arc.
    Arc3Pt,
    /// Arc: center, start, then end projected onto the radius.
    ArcFromCenter,
    /// Circle through three points.
    CircleFrom3Pts,
    /// Ellipse: center, first semiaxis, then distance to its line.
    FullEllipse,
    /// Ellipse: two axis endpoints and a rim point.
    EllipseFrom3Pts,
    /// Elliptic arc: center, major-axis point, start, end.
    ArcOfEllipse,
    /// Hyperbolic arc: center, focus, start, end.
    ArcOfHyperbola,
    /// Parabolic arc: vertex, focus, start, end.
    ArcOfParabola,
}

/// Meaning of the second hyperbola pick. Both inputs use the same gesture and
/// focal curve plan; a derived focus never inherits a vertex pick's bindings.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HyperbolaInput {
    #[default]
    Focus,
    BranchVertex,
}

impl ConicKind {
    /// Number of picks required to determine the curve.
    ///
    /// Click number `arity()` completes the gesture.
    pub fn arity(self) -> usize {
        match self {
            ConicKind::Arc3Pt
            | ConicKind::ArcFromCenter
            | ConicKind::CircleFrom3Pts
            | ConicKind::FullEllipse
            | ConicKind::EllipseFrom3Pts => 3,
            ConicKind::ArcOfEllipse | ConicKind::ArcOfHyperbola | ConicKind::ArcOfParabola => 4,
        }
    }
}

/// Result point role addressed by a pick binding.
///
/// Anchor denotes a circle/conic center or a parabola vertex: the initial
/// defining point, without requiring consumers to distinguish the curve kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConicPointRole {
    Anchor,
    Focus,
    Start,
    End,
}

/// The point with role `role` is exactly at input pick `pick`.
///
/// Only unchanged pick positions may create coincidence bindings. Projected
/// endpoints and inferred foci differ from their input picks; binding them
/// would make a solver deform the constructed curve to enforce a false equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConicBinding {
    pub role: ConicPointRole,
    /// Index in the pick sequence passed to [`plan_conic`].
    pub pick: usize,
}

/// Resolved geometry shared by preview and host application.
///
/// Semiaxes and endpoint parameters are derived from these points by the
/// shared curve mathematics, rather than stored as duplicate state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConicGeometry {
    Circle {
        center: WorldPoint,
        radius: WorldLength,
    },
    Arc {
        center: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        clockwise: bool,
    },
    Ellipse {
        center: WorldPoint,
        focus1: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        clockwise: bool,
    },
    Hyperbola {
        center: WorldPoint,
        focus1: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        clockwise: bool,
    },
    Parabola {
        vertex: WorldPoint,
        focus: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        clockwise: bool,
    },
}

impl ConicGeometry {
    /// Exact circular arc angles for host storage and preview.
    ///
    /// Recomputing a span from clockwise in a renderer could disagree at a full
    /// turn; consume the sweep already resolved by construction.
    ///
    /// Calling this on a noncircular conic is an API error: its parameter has
    /// a different meaning from a circular angle.
    pub fn arc_angles(&self) -> (Angle, Angle) {
        let ConicGeometry::Arc {
            center,
            start,
            end,
            clockwise,
        } = *self
        else {
            panic!("arc_angles() requires a circular arc, got {self:?}");
        };
        let start_angle = Angle::from_vec(start - center);
        let sweep = start_angle.sweep_to(Angle::from_vec(end - center), clockwise);
        (start_angle, Angle(start_angle.radians() + sweep.radians()))
    }
}

/// Construction result: geometry to create and picks to turn into bindings.
///
/// Apply the plan as resolved; recomputing at application would allow the
/// accepted result to diverge from the shown preview.
#[derive(Debug, Clone, PartialEq)]
pub struct ConicPlan {
    pub kind: ConicKind,
    pub geometry: ConicGeometry,
    /// Role-to-pick pairs for unchanged positions. An empty list is valid:
    /// a three-point circle stores none of its construction points.
    pub bindings: Vec<ConicBinding>,
}

/// Explicit reason construction failed.
///
/// Rejection is a result value that the tool can display while retaining input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConicRejection {
    /// Collinear points cannot define a circle.
    Collinear,
    /// Coincident center and rim leave no radius.
    ZeroRadius,
    /// A cursor at the center leaves the end direction undefined.
    ///
    CursorAtCenter,
    /// Coincident focus/axis point and anchor leave no curve frame.
    NoAxis,
    /// Zero semiaxis: ellipse degenerates to a segment, hyperbola to a ray.
    DegenerateConic,
    /// Coincident ends on a noncyclic parameter leave an empty arc.
    ///
    /// Equal elliptic parameters instead denote a valid full turn.
    EmptySpan,
}

impl ConicRejection {
    /// Human-readable reason for rejection previews and diagnostic assertions.
    pub fn reason(self) -> &'static str {
        match self {
            ConicRejection::Collinear => "three points are collinear",
            ConicRejection::ZeroRadius => "centre coincides with a curve point",
            ConicRejection::CursorAtCenter => "cursor at centre: direction is undefined",
            ConicRejection::NoAxis => "focus coincides with anchor: curve axes are undefined",
            ConicRejection::DegenerateConic => "zero semi-axis: degenerate curve",
            ConicRejection::EmptySpan => "coincident endpoints: empty arc",
        }
    }
}

/// Either a constructed curve or an explicit rejection.
#[derive(Debug, Clone, PartialEq)]
pub enum ConicOutcome {
    Built(ConicPlan),
    Rejected(ConicRejection),
}

impl ConicOutcome {
    /// Plan when present, for callers that handle rejection separately.
    pub fn plan(&self) -> Option<&ConicPlan> {
        match self {
            ConicOutcome::Built(plan) => Some(plan),
            ConicOutcome::Rejected(_) => None,
        }
    }
}

// WorldLength::DEGENERATE bounds geometric coincidence, not click proximity.
// Tools judge nearby input in pixels; construction rejects unstable division.

/// Construct a curve from selected positions.
///
/// Requires `picks.len() == kind.arity()` and finite positions. Violations are
/// caller errors; tools supply exactly their collected picks.
///
/// Reject only geometric degeneracy. The tool separately judges gestures too
/// small on screen; construction has no view scale.
pub fn plan_conic(kind: ConicKind, picks: &[WorldPoint]) -> ConicOutcome {
    plan_conic_with_hyperbola_input(kind, picks, HyperbolaInput::Focus)
}

pub fn plan_conic_with_hyperbola_input(
    kind: ConicKind,
    picks: &[WorldPoint],
    input: HyperbolaInput,
) -> ConicOutcome {
    assert_eq!(
        picks.len(),
        kind.arity(),
        "plan_conic: {kind:?} requires {} picks, got {}",
        kind.arity(),
        picks.len()
    );
    // WorldPoint::new guarantees finite positions.

    match kind {
        ConicKind::Arc3Pt => plan_arc_3pt(picks),
        ConicKind::ArcFromCenter => plan_arc_from_center(picks),
        ConicKind::CircleFrom3Pts => plan_circle_3pts(picks),
        ConicKind::FullEllipse => plan_full_ellipse(picks),
        ConicKind::EllipseFrom3Pts => plan_ellipse_3pts(picks),
        ConicKind::ArcOfEllipse => plan_arc_of_ellipse(picks),
        ConicKind::ArcOfHyperbola => {
            if input == HyperbolaInput::BranchVertex {
                plan_hyperbola_from_vertex(picks)
            } else {
                plan_arc_of_hyperbola(picks)
            }
        }
        ConicKind::ArcOfParabola => plan_arc_of_parabola(picks),
    }
}

fn plan_hyperbola_from_vertex(picks: &[WorldPoint]) -> ConicOutcome {
    let (center, vertex, start) = (picks[0], picks[1], picks[2]);
    let axis = vertex - center;
    let a = axis.length().0;
    if a < WorldLength::DEGENERATE.0 {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }
    let u = axis.0 / a;
    let d = (start - center).0;
    let (x, y) = (d.dot(u), d.dot(u.perp()));
    let s = (x / a).powi(2) - 1.;
    if x <= a || s <= 0. || y.abs() < WorldLength::DEGENERATE.0 {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    }
    let b = y.abs() / s.sqrt();
    let c = a.hypot(b);
    if !c.is_finite() {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    }
    let focus = center + WorldVec::from_dvec2(u * c);
    let mut focal = picks.to_vec();
    focal[1] = focus;
    let mut outcome = plan_arc_of_hyperbola(&focal);
    if let ConicOutcome::Built(plan) = &mut outcome {
        plan.bindings
            .retain(|binding| binding.role != ConicPointRole::Focus);
    }
    outcome
}

/// Arc through start, end and an intermediate point on the arc.
///
/// The intermediate point determines the carrier and travel side but is not stored.
fn plan_arc_3pt(picks: &[WorldPoint]) -> ConicOutcome {
    let (start, end, on_arc) = (picks[0], picks[1], picks[2]);
    let Some((center, _radius)) = circumcircle(start, end, on_arc) else {
        return ConicOutcome::Rejected(ConicRejection::Collinear);
    };
    let clockwise = cyclic_direction(
        Angle::from_vec(start - center),
        Angle::from_vec(end - center),
        Angle::from_vec(on_arc - center),
    );
    ConicOutcome::Built(ConicPlan {
        kind: ConicKind::Arc3Pt,
        geometry: ConicGeometry::Arc {
            center,
            start,
            end,
            clockwise,
        },
        bindings: vec![
            ConicBinding {
                role: ConicPointRole::Start,
                pick: 0,
            },
            ConicBinding {
                role: ConicPointRole::End,
                pick: 1,
            },
        ],
    })
}

/// Arc from center, start and end.
///
/// Project the end onto the start's radius; its changed position creates no
/// binding (see [`ConicBinding`]).
fn plan_arc_from_center(picks: &[WorldPoint]) -> ConicOutcome {
    let (center, start, cursor) = (picks[0], picks[1], picks[2]);
    let radius = center.distance(start);
    if radius < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::ZeroRadius);
    }
    let dir = cursor - center;
    if dir.length() < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::CursorAtCenter);
    }
    let end = center + WorldVec::from_dvec2(dir.0.normalize() * radius.0);
    ConicOutcome::Built(ConicPlan {
        kind: ConicKind::ArcFromCenter,
        geometry: ConicGeometry::Arc {
            center,
            start,
            end,
            clockwise: DIRECTION_WHEN_CURSOR_IS_END,
        },
        bindings: vec![
            ConicBinding {
                role: ConicPointRole::Anchor,
                pick: 0,
            },
            ConicBinding {
                role: ConicPointRole::Start,
                pick: 1,
            },
        ],
    })
}

/// Circle through three points.
///
/// Only the inferred center is stored; construction points create no bindings.
fn plan_circle_3pts(picks: &[WorldPoint]) -> ConicOutcome {
    let Some((center, radius)) = circumcircle(picks[0], picks[1], picks[2]) else {
        return ConicOutcome::Rejected(ConicRejection::Collinear);
    };
    if radius < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::ZeroRadius);
    }
    ConicOutcome::Built(ConicPlan {
        kind: ConicKind::CircleFrom3Pts,
        geometry: ConicGeometry::Circle { center, radius },
        bindings: Vec::new(),
    })
}

/// Elliptic arc from center, major-axis point, start and end.
///
/// Infer the focus from the first three points. The axis point is not stored
/// and creates no binding.
fn plan_arc_of_ellipse(picks: &[WorldPoint]) -> ConicOutcome {
    let (center, axis_pt, start, cursor) = (picks[0], picks[1], picks[2], picks[3]);
    if center.distance(axis_pt) < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }
    let Some(focus1) = compute_ellipse_focus(center, axis_pt, start) else {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    };
    let semi_minor = ellipse_semi_minor(center, focus1, start);
    if semi_minor < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    }
    // Coincident focus and center leave a circle with no major-axis frame.
    if ellipse_axes(center, focus1, semi_minor).is_none() {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }

    // Project using the same evaluator as rendering, at the cursor's conic parameter.
    let t_cursor = ellipse_angle(center, focus1, semi_minor, cursor);
    let projected = ellipse_point(center, focus1, semi_minor, t_cursor);
    // A return to the same ray denotes a full turn. Projection roundoff must
    // not turn it into an arbitrarily short arc; retain the exact start only
    // within the f64 error scale of this frame, far below a user-sized span.
    let magnitude =
        center.0.abs().max_element() + center.distance(axis_pt).0 + center.distance(start).0;
    let end = if projected.distance(start).0 <= 32. * f64::EPSILON * magnitude {
        start
    } else {
        projected
    };

    ConicOutcome::Built(ConicPlan {
        kind: ConicKind::ArcOfEllipse,
        geometry: ConicGeometry::Ellipse {
            center,
            focus1,
            start,
            end,
            clockwise: DIRECTION_WHEN_CURSOR_IS_END,
        },
        bindings: vec![
            ConicBinding {
                role: ConicPointRole::Anchor,
                pick: 0,
            },
            ConicBinding {
                role: ConicPointRole::Start,
                pick: 2,
            },
        ],
    })
}

/// The third pick sets perpendicular distance, not a rim point. Reuse the
/// conic focus constructor after ordering the axes; never bind this projected
/// point (or the derived focus) to the cursor's snap target.
fn plan_full_ellipse(picks: &[WorldPoint]) -> ConicOutcome {
    let (center, axis, cursor) = (picks[0], picks[1], picks[2]);
    let first = axis - center;
    let first_radius = first.length();
    if first_radius < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }
    let direction = first.0 / first_radius.0;
    let second_radius = direction.perp_dot((cursor - center).0).abs();
    closed_ellipse(
        ConicKind::FullEllipse,
        center,
        first,
        second_radius,
        vec![ConicBinding {
            role: ConicPointRole::Anchor,
            pick: 0,
        }],
    )
}

/// The endpoints define the centre and first semiaxis; the rim defines the
/// perpendicular radius. All stored points are derived, so none inherits a
/// cursor source, even when a computed point happens to coincide with a pick.
fn plan_ellipse_3pts(picks: &[WorldPoint]) -> ConicOutcome {
    let center = picks[0] + (picks[1] - picks[0]) * 0.5;
    let first = picks[1] - center;
    if first.length() < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }
    let Some(second) = ellipse_other_radius(center, picks[1], picks[2]) else {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    };
    closed_ellipse(
        ConicKind::EllipseFrom3Pts,
        center,
        first,
        second.0,
        Vec::new(),
    )
}

fn closed_ellipse(
    kind: ConicKind,
    center: WorldPoint,
    first: WorldVec,
    second_radius: f64,
    bindings: Vec<ConicBinding>,
) -> ConicOutcome {
    let first_radius = first.length();
    let direction = first.0 / first_radius.0;
    if second_radius < WorldLength::DEGENERATE.0 {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    }
    // A circle has no focal direction in this conic representation. Represent equal
    // radii as the existing circle, including equality to floating point accuracy.
    if (first_radius.0 - second_radius).abs()
        <= 32.0 * f64::EPSILON * first_radius.0.max(second_radius)
    {
        return ConicOutcome::Built(ConicPlan {
            kind,
            geometry: ConicGeometry::Circle {
                center,
                radius: first_radius,
            },
            bindings,
        });
    }
    let (major, minor) = if first_radius.0 > second_radius {
        (
            first,
            WorldVec::from_dvec2(direction.perp() * second_radius),
        )
    } else {
        (
            WorldVec::from_dvec2(direction.perp() * second_radius),
            first,
        )
    };
    let focus1 = compute_ellipse_focus(center, center + major, center + minor)
        .expect("ordered nonzero unequal ellipse axes define a focus");
    if ellipse_axes(center, focus1, minor.length()).is_none() {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }
    let start = center + major;
    // The focal store must be able to recover the second radius from its seam.
    // Extremely thin axes can round the focus onto the vertex; storing that
    // result would turn a nonzero input into a degenerate, unrenderable curve.
    if ellipse_semi_minor(center, focus1, start) < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    }
    ConicOutcome::Built(ConicPlan {
        kind,
        geometry: ConicGeometry::Ellipse {
            center,
            focus1,
            start,
            end: start,
            clockwise: false,
        },
        bindings,
    })
}

/// Hyperbolic arc from center, focus, start and end.
///
/// The unchanged start determines a semiaxis; project the end onto its branch.
fn plan_arc_of_hyperbola(picks: &[WorldPoint]) -> ConicOutcome {
    let (center, focus1, start, cursor) = (picks[0], picks[1], picks[2], picks[3]);
    if center.distance(focus1) < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }
    let semi_minor = hyperbola_semi_minor(center, focus1, start);
    if semi_minor < WorldLength::DEGENERATE {
        return ConicOutcome::Rejected(ConicRejection::DegenerateConic);
    }
    if hyperbola_axes(center, focus1, semi_minor).is_none() {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }

    // The start selects one connected branch; the end must stay on that branch.
    let Some(branch) = hyperbola_branch(center, focus1, semi_minor, start) else {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    };
    let t_start = hyperbola_param(center, focus1, semi_minor, start);
    let t_end = hyperbola_param(center, focus1, semi_minor, cursor);
    if (t_end.get() - t_start.get()).abs() < WorldLength::DEGENERATE.0 {
        return ConicOutcome::Rejected(ConicRejection::EmptySpan);
    }
    let end = hyperbola_point(center, focus1, semi_minor, t_end, branch);

    ConicOutcome::Built(ConicPlan {
        kind: ConicKind::ArcOfHyperbola,
        geometry: ConicGeometry::Hyperbola {
            center,
            focus1,
            start,
            end,
            clockwise: monotone_direction(t_start.get(), t_end.get()),
        },
        bindings: vec![
            ConicBinding {
                role: ConicPointRole::Anchor,
                pick: 0,
            },
            ConicBinding {
                role: ConicPointRole::Focus,
                pick: 1,
            },
            ConicBinding {
                role: ConicPointRole::Start,
                pick: 2,
            },
        ],
    })
}

/// Parabolic arc from vertex, focus, start and end.
///
/// Vertex and focus define the carrier. Project both endpoints to select a
/// span; only the first two picks create bindings.
fn plan_arc_of_parabola(picks: &[WorldPoint]) -> ConicOutcome {
    let (vertex, focus, start_raw, cursor) = (picks[0], picks[1], picks[2], picks[3]);
    if vertex.distance(focus) < WorldLength::DEGENERATE || parabola_frame(vertex, focus).is_none() {
        return ConicOutcome::Rejected(ConicRejection::NoAxis);
    }

    let u_start = parabola_param(vertex, focus, start_raw);
    let u_end = parabola_param(vertex, focus, cursor);
    if (u_end.get() - u_start.get()).abs() < WorldLength::DEGENERATE.0 {
        return ConicOutcome::Rejected(ConicRejection::EmptySpan);
    }
    let start = parabola_point(vertex, focus, u_start);
    let end = parabola_point(vertex, focus, u_end);

    ConicOutcome::Built(ConicPlan {
        kind: ConicKind::ArcOfParabola,
        geometry: ConicGeometry::Parabola {
            vertex,
            focus,
            start,
            end,
            clockwise: monotone_direction(u_start.get(), u_end.get()),
        },
        bindings: vec![
            ConicBinding {
                role: ConicPointRole::Anchor,
                pick: 0,
            },
            ConicBinding {
                role: ConicPointRole::Focus,
                pick: 1,
            },
        ],
    })
}

/// Cyclic travel direction when the endpoint is distinct from the cursor.
///
/// Choose the route containing the cursor. The three-point arc uses its third
/// pick as an intermediate point, rather than an endpoint.
///
fn cyclic_direction(t_start: Angle, t_end: Angle, t_cursor: Angle) -> bool {
    Angle::cursor_wants_clockwise(t_start, t_end, t_cursor)
}

/// Counterclockwise travel for center-defined circular and elliptic arcs
/// whose endpoint is projected from the cursor.
///
/// The cursor is at the endpoint parameter, on the boundary of both routes,
/// so its instantaneous position cannot select a travel side.
///
///
/// Selecting a different side would require gesture history, not this pure plan.
const DIRECTION_WHEN_CURSOR_IS_END: bool = false;

/// Travel direction on a monotone parameter (hyperbola or parabola).
///
/// A connected branch has exactly one route between the ends; parameter
/// ordering determines its direction independently of the cursor.
///
/// Hosts may retain this flag for storage and traces even when evaluation
/// already follows the endpoint parameter order.
fn monotone_direction(param_start: f64, param_end: f64) -> bool {
    param_end < param_start
}
