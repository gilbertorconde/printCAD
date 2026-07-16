use std::collections::HashSet;

use super::*;
use crate::sketch::{Arc, Circle, ConstraintKind, Line};

/// Forward to [`super::handle_click`] with default params and an empty
/// selection (most tools don't consume either).
fn handle_click(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
) -> ToolEffect {
    click_p(
        state,
        tool,
        sketch,
        cursor,
        snap_tol,
        &ToolParams::default(),
    )
}

/// Like [`handle_click`] but with explicit params.
fn click_p(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    params: &ToolParams,
) -> ToolEffect {
    super::handle_click(
        state,
        tool,
        sketch,
        cursor,
        snap_tol,
        params,
        &HashSet::new(),
    )
}

/// Like [`handle_click`] but with an explicit selection.
fn click_sel(
    state: &mut ToolState,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    params: &ToolParams,
    selected: &HashSet<Uuid>,
) -> ToolEffect {
    super::handle_click(state, tool, sketch, cursor, snap_tol, params, selected)
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

fn pt(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
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
fn line_chain_shares_points() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    assert_eq!(
        points(&sketch),
        0,
        "no geometry before the segment completes"
    );
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(10.0, 5.0),
        0.5,
    );
    assert_eq!((points(&sketch), lines(&sketch)), (2, 1));
    // Chain continues: third click adds ONE new point + one line.
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(20.0, 0.0),
        0.5,
    );
    assert_eq!((points(&sketch), lines(&sketch)), (3, 2));
    // Shared middle vertex.
    let line_elems: Vec<&Line> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(line_elems[0].end, line_elems[1].start);
}

#[test]
fn cancelled_first_click_leaves_no_geometry() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    let _ = ToolState::Idle; // Escape would just drop the state
    assert!(sketch.geometry.is_empty());
}

#[test]
fn line_snaps_end_to_existing_point() {
    let mut sketch = Sketch::new("t");
    let existing = pt(&mut sketch, 10.0, 0.0);
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(10.2, 0.1),
        0.5,
    );
    let line = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l),
            _ => None,
        })
        .unwrap();
    assert_eq!(line.end, existing, "end point reused, not duplicated");
    assert_eq!(points(&sketch), 2);
}

#[test]
fn nearly_horizontal_line_gets_auto_constraint() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    handle_click(
        &mut state,
        "sketch.line",
        &mut sketch,
        Vec2D::new(12.0, 0.3),
        0.5,
    );
    assert_eq!(sketch.constraints.len(), 1);
    assert!(matches!(
        sketch.constraints[0].kind,
        ConstraintKind::Horizontal { .. }
    ));
    // And the geometry was snapped level.
    let ys: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.y),
            _ => None,
        })
        .collect();
    assert_eq!(ys, vec![0.0, 0.0]);
}

#[test]
fn rectangle_builds_four_lines_with_constraints() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.rect",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    handle_click(
        &mut state,
        "sketch.rect",
        &mut sketch,
        Vec2D::new(8.0, 5.0),
        0.5,
    );
    assert_eq!((points(&sketch), lines(&sketch)), (4, 4));
    assert_eq!(sketch.constraints.len(), 4);
    assert!(state.is_idle());
    // Corners form a closed loop: every point used exactly twice.
    let uses = point_use_counts(&sketch);
    assert!(uses.values().all(|&n| n == 2));
}

#[test]
fn degenerate_rectangle_rejected() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.rect",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    let fx = handle_click(
        &mut state,
        "sketch.rect",
        &mut sketch,
        Vec2D::new(0.0, 5.0),
        0.5,
    );
    assert!(!fx.changed);
    assert!(sketch.geometry.is_empty());
}

#[test]
fn rect_center_builds_symmetric_rectangle() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.rect_center",
        &mut sketch,
        Vec2D::new(5.0, 3.0),
        0.5,
    );
    assert!(sketch.geometry.is_empty(), "nothing before completion");
    let fx = handle_click(
        &mut state,
        "sketch.rect_center",
        &mut sketch,
        Vec2D::new(9.0, 5.0),
        0.5,
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    assert_eq!((points(&sketch), lines(&sketch)), (4, 4));
    assert_eq!(sketch.constraints.len(), 4, "2 horizontal + 2 vertical");
    // Corners are mirrored through the center: (1,1) .. (9,5).
    let mut xs: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.x),
            _ => None,
        })
        .collect();
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs, vec![1.0, 1.0, 9.0, 9.0]);
}

#[test]
fn circle_two_clicks() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.circle",
        &mut sketch,
        Vec2D::new(1.0, 1.0),
        0.5,
    );
    handle_click(
        &mut state,
        "sketch.circle",
        &mut sketch,
        Vec2D::new(4.0, 5.0),
        0.5,
    );
    let circle = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert!((circle.radius - 5.0).abs() < 1e-5);
    assert!(state.is_idle());
}

