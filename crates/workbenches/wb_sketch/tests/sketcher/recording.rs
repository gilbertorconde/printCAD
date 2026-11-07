//! A session recorded through the UI replays as a script to the same
//! sketch: clicks, typed dimensions, constraint tools, a drag and a delete
//! go in as viewport events, come out as `pc.sketch.*` calls, and run
//! again on the document as it was before.

use super::Harness;
use crate::{CAM_POS, VIEWPORT, all_points};
use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureId, HookOutcome,
    KeyCode, Recorded, Workbench, WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use wb_sketch::sketch::{GeometryElement, Sketch};
use wb_sketch::{SketchFeature, SketchWorkbench};

/// The bench's commands, run on a document, for the replay.
struct Replay {
    doc: Document,
    wb: SketchWorkbench,
    specs: Vec<CommandSpec>,
    kernel: Option<&'static dyn kernel_api::KernelQueries>,
}

impl scripting::Host for Replay {
    fn commands(&self) -> Vec<CommandSpec> {
        self.specs.clone()
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        let spec = self
            .specs
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        spec.check(&args)?;
        let mut ctx = WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0; 3], VIEWPORT);
        ctx.kernel = self.kernel;
        self.wb.run_command(id, &args, &mut ctx)
    }
}

/// What a sketch comes to, in a form two sketches compare by: its points
/// where they are, what kinds of curve it has, and its constraints by kind.
fn summary(sketch: &Sketch) -> (Vec<(i64, i64)>, Vec<&'static str>, Vec<String>) {
    let mut points: Vec<(i64, i64)> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((
                (p.position.x * 1000.0).round() as i64,
                (p.position.y * 1000.0).round() as i64,
            )),
            _ => None,
        })
        .collect();
    points.sort();
    let mut curves: Vec<&'static str> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(_) => None,
            GeometryElement::Line(_) => Some("line"),
            GeometryElement::Circle(_) => Some("circle"),
            GeometryElement::Arc(_) => Some("arc"),
            GeometryElement::Ellipse(_) => Some("ellipse"),
            GeometryElement::BSpline(_) => Some("bspline"),
            GeometryElement::Conic(_) => Some("conic"),
        })
        .collect();
    curves.sort();
    let mut constraints: Vec<String> = sketch
        .constraints
        .iter()
        .map(|c| format!("{:?}", std::mem::discriminant(&c.kind)))
        .collect();
    constraints.sort();
    (points, curves, constraints)
}

#[test]
fn a_session_recorded_through_the_ui_replays_to_the_same_sketch() {
    let (mut s, id, before) = session_on_a_sketch();

    // A rectangle, a closed line chain through its corner, a circle,
    // and a slanted line. The closing point was made within the same call.
    s.full_click(2.0, 2.0, "sketch.rect");
    s.full_click(14.0, 9.0, "sketch.rect");
    s.key(KeyCode::Enter, Some("sketch.rect"));
    s.full_click(-10.0, -3.0, "sketch.line");
    s.full_click(-4.0, -8.0, "sketch.line");
    s.full_click(2.0, 2.0, "sketch.line");
    s.full_click(-10.0, -3.0, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));
    s.full_click(-9.0, 7.0, "sketch.circle");
    s.full_click(-6.5, 7.0, "sketch.circle");
    s.full_click(3.0, -12.0, "sketch.line");
    s.full_click(12.0, -6.0, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));

    // The slanted line, dimensioned at its measured length, then dragged
    // by its middle.
    s.full_click(7.5, -9.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.constrain.dimension"));
    s.key(KeyCode::Enter, Some("sketch.select"));
    s.click(7.5, -9.0, "sketch.select");
    s.mouse_move(8.5, -7.0, "sketch.select");
    s.release(8.5, -7.0, "sketch.select");
    s.key(KeyCode::Escape, Some("sketch.select"));

    // The circle deleted.
    s.full_click(-6.5, 7.0, "sketch.select");
    s.key(KeyCode::Delete, Some("sketch.select"));
    s.key(KeyCode::A, None);

    let done = summary(&s.sketch());
    assert!(!done.0.is_empty());
    let kinds: Vec<&str> = s.recorded.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "sketch.draw",
            "sketch.draw",
            "sketch.draw",
            "sketch.draw",
            "sketch.constrain",
            "sketch.drag",
            "sketch.delete"
        ],
        "{:#?}",
        s.recorded
    );

    assert_replays(&s.recorded, before, id, &s.sketch());
}

/// Lines snapped parallel and square to another, a point on its extension,
/// and an end dragged onto another end replay with the constraints they
/// left: the drag as `sketch.drag` and then `sketch.constrain`.
#[test]
fn snapped_directions_extensions_and_a_joining_drag_replay_too() {
    let (mut s, id, before) = session_on_a_sketch();
    s.wb.options.drag_constraint_delay_ms = 0.0;
    s.free_line((1.0, 10.0), (11.0, 15.0));
    s.full_click(2.0, -12.0, "sketch.line");
    s.full_click(12.0, -6.7, "sketch.line");
    s.full_click(6.4, 4.1, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));
    s.full_click(14.1, 16.8, "sketch.point");
    s.click(6.5, 4.0, "sketch.select");
    s.mouse_move(1.0, 10.0, "sketch.select");
    s.release(1.0, 10.0, "sketch.select");

    for kind in ["Parallel", "Perpendicular", "PointOnLine", "Coincident"] {
        let held = s
            .sketch()
            .constraints
            .iter()
            .any(|c| format!("{:?}", c.kind).starts_with(kind));
        assert!(held, "{kind}: {:?}", s.sketch().constraints);
    }
    let kinds: Vec<&str> = s.recorded.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        kinds[kinds.len() - 2..],
        ["sketch.drag", "sketch.constrain"],
        "{:#?}",
        s.recorded
    );
    assert_replays(&s.recorded, before, id, &s.sketch());
}

/// A line tool's arc, dragged from the end it draws from, records with the
/// line's other points and replays to the same arc.
#[test]
fn a_dragged_arc_replays_as_it_was_drawn() {
    for side in [-1., 1.] {
        for mode in 0..3 {
            let (mut s, id, before) = session_on_a_sketch();
            let mut journal = core_document::history::OpJournal::new(32);
            journal.note(&mut s.doc);
            s.journal = Some(journal);
            s.full_click(0.0, 0.0, "sketch.line");
            s.full_click(10.0, 0.0, "sketch.line");
            s.click(10.0, 0.0, "sketch.line");
            s.mouse_move(12.0, side, "sketch.line");
            s.mouse_move(15.0, 5.0 * side, "sketch.line");
            let mid = s.px_of(
                10. + 5. * std::f64::consts::FRAC_1_SQRT_2,
                side * (5. - 5. * std::f64::consts::FRAC_1_SQRT_2),
            );
            assert!(
                s.overlays()
                    .iter()
                    .any(|l| super::same_color(l.color, super::pal().preview)
                        && (l.start[0] - mid.0).hypot(l.start[1] - mid.1) < 0.2),
                "exact arc preview"
            );
            s.release(15.0, 5.0 * side, "sketch.line");
            s.key(KeyCode::Backspace, Some("sketch.line"));
            assert!(
                crate::all_arcs(&s.sketch()).is_empty(),
                "local undo owns the arc"
            );
            s.click(10., 0., "sketch.line");
            s.mouse_move(15., 5. * side, "sketch.line");
            s.release(15., 5. * side, "sketch.line");
            s.click(15., 5. * side, "sketch.line");
            s.mouse_move(20., 10. * side, "sketch.line");
            for _ in 0..mode {
                s.action("sketch.polyline_arc", "sketch.line");
            }
            s.keys("5\n", Some("sketch.line"));
            s.release(20., 10. * side, "sketch.line");
            s.full_click(25.0, 18.0 * side, "sketch.line");
            s.key(KeyCode::Escape, Some("sketch.line"));

            let done = summary(&s.sketch());
            assert!(done.1.contains(&"arc"), "{done:?}");
            let draws: Vec<&Recorded> = s
                .recorded
                .iter()
                .filter(|r| r.id == "sketch.draw")
                .collect();
            assert_eq!(draws.len(), 1, "one line tool run: {:#?}", s.recorded);
            assert_eq!(
                draws[0].args.get("shared_line"),
                Some(&serde_json::json!(true))
            );
            let live = s.sketch();
            let (lines, arcs) = (crate::all_lines(&live), crate::all_arcs(&live));
            assert_eq!((lines.len(), arcs.len()), (2, 2));
            assert_eq!(
                crate::all_points(&live).len(),
                7,
                "five shared vertices and two centres"
            );
            let arc_ends = |a: &wb_sketch::sketch::Arc| [a.start, a.end];
            assert!(arc_ends(&arcs[0]).contains(&lines[0].end));
            assert!(arc_ends(&arcs[1]).contains(&lines[1].start));
            let joint = arc_ends(&arcs[0])
                .into_iter()
                .find(|p| arc_ends(&arcs[1]).contains(p))
                .unwrap();
            let p = live.point_position(joint).unwrap().to_glam();
            let c0 = live.point_position(arcs[0].center).unwrap().to_glam();
            let c1 = live.point_position(arcs[1].center).unwrap().to_glam();
            if mode == 1 {
                assert!(
                    (p - c0).dot(p - c1).abs() < 1e-6,
                    "perpendicular at the shared UUID"
                );
            } else {
                assert!(
                    (p - c0).perp_dot(p - c1).abs() < 1e-6,
                    "tangent at the shared UUID"
                );
            }
            assert!(live.constraints.iter().any(|c| matches!(c.kind,wb_sketch::sketch::ConstraintKind::Radius {circle,radius} if circle==arcs[1].id && (radius-5.).abs()<1e-12)));
            assert_replays(&s.recorded, before, id, &s.sketch());
            s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
            s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
            s.doc = Document::load_from_bytes(
                s.doc
                    .save_to_bytes(core_document::Compression::None)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(s.sketch()).unwrap(),
                serde_json::to_value(live).unwrap()
            );
        }
    }
}

/// Write `recorded` as a script, run it on `before`, and check sketch `id`
/// comes out as `done`.
pub(super) fn assert_replays(
    recorded: &[Recorded],
    before: Document,
    id: FeatureId,
    live: &Sketch,
) {
    let done = summary(live);
    let (doc, script) = replay(recorded, before, Some(&crate::block_kernel::BLOCK));
    let replayed = SketchFeature::from_json(doc.get_feature_data(id).unwrap())
        .unwrap()
        .sketch;
    assert_eq!(summary(&replayed), done, "{script}");
    assert_eq!(point_bits(&replayed), point_bits(live), "{script}");
}

/// Write `recorded` as a script and run it on `before`: the document it
/// leaves, and the script.
fn replay(
    recorded: &[Recorded],
    before: Document,
    kernel: Option<&'static dyn kernel_api::KernelQueries>,
) -> (Document, String) {
    let mut recorder = scripting::Recorder::default();
    for call in recorded {
        recorder.push(call);
    }
    let script = recorder.script("Recorded in a test");
    let mut context = WorkbenchContext::default();
    SketchWorkbench::default().configure(&mut context);
    let mut replay = Replay {
        doc: before,
        wb: SketchWorkbench::default(),
        specs: context.commands().to_vec(),
        kernel,
    };
    let out = scripting::ScriptEngine::new().run_script(&script, "recorded.lua", &mut replay);
    assert_eq!(out.error, None, "{script}");
    (replay.doc, script)
}

/// Every point's coordinates as bits, in order: a replay that meets the
/// session to the last bit has the same.
fn point_bits(sketch: &Sketch) -> Vec<(u64, u64)> {
    let mut points: Vec<(u64, u64)> = all_points(sketch)
        .into_iter()
        .map(|p| (p.position.x.to_bits(), p.position.y.to_bits()))
        .collect();
    points.sort();
    points
}

fn session_on_a_sketch() -> (Harness, FeatureId, Document) {
    let mut s = Harness::new();
    let id = s.create_sketch();
    let before = s.doc.clone();
    (s, id, before)
}

#[test]
fn axis_rectangles_reuse_vertices_and_keep_typed_sizes_through_history() {
    use wb_sketch::sketch::{ConstraintKind as C, Point, Vec2D};
    for (tool, factor) in [("sketch.rect", 1.), ("sketch.rect_center", 0.5)] {
        for (sx, sy) in [(1., 1.), (-1., 1.), (-1., -1.), (1., -1.)] {
            let (mut s, id, _) = session_on_a_sketch();
            // Fixture boundary: the ordinary contour has four existing corners;
            // the centred one has its centre, clicked corner and a near miss.
            let corners = [
                (20., 20.),
                (20. + 12. * sx, 20.),
                (20. + 12. * sx, 20. + 8. * sy),
                (20., 20. + 8. * sy),
            ];
            let ids = s.edit_sketch(|sketch| {
                let existing = if factor == 1. {
                    corners.to_vec()
                } else {
                    // A nearby point at the computed opposite corner must not
                    // become a third click or pull that corner off the diagonal.
                    vec![
                        (20., 20.),
                        corners[2],
                        (20. - 12. * sx + 0.002, 20. - 8. * sy),
                    ]
                };
                existing
                    .into_iter()
                    .map(|(x, y)| {
                        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
                    })
                    .collect::<Vec<_>>()
            });
            let before = s.doc.clone();
            let mut journal = core_document::history::OpJournal::new(16);
            journal.note(&mut s.doc);
            s.journal = Some(journal);
            s.full_click(20.1, 20.1, tool);
            s.mouse_move(corners[2].0, corners[2].1, tool);
            assert!(s.labels().iter().any(|l| l.text == "Endpoint"));
            assert_eq!(s.sketch().geometry.len(), ids.len());
            s.full_click(corners[2].0, corners[2].1, tool);
            s.key(KeyCode::Enter, Some(tool));
            let shared = s.sketch();
            assert_eq!(
                crate::all_points(&shared).len(),
                if factor == 1. { 4 } else { 6 }
            );
            let lines = crate::all_lines(&shared);
            assert_eq!(lines.len(), 4);
            for i in 0..4 {
                if factor == 1. {
                    assert!(ids.contains(&lines[i].start));
                }
                assert_eq!(lines[i].end, lines[(i + 1) % 4].start);
            }
            if factor != 1. {
                assert!(shared.is_construction(ids[0]));
                assert!(lines.iter().any(|l| l.start == ids[1]));
                assert!(!lines.iter().any(|l| l.start == ids[2]));
                assert!(shared.constraints.iter().any(
                    |c| matches!(c.kind, C::SymmetricAboutPoint { center, .. } if center == ids[0])
                ));
            }
            assert_replays(&s.recorded, before.clone(), id, &shared);
            s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
            assert_eq!(s.sketch().geometry.len(), ids.len());
            s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
            assert_eq!(
                serde_json::to_value(s.sketch()).unwrap(),
                serde_json::to_value(&shared).unwrap()
            );

            s.mouse_move(50., 50., tool);
            s.keys("50\t50\n", Some(tool));
            s.mouse_move(50. + sx * 12., 50. + sy * 8., tool);
            s.type_text("2*(3+3)", Some(tool));
            s.keys("\t", Some(tool));
            s.type_text("4+4", Some(tool));
            s.keys("\n", Some(tool));
            s.key(KeyCode::Enter, Some(tool));
            let live = s.sketch();
            let lengths: Vec<_> = live
                .constraints
                .iter()
                .filter_map(|c| match c.kind {
                    C::DistanceX { value, .. } | C::DistanceY { value, .. } => Some(value),
                    _ => None,
                })
                .collect();
            assert_eq!(lengths.len(), 4, "both rectangles retain their dimensions");
            assert_eq!(&lengths[2..], [12., 8.]);
            let want = Vec2D::new(50. + factor * sx * 12., 50. + factor * sy * 8.);
            assert!(
                crate::all_points(&live)
                    .iter()
                    .any(|p| (p.position - want).to_glam().length() < 1e-7)
            );
            s.key(KeyCode::A, None);
            assert_replays(&s.recorded, before, id, &live);
            s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
            assert_eq!(
                serde_json::to_value(s.sketch()).unwrap(),
                serde_json::to_value(&shared).unwrap()
            );
            s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
            s.doc = Document::load_from_bytes(
                s.doc
                    .save_to_bytes(core_document::Compression::None)
                    .unwrap(),
            )
            .unwrap();
            // Construction is a set; persistence may change its iteration order.
            let values = [s.sketch(), live].map(|sketch| {
                let mut value = serde_json::to_value(sketch).unwrap();
                value["construction"]
                    .as_array_mut()
                    .unwrap()
                    .sort_by_key(|v| v.as_str().unwrap().to_owned());
                value
            });
            assert_eq!(values[0], values[1]);
        }
    }
}

