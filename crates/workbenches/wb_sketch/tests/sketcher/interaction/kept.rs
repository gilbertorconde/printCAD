//! What the view draws from what the bench keeps across frames is what it
//! draws worked out afresh.

use super::Harness;
use core_document::{Workbench, WorkbenchInputEvent};
use sketch_model::sketch::{Circle, ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
use uuid::Uuid;

/// Everything the view draws this frame, as text to compare; `afresh`
/// with what the bench kept from earlier frames forgotten first.
fn drawn(h: &mut Harness, afresh: bool) -> String {
    h.frame();
    h.with_ctx(|wb, ctx| {
        if afresh {
            wb.tab_state();
        }
        let out = wb.viewport(ctx);
        format!(
            "{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
            out.overlays, out.marks, out.labels, out.status, out.hud
        )
    })
}

fn same(h: &mut Harness, step: &str) {
    let kept = drawn(h, false);
    assert_eq!(kept, drawn(h, true), "{step}");
}

/// Through a hover, a selection, a tool's preview, a turn of the view, a
/// label's drag, an edit and its undo, the kept drawing is the fresh one.
#[test]
fn the_view_kept_across_frames_is_the_view_worked_out_afresh() {
    let mut h = Harness::sketching();
    h.frames_close = true;
    h.line((0.0, 0.0), (10.0, 0.0));
    h.line((10.0, 0.0), (10.0, 5.0));
    h.edit_sketch(|s| {
        let line = s
            .geometry
            .iter()
            .find(|g| matches!(g, GeometryElement::Line(_)))
            .map(GeometryElement::id)
            .unwrap();
        s.add_constraint(ConstraintKind::Length { line, length: 10.0 });
    });
    same(&mut h, "the sketch");
    h.mouse_move(5.0, 0.0, "sketch.select");
    same(&mut h, "a hover on a line");
    h.mouse_move(5.0, 3.0, "sketch.select");
    same(&mut h, "a hover off it");
    h.click(5.0, 0.0, "sketch.select");
    h.release(5.0, 0.0, "sketch.select");
    same(&mut h, "a selection");
    h.activate("sketch.line");
    h.mouse_move(3.0, 2.0, "sketch.line");
    same(&mut h, "the line tool's cue");
    h.click(3.0, 2.0, "sketch.line");
    h.mouse_move(7.0, 4.0, "sketch.line");
    same(&mut h, "a line's preview");
    h.key(core_document::KeyCode::Escape, Some("sketch.line"));
    h.activate("sketch.select");
    h.vp[0][0] *= 1.5;
    h.vp[1][1] *= 1.5;
    same(&mut h, "a turn of the view");
    let label = h
        .labels()
        .iter()
        .find(|l| l.background)
        .map(|l| (l.pos[0], l.pos[1]))
        .expect("the dimension's label");
    h.press_px(label);
    let viewport_pos = (label.0 + 20.0, label.1 - 25.0);
    h.event(
        WorkbenchInputEvent::MouseMove { viewport_pos },
        Some("sketch.select"),
    );
    same(&mut h, "a label held away");
    h.release_px(viewport_pos);
    same(&mut h, "a label let go");
    h.edit_sketch(|s| s.constraints.clear());
    same(&mut h, "an edit");
    h.history(false);
    same(&mut h, "its undo");
}

/// Short lines and circles packed close, each with a length or radius and
/// relations between neighbours, so their labels and icons crowd and
/// push one another aside. The lines, then the circles.
fn crowd(s: &mut Sketch) -> (Vec<Uuid>, Vec<Uuid>) {
    let point =
        |s: &mut Sketch, x, y| s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
    let (mut lines, mut circles): (Vec<Uuid>, Vec<Uuid>) = (Vec::new(), Vec::new());
    for k in 0..8 {
        let (x, y) = (f64::from(k % 4) * 1.5, f64::from(k / 4) * 1.2);
        let a = point(s, x, y);
        let b = point(s, x + 1.0, y);
        let line = s.add_geometry(GeometryElement::Line(Line::new(a, b)));
        s.add_constraint(ConstraintKind::Length { line, length: 1.0 });
        s.add_constraint(ConstraintKind::Horizontal { element: line });
        let c = point(s, x + 0.5, y + 3.0);
        let circle = s.add_geometry(GeometryElement::Circle(Circle::new(c, 0.4)));
        s.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 0.4,
        });
        if let (Some(&line1), Some(&circle1)) = (lines.last(), circles.last()) {
            s.add_constraint(ConstraintKind::EqualLength { line1, line2: line });
            s.add_constraint(ConstraintKind::EqualRadius {
                circle1,
                circle2: circle,
            });
        }
        lines.push(line);
        circles.push(circle);
    }
    (lines, circles)
}

/// A crowded sketch's labels, arranged from where the last frame left
/// them, sit where an arrangement from nothing puts them through every
/// kind of change: a relation added (renumbering its kind), one taken out
/// of the middle, a point moved, a value made longer, a label placed by
/// hand, a turn of the view, a selection, and undo and redo.
#[test]
fn crowded_labels_arranged_from_the_last_sit_where_fresh_ones_do() {
    let mut h = Harness::sketching();
    h.frames_close = true;
    let (lines, circles) = h.edit_sketch(crowd);
    same(&mut h, "the crowd");
    h.edit_sketch(|s| {
        s.add_constraint(ConstraintKind::EqualLength {
            line1: lines[0],
            line2: lines[5],
        })
    });
    same(&mut h, "a relation added");
    h.edit_sketch(|s| {
        let equal = |c: &&sketch_model::sketch::Constraint| {
            matches!(c.kind, ConstraintKind::EqualRadius { .. })
        };
        let third = s.constraints.iter().filter(equal).nth(2).unwrap().id;
        s.constraints.retain(|c| c.id != third);
    });
    same(&mut h, "a relation taken out");
    h.edit_sketch(|s| {
        s.constraints
            .retain(|c| c.kind != ConstraintKind::Horizontal { element: lines[0] })
    });
    same(&mut h, "an icon taken out from before the others");
    h.edit_sketch(|s| {
        let end = s.geometry.iter().find_map(|g| match g {
            GeometryElement::Line(l) if l.id == lines[2] => Some(l.end),
            _ => None,
        });
        for g in &mut s.geometry {
            if let GeometryElement::Point(p) = g
                && Some(p.id) == end
            {
                p.position = Vec2D::new(4.2, 0.4);
            }
        }
    });
    same(&mut h, "a point moved");
    h.edit_sketch(|s| {
        for c in &mut s.constraints {
            if let ConstraintKind::Radius { circle, radius } = &mut c.kind
                && *circle == circles[1]
            {
                *radius = 0.123_456;
            }
        }
    });
    same(&mut h, "a value made longer");
    h.edit_sketch(|s| {
        let c = (s.constraints.iter_mut())
            .find(|c| matches!(c.kind, ConstraintKind::Length { .. }))
            .unwrap();
        c.label_offset = Some(Vec2D::new(1.5, 0.6));
    });
    same(&mut h, "a label placed by hand");
    h.vp[0][0] *= 1.3;
    h.vp[1][1] *= 1.3;
    same(&mut h, "a turn of the view");
    h.click(2.0, 0.0, "sketch.select");
    h.release(2.0, 0.0, "sketch.select");
    same(&mut h, "a selection");
    h.history(false);
    same(&mut h, "an undo");
    h.history(false);
    same(&mut h, "another undo");
    h.history(true);
    same(&mut h, "a redo");
}
