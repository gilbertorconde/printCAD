use super::degree_tests::{explicit, foreign};
use super::*;

#[test]
fn knot_refinement_and_its_removal_keep_all_definitions_and_degrees() {
    let points = [
        (0., 0.),
        (4., 8.),
        (9., -5.),
        (17., 9.),
        (24., -2.),
        (30., 4.),
        (40., 0.),
    ]
    .map(|(x, y)| WorldPoint::new(x, y));
    let mut cases = 0;
    for definition in [SplineDefinition::Poles, SplineDefinition::Through] {
        for periodic in [false, true] {
            for p in 1..=MAX_SPLINE_DEGREE {
                let curve = BSplineCurve::build(
                    SplineShape {
                        definition,
                        degree: SplineDegree::new(p),
                        periodic,
                    },
                    &points,
                )
                .unwrap();
                let source = foreign(curve.piece([curve.domain().0, curve.domain().1], false));
                let (a, b) = curve.domain();
                let at = SplineParam(a.0 + (b.0 - a.0) * 0.431);
                let refined = explicit(
                    &curve
                        .with_knot_multiplicity(at, SplineMultiplicity::new(p))
                        .unwrap(),
                    periodic,
                );
                assert_eq!(
                    refined
                        .interior_knots()
                        .iter()
                        .find(|(k, _)| *k == at)
                        .unwrap()
                        .1
                        .get(),
                    p
                );
                let restored = explicit(
                    &refined
                        .with_knot_multiplicity(at, SplineMultiplicity::new(0))
                        .unwrap(),
                    periodic,
                );
                for made in [refined, restored] {
                    let target = foreign(made.piece([a, b], false));
                    // Independent homogeneous de Boor evaluation checks both
                    // exact insertion and fitting away a redundant knot.
                    for i in 0..=997 {
                        let t = a.0 + (b.0 - a.0) * i as f64 / 997.;
                        assert!(
                            source.point(t).distance(target.point(t)).0 < 1e-8,
                            "{definition:?} periodic={periodic} degree={p} t={t}"
                        );
                    }
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 20);
}

#[test]
fn rational_nonunit_domain_and_nonredundant_removal_have_geometric_oracles() {
    let circle = BSplineCurve::build_with_basis(
        SplineShape {
            definition: SplineDefinition::Poles,
            degree: SplineDegree::new(2),
            periodic: false,
        },
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
    let refined = explicit(
        &circle
            .with_knot_multiplicity(SplineParam(3.), SplineMultiplicity::new(2))
            .unwrap(),
        false,
    );
    for p in refined.sample(71) {
        assert!((p.0.length() - 10.).abs() < 1e-9);
    }
    let reduced = explicit(
        &refined
            .with_knot_multiplicity(SplineParam(3.), SplineMultiplicity::new(1))
            .unwrap(),
        false,
    );
    assert!(
        circle
            .sampled_deviation(&reduced, [SplineParam(2.), SplineParam(5.)])
            .0
            < 1e-9
    );
    for at in [f64::NAN, f64::INFINITY, 1., 2., 5., 6.] {
        assert!(
            circle
                .with_knot_multiplicity(SplineParam(at), SplineMultiplicity::new(1))
                .is_err()
        );
    }
    assert!(
        circle
            .with_knot_multiplicity(SplineParam(3.), SplineMultiplicity::new(3))
            .is_err()
    );
    assert!(
        circle
            .with_knot_multiplicity(SplineParam(3.), SplineMultiplicity::new(0))
            .is_err()
    );
    // A V made of two straight spans cannot become a single straight span
    // without losing its peak. This removal's exact maximum error is 10 mm.
    let vee = BSplineCurve::build(
        SplineShape {
            definition: SplineDefinition::Poles,
            degree: SplineDegree::new(1),
            periodic: false,
        },
        &[
            WorldPoint::new(0., 0.),
            WorldPoint::new(10., 10.),
            WorldPoint::new(20., 0.),
        ],
    )
    .unwrap();
    let line = explicit(
        &vee.with_knot_multiplicity(SplineParam(0.5), SplineMultiplicity::new(0))
            .unwrap(),
        false,
    );
    assert!(
        (vee.sampled_deviation(&line, [SplineParam(0.), SplineParam(1.)])
            .0
            - 10.)
            .abs()
            < 1e-10
    );
    for i in 0..=71 {
        let t = i as f64 / 71.;
        assert!(
            line.point(SplineParam(t))
                .distance(WorldPoint::new(20. * t, 0.))
                .0
                < 1e-12
        );
    }
}