#[test]
fn circle3_builds_circumscribed_circle() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    for (x, y) in [(0.0, 0.0), (6.0, 0.0)] {
        handle_click(
            &mut state,
            "sketch.circle3",
            &mut sketch,
            Vec2D::new(x, y),
            0.1,
        );
    }
    assert!(sketch.geometry.is_empty(), "nothing before completion");
    let fx = handle_click(
        &mut state,
        "sketch.circle3",
        &mut sketch,
        Vec2D::new(0.0, 8.0),
        0.1,
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    let circle = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let center = sketch.point_position(circle.center).unwrap();
    assert!((center.to_glam() - glam::Vec2::new(3.0, 4.0)).length() < 1e-3);
    assert!((circle.radius - 5.0).abs() < 1e-3);
}

#[test]
fn circle3_rejects_collinear_points() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    for (x, y) in [(0.0, 0.0), (5.0, 0.0)] {
        handle_click(
            &mut state,
            "sketch.circle3",
            &mut sketch,
            Vec2D::new(x, y),
            0.1,
        );
    }
    let fx = handle_click(
        &mut state,
        "sketch.circle3",
        &mut sketch,
        Vec2D::new(10.0, 0.0),
        0.1,
    );
    assert!(!fx.changed);
    assert!(sketch.geometry.is_empty());
}

#[test]
fn arc_three_clicks_end_projected_to_radius() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.arc",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.1,
    );
    handle_click(
        &mut state,
        "sketch.arc",
        &mut sketch,
        Vec2D::new(5.0, 0.0),
        0.1,
    );
    handle_click(
        &mut state,
        "sketch.arc",
        &mut sketch,
        Vec2D::new(0.0, 7.0),
        0.1,
    );
    let arc = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .unwrap();
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
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    // Endpoints (5,0) and (0,5); rim point on the short CCW side.
    handle_click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        Vec2D::new(5.0, 0.0),
        0.1,
    );
    handle_click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        Vec2D::new(0.0, 5.0),
        0.1,
    );
    let fx = handle_click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        Vec2D::new(3.5355, 3.5355),
        0.1,
    );
    assert!(fx.changed);
    let arc = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .unwrap();
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
        glam::Vec2::new(3.5355, 3.5355)
    ));
    let (_, sweep) = crate::snap::arc_angles(s - c, e - c);
    assert!(
        sweep < std::f32::consts::PI,
        "short side chosen, sweep {sweep}"
    );
}

#[test]
fn arc3_swaps_endpoints_for_clockwise_rim_point() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        Vec2D::new(5.0, 0.0),
        0.1,
    );
    handle_click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        Vec2D::new(0.0, 5.0),
        0.1,
    );
    // Rim point on the far (clockwise) side: endpoints must swap so the
    // stored CCW sweep passes through it.
    handle_click(
        &mut state,
        "sketch.arc3",
        &mut sketch,
        Vec2D::new(-3.5355, -3.5355),
        0.1,
    );
    let arc = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    let c = sketch.point_position(arc.center).unwrap().to_glam();
    let s = sketch.point_position(arc.start).unwrap().to_glam();
    let e = sketch.point_position(arc.end).unwrap().to_glam();
    assert!(crate::geom2d::point_on_arc(
        c,
        s,
        e,
        glam::Vec2::new(-3.5355, -3.5355)
    ));
    assert!((s - glam::Vec2::new(0.0, 5.0)).length() < 1e-3, "swapped");
}

#[test]
fn ellipse_three_clicks_sets_major_and_ratio() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.ellipse",
        &mut sketch,
        Vec2D::new(1.0, 2.0),
        0.1,
    );
    handle_click(
        &mut state,
        "sketch.ellipse",
        &mut sketch,
        Vec2D::new(5.0, 2.0),
        0.1,
    );
    assert!(sketch.geometry.is_empty(), "nothing before completion");
    let fx = handle_click(
        &mut state,
        "sketch.ellipse",
        &mut sketch,
        Vec2D::new(1.0, 3.5),
        0.1,
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    let ellipse = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Ellipse(e) => Some(e.clone()),
            _ => None,
        })
        .unwrap();
    assert!((ellipse.major.x - 4.0).abs() < 1e-4);
    assert!(ellipse.major.y.abs() < 1e-4);
    assert!((ellipse.ratio - 1.5 / 4.0).abs() < 1e-4);
    assert_eq!(points(&sketch), 1, "just the center point");
}

#[test]
fn bspline_clicks_then_finish_builds_spline() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    for (x, y) in [(0.0, 0.0), (4.0, 6.0), (9.0, -2.0), (14.0, 3.0)] {
        handle_click(
            &mut state,
            "sketch.bspline",
            &mut sketch,
            Vec2D::new(x, y),
            0.1,
        );
    }
    assert!(sketch.geometry.is_empty(), "nothing before finish");
    let fx = super::finish_click_sequence(&mut state, &mut sketch, &ToolParams::default());
    assert!(fx.changed);
    assert!(state.is_idle());
    let spline = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::BSpline(b) => Some(b.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(spline.control_points.len(), 4);
    assert!(!spline.periodic);
    assert_eq!(points(&sketch), 4);
}

