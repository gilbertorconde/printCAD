//! Couplings tie two joints' motions: gears, a belt, a rack and pinion, a
//! screw. A grounded frame carries two gears on hinges 30 mm apart and a
//! rack on a slider.

use std::collections::HashMap;

use core_document::{BodyId, BodyPlacement, Document, FeatureId, WorkbenchFeature};
use glam::Vec3;
use wb_assembly::{
    Anchor, Coupling, Drive, Gearing, JointFeature, JointKind, Rigid, counted_couplings, freedom,
    joints, solve,
};

struct Scene {
    doc: Document,
    first: BodyId,
    second: BodyId,
    first_hinge: FeatureId,
    second_hinge: FeatureId,
    rack_slider: FeatureId,
}

fn axis(point: [f32; 3], direction: [f32; 3]) -> Anchor {
    Anchor::Axis { point, direction }
}

fn hinge_on(doc: &mut Document, body: BodyId, frame: BodyId, at: [f32; 3]) -> FeatureId {
    let joint = JointFeature {
        second: None,
        shape: Vec::new(),
        ends: [0.0; 2],
        names: [0; 2],
        kind: JointKind::Hinge {
            offset: 0.0,
            zero: glam::DQuat::IDENTITY.to_array(),
            drive: Drive::default(),
        },
        moving: axis([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        other_body: frame,
        fixed: axis(at, [0.0, 0.0, 1.0]),
    };
    doc.add_feature_in_body(joint, "Hinge".into(), Some(body))
        .unwrap()
}

fn scene() -> Scene {
    let mut doc = Document::new("gears");
    let frame = doc.create_body(None);
    let first = doc.create_body(None);
    let second = doc.create_body(None);
    let rack = doc.create_body(None);
    doc.set_body_placement(
        second,
        BodyPlacement::new(glam::Quat::IDENTITY, Vec3::new(30.0, 0.0, 0.0)),
    );
    doc.set_body_placement(
        rack,
        BodyPlacement::new(glam::Quat::IDENTITY, Vec3::new(0.0, -10.0, 0.0)),
    );
    let ground = Anchor::Plane {
        point: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
    };
    doc.add_feature_in_body(
        JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: JointKind::Ground,
            moving: ground,
            other_body: frame,
            fixed: ground,
        },
        "Ground".into(),
        Some(frame),
    )
    .unwrap();
    let first_hinge = hinge_on(&mut doc, first, frame, [0.0, 0.0, 0.0]);
    let second_hinge = hinge_on(&mut doc, second, frame, [30.0, 0.0, 0.0]);
    let rack_slider = doc
        .add_feature_in_body(
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Slider {
                    turn: glam::DQuat::IDENTITY.to_array(),
                    drive: Drive::default(),
                },
                moving: axis([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
                other_body: frame,
                fixed: axis([0.0, -10.0, 0.0], [1.0, 0.0, 0.0]),
            },
            "Slider".into(),
            Some(rack),
        )
        .unwrap();
    Scene {
        doc,
        first,
        second,
        first_hinge,
        second_hinge,
        rack_slider,
    }
}

fn placements(doc: &Document) -> HashMap<BodyId, Rigid> {
    doc.bodies()
        .iter()
        .map(|b| (b.id, Rigid::from(b.placement)))
        .collect()
}

fn couple(
    doc: &mut Document,
    gearing: Gearing,
    driver: FeatureId,
    driven: FeatureId,
    ratio: f32,
) -> FeatureId {
    let all = joints(doc);
    let find = |id| all.iter().find(|j| j.id == id).unwrap();
    let coupling = Coupling::new(
        gearing,
        find(driver),
        find(driven),
        ratio,
        false,
        &placements(doc),
    )
    .expect("the joints can be tied");
    let body = find(driven).body;
    doc.add_feature_in_body(coupling, gearing.label().into(), Some(body))
        .unwrap()
}

/// Solve and place, the couplings counting their drivers' turns, as the
/// bench does.
fn apply(doc: &mut Document) {
    let moves = solve(doc).expect("every joint holds");
    for (id, coupling) in counted_couplings(doc, &moves) {
        doc.update_feature_data(id, coupling.to_json()).unwrap();
    }
    for (body, placement) in moves {
        doc.set_body_placement(body, placement);
    }
}

fn drive(doc: &mut Document, joint: FeatureId, to: f32) {
    let mut feature = JointFeature::from_json(doc.get_feature_data(joint).unwrap()).unwrap();
    if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = &mut feature.kind {
        drive.to = Some(to);
    }
    doc.update_feature_data(joint, feature.to_json()).unwrap();
}

fn travel(doc: &Document, joint: FeatureId) -> f64 {
    let j = joints(doc).into_iter().find(|j| j.id == joint).unwrap();
    let at = placements(doc);
    j.feature
        .travel(&at[&j.body], &at[&j.feature.other_body])
        .unwrap()
}

#[test]
fn a_coupling_is_made_where_the_joints_stand_and_moves_nothing() {
    let mut s = scene();
    apply(&mut s.doc);
    couple(
        &mut s.doc,
        Gearing::Gears,
        s.first_hinge,
        s.second_hinge,
        0.5,
    );
    assert!(solve(&s.doc).unwrap().is_empty(), "nothing moves");
}

/// Meshed gears turn the other way by their ratio, a belt turns the same
/// way, and a pinion moves its rack by its pitch circle.
#[test]
fn a_driven_joint_moves_by_its_coupling_s_ratio() {
    let quarter = std::f64::consts::FRAC_PI_2 * 10.0;
    for (gearing, ratio, to, moved) in [
        (Gearing::Gears, 0.5, 90.0, -45.0),
        (Gearing::Belt, 2.0, 30.0, 60.0),
        (Gearing::RackAndPinion, 10.0, 90.0, quarter),
    ] {
        let mut s = scene();
        let driven = if gearing == Gearing::RackAndPinion {
            s.rack_slider
        } else {
            s.second_hinge
        };
        couple(&mut s.doc, gearing, s.first_hinge, driven, ratio);
        let start = travel(&s.doc, driven);
        drive(&mut s.doc, s.first_hinge, to);
        apply(&mut s.doc);
        let driver = travel(&s.doc, s.first_hinge);
        assert!(
            (driver - f64::from(to)).abs() < 1e-3,
            "{gearing:?}: the driver at {driver}"
        );
        let change = travel(&s.doc, driven) - start;
        assert!(
            (change - moved).abs() < 1e-2,
            "{gearing:?}: the driven joint moved {change}"
        );
        // Only the turns moved: the second gear stays on its axis.
        let at = s.doc.body_placement(s.second).offset();
        assert!(
            (at - Vec3::new(30.0, 0.0, 0.0)).length() < 1e-3,
            "{gearing:?}: {at:?}"
        );
    }
}

#[test]
fn a_whole_turn_of_the_driver_is_counted_through_the_wrap() {
    let mut s = scene();
    let id = couple(
        &mut s.doc,
        Gearing::Gears,
        s.first_hinge,
        s.second_hinge,
        0.5,
    );
    // The driver goes round once in steps; its angle wraps at a half turn.
    for step in 1..=12 {
        drive(&mut s.doc, s.first_hinge, (step * 30) as f32);
        apply(&mut s.doc);
    }
    let coupling = Coupling::from_json(s.doc.get_feature_data(id).unwrap()).unwrap();
    assert_eq!(coupling.turns, 1, "one whole turn of the driver");
    // Half a turn on at 1:2, not back where it started.
    let second = travel(&s.doc, s.second_hinge);
    assert!(
        (second.abs() - 180.0).abs() < 1e-2,
        "the driven gear half a turn round: {second}"
    );
}

#[test]
fn the_driven_body_follows_and_the_driver_keeps_its_turn() {
    let mut s = scene();
    couple(
        &mut s.doc,
        Gearing::Gears,
        s.first_hinge,
        s.second_hinge,
        0.5,
    );
    let free = freedom(&s.doc);
    let motions = |body| {
        free.iter()
            .find(|(b, _)| *b == body)
            .map_or(0, |(_, m)| m.len())
    };
    assert_eq!(motions(s.first), 1, "the driver turns on its hinge");
    assert_eq!(motions(s.second), 0, "the driven gear follows");
}
