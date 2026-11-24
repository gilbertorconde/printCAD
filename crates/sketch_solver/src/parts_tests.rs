use super::{PartCache, Parted};
use crate::compile::{System, compile_system};
use crate::freedom::{estimate, locked};
use crate::problem::{
    Arc as ArcGeometry, Circle, Constraint, ConstraintId, CurveId, CurveReference, Geometry,
    Length, Line, Point, PointId, PointReference, Problem, Relation as R, Vector,
};
use crate::solve::{SolveOutcome, iterate};
use std::collections::BTreeMap;

/// A fixed-seed generator, so every run draws the same sketches.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Cells of a rectangle, a circle and an arc each, drawn a little off
/// what relations drawn at random ask of them, so solves iterate: sides
/// held level, upright, parallel or to a length, a corner fixed or on the
/// origin or the x axis, the circle sized or tangent to a side, the arc's
/// centre on a corner; now and then a side held to two lengths, which no
/// solve meets, a corner external or held, and a distance from one cell
/// to the next, which joins them.
fn drawn(seed: u64, cells: usize) -> Problem {
    let mut rng = Rng(seed);
    let mut problem = Problem::default();
    let mut next = seed as u128 * 1000;
    let mut id = || {
        next += 1;
        next
    };
    let mut corners_of = Vec::new();
    for cell in 0..cells {
        let (x, y) = (cell as f64 * 10.0, (cell % 3) as f64 * 7.0);
        let (w, h) = (4.0, 3.0);
        let mut jitter = || (rng.unit() - 0.5) * 0.4;
        let corners: Vec<PointReference> = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
            .into_iter()
            .map(|(px, py)| {
                let p = PointId(id());
                problem.geometry.push(Geometry::Point(Point {
                    id: p,
                    position: Vector {
                        x: px + jitter(),
                        y: py + jitter(),
                    },
                }));
                p.into()
            })
            .collect();
        let sides: Vec<CurveReference> = (0..4)
            .map(|k| {
                let l = CurveId(id());
                problem.geometry.push(Geometry::Line(Line {
                    id: l,
                    start: corners[k],
                    end: corners[(k + 1) % 4],
                }));
                l.into()
            })
            .collect();
        let centre = PointId(id());
        problem.geometry.push(Geometry::Point(Point {
            id: centre,
            position: Vector {
                x: x + w + 2.0,
                y: y + h,
            },
        }));
        let circle = CurveId(id());
        problem.geometry.push(Geometry::Circle(Circle {
            id: circle,
            center: centre.into(),
            radius: Length(2.0 + jitter()),
        }));
        let ends: Vec<PointReference> = [(x - 2.0, y), (x, y - 2.0)]
            .into_iter()
            .map(|(px, py)| {
                let p = PointId(id());
                problem.geometry.push(Geometry::Point(Point {
                    id: p,
                    position: Vector { x: px, y: py },
                }));
                p.into()
            })
            .collect();
        let arc = CurveId(id());
        problem.geometry.push(Geometry::Arc(ArcGeometry {
            id: arc,
            center: corners[0],
            start: ends[0],
            end: ends[1],
            radius: Length(2.0 + jitter()),
        }));
        let circle: CurveReference = circle.into();
        let pool = [
            R::Horizontal { element: sides[0] },
            R::Horizontal { element: sides[2] },
            R::Vertical { element: sides[1] },
            R::Vertical { element: sides[3] },
            R::Parallel {
                line1: sides[0],
                line2: sides[2],
            },
            R::Length {
                line: sides[0],
                length: Length(w),
            },
            R::Radius {
                circle,
                radius: Length(2.0),
            },
            R::FixedPoint {
                point: corners[0],
                position: Vector { x, y },
            },
            R::Tangent {
                line_or_circle1: sides[1],
                item2: circle,
            },
            R::Coincident {
                point1: PointReference::Origin,
                point2: corners[0],
            },
            R::PointOnLine {
                point: corners[1],
                line: CurveReference::XAxis,
            },
        ];
        for relation in pool {
            if rng.chance(40) {
                problem.constraints.push(Constraint {
                    id: ConstraintId(id()),
                    kind: relation,
                });
            }
        }
        if rng.chance(15) {
            for length in [w, w + 1.0] {
                problem.constraints.push(Constraint {
                    id: ConstraintId(id()),
                    kind: R::Length {
                        line: sides[2],
                        length: Length(length),
                    },
                });
            }
        }
        if rng.chance(10) {
            problem.external.push(corners[2].into());
        }
        if rng.chance(10) {
            problem.held_points.push(corners[3]);
        }
        corners_of.push(corners);
    }
    for pair in corners_of.windows(2) {
        if rng.chance(20) {
            problem.constraints.push(Constraint {
                id: ConstraintId(id()),
                kind: R::Distance {
                    point1: pair[0][1],
                    point2: pair[1][0],
                    distance: Length(6.0),
                },
            });
        }
    }
    problem.external.sort();
    problem
}

