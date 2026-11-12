use super::*;
use crate::test_support::imported;
use core_document::testing::{call_checked, ctx};
use core_document::{ComponentId, Document};

/// Run a command as a script does: its arguments checked against its
/// registered spec first.
fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
    call_checked(&mut crate::AssemblyWorkbench::default(), doc, id, args)
}

/// Run joint command `id` holding `body`'s `face` to `other`'s
/// `other_face`, with the `extra` arguments.
fn join(
    doc: &mut Document,
    id: &str,
    body: BodyId,
    face: &Value,
    other: BodyId,
    other_face: &Value,
    extra: Value,
) -> CommandResult {
    let mut args = json!({"body": body.0.to_string(), "face": face,
                          "other": other.0.to_string(), "other_face": other_face});
    let args_map = args.as_object_mut().unwrap();
    args_map.extend(extra.as_object().unwrap().clone());
    call(doc, id, args)
}

/// A document of two bodies.
fn two() -> (Document, BodyId, BodyId) {
    let mut doc = Document::new("t");
    let (a, b) = (doc.create_body(None), doc.create_body(None));
    (doc, a, b)
}

/// The feature a command answered with.
fn feature(id: &Value) -> FeatureId {
    FeatureId(uuid::Uuid::parse_str(id.as_str().unwrap()).unwrap())
}

/// Unturned, at (`x`, `y`, `z`).
fn at(x: f32, y: f32, z: f32) -> BodyPlacement {
    BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(x, y, z))
}

/// Where `joint` stands, as `asm.travel` reads it.
fn travel(doc: &mut Document, joint: &Value) -> f64 {
    call(doc, "asm.travel", json!({"joint": joint}))
        .unwrap()
        .as_f64()
        .unwrap()
}

/// How far `body` is turned about Z, in degrees.
fn turned_deg(doc: &Document, body: BodyId) -> f32 {
    let x = doc.body_placement(body).direction([1.0, 0.0, 0.0]);
    x[1].atan2(x[0]).to_degrees()
}

#[test]
fn a_mate_puts_one_body_s_face_on_the_other_s() {
    let (mut doc, a, b) = two();
    call(
        &mut doc,
        "asm.move",
        json!({"body": a.0.to_string(), "by": [0, 0, 50]}),
    )
    .unwrap();
    // a's bottom (at z = 50 where it sits) onto b's top at z = 10.
    let joint = join(
        &mut doc,
        "asm.mate",
        a,
        &json!({"point": [0, 0, 50], "normal": [0, 0, -1]}),
        b,
        &json!({"point": [0, 0, 10], "normal": [0, 0, 1]}),
        json!({"offset": 2}),
    )
    .unwrap();
    let z = doc.body_placement(a).translation[2];
    assert!((z - 12.0).abs() < 1e-3, "{z}");
    call(&mut doc, "asm.set", json!({"joint": joint, "offset": 5})).unwrap();
    assert!((doc.body_placement(a).translation[2] - 15.0).abs() < 1e-3);
    let err = join(
        &mut doc,
        "asm.align",
        a,
        &json!({"point": [0, 0, 0], "normal": [0, 0, 1]}),
        b,
        &json!({"point": [0, 0, 0], "normal": [0, 0, 1]}),
        json!({}),
    );
    assert!(err.is_err(), "an alignment takes round faces");
}

#[test]
fn a_tangent_takes_its_radius_from_the_round_face_and_either_order() {
    let (mut doc, a, b) = two();
    let roller = json!({"axis": {"point": [0, 0, 0], "direction": [1, 0, 0]}, "radius": 4});
    let top = json!({"point": [0, 0, 10], "normal": [0, 0, 1]});
    let joint = join(&mut doc, "asm.tangent", a, &roller, b, &top, json!({})).unwrap();
    assert!((doc.body_placement(a).translation[2] - 14.0).abs() < 1e-3);
    call(&mut doc, "asm.set", json!({"joint": joint, "radius": 6})).unwrap();
    assert!((doc.body_placement(a).translation[2] - 16.0).abs() < 1e-3);
    let both_flat = join(&mut doc, "asm.tangent", a, &top, b, &top, json!({}));
    assert!(both_flat.is_err(), "one face of each");
}

/// Turning a joint's body keeps it there: a driven hinge keeps its
/// angle with the body turned further, a slider turned over runs the
/// other way round, a mate turned over faces the same way.
#[test]
fn a_joint_s_body_turns_and_turns_over_and_stays() {
    let (mut doc, a, b) = two();
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    let hinge = join(
        &mut doc,
        "asm.hinge",
        a,
        &pin,
        b,
        &pin,
        json!({"drive": 30}),
    )
    .unwrap();
    call(&mut doc, "asm.turn", json!({"joint": hinge, "degrees": 20})).unwrap();
    let turned = turned_deg(&doc, a);
    assert!((turned - 50.0).abs() < 1e-2, "{turned}");
    // The drive moved with it: the hinge reads where the body is.
    let reads = travel(&mut doc, &hinge);
    assert!((reads - 50.0).abs() < 1e-2, "{reads}");
    call(&mut doc, "asm.solve", json!({})).unwrap();
    let turned = turned_deg(&doc, a);
    assert!((turned - 50.0).abs() < 1e-2, "stays: {turned}");

    let (mut doc, a, b) = two();
    let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    let mate = join(&mut doc, "asm.mate", a, &top, b, &top, json!({})).unwrap();
    let up = doc.body_placement(a).direction([0.0, 0.0, 1.0]);
    assert!(up[2] < -0.99, "faces the other: {up:?}");
    call(&mut doc, "asm.flip", json!({"joint": mate})).unwrap();
    call(&mut doc, "asm.solve", json!({})).unwrap();
    let up = doc.body_placement(a).direction([0.0, 0.0, 1.0]);
    assert!(up[2] > 0.99, "the same way now: {up:?}");
    let id = feature(&mate);
    let data: JointFeature =
        serde_json::from_value(doc.get_feature_data(id).unwrap().clone()).unwrap();
    assert!(matches!(data.kind, JointKind::Mate { flip: true, .. }));
}

