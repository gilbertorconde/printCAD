use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint};
use sketch_runtime::binding::Binding;
use sketch_runtime::conic::{Applied, ConicPreview, ConicTool};
use sketch_runtime::conic_plan::{
    ConicGeometry, ConicKind, ConicPointRole as Role, HyperbolaInput,
};
use sketch_runtime::geometry::{PickElement, SourceId};
use sketch_runtime::pick::Pick;
use sketch_runtime::point::{Event, Rejection};
use sketch_runtime::scene::{ContactTolerance, PickScene};

pub fn run() {
    let at = WorldPoint::new;
    let contact = ContactTolerance {
        distance: WorldLength::new(1e-8),
        minimum_segment_length: WorldLength::new(1e-8),
    };
    let scale = WorldPerPx::new(0.01);
    let cases = [
        (
            ConicKind::Arc3Pt,
            vec![at(20., 30.), at(60., 30.), at(40., 50.)],
        ),
        (
            ConicKind::ArcFromCenter,
            vec![at(20., 30.), at(60., 30.), at(30., 60.)],
        ),
        (
            ConicKind::CircleFrom3Pts,
            vec![at(20., 30.), at(60., 30.), at(40., 50.)],
        ),
        (
            ConicKind::FullEllipse,
            vec![at(20., 30.), at(60., 30.), at(30., 50.)],
        ),
        (
            ConicKind::EllipseFrom3Pts,
            vec![at(20., 30.), at(60., 30.), at(40., 50.)],
        ),
        (
            ConicKind::ArcOfEllipse,
            vec![at(20., 30.), at(60., 30.), at(30., 50.), at(0., 20.)],
        ),
        (
            ConicKind::ArcOfHyperbola,
            vec![at(20., 30.), at(60., 30.), at(50., 50.), at(60., 10.)],
        ),
        (
            ConicKind::ArcOfParabola,
            vec![at(20., 30.), at(40., 30.), at(50., 50.), at(60., 10.)],
        ),
    ];
    for (kind, positions) in cases {
        let mut scene = PickScene::default();
        let mut tool = ConicTool::new(kind);
        for (i, pos) in positions.iter().copied().enumerate() {
            let pick = Pick {
                cursor: pos,
                pos,
                snap: None,
            };
            let preview = tool.preview_with_pick(&scene, pick.clone());
            let step = tool.step(&scene, scale, Event::Click, Some(pick.clone()));
            if i + 1 < positions.len() {
                assert!(step.plan.is_none());
                continue;
            }
            let plan = step.plan.unwrap();
            let ConicPreview::Curve { plan: drawn, .. } = preview else {
                panic!("curve preview")
            };
            assert_eq!(drawn, plan.curve);
            for role in [Role::Anchor, Role::Start, Role::End] {
                if role != Role::Anchor
                    && matches!(plan.curve.geometry, ConicGeometry::Circle { .. })
                {
                    continue;
                }
                let point = plan.point(role);
                if !plan.curve.bindings.iter().any(|b| b.role == role) {
                    assert!(point.bindings.is_empty());
                }
            }
            let saved = plan.clone();
            tool.applied(Err(Rejection::Refused));
            assert_eq!(tool.collected(), positions.len() - 1);
            let retry = tool
                .step(&scene, scale, Event::Click, Some(pick))
                .plan
                .unwrap();
            assert_eq!(retry, saved);
            let made = Applied {
                curve: SourceId(100),
                points: vec![(Role::Anchor, SourceId(101))],
            };
            tool.applied(Ok(made.clone()));
            assert_eq!(tool.application, Some(Ok(made)));
            assert!(!tool.gesture_alive());
            let step = tool.step(&scene, scale, Event::Cancel, None);
            assert!(step.deactivate && step.plan.is_none());
            scene.sync([]);
        }
    }
    // A live chosen vertex follows host movement without stealing a new source.
    let mut scene = PickScene::default();
    scene.sync([PickElement::Point {
        id: SourceId(1),
        pos: at(20., 30.),
    }]);
    let mut tool = ConicTool::new(ConicKind::Arc3Pt);
    let first = tool.pick(&scene, at(20.01, 30.), scale, contact);
    tool.step(&scene, scale, Event::Click, Some(first));
    scene.sync([PickElement::Point {
        id: SourceId(1),
        pos: at(21., 30.),
    }]);
    for pos in [at(60., 30.), at(40., 50.)] {
        let pick = tool.pick(&scene, pos, scale, contact);
        let step = tool.step(&scene, scale, Event::Click, Some(pick));
        if let Some(plan) = step.plan {
            assert_eq!(plan.point(Role::Start).pick.pos, at(21., 30.));
            assert!(matches!(
                plan.point(Role::Start).bindings.as_slice(),
                [Binding::Coincident(_)]
            ));
            assert!(plan.point(Role::Anchor).bindings.is_empty());
        }
    }
    tool.deactivate();
    // Numeric placements bypass repeated object snapping and pointer jitter.
    let mut tool = ConicTool::new(ConicKind::Arc3Pt);
    let mut numeric_plan = None;
    for pos in [at(20., 30.), at(20.001, 30.), at(20.0005, 30.001)] {
        tool.placement.position = Some(pos);
        let pick = tool.pick(&scene, at(100., 100.), WorldPerPx::new(1.), contact);
        assert!(pick.snap.is_none());
        let step = tool.step(&scene, WorldPerPx::new(1.), Event::Click, Some(pick));
        if let Some(plan) = step.plan {
            assert!(
                plan.curve
                    .bindings
                    .iter()
                    .all(|b| plan.point(b.role).bindings.is_empty())
            );
            numeric_plan = Some(plan);
        }
    }
    assert!(numeric_plan.is_some(), "subpixel numeric arc must complete");
    assert!(tool.gesture_alive());
    tool.deactivate();
    // Branch-vertex and focus inputs use the same focal constructor.
    let mut tool = ConicTool::new(ConicKind::ArcOfHyperbola);
    tool.hyperbola_input = HyperbolaInput::BranchVertex;
    let mut vertex_plan = None;
    for pos in [at(20., 30.), at(40., 30.), at(50., 50.), at(60., 10.)] {
        let step = tool.step(
            &scene,
            scale,
            Event::Click,
            Some(Pick {
                cursor: pos,
                pos,
                snap: None,
            }),
        );
        if let Some(plan) = step.plan {
            assert!(!plan.curve.bindings.iter().any(|b| b.role == Role::Focus));
            vertex_plan = Some(plan);
        }
    }
    assert!(vertex_plan.is_some(), "branch vertex gesture must complete");
    println!(
        "independent conics: 8 gestures, preview/commit, refusal/retry, identities, live vertex, numeric placement, focus/vertex and cancel verified"
    );
}