#[test]
fn bspline_with_too_few_points_cancels() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    for (x, y) in [(0.0, 0.0), (4.0, 6.0)] {
        handle_click(
            &mut state,
            "sketch.bspline",
            &mut sketch,
            Vec2D::new(x, y),
            0.1,
        );
    }
    let fx = super::finish_click_sequence(&mut state, &mut sketch, &ToolParams::default());
    assert!(!fx.changed);
    assert!(state.is_idle());
    assert!(sketch.geometry.is_empty(), "no orphan control points");
}

#[test]
fn bspline_periodic_param_is_respected() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    let params = ToolParams {
        bspline_periodic: true,
        ..ToolParams::default()
    };
    for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        click_p(
            &mut state,
            "sketch.bspline",
            &mut sketch,
            Vec2D::new(x, y),
            0.1,
            &params,
        );
    }
    super::finish_click_sequence(&mut state, &mut sketch, &params);
    let spline = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::BSpline(b) => Some(b.clone()),
            _ => None,
        })
        .unwrap();
    assert!(spline.periodic);
    // A periodic spline is a closed wire on its own.
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
}

#[test]
fn polygon_two_clicks_builds_closed_ngon() {
    for sides in [3u32, 6, 12] {
        let mut sketch = Sketch::new("t");
        let mut state = ToolState::default();
        let params = ToolParams {
            polygon_sides: sides,
            ..ToolParams::default()
        };
        click_p(
            &mut state,
            "sketch.polygon",
            &mut sketch,
            Vec2D::new(2.0, 1.0),
            0.5,
            &params,
        );
        assert!(sketch.geometry.is_empty(), "nothing before completion");
        let fx = click_p(
            &mut state,
            "sketch.polygon",
            &mut sketch,
            Vec2D::new(7.0, 1.0),
            0.5,
            &params,
        );
        assert!(fx.changed);
        assert!(state.is_idle());
        let n = sides as usize;
        assert_eq!((points(&sketch), lines(&sketch)), (n, n));
        // Closed loop: every vertex used by exactly two lines.
        let uses = point_use_counts(&sketch);
        assert_eq!(uses.len(), n);
        assert!(uses.values().all(|&c| c == 2), "closed loop for n={n}");
        // All vertices on the circumscribed circle of radius 5.
        for g in &sketch.geometry {
            if let GeometryElement::Point(p) = g {
                let r = (p.position - Vec2D::new(2.0, 1.0)).to_glam().length();
                assert!((r - 5.0).abs() < 1e-4, "vertex off circle: r={r}");
            }
        }
    }
}

#[test]
fn polygon_first_vertex_at_click_position() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.polygon",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
    );
    handle_click(
        &mut state,
        "sketch.polygon",
        &mut sketch,
        Vec2D::new(3.0, 4.0),
        0.5,
    );
    let hit = sketch.geometry.iter().any(|g| match g {
        GeometryElement::Point(p) => (p.position - Vec2D::new(3.0, 4.0)).to_glam().length() < 1e-4,
        _ => false,
    });
    assert!(hit, "clicked vertex is a polygon vertex");
}

#[test]
fn degenerate_polygon_rejected() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.polygon",
        &mut sketch,
        Vec2D::new(1.0, 1.0),
        0.5,
    );
    let fx = handle_click(
        &mut state,
        "sketch.polygon",
        &mut sketch,
        Vec2D::new(1.0, 1.0),
        0.5,
    );
    assert!(!fx.changed, "vertex == center is degenerate");
    assert!(sketch.geometry.is_empty());
}

#[test]
fn slot_two_clicks_builds_closed_stadium() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    let params = ToolParams {
        slot_width: 4.0,
        ..ToolParams::default()
    };
    click_p(
        &mut state,
        "sketch.slot",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
        &params,
    );
    assert!(sketch.geometry.is_empty(), "nothing before completion");
    let fx = click_p(
        &mut state,
        "sketch.slot",
        &mut sketch,
        Vec2D::new(10.0, 0.0),
        0.5,
        &params,
    );
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

#[test]
fn slot_works_on_diagonal_centerline() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.slot",
        &mut sketch,
        Vec2D::new(1.0, 1.0),
        0.5,
    );
    handle_click(
        &mut state,
        "sketch.slot",
        &mut sketch,
        Vec2D::new(7.0, 9.0),
        0.5,
    );
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
}

#[test]
fn degenerate_slot_rejected() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    handle_click(
        &mut state,
        "sketch.slot",
        &mut sketch,
        Vec2D::new(3.0, 3.0),
        0.5,
    );
    let fx = handle_click(
        &mut state,
        "sketch.slot",
        &mut sketch,
        Vec2D::new(3.0, 3.0),
        0.5,
    );
    assert!(!fx.changed, "zero-length centerline is degenerate");
    assert!(sketch.geometry.is_empty());
}

