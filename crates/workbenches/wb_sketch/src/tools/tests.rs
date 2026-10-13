use std::collections::HashSet;

use super::*;
use crate::sketch::{Arc, Circle, ConstraintKind, Line};
use crate::test_kinds::*;

/// A cursor given as a point or as `(x, y)`.
trait At {
    fn at(self) -> Vec2D;
}
impl At for Vec2D {
    fn at(self) -> Vec2D {
        self
    }
}
impl At for (f64, f64) {
    fn at(self) -> Vec2D {
        Vec2D::new(self.0, self.1)
    }
}

/// Click `tool` with default params and an empty selection (most tools
/// consume neither).
fn click(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: impl At,
    snap_tol: f64,
) -> ToolEffect {
    With::new(&ToolParams::default(), &HashSet::new(), snap_tol).click(state, sketch, tool, cursor)
}

/// The default parameters as `f` changes them.
fn params(f: impl FnOnce(&mut ToolParams)) -> ToolParams {
    let mut p = ToolParams::default();
    f(&mut p);
    p
}

/// What a test's clicks carry besides where they land: the tool's
/// parameters, the selection and the snap tolerance.
struct With {
    params: ToolParams,
    selected: HashSet<Uuid>,
    tol: f64,
}

impl With {
    fn new(params: &ToolParams, selected: &HashSet<Uuid>, tol: f64) -> Self {
        Self {
            params: *params,
            selected: selected.clone(),
            tol,
        }
    }

    fn click(
        &self,
        state: &mut ToolState,
        sketch: &mut Sketch,
        tool: &str,
        cursor: impl At,
    ) -> ToolEffect {
        let at = cursor.at();
        super::handle_click(
            state,
            tool,
            sketch,
            at,
            self.tol,
            &self.params,
            &self.selected,
        )
    }

    /// Every cursor in turn; the last click's effect.
    fn clicks<C: At>(
        &self,
        state: &mut ToolState,
        sketch: &mut Sketch,
        tool: &str,
        cursors: impl IntoIterator<Item = C>,
    ) -> ToolEffect {
        let mut last = None;
        for at in cursors {
            last = Some(self.click(state, sketch, tool, at));
        }
        last.expect("at least one click")
    }
}

/// An empty sketch and a tool at rest.
fn fresh() -> (Sketch, ToolState) {
    (Sketch::new("t"), ToolState::default())
}

fn count_kind(sketch: &Sketch, f: impl Fn(&GeometryElement) -> bool) -> usize {
    sketch.geometry.iter().filter(|g| f(g)).count()
}
fn points(s: &Sketch) -> usize {
    count_kind(s, |g| matches!(g, GeometryElement::Point(_)))
}
fn lines(s: &Sketch) -> usize {
    count_kind(s, |g| matches!(g, GeometryElement::Line(_)))
}
fn arcs(s: &Sketch) -> usize {
    count_kind(s, |g| matches!(g, GeometryElement::Arc(_)))
}
fn circles(s: &Sketch) -> usize {
    count_kind(s, |g| matches!(g, GeometryElement::Circle(_)))
}

/// Whether a point of the sketch stands within 1e-4 of `(x, y)`.
fn has_point(sketch: &Sketch, x: f64, y: f64) -> bool {
    sketch.geometry.iter().any(|g| match g {
        GeometryElement::Point(p) => (p.position - Vec2D::new(x, y)).to_glam().length() < 1e-4,
        _ => false,
    })
}

/// Fix a point where it stands.
fn pin(sketch: &mut Sketch, point: Uuid) {
    let position = sketch.point_position(point).unwrap();
    sketch.add_constraint(ConstraintKind::FixedPoint { point, position });
}

fn pt(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
    sketch.add_geometry(GeometryElement::Point(crate::sketch::Point::new(
        Vec2D::new(x, y),
    )))
}
fn line_between(sketch: &mut Sketch, a: Uuid, b: Uuid) -> Uuid {
    sketch.add_geometry(GeometryElement::Line(Line::new(a, b)))
}

/// Number of curve references per point id (2 everywhere = closed loop).
fn point_use_counts(sketch: &Sketch) -> std::collections::HashMap<Uuid, usize> {
    let mut uses = std::collections::HashMap::new();
    for g in &sketch.geometry {
        for pid in Sketch::curve_point_ids(g) {
            *uses.entry(pid).or_default() += 1;
        }
    }
    uses
}

#[test]
fn line_snaps_end_to_existing_point() {
    let (mut sketch, mut state) = fresh();
    let existing = pt(&mut sketch, 10.0, 0.0);
    click(&mut state, "sketch.line", &mut sketch, (0.0, 0.0), 0.5);
    click(&mut state, "sketch.line", &mut sketch, (10.2, 0.1), 0.5);
    let line = line_of(&sketch);
    assert_eq!(line.end, existing, "end point reused, not duplicated");
    assert_eq!(points(&sketch), 2);
}

#[test]
fn a_rectangle_freed_of_the_axes_turns_as_a_whole() {
    let (mut sketch, mut state) = fresh();
    for at in [Vec2D::new(2.0, 1.0), Vec2D::new(12.0, 6.0)] {
        click(&mut state, "sketch.rect", &mut sketch, at, 0.5);
    }
    let all: HashSet<Uuid> = sketch.geometry.iter().map(GeometryElement::id).collect();
    assert_eq!(remove_axis_alignment(&mut sketch, &all), 4);
    let kinds: Vec<&str> = sketch
        .constraints
        .iter()
        .map(|c| match c.kind {
            ConstraintKind::Parallel { .. } => "parallel",
            ConstraintKind::Perpendicular { .. } => "perpendicular",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds.len(),
        3,
        "the first line's own constraint goes: {:?}",
        sketch.constraints
    );
    assert!(kinds.contains(&"parallel") && kinds.contains(&"perpendicular"));
    assert!(!kinds.contains(&"other"));
    // A corner held off the axes: the shape turns and stays a rectangle.
    let corner = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Point(p) if p.position.x > 11.0 && p.position.y > 5.0 => Some(p.id),
            _ => None,
        })
        .unwrap();
    let origin = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Point(p) if p.position.x < 3.0 && p.position.y < 2.0 => Some(p.id),
            _ => None,
        })
        .unwrap();
    pin(&mut sketch, origin);
    if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(corner) {
        p.position = Vec2D::new(8.0, 8.0);
    }
    crate::solver::solve_holding(&mut sketch, &[corner]);
    for g in &sketch.geometry {
        if let GeometryElement::Line(a) = g {
            for h in &sketch.geometry {
                if let GeometryElement::Line(b) = h {
                    let d = |l: &Line| {
                        (sketch.point_position(l.end).unwrap().to_glam()
                            - sketch.point_position(l.start).unwrap().to_glam())
                        .normalize()
                    };
                    let (da, db) = (d(a), d(b));
                    let square = da.dot(db).abs() < 1e-3;
                    let along = da.perp_dot(db).abs() < 1e-3;
                    assert!(square || along, "the corners stay square");
                }
            }
        }
    }
    let at = sketch.point_position(corner).unwrap();
    assert!((at.x - 8.0).abs() < 1e-3 && (at.y - 8.0).abs() < 1e-3);
    // Nothing on the axes is left to hold it level.
    assert!(sketch.constraints.iter().all(|c| !matches!(
        c.kind,
        ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. }
    )));
}

/// A shape a tool cannot make draws nothing, and where the reason is not
/// plain from the clicks, says why.
#[test]
fn degenerate_shapes_draw_nothing() {
    let plain = ToolParams::default();
    // Half the width, 6, is more than the centreline's radius, 5.
    let wide_slot = params(|p| p.slot_width = 12.0);
    let thick_wall = params(|p| p.offset_distance = 3.0);
    type Case<'a> = (&'a str, &'a str, &'a ToolParams, &'a [(f64, f64)], bool);
    let cases: [Case; 7] = [
        (
            "a rectangle with no width",
            "sketch.rect",
            &plain,
            &[(0.0, 0.0), (0.0, 5.0)],
            false,
        ),
        (
            "a circle through three collinear points",
            "sketch.circle3",
            &plain,
            &[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)],
            false,
        ),
        (
            "a polygon whose vertex is its centre",
            "sketch.polygon",
            &plain,
            &[(1.0, 1.0), (1.0, 1.0)],
            false,
        ),
        (
            "a slot with no length",
            "sketch.slot",
            &plain,
            &[(3.0, 3.0), (3.0, 3.0)],
            false,
        ),
        (
            "an arc slot wider than its radius",
            "sketch.arc_slot",
            &wide_slot,
            &[(0.0, 0.0), (5.0, 0.0), (0.0, 5.0)],
            false,
        ),
        (
            "a three-point rectangle whose third click is on its first edge's line",
            "sketch.rect3",
            &plain,
            &[(1.0, 1.0), (5.0, 1.0), (9.0, 1.0)],
            false,
        ),
        (
            "a frame too thin for its wall",
            "sketch.rect_frame",
            &thick_wall,
            &[(1.0, 1.0), (11.0, 6.0)],
            true,
        ),
    ];
    for (what, tool, params, clicks, says_why) in cases {
        let (mut sketch, mut state) = fresh();
        let w = With::new(params, &HashSet::new(), 0.1);
        let fx = w.clicks(&mut state, &mut sketch, tool, clicks.iter().copied());
        assert!(!fx.changed, "{what}");
        assert!(sketch.geometry.is_empty(), "{what}");
        assert_eq!(fx.log.is_some(), says_why, "{what}: whether it says why");
    }
}

#[test]
fn arc_three_clicks_end_projected_to_radius() {
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.arc", &mut sketch, (0.0, 0.0), 0.1);
    click(&mut state, "sketch.arc", &mut sketch, (5.0, 0.0), 0.1);
    click(&mut state, "sketch.arc", &mut sketch, (0.0, 7.0), 0.1);
    let arc = arc_of(&sketch);
    assert!((arc.radius - 5.0).abs() < 1e-5);
    let end = sketch.point_position(arc.end).unwrap();
    assert!(
        (end.to_glam().length() - 5.0).abs() < 1e-4,
        "end lies on the arc"
    );
    assert!(state.is_idle());
}

#[test]
fn arc3_stores_ccw_arc_through_rim_point() {
    let (mut sketch, mut state) = fresh();
    // Endpoints (5,0) and (0,5); rim point on the short CCW side.
    click(&mut state, "sketch.arc3", &mut sketch, (5.0, 0.0), 0.1);
    click(&mut state, "sketch.arc3", &mut sketch, (0.0, 5.0), 0.1);
    let fx = click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        (3.5355, 3.5355),
        0.1,
    );
    assert!(fx.changed);
    let arc = arc_of(&sketch);
    let c = sketch.point_position(arc.center).unwrap().to_glam();
    let s = sketch.point_position(arc.start).unwrap().to_glam();
    let e = sketch.point_position(arc.end).unwrap().to_glam();
    assert!(c.length() < 1e-2, "center near origin: {c:?}");
    assert!((arc.radius - 5.0).abs() < 1e-3);
    // Rim point inside the stored CCW sweep.
    assert!(crate::geom2d::point_on_arc(
        c,
        s,
        e,
        glam::DVec2::new(3.5355, 3.5355)
    ));
    let (_, sweep) = crate::snap::arc_angles(s - c, e - c);
    assert!(
        sweep < std::f64::consts::PI,
        "short side chosen, sweep {sweep}"
    );
}

#[test]
fn arc3_swaps_endpoints_for_clockwise_rim_point() {
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.arc3", &mut sketch, (5.0, 0.0), 0.1);
    click(&mut state, "sketch.arc3", &mut sketch, (0.0, 5.0), 0.1);
    // Rim point on the far (clockwise) side: endpoints must swap so the
    // stored CCW sweep passes through it.
    click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        (-3.5355, -3.5355),
        0.1,
    );
    let arc = arc_of(&sketch);
    let c = sketch.point_position(arc.center).unwrap().to_glam();
    let s = sketch.point_position(arc.start).unwrap().to_glam();
    let e = sketch.point_position(arc.end).unwrap().to_glam();
    assert!(crate::geom2d::point_on_arc(
        c,
        s,
        e,
        glam::DVec2::new(-3.5355, -3.5355)
    ));
    assert!((s - glam::DVec2::new(0.0, 5.0)).length() < 1e-3, "swapped");
}

#[test]
fn bspline_with_too_few_points_cancels() {
    let (mut sketch, mut state) = fresh();
    for (x, y) in [(0.0, 0.0), (4.0, 6.0)] {
        click(&mut state, "sketch.bspline", &mut sketch, (x, y), 0.1);
    }
    let fx = super::finish_click_sequence(&mut state, &mut sketch, &ToolParams::default());
    assert!(!fx.changed);
    assert!(state.is_idle());
    assert!(sketch.geometry.is_empty(), "no orphan control points");
}

