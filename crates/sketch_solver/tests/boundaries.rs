use sketch_solver::compile::compile;
use sketch_solver::input::InputError;
use sketch_solver::problem::*;
use sketch_solver::solve::{SolveOutcome, solve};
use std::f64::consts::{PI, TAU};

fn point(id: u128, x: f64, y: f64) -> Geometry {
    Geometry::Point(Point {
        id: PointId(id),
        position: Vector { x, y },
    })
}
fn constraint(id: u128, kind: Relation) -> Constraint {
    Constraint {
        id: ConstraintId(id),
        kind,
    }
}

fn directional_line() -> Problem {
    Problem {
        geometry: vec![
            point(1, 0.0, 0.0),
            point(2, 8.0, 3.0),
            Geometry::Line(Line {
                id: CurveId(3),
                start: PointId(1).into(),
                end: PointId(2).into(),
            }),
        ],
        constraints: vec![
            constraint(
                1,
                Relation::FixedPoint {
                    point: PointId(1).into(),
                    position: Vector { x: 0.0, y: 0.0 },
                },
            ),
            constraint(
                2,
                Relation::Length {
                    line: CurveId(3).into(),
                    length: Length(10.0),
                },
            ),
            constraint(
                3,
                Relation::AngleToAxis {
                    line: CurveId(3).into(),
                    axis: AxisDirection::Horizontal,
                    angle_rad: Radians(0.0),
                },
            ),
        ],
        ..Problem::default()
    }
}

#[test]
fn directional_angles_are_periodic_across_boundaries_and_repeated_transitions() {
    let mut input = directional_line();
    let mut attempts = 0;
    for _ in 0..3 {
        for degrees in [
            -720.0, -450.0, -360.0, -270.0, -180.0, 180.0, 270.0, 360.0, 450.0, 720.0,
        ] {
            for epsilon in [-1e-7, 0.0, 1e-7] {
                let angle = degrees * PI / 180.0 + epsilon;
                input.constraints[2].kind = Relation::AngleToAxis {
                    line: CurveId(3).into(),
                    axis: AxisDirection::Horizontal,
                    angle_rad: Radians(angle),
                };
                let answer = solve(&input).unwrap();
                assert!(
                    matches!(answer.outcome, SolveOutcome::Converged { .. }),
                    "angle={angle}: {:?}",
                    answer.outcome
                );
                let at = compile(&input)
                    .unwrap()
                    .point_variable(PointId(2).into())
                    .unwrap()
                    .0;
                let (x, y) = (answer.values[at], answer.values[at + 1]);
                assert!(
                    (x / 10.0 - angle.cos()).abs() < 1e-7 && (y / 10.0 - angle.sin()).abs() < 1e-7,
                    "angle={angle}, ({x},{y})"
                );
                let Geometry::Point(p) = &mut input.geometry[1] else {
                    unreachable!()
                };
                p.position = Vector { x, y };
                attempts += 1;
            }
        }
    }
    assert_eq!(attempts, 90);
}

#[test]
fn arc_sweep_domain_distinguishes_full_turn_and_rejects_wrapped_targets() {
    let mut input = Problem {
        geometry: vec![
            point(1, 0.0, 0.0),
            point(2, 2.0, 0.0),
            Geometry::Arc(Arc {
                id: CurveId(3),
                center: PointId(1).into(),
                start: PointId(2).into(),
                end: PointId(2).into(),
                radius: Length(2.0),
            }),
        ],
        held_points: vec![PointId(1).into(), PointId(2).into()],
        constraints: vec![constraint(
            1,
            Relation::ArcAngle {
                arc: CurveId(3).into(),
                angle_rad: Radians(TAU),
            },
        )],
        ..Problem::default()
    };
    assert!(matches!(
        solve(&input).unwrap().outcome,
        SolveOutcome::Converged { .. }
    ));
    for degrees in [-720.0, -450.0, -360.0, -270.0, -180.0, 0.0, 450.0, 720.0] {
        input.constraints[0].kind = Relation::ArcAngle {
            arc: CurveId(3).into(),
            angle_rad: Radians(degrees * PI / 180.0),
        };
        assert!(
            matches!(solve(&input), Err(InputError::InvalidRelation { .. })),
            "{degrees}"
        );
    }
    for angle in [PI, PI + 1e-7, TAU - 1e-7, TAU] {
        input.constraints[0].kind = Relation::ArcAngle {
            arc: CurveId(3).into(),
            angle_rad: Radians(angle),
        };
        assert!(compile(&input).is_ok());
    }
    input.constraints[0].kind = Relation::ArcAngle {
        arc: CurveId(3).into(),
        angle_rad: Radians(TAU + 1e-7),
    };
    assert!(compile(&input).is_err());
}

