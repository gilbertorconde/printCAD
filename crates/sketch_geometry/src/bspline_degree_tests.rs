use super::foreign::ForeignSpline;
use super::*;

pub(super) fn explicit(piece: &BSplinePiece, periodic: bool) -> BSplineCurve {
    BSplineCurve::build_with_basis(
        SplineShape {
            definition: SplineDefinition::Poles,
            degree: SplineDegree::new(piece.degree as u32),
            periodic,
        },
        Some(&SplineBasis {
            knots: piece.knots.clone(),
            weights: piece.weights.clone(),
        }),
        &piece.poles,
    )
    .unwrap()
}

pub(super) fn foreign(piece: BSplinePiece) -> ForeignSpline {
    ForeignSpline {
        degree: piece.degree,
        knots: piece.knots,
        poles: piece.poles,
        weights: piece.weights,
    }
}

#[test]
fn elevation_changes_basis_for_all_degrees_point_definitions_and_periodic_seams() {
    let pts: Vec<_> = [
        (0., 0.),
        (4., 8.),
        (9., -5.),
        (17., 9.),
        (24., -2.),
        (30., 4.),
        (40., 0.),
    ]
    .map(|(x, y)| WorldPoint::new(x, y))
    .into();
    let mut cases = 0;
    for definition in [SplineDefinition::Poles, SplineDefinition::Through] {
        for periodic in [false, true] {
            for p in 1..MAX_SPLINE_DEGREE {
                let curve = BSplineCurve::build(
                    SplineShape {
                        definition,
                        degree: SplineDegree::new(p),
                        periodic,
                    },
                    &pts,
                )
                .unwrap();
                let old = foreign(curve.piece([curve.domain().0, curve.domain().1], false));
                for q in p + 1..=MAX_SPLINE_DEGREE {
                    let piece = curve.with_degree(SplineDegree::new(q)).unwrap();
                    let raised = explicit(&piece, periodic);
                    let new = foreign(piece);
                    let (a, b) = old.domain();
                    for i in 0..=997 {
                        let t = a + (b - a) * i as f64 / 997.;
                        // de Boor in homogeneous coordinates, independent of the
                        // production Cox-de Boor basis/evaluator and Greville fit.
                        assert!(
                            old.point(t).distance(new.point(t)).0 < 1e-8,
                            "{definition:?} {periodic} {p}→{q} at {t}"
                        );
                    }
                    assert!(
                        curve
                            .sampled_deviation(&raised, [curve.domain().0, curve.domain().1])
                            .0
                            < 1e-8
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 40);
}

#[test]
fn rational_circle_survives_elevation_and_reduction_reports_real_error() {
    let shape = SplineShape {
        definition: SplineDefinition::Poles,
        degree: SplineDegree::new(2),
        periodic: false,
    };
    let curve = BSplineCurve::build_with_basis(
        shape,
        Some(&SplineBasis {
            knots: vec![2., 2., 2., 5., 5., 5.],
            weights: vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.],
        }),
        &[
            WorldPoint::new(10., 0.),
            WorldPoint::new(10., 10.),
            WorldPoint::new(0., 10.),
        ],
    )
    .unwrap();
    for degree in 3..=5 {
        let raised = explicit(
            &curve.with_degree(SplineDegree::new(degree)).unwrap(),
            false,
        );
        for p in raised.sample(131) {
            assert!((p.0.length() - 10.).abs() < 1e-9);
        }
        let reduced = explicit(&raised.with_degree(SplineDegree::new(2)).unwrap(), false);
        assert!(
            curve
                .sampled_deviation(&reduced, [SplineParam(2.), SplineParam(5.)])
                .0
                < 1e-9
        );
    }
    let line = explicit(&curve.with_degree(SplineDegree::new(1)).unwrap(), false);
    let error = curve
        .sampled_deviation(&line, [SplineParam(2.), SplineParam(5.)])
        .0;
    assert!(error > 2.9 && error < 3.0, "{error}");
    assert!(line.start().distance(curve.start()).0 < 1e-12);
    assert!(line.end().distance(curve.end()).0 < 1e-12);
}