#[test]
fn bspline_periodic_param_is_respected() {
    let (mut sketch, mut state) = fresh();
    let params = params(|p| p.bspline_periodic = true);
    let w = With::new(&params, &HashSet::new(), 0.1);
    w.clicks(
        &mut state,
        &mut sketch,
        "sketch.bspline",
        [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
    );
    super::finish_click_sequence(&mut state, &mut sketch, &params);
    let spline = spline_of(&sketch);
    assert!(spline.periodic);
    // A periodic spline is a closed wire on its own.
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
}

#[test]
fn polygon_two_clicks_builds_closed_ngon() {
    for sides in [3u32, 6, 12] {
        let (mut sketch, mut state) = fresh();
        let params = params(|p| p.polygon_sides = sides);
        let w = With::new(&params, &HashSet::new(), 0.5);
        w.click(&mut state, &mut sketch, "sketch.polygon", (2.0, 1.0));
        assert!(sketch.geometry.is_empty(), "nothing before completion");
        let fx = w.click(&mut state, &mut sketch, "sketch.polygon", (7.0, 1.0));
        assert!(fx.changed);
        assert!(state.is_idle());
        let n = sides as usize;
        // The vertices and the centre; the sides.
        assert_eq!((points(&sketch), lines(&sketch)), (n + 1, n));
        // Closed loop: every vertex used by exactly two lines.
        // (The centre is used once, by the construction circle.)
        let uses = point_use_counts(&sketch);
        assert_eq!(uses.len(), n + 1);
        assert_eq!(
            uses.values().filter(|&&c| c == 2).count(),
            n,
            "closed loop for n={n}"
        );
        assert!(has_point(&sketch, 7.0, 1.0), "the second click is a vertex");
        // All vertices on the circumscribed circle of radius 5, and held
        // there, every side as long as the first.
        let center = Vec2D::new(2.0, 1.0);
        for g in &sketch.geometry {
            if let GeometryElement::Point(p) = g
                && (p.position - center).to_glam().length() > 1e-4
            {
                let r = (p.position - center).to_glam().length();
                assert!((r - 5.0).abs() < 1e-4, "vertex off circle: r={r}");
            }
        }
        let count = |f: fn(&ConstraintKind) -> bool| {
            sketch.constraints.iter().filter(|c| f(&c.kind)).count()
        };
        assert_eq!(
            count(|k| matches!(k, ConstraintKind::PointOnCircle { .. })),
            n
        );
        assert_eq!(
            count(|k| matches!(k, ConstraintKind::EqualLength { .. })),
            n - 1
        );
    }
}

#[test]
fn slot_two_clicks_builds_closed_stadium() {
    let (mut sketch, mut state) = fresh();
    let params = params(|p| p.slot_width = 4.0);
    let w = With::new(&params, &HashSet::new(), 0.5);
    w.click(&mut state, &mut sketch, "sketch.slot", (0.0, 0.0));
    assert!(sketch.geometry.is_empty(), "nothing before completion");
    let fx = w.click(&mut state, &mut sketch, "sketch.slot", (10.0, 0.0));
    assert!(fx.changed);
    assert!(state.is_idle());
    // 4 junction points + 2 arc centers, 2 lines, 2 cap arcs.
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (6, 2, 2));
    // Junction points shared by exactly one line + one arc; the two arc
    // centers referenced once each.
    let uses = point_use_counts(&sketch);
    let mut counts: Vec<usize> = uses.values().copied().collect();
    counts.sort_unstable();
    assert_eq!(counts, vec![1, 1, 2, 2, 2, 2]);

    // The classic slot must be ONE closed profile wire of 4 segments.
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);

    // CCW-consistent caps bulge OUTWARD: the arc midpoints must sit on
    // the centerline extended past each end (x = -2 and x = 12).
    let mut arc_mid_xs: Vec<f64> = wires[0]
        .segments
        .iter()
        .filter_map(|s| match s {
            kernel_api::ProfileSegment::Arc { mid, .. } => Some(mid[0]),
            _ => None,
        })
        .collect();
    arc_mid_xs.sort_by(f64::total_cmp);
    assert_eq!(arc_mid_xs.len(), 2);
    assert!((arc_mid_xs[0] + 2.0).abs() < 1e-4, "left cap bulges left");
    assert!(
        (arc_mid_xs[1] - 12.0).abs() < 1e-4,
        "right cap bulges right"
    );
}

/// Rectangle (0,0)-(w,h) built from shared corner points.
fn build_rectangle(sketch: &mut Sketch, w: f64, h: f64) -> [Uuid; 4] {
    let a = pt(sketch, 0.0, 0.0);
    let b = pt(sketch, w, 0.0);
    let c = pt(sketch, w, h);
    let d = pt(sketch, 0.0, h);
    line_between(sketch, a, b);
    line_between(sketch, b, c);
    line_between(sketch, c, d);
    line_between(sketch, d, a);
    [a, b, c, d]
}

#[test]
fn fillet_rounds_rectangle_corner_tangentially() {
    let (mut sketch, mut state) = fresh();
    build_rectangle(&mut sketch, 12.0, 8.0);
    let w = With::new(&params(|p| p.fillet_radius = 2.0), &HashSet::new(), 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.fillet", (12.0, 8.0));
    assert!(fx.changed);
    // Corner point replaced by 2 tangent points + 1 arc center.
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (6, 4, 1));

    let arc = arc_of(&sketch);
    let center = sketch.point_position(arc.center).unwrap();
    let start = sketch.point_position(arc.start).unwrap();
    let end = sketch.point_position(arc.end).unwrap();
    // 90° corner with R=2: center at (10, 6), tangent points at
    // (12, 6) and (10, 8).
    assert!((center.to_glam() - glam::DVec2::new(10.0, 6.0)).length() < 1e-4);
    assert!((arc.radius - 2.0).abs() < 1e-5);
    assert!(((start.to_glam() - center.to_glam()).length() - 2.0).abs() < 1e-4);
    assert!(((end.to_glam() - center.to_glam()).length() - 2.0).abs() < 1e-4);
    // The arc bridges on the corner side: its midpoint bulges toward
    // the removed corner (12,8), i.e. beyond the chord.
    let (start_angle, sweep) =
        crate::snap::arc_angles((start - center).to_glam(), (end - center).to_glam());
    assert!(
        (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-3,
        "quarter-circle sweep, got {sweep}"
    );
    let mid_angle = start_angle + sweep * 0.5;
    let mid = center.to_glam() + 2.0 * glam::DVec2::new(mid_angle.cos(), mid_angle.sin());
    assert!(
        mid.x > 10.5 && mid.y > 6.5,
        "arc bulges toward the corner: {mid:?}"
    );

    // Profile survives: one closed wire of 4 lines + 1 arc.
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 5);
}

#[test]
fn a_fillet_that_cannot_be_made_changes_nothing() {
    // A 3 × 3 rectangle, and a line of two collinear halves along y = 20.
    let (mut sketch, _) = fresh();
    build_rectangle(&mut sketch, 3.0, 3.0);
    let [a, m, b] = [0.0, 5.0, 10.0].map(|x| pt(&mut sketch, x, 20.0));
    line_between(&mut sketch, a, m);
    line_between(&mut sketch, m, b);
    for (what, radius, at) in [
        // A 90° corner's tangent offset is the radius: 4 cannot fit in 3.
        ("a radius too large", 4.0, (3.0, 3.0)),
        ("empty space", 1.0, (5.0, 10.0)),
        ("an end with one line", 1.0, (0.0, 20.0)),
        ("a corner with no angle", 1.0, (5.0, 20.0)),
    ] {
        // Each row is a first click: a curve picked by the row before
        // would take this click as its second.
        let mut state = ToolState::default();
        let w = With::new(&params(|p| p.fillet_radius = radius), &HashSet::new(), 0.5);
        let fx = w.click(&mut state, &mut sketch, "sketch.fillet", at);
        assert!(!fx.changed, "{what}");
        let counts = (points(&sketch), lines(&sketch), arcs(&sketch));
        assert_eq!(counts, (7, 6, 0), "{what}: the sketch untouched");
    }
}

#[test]
fn fillet_drops_constraints_on_removed_corner() {
    let (mut sketch, mut state) = fresh();
    let [.., c, _] = build_rectangle(&mut sketch, 12.0, 8.0);
    pin(&mut sketch, c);
    let w = With::new(&params(|p| p.fillet_radius = 2.0), &HashSet::new(), 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.fillet", (12.0, 8.0));
    assert!(fx.changed);
    assert!(
        !sketch
            .constraints
            .iter()
            .any(|con| crate::sketch::constraint_refs(&con.kind).contains(&c)),
        "constraint on the removed corner dropped"
    );
    assert_eq!(
        sketch
            .constraints
            .iter()
            .filter(|con| matches!(con.kind, ConstraintKind::Tangent { .. }))
            .count(),
        2,
        "the arc held tangent to both lines"
    );
    assert!(sketch.get_geometry(c).is_none(), "corner point removed");
}

#[test]
fn trim_without_intersections_deletes_element_and_orphans() {
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    sketch.add_constraint(ConstraintKind::Horizontal { element: l });

    let fx = click(&mut state, "sketch.trim", &mut sketch, (5.0, 0.1), 0.5);
    assert!(fx.changed);
    assert!(sketch.geometry.is_empty(), "line and orphan endpoints gone");
    assert!(sketch.constraints.is_empty(), "line constraint dropped");
}

#[test]
fn trim_end_span_keeps_shared_endpoint() {
    let (mut sketch, mut state) = fresh();
    // Horizontal line whose start also anchors another line; one cutter.
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 20.0, 0.0);
    line_between(&mut sketch, a, b);
    let c = pt(&mut sketch, 0.0, 10.0);
    line_between(&mut sketch, a, c); // shares point a
    let d1 = pt(&mut sketch, 5.0, -5.0);
    let d2 = pt(&mut sketch, 5.0, 5.0);
    line_between(&mut sketch, d1, d2);

    // Click past the cutter: the (5..20) span goes; endpoint b is orphaned
    // and removed, endpoint a stays (still used by the second line).
    let fx = click(&mut state, "sketch.trim", &mut sketch, (12.0, 0.1), 0.5);
    assert!(fx.changed);
    assert!(sketch.get_geometry(a).is_some(), "shared endpoint kept");
    assert!(
        sketch.get_geometry(b).is_none(),
        "orphaned endpoint removed"
    );
    let spans: Vec<f64> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                let s = sketch.point_position(l.start)?;
                let e = sketch.point_position(l.end)?;
                (s.y.abs() < 1e-4 && e.y.abs() < 1e-4).then(|| s.x.max(e.x))
            }
            _ => None,
        })
        .collect();
    assert_eq!(spans.len(), 1);
    assert!((spans[0] - 5.0).abs() < 1e-3, "shortened to the cutter");
}

#[test]
fn trim_circle_span_becomes_arc_keeping_id() {
    let (mut sketch, mut state) = fresh();
    let center = pt(&mut sketch, 0.0, 0.0);
    let circle_id = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
    sketch.add_constraint(ConstraintKind::Radius {
        circle: circle_id,
        radius: 5.0,
    });
    // Vertical cutter through the circle: hits (0,5) and (0,-5).
    let a = pt(&mut sketch, 0.0, -10.0);
    let b = pt(&mut sketch, 0.0, 10.0);
    line_between(&mut sketch, a, b);

    // Click the right side of the rim: that half is removed.
    let fx = click(&mut state, "sketch.trim", &mut sketch, (5.1, 0.0), 0.5);
    assert!(fx.changed);
    assert_eq!(circles(&sketch), 0);
    let arc = arc_of(&sketch);
    assert_eq!(arc.id, circle_id, "arc keeps the circle's id");
    assert_eq!(sketch.constraints.len(), 1, "radius constraint survives");
    // Kept half passes through (-5, 0).
    let s = sketch.point_position(arc.start).unwrap().to_glam();
    let e = sketch.point_position(arc.end).unwrap().to_glam();
    assert!(crate::geom2d::point_on_arc(
        glam::DVec2::ZERO,
        s,
        e,
        glam::DVec2::new(-5.0, 0.0)
    ));
    assert!(!crate::geom2d::point_on_arc(
        glam::DVec2::ZERO,
        s,
        e,
        glam::DVec2::new(5.0, 0.0)
    ));
}

/// With the axes cutting, a line crossing the Y axis trims there and its
/// new end is held on the axis; with them off the whole line goes.
#[test]
fn trim_cuts_at_the_axes_when_asked_and_holds_the_end_there() {
    let crossing = || {
        let mut sketch = Sketch::new("t");
        let a = pt(&mut sketch, -6.0, 3.0);
        let b = pt(&mut sketch, 10.0, 3.0);
        line_between(&mut sketch, a, b);
        (sketch, a)
    };
    let axes = params(|p| p.cut_at_axes = true);

    let (mut sketch, a) = crossing();
    let w = With::new(&axes, &HashSet::new(), 0.5);
    let fx = w.click(
        &mut ToolState::default(),
        &mut sketch,
        "sketch.trim",
        (5.0, 3.1),
    );
    assert!(fx.changed);
    assert_eq!(lines(&sketch), 1);
    let kept = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some((l.start, l.end)),
            _ => None,
        })
        .unwrap();
    assert_eq!(kept.0, a, "the left part stays");
    let end = sketch.point_position(kept.1).unwrap();
    assert!(
        end.x.abs() < 1e-4 && (end.y - 3.0).abs() < 1e-4,
        "cut at the axis: {end:?}"
    );
    assert!(
        sketch.constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::PointOnLine { point, line } if point == kept.1 && line == crate::sketch::Y_AXIS_ID
        )),
        "the new end is held on the Y axis"
    );

    let (mut sketch, _) = crossing();
    click(
        &mut ToolState::default(),
        "sketch.trim",
        &mut sketch,
        (5.0, 3.1),
        0.5,
    );
    assert_eq!(lines(&sketch), 0, "off, nothing cuts it");
}

/// With the axes cutting, Extend reaches the X axis when nothing else is
/// in the way, and holds the end there.
#[test]
fn extend_reaches_an_axis_when_asked() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 4.0, 8.0);
    let b = pt(&mut sketch, 4.0, 3.0);
    line_between(&mut sketch, a, b);
    let axes = params(|p| p.cut_at_axes = true);
    let w = With::new(&axes, &HashSet::new(), 0.5);
    let fx = w.click(
        &mut ToolState::default(),
        &mut sketch,
        "sketch.extend",
        (4.1, 4.0),
    );
    assert!(fx.changed);
    let end = sketch.point_position(b).unwrap();
    assert!(
        (end.x - 4.0).abs() < 1e-4 && end.y.abs() < 1e-4,
        "on the X axis: {end:?}"
    );
    assert!(sketch.constraints.iter().any(|c| matches!(
        c.kind,
        ConstraintKind::PointOnLine { point, line } if point == b && line == crate::sketch::X_AXIS_ID
    )));
}

#[test]
fn extend_arc_end_reaches_circle() {
    let (mut sketch, mut state) = fresh();
    // Quarter arc around origin from (5,0) to (0,5).
    let c = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 5.0, 0.0);
    let e = pt(&mut sketch, 0.0, 5.0);
    sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
    // A wall along y = 0 to the left, crossing the arc's circle at (-5, 0).
    let w1 = pt(&mut sketch, -10.0, 0.0);
    let w2 = pt(&mut sketch, -2.0, 0.0);
    line_between(&mut sketch, w1, w2);

    // Click near the arc's end half (close to (0,5)).
    let fx = click(&mut state, "sketch.extend", &mut sketch, (0.5, 5.0), 0.6);
    assert!(fx.changed);
    let end = sketch.point_position(e).unwrap().to_glam();
    assert!(
        (end - glam::DVec2::new(-5.0, 0.0)).length() < 1e-3,
        "arc end swept CCW to the wall: {end:?}"
    );
}