#[test]
fn a_saved_document_and_coordinate_recording_continue_in_the_live_drawing_runtime() {
    let old = Document::load_from_bytes(
        include_bytes!("../../../../app_shell/tests/corpus/0.11.0/surfaces-sewn-solid-1.prtcad")
            .to_vec(),
    )
    .unwrap();
    let id = *old
        .feature_tree()
        .all_nodes()
        .find(|(_, node)| node.workbench_id.as_str() == "wb.sketch")
        .unwrap()
        .0;
    let original = SketchFeature::from_json(old.get_feature_data(id).unwrap()).unwrap();
    for (tool, points, lines, arcs) in [
        (
            "arc",
            serde_json::json!([[-60., -50.], [-59.98, -50.], [-60., -49.98]]),
            0,
            1,
        ),
        (
            "arc3",
            serde_json::json!([[-60., -50.], [-59.98, -50.], [-59.99, -49.99]]),
            0,
            1,
        ),
        (
            "circle3",
            serde_json::json!([[-60., -50.], [-59.98, -50.], [-59.99, -49.99]]),
            0,
            0,
        ),
        (
            "ellipse",
            serde_json::json!([[-60., -50.], [-59.98, -50.], [-59.99, -49.99]]),
            0,
            0,
        ),
        (
            "ellipse_arc",
            serde_json::json!([
                [-60., -50.],
                [-59.98, -50.],
                [-59.988, -49.992],
                [-60.02, -50.]
            ]),
            0,
            0,
        ),
        (
            "ellipse3",
            serde_json::json!([[-60.02, -50.], [-59.98, -50.], [-59.988, -49.992]]),
            0,
            0,
        ),
        (
            "parabola",
            serde_json::json!([
                [-60., -50.],
                [-59.98, -50.],
                [-59.975, -49.99],
                [-59.96, -50.02]
            ]),
            0,
            0,
        ),
        (
            "hyperbola",
            serde_json::json!([
                [-60., -50.],
                [-59.98, -50.],
                [-59.975, -49.98875],
                [-59.96, -50.02]
            ]),
            0,
            0,
        ),
        (
            "rect",
            // Coordinate commands accept sizes below a pixel of the shared mouse gesture.
            serde_json::json!([[-60., -50.], [-59.98, -49.992]]),
            4,
            0,
        ),
        (
            "rect_center",
            serde_json::json!([[-60., -50.], [-59.99, -49.996]]),
            4,
            0,
        ),
        (
            "rect3",
            serde_json::json!([[-60., -50.], [-59.98, -49.99], [-59.982, -49.986]]),
            4,
            0,
        ),
        (
            "rect_center3",
            serde_json::json!([[-60., -50.], [-59.99, -49.996], [-60.004, -49.99]]),
            4,
            0,
        ),
        (
            "polyline",
            serde_json::json!([[-60., -50.], [-52., -50.], "arc", [-48., -46.]]),
            1,
            1,
        ),
    ] {
        let (mut doc, _) = replay(
            &[Recorded {
                id: "sketch.draw".into(),
                args: serde_json::from_value(serde_json::json!({
                    "sketch": id, "tool": tool, "tolerance": 0.001, "points": points
                }))
                .unwrap(),
                result: serde_json::Value::Null,
            }],
            old.clone(),
            None,
        );
        let loaded =
            Document::load_from_bytes(doc.save_to_bytes(core_document::Compression::None).unwrap())
                .unwrap();
        let data = SketchFeature::from_json(loaded.get_feature_data(id).unwrap()).unwrap();
        let made: Vec<_> = data
            .sketch
            .geometry
            .iter()
            .filter(|g| original.sketch.get_geometry(g.id()).is_none())
            .collect();
        assert_eq!(
            made.iter()
                .filter(|g| matches!(g, GeometryElement::Line(_)))
                .count(),
            lines,
            "{tool}"
        );
        assert_eq!(
            made.iter()
                .filter(|g| matches!(g, GeometryElement::Arc(_)))
                .count(),
            arcs,
            "{tool}"
        );
        if matches!(tool, "ellipse" | "ellipse3" | "ellipse_arc") {
            let ellipse = made
                .iter()
                .find_map(|g| match g {
                    GeometryElement::Ellipse(e) => Some(e),
                    _ => None,
                })
                .unwrap();
            assert!((ellipse.major.to_glam().length() - 0.02).abs() < 1e-9);
            assert!((ellipse.ratio - 0.5).abs() < 1e-9);
            assert_eq!(ellipse.arc.is_none(), tool != "ellipse_arc");
        }
        if tool == "parabola" {
            let conic = made
                .iter()
                .find_map(|g| match g {
                    GeometryElement::Conic(c) => Some(c),
                    _ => None,
                })
                .unwrap();
            assert!((conic.axis.x - 0.02).abs() < 1e-9 && conic.axis.y == 0.);
            for (point, x, y) in [
                (conic.start, -59.99875, -49.99),
                (conic.end, -59.995, -50.02),
            ] {
                let pos = data.sketch.point_position(point).unwrap();
                assert!((pos.x - x).hypot(pos.y - y) < 1e-9);
            }
        }
        if tool == "hyperbola" {
            let conic = made
                .iter()
                .find_map(|g| {
                    if let GeometryElement::Conic(c) = g {
                        Some(c)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!((conic.axis.x - 0.02).abs() < 1e-9 && conic.axis.y == 0.);
            assert!((conic.minor - 0.015).abs() < 1e-9);
            let end = data.sketch.point_position(conic.end).unwrap();
            assert!((end.x + 60. - 1. / 30.).abs() < 1e-9 && (end.y + 50.02).abs() < 1e-9);
        }
        if tool == "circle3" {
            let circle = made
                .iter()
                .find_map(|g| {
                    if let GeometryElement::Circle(c) = g {
                        Some(c)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!((circle.radius - 0.01).abs() < 1e-9);
            let center = data.sketch.point_position(circle.center).unwrap();
            assert!((center.x + 59.99).abs() < 1e-9 && (center.y + 50.).abs() < 1e-9);
        } else {
            assert!(made.iter().any(|g| matches!(g, GeometryElement::Point(p) if p.position == wb_sketch::sketch::Vec2D::new(-60., -50.))));
        }
        for element in &original.sketch.geometry {
            assert_eq!(
                serde_json::to_value(data.sketch.get_geometry(element.id()).unwrap()).unwrap(),
                serde_json::to_value(element).unwrap()
            );
        }
    }
    let (with_point, _) = replay(
        &[Recorded {
            id: "sketch.draw".into(),
            args: serde_json::from_value(serde_json::json!({
                "sketch": id, "tool": "point", "points": [[-0.1, -0.1]]
            }))
            .unwrap(),
            result: serde_json::Value::Null,
        }],
        old.clone(),
        None,
    );
    let point_data = SketchFeature::from_json(with_point.get_feature_data(id).unwrap()).unwrap();
    let point = crate::all_points(&point_data.sketch)
        .into_iter()
        .find(|p| original.sketch.get_geometry(p.id).is_none())
        .unwrap();
    assert_eq!(point.position, wb_sketch::sketch::Vec2D::new(-0.1, -0.1));
    let call = Recorded {
        id: "sketch.draw".into(),
        args: serde_json::from_value(serde_json::json!({
            "sketch": id, "tool": "line", "tolerance": 0.001,
            "points": [[-15., -12.], [-6., -12.], [-0.1, -0.1]]
        }))
        .unwrap(),
        result: serde_json::Value::Null,
    };
    let circle_call = Recorded {
        id: "sketch.draw".into(),
        args: serde_json::from_value(serde_json::json!({
            "sketch": id, "tool": "circle", "tolerance": 0.001,
            "points": [[-25., -20.], [-22., -16.]]
        }))
        .unwrap(),
        result: serde_json::Value::Null,
    };
    let (doc, _) = replay(&[call, circle_call], old, None);
    let mut s = Harness::new();
    s.doc = doc;
    s.active_object = Some(id);
    s.key(KeyCode::A, None);
    let loaded = s.sketch();
    let old_circle = loaded
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) if original.sketch.get_geometry(c.id).is_none() => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(old_circle.radius, 5.);
    assert_eq!(
        loaded.point_position(old_circle.center).unwrap(),
        wb_sketch::sketch::Vec2D::new(-25., -20.)
    );
    for element in &original.sketch.geometry {
        assert_eq!(
            serde_json::to_value(loaded.get_geometry(element.id()).unwrap()).unwrap(),
            serde_json::to_value(element).unwrap()
        );
    }
    let coordinate_lines: Vec<_> = crate::all_lines(&loaded)
        .into_iter()
        .filter(|l| original.sketch.get_geometry(l.id).is_none())
        .collect();
    assert_eq!(coordinate_lines.len(), 2);
    assert_eq!(coordinate_lines[0].end, coordinate_lines[1].start);
    let end = coordinate_lines[1].end;
    let at = loaded.point_position(end).unwrap();
    assert!((at.x + 0.1).hypot(at.y + 0.1) < 1e-8);
    let before = s.doc.clone();
    let mut journal = core_document::history::OpJournal::new(16);
    journal.note(&mut s.doc);
    s.journal = Some(journal);
    s.full_click(-0.1, -0.1, "sketch.line");
    assert_eq!(
        serde_json::to_value(s.sketch()).unwrap(),
        serde_json::to_value(&loaded).unwrap()
    );
    s.full_click(-12., 4., "sketch.line");
    s.full_click(-18., 9., "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));
    let live = s.sketch();
    let added: Vec<_> = crate::all_lines(&live)
        .into_iter()
        .filter(|l| loaded.get_geometry(l.id).is_none())
        .collect();
    assert_eq!(added.len(), 2);
    assert_eq!(added[0].start, end, "reuse the saved UUID");
    assert_eq!(added[0].end, added[1].start);
    assert_eq!(s.recorded.len(), 1);
    assert_eq!(s.recorded[0].args["shared_line"], true);
    assert_replays(&s.recorded, before, id, &live);
    for _ in 0..2 {
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
    }
    assert_eq!(
        serde_json::to_value(s.sketch()).unwrap(),
        serde_json::to_value(loaded).unwrap()
    );
    for _ in 0..2 {
        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
    }
    s.doc = Document::load_from_bytes(
        s.doc
            .save_to_bytes(core_document::Compression::None)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(s.sketch()).unwrap(),
        serde_json::to_value(live).unwrap()
    );
}

#[test]
fn point_and_circle_placement_preserve_vertices_bindings_numeric_input_and_history() {
    use wb_sketch::sketch::{Circle, ConstraintKind as C, Line, Point, Vec2D};
    for tool in ["sketch.point", "sketch.circle"] {
        for (cursor, want, label, kind) in [
            ((10.2, 20.1), (10., 20.), "Endpoint", "reuse"),
            ((45.2, 20.1), (45., 20.), "Center", "reuse"),
            ((20.2, 20.1), (20., 20.), "Midpoint", "SymmetricAboutPoint"),
            ((50.2, 20.1), (50., 20.), "Quadrant", "CircularPoint"),
            ((0.1, 0.1), (0., 0.), "Origin", "Coincident"),
            ((0.1, -10.2), (0., -10.2), "On axis", "PointOnLine"),
            ((3.1, 20.1), (3.1, 20.), "On extension", "PointOnLine"),
            ((14.1, 20.1), (14.1, 20.), "On curve", "PointOnLine"),
            ((-12.1, -13.2), (-12.1, -13.2), "", "free"),
            ((10.2, 20.1), (10.2, 20.1), "", "off"),
            ((-12.1, -13.2), (-10., -15.), "", "grid"),
            ((10.2, 20.1), (10.3, 20.2), "", "typed"),
        ] {
            // Extensions are point/line carriers; a circle uses ordinary object picks.
            if tool == "sketch.circle" && label == "On extension" {
                continue;
            }
            let (mut s, id, _) = session_on_a_sketch();
            s.edit_sketch(|sketch| {
                let mut p = |x, y| {
                    let position = Vec2D::new(x, y);
                    let point = sketch.add_geometry(GeometryElement::Point(Point::new(position)));
                    sketch.add_constraint(C::FixedPoint { point, position });
                    point
                };
                let a = p(10., 20.);
                let b = p(30., 20.);
                let center = p(45., 20.);
                sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
                sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.)));
            });
            let before = s.doc.clone();
            let initial = s.sketch();
            let mut journal = core_document::history::OpJournal::new(16);
            journal.note(&mut s.doc);
            let initial_entry = journal.undo_entry_id();
            s.journal = Some(journal);
            if kind == "off" {
                s.activate("sketch.snap");
            }
            if matches!(kind, "grid" | "typed") {
                s.wb.options.grid_on = true;
                s.wb.options.grid_snap = true;
                s.wb.options.grid_auto = false;
                s.wb.options.grid_size = 5.;
            }
            s.mouse_move(cursor.0, cursor.1, tool);
            if !label.is_empty() {
                assert!(
                    s.labels().iter().any(|l| l.text == label),
                    "{kind}: {:?}",
                    s.labels()
                );
            }
            if kind == "typed" {
                s.type_text("10+0.3", Some(tool));
                s.keys("\t20.2\n", Some(tool));
            } else {
                s.full_click(cursor.0, cursor.1, tool);
            }
            if tool == "sketch.circle" {
                assert_eq!(
                    serde_json::to_value(s.sketch()).unwrap(),
                    serde_json::to_value(&initial).unwrap()
                );
                s.mouse_move(-12., -13., tool);
                s.type_text("2*(3+1)", Some(tool));
                s.full_click(-12., -13., tool);
                let live = s.sketch();
                let circle = live
                    .geometry
                    .iter()
                    .find_map(|g| match g {
                        GeometryElement::Circle(c) if initial.get_geometry(c.id).is_none() => {
                            Some(c)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!((circle.radius - 4.).abs() < 1e-8);
                if kind == "reuse" {
                    let center = initial
                        .geometry
                        .iter()
                        .find_map(|g| match g {
                            GeometryElement::Point(p)
                                if p.position == Vec2D::new(want.0, want.1) =>
                            {
                                Some(p.id)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(circle.center, center);
                    assert_eq!(
                        crate::all_points(&live).len(),
                        crate::all_points(&initial).len()
                    );
                }
            }
            s.key(KeyCode::Escape, Some(tool));
            let live = s.sketch();
            if kind == "reuse" && tool == "sketch.point" {
                assert_eq!(
                    serde_json::to_value(&live).unwrap(),
                    serde_json::to_value(&initial).unwrap()
                );
                assert_eq!(s.journal.as_ref().unwrap().undo_entry_id(), initial_entry);
            } else {
                let made = crate::all_points(&live)
                    .into_iter()
                    .find(|p| {
                        if kind == "reuse" {
                            p.position == Vec2D::new(want.0, want.1)
                        } else {
                            initial.get_geometry(p.id).is_none()
                        }
                    })
                    .unwrap();
                // Free coordinates cross the viewport's f32 raycast. Computed
                // snap points and typed coordinates are judged at solver precision.
                let tolerance = if matches!(kind, "PointOnLine" | "free" | "off") {
                    1e-3
                } else {
                    1e-6
                };
                assert!(
                    (made.position.x - want.0).hypot(made.position.y - want.1) < tolerance,
                    "{kind}: {made:?}"
                );
                let bindings: Vec<_> = live
                    .constraints
                    .iter()
                    .filter(|c| !initial.constraints.iter().any(|old| old.id == c.id))
                    .collect();
                if matches!(kind, "free" | "grid" | "off" | "typed" | "reuse") {
                    assert!(bindings.is_empty(), "{kind}: {bindings:?}");
                } else {
                    assert_eq!(bindings.len(), 1, "{kind}: {bindings:?}");
                    assert!(format!("{:?}", bindings[0].kind).starts_with(kind));
                }
                s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                assert_eq!(
                    serde_json::to_value(s.sketch()).unwrap(),
                    serde_json::to_value(initial).unwrap()
                );
                s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                assert_replays(&s.recorded, before, id, &live);
                s.doc = Document::load_from_bytes(
                    s.doc
                        .save_to_bytes(core_document::Compression::None)
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    serde_json::to_value(s.sketch()).unwrap(),
                    serde_json::to_value(live).unwrap()
                );
            }
        }
    }
}

#[test]
fn typed_line_coordinates_override_object_snap_and_grid_through_replay() {
    use wb_sketch::sketch::{ConstraintKind as C, Point, Vec2D};
    for typed in [false, true] {
        let (mut s, id, _) = session_on_a_sketch();
        s.edit_sketch(|sketch| {
            for x in [10.4, 20.4] {
                let position = Vec2D::new(x, 20.1);
                let point = sketch.add_geometry(GeometryElement::Point(Point::new(position)));
                sketch.add_constraint(C::FixedPoint { point, position });
            }
        });
        let before = s.doc.clone();
        let mut journal = core_document::history::OpJournal::new(32);
        journal.note(&mut s.doc);
        s.journal = Some(journal);
        s.wb.options.grid_on = true;
        s.wb.options.grid_snap = true;
        s.wb.options.grid_auto = false;
        s.wb.options.grid_size = 5.;
        s.mouse_move(10.6, 20.3, "sketch.line");
        assert!(s.labels().iter().any(|l| l.text == "Endpoint"));
        if typed {
            s.keys("10.3\t20.2\n", Some("sketch.line"));
        } else {
            s.full_click(10.6, 20.3, "sketch.line");
        }
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            serde_json::to_value(
                SketchFeature::from_json(before.get_feature_data(id).unwrap())
                    .unwrap()
                    .sketch
            )
            .unwrap()
        );
        s.mouse_move(20.6, 20.3, "sketch.line");
        if typed {
            s.keys("10\t0\n", Some("sketch.line"));
        } else {
            s.full_click(20.6, 20.3, "sketch.line");
        }
        s.full_click(-10., -5., "sketch.line");
        s.key(KeyCode::Escape, Some("sketch.line"));
        let live = s.sketch();
        let lines = crate::all_lines(&live);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].end, lines[1].start);
        let (x, y) = if typed { (10.3, 20.2) } else { (10.4, 20.1) };
        for (point, want) in [
            (lines[0].start, Vec2D::new(x, y)),
            (lines[0].end, Vec2D::new(x + 10., y)),
        ] {
            assert!(
                (live.point_position(point).unwrap() - want)
                    .to_glam()
                    .length()
                    < 1e-7
            );
        }
        if typed {
            assert_eq!(
                live.constraints
                    .iter()
                    .filter(|c| matches!(c.kind, C::Distance { .. } | C::AngleToAxis { .. }))
                    .count(),
                2
            );
            assert!(!live.constraints.iter().any(|c| matches!(
                c.kind,
                C::Coincident { .. } | C::PointOnLine { .. } | C::Horizontal { .. }
            )));
        }
        assert_replays(&s.recorded, before, id, &live);
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
        s.doc = Document::load_from_bytes(
            s.doc
                .save_to_bytes(core_document::Compression::None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            serde_json::to_value(live).unwrap()
        );
    }
}

#[test]
fn a_shared_line_chain_owns_only_its_links_and_replays_after_local_undo() {
    use wb_sketch::sketch::{ConstraintKind, Line, Point, Vec2D};
    for tool in ["sketch.line", "sketch.polyline"] {
        let (mut s, id, _) = session_on_a_sketch();
        // Fixture boundary: an existing fixed source. Every new line below enters
        // through the viewport, including its midpoint binding and typed length.
        s.edit_sketch(|sketch| {
            let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5., 10.))));
            let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(35., 10.))));
            sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
            for (point, position) in [(a, Vec2D::new(5., 10.)), (b, Vec2D::new(35., 10.))] {
                sketch.add_constraint(ConstraintKind::FixedPoint { point, position });
            }
        });
        let before = s.doc.clone();
        let mut journal = core_document::history::OpJournal::new(32);
        journal.note(&mut s.doc);
        s.journal = Some(journal);
        let initial = serde_json::to_value(s.sketch()).unwrap();
        s.mouse_move(20.1, 10.2, tool);
        assert!(s.labels().iter().any(|label| label.text == "Midpoint"));
        s.full_click(20.1, 10.2, tool);
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            initial,
            "first click is intent only"
        );
        s.mouse_move(8., -12., tool);
        let (a, b) = (s.px_of(20., 10.), s.px_of(8., -12.));
        assert!(
            s.overlays().iter().any(|line| {
                super::same_color(line.color, super::pal().preview)
                    && (line.start[0] - a.0).abs() < 0.1
                    && (line.start[1] - a.1).abs() < 0.1
                    && (line.end[0] - b.0).abs() < 0.1
                    && (line.end[1] - b.1).abs() < 0.1
            }),
            "preview runs from the selected midpoint to the click"
        );
        s.full_click(8., -12., tool);
        let first = s.journal.as_ref().unwrap().undo_entry_id().unwrap();
        s.full_click(27., -16., tool);
        assert_ne!(s.journal.as_ref().unwrap().undo_entry_id(), Some(first));
        assert_eq!(crate::all_lines(&s.sketch()).len(), 3);
        s.keys("7", Some(tool));
        s.key(KeyCode::Backspace, Some(tool));
        assert_eq!(
            crate::all_lines(&s.sketch()).len(),
            3,
            "Backspace edits the buffer first"
        );
        s.key(KeyCode::Backspace, Some(tool));
        assert_eq!(
            crate::all_lines(&s.sketch()).len(),
            2,
            "then it undoes just the last link"
        );
        assert_eq!(s.journal.as_ref().unwrap().undo_entry_id(), Some(first));
        s.mouse_move(28., -17., tool);
        s.keys("13\n", Some(tool));
        let live = s.sketch();
        let lines = crate::all_lines(&live);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[1].end, lines[2].start,
            "one UUID at the chain junction"
        );
        assert!(live.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::SymmetricAboutPoint { center, .. } if center == lines[1].start)));
        let distance = |p, q| {
            (live.point_position(p).unwrap().to_glam() - live.point_position(q).unwrap().to_glam())
                .length()
        };
        assert!((distance(lines[2].start, lines[2].end) - 13.).abs() < 1e-7);
        s.key(KeyCode::Escape, Some(tool));
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            serde_json::to_value(&live).unwrap()
        );
        assert_replays(&s.recorded, before, id, &live);
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
        s.mouse_move(28., -17., tool);
        assert_eq!(crate::all_lines(&s.sketch()).len(), 2);
        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
        s.mouse_move(28., -17., tool);
        let bytes = s
            .doc
            .save_to_bytes(core_document::Compression::None)
            .unwrap();
        s.doc = Document::load_from_bytes(bytes).unwrap();
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            serde_json::to_value(live).unwrap()
        );
    }
}

