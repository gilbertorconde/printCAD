use sketch_solver::compile::compile;
use sketch_solver::input::InputError;
use sketch_solver::problem::*;
use sketch_solver::solve::{SolveOutcome, solve};

const A: PointId = PointId(1);
const B: PointId = PointId(2);
const C: PointId = PointId(3);
const AB: CurveId = CurveId(11);
const BC: CurveId = CurveId(12);
const CA: CurveId = CurveId(13);
const GROUP: ConstraintId = ConstraintId(100);

fn at(x: f64, y: f64) -> Vector {
    Vector { x, y }
}

fn point(id: PointId, position: Vector) -> Geometry {
    Geometry::Point(Point { id, position })
}

fn line(id: CurveId, start: PointId, end: PointId) -> Geometry {
    Geometry::Line(Line {
        id,
        start: start.into(),
        end: end.into(),
    })
}

/// A 3-4-5 triangle grouped where it stands, its corners then moved to
/// `corners` (the group keeps the shape it was made with).
fn triangle(corners: [Vector; 3]) -> Problem {
    let made = [at(0.0, 0.0), at(4.0, 0.0), at(0.0, 3.0)];
    Problem {
        geometry: vec![
            point(A, corners[0]),
            point(B, corners[1]),
            point(C, corners[2]),
            line(AB, A, B),
            line(BC, B, C),
            line(CA, C, A),
        ],
        constraints: vec![Constraint {
            id: GROUP,
            kind: Relation::Group {
                elements: vec![AB.into(), BC.into(), CA.into()],
                reference: [A, B, C]
                    .into_iter()
                    .zip(made)
                    .map(|(id, position)| GroupReference::Point {
                        point: id.into(),
                        position,
                    })
                    .collect(),
            },
        }],
        ..Problem::default()
    }
}

fn solved(problem: &Problem) -> impl Fn(PointId) -> Vector {
    let system = compile(problem).unwrap();
    let answer = solve(problem).unwrap();
    assert!(
        matches!(answer.outcome, SolveOutcome::Converged { .. }),
        "{:?}",
        answer.outcome
    );
    move |id| {
        let k = system.point_variable(id.into()).unwrap().0;
        at(answer.values[k], answer.values[k + 1])
    }
}

fn distance(a: Vector, b: Vector) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn assert_shape(position: &impl Fn(PointId) -> Vector) {
    let (a, b, c) = (position(A), position(B), position(C));
    assert!((distance(a, b) - 4.0).abs() < 1e-7, "{a:?} {b:?}");
    assert!((distance(b, c) - 5.0).abs() < 1e-7, "{b:?} {c:?}");
    assert!((distance(c, a) - 3.0).abs() < 1e-7, "{c:?} {a:?}");
}

#[test]
fn a_dragged_group_keeps_its_shape() {
    // One corner pulled away and held there: the rest follow it.
    let mut problem = triangle([at(10.0, 5.0), at(4.0, 0.0), at(0.0, 3.0)]);
    problem.held_points = vec![A.into()];
    let position = solved(&problem);
    assert_shape(&position);
    assert!(distance(position(A), at(10.0, 5.0)) < 1e-9);
}

#[test]
fn a_turned_group_keeps_its_shape() {
    let mut problem = triangle([at(0.0, 0.0), at(4.0, 0.0), at(0.0, 3.0)]);
    problem.constraints.push(Constraint {
        id: ConstraintId(101),
        kind: Relation::AngleToAxis {
            line: AB.into(),
            axis: AxisDirection::Horizontal,
            angle_rad: Radians(1.0),
        },
    });
    let position = solved(&problem);
    assert_shape(&position);
    let (a, b) = (position(A), position(B));
    assert!(((b.y - a.y).atan2(b.x - a.x) - 1.0).abs() < 1e-7);
}

#[test]
fn a_group_starts_from_where_its_members_stand() {
    // Already turned and moved since it was made: nothing to do.
    let (sin, cos) = 0.5f64.sin_cos();
    let put = |x: f64, y: f64| at(7.0 + cos * x - sin * y, -2.0 + sin * x + cos * y);
    let problem = triangle([put(0.0, 0.0), put(4.0, 0.0), put(0.0, 3.0)]);
    let answer = solve(&problem).unwrap();
    assert_eq!(answer.outcome, SolveOutcome::Converged { iterations: 0 });
}