#[test]
fn split_arc_produces_two_ccw_arcs() {
    let (mut sketch, mut state) = fresh();
    let c = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 5.0, 0.0);
    let e = pt(&mut sketch, -5.0, 0.0);
    sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));

    // Click the top of the semicircle.
    let fx = click(&mut state, "sketch.split", &mut sketch, (0.0, 5.1), 0.5);
    assert!(fx.changed);
    assert_eq!(arcs(&sketch), 2);
    let arcs_v: Vec<Arc> = all_arcs(&sketch);
    assert_eq!(arcs_v[0].end, arcs_v[1].start, "shared split point");
    assert!(
        sketch.constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::EqualRadius { circle1, circle2 }
                if circle1 == arcs_v[0].id && circle2 == arcs_v[1].id
        )),
        "the halves keep one radius"
    );
    for a in &arcs_v {
        let sp = sketch.point_position(a.start).unwrap().to_glam();
        let ep = sketch.point_position(a.end).unwrap().to_glam();
        let (_, sweep) = crate::snap::arc_angles(sp, ep);
        assert!(
            (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-3,
            "quarter sweep each, got {sweep}"
        );
    }
}

#[test]
fn offset_single_circle_is_concentric() {
    let (mut sketch, mut state) = fresh();
    let center = pt(&mut sketch, 3.0, 3.0);
    let circle_id = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
    let selected: HashSet<Uuid> = [circle_id].into_iter().collect();
    let params = params(|p| p.offset_distance = 2.0);
    // Click outside the rim: the copy grows.
    let w = With::new(&params, &selected, 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.offset", (12.0, 3.0));
    assert!(fx.changed);
    let radii: Vec<f64> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Circle(c) => {
                assert_eq!(c.center, center, "offset circle shares the center point");
                Some(c.radius)
            }
            _ => None,
        })
        .collect();
    let mut radii = radii;
    radii.sort_by(f64::total_cmp);
    assert_eq!(radii, vec![5.0, 7.0]);
    assert_eq!(points(&sketch), 1);
}

#[test]
fn translate_moves_selection_and_shared_points() {
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams::default();
    let w = With::new(&params, &selected, 0.1);
    w.click(
        &mut state,
        &mut sketch,
        "sketch.translate",
        Vec2D::new(20.0, 20.0),
    );
    let w = With::new(&params, &selected, 0.1);
    let fx = w.click(
        &mut state,
        &mut sketch,
        "sketch.translate",
        Vec2D::new(25.0, 23.0),
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    let pa = sketch.point_position(a).unwrap().to_glam();
    let pb = sketch.point_position(b).unwrap().to_glam();
    assert!((pa - glam::DVec2::new(5.0, 3.0)).length() < 1e-3);
    assert!((pb - glam::DVec2::new(15.0, 3.0)).length() < 1e-3);
    assert_eq!((points(&sketch), lines(&sketch)), (2, 1), "no copies");
}

#[test]
fn translate_with_copies_builds_array_preserving_sharing() {
    let (mut sketch, mut state) = fresh();
    // Two lines sharing a middle point: internal sharing must be preserved
    // in each copy.
    let a = pt(&mut sketch, 0.0, 0.0);
    let m = pt(&mut sketch, 5.0, 5.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l1 = line_between(&mut sketch, a, m);
    let l2 = line_between(&mut sketch, m, b);
    let selected: HashSet<Uuid> = [l1, l2].into_iter().collect();
    let params = params(|p| p.copies = 2);
    let w = With::new(&params, &selected, 0.1);
    w.click(&mut state, &mut sketch, "sketch.translate", (20.0, 20.0));
    let w = With::new(&params, &selected, 0.1);
    let fx = w.click(
        &mut state,
        &mut sketch,
        "sketch.translate",
        Vec2D::new(40.0, 20.0),
    );
    assert!(fx.changed);
    // Originals + 2 copies: 9 points, 6 lines.
    assert_eq!((points(&sketch), lines(&sketch)), (9, 6));
    // Originals unmoved.
    assert!(sketch.point_position(a).unwrap().to_glam().length() < 1e-6);
    // Each copy shares its own middle point (every point used ≤ 2 times,
    // apex points exactly twice).
    let uses = point_use_counts(&sketch);
    let doubles = uses.values().filter(|&&n| n == 2).count();
    assert_eq!(doubles, 3, "one shared apex per copy + original");
    // Second copy apex at (5,5) + 2Δ = (45, 5).
    assert!(has_point(&sketch, 45.0, 5.0));
}

#[test]
fn rotate_selection_about_pivot() {
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams::default();
    // Pivot at the origin, where it snaps to point `a` at the same spot.
    let w = With::new(&params, &selected, 0.1);
    w.click(&mut state, &mut sketch, "sketch.rotate", (0.0, 0.0));
    // Reference along +x, target along +y: 90° CCW.
    w.click(&mut state, &mut sketch, "sketch.rotate", (20.0, 0.0));
    let fx = w.click(&mut state, &mut sketch, "sketch.rotate", (0.0, 20.0));
    assert!(fx.changed);
    let pb = sketch.point_position(b).unwrap().to_glam();
    assert!(
        (pb - glam::DVec2::new(0.0, 10.0)).length() < 1e-3,
        "endpoint rotated 90°: {pb:?}"
    );
}

#[test]
fn pattern_steps_count_copies_or_elements_on_one_side_or_both() {
    let with = |copies, total, symmetric| ToolParams {
        copies,
        copies_total: total,
        copies_symmetric: symmetric,
        ..ToolParams::default()
    };
    let steps = |p: ToolParams| super::pattern_multiples(&p);
    assert_eq!(steps(with(0, false, false)), None, "no copies: a move");
    assert_eq!(steps(with(3, false, false)), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(
        steps(with(2, false, true)),
        Some(vec![-2.0, -1.0, 1.0, 2.0])
    );
    let thirds = steps(with(4, true, false)).unwrap();
    for (got, want) in thirds.iter().zip([1.0 / 3.0, 2.0 / 3.0, 1.0]) {
        assert!(
            (got - want).abs() < 1e-6,
            "the click spans all four: {thirds:?}"
        );
    }
    assert_eq!(
        steps(with(1, true, false)),
        None,
        "the original alone: a move"
    );
    assert_eq!(steps(with(3, true, true)), Some(vec![-1.0, -0.5, 0.5, 1.0]));
}

/// A full turn takes only the centre: four elements a quarter turn apart,
/// held by one polar pitch through the original in turn order.
#[test]
fn rotate_full_turn_spreads_the_elements_evenly_from_one_click() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 10.0, 0.0);
    let b = pt(&mut sketch, 12.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams {
        copies: 4,
        copies_total: true,
        copies_linked: true,
        rotate_full_turn: true,
        ..ToolParams::default()
    };
    let w = With::new(&params, &selected, 0.1);
    let fx = w.click(
        &mut ToolState::default(),
        &mut sketch,
        "sketch.rotate",
        (0.0, 0.0),
    );
    assert!(fx.changed);
    assert_eq!(lines(&sketch), 4, "the original and three copies");
    for want in [(0.0, 10.0), (-10.0, 0.0), (0.0, -10.0)] {
        assert!(
            has_point(&sketch, want.0, want.1),
            "a copy's start at {want:?}"
        );
    }
    let pitch = sketch
        .constraints
        .iter()
        .find_map(|c| match &c.kind {
            ConstraintKind::PolarPitch {
                points, angle_rad, ..
            } => Some((points.clone(), *angle_rad)),
            _ => None,
        })
        .expect("a polar pitch");
    assert_eq!(pitch.0.len(), 4);
    assert_eq!(pitch.0[0], a, "the original first, the rest in turn");
    assert!((pitch.1 - std::f64::consts::FRAC_PI_2).abs() < 1e-5);
}

/// Symmetric copies go both ways from the original, and a linked pitch
/// runs through them in order with the original in the middle.
#[test]
fn translate_symmetric_copies_go_both_ways() {
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 0.0, 4.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams {
        copies: 1,
        copies_symmetric: true,
        copies_linked: true,
        ..ToolParams::default()
    };
    let w = With::new(&params, &selected, 0.1);
    w.clicks(
        &mut state,
        &mut sketch,
        "sketch.translate",
        [Vec2D::new(20.0, 20.0), Vec2D::new(25.0, 20.0)],
    );
    assert_eq!(lines(&sketch), 3);
    for x in [-5.0f64, 5.0] {
        assert!(has_point(&sketch, x, 0.0));
    }
    let (points, distance) = sketch
        .constraints
        .iter()
        .find_map(|c| match &c.kind {
            ConstraintKind::Pitch {
                points, distance, ..
            } => Some((points.clone(), *distance)),
            _ => None,
        })
        .expect("a pitch");
    assert_eq!(points.len(), 3);
    assert_eq!(points[1], a, "the original between its copies");
    assert!((distance - 5.0).abs() < 1e-4);
}

#[test]
fn scale_selection_scales_radii_too() {
    let (mut sketch, mut state) = fresh();
    let center = pt(&mut sketch, 4.0, 0.0);
    let circle_id = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
    let selected: HashSet<Uuid> = [circle_id].into_iter().collect();
    let params = ToolParams::default();
    // Base at the origin, reference at (10, 0), target at (20, 0): a factor
    // of 2, clicked clear of the circle's points so nothing snaps.
    let w = With::new(&params, &selected, 0.1);
    w.click(&mut state, &mut sketch, "sketch.scale", (0.0, 0.0));
    w.click(&mut state, &mut sketch, "sketch.scale", (10.0, 0.0));
    let fx = w.click(&mut state, &mut sketch, "sketch.scale", (20.0, 0.0));
    assert!(fx.changed);
    let c = sketch.point_position(center).unwrap().to_glam();
    assert!((c - glam::DVec2::new(8.0, 0.0)).length() < 1e-3);
    let r = circle_of(&sketch).radius;
    assert!((r - 4.0).abs() < 1e-3, "radius scaled with the geometry");
}

#[test]
fn mirror_arc_copy_stays_ccw() {
    let (mut sketch, mut state) = fresh();
    // Upper semicircle arc from (5,0) to (-5,0), mirrored about the x-axis.
    let c = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 5.0, 0.0);
    let e = pt(&mut sketch, -5.0, 0.0);
    let arc_id = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
    let selected: HashSet<Uuid> = [arc_id].into_iter().collect();
    let params = ToolParams::default();
    // Two-point axis along the x-axis (empty space clicks).
    let w = With::new(&params, &selected, 0.1);
    w.click(&mut state, &mut sketch, "sketch.mirror", (-20.0, 0.0));
    let fx = w.click(&mut state, &mut sketch, "sketch.mirror", (20.0, 0.0));
    assert!(fx.changed);
    assert_eq!(arcs(&sketch), 2);
    // The mirrored arc must pass through (0,-5) with a CCW sweep of π.
    let copy = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Arc(a) if a.id != arc_id => Some(a.clone()),
            _ => None,
        })
        .next()
        .unwrap();
    let cc = sketch.point_position(copy.center).unwrap().to_glam();
    let cs = sketch.point_position(copy.start).unwrap().to_glam();
    let ce = sketch.point_position(copy.end).unwrap().to_glam();
    assert!(crate::geom2d::point_on_arc(
        cc,
        cs,
        ce,
        glam::DVec2::new(0.0, -5.0)
    ));
    let (_, sweep) = crate::snap::arc_angles(cs - cc, ce - cc);
    assert!((sweep - std::f64::consts::PI).abs() < 1e-3);
}

/// Two ends of the major axis, then a rim point: the ellipse is centred
/// between the ends and passes through the rim point.
#[test]
fn a_three_point_ellipse_passes_through_its_rim_point() {
    let (mut sketch, mut state) = fresh();
    for p in [Vec2D::new(-4.0, 0.0), Vec2D::new(4.0, 0.0)] {
        click(&mut state, "sketch.ellipse3", &mut sketch, p, 0.01);
    }
    assert!(sketch.geometry.is_empty(), "nothing before the rim point");
    let rim = Vec2D::new(2.0, 3.0_f64.sqrt());
    assert!(click(&mut state, "sketch.ellipse3", &mut sketch, rim, 0.01).changed);
    let ellipse = ellipse_of(&sketch);
    let center = sketch.point_position(ellipse.center).expect("its center");
    assert!(center.to_glam().length() < 1e-5, "centred between the ends");
    assert!((ellipse.major.to_glam().length() - 4.0).abs() < 1e-4);
    // x²/16 + y²/b² = 1 through (2, √3) gives b = 2.
    assert!(
        (ellipse.ratio - 0.5).abs() < 1e-4,
        "ratio {}",
        ellipse.ratio
    );
}

/// Center, major vertex, start on the rim, end: an arc of the ellipse
/// whose endpoints are points held on it, and whose profile segment is an
/// elliptical arc from start to end.
#[test]
fn an_arc_of_ellipse_ends_on_its_ellipse_and_profiles_as_an_arc() {
    let (mut sketch, mut state) = fresh();
    for p in [
        Vec2D::new(0.0, 0.0),
        Vec2D::new(4.0, 0.0),
        Vec2D::new(4.0 * (0.5f64).sqrt(), 2.0 * (0.5f64).sqrt()),
    ] {
        click(&mut state, "sketch.ellipse_arc", &mut sketch, p, 0.01);
    }
    assert!(
        !sketch
            .geometry
            .iter()
            .any(|g| matches!(g, GeometryElement::Ellipse(_)))
    );
    assert!(
        click(
            &mut state,
            "sketch.ellipse_arc",
            &mut sketch,
            (-3.0, 0.5),
            0.01
        )
        .changed
    );
    let ellipse = ellipse_of(&sketch);
    let arc = ellipse.arc.expect("it is an arc");
    assert!((ellipse.ratio - 0.5).abs() < 1e-4);
    let (t0, t1) = ellipse.param_span(&sketch).expect("a span");
    assert!(
        (t0 - std::f64::consts::FRAC_PI_4).abs() < 1e-3,
        "starts at 45°: {t0}"
    );
    assert!(
        t1 > t0 && t1 < std::f64::consts::PI + 1e-3,
        "ends before the far vertex: {t1}"
    );
    for end in [arc.start, arc.end] {
        assert!(sketch.constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::PointOnEllipse { point, ellipse: e } if point == end && e == ellipse.id
        )));
    }
    // Closed with a line from end back to start, it is one wire of an
    // elliptical arc and a line.
    sketch.add_geometry(GeometryElement::Line(Line::new(arc.end, arc.start)));
    let wires = crate::profile::extract_wires(&sketch).expect("a closed profile");
    assert_eq!(wires.len(), 1);
    assert!(
        wires[0]
            .segments
            .iter()
            .any(|s| matches!(s, kernel_api::ProfileSegment::EllipseArc { .. }))
    );
}