#[test]
fn a_polyline_uses_one_mixed_chain_for_modes_preview_closure_and_history() {
    use wb_sketch::sketch::Vec2D;
    let tool = "sketch.polyline";
    for side in [-1., 1.] {
        let (mut s, id, before) = session_on_a_sketch();
        let mut journal = core_document::history::OpJournal::new(32);
        journal.note(&mut s.doc);
        s.journal = Some(journal);
        s.activate("sketch.snap");
        s.full_click(0., 0., tool);
        s.full_click(20., 0., tool);
        for (x, y, cx, cy, span) in [
            (30., 10., 20., 10., 90_f64),
            (40., 20., 30., 20., 90.),
            (50., 30., 50., 20., 270.),
        ] {
            s.action("sketch.polyline_arc", tool);
            let n = crate::all_arcs(&s.sketch()).len();
            if n == 0 {
                let unchanged = serde_json::to_value(s.sketch()).unwrap();
                s.full_click(30., 0., tool);
                assert_eq!(
                    serde_json::to_value(s.sketch()).unwrap(),
                    unchanged,
                    "a collinear endpoint cannot silently turn an arc into a line"
                );
            }
            s.mouse_move(x, y * side, tool);
            let theta = if n < 2 { -45. } else { 315. };
            let mid = s.px_of(
                cx + 10. * f64::to_radians(theta).cos(),
                side * (cy + 10. * f64::to_radians(theta).sin()),
            );
            assert!(
                s.overlays()
                    .iter()
                    .any(|l| super::same_color(l.color, super::pal().preview)
                        && (l.start[0] - mid.0).hypot(l.start[1] - mid.1) < 0.3),
                "the exact planned arc is visible"
            );
            s.full_click(x, y * side, tool);
            let live = s.sketch();
            let arcs = crate::all_arcs(&live);
            let arc = arcs.last().unwrap();
            let center = live.point_position(arc.center).unwrap();
            // Pixel input and the host's under-constrained solve can settle
            // nearby. These bounds are much smaller than one viewport pixel.
            assert!(
                (center - Vec2D::new(cx, cy * side)).to_glam().length() < 2e-3,
                "arc {n} side {side}: {center:?}"
            );
            assert!((arc.radius - 10.).abs() < 2e-3);
            let a = live.point_position(arc.start).unwrap() - center;
            let b = live.point_position(arc.end).unwrap() - center;
            let sweep = (b.y.atan2(b.x) - a.y.atan2(a.x)).rem_euclid(std::f64::consts::TAU);
            assert!((sweep.to_degrees() - span).abs() < 0.02);
        }
        // Change mode after a commit, then undo its click. The recording must
        // retain those switches without replaying the removed arc.
        for _ in 0..2 {
            s.action("sketch.polyline_arc", tool);
        }
        s.key(KeyCode::Backspace, Some(tool));
        assert_eq!(crate::all_arcs(&s.sketch()).len(), 2);
        for _ in 0..2 {
            s.action("sketch.polyline_arc", tool);
        }
        s.full_click(50., 35. * side, tool);
        assert_eq!(crate::all_arcs(&s.sketch()).len(), 3);
        // Reverse stays selected after its commit; one switch returns to Line.
        s.action("sketch.polyline_arc", tool);
        s.activate("sketch.snap");
        s.full_click(0., 0., tool);
        let live = s.sketch();
        let lines = crate::all_lines(&live);
        assert_eq!(
            (
                lines.len(),
                crate::all_arcs(&live).len(),
                all_points(&live).len()
            ),
            (2, 3, 8)
        );
        assert_eq!(lines[0].start, lines[1].end, "closure reuses its UUID");
        // Closure finishes the machine: this click only starts another chain.
        s.full_click(-20., -15., tool);
        s.key(KeyCode::Escape, Some(tool));
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            serde_json::to_value(&live).unwrap()
        );
        assert!(s.recorded.iter().all(|r| r.args["shared_polyline"] == true));
        assert_replays(&s.recorded, before, id, &live);
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
        assert_eq!(crate::all_lines(&s.sketch()).len(), 1);
        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
        s.doc = Document::load_from_bytes(
            s.doc
                .save_to_bytes(core_document::Compression::None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            serde_json::to_value(live).unwrap()
        );
    }
}

#[test]
fn circular_line_bindings_follow_sources_through_history_reload_and_replay() {
    use wb_sketch::sketch::{Circle, ConstraintKind as C, Point, Vec2D};
    for reverse in [false, true] {
        for (q, (dx, dy)) in [(1., 0.), (0., 1.), (-1., 0.), (0., -1.)]
            .into_iter()
            .enumerate()
        {
            let (mut s, id, _) = session_on_a_sketch();
            let (center, circle) = s.edit_sketch(|sketch| {
                let center =
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(8., 7.))));
                let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.)));
                sketch.set_construction(circle, true);
                if reverse {
                    sketch.geometry.reverse();
                }
                (center, circle)
            });
            let before = s.doc.clone();
            let mut journal = core_document::history::OpJournal::new(32);
            journal.note(&mut s.doc);
            s.journal = Some(journal);
            let initial = serde_json::to_value(s.sketch()).unwrap();
            let (x, y) = (8. + 5. * dx, 7. + 5. * dy);
            s.mouse_move(x + 0.1, y + 0.1, "sketch.line");
            assert!(s.labels().iter().any(|l| l.text == "Quadrant"));
            s.full_click(x + 0.1, y + 0.1, "sketch.line");
            assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
            s.full_click(-16., -12., "sketch.line");
            s.full_click(-7., -20., "sketch.line");
            s.key(KeyCode::Backspace, Some("sketch.line"));
            assert_eq!(crate::all_lines(&s.sketch()).len(), 1);
            s.full_click(-7., -20., "sketch.line");
            s.key(KeyCode::Escape, Some("sketch.line"));
            let live = s.sketch();
            let lines = crate::all_lines(&live);
            let bound = lines[0].start;
            assert_eq!(lines[0].end, lines[1].start);
            assert!(live.constraints.iter().any(|c| matches!(c.kind, C::CircularPoint {point,circle: curve,angle_rad} if point==bound && curve==circle && (angle_rad-q as f64*std::f64::consts::FRAC_PI_2).abs()<1e-12)));
            assert!(live.is_construction(circle));
            assert_replays(&s.recorded, before, id, &live);
            s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
            s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
            let bytes = s
                .doc
                .save_to_bytes(core_document::Compression::None)
                .unwrap();
            s.doc = Document::load_from_bytes(bytes).unwrap();
            assert_eq!(
                serde_json::to_value(s.sketch()).unwrap(),
                serde_json::to_value(live).unwrap()
            );
            // Source edit boundary: hold a new centre and radius without moving
            // the bound point's initial approximation. Solve must keep its side.
            for (cx, cy, r) in [(30., 25., 7.), (-20., -15., 4.)] {
                s.edit_sketch(|sketch| {
                    if let GeometryElement::Point(p) = sketch.get_geometry_mut(center).unwrap() {
                        p.position = Vec2D::new(cx, cy);
                    }
                    if let GeometryElement::Circle(c) = sketch.get_geometry_mut(circle).unwrap() {
                        c.radius = r;
                    }
                });
                let mut solved = s.sketch();
                solved.add_constraint(C::FixedPoint {
                    point: center,
                    position: Vec2D::new(cx, cy),
                });
                solved.add_constraint(C::Radius { circle, radius: r });
                assert!(matches!(
                    wb_sketch::operations::solve(&mut solved).unwrap(),
                    wb_sketch::operations::SolveOutcome::Converged { .. }
                ));
                let p = solved.point_position(bound).unwrap();
                assert!(
                    (p.x - (cx + r * dx)).hypot(p.y - (cy + r * dy)) < 1e-6,
                    "{p:?}"
                );
            }
        }
    }
}

