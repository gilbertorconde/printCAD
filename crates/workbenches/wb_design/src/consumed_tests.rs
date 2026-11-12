use super::*;
use crate::feature::{EdgeSel, MirrorPlane};
use crate::test_support::{pad, rect_sketch};
use core_document::Document;

/// A pad consumes its sketch; a mirror of the pad, a whole-body
/// mirror on the sketch's plane and a fillet of every edge work on the
/// body and consume nothing, whatever they read.
#[test]
fn only_a_feature_made_from_a_sketch_consumes_it() {
    let mut doc = Document::new("t");
    let body = Some(doc.create_body(None));
    let sketch = doc
        .add_feature_in_body(rect_sketch(10.0, 10.0), "Sketch".into(), body)
        .unwrap();
    let pad = doc
        .add_feature_in_body(pad(sketch, 5.0), "Pad".into(), body)
        .unwrap();
    let mirror = |originals, plane| DesignFeature::Mirrored {
        originals,
        plane,
        refine: false,
        boolean_tolerance: None,
        skipped: Vec::new(),
    };
    let of_pad = doc
        .add_feature_in_body(mirror(vec![pad], MirrorPlane::YZ), "Mirrored".into(), body)
        .unwrap();
    let on_sketch = mirror(Vec::new(), MirrorPlane::Sketch { sketch, axis: None });
    let whole = doc
        .add_feature_in_body(on_sketch, "Mirrored_1".into(), body)
        .unwrap();
    let fillet = DesignFeature::Fillet {
        radius: 1.0,
        edges: EdgeSel::All,
        follow_tangent: false,
    };
    let fillet = doc
        .add_feature_in_body(fillet, "Fillet".into(), body)
        .unwrap();
    let wb = DesignWorkbench::default();
    let consumed = |id| wb.consumed_feature(&doc, doc.get_feature_meta(id).unwrap());
    assert_eq!(consumed(pad), Some(sketch));
    assert!(!doc.feature_tree().dependencies(whole).is_empty());
    for id in [of_pad, whole, fillet] {
        assert_eq!(consumed(id), None);
    }
}
