//! Arrays of linked copies: each occurrence a copy the array owns, placed
//! by the pattern from where the source sits; an edit is one op and one
//! undo step, whatever copies it makes or takes away; the ops replay.

use core_document::history::OpJournal;
use core_document::{ArrayPattern, BodyId, BodyPlacement, Document};
use glam::{Quat, Vec3};

use crate::support::{assert_replays, geometry, reloaded, triangle};

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
}

fn row(spacing: f32) -> ArrayPattern {
    ArrayPattern::Linear {
        direction: [1.0, 0.0, 0.0],
        spacing,
    }
}

/// A body with a shape and a snapshot.
fn part(doc: &mut Document) -> BodyId {
    let body = doc.create_body(Some("Peg".into()));
    doc.set_imported_geometry(body, geometry(triangle()));
    doc.set_imported_brep_data(body, b"ogeom peg".to_vec(), Vec::new());
    body
}

/// Where each copy's first corner is drawn, in occurrence order.
fn corners(doc: &Document, array: core_document::ArrayId) -> Vec<[f32; 3]> {
    doc.copy_array(array)
        .unwrap()
        .instances
        .iter()
        .map(|i| doc.imported_geometry(i.body).unwrap().mesh.positions[0])
        .collect()
}

#[test]
fn a_row_places_its_copies_along_the_direction_and_follows_the_source() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(20.0), 4).unwrap();
    let a = doc.copy_array(array).unwrap().clone();
    assert_eq!(a.instances.len(), 3, "the source and three copies");
    assert_eq!(
        a.instances.iter().map(|i| i.index).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    for i in &a.instances {
        assert_eq!(doc.copy_source(i.body), Some(source));
        assert_eq!(
            doc.imported_brep_blob(i.body),
            Some(&b"ogeom peg"[..]),
            "one shape, which the parts list counts as one part"
        );
    }
    let at = corners(&doc, array);
    assert!(close(at[0], [20.0, 0.0, 0.0]) && close(at[2], [60.0, 0.0, 0.0]));

    doc.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(0.0, 5.0, 0.0)),
    );
    assert!(
        close(corners(&doc, array)[1], [40.0, 5.0, 0.0]),
        "moving the source carries the row"
    );
}

#[test]
fn a_ring_turns_its_copies_about_the_axis() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    doc.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0)),
    );
    let ring = ArrayPattern::Polar {
        center: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        angle: 90.0,
    };
    let array = doc.create_copy_array(source, ring, 4).unwrap();
    let at = corners(&doc, array);
    assert!(close(at[0], [0.0, 10.0, 0.0]), "{at:?}");
    assert!(close(at[1], [-10.0, 0.0, 0.0]), "{at:?}");
    assert!(close(at[2], [0.0, -10.0, 0.0]), "{at:?}");
    let turned = doc.body_placement(doc.copy_array(array).unwrap().instances[0].body);
    assert!(
        close(turned.direction([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]),
        "each copy turned with its place"
    );
}

#[test]
fn a_new_count_is_one_op_and_one_undo_step() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(16);
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(10.0), 3).unwrap();
    journal.note(&mut doc);
    let first = doc.copy_array(array).unwrap().clone();
    let _ = doc.take_pending_ops();

    doc.set_copy_array(array, row(12.0), 6, Vec::new()).unwrap();
    assert_eq!(doc.take_pending_ops().len(), 1, "one op for the whole edit");
    journal.note(&mut doc);
    assert_eq!(doc.bodies().len(), 6);
    let grown = doc.copy_array(array).unwrap().clone();
    assert_eq!(
        &grown.instances[..2],
        &first.instances[..],
        "the copies it had are kept"
    );
    assert!(close(corners(&doc, array)[4], [60.0, 0.0, 0.0]));

    journal.undo(&mut doc).expect("undoes");
    assert_eq!(doc.bodies().len(), 3, "the three copies it made are gone");
    assert_eq!(doc.copy_array(array), Some(&first));
    assert!(
        close(corners(&doc, array)[1], [20.0, 0.0, 0.0]),
        "placed back"
    );

    journal.redo(&mut doc).expect("redoes");
    assert_eq!(doc.copy_array(array), Some(&grown), "the same copies again");

    doc.set_copy_array(array, row(12.0), 2, Vec::new()).unwrap();
    journal.note(&mut doc);
    assert_eq!(doc.bodies().len(), 2);
    journal.undo(&mut doc).expect("undoes");
    assert_eq!(doc.bodies().len(), 6, "fewer, then as many again");
    assert!(
        doc.copy_array(array)
            .unwrap()
            .instances
            .iter()
            .all(|i| doc.imported_geometry(i.body).is_some()),
        "each copy made again takes the source's shape"
    );
}