#[test]
fn arc_slot_three_clicks_builds_closed_profile() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    let params = ToolParams {
        slot_width: 2.0,
        ..ToolParams::default()
    };
    // Center, centerline start (r=5), quarter-turn end.
    click_p(
        &mut state,
        "sketch.arc_slot",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.1,
        &params,
    );
    click_p(
        &mut state,
        "sketch.arc_slot",
        &mut sketch,
        Vec2D::new(5.0, 0.0),
        0.1,
        &params,
    );
    assert!(sketch.geometry.is_empty(), "nothing before completion");
    let fx = click_p(
        &mut state,
        "sketch.arc_slot",
        &mut sketch,
        Vec2D::new(0.0, 5.0),
        0.1,
        &params,
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    // Center + 2 cap centers + 4 rail junctions; 4 arcs, no lines.
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (7, 0, 4));
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
    // Rail radii are r ± width/2.
    let mut radii: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.radius),
            _ => None,
        })
        .collect();
    radii.sort_by(f32::total_cmp);
    assert_eq!(radii, vec![1.0, 1.0, 4.0, 6.0]);
}

#[test]
fn arc_slot_rejects_width_wider_than_radius() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    let params = ToolParams {
        slot_width: 12.0, // half-width 6 > centerline radius 5
        ..ToolParams::default()
    };
    click_p(
        &mut state,
        "sketch.arc_slot",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.1,
        &params,
    );
    click_p(
        &mut state,
        "sketch.arc_slot",
        &mut sketch,
        Vec2D::new(5.0, 0.0),
        0.1,
        &params,
    );
    let fx = click_p(
        &mut state,
        "sketch.arc_slot",
        &mut sketch,
        Vec2D::new(0.0, 5.0),
        0.1,
        &params,
    );
    assert!(!fx.changed);
    assert!(sketch.geometry.is_empty());
}

/// Rectangle (0,0)-(w,h) built from shared corner points.
fn build_rectangle(sketch: &mut Sketch, w: f32, h: f32) -> [Uuid; 4] {
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

fn fillet_params(radius: f32) -> ToolParams {
    ToolParams {
        fillet_radius: radius,
        ..ToolParams::default()
    }
}

#[test]
fn fillet_rounds_rectangle_corner_tangentially() {
    let mut sketch = Sketch::new("t");
    build_rectangle(&mut sketch, 12.0, 8.0);
    let mut state = ToolState::default();
    let fx = click_p(
        &mut state,
        "sketch.fillet",
        &mut sketch,
        Vec2D::new(12.0, 8.0),
        0.5,
        &fillet_params(2.0),
    );
    assert!(fx.changed);
    // Corner point replaced by 2 tangent points + 1 arc center.
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (6, 4, 1));

    let arc = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    let center = sketch.point_position(arc.center).unwrap();
    let start = sketch.point_position(arc.start).unwrap();
    let end = sketch.point_position(arc.end).unwrap();
    // 90° corner with R=2: center at (10, 6), tangent points at
    // (12, 6) and (10, 8).
    assert!((center.to_glam() - glam::Vec2::new(10.0, 6.0)).length() < 1e-4);
    assert!((arc.radius - 2.0).abs() < 1e-5);
    assert!(((start.to_glam() - center.to_glam()).length() - 2.0).abs() < 1e-4);
    assert!(((end.to_glam() - center.to_glam()).length() - 2.0).abs() < 1e-4);
    // The arc bridges on the corner side: its midpoint bulges toward
    // the removed corner (12,8), i.e. beyond the chord.
    let (start_angle, sweep) =
        crate::snap::arc_angles((start - center).to_glam(), (end - center).to_glam());
    assert!(
        (sweep - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
        "quarter-circle sweep, got {sweep}"
    );
    let mid_angle = start_angle + sweep * 0.5;
    let mid = center.to_glam() + 2.0 * glam::Vec2::new(mid_angle.cos(), mid_angle.sin());
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
fn fillet_radius_too_large_rejected() {
    let mut sketch = Sketch::new("t");
    build_rectangle(&mut sketch, 3.0, 3.0);
    let mut state = ToolState::default();
    // 90° corner: tangent offset equals the radius; 4 > 3 cannot fit.
    let fx = click_p(
        &mut state,
        "sketch.fillet",
        &mut sketch,
        Vec2D::new(3.0, 3.0),
        0.5,
        &fillet_params(4.0),
    );
    assert!(!fx.changed);
    assert_eq!(
        (points(&sketch), lines(&sketch), arcs(&sketch)),
        (4, 4, 0),
        "sketch untouched"
    );
}

#[test]
fn fillet_ignores_non_corner_clicks() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    line_between(&mut sketch, a, b);
    let mut state = ToolState::default();
    let params = fillet_params(2.0);

    // Empty space: no point within tolerance.
    let fx = click_p(
        &mut state,
        "sketch.fillet",
        &mut sketch,
        Vec2D::new(5.0, 5.0),
        0.5,
        &params,
    );
    assert!(!fx.changed);
    // Endpoint with only ONE line attached.
    let fx = click_p(
        &mut state,
        "sketch.fillet",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.5,
        &params,
    );
    assert!(!fx.changed);
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (2, 1, 0));
}

