//! Bounded planted-feasible inputs and structural reduction of failed cases.

use sketch_solver::compile::compile;
use sketch_solver::problem::*;
use sketch_solver::solve::{Iteration, SolveOutcome, solve};

#[derive(Clone, Copy)]
struct Seed(u64);
impl Seed {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1_u64 << 53) as f64
    }
}

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
fn fixed(id: u128, p: u128, x: f64, y: f64) -> Constraint {
    constraint(
        id,
        Relation::FixedPoint {
            point: PointId(p).into(),
            position: Vector { x, y },
        },
    )
}
fn at(problem: &Problem, answer: &Iteration, id: u128) -> Vector {
    let index = compile(problem)
        .unwrap()
        .point_variable(PointId(id).into())
        .unwrap()
        .0;
    Vector {
        x: answer.values[index],
        y: answer.values[index + 1],
    }
}
fn distance(a: Vector, b: Vector) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

enum Expected {
    Line {
        start: Vector,
        length: f64,
        angle: f64,
    },
    Circle {
        center: Vector,
        radius: f64,
    },
    Tangent {
        radius1: f64,
        radius2: f64,
        internal: bool,
    },
}

fn planted(seed: u64, family: usize) -> (Problem, Expected) {
    let mut random = Seed(seed);
    let x = random.next() * 20.0 - 10.0;
    let y = random.next() * 20.0 - 10.0;
    let size = 1.0 + 40.0 * random.next();
    let angle = (random.next() - 0.5) * std::f64::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    let mut input = Problem::default();
    match family {
        0 => {
            input.geometry = vec![
                point(1, x, y),
                point(2, x + size * (cos + 0.15), y + size * (sin - 0.12)),
                Geometry::Line(Line {
                    id: CurveId(3),
                    start: PointId(1).into(),
                    end: PointId(2).into(),
                }),
            ];
            input.constraints = vec![
                fixed(1, 1, x, y),
                constraint(
                    2,
                    Relation::Length {
                        line: CurveId(3).into(),
                        length: Length(size),
                    },
                ),
                constraint(
                    3,
                    Relation::AngleToAxis {
                        line: CurveId(3).into(),
                        axis: AxisDirection::Horizontal,
                        angle_rad: Radians(angle),
                    },
                ),
            ];
            (
                input,
                Expected::Line {
                    start: Vector { x, y },
                    length: size,
                    angle,
                },
            )
        }
        1 => {
            input.geometry = vec![
                point(1, x, y),
                point(2, x + size * cos * 1.2, y + size * sin * 1.2),
                Geometry::Circle(Circle {
                    id: CurveId(3),
                    center: PointId(1).into(),
                    radius: Length(size * 0.8),
                }),
            ];
            input.constraints = vec![
                fixed(1, 1, x, y),
                constraint(
                    2,
                    Relation::Radius {
                        circle: CurveId(3).into(),
                        radius: Length(size),
                    },
                ),
                constraint(
                    3,
                    Relation::PointOnCircle {
                        point: PointId(2).into(),
                        circle: CurveId(3).into(),
                    },
                ),
            ];
            (
                input,
                Expected::Circle {
                    center: Vector { x, y },
                    radius: size,
                },
            )
        }
        _ => {
            let other = size * (0.2 + random.next() * 0.3);
            let internal = seed.is_multiple_of(2);
            let separation = if internal {
                (size - other) * 0.9
            } else {
                (size + other) * 1.1
            };
            input.geometry = vec![
                point(1, 0.0, 0.0),
                point(2, separation, 0.0),
                Geometry::Circle(Circle {
                    id: CurveId(3),
                    center: PointId(1).into(),
                    radius: Length(size),
                }),
                Geometry::Circle(Circle {
                    id: CurveId(4),
                    center: PointId(2).into(),
                    radius: Length(other),
                }),
            ];
            input.constraints = vec![
                fixed(1, 1, 0.0, 0.0),
                constraint(
                    2,
                    Relation::Radius {
                        circle: CurveId(3).into(),
                        radius: Length(size),
                    },
                ),
                constraint(
                    3,
                    Relation::Radius {
                        circle: CurveId(4).into(),
                        radius: Length(other),
                    },
                ),
                constraint(
                    4,
                    Relation::HorizontalPoints {
                        point1: PointId(1).into(),
                        point2: PointId(2).into(),
                    },
                ),
                constraint(
                    5,
                    Relation::Tangent {
                        line_or_circle1: CurveId(3).into(),
                        item2: CurveId(4).into(),
                    },
                ),
            ];
            (
                input,
                Expected::Tangent {
                    radius1: size,
                    radius2: other,
                    internal,
                },
            )
        }
    }
}