/// A polyline of lines and a tangent arc, drawn in one chain: the arc
/// leaves tangent to the line before it, and a click back on the start
/// closes the shape into one profile.
#[test]
fn a_polyline_chains_lines_and_a_tangent_arc_into_a_closed_shape() {
    let (mut sketch, mut state) = fresh();
    let click = |state: &mut ToolState, sketch: &mut Sketch, x: f64, y: f64| {
        click(state, "sketch.polyline", sketch, (x, y), 0.2).changed
    };
    click(&mut state, &mut sketch, 0.0, 0.0);
    assert!(click(&mut state, &mut sketch, 10.0, 0.0));
    assert!(toggle_polyline_arc(&mut state), "arcs from here");
    // Leaving (10, 0) heading +x and ending at (10, 10): a left half turn
    // about (10, 5).
    assert!(click(&mut state, &mut sketch, 10.0, 10.0));
    // M cycles: a square arc, a reversed arc, then lines again.
    for _ in 0..3 {
        assert!(toggle_polyline_arc(&mut state));
    }
    assert!(matches!(
        state,
        ToolState::PolylineFrom {
            segment: PolySegment::Line,
            ..
        }
    ));
    assert!(click(&mut state, &mut sketch, 0.0, 10.0));
    assert!(click(&mut state, &mut sketch, 0.0, 0.0));
    assert!(state.is_idle(), "a click on the start closes the polyline");

    let arcs: Vec<Arc> = all_arcs(&sketch);
    assert_eq!(arcs.len(), 1);
    let center = sketch.point_position(arcs[0].center).expect("center");
    assert!((center.x - 10.0).abs() < 1e-4 && (center.y - 5.0).abs() < 1e-4);
    assert!((arcs[0].radius - 5.0).abs() < 1e-4);
    assert_eq!(
        count_kind(&sketch, |g| matches!(g, GeometryElement::Line(_))),
        3
    );
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Tangent { .. })),
        "the arc is held tangent to the line before it"
    );
    let wires = crate::profile::extract_wires(&sketch).expect("a closed profile");
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
}

#[test]
fn a_transform_with_nothing_selected_says_so_at_its_first_click() {
    let (mut sketch, mut state) = fresh();
    for tool in [
        "sketch.translate",
        "sketch.rotate",
        "sketch.scale",
        "sketch.mirror",
    ] {
        let effect = click(&mut state, tool, &mut sketch, (1.0, 1.0), 0.5);
        assert!(!effect.changed);
        assert!(effect.log.is_some(), "{tool} says why");
        assert!(state.is_idle(), "{tool} did not start");
    }
}

/// Rotating a rectangle keeps it rotated and square: its horizontal and
/// vertical constraints turn with it instead of pulling it back.
#[test]
fn a_rotated_rectangle_stays_rotated_and_square() {
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.rect", &mut sketch, (0.0, 0.0), 0.5);
    click(&mut state, "sketch.rect", &mut sketch, (20.0, 10.0), 0.5);
    let everything: HashSet<Uuid> = sketch.geometry.iter().map(|g| g.id()).collect();
    let angle = 30f64.to_radians();
    With::new(&ToolParams::default(), &everything, 0.5).clicks(
        &mut state,
        &mut sketch,
        "sketch.rotate",
        [
            Vec2D::new(0.0, 0.0),
            Vec2D::new(10.0, 0.0),
            Vec2D::new(10.0 * angle.cos(), 10.0 * angle.sin()),
        ],
    );
    crate::solver::solve(&mut sketch);
    let lines: Vec<glam::DVec2> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(
                (sketch.point_position(l.end).unwrap() - sketch.point_position(l.start).unwrap())
                    .to_glam(),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 4);
    let bottom = lines[0];
    assert!(
        (bottom.y.atan2(bottom.x) - angle).abs() < 1e-3,
        "still turned 30°: {bottom:?}"
    );
    for pair in lines.windows(2) {
        assert!(
            pair[0].normalize().dot(pair[1].normalize()).abs() < 1e-3,
            "square corners"
        );
    }
}

/// A copy keeps the shape's constraints: a mirrored rectangle is still a
/// rectangle to the solver.
#[test]
fn a_copy_carries_the_constraints_of_what_it_copies() {
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.rect", &mut sketch, (0.0, 0.0), 0.5);
    click(&mut state, "sketch.rect", &mut sketch, (20.0, 10.0), 0.5);
    let shape = |sketch: &Sketch| {
        sketch
            .constraints
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. }
                )
            })
            .count()
    };
    let before = shape(&sketch);
    let everything: HashSet<Uuid> = sketch.geometry.iter().map(|g| g.id()).collect();
    transform::copy_selection(
        &mut sketch,
        &everything,
        &transform::Similarity::translation(glam::DVec2::new(40.0, 0.0)),
    );
    // The copy's own shape comes with it; a pin to the origin does not.
    assert_eq!(shape(&sketch), 2 * before);
}

/// A fillet on a dimensioned rectangle holds: the dimension of the edge it
/// shortened goes, so the solve does not stretch the edge back and tear
/// the fillet.
#[test]
fn a_fillet_on_a_dimensioned_corner_survives_the_solve() {
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.rect", &mut sketch, (3.0, 3.0), 0.5);
    click(&mut state, "sketch.rect", &mut sketch, (23.0, 13.0), 0.5);
    let bottom = line_of(&sketch).id;
    sketch.add_constraint(ConstraintKind::Length {
        line: bottom,
        length: 20.0,
    });
    let params = params(|p| p.fillet_radius = 3.0);
    let w = With::new(&params, &HashSet::new(), 0.5);
    let effect = w.click(&mut state, &mut sketch, "sketch.fillet", (23.0, 3.0));
    assert!(effect.changed);
    converged(&mut sketch);
    let arc = sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Arc(a) => Some(a.clone()),
        _ => None,
    });
    let arc = arc.expect("the fillet's arc");
    let (c, s) = (
        sketch.point_position(arc.center).unwrap(),
        sketch.point_position(arc.start).unwrap(),
    );
    assert!(
        ((s - c).to_glam().length() - 3.0).abs() < 1e-3,
        "radius kept"
    );
}

/// A split line's halves share the split point, on the line where it was
/// clicked, and stay on one level line.
#[test]
fn a_split_line_stays_one_line() {
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.line", &mut sketch, (3.0, 4.0), 0.5);
    click(&mut state, "sketch.line", &mut sketch, (23.0, 4.0), 0.5);
    let line = sketch
        .geometry
        .iter()
        .find(|g| matches!(g, GeometryElement::Line(_)));
    let line = line.unwrap().id();
    click(&mut state, "sketch.split", &mut sketch, (10.0, 4.1), 0.5);
    let halves: Vec<Line> = all_lines(&sketch);
    assert!(halves.iter().any(|h| h.id == line), "the line keeps its id");
    assert_eq!(halves[0].end, halves[1].start, "shared split point");
    let m = sketch.point_position(halves[0].end).unwrap();
    assert!((m.to_glam() - glam::DVec2::new(10.0, 4.0)).length() < 1e-3);
    let level = sketch
        .constraints
        .iter()
        .filter(|c| matches!(c.kind, ConstraintKind::Horizontal { .. }))
        .count();
    assert_eq!(level, 2, "both halves level");
}

/// The shapes a tool constrains as it draws them close one profile and
/// are held consistently: they solve, nothing is redundant or in
/// conflict, and what is left free is the shape's own freedom (a straight
/// slot's two ends and width; an arc slot's centre, its two rails' radii
/// and the angles its ends stand at).
#[test]
fn shape_tools_constrain_their_shapes_cleanly() {
    // The tool, its clicks, the profile's segments and the freedom left.
    type Case<'a> = (&'a str, &'a [(f64, f64)], usize, Option<i32>);
    let cases: [Case; 5] = [
        ("sketch.slot", &[(2.0, 2.0), (12.0, 5.0)], 4, None),
        ("sketch.slot", &[(3.0, 3.0), (13.0, 3.0)], 4, Some(5)),
        (
            "sketch.arc_slot",
            &[(3.0, 2.0), (13.0, 2.0), (3.0, 12.0)],
            4,
            Some(6),
        ),
        ("sketch.polygon", &[(2.0, 2.0), (7.0, 3.0)], 6, None),
        ("sketch.rect_center", &[(5.0, 3.0), (9.0, 5.0)], 4, None),
    ];
    for (tool, clicks, segments, dof) in cases {
        let (mut sketch, mut state) = fresh();
        for &at in clicks {
            click(&mut state, tool, &mut sketch, at, 0.5);
        }
        assert!(state.is_idle(), "{tool} committed");
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1, "{tool} {clicks:?}");
        assert_eq!(wires[0].segments.len(), segments, "{tool} {clicks:?}");
        let diagnosis = crate::solver::diagnose(&sketch);
        assert!(diagnosis.analyzed, "{tool}");
        assert!(diagnosis.redundant.is_empty(), "{tool}: {diagnosis:?}");
        assert!(diagnosis.conflicting.is_empty(), "{tool}: {diagnosis:?}");
        if let Some(dof) = dof {
            assert_eq!(diagnosis.dof, dof, "{tool}");
        }
        let outcome = crate::solver::solve(&mut sketch);
        assert!(
            matches!(outcome, crate::solver::SolveOutcome::Converged { .. }),
            "{tool}: {outcome:?}"
        );
    }
}

/// Every line's direction, by id.
fn line_dirs(sketch: &Sketch) -> Vec<glam::DVec2> {
    sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                Some((sketch.point_position(l.end)? - sketch.point_position(l.start)?).to_glam())
            }
            _ => None,
        })
        .collect()
}

/// The four edges, in order, make a rectangle: every corner square and
/// the loop closed.
fn assert_rectangle(sketch: &Sketch) {
    let dirs = line_dirs(sketch);
    assert_eq!(dirs.len(), 4);
    for i in 0..4 {
        let (d0, d1) = (dirs[i], dirs[(i + 1) % 4]);
        assert!(
            d0.normalize().dot(d1.normalize()).abs() < 1e-3,
            "corner {i} is square: {d0:?} {d1:?}"
        );
    }
    assert!(point_use_counts(sketch).values().all(|&n| n == 2));
}

#[test]
fn a_rectangle_from_three_corners_turns_with_its_first_edge() {
    let (mut sketch, mut state) = fresh();
    for p in [(1.0, 1.0), (9.0, 7.0)] {
        let fx = click(&mut state, "sketch.rect3", &mut sketch, (p.0, p.1), 0.1);
        assert!(!fx.changed, "nothing before the third click");
    }
    // The third click only says how wide: 5 to the left of the edge.
    let fx = click(
        &mut state,
        "sketch.rect3",
        &mut sketch,
        (1.0 - 3.0 + 2.0, 1.0 + 4.0 + 1.5),
        0.1,
    );
    assert!(fx.changed && state.is_idle());
    assert_eq!((points(&sketch), lines(&sketch)), (4, 4));
    assert_eq!(sketch.constraints.len(), 3, "2 parallel + 1 perpendicular");
    assert_rectangle(&sketch);
    let dirs = line_dirs(&sketch);
    assert!((dirs[0].length() - 10.0).abs() < 1e-4);
    assert!((dirs[1].length() - 5.0).abs() < 1e-4);

    // Pulling one corner out of square, the constraints put it back.
    let corner = match &sketch.geometry[2] {
        GeometryElement::Point(p) => p.id,
        _ => unreachable!(),
    };
    if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(corner) {
        p.position = Vec2D::new(p.position.x + 1.0, p.position.y - 0.5);
    }
    crate::solver::solve_holding(&mut sketch, &[corner]);
    assert_rectangle(&sketch);
}

#[test]
fn a_rectangle_from_its_centre_and_two_corners() {
    let (mut sketch, mut state) = fresh();
    let tool = "sketch.rect_center3";
    click(&mut state, tool, &mut sketch, (1.0, 1.0), 0.1);
    click(&mut state, tool, &mut sketch, (5.0, 4.0), 0.1);
    // Toward the next corner: it lands as far out as the first.
    let fx = click(&mut state, tool, &mut sketch, (-5.0, 9.0), 0.1);
    assert!(fx.changed && state.is_idle());
    assert_eq!((points(&sketch), lines(&sketch)), (5, 4));
    assert_rectangle(&sketch);
    assert!(
        sketch.geometry.iter().any(|g| matches!(
            g,
            GeometryElement::Point(p)
                if (p.position.x + 2.0).abs() < 1e-4 && (p.position.y - 5.0).abs() < 1e-4
        )),
        "the second corner at (-2, 5)"
    );
    // The centre is construction and holds the corners symmetric.
    let centre = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Point(p) if sketch.is_construction(p.id) => Some(p.id),
            _ => None,
        })
        .expect("construction centre");
    if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(centre) {
        p.position = Vec2D::new(3.0, 2.0);
    }
    crate::solver::solve_holding(&mut sketch, &[centre]);
    assert_rectangle(&sketch);
    let sum = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) if p.id != centre => Some(p.position.to_glam()),
            _ => None,
        })
        .sum::<glam::DVec2>();
    assert!((sum / 4.0 - glam::DVec2::new(3.0, 2.0)).length() < 1e-3);
}

#[test]
fn a_frame_is_two_rectangles_a_dimensioned_wall_apart() {
    let (mut sketch, mut state) = fresh();
    let params = params(|p| p.offset_distance = 1.5);
    let w = With::new(&params, &HashSet::new(), 0.1);
    w.click(&mut state, &mut sketch, "sketch.rect_frame", (1.0, 1.0));
    let fx = w.click(&mut state, &mut sketch, "sketch.rect_frame", (11.0, 7.0));
    assert!(fx.changed && state.is_idle());
    assert_eq!((points(&sketch), lines(&sketch)), (8, 8));
    assert_eq!(sketch.constraints.len(), 12, "8 H/V + 4 wall distances");
    let wires = crate::profile::extract_wires(&sketch).expect("two closed outlines");
    assert_eq!(wires.len(), 2);
    let inner_at = |sketch: &Sketch, x: f64, y: f64| {
        sketch.geometry.iter().any(|g| {
            matches!(g, GeometryElement::Point(p)
                if (p.position.x - x).abs() < 1e-3 && (p.position.y - y).abs() < 1e-3)
        })
    };
    assert!(inner_at(&sketch, 2.5, 2.5) && inner_at(&sketch, 9.5, 5.5));

    // A thicker wall: every gap follows its dimension.
    for c in &mut sketch.constraints {
        if let ConstraintKind::DistanceX { value, .. } | ConstraintKind::DistanceY { value, .. } =
            &mut c.kind
        {
            *value = 2.0;
        }
    }
    converged(&mut sketch);
    for c in &sketch.constraints {
        if let ConstraintKind::DistanceX { a, b: Some(b), .. } = c.kind {
            let gap = sketch.point_position(b).unwrap() - sketch.point_position(a).unwrap();
            assert!((gap.x.abs() - 2.0).abs() < 1e-3 && (gap.y.abs() - 2.0).abs() < 1e-3);
        }
    }
    assert_eq!(crate::profile::extract_wires(&sketch).unwrap().len(), 2);
}