#[test]
fn fillet_rejects_collinear_segments() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let m = pt(&mut sketch, 5.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    line_between(&mut sketch, a, m);
    line_between(&mut sketch, m, b);
    let mut state = ToolState::default();
    let fx = click_p(
        &mut state,
        "sketch.fillet",
        &mut sketch,
        Vec2D::new(5.0, 0.0),
        0.5,
        &fillet_params(1.0),
    );
    assert!(!fx.changed, "collinear corner has no angle to round");
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (3, 2, 0));
}

#[test]
fn fillet_drops_constraints_on_removed_corner() {
    let mut sketch = Sketch::new("t");
    let [.., c, _] = build_rectangle(&mut sketch, 12.0, 8.0);
    sketch.add_constraint(ConstraintKind::FixedPoint {
        point: c,
        position: Vec2D::new(12.0, 8.0),
    });
    let mut state = ToolState::default();
    let fx = click_p(
        &mut state,
        "sketch.fillet",
        &mut sketch,
        Vec2D::new(12.0, 8.0),
        0.5,
        &fillet_params(2.0),
    );
    assert!(fx.changed);
    assert!(
        sketch.constraints.is_empty(),
        "constraint on the removed corner dropped"
    );
    assert!(sketch.get_geometry(c).is_none(), "corner point removed");
}

#[test]
fn chamfer_cuts_rectangle_corner_with_line() {
    let mut sketch = Sketch::new("t");
    build_rectangle(&mut sketch, 12.0, 8.0);
    let mut state = ToolState::default();
    let params = ToolParams {
        chamfer_length: 2.0,
        ..ToolParams::default()
    };
    let fx = click_p(
        &mut state,
        "sketch.chamfer",
        &mut sketch,
        Vec2D::new(12.0, 8.0),
        0.5,
        &params,
    );
    assert!(fx.changed);
    // Corner point replaced by 2 setback points; extra chamfer line.
    assert_eq!((points(&sketch), lines(&sketch), arcs(&sketch)), (5, 5, 0));
    // Setback points 2mm from the removed corner along each edge.
    let expect = [glam::Vec2::new(10.0, 8.0), glam::Vec2::new(12.0, 6.0)];
    for target in expect {
        assert!(
            sketch.geometry.iter().any(|g| match g {
                GeometryElement::Point(p) => (p.position.to_glam() - target).length() < 1e-4,
                _ => false,
            }),
            "setback point at {target:?}"
        );
    }
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 5, "4 shortened edges + chamfer");
}

#[test]
fn trim_middle_span_leaves_two_lines() {
    let mut sketch = Sketch::new("t");
    // Horizontal target crossed by two vertical cutters at x=5 and x=15.
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 20.0, 0.0);
    line_between(&mut sketch, a, b);
    let c1a = pt(&mut sketch, 5.0, -5.0);
    let c1b = pt(&mut sketch, 5.0, 5.0);
    line_between(&mut sketch, c1a, c1b);
    let c2a = pt(&mut sketch, 15.0, -5.0);
    let c2b = pt(&mut sketch, 15.0, 5.0);
    line_between(&mut sketch, c2a, c2b);

    let mut state = ToolState::default();
    let fx = handle_click(
        &mut state,
        "sketch.trim",
        &mut sketch,
        Vec2D::new(10.0, 0.1),
        0.5,
    );
    assert!(fx.changed);
    assert_eq!((points(&sketch), lines(&sketch)), (8, 4), "two halves left");
    // The retained halves end exactly at the cutters.
    let spans: Vec<(f32, f32)> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                let s = sketch.point_position(l.start)?;
                let e = sketch.point_position(l.end)?;
                (s.y.abs() < 1e-4 && e.y.abs() < 1e-4).then(|| (s.x.min(e.x), s.x.max(e.x)))
            }
            _ => None,
        })
        .collect();
    assert!(spans.contains(&(0.0, 5.0)), "left half kept: {spans:?}");
    assert!(spans.contains(&(15.0, 20.0)), "right half kept: {spans:?}");
}

#[test]
fn trim_without_intersections_deletes_element_and_orphans() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    sketch.add_constraint(ConstraintKind::Horizontal { element: l });

    let mut state = ToolState::default();
    let fx = handle_click(
        &mut state,
        "sketch.trim",
        &mut sketch,
        Vec2D::new(5.0, 0.1),
        0.5,
    );
    assert!(fx.changed);
    assert!(sketch.geometry.is_empty(), "line and orphan endpoints gone");
    assert!(sketch.constraints.is_empty(), "line constraint dropped");
}

