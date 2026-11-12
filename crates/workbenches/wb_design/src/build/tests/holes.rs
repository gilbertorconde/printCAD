//! The plan tests of holes: their sizes, seats, points and threads.

use super::*;
use crate::build::holes;

#[test]
fn hole_feature_emits_cut_circles() {
    let (mut doc, body, base_sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(base_sketch, 5.0), "Pad");

    let mut hole_sketch = Sketch::new("holes");
    for x in [2.0f64, 8.0] {
        let center =
            hole_sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 2.5))));
        hole_sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
    }
    let hole_sketch_id = add_sketch(&mut doc, body, hole_sketch);
    let counterbored = hole(hole_sketch_id, |h| {
        *h.depth = 4.0;
        *h.cut = HoleCut::Counterbore {
            diameter: 6.0,
            depth: 1.5,
        };
    });
    add(&mut doc, body, counterbored, "Hole");

    let plan = body_build_ops(&doc, body).unwrap();
    // Pad + hole drill + counterbore.
    assert_eq!(plan.ops.len(), 3);
    assert_eq!(plan.op_features[1], plan.op_features[2]);
    for op in &plan.ops[1..] {
        assert_eq!(boolean_of(op), BooleanOp::Cut);
        let SolidOp::Sweep { profile, .. } = op else {
            panic!("hole ops are sweeps");
        };
        assert_eq!(profile.wires.len(), 2, "one wire per hole center");
    }
}

/// An M6 hole 10 deep, its thread modeled `length` long or cleared
/// with `clearance`; the ops it makes.
fn m6_hole(
    threaded: bool,
    length: crate::feature::ThreadLength,
    clearance: Option<f32>,
) -> (DesignFeature, Vec<SolidOp>) {
    let (mut doc, body, base_sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(base_sketch, 20.0), "Pad");
    let sketch = points_sketch(&mut doc, body, &[(2.0, 2.5)]);
    let m6 = hole(sketch, |h| {
        *h.clearance = clearance;
        *h.thread_length = length;
        *h.depth = 10.0;
        *h.thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6"));
        *h.threaded = threaded;
        *h.modeled_thread = threaded;
        *h.thread_depth = 4.0;
    });
    let ops = hole_ops(&doc, &m6).unwrap();
    (m6, ops)
}

/// A 5 mm hole at `sketch`'s points showing an M6 thread, as `edit`
/// changes it.
fn cosmetic_m6(sketch: FeatureId, edit: impl FnOnce(&mut HoleFields)) -> DesignFeature {
    hole(sketch, |h| {
        *h.diameter = 5.0;
        *h.thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6"));
        *h.threaded = true;
        *h.cosmetic_thread = true;
        edit(h);
    })
}

/// The height of the thread's helix: its length and one pitch.
fn thread_run(ops: &[SolidOp]) -> f64 {
    ops.iter()
        .find_map(|op| match op {
            SolidOp::Sweep {
                kind: SweepKind::Helix { height, .. },
                ..
            } => Some(*height),
            _ => None,
        })
        .expect("a thread")
}

#[test]
fn a_modeled_thread_runs_as_long_as_asked() {
    use crate::feature::ThreadLength;
    // M6 coarse: pitch 1.
    let given = thread_run(&m6_hole(true, ThreadLength::Given, None).1);
    let whole = thread_run(&m6_hole(true, ThreadLength::HoleDepth, None).1);
    let run_out = thread_run(&m6_hole(true, ThreadLength::RunOut, None).1);
    assert!((given - 5.0).abs() < 1e-9, "{given}");
    assert!((whole - 11.0).abs() < 1e-9, "{whole}");
    assert!((run_out - 8.0).abs() < 1e-9, "{run_out}");
}

/// A threaded metric hole drills the table's tap size, a clearance hole
/// the fit's size, or a clearance of its own in the fit's place.
#[test]
fn a_metric_hole_takes_its_size_from_the_table_or_a_clearance_of_its_own() {
    use crate::feature::ThreadLength;
    let (tapped, _) = m6_hole(true, ThreadLength::Given, None);
    assert!((hole_diameter(&tapped) - 5.0).abs() < 1e-6, "M6 tap drill");
    let (normal, _) = m6_hole(false, ThreadLength::Given, None);
    assert!(
        (hole_diameter(&normal) - 6.6).abs() < 1e-4,
        "ISO 273 normal"
    );
    let (own, _) = m6_hole(false, ThreadLength::Given, Some(6.3));
    assert!((hole_diameter(&own) - 6.3).abs() < 1e-6);
}