fn geometric_oracle(problem: &Problem, answer: &Iteration, expected: &Expected) -> bool {
    if !matches!(answer.outcome, SolveOutcome::Converged { .. }) {
        return false;
    }
    let a = at(problem, answer, 1);
    let b = at(problem, answer, 2);
    let tolerance = 1e-6;
    match *expected {
        Expected::Line {
            start,
            length,
            angle,
        } => {
            distance(a, start) < tolerance
                && (distance(a, b) - length).abs() < tolerance * length.max(1.0)
                && ((b.x - a.x) / length - angle.cos()).abs() < tolerance
                && ((b.y - a.y) / length - angle.sin()).abs() < tolerance
        }
        Expected::Circle { center, radius } => {
            distance(a, center) < tolerance
                && (distance(center, b) - radius).abs() < tolerance * radius.max(1.0)
        }
        Expected::Tangent {
            radius1,
            radius2,
            internal,
        } => {
            let expected = if internal {
                (radius1 - radius2).abs()
            } else {
                radius1 + radius2
            };
            distance(a, Vector { x: 0.0, y: 0.0 }) < tolerance
                && (distance(a, b) - expected).abs() < tolerance * expected.max(1.0)
        }
    }
}

fn shrink(mut input: Problem, fails: impl Fn(&Problem) -> bool) -> Problem {
    for index in (0..input.constraints.len()).rev() {
        let mut candidate = input.clone();
        candidate.constraints.remove(index);
        if sketch_solver::input::validate(&candidate).is_ok() && fails(&candidate) {
            input = candidate;
        }
    }
    for index in (0..input.geometry.len()).rev() {
        let mut candidate = input.clone();
        candidate.geometry.remove(index);
        if sketch_solver::input::validate(&candidate).is_ok() && fails(&candidate) {
            input = candidate;
        }
    }
    input
}

#[test]
fn bounded_planted_families_satisfy_independent_geometry() {
    for seed in 1..=96 {
        for family in 0..3 {
            let (input, expected) = planted(seed, family);
            let answer = solve(&input).unwrap();
            if !geometric_oracle(&input, &answer, &expected) {
                let reduced = shrink(input, |p| {
                    solve(p).is_ok_and(|a| {
                        !matches!(
                            a.outcome,
                            SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
                        )
                    })
                });
                #[cfg(feature = "serde")]
                eprintln!(
                    "{}",
                    serde_json::to_string(&sketch_solver::replay::Fixture {
                        schema_version: 1,
                        problem: reduced.clone()
                    })
                    .unwrap()
                );
                panic!("seed={seed}, family={family}, minimal={reduced:?}");
            }
        }
    }
}

#[test]
fn shrinking_preserves_a_conflict_and_removes_irrelevant_inputs() {
    let (mut input, _) = planted(42, 0);
    input.geometry.push(point(99, 7.0, 9.0));
    let Relation::Length { length, .. } = input.constraints[1].kind else {
        unreachable!()
    };
    input.constraints.push(constraint(
        10,
        Relation::Length {
            line: CurveId(3).into(),
            length: Length(length.0 + 2.0),
        },
    ));
    let reduced = shrink(input, |p| {
        solve(p).is_ok_and(|a| matches!(a.outcome, SolveOutcome::NotConverged { .. }))
    });
    assert_eq!(reduced.constraints.len(), 2);
    assert_eq!(reduced.geometry.len(), 3);
    for index in 0..reduced.constraints.len() {
        let mut without = reduced.clone();
        without.constraints.remove(index);
        assert!(matches!(
            solve(&without).unwrap().outcome,
            SolveOutcome::Converged { .. }
        ));
    }
}

#[test]
fn independent_oracle_rejects_an_answer_mutation() {
    let (input, expected) = planted(5, 0);
    let mut answer = solve(&input).unwrap();
    assert!(geometric_oracle(&input, &answer, &expected));
    let index = compile(&input)
        .unwrap()
        .point_variable(PointId(2).into())
        .unwrap()
        .0;
    answer.values[index] += 0.5;
    assert!(!geometric_oracle(&input, &answer, &expected));
}
