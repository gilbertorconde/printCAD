use sketch_geometry::analytic_spline::{conic, elliptic};
use sketch_geometry::bspline::{
    BSplineCurve, SplineBasis, SplineDefinition, SplineDegree, SplineShape,
};
use sketch_geometry::bspline_window::SplineWindow;
use sketch_geometry::conic::ConicArcParams;
use sketch_geometry::world::{WorldLength, WorldOffset, WorldPerPx, WorldPoint, WorldVec};
use sketch_runtime::binding::Binding;
use sketch_runtime::geometry::{PickElement, SourceId, SplineSpan};
use sketch_runtime::hover::{self, HoverGeom, HoverObj};
use sketch_runtime::pick::{Pick, PointRole, Snap, SnapKind, Source, SourceRef};
use sketch_runtime::point::{Applied, Event, PointTool, Rejection};
use sketch_runtime::scene::{ContactTolerance, PickScene};
use sketch_runtime::snap::{self, SnapGeom, SnapObj};

mod circle;
mod conic;
mod line_chain;
mod rectangle;

#[test]
fn independent_runtime() {
    line_chain::run();
    circle::run();
    conic::run();
    rectangle::run();
    geometric_scene();
    exact_carriers();
    snap_projections();
    hover_targets();
    // Two host carriers x = 40 and y = 50; the selected intersection is exact.
    let source = |id| SourceRef {
        source: Source::Geometry(SourceId(id)),
        role: PointRole::Edge,
    };
    let mut objects = [
        SnapObj {
            geom: SnapGeom::Segment(WorldPoint::new(40., 20.), WorldPoint::new(40., 80.)),
            source: source(5),
        },
        SnapObj {
            geom: SnapGeom::Segment(WorldPoint::new(10., 50.), WorldPoint::new(70., 50.)),
            source: source(6),
        },
    ];
    let mut selected = None;
    for scale in [0.01, 1., 100.] {
        let tolerance = snap::snap_tolerance(WorldPerPx::new(scale));
        let cursor = WorldPoint::new(40. + tolerance.0 * 0.1, 50. + tolerance.0 * 0.2);
        for _ in 0..2 {
            let result = snap::snap_at(cursor, &objects, tolerance).unwrap();
            assert_eq!(result.snapped_pos, WorldPoint::new(40., 50.));
            assert_eq!(result.kind, SnapKind::Intersection);
            assert_eq!(
                (result.source, result.source2),
                (source(5), Some(source(6)))
            );
            assert!(result.is_excluded(|s| s == source(5)));
            assert!(result.is_excluded(|s| s == source(6)));
            assert!(!result.is_excluded(|s| s == source(7)));
            selected = Some(Pick {
                cursor,
                pos: result.snapped_pos,
                snap: Some(Snap {
                    kind: result.kind,
                    source: result.source,
                    source2: result.source2,
                }),
            });
            objects.reverse();
        }
    }
    let pick = selected.unwrap();
    let mut tool = PointTool::default();
    let preview = tool.step(Event::Hover, Some(pick.clone())).preview.unwrap();
    let plan = tool.step(Event::Click, Some(pick)).plan.unwrap();
    assert_eq!(preview, plan.pick);
    assert_eq!(
        plan.bindings,
        [Binding::OnCurve(SourceId(5)), Binding::OnCurve(SourceId(6))]
    );
    // This consumer's storage and identity are entirely host-owned.
    let points = [(SourceId(17), plan.pick.pos)];
    tool.applied(Ok(Applied { point: points[0].0 }));
    assert_eq!(tool.application.unwrap().unwrap().point, points[0].0);
    assert_eq!((points[0].1.0.x, points[0].1.0.y), (40., 50.));
    tool.applied(Err(Rejection::MissingSource(SourceId(6))));
    assert_eq!(
        tool.application,
        Some(Err(Rejection::MissingSource(SourceId(6))))
    );
    assert_eq!(points.len(), 1);
    println!(
        "independent runtime: raw intersection at 3 scales and 2 orders, exclusions, preview, two bindings, host identity and refusal verified"
    );
}

