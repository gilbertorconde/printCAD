//! The rectangle tools through the viewport: the preview draws the
//! rectangle the click then lands, a refusal included, and a corner an
//! edit took away ends the gesture with nothing half made.

use super::Harness;
use crate::all_lines;
use core_document::Workbench;
use wb_sketch::sketch::{ConstraintKind, GeometryElement, Point, Vec2D};

fn sketching() -> Harness {
    let mut h = Harness::new();
    h.create_sketch();
    // Unsolved, so nothing moves what the click made after it.
    h.wb.options.auto_update = false;
    h
}

/// The preview's segments as the viewport draws them: ends in pixels, and
/// whether dashed.
fn preview(h: &mut Harness) -> Vec<([f32; 2], [f32; 2], bool)> {
    h.overlays()
        .iter()
        .filter(|o| super::same_color(o.color, super::pal().preview))
        .map(|o| (o.start, o.end, o.dash.is_some()))
        .collect()
}

/// Every variant's last click lands every line end on a vertex of the
/// solid preview drawn just before it.
#[test]
fn the_preview_draws_the_rectangle_the_click_lands() {
    let cases: &[(&str, &[(f64, f64)])] = &[
        ("sketch.rect", &[(0.0, 0.0), (10.0, 6.0)]),
        ("sketch.rect:center", &[(0.0, 0.0), (6.0, 4.0)]),
        ("sketch.rect:rounded", &[(0.0, 0.0), (12.0, 8.0)]),
        ("sketch.rect:frame", &[(0.0, 0.0), (12.0, 8.0)]),
        ("sketch.rect:3pt", &[(0.0, 0.0), (10.0, 3.0), (8.0, 9.0)]),
        (
            "sketch.rect:center3pt",
            &[(0.0, 0.0), (5.0, 1.0), (1.0, 6.0)],
        ),
    ];
    for (tool, clicks) in cases {
        let mut h = sketching();
        let (last, first) = clicks.split_last().unwrap();
        for &(x, y) in first {
            h.full_click(x, y, tool);
        }
        h.mouse_move(last.0, last.1, tool);
        let drawn = preview(&mut h);
        assert!(
            !drawn.is_empty() && drawn.iter().all(|s| !s.2),
            "{tool}: a solid preview"
        );
        h.full_click(last.0, last.1, tool);
        let sketch = h.sketch();
        let lines = all_lines(&sketch);
        assert!(lines.len() >= 4, "{tool}: made");
        for id in lines.iter().flat_map(|l| [l.start, l.end]) {
            let at = sketch.point_position(id).unwrap();
            let px = h.px_of(at.x, at.y);
            let near = |p: [f32; 2]| (p[0] - px.0).abs() < 0.05 && (p[1] - px.1).abs() < 0.05;
            assert!(
                drawn.iter().any(|s| near(s.0) || near(s.1)),
                "{tool}: {at:?} is not on the preview"
            );
        }
    }
}

/// A radius or wall that does not fit draws the outline dashed, and the
/// click then adds nothing; at a size that fits, every corner is rounded,
/// one sitting on a line's end included, which keeps its point.
#[test]
fn a_refused_rectangle_is_refused_by_the_preview_and_the_click() {
    for (tool, radius, wall) in [
        ("sketch.rect:rounded", 5.0, 2.0),
        ("sketch.rect:frame", 2.0, 5.0),
        ("sketch.rect:rounded", f64::NAN, 2.0),
    ] {
        let mut h = sketching();
        h.wb.tool_params_mut().fillet_radius = radius;
        h.wb.tool_params_mut().offset_distance = wall;
        h.full_click(0.0, 0.0, tool);
        h.mouse_move(12.0, 8.0, tool);
        if radius.is_finite() {
            let drawn = preview(&mut h);
            assert!(
                !drawn.is_empty() && drawn.iter().all(|s| s.2),
                "{tool}: dashed"
            );
        }
        h.full_click(12.0, 8.0, tool);
        assert_eq!(h.counts(), (0, 0, 0, 0), "{tool}: nothing made");
    }

    let mut h = sketching();
    h.line((-10.0, 0.0), (0.0, 0.0));
    h.wb.tool_params_mut().fillet_radius = 2.0;
    h.full_click(0.0, 0.0, "sketch.rect:rounded");
    h.full_click(12.0, 8.0, "sketch.rect:rounded");
    let sketch = h.sketch();
    assert_eq!(crate::all_arcs(&sketch).len(), 4, "every corner rounded");
    assert!(h.point_at(0.0, 0.0), "the line's end stays");
}