/// A quarter turn of a hinge's zero turns the body with it and leaves
/// the hinge reading and holding what it did; back undoes it. A fixed
/// joint's held turn turns its body about its normal; a mate, which
/// keeps no turn, is refused.
#[test]
fn a_quarter_turn_moves_a_joint_s_own_turn_and_its_body() {
    let (mut doc, a, b) = two();
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    let hinge = join(
        &mut doc,
        "asm.hinge",
        a,
        &pin,
        b,
        &pin,
        json!({"drive": 30, "limits": [-45, 45]}),
    )
    .unwrap();
    let angle = |doc: &Document| turned_deg(doc, a);
    call(&mut doc, "asm.turn90", json!({"joint": hinge})).unwrap();
    assert!((angle(&doc) - 120.0).abs() < 1e-2, "{}", angle(&doc));
    assert!(
        (travel(&mut doc, &hinge) - 30.0).abs() < 1e-2,
        "the hinge reads as it did"
    );
    let id = feature(&hinge);
    let data: JointFeature =
        serde_json::from_value(doc.get_feature_data(id).unwrap().clone()).unwrap();
    let JointKind::Hinge { drive, .. } = data.kind else {
        panic!("a hinge");
    };
    assert_eq!(drive.to, Some(30.0));
    assert_eq!(drive.limits, Some([-45.0, 45.0]));
    call(&mut doc, "asm.solve", json!({})).unwrap();
    assert!((angle(&doc) - 120.0).abs() < 1e-2, "stays: {}", angle(&doc));
    call(
        &mut doc,
        "asm.turn90",
        json!({"joint": hinge, "back": true}),
    )
    .unwrap();
    assert!((angle(&doc) - 30.0).abs() < 1e-2, "{}", angle(&doc));

    let (mut doc, a, b) = two();
    call(
        &mut doc,
        "asm.move",
        json!({"body": a.0.to_string(), "by": [5, 0, 0]}),
    )
    .unwrap();
    let fixed = call(
        &mut doc,
        "asm.fix",
        json!({"body": a.0.to_string(), "other": b.0.to_string()}),
    )
    .unwrap();
    call(&mut doc, "asm.turn90", json!({"joint": fixed})).unwrap();
    call(&mut doc, "asm.solve", json!({})).unwrap();
    let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
    assert!(x[1] > 0.999, "a quarter turn about Z: {x:?}");
    assert!(crate::unmet(&doc).is_empty(), "the joint holds it there");

    let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    let c = doc.create_body(None);
    let mate = join(&mut doc, "asm.mate", c, &top, b, &top, json!({})).unwrap();
    assert!(call(&mut doc, "asm.turn90", json!({"joint": mate})).is_err());
}

/// Selecting a body's joints asks the tree for every joint it moves by
/// or is held to, in history order, and answers them.
#[test]
fn a_body_s_joints_are_selected_together() {
    let mut doc = Document::new("t");
    let [a, b, c] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    let mate = |doc: &mut Document, body: BodyId, other: BodyId| {
        join(doc, "asm.mate", body, &top, other, &top, json!({})).unwrap()
    };
    let ground = call(&mut doc, "asm.ground", json!({"body": a.0.to_string()})).unwrap();
    let ab = mate(&mut doc, b, a);
    let cb = mate(&mut doc, c, b);

    let selected = |doc: &mut Document, body: BodyId| {
        let mut ctx = ctx(doc);
        let answer = run(
            "asm.select_joints",
            &object(json!({"body": body.0.to_string()})),
            &mut ctx,
        )
        .unwrap();
        let requests = core_document::HookOutcome::take(&mut ctx).requests;
        let [core_document::HostRequest::SelectFeatures(features)] = requests.as_slice() else {
            panic!("{requests:?}");
        };
        let answered: Vec<FeatureId> = answer
            .as_array()
            .unwrap()
            .iter()
            .map(|r| feature(&r["joint"]))
            .collect();
        assert_eq!(&answered, features);
        answered
    };
    assert_eq!(selected(&mut doc, a), [feature(&ground), feature(&ab)]);
    assert_eq!(selected(&mut doc, b), [feature(&ab), feature(&cb)]);
    assert_eq!(selected(&mut doc, c), [feature(&cb)]);
    let d = doc.create_body(None);
    assert!(selected(&mut doc, d).is_empty(), "none: the document");
}

/// Turning either of two geared hinges turns both: the driver by what
/// it is turned, or by what takes the driven one along.
#[test]
fn a_turn_of_either_geared_hinge_turns_both() {
    let mut doc = Document::new("t");
    let [base, g1, g2] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    let pin = |x: f32| json!({"axis": {"point": [x, 0, 0], "direction": [0, 0, 1]}});
    let hinge = |doc: &mut Document, body: BodyId, x: f32| {
        join(doc, "asm.hinge", body, &pin(x), base, &pin(x), json!({})).unwrap()
    };
    let h1 = hinge(&mut doc, g1, -20.0);
    let h2 = hinge(&mut doc, g2, 10.0);
    call(
        &mut doc,
        "asm.couple",
        json!({"driver": h1, "driven": h2, "gearing": "gears", "ratio": 2}),
    )
    .unwrap();
    call(&mut doc, "asm.turn", json!({"joint": h1, "degrees": 30})).unwrap();
    assert!((travel(&mut doc, &h1) - 30.0).abs() < 1e-2);
    assert!((travel(&mut doc, &h2) + 60.0).abs() < 1e-2);
    call(&mut doc, "asm.turn", json!({"joint": h2, "degrees": 30})).unwrap();
    assert!((travel(&mut doc, &h1) - 15.0).abs() < 1e-2);
    assert!((travel(&mut doc, &h2) + 30.0).abs() < 1e-2);
    // The drives held for the turns are taken off again.
    for joint in crate::joints(&doc) {
        if let JointKind::Hinge { drive, .. } = joint.feature.kind {
            assert_eq!(drive.to, None);
        }
    }
}

