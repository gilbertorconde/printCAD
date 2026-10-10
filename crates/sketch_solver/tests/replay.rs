#![cfg(feature = "serde")]

use sketch_solver::replay::{Fixture, Regression, ReplayError, SCHEMA_VERSION, run};

#[test]
fn saved_semantic_corpus_replays_without_application_types() {
    let corpus: Vec<Regression> =
        serde_json::from_str(include_str!("../fixtures/migration.json")).unwrap();
    assert_eq!(corpus.len(), 24);
    let mut successes = 0;
    let mut invalid = 0;
    for case in corpus {
        let actual = run(&case.fixture);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&case.expected).unwrap(),
            "{}",
            case.name
        );
        match actual {
            Ok(answer) => {
                successes += 1;
                assert_eq!(answer.compiled.equations.len(), answer.residuals.len());
                assert_eq!(
                    answer.compiled.constraints.len(),
                    case.fixture.problem.constraints.len()
                );
                assert_eq!(
                    answer
                        .compiled
                        .specifications
                        .iter()
                        .map(|s| s.dim())
                        .sum::<usize>(),
                    answer.compiled.equations.len()
                );
                assert!(answer.residuals.iter().all(|r| r.value.is_finite()));
            }
            Err(ReplayError::Input(_)) => {
                invalid += 1;
            }
            Err(error) => panic!("unexpected replay error: {error}"),
        }
    }
    assert_eq!((successes, invalid), (23, 1));
}

#[test]
fn serialization_preserves_double_precision_and_large_identities() {
    let mut fixture = Fixture {
        schema_version: SCHEMA_VERSION,
        problem: Default::default(),
    };
    use sketch_solver::problem::*;
    let mut state = 0x635a_da31_8940_8271_u64;
    for i in 0..256 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let x = f64::from_bits(state & 0xffef_ffff_ffff_ffff);
        let id = PointId((u128::from(state) << 64) | i);
        fixture.problem.geometry.push(Geometry::Point(Point {
            id,
            position: Vector { x, y: -x },
        }));
    }
    let text = serde_json::to_string(&fixture).unwrap();
    let restored: Fixture = serde_json::from_str(&text).unwrap();
    assert_eq!(fixture.problem, restored.problem);
    for (a, b) in fixture
        .problem
        .geometry
        .iter()
        .zip(restored.problem.geometry)
    {
        let (Geometry::Point(a), Geometry::Point(b)) = (a, b) else {
            unreachable!()
        };
        assert_eq!(a.position.x.to_bits(), b.position.x.to_bits());
        assert_eq!(a.position.y.to_bits(), b.position.y.to_bits());
    }
}

#[test]
fn schema_and_malformed_input_are_rejected_explicitly() {
    let fixture = Fixture {
        schema_version: SCHEMA_VERSION + 1,
        problem: Default::default(),
    };
    assert!(matches!(
        run(&fixture),
        Err(ReplayError::UnsupportedSchema(2))
    ));
    assert!(serde_json::from_str::<Fixture>(r#"{"schema_version":1,"problem":false}"#).is_err());
    assert!(
        serde_json::from_str::<Fixture>(r#"{"schema_version":1,"problem":{},"unknown":0}"#)
            .is_err()
    );
    let mut corpus: Vec<Regression> =
        serde_json::from_str(include_str!("../fixtures/migration.json")).unwrap();
    let mut fixture = corpus.remove(1).fixture;
    fixture
        .problem
        .geometry
        .push(fixture.problem.geometry[0].clone());
    assert!(matches!(run(&fixture), Err(ReplayError::Input(_))));
}
