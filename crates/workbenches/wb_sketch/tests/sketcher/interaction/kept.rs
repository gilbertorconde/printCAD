//! What the view draws from what the bench keeps across frames is what it
//! draws worked out afresh.

use super::Harness;
use core_document::{Workbench, WorkbenchInputEvent};
use wb_sketch::sketch::{ConstraintKind, GeometryElement};

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