fn geometric_scene() {
    let contact = ContactTolerance {
        distance: WorldLength::new(1e-8),
        minimum_segment_length: WorldLength::new(1e-8),
    };
    let point = |id, x, y| PickElement::Point {
        id: SourceId(id),
        pos: WorldPoint::new(x, y),
    };
    let edge = |id| SourceRef {
        source: Source::Geometry(SourceId(id)),
        role: PointRole::Edge,
    };
    let mut input = vec![
        point(1, 40., 20.),
        point(2, 40., 80.),
        point(3, 10., 50.),
        point(4, 70., 50.),
        PickElement::Segment {
            id: SourceId(5),
            a: SourceId(1),
            b: SourceId(2),
        },
        PickElement::Segment {
            id: SourceId(6),
            a: SourceId(3),
            b: SourceId(4),
        },
    ];
    let mut scene = PickScene::default();
    for shift in [0., 9., -3.] {
        for element in &mut input {
            if let PickElement::Point { id, pos } = element
                && (*id == SourceId(1) || *id == SourceId(2))
            {
                pos.0.x = 40. + shift;
            }
        }
        for _ in 0..2 {
            scene.sync(input.clone());
            let hit = scene
                .snap_at(
                    WorldPoint::new(40. + shift + 0.1, 50.1),
                    WorldPerPx::new(0.05),
                    contact,
                )
                .unwrap();
            assert_eq!(
                (hit.source, hit.source2, hit.kind),
                (edge(5), Some(edge(6)), SnapKind::Intersection)
            );
            assert_eq!(hit.snapped_pos, WorldPoint::new(40. + shift, 50.));
            assert!(hit.is_excluded(|s| s == edge(6)));
            input.reverse();
        }
        scene.sync([]);
        assert!(
            scene
                .snap_at(
                    WorldPoint::new(40. + shift, 50.),
                    WorldPerPx::new(0.05),
                    contact
                )
                .is_none()
        );
    }
    // The same geometric vertex has one host address across both edge roles.
    scene.sync([
        point(1, 20., 20.),
        point(2, 40., 20.),
        point(3, 40., 40.),
        PickElement::Segment {
            id: SourceId(5),
            a: SourceId(1),
            b: SourceId(2),
        },
        PickElement::Segment {
            id: SourceId(6),
            a: SourceId(2),
            b: SourceId(3),
        },
    ]);
    for (id, role) in [(5, PointRole::End), (6, PointRole::Start)] {
        assert_eq!(
            scene.point_source(SourceRef { role, ..edge(id) }),
            Some(SourceId(2))
        );
    }
    // Older selected-point records can carry a Start role on the point itself.
    assert_eq!(
        scene.point_source(SourceRef {
            source: Source::Geometry(SourceId(2)),
            role: PointRole::Start
        }),
        Some(SourceId(2))
    );
    // Exact quarter ellipse: the middle pole is outside the visible carrier.
    // It never becomes a point target; both carriers of its contact survive.
    let q = (0.5_f64).sqrt();
    scene.sync([
        point(1, 50., 50.),
        point(2, 58., 50.),
        point(3, 50., 56.),
        point(4, 50. + 8. * q, 48.),
        point(5, 50. + 8. * q, 58.),
        PickElement::RationalCurve {
            id: SourceId(7),
            poles: vec![
                WorldPoint::new(58., 50.),
                WorldPoint::new(58., 56.),
                WorldPoint::new(50., 56.),
            ],
            basis: SplineBasis {
                knots: vec![0., 0., 0., 1., 1., 1.],
                weights: vec![1., q, 1.],
            },
            periodic: false,
            endpoints: Some([SourceId(2), SourceId(3)]),
            center: Some(SourceId(1)),
        },
        PickElement::Segment {
            id: SourceId(8),
            a: SourceId(4),
            b: SourceId(5),
        },
    ]);
    let cursor = WorldPoint::new(50. + 8. * q + 0.01, 50. + 6. * q + 0.01);
    let hit = scene
        .snap_at(cursor, WorldPerPx::new(0.005), contact)
        .unwrap();
    assert_eq!(
        (hit.source, hit.source2, hit.kind),
        (edge(7), Some(edge(8)), SnapKind::Intersection)
    );
    assert!(
        hit.snapped_pos
            .distance(WorldPoint::new(50. + 8. * q, 50. + 6. * q))
            .0
            < 1e-8
    );
    let mut point_tool = PointTool::default();
    let pick = point_tool.pick(
        &scene,
        cursor,
        WorldPerPx::new(0.005),
        contact,
        Default::default(),
    );
    assert_eq!(pick.pos, hit.snapped_pos);
    let plan = point_tool.step(Event::Click, Some(pick)).plan.unwrap();
    assert_eq!(
        plan.bindings,
        [Binding::OnCurve(SourceId(7)), Binding::OnCurve(SourceId(8))]
    );
    point_tool.applied(Ok(Applied {
        point: SourceId(24),
    }));
    assert_eq!(
        point_tool.application,
        Some(Ok(Applied {
            point: SourceId(24)
        }))
    );
    let private = WorldPoint::new(58., 56.);
    assert!(
        scene
            .snap_at(private, WorldPerPx::new(0.005), contact)
            .is_none()
    );
    assert!(scene.hover_at(private, WorldPerPx::new(0.005)).is_none());
    assert_eq!(
        scene.point_source(SourceRef {
            role: PointRole::Start,
            ..edge(7)
        }),
        Some(SourceId(2))
    );
    // A visible window keeps its real endpoints; carrier poles outside that
    // window remain editable model points, but do not enlarge the curve target.
    let shape = SplineShape {
        definition: SplineDefinition::Poles,
        degree: SplineDegree::new(2),
        periodic: false,
    };
    let poles = [
        WorldPoint::new(20., 20.),
        WorldPoint::new(30., 40.),
        WorldPoint::new(40., 20.),
    ];
    let curve = BSplineCurve::build(shape, &poles).unwrap();
    let params = [
        sketch_geometry::bspline::SplineParam(0.25),
        sketch_geometry::bspline::SplineParam(0.75),
    ];
    let ends = params.map(|t| curve.point(t));
    scene.sync([
        point(1, 20., 20.),
        point(2, 30., 40.),
        point(3, 40., 20.),
        point(4, ends[0].0.x, ends[0].0.y),
        point(5, ends[1].0.x, ends[1].0.y),
        PickElement::BSpline {
            id: SourceId(9),
            poles: vec![SourceId(1), SourceId(2), SourceId(3)],
            shape,
            basis: None,
            span: Some(SplineSpan {
                endpoints: [SourceId(4), SourceId(5)],
                parameters: params,
            }),
        },
    ]);
    let hidden = curve.point(sketch_geometry::bspline::SplineParam(0.1));
    assert!(scene.hover_at(hidden, WorldPerPx::new(0.001)).is_none());
    assert_eq!(
        scene.point_source(SourceRef {
            role: PointRole::Start,
            ..edge(9)
        }),
        Some(SourceId(4))
    );
    let placement = sketch_runtime::placement::Placement {
        position: Some(WorldPoint::new(12.3, 4.56)),
        grid: Some(sketch_runtime::placement::GridSnap::new(WorldLength::new(
            10.,
        ))),
        ..Default::default()
    };
    let typed = placement
        .resolve_with::<SourceRef>(cursor, |_| panic!("a typed point must not query objects"));
    assert_eq!(typed.pos, WorldPoint::new(12.3, 4.56));
    assert!(typed.snap.is_none());
    println!(
        "independent scene: incremental move/reorder/delete/restore, shared vertex, private rational poles, two-carrier contact, host application, numeric placement and exact trimmed window verified"
    );
}