/// What a solve leaves, by element: each point's coordinates and each
/// curve's radius, to the bit.
type Answer = BTreeMap<String, u64>;

fn read(answer: &mut Answer, sys: &System, x: &[f64]) {
    for (point, &k) in &sys.point_vars {
        answer.insert(format!("{point:?}.x"), x[k].to_bits());
        answer.insert(format!("{point:?}.y"), x[k + 1].to_bits());
    }
    for (curve, &k) in &sys.radius_vars {
        answer.insert(format!("{curve:?}.r"), x[k].to_bits());
    }
}

/// The solve of `problem` compiled as one system, and its parts' from
/// `cache`, element by element with the outcome, iterations, residual and
/// threshold: equal to the bit.
fn solves_alike(problem: &Problem, cache: &mut PartCache, what: &str) -> SolveOutcome {
    let whole = compile_system(problem, None, false);
    let one = iterate(&whole, problem.settings);
    let mut expected = Answer::new();
    read(&mut expected, &whole, &one.values);

    let parted = Parted::of(&std::sync::Arc::new(problem.clone()), cache);
    let solved = parted.iterate(problem.settings);
    let mut got = Answer::new();
    for ((_, sys), x) in parted.systems().zip(&solved.values) {
        read(&mut got, sys, x.as_deref().unwrap_or(&sys.vars));
    }
    assert_eq!(got, expected, "{what}: values");
    let outcome = |o: SolveOutcome| match o {
        SolveOutcome::NotConverged { residual } => format!("not converged {}", residual.to_bits()),
        other => format!("{other:?}"),
    };
    assert_eq!(outcome(solved.outcome), outcome(one.outcome), "{what}");
    assert_eq!(solved.iterations, one.iterations, "{what}");
    assert_eq!(solved.residual.to_bits(), one.residual.to_bits(), "{what}");
    assert_eq!(
        solved.threshold.to_bits(),
        one.threshold.to_bits(),
        "{what}"
    );
    assert_eq!(parted.estimate(), estimate(problem), "{what}: freedom");
    assert_eq!(parted.locked(), locked(problem), "{what}: locked");
    one.outcome
}

#[test]
fn parts_solve_and_measure_as_the_whole_system_does() {
    let mut cache = PartCache::default();
    let (mut parted_at_all, mut iterated, mut failed) = (0, 0, 0);
    for seed in 1..60 {
        let problem = drawn(seed, 1 + (seed as usize % 7));
        match solves_alike(&problem, &mut cache, &format!("seed {seed}")) {
            SolveOutcome::Converged { iterations } if iterations > 0 => iterated += 1,
            SolveOutcome::NotConverged { .. } => failed += 1,
            _ => {}
        }
        parted_at_all +=
            usize::from(Parted::of(&std::sync::Arc::new(problem.clone()), &mut cache).len() > 1);
    }
    // Most sketches fall in parts; some solve and some cannot.
    assert!(parted_at_all > 40, "{parted_at_all}");
    assert!(
        iterated > 10 && failed > 5,
        "{iterated} solved, {failed} not"
    );
}

/// The value of the length or radius relation `c` holds, raised by `by`.
fn raise(c: &mut Constraint, by: f64) -> bool {
    match &mut c.kind {
        R::Length { length, .. } | R::Radius { radius: length, .. } => {
            length.0 += by;
            true
        }
        _ => false,
    }
}

