//! Selected sources in drawing commands. A source made by an earlier click
//! of the same command is addressed by its creation order, not its run's UUID.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::sketch::{GeometryElement, ORIGIN_ID, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID};
use crate::snap::{Snap, SnapKind, SnapTarget};

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct CreatedElement(usize);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Source {
    Id(Uuid),
    Created(CreatedElement),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedSnap {
    pos: Vec2D,
    kind: Option<SnapKind>,
    point: Option<Source>,
    curve: Option<Source>,
    curve2: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on: Option<SnapKind>,
}

fn created(sketch: &Sketch, before: &HashSet<Uuid>) -> Vec<Uuid> {
    sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !before.contains(id))
        .collect()
}

pub(crate) fn record(pick: Snap, sketch: &Sketch, before: &HashSet<Uuid>) -> Value {
    let created = created(sketch, before);
    let source = |id| match created.iter().position(|made| *made == id) {
        Some(index) => Source::Created(CreatedElement(index)),
        None => Source::Id(id),
    };
    serde_json::to_value(RecordedSnap {
        pos: pick.pos,
        kind: pick.kind,
        point: match pick.target {
            SnapTarget::Existing(id) => Some(source(id)),
            _ => None,
        },
        curve: pick.curve.map(source),
        curve2: pick.curve2.map(source),
        on: pick.on,
    })
    .unwrap()
}

pub(crate) fn read(value: &Value, sketch: &Sketch, before: &HashSet<Uuid>) -> Result<Snap, String> {
    let recorded: RecordedSnap =
        serde_json::from_value(value.clone()).map_err(|e| format!("snap: {e}"))?;
    if !recorded.pos.x.is_finite() || !recorded.pos.y.is_finite() {
        return Err("snap position must be finite".into());
    }
    let created = created(sketch, before);
    let source = |s| match s {
        Source::Id(id) => Ok(id),
        Source::Created(CreatedElement(index)) => created
            .get(index)
            .copied()
            .ok_or_else(|| format!("snap source {index} has not been created")),
    };
    let point = recorded.point.map(source).transpose()?;
    let curve = recorded.curve.map(source).transpose()?;
    let curve2 = recorded.curve2.map(source).transpose()?;
    let is_line = |id| {
        id == X_AXIS_ID
            || id == Y_AXIS_ID
            || matches!(sketch.get_geometry(id), Some(GeometryElement::Line(_)))
    };
    let is_curve = |id| {
        is_line(id)
            || sketch
                .get_geometry(id)
                .is_some_and(|g| !matches!(g, GeometryElement::Point(_)))
    };
    let valid = if let Some(on) = recorded.on {
        let direction = match recorded.kind {
            Some(SnapKind::Horizontal | SnapKind::Vertical) => curve.is_none(),
            Some(SnapKind::Parallel | SnapKind::PerpendicularTo) => curve.is_some_and(is_line),
            _ => false,
        };
        let landing = match on {
            SnapKind::OnAxis => curve2.is_some_and(|id| id == X_AXIS_ID || id == Y_AXIS_ID),
            SnapKind::OnCurve => {
                curve2.is_some_and(|id| is_curve(id) && id != X_AXIS_ID && id != Y_AXIS_ID)
            }
            _ => false,
        };
        point.is_none() && direction && landing && curve != curve2
    } else {
        match recorded.kind {
            None | Some(SnapKind::Horizontal | SnapKind::Vertical) => {
                point.is_none() && curve.is_none() && curve2.is_none()
            }
            Some(SnapKind::Endpoint | SnapKind::Center) => {
                point.is_some_and(|id| sketch.point_position(id).is_some())
                    && curve.is_none()
                    && curve2.is_none()
            }
            Some(SnapKind::Origin) => {
                point.is_none() && curve == Some(ORIGIN_ID) && curve2.is_none()
            }
            Some(SnapKind::Intersection) => {
                point.is_none()
                    && curve.is_some_and(is_curve)
                    && curve2.is_some_and(is_curve)
                    && curve != curve2
            }
            Some(
                SnapKind::Midpoint
                | SnapKind::Perpendicular
                | SnapKind::Parallel
                | SnapKind::PerpendicularTo
                | SnapKind::OnExtension
                | SnapKind::OnAxis,
            ) => point.is_none() && curve.is_some_and(is_line) && curve2.is_none(),
            Some(SnapKind::Tangent | SnapKind::OnCurve) => {
                point.is_none() && curve.is_some_and(is_curve) && curve2.is_none()
            }
            Some(SnapKind::Quadrant(q)) => {
                q < 4
                    && point.is_none()
                    && curve.is_some_and(|id| {
                        matches!(sketch.get_geometry(id), Some(GeometryElement::Circle(_)))
                    })
                    && curve2.is_none()
            }
        }
    };
    if !valid {
        return Err("snap sources do not match their role or are missing".into());
    }
    let pos = match point {
        Some(id) => sketch.point_position(id).unwrap(),
        None => recorded.pos,
    };
    Ok(Snap {
        pos,
        kind: recorded.kind,
        target: point.map_or(SnapTarget::New(pos), SnapTarget::Existing),
        curve,
        curve2,
        on: recorded.on,
    })
}