fn hover_targets() {
    use sketch_runtime::pick::Axis;
    let model_point = Source::Geometry(SourceId(1));
    let curve = Source::Geometry(SourceId(2));
    let mut objects = vec![
        HoverObj {
            geo: model_point,
            geom: HoverGeom::Point(WorldPoint::new(1.1, 0.)),
        },
        HoverObj {
            geo: curve,
            geom: HoverGeom::Circle {
                center: WorldPoint::new(4., 0.),
                radius: WorldLength::new(3.),
            },
        },
        HoverObj {
            geo: Source::Origin,
            geom: HoverGeom::Origin,
        },
        HoverObj {
            geo: Source::Axis(Axis::Horizontal),
            geom: HoverGeom::HAxis,
        },
        HoverObj {
            geo: Source::Axis(Axis::Vertical),
            geom: HoverGeom::VAxis,
        },
    ];
    let tolerance = hover::hover_tolerance(WorldPerPx::new(1.));
    assert_eq!(tolerance, WorldLength::new(14.));
    for expected in [
        model_point,
        Source::Origin,
        curve,
        Source::Axis(Axis::Horizontal),
        Source::Axis(Axis::Vertical),
    ] {
        for _ in 0..2 {
            let ranked = hover::rank_hover(&objects, WorldPoint::ORIGIN, tolerance);
            assert_eq!(ranked[0].geo, expected);
            assert_eq!(
                hover::hover_at(&objects, WorldPoint::ORIGIN, tolerance),
                Some(expected)
            );
            objects.reverse();
        }
        objects.retain(|object| object.geo != expected);
    }
    assert!(hover::hover_at(&objects, WorldPoint::ORIGIN, tolerance).is_none());
    println!(
        "independent hover: model point, origin, curve and axes priorities at 2 orders verified"
    );
}

