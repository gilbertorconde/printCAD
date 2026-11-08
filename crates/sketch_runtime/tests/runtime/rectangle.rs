use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint};
use sketch_runtime::binding::Binding;
use sketch_runtime::geometry::{PickElement, SourceId};
use sketch_runtime::pick::{Pick, PointRole, Snap, SnapKind, Source, SourceRef};
use sketch_runtime::point::Rejection;
use sketch_runtime::rect::{
    Applied, Effect, RectPhase, RectPlacement, RectPreview, RectTool, ToolEvent, ToolWorld,
};
use sketch_runtime::scene::{ContactTolerance, PickScene};

fn world(scene: &PickScene) -> ToolWorld<'_> {
    ToolWorld {
        scene,
        view_scale: WorldPerPx::new(0.1),
        tolerance: ContactTolerance {
            distance: WorldLength::new(1e-8),
            minimum_segment_length: WorldLength::new(1e-8),
        },
        label_offset: WorldLength::new(9.),
    }
}
fn free(x: f64, y: f64) -> Pick {
    let pos = WorldPoint::new(x, y);
    Pick {
        cursor: pos,
        pos,
        snap: None,
    }
}
pub fn run() {
    for placement in [
        RectPlacement::TwoCorners,
        RectPlacement::CenterCorner,
        RectPlacement::ThreeCorners,
        RectPlacement::CenterTwoCorners,
    ] {
        let mut scene = PickScene::default();
        scene.sync([
            PickElement::Point {
                id: SourceId(1),
                pos: WorldPoint::new(20., 30.),
            },
            PickElement::Point {
                id: SourceId(2),
                pos: WorldPoint::new(60., 30.),
            },
        ]);
        let mut tool = RectTool::default();
        tool.set_placement(placement);
        let mut first = free(20., 30.);
        first.snap = Some(Snap {
            kind: SnapKind::Endpoint,
            source: SourceRef {
                source: Source::Geometry(SourceId(1)),
                role: PointRole::Start,
            },
            source2: None,
        });
        assert!(tool.click_with_pick(&world(&scene), first).effect.is_none());
        // Solving may move a selected vertex; its identity survives and preview follows.
        scene.sync([
            PickElement::Point {
                id: SourceId(1),
                pos: WorldPoint::new(21., 30.),
            },
            PickElement::Point {
                id: SourceId(2),
                pos: WorldPoint::new(60., 30.),
            },
        ]);
        let second = if placement.rotated() {
            free(60., 30.)
        } else {
            free(60., 70.)
        };
        let last = if placement == RectPlacement::CenterTwoCorners {
            free(21., 90.)
        } else {
            free(60., 70.)
        };
        let (preview, step) = if placement.rotated() {
            assert!(
                tool.click_with_pick(&world(&scene), second)
                    .effect
                    .is_none()
            );
            (
                tool.preview_in_scene(&scene, last.clone()),
                tool.click_with_pick(&world(&scene), last),
            )
        } else {
            (
                tool.preview_in_scene(&scene, second.clone()),
                tool.click_with_pick(&world(&scene), second),
            )
        };
        let RectPreview::Rubber { corners, .. } = preview else {
            panic!("rectangle preview")
        };
        let Some(Effect::RectDraft { draft, bindings }) = step.effect else {
            panic!("rectangle plan")
        };
        assert_eq!(draft.corners(), corners);
        if placement.is_centered() {
            assert_eq!(draft.center, WorldPoint::new(21., 30.));
        } else {
            assert!(corners.contains(&WorldPoint::new(21., 30.)));
        }
        let plan = tool.outline_plan(&scene);
        assert_eq!(plan.center.is_some(), placement.is_centered());
        let sources: Vec<_> = bindings.plans().flat_map(|(_, p)| p.bindings).collect();
        assert!(matches!(sources.as_slice(), [Binding::Coincident(_)]));
        let [a, b, c, d] = corners;
        assert!((b - a).0.dot((d - a).0).abs() < 1e-8);
        assert!(a.midpoint(c).distance(b.midpoint(d)).0 < 1e-8);
        assert!((b - a).0.perp_dot((d - a).0) > 0.);
        // Unacknowledged application and an explicit refusal both keep the draft open.
        assert!(
            tool.step(&world(&scene), ToolEvent::Commit)
                .effect
                .is_none()
        );
        tool.applied(Err(Rejection::MissingSource(SourceId(1))));
        let mut retry = tool.clone();
        retry.retry_placement();
        assert!(matches!(
            retry.click_with_pick(&world(&scene), free(70., 90.)).effect,
            Some(Effect::RectDraft { .. })
        ));
        assert!(
            tool.step(&world(&scene), ToolEvent::Commit)
                .effect
                .is_none()
        );
        tool.applied(Ok(Applied {
            corners: [SourceId(10), SourceId(11), SourceId(12), SourceId(13)],
            sides: [SourceId(20), SourceId(21), SourceId(22), SourceId(23)],
            center: placement.is_centered().then_some(SourceId(30)),
        }));
        assert!(matches!(
            tool.step(&world(&scene), ToolEvent::Commit).effect,
            Some(Effect::DraftCommit)
        ));
        assert!(matches!(tool.phase, RectPhase::Idle));
        tool.begin_numeric(&world(&scene));
        assert!(matches!(tool.deactivate(), Some(Effect::DraftCancel)));
        assert!(!tool.gesture_alive());
    }
    let scene = PickScene::default();
    let mut tool = RectTool::default();
    tool.placement_input.position = Some(WorldPoint::new(0., 0.));
    tool.step(
        &world(&scene),
        ToolEvent::Click {
            cursor: WorldPoint::new(70., 90.),
        },
    );
    tool.placement_input.position = Some(WorldPoint::new(0.001, 0.002));
    assert!(matches!(
        tool.step(
            &world(&scene),
            ToolEvent::Click {
                cursor: WorldPoint::new(70., 90.)
            }
        )
        .effect,
        Some(Effect::RectDraft { .. })
    ));
    assert_eq!(tool.draft().unwrap().width(), WorldLength::new(0.001));
    assert!(tool.bindings().unwrap().sites().next().is_none());
    println!(
        "independent rectangles: 4 placements, live source, preview/geometry, directed contour, acknowledgement/refusal, numeric and cancel verified"
    );
}