#[test]
fn drawing_at_an_exact_curve_crossing_keeps_sources_history_and_replay() {
    use wb_sketch::sketch::{
        BSpline, Conic, ConicKind, ConstraintKind as C, Ellipse, Line, Point, Vec2D,
    };
    for (tool, role) in [
        "sketch.line",
        "sketch.polyline",
        "sketch.point",
        "sketch.circle",
        "sketch.arc",
        "sketch.arc3",
        "sketch.ellipse",
        "sketch.ellipse3",
        "sketch.ellipse_arc",
        "sketch.hyperbola",
        "sketch.parabola",
        "sketch.rect",
        "sketch.rect_center",
        "sketch.rect3",
        "sketch.rect_center3",
    ]
    .into_iter()
    .flat_map(|tool| {
        (0..match tool {
            "sketch.hyperbola" => 3,
            "sketch.ellipse3" => 3,
            "sketch.parabola" => 2,
            _ => 1,
        })
            .filter(move |role| tool != "sketch.hyperbola" || *role != 1)
            .map(move |role| (tool, role))
    }) {
        for family in 0..7 {
            for (reverse, automatic) in [(false, true), (true, true), (false, false), (true, false)]
            {
                let (mut s, id, _) = session_on_a_sketch();
                s.wb.options.auto_constraints = automatic;
                let (spline, crossing, other, at) = s.edit_sketch(|sketch| {
                    let (spline, at) = if (2..6).contains(&family) {
                        let mut p = |x, y| {
                            let position = Vec2D::new(x, y);
                            let point =
                                sketch.add_geometry(GeometryElement::Point(Point::new(position)));
                            sketch.add_constraint(C::FixedPoint { point, position });
                            point
                        };
                        let center = p(8., 7.);
                        let (element, at) = match family {
                            2 => (
                                GeometryElement::Ellipse(Ellipse::new(
                                    center,
                                    Vec2D::new(10., 0.),
                                    0.5,
                                )),
                                Vec2D::new(16., 10.),
                            ),
                            3 => {
                                let (start, end) = (p(18., 7.), p(-2., 7.));
                                (
                                    GeometryElement::Ellipse(Ellipse::new_arc(
                                        center,
                                        Vec2D::new(10., 0.),
                                        0.5,
                                        start,
                                        end,
                                    )),
                                    Vec2D::new(16., 10.),
                                )
                            }
                            4 => {
                                let (start, end) = (p(12.5, 1.), p(12.5, 13.));
                                (
                                    GeometryElement::Conic(Conic::new(
                                        ConicKind::Parabola,
                                        center,
                                        Vec2D::new(2., 0.),
                                        0.,
                                        start,
                                        end,
                                    )),
                                    Vec2D::new(10., 11.),
                                )
                            }
                            5 => {
                                let (start, end) = (
                                    p(8. + 4. * 1.0_f64.cosh(), 7. - 3. * 1.0_f64.sinh()),
                                    p(8. + 4. * 1.0_f64.cosh(), 7. + 3. * 1.0_f64.sinh()),
                                );
                                (
                                    GeometryElement::Conic(Conic::new(
                                        ConicKind::Hyperbola,
                                        center,
                                        Vec2D::new(4., 0.),
                                        3.,
                                        start,
                                        end,
                                    )),
                                    Vec2D::new(13., 9.25),
                                )
                            }
                            _ => unreachable!(),
                        };
                        (sketch.add_geometry(element), at)
                    } else {
                        let periodic = family == 1;
                        let positions = if family == 6 {
                            vec![(-2., 17.), (8., -3.), (18., 17.)]
                        } else if periodic {
                            vec![(3., 6.), (10., 18.), (23., 12.), (19., 2.), (6., 1.)]
                        } else {
                            vec![(13., 7.), (13., 12.), (8., 12.)]
                        };
                        let poles: Vec<_> = positions
                            .iter()
                            .map(|&(x, y)| {
                                let position = Vec2D::new(x, y);
                                let point = sketch
                                    .add_geometry(GeometryElement::Point(Point::new(position)));
                                sketch.add_constraint(C::FixedPoint { point, position });
                                point
                            })
                            .collect();
                        let mut b = BSpline::new(poles, periodic);
                        b.degree = 2;
                        b.weights = if family == 6 {
                            vec![1.; 3]
                        } else if periodic {
                            vec![1., 0.7, 1.2, 0.8, 1.]
                        } else {
                            vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.]
                        };
                        let basis = wb_sketch::spline::basis_of(&b).unwrap();
                        let at = if family == 6 {
                            Vec2D::new(8., 7.)
                        } else if periodic {
                            let [x, y] = basis.eval(&b.control_positions(sketch).unwrap(), 3.37);
                            Vec2D::new(x, y)
                        } else {
                            Vec2D::new(12., 10.)
                        };
                        (sketch.add_geometry(GeometryElement::BSpline(b)), at)
                    };
                    let mut line_at = |y| {
                        let mut point_at = |x| {
                            let position = Vec2D::new(x, y);
                            let point =
                                sketch.add_geometry(GeometryElement::Point(Point::new(position)));
                            sketch.add_constraint(C::FixedPoint { point, position });
                            point
                        };
                        let a = point_at(-20.);
                        let b = point_at(30.);
                        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)))
                    };
                    let crossing = line_at(at.y);
                    let other = line_at(at.y + 0.002);
                    if reverse {
                        sketch.geometry.reverse();
                    }
                    (spline, crossing, other, at)
                });
                if tool == "sketch.ellipse3" {
                    let offsets = [
                        glam::DVec2::new(-10., 0.),
                        glam::DVec2::new(10., 0.),
                        glam::DVec2::new(6., 4.),
                    ];
                    s.edit_sketch(|sketch| {
                        for (i, offset) in offsets.into_iter().enumerate() {
                            if i != role {
                                let p = at.to_glam() - offsets[role] + offset;
                                sketch.add_geometry(GeometryElement::Point(Point::new(
                                    Vec2D::from_glam(p),
                                )));
                            }
                        }
                    });
                }
                let before = s.doc.clone();
                let mut journal = core_document::history::OpJournal::new(32);
                journal.note(&mut s.doc);
                s.journal = Some(journal);
                s.mouse_move(at.x, at.y, tool);
                assert!(
                    s.labels().iter().any(|l| l.text == "Intersection"),
                    "family {family}, reverse {reverse}: {:?}",
                    s.labels()
                );
                let initial = serde_json::to_value(s.sketch()).unwrap();
                s.key(KeyCode::Escape, Some(tool));
                assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                if tool == "sketch.ellipse3" {
                    let offsets = [
                        glam::DVec2::new(-10., 0.),
                        glam::DVec2::new(10., 0.),
                        glam::DVec2::new(6., 4.),
                    ];
                    let center = at.to_glam() - offsets[role];
                    for offset in offsets {
                        let p = center + offset;
                        s.full_click(p.x, p.y, tool);
                    }
                    let live = s.sketch();
                    let e = live
                        .geometry
                        .iter()
                        .rev()
                        .find_map(|g| {
                            if let GeometryElement::Ellipse(e) = g {
                                Some(e)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    assert!(
                        (live.point_position(e.center).unwrap().to_glam() - center).length() < 1e-7
                    );
                    assert!((e.major.to_glam().length() - 10.).abs() < 1e-7);
                    assert!((e.ratio - 0.5).abs() < 1e-7);
                    assert_eq!(
                        live.constraints.len(),
                        initial["constraints"].as_array().unwrap().len(),
                        "derived points have no bindings"
                    );
                    let command = s.recorded.last().unwrap();
                    assert_eq!(command.args["shared_conic"], serde_json::json!(true));
                    let snap = &command.args["points"][role]["snap"];
                    let sources: std::collections::HashSet<_> = ["curve", "curve2"]
                        .map(|key| snap[key].as_str().unwrap().parse::<uuid::Uuid>().unwrap())
                        .into_iter()
                        .collect();
                    assert_eq!(sources, [spline, crossing].into_iter().collect());
                    assert_replays(&s.recorded, before, id, &live);
                    s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                    assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                    s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                    s.doc = Document::load_from_bytes(
                        s.doc
                            .save_to_bytes(core_document::Compression::None)
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        serde_json::to_value(s.sketch()).unwrap(),
                        serde_json::to_value(live).unwrap()
                    );
                    continue;
                }
                if matches!(
                    tool,
                    "sketch.circle"
                        | "sketch.arc"
                        | "sketch.arc3"
                        | "sketch.ellipse"
                        | "sketch.ellipse_arc"
                        | "sketch.rect"
                        | "sketch.rect_center"
                        | "sketch.rect3"
                        | "sketch.rect_center3"
                ) {
                    s.full_click(at.x, at.y, tool);
                    s.mouse_move(-12., -10., tool);
                    assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                    s.key(KeyCode::Escape, Some(tool));
                    if matches!(tool, "sketch.circle" | "sketch.arc" | "sketch.arc3") {
                        assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
                    }
                    assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                }
                s.full_click(at.x, at.y, tool);
                let bound = if tool == "sketch.point" {
                    let live = s.sketch();
                    assert_eq!(
                        live.geometry.len(),
                        initial["geometry"].as_array().unwrap().len() + 1
                    );
                    let bound = crate::all_points(&live).last().unwrap().id;
                    s.key(KeyCode::Escape, Some(tool));
                    assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
                    bound
                } else if matches!(tool, "sketch.arc" | "sketch.arc3") {
                    if reverse {
                        s.key(KeyCode::Escape, Some(tool));
                        s.full_click(-12., -10., tool);
                        s.full_click(at.x, at.y, tool);
                    } else {
                        s.full_click(-12., -10., tool);
                    }
                    s.mouse_move(-5., -18., tool);
                    assert!(
                        s.overlays()
                            .iter()
                            .any(|l| super::same_color(l.color, super::pal().preview))
                    );
                    s.full_click(-5., -18., tool);
                    s.key(KeyCode::Escape, Some(tool));
                    assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
                    let live = s.sketch();
                    let arc = live
                        .geometry
                        .iter()
                        .find_map(|g| match g {
                            GeometryElement::Arc(a) => Some(a),
                            _ => None,
                        })
                        .unwrap();
                    [arc.center, arc.start, arc.end]
                        .into_iter()
                        .find(|p| (live.point_position(*p).unwrap() - at).to_glam().length() < 1e-7)
                        .unwrap()
                } else if matches!(tool, "sketch.hyperbola" | "sketch.parabola") {
                    s.key(KeyCode::Escape, Some(tool));
                    let (cx, cy) = match role {
                        0 => (at.x, at.y),
                        1 => (at.x - 10., at.y),
                        _ => (at.x - 10., at.y - 4.5),
                    };
                    for (x, y) in [
                        (cx, cy),
                        (cx + if tool == "sketch.hyperbola" { 8. } else { 10. }, cy),
                        (cx + 10., cy + 4.5),
                        (cx, cy - 6.),
                    ] {
                        s.full_click(x, y, tool);
                    }
                    s.key(KeyCode::Escape, Some(tool));
                    let live = s.sketch();
                    let c = live
                        .geometry
                        .iter()
                        .rev()
                        .find_map(|g| {
                            if let GeometryElement::Conic(c) = g {
                                Some(c)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    match role {
                        0 => c.center,
                        1 => live
                            .constraints
                            .iter()
                            .find_map(|r| {
                                if let C::InternalAlignment { element, curve, .. } = r.kind
                                    && curve == c.id
                                {
                                    Some(element)
                                } else {
                                    None
                                }
                            })
                            .unwrap(),
                        _ => c.start,
                    }
                } else if tool == "sketch.ellipse_arc" {
                    s.key(KeyCode::Escape, Some(tool));
                    let (cx, cy) = if reverse {
                        (at.x - 6., at.y - 4.)
                    } else {
                        (at.x, at.y)
                    };
                    for (x, y) in [
                        (cx, cy),
                        (cx + 10., cy),
                        (cx + 6., cy + 4.),
                        (cx - 12., cy - 3.),
                    ] {
                        s.full_click(x, y, tool);
                    }
                    s.key(KeyCode::Escape, Some(tool));
                    let e = s
                        .sketch()
                        .geometry
                        .into_iter()
                        .rev()
                        .find_map(|g| match g {
                            GeometryElement::Ellipse(e) => Some(e),
                            _ => None,
                        })
                        .unwrap();
                    if reverse {
                        e.arc.unwrap().start
                    } else {
                        e.center
                    }
                } else if tool == "sketch.ellipse" {
                    s.full_click(at.x + 10., at.y, tool);
                    s.full_click(at.x + 3., at.y + if reverse { 10. } else { 5. }, tool);
                    s.key(KeyCode::Escape, Some(tool));
                    s.sketch()
                        .geometry
                        .iter()
                        .rev()
                        .find_map(|g| match g {
                            GeometryElement::Ellipse(e) => Some(e.center),
                            GeometryElement::Circle(c) => Some(c.center),
                            _ => None,
                        })
                        .unwrap()
                } else if tool == "sketch.circle" {
                    assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                    s.mouse_move(-20., at.y, tool);
                    let overlays = s.overlays();
                    assert!(
                        overlays
                            .iter()
                            .any(|l| super::same_color(l.color, super::pal().preview))
                    );
                    s.full_click(-20., at.y, tool);
                    let live = s.sketch();
                    let circles: Vec<_> = live
                        .geometry
                        .iter()
                        .filter_map(|g| match g {
                            GeometryElement::Circle(c) => Some(c),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(circles.len(), 1);
                    assert!(
                        (circles[0].radius - (at.x + 20.)).abs() < 1e-7,
                        "radius to the fixed rim endpoint: {}",
                        circles[0].radius
                    );
                    s.key(KeyCode::Escape, Some(tool));
                    assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
                    circles[0].center
                } else if matches!(
                    tool,
                    "sketch.rect" | "sketch.rect_center" | "sketch.rect3" | "sketch.rect_center3"
                ) {
                    if matches!(tool, "sketch.rect3" | "sketch.rect_center3") {
                        s.full_click(-12., -10., tool);
                    }
                    s.mouse_move(-5., -18., tool);
                    assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                    assert!(
                        s.overlays()
                            .iter()
                            .filter(|l| super::same_color(l.color, super::pal().preview))
                            .count()
                            >= 4
                    );
                    if tool == "sketch.rect_center" {
                        s.type_text(&format!("{}", 2. * (at.x + 12.)), Some(tool));
                        s.keys("\t", Some(tool));
                        s.type_text(&format!("{}", 2. * (at.y + 10.)), Some(tool));
                        s.keys("\n", Some(tool));
                    } else if matches!(tool, "sketch.rect3" | "sketch.rect_center3") {
                        s.full_click(-5., -18., tool);
                    } else {
                        s.full_click(-12., -10., tool);
                    }
                    s.key(KeyCode::Enter, Some(tool));
                    let live = s.sketch();
                    let made: Vec<_> = crate::all_lines(&live)
                        .into_iter()
                        .filter(|l| l.id != crossing && l.id != other)
                        .collect();
                    assert_eq!(made.len(), 4);
                    for i in 0..4 {
                        assert_eq!(made[i].end, made[(i + 1) % 4].start);
                    }
                    if matches!(tool, "sketch.rect_center" | "sketch.rect_center3") {
                        live.constraints
                            .iter()
                            .find_map(|c| match c.kind {
                                C::SymmetricAboutPoint { center, .. } => Some(center),
                                _ => None,
                            })
                            .expect("centred rectangle keeps its midpoint relation")
                    } else if tool == "sketch.rect3" {
                        made[0].start
                    } else {
                        made[1].end
                    }
                } else {
                    assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
                    s.full_click(-12., -10., tool);
                    s.full_click(-5., -18., tool);
                    s.key(KeyCode::Backspace, Some(tool));
                    s.full_click(-5., -18., tool);
                    s.key(KeyCode::Escape, Some(tool));
                    let live = s.sketch();
                    let made: Vec<_> = crate::all_lines(&live)
                        .into_iter()
                        .filter(|l| l.id != crossing && l.id != other)
                        .collect();
                    assert_eq!(made.len(), 2);
                    assert_eq!(made[0].end, made[1].start);
                    made[0].start
                };
                let live = s.sketch();
                assert!(
                    (live.point_position(bound).unwrap() - at)
                        .to_glam()
                        .length()
                        < 1e-7
                );
                let sources: std::collections::HashSet<_> = live
                    .constraints
                    .iter()
                    .filter_map(|c| match c.kind {
                        C::PointOnCurve { point, curve } if point == bound => Some(curve),
                        C::PointOnLine { point, line } if point == bound => Some(line),
                        _ => None,
                    })
                    .collect();
                assert_eq!(sources, [spline, crossing].into_iter().collect());
                if !automatic
                    && !matches!(
                        tool,
                        "sketch.rect"
                            | "sketch.rect_center"
                            | "sketch.rect3"
                            | "sketch.rect_center3"
                    )
                {
                    assert!(!live.constraints.iter().any(|c| matches!(
                        c.kind,
                        C::Horizontal { .. }
                            | C::Vertical { .. }
                            | C::Parallel { .. }
                            | C::Perpendicular { .. }
                            | C::Tangent { .. }
                            | C::TangentCurves { .. }
                    )));
                }
                assert!(s.recorded.iter().any(|r| r.args.get(match tool {
                    "sketch.line" => "shared_line",
                    "sketch.polyline" => "shared_polyline",
                    "sketch.point" => "shared_point",
                    "sketch.circle" => "shared_circle",
                    "sketch.arc" | "sketch.arc3" | "sketch.ellipse" | "sketch.ellipse_arc"
                    | "sketch.hyperbola" | "sketch.parabola" => "shared_conic",
                    "sketch.rect"
                    | "sketch.rect_center"
                    | "sketch.rect3"
                    | "sketch.rect_center3" => "shared_rect",
                    _ => unreachable!(),
                }) == Some(&serde_json::json!(true))));
                assert_replays(&s.recorded, before, id, &live);
                s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                if matches!(
                    tool,
                    "sketch.point"
                        | "sketch.circle"
                        | "sketch.arc"
                        | "sketch.arc3"
                        | "sketch.ellipse"
                        | "sketch.ellipse_arc"
                        | "sketch.rect"
                        | "sketch.rect_center"
                        | "sketch.rect3"
                        | "sketch.rect_center3"
                ) {
                    assert!(s.sketch().get_geometry(bound).is_none());
                }
                s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                let bytes = s
                    .doc
                    .save_to_bytes(core_document::Compression::None)
                    .unwrap();
                s.doc = Document::load_from_bytes(bytes).unwrap();
                assert_eq!(
                    serde_json::to_value(s.sketch()).unwrap(),
                    serde_json::to_value(live).unwrap()
                );
                if tool == "sketch.rect_center" {
                    let sizes: Vec<_> = s
                        .sketch()
                        .constraints
                        .iter()
                        .filter_map(|c| match c.kind {
                            C::DistanceX { value, .. } | C::DistanceY { value, .. } => {
                                Some((c.id, value))
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(sizes.len(), 2);
                    for (constraint, size) in sizes {
                        let args = serde_json::from_value(serde_json::json!({
                            "sketch": id, "constraint": constraint, "value": size * 1.5
                        }))
                        .unwrap();
                        let mut ctx =
                            WorkbenchRuntimeContext::new(&mut s.doc, CAM_POS, [0.; 3], VIEWPORT);
                        s.wb.run_command("sketch.set_value", &args, &mut ctx)
                            .unwrap();
                    }
                }
                // Source-edit boundary: translate the fixture, leaving the drawn
                // endpoint's initial approximation where it was. Both memberships
                // must carry it to the translated intersection.
                let mut moved = s.sketch();
                let mut fixed = std::collections::HashSet::new();
                for c in &mut moved.constraints {
                    if let C::FixedPoint { point, position } = &mut c.kind {
                        position.y += 3.;
                        fixed.insert(*point);
                    }
                }
                for g in &mut moved.geometry {
                    if let GeometryElement::Point(p) = g
                        && fixed.contains(&p.id)
                    {
                        p.position.y += 3.;
                    }
                }
                let outcome = wb_sketch::operations::solve(&mut moved).unwrap();
                assert!(
                    matches!(
                        outcome,
                        wb_sketch::operations::SolveOutcome::Converged { .. }
                    ),
                    "{tool}/{role}/{family}/{reverse}/{automatic}: {outcome:?}"
                );
                let p = moved.point_position(bound).unwrap();
                if tool == "sketch.rect_center" {
                    let lines: Vec<_> = crate::all_lines(&moved)
                        .into_iter()
                        .filter(|l| l.id != crossing && l.id != other)
                        .collect();
                    let mean = lines.iter().fold(glam::DVec2::ZERO, |sum, l| {
                        sum + moved.point_position(l.start).unwrap().to_glam()
                    }) / 4.;
                    assert!((mean - p.to_glam()).length() < 1e-6);
                    for (line, want) in [
                        (&lines[0], 3. * (at.x + 12.)),
                        (&lines[1], 3. * (at.y + 10.)),
                    ] {
                        let a = moved.point_position(line.start).unwrap();
                        let b = moved.point_position(line.end).unwrap();
                        assert!(((a - b).to_glam().length() - want).abs() < 1e-6);
                    }
                }
                if family == 6 {
                    // At a double root geometric position error is proportional
                    // to sqrt(residual). Judge the two independent equations in
                    // their length units against the requested solver tolerance.
                    let eps = 2. * 30. * moved.solver.tolerance;
                    assert!((p.y - 10.).abs() < eps);
                    assert!(((p.x - 8.).powi(2) / 10. - (p.y - 10.)).abs() < eps);
                } else {
                    assert!(
                        (p - Vec2D::new(at.x, at.y + 3.)).to_glam().length() < 1e-6,
                        "family {family}: {p:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_line_at_a_circular_crossing_keeps_both_selected_sources() {
    use wb_sketch::sketch::{Arc, Circle, ConstraintKind as C, Line, Point, Vec2D};
    for arc in [false, true] {
        for reverse in [false, true] {
            let (mut s, id, _) = session_on_a_sketch();
            let (round, line, other) = s.edit_sketch(|sketch| {
                let mut p = |x, y| {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
                };
                let (center, start, end, a, b, c, d) = (
                    p(8., 7.),
                    p(13., 7.),
                    p(3., 7.),
                    p(-20., 10.),
                    p(25., 10.),
                    p(-20., 10.002),
                    p(25., 10.002),
                );
                let round = sketch.add_geometry(if arc {
                    GeometryElement::Arc(Arc::new(center, start, end, 5.))
                } else {
                    GeometryElement::Circle(Circle::new(center, 5.))
                });
                let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
                let other = sketch.add_geometry(GeometryElement::Line(Line::new(c, d)));
                for (point, position) in [
                    (center, Vec2D::new(8., 7.)),
                    (start, Vec2D::new(13., 7.)),
                    (end, Vec2D::new(3., 7.)),
                    (a, Vec2D::new(-20., 10.)),
                    (b, Vec2D::new(25., 10.)),
                    (c, Vec2D::new(-20., 10.002)),
                    (d, Vec2D::new(25., 10.002)),
                ] {
                    sketch.add_constraint(C::FixedPoint { point, position });
                }
                sketch.add_constraint(C::Radius {
                    circle: round,
                    radius: 5.,
                });
                if reverse {
                    sketch.geometry.reverse();
                }
                (round, line, other)
            });
            let before = s.doc.clone();
            s.mouse_move(12., 10., "sketch.line");
            assert!(s.labels().iter().any(|l| l.text == "Intersection"));
            s.full_click(12., 10., "sketch.line");
            s.full_click(22., -12., "sketch.line");
            s.full_click(28., -20., "sketch.line");
            s.key(KeyCode::Escape, Some("sketch.line"));
            let live = s.sketch();
            let made: Vec<_> = crate::all_lines(&live)
                .into_iter()
                .filter(|l| l.id != line && l.id != other)
                .collect();
            assert_eq!(made.len(), 2);
            assert_eq!(made[0].end, made[1].start);
            let bound = made[0].start;
            let p = live.point_position(bound).unwrap();
            assert!((p.x - 12.).hypot(p.y - 10.) < 1e-7);
            let sources: std::collections::HashSet<_> = live
                .constraints
                .iter()
                .filter_map(|c| match c.kind {
                    C::PointOnCircle { point, circle } if point == bound => Some(circle),
                    C::PointOnLine { point, line } if point == bound => Some(line),
                    _ => None,
                })
                .collect();
            assert_eq!(sources, [round, line].into_iter().collect());
            assert_replays(&s.recorded, before, id, &live);
        }
    }
}

#[test]
fn external_history_invalidates_local_line_undo_before_the_next_preview() {
    for tool in ["sketch.line", "sketch.polyline"] {
        let (mut s, _, _) = session_on_a_sketch();
        let mut journal = core_document::history::OpJournal::new(32);
        journal.note(&mut s.doc);
        s.journal = Some(journal);
        s.full_click(10., 10., tool);
        s.full_click(25., 14., tool);
        let junction = crate::all_lines(&s.sketch())[0].end;
        s.edit_sketch(|sketch| {
            let Some(GeometryElement::Point(point)) = sketch.get_geometry_mut(junction) else {
                panic!("junction")
            };
            point.position.y += 8.;
        });
        s.journal.as_mut().unwrap().note(&mut s.doc);
        let external = serde_json::to_value(s.sketch()).unwrap();
        s.key(KeyCode::Backspace, Some(tool));
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            external,
            "a foreign edit owns the top gesture"
        );
        s.mouse_move(40., 17., tool);
        let at = s.px_of(25., 22.);
        assert!(s.overlays().iter().any(|line| super::same_color(
            line.color,
            super::pal().preview
        ) && (line.start[0] - at.0).abs() < 0.1
            && (line.start[1] - at.1).abs() < 0.1));
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
        // A real input precedes on_frame: a dead junction must not be refreshed.
        s.mouse_move(40., 17., tool);
        assert_eq!(crate::all_lines(&s.sketch()).len(), 0);
        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
        s.mouse_move(40., 17., tool);
        s.full_click(40., 17., tool);
        assert_eq!(
            crate::all_lines(&s.sketch()).len(),
            1,
            "redo does not resurrect the old input phase"
        );
        let before_held = serde_json::to_value(s.sketch()).unwrap();
        let prior = s.journal.as_ref().unwrap().undo_entry_id();
        s.journal.as_mut().unwrap().hold(true);
        s.full_click(60., 29., tool);
        s.full_click(72., 46., tool);
        assert_eq!(s.journal.as_ref().unwrap().undo_entry_id(), prior);
        let held = serde_json::to_value(s.sketch()).unwrap();
        s.key(KeyCode::Backspace, Some(tool));
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            held,
            "held links cannot borrow the prior gesture ID"
        );
        s.key(KeyCode::Escape, Some(tool));
        s.journal.as_mut().unwrap().hold(false);
        s.journal.as_mut().unwrap().note(&mut s.doc);
        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
        assert_eq!(
            serde_json::to_value(s.sketch()).unwrap(),
            before_held,
            "the held run stays one host undo step"
        );
    }
}

#[test]
fn selected_crossing_sources_survive_deferred_clicks_history_and_replay() {
    use std::collections::HashSet;
    use wb_sketch::sketch::{ConstraintKind, Line, Point, Vec2D};

    for tool in ["sketch.line", "sketch.polyline", "sketch.circle"] {
        for reverse in [false, true] {
            let (mut s, id, _) = session_on_a_sketch();
            let mut curves = Vec::new();
            // Fixture boundary: two crossing lines and a third within the
            // materialization epsilon, with coincident midpoints nearby.
            s.edit_sketch(|sketch| {
                for (a, b) in [
                    ((5., 10.), (35., 10.)),
                    ((6., 10.0001), (34., 10.0001)),
                    ((20., -5.), (20., 25.)),
                ] {
                    let ends = [a, b].map(|(x, y)| {
                        let position = Vec2D::new(x, y);
                        let point =
                            sketch.add_geometry(GeometryElement::Point(Point::new(position)));
                        sketch.add_constraint(ConstraintKind::FixedPoint { point, position });
                        point
                    });
                    curves.push(
                        sketch.add_geometry(GeometryElement::Line(Line::new(ends[0], ends[1]))),
                    );
                }
                if reverse {
                    sketch.geometry.reverse();
                }
            });
            s.recorded.clear();
            let before = s.doc.clone();
            let initial = serde_json::to_value(s.sketch()).unwrap();
            let mut journal = core_document::history::OpJournal::new(16);
            journal.note(&mut s.doc);
            s.mouse_move(20., 10., tool);
            assert!(s.labels().iter().any(|label| label.text == "Intersection"));
            s.full_click(20., 10., tool);
            assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
            s.full_click(28., 22., tool);
            s.key(KeyCode::Escape, Some(tool));
            journal.note(&mut s.doc);
            let live = s.sketch();
            let judge = |sketch: &Sketch| {
                let at = all_points(sketch)
                    .into_iter()
                    .find(|p| {
                        (p.position.x - 20.).abs() < 1e-6 && (p.position.y - 10.).abs() < 1e-6
                    })
                    .unwrap()
                    .id;
                let sources: HashSet<_> = sketch
                    .constraints
                    .iter()
                    .filter_map(|c| match c.kind {
                        ConstraintKind::PointOnLine { point, line } if point == at => Some(line),
                        ConstraintKind::Midpoint { point, .. } if point == at => {
                            panic!("an intersection is not a midpoint binding")
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    sources,
                    HashSet::from([curves[0], curves[2]]),
                    "{tool}, reverse={reverse}"
                );
            };
            judge(&live);
            let recorded = s.recorded.last().unwrap();
            let pick = &recorded.args["points"][0]["snap"];
            assert_eq!(pick["kind"], "intersection");
            if tool == "sketch.line" && !reverse {
                let mut invalid = recorded.args.clone();
                invalid["points"][0]["snap"]["curve2"] = serde_json::json!(9999);
                let mut rejected = before.clone();
                let mut wb = SketchWorkbench::default();
                let mut ctx =
                    WorkbenchRuntimeContext::new(&mut rejected, CAM_POS, [0.; 3], VIEWPORT);
                assert!(wb.run_command("sketch.draw", &invalid, &mut ctx).is_err());
                assert_eq!(rejected.get_feature_data(id), before.get_feature_data(id));
            }
            let mut recorded = s.recorded.clone();
            recorded.last_mut().unwrap().args["points"][0]["x"] = serde_json::json!(-7.);
            recorded.last_mut().unwrap().args["points"][0]["y"] = serde_json::json!(31.);
            let (peer, _) = replay(&recorded, before.clone(), None);
            judge(
                &SketchFeature::from_json(peer.get_feature_data(id).unwrap())
                    .unwrap()
                    .sketch,
            );
            assert_replays(&s.recorded, before, id, &live);
            journal.undo(&mut s.doc).unwrap();
            assert_eq!(serde_json::to_value(s.sketch()).unwrap(), initial);
            journal.redo(&mut s.doc).unwrap();
            let bytes = s
                .doc
                .save_to_bytes(core_document::Compression::None)
                .unwrap();
            s.doc = Document::load_from_bytes(bytes).unwrap();
            assert_eq!(
                serde_json::to_value(s.sketch()).unwrap(),
                serde_json::to_value(live).unwrap()
            );
        }
    }
}

#[test]
fn typed_lengths_polyline_arcs_and_a_spline_replay_too() {
    let (mut s, id, before) = session_on_a_sketch();
    // A line whose length is typed, committed with Enter: a driving
    // distance comes with it.
    s.full_click(1.0, 1.0, "sketch.line");
    s.mouse_move(6.0, 1.5, "sketch.line");
    s.keys("12\n\x1b", Some("sketch.line"));
    // A polyline: a straight segment, then tangent arcs.
    s.full_click(-12.0, -2.0, "sketch.polyline");
    s.full_click(-6.0, -2.0, "sketch.polyline");
    s.action("sketch.polyline_arc", "sketch.polyline");
    s.full_click(-3.0, -5.0, "sketch.polyline");
    s.key(KeyCode::Escape, Some("sketch.polyline"));
    // A spline through four points, finished with Enter.
    for (x, y) in [(-10.0, 6.0), (-7.0, 9.0), (-3.0, 6.0), (0.0, 9.0)] {
        s.full_click(x, y, "sketch.bspline");
    }
    s.key(KeyCode::Enter, Some("sketch.bspline"));
    s.key(KeyCode::A, None);

    let sketch = s.sketch();
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, wb_sketch::sketch::ConstraintKind::Distance { distance, .. } if (distance - 12.0).abs() < 1e-4)),
        "the typed length is a constraint"
    );
    let done = summary(&sketch);
    assert!(
        done.1.contains(&"arc") && done.1.contains(&"bspline"),
        "{done:?}"
    );
    assert_replays(&s.recorded, before, id, &s.sketch());
}

#[test]
fn typed_sweeps_widths_and_a_first_point_replay_too() {
    let (mut s, id, _) = session_on_a_sketch();
    // A nearby fixed point must not steal a typed first point, even though
    // it lies inside the mouse snap tolerance.
    let nearby = s.edit_sketch(|sketch| {
        let position = wb_sketch::sketch::Vec2D::new(3.1, 2.0);
        let point = sketch.add_geometry(GeometryElement::Point(wb_sketch::sketch::Point::new(
            position,
        )));
        sketch.add_constraint(wb_sketch::sketch::ConstraintKind::FixedPoint { point, position });
        point
    });
    let before = s.doc.clone();
    s.mouse_move(4.0, 4.0, "sketch.line");
    s.keys("3\t2\n", Some("sketch.line"));
    s.full_click(9.0, 5.0, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));
    s.full_click(12., -8., "sketch.circle");
    s.type_text("2*(3+2)", Some("sketch.circle"));
    s.keys("\n\x1b", Some("sketch.circle"));
    // An arc turned through a typed sweep.
    s.full_click(-8.0, 3.0, "sketch.arc");
    s.full_click(-3.0, 3.0, "sketch.arc");
    s.keys("45\n", Some("sketch.arc"));
    // A slot of a typed width, the cursor crossing back over its start
    // after its length is typed.
    s.full_click(2.0, -6.0, "sketch.slot");
    s.mouse_move(8.0, -5.5, "sketch.slot");
    s.key(KeyCode::Key9, Some("sketch.slot"));
    s.mouse_move(-4.0, -6.5, "sketch.slot");
    s.keys("\t\t2\n", Some("sketch.slot"));
    s.key(KeyCode::A, None);

    let sketch = s.sketch();
    assert!(sketch.constraints.iter().any(|c| matches!(c.kind, wb_sketch::sketch::ConstraintKind::Diameter { diameter, .. } if diameter == 10.)));
    let line = crate::all_lines(&sketch).into_iter().next().unwrap();
    assert_ne!(
        line.start, nearby,
        "typed input does not share the nearby vertex"
    );
    let start = sketch.point_position(line.start).unwrap();
    assert!(
        (start.x - 3.0).abs() < 1e-6 && (start.y - 2.0).abs() < 1e-6,
        "{start:?}"
    );
    let done = summary(&sketch);
    assert!(done.1.contains(&"arc"), "{done:?}");
    assert_replays(&s.recorded, before, id, &s.sketch());
}

impl Harness {
    /// A menu command of the bench, as the Edit menu runs it.
    fn edit_menu(&mut self, id: &str) {
        let recorded = self.with_ctx(|wb, ctx| {
            wb.on_command(id, &core_document::MenuScope::EditMenu, ctx);
            HookOutcome::take(ctx).recorded
        });
        self.recorded.extend(recorded);
    }
}

/// What a sketch comes to: see [`summary`].
type Summary = (Vec<(i64, i64)>, Vec<&'static str>, Vec<String>);

/// Every sketch of a document by name, with its plane and what it holds.
fn all_sketches(doc: &Document) -> Vec<(String, [i64; 9], Summary)> {
    let r = |v: f64| (v * 1000.0).round() as i64;
    let mut out: Vec<_> = doc
        .feature_tree()
        .all_nodes()
        .filter_map(|(_, n)| {
            let f = SketchFeature::from_json(&n.data).ok()?;
            let p = f.plane;
            let plane = [
                r(p.origin[0]),
                r(p.origin[1]),
                r(p.origin[2]),
                r(p.normal[0]),
                r(p.normal[1]),
                r(p.normal[2]),
                r(p.x_axis[0]),
                r(p.x_axis[1]),
                r(p.x_axis[2]),
            ];
            Some((n.name.clone(), plane, summary(&f.sketch)))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn arrays_clipboard_mirrors_planes_and_clearing_replay_too() {
    let (mut s, _, before) = session_on_a_sketch();
    s.full_click(2.0, 2.0, "sketch.rect");
    s.full_click(10.0, 8.0, "sketch.rect");
    s.key(KeyCode::Enter, Some("sketch.rect"));
    s.full_click(-8.0, 6.0, "sketch.circle");
    s.full_click(-5.5, 6.0, "sketch.circle");
    s.key(KeyCode::Escape, Some("sketch.select"));
    // The circle, repeated in a two by two array.
    s.full_click(-5.5, 6.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.array"));
    s.key(KeyCode::Escape, Some("sketch.select"));
    // The rectangle's bottom edge cut, and pasted where the cursor is.
    s.full_click(6.0, 2.0, "sketch.select");
    s.edit_menu("edit.cut");
    s.mouse_move(-12.0, -10.0, "sketch.select");
    s.edit_menu("edit.paste");
    s.key(KeyCode::Escape, Some("sketch.select"));
    // A mirrored copy as a sketch of its own, then this one's plane
    // flipped and every constraint cleared.
    s.key(KeyCode::A, Some("sketch.mirror_sketch"));
    s.key(KeyCode::A, Some("sketch.reorient"));
    s.key(KeyCode::A, Some("sketch.delete_all_constraints"));
    s.key(KeyCode::A, None);

    let kinds: Vec<&str> = s.recorded.iter().map(|r| r.id.as_str()).collect();
    for kind in [
        "sketch.array",
        "sketch.delete",
        "sketch.paste",
        "sketch.mirror_sketch",
        "sketch.set_plane",
    ] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    let done = all_sketches(&s.doc);
    assert_eq!(done.len(), 2, "the sketch and its mirror");

    let (doc, script) = replay(&s.recorded, before, None);
    assert_eq!(all_sketches(&doc), done, "{script}");
}

#[test]
fn turned_rectangles_and_a_frame_replay_too() {
    let (mut s, id, before) = session_on_a_sketch();
    for (x, y) in [(1.0, 1.0), (9.0, 7.0), (0.0, 6.5)] {
        s.full_click(x, y, "sketch.rect:3pt");
    }
    s.key(KeyCode::Enter, Some("sketch.rect:3pt"));
    for (x, y) in [(-10.0, 1.0), (-6.0, 4.0), (-16.0, 9.0)] {
        s.full_click(x, y, "sketch.rect:center3pt");
    }
    s.key(KeyCode::Enter, Some("sketch.rect:center3pt"));
    for (x, y) in [(2.0, -12.0), (14.0, -3.0)] {
        s.full_click(x, y, "sketch.rect:frame");
    }
    s.key(KeyCode::Enter, Some("sketch.rect:frame"));
    s.key(KeyCode::A, None);

    let done = summary(&s.sketch());
    assert_eq!(
        done.1.iter().filter(|c| **c == "line").count(),
        16,
        "{done:?}"
    );
    assert_eq!(
        s.recorded.iter().filter(|r| r.id == "sketch.draw").count(),
        3
    );
    assert_replays(&s.recorded, before, id, &s.sketch());
}

#[test]
fn rotated_rectangles_keep_directed_edges_sources_numeric_sizes_and_history() {
    use wb_sketch::sketch::{ConstraintKind as C, Point, Vec2D};
    for centered in [false, true] {
        let tool = if centered {
            "sketch.rect_center3"
        } else {
            "sketch.rect3"
        };
        for side in [-1., 1.] {
            for boundary in [-180., 180., 270., 360., 450., 720.] {
                for epsilon in [-0.001, 0., 0.001] {
                    let deg: f64 = boundary + epsilon;
                    let u = glam::DVec2::from_angle(deg.to_radians());
                    let n = u.perp() * side;
                    let a = glam::DVec2::new(7., 6.);
                    let b = a + u * if centered { 10. } else { 12. };
                    // The third click is deliberately away from a contour corner.
                    let toward = if centered {
                        a + glam::DVec2::from_angle(deg.to_radians() + side * 2. * 0.6_f64.asin())
                            * 17.
                    } else {
                        b + n * 8. + u * 3.
                    };
                    let (mut s, id, _) = session_on_a_sketch();
                    let ids = s.edit_sketch(|sketch| {
                        [a, b, toward].map(|p| {
                            sketch.add_geometry(GeometryElement::Point(Point::new(
                                Vec2D::from_glam(p),
                            )))
                        })
                    });
                    let before = s.doc.clone();
                    let mut journal = core_document::history::OpJournal::new(16);
                    journal.note(&mut s.doc);
                    s.journal = Some(journal);
                    s.full_click(a.x, a.y, tool);
                    s.mouse_move(b.x, b.y, tool);
                    assert!(
                        s.overlays()
                            .iter()
                            .any(|l| super::same_color(l.color, super::pal().preview))
                    );
                    s.full_click(b.x, b.y, tool);
                    assert_eq!(s.sketch().geometry.len(), 3);
                    s.key(KeyCode::Escape, Some(tool));
                    assert_eq!(s.sketch().geometry.len(), 3);
                    s.full_click(a.x, a.y, tool);
                    s.full_click(b.x, b.y, tool);
                    s.mouse_move(toward.x, toward.y, tool);
                    assert!(
                        s.overlays()
                            .iter()
                            .filter(|l| super::same_color(l.color, super::pal().preview))
                            .count()
                            >= 4
                    );
                    s.full_click(toward.x, toward.y, tool);
                    s.key(KeyCode::Enter, Some(tool));
                    let live = s.sketch();
                    let lines = crate::all_lines(&live);
                    assert_eq!(lines.len(), 4);
                    assert_eq!(lines[0].start, ids[usize::from(centered)]);
                    if !centered {
                        assert_eq!(lines[0].end, ids[1]);
                    }
                    for i in 0..4 {
                        assert_eq!(lines[i].end, lines[(i + 1) % 4].start);
                        assert_ne!(
                            lines[i].start, ids[2],
                            "direction pick is not a contour vertex"
                        );
                    }
                    let lengths: Vec<_> = lines
                        .iter()
                        .map(|l| {
                            (live.point_position(l.end).unwrap()
                                - live.point_position(l.start).unwrap())
                            .to_glam()
                            .length()
                        })
                        .collect();
                    let expected = if centered {
                        [12., 16., 12., 16.]
                    } else {
                        [12., 8., 12., 8.]
                    };
                    for (got, want) in lengths.iter().zip(expected) {
                        assert!(
                            (got - want).abs() < 1e-6,
                            "{tool} {deg} {side}: {lengths:?}"
                        );
                    }
                    if !centered {
                        let end = live.point_position(lines[1].end).unwrap().to_glam();
                        assert!((end - (b + n * 8.)).length() < 1e-6);
                    }
                    assert_eq!(
                        live.constraints
                            .iter()
                            .filter(|c| matches!(c.kind, C::Parallel { .. }))
                            .count(),
                        2
                    );
                    assert_eq!(
                        live.constraints
                            .iter()
                            .filter(|c| matches!(c.kind, C::Perpendicular { .. }))
                            .count(),
                        1
                    );
                    assert!(
                        !live
                            .constraints
                            .iter()
                            .any(|c| matches!(c.kind, C::Horizontal { .. } | C::Vertical { .. }))
                    );
                    assert!(
                        s.recorded.last().unwrap().args.get("shared_rect")
                            == Some(&serde_json::json!(true))
                    );
                    assert_replays(&s.recorded, before, id, &live);
                    s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                    assert_eq!(s.sketch().geometry.len(), 3);
                    s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                    s.doc = Document::load_from_bytes(
                        s.doc
                            .save_to_bytes(core_document::Compression::None)
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(point_bits(&s.sketch()), point_bits(&live));
                    assert_eq!(
                        serde_json::to_value(s.sketch()).unwrap(),
                        serde_json::to_value(&live).unwrap()
                    );
                }
            }
            let (mut s, id, before) = session_on_a_sketch();
            s.mouse_move(7., 6., tool);
            s.keys("7\t6\n", Some(tool));
            s.mouse_move(16., 11., tool);
            s.type_text(if centered { "5+5" } else { "6+6" }, Some(tool));
            s.keys("\t", Some(tool));
            s.type_text("450", Some(tool));
            s.keys("\n", Some(tool));
            s.mouse_move(7. - side * 8., if centered { 10. } else { 18. }, tool);
            s.type_text(if centered { "6+6" } else { "4+4" }, Some(tool));
            s.keys("\n", Some(tool));
            s.key(KeyCode::Enter, Some(tool));
            let live = s.sketch();
            let lines = crate::all_lines(&live);
            assert_eq!(lines.len(), 4);
            let length = |i: usize| {
                (live.point_position(lines[i].end).unwrap()
                    - live.point_position(lines[i].start).unwrap())
                .to_glam()
                .length()
            };
            assert!((length(0) - 12.).abs() < 1e-6);
            assert!((length(1) - if centered { 16. } else { 8. }).abs() < 1e-6);
            assert!(live.constraints.iter().any(|c| matches!(c.kind, C::Distance { distance, .. } if (distance - if centered {16.} else {8.}).abs() < 1e-8)));
            if !centered {
                let d = (live.point_position(lines[0].end).unwrap()
                    - live.point_position(lines[0].start).unwrap())
                .to_glam();
                assert!(d.x.abs() < 1e-6 && (d.y - 12.).abs() < 1e-6);
            }
            assert_replays(&s.recorded, before, id, &live);
        }
    }
}

#[test]
fn circular_arcs_keep_picked_roles_directed_preview_and_history() {
    use wb_sketch::sketch::{Point, Vec2D};
    for tool in ["sketch.arc3", "sketch.arc"] {
        for boundary in [-180., 180., 270., 360., 450., 720.] {
            for epsilon in [-0.001, 0., 0.001] {
                for sweep in [-270., -180., -90., 90., 180., 270.] {
                    let angle: f64 = boundary + epsilon;
                    let sweep: f64 = sweep;
                    let center = glam::DVec2::new(3., 4.);
                    let at = |deg: f64| center + 8. * glam::DVec2::from_angle(deg.to_radians());
                    let positions = if tool == "sketch.arc" {
                        [
                            center,
                            at(angle),
                            center
                                + (if epsilon == 0. { 1. } else { 1.5 })
                                    * (at(angle + sweep) - center),
                        ]
                    } else {
                        [at(angle), at(angle + sweep), at(angle + sweep / 2.)]
                    };
                    let (mut s, id, _) = session_on_a_sketch();
                    let ids = s.edit_sketch(|sketch| {
                        positions.map(|p| {
                            sketch.add_geometry(GeometryElement::Point(Point::new(
                                Vec2D::from_glam(p),
                            )))
                        })
                    });
                    let before = s.doc.clone();
                    let mut journal = core_document::history::OpJournal::new(8);
                    journal.note(&mut s.doc);
                    s.journal = Some(journal);
                    for p in &positions[..2] {
                        s.full_click(p.x, p.y, tool);
                    }
                    assert_eq!(s.sketch().geometry.len(), 3);
                    s.mouse_move(positions[2].x, positions[2].y, tool);
                    assert!(s.labels().iter().any(|l| l.text == "Endpoint"));
                    let preview: Vec<_> = s
                        .overlays()
                        .into_iter()
                        .filter(|l| super::same_color(l.color, super::pal().preview))
                        .collect();
                    assert_eq!(preview.len(), 64, "one sampled arc");
                    let px = |s: &mut Harness, p: glam::DVec2| {
                        let (x, y) = s.px_of(p.x, p.y);
                        glam::Vec2::new(x, y)
                    };
                    for (line, i) in preview.iter().zip(0..64) {
                        let sweep = if tool == "sketch.arc" {
                            sweep.rem_euclid(360.)
                        } else {
                            sweep
                        };
                        let want = px(&mut s, at(angle + sweep * i as f64 / 64.));
                        assert!((glam::Vec2::from_array(line.start) - want).length() < 0.01);
                    }
                    s.full_click(positions[2].x, positions[2].y, tool);
                    let live = s.sketch();
                    let arc = live
                        .geometry
                        .iter()
                        .find_map(|g| match g {
                            GeometryElement::Arc(a) => Some(a),
                            _ => None,
                        })
                        .unwrap();
                    let ends = if tool == "sketch.arc" {
                        assert_eq!(arc.center, ids[0]);
                        assert_ne!(arc.end, ids[2], "projected end has no cursor identity");
                        [ids[1], arc.end]
                    } else if sweep > 0. {
                        [ids[0], ids[1]]
                    } else {
                        [ids[1], ids[0]]
                    };
                    assert_eq!([arc.start, arc.end], ends);
                    assert_eq!(
                        crate::all_points(&live).len(),
                        4,
                        "only the derived point is new"
                    );
                    assert!((arc.radius - 8.).abs() < 1e-8);
                    assert!(
                        (live.point_position(arc.center).unwrap().to_glam() - center).length()
                            < 1e-8
                    );
                    assert!(
                        live.constraints.is_empty(),
                        "the on-arc pick is construction input only"
                    );
                    assert_eq!(
                        s.recorded.last().unwrap().args.get("shared_conic"),
                        Some(&serde_json::json!(true))
                    );
                    assert_replays(&s.recorded, before, id, &live);
                    s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                    assert_eq!(s.sketch().geometry.len(), 3);
                    s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                    s.doc = Document::load_from_bytes(
                        s.doc
                            .save_to_bytes(core_document::Compression::None)
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        serde_json::to_value(s.sketch()).unwrap(),
                        serde_json::to_value(&live).unwrap()
                    );
                }
            }
        }
    }
    // A rejected final click and a repeated endpoint leave both collected
    // roles available. Cancel exits without materialising a partial curve.
    let tool = "sketch.arc3";
    let (mut s, id, _) = session_on_a_sketch();
    s.edit_sketch(|sketch| {
        for x in [20., 40., 30.] {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 20.))));
        }
    });
    let before = s.doc.clone();
    for (x, y) in [(20., 20.), (40., 20.), (30., 20.), (40., 20.)] {
        s.full_click(x, y, tool);
    }
    s.mouse_move(30., 20., tool);
    assert!(
        s.labels().iter().any(|l| l.text.contains("Collinear")),
        "{:?}; {:?}",
        s.labels(),
        s.sketch()
    );
    assert_eq!(s.sketch().geometry.len(), 3);
    s.key(KeyCode::Escape, Some(tool));
    assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
    for side in [-1., 1.] {
        s.mouse_move(20., 20., tool);
        s.keys("20\t20\n", Some(tool));
        s.mouse_move(40., 20., tool);
        s.keys("20\t0\n", Some(tool));
        s.mouse_move(30., 20. + side * 15., tool);
        s.type_text("5+10", Some(tool));
        s.keys("\n", Some(tool));
        let live = s.sketch();
        let radius = live
            .geometry
            .iter()
            .rev()
            .find_map(|g| match g {
                GeometryElement::Arc(a) => Some(a.radius),
                _ => None,
            })
            .unwrap();
        assert!((radius - 15.).abs() < 1e-7);
        assert!(live.constraints.iter().any(|c| matches!(
            c.kind,
            wb_sketch::sketch::ConstraintKind::Radius { radius: 15., .. }
        )));
        assert_replays(&s.recorded, before.clone(), id, &live);
    }
}

#[test]
fn three_point_circle_preview_free_center_rejection_numbers_and_replay() {
    use wb_sketch::sketch::{ConstraintKind as C, Point, Vec2D};
    let tool = "sketch.circle3";
    for reverse in [false, true] {
        for deg in [-180_f64, 180., 270., 360., 450., 720.] {
            let center = glam::DVec2::new(30., 20.);
            let at = |angle: f64| center + 10. * glam::DVec2::from_angle(angle.to_radians());
            let mut positions = [at(deg), at(deg + 120.), at(deg + 240.)];
            if reverse {
                positions.reverse();
            }
            let (mut s, id, _) = session_on_a_sketch();
            let original_center = s.edit_sketch(|sketch| {
                for p in positions {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(p))));
                }
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(center))))
            });
            let before = s.doc.clone();
            let mut journal = core_document::history::OpJournal::new(8);
            journal.note(&mut s.doc);
            s.journal = Some(journal);
            for p in &positions[..2] {
                s.full_click(p.x, p.y, tool);
            }
            assert_eq!(s.sketch().geometry.len(), 4);
            s.mouse_move(positions[2].x, positions[2].y, tool);
            assert!(s.labels().iter().any(|l| l.text == "Endpoint"));
            let preview: Vec<_> = s
                .overlays()
                .into_iter()
                .filter(|l| super::same_color(l.color, super::pal().preview))
                .collect();
            assert_eq!(preview.len(), 48);
            for line in preview {
                for p in [line.start, line.end] {
                    let (cx, cy) = s.px_of(center.x, center.y);
                    let (rx, ry) = s.px_of(center.x + 10., center.y);
                    let distance = (glam::Vec2::from_array(p) - glam::Vec2::new(cx, cy)).length();
                    assert!((distance - glam::Vec2::new(rx - cx, ry - cy).length()).abs() < 0.01);
                }
            }
            s.full_click(positions[2].x, positions[2].y, tool);
            let live = s.sketch();
            let circle = live
                .geometry
                .iter()
                .find_map(|g| {
                    if let GeometryElement::Circle(c) = g {
                        Some(c)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(live.geometry.len(), 6);
            assert_ne!(circle.center, original_center);
            assert!((circle.radius - 10.).abs() < 1e-8);
            assert!(
                (live.point_position(circle.center).unwrap().to_glam() - center).length() < 1e-8
            );
            assert!(
                live.constraints.is_empty(),
                "rim picks and derived centre have no bindings"
            );
            assert_eq!(
                s.recorded.last().unwrap().args.get("shared_conic"),
                Some(&serde_json::json!(true))
            );
            assert_replays(&s.recorded, before, id, &live);
            s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
            assert_eq!(s.sketch().geometry.len(), 4);
            s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
            s.doc = Document::load_from_bytes(
                s.doc
                    .save_to_bytes(core_document::Compression::None)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(s.sketch()).unwrap(),
                serde_json::to_value(live).unwrap()
            );
        }
    }
    for side in [-1., 1.] {
        let (mut s, id, _) = session_on_a_sketch();
        s.edit_sketch(|sketch| {
            for x in [20., 30., 40.] {
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 20.))));
            }
        });
        let before = s.doc.clone();
        for (x, y) in [(20., 20.), (20., 20.), (40., 20.), (30., 20.), (40., 20.)] {
            s.full_click(x, y, tool);
        }
        s.mouse_move(30., 20., tool);
        assert!(s.labels().iter().any(|l| l.text.contains("Collinear")));
        assert_eq!(s.sketch().geometry.len(), 3);
        s.full_click(30., 20. + side * 10., tool);
        assert_eq!(s.sketch().geometry.len(), 5, "retry retains both rim picks");
        assert_replays(&s.recorded, before.clone(), id, &s.sketch());
        s.full_click(20., 20., tool);
        s.full_click(40., 20., tool);
        s.key(KeyCode::Escape, Some(tool));
        assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
        assert_eq!(s.sketch().geometry.len(), 5);
        s.mouse_move(20., 50., tool);
        s.keys("20\t50\n", Some(tool));
        s.mouse_move(40., 50., tool);
        s.keys("20\t0\n", Some(tool));
        s.mouse_move(30., 50. + side * 15., tool);
        s.type_text("10+20", Some(tool));
        s.keys("\n", Some(tool));
        let live = s.sketch();
        assert!(
            live.geometry
                .iter()
                .any(|g| matches!(g, GeometryElement::Circle(c) if (c.radius - 15.).abs() < 1e-7))
        );
        assert!(
            live.constraints
                .iter()
                .any(|c| matches!(c.kind, C::Diameter { diameter: 30., .. }))
        );
        assert_replays(&s.recorded, before, id, &live);
    }
}

#[test]
fn focal_conics_projected_picks_preview_bindings_numbers_and_history() {
    use wb_sketch::sketch::{ConstraintKind as C, InternalRole, Point, Vec2D};
    for tool in ["sketch.hyperbola", "sketch.parabola"] {
        let parabola = tool == "sketch.parabola";
        for deg in [-180_f64, 180., 270., 360., 450., 720.] {
            for epsilon in [-0.001, 0., 0.001] {
                for branch in [-1., 1.] {
                    if parabola && branch < 0. {
                        continue;
                    }
                    for direction in [-1., 1.] {
                        let center = glam::DVec2::new(20., 20.);
                        let u = glam::DVec2::from_angle((deg + epsilon).to_radians());
                        let at = |t: f64| {
                            if parabola {
                                center + u * ((6. * t).powi(2) / 16.) + u.perp() * (6. * t)
                            } else {
                                center + u * (branch * 8. * t.cosh()) + u.perp() * (6. * t.sinh())
                            }
                        };
                        let positions = [
                            center,
                            center + u * if parabola { 4. } else { branch * 8. },
                            at(direction * 0.6)
                                - if parabola { u * 12. } else { glam::DVec2::ZERO },
                            center - u * (branch * 20.)
                                + u.perp()
                                    * (6.
                                        * if parabola {
                                            -direction * 0.8
                                        } else {
                                            (-direction * 0.8_f64).sinh()
                                        }),
                        ];
                        let (mut s, id, _) = session_on_a_sketch();
                        let sources = s.edit_sketch(|sketch| {
                            positions.map(|p| {
                                sketch.add_geometry(GeometryElement::Point(Point::new(
                                    Vec2D::from_glam(p),
                                )))
                            })
                        });
                        let before = s.doc.clone();
                        let mut journal = core_document::history::OpJournal::new(8);
                        journal.note(&mut s.doc);
                        s.journal = Some(journal);
                        for p in &positions[..3] {
                            s.full_click(p.x, p.y, tool);
                        }
                        s.mouse_move(positions[3].x, positions[3].y, tool);
                        let preview: Vec<_> = s
                            .overlays()
                            .into_iter()
                            .filter(|l| super::same_color(l.color, super::pal().preview))
                            .collect();
                        assert_eq!(preview.len(), 48);
                        for (i, line) in preview.iter().enumerate() {
                            let want = at(direction * (0.6 - 1.4 * i as f64 / 48.));
                            let (x, y) = s.px_of(want.x, want.y);
                            assert!((line.start[0] - x).hypot(line.start[1] - y) < 0.001);
                        }
                        s.full_click(positions[3].x, positions[3].y, tool);
                        let live = s.sketch();
                        let c = live
                            .geometry
                            .iter()
                            .find_map(|g| {
                                if let GeometryElement::Conic(c) = g {
                                    Some(c)
                                } else {
                                    None
                                }
                            })
                            .unwrap();
                        assert_eq!(c.center, sources[0]);
                        if parabola {
                            assert!(!sources.contains(&c.start));
                        } else {
                            assert_eq!(c.start, sources[2]);
                        }
                        assert!(
                            (live.point_position(c.start).unwrap().to_glam() - at(direction * 0.6))
                                .length()
                                < 1e-8
                        );
                        assert!(!sources.contains(&c.end));
                        assert!(
                            (c.axis.to_glam() - u * if parabola { 4. } else { branch * 8. })
                                .length()
                                < 1e-8,
                            "{deg}/{epsilon}/{branch}/{direction}: {c:?}, {:?}",
                            live.constraints
                        );
                        assert!((c.minor - if parabola { 0. } else { 6. }).abs() < 1e-8);
                        assert!(
                            (live.point_position(c.end).unwrap().to_glam() - at(-direction * 0.8))
                                .length()
                                < 1e-8
                        );
                        assert_eq!(
                            live.constraints.iter().any(|r| matches!(r.kind, C::InternalAlignment { element, curve, role: InternalRole::Focus1 } if element == sources[1] && curve == c.id)),
                            parabola,
                            "a branch vertex is not a focus binding"
                        );
                        assert_eq!(
                            live.constraints.len(),
                            usize::from(parabola),
                            "only a selected focus has an internal alignment"
                        );
                        assert_eq!(
                            s.recorded.last().unwrap().args.get("shared_conic"),
                            Some(&serde_json::json!(true))
                        );
                        assert_replays(&s.recorded, before, id, &live);
                        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                        assert_eq!(s.sketch().geometry.len(), 4);
                        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                        s.doc = Document::load_from_bytes(
                            s.doc
                                .save_to_bytes(core_document::Compression::None)
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(
                            serde_json::to_value(s.sketch()).unwrap(),
                            serde_json::to_value(live).unwrap()
                        );
                        s.mouse_move(positions[0].x, positions[0].y, tool);
                    }
                }
            }
        }
        for deg in [-180_f64, 180., 270., 360., 450., 720.] {
            let (mut s, id, before) = session_on_a_sketch();
            s.mouse_move(21., 21., tool);
            s.keys("20\t20\n", Some(tool));
            s.full_click(20., 20., tool);
            s.mouse_move(35., 25., tool);
            s.type_text(if parabola { "5+5" } else { "3+5" }, Some(tool));
            s.keys("\t", Some(tool));
            s.type_text(&deg.to_string(), Some(tool));
            s.keys("\n", Some(tool));
            let u = glam::DVec2::from_angle(deg.to_radians());
            let center = glam::DVec2::splat(20.);
            let at = |t: f64| {
                if parabola {
                    center + u * ((6. * t).powi(2) / 40.) + u.perp() * (6. * t)
                } else {
                    center + u * (8. * t.cosh()) + u.perp() * (6. * t.sinh())
                }
            };
            for p in [at(0.6), at(-0.8)] {
                s.mouse_move(p.x, p.y, tool);
                let d = p - center;
                s.type_text(&format!("{}", d.length()), Some(tool));
                s.keys("\t", Some(tool));
                s.type_text(&format!("{}", d.y.atan2(d.x).to_degrees()), Some(tool));
                s.keys("\n", Some(tool));
            }
            let live = s.sketch();
            let c = live
                .geometry
                .iter()
                .find_map(|g| {
                    if let GeometryElement::Conic(c) = g {
                        Some(c)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("{deg}: {:?}, {:?}", live.geometry, s.labels()));
            assert!((c.axis.to_glam() - u * if parabola { 10. } else { 8. }).length() < 1e-5);
            assert!((c.minor - if parabola { 0. } else { 6. }).abs() < 1e-5);
            assert!((live.point_position(c.start).unwrap().to_glam() - at(0.6)).length() < 1e-5);
            assert!((live.point_position(c.end).unwrap().to_glam() - at(-0.8)).length() < 1e-5);
            assert_replays(&s.recorded, before, id, &live);
            s.full_click(45., 30., tool);
            s.key(KeyCode::Escape, Some(tool));
            assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
        }
        if !parabola {
            let (mut s, _, _) = session_on_a_sketch();
            s.edit_sketch(|sketch| {
                for x in [20., 30., 35.] {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 20.))));
                }
            });
            for (x, y) in [(20., 20.), (30., 20.), (35., 20.), (35., 25.)] {
                s.full_click(x, y, tool);
            }
            assert_eq!(s.sketch().geometry.len(), 3);
            assert!(
                s.labels()
                    .iter()
                    .any(|l| l.text.contains("Zero minor radius"))
            );
            s.key(KeyCode::Escape, Some(tool));
            s.edit_sketch(|sketch| {
                for (x, y) in [(31., 24.5), (15., 24.5)] {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
                }
            });
            for (x, y) in [
                (20., 20.),
                (30., 20.),
                (31., 24.5),
                (31., 24.5),
                (15., 24.5),
            ] {
                s.full_click(x, y, tool);
            }
            assert_eq!(s.sketch().geometry.len(), 5, "empty span keeps the picks");
            s.full_click(15., 15., tool);
            assert!(
                s.sketch()
                    .geometry
                    .iter()
                    .any(|g| matches!(g, GeometryElement::Conic(_)))
            );
        } else {
            let (mut s, _, _) = session_on_a_sketch();
            s.edit_sketch(|sketch| {
                for (x, y) in [(20., 20.), (24., 20.), (12., 28.), (5., 28.), (5., 8.)] {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
                }
            });
            for (x, y) in [
                (20., 20.),
                (20., 20.),
                (24., 20.),
                (12., 28.),
                (12., 28.),
                (5., 28.),
            ] {
                s.full_click(x, y, tool);
            }
            assert_eq!(
                s.sketch().geometry.len(),
                5,
                "projected empty span keeps the picks"
            );
            assert!(s.labels().iter().any(|l| l.text.contains("Empty arc")));
            s.full_click(5., 8., tool);
            assert!(
                s.sketch()
                    .geometry
                    .iter()
                    .any(|g| matches!(g, GeometryElement::Conic(_)))
            );
            let accepted = serde_json::to_value(s.sketch()).unwrap();
            s.full_click(40., 40., tool);
            s.key(KeyCode::Escape, Some(tool));
            assert_eq!(serde_json::to_value(s.sketch()).unwrap(), accepted);
        }
    }
}

#[test]
fn ellipse_arc_preview_vertices_projection_numbers_and_history() {
    use wb_sketch::sketch::{ConstraintKind as C, Point, Vec2D};
    let tool = "sketch.ellipse_arc";
    for deg in [-180_f64, 180., 270., 360., 450., 720.] {
        for epsilon in [-0.001, 0., 0.001] {
            for input_sweep in [
                -180_f64, -90., 90., 180., 270., 359.999, 360., 360.001, 450., 720.,
            ] {
                let sweep = match input_sweep.rem_euclid(360.) {
                    0. => 360.,
                    s => s,
                };
                let center = glam::DVec2::new(20., 20.);
                let u = glam::DVec2::from_angle((deg + epsilon).to_radians());
                let t0 = 0.8_f64.atan2(0.6);
                let t1 = t0 + input_sweep.to_radians();
                let rim = |t: f64| center + u * (10. * t.cos()) + u.perp() * (5. * t.sin());
                let positions = [
                    center,
                    center + u * 10.,
                    rim(t0),
                    center + (rim(t1) - center) * 2.,
                ];
                let (mut s, id, _) = session_on_a_sketch();
                let sources = s.edit_sketch(|sketch| {
                    positions.map(|p| {
                        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(p))))
                    })
                });
                let before = s.doc.clone();
                let mut journal = core_document::history::OpJournal::new(8);
                journal.note(&mut s.doc);
                s.journal = Some(journal);
                for p in &positions[..3] {
                    s.full_click(p.x, p.y, tool);
                }
                s.mouse_move(positions[3].x, positions[3].y, tool);
                let preview: Vec<_> = s
                    .overlays()
                    .into_iter()
                    .filter(|l| super::same_color(l.color, super::pal().preview))
                    .collect();
                assert_eq!(preview.len(), 48);
                for (i, line) in preview.iter().enumerate() {
                    for (j, [x, y]) in [line.start, line.end].into_iter().enumerate() {
                        let expected = rim(t0 + sweep.to_radians() * (i + j) as f64 / 48.);
                        let (ex, ey) = s.px_of(expected.x, expected.y);
                        // Screen coordinates have f32 precision; compare in pixels.
                        assert!(
                            (x - ex).hypot(y - ey) < 0.001,
                            "{deg}/{epsilon}/{sweep}: {x},{y} != {ex},{ey}"
                        );
                    }
                }
                s.full_click(positions[3].x, positions[3].y, tool);
                let live = s.sketch();
                let e = crate::ellipse_of(&live);
                let arc = e.arc.unwrap();
                assert_eq!((e.center, arc.start), (sources[0], sources[2]));
                assert!(
                    !sources.contains(&arc.end),
                    "projected end gets no cursor identity"
                );
                assert_eq!(live.geometry.len(), 6);
                assert_eq!(
                    live.constraints.len(),
                    2,
                    "only the ellipse's endpoint memberships"
                );
                assert!(
                    (live.point_position(arc.end).unwrap().to_glam() - rim(t1)).length() < 1e-8
                );
                assert!((e.major.to_glam().length() - 10.).abs() < 1e-8);
                assert!((e.ratio - 0.5).abs() < 1e-8);
                let (a, b) = e.param_span(&live).unwrap();
                assert!((b - a - sweep.to_radians()).abs() < 1e-8);
                assert_eq!(
                    s.recorded.last().unwrap().args.get("shared_conic"),
                    Some(&serde_json::json!(true))
                );
                assert_replays(&s.recorded, before, id, &live);
                s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                assert_eq!(s.sketch().geometry.len(), 4);
                s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                s.doc = Document::load_from_bytes(
                    s.doc
                        .save_to_bytes(core_document::Compression::None)
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    serde_json::to_value(s.sketch()).unwrap(),
                    serde_json::to_value(live).unwrap()
                );
                s.mouse_move(positions[0].x, positions[0].y, tool);
            }
        }
    }
    for deg in [-180_f64, 180., 270., 360., 450., 720.] {
        let (mut s, id, before) = session_on_a_sketch();
        s.full_click(20., 20., tool);
        s.full_click(20., 20., tool);
        s.mouse_move(40., 20., tool);
        s.type_text("5+5", Some(tool));
        s.keys("\n", Some(tool));
        s.mouse_move(26., 24., tool);
        s.type_text("2+3", Some(tool));
        s.keys("\n", Some(tool));
        s.mouse_move(5., 25., tool);
        s.type_text(&deg.to_string(), Some(tool));
        s.keys("\n", Some(tool));
        if matches!(deg, 360. | 720.) {
            assert!(
                s.sketch().geometry.is_empty(),
                "return to a collected pick retains the gesture"
            );
            s.type_text("450", Some(tool));
            s.keys("\n", Some(tool));
        }
        let live = s.sketch();
        let e = crate::ellipse_of(&live);
        assert!((e.major.to_glam().length() - 10.).abs() < 1e-6);
        assert!((e.ratio - 0.5).abs() < 1e-6);
        assert!(
            live.constraints
                .iter()
                .any(|c| matches!(c.kind, C::PointOnEllipse { .. }))
        );
        assert_replays(&s.recorded, before, id, &live);
        s.full_click(40., 20., tool);
        s.key(KeyCode::Escape, Some(tool));
        assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
    }
    let (mut s, _, _) = session_on_a_sketch();
    s.edit_sketch(|sketch| {
        for (x, y) in [(20., 20.), (30., 20.), (25., 20.)] {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
        }
    });
    for (x, y) in [(20., 20.), (30., 20.), (25., 20.), (15., 25.)] {
        s.full_click(x, y, tool);
    }
    assert_eq!(s.sketch().geometry.len(), 3);
    assert!(
        s.labels()
            .iter()
            .any(|l| l.text.contains("Zero minor radius")),
        "{:?}",
        s.labels()
    );
    s.key(KeyCode::Escape, Some(tool));
    assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
    let (mut s, _, _) = session_on_a_sketch();
    for (x, y) in [(20., 20.), (30., 20.), (26., 24.), (26., 24.)] {
        s.full_click(x, y, tool);
    }
    assert!(s.sketch().geometry.is_empty());
    s.full_click(0., 20., tool);
    assert_eq!(s.sketch().geometry.len(), 4);
}