fn snap_projections() {
    use sketch_geometry::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
    let center = WorldPoint::new(20., 30.);
    let (poles, basis) = elliptic(
        center,
        WorldVec::new(5., 0.),
        WorldVec::new(0., 5.),
        0.,
        std::f64::consts::TAU,
    )
    .unwrap();
    let cases = [
        (SnapGeom::Point(center), center),
        (
            SnapGeom::Segment(center, WorldPoint::new(30., 30.)),
            WorldPoint::new(25., 30.),
        ),
        (
            SnapGeom::Line(center, WorldPoint::new(21., 30.)),
            WorldPoint::new(25., 30.),
        ),
        (
            SnapGeom::Circle {
                center,
                radius: WorldLength::new(5.),
            },
            WorldPoint::new(25., 30.),
        ),
        (
            SnapGeom::Arc {
                center,
                radius: WorldLength::new(5.),
                start_angle: Angle(450_f64.to_radians()),
                sweep_angle: Angle(270_f64.to_radians()),
            },
            WorldPoint::new(20., 25.),
        ),
        (
            SnapGeom::ArcOfEllipse {
                center,
                focus1: WorldPoint::new(23., 30.),
                semi_minor: WorldLength::new(4.),
                t_start: Angle(0.),
                t_end: Angle(std::f64::consts::TAU),
                clockwise: false,
            },
            WorldPoint::new(25., 30.),
        ),
        (
            SnapGeom::ArcOfHyperbola {
                center,
                focus1: WorldPoint::new(25., 30.),
                semi_minor: WorldLength::new(4.),
                t_start: HyperbolaParam(-1.),
                t_end: HyperbolaParam(1.),
                branch: HyperbolaBranch::Positive,
            },
            WorldPoint::new(23., 30.),
        ),
        (
            SnapGeom::ArcOfHyperbola {
                center,
                focus1: WorldPoint::new(25., 30.),
                semi_minor: WorldLength::new(4.),
                t_start: HyperbolaParam(-1.),
                t_end: HyperbolaParam(1.),
                branch: HyperbolaBranch::Negative,
            },
            WorldPoint::new(17., 30.),
        ),
        (
            SnapGeom::ArcOfParabola {
                vertex: center,
                focus: WorldPoint::new(22., 30.),
                u_start: ParabolaParam(-5.),
                u_end: ParabolaParam(5.),
            },
            WorldPoint::new(22., 34.),
        ),
        (
            SnapGeom::BSpline(carrier(poles, basis, true)),
            WorldPoint::new(25., 30.),
        ),
    ];
    let source = SourceRef {
        source: Source::Geometry(SourceId(42)),
        role: PointRole::Edge,
    };
    let count = cases.len();
    for (geom, expected) in cases {
        let objects = [SnapObj { geom, source }];
        let result = snap::find_snap(expected, &objects, WorldLength::new(0.1)).unwrap();
        assert!(
            result.snapped_pos.distance(expected).0 < 1e-7,
            "{:?}",
            objects[0]
        );
        assert_eq!(result.source, source);
        assert!(result.source2.is_none());
    }
    let objects = [SnapObj {
        geom: SnapGeom::Point(center),
        source,
    }];
    assert!(
        snap::find_snap(
            center + WorldVec::new(1., 0.),
            &objects,
            WorldLength::new(1.)
        )
        .is_none()
    );
    assert!(
        snap::find_snap(
            center + WorldVec::new(1. - 1e-8, 0.),
            &objects,
            WorldLength::new(1.)
        )
        .is_some()
    );
    println!(
        "independent snap: {count} analytic/rational projections and strict tolerance boundary verified"
    );
}

