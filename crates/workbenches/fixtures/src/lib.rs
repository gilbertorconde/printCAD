//! Ready-made scenes built from the benches' own feature types, for the
//! app's headless bench hooks and for tests. The host never names a bench;
//! this crate composes them on its behalf.

use core_document::{BodyId, Document, FeatureId};
use wb_part::{ExtrudeMode, PartFeature};
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{
    Circle, Constraint, ConstraintKind, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D,
};

/// How far a scene goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// A constrained sketch, to be opened for editing.
    Sketch,
    /// The sketch padded, the pad selected.
    Pad,
    /// The pad with a face sketch pocketed into its top, the pocket selected.
    Pocket,
}

impl Scene {
    /// The scene a bench-hook value names: `pad`, `pocket`, anything else
    /// the bare sketch.
    pub fn named(value: &str) -> Self {
        match value {
            "pad" => Scene::Pad,
            "pocket" => Scene::Pocket,
            _ => Scene::Sketch,
        }
    }
}

/// What the host does with the scene once built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneHandles {
    /// The feature to open for editing (the bare sketch).
    pub activate: Option<FeatureId>,
    /// The feature to select in the tree (the pad or the pocket).
    pub select: Option<FeatureId>,
}

/// A body's worth of features on `body`: a rectangle with a hole, fully
/// dimensioned, and what `scene` builds on it.
pub fn open_sketch_scene(
    document: &mut Document,
    body: Option<BodyId>,
    scene: Scene,
) -> Result<SceneHandles, String> {
    let mut sketch = Sketch::new("Sketch");
    let p = |s: &mut Sketch, x: f32, y: f32| {
        s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    };
    let a = p(&mut sketch, 0.0, 0.0);
    let b = p(&mut sketch, 80.0, 0.0);
    let c = p(&mut sketch, 80.0, 24.0);
    let d = p(&mut sketch, 0.0, 24.0);
    let bottom = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
    let right = sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
    sketch.add_geometry(GeometryElement::Line(Line::new(c, d)));
    sketch.add_geometry(GeometryElement::Line(Line::new(d, a)));
    let center = p(&mut sketch, 22.0, 12.0);
    let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 7.2)));
    for kind in [
        ConstraintKind::Horizontal { element: bottom },
        ConstraintKind::Vertical { element: right },
        ConstraintKind::Length {
            line: bottom,
            length: 80.0,
        },
        ConstraintKind::Length {
            line: right,
            length: 24.0,
        },
        ConstraintKind::Diameter {
            circle,
            diameter: 14.4,
        },
    ] {
        sketch.constraints.push(Constraint::new(kind));
    }
    let plane = sketch.plane;
    let sketch_id = document
        .add_feature_in_body(SketchFeature::new(sketch, plane), "Sketch".into(), body)
        .map_err(|err| format!("sketch: {err}"))?;
    if scene == Scene::Sketch {
        return Ok(SceneHandles {
            activate: Some(sketch_id),
            select: None,
        });
    }

    let pad = PartFeature::Pad {
        refine: false,
        sketch: sketch_id,
        length: 20.0,
        reversed: false,
        symmetric: false,
        mode: ExtrudeMode::Dimension,
        length2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
    };
    let pad_id = document
        .add_feature_in_body(pad, "Pad".into(), body)
        .map_err(|err| format!("pad: {err}"))?;
    document.mark_feature_dirty(pad_id);
    document.set_feature_visible(sketch_id, false);
    if scene == Scene::Pad {
        return Ok(SceneHandles {
            activate: None,
            select: Some(pad_id),
        });
    }

    let mut top = Sketch::new("sketch_1");
    top.plane = SketchPlane::from_face([0.0, 0.0, 20.0], [0.0, 0.0, 1.0]);
    let center = top.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40.0, 12.0))));
    top.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
    let plane = top.plane;
    let top_id = document
        .add_feature_in_body(SketchFeature::new(top, plane), "sketch_1".into(), body)
        .map_err(|err| format!("face sketch: {err}"))?;
    let pocket = PartFeature::Pocket {
        refine: false,
        sketch: top_id,
        depth: 5.0,
        reversed: false,
        symmetric: false,
        through_all: false,
        mode: ExtrudeMode::Dimension,
        depth2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
    };
    let pocket_id = document
        .add_feature_in_body(pocket, "Pocket".into(), body)
        .map_err(|err| format!("pocket: {err}"))?;
    document.mark_feature_dirty(pocket_id);
    document.set_feature_visible(top_id, false);
    Ok(SceneHandles {
        activate: None,
        select: Some(pocket_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scene_builds_on_the_body_and_names_what_to_do_next() {
        for (scene, features) in [(Scene::Sketch, 1), (Scene::Pad, 2), (Scene::Pocket, 4)] {
            let mut doc = Document::new("t");
            let body = doc.create_body(None);
            let handles = open_sketch_scene(&mut doc, Some(body), scene).unwrap();
            assert_eq!(
                doc.feature_tree().all_nodes().count(),
                features,
                "{scene:?}"
            );
            assert!(
                doc.feature_tree()
                    .all_nodes()
                    .all(|(_, n)| n.body == Some(body)),
                "{scene:?}: every feature sits on the body"
            );
            match scene {
                Scene::Sketch => assert!(handles.activate.is_some() && handles.select.is_none()),
                _ => assert!(handles.activate.is_none() && handles.select.is_some()),
            }
        }
        assert_eq!(Scene::named("pad"), Scene::Pad);
        assert_eq!(Scene::named("1"), Scene::Sketch);
    }
}