#[test]
fn center_ellipse_axes_preview_bindings_numbers_and_history() {
    use wb_sketch::sketch::{Point, Vec2D};
    for tool in ["sketch.ellipse", "sketch.ellipse3"] {
        let three = tool == "sketch.ellipse3";
        for deg in [-180_f64, 180., 270., 360., 450., 720.] {
            for epsilon in [-0.001, 0., 0.001] {
                for b in [5., 10., 15.] {
                    for side in [-1., 1.] {
                        let center = glam::DVec2::new(30., 20.);
                        let u = glam::DVec2::from_angle((deg + epsilon).to_radians());
                        let positions = [
                            if three { center - u * 10. } else { center },
                            center + u * 10.,
                            center + u * 6. + u.perp() * (side * b * if three { 0.8 } else { 1. }),
                        ];
                        let (mut s, id, _) = session_on_a_sketch();
                        let anchor = s.edit_sketch(|sketch| {
                            let points = positions.map(|p| {
                                sketch.add_geometry(GeometryElement::Point(Point::new(
                                    Vec2D::from_glam(p),
                                )))
                            });
                            points[0]
                        });
                        let before = s.doc.clone();
                        let mut journal = core_document::history::OpJournal::new(8);
                        journal.note(&mut s.doc);
                        s.journal = Some(journal);
                        for p in &positions[..2] {
                            s.full_click(p.x, p.y, tool);
                        }
                        s.mouse_move(positions[2].x, positions[2].y, tool);
                        let preview: Vec<_> = s
                            .overlays()
                            .into_iter()
                            .filter(|l| super::same_color(l.color, super::pal().preview))
                            .collect();
                        assert_eq!(preview.len(), 48);
                        let (cx, cy) = s.px_of(center.x, center.y);
                        let (rx, _) = s.px_of(center.x + 1., center.y);
                        for line in preview {
                            for [x, y] in [line.start, line.end] {
                                let d = glam::DVec2::new(
                                    ((x - cx) / (rx - cx)) as f64,
                                    -((y - cy) / (rx - cx)) as f64,
                                );
                                assert!(
                                    ((d.dot(u) / 10.).powi(2) + (d.dot(u.perp()) / b).powi(2) - 1.)
                                        .abs()
                                        < 1e-5
                                );
                            }
                        }
                        s.full_click(positions[2].x, positions[2].y, tool);
                        let live = s.sketch();
                        let check = |sketch: &Sketch| {
                            assert_eq!(
                                sketch.geometry.len(),
                                if three { 5 } else { 4 },
                                "only the centre is a model vertex"
                            );
                            assert!(
                                sketch.constraints.is_empty(),
                                "axis and cursor sources remain unbound"
                            );
                            match sketch.geometry.last().unwrap() {
                                GeometryElement::Circle(c) => {
                                    assert_eq!(b, 10.);
                                    assert_eq!(c.center == anchor, !three);
                                    assert!((c.radius - 10.).abs() < 1e-8);
                                }
                                GeometryElement::Ellipse(e) => {
                                    assert_ne!(b, 10.);
                                    assert_eq!(e.center == anchor, !three);
                                    assert!(e.arc.is_none());
                                    let v = e.major.to_glam();
                                    assert!((v.length() - b.max(10.)).abs() < 1e-8);
                                    assert!((v.length() * e.ratio - b.min(10.)).abs() < 1e-8);
                                    assert!(
                                        v.normalize().dot(if b > 10. { u.perp() } else { u })
                                            > 1. - 1e-8
                                    );
                                }
                                _ => panic!("ellipse or equal-axis circle"),
                            }
                        };
                        check(&live);
                        assert_eq!(
                            s.recorded.last().unwrap().args.get("shared_conic"),
                            Some(&serde_json::json!(true))
                        );
                        assert_replays(&s.recorded, before.clone(), id, &live);
                        let (doc, _) = replay(&s.recorded, before, None);
                        let replayed = SketchFeature::from_json(doc.get_feature_data(id).unwrap())
                            .unwrap()
                            .sketch;
                        // Replay allocates new UUIDs, while keeping the existing centre.
                        check(&replayed);
                        s.journal.as_mut().unwrap().undo(&mut s.doc).unwrap();
                        assert_eq!(s.sketch().geometry.len(), 3);
                        s.journal.as_mut().unwrap().redo(&mut s.doc).unwrap();
                        s.doc = Document::load_from_bytes(
                            s.doc
                                .save_to_bytes(core_document::Compression::None)
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(
                            serde_json::to_value(s.sketch()).unwrap(),
                            serde_json::to_value(live).unwrap()
                        );
                    }
                }
            }
        }
        for (side, second) in [(-1., 5_f64), (1., 10.), (-1., 15.)] {
            let (mut s, id, _) = session_on_a_sketch();
            s.edit_sketch(|sketch| {
                for x in [20., 25., 30.] {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 20.))));
                }
            });
            let before = s.doc.clone();
            for (x, y) in [(20., 20.), (20., 20.), (30., 20.), (25., 20.)] {
                s.full_click(x, y, tool);
                assert_eq!(s.sketch().geometry.len(), 3);
            }
            s.mouse_move(25., 20., tool);
            assert!(
                s.labels()
                    .iter()
                    .any(|l| l.text.contains("Zero minor radius")),
                "{:?}",
                s.labels()
            );
            assert_eq!(s.sketch().geometry.len(), 3);
            s.full_click(23., 20. + side * 5., tool);
            assert_eq!(s.sketch().geometry.len(), if three { 5 } else { 4 });
            s.full_click(50., 20., tool);
            s.full_click(60., 20., tool);
            s.key(KeyCode::Escape, Some(tool));
            assert_eq!(s.tool_request.as_deref(), Some("sketch.select"));
            s.mouse_move(50., 50., tool);
            s.keys("50\t50\n", Some(tool));
            s.mouse_move(70., 50., tool);
            s.type_text(if three { "10+10" } else { "5+5" }, Some(tool));
            s.keys("\n", Some(tool));
            s.mouse_move(53., 50. + side * 20., tool);
            s.type_text(&format!("5+{}", second - 5.), Some(tool));
            s.keys("\n", Some(tool));
            let live = s.sketch();
            match live.geometry.last().unwrap() {
                GeometryElement::Ellipse(e) => {
                    assert!((e.major.to_glam().length() - second.max(10.)).abs() < 1e-7);
                    assert!((e.ratio - second.min(10.) / second.max(10.)).abs() < 1e-7);
                }
                GeometryElement::Circle(c) => {
                    assert_eq!(second, 10.);
                    assert!((c.radius - 10.).abs() < 1e-7);
                }
                _ => panic!("typed ellipse or circle"),
            }
            assert_replays(&s.recorded, before, id, &live);
        }
    }
}