/// A slider turns its body onto the axis, whichever way the body's
/// own axis pointed when it was made.
#[test]
fn a_slider_turns_its_body_onto_the_axis() {
    for along in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]] {
        let (mut doc, a, b) = two();
        let peg = json!({"axis": {"point": [0, 0, 20], "direction": along}});
        let hole = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        join(&mut doc, "asm.slider", a, &peg, b, &hole, json!({})).unwrap();
        let d = doc.body_placement(a).direction(along);
        let n = glam::Vec3::from_array(d).normalize();
        assert!(n.z.abs() > 0.999, "{along:?} now {d:?}");
    }
}

/// Faces that start exactly parallel are turned square.
#[test]
fn perpendicular_faces_start_parallel_and_end_square() {
    let (mut doc, a, b) = two();
    let up = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    join(&mut doc, "asm.perpendicular", a, &up, b, &up, json!({})).unwrap();
    let n = doc.body_placement(a).direction([0.0, 0.0, 1.0]);
    assert!(n[2].abs() < 1e-3, "square to Z: {n:?}");
}

/// A part set bought is bought for every body of its shape, the
/// bench leaves them out of what is printed, and the list says so.
#[test]
fn a_part_is_marked_bought_and_given_a_column() {
    use core_document::Workbench;
    let mut doc = Document::new("t");
    let (a, b) = (doc.create_body(Some("Screw".into())), doc.create_body(None));
    doc.set_imported_brep_data(a, b"screw".to_vec(), Vec::new());
    doc.set_imported_brep_data(b, b"screw".to_vec(), Vec::new());
    call(
        &mut doc,
        "asm.part",
        json!({"body": b.0.to_string(), "bought": true, "number": 4,
               "values": {"Supplier": "Fasteners Ltd"}}),
    )
    .unwrap();
    let listed = call(&mut doc, "asm.parts", json!({})).unwrap();
    assert_eq!(listed[0]["bought"], json!(true));
    assert_eq!(listed[0]["number"], json!(4));
    assert_eq!(listed[0]["values"]["Supplier"], json!("Fasteners Ltd"));
    let mut not_made = crate::AssemblyWorkbench::default().not_printed(&doc, &[]);
    not_made.sort();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(not_made, both);
}

/// Copies turned about an axis spread over a whole turn with the
/// original: three about Z make four in all, a quarter turn apart.
#[test]
fn copies_spread_round_an_axis() {
    let mut doc = Document::new("t");
    let arm = doc.create_body(Some("Arm".into()));
    doc.set_body_placement(arm, at(10.0, 0.0, 0.0));
    let made = call(
        &mut doc,
        "asm.copy",
        json!({"body": arm.0.to_string(), "count": 3,
               "around": {"point": [0, 0, 0], "direction": [0, 0, 1]}}),
    )
    .unwrap();
    let at: Vec<[f32; 3]> = made
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let id = BodyId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap());
            doc.body_placement(id).translation
        })
        .collect();
    let want = [[0.0, 10.0], [-10.0, 0.0], [0.0, -10.0]];
    for (got, want) in at.iter().zip(want) {
        assert!(
            (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3,
            "{at:?}"
        );
    }
}

/// A body replaced by another: the mate on the old body's underside
/// goes to the new body's, which sits on the base; the old is hidden.
#[test]
fn a_replacement_takes_the_joints_on_its_own_faces() {
    let mut doc = Document::new("t");
    let [base, old, new] = [
        doc.create_body(Some("Base".into())),
        doc.create_body(Some("Old".into())),
        doc.create_body(Some("New".into())),
    ];
    let with_bottom = |z: f32| {
        imported(
            kernel_api::TriMesh {
                positions: vec![[0.0, 0.0, z], [10.0, 0.0, z], [0.0, 10.0, z]],
                normals: vec![[0.0, 0.0, -1.0]; 3],
                indices: vec![0, 2, 1],
                faces: vec![0],
                face_surfaces: vec![kernel_api::FaceSurface::Plane {
                    origin: [0.0, 0.0, z],
                    normal: [0.0, 0.0, -1.0],
                }],
                ..kernel_api::TriMesh::default()
            },
            None,
        )
    };
    doc.set_imported_geometry(old, with_bottom(0.0));
    doc.set_imported_geometry(new, with_bottom(2.0));
    doc.set_body_placement(old, at(0.0, 0.0, 30.0));
    join(
        &mut doc,
        "asm.distance",
        old,
        &json!({"point": [0, 0, 30], "normal": [0, 0, -1]}),
        base,
        &json!({"point": [0, 0, 10], "normal": [0, 0, 1]}),
        json!({"offset": 0}),
    )
    .unwrap();
    assert!((doc.body_placement(old).translation[2] - 10.0).abs() < 1e-3);
    let report = call(
        &mut doc,
        "asm.replace",
        json!({"body": old.0.to_string(), "with": new.0.to_string()}),
    )
    .unwrap();
    assert_eq!(report["unmatched"], json!([]), "{report}");
    let joint = crate::joints(&doc)
        .into_iter()
        .find(|j| j.feature.kind != JointKind::Ground)
        .unwrap();
    assert_eq!(joint.body, new);
    // Its underside, 2 up in its own frame, on the base's top at 10.
    assert!((doc.body_placement(new).translation[2] - 8.0).abs() < 1e-3);
    assert!(doc.body(old).is_some_and(|b| b.hidden));
}

