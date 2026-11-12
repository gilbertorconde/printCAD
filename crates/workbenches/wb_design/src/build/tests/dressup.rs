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