#[test]
fn a_trim_stroke_finds_what_it_crosses_first() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 1.0, -5.0);
    let b = pt(&mut sketch, 1.0, 5.0);
    line_between(&mut sketch, a, b);
    let c = pt(&mut sketch, 4.0, 4.0);
    sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 1.0)));
    let from = Vec2D::new(-1.0, 4.0);
    let to = Vec2D::new(9.0, 4.0);
    let first = next_stroke_crossing(&sketch, from, to).expect("the line");
    assert!((first.x - 1.0).abs() < 1e-4 && (first.y - 4.0).abs() < 1e-4);
    // From there on, the circle's near side, then its far side.
    let second = next_stroke_crossing(&sketch, first, to).expect("the circle");
    assert!((second.x - 3.0).abs() < 1e-4, "{second:?}");
    let third = next_stroke_crossing(&sketch, second, to).expect("the circle again");
    assert!((third.x - 5.0).abs() < 1e-4, "{third:?}");
    assert!(next_stroke_crossing(&sketch, third, to).is_none());
    // A path that stops short crosses nothing.
    assert!(next_stroke_crossing(&sketch, from, Vec2D::new(0.5, 4.0)).is_none());
}

/// The spline passes within `tol` of `p`.
fn passes_through(sketch: &Sketch, spline: &crate::sketch::BSpline, p: Vec2D, tol: f64) -> bool {
    spline
        .points(sketch, 2000)
        .unwrap()
        .iter()
        .any(|q| (*q - p).to_glam().length() < tol)
}

#[test]
fn a_spline_through_points_passes_through_every_click_and_follows_them() {
    let (mut sketch, mut state) = fresh();
    let params = ToolParams {
        bspline_interpolate: true,
        bspline_degree: 4,
        ..ToolParams::default()
    };
    let clicks = [(1.0, 1.0), (4.0, 5.0), (8.0, 6.0), (12.0, 2.0), (15.0, 4.0)];
    let w = With::new(&params, &HashSet::new(), 0.1);
    w.clicks(&mut state, &mut sketch, "sketch.bspline", clicks);
    let fx = finish_click_sequence(&mut state, &mut sketch, &params);
    assert!(fx.changed);
    let spline = spline_of(&sketch);
    assert_eq!(spline.degree, 4);
    assert_eq!(spline.fit_points.len(), 5);
    assert_eq!(spline.control_points.len(), 5);
    assert_eq!(
        (spline.control_points[0], spline.control_points[4]),
        (spline.fit_points[0], spline.fit_points[4]),
        "the ends are one point each"
    );
    assert_eq!(
        points(&sketch),
        5 + 3,
        "five clicks and three inner controls"
    );
    for (x, y) in clicks {
        assert!(passes_through(&sketch, &spline, Vec2D::new(x, y), 0.02));
    }
    // The profile carries its degree and knots.
    let b = pt(&mut sketch, 1.0, -3.0);
    let a = spline.fit_points[0];
    let e = spline.fit_points[4];
    let c = pt(&mut sketch, 15.0, -3.0);
    line_between(&mut sketch, e, c);
    line_between(&mut sketch, c, b);
    line_between(&mut sketch, b, a);
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert!(wires[0].segments.iter().any(|s| matches!(
        s,
        kernel_api::ProfileSegment::Nurbs { degree: 4, knots, control_points, .. }
            if knots.len() == 10 && control_points.len() == 5
    )));

    // Dragged, a point it passes through takes the curve with it.
    let middle = spline.fit_points[2];
    if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(middle) {
        p.position = Vec2D::new(8.0, 9.0);
    }
    crate::solver::solve_holding(&mut sketch, &[middle]);
    let spline = spline_of(&sketch);
    assert!(passes_through(&sketch, &spline, Vec2D::new(8.0, 9.0), 0.02));
    for (x, y) in [clicks[0], clicks[1], clicks[3], clicks[4]] {
        assert!(passes_through(&sketch, &spline, Vec2D::new(x, y), 0.02));
    }
}

#[test]
fn a_closed_spline_through_points_is_its_own_profile() {
    let (mut sketch, mut state) = fresh();
    let params = ToolParams {
        bspline_interpolate: true,
        bspline_periodic: true,
        bspline_degree: 2,
        ..ToolParams::default()
    };
    let clicks = [(1.0, 1.0), (9.0, 1.0), (9.0, 7.0), (1.0, 7.0)];
    let w = With::new(&params, &HashSet::new(), 0.1);
    w.clicks(&mut state, &mut sketch, "sketch.bspline", clicks);
    finish_click_sequence(&mut state, &mut sketch, &params);
    let spline = spline_of(&sketch);
    assert!(spline.periodic && spline.knots.is_empty());
    for (x, y) in clicks {
        assert!(passes_through(&sketch, &spline, Vec2D::new(x, y), 0.02));
    }
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert!(matches!(
        &wires[0].segments[..],
        [kernel_api::ProfileSegment::Nurbs {
            degree: 2,
            periodic: true,
            ..
        }]
    ));
}

#[test]
fn a_control_point_spline_takes_its_degree_and_a_cubic_stays_as_it_was() {
    for (degree, nurbs) in [(2, true), (3, false), (5, true)] {
        let (mut sketch, mut state) = fresh();
        let params = params(|p| p.bspline_degree = degree);
        With::new(&params, &HashSet::new(), 0.1).clicks(
            &mut state,
            &mut sketch,
            "sketch.bspline",
            [
                (1.0, 1.0),
                (3.0, 5.0),
                (6.0, 6.0),
                (8.0, 2.0),
                (10.0, 5.0),
                (12.0, 1.0),
            ],
        );
        finish_click_sequence(&mut state, &mut sketch, &params);
        let spline = spline_of(&sketch);
        assert_eq!(spline.degree, degree);
        assert!(spline.fit_points.is_empty());
        // Clamped: it starts and ends on its end control points.
        let curve = spline.points(&sketch, 10).unwrap();
        assert!((curve[0] - Vec2D::new(1.0, 1.0)).to_glam().length() < 1e-5);
        assert!((curve[10] - Vec2D::new(12.0, 1.0)).to_glam().length() < 1e-5);
        let first = spline.control_points[0];
        let last = spline.control_points[5];
        line_between(&mut sketch, last, first);
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        let is_nurbs = wires[0]
            .segments
            .iter()
            .any(|s| matches!(s, kernel_api::ProfileSegment::Nurbs { .. }));
        assert_eq!(is_nurbs, nurbs, "degree {degree}");
    }
}

#[test]
fn a_spline_stored_before_degrees_is_a_cubic() {
    let json = serde_json::json!({
        "id": Uuid::new_v4(),
        "control_points": [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
        "periodic": false,
    });
    let spline: crate::sketch::BSpline = serde_json::from_value(json).unwrap();
    assert_eq!(spline.degree, 3);
    assert!(spline.is_default_cubic() && spline.fit_points.is_empty());
    let back = serde_json::to_value(&spline).unwrap();
    assert!(back.get("degree").is_none() && back.get("knots").is_none());
}

/// A line, a tangent quarter arc and a line after it: one smooth chain.
fn smooth_chain(sketch: &mut Sketch) -> (Uuid, Uuid, [Uuid; 3]) {
    let a = pt(sketch, 0.0, 0.0);
    let b = pt(sketch, 10.0, 0.0);
    let c = pt(sketch, 10.0, 5.0);
    let d = pt(sketch, 15.0, 5.0);
    let e = pt(sketch, 15.0, 15.0);
    let first = line_between(sketch, a, b);
    let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, b, d, 5.0)));
    let last = line_between(sketch, d, e);
    (a, e, [first, arc, last])
}

#[test]
fn joining_a_chain_makes_one_spline_on_its_ends_that_follows_it() {
    let mut sketch = Sketch::new("t");
    let (a, e, curves) = smooth_chain(&mut sketch);
    // Something meets the chain's end: it keeps meeting it.
    let f = pt(&mut sketch, 0.0, 15.0);
    line_between(&mut sketch, e, f);
    line_between(&mut sketch, f, a);
    let before = sketch.clone();
    let selected: HashSet<Uuid> = curves.into_iter().collect();
    let fx = join(&mut sketch, &selected, 0.01);
    assert!(fx.changed, "{:?}", fx.log);
    assert_eq!(arcs(&sketch), 0);
    assert_eq!(lines(&sketch), 2, "only the two that closed the shape");
    let spline = spline_of(&sketch);
    assert_eq!(spline.control_points[0], a);
    assert_eq!(*spline.control_points.last().unwrap(), e);
    assert!(sketch.get_geometry(curves[1]).is_none());
    // The joint points and the arc's centre went with the curves.
    assert_eq!(
        points(&sketch),
        3 + spline.control_points.len() - 2,
        "a, e, f and the inner control points"
    );
    // It follows the old chain: every sample of it lies near the spline.
    let curve = spline.points(&sketch, 4000).unwrap();
    for g in &before.geometry {
        if !selected.contains(&g.id()) {
            continue;
        }
        let samples: Vec<Vec2D> = match g {
            GeometryElement::Line(l) => {
                let (p, q) = (
                    before.point_position(l.start).unwrap(),
                    before.point_position(l.end).unwrap(),
                );
                (0..=20)
                    .map(|i| Vec2D::from_glam(p.to_glam().lerp(q.to_glam(), i as f64 / 20.0)))
                    .collect()
            }
            _ => (0..=20)
                .map(|i| {
                    let t = std::f64::consts::FRAC_PI_2 * i as f64 / 20.0;
                    Vec2D::new(10.0 + 5.0 * t.sin(), 5.0 - 5.0 * t.cos())
                })
                .collect(),
        };
        for s in samples {
            let nearest = curve
                .iter()
                .map(|p| (*p - s).to_glam().length())
                .fold(f64::MAX, f64::min);
            assert!(nearest < 0.02, "{s:?} is {nearest} from the spline");
        }
    }
    let wires = crate::profile::extract_wires(&sketch).expect("still closed");
    assert_eq!(wires[0].segments.len(), 3);
}

#[test]
fn join_refuses_branches_gaps_and_circles() {
    let mut sketch = Sketch::new("t");
    let (_, _, curves) = smooth_chain(&mut sketch);
    // A gap: the first and last lines alone do not meet.
    let apart: HashSet<Uuid> = [curves[0], curves[2]].into_iter().collect();
    let fx = join(&mut sketch, &apart, 0.01);
    assert!(!fx.changed && fx.log.is_some());
    // A circle never joins.
    let c = pt(&mut sketch, 30.0, 30.0);
    let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 2.0)));
    let with_circle: HashSet<Uuid> = [curves[0], curves[1], circle].into_iter().collect();
    assert!(!join(&mut sketch, &with_circle, 0.01).changed);
    assert_eq!(arcs(&sketch), 1, "nothing was touched");
}

/// How far each end of every conic arc is from its curve.
fn worst_end_miss(sketch: &Sketch) -> f64 {
    sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Conic(c) => Some(c),
            _ => None,
        })
        .flat_map(|c| {
            let shape = crate::conic::Shape::of(c, sketch).unwrap();
            [c.start, c.end].map(|id| {
                let p = sketch.point_position(id).unwrap();
                shape.distance([p.x, p.y]).abs()
            })
        })
        .fold(0.0, f64::max)
}

#[test]
fn an_arc_of_parabola_from_vertex_focus_and_two_ends() {
    let (mut sketch, mut state) = fresh();
    let tool = "sketch.parabola";
    for (x, y) in [(1.0, 1.0), (1.0, 3.0), (-3.0, 2.0)] {
        let fx = click(&mut state, tool, &mut sketch, (x, y), 0.1);
        assert!(!fx.changed);
    }
    let fx = click(&mut state, tool, &mut sketch, (5.0, 7.0), 0.1);
    assert!(fx.changed && state.is_idle());
    assert_eq!(points(&sketch), 3, "vertex and two ends");
    let conic = conic_of(&sketch);
    assert_eq!(conic.kind, crate::sketch::ConicKind::Parabola);
    // The ends land on the curve level with the clicks across its axis.
    let at = |id| sketch.point_position(id).unwrap();
    assert!((at(conic.start) - Vec2D::new(-3.0, 3.0)).to_glam().length() < 1e-5);
    assert!((at(conic.end) - Vec2D::new(5.0, 3.0)).to_glam().length() < 1e-5);
    assert!(worst_end_miss(&sketch) < 1e-6);

    // Closed by a line, it is a profile of an exact rational quadratic.
    line_between(&mut sketch, conic.end, conic.start);
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert!(wires[0].segments.iter().any(|s| matches!(
        s,
        kernel_api::ProfileSegment::Nurbs { degree: 2, control_points, weights, .. }
            if control_points.len() == 3 && weights.is_empty()
    )));

    // Its vertex dragged, the ends stay on the curve.
    if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(conic.center) {
        p.position = Vec2D::new(2.0, 0.0);
    }
    crate::solver::solve_holding(&mut sketch, &[conic.center]);
    assert!(
        worst_end_miss(&sketch) < 1e-4,
        "{}",
        worst_end_miss(&sketch)
    );
}

#[test]
fn an_arc_of_hyperbola_opens_as_wide_as_its_start_says() {
    let (mut sketch, mut state) = fresh();
    let tool = "sketch.hyperbola";
    click(&mut state, tool, &mut sketch, (1.0, 1.0), 0.1);
    click(&mut state, tool, &mut sketch, (4.0, 1.0), 0.1);
    // Inside the vertex no branch passes: the click is ignored.
    click(&mut state, tool, &mut sketch, (3.0, 3.0), 0.1);
    assert!(matches!(state, ToolState::ConicAxis { .. }));
    click(&mut state, tool, &mut sketch, (6.0, 3.0), 0.1);
    let fx = click(&mut state, tool, &mut sketch, (9.0, -1.0), 0.1);
    assert!(fx.changed);
    let conic = conic_of(&sketch);
    assert!((conic.minor - 1.5).abs() < 1e-4, "{}", conic.minor);
    let at = |id| sketch.point_position(id).unwrap();
    assert!((at(conic.start) - Vec2D::new(6.0, 3.0)).to_glam().length() < 1e-4);
    assert!((at(conic.end) - Vec2D::new(6.0, -1.0)).to_glam().length() < 1e-4);
    line_between(&mut sketch, conic.end, conic.start);
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert!(wires[0].segments.iter().any(|s| matches!(
        s,
        kernel_api::ProfileSegment::Nurbs { weights, .. } if weights.len() == 3 && weights[1] > 1.0
    )));
}

