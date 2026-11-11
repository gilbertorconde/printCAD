//! The line and polyline gesture against the host's history: each segment
//! is an undo step the gesture keeps, so Backspace or the host's undo puts
//! the chain back before it, and a tool change, Escape, a tab switch or an
//! outside edit leaves nothing half made.

use super::Harness;
use super::recording::{assert_replays, session_on_a_sketch};
use crate::{all_lines, all_points};
use core_document::{KeyCode, Workbench, WorkbenchInputEvent};
use wb_sketch::sketch::{ConstraintKind, GeometryElement, Point, Vec2D};

const LINE: &str = "sketch.line";
const POLYLINE: &str = "sketch.polyline";

/// A harness editing a fresh sketch whose frames close undo steps.
fn sketching() -> Harness {
    let mut h = Harness::new();
    h.frames_close = true;
    h.create_sketch();
    h
}

fn clicks(h: &mut Harness, tool: &str, points: &[(f64, f64)]) {
    for &(x, y) in points {
        h.full_click(x, y, tool);
    }
}

/// Whether a point of the sketch is within a hundredth of `(x, y)`.
fn has_point(h: &Harness, x: f64, y: f64) -> bool {
    all_points(&h.sketch())
        .iter()
        .any(|p| (p.position.x - x).hypot(p.position.y - y) < 0.01)
}

/// Backspace and the host's undo each take back the newest segment and
/// draw on from its start; redo brings one back; the recorded call keeps
/// only the clicks whose segments stayed, and replays to the same sketch.
#[test]
fn a_segment_taken_back_puts_the_chain_before_it() {
    let (mut h, id, before) = session_on_a_sketch();
    h.frames_close = true;
    clicks(&mut h, LINE, &[(0.0, 0.0), (10.0, 0.0), (10.0, 8.0)]);
    h.key(KeyCode::Backspace, Some(LINE));
    assert_eq!(h.counts(), (2, 1, 0, 0), "the newest segment taken back");
    h.full_click(20.0, 5.0, LINE);
    assert_eq!(h.counts(), (3, 2, 0, 0), "drawn on from (10, 0)");
    h.history(false);
    assert_eq!(h.counts(), (2, 1, 0, 0), "the host's undo does the same");
    h.history(true);
    assert_eq!(h.counts(), (3, 2, 0, 0), "redo brings it back");
    h.history(false);
    h.full_click(10.0, -8.0, LINE);
    assert!(has_point(&h, 10.0, -8.0) && !has_point(&h, 10.0, 8.0));
    let sketch = h.sketch();
    let corner = all_lines(&sketch)
        .iter()
        .map(|l| l.end)
        .find(|id| {
            sketch
                .point_position(*id)
                .is_some_and(|p| p.x > 9.0 && p.y.abs() < 0.01)
        })
        .expect("the first segment's end");
    assert!(
        all_lines(&sketch).iter().any(|l| l.start == corner),
        "the segment after an undo starts on the shared vertex"
    );
    h.key(KeyCode::Escape, Some(LINE));
    h.key(KeyCode::Backspace, Some(LINE));
    assert_eq!(
        h.counts().1,
        2,
        "with the chain ended, Backspace takes nothing"
    );
    assert_replays(&h.recorded, before, id, &h.sketch());
}

/// A tool change, Escape or a tab switch: the first two end the chain and
/// take nothing back, the switch keeps it, Backspace and all.
#[test]
fn a_chain_ends_with_its_tool_and_survives_a_tab_switch() {
    let mut h = sketching();
    h.full_click(0.0, 0.0, LINE);
    h.key(KeyCode::Escape, Some(LINE));
    assert_eq!(h.counts(), (0, 0, 0, 0), "a cancelled start makes nothing");

    clicks(&mut h, LINE, &[(0.0, 0.0), (10.0, 0.0), (10.0, 8.0)]);
    h.event(WorkbenchInputEvent::ToolActivated, Some("sketch.circle"));
    h.key(KeyCode::Backspace, Some("sketch.circle"));
    assert_eq!(
        h.counts().1,
        2,
        "another tool's Backspace leaves the segments"
    );
    h.history(false);
    assert_eq!(h.counts().1, 1, "the host still undoes them");

    clicks(&mut h, LINE, &[(0.0, 20.0), (10.0, 20.0)]);
    let parked = h.wb.suspend_session();
    h.wb.resume_session(parked);
    h.key(KeyCode::Backspace, Some(LINE));
    assert_eq!(
        h.counts().1,
        1,
        "back from another tab, Backspace still takes it back"
    );
    h.full_click(0.0, 30.0, LINE);
    assert!(has_point(&h, 0.0, 30.0), "and the chain draws on");
}