/// A cosmetic M6 thread down a 10 mm hole: a helix per hole of the
/// thread's 1 mm pitch, ten turns, just inside the 5 mm tap bore, and
/// its designation in the hole's name; none without the switch.
#[test]
fn a_cosmetic_thread_draws_a_helix_down_the_bore() {
    let (mut doc, body, _) = doc_with_body_sketch();
    let holes = points_sketch(&mut doc, body, &[(3.0, 4.0)]);
    let mut hole = cosmetic_m6(holes, |h| *h.depth = 10.0);
    assert_eq!(hole.cosmetic_thread().as_deref(), Some("M6"));
    let lines = cosmetic_thread_lines(&doc, Some(body), &hole);
    assert_eq!(lines.edges.len() / 2, 10 * holes::THREAD_SEGMENTS_PER_TURN);
    let bore = f64::from(hole_diameter(&hole)) * 0.5;
    for p in &lines.positions {
        let r = (f64::from(p[0]) - 3.0).hypot(f64::from(p[1]) - 4.0);
        assert!(
            r < bore && r > bore * 0.95,
            "just inside the bore: {r} of {bore}"
        );
        assert!((-10.0..=0.0).contains(&p[2]), "down the hole: {}", p[2]);
    }
    set_hole(&mut hole, |h| *h.cosmetic_thread = false);
    assert!(
        cosmetic_thread_lines(&doc, Some(body), &hole)
            .positions
            .is_empty()
    );

    // The document keeps the thread once evaluated: its designation,
    // where its bore opens and which way it runs, until the hole
    // leaves the build.
    set_hole(&mut hole, |h| *h.cosmetic_thread = true);
    let id = add(&mut doc, body, hole.clone(), "Hole");
    let mut registry = core_document::DocumentService::default();
    registry
        .register_workbench(Box::new(crate::DesignWorkbench::default()))
        .unwrap();
    registry.evaluate(&mut doc);
    let threads = doc.cosmetic_threads();
    assert_eq!(threads.len(), 1, "{threads:?}");
    let thread = &threads[0];
    assert_eq!((thread.feature, thread.body), (id, body));
    assert_eq!(thread.label(), "M6 thread");
    assert_eq!(thread.mouth, [3.0, 4.0, 0.0]);
    assert_eq!(thread.axis, [0.0, 0.0, -1.0]);
    assert!((thread.radius - f64::from(hole_diameter(&hole)) * 0.5).abs() < 1e-9);
    doc.set_feature_suppressed(id, true);
    registry.evaluate(&mut doc);
    assert!(doc.cosmetic_threads().is_empty(), "a suppressed hole");
}

/// A mirror or a pattern threads its copies of a threaded hole, across
/// a mirror cut to the same hand; a skipped copy has none, and a mirror
/// of the whole body takes the hole too.
#[test]
fn copies_of_a_threaded_hole_are_threaded() {
    use crate::feature::MirrorPlane;
    let (mut doc, body, _) = doc_with_body_sketch();
    let holes = points_sketch(&mut doc, body, &[(3.0, 4.0)]);
    let hole = add(
        &mut doc,
        body,
        cosmetic_m6(holes, |h| *h.depth = 10.0),
        "Hole",
    );
    let mirror = |originals: Vec<FeatureId>, skipped: Vec<u32>| DesignFeature::Mirrored {
        originals,
        plane: MirrorPlane::YZ,
        refine: false,
        boolean_tolerance: None,
        skipped,
    };
    let copy = add(&mut doc, body, mirror(vec![hole], vec![]), "Mirrored");
    let bores = threaded_bores_of(&doc, body, copy);
    assert_eq!(bores.len(), 1, "{bores:?}");
    assert_eq!((bores[0].hole, bores[0].mouth), (hole, [-3.0, 4.0, 0.0]));
    assert_eq!(bores[0].into, [0.0, 0.0, -1.0]);
    // Which way each helix turns about its bore, looking down it.
    let hand = |lines: &kernel_api::TriMesh, centre: [f64; 2]| {
        let at = |i: usize| {
            let p = lines.positions[i];
            (f64::from(p[0]) - centre[0], f64::from(p[1]) - centre[1])
        };
        let ((ax, ay), (bx, by)) = (at(0), at(1));
        (ax * by - ay * bx).signum()
    };
    let own = cosmetic_thread_lines(
        &doc,
        Some(body),
        &DesignFeature::from_json(doc.feature_values(hole).unwrap()).unwrap(),
    );
    let copied = copied_thread_lines(&doc, body, copy);
    assert_eq!(copied.edges.len(), own.edges.len());
    assert_eq!(
        hand(&copied, [-3.0, 4.0]),
        hand(&own, [3.0, 4.0]),
        "the same hand"
    );

    let skipped = add(&mut doc, body, mirror(vec![hole], vec![1]), "Skipped");
    assert!(threaded_bores_of(&doc, body, skipped).is_empty());
    let whole = add(&mut doc, body, mirror(vec![], vec![]), "Whole body");
    let bores = threaded_bores_of(&doc, body, whole);
    assert!(
        bores.iter().any(|b| b.mouth == [-3.0, 4.0, 0.0])
            && bores.iter().any(|b| b.mouth == [3.0, 4.0, 0.0]),
        "the hole and the first mirror's copy, each mirrored: {bores:?}"
    );
}