#[test]
fn a_conic_arc_splits_into_two_of_the_same_curve() {
    let (mut sketch, mut state) = fresh();
    for (x, y) in [(1.0, 1.0), (1.0, 3.0), (-3.0, 2.0), (5.0, 7.0)] {
        click(&mut state, "sketch.parabola", &mut sketch, (x, y), 0.1);
    }
    // At the vertex, right on the curve.
    let fx = click(&mut state, "sketch.split", &mut sketch, (1.0, 1.05), 0.1);
    assert!(fx.changed, "{:?}", fx.log);
    let halves: Vec<crate::sketch::Conic> = all_conics(&sketch);
    assert_eq!(halves.len(), 2);
    assert_eq!(halves[0].center, halves[1].center);
    assert_eq!(halves[0].end, halves[1].start);
    let middle = sketch.point_position(halves[0].end).unwrap();
    assert!((middle - Vec2D::new(1.0, 1.0)).to_glam().length() < 1e-4);
    assert!(worst_end_miss(&sketch) < 1e-6);
}

#[test]
fn a_turned_or_mirrored_conic_keeps_its_ends_on_it() {
    let (mut sketch, mut state) = fresh();
    for (x, y) in [(1.0, 1.0), (4.0, 1.0), (6.0, 3.0), (9.0, -1.0)] {
        click(&mut state, "sketch.hyperbola", &mut sketch, (x, y), 0.1);
    }
    let all: HashSet<Uuid> = sketch.geometry.iter().map(|g| g.id()).collect();
    let turn = Similarity::rotation_about(glam::DVec2::new(2.0, 2.0), 0.7);
    let copies = copy_from(&sketch.clone(), &mut sketch, &all, &turn);
    assert!(copies > 0);
    let mirror = Similarity::mirror_about(glam::DVec2::ZERO, glam::DVec2::Y);
    copy_from(&sketch.clone(), &mut sketch, &all, &mirror);
    let conics = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Conic(_)))
        .count();
    assert_eq!(conics, 3);
    assert!(
        worst_end_miss(&sketch) < 1e-4,
        "{}",
        worst_end_miss(&sketch)
    );
}

#[test]
fn a_mirror_about_a_point_can_move_the_original_or_keep_a_linked_image() {
    let subject = |sketch: &mut Sketch| {
        let a = pt(sketch, 2.0, 2.0);
        let b = pt(sketch, 8.0, 5.0);
        let l = line_between(sketch, a, b);
        (a, l)
    };
    let find = |sketch: &Sketch, x: f64, y: f64| {
        sketch.geometry.iter().find_map(|g| match g {
            GeometryElement::Point(p)
                if (p.position.to_glam() - glam::DVec2::new(x, y)).length() < 1e-3 =>
            {
                Some(p.id)
            }
            _ => None,
        })
    };
    // Through the origin, the original taken away: it has moved.
    let mut sketch = Sketch::new("t");
    let (_, l) = subject(&mut sketch);
    let selected: HashSet<Uuid> = [l].into();
    let params = ToolParams {
        mirror_center: true,
        mirror_keep: false,
        ..ToolParams::default()
    };
    let mut state = ToolState::default();
    let w = With::new(&params, &selected, 0.1);
    let fx = w.click(&mut state, &mut sketch, "sketch.mirror", (0.0, 0.0));
    assert!(fx.changed);
    assert_eq!(lines(&sketch), 1, "only the image is left");
    assert!(find(&sketch, -2.0, -2.0).is_some() && find(&sketch, 2.0, 2.0).is_none());

    // Linked: moving the original's end moves the image's with it.
    let mut sketch = Sketch::new("t");
    let (a, l) = subject(&mut sketch);
    let selected: HashSet<Uuid> = [l].into();
    let params = ToolParams {
        mirror_center: true,
        mirror_linked: true,
        ..ToolParams::default()
    };
    let mut state = ToolState::default();
    let w = With::new(&params, &selected, 0.1);
    w.click(&mut state, &mut sketch, "sketch.mirror", (0.0, 0.0));
    let image = find(&sketch, -2.0, -2.0).expect("the image of the first end");
    sketch.add_constraint(ConstraintKind::FixedPoint {
        point: a,
        position: Vec2D::new(3.0, 1.0),
    });
    crate::solver::solve(&mut sketch);
    let moved = sketch.point_position(image).unwrap().to_glam();
    assert!(
        (moved - glam::DVec2::new(-3.0, -1.0)).length() < 1e-3,
        "the image followed: {moved:?}"
    );
}

#[test]
fn a_scale_with_copies_keeps_the_original_and_scales_each_copy_again() {
    let (mut sketch, mut state) = fresh();
    let center = pt(&mut sketch, 4.0, 0.0);
    let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
    let selected: HashSet<Uuid> = [circle].into();
    let params = params(|p| p.copies = 2);
    for x in [0.0, 10.0, 20.0] {
        let w = With::new(&params, &selected, 0.1);
        w.click(&mut state, &mut sketch, "sketch.scale", (x, 0.0));
    }
    let mut radii: Vec<f64> = all_circles(&sketch).iter().map(|c| c.radius).collect();
    radii.sort_by(f64::total_cmp);
    assert_eq!(radii.len(), 3, "the original and two copies");
    for (got, want) in radii.iter().zip([2.0, 4.0, 8.0]) {
        assert!((got - want).abs() < 1e-3, "{radii:?}");
    }
}

#[test]
fn a_polyline_arc_can_leave_square_to_the_last_segment_or_back_along_it() {
    let arc_center = |segment: PolySegment| {
        let (mut sketch, mut state) = fresh();
        let click = |state: &mut ToolState, sketch: &mut Sketch, x: f64, y: f64| {
            click(state, "sketch.polyline", sketch, (x, y), 0.2).changed
        };
        click(&mut state, &mut sketch, 0.0, 0.0);
        click(&mut state, &mut sketch, 10.0, 0.0);
        set_polyline_segment(&mut state, segment);
        assert!(click(&mut state, &mut sketch, 20.0, 0.0));
        let arc = arc_of(&sketch);
        (
            sketch.point_position(arc.center).unwrap(),
            arc.radius,
            sketch,
        )
    };
    // Leaving (10, 0) straight up and ending at (20, 0): a half circle
    // about (15, 0).
    let (c, r, sketch) = arc_center(PolySegment::Perpendicular);
    assert!((c.x - 15.0).abs() < 1e-4 && c.y.abs() < 1e-4 && (r - 5.0).abs() < 1e-4);
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::AngleAtPoint { angle_rad, .. } if (angle_rad.to_degrees().abs() - 90.0).abs() < 1e-3)),
        "held square to the line"
    );
    // Leaving (10, 0) back along -x and ending at (10, 8): a turn about
    // (10, 4).
    let (mut sketch, mut state) = fresh();
    click(&mut state, "sketch.polyline", &mut sketch, (0.0, 0.0), 0.2);
    click(&mut state, "sketch.polyline", &mut sketch, (10.0, 0.0), 0.2);
    set_polyline_segment(&mut state, PolySegment::Reverse);
    let made = click(&mut state, "sketch.polyline", &mut sketch, (10.0, 8.0), 0.2);
    assert!(made.changed);
    let arc = arc_of(&sketch);
    let c = sketch.point_position(arc.center).unwrap();
    assert!(
        (c.x - 10.0).abs() < 1e-4 && (c.y - 4.0).abs() < 1e-4,
        "{c:?}"
    );
    assert!((arc.radius - 4.0).abs() < 1e-4);
}

fn the_arc(sketch: &Sketch) -> (glam::DVec2, Arc) {
    let arc = sketch
        .geometry
        .iter()
        .rev()
        .find_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    (sketch.point_position(arc.center).unwrap().to_glam(), arc)
}

#[test]
fn a_fillet_rounds_the_corner_of_a_line_and_an_arc() {
    // A D: the upper half of a circle of 10 closed by its diameter.
    let (mut sketch, mut state) = fresh();
    let c = pt(&mut sketch, 0.0, 0.0);
    let r = pt(&mut sketch, 10.0, 0.0);
    let l = pt(&mut sketch, -10.0, 0.0);
    line_between(&mut sketch, l, r);
    sketch.add_geometry(GeometryElement::Arc(Arc::new(c, r, l, 10.0)));
    let w = With::new(&params(|p| p.fillet_radius = 2.0), &HashSet::new(), 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.fillet", (10.0, 0.0));
    assert!(fx.changed, "{:?}", fx.log);
    let (centre, _) = the_arc(&sketch);
    // Inside the D: 2 above the line, 8 from the circle's centre.
    assert!((centre.y - 2.0).abs() < 1e-4, "{centre:?}");
    assert!((centre.length() - 8.0).abs() < 1e-4, "{centre:?}");
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 3);
    converged(&mut sketch);
}

#[test]
fn a_fillet_joins_two_lines_that_do_not_meet() {
    // An L with its corner missing: each line stops short of it.
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, 3.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let c = pt(&mut sketch, 0.0, 4.0);
    let d = pt(&mut sketch, 0.0, 10.0);
    let bottom = line_between(&mut sketch, a, b);
    let side = line_between(&mut sketch, c, d);
    let params = params(|p| p.fillet_radius = 1.0);
    let w = With::new(&params, &HashSet::new(), 0.5);
    let first = w.click(&mut state, &mut sketch, "sketch.fillet", (8.0, 0.0));
    assert!(!first.changed);
    assert!(matches!(state, ToolState::CornerFirst { .. }));
    let fx = w.click(&mut state, &mut sketch, "sketch.fillet", (0.0, 8.0));
    assert!(fx.changed, "{:?}", fx.log);
    assert!(state.is_idle());
    let (centre, arc) = the_arc(&sketch);
    assert!(
        (centre - glam::DVec2::new(1.0, 1.0)).length() < 1e-4,
        "{centre:?}"
    );
    // Each line runs on to the arc: the bottom is extended back to x = 1,
    // the side down to y = 1, and they share the arc's ends.
    let ends = |id: Uuid| match sketch.get_geometry(id) {
        Some(GeometryElement::Line(l)) => (l.start, l.end),
        _ => panic!(),
    };
    let (bs, _) = ends(bottom);
    let (ss, _) = ends(side);
    assert!([arc.start, arc.end].contains(&bs));
    assert!([arc.start, arc.end].contains(&ss));
    let at = |p: Uuid| sketch.point_position(p).unwrap().to_glam();
    assert!((at(bs) - glam::DVec2::new(1.0, 0.0)).length() < 1e-4);
    assert!((at(ss) - glam::DVec2::new(0.0, 1.0)).length() < 1e-4);
    // The old ends are gone.
    assert!(sketch.get_geometry(a).is_none() && sketch.get_geometry(c).is_none());
}

#[test]
fn a_fillet_trims_lines_that_cross() {
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, -5.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let c = pt(&mut sketch, 0.0, -5.0);
    let d = pt(&mut sketch, 0.0, 10.0);
    line_between(&mut sketch, a, b);
    line_between(&mut sketch, c, d);
    let params = params(|p| p.fillet_radius = 2.0);
    let w = With::new(&params, &HashSet::new(), 0.5);
    w.clicks(
        &mut state,
        &mut sketch,
        "sketch.fillet",
        [Vec2D::new(8.0, 0.0), Vec2D::new(0.0, 8.0)],
    );
    let (centre, _) = the_arc(&sketch);
    assert!(
        (centre - glam::DVec2::new(2.0, 2.0)).length() < 1e-4,
        "{centre:?}"
    );
    // The parts past the crossing are cut away.
    let lowest = all_points(&sketch)
        .into_iter()
        .map(|p| p.position)
        .fold(f64::MAX, |m, p| m.min(p.x).min(p.y));
    assert!(lowest > -1e-4, "{lowest}");
}

#[test]
fn a_fillet_touches_a_circle_and_leaves_it_whole() {
    let (mut sketch, mut state) = fresh();
    let o = pt(&mut sketch, 0.0, 0.0);
    let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(o, 5.0)));
    let a = pt(&mut sketch, 8.0, -10.0);
    let b = pt(&mut sketch, 8.0, 10.0);
    line_between(&mut sketch, a, b);
    let params = params(|p| p.fillet_radius = 2.0);
    let w = With::new(&params, &HashSet::new(), 0.5);
    w.click(&mut state, &mut sketch, "sketch.fillet", (0.0, 5.0));
    let fx = w.click(&mut state, &mut sketch, "sketch.fillet", (8.0, 8.0));
    assert!(fx.changed, "{:?}", fx.log);
    let (centre, _) = the_arc(&sketch);
    assert!((centre.length() - 7.0).abs() < 1e-3, "{centre:?}");
    assert!((centre.x - 6.0).abs() < 1e-3, "{centre:?}");
    assert!(centre.y > 0.0, "on the side of the line's pick: {centre:?}");
    assert!(matches!(
        sketch.get_geometry(circle),
        Some(GeometryElement::Circle(_))
    ));
}

#[test]
fn a_chamfer_joins_two_lines_that_do_not_meet() {
    let (mut sketch, mut state) = fresh();
    let a = pt(&mut sketch, 3.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let c = pt(&mut sketch, 0.0, 4.0);
    let d = pt(&mut sketch, 0.0, 10.0);
    line_between(&mut sketch, a, b);
    line_between(&mut sketch, c, d);
    let params = ToolParams {
        chamfer_length: 2.0,
        corner_keep: true,
        ..ToolParams::default()
    };
    let w = With::new(&params, &HashSet::new(), 0.5);
    w.clicks(
        &mut state,
        &mut sketch,
        "sketch.chamfer",
        [Vec2D::new(8.0, 0.0), Vec2D::new(0.0, 8.0)],
    );
    assert_eq!(lines(&sketch), 3);
    assert!(has_point(&sketch, 2.0, 0.0) && has_point(&sketch, 0.0, 2.0));
    // The corner is kept, as construction, on both lines.
    let corner = sketch
        .geometry
        .iter()
        .find(|g| matches!(g, GeometryElement::Point(p) if p.position.to_glam().length() < 1e-4))
        .map(|g| g.id())
        .expect("the corner kept");
    assert!(sketch.is_construction(corner));
    assert_eq!(
        sketch
            .constraints
            .iter()
            .filter(
                |c| matches!(c.kind, ConstraintKind::PointOnLine { point, .. } if point == corner)
            )
            .count(),
        2
    );
}

#[test]
fn a_fillet_can_keep_the_corner_and_what_holds_it() {
    let (mut sketch, mut state) = fresh();
    let [.., c, _] = build_rectangle(&mut sketch, 12.0, 8.0);
    pin(&mut sketch, c);
    let params = ToolParams {
        fillet_radius: 2.0,
        corner_keep: true,
        ..ToolParams::default()
    };
    let w = With::new(&params, &HashSet::new(), 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.fillet", (12.0, 8.0));
    assert!(fx.changed);
    assert!(sketch.is_construction(c));
    assert!(sketch.constraints.iter().any(|con| matches!(
        con.kind,
        ConstraintKind::FixedPoint { point, .. } if point == c
    )));
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    converged(&mut sketch);
}

fn offset_params(f: impl FnOnce(&mut ToolParams)) -> ToolParams {
    let mut p = ToolParams {
        offset_distance: 1.0,
        ..ToolParams::default()
    };
    f(&mut p);
    p
}

/// A 10 × 5 rectangle, its lines selected.
fn selected_rectangle(sketch: &mut Sketch) -> HashSet<Uuid> {
    build_rectangle(sketch, 10.0, 5.0);
    sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Line(_)))
        .map(|g| g.id())
        .collect()
}

