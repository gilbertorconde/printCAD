//! Grounding a body, and asking what the others may still do.

use core_document::{BodyPlacement, Document, Workbench, WorkbenchRuntimeContext};
use glam::{Quat, Vec3};
use serde_json::{Value, json};
use wb_assembly::AssemblyWorkbench;

fn run(wb: &mut AssemblyWorkbench, doc: &mut Document, id: &str, args: Value) -> Value {
    let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    wb.run_command(id, args.as_object().unwrap(), &mut ctx)
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn a_grounded_body_stays_and_the_others_say_what_they_may_still_do() {
    let mut wb = AssemblyWorkbench::default();
    let mut doc = Document::new("t");
    let base = doc.create_body(None);
    let part = doc.create_body(None);
    doc.set_body_placement(
        base,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, 3.0)),
    );
    doc.set_body_placement(
        part,
        BodyPlacement::new(Quat::from_rotation_x(0.4), Vec3::new(30.0, 0.0, 20.0)),
    );
    let (b, p) = (base.0.to_string(), part.0.to_string());
    let top = json!({"point": [0.0, 0.0, 10.0], "normal": [0.0, 0.0, 1.0]});
    let under = json!({"point": [30.0, 0.0, 20.0], "normal": [0.0, 0.0, -1.0]});
    // The base is joined to the part too: without a ground, both would
    // move.
    run(
        &mut wb,
        &mut doc,
        "asm.mate",
        json!({"body": p, "face": under, "other": b, "other_face": top}),
    );
    run(&mut wb, &mut doc, "asm.ground", json!({"body": b}));
    // A joint the grounded base owns still holds, by the part moving: the
    // part's underside where the first mate put it.
    let resting = json!({"point": [30.0, 0.0, 10.0], "normal": [0.0, 0.0, -1.0]});
    run(
        &mut wb,
        &mut doc,
        "asm.mate",
        json!({"body": b, "face": top, "other": p, "other_face": resting}),
    );
    assert_eq!(
        doc.body_placement(base).offset(),
        Vec3::new(0.0, 0.0, 3.0),
        "the ground stays"
    );

    let free = run(&mut wb, &mut doc, "asm.freedom", json!({"body": p}));
    assert_eq!(free[0]["free"], 3, "{free}");
    let grounded = run(&mut wb, &mut doc, "asm.freedom", json!({"body": b}));
    assert!(
        grounded.as_array().unwrap().is_empty(),
        "a grounded body is not free"
    );

    run(
        &mut wb,
        &mut doc,
        "asm.ground",
        json!({"body": b, "grounded": false}),
    );
    let ground_left = doc
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.name == "Ground")
        .count();
    assert_eq!(ground_left, 0, "ungrounding takes the ground joint away");
}