/// A copy is threaded where the solid shows its bore: a rim of the
/// bore's radius round its mouth, not merely a solid face there.
#[test]
fn a_copy_is_threaded_where_the_solid_shows_its_bore() {
    let bore = Bore {
        hole: FeatureId(uuid::Uuid::nil()),
        mouth: [10.0, 0.0, 5.0],
        into: [0.0, 0.0, -1.0],
        across: [1.0, 0.0, 0.0],
    };
    let rim = |r: f32| kernel_api::TriMesh {
        positions: (0..12)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 12.0;
                [10.0 + r * a.cos(), r * a.sin(), 5.0]
            })
            .collect(),
        ..Default::default()
    };
    assert!(bore_drawn(&rim(1.25), &bore, 1.25));
    assert!(!bore_drawn(&rim(4.0), &bore, 1.25), "another round there");
    assert!(!bore_drawn(&kernel_api::TriMesh::default(), &bore, 1.25));
}

/// A cosmetic thread through all of an 8 mm plate stops at the plate's
/// far side, though the plate is far wider than it is thick: at the
/// bore's exit once the bore is cut, at the solid's far face along the
/// hole before, and a thread depth longer than the hole stops there too.
#[test]
fn a_cosmetic_thread_through_all_stops_where_the_hole_leaves_the_solid() {
    let (mut doc, body, _) = doc_with_body_sketch();
    let holes = points_sketch(&mut doc, body, &[(3.0, 4.0)]);
    let hole = |thread_depth: f32| {
        cosmetic_m6(holes, |h| {
            *h.depth = 0.0;
            *h.through_all = true;
            *h.thread_depth = thread_depth;
        })
    };
    let plate = |bored: bool| {
        let mut positions = Vec::new();
        for corner in 0..8 {
            positions.push([
                if corner & 1 == 0 { -50.0 } else { 50.0 },
                if corner & 2 == 0 { -50.0 } else { 50.0 },
                if corner & 4 == 0 { -8.0 } else { 0.0 },
            ]);
        }
        if bored {
            for k in 0..16 {
                let (sin, cos) = (std::f32::consts::TAU * k as f32 / 16.0).sin_cos();
                for z in [0.0, -8.0] {
                    positions.push([3.0 + 2.5 * cos, 4.0 + 2.5 * sin, z]);
                }
            }
        }
        core_document::ImportedGeometry {
            mesh: std::sync::Arc::new(kernel_api::TriMesh {
                positions,
                ..Default::default()
            }),
            revision: 1,
            bounds_mm: Some(([-50.0, -50.0, -8.0], [50.0, 50.0, 0.0])),
            ..bare_geometry(None)
        }
    };
    for bored in [true, false] {
        doc.set_imported_geometry(body, plate(bored));
        for thread_depth in [0.0, 30.0] {
            let lines = cosmetic_thread_lines(&doc, Some(body), &hole(thread_depth));
            assert!(!lines.positions.is_empty());
            let deepest = lines
                .positions
                .iter()
                .map(|p| p[2])
                .fold(f32::INFINITY, f32::min);
            assert!(
                (deepest + 8.0).abs() < 1e-3,
                "bored {bored}, thread depth {thread_depth}: ends at {deepest}, not -8"
            );
            assert!(lines.positions.iter().all(|p| p[2] <= 1e-4));
        }
    }
}