#[test]
fn an_offset_outward_can_round_its_corners() {
    let (mut sketch, mut state) = fresh();
    let selected = selected_rectangle(&mut sketch);
    let w = With::new(&offset_params(|p| p.offset_round = true), &selected, 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.offset", (5.0, -3.0));
    assert!(fx.changed);
    // Four copies and four quarter arcs about the original corners.
    assert_eq!((lines(&sketch), arcs(&sketch)), (8, 4));
    for g in &sketch.geometry {
        if let GeometryElement::Arc(a) = g {
            assert!((a.radius - 1.0).abs() < 1e-5);
            let c = sketch.point_position(a.center).unwrap().to_glam();
            assert!(
                [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)]
                    .iter()
                    .any(|&(x, y)| (c - glam::DVec2::new(x, y)).length() < 1e-5)
            );
        }
    }
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 2);
    converged(&mut sketch);
}

#[test]
fn an_inward_offset_does_not_round_its_corners() {
    let (mut sketch, mut state) = fresh();
    let selected = selected_rectangle(&mut sketch);
    let w = With::new(&offset_params(|p| p.offset_round = true), &selected, 0.5);
    w.click(&mut state, &mut sketch, "sketch.offset", (5.0, 2.5));
    assert_eq!((lines(&sketch), arcs(&sketch)), (8, 0));
}

#[test]
fn an_offset_to_both_sides_replacing_the_original() {
    let (mut sketch, mut state) = fresh();
    let selected = selected_rectangle(&mut sketch);
    let w = With {
        params: offset_params(|p| {
            p.offset_both = true;
            p.offset_delete = true;
        }),
        selected: selected.clone(),
        tol: 0.5,
    };
    w.click(&mut state, &mut sketch, "sketch.offset", (5.0, 2.5));
    assert_eq!(lines(&sketch), 8);
    assert!(selected.iter().all(|id| sketch.get_geometry(*id).is_none()));
    let xs: Vec<f64> = all_points(&sketch)
        .into_iter()
        .map(|p| p.position.x)
        .collect();
    assert!(xs.iter().any(|x| (x - 1.0).abs() < 1e-4));
    assert!(xs.iter().any(|x| (x + 1.0).abs() < 1e-4));
    assert!(
        !xs.iter().any(|x| x.abs() < 1e-4),
        "the original corners are gone"
    );
}

#[test]
fn a_linked_offset_follows_its_dimension() {
    let (mut sketch, mut state) = fresh();
    let selected = selected_rectangle(&mut sketch);
    for g in sketch.geometry.clone() {
        if let GeometryElement::Point(p) = g {
            pin(&mut sketch, p.id);
        }
    }
    let w = With {
        params: offset_params(|p| {
            p.offset_linked = true;
            p.offset_round = true;
        }),
        selected: selected.clone(),
        tol: 0.5,
    };
    w.click(&mut state, &mut sketch, "sketch.offset", (5.0, -3.0));
    let offset = sketch
        .constraints
        .iter()
        .position(|c| matches!(c.kind, ConstraintKind::Offset { .. }))
        .expect("one offset dimension");
    let ConstraintKind::Offset { pairs, .. } = &sketch.constraints[offset].kind else {
        unreachable!()
    };
    assert_eq!(pairs.len(), 4);
    let kind = crate::sketch::with_dimension_value(&sketch.constraints[offset].kind, 2.5);
    sketch.constraints[offset].kind = kind;
    converged(&mut sketch);
    // Every copy stands 2.5 out, the corners round with radius 2.5.
    let lowest = all_points(&sketch)
        .into_iter()
        .map(|p| p.position.y)
        .fold(f64::MAX, f64::min);
    assert!((lowest + 2.5).abs() < 1e-3, "{lowest}");
    for g in &sketch.geometry {
        if let GeometryElement::Arc(a) = g {
            let c = sketch.point_position(a.center).unwrap();
            let s = sketch.point_position(a.start).unwrap();
            assert!(((s - c).to_glam().length() - 2.5).abs() < 1e-3);
        }
    }
}

#[test]
fn an_ellipse_offsets_as_a_closed_spline() {
    let (mut sketch, mut state) = fresh();
    let c = pt(&mut sketch, 0.0, 0.0);
    let ellipse = sketch.add_geometry(GeometryElement::Ellipse(crate::sketch::Ellipse::new(
        c,
        Vec2D::new(10.0, 0.0),
        0.5,
    )));
    let selected: HashSet<Uuid> = [ellipse].into_iter().collect();
    let w = With::new(&offset_params(|p| p.offset_distance = 2.0), &selected, 0.5);
    let fx = w.click(&mut state, &mut sketch, "sketch.offset", (0.0, 9.0));
    assert!(fx.changed, "{:?}", fx.log);
    let spline = spline_of(&sketch).id;
    let samples = crate::measure::curve_samples(&sketch, spline).unwrap();
    // Outside the ellipse, about 2 from it: at its ends 12 out along x and
    // 7 up at its top.
    let max_x = samples.iter().map(|p| p.x).fold(f64::MIN, f64::max);
    let max_y = samples.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    assert!((max_x - 12.0).abs() < 0.1, "{max_x}");
    assert!((max_y - 7.0).abs() < 0.1, "{max_y}");
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 2, "the copy is closed");
}

/// An ellipse of radii 10 and 5 about the origin, and a vertical line
/// through x = 4 across it.
fn ellipse_and_line(sketch: &mut Sketch, arc: bool) -> (Uuid, Uuid) {
    let c = pt(sketch, 0.0, 0.0);
    let ellipse = if arc {
        // The upper half, from (10, 0) round to (-10, 0).
        let s = pt(sketch, 10.0, 0.0);
        let e = pt(sketch, -10.0, 0.0);
        crate::sketch::Ellipse::new_arc(c, Vec2D::new(10.0, 0.0), 0.5, s, e)
    } else {
        crate::sketch::Ellipse::new(c, Vec2D::new(10.0, 0.0), 0.5)
    };
    let ellipse = sketch.add_geometry(GeometryElement::Ellipse(ellipse));
    let a = pt(sketch, 4.0, -8.0);
    let b = pt(sketch, 4.0, 8.0);
    let line = line_between(sketch, a, b);
    (ellipse, line)
}

fn ellipse_arc_span(sketch: &Sketch, id: Uuid) -> Option<(f64, f64)> {
    match sketch.get_geometry(id)? {
        GeometryElement::Ellipse(e) => e.param_span(sketch),
        _ => None,
    }
}

#[test]
fn trimming_a_whole_ellipse_leaves_an_arc_of_it() {
    let (mut sketch, mut state) = fresh();
    let (ellipse, _) = ellipse_and_line(&mut sketch, false);
    // Click its right end, past the line: that part goes.
    let fx = click(&mut state, "sketch.trim", &mut sketch, (10.0, 0.0), 0.5);
    assert!(fx.changed, "{:?}", fx.log);
    let (t0, t1) = ellipse_arc_span(&sketch, ellipse).unwrap();
    let x_at = |t: f64| 10.0 * t.cos();
    // What stays runs from the line's top crossing round the left.
    assert!((x_at(t0) - 4.0).abs() < 1e-3 && (x_at(t1) - 4.0).abs() < 1e-3);
    let mid = (t0 + t1) * 0.5;
    assert!(x_at(mid) < -9.0, "the left side stays");
}

#[test]
fn trimming_an_arc_of_an_ellipse_shortens_it() {
    let (mut sketch, mut state) = fresh();
    let (ellipse, _) = ellipse_and_line(&mut sketch, true);
    let near_right = Vec2D::new(10.0 * 0.3f64.cos(), 5.0 * 0.3f64.sin());
    let fx = click(&mut state, "sketch.trim", &mut sketch, near_right, 0.5);
    assert!(fx.changed, "{:?}", fx.log);
    let (t0, t1) = ellipse_arc_span(&sketch, ellipse).unwrap();
    assert!((10.0 * t0.cos() - 4.0).abs() < 1e-3, "starts at the line");
    assert!(
        (t1 - std::f64::consts::PI).abs() < 1e-3,
        "still ends at (-10, 0)"
    );
    converged(&mut sketch);
}

#[test]
fn splitting_an_arc_of_an_ellipse_makes_two_that_meet() {
    let (mut sketch, mut state) = fresh();
    let (ellipse, _) = ellipse_and_line(&mut sketch, true);
    let top = Vec2D::new(0.0, 5.0);
    let fx = click(&mut state, "sketch.split", &mut sketch, top, 0.5);
    assert!(fx.changed, "{:?}", fx.log);
    let arcs: Vec<_> = all_ellipses(&sketch);
    assert_eq!(arcs.len(), 2);
    let first = arcs.iter().find(|e| e.id == ellipse).unwrap();
    let second = arcs.iter().find(|e| e.id != ellipse).unwrap();
    assert_eq!(first.arc.unwrap().end, second.arc.unwrap().start);
}

#[test]
fn extending_an_arc_of_an_ellipse_to_a_line() {
    let (mut sketch, mut state) = fresh();
    let c = pt(&mut sketch, 0.0, 0.0);
    // A quarter from (10, 0) to the top (0, 5).
    let s = pt(&mut sketch, 10.0, 0.0);
    let e = pt(&mut sketch, 0.0, 5.0);
    let ellipse = sketch.add_geometry(GeometryElement::Ellipse(crate::sketch::Ellipse::new_arc(
        c,
        Vec2D::new(10.0, 0.0),
        0.5,
        s,
        e,
    )));
    let a = pt(&mut sketch, -6.0, -8.0);
    let b = pt(&mut sketch, -6.0, 8.0);
    line_between(&mut sketch, a, b);
    let near_end = Vec2D::new(10.0 * 1.4f64.cos(), 5.0 * 1.4f64.sin());
    let fx = click(&mut state, "sketch.extend", &mut sketch, near_end, 0.5);
    assert!(fx.changed, "{:?}", fx.log);
    let end = sketch.point_position(e).unwrap();
    assert!((end.x + 6.0).abs() < 1e-3 && end.y > 0.0, "{end:?}");
    let (_, t1) = ellipse_arc_span(&sketch, ellipse).unwrap();
    assert!(t1 > std::f64::consts::FRAC_PI_2);
}

/// An open cubic spline from (0, 0) to (30, 0) and a vertical line through
/// x = 15 across it.
fn spline_and_line(sketch: &mut Sketch, periodic: bool) -> (Uuid, Vec<Uuid>) {
    let pts: Vec<Uuid> = [
        (0.0, 0.0),
        (8.0, 10.0),
        (15.0, -6.0),
        (22.0, 10.0),
        (30.0, 0.0),
    ]
    .iter()
    .map(|&(x, y)| pt(sketch, x, y))
    .collect();
    let spline = sketch.add_geometry(GeometryElement::BSpline(crate::sketch::BSpline::new(
        pts.clone(),
        periodic,
    )));
    let a = pt(sketch, 15.0, -20.0);
    let b = pt(sketch, 15.0, 20.0);
    line_between(sketch, a, b);
    (spline, pts)
}

fn spline_samples(sketch: &Sketch, id: Uuid) -> Vec<glam::DVec2> {
    crate::measure::curve_samples(sketch, id).unwrap()
}

