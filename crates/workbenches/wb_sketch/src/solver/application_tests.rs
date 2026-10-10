use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Constraint, Ellipse, Line, Point};
    use std::f32::consts::PI;

    fn add_point(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    fn add_line(sketch: &mut Sketch, start: Uuid, end: Uuid) -> Uuid {
        sketch.add_geometry(GeometryElement::Line(Line::new(start, end)))
    }

    fn fix(sketch: &mut Sketch, point: Uuid, x: f32, y: f32) {
        sketch.add_constraint(ConstraintKind::FixedPoint {
            point,
            position: Vec2D::new(x, y),
        });
    }

    fn pos(sketch: &Sketch, id: Uuid) -> glam::Vec2 {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Point(p)) => p.position.to_glam(),
            _ => panic!("expected point"),
        }
    }

    fn circle_radius(sketch: &Sketch, id: Uuid) -> f32 {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Circle(c)) => c.radius,
            Some(GeometryElement::Arc(a)) => a.radius,
            _ => panic!("expected circle or arc"),
        }
    }

    fn assert_converged(outcome: SolveOutcome) {
        assert!(
            matches!(outcome, SolveOutcome::Converged { .. }),
            "expected convergence, got {outcome:?}"
        );
    }

    fn assert_near(actual: f32, expected: f32, tol: f32) {
        assert!(
            (actual - expected).abs() <= tol,
            "expected {expected}, got {actual} (tol {tol})"
        );
    }

    #[test]
    fn a_held_point_stays_where_it_was_put_and_the_rest_gives_way() {
        let mut sketch = Sketch::new("held");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        // The end dragged up and in.
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(b) {
            p.position = Vec2D::new(5.0, 5.0);
        }
        let outcome = solve_holding(&mut sketch, &[b]);
        assert!(
            matches!(outcome, SolveOutcome::Converged { .. }),
            "{outcome:?}"
        );
        let (pa, pb) = (
            sketch.point_position(a).unwrap(),
            sketch.point_position(b).unwrap(),
        );
        assert_near(pb.x, 5.0, 1e-6);
        assert_near(pb.y, 5.0, 1e-6);
        assert_near((pb - pa).to_glam().length(), 10.0, 1e-4);

        // Held where the constraints cannot reach: the plain solve decides.
        sketch.add_constraint(ConstraintKind::FixedPoint {
            point: a,
            position: Vec2D::new(0.0, 0.0),
        });
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(b) {
            p.position = Vec2D::new(40.0, 0.0);
        }
        let outcome = solve_holding(&mut sketch, &[b]);
        assert!(
            matches!(outcome, SolveOutcome::Converged { .. }),
            "{outcome:?}"
        );
        let pb = sketch.point_position(b).unwrap();
        assert_near(pb.to_glam().length(), 10.0, 1e-4);
    }

    #[test]
    fn empty_sketch_returns_nothing_to_solve() {
        let mut sketch = Sketch::new("empty");
        assert_eq!(solve(&mut sketch), SolveOutcome::NothingToSolve);
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn constraints_on_missing_geometry_are_skipped() {
        let mut sketch = Sketch::new("missing");
        add_point(&mut sketch, 1.0, 2.0);
        sketch.add_constraint(ConstraintKind::Length {
            line: Uuid::new_v4(),
            length: 5.0,
        });
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: Uuid::new_v4(),
            point2: Uuid::new_v4(),
        });
        // Horizontal on a non-line element is also skipped.
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: p });
        assert_eq!(solve(&mut sketch), SolveOutcome::NothingToSolve);
    }

    #[test]
    fn fixed_point_moves_point() {
        let mut sketch = Sketch::new("fixed");
        let p = add_point(&mut sketch, 3.0, 4.0);
        fix(&mut sketch, p, 1.0, 2.0);
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x, 1.0, 1e-4);
        assert_near(pos(&sketch, p).y, 2.0, 1e-4);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn horizontal_levels_sloped_line() {
        let mut sketch = Sketch::new("horizontal");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 2.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, b).y, pos(&sketch, a).y, 1e-4);
        // The end's x stays free, so the sketch is not fully constrained.
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn vertical_aligns_sloped_line() {
        let mut sketch = Sketch::new("vertical");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 2.0, 10.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Vertical { element: line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, b).x, pos(&sketch, a).x, 1e-4);
    }

    #[test]
    fn length_sets_line_length_keeping_direction() {
        let mut sketch = Sketch::new("length");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 3.0, 4.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        assert_converged(solve(&mut sketch));
        let dir = (pos(&sketch, b) - pos(&sketch, a)).normalize();
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 10.0, 1e-3);
        // Direction should be roughly preserved (initially (0.6, 0.8)).
        assert!(dir.dot(glam::Vec2::new(0.6, 0.8)) > 0.9);
    }

    #[test]
    fn coincident_merges_two_points() {
        let mut sketch = Sketch::new("coincident");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 2.0, 2.0);
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: p1,
            point2: p2,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p1) - pos(&sketch, p2)).length(), 0.0, 1e-4);
    }

    #[test]
    fn distance_constraint_separates_points() {
        let mut sketch = Sketch::new("distance");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 1.0, 0.0);
        fix(&mut sketch, p1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Distance {
            point1: p1,
            point2: p2,
            distance: 5.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p2) - pos(&sketch, p1)).length(), 5.0, 1e-3);
    }

    #[test]
    fn radius_constraint_resizes_circle() {
        let mut sketch = Sketch::new("radius");
        let center = add_point(&mut sketch, 1.0, 1.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 5.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, circle), 5.0, 1e-4);
    }

    #[test]
    fn equal_radius_matches_two_circles() {
        let mut sketch = Sketch::new("equal_radius");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 10.0, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 2.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 6.0)));
        sketch.add_constraint(ConstraintKind::EqualRadius { circle1, circle2 });
        assert_converged(solve(&mut sketch));
        assert_near(
            circle_radius(&sketch, circle1),
            circle_radius(&sketch, circle2),
            1e-4,
        );
    }

    #[test]
    fn equal_length_matches_two_lines() {
        let mut sketch = Sketch::new("equal_length");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let c = add_point(&mut sketch, 0.0, 5.0);
        let d = add_point(&mut sketch, 4.0, 5.0);
        let line1 = add_line(&mut sketch, a, b);
        let line2 = add_line(&mut sketch, c, d);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        fix(&mut sketch, c, 0.0, 5.0);
        sketch.add_constraint(ConstraintKind::EqualLength { line1, line2 });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, d) - pos(&sketch, c)).length(), 10.0, 1e-3);
    }

    #[test]
    fn parallel_and_perpendicular_pair() {
        let mut sketch = Sketch::new("parallel_perpendicular");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let base = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);

        let c = add_point(&mut sketch, 0.0, 2.0);
        let d = add_point(&mut sketch, 8.0, 4.0);
        let para = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 2.0);
        sketch.add_constraint(ConstraintKind::Parallel {
            line1: base,
            line2: para,
        });

        let e = add_point(&mut sketch, 5.0, 1.0);
        let f = add_point(&mut sketch, 6.0, 9.0);
        let perp = add_line(&mut sketch, e, f);
        fix(&mut sketch, e, 5.0, 1.0);
        sketch.add_constraint(ConstraintKind::Perpendicular {
            line1: base,
            line2: perp,
        });

        assert_converged(solve(&mut sketch));
        // Parallel to the x-axis base: equal y at both ends.
        assert_near(pos(&sketch, d).y, pos(&sketch, c).y, 1e-3);
        // Perpendicular to the x-axis base: equal x at both ends.
        assert_near(pos(&sketch, f).x, pos(&sketch, e).x, 1e-3);
    }

    #[test]
    fn point_on_line_projects_point() {
        let mut sketch = Sketch::new("point_on_line");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let p = add_point(&mut sketch, 4.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnLine { point: p, line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).y, 0.0, 1e-3);
    }

    #[test]
    fn point_on_circle_snaps_to_radius() {
        let mut sketch = Sketch::new("point_on_circle");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
        fix(&mut sketch, center, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 5.0,
        });
        let p = add_point(&mut sketch, 8.0, 0.0);
        sketch.add_constraint(ConstraintKind::PointOnCircle { point: p, circle });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p) - pos(&sketch, center)).length(), 5.0, 1e-3);
    }

    #[test]
    fn angle_constraint_rotates_line() {
        let mut sketch = Sketch::new("angle");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let base = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let c = add_point(&mut sketch, 0.0, 0.0);
        let d = add_point(&mut sketch, 10.0, 1.0);
        let rotated = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Angle {
            line1: base,
            line2: rotated,
            angle_rad: PI / 4.0,
        });
        assert_converged(solve(&mut sketch));
        let dir = pos(&sketch, d) - pos(&sketch, c);
        assert_near(dir.y.atan2(dir.x), PI / 4.0, 1e-3);
    }

    #[test]
    fn rectangle_solves_and_reports_dimensions() {
        let mut sketch = Sketch::new("rectangle");
        // Roughly a 4 x 3 rectangle, perturbed.
        let a = add_point(&mut sketch, 0.1, -0.1);
        let b = add_point(&mut sketch, 3.8, 0.2);
        let c = add_point(&mut sketch, 4.1, 3.2);
        let d = add_point(&mut sketch, -0.2, 2.9);
        let bottom = add_line(&mut sketch, a, b);
        let right = add_line(&mut sketch, b, c);
        let top = add_line(&mut sketch, c, d);
        let left = add_line(&mut sketch, d, a);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: bottom });
        sketch.add_constraint(ConstraintKind::Horizontal { element: top });
        sketch.add_constraint(ConstraintKind::Vertical { element: right });
        sketch.add_constraint(ConstraintKind::Vertical { element: left });
        sketch.add_constraint(ConstraintKind::Length {
            line: bottom,
            length: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Length {
            line: right,
            length: 3.0,
        });

        assert_converged(solve(&mut sketch));
        let (pa, pb, pc, pd) = (
            pos(&sketch, a),
            pos(&sketch, b),
            pos(&sketch, c),
            pos(&sketch, d),
        );
        assert_near(pa.x, 0.0, 1e-3);
        assert_near(pa.y, 0.0, 1e-3);
        assert_near((pb - pa).length(), 4.0, 1e-3);
        assert_near((pc - pb).length(), 3.0, 1e-3);
        assert_near(pb.y, pa.y, 1e-3);
        assert_near(pc.x, pb.x, 1e-3);
        assert_near(pd.y, pc.y, 1e-3);
        assert_near(pd.x, pa.x, 1e-3);
        // 8 variables, 8 independent equations: fully constrained.
        assert_eq!(dof_estimate(&sketch), 0);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn overconstrained_but_consistent_converges() {
        let mut sketch = Sketch::new("overconstrained");
        let a = add_point(&mut sketch, 0.5, 0.2);
        let b = add_point(&mut sketch, 9.7, 0.1);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        // Redundant but consistent with the fixed endpoints.
        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 10.0, 1e-3);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn contradictory_lengths_return_not_converged() {
        let mut sketch = Sketch::new("contradictory");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        sketch.add_constraint(ConstraintKind::Length { line, length: 8.0 });
        match solve(&mut sketch) {
            SolveOutcome::NotConverged { residual } => assert!(residual > 1e-3),
            other => panic!("expected NotConverged, got {other:?}"),
        }
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn arc_endpoints_follow_radius_constraint() {
        let mut sketch = Sketch::new("arc");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let start = add_point(&mut sketch, 5.0, 0.0);
        let end = add_point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 5.0)));
        fix(&mut sketch, center, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc,
            radius: 3.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, arc), 3.0, 1e-3);
        // Implicit arc-consistency residuals pull the endpoints onto the
        // new radius.
        assert_near(
            (pos(&sketch, start) - pos(&sketch, center)).length(),
            3.0,
            1e-3,
        );
        assert_near(
            (pos(&sketch, end) - pos(&sketch, center)).length(),
            3.0,
            1e-3,
        );
    }

    #[test]
    fn tangent_line_circle_moves_center_onto_offset() {
        let mut sketch = Sketch::new("tangent_line_circle");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let center = add_point(&mut sketch, 5.0, 3.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: line,
            item2: circle,
        });
        assert_converged(solve(&mut sketch));
        // Perpendicular distance from the center to the x-axis line must
        // equal the radius (center approached from y=3, so it lands at +2).
        assert_near(pos(&sketch, center).y, 2.0, 1e-3);
        assert_near(circle_radius(&sketch, circle), 2.0, 1e-4);
    }

    #[test]
    fn tangent_line_circle_works_with_swapped_operands() {
        let mut sketch = Sketch::new("tangent_swapped");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let center = add_point(&mut sketch, 5.0, 3.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        fix(&mut sketch, center, 5.0, 3.0);
        // Circle first, line second: the radius adapts instead.
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: circle,
            item2: line,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, circle), 3.0, 1e-3);
    }

    #[test]
    fn tangent_circles_external_branch_stays_external() {
        let mut sketch = Sketch::new("tangent_external");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 10.0, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 3.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 4.0)));
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle1,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle2,
            radius: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: circle1,
            item2: circle2,
        });
        assert_converged(solve(&mut sketch));
        // Externally-tangent circles STAY external: center distance is
        // r1 + r2 = 7, not |r1 - r2| = 1.
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 7.0, 1e-3);
    }

    #[test]
    fn tangent_circles_internal_branch_when_overlapping() {
        let mut sketch = Sketch::new("tangent_internal");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 1.5, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 3.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 4.0)));
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle1,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle2,
            radius: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: circle1,
            item2: circle2,
        });
        assert_converged(solve(&mut sketch));
        // One circle inside the other: the internal branch |r1 - r2| = 1 is
        // closer than the external 7, so the solve keeps them nested.
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 1.0, 1e-3);
    }

    #[test]
    fn symmetric_mirrors_point_about_line() {
        let mut sketch = Sketch::new("symmetric");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let p1 = add_point(&mut sketch, 3.0, 4.0);
        let p2 = add_point(&mut sketch, 5.0, -2.0);
        fix(&mut sketch, p1, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Symmetric {
            point1: p1,
            point2: p2,
            line,
        });
        assert_converged(solve(&mut sketch));
        // Mirror of (3, 4) about the x-axis is (3, -4).
        assert_near(pos(&sketch, p2).x, 3.0, 1e-3);
        assert_near(pos(&sketch, p2).y, -4.0, 1e-3);
    }

    #[test]
    fn symmetric_about_diagonal_line() {
        let mut sketch = Sketch::new("symmetric_diag");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 10.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 10.0);
        let p1 = add_point(&mut sketch, 6.0, 2.0);
        let p2 = add_point(&mut sketch, 1.0, 5.0);
        fix(&mut sketch, p1, 6.0, 2.0);
        sketch.add_constraint(ConstraintKind::Symmetric {
            point1: p1,
            point2: p2,
            line,
        });
        assert_converged(solve(&mut sketch));
        // Mirror of (6, 2) about y = x is (2, 6).
        assert_near(pos(&sketch, p2).x, 2.0, 1e-3);
        assert_near(pos(&sketch, p2).y, 6.0, 1e-3);
    }

    #[test]
    fn midpoint_pins_point_to_line_center() {
        let mut sketch = Sketch::new("midpoint");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 4.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 4.0);
        let p = add_point(&mut sketch, 7.0, 7.0);
        sketch.add_constraint(ConstraintKind::Midpoint { point: p, line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x, 5.0, 1e-3);
        assert_near(pos(&sketch, p).y, 2.0, 1e-3);
    }

    #[test]
    fn dof_of_unconstrained_line_is_four() {
        let mut sketch = Sketch::new("dof_line");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        assert_eq!(dof_estimate(&sketch), 4);
    }

    #[test]
    fn dof_of_fully_fixed_line_is_zero() {
        let mut sketch = Sketch::new("dof_fixed");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        assert_eq!(dof_estimate(&sketch), 0);
    }

    #[test]
    fn already_satisfied_converges_immediately() {
        let mut sketch = Sketch::new("satisfied");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert_eq!(
            solve(&mut sketch),
            SolveOutcome::Converged { iterations: 0 }
        );
    }

    #[test]
    fn diameter_constraint_resizes_circle() {
        let mut sketch = Sketch::new("diameter");
        let center = add_point(&mut sketch, 1.0, 1.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.add_constraint(ConstraintKind::Diameter {
            circle,
            diameter: 9.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, circle), 4.5, 1e-4);
    }

    #[test]
    fn distance_x_and_y_between_points() {
        let mut sketch = Sketch::new("dxdy");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 1.0, 1.0);
        fix(&mut sketch, p1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::DistanceX {
            a: p1,
            b: Some(p2),
            value: 7.0,
        });
        sketch.add_constraint(ConstraintKind::DistanceY {
            a: p1,
            b: Some(p2),
            value: 3.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p2).x - pos(&sketch, p1).x).abs(), 7.0, 1e-3);
        assert_near((pos(&sketch, p2).y - pos(&sketch, p1).y).abs(), 3.0, 1e-3);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn distance_x_from_origin() {
        let mut sketch = Sketch::new("dx_origin");
        let p = add_point(&mut sketch, 2.0, 5.0);
        sketch.add_constraint(ConstraintKind::DistanceX {
            a: p,
            b: None,
            value: 6.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x.abs(), 6.0, 1e-3);
        assert_near(pos(&sketch, p).y, 5.0, 1e-4);
    }

    #[test]
    fn block_freezes_line_while_other_geometry_moves() {
        let mut sketch = Sketch::new("block");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        let blocked_line = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l.id),
                _ => None,
            })
            .unwrap();
        sketch.add_constraint(ConstraintKind::Block {
            element: blocked_line,
        });
        // A free point pulled onto the blocked endpoint: only the point moves.
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: p,
            point2: b,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, a).x, 0.0, 1e-4);
        assert_near(pos(&sketch, b).x, 10.0, 1e-4);
        assert_near(pos(&sketch, b).y, 0.0, 1e-4);
        assert_near(pos(&sketch, p).x, 10.0, 1e-3);
        assert_near(pos(&sketch, p).y, 0.0, 1e-3);
    }

    #[test]
    fn length_on_blocked_line_conflicts() {
        let mut sketch = Sketch::new("block_conflict");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Block { element: line });
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        assert!(matches!(
            solve(&mut sketch),
            SolveOutcome::NotConverged { .. }
        ));
    }

    #[test]
    fn symmetric_about_point_centers_pair() {
        let mut sketch = Sketch::new("sym_point");
        let p1 = add_point(&mut sketch, 2.0, 3.0);
        let center = add_point(&mut sketch, 5.0, 5.0);
        let p2 = add_point(&mut sketch, 9.0, 9.0);
        fix(&mut sketch, p1, 2.0, 3.0);
        fix(&mut sketch, center, 5.0, 5.0);
        sketch.add_constraint(ConstraintKind::SymmetricAboutPoint {
            point1: p1,
            point2: p2,
            center,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p2).x, 8.0, 1e-3);
        assert_near(pos(&sketch, p2).y, 7.0, 1e-3);
    }

    #[test]
    fn angle_to_axis_rotates_line() {
        let mut sketch = Sketch::new("angle_axis");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 1.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::AngleToAxis {
            line,
            axis: AxisDirection::Horizontal,
            angle_rad: PI / 4.0,
        });
        assert_converged(solve(&mut sketch));
        let dir = pos(&sketch, b) - pos(&sketch, a);
        assert_near(dir.y.atan2(dir.x), PI / 4.0, 1e-3);

        // Vertical axis, zero angle: the line becomes vertical.
        let c = add_point(&mut sketch, 20.0, 0.0);
        let d = add_point(&mut sketch, 21.0, 10.0);
        let line2 = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 20.0, 0.0);
        sketch.add_constraint(ConstraintKind::AngleToAxis {
            line: line2,
            axis: AxisDirection::Vertical,
            angle_rad: 0.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, d).x, pos(&sketch, c).x, 1e-3);
    }

    #[test]
    fn tangent_line_arc_moves_arc_center() {
        let mut sketch = Sketch::new("tangent_arc");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let center = add_point(&mut sketch, 5.0, 3.0);
        let start = add_point(&mut sketch, 7.0, 3.0);
        let end = add_point(&mut sketch, 5.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 2.0)));
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: line,
            item2: arc,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, center).y, 2.0, 1e-3);
        // Implicit arc consistency: endpoints follow the radius.
        assert_near(
            (pos(&sketch, start) - pos(&sketch, center)).length(),
            2.0,
            1e-3,
        );
    }

    #[test]
    fn tangent_arc_arc_separates_centers() {
        let mut sketch = Sketch::new("tangent_arcs");
        let make_arc = |sketch: &mut Sketch, cx: f32, r: f32| {
            let c = add_point(sketch, cx, 0.0);
            let s = add_point(sketch, cx + r, 0.0);
            let e = add_point(sketch, cx, r);
            let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, r)));
            (c, arc)
        };
        let (c1, arc1) = make_arc(&mut sketch, 0.0, 3.0);
        let (c2, arc2) = make_arc(&mut sketch, 10.0, 4.0);
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc1,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc2,
            radius: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: arc1,
            item2: arc2,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 7.0, 1e-3);
    }

    #[test]
    fn point_on_ellipse_pulls_point_to_boundary() {
        let mut sketch = Sketch::new("point_on_ellipse");
        let center = add_point(&mut sketch, 0.0, 0.0);
        fix(&mut sketch, center, 0.0, 0.0);
        let ellipse = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
            center,
            Vec2D::new(4.0, 0.0),
            0.5,
        )));
        // Off the minor vertex: pulled to (0, 2).
        let p = add_point(&mut sketch, 0.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnEllipse { point: p, ellipse });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x, 0.0, 1e-3);
        assert_near(pos(&sketch, p).y, 2.0, 1e-3);
    }

    #[test]
    fn reference_dimension_does_not_constrain() {
        let mut sketch = Sketch::new("reference");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        let mut reference = Constraint::new(ConstraintKind::Length { line, length: 10.0 });
        reference.driving = false;
        sketch.constraints.push(reference);
        assert_converged(solve(&mut sketch));
        // The line keeps its 6-long geometry; the reference only measures.
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 6.0, 1e-4);
        assert_near(
            crate::sketch::measured_value(&sketch, &sketch.constraints[1].kind).unwrap(),
            6.0,
            1e-4,
        );
    }

    #[test]
    fn inactive_constraint_is_skipped() {
        let mut sketch = Sketch::new("inactive");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let mut disabled = Constraint::new(ConstraintKind::Length { line, length: 8.0 });
        disabled.active = false;
        sketch.constraints.push(disabled);
        // With the contradictory 8-length disabled, the solve converges.
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 5.0, 1e-3);
    }

    #[test]
    fn diagnose_flags_conflicting_lengths() {
        let mut sketch = Sketch::new("diag_conflict");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        let len5 = sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let len8 = sketch.add_constraint(ConstraintKind::Length { line, length: 8.0 });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.contains(&len5));
        assert!(diag.conflicting.contains(&len8));
        // The anchor is not part of the conflict: removing it doesn't help.
        assert_eq!(diag.conflicting.len(), 2);
    }

    #[test]
    fn diagnose_flags_redundant_duplicate_parallel() {
        let mut sketch = Sketch::new("diag_redundant");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let base = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let c = add_point(&mut sketch, 0.0, 2.0);
        let d = add_point(&mut sketch, 8.0, 2.0);
        let other = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 2.0);
        let par1 = sketch.add_constraint(ConstraintKind::Parallel {
            line1: base,
            line2: other,
        });
        let par2 = sketch.add_constraint(ConstraintKind::Parallel {
            line1: base,
            line2: other,
        });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.is_empty());
        // Either copy can go without losing rank: both are flagged.
        assert!(diag.redundant.contains(&par1));
        assert!(diag.redundant.contains(&par2));
        // The fixes are all independent: none flagged.
        assert_eq!(diag.redundant.len(), 2);
    }

    #[test]
    fn diagnose_healthy_sketch_reports_nothing() {
        let mut sketch = Sketch::new("diag_ok");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.is_empty() && diag.redundant.is_empty());
        assert_eq!(diag.dof, 1, "the free endpoint's x remains");
    }

    #[test]
    fn diagnose_early_outs_above_constraint_limit() {
        let mut sketch = Sketch::new("diag_limit");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
        for _ in 0..61 {
            sketch.add_constraint(ConstraintKind::Radius {
                circle,
                radius: 5.0,
            });
        }
        let diag = diagnose(&sketch);
        assert!(!diag.analyzed, "too many constraints to analyze");
        assert!(diag.conflicting.is_empty() && diag.redundant.is_empty());
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    use crate::sketch::{ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
    use crate::sketch::{ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID};

    fn add_point(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    #[test]
    fn a_point_made_coincident_with_the_origin_lands_on_it() {
        let mut sketch = Sketch::new("t");
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: p,
            point2: ORIGIN_ID,
        });
        solve(&mut sketch);
        let at = sketch.point_position(p).unwrap();
        assert!(at.x.abs() < 1e-4 && at.y.abs() < 1e-4, "landed at {at:?}");
    }

    #[test]
    fn a_point_put_on_an_axis_slides_onto_it() {
        let mut sketch = Sketch::new("t");
        let p = add_point(&mut sketch, 5.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnLine {
            point: p,
            line: X_AXIS_ID,
        });
        solve(&mut sketch);
        let at = sketch.point_position(p).unwrap();
        assert!(at.y.abs() < 1e-4, "dropped to the axis: {at:?}");
        assert!((at.x - 5.0).abs() < 0.5, "slid along it, not to the origin");

        let q = add_point(&mut sketch, 4.0, 7.0);
        sketch.add_constraint(ConstraintKind::PointOnLine {
            point: q,
            line: Y_AXIS_ID,
        });
        solve(&mut sketch);
        let at = sketch.point_position(q).unwrap();
        assert!(at.x.abs() < 1e-4, "dropped to the vertical axis: {at:?}");
    }

    #[test]
    fn the_references_hold_still_and_cost_no_freedom() {
        // A line from the origin along the X axis with a driving length has
        // nothing left to move.
        let mut sketch = Sketch::new("t");
        let a = add_point(&mut sketch, 1.0, 1.0);
        let b = add_point(&mut sketch, 6.0, 2.0);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: a,
            point2: ORIGIN_ID,
        });
        sketch.add_constraint(ConstraintKind::PointOnLine {
            point: b,
            line: X_AXIS_ID,
        });
        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        solve(&mut sketch);

        assert_eq!(dof_estimate(&sketch), 0, "nothing is free any more");
        assert!(sketch.is_fully_constrained);
        let start = sketch.point_position(a).unwrap();
        let end = sketch.point_position(b).unwrap();
        assert!(start.x.abs() < 1e-4 && start.y.abs() < 1e-4);
        assert!(end.y.abs() < 1e-4 && (end.x.abs() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_sketch_that_never_mentions_them_solves_exactly_as_before() {
        let mut sketch = Sketch::new("t");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 4.0, 0.0);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(ConstraintKind::Length { line, length: 6.0 });
        let before = dof_estimate(&sketch);
        solve(&mut sketch);
        assert_eq!(before, 3, "four point variables, one length residual");
        assert_eq!(dof_estimate(&sketch), 3);
    }
}
#[cfg(test)]
mod freedom {
    use super::*;
    use crate::sketch::{Line, Point};