/// A size the table does not have is an error on the hole, whatever
/// its thread settings, never a panic.
#[test]
fn a_hole_of_a_size_the_table_lacks_fails_cleanly() {
    for (threaded, modeled_thread) in [(false, false), (true, false), (true, true)] {
        let error = hole_plan(|h| {
            *h.thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M999"));
            *h.threaded = threaded;
            *h.modeled_thread = modeled_thread;
            *h.thread_depth = 3.0;
        })
        .unwrap_err();
        assert!(
            error.contains("no ISO metric coarse size \"M999\""),
            "threaded {threaded}, modeled {modeled_thread}: {error}"
        );
    }
}

/// A padded body with one hole position and a hole, as `edit` changes
/// it, drilled there: the hole's ops, or its build error.
fn hole_plan(edit: impl FnOnce(&mut HoleFields)) -> Result<Vec<SolidOp>, String> {
    let (mut doc, body, base_sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(base_sketch, 10.0), "Pad");
    let holes = points_sketch(&mut doc, body, &[(2.0, 2.5)]);
    add(&mut doc, body, hole(holes, edit), "Hole");
    body_build_ops(&doc, body)
        .map(|plan| plan.ops[1..].to_vec())
        .map_err(|e| e.message)
}

fn blind_distance(op: &SolidOp) -> (f64, f64) {
    let SolidOp::Sweep {
        kind:
            SweepKind::Extrude {
                termination: ExtrudeTermination::Blind { distance },
                taper_deg,
                ..
            },
        ..
    } = op
    else {
        panic!("a blind extrude: {op:?}");
    };
    (*distance, *taper_deg)
}

fn cone_of(op: &SolidOp) -> (f64, f64, f64, [f64; 3]) {
    let SolidOp::Primitive {
        kind:
            kernel_api::PrimitiveKind::Cone {
                radius1,
                radius2,
                height,
                ..
            },
        placement,
        op,
    } = op
    else {
        panic!("a cone: {op:?}");
    };
    assert_eq!(*op, BooleanOp::Cut);
    (*radius1, *radius2, *height, placement.origin)
}

#[test]
fn an_angled_point_adds_a_cone_below_the_wall_or_within_the_depth() {
    let point = |in_depth: bool| {
        hole_plan(|h| {
            *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
            *h.point_in_depth = in_depth;
        })
        .unwrap()
    };
    let tip = 1.5 / 59f64.to_radians().tan();
    let below = point(false);
    assert_eq!(below.len(), 2);
    assert!((blind_distance(&below[0]).0 - 6.0).abs() < 1e-9);
    let (r1, r2, h, origin) = cone_of(&below[1]);
    assert!((r1 - 1.5).abs() < 1e-9 && r2 == 0.0 && (h - tip).abs() < 1e-9);
    assert!(
        (origin[2] + 6.0).abs() < 1e-9,
        "at the wall's end: {origin:?}"
    );
    let within = point(true);
    assert!((blind_distance(&within[0]).0 - (6.0 - tip)).abs() < 1e-9);
    let (.., origin) = cone_of(&within[1]);
    assert!((origin[2] + 6.0 - tip).abs() < 1e-9);
    // Through all has no bottom to point.
    let through = hole_plan(|h| {
        *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
        *h.through_all = true;
    })
    .unwrap();
    assert_eq!(through.len(), 1);
}

#[test]
fn a_point_deeper_than_the_hole_is_an_error() {
    let error = hole_plan(|h| {
        *h.depth = 0.5;
        *h.drill_point = DrillPoint::Angled { angle_deg: 135.0 };
        *h.point_in_depth = true;
    })
    .unwrap_err();
    assert!(error.contains("drill point is deeper"), "{error}");
}