#[test]
fn trimming_a_spline_keeps_the_rest_of_the_same_curve() {
    let (mut sketch, mut state) = fresh();
    let (spline, pts) = spline_and_line(&mut sketch, false);
    let before = spline_samples(&sketch, spline);
    // Click near its far end, right of the line.
    let near_end = before[before.len() - 8];
    let fx = click(
        &mut state,
        "sketch.trim",
        &mut sketch,
        Vec2D::from_glam(near_end),
        0.5,
    );
    assert!(fx.changed, "{:?}", fx.log);
    let after = spline_samples(&sketch, spline);
    // It still starts where it did and now stops on the line.
    let GeometryElement::BSpline(b) = sketch.get_geometry(spline).unwrap() else {
        panic!()
    };
    assert_eq!(b.control_points[0], pts[0]);
    assert!((after.last().unwrap().x - 15.0).abs() < 1e-3);
    // Every point of what stays lies on the original curve.
    for p in &after {
        let d = before
            .windows(2)
            .map(|w| {
                let d = w[1] - w[0];
                let t = ((*p - w[0]).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
                (*p - (w[0] + d * t)).length()
            })
            .fold(f64::MAX, f64::min);
        assert!(d < 0.05, "{p:?} off the curve by {d}");
    }
    assert!(
        sketch.get_geometry(*pts.last().unwrap()).is_none(),
        "the cut-off end goes"
    );
}

#[test]
fn splitting_a_spline_makes_two_meeting_at_one_point() {
    let (mut sketch, mut state) = fresh();
    let (spline, pts) = spline_and_line(&mut sketch, false);
    let before = spline_samples(&sketch, spline);
    let at = before[before.len() / 3];
    let fx = click(
        &mut state,
        "sketch.split",
        &mut sketch,
        Vec2D::from_glam(at),
        0.5,
    );
    assert!(fx.changed, "{:?}", fx.log);
    let splines: Vec<_> = all_splines(&sketch);
    assert_eq!(splines.len(), 2);
    let first = splines.iter().find(|b| b.id == spline).unwrap();
    let second = splines.iter().find(|b| b.id != spline).unwrap();
    assert_eq!(first.control_points[0], pts[0]);
    assert_eq!(second.control_points.last(), pts.last());
    assert_eq!(first.control_points.last(), second.control_points.first());
    let joint = sketch
        .point_position(*first.control_points.last().unwrap())
        .unwrap()
        .to_glam();
    assert!((joint - at).length() < 0.1, "{joint:?} vs {at:?}");
}

#[test]
fn trimming_a_closed_spline_opens_it() {
    let (mut sketch, mut state) = fresh();
    let (spline, _) = spline_and_line(&mut sketch, true);
    let before = spline_samples(&sketch, spline);
    let right = before
        .iter()
        .copied()
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap();
    let fx = click(
        &mut state,
        "sketch.trim",
        &mut sketch,
        Vec2D::from_glam(right),
        0.5,
    );
    assert!(fx.changed, "{:?}", fx.log);
    let GeometryElement::BSpline(b) = sketch.get_geometry(spline).unwrap() else {
        panic!()
    };
    assert!(!b.periodic);
    let after = spline_samples(&sketch, spline);
    assert!(
        after.iter().all(|p| p.x < 15.0 + 1e-3),
        "the right side is gone"
    );
    for end in [after[0], *after.last().unwrap()] {
        assert!((end.x - 15.0).abs() < 1e-3, "{end:?}");
    }
}

#[test]
fn a_line_trims_back_to_an_ellipse_it_crosses() {
    let (mut sketch, mut state) = fresh();
    let (_, line) = ellipse_and_line(&mut sketch, false);
    // The line's top, outside the ellipse, goes.
    let fx = click(&mut state, "sketch.trim", &mut sketch, (4.0, 7.0), 0.5);
    assert!(fx.changed, "{:?}", fx.log);
    let GeometryElement::Line(l) = sketch.get_geometry(line).unwrap() else {
        panic!()
    };
    let end = sketch.point_position(l.end).unwrap();
    let y = 5.0 * (1.0f64 - 0.16).sqrt();
    assert!((end.y - y).abs() < 1e-3, "{end:?}");
}

#[test]
fn trimming_a_parabola_moves_its_end_to_the_crossing() {
    let (mut sketch, mut state) = fresh();
    let v = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 9.0, -6.0);
    let e = pt(&mut sketch, 9.0, 6.0);
    let parabola = sketch.add_geometry(GeometryElement::Conic(crate::sketch::Conic::new(
        crate::sketch::ConicKind::Parabola,
        v,
        Vec2D::new(1.0, 0.0),
        0.0,
        s,
        e,
    )));
    let a = pt(&mut sketch, 4.0, -10.0);
    let b = pt(&mut sketch, 4.0, 10.0);
    line_between(&mut sketch, a, b);
    let fx = click(&mut state, "sketch.trim", &mut sketch, (6.25, 5.0), 0.5);
    assert!(fx.changed, "{:?}", fx.log);
    let GeometryElement::Conic(k) = sketch.get_geometry(parabola).unwrap() else {
        panic!()
    };
    assert_eq!(k.start, s);
    let end = sketch.point_position(k.end).unwrap();
    assert!(
        (end.x - 4.0).abs() < 1e-3 && (end.y - 4.0).abs() < 1e-3,
        "{end:?}"
    );
    assert!(sketch.get_geometry(e).is_none());
}

fn converged(sketch: &mut Sketch) {
    let outcome = crate::solver::solve(sketch);
    assert!(
        matches!(outcome, crate::solver::SolveOutcome::Converged { .. }),
        "{outcome:?}"
    );
}

fn set_dimension(sketch: &mut Sketch, pick: impl Fn(&ConstraintKind) -> bool, value: f64) {
    let c = sketch
        .constraints
        .iter_mut()
        .find(|c| pick(&c.kind))
        .expect("the dimension");
    c.kind = crate::sketch::with_dimension_value(&c.kind, value);
}

/// A circle of radius 2 about (0, 0), its centre fixed, selected.
fn fixed_circle(sketch: &mut Sketch, x: f64, y: f64) -> (Uuid, Uuid, HashSet<Uuid>) {
    let c = pt(sketch, x, y);
    pin(sketch, c);
    let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 2.0)));
    sketch.add_constraint(ConstraintKind::Radius {
        circle,
        radius: 2.0,
    });
    (c, circle, [circle].into_iter().collect())
}

fn circle_centres(sketch: &Sketch) -> Vec<glam::DVec2> {
    sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Circle(c) => sketch.point_position(c.center).map(|p| p.to_glam()),
            _ => None,
        })
        .collect()
}

#[test]
fn linked_translated_copies_follow_the_original_and_one_pitch() {
    let (mut sketch, mut state) = fresh();
    let (_, circle, selected) = fixed_circle(&mut sketch, 0.0, 0.0);
    let params = ToolParams {
        copies: 3,
        copies_linked: true,
        ..ToolParams::default()
    };
    let w = With::new(&params, &selected, 0.01);
    w.clicks(
        &mut state,
        &mut sketch,
        "sketch.translate",
        [Vec2D::new(0.0, 0.0), Vec2D::new(10.0, 0.0)],
    );
    assert_eq!(circle_centres(&sketch).len(), 4);
    set_dimension(
        &mut sketch,
        |k| matches!(k, ConstraintKind::Pitch { .. }),
        15.0,
    );
    set_dimension(
        &mut sketch,
        |k| matches!(k, ConstraintKind::Radius { circle: c, .. } if *c == circle),
        3.0,
    );
    converged(&mut sketch);
    let mut xs: Vec<f64> = circle_centres(&sketch).iter().map(|p| p.x).collect();
    xs.sort_by(f64::total_cmp);
    for (x, want) in xs.iter().zip([0.0, 15.0, 30.0, 45.0]) {
        assert!((x - want).abs() < 1e-3, "{xs:?}");
    }
    for g in &sketch.geometry {
        if let GeometryElement::Circle(c) = g {
            assert!((c.radius - 3.0).abs() < 1e-3);
        }
    }
}

#[test]
fn linked_rotated_copies_turn_by_one_angle() {
    let (mut sketch, mut state) = fresh();
    let o = pt(&mut sketch, 0.0, 0.0);
    pin(&mut sketch, o);
    let (_, _, selected) = fixed_circle(&mut sketch, 10.0, 0.0);
    let params = ToolParams {
        copies: 2,
        copies_linked: true,
        ..ToolParams::default()
    };
    With::new(&params, &selected, 0.01).clicks(
        &mut state,
        &mut sketch,
        "sketch.rotate",
        [
            Vec2D::new(0.0, 0.0),
            Vec2D::new(10.0, 0.0),
            Vec2D::new(0.0, 10.0),
        ],
    );
    let pitch = sketch
        .constraints
        .iter()
        .find_map(|c| match &c.kind {
            ConstraintKind::PolarPitch {
                center,
                points,
                angle_rad,
            } => Some((*center, points.len(), *angle_rad)),
            _ => None,
        })
        .expect("an angular pitch");
    assert_eq!(pitch.0, o, "turns about the point at the pivot");
    assert_eq!(pitch.1, 3);
    assert!((pitch.2 - std::f64::consts::FRAC_PI_2).abs() < 1e-4);
    set_dimension(
        &mut sketch,
        |k| matches!(k, ConstraintKind::PolarPitch { .. }),
        60.0,
    );
    converged(&mut sketch);
    let mut angles: Vec<f64> = circle_centres(&sketch)
        .iter()
        .map(|p| {
            assert!((p.length() - 10.0).abs() < 1e-3);
            p.y.atan2(p.x).to_degrees()
        })
        .collect();
    angles.sort_by(f64::total_cmp);
    for (a, want) in angles.iter().zip([0.0, 60.0, 120.0]) {
        assert!((a - want).abs() < 1e-2, "{angles:?}");
    }
}

#[test]
fn a_linked_array_is_spaced_by_its_two_pitches() {
    let mut sketch = Sketch::new("t");
    let (_, _, selected) = fixed_circle(&mut sketch, 0.0, 0.0);
    let effect = crate::tools::array(&mut sketch, &selected, 2, 3, 10.0, 8.0, true);
    assert!(effect.changed);
    assert_eq!(circle_centres(&sketch).len(), 6);
    set_dimension(
        &mut sketch,
        |k| matches!(k, ConstraintKind::Pitch { across: false, .. }),
        12.0,
    );
    set_dimension(
        &mut sketch,
        |k| matches!(k, ConstraintKind::Pitch { across: true, .. }),
        5.0,
    );
    converged(&mut sketch);
    let centres = circle_centres(&sketch);
    for want in [
        (0.0, 0.0),
        (12.0, 0.0),
        (24.0, 0.0),
        (0.0, 5.0),
        (12.0, 5.0),
        (24.0, 5.0),
    ] {
        let want = glam::DVec2::new(want.0, want.1);
        assert!(
            centres.iter().any(|c| (*c - want).length() < 1e-3),
            "{want:?} in {centres:?}"
        );
    }
}

/// A 3-4-5 triangle's corners, its sides grouped from a selection as the
/// toolbar's Group does.
fn grouped_triangle(sketch: &mut Sketch) -> ([Uuid; 3], [Uuid; 3]) {
    let corners = [
        pt(sketch, 0.0, 0.0),
        pt(sketch, 4.0, 0.0),
        pt(sketch, 0.0, 3.0),
    ];
    let sides = [
        line_between(sketch, corners[0], corners[1]),
        line_between(sketch, corners[1], corners[2]),
        line_between(sketch, corners[2], corners[0]),
    ];
    let selected: HashSet<Uuid> = sides.into_iter().collect();
    let shape = crate::constrain::SelectionShape::of(sketch, &selected);
    let kinds = crate::constrain::kinds_for("group", &shape, sketch).expect("sides group");
    assert_eq!(kinds.len(), 1);
    for kind in kinds {
        sketch.add_constraint(kind);
    }
    (corners, sides)
}

fn side_lengths(sketch: &Sketch, corners: [Uuid; 3]) -> [f64; 3] {
    let at = |i: usize| sketch.point_position(corners[i]).unwrap().to_glam();
    [
        at(0).distance(at(1)),
        at(1).distance(at(2)),
        at(2).distance(at(0)),
    ]
}

#[test]
fn a_group_made_from_a_selection_has_three_degrees_of_freedom_and_drags_whole() {
    let mut sketch = Sketch::new("t");
    let (corners, sides) = grouped_triangle(&mut sketch);
    assert_eq!(crate::solver::dof_estimate(&sketch), 3);

    // A drag of one corner carries every member by the same step.
    let before: Vec<Vec2D> = corners
        .iter()
        .map(|c| sketch.point_position(*c).unwrap())
        .collect();
    let targets = crate::step::drag_targets(&sketch, &[corners[0]]);
    assert_eq!(targets.len(), 3, "every member's point is carried");
    crate::step::drag(&mut sketch, &targets, Vec2D::new(5.0, 1.0));
    let held: Vec<Uuid> = targets.iter().map(|(id, _)| *id).collect();
    let outcome = crate::solver::solve_holding(&mut sketch, &held);
    assert!(matches!(
        outcome,
        crate::solver::SolveOutcome::Converged { .. }
    ));
    for (corner, was) in corners.iter().zip(before) {
        let now = sketch.point_position(*corner).unwrap();
        assert!((now.x - was.x - 5.0).abs() < 1e-4 && (now.y - was.y - 1.0).abs() < 1e-4);
    }

    // An angle on one side turns the whole group, its shape kept.
    sketch.add_constraint(ConstraintKind::AngleToAxis {
        line: sides[0],
        axis: crate::sketch::AxisDirection::Horizontal,
        angle_rad: 0.5,
    });
    converged(&mut sketch);
    let [a, b, c] = side_lengths(&sketch, corners);
    assert!((a - 4.0).abs() < 1e-4 && (b - 5.0).abs() < 1e-4 && (c - 3.0).abs() < 1e-4);
    assert_eq!(crate::solver::dof_estimate(&sketch), 2);
}

#[test]
fn copies_of_a_group_hold_their_own_shape_where_they_land() {
    let mut sketch = Sketch::new("t");
    let (_, sides) = grouped_triangle(&mut sketch);
    let selected: HashSet<Uuid> = sides.into_iter().collect();

    // Copied to the clipboard and pasted elsewhere, and copied turned.
    let mut clip = Sketch::new("clipboard");
    copy_from(
        &sketch,
        &mut clip,
        &selected,
        &Similarity::translation(glam::DVec2::ZERO),
    );
    crate::commands::paste(&mut sketch, &clip, Vec2D::new(10.0, 2.0));
    let turn = Similarity::rotation_about(glam::DVec2::new(1.0, 1.0), 0.8);
    copy_from(&sketch.clone(), &mut sketch, &selected, &turn);

    let groups: Vec<&ConstraintKind> = sketch
        .constraints
        .iter()
        .map(|c| &c.kind)
        .filter(|k| matches!(k, ConstraintKind::Group { .. }))
        .collect();
    assert_eq!(groups.len(), 3, "each copy carries a group");
    for kind in &groups {
        let ConstraintKind::Group {
            elements,
            reference,
        } = kind
        else {
            unreachable!()
        };
        assert_eq!(elements.len(), 3);
        // Each group holds its own members where they stand.
        for entry in reference {
            let crate::sketch::GroupReference::Point { point, at } = entry else {
                panic!("a triangle of lines holds only points");
            };
            let now = sketch.point_position(*point).unwrap();
            assert!((now - *at).to_glam().length() < 1e-4);
        }
    }
    assert_eq!(crate::solver::dof_estimate(&sketch), 9);
    let before = sketch.geometry.clone();
    converged(&mut sketch);
    for (was, now) in before.iter().zip(&sketch.geometry) {
        if let (GeometryElement::Point(was), GeometryElement::Point(now)) = (was, now) {
            assert!((was.position - now.position).to_glam().length() < 1e-4);
        }
    }
}

#[test]
fn copying_a_group_moves_its_reference_with_the_copy() {
    // Taken through `copy_constraints` alone, as a carbon copy onto
    // another plane does: the shape goes through the transform.
    let mut sketch = Sketch::new("t");
    let (corners, sides) = grouped_triangle(&mut sketch);
    let selected: HashSet<Uuid> = sides.into_iter().collect();
    let turn = Similarity::rotation_about(glam::DVec2::ZERO, std::f64::consts::FRAC_PI_2);
    let mut target = Sketch::new("target");
    let map = copy_mapped(&sketch, &mut target, &selected, &turn);
    assert_eq!(copy_constraints(&sketch, &mut target, &map, &turn), 1);
    let Some(ConstraintKind::Group { reference, .. }) = target.constraints.first().map(|c| &c.kind)
    else {
        panic!("the group was copied");
    };
    for entry in reference {
        let crate::sketch::GroupReference::Point { point, at } = entry else {
            panic!("only points");
        };
        let original = corners
            .iter()
            .find(|c| map[*c] == *point)
            .expect("a copied corner");
        let want = turn.apply(sketch.point_position(*original).unwrap());
        assert!((*at - want).to_glam().length() < 1e-5);
    }
}
