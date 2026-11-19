use std::collections::HashMap;
use uuid::Uuid;
use wb_sketch::operations::{PointJoinError, SolveOutcome, join_points, solve};
use wb_sketch::sketch::{
    Circle, ConstraintKind, GeometryElement, Line, ORIGIN_ID, Point, Sketch, Vec2D, group_of,
};

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

/// A circle at (`x`, `y`) and its centre.
fn circle(sketch: &mut Sketch, x: f64, y: f64) -> (Uuid, Uuid) {
    let center = point(sketch, x, y);
    let c = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
    (c, center)
}

/// The kinds of the sketch's constraints `pick` takes.
fn kinds<T>(sketch: &Sketch, pick: impl Fn(&ConstraintKind) -> Option<T>) -> Vec<T> {
    sketch
        .constraints
        .iter()
        .filter_map(|c| pick(&c.kind))
        .collect()
}

/// Deleting one member of a linked relation keeps it on the rest: an
/// offset its other pairs, a group its other members, a pitch or an
/// angular pitch the copies either side of the gap, each run of them
/// stepped as before, so nothing moves. It goes only when too few
/// members are left for it to say anything.
#[test]
fn deleting_a_member_keeps_a_linked_relation_on_the_rest() {
    let pitches = |s: &Sketch| {
        kinds(s, |k| match k {
            ConstraintKind::Pitch {
                points,
                columns,
                across,
                ..
            } => Some((points.clone(), *columns, *across)),
            _ => None,
        })
    };
    // A row of six: the last copy deleted shortens it; a middle one
    // splits it into the runs either side.
    let mut sketch = Sketch::new("row");
    let row: Vec<(Uuid, Uuid)> = (0..6)
        .map(|i| circle(&mut sketch, 10.0 * i as f64, 0.0))
        .collect();
    let centers: Vec<Uuid> = row.iter().map(|r| r.1).collect();
    sketch.add_constraint(ConstraintKind::Pitch {
        points: centers.clone(),
        columns: 6,
        distance: 10.0,
        across: false,
        direction: Some(Vec2D::new(1.0, 0.0)),
    });
    sketch.remove_geometry_cascade(&[row[5].0]);
    assert_eq!(pitches(&sketch), vec![(centers[..5].to_vec(), 5, false)]);
    sketch.remove_geometry_cascade(&[row[2].0]);
    assert_eq!(
        pitches(&sketch),
        vec![
            (centers[..2].to_vec(), 2, false),
            (centers[3..5].to_vec(), 2, false)
        ]
    );
    let before = serde_json::to_value(&sketch).unwrap();
    solve(&mut sketch).unwrap();
    assert_eq!(
        serde_json::to_value(&sketch).unwrap(),
        before,
        "nothing moves"
    );
    sketch.remove_geometry_cascade(&[row[1].0, row[4].0]);
    assert!(
        pitches(&sketch).is_empty(),
        "single copies step from nothing"
    );

    // A grid of two rows of three, a pitch along the rows and one down the
    // first column: the second row's middle copy deleted leaves the
    // first row stepped and the first column.
    let mut sketch = Sketch::new("grid");
    let grid: Vec<(Uuid, Uuid)> = (0..6)
        .map(|i| circle(&mut sketch, 10.0 * (i % 3) as f64, 8.0 * (i / 3) as f64))
        .collect();
    let centers: Vec<Uuid> = grid.iter().map(|g| g.1).collect();
    for (across, distance, way) in [
        (false, 10.0, Vec2D::new(1.0, 0.0)),
        (true, 8.0, Vec2D::new(0.0, 1.0)),
    ] {
        sketch.add_constraint(ConstraintKind::Pitch {
            points: centers.clone(),
            columns: 3,
            distance,
            across,
            direction: Some(way),
        });
    }
    sketch.remove_geometry_cascade(&[grid[4].0]);
    assert_eq!(
        pitches(&sketch),
        vec![
            (centers[..3].to_vec(), 3, false),
            (vec![centers[0], centers[3]], 1, true)
        ]
    );
    let before = serde_json::to_value(&sketch).unwrap();
    solve(&mut sketch).unwrap();
    assert_eq!(
        serde_json::to_value(&sketch).unwrap(),
        before,
        "nothing moves"
    );

    // Four turned copies about a pivot: a middle one deleted leaves the
    // two before it; the pivot deleted takes the pitch.
    let mut sketch = Sketch::new("polar");
    let pivot = point(&mut sketch, 0.0, 0.0);
    let ring: Vec<(Uuid, Uuid)> = (0..4)
        .map(|i| {
            let a = std::f64::consts::FRAC_PI_2 * i as f64;
            circle(&mut sketch, 10.0 * a.cos(), 10.0 * a.sin())
        })
        .collect();
    let centers: Vec<Uuid> = ring.iter().map(|r| r.1).collect();
    sketch.add_constraint(ConstraintKind::PolarPitch {
        center: pivot,
        points: centers.clone(),
        angle_rad: std::f64::consts::FRAC_PI_2,
    });
    let polar = |s: &Sketch| {
        kinds(s, |k| match k {
            ConstraintKind::PolarPitch { points, .. } => Some(points.clone()),
            _ => None,
        })
    };
    sketch.remove_geometry_cascade(&[ring[2].0]);
    assert_eq!(polar(&sketch), vec![centers[..2].to_vec()]);
    sketch.remove_geometry_cascade(&[pivot]);
    assert!(polar(&sketch).is_empty());

    // An offset keeps its other pairs; a group its other members, its
    // shape theirs, and goes once a lone point is all it holds.
    let mut sketch = Sketch::new("offset");
    let line = |s: &mut Sketch, y: f64| {
        let (a, b) = (point(s, 0.0, y), point(s, 10.0, y));
        s.add_geometry(GeometryElement::Line(Line::new(a, b)))
    };
    let [l1, o1, l2, o2] = [0.0, 2.0, 20.0, 22.0].map(|y| line(&mut sketch, y));
    sketch.add_constraint(ConstraintKind::Offset {
        pairs: vec![[l1, o1], [l2, o2]],
        distance: 2.0,
    });
    let offsets = |s: &Sketch| {
        kinds(s, |k| match k {
            ConstraintKind::Offset { pairs, .. } => Some(pairs.clone()),
            _ => None,
        })
    };
    sketch.remove_geometry_cascade(&[o2]);
    assert_eq!(offsets(&sketch), vec![vec![[l1, o1]]]);
    sketch.remove_geometry_cascade(&[o1]);
    assert!(offsets(&sketch).is_empty());

    let lone = point(&mut sketch, 5.0, 5.0);
    let (c, _) = circle(&mut sketch, 30.0, 0.0);
    sketch.add_constraint(group_of(&sketch, &[l1, c, lone]).unwrap());
    sketch.remove_geometry_cascade(&[c]);
    let alone = group_of(&sketch, &[l1, lone]).unwrap();
    let groups = |s: &Sketch| {
        kinds(s, |k| {
            matches!(k, ConstraintKind::Group { .. }).then(|| serde_json::to_value(k).unwrap())
        })
    };
    assert_eq!(groups(&sketch), vec![serde_json::to_value(alone).unwrap()]);
    sketch.remove_geometry_cascade(&[l1]);
    assert!(groups(&sketch).is_empty(), "a lone point moves as it likes");
}