#[test]
fn zero_lengths_coincident_centers_and_fully_held_inputs_stay_finite() {
    let mut input = directional_line();
    let Geometry::Point(p) = &mut input.geometry[1] else {
        unreachable!()
    };
    p.position = Vector { x: 0.0, y: 0.0 };
    input.constraints[1].kind = Relation::Length {
        line: CurveId(3).into(),
        length: Length(0.0),
    };
    let answer = solve(&input).unwrap();
    assert!(matches!(answer.outcome, SolveOutcome::Converged { .. }));
    assert!(answer.values.iter().all(|v| v.is_finite()));
    input = Problem {
        geometry: vec![
            point(1, 0.0, 0.0),
            point(2, 0.0, 0.0),
            Geometry::Circle(Circle {
                id: CurveId(3),
                center: PointId(1).into(),
                radius: Length(2.0),
            }),
            Geometry::Circle(Circle {
                id: CurveId(4),
                center: PointId(2).into(),
                radius: Length(2.0),
            }),
        ],
        constraints: vec![constraint(
            1,
            Relation::Tangent {
                line_or_circle1: CurveId(3).into(),
                item2: CurveId(4).into(),
            },
        )],
        held_points: vec![PointId(1).into(), PointId(2).into()],
        ..Problem::default()
    };
    let answer = solve(&input).unwrap();
    assert!(matches!(answer.outcome, SolveOutcome::Converged { .. }));
    assert!(answer.values.iter().all(|v| v.is_finite()));
    input.constraints.clear();
    input.external = input.geometry.iter().map(Geometry::id).collect();
    assert_eq!(sketch_solver::freedom::analyze(&input).unwrap().degrees, 0);
    assert!(matches!(
        solve(&input).unwrap().outcome,
        SolveOutcome::NothingToSolve
    ));
}

#[test]
fn ellipse_equal_axes_on_either_side_preserve_double_precision_sizes() {
    for epsilon in [-1e-6, 0.0, 1e-6] {
        let input = Problem {
            geometry: vec![
                point(1, 0.0, 0.0),
                Geometry::Ellipse(Ellipse {
                    id: CurveId(2),
                    center: PointId(1).into(),
                    major: Vector { x: 0.9, y: 0.0 },
                    minor: Length(1.1),
                    arc: None,
                }),
            ],
            held_points: vec![PointId(1).into()],
            constraints: vec![
                constraint(
                    1,
                    Relation::EllipseRadius {
                        ellipse: CurveId(2).into(),
                        major: true,
                        radius: Length(1.0),
                    },
                ),
                constraint(
                    2,
                    Relation::EllipseRadius {
                        ellipse: CurveId(2).into(),
                        major: false,
                        radius: Length(1.0 + epsilon),
                    },
                ),
            ],
            ..Problem::default()
        };
        let answer = solve(&input).unwrap();
        assert!(matches!(answer.outcome, SolveOutcome::Converged { .. }));
        let k = compile(&input)
            .unwrap()
            .shape_variable(CurveId(2).into())
            .unwrap()
            .0;
        assert!((answer.values[k].hypot(answer.values[k + 1]) - 1.0).abs() < 1e-8);
        assert!((answer.values[k + 2] - (1.0 + epsilon)).abs() < 1e-8);
    }
}