#[test]
fn a_free_group_has_three_degrees_of_freedom() {
    let mut problem = triangle([at(0.0, 0.0), at(4.0, 0.0), at(0.0, 3.0)]);
    assert_eq!(
        sketch_solver::freedom::analyze(&problem).unwrap().degrees,
        3
    );
    // A dimension among the members rests rather than counting twice.
    problem.constraints.push(Constraint {
        id: ConstraintId(101),
        kind: Relation::Length {
            line: AB.into(),
            length: Length(4.0),
        },
    });
    assert_eq!(
        sketch_solver::freedom::analyze(&problem).unwrap().degrees,
        3
    );
    let diagnosis = sketch_solver::diagnosis::analyze(&problem).unwrap();
    assert!(diagnosis.redundant.is_empty() && diagnosis.conflicting.is_empty());
    assert_eq!(diagnosis.dof, 3);
}

#[test]
fn one_orientation_among_the_members_turns_the_group_and_the_rest_rest() {
    let mut problem = triangle([at(0.0, 0.0), at(4.0, 0.0), at(0.0, 3.0)]);
    problem.constraints.extend([
        Constraint {
            id: ConstraintId(101),
            kind: Relation::Horizontal { element: AB.into() },
        },
        Constraint {
            id: ConstraintId(102),
            kind: Relation::Vertical { element: CA.into() },
        },
    ]);
    let diagnosis = sketch_solver::diagnosis::analyze(&problem).unwrap();
    assert_eq!(diagnosis.dof, 2);
    assert!(diagnosis.redundant.is_empty() && diagnosis.conflicting.is_empty());
}

#[test]
fn a_group_with_a_fixed_point_and_an_angle_is_fully_constrained() {
    let mut problem = triangle([at(1.0, 1.0), at(5.0, 1.0), at(1.0, 4.0)]);
    problem.constraints.extend([
        Constraint {
            id: ConstraintId(101),
            kind: Relation::FixedPoint {
                point: A.into(),
                position: at(2.0, 3.0),
            },
        },
        Constraint {
            id: ConstraintId(102),
            kind: Relation::Angle {
                line1: CurveReference::XAxis,
                line2: AB.into(),
                angle_rad: Radians(-0.25),
            },
        },
    ]);
    assert_eq!(
        sketch_solver::freedom::analyze(&problem).unwrap().degrees,
        0
    );
    let position = solved(&problem);
    assert_shape(&position);
    assert!(distance(position(A), at(2.0, 3.0)) < 1e-8);
    let diagnosis = sketch_solver::diagnosis::analyze(&problem).unwrap();
    assert!(diagnosis.redundant.is_empty() && diagnosis.conflicting.is_empty());
}

#[test]
fn a_grouped_arc_turns_whole() {
    let (centre, start, end, arc) = (PointId(1), PointId(2), PointId(3), CurveId(10));
    let made = [at(0.0, 0.0), at(2.0, 0.0), at(0.0, 2.0)];
    let mut problem = Problem {
        geometry: vec![
            point(centre, made[0]),
            point(start, made[1]),
            point(end, made[2]),
            Geometry::Arc(Arc {
                id: arc,
                center: centre.into(),
                start: start.into(),
                end: end.into(),
                radius: Length(2.0),
            }),
        ],
        constraints: vec![Constraint {
            id: GROUP,
            kind: Relation::Group {
                elements: vec![arc.into()],
                reference: [centre, start, end]
                    .into_iter()
                    .zip(made)
                    .map(|(id, position)| GroupReference::Point {
                        point: id.into(),
                        position,
                    })
                    .chain([GroupReference::Radius {
                        curve: arc.into(),
                        radius: Length(2.0),
                    }])
                    .collect(),
            },
        }],
        ..Problem::default()
    };
    assert_eq!(
        sketch_solver::freedom::analyze(&problem).unwrap().degrees,
        3
    );
    problem.constraints.push(Constraint {
        id: ConstraintId(101),
        kind: Relation::VerticalPoints {
            point1: centre.into(),
            point2: start.into(),
        },
    });
    let position = solved(&problem);
    let (c, s, e) = (position(centre), position(start), position(end));
    assert!((c.x - s.x).abs() < 1e-8);
    assert!((distance(c, s) - 2.0).abs() < 1e-8 && (distance(c, e) - 2.0).abs() < 1e-8);
    assert!((distance(s, e) - 8f64.sqrt()).abs() < 1e-8);
}

#[test]
fn a_group_must_hold_every_point_of_its_members() {
    let mut problem = triangle([at(0.0, 0.0), at(4.0, 0.0), at(0.0, 3.0)]);
    if let Relation::Group { reference, .. } = &mut problem.constraints[0].kind {
        reference.pop();
    }
    assert!(matches!(
        compile(&problem),
        Err(InputError::InvalidRelation {
            constraint: GROUP,
            ..
        })
    ));
}
