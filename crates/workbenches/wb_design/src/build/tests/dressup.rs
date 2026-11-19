//! The plan tests of drafts and picked edges.

use super::*;
use crate::build::dressup::edge_selection;
use kernel_api::EdgeSelection;

#[test]
fn picked_edges_reach_the_kernel_as_probe_points() {
    let picks = vec![
        crate::feature::EdgePick {
            faces: [0, 0],
            point: [1.0, 2.0, 3.0],
            direction: [1.0, 0.0, 0.0],
        },
        crate::feature::EdgePick {
            faces: [0, 0],
            point: [4.0, 5.0, 6.0],
            direction: [0.0, 1.0, 0.0],
        },
    ];
    match edge_selection(&crate::feature::EdgeSel::Edges(picks)).unwrap() {
        EdgeSelection::Picked(probes) => {
            let points: Vec<[f64; 3]> = probes.iter().map(|p| p.point).collect();
            assert_eq!(points, vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
            assert_eq!(probes[1].direction, [0.0, 1.0, 0.0]);
        }
        other => panic!("picked edges map to probes, not {other:?}"),
    }
}

#[test]
fn a_draft_stands_on_a_plane_and_pulls_along_a_datum_line() {
    use crate::feature::{PlaneTarget, PullRef};
    use core_document::{BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let (mut doc, body, sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch, 5.0), "Pad");
    let line = doc
        .add_feature_in_body(
            DatumFeature {
                shape: DatumShape::Line { length: 10.0 },
                attachment: DatumAttachment::BasePlane(BasePlane::YZ),
                offset: Default::default(),
            },
            "Line".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        DesignFeature::Draft {
            angle_deg: 5.0,
            neutral: FacePick {
                point: [0.0; 3],
                normal: [1.0, 0.0, 0.0],
                name: 0,
            },
            faces: vec![FacePick {
                point: [0.0, 2.5, 2.5],
                normal: [-1.0, 0.0, 0.0],
                name: 0,
            }],
            reversed: false,
            neutral_plane: Some(PlaneTarget::Base(BasePlane::XY)),
            pull: Some(PullRef::Datum(line)),
        },
        "Draft".into(),
        Some(body),
    )
    .unwrap();
    let ops = body_build_ops(&doc, body).unwrap().ops;
    let Some(SolidOp::Draft {
        neutral_normal,
        pull_dir,
        ..
    }) = ops.iter().find(|op| matches!(op, SolidOp::Draft { .. }))
    else {
        panic!("{ops:?}");
    };
    assert_eq!(*neutral_normal, [0.0, 0.0, 1.0]);
    // The YZ plane's line runs along its x-axis, the world's y.
    assert_eq!(pull_dir.map(|d| d.map(f64::abs)), Some([0.0, 1.0, 0.0]));
}

/// A chamfer's second size and angle and a thickness's value are checked
/// when the feature builds, as a script may set them past the panel's
/// limits.
#[test]
fn a_chamfer_s_second_size_and_angle_and_a_thickness_are_checked() {
    let chamfer = |fields: serde_json::Value| {
        let mut value = serde_json::json!({"Chamfer": {"size": 1.0}});
        value["Chamfer"]
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        crate::build::dressup::edge_op(&DesignFeature::from_json(&value).unwrap())
    };
    let two = |size2: f32| chamfer(serde_json::json!({"mode": "TwoDistances", "size2": size2}));
    let angled = |deg: f32| chamfer(serde_json::json!({"mode": "DistanceAngle", "angle_deg": deg}));
    assert!(two(0.0).unwrap_err().contains("second size"));
    assert!(two(0.5).is_ok());
    for deg in [0.0, 90.0, -10.0] {
        assert!(angled(deg).unwrap_err().contains("angle"), "{deg}");
    }
    assert!(angled(45.0).is_ok());

    let (mut doc, body, sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch, 5.0), "Pad");
    let shell = DesignFeature::Thickness {
        value: 0.0,
        faces: vec![FacePick {
            point: [0.0, 0.0, 5.0],
            normal: [0.0, 0.0, 1.0],
            name: 0,
        }],
        inward: true,
        join: Default::default(),
        both_sides: false,
    };
    add(&mut doc, body, shell, "Thickness");
    let error = body_build_ops(&doc, body).unwrap_err().message;
    assert!(error.contains("thickness must be positive"), "{error}");
}