#[test]
fn trim_end_span_keeps_shared_endpoint() {
    let mut sketch = Sketch::new("t");
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
    let mut state = ToolState::default();
    let fx = handle_click(
        &mut state,
        "sketch.trim",
        &mut sketch,
        Vec2D::new(12.0, 0.1),
        0.5,
    );
    assert!(fx.changed);
    assert!(sketch.get_geometry(a).is_some(), "shared endpoint kept");
    assert!(
        sketch.get_geometry(b).is_none(),
        "orphaned endpoint removed"
    );
    let spans: Vec<f32> = sketch
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
    let mut sketch = Sketch::new("t");
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
    let mut state = ToolState::default();
    let fx = handle_click(
        &mut state,
        "sketch.trim",
        &mut sketch,
        Vec2D::new(5.1, 0.0),
        0.5,
    );
    assert!(fx.changed);
    assert_eq!(circles(&sketch), 0);
    let arc = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(arc) => Some(arc.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(arc.id, circle_id, "arc keeps the circle's id");
    assert_eq!(sketch.constraints.len(), 1, "radius constraint survives");
    // Kept half passes through (-5, 0).
    let s = sketch.point_position(arc.start).unwrap().to_glam();
    let e = sketch.point_position(arc.end).unwrap().to_glam();
    assert!(crate::geom2d::point_on_arc(
        glam::Vec2::ZERO,
        s,
        e,
        glam::Vec2::new(-5.0, 0.0)
    ));
    assert!(!crate::geom2d::point_on_arc(
        glam::Vec2::ZERO,
        s,
        e,
        glam::Vec2::new(5.0, 0.0)
    ));
}

#[test]
fn extend_line_reaches_nearest_intersection() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 5.0, 0.0);
    line_between(&mut sketch, a, b);
    // Two vertical walls; the nearer one (x=10) must win.
    let w1a = pt(&mut sketch, 10.0, -5.0);
    let w1b = pt(&mut sketch, 10.0, 5.0);
    line_between(&mut sketch, w1a, w1b);
    let w2a = pt(&mut sketch, 14.0, -5.0);
    let w2b = pt(&mut sketch, 14.0, 5.0);
    line_between(&mut sketch, w2a, w2b);

    let mut state = ToolState::default();
    // Click the end half of the short line.
    let fx = handle_click(
        &mut state,
        "sketch.extend",
        &mut sketch,
        Vec2D::new(4.0, 0.1),
        0.5,
    );
    assert!(fx.changed);
    let end = sketch.point_position(b).unwrap();
    assert!(
        (end.to_glam() - glam::Vec2::new(10.0, 0.0)).length() < 1e-3,
        "end extended to the nearest wall: {end:?}"
    );

    // No intersection behind the start: extending that end is a no-op.
    let fx = handle_click(
        &mut state,
        "sketch.extend",
        &mut sketch,
        Vec2D::new(1.0, 0.1),
        0.5,
    );
    assert!(!fx.changed);
}

#[test]
fn extend_arc_end_reaches_circle() {
    let mut sketch = Sketch::new("t");
    // Quarter arc around origin from (5,0) to (0,5).
    let c = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 5.0, 0.0);
    let e = pt(&mut sketch, 0.0, 5.0);
    sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
    // A wall crossing the arc's circle at (-5, 0) (and (0,±?) no: the line
    // x = -5 is tangent... use the horizontal line y = 0 extended left).
    let w1 = pt(&mut sketch, -10.0, 0.0);
    let w2 = pt(&mut sketch, -2.0, 0.0);
    line_between(&mut sketch, w1, w2);

    let mut state = ToolState::default();
    // Click near the arc's end half (close to (0,5)).
    let fx = handle_click(
        &mut state,
        "sketch.extend",
        &mut sketch,
        Vec2D::new(0.5, 5.0),
        0.6,
    );
    assert!(fx.changed);
    let end = sketch.point_position(e).unwrap().to_glam();
    assert!(
        (end - glam::Vec2::new(-5.0, 0.0)).length() < 1e-3,
        "arc end swept CCW to the wall: {end:?}"
    );
}

#[test]
fn split_line_at_click_shares_new_point() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);

    let mut state = ToolState::default();
    let fx = handle_click(
        &mut state,
        "sketch.split",
        &mut sketch,
        Vec2D::new(4.0, 0.1),
        0.5,
    );
    assert!(fx.changed);
    assert_eq!((points(&sketch), lines(&sketch)), (3, 2));
    // The halves share the new midpoint; the original id survives.
    let halves: Vec<Line> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(l.clone()),
            _ => None,
        })
        .collect();
    assert!(halves.iter().any(|h| h.id == l));
    assert_eq!(halves[0].end, halves[1].start, "shared split point");
    let m = sketch.point_position(halves[0].end).unwrap();
    assert!((m.to_glam() - glam::Vec2::new(4.0, 0.0)).length() < 1e-3);
}

#[test]
fn split_arc_produces_two_ccw_arcs() {
    let mut sketch = Sketch::new("t");
    let c = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 5.0, 0.0);
    let e = pt(&mut sketch, -5.0, 0.0);
    sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));

    let mut state = ToolState::default();
    // Click the top of the semicircle.
    let fx = handle_click(
        &mut state,
        "sketch.split",
        &mut sketch,
        Vec2D::new(0.0, 5.1),
        0.5,
    );
    assert!(fx.changed);
    assert_eq!(arcs(&sketch), 2);
    let arcs_v: Vec<Arc> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(arcs_v[0].end, arcs_v[1].start, "shared split point");
    for a in &arcs_v {
        let sp = sketch.point_position(a.start).unwrap().to_glam();
        let ep = sketch.point_position(a.end).unwrap().to_glam();
        let (_, sweep) = crate::snap::arc_angles(sp, ep);
        assert!(
            (sweep - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "quarter sweep each, got {sweep}"
        );
    }
}

