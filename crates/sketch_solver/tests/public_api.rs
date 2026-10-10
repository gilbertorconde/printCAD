use sketch_solver::compile::compile;
use sketch_solver::input::InputError;
use sketch_solver::problem::*;
use sketch_solver::solve::{SolveOutcome, solve};

fn line_problem() -> Problem {
    let a = PointId(1);
    let b = PointId(2);
    let line = CurveId(3);
    Problem {
        geometry: vec![
            Geometry::Point(Point {
                id: a,
                position: Vector { x: 0.0, y: 0.0 },
            }),
            Geometry::Point(Point {
                id: b,
                position: Vector { x: 5.0, y: 5.0 },
            }),
            Geometry::Line(Line {
                id: line,
                start: a.into(),
                end: b.into(),
            }),
        ],
        constraints: vec![
            Constraint {
                id: ConstraintId(1),
                kind: Relation::FixedPoint {
                    point: a.into(),
                    position: Vector { x: 0.0, y: 0.0 },
                },
            },
            Constraint {
                id: ConstraintId(2),
                kind: Relation::Horizontal {
                    element: line.into(),
                },
            },
            Constraint {
                id: ConstraintId(3),
                kind: Relation::Length {
                    line: line.into(),
                    length: Length(10.0),
                },
            },
        ],
        ..Problem::default()
    }
}

#[test]
fn consumer_solves_a_pinned_horizontal_line_with_public_types() {
    let input = line_problem();
    let compiled = compile(&input).unwrap();
    let answer = solve(&input).unwrap();
    assert!(matches!(answer.outcome, SolveOutcome::Converged { .. }));
    let a = compiled.point_variable(PointId(1).into()).unwrap().0;
    let b = compiled.point_variable(PointId(2).into()).unwrap().0;
    assert!(answer.values[a].abs() < 1e-8 && answer.values[a + 1].abs() < 1e-8);
    assert!(answer.values[b + 1].abs() < 1e-8);
    let distance =
        (answer.values[b] - answer.values[a]).hypot(answer.values[b + 1] - answer.values[a + 1]);
    assert!((distance - 10.0).abs() < 1e-8);
    assert_eq!(sketch_solver::freedom::estimate(&input).degrees, 0);
    let diagnosis = sketch_solver::diagnosis::diagnose(&input);
    assert!(
        diagnosis.analyzed && diagnosis.redundant.is_empty() && diagnosis.conflicting.is_empty()
    );
}

#[test]
fn malformed_input_is_distinct_from_numerical_failure() {
    let mut input = line_problem();
    input.geometry.push(input.geometry[0].clone());
    assert!(matches!(
        solve(&input),
        Err(InputError::DuplicateGeometry(_))
    ));
    input.geometry.pop();
    input.constraints[0].kind = Relation::FixedPoint {
        point: PointId(90).into(),
        position: Vector { x: 0.0, y: 0.0 },
    };
    assert!(matches!(
        solve(&input),
        Err(InputError::MissingReference(_))
    ));
    input = line_problem();
    input.constraints[2].kind = Relation::Radius {
        circle: CurveId(3).into(),
        radius: Length(10.0),
    };
    assert!(matches!(
        solve(&input),
        Err(InputError::InvalidRelation { .. })
    ));
    input = line_problem();
    input.settings.tolerance = f64::NAN;
    assert!(matches!(solve(&input), Err(InputError::InvalidSettings(_))));
    input = line_problem();
    input.constraints.push(Constraint {
        id: ConstraintId(4),
        kind: Relation::Length {
            line: CurveId(3).into(),
            length: Length(12.0),
        },
    });
    assert!(matches!(
        solve(&input).unwrap().outcome,
        SolveOutcome::NotConverged { .. }
    ));
}

#[test]
fn held_and_external_variables_are_immovable() {
    let mut input = line_problem();
    input.constraints.clear();
    input.external.push(CurveId(3).into());
    input.external.extend([
        ItemReference::from(PointId(1)),
        ItemReference::from(PointId(2)),
    ]);
    let compiled = compile(&input).unwrap();
    assert!(compiled.free.is_empty());
    assert_eq!(sketch_solver::freedom::estimate(&input).degrees, 0);
    input.external.clear();
    input.held_points.extend([
        PointReference::from(PointId(1)),
        PointReference::from(PointId(2)),
    ]);
    assert!(compile(&input).unwrap().free.is_empty());
}

#[test]
fn explicit_settings_control_iteration_and_rank() {
    let mut input = line_problem();
    input.settings.max_iterations = 1;
    let answer = solve(&input).unwrap();
    assert_eq!(answer.iterations, 1);
    assert!(matches!(answer.outcome, SolveOutcome::NotConverged { .. }));
    input.settings.max_iterations = 100;
    input.settings.rank_tolerance = 2.0;
    assert_eq!(sketch_solver::freedom::estimate(&input).rank, 0);
}