#[test]
fn a_tapered_hole_narrows_its_extrude_and_its_point() {
    let ops = hole_plan(|h| {
        *h.taper_deg = 3.0;
        *h.drill_point = DrillPoint::Angled { angle_deg: 118.0 };
    })
    .unwrap();
    let (distance, taper) = blind_distance(&ops[0]);
    assert!((distance - 6.0).abs() < 1e-9 && (taper + 3.0).abs() < 1e-9);
    let (r1, ..) = cone_of(&ops[1]);
    assert!((r1 - (1.5 - 6.0 * 3f64.to_radians().tan())).abs() < 1e-9);
    let closed = hole_plan(|h| *h.taper_deg = 40.0).unwrap_err();
    assert!(closed.contains("taper closes"), "{closed}");
}

#[test]
fn spotface_and_counterdrill_cut_their_mouths() {
    let spot = hole_plan(|h| {
        *h.cut = HoleCut::Spotface {
            diameter: 6.0,
            depth: 0.5,
        }
    })
    .unwrap();
    assert_eq!(spot.len(), 2);
    assert!((blind_distance(&spot[1]).0 - 0.5).abs() < 1e-9);
    let drill = hole_plan(|h| {
        *h.cut = HoleCut::Counterdrill {
            diameter: 5.0,
            depth: 2.0,
            angle_deg: 90.0,
        }
    })
    .unwrap();
    assert_eq!(drill.len(), 3, "drill, bore, cone");
    assert!((blind_distance(&drill[1]).0 - 2.0).abs() < 1e-9);
    let (r1, r2, h, origin) = cone_of(&drill[2]);
    assert!((r1 - 2.5).abs() < 1e-9 && (r2 - 1.5).abs() < 1e-9 && (h - 1.0).abs() < 1e-9);
    assert!((origin[2] + 2.0).abs() < 1e-9);
    let narrow = hole_plan(|h| {
        *h.cut = HoleCut::Spotface {
            diameter: 2.0,
            depth: 0.5,
        }
    })
    .unwrap_err();
    assert!(narrow.contains("spotface diameter"), "{narrow}");
}

#[test]
fn a_screw_seat_takes_its_size_from_the_metric_thread() {
    let seat = |standard: ThreadStandard, size: &str, seat: ScrewSeat| {
        hole_plan(|h| {
            *h.thread = Some(ThreadSpec::new(standard, size));
            *h.cut = HoleCut::Seat { seat };
        })
    };
    let bore = seat(ThreadStandard::IsoMetricCoarse, "M6", ScrewSeat::SocketHead).unwrap();
    assert!((blind_distance(&bore[1]).0 - 6.4).abs() < 1e-6);
    let SolidOp::Sweep { profile, .. } = &bore[1] else {
        panic!()
    };
    let ProfileSegment::Circle { radius, .. } = profile.wires[0].segments[0] else {
        panic!()
    };
    assert!((radius - 5.5).abs() < 1e-6, "DIN 974-1 Ø11");
    let sink = seat(
        ThreadStandard::IsoMetricFine,
        "M8x1",
        ScrewSeat::Countersunk,
    )
    .unwrap();
    assert_eq!(sink.len(), 2);
    let inch = seat(ThreadStandard::Unc, "1/4-20", ScrewSeat::SocketHead).unwrap_err();
    assert!(inch.contains("ISO metric"), "{inch}");
    let small = seat(
        ThreadStandard::IsoMetricCoarse,
        "M2",
        ScrewSeat::Countersunk,
    )
    .unwrap_err();
    assert!(small.contains("no ISO 10642 seat for M2"), "{small}");
}

