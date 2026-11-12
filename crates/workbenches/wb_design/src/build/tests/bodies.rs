//! The plan tests of booleans between bodies.

use super::*;

#[test]
fn a_body_is_never_its_own_boolean_tool() {
    let (mut doc, body, base_sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(base_sketch, 5.0), "Pad");
    doc.add_feature_in_body(
        DesignFeature::BodyBoolean {
            more_tools: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            tool_body: body,
            kind: kernel_api::BoolKind::Cut,
        },
        "Boolean".into(),
        Some(body),
    )
    .unwrap();
    let error = body_build_ops(&doc, body).unwrap_err();
    assert!(error.message.contains("its own tool"), "{}", error.message);
}

/// A Boolean with two tool bodies is planned once and then left alone
/// until one of its tools changes.
#[test]
fn a_boolean_with_two_tools_rebuilds_once() {
    let (mut doc, body, base_sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(base_sketch, 5.0), "Pad");
    let tools: Vec<BodyId> = (0..2)
        .map(|_| {
            let tool = doc.create_body(None);
            doc.set_imported_geometry(tool, bare_geometry(None));
            tool
        })
        .collect();
    doc.add_feature_in_body(
        DesignFeature::BodyBoolean {
            more_tools: vec![tools[1]],
            refine: false,
            boolean_tolerance: None,
            tool_body: tools[0],
            kind: kernel_api::BoolKind::Cut,
        },
        "Boolean".into(),
        Some(body),
    )
    .unwrap();
    let first = rebuild_jobs(&mut doc);
    assert!(first.iter().any(|j| j.body == body));
    let again = rebuild_jobs(&mut doc);
    assert!(
        again.iter().all(|j| j.body != body),
        "nothing changed, nothing to rebuild"
    );
}
