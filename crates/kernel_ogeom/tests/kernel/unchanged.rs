//! A feature that adds or removes material but leaves the solid as it was
//! builds, and its row warns that it changed nothing.

use core_document::{BodyId, Document, FeatureId};
use sketch_model::SketchFeature;
use sketch_model::sketch::{GeometryElement, Point, Sketch, SketchPlane, Vec2D};
use wb_design::DesignFeature;

use super::common::{
    Features, body_volume, extrude, pad, pocket, rect_at, registry, settle_as_host,
};

/// A 20×20×10 block in a body of its own.
fn block() -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0),
    );
    let block = doc.add_in(body, "Pad", pad(base, 10.0));
    (doc, body, block)
}

/// A sketch on the block's top face, its normal up.
fn on_top() -> SketchPlane {
    SketchPlane::from_face([10.0, 10.0, 10.0], [0.0, 0.0, 1.0])
}

/// A plain hole 4 mm across and 5 mm deep at the points of `sketch`.
fn hole(sketch: FeatureId, reversed: bool) -> DesignFeature {
    DesignFeature::Hole {
        cosmetic_thread: false,
        start: Default::default(),
        clearance: None,
        thread_length: Default::default(),
        refine: false,
        boolean_tolerance: None,
        sketch,
        diameter: 4.0,
        depth: 5.0,
        through_all: false,
        cut: wb_design::HoleCut::None,
        thread: None,
        threaded: false,
        modeled_thread: false,
        thread_depth: 0.0,
        fit: wb_design::HoleFit::Normal,
        drill_point: wb_design::DrillPoint::Flat,
        point_in_depth: false,
        taper_deg: 0.0,
        nut_trap: None,
        shape: Default::default(),
        bridge: None,
        reversed,
    }
}

/// What `feature`'s row warns of once the body has settled, the build
/// having succeeded.
fn warned(doc: &mut Document, body: BodyId, feature: FeatureId) -> Option<String> {
    let registry = registry();
    wb_design::mark_all_design_features_dirty(doc);
    settle_as_host(&registry, doc);
    assert!(doc.imported_geometry(body).is_some(), "built");
    let node = doc.get_feature_meta(feature).unwrap();
    assert!(node.error.is_none(), "{:?}", node.error);
    registry.feature_warning(doc, None, feature, node)
}

/// A pocket reversed off the top face cuts the air above it: the solid is
/// as it was, and its row says the pocket removes nothing. Turned back
/// into the material, it cuts and the warning goes.
#[test]
fn a_pocket_that_removes_nothing_is_warned_of() {
    let (mut doc, body, _) = block();
    let sketch = doc.add_in(body, "top", rect_at(on_top(), 2.0, 2.0, 6.0, 6.0));
    let reversed = |reversed: bool| {
        extrude(
            "Pocket",
            sketch,
            serde_json::json!({"depth": 3.0, "reversed": reversed}),
        )
    };
    let cut = doc.add_in(body, "Pocket", reversed(true));
    let said = warned(&mut doc, body, cut).expect("warned");
    assert!((body_volume(&doc, body) - 4000.0).abs() < 1e-6);
    assert!(said.contains("pocket removes nothing"), "{said}");
    assert!(said.contains("away from the material"), "{said}");

    doc.set_feature(cut, reversed(false));
    doc.mark_feature_dirty(cut);
    assert_eq!(warned(&mut doc, body, cut), None);
    assert!((body_volume(&doc, body) - (4000.0 - 48.0)).abs() < 1e-6);
}

/// A pad wholly inside the block adds nothing, a hole drilled off the top
/// face removes nothing, and a mirror whose copy of a pocket lands past
/// the material removes nothing: each is warned of in its own words.
#[test]
fn pads_holes_and_copies_that_change_nothing_are_warned_of() {
    let (mut doc, body, _) = block();
    let inside = SketchPlane {
        origin: [0.0, 0.0, 2.0],
        ..SketchPlane::xy()
    };
    let sketch = doc.add_in(body, "inside", rect_at(inside, 5.0, 5.0, 15.0, 15.0));
    let buried = doc.add_in(body, "Pad", pad(sketch, 4.0));
    let said = warned(&mut doc, body, buried).expect("warned");
    assert!(said.contains("pad adds nothing"), "{said}");

    let (mut doc, body, _) = block();
    let mut points = Sketch::new("holes");
    points.plane = on_top();
    points.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
    let points = doc.add_in(body, "holes", SketchFeature::new(points, on_top()));
    let drilled = doc.add_in(body, "Hole", hole(points, true));
    let said = warned(&mut doc, body, drilled).expect("warned");
    assert!(said.contains("hole removes nothing"), "{said}");
    doc.set_feature(drilled, hole(points, false));
    doc.mark_feature_dirty(drilled);
    assert_eq!(warned(&mut doc, body, drilled), None);

    // The pocket cuts at the block's corner; its mirror across YZ lands
    // past the block, on the far side of x = 0.
    let (mut doc, body, _) = block();
    let sketch = doc.add_in(body, "top", rect_at(on_top(), 0.0, 0.0, 4.0, 4.0));
    let cut = doc.add_in(body, "Pocket", pocket(sketch, 3.0));
    let mirror = doc.add_in(
        body,
        "Mirror",
        DesignFeature::Mirrored {
            skipped: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            originals: vec![cut],
            plane: wb_design::MirrorPlane::YZ,
        },
    );
    let said = warned(&mut doc, body, mirror).expect("warned");
    assert!(said.contains("mirrored removes nothing"), "{said}");
    assert_eq!(warned(&mut doc, body, cut), None, "the pocket itself cuts");
}

/// A boolean cut by a body that does not meet the target removes nothing.
#[test]
fn a_boolean_whose_tool_misses_is_warned_of() {
    let (mut doc, target, _) = block();
    let tool = doc.create_body(Some("Tool".into()));
    let away = doc.add_in(
        tool,
        "away",
        rect_at(SketchPlane::xy(), 40.0, 40.0, 50.0, 50.0),
    );
    doc.add_in(tool, "Pad", pad(away, 5.0));
    let boolean = doc.add_in(
        target,
        "Boolean",
        DesignFeature::BodyBoolean {
            more_tools: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            tool_body: tool,
            kind: kernel_api::BoolKind::Cut,
        },
    );
    let said = warned(&mut doc, target, boolean).expect("warned");
    assert!(said.contains("boolean removes nothing"), "{said}");
}