/// A corner picked on a point an edit then takes away: the next click
/// starts afresh, for the rectangles and for the tools on the existing
/// path (polygon, slot, arc slot) alike; nothing half made is left.
#[test]
fn a_corner_taken_away_ends_the_gesture() {
    let tools = [
        "sketch.rect",
        "sketch.rect:3pt",
        "sketch.rect:center3pt",
        "sketch.polygon",
        "sketch.slot",
        "sketch.slot:arc",
    ];
    for tool in tools {
        let mut h = sketching();
        h.edit_sketch(|s| {
            s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 5.0))));
        });
        h.full_click(5.0, 5.0, tool);
        h.edit_sketch(|s| s.geometry.clear());
        for (x, y) in [(15.0, 12.0), (20.0, 4.0), (9.0, 14.0)] {
            h.full_click(x, y, tool);
        }
        let sketch = h.sketch();
        for g in &sketch.geometry {
            let ids = wb_sketch::sketch::Sketch::curve_point_ids(g);
            assert!(
                ids.iter().all(|id| sketch.point_position(*id).is_some()),
                "{tool}: a curve on a point that is gone"
            );
        }
    }
}

/// A typed first corner on an existing point takes that point.
#[test]
fn a_typed_corner_on_a_point_reuses_it() {
    let mut h = sketching();
    h.edit_sketch(|s| {
        s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(4.0, 3.0))));
    });
    h.mouse_move(4.3, 3.2, "sketch.rect");
    h.keys("4\t3\n", Some("sketch.rect"));
    h.full_click(14.0, 9.0, "sketch.rect");
    assert_eq!(h.counts().0, 4, "four corners, the point among them");
}

/// A rectangle's last click opens a card by it, its width taking the
/// keys, nothing stored until OK: a size below zero or a wall with no
/// room says why and holds OK back, Cancel leaves nothing, and OK stores
/// the sizes typed as dimensions in one undo step and one recorded call
/// that replays to the same sketch.
#[test]
fn a_rectangle_waits_in_its_card_until_ok_and_replays() {
    let (mut h, id, before) = super::recording::session_on_a_sketch();
    h.ok_new_dimensions = false;
    h.full_click(0.0, 0.0, "sketch.rect:frame");
    h.full_click(10.0, 6.0, "sketch.rect:frame");
    let card = h.card().expect("the card");
    assert_eq!(
        (card.title.as_str(), card.focus.as_deref(), card.refusal),
        ("Frame", Some("rect.width"), None)
    );
    assert_eq!(h.counts(), (0, 0, 0, 0), "nothing stored before OK");
    h.close_card(false);
    assert_eq!(h.counts(), (0, 0, 0, 0), "Cancel leaves nothing");
    assert!(h.card().is_none());

    h.full_click(0.0, 0.0, "sketch.rect:frame");
    h.full_click(10.0, 6.0, "sketch.rect:frame");
    for (field, refused) in [
        ("rect.width", "-3"),
        ("rect.size", "4"),
        ("rect.height", "x"),
    ] {
        h.card_type(field, refused);
        assert!(h.card().unwrap().refusal.is_some(), "{refused}: says why");
        h.close_card(true);
        assert_eq!(h.counts(), (0, 0, 0, 0), "{refused}: nothing applied");
        h.card_type(field, if field == "rect.size" { "1" } else { "8" });
    }
    h.card_type("rect.width", "16");
    let closed = h.gestures_closed;
    h.close_card(true);
    assert_eq!(h.gestures_closed, closed + 1, "OK is one undo step");
    let sketch = h.sketch();
    assert_eq!(all_lines(&sketch).len(), 8, "a frame");
    let held = |want: f64| {
        sketch.constraints.iter().any(|c| match c.kind {
            wb_sketch::sketch::ConstraintKind::Length { length, .. } => {
                (length - want).abs() < 1e-9
            }
            _ => false,
        })
    };
    assert!(held(16.0) && held(8.0), "the sizes typed are held");
    super::recording::assert_replays(&h.recorded, before, id, &sketch);
}