#[test]
fn edited_parts_are_compiled_again_and_untouched_ones_kept() {
    // One cache through a run of edits, each compared with the whole
    // system solved afresh: a part kept from before an edit that changed
    // it answers what it answered then, and the solve tells it apart.
    let mut cache = PartCache::default();
    let mut problem = drawn(7, 8);
    let _ = solves_alike(&problem, &mut cache, "drawn");
    let mut rng = Rng(99);
    for step in 0..64 {
        let what = format!("step {step}");
        match step % 8 {
            // A point moved; now and then a fixed one far, which its
            // solve brings back in steps.
            0 => {
                let fixed = (problem.constraints.iter()).find_map(|c| match c.kind {
                    R::FixedPoint { point, .. } if step % 10 == 0 => Some(point),
                    _ => None,
                });
                let at = match fixed {
                    Some(point) => (problem.geometry.iter())
                        .position(|g| g.id() == point.into())
                        .unwrap_or(0),
                    None => rng.next() as usize % problem.geometry.len(),
                };
                if let Geometry::Point(p) = &mut problem.geometry[at] {
                    p.position.x += if fixed.is_some() { 40.0 } else { 0.25 };
                }
            }
            // A dimension's value changed.
            1 => {
                let at = rng.next() as usize % problem.constraints.len().max(1);
                if let Some(c) = problem.constraints.get_mut(at) {
                    raise(c, 0.5);
                }
            }
            // Two cells joined by a distance, then that taken away.
            2 => {
                let points: Vec<PointReference> = (problem.geometry.iter())
                    .filter_map(|g| match g {
                        Geometry::Point(p) => Some(p.id.into()),
                        _ => None,
                    })
                    .collect();
                let (a, b) = (points[3], points[points.len() - 5]);
                problem.constraints.push(Constraint {
                    id: ConstraintId(1_000_000 + step as u128),
                    kind: R::Distance {
                        point1: a,
                        point2: b,
                        distance: Length(30.0),
                    },
                });
            }
            3 => {
                problem
                    .constraints
                    .retain(|c| c.id.0 < 1_000_000 || rng.chance(50));
            }
            // Elements added: a line between two cells' corners, which
            // joins them, and a circle on a new centre, then a radius on
            // it and a constraint naming nothing the sketch holds.
            5 => {
                let points: Vec<PointReference> = (problem.geometry.iter())
                    .filter_map(|g| match g {
                        Geometry::Point(p) => Some(p.id.into()),
                        _ => None,
                    })
                    .collect();
                let base = 2_000_000 + 10 * step as u128;
                problem.geometry.push(Geometry::Line(Line {
                    id: CurveId(base),
                    start: points[rng.next() as usize % points.len()],
                    end: points[rng.next() as usize % points.len()],
                }));
                problem.geometry.push(Geometry::Point(Point {
                    id: PointId(base + 1),
                    position: Vector {
                        x: -20.0,
                        y: step as f64,
                    },
                }));
                problem.geometry.push(Geometry::Circle(Circle {
                    id: CurveId(base + 2),
                    center: PointId(base + 1).into(),
                    radius: Length(1.5),
                }));
            }
            6 => {
                let circle = (problem.geometry.iter()).rev().find_map(|g| match g {
                    Geometry::Circle(c) => Some(c.id),
                    _ => None,
                });
                if let Some(circle) = circle {
                    problem.constraints.push(Constraint {
                        id: ConstraintId(3_000_000 + step as u128),
                        kind: R::Radius {
                            circle: circle.into(),
                            radius: Length(1.0),
                        },
                    });
                }
                problem.constraints.push(Constraint {
                    id: ConstraintId(4_000_000 + step as u128),
                    kind: R::PointOnLine {
                        point: PointReference::Origin,
                        line: CurveReference::XAxis,
                    },
                });
            }
            // A point held, then let go.
            _ => {
                if problem.held_points.is_empty() {
                    if let Some(Geometry::Point(p)) = problem.geometry.first() {
                        problem.held_points.push(p.id.into());
                    }
                } else {
                    problem.held_points.clear();
                }
            }
        }
        let _ = solves_alike(&problem, &mut cache, &what);
        // The sketch as the solve left it, as a store writes it back: the
        // parts that moved change, the others stay.
        let whole = compile_system(&problem, None, false);
        let answer = iterate(&whole, problem.settings);
        for g in &mut problem.geometry {
            if let Geometry::Point(p) = g
                && let Some(&k) = whole.point_vars.get(&p.id.into())
            {
                p.position = Vector {
                    x: answer.values[k],
                    y: answer.values[k + 1],
                };
            }
        }
        let _ = solves_alike(&problem, &mut cache, &format!("{what}, stored"));
    }
}

#[test]
fn a_constraint_naming_no_element_joins_a_part_of_its_own() {
    let mut cache = PartCache::default();
    let mut problem = drawn(3, 4);
    problem.constraints.push(Constraint {
        id: ConstraintId(5),
        kind: R::Horizontal {
            element: CurveReference::Curve(CurveId(424242)),
        },
    });
    problem.constraints.push(Constraint {
        id: ConstraintId(6),
        kind: R::PointOnLine {
            point: PointReference::Origin,
            line: CurveReference::YAxis,
        },
    });
    let _ = solves_alike(&problem, &mut cache, "loose");
    // Repeated ids keep the problem whole.
    let first = problem.constraints[0].clone();
    problem.constraints.push(first);
    assert_eq!(
        Parted::of(&std::sync::Arc::new(problem.clone()), &mut cache).len(),
        1
    );
    let _ = solves_alike(&problem, &mut cache, "repeated");
}