#[test]
fn deleting_a_copy_skips_it_and_undoes() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(16);
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(10.0), 4).unwrap();
    journal.note(&mut doc);
    let middle = doc.copy_array(array).unwrap().instances[1].clone();
    assert!(doc.remove_body(middle.body));
    journal.note(&mut doc);
    let a = doc.copy_array(array).unwrap();
    assert_eq!(a.skipped, [2]);
    assert_eq!(a.count, 4, "still four occurrences, one left out");
    assert!(doc.bodies().iter().all(|b| b.id != middle.body));
    journal.undo(&mut doc).expect("undoes");
    assert!(doc.bodies().iter().any(|b| b.id == middle.body), "back");
    assert!(doc.copy_array(array).unwrap().skipped.is_empty());

    assert!(doc.remove_copy_array(array));
    assert_eq!(doc.bodies().len(), 1, "the source stays alone");
}

#[test]
fn array_edits_replay_and_survive_a_save() {
    let mut live = Document::new("t");
    let source = part(&mut live);
    let baseline = live.clone();
    let _ = live.take_pending_ops();

    let array = live.create_copy_array(source, row(10.0), 3).unwrap();
    live.set_copy_array(array, row(15.0), 5, vec![3]).unwrap();
    live.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, 2.0)),
    );
    let ops = live.take_pending_ops();
    assert_replays(&live, baseline, &ops);

    let loaded = reloaded(&mut live);
    assert_eq!(loaded.copy_array(array), live.copy_array(array));
    assert!(close(corners(&loaded, array)[2], [60.0, 0.0, 2.0]));
}

#[test]
fn an_array_s_copy_is_not_a_source_and_counts_are_bounded() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    let array = doc.create_copy_array(source, row(10.0), 2).unwrap();
    let copy = doc.copy_array(array).unwrap().instances[0].body;
    assert!(doc.create_copy_array(copy, row(10.0), 2).is_err());
    assert!(doc.set_copy_array(array, row(10.0), 0, Vec::new()).is_err());
    let flat = ArrayPattern::Linear {
        direction: [0.0; 3],
        spacing: 5.0,
    };
    assert_eq!(
        doc.set_copy_array(array, flat, 3, Vec::new()),
        Err(core_document::ArrayError::BadPattern),
        "a zero direction"
    );
}

