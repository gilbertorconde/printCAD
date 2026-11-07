use sketch_geometry::world::WorldPoint;

use crate::binding::Binding;
use crate::geometry::SourceId;
use crate::pick::{Axis, Pick, PointRole, Snap, SnapKind, Source, SourceRef};
use crate::point::{Applied, Event, PointTool, Rejection};

fn source(id: u128, role: PointRole) -> SourceRef {
    SourceRef {
        source: Source::Geometry(SourceId(id)),
        role,
    }
}

#[test]
fn selected_pick_drives_preview_plan_and_explicit_host_response() {
    let a = source(5, PointRole::Edge);
    let b = source(6, PointRole::Edge);
    let origin = SourceRef {
        source: Source::Origin,
        role: PointRole::Start,
    };
    let axis = SourceRef {
        source: Source::Axis(Axis::Horizontal),
        role: PointRole::Edge,
    };
    for (kind, at, other, bindings) in [
        (
            SnapKind::Intersection,
            a,
            Some(b),
            vec![Binding::OnCurve(SourceId(5)), Binding::OnCurve(SourceId(6))],
        ),
        (
            SnapKind::Endpoint,
            source(1, PointRole::Start),
            None,
            vec![Binding::Coincident(source(1, PointRole::Start))],
        ),
        (
            SnapKind::Center,
            source(2, PointRole::Center),
            None,
            vec![Binding::Coincident(source(2, PointRole::Center))],
        ),
        (
            SnapKind::Midpoint,
            source(3, PointRole::Mid),
            None,
            vec![Binding::Midpoint(SourceId(3))],
        ),
        (
            SnapKind::Quadrant,
            source(4, PointRole::Quadrant(3)),
            None,
            vec![Binding::Quadrant {
                circle: SourceId(4),
                quadrant: 3,
            }],
        ),
        (
            SnapKind::Endpoint,
            origin,
            None,
            vec![
                Binding::OnAxis(Axis::Horizontal),
                Binding::OnAxis(Axis::Vertical),
            ],
        ),
        (
            SnapKind::OnAxis,
            axis,
            None,
            vec![Binding::OnAxis(Axis::Horizontal)],
        ),
        (
            SnapKind::OnExtension,
            a,
            None,
            vec![Binding::OnCurve(SourceId(5))],
        ),
        (SnapKind::Parallel, a, None, vec![]),
    ] {
        let pick = Pick {
            cursor: WorldPoint::new(40.1, 50.),
            pos: WorldPoint::new(40., 50.),
            snap: Some(Snap {
                kind,
                source: at,
                source2: other,
            }),
        };
        let mut tool = PointTool::default();
        let hover = tool.step(Event::Hover, Some(pick.clone()));
        assert!(hover.plan.is_none());
        let click = tool.step(Event::Click, Some(pick.clone()));
        let plan = click.plan.unwrap();
        assert_eq!(hover.preview.unwrap(), plan.pick);
        assert_eq!(click.preview.unwrap(), pick);
        assert_eq!(plan.bindings, bindings);
        let applied = Applied {
            point: SourceId(81),
        };
        tool.applied(Ok(applied));
        assert_eq!(tool.application, Some(Ok(applied)));
        tool.applied(Err(Rejection::MissingSource(SourceId(5))));
        assert_eq!(
            tool.application,
            Some(Err(Rejection::MissingSource(SourceId(5))))
        );
        let cancel = tool.step(Event::Cancel, None);
        assert!(cancel.deactivate && cancel.plan.is_none() && cancel.preview.is_none());
    }
}

#[test]
fn explicit_position_has_no_implicit_binding() {
    let pick = Pick {
        cursor: WorldPoint::new(0., 0.),
        pos: WorldPoint::new(12.3, 4.56),
        snap: None,
    };
    let step = PointTool::default().step(Event::Click, Some(pick));
    assert!(step.plan.unwrap().bindings.is_empty());
}
