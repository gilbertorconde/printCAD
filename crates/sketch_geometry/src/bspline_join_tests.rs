use super::*;

// Independent rational Bernstein evaluation for a single Bezier span.
fn bernstein(points: &[WorldPoint], weights: &[f64], t: f64) -> WorldPoint {
    let p = points.len() - 1;
    let mut numerator = DVec2::ZERO;
    let mut denominator = 0.0;
    let mut choose = 1.0;
    for i in 0..=p {
        if i > 0 {
            choose *= (p + 1 - i) as f64 / i as f64;
        }
        let v = choose * t.powi(i as i32) * (1.0 - t).powi((p - i) as i32) * weights[i];
        numerator += points[i].0 * v;
        denominator += v;
    }
    WorldPoint::from_dvec2(numerator / denominator)
}

#[test]
fn joins_of_all_degrees_and_weight_scales_match_independent_beziers() {
    for p in 1..=5 {
        for q in 1..=5 {
            let make = |degree: usize, second: bool| {
                let start = if second { 10.0 } else { 0.0 };
                let poles = (0..=degree)
                    .map(|i| {
                        let t = i as f64 / degree as f64;
                        WorldPoint::new(
                            start + 10.0 * t,
                            if second {
                                8.0 * t * (1.0 - t)
                            } else {
                                -6.0 * t * (1.0 - t)
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                let weights = (0..=degree)
                    .map(|i| (if second { 3.7 } else { 0.23 }) * (1.0 + i as f64))
                    .collect();
                let mut knots = vec![-3.0; degree + 1];
                knots.extend(std::iter::repeat_n(7.0, degree + 1));
                BSplinePiece {
                    poles,
                    weights,
                    knots,
                    degree,
                    range: [SplineParam(-3.0), SplineParam(7.0)],
                }
            };
            let pieces = [make(p, false), make(q, true)];
            let joined = BSplineCurve::join_pieces(&pieces).unwrap();
            let curve = BSplineCurve::build_with_basis(
                SplineShape {
                    definition: SplineDefinition::Poles,
                    degree: SplineDegree::new(joined.degree as u32),
                    periodic: false,
                },
                Some(&SplineBasis {
                    knots: joined.knots,
                    weights: joined.weights,
                }),
                &joined.poles,
            )
            .unwrap();
            for (i, source) in pieces.iter().enumerate() {
                for t in (0..=100)
                    .map(|k| k as f64 / 100.0)
                    .chain([1e-9, 1.0 - 1e-9])
                {
                    let expected = bernstein(&source.poles, &source.weights, t);
                    let actual = curve.point(SplineParam(i as f64 + t));
                    assert!(
                        actual.distance(expected).0 < 1e-10,
                        "p={p} q={q} t={t}: {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    }
}