#[test]
fn offset_rectangle_inward_builds_closed_loop() {
    let mut sketch = Sketch::new("t");
    build_rectangle(&mut sketch, 10.0, 5.0);
    let selected: HashSet<Uuid> = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Line(_)))
        .map(|g| g.id())
        .collect();
    let params = ToolParams {
        offset_distance: 1.0,
        ..ToolParams::default()
    };
    let mut state = ToolState::default();
    // Click inside: the copy shrinks inward.
    let fx = click_sel(
        &mut state,
        "sketch.offset",
        &mut sketch,
        Vec2D::new(5.0, 2.5),
        0.5,
        &params,
        &selected,
    );
    assert!(fx.changed);
    assert_eq!((points(&sketch), lines(&sketch)), (8, 8));
    let wires = crate::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 2, "original + offset loop both closed");
    // The offset corners sit 1mm inside the original ones.
    for corner in [(1.0, 1.0), (9.0, 1.0), (9.0, 4.0), (1.0, 4.0)] {
        let target = glam::Vec2::new(corner.0, corner.1);
        assert!(
            sketch.geometry.iter().any(|g| match g {
                GeometryElement::Point(p) => (p.position.to_glam() - target).length() < 1e-3,
                _ => false,
            }),
            "inset corner at {target:?}"
        );
    }
}

#[test]
fn offset_single_circle_is_concentric() {
    let mut sketch = Sketch::new("t");
    let center = pt(&mut sketch, 3.0, 3.0);
    let circle_id = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
    let selected: HashSet<Uuid> = [circle_id].into_iter().collect();
    let params = ToolParams {
        offset_distance: 2.0,
        ..ToolParams::default()
    };
    let mut state = ToolState::default();
    // Click outside the rim: the copy grows.
    let fx = click_sel(
        &mut state,
        "sketch.offset",
        &mut sketch,
        Vec2D::new(12.0, 3.0),
        0.5,
        &params,
        &selected,
    );
    assert!(fx.changed);
    let radii: Vec<f32> = sketch
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
    radii.sort_by(f32::total_cmp);
    assert_eq!(radii, vec![5.0, 7.0]);
    assert_eq!(points(&sketch), 1);
}

#[test]
fn translate_moves_selection_and_shared_points() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams::default();
    let mut state = ToolState::default();
    click_sel(
        &mut state,
        "sketch.translate",
        &mut sketch,
        Vec2D::new(20.0, 20.0), // base (empty space)
        0.1,
        &params,
        &selected,
    );
    let fx = click_sel(
        &mut state,
        "sketch.translate",
        &mut sketch,
        Vec2D::new(25.0, 23.0), // destination: Δ = (5, 3)
        0.1,
        &params,
        &selected,
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    let pa = sketch.point_position(a).unwrap().to_glam();
    let pb = sketch.point_position(b).unwrap().to_glam();
    assert!((pa - glam::Vec2::new(5.0, 3.0)).length() < 1e-3);
    assert!((pb - glam::Vec2::new(15.0, 3.0)).length() < 1e-3);
    assert_eq!((points(&sketch), lines(&sketch)), (2, 1), "no copies");
}

#[test]
fn translate_with_copies_builds_array_preserving_sharing() {
    let mut sketch = Sketch::new("t");
    // Two lines sharing a middle point: internal sharing must be preserved
    // in each copy.
    let a = pt(&mut sketch, 0.0, 0.0);
    let m = pt(&mut sketch, 5.0, 5.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l1 = line_between(&mut sketch, a, m);
    let l2 = line_between(&mut sketch, m, b);
    let selected: HashSet<Uuid> = [l1, l2].into_iter().collect();
    let params = ToolParams {
        copies: 2,
        ..ToolParams::default()
    };
    let mut state = ToolState::default();
    click_sel(
        &mut state,
        "sketch.translate",
        &mut sketch,
        Vec2D::new(20.0, 20.0),
        0.1,
        &params,
        &selected,
    );
    let fx = click_sel(
        &mut state,
        "sketch.translate",
        &mut sketch,
        Vec2D::new(40.0, 20.0), // Δ = (20, 0)
        0.1,
        &params,
        &selected,
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
    assert!(sketch.geometry.iter().any(|g| match g {
        GeometryElement::Point(p) =>
            (p.position.to_glam() - glam::Vec2::new(45.0, 5.0)).length() < 1e-3,
        _ => false,
    }));
}

#[test]
fn rotate_selection_about_pivot() {
    let mut sketch = Sketch::new("t");
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams::default();
    let mut state = ToolState::default();
    // Pivot at origin — but (0,0) snaps to point `a`, same position anyway.
    click_sel(
        &mut state,
        "sketch.rotate",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.1,
        &params,
        &selected,
    );
    // Reference along +x, target along +y: 90° CCW.
    click_sel(
        &mut state,
        "sketch.rotate",
        &mut sketch,
        Vec2D::new(20.0, 0.0),
        0.1,
        &params,
        &selected,
    );
    let fx = click_sel(
        &mut state,
        "sketch.rotate",
        &mut sketch,
        Vec2D::new(0.0, 20.0),
        0.1,
        &params,
        &selected,
    );
    assert!(fx.changed);
    let pb = sketch.point_position(b).unwrap().to_glam();
    assert!(
        (pb - glam::Vec2::new(0.0, 10.0)).length() < 1e-3,
        "endpoint rotated 90°: {pb:?}"
    );
}

#[test]
fn scale_selection_scales_radii_too() {
    let mut sketch = Sketch::new("t");
    let center = pt(&mut sketch, 4.0, 0.0);
    let circle_id = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
    let selected: HashSet<Uuid> = [circle_id].into_iter().collect();
    let params = ToolParams::default();
    let mut state = ToolState::default();
    // Base at origin, reference at (1,0)... rounded up: use (10,0) → (20,0)
    // for factor 2 without snapping interference.
    click_sel(
        &mut state,
        "sketch.scale",
        &mut sketch,
        Vec2D::new(0.0, 0.0),
        0.1,
        &params,
        &selected,
    );
    click_sel(
        &mut state,
        "sketch.scale",
        &mut sketch,
        Vec2D::new(10.0, 0.0),
        0.1,
        &params,
        &selected,
    );
    let fx = click_sel(
        &mut state,
        "sketch.scale",
        &mut sketch,
        Vec2D::new(20.0, 0.0),
        0.1,
        &params,
        &selected,
    );
    assert!(fx.changed);
    let c = sketch.point_position(center).unwrap().to_glam();
    assert!((c - glam::Vec2::new(8.0, 0.0)).length() < 1e-3);
    let r = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.radius),
            _ => None,
        })
        .unwrap();
    assert!((r - 4.0).abs() < 1e-3, "radius scaled with the geometry");
}

