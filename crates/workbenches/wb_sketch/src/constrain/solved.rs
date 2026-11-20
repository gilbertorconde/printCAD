//! The constraints the toolbar picks for a selection, solved: what the
//! tool makes of the picks holds once the sketch solves.

use uuid::Uuid;

use crate::measure;
use crate::operations::SolveOutcome;
use crate::sketch::{Arc, ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::solver::solve;

fn add_point(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
}

fn add_line(sketch: &mut Sketch, start: Uuid, end: Uuid) -> Uuid {
    sketch.add_geometry(GeometryElement::Line(Line::new(start, end)))
}

fn converged(sketch: &mut Sketch) {
    let outcome = solve(sketch);
    assert!(
        matches!(outcome, SolveOutcome::Converged { .. }),
        "expected convergence, got {outcome:?}"
    );
}

/// Fix a point where it stands.
fn pin(sketch: &mut Sketch, p: Uuid) {
    let position = sketch.point_position(p).unwrap();
    sketch.add_constraint(ConstraintKind::FixedPoint { point: p, position });
}

fn near(actual: f64, expected: f64) {
    assert_near(actual, expected, 1e-6);
}

fn assert_near(actual: f64, expected: f64, tol: f64) {
    assert!(
        (actual - expected).abs() <= tol,
        "expected {expected}, got {actual} (tol {tol})"
    );
}

#[test]
fn an_arc_opens_to_its_angle_and_three_points_hold_theirs() {
    let mut sketch = Sketch::new("t");
    let c = add_point(&mut sketch, 0.0, 0.0);
    let s = add_point(&mut sketch, 5.0, 0.0);
    let e = add_point(&mut sketch, 0.0, 5.0);
    let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
    pin(&mut sketch, c);
    pin(&mut sketch, s);
    sketch.add_constraint(ConstraintKind::Radius {
        circle: arc,
        radius: 5.0,
    });
    sketch.add_constraint(ConstraintKind::ArcAngle {
        arc,
        angle_rad: 135f64.to_radians(),
    });
    converged(&mut sketch);
    near(
        measure::arc_sweep(&sketch, arc).unwrap().to_degrees(),
        135.0,
    );

    // Picked arm, corner, arm: the corner is the vertex, and a reflex
    // pick is turned round to the short way.
    let mut sketch = Sketch::new("t");
    let arm1 = add_point(&mut sketch, 10.0, 0.0);
    let corner = add_point(&mut sketch, 0.0, 0.0);
    let arm2 = add_point(&mut sketch, 0.0, 10.0);
    let selected: std::collections::HashSet<Uuid> = [arm1, corner, arm2].into();
    let shape = super::SelectionShape::picked_in(&sketch, &selected, &[arm2, corner, arm1]);
    let kinds = super::kinds_for("angle_three_points", &shape, &sketch).unwrap();
    let [
        ConstraintKind::AngleThreePoints {
            vertex, angle_rad, ..
        },
    ] = kinds[..]
    else {
        panic!("{kinds:?}");
    };
    assert_eq!(vertex, corner);
    near(angle_rad.to_degrees(), 90.0);
    pin(&mut sketch, arm1);
    pin(&mut sketch, corner);
    sketch.add_constraint(crate::sketch::with_dimension_value(&kinds[0], 60.0));
    sketch.add_constraint(ConstraintKind::Distance {
        point1: corner,
        point2: arm2,
        distance: 10.0,
    });
    converged(&mut sketch);
    let at = sketch.point_position(arm2).unwrap();
    near(at.y.atan2(at.x).to_degrees(), 60.0);
}

#[test]
fn two_ends_picked_are_joined_smoothly() {
    let mut sketch = Sketch::new("t");
    let (a, b) = (
        add_point(&mut sketch, 0.0, 0.0),
        add_point(&mut sketch, 10.0, 0.0),
    );
    let line = add_line(&mut sketch, a, b);
    pin(&mut sketch, a);
    pin(&mut sketch, b);
    let c = add_point(&mut sketch, 12.0, 6.0);
    let s = add_point(&mut sketch, 12.0, 1.0);
    let e = add_point(&mut sketch, 17.0, 6.0);
    let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
    let selected: std::collections::HashSet<Uuid> = [b, s].into();
    let shape = super::SelectionShape::of(&sketch, &selected);
    let kinds = super::kinds_for("tangent", &shape, &sketch).unwrap();
    assert!(
        matches!(
            kinds[..],
            [
                ConstraintKind::Coincident { .. },
                ConstraintKind::Tangent { line_or_circle1, item2 },
            ] if line_or_circle1 == line && item2 == arc
        ),
        "{kinds:?}"
    );
    for kind in kinds {
        sketch.add_constraint(kind);
    }
    converged(&mut sketch);
    let joint = sketch.point_position(s).unwrap();
    assert_near(joint.x, 10.0, 1e-3);
    assert_near(joint.y, 0.0, 1e-3);
    let center = sketch.point_position(c).unwrap();
    assert_near(center.x, 10.0, 1e-3);
    let r = match sketch.get_geometry(arc) {
        Some(GeometryElement::Arc(a)) => a.radius,
        _ => unreachable!(),
    };
    assert_near(center.y.abs(), r, 1e-3);

    // A point and the two curves meeting at it take the same join.
    let selected: std::collections::HashSet<Uuid> = [b, line, arc].into();
    let shape = super::SelectionShape::of(&sketch, &selected);
    let kinds = super::kinds_for("tangent", &shape, &sketch).unwrap();
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, ConstraintKind::Tangent { .. })),
        "{kinds:?}"
    );
}

#[test]
fn a_gap_holds_two_parallel_lines_apart_and_the_dimension_tool_picks_it() {
    let mut sketch = Sketch::new("t");
    let (a, b) = (
        add_point(&mut sketch, 0.0, 0.0),
        add_point(&mut sketch, 10.0, 0.0),
    );
    let (c, d) = (
        add_point(&mut sketch, 0.0, 3.0),
        add_point(&mut sketch, 10.0, 3.0),
    );
    let (bottom, top) = (add_line(&mut sketch, a, b), add_line(&mut sketch, c, d));
    pin(&mut sketch, a);
    pin(&mut sketch, b);
    sketch.add_constraint(ConstraintKind::Parallel {
        line1: bottom,
        line2: top,
    });
    let shape = super::SelectionShape::of(&sketch, &std::collections::HashSet::from([bottom, top]));
    assert_eq!(
        super::dimension_in(&shape, &sketch),
        Some("distance"),
        "parallel lines take a distance, not an angle"
    );
    let kinds = super::kinds_for("distance", &shape, &sketch).unwrap();
    assert!(
        matches!(kinds[0], ConstraintKind::Gap { distance, .. } if (distance - 3.0).abs() < 1e-4),
        "{kinds:?}"
    );
    sketch.add_constraint(ConstraintKind::Gap {
        item1: bottom,
        item2: top,
        distance: 5.0,
    });
    converged(&mut sketch);
    near(measure::gap(&sketch, bottom, top).unwrap().distance, 5.0);
    near(sketch.point_position(c).unwrap().y, 5.0);
    near(sketch.point_position(d).unwrap().y, 5.0);
}
