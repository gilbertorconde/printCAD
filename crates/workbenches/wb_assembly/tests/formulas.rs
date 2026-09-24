//! A joint's offset set by a variable: the body follows when the variable
//! changes, as the host settles formulas before an undo step closes.

use core_document::{
    BodyId, Document, DocumentService, Variable, VariableSet, WorkbenchFeature,
    WorkbenchRuntimeContext,
};
use wb_assembly::{Anchor, AssemblyWorkbench, JOINT_KIND, JointFeature, JointKind};

/// What the host does before closing an undo step.
fn settle(registry: &mut DocumentService, doc: &mut Document) {
    registry.evaluate(doc);
    let moved = doc.take_moved_values();
    if moved.is_empty() {
        return;
    }
    for id in registry.ids().to_vec() {
        let wb = registry.workbench_mut(&id).unwrap();
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        wb.values_moved(&mut ctx, &moved);
    }
}

fn height(doc: &Document, part: BodyId) -> f32 {
    doc.body_placement(part).point([0.0, 0.0, 0.0])[2]
}

#[test]
fn a_variable_offset_moves_the_body_and_settling_again_records_nothing() {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(AssemblyWorkbench::default()))
        .unwrap();
    let mut doc = Document::new("t");
    let base = doc.create_body(None);
    let part = doc.create_body(None);
    let gap = doc
        .add_feature(
            VariableSet {
                variables: vec![Variable {
                    name: "g".into(),
                    formula: "2 mm".into(),
                    comment: String::new(),
                }],
            },
            "Gap".into(),
        )
        .unwrap();
    let joint = doc
        .add_feature_in_body(
            JointFeature {
                kind: JointKind::Mate {
                    flip: false,
                    offset: 0.0,
                },
                moving: Anchor::Plane {
                    point: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, -1.0],
                },
                other_body: base,
                fixed: Anchor::Plane {
                    point: [0.0, 0.0, 10.0],
                    normal: [0.0, 0.0, 1.0],
                },
            },
            "Mate".into(),
            Some(part),
        )
        .unwrap();
    assert_eq!(
        doc.get_feature_meta(joint).unwrap().workbench_id.as_str(),
        JOINT_KIND
    );
    doc.set_feature_formula(joint, "/kind/Mate/offset", Some("Gap.g * 2".into()))
        .unwrap();

    settle(&mut registry, &mut doc);
    assert!(
        (height(&doc, part) - 14.0).abs() < 1e-3,
        "{}",
        height(&doc, part)
    );

    let mut set = VariableSet::from_json(doc.get_feature_data(gap).unwrap()).unwrap();
    set.variables[0].formula = "0.5 mm".into();
    doc.update_feature_data(gap, set.to_json()).unwrap();
    settle(&mut registry, &mut doc);
    assert!(
        (height(&doc, part) - 11.0).abs() < 1e-3,
        "{}",
        height(&doc, part)
    );

    // Nothing moved since: settling records no edit.
    doc.take_pending_ops();
    settle(&mut registry, &mut doc);
    assert!(doc.take_pending_ops().is_empty());
}
