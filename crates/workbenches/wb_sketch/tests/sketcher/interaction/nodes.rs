//! Splines edited by their nodes, and the path tool that draws them: a
//! double click brings a spline into node form, a node's press shows and
//! carries its handles, a handle's drag holds its node and the other
//! handle in line, a right click's card sets a node's kind, and the path
//! tool's press places a node whose drag pulls its handles. Clicks land
//! through the view's pixels, so where they land is held to 1e-3 mm.

use super::Harness;
use core_document::KeyCode;
use wb_sketch::nodes::{self, NodeKind};
use wb_sketch::sketch::{GeometryElement, Sketch, Vec2D};

fn at(sketch: &Sketch, id: uuid::Uuid) -> Vec2D {
    sketch.point_position(id).expect("a point")
}

fn spline_id(sketch: &Sketch) -> uuid::Uuid {
    sketch
        .geometry
        .iter()
        .find(|g| matches!(g, GeometryElement::BSpline(_)))
        .map(GeometryElement::id)
        .expect("a spline")
}

/// The two handles and the node lie in one line, the node between them.
fn in_line(b: Vec2D, p: Vec2D, a: Vec2D) -> bool {
    let (into, out) = ((p - b).to_glam(), (a - p).to_glam());
    into.perp_dot(out).abs() <= 1e-6 * into.length() * out.length() && into.dot(out) > 0.0
}

/// A spline drawn by its control points, then double-clicked: its nodes.
fn spline_in_node_edit() -> (Harness, uuid::Uuid) {
    let mut h = Harness::sketching();
    for (x, y) in [
        (0.0, 0.0),
        (6.0, 6.0),
        (12.0, -3.0),
        (18.0, 7.0),
        (24.0, 0.0),
        (30.0, 4.0),
    ] {
        h.full_click(x, y, "sketch.bspline");
    }
    h.key(KeyCode::Enter, Some("sketch.bspline"));
    let sketch = h.sketch();
    let id = spline_id(&sketch);
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(id) else {
        unreachable!()
    };
    let on = b.polyline(&sketch, 64).unwrap()[20];
    h.tap(on.x, on.y);
    h.tap(on.x, on.y);
    (h, id)
}

/// A double click on a spline shows its nodes, the curve kept; a node's
/// press picks it and a drag carries its handles with it; a handle's
/// drag holds the node where it is and the smooth node's other handle in
/// line with it.
#[test]
fn a_node_carries_its_handles_and_a_handle_turns_about_its_node() {
    let (mut h, id) = spline_in_node_edit();
    assert_eq!(h.wb.node_edit_state(), Some((id, None)), "under node edit");
    let sketch = h.sketch();
    let path = nodes::nodes_of(&sketch, id).expect("node form");
    let node = path.nodes[1];
    assert_eq!(nodes::kind_of(&sketch, &node), NodeKind::Smooth);
    let (p, b, a) = (
        at(&sketch, node.point),
        at(&sketch, node.before.unwrap()),
        at(&sketch, node.after.unwrap()),
    );

    h.drag((p.x, p.y), (p.x + 2.0, p.y + 3.0));
    assert_eq!(
        h.wb.node_edit_state(),
        Some((id, Some(node.point))),
        "picked"
    );
    let sketch = h.sketch();
    let moved = |id, was: Vec2D| (at(&sketch, id) - was).to_glam();
    for (point, was) in [
        (node.point, p),
        (node.before.unwrap(), b),
        (node.after.unwrap(), a),
    ] {
        let d = moved(point, was);
        assert!(
            (d.x - 2.0).abs() < 1e-3 && (d.y - 3.0).abs() < 1e-3,
            "carried by the step: {d:?}"
        );
    }

    // The picked node shows its handles, a line from it to each; another
    // node's handles draw no dot.
    let px = |h: &mut Harness, p: Vec2D| h.px_of(p.x, p.y);
    let node_px = px(&mut h, at(&sketch, node.point));
    let handle_px: Vec<(f32, f32)> = node
        .handles()
        .map(|id| px(&mut h, at(&sketch, id)))
        .collect();
    let lines = h.overlays();
    let near = |a: [f32; 2], b: (f32, f32)| (a[0] - b.0).hypot(a[1] - b.1) < 0.5;
    for end in &handle_px {
        assert!(
            lines
                .iter()
                .any(|l| (near(l.start, *end) && near(l.end, node_px))
                    || (near(l.end, *end) && near(l.start, node_px))),
            "a line to the handle at {end:?}"
        );
    }
    let other = path.nodes[2].after.unwrap();
    let other_px = px(&mut h, at(&sketch, other));
    assert!(
        !h.marks().iter().any(|m| near(m.pos, other_px)),
        "another node's handle is not shown"
    );

    let (p, a) = (at(&sketch, node.point), at(&sketch, node.after.unwrap()));
    h.drag((a.x, a.y), (a.x + 1.0, a.y + 4.0));
    let sketch = h.sketch();
    assert!(
        (at(&sketch, node.point) - p).to_glam().length() < 1e-3,
        "the node held"
    );
    let after = at(&sketch, node.after.unwrap());
    assert!(
        (after - Vec2D::new(a.x + 1.0, a.y + 4.0))
            .to_glam()
            .length()
            < 1e-3
    );
    let before = at(&sketch, node.before.unwrap());
    assert!(
        in_line(before, at(&sketch, node.point), after),
        "the other handle followed"
    );
}