fn carrier(poles: Vec<WorldPoint>, basis: SplineBasis, periodic: bool) -> SplineWindow {
    SplineWindow::whole(
        BSplineCurve::build_with_basis(
            SplineShape {
                definition: SplineDefinition::Poles,
                degree: SplineDegree::new(2),
                periodic,
            },
            Some(&basis),
            &poles,
        )
        .unwrap(),
    )
}

/// Geometric equations and travel, without document or solver access.
fn exact_carriers() {
    use std::f64::consts::{PI, TAU};
    let center = WorldPoint::new(20., 30.);
    let mut spans = 0;
    for degrees in [180.0_f64, 270., 360., 450., 720., -450.] {
        for epsilon in [-1e-7, 0., 1e-7] {
            for sign in [-1., 1.] {
                let sweep = (degrees + epsilon).to_radians() * sign;
                let (poles, basis) = elliptic(
                    center,
                    WorldVec::new(5., 0.),
                    WorldVec::new(0., 5.),
                    PI * 2.5,
                    sweep,
                )
                .unwrap();
                let window = carrier(poles, basis, false);
                assert!((window.length().0 - 5. * sweep.abs()).abs() < 1e-7);
                for (_, p) in window.samples() {
                    assert!((p.distance(center).0 - 5.).abs() < 1e-10);
                    let t = window.nearest(p).0;
                    assert!((p - center).0.perp_dot(window.curve().derivative(t).0) * sweep > 0.);
                }
                spans += 1;
            }
        }
    }
    assert!(elliptic(center, WorldVec::new(5., 0.), WorldVec::new(0., 5.), 0., 0.).is_err());
    let (poles, basis) = elliptic(
        center,
        WorldVec::new(5., 0.),
        WorldVec::new(0., 5.),
        0.,
        TAU,
    )
    .unwrap();
    let closed = carrier(poles, basis, true);
    let seam = [
        closed.param_at(WorldOffset::new(closed.length().0 * 0.75)),
        closed.param_at(WorldOffset::new(closed.length().0 * 1.25)),
    ];
    let window = SplineWindow::on_window(closed.curve().clone(), seam);
    assert!((window.length().0 - 5. * PI).abs() < 1e-8);
    assert!(
        window
            .point(window.nearest(WorldPoint::new(25.1, 30.)).0)
            .distance(WorldPoint::new(25., 30.))
            .0
            < 1e-8
    );

    for branch in [-1., 1.] {
        for reverse in [false, true] {
            let at = |t: f64| WorldPoint::new(20. + branch * 3. * t.cosh(), 30. + 4. * t.sinh());
            let [a, b] = if reverse { [1.1, -0.7] } else { [-0.7, 1.1] };
            let params =
                ConicArcParams::hyperbola(center, WorldPoint::new(25., 30.), at(a), at(b)).unwrap();
            let (poles, basis) = conic(params, at(a), at(b)).unwrap();
            let window = carrier(poles, basis, false);
            assert!(window.point(window.window()[0]).distance(at(a)).0 < 1e-10);
            assert!(window.point(window.window()[1]).distance(at(b)).0 < 1e-10);
            for (_, p) in window.samples() {
                let q = p - center;
                assert!((q.0.x.powi(2) / 9. - q.0.y.powi(2) / 16. - 1.).abs() < 1e-10);
                assert!(q.0.x * branch > 0.);
                assert!(window.curve().derivative(window.nearest(p).0).0.y * (b - a) > 0.);
            }
        }
    }
    for [a, b] in [[-4., 6.], [6., -4.]] {
        let at = |u: f64| WorldPoint::new(20. + u * u / 8., 30. + u);
        let params = ConicArcParams::parabola(center, WorldPoint::new(22., 30.), at(a), at(b));
        let (poles, basis) = conic(params, at(a), at(b)).unwrap();
        let window = carrier(poles, basis, false);
        for (_, p) in window.samples() {
            let q = p - center;
            assert!((q.0.y.powi(2) - 8. * q.0.x).abs() < 1e-10);
        }
    }
    println!(
        "independent geometry: {spans} directed spans, seam window, both hyperbola branches and parabola verified"
    );
}