#[test]
fn a_modeled_thread_takes_its_standards_form_hand_and_taper() {
    let thread = |spec: ThreadSpec| {
        let ops = hole_plan(|h| {
            *h.thread = Some(spec);
            *h.threaded = true;
            *h.modeled_thread = true;
            *h.thread_depth = 4.0;
        })
        .unwrap();
        let Some(SolidOp::Sweep {
            profile,
            kind:
                SweepKind::Helix {
                    pitch,
                    left_handed,
                    cone_angle_deg,
                    ..
                },
            ..
        }) = ops.last()
        else {
            panic!("a helix last: {ops:?}");
        };
        let corners: Vec<[f64; 2]> = profile.wires[0]
            .segments
            .iter()
            .map(|s| match s {
                ProfileSegment::Line { start, .. } => *start,
                _ => panic!(),
            })
            .collect();
        // The flank's slope: out from the wall to the major radius, and
        // along the axis from root to crest.
        let flank = ((corners[1][1] - corners[0][1]) / (corners[1][0] - corners[0][0]))
            .atan()
            .to_degrees();
        (*pitch, *left_handed, *cone_angle_deg, corners[1][0], flank)
    };
    let (pitch, left, cone, major, flank) =
        thread(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6"));
    assert!((pitch - 1.0).abs() < 1e-9 && !left && cone == 0.0);
    assert!((major - 3.0).abs() < 1e-9 && (flank - 30.0).abs() < 1e-6);

    let mut lh = ThreadSpec::new(ThreadStandard::Bsw, "1/4");
    lh.left_handed = true;
    let (pitch, left, _, _, flank) = thread(lh);
    assert!((pitch - 25.4 / 20.0).abs() < 1e-9 && left);
    assert!((flank - 27.5).abs() < 1e-6, "55° Whitworth form: {flank}");

    let (_, _, cone, major, _) = thread(ThreadSpec::new(ThreadStandard::Npt, "1/2"));
    let taper = crate::hole_tables::pipe_taper_deg();
    assert!((cone + taper).abs() < 1e-9, "narrows 1:16: {cone}");
    let npt = ThreadStandard::Npt.size("1/2").unwrap();
    let pitch = 25.4 / 14.0;
    assert!(
        (major - (npt.major * 0.5 + pitch / 32.0)).abs() < 1e-9,
        "a pitch above the face, a pitch's taper wider"
    );

    let mut g = ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6");
    g.class = "6G".into();
    let (.., major, _) = thread(g);
    assert!((major - (3.0 + 0.026 * 0.5)).abs() < 1e-9, "6G sits EI out");
}

#[test]
fn a_thread_the_standard_lacks_fails_cleanly() {
    let mut spec = ThreadSpec::new(ThreadStandard::Unc, "1/4-20");
    spec.class = "6H".into();
    let error = hole_plan(|h| *h.thread = Some(spec)).unwrap_err();
    assert!(error.contains("UNC has no class 6H"), "{error}");
}

#[test]
fn a_tapered_standard_tapers_the_tapped_drill_only() {
    let drilled = |threaded: bool| {
        hole_plan(|h| {
            *h.thread = Some(ThreadSpec::new(ThreadStandard::BspTaper, "1/4"));
            *h.threaded = threaded;
            *h.taper_deg = 7.0;
        })
        .unwrap()
    };
    let (_, taper) = blind_distance(&drilled(true)[0]);
    assert!((taper + crate::hole_tables::pipe_taper_deg()).abs() < 1e-9);
    let (_, taper) = blind_distance(&drilled(false)[0]);
    assert!((taper + 7.0).abs() < 1e-9, "a clearance hole keeps its own");
}

/// A hole starts on its sketch's plane, carried along the drill onto
/// another sketch's plane, then its offset further in; the plane it
/// starts on is a dependency.
#[test]
fn a_hole_starts_where_its_start_says() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let mut lifted = Sketch::new("lifted");
    lifted.plane.origin = [0.0, 0.0, 7.0];
    let lifted = add_sketch(&mut doc, body, lifted);
    let mut hole = match DesignFeature::from_json(&serde_json::json!({"Hole": {
        "sketch": sketch_id, "diameter": 2.0, "depth": 3.0, "through_all": false,
    }})) {
        Ok(hole) => hole,
        Err(e) => panic!("{e}"),
    };
    let start_z = |doc: &Document, hole: &DesignFeature| {
        let sketch = load_sketch(doc, sketch_id).unwrap();
        hole_plane(doc, hole, &sketch).unwrap().0.origin[2]
    };
    assert!(start_z(&doc, &hole).abs() < 1e-9);
    if let DesignFeature::Hole { start, .. } = &mut hole {
        start.offset = 2.0;
    }
    assert!(
        (start_z(&doc, &hole) + 2.0).abs() < 1e-9,
        "into the material"
    );
    if let DesignFeature::Hole { start, .. } = &mut hole {
        start.at = Some(lifted);
    }
    assert!((start_z(&doc, &hole) - 5.0).abs() < 1e-9);
    assert!(hole.dependencies().contains(&lifted));
}