/// A right click on a node opens its card; making it a corner frees its
/// handles, so dragging one leaves the other where it is. Escape ends the
/// edit.
#[test]
fn a_right_click_breaks_a_node_s_handles_apart() {
    let (mut h, id) = spline_in_node_edit();
    let node = nodes::nodes_of(&h.sketch(), id).unwrap().nodes[2];
    let p = at(&h.sketch(), node.point);
    h.right_click(p.x, p.y, "sketch.select");
    let card = h.card().expect("the node's card");
    assert_eq!((card.title.as_str(), card.ok_only), ("Node", true));
    h.card_event(bench_api::PanelEvent::Choice {
        id: "node.kind".into(),
        index: 0,
    });
    h.close_card(true);
    assert_eq!(nodes::kind_of(&h.sketch(), &node), NodeKind::Corner);

    let sketch = h.sketch();
    let (before, a) = (
        at(&sketch, node.before.unwrap()),
        at(&sketch, node.after.unwrap()),
    );
    h.drag((a.x, a.y), (a.x + 3.0, a.y - 2.0));
    let sketch = h.sketch();
    assert!(
        (at(&sketch, node.before.unwrap()) - before)
            .to_glam()
            .length()
            < 1e-6,
        "left alone"
    );

    h.key(KeyCode::Escape, Some("sketch.select"));
    assert_eq!(h.wb.node_edit_state(), None);
}

/// The path tool: a click places a corner node, a press dragged places a
/// node whose handle the drag pulls out (the other mirrored, the node
/// smooth); Enter finishes the path open. Its recording replays.
#[test]
fn the_path_tool_places_nodes_and_pulls_handles() {
    let (mut h, id, before) = super::recording::session_on_a_sketch();
    h.full_click(0.0, 0.0, "sketch.path");
    h.click(10.0, 0.0, "sketch.path");
    h.mouse_move(14.0, 4.0, "sketch.path");
    h.release(14.0, 4.0, "sketch.path");
    h.full_click(20.0, 0.0, "sketch.path");
    h.key(KeyCode::Enter, Some("sketch.path"));
    let sketch = h.sketch();
    let path = nodes::nodes_of(&sketch, spline_id(&sketch)).expect("a path in node form");
    assert_eq!(path.nodes.len(), 3);
    assert!(!path.closed);
    let mid = path.nodes[1];
    assert_eq!(nodes::kind_of(&sketch, &mid), NodeKind::Smooth);
    let close = |id, x: f64, y: f64| (at(&sketch, id) - Vec2D::new(x, y)).to_glam().length() < 1e-3;
    assert!(close(mid.point, 10.0, 0.0));
    assert!(
        close(mid.after.unwrap(), 14.0, 4.0),
        "the handle where the drag let go"
    );
    assert!(close(mid.before.unwrap(), 6.0, -4.0), "the other mirrored");
    super::recording::assert_replays(&h.recorded, before, id, &sketch);
}

/// A click on the first node closes the path: its first and last control
/// point one point, the loop a closed profile.
#[test]
fn the_path_tool_closes_on_its_first_node() {
    let mut h = Harness::sketching();
    for (x, y) in [(0.0, 10.0), (10.0, 10.0), (10.0, 20.0), (0.0, 10.0)] {
        h.full_click(x, y, "sketch.path");
    }
    let sketch = h.sketch();
    let path = nodes::nodes_of(&sketch, spline_id(&sketch)).expect("a path");
    assert!(path.closed);
    assert_eq!(path.nodes.len(), 3);
    let wires = wb_sketch::profile::extract_wires(&sketch).expect("a profile");
    assert_eq!(wires.len(), 1, "one closed loop");
}