    #[test]
    fn a_pinned_line_frees_its_far_end_until_length_and_direction_hold_it() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(ConstraintKind::FixedPoint {
            point: a,
            position: Vec2D::new(0.0, 0.0),
        });
        let free = free_points(&sketch);
        assert!(!free.contains(&a), "the pinned point stays put");
        assert!(free.contains(&b), "the far end swings");

        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert!(free_points(&sketch).is_empty(), "nothing moves any more");
    }
}

#[cfg(test)]
mod curve_constraints {
    use super::*;
    use crate::measure;
    use crate::sketch::{Arc, Circle, Line, Point};

    fn point(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    fn line(sketch: &mut Sketch, a: Uuid, b: Uuid) -> Uuid {
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)))
    }

    fn circle(sketch: &mut Sketch, x: f32, y: f32, r: f32) -> (Uuid, Uuid) {
        let c = point(sketch, x, y);
        (
            c,
            sketch.add_geometry(GeometryElement::Circle(Circle::new(c, r))),
        )
    }

    fn fix(sketch: &mut Sketch, p: Uuid) {
        let position = sketch.point_position(p).unwrap();
        sketch.add_constraint(ConstraintKind::FixedPoint { point: p, position });
    }

    fn converged(sketch: &mut Sketch) {
        let outcome = solve(sketch);
        assert!(
            matches!(outcome, SolveOutcome::Converged { .. }),
            "{outcome:?}"
        );
    }

    fn near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn an_arc_length_sweeps_the_end_round() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 5.0, 0.0);
        let e = point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        fix(&mut sketch, c);
        fix(&mut sketch, s);
        let three_eighths = 5.0 * std::f32::consts::PI * 0.75;
        sketch.add_constraint(ConstraintKind::ArcLength {
            arc,
            length: three_eighths,
        });
        converged(&mut sketch);
        near(measure::arc_length(&sketch, arc).unwrap(), three_eighths);
        let end = sketch.point_position(e).unwrap();
        near(end.x, -5.0 / 2f32.sqrt());
        near(end.y, 5.0 / 2f32.sqrt());
    }

    #[test]
    fn an_arc_opens_to_its_angle_and_three_points_hold_theirs() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 5.0, 0.0);
        let e = point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        fix(&mut sketch, c);
        fix(&mut sketch, s);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc,
            radius: 5.0,
        });
        sketch.add_constraint(ConstraintKind::ArcAngle {
            arc,
            angle_rad: 135f32.to_radians(),
        });
        converged(&mut sketch);
        near(
            measure::arc_sweep(&sketch, arc).unwrap().to_degrees(),
            135.0,
        );

        // Picked arm, corner, arm: the corner is the vertex, and a reflex
        // pick is turned round to the short way.
        let mut sketch = Sketch::new("t");
        let arm1 = point(&mut sketch, 10.0, 0.0);
        let corner = point(&mut sketch, 0.0, 0.0);
        let arm2 = point(&mut sketch, 0.0, 10.0);
        let selected: std::collections::HashSet<Uuid> = [arm1, corner, arm2].into();
        let shape =
            crate::constrain::SelectionShape::picked_in(&sketch, &selected, &[arm2, corner, arm1]);
        let kinds = crate::constrain::kinds_for("angle_three_points", &shape, &sketch).unwrap();
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
        fix(&mut sketch, arm1);
        fix(&mut sketch, corner);
        sketch.add_constraint(with_dimension_value_kind(&kinds[0], 60.0));
        sketch.add_constraint(ConstraintKind::Distance {
            point1: corner,
            point2: arm2,
            distance: 10.0,
        });
        converged(&mut sketch);
        let at = sketch.point_position(arm2).unwrap();
        near(at.y.atan2(at.x).to_degrees(), 60.0);
    }

    fn with_dimension_value_kind(kind: &ConstraintKind, v: f32) -> ConstraintKind {
        crate::sketch::with_dimension_value(kind, v)
    }

    /// The distance from `p` to the nearest of `samples`.
    fn off(samples: &[glam::Vec2], p: glam::Vec2) -> f32 {
        samples
            .iter()
            .map(|q| (*q - p).length())
            .fold(f32::INFINITY, f32::min)
    }

    fn spline_samples(sketch: &Sketch, spline: Uuid) -> Vec<glam::Vec2> {
        let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
            panic!("not a spline");
        };
        let basis = crate::spline::basis_of(b).unwrap();
        let control: Vec<[f64; 2]> = b
            .control_points
            .iter()
            .map(|id| {
                let p = sketch.point_position(*id).unwrap();
                [f64::from(p.x), f64::from(p.y)]
            })
            .collect();
        crate::spline::sample_basis(&basis, &control, 2000)
            .into_iter()
            .map(|p| p.to_glam())
            .collect()
    }

    #[test]
    fn a_point_is_held_on_a_spline_and_on_a_parabola() {
        let mut sketch = Sketch::new("t");
        let control: Vec<Uuid> = [(0.0, 0.0), (5.0, 8.0), (10.0, -4.0), (15.0, 3.0)]
            .iter()
            .map(|(x, y)| point(&mut sketch, *x, *y))
            .collect();
        for p in &control {
            fix(&mut sketch, *p);
        }
        let spline = sketch.add_geometry(GeometryElement::BSpline(crate::sketch::BSpline::new(
            control, false,
        )));
        let p = point(&mut sketch, 7.0, 9.0);
        sketch.add_constraint(ConstraintKind::PointOnCurve {
            point: p,
            curve: spline,
        });
        converged(&mut sketch);
        let at = sketch.point_position(p).unwrap().to_glam();
        assert!(off(&spline_samples(&sketch, spline), at) < 0.02, "{at:?}");

        // y² = 4·2·x about the origin, opening along +x.
        let mut sketch = Sketch::new("t");
        let vertex = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 2.0, -4.0);
        let e = point(&mut sketch, 8.0, 8.0);
        fix(&mut sketch, vertex);
        let conic = sketch.add_geometry(GeometryElement::Conic(crate::sketch::Conic::new(
            crate::sketch::ConicKind::Parabola,
            vertex,
            Vec2D::new(2.0, 0.0),
            0.0,
            s,
            e,
        )));
        let p = point(&mut sketch, 5.0, 1.0);
        sketch.add_constraint(ConstraintKind::PointOnCurve {
            point: p,
            curve: conic,
        });
        converged(&mut sketch);
        let at = sketch.point_position(p).unwrap();
        near(at.y * at.y, 8.0 * at.x);
    }

    #[test]
    fn an_ellipse_takes_its_radii_and_a_spline_its_length() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        fix(&mut sketch, c);
        let ellipse = sketch.add_geometry(GeometryElement::Ellipse(crate::sketch::Ellipse::new(
            c,
            Vec2D::new(6.0, 0.0),
            0.5,
        )));
        for (major, radius) in [(true, 8.0), (false, 2.0)] {
            sketch.add_constraint(ConstraintKind::EllipseRadius {
                ellipse,
                major,
                radius,
            });
        }
        converged(&mut sketch);
        let Some(GeometryElement::Ellipse(e)) = sketch.get_geometry(ellipse) else {
            unreachable!()
        };
        near(e.major.to_glam().length(), 8.0);
        near(e.major.to_glam().length() * e.ratio, 2.0);

        let mut sketch = Sketch::new("t");
        let control: Vec<Uuid> = [(0.0, 0.0), (5.0, 8.0), (10.0, -4.0), (15.0, 3.0)]
            .iter()
            .map(|(x, y)| point(&mut sketch, *x, *y))
            .collect();
        for p in &control[..3] {
            fix(&mut sketch, *p);
        }
        let spline = sketch.add_geometry(GeometryElement::BSpline(crate::sketch::BSpline::new(
            control.clone(),
            false,
        )));
        let now = measure::curve_length(&sketch, spline).unwrap();
        sketch.add_constraint(ConstraintKind::CurveLength {
            curve: spline,
            length: now + 4.0,
        });
        converged(&mut sketch);
        let after = measure::curve_length(&sketch, spline).unwrap();
        assert!((after - (now + 4.0)).abs() < 0.05, "{now} -> {after}");
    }

    #[test]
    fn a_line_touches_an_ellipse_and_crosses_a_spline_square() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        fix(&mut sketch, c);
        let ellipse = sketch.add_geometry(GeometryElement::Ellipse(crate::sketch::Ellipse::new(
            c,
            Vec2D::new(6.0, 0.0),
            0.5,
        )));
        let (a, b) = (
            point(&mut sketch, -10.0, 5.0),
            point(&mut sketch, 10.0, 4.0),
        );
        let l = line(&mut sketch, a, b);
        fix(&mut sketch, a);
        sketch.add_constraint(ConstraintKind::TangentCurves {
            curve1: l,
            curve2: ellipse,
        });
        converged(&mut sketch);
        // The line's nearest approach to the ellipse is a touch.
        let (pa, pb) = (
            sketch.point_position(a).unwrap().to_glam(),
            sketch.point_position(b).unwrap().to_glam(),
        );
        let ring: Vec<glam::Vec2> = (0..4000)
            .map(|i| {
                let t = i as f32 / 4000.0 * std::f32::consts::TAU;
                glam::Vec2::new(6.0 * t.cos(), 3.0 * t.sin())
            })
            .collect();
        let dir = (pb - pa).normalize();
        let gap = ring
            .iter()
            .map(|q| (*q - pa).perp_dot(dir).abs())
            .fold(f32::INFINITY, f32::min);
        assert!(gap < 0.01, "touches: {gap}");

        let mut sketch = Sketch::new("t");
        let control: Vec<Uuid> = [(0.0, 0.0), (5.0, 8.0), (10.0, -4.0), (15.0, 3.0)]
            .iter()
            .map(|(x, y)| point(&mut sketch, *x, *y))
            .collect();
        for p in &control {
            fix(&mut sketch, *p);
        }
        let spline = sketch.add_geometry(GeometryElement::BSpline(crate::sketch::BSpline::new(
            control, false,
        )));
        let (a, b) = (point(&mut sketch, 7.0, -6.0), point(&mut sketch, 8.0, 8.0));
        let l = line(&mut sketch, a, b);
        fix(&mut sketch, a);
        sketch.add_constraint(ConstraintKind::PerpendicularCurves {
            curve1: l,
            curve2: spline,
        });
        converged(&mut sketch);
        let samples = spline_samples(&sketch, spline);
        let (pa, pb) = (
            sketch.point_position(a).unwrap().to_glam(),
            sketch.point_position(b).unwrap().to_glam(),
        );
        let dir = (pb - pa).normalize();
        // Where the line meets the spline, the spline runs across it.
        let (i, _) = samples
            .iter()
            .enumerate()
            .map(|(i, q)| (i, (*q - pa).perp_dot(dir).abs()))
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .unwrap();
        let along = (samples[i + 1] - samples[i.saturating_sub(1)]).normalize();
        assert!(along.dot(dir).abs() < 0.02, "square: {}", along.dot(dir));
    }

    #[test]
    fn the_solver_goes_as_far_as_the_sketch_lets_it() {
        let build = || {
            let mut sketch = Sketch::new("t");
            let a = point(&mut sketch, 0.0, 0.0);
            let b = point(&mut sketch, 1.0, 0.5);
            fix(&mut sketch, a);
            sketch.add_constraint(ConstraintKind::Distance {
                point1: a,
                point2: b,
                distance: 500.0,
            });
            sketch
        };
        let mut short = build();
        short.solver.max_iterations = 1;
        assert!(matches!(
            solve(&mut short),
            SolveOutcome::NotConverged { .. }
        ));
        let mut full = build();
        assert!(matches!(solve(&mut full), SolveOutcome::Converged { .. }));
        // Settings never changed are not written with the sketch.
        let saved = serde_json::to_value(&full).unwrap();
        assert!(saved.get("solver").is_none(), "{saved}");
        short.solver.max_iterations = 50;
        let saved = serde_json::to_value(&short).unwrap();
        let back: Sketch = serde_json::from_value(saved).unwrap();
        assert_eq!(back.solver.max_iterations, 50);
    }

    #[test]
    fn two_ends_picked_are_joined_smoothly() {
        let mut sketch = Sketch::new("t");
        let (a, b) = (point(&mut sketch, 0.0, 0.0), point(&mut sketch, 10.0, 0.0));
        let line = line(&mut sketch, a, b);
        fix(&mut sketch, a);
        fix(&mut sketch, b);
        let c = point(&mut sketch, 12.0, 6.0);
        let s = point(&mut sketch, 12.0, 1.0);
        let e = point(&mut sketch, 17.0, 6.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        let selected: std::collections::HashSet<Uuid> = [b, s].into();
        let shape = crate::constrain::SelectionShape::of(&sketch, &selected);
        let kinds = crate::constrain::kinds_for("tangent", &shape, &sketch).unwrap();
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
        near(joint.x, 10.0);
        near(joint.y, 0.0);
        let center = sketch.point_position(c).unwrap();
        near(center.x, 10.0);
        let r = match sketch.get_geometry(arc) {
            Some(GeometryElement::Arc(a)) => a.radius,
            _ => unreachable!(),
        };
        near(center.y.abs(), r);

        // A point and the two curves meeting at it take the same join.
        let selected: std::collections::HashSet<Uuid> = [b, line, arc].into();
        let shape = crate::constrain::SelectionShape::of(&sketch, &selected);
        let kinds = crate::constrain::kinds_for("tangent", &shape, &sketch).unwrap();
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, ConstraintKind::Tangent { .. })),
            "{kinds:?}"
        );
    }

    #[test]
    fn two_ellipses_are_held_the_same_size() {
        let mut sketch = Sketch::new("t");
        let c1 = point(&mut sketch, 0.0, 0.0);
        let c2 = point(&mut sketch, 20.0, 0.0);
        let e1 = sketch.add_geometry(GeometryElement::Ellipse(crate::sketch::Ellipse::new(
            c1,
            Vec2D::new(6.0, 0.0),
            0.5,
        )));
        let e2 = sketch.add_geometry(GeometryElement::Ellipse(crate::sketch::Ellipse::new(
            c2,
            Vec2D::new(0.0, 4.0),
            0.25,
        )));
        sketch.add_constraint(ConstraintKind::EqualEllipse {
            ellipse1: e1,
            ellipse2: e2,
        });
        converged(&mut sketch);
        let size = |id: Uuid| match sketch.get_geometry(id) {
            Some(GeometryElement::Ellipse(e)) => {
                let major = e.major.to_glam().length();
                (major, major * e.ratio)
            }
            _ => panic!("not an ellipse"),
        };
        let ((a1, b1), (a2, b2)) = (size(e1), size(e2));
        near(a1, a2);
        near(b1, b2);
    }

    #[test]
    fn two_points_are_held_level_and_plumb() {
        let mut sketch = Sketch::new("t");
        let a = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, 10.0, 2.0);
        let c = point(&mut sketch, 3.0, 8.0);
        fix(&mut sketch, a);
        sketch.add_constraint(ConstraintKind::HorizontalPoints {
            point1: a,
            point2: b,
        });
        sketch.add_constraint(ConstraintKind::VerticalPoints {
            point1: a,
            point2: c,
        });
        converged(&mut sketch);
        near(sketch.point_position(b).unwrap().y, 0.0);
        near(sketch.point_position(c).unwrap().x, 0.0);
    }

    #[test]
    fn a_gap_holds_two_parallel_lines_apart_and_the_dimension_tool_picks_it() {
        let mut sketch = Sketch::new("t");
        let (a, b) = (point(&mut sketch, 0.0, 0.0), point(&mut sketch, 10.0, 0.0));
        let (c, d) = (point(&mut sketch, 0.0, 3.0), point(&mut sketch, 10.0, 3.0));
        let (bottom, top) = (line(&mut sketch, a, b), line(&mut sketch, c, d));
        fix(&mut sketch, a);
        fix(&mut sketch, b);
        sketch.add_constraint(ConstraintKind::Parallel {
            line1: bottom,
            line2: top,
        });
        let shape = crate::constrain::SelectionShape::of(
            &sketch,
            &std::collections::HashSet::from([bottom, top]),
        );
        assert_eq!(
            crate::constrain::dimension_in(&shape, &sketch),
            Some("distance"),
            "parallel lines take a distance, not an angle"
        );
        let kinds = crate::constrain::kinds_for("distance", &shape, &sketch).unwrap();
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

    #[test]
    fn a_gap_holds_two_circles_apart_and_one_inside_another() {
        let mut sketch = Sketch::new("t");
        let (c1, a) = circle(&mut sketch, 0.0, 0.0, 2.0);
        let (_, b) = circle(&mut sketch, 9.0, 1.0, 3.0);
        fix(&mut sketch, c1);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: a,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: b,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Gap {
            item1: a,
            item2: b,
            distance: 1.5,
        });
        converged(&mut sketch);
        near(measure::gap(&sketch, a, b).unwrap().distance, 1.5);

        let mut sketch = Sketch::new("nested");
        let (c1, big) = circle(&mut sketch, 0.0, 0.0, 10.0);
        let (c2, small) = circle(&mut sketch, 1.0, 1.0, 2.0);
        fix(&mut sketch, c1);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: big,
            radius: 10.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: small,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Gap {
            item1: small,
            item2: big,
            distance: 1.0,
        });
        converged(&mut sketch);
        near(measure::gap(&sketch, small, big).unwrap().distance, 1.0);
        near(sketch.point_position(c2).unwrap().to_glam().length(), 7.0);
    }

    #[test]
    fn a_gap_keeps_a_circle_off_a_line_and_points_off_both() {
        let mut sketch = Sketch::new("t");
        let s = point(&mut sketch, -10.0, 0.0);
        let e = point(&mut sketch, 10.0, 0.0);
        let ground = line(&mut sketch, s, e);
        fix(&mut sketch, s);
        fix(&mut sketch, e);
        let (_, wheel) = circle(&mut sketch, 1.0, 6.0, 2.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: wheel,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Gap {
            item1: ground,
            item2: wheel,
            distance: 0.5,
        });
        let p = point(&mut sketch, 3.0, -4.0);
        sketch.add_constraint(ConstraintKind::Gap {
            item1: p,
            item2: ground,
            distance: 2.0,
        });
        let q = point(&mut sketch, 1.0, 12.0);
        sketch.add_constraint(ConstraintKind::Gap {
            item1: wheel,
            item2: q,
            distance: 3.0,
        });
        converged(&mut sketch);
        near(measure::gap(&sketch, ground, wheel).unwrap().distance, 0.5);
        near(sketch.point_position(p).unwrap().y, -2.0);
        near(measure::gap(&sketch, q, wheel).unwrap().distance, 3.0);
    }

    #[test]
    fn an_angle_at_a_point_turns_a_line_against_an_arc() {
        let mut sketch = Sketch::new("t");
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 5.0, 0.0);
        let e = point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        sketch.add_constraint(ConstraintKind::Block { element: arc });
        let far = point(&mut sketch, 9.0, 1.0);
        let spoke = line(&mut sketch, s, far);
        sketch.add_constraint(ConstraintKind::Length {
            line: spoke,
            length: 4.0,
        });
        let angle = 60f32.to_radians();
        sketch.add_constraint(ConstraintKind::AngleAtPoint {
            curve1: spoke,
            curve2: arc,
            point: s,
            angle_rad: angle,
        });
        converged(&mut sketch);
        near(
            measure::angle_at_point(&sketch, spoke, arc, s).unwrap(),
            angle,
        );
        // The arc's tangent at its start points straight up; the spoke
        // runs 60° clockwise of it.
        let d = (sketch.point_position(far).unwrap() - sketch.point_position(s).unwrap()).to_glam();
        near(d.y.atan2(d.x), 30f32.to_radians());
    }

    #[test]
    fn a_refraction_bends_the_ray_leaving_the_interface() {
        let mut sketch = Sketch::new("t");
        let l = point(&mut sketch, -10.0, 0.0);
        let r = point(&mut sketch, 10.0, 0.0);
        let interface = line(&mut sketch, l, r);
        sketch.add_constraint(ConstraintKind::Block { element: interface });
        let source = point(&mut sketch, -4.0, 4.0);
        let hit = point(&mut sketch, 0.0, 0.0);
        let exit = point(&mut sketch, 3.0, -5.0);
        let ray1 = line(&mut sketch, source, hit);
        let ray2 = line(&mut sketch, hit, exit);
        fix(&mut sketch, source);
        fix(&mut sketch, hit);
        sketch.add_constraint(ConstraintKind::Length {
            line: ray2,
            length: 5.0,
        });
        sketch.add_constraint(ConstraintKind::Refraction {
            ray1,
            ray2,
            interface,
            point: hit,
            ratio: 1.5,
        });
        converged(&mut sketch);
        near(
            measure::refraction_ratio(&sketch, ray1, ray2, interface, hit).unwrap(),
            1.5,
        );
        // sin 45° / sin θ = 1.5, and the ray carries on below.
        let out = sketch.point_position(exit).unwrap().to_glam();
        near(out.x / out.length(), 45f32.to_radians().sin() / 1.5);
        assert!(out.y < 0.0, "refracted, not reflected: {out:?}");
    }

    #[test]
    fn a_new_constraint_supersedes_the_older_one_it_repeats() {
        let mut sketch = Sketch::new("t");
        let a = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, 10.0, 0.0);
        let level = line(&mut sketch, a, b);
        let horizontal = sketch.add_constraint(ConstraintKind::Horizontal { element: level });
        let length = sketch.add_constraint(ConstraintKind::Length {
            line: level,
            length: 10.0,
        });
        let parallel = sketch.add_constraint(ConstraintKind::Parallel {
            line1: level,
            line2: crate::sketch::X_AXIS_ID,
        });
        assert_eq!(superseded(&sketch, &[parallel]), vec![horizontal]);
        assert!(superseded(&sketch, &[length]).is_empty(), "nothing to take");

        // Redundancy the sketch had already is none of the new one's doing.
        let mut sketch = Sketch::new("t");
        let a = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, 10.0, 0.0);
        let level = line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Horizontal { element: level });
        sketch.add_constraint(ConstraintKind::Horizontal { element: level });
        let length = sketch.add_constraint(ConstraintKind::Length {
            line: level,
            length: 10.0,
        });
        assert!(superseded(&sketch, &[length]).is_empty());
    }
}
