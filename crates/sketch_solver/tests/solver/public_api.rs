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
fn a_circular_point_keeps_its_direction_when_the_centre_crosses_its_old_position() {
    // Seeds stay on the old side. The solved point must follow the live centre
    // and radius through both crossings, rather than choose an H/V branch.
    for (angle, direction) in [
        (0., (1., 0.)),
        (90., (0., 1.)),
        (180., (-1., 0.)),
        (270., (0., -1.)),
        (360., (1., 0.)),
        (450., (0., 1.)),
        (720., (1., 0.)),
        (-90., (0., -1.)),
        (-450., (0., -1.)),
    ] {
        for epsilon in [-1e-8_f64, 0., 1e-8] {
            for (x, y, r) in [(40., 50., 15.), (-30., -20., 7.)] {
                let angle: f64 = angle;
                let (sin, cos) = epsilon.sin_cos();
                let expected = (
                    x + r * (direction.0 * cos - direction.1 * sin),
                    y + r * (direction.1 * cos + direction.0 * sin),
                );
                let problem = Problem {
                    geometry: vec![
                        Geometry::Point(Point {
                            id: PointId(1),
                            position: Vector { x, y },
                        }),
                        Geometry::Point(Point {
                            id: PointId(2),
                            position: Vector {
                                x: 10. * direction.0,
                                y: 10. * direction.1,
                            },
                        }),
                        Geometry::Circle(Circle {
                            id: CurveId(3),
                            center: PointId(1).into(),
                            radius: Length(r),
                        }),
                    ],
                    constraints: vec![Constraint {
                        id: ConstraintId(1),
                        kind: Relation::CircularPoint {
                            point: PointId(2).into(),
                            circle: CurveId(3).into(),
                            angle: Radians(angle.to_radians() + epsilon),
                        },
                    }],
                    external: vec![PointId(1).into(), CurveId(3).into()],
                    ..Problem::default()
                };
                let answer = solve(&problem).unwrap();
                assert!(
                    matches!(answer.outcome, SolveOutcome::Converged { .. }),
                    "{answer:?}"
                );
                let k = compile(&problem)
                    .unwrap()
                    .point_variable(PointId(2).into())
                    .unwrap()
                    .0;
                assert!(
                    (answer.values[k] - expected.0).hypot(answer.values[k + 1] - expected.1) < 1e-7
                );
                assert_eq!(sketch_solver::freedom::estimate(&problem).degrees, 0);
            }
        }
    }
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
    let mut spline = BSpline {
        id: CurveId(4),
        degree: 1,
        control_points: vec![PointId(1).into(), PointId(2).into()],
        fit_points: vec![PointId(1).into(), PointId(2).into()],
        fit_params: vec![0.0, 1.0],
        knots: Vec::new(),
        weights: Vec::new(),
        periodic: false,
    };
    input = line_problem();
    input.geometry.push(Geometry::BSpline(spline.clone()));
    assert!(compile(&input).is_ok());
    for params in [
        vec![f64::NAN, 1.0],
        vec![0.0, 0.0],
        vec![-1.0, 1.0],
        vec![0.0, 2.0],
        vec![0.0],
        vec![0.0, 0.5, 1.0],
    ] {
        spline.fit_params = params;
        *input.geometry.last_mut().unwrap() = Geometry::BSpline(spline.clone());
        assert!(matches!(compile(&input), Err(InputError::InvalidSpline(_))));
    }
    spline.fit_params.clear();
    *input.geometry.last_mut().unwrap() = Geometry::BSpline(spline);
    assert!(
        compile(&input).is_ok(),
        "legacy topology-only fit points remain valid"
    );
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
