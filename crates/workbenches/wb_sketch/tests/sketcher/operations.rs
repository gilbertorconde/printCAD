use std::collections::HashMap;
use uuid::Uuid;
use wb_sketch::operations::{PointJoinError, SolveOutcome, join_points, solve};
use wb_sketch::sketch::{ConstraintKind, GeometryElement, Line, ORIGIN_ID, Point, Sketch, Vec2D};

fn point(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
}

#[test]
fn invalid_alias_maps_return_errors_without_partial_mutation() {
    let mut sketch = Sketch::new("aliases");
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 1.0, 0.0);
    let c = point(&mut sketch, 2.0, 0.0);
    let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
    let missing = Uuid::new_v4();
    let before = serde_json::to_value(&sketch).unwrap();
    for (aliases, expected) in [
        (
            vec![(a, b), (c, missing)],
            PointJoinError::MissingPoint(missing),
        ),
        (vec![(missing, b)], PointJoinError::MissingPoint(missing)),
        (vec![(line, b)], PointJoinError::MissingPoint(line)),
        (vec![(a, line)], PointJoinError::MissingPoint(line)),
        (vec![(ORIGIN_ID, b)], PointJoinError::ImmutableOrigin),
    ] {
        assert_eq!(
            join_points(&mut sketch, &aliases.into_iter().collect()),
            Err(expected)
        );
        assert_eq!(serde_json::to_value(&sketch).unwrap(), before);
    }
    for aliases in [vec![(a, a)], vec![(a, b), (b, c)], vec![(a, b), (b, a)]] {
        assert!(matches!(
            join_points(&mut sketch, &aliases.into_iter().collect()),
            Err(PointJoinError::NonCanonicalAlias { .. })
        ));
        assert_eq!(serde_json::to_value(&sketch).unwrap(), before);
    }
}

#[test]
fn explicit_aliases_follow_origin_and_keep_other_points_separate() {
    let mut sketch = Sketch::new("origin");
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 0.0, 0.0);
    let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
    sketch.set_construction(a, true);
    sketch.add_constraint(ConstraintKind::FixedPoint {
        point: a,
        position: Vec2D::new(0.0, 0.0),
    });
    join_points(&mut sketch, &HashMap::from([(a, ORIGIN_ID)])).unwrap();
    assert!(sketch.get_geometry(a).is_none());
    assert!(
        sketch.get_geometry(b).is_some(),
        "proximity does not join points"
    );
    assert!(
        matches!(sketch.get_geometry(line), Some(GeometryElement::Line(l)) if l.start == ORIGIN_ID && l.end == b)
    );
    assert!(!sketch.is_construction(ORIGIN_ID));
    assert!(!sketch.is_construction(a));
    assert!(matches!(
        sketch.constraints[0].kind,
        ConstraintKind::FixedPoint {
            point: ORIGIN_ID,
            ..
        }
    ));
    join_points(&mut sketch, &HashMap::new()).unwrap();
}

#[test]
fn checked_solve_rejects_invalid_input_before_storage_write_back() {
    let mut base = Sketch::new("invalid");
    let a = point(&mut base, 0.0, 0.0);
    let b = point(&mut base, 10.0, 0.0);
    base.add_geometry(GeometryElement::Line(Line::new(a, b)));
    for case in 0..6 {
        let mut sketch = base.clone();
        match case {
            0 => {
                sketch.geometry.push(sketch.geometry[0].clone());
            }
            1 => {
                sketch.add_constraint(ConstraintKind::Length {
                    line: Uuid::new_v4(),
                    length: 1.0,
                });
            }
            2 => {
                sketch.add_constraint(ConstraintKind::Length {
                    line: a,
                    length: 1.0,
                });
            }
            3 => {
                sketch.solver.tolerance = 0.0;
            }
            4 => {
                sketch.solver.tolerance = f64::INFINITY;
            }
            5 => {
                sketch.solver.max_iterations = 0;
            }
            _ => unreachable!(),
        }
        let before = serde_json::to_value(&sketch).unwrap();
        assert!(solve(&mut sketch).is_err(), "case {case}");
        assert_eq!(serde_json::to_value(&sketch).unwrap(), before);
    }
    let GeometryElement::Point(p) = &mut base.geometry[0] else {
        unreachable!()
    };
    p.position.x = f64::NAN;
    assert!(matches!(
        solve(&mut base),
        Err(sketch_solver::input::InputError::NonFiniteGeometry(_))
    ));
    assert!(matches!(&base.geometry[0], GeometryElement::Point(p) if p.position.x.is_nan()));
}

#[test]
fn checked_solve_stores_geometry_and_distinguishes_conflicts_from_bad_input() {
    let mut sketch = Sketch::new("length");
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 10.0, 0.0);
    let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
    assert_eq!(solve(&mut sketch).unwrap(), SolveOutcome::NothingToSolve);
    sketch.add_constraint(ConstraintKind::FixedPoint {
        point: a,
        position: Vec2D::new(0.0, 0.0),
    });
    sketch.add_constraint(ConstraintKind::Horizontal { element: line });
    sketch.add_constraint(ConstraintKind::Length { line, length: 12.0 });
    assert!(matches!(
        solve(&mut sketch).unwrap(),
        SolveOutcome::Converged { .. }
    ));
    let end = sketch.point_position(b).unwrap();
    assert!((end.x - 12.0).abs() <= 1e-6 && end.y.abs() <= 1e-6);
    let stored = serde_json::to_value(&sketch).unwrap();
    assert!(matches!(
        solve(&mut sketch).unwrap(),
        SolveOutcome::Converged { .. }
    ));
    assert_eq!(
        serde_json::to_value(&sketch).unwrap(),
        stored,
        "solving the stored result stays still"
    );
    sketch.add_constraint(ConstraintKind::FixedPoint {
        point: b,
        position: Vec2D::new(10.0, 0.0),
    });
    assert!(matches!(
        solve(&mut sketch).unwrap(),
        SolveOutcome::NotConverged { .. }
    ));
    assert!(sketch.unsolved);
}
