use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure;
    use crate::sketch::{Arc, Circle, Constraint, Ellipse, Line, Point};
    use crate::sketch::{ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID};
    use std::f64::consts::PI;

    fn add_point(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    fn add_line(sketch: &mut Sketch, start: Uuid, end: Uuid) -> Uuid {
        sketch.add_geometry(GeometryElement::Line(Line::new(start, end)))
    }

    fn fix(sketch: &mut Sketch, point: Uuid, x: f64, y: f64) {
        sketch.add_constraint(ConstraintKind::FixedPoint {
            point,
            position: Vec2D::new(x, y),
        });
    }

    fn pos(sketch: &Sketch, id: Uuid) -> glam::DVec2 {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Point(p)) => p.position.to_glam(),
            _ => panic!("expected point"),
        }
    }

    fn circle_radius(sketch: &Sketch, id: Uuid) -> f64 {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Circle(c)) => c.radius,
            Some(GeometryElement::Arc(a)) => a.radius,
            _ => panic!("expected circle or arc"),
        }
    }

    /// A line between two points, both fixed where they stand: its two
    /// ends and the line.
    fn fixed_line(sketch: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> (Uuid, Uuid, Uuid) {
        let (pa, pb) = (add_point(sketch, a.0, a.1), add_point(sketch, b.0, b.1));
        let line = add_line(sketch, pa, pb);
        fix(sketch, pa, a.0, a.1);
        fix(sketch, pb, b.0, b.1);
        (pa, pb, line)
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

    fn circle(sketch: &mut Sketch, x: f64, y: f64, r: f64) -> (Uuid, Uuid) {
        let c = add_point(sketch, x, y);
        (
            c,
            sketch.add_geometry(GeometryElement::Circle(Circle::new(c, r))),
        )
    }

    fn ellipse(sketch: &mut Sketch, centre: Uuid, major: (f64, f64), ratio: f64) -> Uuid {
        let major = Vec2D::new(major.0, major.1);
        sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(centre, major, ratio)))
    }

    /// An open cubic through (0, 0), (5, 8), (10, -4) and (15, 3), its
    /// first `fixed` control points fixed.
    fn wavy_spline(sketch: &mut Sketch, fixed: usize) -> Uuid {
        let control: Vec<Uuid> = [(0.0, 0.0), (5.0, 8.0), (10.0, -4.0), (15.0, 3.0)]
            .iter()
            .map(|(x, y)| add_point(sketch, *x, *y))
            .collect();
        for p in &control[..fixed] {
            pin(sketch, *p);
        }
        sketch.add_geometry(GeometryElement::BSpline(crate::sketch::BSpline::new(
            control, false,
        )))
    }

    fn near(actual: f64, expected: f64) {
        assert_near(actual, expected, 1e-3);
    }

    /// The constraints the tests add most, by kind; each answers its id.
    trait Constrain {
        fn radius(&mut self, circle: Uuid, radius: f64) -> Uuid;
        fn length(&mut self, line: Uuid, length: f64) -> Uuid;
        fn gap(&mut self, item1: Uuid, item2: Uuid, distance: f64) -> Uuid;
        fn tangent(&mut self, line_or_circle1: Uuid, item2: Uuid) -> Uuid;
        fn coincident(&mut self, point1: Uuid, point2: Uuid) -> Uuid;
        fn parallel(&mut self, line1: Uuid, line2: Uuid) -> Uuid;
    }

    impl Constrain for Sketch {
        fn radius(&mut self, circle: Uuid, radius: f64) -> Uuid {
            self.add_constraint(ConstraintKind::Radius { circle, radius })
        }
        fn length(&mut self, line: Uuid, length: f64) -> Uuid {
            self.add_constraint(ConstraintKind::Length { line, length })
        }
        fn gap(&mut self, item1: Uuid, item2: Uuid, distance: f64) -> Uuid {
            let kind = ConstraintKind::Gap {
                item1,
                item2,
                distance,
            };
            self.add_constraint(kind)
        }
        fn tangent(&mut self, line_or_circle1: Uuid, item2: Uuid) -> Uuid {
            let kind = ConstraintKind::Tangent {
                line_or_circle1,
                item2,
            };
            self.add_constraint(kind)
        }
        fn coincident(&mut self, point1: Uuid, point2: Uuid) -> Uuid {
            self.add_constraint(ConstraintKind::Coincident { point1, point2 })
        }
        fn parallel(&mut self, line1: Uuid, line2: Uuid) -> Uuid {
            self.add_constraint(ConstraintKind::Parallel { line1, line2 })
        }
    }

    fn assert_near(actual: f64, expected: f64, tol: f64) {
        assert!(
            (actual - expected).abs() <= tol,
            "expected {expected}, got {actual} (tol {tol})"
        );
    }

    #[test]
    fn constraints_on_missing_geometry_are_skipped() {
        let mut sketch = Sketch::new("missing");
        add_point(&mut sketch, 1.0, 2.0);
        sketch.length(Uuid::new_v4(), 5.0);
        sketch.coincident(Uuid::new_v4(), Uuid::new_v4());
        // Horizontal on a non-line element is also skipped.
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: p });
        assert_eq!(solve(&mut sketch), SolveOutcome::NothingToSolve);
    }

    #[test]
    fn length_sets_line_length_keeping_direction() {
        let mut sketch = Sketch::new("length");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 3.0, 4.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.length(line, 10.0);
        converged(&mut sketch);
        let dir = (pos(&sketch, b) - pos(&sketch, a)).normalize();
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 10.0, 1e-3);
        // Direction should be roughly preserved (initially (0.6, 0.8)).
        assert!(dir.dot(glam::DVec2::new(0.6, 0.8)) > 0.9);
    }

    #[test]
    fn coincident_merges_two_points() {
        let mut sketch = Sketch::new("coincident");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 2.0, 2.0);
        sketch.coincident(p1, p2);
        converged(&mut sketch);
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
        converged(&mut sketch);
        assert_near((pos(&sketch, p2) - pos(&sketch, p1)).length(), 5.0, 1e-3);
    }

    #[test]
    fn equal_radius_matches_two_circles() {
        let mut sketch = Sketch::new("equal_radius");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 10.0, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 2.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 6.0)));
        sketch.add_constraint(ConstraintKind::EqualRadius { circle1, circle2 });
        converged(&mut sketch);
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
        converged(&mut sketch);
        assert_near((pos(&sketch, d) - pos(&sketch, c)).length(), 10.0, 1e-3);
    }

    #[test]
    fn parallel_and_perpendicular_pair() {
        let mut sketch = Sketch::new("parallel_perpendicular");
        let (_, _, base) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));

        let c = add_point(&mut sketch, 0.0, 2.0);
        let d = add_point(&mut sketch, 8.0, 4.0);
        let para = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 2.0);
        sketch.parallel(base, para);

        let e = add_point(&mut sketch, 5.0, 1.0);
        let f = add_point(&mut sketch, 6.0, 9.0);
        let perp = add_line(&mut sketch, e, f);
        fix(&mut sketch, e, 5.0, 1.0);
        sketch.add_constraint(ConstraintKind::Perpendicular {
            line1: base,
            line2: perp,
        });

        converged(&mut sketch);
        // Parallel to the x-axis base: equal y at both ends.
        assert_near(pos(&sketch, d).y, pos(&sketch, c).y, 1e-3);
        // Perpendicular to the x-axis base: equal x at both ends.
        assert_near(pos(&sketch, f).x, pos(&sketch, e).x, 1e-3);
    }

    #[test]
    fn point_on_line_projects_point() {
        let mut sketch = Sketch::new("point_on_line");
        let (_, _, line) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));
        let p = add_point(&mut sketch, 4.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnLine { point: p, line });
        converged(&mut sketch);
        assert_near(pos(&sketch, p).y, 0.0, 1e-3);
    }

    #[test]
    fn point_on_circle_snaps_to_radius() {
        let mut sketch = Sketch::new("point_on_circle");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
        fix(&mut sketch, center, 0.0, 0.0);
        sketch.radius(circle, 5.0);
        let p = add_point(&mut sketch, 8.0, 0.0);
        sketch.add_constraint(ConstraintKind::PointOnCircle { point: p, circle });
        converged(&mut sketch);
        assert_near((pos(&sketch, p) - pos(&sketch, center)).length(), 5.0, 1e-3);
    }

    #[test]
    fn angle_constraint_rotates_line() {
        let mut sketch = Sketch::new("angle");
        let (_, _, base) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));
        let c = add_point(&mut sketch, 0.0, 0.0);
        let d = add_point(&mut sketch, 10.0, 1.0);
        let rotated = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Angle {
            line1: base,
            line2: rotated,
            angle_rad: PI / 4.0,
        });
        converged(&mut sketch);
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
        sketch.length(bottom, 4.0);
        sketch.length(right, 3.0);

        converged(&mut sketch);
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
        sketch.length(line, 10.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        converged(&mut sketch);
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 10.0, 1e-3);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn arc_endpoints_follow_radius_constraint() {
        let mut sketch = Sketch::new("arc");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let start = add_point(&mut sketch, 5.0, 0.0);
        let end = add_point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 5.0)));
        fix(&mut sketch, center, 0.0, 0.0);
        sketch.radius(arc, 3.0);
        converged(&mut sketch);
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
        let (_, _, line) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));
        let center = add_point(&mut sketch, 5.0, 3.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.radius(circle, 2.0);
        sketch.tangent(line, circle);
        converged(&mut sketch);
        // Perpendicular distance from the center to the x-axis line must
        // equal the radius (center approached from y=3, so it lands at +2).
        assert_near(pos(&sketch, center).y, 2.0, 1e-3);
        assert_near(circle_radius(&sketch, circle), 2.0, 1e-4);
    }

    #[test]
    fn tangent_line_circle_works_with_swapped_operands() {
        let mut sketch = Sketch::new("tangent_swapped");
        let (_, _, line) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));
        let center = add_point(&mut sketch, 5.0, 3.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        fix(&mut sketch, center, 5.0, 3.0);
        // Circle first, line second: the radius adapts instead.
        sketch.tangent(circle, line);
        converged(&mut sketch);
        assert_near(circle_radius(&sketch, circle), 3.0, 1e-3);
    }

    #[test]
    fn midpoint_pins_point_to_line_center() {
        let mut sketch = Sketch::new("midpoint");
        let (_, _, line) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 4.0));
        let p = add_point(&mut sketch, 7.0, 7.0);
        sketch.add_constraint(ConstraintKind::Midpoint { point: p, line });
        converged(&mut sketch);
        assert_near(pos(&sketch, p).x, 5.0, 1e-3);
        assert_near(pos(&sketch, p).y, 2.0, 1e-3);
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
    fn block_freezes_line_while_other_geometry_moves() {
        let mut sketch = Sketch::new("block");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let blocked_line = add_line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Block {
            element: blocked_line,
        });
        // A free point pulled onto the blocked endpoint: only the point moves.
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.coincident(p, b);
        converged(&mut sketch);
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
        sketch.length(line, 5.0);
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
        converged(&mut sketch);
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
        converged(&mut sketch);
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
        converged(&mut sketch);
        assert_near(pos(&sketch, d).x, pos(&sketch, c).x, 1e-3);
    }

    #[test]
    fn tangent_line_arc_moves_arc_center() {
        let mut sketch = Sketch::new("tangent_arc");
        let (_, _, line) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));
        let center = add_point(&mut sketch, 5.0, 3.0);
        let start = add_point(&mut sketch, 7.0, 3.0);
        let end = add_point(&mut sketch, 5.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 2.0)));
        sketch.radius(arc, 2.0);
        sketch.tangent(line, arc);
        converged(&mut sketch);
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
        let make_arc = |sketch: &mut Sketch, cx: f64, r: f64| {
            let c = add_point(sketch, cx, 0.0);
            let s = add_point(sketch, cx + r, 0.0);
            let e = add_point(sketch, cx, r);
            let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, r)));
            (c, arc)
        };
        let (c1, arc1) = make_arc(&mut sketch, 0.0, 3.0);
        let (c2, arc2) = make_arc(&mut sketch, 10.0, 4.0);
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.radius(arc1, 3.0);
        sketch.radius(arc2, 4.0);
        sketch.tangent(arc1, arc2);
        converged(&mut sketch);
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 7.0, 1e-3);
    }

    #[test]
    fn point_on_ellipse_pulls_point_to_boundary() {
        let mut sketch = Sketch::new("point_on_ellipse");
        let center = add_point(&mut sketch, 0.0, 0.0);
        fix(&mut sketch, center, 0.0, 0.0);
        let ellipse = ellipse(&mut sketch, center, (4.0, 0.0), 0.5);
        // Off the minor vertex: pulled to (0, 2).
        let p = add_point(&mut sketch, 0.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnEllipse { point: p, ellipse });
        converged(&mut sketch);
        assert_near(pos(&sketch, p).x, 0.0, 1e-3);
        assert_near(pos(&sketch, p).y, 2.0, 1e-3);
    }

    /// A reference dimension only measures and an inactive one is left
    /// out: neither fights the driving length.
    #[test]
    fn reference_and_inactive_dimensions_do_not_constrain() {
        let mut sketch = Sketch::new("reference");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.length(line, 5.0);
        let mut reference = Constraint::new(ConstraintKind::Length { line, length: 10.0 });
        reference.driving = false;
        sketch.constraints.push(reference);
        let mut disabled = Constraint::new(ConstraintKind::Length { line, length: 8.0 });
        disabled.active = false;
        sketch.constraints.push(disabled);
        converged(&mut sketch);
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 5.0, 1e-3);
        let measured = crate::sketch::measured_value(&sketch, &sketch.constraints[2].kind);
        assert_near(measured.unwrap(), 5.0, 1e-3);
    }

    /// Horizontal and vertical distances between two points, and from the
    /// origin when there is no second point.
    #[test]
    fn distance_x_and_y_between_points_and_from_the_origin() {
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
        converged(&mut sketch);
        assert_near((pos(&sketch, p2).x - pos(&sketch, p1).x).abs(), 7.0, 1e-3);
        assert_near((pos(&sketch, p2).y - pos(&sketch, p1).y).abs(), 3.0, 1e-3);
        assert!(sketch.is_fully_constrained);

        let p = add_point(&mut sketch, 2.0, 5.0);
        sketch.add_constraint(ConstraintKind::DistanceX {
            a: p,
            b: None,
            value: 6.0,
        });
        converged(&mut sketch);
        assert_near(pos(&sketch, p).x.abs(), 6.0, 1e-3);
        assert_near(pos(&sketch, p).y, 5.0, 1e-4);
    }

    #[test]
    fn diagnose_flags_conflicting_lengths() {
        let mut sketch = Sketch::new("diag_conflict");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        let len5 = sketch.length(line, 5.0);
        let len8 = sketch.length(line, 8.0);
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
        let (_, _, base) = fixed_line(&mut sketch, (0.0, 0.0), (10.0, 0.0));
        let c = add_point(&mut sketch, 0.0, 2.0);
        let d = add_point(&mut sketch, 8.0, 2.0);
        let other = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 2.0);
        let par1 = sketch.parallel(base, other);
        let par2 = sketch.parallel(base, other);
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
        let b = add_point(&mut sketch, 10.0, 2.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.is_empty() && diag.redundant.is_empty());
        assert_eq!(diag.dof, 1, "the free endpoint's x remains");
        // The solve levels the sloped line, and the free x leaves the
        // sketch short of fully constrained.
        converged(&mut sketch);
        assert_near(pos(&sketch, b).y, pos(&sketch, a).y, 1e-4);
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn symmetric_mirrors_a_point_about_a_line() {
        // (line's far end, the held point, the free one's start, its mirror)
        for (end, held, free, mirror) in [
            ((10.0, 0.0), (3.0, 4.0), (5.0, -2.0), (3.0, -4.0)),
            ((10.0, 10.0), (6.0, 2.0), (1.0, 5.0), (2.0, 6.0)),
        ] {
            let mut sketch = Sketch::new("symmetric");
            let (_, _, line) = fixed_line(&mut sketch, (0.0, 0.0), end);
            let p1 = add_point(&mut sketch, held.0, held.1);
            let p2 = add_point(&mut sketch, free.0, free.1);
            fix(&mut sketch, p1, held.0, held.1);
            sketch.add_constraint(ConstraintKind::Symmetric {
                point1: p1,
                point2: p2,
                line,
            });
            converged(&mut sketch);
            let at = pos(&sketch, p2);
            assert!(
                (at - glam::DVec2::from(mirror)).length() < 1e-3,
                "{held:?} about the line to {end:?}: {at:?}"
            );
        }
    }

    #[test]
    fn radius_and_diameter_resize_a_circle() {
        type Resize = fn(Uuid) -> ConstraintKind;
        let cases: [(Resize, f64); 2] = [
            (
                |circle| ConstraintKind::Radius {
                    circle,
                    radius: 5.0,
                },
                5.0,
            ),
            (
                |circle| ConstraintKind::Diameter {
                    circle,
                    diameter: 9.0,
                },
                4.5,
            ),
        ];
        for (kind, radius) in cases {
            let mut sketch = Sketch::new("radius");
            let center = add_point(&mut sketch, 1.0, 1.0);
            let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
            sketch.add_constraint(kind(circle));
            converged(&mut sketch);
            assert_near(circle_radius(&sketch, circle), radius, 1e-4);
        }
    }

    #[test]
    fn a_line_has_four_degrees_of_freedom_until_its_ends_are_fixed() {
        let mut sketch = Sketch::new("dof_line");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        assert_eq!(dof_estimate(&sketch), 4);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        assert_eq!(dof_estimate(&sketch), 0);
    }

    #[test]
    fn diagnose_early_outs_above_constraint_limit() {
        let mut sketch = Sketch::new("diag_limit");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
        for _ in 0..61 {
            sketch.radius(circle, 5.0);
        }
        let diag = diagnose(&sketch);
        assert!(!diag.analyzed, "too many constraints to analyze");
        assert!(diag.conflicting.is_empty() && diag.redundant.is_empty());
    }

    /// Every number of the geometry, as bits.
    fn geometry_bits(sketch: &Sketch) -> Vec<u64> {
        let mut out = Vec::new();
        for g in &sketch.geometry {
            match g {
                GeometryElement::Point(p) => out.extend([p.position.x, p.position.y]),
                GeometryElement::Circle(c) => out.push(c.radius),
                GeometryElement::Arc(a) => out.push(a.radius),
                GeometryElement::Ellipse(e) => out.extend([e.major.x, e.major.y, e.ratio]),
                GeometryElement::Conic(c) => out.extend([c.axis.x, c.axis.y, c.minor]),
                GeometryElement::Line(_) | GeometryElement::BSpline(_) => {}
            }
        }
        out.into_iter().map(f64::to_bits).collect()
    }

    /// The solver's answer is stored as it is, so a sketch it has solved
    /// is solved: solving it again moves nothing, to the last bit.
    #[test]
    fn solving_a_solved_sketch_again_moves_nothing() {
        let mut sketch = Sketch::new("solved");
        // A slanted quadrilateral off the origin, with awkward dimensions.
        let a = add_point(&mut sketch, 12.3, -4.1);
        let b = add_point(&mut sketch, 112.0, -3.0);
        let c = add_point(&mut sketch, 110.0, 37.0);
        let d = add_point(&mut sketch, 15.0, 41.0);
        let bottom = add_line(&mut sketch, a, b);
        let right = add_line(&mut sketch, b, c);
        let top = add_line(&mut sketch, c, d);
        add_line(&mut sketch, d, a);
        fix(&mut sketch, a, 12.345678, -4.000001);
        sketch.length(bottom, 100.000003);
        sketch.add_constraint(ConstraintKind::AngleToAxis {
            line: bottom,
            axis: AxisDirection::Horizontal,
            angle_rad: 0.1,
        });
        sketch.add_constraint(ConstraintKind::Perpendicular {
            line1: bottom,
            line2: right,
        });
        sketch.parallel(bottom, top);
        sketch.add_constraint(ConstraintKind::Distance {
            point1: b,
            point2: c,
            distance: 40.000007,
        });
        // A circle touching the top, an arc on a corner, an ellipse.
        let centre = add_point(&mut sketch, 60.0, 20.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, 9.0)));
        sketch.radius(circle, 7.777777);
        sketch.tangent(top, circle);
        let arc_centre = add_point(&mut sketch, 30.0, 10.0);
        let start = add_point(&mut sketch, 35.0, 10.0);
        let end = add_point(&mut sketch, 30.0, 15.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(arc_centre, start, end, 5.0)));
        sketch.radius(arc, 3.333333);
        let middle = add_point(&mut sketch, 80.0, 10.0);
        ellipse(&mut sketch, middle, (6.0, 1.0), 0.6);

        converged(&mut sketch);
        let once = geometry_bits(&sketch);
        converged(&mut sketch);
        assert_eq!(
            geometry_bits(&sketch),
            once,
            "a second solve moved the sketch"
        );
        let length = (pos(&sketch, b) - pos(&sketch, a)).length();
        assert_near(length, 100.000003, 1e-9);
    }

    #[test]
    fn a_point_made_coincident_with_the_origin_lands_on_it() {
        let mut sketch = Sketch::new("t");
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.coincident(p, ORIGIN_ID);
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
        let line = add_line(&mut sketch, a, b);
        sketch.coincident(a, ORIGIN_ID);
        sketch.add_constraint(ConstraintKind::PointOnLine {
            point: b,
            line: X_AXIS_ID,
        });
        sketch.length(line, 10.0);
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
        let line = add_line(&mut sketch, a, b);
        sketch.length(line, 6.0);
        let before = dof_estimate(&sketch);
        solve(&mut sketch);
        assert_eq!(before, 3, "four point variables, one length residual");
        assert_eq!(dof_estimate(&sketch), 3);
    }

    #[test]
    fn a_pinned_line_frees_its_far_end_until_length_and_direction_hold_it() {
        let mut sketch = Sketch::new("t");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        pin(&mut sketch, a);
        let free = free_points(&sketch);
        assert!(!free.contains(&a), "the pinned point stays put");
        assert!(free.contains(&b), "the far end swings");

        sketch.length(line, 10.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert!(free_points(&sketch).is_empty(), "nothing moves any more");
    }

    #[test]
    fn an_arc_length_sweeps_the_end_round() {
        let mut sketch = Sketch::new("t");
        let c = add_point(&mut sketch, 0.0, 0.0);
        let s = add_point(&mut sketch, 5.0, 0.0);
        let e = add_point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        pin(&mut sketch, c);
        pin(&mut sketch, s);
        let three_eighths = 5.0 * std::f64::consts::PI * 0.75;
        sketch.add_constraint(ConstraintKind::ArcLength {
            arc,
            length: three_eighths,
        });
        converged(&mut sketch);
        near(measure::arc_length(&sketch, arc).unwrap(), three_eighths);
        let end = sketch.point_position(e).unwrap();
        near(end.x, -5.0 / 2f64.sqrt());
        near(end.y, 5.0 / 2f64.sqrt());
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
        sketch.radius(arc, 5.0);
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
        pin(&mut sketch, arm1);
        pin(&mut sketch, corner);
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

    fn with_dimension_value_kind(kind: &ConstraintKind, v: f64) -> ConstraintKind {
        crate::sketch::with_dimension_value(kind, v)
    }

    /// The distance from `p` to the nearest of `samples`.
    fn off(samples: &[glam::DVec2], p: glam::DVec2) -> f64 {
        samples
            .iter()
            .map(|q| (*q - p).length())
            .fold(f64::INFINITY, f64::min)
    }

    fn spline_samples(sketch: &Sketch, spline: Uuid) -> Vec<glam::DVec2> {
        let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
            panic!("not a spline");
        };
        let basis = crate::spline::basis_of(b).unwrap();
        let control: Vec<[f64; 2]> = b
            .control_points
            .iter()
            .map(|id| {
                let p = sketch.point_position(*id).unwrap();
                [p.x, p.y]
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
        let spline = wavy_spline(&mut sketch, 4);
        let p = add_point(&mut sketch, 7.0, 9.0);
        sketch.add_constraint(ConstraintKind::PointOnCurve {
            point: p,
            curve: spline,
        });
        converged(&mut sketch);
        let at = sketch.point_position(p).unwrap().to_glam();
        assert!(off(&spline_samples(&sketch, spline), at) < 0.02, "{at:?}");

        // y² = 4·2·x about the origin, opening along +x.
        let mut sketch = Sketch::new("t");
        let vertex = add_point(&mut sketch, 0.0, 0.0);
        let s = add_point(&mut sketch, 2.0, -4.0);
        let e = add_point(&mut sketch, 8.0, 8.0);
        pin(&mut sketch, vertex);
        let conic = sketch.add_geometry(GeometryElement::Conic(crate::sketch::Conic::new(
            crate::sketch::ConicKind::Parabola,
            vertex,
            Vec2D::new(2.0, 0.0),
            0.0,
            s,
            e,
        )));
        let p = add_point(&mut sketch, 5.0, 1.0);
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
        let c = add_point(&mut sketch, 0.0, 0.0);
        pin(&mut sketch, c);
        let ellipse = ellipse(&mut sketch, c, (6.0, 0.0), 0.5);
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
        let spline = wavy_spline(&mut sketch, 3);
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
        let c = add_point(&mut sketch, 0.0, 0.0);
        pin(&mut sketch, c);
        let ellipse = ellipse(&mut sketch, c, (6.0, 0.0), 0.5);
        let (a, b) = (
            add_point(&mut sketch, -10.0, 5.0),
            add_point(&mut sketch, 10.0, 4.0),
        );
        let l = add_line(&mut sketch, a, b);
        pin(&mut sketch, a);
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
        let ring: Vec<glam::DVec2> = (0..4000)
            .map(|i| {
                let t = i as f64 / 4000.0 * std::f64::consts::TAU;
                glam::DVec2::new(6.0 * t.cos(), 3.0 * t.sin())
            })
            .collect();
        let dir = (pb - pa).normalize();
        let gap = ring
            .iter()
            .map(|q| (*q - pa).perp_dot(dir).abs())
            .fold(f64::INFINITY, f64::min);
        assert!(gap < 0.01, "touches: {gap}");

        let mut sketch = Sketch::new("t");
        let spline = wavy_spline(&mut sketch, 4);
        let (a, b) = (
            add_point(&mut sketch, 7.0, -6.0),
            add_point(&mut sketch, 8.0, 8.0),
        );
        let l = add_line(&mut sketch, a, b);
        pin(&mut sketch, a);
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
            let a = add_point(&mut sketch, 0.0, 0.0);
            let b = add_point(&mut sketch, 1.0, 0.5);
            pin(&mut sketch, a);
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
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 20.0, 0.0);
        let e1 = ellipse(&mut sketch, c1, (6.0, 0.0), 0.5);
        let e2 = ellipse(&mut sketch, c2, (0.0, 4.0), 0.25);
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
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 2.0);
        let c = add_point(&mut sketch, 3.0, 8.0);
        pin(&mut sketch, a);
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
        sketch.parallel(bottom, top);
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
        sketch.gap(bottom, top, 5.0);
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
        pin(&mut sketch, c1);
        sketch.radius(a, 2.0);
        sketch.radius(b, 3.0);
        sketch.gap(a, b, 1.5);
        converged(&mut sketch);
        near(measure::gap(&sketch, a, b).unwrap().distance, 1.5);

        let mut sketch = Sketch::new("nested");
        let (c1, big) = circle(&mut sketch, 0.0, 0.0, 10.0);
        let (c2, small) = circle(&mut sketch, 1.0, 1.0, 2.0);
        pin(&mut sketch, c1);
        sketch.radius(big, 10.0);
        sketch.radius(small, 2.0);
        sketch.gap(small, big, 1.0);
        converged(&mut sketch);
        near(measure::gap(&sketch, small, big).unwrap().distance, 1.0);
        near(sketch.point_position(c2).unwrap().to_glam().length(), 7.0);
    }

    #[test]
    fn a_gap_keeps_a_circle_off_a_line_and_points_off_both() {
        let mut sketch = Sketch::new("t");
        let s = add_point(&mut sketch, -10.0, 0.0);
        let e = add_point(&mut sketch, 10.0, 0.0);
        let ground = add_line(&mut sketch, s, e);
        pin(&mut sketch, s);
        pin(&mut sketch, e);
        let (_, wheel) = circle(&mut sketch, 1.0, 6.0, 2.0);
        sketch.radius(wheel, 2.0);
        sketch.gap(ground, wheel, 0.5);
        let p = add_point(&mut sketch, 3.0, -4.0);
        sketch.gap(p, ground, 2.0);
        let q = add_point(&mut sketch, 1.0, 12.0);
        sketch.gap(wheel, q, 3.0);
        converged(&mut sketch);
        near(measure::gap(&sketch, ground, wheel).unwrap().distance, 0.5);
        near(sketch.point_position(p).unwrap().y, -2.0);
        near(measure::gap(&sketch, q, wheel).unwrap().distance, 3.0);
    }

    #[test]
    fn an_angle_at_a_point_turns_a_line_against_an_arc() {
        let mut sketch = Sketch::new("t");
        let c = add_point(&mut sketch, 0.0, 0.0);
        let s = add_point(&mut sketch, 5.0, 0.0);
        let e = add_point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        sketch.add_constraint(ConstraintKind::Block { element: arc });
        let far = add_point(&mut sketch, 9.0, 1.0);
        let spoke = add_line(&mut sketch, s, far);
        sketch.length(spoke, 4.0);
        let angle = 60f64.to_radians();
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
        near(d.y.atan2(d.x), 30f64.to_radians());
    }

    #[test]
    fn a_refraction_bends_the_ray_leaving_the_interface() {
        let mut sketch = Sketch::new("t");
        let l = add_point(&mut sketch, -10.0, 0.0);
        let r = add_point(&mut sketch, 10.0, 0.0);
        let interface = add_line(&mut sketch, l, r);
        sketch.add_constraint(ConstraintKind::Block { element: interface });
        let source = add_point(&mut sketch, -4.0, 4.0);
        let hit = add_point(&mut sketch, 0.0, 0.0);
        let exit = add_point(&mut sketch, 3.0, -5.0);
        let ray1 = add_line(&mut sketch, source, hit);
        let ray2 = add_line(&mut sketch, hit, exit);
        pin(&mut sketch, source);
        pin(&mut sketch, hit);
        sketch.length(ray2, 5.0);
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
        near(out.x / out.length(), 45f64.to_radians().sin() / 1.5);
        assert!(out.y < 0.0, "refracted, not reflected: {out:?}");
    }

    #[test]
    fn a_new_constraint_supersedes_the_older_one_it_repeats() {
        let mut sketch = Sketch::new("t");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let level = add_line(&mut sketch, a, b);
        let horizontal = sketch.add_constraint(ConstraintKind::Horizontal { element: level });
        let length = sketch.length(level, 10.0);
        let parallel = sketch.parallel(level, crate::sketch::X_AXIS_ID);
        assert_eq!(superseded(&sketch, &[parallel]), vec![horizontal]);
        assert!(superseded(&sketch, &[length]).is_empty(), "nothing to take");

        // Redundancy the sketch had already is none of the new one's doing.
        let mut sketch = Sketch::new("t");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let level = add_line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Horizontal { element: level });
        sketch.add_constraint(ConstraintKind::Horizontal { element: level });
        let length = sketch.length(level, 10.0);
        assert!(superseded(&sketch, &[length]).is_empty());
    }
}