#[test]
fn center_arc_numeric_radius_sweep_rejection_and_retry_replay() {
    use wb_sketch::sketch::ConstraintKind as C;
    let tool = "sketch.arc";
    for angle in [-180_f64, 180., 270., 360., 450., 720.] {
        for epsilon in [-0.001, 0., 0.001] {
            let (mut s, id, before) = session_on_a_sketch();
            s.full_click(20., 20., tool);
            s.full_click(20., 20., tool); // zero radius does not collect a start
            s.mouse_move(35., 20., tool);
            s.type_text("5+10", Some(tool));
            s.keys("\n", Some(tool));
            s.mouse_move(20., 20., tool);
            s.full_click(20., 20., tool); // cursor at center leaves the gesture alive
            assert!(s.sketch().geometry.is_empty());
            s.mouse_move(20., 35., tool);
            s.type_text(&format!("{}", angle + epsilon), Some(tool));
            s.keys("\n", Some(tool));
            let mut sweep = (angle + epsilon).rem_euclid(360.);
            if (angle == 360. || angle == 720.) && epsilon == 0. {
                assert!(
                    s.sketch().geometry.is_empty(),
                    "a numerically coincident start is refused"
                );
                s.type_text("450", Some(tool));
                s.keys("\n", Some(tool));
                sweep = 90.;
            }
            let live = s.sketch();
            let arc = live
                .geometry
                .iter()
                .find_map(|g| match g {
                    GeometryElement::Arc(a) => Some(a),
                    _ => None,
                })
                .unwrap();
            assert!((arc.radius - 15.).abs() < 1e-8);
            assert!(
                live.constraints
                    .iter()
                    .any(|c| matches!(c.kind, C::Radius { radius: 15., .. }))
            );
            assert!(live.constraints.iter().any(|c| matches!(c.kind, C::ArcAngle { angle_rad, .. } if (angle_rad.to_degrees() - sweep).abs() < 1e-7)));
            let end = live.point_position(arc.end).unwrap().to_glam();
            let center = live.point_position(arc.center).unwrap().to_glam();
            let start = live.point_position(arc.start).unwrap().to_glam() - center;
            let want =
                center + 15. * glam::DVec2::from_angle(start.y.atan2(start.x) + sweep.to_radians());
            assert!((end - want).length() < 1e-7);
            assert_replays(&s.recorded, before, id, &live);
        }
    }
}

