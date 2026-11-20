//! The plan tests of ribs.

use super::*;
use kernel_api::RibSide;

fn rib(sketch: FeatureId) -> DesignFeature {
    DesignFeature::Rib {
        sketch,
        thickness: 2.5,
        side: RibSide::Front,
        along_normal: true,
        refine: false,
        boolean_tolerance: None,
    }
}

/// A rib grows from its sketch's open chain, in order, with its
/// thickness, side and way; a closed sketch is refused on the rib.
#[test]
fn a_rib_grows_from_an_open_chain_and_refuses_a_closed_one() {
    let (mut doc, body, sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch, 6.0), "Pad");
    let mut open = Sketch::new("line");
    let corners = [(0.0, 0.0), (5.0, 0.0), (5.0, 4.0)]
        .map(|(x, y)| open.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    open.add_geometry(GeometryElement::Line(Line::new(corners[0], corners[1])));
    open.add_geometry(GeometryElement::Line(Line::new(corners[1], corners[2])));
    let line = add_sketch(&mut doc, body, open);
    let id = add(&mut doc, body, rib(line), "Rib");
    let plan = body_build_ops(&doc, body).unwrap();
    let Some(SolidOp::Rib { profile, form }) = plan.ops.last() else {
        panic!("a rib op, not {:?}", plan.ops.last());
    };
    assert_eq!(profile.wires.len(), 1);
    assert_eq!(profile.wires[0].segments.len(), 2);
    assert_eq!(
        (form.thickness, form.side, form.along_normal),
        (2.5, RibSide::Front, true)
    );
    assert!(doc.feature_tree().dependencies(id).contains(&line));

    doc.update_feature_data(id, rib(sketch).to_json()).unwrap();
    let err = body_build_ops(&doc, body).unwrap_err();
    assert_eq!(err.feature, Some(id));
    assert!(err.message.contains("open"), "{}", err.message);
}