#[test]
fn diagnosis_limits_and_multiple_conflicts_are_explicit() {
    for count in [60, 61] {
        let input = Problem {
            geometry: vec![point(1, 0.0, 0.0)],
            constraints: (0..count)
                .map(|id| {
                    constraint(
                        id,
                        Relation::FixedPoint {
                            point: PointId(1).into(),
                            position: Vector { x: 0.0, y: 0.0 },
                        },
                    )
                })
                .collect(),
            ..Problem::default()
        };
        let diagnosis = sketch_solver::diagnosis::analyze(&input).unwrap();
        assert_eq!(diagnosis.analyzed, count == 60);
        assert_eq!(diagnosis.dof, 0);
        assert_eq!(diagnosis.redundant.len(), if count == 60 { 60 } else { 0 });
    }
    let input = Problem {
        geometry: vec![point(1, 0.0, 0.0), point(2, 0.0, 0.0)],
        constraints: vec![
            constraint(
                1,
                Relation::FixedPoint {
                    point: PointId(1).into(),
                    position: Vector { x: 0.0, y: 0.0 },
                },
            ),
            constraint(
                2,
                Relation::FixedPoint {
                    point: PointId(1).into(),
                    position: Vector { x: 1.0, y: 0.0 },
                },
            ),
            constraint(
                3,
                Relation::FixedPoint {
                    point: PointId(2).into(),
                    position: Vector { x: 0.0, y: 0.0 },
                },
            ),
            constraint(
                4,
                Relation::FixedPoint {
                    point: PointId(2).into(),
                    position: Vector { x: 1.0, y: 0.0 },
                },
            ),
        ],
        ..Problem::default()
    };
    assert!(matches!(
        solve(&input).unwrap().outcome,
        SolveOutcome::NotConverged { .. }
    ));
    let diagnosis = sketch_solver::diagnosis::analyze(&input).unwrap();
    assert!(diagnosis.analyzed && diagnosis.conflicting.is_empty());
}

#[test]
fn gap_branches_are_chosen_from_geometry_on_both_sides() {
    for (inside, nested) in [(false, false), (true, false), (false, true), (true, true)] {
        let (r1, r2) = (5.0, 2.0);
        let target = 1.0;
        let separation = if nested {
            r1 - r2 - target
        } else {
            r1 + r2 + target
        };
        let from_center = if inside { r1 - target } else { r1 + target };
        let input = Problem {
            geometry: vec![
                point(1, 0.0, 0.0),
                point(2, separation * 1.01, 0.0),
                point(5, from_center * 1.01, 0.0),
                Geometry::Circle(Circle {
                    id: CurveId(3),
                    center: PointId(1).into(),
                    radius: Length(r1),
                }),
                Geometry::Circle(Circle {
                    id: CurveId(4),
                    center: PointId(2).into(),
                    radius: Length(r2),
                }),
            ],
            external: vec![PointId(1).into(), CurveId(3).into(), CurveId(4).into()],
            constraints: vec![
                constraint(
                    1,
                    Relation::Gap {
                        item1: PointId(5).into(),
                        item2: CurveId(3).into(),
                        distance: Length(target),
                    },
                ),
                constraint(
                    2,
                    Relation::Gap {
                        item1: CurveId(3).into(),
                        item2: CurveId(4).into(),
                        distance: Length(target),
                    },
                ),
            ],
            ..Problem::default()
        };
        let system = compile(&input).unwrap();
        let answer = solve(&input).unwrap();
        assert!(matches!(answer.outcome, SolveOutcome::Converged { .. }));
        let b = system.point_variable(PointId(2).into()).unwrap().0;
        let p = system.point_variable(PointId(5).into()).unwrap().0;
        assert!((answer.values[b].hypot(answer.values[b + 1]) - separation).abs() < 1e-8);
        assert!((answer.values[p].hypot(answer.values[p + 1]) - from_center).abs() < 1e-8);
    }
}