#[test]
fn mirror_about_line_element_copies_geometry() {
    let mut sketch = Sketch::new("t");
    // Axis: the x-axis as a line element. Subject: a line above it.
    let ax_a = pt(&mut sketch, -20.0, 0.0);
    let ax_b = pt(&mut sketch, 20.0, 0.0);
    line_between(&mut sketch, ax_a, ax_b);
    let a = pt(&mut sketch, 2.0, 2.0);
    let b = pt(&mut sketch, 8.0, 5.0);
    let l = line_between(&mut sketch, a, b);
    let selected: HashSet<Uuid> = [l].into_iter().collect();
    let params = ToolParams::default();
    let mut state = ToolState::default();
    // Click the axis line mid-span (no point nearby).
    let fx = click_sel(
        &mut state,
        "sketch.mirror",
        &mut sketch,
        Vec2D::new(12.0, 0.1),
        0.5,
        &params,
        &selected,
    );
    assert!(fx.changed);
    assert!(state.is_idle());
    assert_eq!((points(&sketch), lines(&sketch)), (6, 3));
    // Mirrored endpoints at (2,-2) and (8,-5); originals untouched.
    for target in [glam::Vec2::new(2.0, -2.0), glam::Vec2::new(8.0, -5.0)] {
        assert!(
            sketch.geometry.iter().any(|g| match g {
                GeometryElement::Point(p) => (p.position.to_glam() - target).length() < 1e-3,
                _ => false,
            }),
            "mirrored point at {target:?}"
        );
    }
    assert!(
        (sketch.point_position(a).unwrap() - Vec2D::new(2.0, 2.0))
            .to_glam()
            .length()
            .abs()
            < 1e-6
    );
}

#[test]
fn mirror_arc_copy_stays_ccw() {
    let mut sketch = Sketch::new("t");
    // Upper semicircle arc from (5,0) to (-5,0), mirrored about the x-axis.
    let c = pt(&mut sketch, 0.0, 0.0);
    let s = pt(&mut sketch, 5.0, 0.0);
    let e = pt(&mut sketch, -5.0, 0.0);
    let arc_id = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
    let selected: HashSet<Uuid> = [arc_id].into_iter().collect();
    let params = ToolParams::default();
    let mut state = ToolState::default();
    // Two-point axis along the x-axis (empty space clicks).
    click_sel(
        &mut state,
        "sketch.mirror",
        &mut sketch,
        Vec2D::new(-20.0, 0.0),
        0.1,
        &params,
        &selected,
    );
    let fx = click_sel(
        &mut state,
        "sketch.mirror",
        &mut sketch,
        Vec2D::new(20.0, 0.0),
        0.1,
        &params,
        &selected,
    );
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
        glam::Vec2::new(0.0, -5.0)
    ));
    let (_, sweep) = crate::snap::arc_angles(cs - cc, ce - cc);
    assert!((sweep - std::f32::consts::PI).abs() < 1e-3);
}

#[test]
fn point_tool_places_point() {
    let mut sketch = Sketch::new("t");
    let mut state = ToolState::default();
    let fx = handle_click(
        &mut state,
        "sketch.point",
        &mut sketch,
        Vec2D::new(2.0, 3.0),
        0.5,
    );
    assert!(fx.changed);
    assert_eq!(points(&sketch), 1);
    assert!(state.is_idle());
}
