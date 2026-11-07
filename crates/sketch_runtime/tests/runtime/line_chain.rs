use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint};
use sketch_runtime::geometry::{PickElement, SourceId};
use sketch_runtime::line::{Applied, Event, LineHistoryId, LinePhase, LineTool, SegmentStart};
use sketch_runtime::pick::{Pick, PointRole, Snap, SnapKind, Source, SourceRef};
use sketch_runtime::point::Rejection;
use sketch_runtime::polyline::{self, ArcRelation, PolylineCurve, PolylineMode};
use sketch_runtime::scene::{ContactTolerance, PickScene};

/// A deliberately small host: geometry and receipts, no document or solver.
pub fn run() {
    let scale = WorldPerPx::new(0.1);
    let tolerance = ContactTolerance {
        distance: WorldLength::new(1e-6),
        minimum_segment_length: WorldLength::new(1e-6),
    };
    let free = |x, y| Pick {
        cursor: WorldPoint::new(x, y),
        pos: WorldPoint::new(x, y),
        snap: None,
    };
    for side in [-1., 1.] {
        let mut scene = PickScene::default();
        let mut input = Vec::new();
        let mut tool = LineTool::default();
        assert!(
            tool.step(&scene, scale, Event::Click, Some(free(20., 30.)))
                .plan
                .is_none()
        );
        let plan = tool
            .step(&scene, scale, Event::Click, Some(free(40., 30.)))
            .plan
            .unwrap();
        let SegmentStart::Fresh(start) = plan.start else {
            panic!("fresh first link")
        };
        assert_eq!(start.pos, WorldPoint::new(20., 30.));
        input.extend([
            PickElement::Point {
                id: SourceId(1),
                pos: start.pos,
            },
            PickElement::Point {
                id: SourceId(2),
                pos: plan.end.pick.pos,
            },
            PickElement::Segment {
                id: SourceId(3),
                a: SourceId(1),
                b: SourceId(2),
            },
        ]);
        scene.sync(input.clone());
        tool.applied(
            &scene,
            Ok(Applied {
                curve: SourceId(3),
                a: SourceId(1),
                b: SourceId(2),
            }),
            Some(LineHistoryId(100)),
        );
        assert!(tool.can_undo_in_history(LineHistoryId(100)));
        assert!(!tool.can_undo_in_history(LineHistoryId(101)));
        // A solve moved the junction, keeping its identity and the incoming line.
        input[1] = PickElement::Point {
            id: SourceId(2),
            pos: WorldPoint::new(50., 30.),
        };
        scene.sync(input.clone());
        for (i, mode) in [
            PolylineMode::Tangent,
            PolylineMode::Perpendicular,
            PolylineMode::Reverse,
        ]
        .into_iter()
        .enumerate()
        {
            tool.mode = mode;
            let pick = free(60. + i as f64 * 10., 30. + side * (10. + i as f64 * 10.));
            let preview = tool.preview_with_pick(&scene, &pick);
            let plan = tool
                .step(&scene, scale, Event::Click, Some(pick))
                .plan
                .unwrap();
            let PolylineCurve::Arc(arc) = plan.curve else {
                panic!("arc mode")
            };
            let SegmentStart::Vertex { point, pos } = plan.start else {
                panic!("shared vertex")
            };
            assert_eq!(scene.point_position(point), Some(pos));
            assert_eq!(arc.start, pos);
            assert_eq!(preview.pick(), &plan.end.pick);
            assert!(
                (arc.center.distance(arc.start).0 - arc.center.distance(arc.end).0).abs() < 1e-9
            );
            let incoming = polyline::heading_into(&scene, tool.previous_link().unwrap(), point);
            let departure = (arc.start - arc.center).0.perp().normalize()
                * if arc.clockwise { -1. } else { 1. };
            let dot = incoming.0.dot(departure);
            match mode {
                PolylineMode::Tangent => assert!((dot - 1.).abs() < 1e-10),
                PolylineMode::Reverse => assert!((dot + 1.).abs() < 1e-10),
                PolylineMode::Perpendicular => {
                    assert!(dot.abs() < 1e-10);
                    assert!(matches!(plan.relation, Some(ArcRelation::Perpendicular(_))));
                }
                _ => unreachable!(),
            }
            // An explicit refusal restores the input phase without a receipt.
            tool.applied(&scene, Err(Rejection::MissingSource(SourceId(999))), None);
            assert!(matches!(tool.phase, LinePhase::Placing { .. }));
            let retry = tool
                .step(&scene, scale, Event::Click, Some(plan.end.pick.clone()))
                .plan
                .unwrap();
            assert_eq!(retry.end, plan.end);
            let end = SourceId(10 + i as u128 * 3);
            let center = SourceId(end.0 + 1);
            let curve = SourceId(end.0 + 2);
            let a = Angle::from_vec(arc.start - arc.center);
            input.extend([
                PickElement::Point {
                    id: end,
                    pos: arc.end,
                },
                PickElement::Point {
                    id: center,
                    pos: arc.center,
                },
                PickElement::Arc {
                    id: curve,
                    center,
                    start: point,
                    end,
                    start_angle: a,
                    sweep_angle: a.sweep_to(Angle::from_vec(arc.end - arc.center), arc.clockwise),
                },
            ]);
            scene.sync(input.clone());
            tool.applied(
                &scene,
                Ok(Applied {
                    curve,
                    a: point,
                    b: end,
                }),
                Some(LineHistoryId(200 + i as u128)),
            );
        }
        // Closing requires the selected first vertex, not its coordinates alone.
        tool.mode = PolylineMode::Line;
        let mut pick = free(20., 30.);
        pick.snap = Some(Snap {
            kind: SnapKind::Endpoint,
            source: SourceRef {
                source: Source::Geometry(SourceId(1)),
                role: PointRole::Edge,
            },
            source2: None,
        });
        let plan = tool
            .step(&scene, scale, Event::Click, Some(pick))
            .plan
            .unwrap();
        assert_eq!(plan.close, Some(SourceId(1)));
        tool.applied(&scene, Err(Rejection::Refused), None);
        let preview = tool.pick(&scene, WorldPoint::new(80., 35.), scale, tolerance);
        assert!(preview.pos.0.is_finite());
        let count = input.len();
        assert!(tool.step(&scene, scale, Event::Cancel, None).deactivate);
        assert!(!tool.gesture_alive());
        assert_eq!(input.len(), count);
    }
    println!(
        "independent mixed chain: 2 sides, 6 arc plans/retries, live junctions, closure, refusal, receipts, cancel"
    );
}