#[test]
fn a_trim_stroke_replays_as_one_call() {
    let (mut s, id, before) = session_on_a_sketch();
    s.full_click(0.0, 0.0, "sketch.line");
    s.full_click(20.0, 0.0, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));
    for x in [5.0, 10.0, 15.0] {
        s.full_click(x, -5.0, "sketch.line");
        s.full_click(x, 5.0, "sketch.line");
        s.key(KeyCode::Escape, Some("sketch.line"));
    }
    s.click(2.0, 3.0, "sketch.trim");
    s.mouse_move(18.0, 3.0, "sketch.trim");
    s.release(18.0, 3.0, "sketch.trim");
    s.key(KeyCode::A, None);
    let draws = s.recorded.iter().filter(|r| r.id == "sketch.draw").count();
    assert_eq!(draws, 5, "four lines and one stroke: {:#?}", s.recorded);
    assert_replays(&s.recorded, before, id, &s.sketch());
}

#[test]
fn a_spline_through_points_replays_too() {
    let (mut s, id, before) = session_on_a_sketch();
    for (x, y) in [(1.0, 1.0), (5.0, 6.0), (10.0, 4.0), (14.0, 8.0)] {
        s.full_click(x, y, "sketch.bspline:through");
    }
    s.key(KeyCode::Enter, Some("sketch.bspline:through"));
    for (x, y) in [(-12.0, -2.0), (-6.0, -8.0), (-2.0, -3.0)] {
        s.full_click(x, y, "sketch.bspline:through_periodic");
    }
    s.key(KeyCode::Enter, Some("sketch.bspline:through_periodic"));
    s.key(KeyCode::A, None);
    let sketch = s.sketch();
    let fitted = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::BSpline(b) if !b.fit_points.is_empty()))
        .count();
    assert_eq!(fitted, 2);
    assert_replays(&s.recorded, before, id, &s.sketch());
}