/// An edit from outside the gesture: one that takes segments rewinds the
/// chain to before them, one that takes the point it starts from ends it,
/// and the next click starts afresh rather than failing.
#[test]
fn an_outside_edit_rewinds_or_ends_the_chain() {
    let mut h = sketching();
    clicks(&mut h, LINE, &[(0.0, 0.0), (10.0, 0.0), (10.0, 8.0)]);
    let lines = |s: &mut wb_sketch::sketch::Sketch, keep: usize| {
        let mut seen = 0;
        s.geometry.retain(|g| {
            let line = matches!(g, GeometryElement::Line(_));
            seen += usize::from(line);
            !line || seen <= keep
        });
    };
    h.edit_sketch(|s| lines(s, 1));
    h.full_click(20.0, 0.0, LINE);
    assert!(has_point(&h, 20.0, 0.0));
    assert_eq!(h.counts().1, 2, "drawn on from the segment before it");
    h.edit_sketch(|s| lines(s, 0));
    h.full_click(5.0, 5.0, LINE);
    assert_eq!(
        h.counts().1,
        1,
        "every segment gone: drawn on from the first click"
    );
    h.key(KeyCode::Escape, Some(LINE));

    h.edit_sketch(|s| {
        s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40.0, 0.0))));
    });
    h.full_click(40.0, 0.0, LINE);
    h.edit_sketch(|s| s.geometry.retain(|g| s_point_at(g, 40.0)));
    let before = h.counts();
    h.full_click(50.0, 5.0, LINE);
    assert_eq!(
        h.counts(),
        before,
        "its start gone, the click starts afresh"
    );
    h.full_click(60.0, 5.0, LINE);
    assert_eq!(h.counts().1, before.1 + 1);
}

/// Whether `g` is anything but a point at `(x, 0)`.
fn s_point_at(g: &GeometryElement, x: f64) -> bool {
    !matches!(g, GeometryElement::Point(p) if (p.position.x - x).abs() < 1e-9 && p.position.y == 0.0)
}

/// A polyline back on its first point closes into a profile of shared
/// vertices; a typed length is where the end goes, an existing point
/// exactly there taken rather than doubled.
#[test]
fn closed_chains_profile_and_typed_ends_keep_their_vertex() {
    let mut h = sketching();
    clicks(
        &mut h,
        POLYLINE,
        &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)],
    );
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).expect("a closed profile");
    assert_eq!(wires.len(), 1);
    assert_eq!(h.counts(), (3, 3, 0, 0));

    h.edit_sketch(|s| {
        s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(30.0, 0.0))));
    });
    h.full_click(30.0, 20.0, LINE);
    h.mouse_move(30.0, 15.0, LINE);
    h.keys("20\n", Some(LINE));
    assert_eq!(h.counts().0, 5, "the typed end took the point there");
    h.mouse_move(30.0, -5.0, LINE);
    h.keys("7\n", Some(LINE));
    assert!(
        has_point(&h, 30.0, -7.0),
        "a typed length lands where it says"
    );
}

/// A recording written before the runtime: a line whose level direction
/// the snap carried onto a circle keeps both its end on the circle and
/// its level.
#[test]
fn a_recorded_level_end_on_a_curve_stays_level() {
    let mut h = sketching();
    h.full_click(10.0, 0.0, "sketch.circle");
    h.full_click(16.0, 0.0, "sketch.circle");
    let crossing = 10.0 - 11.0f64.sqrt();
    let args = serde_json::json!({
        "sketch": h.active_object.unwrap().0.to_string(),
        "tool": "line",
        "tolerance": 0.4,
        "points": [[0.0, 5.0], [crossing + 0.05, 5.2]],
    });
    let args = args.as_object().unwrap().clone();
    h.with_ctx(|wb, ctx| {
        wb.run_command("sketch.draw", &args, ctx)
            .expect("the recorded call runs")
    });
    let sketch = h.sketch();
    let held = |want: fn(&ConstraintKind) -> bool| sketch.constraints.iter().any(|c| want(&c.kind));
    assert!(
        held(|k| matches!(k, ConstraintKind::Horizontal { .. })),
        "{:?}",
        sketch.constraints
    );
    assert!(held(|k| matches!(k, ConstraintKind::PointOnCircle { .. })));
}

/// The preview ends where the click then lands: on a curve near the
/// cursor, the same projected point (unsolved, so nothing moves it after).
#[test]
fn the_preview_ends_where_the_click_lands() {
    let mut h = sketching();
    h.wb.options.auto_update = false;
    h.line((5.0, -5.0), (5.0, 5.0));
    h.full_click(0.0, 0.0, LINE);
    h.mouse_move(5.15, 2.0, LINE);
    let preview = h.overlays();
    let preview = preview
        .iter()
        .find(|o| super::same_color(o.color, super::pal().preview))
        .expect("a preview segment");
    h.full_click(5.15, 2.0, LINE);
    let sketch = h.sketch();
    let made = all_lines(&sketch).last().cloned().expect("the new line");
    let end = sketch.point_position(made.end).unwrap();
    assert!((end.x - 5.0).abs() < 1e-3, "on the curve: {end:?}");
    let px = h.px_of(end.x, end.y);
    assert!(
        (px.0 - preview.end[0]).abs() < 0.01 && (px.1 - preview.end[1]).abs() < 0.01,
        "{px:?} vs {:?}",
        preview.end
    );
}