/// A motion drives a hinge by a formula of time, frame by frame, and
/// moves nothing in the document.
#[test]
fn a_motion_drives_its_joints_over_time() {
    let (mut doc, a, b) = two();
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    let hinge = join(&mut doc, "asm.hinge", a, &pin, b, &pin, json!({"drive": 0})).unwrap();
    let study = call(
        &mut doc,
        "asm.motion",
        json!({"drives": [{"joint": hinge, "formula": "90 * t"}], "start": 0, "end": 1, "step": 0.5}),
    )
    .unwrap();
    let frames = call(&mut doc, "asm.motion_frames", json!({"study": study})).unwrap();
    let frames = frames.as_array().unwrap();
    assert_eq!(frames.len(), 3);
    let turned = |frame: &Value| {
        let row = frame["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["body"] == json!(a.0.to_string()))
            .unwrap()
            .clone();
        let q: [f32; 4] = serde_json::from_value(row["rotation"].clone()).unwrap();
        let x = glam::Quat::from_array(q) * glam::Vec3::X;
        x.y.atan2(x.x).to_degrees()
    };
    assert!((turned(&frames[1]) - 45.0).abs() < 1e-2);
    assert!((turned(&frames[2]) - 90.0).abs() < 1e-2);
    let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
    assert!(x[1].abs() < 1e-4, "the document is not moved");
    let bad = call(
        &mut doc,
        "asm.motion",
        json!({"drives": [{"joint": hinge, "formula": "90 * "}]}),
    );
    assert!(bad.is_err());
}

/// An exploded view kept by a script plays its steps in order.
#[test]
fn an_exploded_view_is_kept_and_played() {
    let (mut doc, a, b) = two();
    let view = call(
        &mut doc,
        "asm.exploded_view",
        json!({"steps": [
            {"bodies": [a.0.to_string()], "shift": [0, 0, 10]},
            {"bodies": [a.0.to_string(), b.0.to_string()], "shift": [4, 0, 0]},
        ]}),
    )
    .unwrap();
    let at = call(&mut doc, "asm.explode_at", json!({"view": view, "at": 1.5})).unwrap();
    let of = |body: BodyId| {
        at.as_array()
            .unwrap()
            .iter()
            .find(|r| r["body"] == json!(body.0.to_string()))
            .unwrap()["translation"]
            .clone()
    };
    assert_eq!(of(a), json!([2.0, 0.0, 10.0]));
    assert_eq!(of(b), json!([2.0, 0.0, 0.0]));
    assert_eq!(doc.body_placement(a).translation, [0.0; 3], "nothing moved");
}

/// A saved state brings back a driven hinge's angle, where the bodies
/// sat and which were hidden.
#[test]
fn a_saved_state_is_returned_to() {
    let (mut doc, a, b) = two();
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    let hinge = join(&mut doc, "asm.hinge", a, &pin, b, &pin, json!({"drive": 0})).unwrap();
    let folded = call(&mut doc, "asm.save_state", json!({"name": "Folded"})).unwrap();
    call(&mut doc, "asm.set", json!({"joint": hinge, "drive": 90})).unwrap();
    doc.set_body_visible(b, false);
    assert!((turned_deg(&doc, a) - 90.0).abs() < 1e-2);
    call(&mut doc, "asm.restore_state", json!({"state": folded})).unwrap();
    assert!(turned_deg(&doc, a).abs() < 1e-2, "{}", turned_deg(&doc, a));
    assert!(doc.bodies().iter().all(|x| !x.hidden));
}

/// A parallel joint added to a mate holds nothing new: it is reported
/// redundant; the mate alone is not.
#[test]
fn a_joint_holding_nothing_new_is_redundant() {
    let (mut doc, a, b) = two();
    let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    let under = json!({"point": [0, 0, 0], "normal": [0, 0, -1]});
    join(&mut doc, "asm.mate", a, &under, b, &top, json!({})).unwrap();
    assert_eq!(
        call(&mut doc, "asm.redundant", json!({})).unwrap(),
        json!([])
    );
    join(
        &mut doc,
        "asm.parallel",
        a,
        &under,
        b,
        &top,
        json!({"name": "Extra"}),
    )
    .unwrap();
    let found = call(&mut doc, "asm.redundant", json!({})).unwrap();
    let names: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Extra"), "{names:?}");
}

/// A tab centred in a slot: its middle on the slot's middle.
#[test]
fn a_tab_is_centred_in_its_slot() {
    let (mut doc, tab, slot) = two();
    let face = |x: f32, n: f32| json!({"point": [x, 0, 0], "normal": [n, 0, 0]});
    join(
        &mut doc,
        "asm.width",
        tab,
        &face(0.0, -1.0),
        slot,
        &face(10.0, 1.0),
        json!({"face2": face(4.0, 1.0), "other_face2": face(20.0, -1.0)}),
    )
    .unwrap();
    let middle = doc.body_placement(tab).point([2.0, 0.0, 0.0]);
    assert!((middle[0] - 15.0).abs() < 1e-3, "{middle:?}");
}

/// A point on a path runs along the edge it was put on; a follower on
/// a cam sits a roller's radius off the cam's face.
#[test]
fn a_path_and_a_cam_hold_to_the_other_body_s_edge_and_face() {
    let (mut doc, a, b) = two();
    // An L-shaped edge along X then Y at height 10, and a floor face at
    // z = 10 facing up.
    let mesh = kernel_api::TriMesh {
        positions: vec![
            [0.0, 0.0, 10.0],
            [20.0, 0.0, 10.0],
            [20.0, 20.0, 10.0],
            [0.0, 20.0, 10.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        faces: vec![0, 0],
        edges: vec![0, 1, 1, 2],
        edge_ids: vec![4, 4],
        ..kernel_api::TriMesh::default()
    };
    doc.set_imported_geometry(b, imported(mesh, None));
    doc.set_body_placement(a, at(18.0, 6.0, 15.0));
    join(
        &mut doc,
        "asm.path",
        a,
        &json!({"point": [18, 6, 15]}),
        b,
        &json!({"point": [20, 5, 10]}),
        json!({}),
    )
    .unwrap();
    let at = doc.body_placement(a).point([0.0; 3]);
    let joint = crate::joints(&doc)
        .into_iter()
        .find(|j| j.body == a)
        .unwrap();
    assert!(
        (at[0] - 20.0).abs() < 1e-3 && (at[2] - 10.0).abs() < 1e-3,
        "{at:?}"
    );
    assert_eq!(joint.feature.shape.len(), 3, "the whole L");

    let mut doc2 = doc.clone();
    let doc = &mut doc2;
    for id in crate::joints(doc).iter().map(|j| j.id).collect::<Vec<_>>() {
        doc.remove_feature(id).unwrap();
    }
    join(
        doc,
        "asm.cam",
        a,
        &json!({"point": [5, 5, 30]}),
        b,
        &json!({"point": [5, 5, 10]}),
        json!({"radius": 2}),
    )
    .unwrap();
    // The follower was picked at (5, 5, 30) where the body stood.
    let cam = crate::joints(doc)
        .into_iter()
        .find(|j| j.body == a)
        .unwrap();
    let Anchor::Point { point } = cam.feature.moving else {
        panic!()
    };
    let follower = doc.body_placement(a).point(point);
    assert!((follower[2] - 12.0).abs() < 1e-3, "{follower:?}");
}

/// A pin in a slot rides on the slot's line with four motions left; a
/// universal joint crosses two pins at a point with two.
#[test]
fn a_slot_and_a_universal_joint_leave_what_they_should() {
    let motions = |doc: &Document, body: BodyId| {
        crate::freedom(doc)
            .into_iter()
            .find(|(b, _)| *b == body)
            .map(|(_, m)| m.len())
            .unwrap()
    };
    let (mut doc, a, b) = two();
    join(
        &mut doc,
        "asm.slot",
        a,
        &json!({"point": [0, 0, 5]}),
        b,
        &json!({"axis": {"point": [10, 0, 0], "direction": [1, 0, 0]}}),
        json!({}),
    )
    .unwrap();
    let pin = doc.body_placement(a).point([0.0, 0.0, 5.0]);
    assert!(pin[1].abs() < 1e-3 && pin[2].abs() < 1e-3, "{pin:?}");
    assert_eq!(motions(&doc, a), 4);

    let (mut doc, a, b) = two();
    doc.set_body_placement(a, at(3.0, 2.0, 1.0));
    join(
        &mut doc,
        "asm.universal",
        a,
        &json!({"axis": {"point": [3, 2, 1], "direction": [1, 0, 0]}}),
        b,
        &json!({"axis": {"point": [0, 0, 0], "direction": [0, 1, 0]}}),
        json!({}),
    )
    .unwrap();
    let centre = doc.body_placement(a).point([0.0; 3]);
    assert!(glam::Vec3::from_array(centre).length() < 1e-3, "{centre:?}");
    assert_eq!(motions(&doc, a), 2);
}

/// An end moved along its normal moves the body with it: a mate with
/// the other end raised 3 sits 3 higher.
#[test]
fn an_end_offset_moves_the_body() {
    let (mut doc, a, b) = two();
    let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    let mate = join(
        &mut doc,
        "asm.mate",
        a,
        &json!({"point": [0, 0, 0], "normal": [0, 0, -1]}),
        b,
        &top,
        json!({}),
    )
    .unwrap();
    call(&mut doc, "asm.set", json!({"joint": mate, "fixed_end": 3})).unwrap();
    assert!((doc.body_placement(a).translation[2] - 3.0).abs() < 1e-3);
    call(&mut doc, "asm.set", json!({"joint": mate, "moving_end": 1})).unwrap();
    // The moving end, its underside, pushed 1 down its own normal.
    assert!((doc.body_placement(a).translation[2] - 4.0).abs() < 1e-3);
}

/// A ball joint puts two points together and leaves three turns free;
/// a distance holds between points or parallel axes as it does between
/// faces, and parallel takes axes.
#[test]
fn points_and_axes_are_held_as_faces_are() {
    let (mut doc, a, b) = two();
    let ball = json!({"centre": [0, 0, 20], "point": [0, 0, 25]});
    let socket = json!({"point": [5, 5, 5]});
    join(&mut doc, "asm.ball", a, &ball, b, &socket, json!({})).unwrap();
    let centre = doc.body_placement(a).point([0.0, 0.0, 20.0]);
    assert!(
        (centre[0] - 5.0).abs() < 1e-3
            && (centre[1] - 5.0).abs() < 1e-3
            && (centre[2] - 5.0).abs() < 1e-3,
        "{centre:?}"
    );
    let free = crate::freedom(&doc);
    let motions = &free.iter().find(|(body, _)| *body == a).unwrap().1;
    assert_eq!(motions.len(), 3, "three turns: {motions:?}");

    let (mut doc, a, b) = two();
    let axis = |x: f32| json!({"axis": {"point": [x, 0, 0], "direction": [0, 0, 1]}});
    join(
        &mut doc,
        "asm.parallel",
        a,
        &axis(0.0),
        b,
        &axis(0.0),
        json!({}),
    )
    .unwrap();
    join(
        &mut doc,
        "asm.distance",
        a,
        &axis(0.0),
        b,
        &axis(0.0),
        json!({"offset": 30}),
    )
    .unwrap();
    let at = doc.body_placement(a).point([0.0, 0.0, 0.0]);
    assert!(
        ((at[0] * at[0] + at[1] * at[1]).sqrt() - 30.0).abs() < 1e-3,
        "{at:?}"
    );

    let (mut doc, a, b) = two();
    join(
        &mut doc,
        "asm.distance",
        a,
        &json!({"point": [0, 0, 0]}),
        b,
        &json!({"point": [0, 0, 0]}),
        json!({"offset": 12}),
    )
    .unwrap();
    let at = doc.body_placement(a).point([0.0, 0.0, 0.0]);
    assert!(
        (glam::Vec3::from_array(at).length() - 12.0).abs() < 1e-3,
        "{at:?}"
    );
}

/// A group moves as one: the body held to its first member follows it
/// when a mate moves that one.
#[test]
fn a_rigid_group_moves_as_one() {
    let mut doc = Document::new("t");
    let [base, lid, knob] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    doc.set_body_placement(lid, at(0.0, 0.0, 30.0));
    doc.set_body_placement(knob, at(5.0, 0.0, 40.0));
    let group = call(
        &mut doc,
        "asm.group",
        json!({"bodies": [lid.0.to_string(), knob.0.to_string()]}),
    )
    .unwrap();
    assert!(group.is_string());
    // The lid's underside, 30 up, onto the base's top at 10.
    join(
        &mut doc,
        "asm.distance",
        lid,
        &json!({"point": [0, 0, 30], "normal": [0, 0, -1]}),
        base,
        &json!({"point": [0, 0, 10], "normal": [0, 0, 1]}),
        json!({"offset": 0}),
    )
    .unwrap();
    let lid_z = doc.body_placement(lid).translation[2];
    let knob_z = doc.body_placement(knob).translation[2];
    assert!((lid_z - 10.0).abs() < 1e-3, "{lid_z}");
    assert!(
        (knob_z - 20.0).abs() < 1e-3,
        "the knob keeps its 10 above: {knob_z}"
    );
    assert!(
        call(
            &mut doc,
            "asm.group",
            json!({"bodies": [lid.0.to_string()]})
        )
        .is_err()
    );
}

/// A rigid component moves as one whichever of its bodies a joint
/// holds, and the joints inside it rest until it is made flexible.
#[test]
fn a_rigid_component_moves_as_one_and_a_flexible_one_solves_inside() {
    let mut doc = Document::new("t");
    let [base, lid, knob] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    doc.set_body_placement(lid, at(0.0, 0.0, 30.0));
    doc.set_body_placement(knob, at(5.0, 0.0, 40.0));
    let component = doc.create_component("Lid".into(), None).unwrap();
    doc.set_body_component(lid, Some(component)).unwrap();
    doc.set_body_component(knob, Some(component)).unwrap();
    // The knob's underside, 40 up, onto the base's top at 10.
    join(
        &mut doc,
        "asm.distance",
        knob,
        &json!({"point": [5, 0, 40], "normal": [0, 0, -1]}),
        base,
        &json!({"point": [0, 0, 10], "normal": [0, 0, 1]}),
        json!({"offset": 0}),
    )
    .unwrap();
    let z = |doc: &Document, b| doc.body_placement(b).translation[2];
    assert!((z(&doc, knob) - 10.0).abs() < 1e-3, "{}", z(&doc, knob));
    assert!(
        (z(&doc, lid) - 0.0).abs() < 1e-3,
        "the lid follows: {}",
        z(&doc, lid)
    );
    // A joint inside the rigid component rests.
    join(
        &mut doc,
        "asm.distance",
        lid,
        &json!({"point": [0, 0, 0], "normal": [0, 0, 1]}),
        knob,
        &json!({"point": [5, 0, 10], "normal": [0, 0, -1]}),
        json!({"offset": 4}),
    )
    .unwrap();
    assert!((z(&doc, lid) - 0.0).abs() < 1e-3, "{}", z(&doc, lid));
    // Flexible, it holds: the lid's top 4 under the knob.
    let mut flexible = doc.component(component).unwrap().clone();
    flexible.flexible = true;
    doc.update_component(flexible).unwrap();
    let moves = crate::solve(&doc).unwrap();
    crate::place_bodies(&mut doc, &moves);
    assert!((z(&doc, knob) - 10.0).abs() < 1e-3, "{}", z(&doc, knob));
    assert!((z(&doc, lid) - 6.0).abs() < 1e-3, "{}", z(&doc, lid));
}

/// Components from commands: made, nested, flexible, emptied, taken
/// apart.
#[test]
fn components_are_made_nested_and_taken_apart() {
    let mut doc = Document::new("t");
    let [a, b, c] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    let id = |v: Value| ComponentId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap());
    let outer = id(call(
        &mut doc,
        "asm.component",
        json!({"bodies": [a.0.to_string()], "parent": null}),
    )
    .unwrap());
    assert_eq!(doc.component(outer).unwrap().name, "Component 1");
    let inner = id(call(
        &mut doc,
        "asm.component",
        json!({"bodies": [b.0.to_string(), c.0.to_string()], "parent": outer.0.to_string(),
               "flexible": true, "name": "Hinge"}),
    )
    .unwrap());
    assert_eq!(doc.component_bodies(outer), vec![a, b, c]);
    assert!(doc.component(inner).unwrap().flexible);
    // Inside itself is refused.
    assert!(
        call(
            &mut doc,
            "asm.component_set",
            json!({"component": outer.0.to_string(), "parent": inner.0.to_string()}),
        )
        .is_err()
    );
    call(
        &mut doc,
        "asm.component_set",
        json!({"component": inner.0.to_string(), "parent": null}),
    )
    .unwrap();
    assert_eq!(doc.component_bodies(outer), vec![a]);
    // "top" stands for null where a script cannot write one.
    call(
        &mut doc,
        "asm.component_set",
        json!({"component": inner.0.to_string(), "parent": outer.0.to_string()}),
    )
    .unwrap();
    assert_eq!(doc.component_bodies(outer), vec![a, b, c]);
    call(
        &mut doc,
        "asm.component_set",
        json!({"component": inner.0.to_string(), "parent": "top"}),
    )
    .unwrap();
    assert_eq!(doc.component_bodies(outer), vec![a]);
    call(
        &mut doc,
        "asm.component_add",
        json!({"bodies": [c.0.to_string()]}),
    )
    .unwrap();
    assert_eq!(doc.component_of(c), None);
    call(
        &mut doc,
        "asm.component_remove",
        json!({"component": inner.0.to_string()}),
    )
    .unwrap();
    assert_eq!(doc.component_of(b), None);
    assert_eq!(doc.components().len(), 1);
}

/// A joint that cannot hold with the others is not made; one already in
/// the document is listed as unmet, while one that holds is not, and it
/// holds up only the bodies joined to it: a joint between two other
/// bodies still solves.
#[test]
fn a_joint_that_cannot_hold_is_refused_or_unmet_and_blocks_nothing_else() {
    let mut doc = Document::new("t");
    let [a, b, c, d] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    let up = |z: f32| json!({"point": [0, 0, z], "normal": [0, 0, 1]});
    let down = |z: f32| json!({"point": [0, 0, z], "normal": [0, 0, -1]});
    // B's bottom 5 above A's top.
    join(
        &mut doc,
        "asm.distance",
        b,
        &down(0.0),
        a,
        &up(0.0),
        json!({"offset": 5}),
    )
    .unwrap();
    assert_eq!(call(&mut doc, "asm.unmet", json!({})).unwrap(), json!([]));
    let joints_before = crate::joints(&doc).len();
    // The same faces 9 apart as well: it cannot hold.
    let refused = join(
        &mut doc,
        "asm.distance",
        b,
        &down(0.0),
        a,
        &up(0.0),
        json!({"offset": 9}),
    );
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(crate::joints(&doc).len(), joints_before, "not left behind");
    // A conflicting joint written straight into the document, as an
    // old file might hold it.
    let mut conflicting = crate::joints(&doc)
        .into_iter()
        .find(|j| matches!(j.feature.kind, JointKind::Distance { .. }))
        .unwrap()
        .feature;
    conflicting.kind = JointKind::Distance { offset: 9.0 };
    let clash = doc
        .add_feature_in_body(conflicting, "Clash".into(), Some(b))
        .unwrap();
    let found = call(&mut doc, "asm.unmet", json!({})).unwrap();
    assert_eq!(
        found,
        json!([{"joint": clash.0.to_string(), "name": "Clash"}])
    );
    // The rest is still placed.
    doc.set_body_placement(d, at(0.0, 0.0, 40.0));
    let made = join(
        &mut doc,
        "asm.distance",
        d,
        &down(40.0),
        c,
        &up(0.0),
        json!({"offset": 3}),
    );
    assert!(made.is_ok(), "{made:?}");
    let z = doc.body_placement(d).translation[2] - doc.body_placement(c).translation[2];
    assert!((z - 3.0).abs() < 1e-3, "D's bottom 3 above C's top: {z}");
}

/// A joint to the world holds the body to the origin's planes, and
/// grounds nothing.
#[test]
fn a_body_mates_to_the_origin() {
    let mut doc = Document::new("t");
    let a = doc.create_body(None);
    doc.set_body_placement(a, at(3.0, 4.0, 25.0));
    let bottom = json!({"point": [3, 4, 25], "normal": [0, 0, -1]});
    let xy = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    join(
        &mut doc,
        "asm.mate",
        a,
        &bottom,
        crate::WORLD,
        &xy,
        json!({}),
    )
    .unwrap();
    let t = doc.body_placement(a).translation;
    assert!(t[2].abs() < 1e-3, "on the XY plane: {t:?}");
    assert!(
        crate::joints(&doc)
            .iter()
            .all(|j| j.feature.kind != JointKind::Ground),
        "the origin needs no ground"
    );
}

/// A joint made another kind keeps its faces and name; faces of the
/// wrong sort for the kind asked are refused, and new faces go in.
#[test]
fn a_joint_changes_kind_and_faces() {
    let (mut doc, a, b) = two();
    let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
    let mate = join(&mut doc, "asm.mate", a, &top, b, &top, json!({})).unwrap();
    let id = feature(&mate);
    let kind = |doc: &Document| {
        serde_json::from_value::<JointFeature>(doc.get_feature_data(id).unwrap().clone())
            .unwrap()
            .kind
    };
    call(
        &mut doc,
        "asm.set",
        json!({"joint": mate, "kind": "parallel"}),
    )
    .unwrap();
    assert_eq!(kind(&doc), JointKind::Parallel);
    assert!(call(&mut doc, "asm.set", json!({"joint": mate, "kind": "hinge"})).is_err());
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    call(
        &mut doc,
        "asm.set",
        json!({"joint": mate, "kind": "hinge", "face": pin, "other_face": pin}),
    )
    .unwrap();
    assert!(matches!(kind(&doc), JointKind::Hinge { .. }));
    assert_eq!(doc.get_feature_meta(id).unwrap().name, "Mate 1");
}

/// An alignment's turn and slide are each driven, and an alignment
/// stored as a plain word still loads, both free.
#[test]
fn an_alignment_drives_its_turn_and_its_slide() {
    let (mut doc, a, b) = two();
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    let joint = join(
        &mut doc,
        "asm.align",
        a,
        &pin,
        b,
        &pin,
        json!({"slide_drive": 5}),
    )
    .unwrap();
    assert!((doc.body_placement(a).translation[2] - 5.0).abs() < 1e-3);
    call(
        &mut doc,
        "asm.set",
        json!({"joint": joint, "turn_drive": 30}),
    )
    .unwrap();
    let turned = turned_deg(&doc, a);
    assert!((turned - 30.0).abs() < 1e-2, "{turned}");
    assert!((doc.body_placement(a).translation[2] - 5.0).abs() < 1e-3);
    let free = crate::freedom(&doc);
    let motions = &free.iter().find(|(body, _)| *body == a).unwrap().1;
    assert!(motions.is_empty(), "both held: {motions:?}");

    let old = json!({
        "kind": "Align",
        "moving": {"Axis": {"point": [0.0, 0.0, 0.0], "direction": [0.0, 0.0, 1.0]}},
        "other_body": b.0.to_string(),
        "fixed": {"Axis": {"point": [0.0, 0.0, 0.0], "direction": [0.0, 0.0, 1.0]}},
    });
    let read: JointFeature = serde_json::from_value(old).expect("an old alignment loads");
    assert_eq!(read.kind, JointKind::align());
}

#[test]
fn a_hinge_is_driven_to_an_angle_and_kept_within_its_limits() {
    let (mut doc, a, b) = two();
    let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
    let joint = join(&mut doc, "asm.hinge", a, &pin, b, &pin, json!({})).unwrap();
    assert!(travel(&mut doc, &joint).abs() < 1e-6, "made where it sits");
    call(&mut doc, "asm.set", json!({"joint": joint, "drive": 30})).unwrap();
    assert!((travel(&mut doc, &joint) - 30.0).abs() < 1e-3);
    let turned = turned_deg(&doc, a);
    assert!((turned - 30.0).abs() < 1e-2, "{turned}");
    // Let go and limited below where it is: it turns back to the limit.
    call(
        &mut doc,
        "asm.set",
        json!({"joint": joint, "drive": false, "limits": [-10, 10]}),
    )
    .unwrap();
    assert!((travel(&mut doc, &joint) - 10.0).abs() < 1e-2);
    // At its limit it may still turn, back the other way.
    let motions = &crate::freedom(&doc)[0].1;
    assert_eq!(motions.len(), 1);
    assert!(
        matches!(motions[0], crate::Motion::Turn { at_limit: true, .. }),
        "{motions:?}"
    );
    assert!(motions[0].describe().ends_with("(one way, at its limit)"));
    call(
        &mut doc,
        "asm.set",
        json!({"joint": joint, "limits": [-45, 45]}),
    )
    .unwrap();
    assert!(
        matches!(
            crate::freedom(&doc)[0].1[0],
            crate::Motion::Turn {
                at_limit: false,
                ..
            }
        ),
        "well within its limits"
    );
    let bad = call(
        &mut doc,
        "asm.set",
        json!({"joint": joint, "limits": [5, -5]}),
    );
    assert!(bad.is_err());
}

#[test]
fn a_slider_is_driven_along_its_axis() {
    let (mut doc, a, b) = two();
    let rail = json!({"axis": {"point": [0, 0, 0], "direction": [1, 0, 0]}});
    let joint = join(
        &mut doc,
        "asm.slider",
        a,
        &rail,
        b,
        &rail,
        json!({"drive": 12.5}),
    )
    .unwrap();
    assert!((doc.body_placement(a).translation[0] - 12.5).abs() < 1e-3);
    assert!(crate::freedom(&doc)[0].1.is_empty(), "driven: nothing left");
    assert!((travel(&mut doc, &joint) - 12.5).abs() < 1e-3);
}

#[test]
fn a_fixed_joint_needs_no_faces_and_carries_the_body_along() {
    let (mut doc, a, b) = two();
    call(
        &mut doc,
        "asm.move",
        json!({"body": a.0.to_string(), "by": [5, 6, 7]}),
    )
    .unwrap();
    call(
        &mut doc,
        "asm.fix",
        json!({"body": a.0.to_string(), "other": b.0.to_string()}),
    )
    .unwrap();
    call(
        &mut doc,
        "asm.move",
        json!({"body": b.0.to_string(), "by": [100, 0, 0]}),
    )
    .unwrap();
    call(&mut doc, "asm.solve", json!({})).unwrap();
    let at = doc.body_placement(a).translation;
    assert!(
        (at[0] - 105.0).abs() < 1e-3 && (at[1] - 6.0).abs() < 1e-3,
        "{at:?}"
    );
}

#[test]
fn a_move_turns_about_a_point_and_steps() {
    let mut doc = Document::new("t");
    let a = doc.create_body(None);
    call(
        &mut doc,
        "asm.move",
        json!({"body": a.0.to_string(), "turn": 90, "about": [10, 0, 0], "by": [0, 0, 5]}),
    )
    .unwrap();
    let placement = doc.body_placement(a);
    // The origin turned a quarter about (10, 0, 0) lands on (10, -10, 0).
    let p = placement.point([0.0, 0.0, 0.0]);
    assert!(
        (p[0] - 10.0).abs() < 1e-4 && (p[1] + 10.0).abs() < 1e-4 && (p[2] - 5.0).abs() < 1e-4,
        "{p:?}"
    );
    let listed = call(&mut doc, "asm.placement", json!({"body": a.0.to_string()})).unwrap();
    assert_eq!(listed["translation"][2], json!(5.0));
}

/// `ORDER` names every registered command but the joint tools' once and
/// nothing else, so a new command takes its place in the scripting reference
/// on purpose.
#[test]
fn the_order_names_every_command() {
    let mut registered = core_document::WorkbenchContext::default();
    register(&mut registered);
    let joints: Vec<&str> = crate::joint::JointTool::ALL
        .iter()
        .map(|t| t.command())
        .collect();
    let mut ids: Vec<&str> = registered
        .commands()
        .iter()
        .map(|c| c.id.as_str())
        .filter(|id| !joints.contains(id))
        .collect();
    let mut listed = ORDER.to_vec();
    ids.sort_unstable();
    listed.sort_unstable();
    assert_eq!(ids, listed);
}

#[test]
fn every_section_of_the_joints_prose_names_a_command() {
    let bench = &mut crate::AssemblyWorkbench::default();
    core_document::testing::assert_prose_names_commands(bench, joints::prose());
}