#[test]
fn joining_a_polyline_leaves_one_spline_and_replays() {
    let (mut s, id, before) = session_on_a_sketch();
    // This join recipe uses exact unsnapped coordinates, including (1,1)
    // within the origin's object-snap reach.
    s.activate("sketch.snap");
    s.full_click(1.0, 1.0, "sketch.polyline");
    s.full_click(11.0, 1.0, "sketch.polyline");
    s.action("sketch.polyline_arc", "sketch.polyline");
    s.full_click(16.0, 6.0, "sketch.polyline");
    // The switch cycles square and reversed arcs before lines again.
    for _ in 0..3 {
        s.action("sketch.polyline_arc", "sketch.polyline");
    }
    s.full_click(16.0, 16.0, "sketch.polyline");
    s.key(KeyCode::Escape, Some("sketch.polyline"));
    s.key(KeyCode::Escape, Some("sketch.select"));
    let arc_middle = (
        11.0 + 5.0 * std::f64::consts::FRAC_1_SQRT_2,
        6.0 - 5.0 * std::f64::consts::FRAC_1_SQRT_2,
    );
    for (x, y) in [(6.0, 1.0), arc_middle, (16.0, 11.0)] {
        s.full_click(x, y, "sketch.select");
    }
    s.key(KeyCode::A, Some("sketch.join"));
    s.key(KeyCode::A, None);

    let sketch = s.sketch();
    let done = summary(&sketch);
    assert_eq!(done.1, ["bspline"], "{done:?}");
    assert!(s.recorded.iter().any(|r| r.id == "sketch.join"));
    assert_replays(&s.recorded, before, id, &s.sketch());
}

#[test]
fn conic_arcs_replay_too() {
    let (mut s, id, before) = session_on_a_sketch();
    for (x, y) in [(21.0, 21.0), (21.0, 23.0), (17.0, 22.0), (25.0, 27.0)] {
        s.full_click(x, y, "sketch.ellipse:parabola");
    }
    for (x, y) in [(10.0, 1.0), (13.0, 1.0), (15.0, 3.0), (18.0, -1.0)] {
        s.full_click(x, y, "sketch.ellipse:hyperbola");
    }
    s.full_click(23.0, 21.5, "sketch.split");
    s.key(KeyCode::A, None);
    let done = summary(&s.sketch());
    assert_eq!(done.1, ["conic", "conic", "conic"], "{done:?}");
    assert_replays(&s.recorded, before, id, &s.sketch());
}

#[test]
fn editing_aids_replay_too() {
    let (mut s, id, before) = session_on_a_sketch();
    s.full_click(0.0, 0.0, "sketch.ellipse");
    s.full_click(10.0, 0.0, "sketch.ellipse");
    s.full_click(5.0, 4.0, "sketch.ellipse");
    s.full_click(0.0, 4.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.internal_geometry"));
    s.key(KeyCode::Escape, Some("sketch.select"));
    // A rectangle let go of the axes.
    s.full_click(20.0, 1.0, "sketch.rect");
    s.full_click(30.0, 6.0, "sketch.rect");
    s.key(KeyCode::Enter, Some("sketch.rect"));
    // Start outside the shared point/axis reach so this is a box gesture.
    s.click(18.0, -3.0, "sketch.select");
    s.mouse_move(32.0, 8.0, "sketch.select");
    s.release(32.0, 8.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.remove_axis_alignment"));
    s.key(KeyCode::Escape, Some("sketch.select"));
    s.key(KeyCode::A, Some("sketch.section_view"));
    let kinds: Vec<&str> = s.recorded.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "sketch.draw",
            "sketch.internal_geometry",
            "sketch.draw",
            "sketch.remove_axis_alignment",
            "sketch.section_view"
        ],
        "{:#?}",
        s.recorded
    );
    assert_replays(&s.recorded, before, id, &s.sketch());
}

/// Constraints taken to edges and a vertex of the model replay: each new
/// projection as `sketch.external` before the constraint naming what it
/// made, and a vertex of an edge already in the sketch named as it is.
#[test]
fn constraints_on_model_edges_replay_with_their_projections() {
    use crate::block_kernel::{BLOCK, DEPTH, add_block, edge_at};
    let (mut s, id, _) = session_on_a_sketch();
    let body = add_block(&mut s.doc);
    s.kernel = Some(&BLOCK);
    let before = s.doc.clone();
    s.full_click(5.0, 5.0, "sketch.point");
    s.full_click(14.0, 13.0, "sketch.point");
    s.key(KeyCode::Escape, Some("sketch.point"));

    // The first point's distance to the block's left edge, then the
    // second point onto the corner that edge ends at.
    s.full_click(5.0, 5.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.constrain.distance"));
    s.pick_edge(edge_at(body, [20.0, 5.0, DEPTH], [0.0, 1.0, 0.0]));
    s.key(KeyCode::Enter, Some("sketch.select"));
    s.full_click(14.0, 13.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.constrain.coincident"));
    s.pick_edge(edge_at(body, [20.0, 9.9, DEPTH], [0.0, 1.0, 0.0]));

    let sketch = s.sketch();
    assert_eq!(sketch.external.len(), 1, "{:?}", sketch.external);
    assert_eq!(sketch.constraints.len(), 2, "{:?}", sketch.constraints);
    let kinds: Vec<&str> = s
        .recorded
        .iter()
        .map(|r| r.id.as_str())
        .filter(|id| *id != "sketch.draw")
        .collect();
    assert_eq!(
        kinds,
        ["sketch.external", "sketch.constrain", "sketch.constrain"],
        "{:#?}",
        s.recorded
    );
    assert_replays(&s.recorded, before, id, &sketch);
}