/// Auto remove redundants on, a rectangle drawn onto another's wall (its
/// two ends reused, the corners between them on the other's edges) drops
/// the vertical it repeats on its second line between those points: the
/// first rectangle keeps its own, nothing is left redundant, a replay of
/// the recording makes the same sketch and one undo takes the second
/// rectangle back whole. Off, both verticals stay.
#[test]
fn a_rectangle_on_another_s_wall_drops_the_constraint_it_repeats() {
    let status = |h: &mut Harness| {
        let egui_ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&egui_ctx);
        super::task_panel_frame(h, &egui_ctx, Vec::new());
        let ctx = core_document::WorkbenchRuntimeContext::new(
            &mut h.doc,
            crate::CAM_POS,
            [0.0; 3],
            crate::VIEWPORT,
        );
        h.wb.task(&ctx).and_then(|t| t.status).unwrap().text
    };
    let cases: [(&str, [(f64, f64); 4]); 2] = [
        (
            "sketch.rect",
            [(-23.0, -16.0), (23.0, 16.0), (13.0, -16.0), (23.0, 16.0)],
        ),
        (
            "sketch.rect:center",
            [(0.0, 0.0), (23.0, 16.0), (18.0, 0.0), (23.0, 16.0)],
        ),
    ];
    for (tool, clicks) in cases {
        for remove in [true, false] {
            let (mut h, id, before) = super::recording::session_on_a_sketch();
            h.wb.options.auto_remove_redundant = remove;
            for (x, y) in clicks {
                h.full_click(x, y, tool);
            }
            let sketch = h.sketch();
            let first: Vec<_> = all_lines(&sketch)[..4].iter().map(|l| l.id).collect();
            let verticals: Vec<bool> = sketch
                .constraints
                .iter()
                .filter_map(|c| match c.kind {
                    ConstraintKind::Vertical { element } => Some(first.contains(&element)),
                    _ => None,
                })
                .collect();
            if remove {
                assert_eq!(verticals, [true, true, false], "{tool}: the new one goes");
                assert_ne!(status(&mut h), "Redundant constraints", "{tool}");
                super::recording::assert_replays(&h.recorded, before, id, &sketch);
                h.journal.undo(&mut h.doc).unwrap();
                let back = h.sketch();
                assert_eq!(all_lines(&back).len(), 4, "{tool}");
                assert!(back.constraints.iter().any(
                    |c| matches!(c.kind, ConstraintKind::Vertical { element } if element == first[1])
                ));
            } else {
                assert_eq!(verticals.len(), 4, "{tool}: both stay");
                assert_eq!(status(&mut h), "Redundant constraints", "{tool}");
            }
        }
    }
}

/// With Reuse existing lines on, a shape drawn against the first
/// rectangle's right wall uses that line: one line there, its vertical
/// once, one point at its end. Off, the shape adds its own line on top.
/// Either way the recording replays to the same sketch and one undo
/// takes the second shape back whole.
#[test]
fn a_shape_against_an_existing_line_reuses_it_when_asked() {
    let cases: [(&str, [(f64, f64); 2]); 3] = [
        ("sketch.rect", [(13.0, -16.0), (23.0, 16.0)]),
        ("sketch.polygon:4", [(39.0, 0.0), (23.0, 16.0)]),
        ("sketch.slot", [(28.0, -16.0), (28.0, 16.0)]),
    ];
    let near = |p: Vec2D, x: f64, y: f64| (p.x - x).abs() < 1e-6 && (p.y - y).abs() < 1e-6;
    for (tool, clicks) in cases {
        for reuse in [true, false] {
            let (mut h, id, before) = super::recording::session_on_a_sketch();
            h.wb.options.reuse_lines = reuse;
            h.wb.tool_params_mut().slot_width = 10.0;
            // On grid points, so a derived corner lands exactly on a held one.
            let o = &mut h.wb.options;
            (o.grid_on, o.grid_snap, o.grid_auto, o.grid_size) = (true, true, false, 1.0);
            for (x, y) in [(-23.0, -16.0), (23.0, 16.0)] {
                h.full_click(x, y, "sketch.rect");
            }
            let one = h.counts();
            for (x, y) in clicks {
                h.full_click(x, y, tool);
            }
            let sketch = h.sketch();
            let at = |id| sketch.point_position(id).unwrap();
            let walls: Vec<_> = all_lines(&sketch)
                .into_iter()
                .filter(|l| {
                    let (a, b) = (at(l.start), at(l.end));
                    let ends = |a, b| near(a, 23.0, -16.0) && near(b, 23.0, 16.0);
                    ends(a, b) || ends(b, a)
                })
                .collect();
            let corners = sketch
                .geometry
                .iter()
                .filter(|g| matches!(g, GeometryElement::Point(p) if near(p.position, 23.0, 16.0)))
                .count();
            let verticals = sketch
                .constraints
                .iter()
                .filter(|c| matches!(c.kind, ConstraintKind::Vertical { element } if walls.iter().any(|w| w.id == element)))
                .count();
            let n = if reuse { 1 } else { 2 };
            assert_eq!(walls.len(), n, "{tool} reuse {reuse}");
            assert!(
                corners == 1 || !reuse,
                "{tool}: one point at the wall's end"
            );
            if tool == "sketch.rect" {
                assert_eq!(verticals, n, "{tool} reuse {reuse}");
            }
            super::recording::assert_replays(&h.recorded, before, id, &sketch);
            h.journal.undo(&mut h.doc).unwrap();
            assert_eq!(h.counts(), one, "{tool} reuse {reuse}");
            assert!(h.sketch().constraints.iter().any(
                |c| matches!(c.kind, ConstraintKind::Vertical { element } if element == walls[0].id)
            ));
        }
    }
}
