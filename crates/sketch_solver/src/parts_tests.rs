use super::{PartCache, Parted};
use crate::compile::{System, compile_system};
use crate::diagnosis::{Diagnosis, diagnose, solved_at};
use crate::freedom::{estimate, locked};
use crate::problem::{
    Arc as ArcGeometry, Circle, Constraint, ConstraintId, CurveId, CurveReference, Geometry,
    Length, Line, Point, PointId, PointReference, Problem, Relation as R, Vector,
};
use crate::solve::SolveOutcome::{self, NotConverged};
use crate::solve::iterate;
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

/// What [`solves_alike`] found: the whole system's outcome and its
/// diagnosis, and whether the parts answered the diagnosis themselves.
struct Alike {
    outcome: SolveOutcome,
    diagnosis: Diagnosis,
    by_parts: bool,
}

/// The solve of `problem` compiled as one system, and its parts' from
/// `cache`, element by element with the outcome, iterations, residual and
/// threshold: equal to the bit. Their diagnoses too, where `diagnosed`
/// takes the outcome: the parts' where they answer one, from the problem
/// as each solve leaves it.
fn solves_alike(
    problem: &Problem,
    cache: &mut PartCache,
    what: &str,
    diagnosed: &dyn Fn(SolveOutcome) -> bool,
) -> Alike {
    let whole = compile_system(problem, None, false);
    let one = iterate(&whole, problem.settings);
    let mut expected = Answer::new();
    read(&mut expected, &whole, &one.values);

    let parted = Parted::of(&std::sync::Arc::new(problem.clone()), cache);
    let solved = parted.iterate(problem.settings);
    let mut got = Answer::new();
    for (k, (_, sys)) in parted.systems().enumerate() {
        read(&mut got, sys, solved.values.of(k).unwrap_or(&sys.vars));
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

    if !diagnosed(one.outcome) {
        return Alike {
            outcome: one.outcome,
            diagnosis: Diagnosis::default(),
            by_parts: false,
        };
    }
    let diagnosis = diagnose(problem);
    let solved_problem = std::sync::Arc::new(solved_at(problem, &whole, &one.values));
    let solved_parts = Parted::of(&solved_problem, cache);
    let by_parts = parted.diagnose(
        problem,
        one.outcome,
        &solved_problem,
        &solved_parts,
        estimate(problem).degrees,
    );
    if let Some(by_parts) = &by_parts {
        assert_eq!(by_parts, &diagnosis, "{what}: diagnosis");
    }
    Alike {
        outcome: one.outcome,
        diagnosis,
        by_parts: by_parts.is_some(),
    }
}

#[test]
fn parts_solve_and_measure_as_the_whole_system_does() {
    let mut cache = PartCache::default();
    let (mut parted_at_all, mut iterated, mut failed) = (0, 0, 0);
    let (mut redundant, mut conflicting, mut by_parts) = (0, 0, 0);
    for seed in 1..60 {
        let problem = drawn(seed, 1 + (seed as usize % 7));
        // A failing sketch's whole diagnosis solves it once per
        // constraint: the smaller ones are enough.
        let few = problem.constraints.len() <= 24;
        let diagnosed = |o: SolveOutcome| few || !matches!(o, NotConverged { .. });
        let alike = solves_alike(&problem, &mut cache, &format!("seed {seed}"), &diagnosed);
        match alike.outcome {
            SolveOutcome::Converged { iterations } if iterations > 0 => iterated += 1,
            SolveOutcome::NotConverged { .. } => failed += 1,
            _ => {}
        }
        redundant += usize::from(!alike.diagnosis.redundant.is_empty());
        conflicting += usize::from(!alike.diagnosis.conflicting.is_empty());
        by_parts += usize::from(alike.by_parts);
        parted_at_all +=
            usize::from(Parted::of(&std::sync::Arc::new(problem.clone()), &mut cache).len() > 1);
    }
    // Most sketches fall in parts; some solve and some cannot.
    assert!(parted_at_all > 40, "{parted_at_all}");
    assert!(
        iterated > 10 && failed > 5,
        "{iterated} solved, {failed} not"
    );
    // Their diagnoses name redundant and conflicting constraints, nearly
    // all of them read from the parts.
    assert!(
        redundant > 3 && conflicting > 3 && by_parts > 40,
        "{redundant} redundant, {conflicting} conflicting, {by_parts} by parts"
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
    let _ = solves_alike(&problem, &mut cache, "drawn", &|_| true);
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
        let _ = solves_alike(&problem, &mut cache, &what, &|_| step % 8 == 3);
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
        let _ = solves_alike(&problem, &mut cache, &format!("{what}, stored"), &|_| false);
    }
}

/// The parts of `problem` split afresh, in the order of their first
/// elements, with what solving and measuring them answers.
fn split_afresh(parted: &Parted) -> (Vec<Problem>, crate::freedom::Freedom) {
    let mut problems: Vec<Problem> = parted.systems().map(|(p, _)| p.clone()).collect();
    problems.sort_by_key(|p| p.geometry.first().map(|g| g.id()));
    (problems, parted.estimate())
}

#[test]
fn a_split_edited_by_a_list_of_changes_is_the_problem_split_afresh() {
    // A kept split edited through what each step lists (points moved,
    // elements and constraints added) holds the problem a full edit makes
    // and splits it as a split afresh does, solving to the same answer.
    let mut cache = PartCache::default();
    let mut problem = std::sync::Arc::new(drawn(11, 8));
    let mut stamp = Parted::of(&problem, &mut cache).stamp().expect("kept");
    let mut rng = Rng(5);
    for step in 0..48u128 {
        let mut expect = (*problem).clone();
        let points: Vec<usize> = (expect.geometry.iter().enumerate())
            .filter(|(_, g)| matches!(g, Geometry::Point(_)))
            .map(|(i, _)| i)
            .collect();
        let moved = points[rng.next() as usize % points.len()];
        if let Geometry::Point(p) = &mut expect.geometry[moved] {
            p.position.y += 0.125;
        }
        let pick = |rng: &mut Rng| match expect.geometry[points[rng.next() as usize % points.len()]]
        {
            Geometry::Point(ref p) => PointReference::from(p.id),
            _ => unreachable!(),
        };
        let (a, b) = (pick(&mut rng), pick(&mut rng));
        let base = 5_000_000 + 10 * step;
        let mut added = vec![Geometry::Point(Point {
            id: PointId(base),
            position: Vector {
                x: -30.0,
                y: step as f64,
            },
        })];
        added.push(match step % 3 {
            0 => Geometry::Line(Line {
                id: CurveId(base + 1),
                start: a,
                end: b,
            }),
            1 => Geometry::Line(Line {
                id: CurveId(base + 1),
                start: a,
                end: PointId(base).into(),
            }),
            _ => Geometry::Circle(Circle {
                id: CurveId(base + 1),
                center: PointId(base).into(),
                radius: Length(2.0),
            }),
        });
        let constraint = Constraint {
            id: ConstraintId(base),
            kind: R::Distance {
                point1: PointId(base).into(),
                point2: b,
                distance: Length(12.0),
            },
        };
        expect.geometry.extend(added.iter().cloned());
        expect.constraints.push(constraint.clone());
        let (edited, parts) = Parted::edited(stamp, &mut cache, |edit| {
            edit.set_geometry(moved, expect.geometry[moved].clone());
            for g in &added {
                edit.push_geometry(g.clone());
            }
            if step % 4 != 1 {
                edit.push_constraint(constraint.clone());
            }
            true
        })
        .expect("the split is kept");
        if step % 4 == 1 {
            expect.constraints.pop();
        }
        assert_eq!(*edited, expect, "step {step}");
        let alone = Parted::of(
            &std::sync::Arc::new(expect.clone()),
            &mut PartCache::default(),
        );
        assert_eq!(split_afresh(&parts), split_afresh(&alone), "step {step}");
        let (ours, theirs) = (
            parts.iterate(expect.settings),
            alone.iterate(expect.settings),
        );
        assert_eq!(
            (ours.outcome, ours.iterations, ours.residual.to_bits()),
            (theirs.outcome, theirs.iterations, theirs.residual.to_bits()),
            "step {step}"
        );
        // The facts kept summed block by block as parts were replaced
        // and added are those of every part summed afresh.
        let mut afresh = super::Summary::default();
        for entry in parts.parts.iter() {
            afresh.add(&entry.facts);
        }
        assert_eq!(parts.summary, afresh, "step {step}");
        assert_eq!(parts.parts.summary(), afresh, "step {step}");
        stamp = parts.stamp().expect("kept");
        problem = edited;
    }
    // A stamp no longer kept, or an element set to one naming other
    // points, answers nothing.
    let stale = Parted::of(&problem, &mut cache).stamp().expect("kept");
    let next = Parted::of(&problem, &mut cache).stamp().expect("kept");
    assert_ne!(stale, next);
    assert!(Parted::edited(stale, &mut cache, |_| true).is_none());
    let line = (problem.geometry.iter()).position(|g| matches!(g, Geometry::Line(_)));
    let mut other = problem.geometry[line.expect("a line")].clone();
    if let Geometry::Line(l) = &mut other {
        std::mem::swap(&mut l.start, &mut l.end);
    }
    let broken = Parted::edited(next, &mut cache, |edit| {
        edit.set_geometry(line.expect("a line"), other.clone());
        true
    });
    assert!(broken.is_none());
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
    let _ = solves_alike(&problem, &mut cache, "loose", &|_| true);
    // Repeated ids keep the problem whole.
    let first = problem.constraints[0].clone();
    problem.constraints.push(first);
    assert_eq!(
        Parted::of(&std::sync::Arc::new(problem.clone()), &mut cache).len(),
        1
    );
    let _ = solves_alike(&problem, &mut cache, "repeated", &|_| true);
}

/// A row of rectangles, each its own part, every third with its sides
/// held level and upright, its bottom and left sides to a length and a
/// corner fixed, drawn a little off so its solve iterates (as many
/// constraints as a diagnosis takes); with the bottom side of each cell in
/// `twice` held to a second length too, which no solve meets.
fn rectangles(cells: usize, twice: &[usize]) -> (Problem, Vec<ConstraintId>) {
    let mut problem = Problem::default();
    let mut next = 0u128;
    let mut id = || {
        next += 1;
        next
    };
    let mut seconds = Vec::new();
    for cell in 0..cells {
        let (x, y) = (cell as f64 * 10.0, 0.0);
        let corners: Vec<PointReference> = [(0.0, 0.0), (4.1, 0.1), (4.0, 3.2), (-0.1, 2.9)]
            .into_iter()
            .map(|(dx, dy)| {
                let p = PointId(id());
                problem.geometry.push(Geometry::Point(Point {
                    id: p,
                    position: Vector {
                        x: x + dx,
                        y: y + dy,
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
        if cell % 3 != 0 {
            continue;
        }
        let mut kinds = vec![
            R::Horizontal { element: sides[0] },
            R::Horizontal { element: sides[2] },
            R::Vertical { element: sides[1] },
            R::Vertical { element: sides[3] },
            R::Length {
                line: sides[0],
                length: Length(4.0),
            },
            R::Length {
                line: sides[3],
                length: Length(3.0),
            },
            R::FixedPoint {
                point: corners[0],
                position: Vector { x, y },
            },
        ];
        if twice.contains(&cell) {
            kinds.push(R::Length {
                line: sides[0],
                length: Length(5.0),
            });
        }
        for kind in kinds {
            let c = ConstraintId(id());
            if matches!(kind, R::Length { line, .. } if line == sides[0]) && twice.contains(&cell) {
                seconds.push(c);
            }
            problem.constraints.push(Constraint { id: c, kind });
        }
    }
    (problem, seconds)
}

/// A large sketch with one part held to two lengths: its parts name the
/// conflict the whole problem's diagnosis names, the two lengths alone,
/// probing that part only; with two such parts, neither is a conflict
/// alone, as the whole diagnosis finds too.
#[test]
fn a_conflict_in_one_part_is_found_by_that_part_alone() {
    let mut cache = PartCache::default();
    for (twice, expected) in [(&[18][..], 2), (&[3, 21][..], 0)] {
        let (problem, seconds) = rectangles(24, twice);
        let parted = Parted::of(&std::sync::Arc::new(problem.clone()), &mut cache);
        assert_eq!(parted.len(), 24);
        let solved = parted.iterate(problem.settings);
        assert!(matches!(solved.outcome, NotConverged { .. }));
        let whole = diagnose(&problem);
        let by_parts = parted
            .diagnose(
                &problem,
                solved.outcome,
                &problem,
                &parted,
                parted.estimate().degrees,
            )
            .expect("the parts answer");
        assert_eq!(by_parts, whole, "{twice:?}");
        assert!(whole.analyzed, "{twice:?}");
        assert_eq!(whole.conflicting.len(), expected, "{twice:?}");
        if expected > 0 {
            assert_eq!(whole.conflicting, seconds, "{twice:?}");
        }
    }
}
