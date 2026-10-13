//! The Assembly bench's integration tests, one program.
//!
//! One program rather than one per file: each would link the whole crate
//! and its dependencies again, which costs far more than running the tests.

mod couplings;
mod following;
mod formulas;
mod grounding;

use core_document::{BodyId, Document, DocumentService, WorkbenchRuntimeContext};

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

/// How high `body`'s origin stands.
fn height(doc: &Document, body: BodyId) -> f32 {
    doc.body_placement(body).point([0.0, 0.0, 0.0])[2]
}
