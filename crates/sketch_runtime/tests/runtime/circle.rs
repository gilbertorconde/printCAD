use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint};
use sketch_runtime::binding::Binding;
use sketch_runtime::circle::{Applied, CirclePhase, CirclePreview, CircleTool};
use sketch_runtime::geometry::{PickElement, SourceId};
use sketch_runtime::pick::{Pick, PointRole, Snap, SnapKind, Source, SourceRef};
use sketch_runtime::placement::Placement;
use sketch_runtime::point::{Event, Rejection};
use sketch_runtime::scene::{ContactTolerance, PickScene};

pub fn run() {
    let scale = WorldPerPx::new(1.);
    let source = SourceRef {
        source: Source::Geometry(SourceId(1)),
        role: PointRole::Start,
    };
    let point = |x| PickElement::Point {
        id: SourceId(1),
        pos: WorldPoint::new(x, 20.),
    };
    let mut scene = PickScene::default();
    scene.sync([point(10.)]);
    let mut tool = CircleTool::default();
    let center = Pick {
        cursor: WorldPoint::new(10., 20.),
        pos: WorldPoint::new(10., 20.),
        snap: Some(Snap {
            kind: SnapKind::Endpoint,
            source,
            source2: None,
        }),
    };
    assert!(
        tool.step(&scene, scale, Event::Click, Some(center))
            .plan
            .is_none()
    );
    scene.sync([point(15.)]);
    // Numeric placement is intentional even below one screen pixel and must
    // never inherit a nearby point's source through another object query.
    tool.placement = Placement {
        position: Some(WorldPoint::new(15.01, 20.)),
        ..Default::default()
    };
    let rim = tool.pick(
        &scene,
        WorldPoint::new(10., 20.),
        scale,
        ContactTolerance {
            distance: WorldLength::new(1e-6),
            minimum_segment_length: WorldLength::new(1e-6),
        },
    );
    assert!(rim.snap.is_none());
    let CirclePreview::Rubber { center, radius, .. } = tool.preview_with_pick(&scene, &rim) else {
        unreachable!()
    };
    assert_eq!(center, WorldPoint::new(15., 20.));
    assert!((radius.0 - 0.01).abs() < 1e-12);
    let plan = tool
        .step(&scene, scale, Event::Click, Some(rim.clone()))
        .plan
        .unwrap();
    assert_eq!(plan.radius, radius);
    assert_eq!(plan.center.bindings, [Binding::Coincident(source)]);
    assert_eq!(plan.center.pick.pos, center);
    tool.applied(Err(Rejection::MissingSource(SourceId(1))));
    assert!(matches!(tool.phase, CirclePhase::Placing { .. }));
    let retried = tool
        .step(&scene, scale, Event::Click, Some(rim))
        .plan
        .unwrap();
    assert_eq!(retried.radius, radius);
    let applied = Applied {
        circle: SourceId(8),
        center: SourceId(1),
    };
    tool.applied(Ok(applied));
    assert_eq!(tool.application, Some(Ok(applied)));
    assert!(!tool.gesture_alive());
    assert!(tool.step(&scene, scale, Event::Cancel, None).deactivate);
    println!(
        "independent circle: live centre, numeric subpixel radius, centre-only bindings, refusal/retry, host identities and cancel verified"
    );
}