/// Three stations a quarter turn apart round the circle of radius 10
/// about the origin, each running the way the circle turns.
fn round_path(align: bool) -> ArrayPattern {
    let station =
        |point: [f32; 3], tangent: [f32; 3]| core_document::PathStation { point, tangent };
    ArrayPattern::Path {
        along: core_document::PathSource::Edge {
            body: BodyId::new(),
            point: [0.0; 3],
        },
        stations: vec![
            station([10.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            station([0.0, 10.0, 0.0], [-1.0, 0.0, 0.0]),
            station([-10.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
        ],
        spacing: None,
        align,
        normal: Some([0.0, 0.0, 1.0]),
    }
}

#[test]
fn a_path_array_places_copies_at_its_stations_turned_when_aligned_and_replays() {
    let mut live = Document::new("t");
    let source = part(&mut live);
    live.set_body_placement(
        source,
        BodyPlacement::new(Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0)),
    );
    let baseline = live.clone();
    let _ = live.take_pending_ops();

    let array = live
        .create_copy_array(source, round_path(false), 3)
        .unwrap();
    // The source's corner at (10, 0): the copies' at the stations, kept
    // square to the axes.
    assert!(close(corners(&live, array)[1], [-10.0, 0.0, 0.0]));
    let mesh = |doc: &Document, i: usize| {
        let body = doc.copy_array(array).unwrap().instances[i].body;
        doc.imported_geometry(body).unwrap().mesh.positions.clone()
    };
    assert!(
        close(mesh(&live, 0)[1], [1.0, 10.0, 0.0]),
        "moved, not turned"
    );

    live.set_copy_array(array, round_path(true), 3, Vec::new())
        .unwrap();
    assert!(
        close(mesh(&live, 0)[1], [0.0, 11.0, 0.0]),
        "turned with the path"
    );
    assert!(close(mesh(&live, 1)[1], [-11.0, 0.0, 0.0]), "half a turn");
    assert_eq!(
        live.set_copy_array(array, round_path(true), 4, Vec::new()),
        Err(core_document::ArrayError::TooFewPlaces { places: 3 }),
        "a station per occurrence"
    );

    let ops = live.take_pending_ops();
    assert_replays(&live, baseline, &ops);
}

#[test]
fn a_component_array_copies_every_body_together_and_skips_whole_occurrences() {
    let mut live = Document::new("t");
    let peg = part(&mut live);
    let cap = part(&mut live);
    live.set_body_placement(cap, BodyPlacement::new(Quat::IDENTITY, Vec3::Z * 5.0));
    let unit = live.create_component("Pin".into(), None).unwrap();
    live.set_body_component(peg, Some(unit)).unwrap();
    live.set_body_component(cap, Some(unit)).unwrap();
    let baseline = live.clone();
    let _ = live.take_pending_ops();

    let array = live
        .create_component_array(unit, row(20.0), 3)
        .expect("an array of the component");
    assert_eq!(live.bodies().len(), 6, "two copies of each body");
    let second = live.copy_array(array).unwrap().instances[1].clone();
    assert_eq!(second.members.len(), 1);
    let cap_copy = second.members[0].body;
    let drawn = |doc: &Document, body| doc.imported_geometry(body).unwrap().mesh.positions[0];
    assert!(
        close(drawn(&live, cap_copy), [40.0, 0.0, 5.0]),
        "the cap's own offset kept"
    );
    let group = second.group.expect("each occurrence held together");
    assert_eq!(live.component_bodies(group), vec![second.body, cap_copy]);
    assert!(!live.component(group).unwrap().flexible);

    // Moving a body of the component carries its copies.
    live.set_body_placement(cap, BodyPlacement::new(Quat::IDENTITY, Vec3::Z * 7.0));
    assert!(close(drawn(&live, cap_copy), [40.0, 0.0, 7.0]));

    // Deleting one copy skips its whole occurrence.
    assert!(live.skip_array_instance(cap_copy));
    let a = live.copy_array(array).unwrap();
    assert_eq!(a.skipped, vec![2]);
    assert!(live.bodies().iter().all(|b| b.id != second.body));
    assert!(live.component(group).is_none());
    assert_eq!(live.array_instance(cap_copy), None);

    let ops = live.take_pending_ops();
    assert_replays(&live, baseline, &ops);
    let empty = live.create_component("Empty".into(), None).unwrap();
    assert_eq!(
        live.create_component_array(empty, row(5.0), 2),
        Err(core_document::ArrayError::EmptyComponent)
    );
}

#[test]
fn a_point_array_stands_a_copy_at_each_point_after_the_first() {
    let mut doc = Document::new("t");
    let source = part(&mut doc);
    let points = ArrayPattern::Points {
        from: core_document::FeatureId(uuid::Uuid::new_v4()),
        points: vec![[5.0, 5.0, 0.0], [25.0, 5.0, 0.0], [5.0, 35.0, 3.0]],
    };
    let array = doc.create_copy_array(source, points, 3).unwrap();
    let at = corners(&doc, array);
    assert!(close(at[0], [20.0, 0.0, 0.0]) && close(at[1], [0.0, 30.0, 3.0]));
}
