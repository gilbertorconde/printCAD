//! Constraint tools on a picked selection: the rules that read the order
//! of the picks take it as picked, whatever ids the elements have, and a
//! relation the sketch holds already is not added again.

use super::Harness;
use core_document::{KeyCode, Workbench};
use sketch_model::sketch::{ConstraintKind, GeometryElement, Point, Vec2D};
use uuid::Uuid;

/// Points at `at` with the ids `ids`, picked in that order, then `tool`.
fn pick_points_then(h: &mut Harness, at: &[(f64, f64)], ids: &[u128], tool: &str) {
    h.edit_sketch(|s| {
        for (&(x, y), &id) in at.iter().zip(ids) {
            s.add_geometry(GeometryElement::Point(Point {
                id: Uuid::from_u128(id),
                position: Vec2D::new(x, y),
            }));
        }
    });
    h.tap(at[0].0, at[0].1);
    for &(x, y) in &at[1..] {
        h.click_ctrl(x, y, "sketch.select");
        h.release_ctrl(x, y, "sketch.select");
    }
    assert!(h.tool_enabled(tool), "{tool}");
    h.key(KeyCode::A, Some(tool));
}

/// Symmetric on three points makes the last picked the centre, and the
/// angle by three points the middle one its corner, whichever way the ids
/// sort.
#[test]
fn three_point_rules_read_the_picks_in_the_order_made() {
    for ids in [[3, 2, 1], [1, 2, 3], [2, 3, 1], [1, 3, 2]] {
        let mut h = Harness::sketching();
        let at = [(-10.0, 3.0), (10.0, 3.0), (0.0, 3.0)];
        pick_points_then(&mut h, &at, &ids, "sketch.constrain.symmetric");
        let center = h.sketch().constraints.iter().find_map(|c| match c.kind {
            ConstraintKind::SymmetricAboutPoint { center, .. } => Some(center),
            _ => None,
        });
        assert_eq!(center, Some(Uuid::from_u128(ids[2])), "ids {ids:?}");

        let mut h = Harness::sketching();
        let at = [(12.0, 2.0), (2.0, 2.0), (2.0, 12.0)];
        pick_points_then(&mut h, &at, &ids, "sketch.constrain.angle_three_points");
        let corner = h.sketch().constraints.iter().find_map(|c| match c.kind {
            ConstraintKind::AngleThreePoints {
                vertex, angle_rad, ..
            } => Some((vertex, angle_rad.to_degrees())),
            _ => None,
        });
        let (vertex, degrees) = corner.expect("an angle by three points");
        assert_eq!(vertex, Uuid::from_u128(ids[1]), "ids {ids:?}");
        assert!((degrees - 90.0).abs() < 1e-6, "{degrees}");
    }
}

/// Horizontal on a line held horizontal already adds nothing, from the
/// toolbar or the command, and the command says why.
#[test]
fn a_relation_held_already_is_not_added_again() {
    let mut h = Harness::sketching();
    h.line((2.0, 3.0), (12.0, 5.0));
    let horizontals = |h: &Harness| {
        let sketch = h.sketch();
        let level = |c: &&sketch_model::sketch::Constraint| {
            matches!(c.kind, ConstraintKind::Horizontal { .. })
        };
        sketch.constraints.iter().filter(level).count()
    };
    for _ in 0..2 {
        h.key(KeyCode::Escape, Some("sketch.select"));
        h.tap(7.0, 4.0);
        h.key(KeyCode::A, Some("sketch.constrain.horizontal"));
    }
    assert_eq!(horizontals(&h), 1);
    assert_eq!(h.sketch().constraints.len(), 1);

    let sketch = h.active_object.expect("a sketch").0.to_string();
    let line = crate::line_of(&h.sketch()).id.to_string();
    let args = serde_json::json!({"sketch": sketch, "kind": "horizontal", "items": [line]});
    let serde_json::Value::Object(args) = args else {
        unreachable!()
    };
    let refused = h.with_ctx(|wb, ctx| wb.run_command("sketch.constrain", &args, ctx));
    assert!(refused.unwrap_err().to_string().contains("already holds"));
    assert_eq!(horizontals(&h), 1);
}

/// Two picked elements, then `tool`.
fn pick_two_then(h: &mut Harness, a: (f64, f64), b: (f64, f64), tool: &str) {
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.tap(a.0, a.1);
    h.click_ctrl(b.0, b.1, "sketch.select");
    h.release_ctrl(b.0, b.1, "sketch.select");
    assert!(h.tool_enabled(tool), "{tool}");
    h.key(KeyCode::A, Some(tool));
}

/// Coincident on circles that share a centre already says so, from the
/// toolbar and the command, rather than that the selection does not fit.
#[test]
fn coincident_on_concentric_circles_says_they_share_a_centre() {
    let mut h = Harness::sketching();
    h.click(5.0, 5.0, "sketch.circle");
    h.click(7.0, 5.0, "sketch.circle");
    h.click(5.0, 5.0, "sketch.circle");
    h.click(8.0, 5.0, "sketch.circle");
    let circles = crate::all_circles(&h.sketch());
    assert_eq!(circles[0].center, circles[1].center, "the centre is shared");
    let before = h.sketch().constraints.len();
    pick_two_then(
        &mut h,
        (7.0, 5.0),
        (8.0, 5.0),
        "sketch.constrain.coincident",
    );
    assert_eq!(h.sketch().constraints.len(), before, "nothing added");
    let said = h.logs.last().cloned().unwrap_or_default();
    assert!(said.contains("share a centre"), "{:?}", h.logs);

    let args = serde_json::json!({
        "sketch": h.active_object.expect("a sketch").0.to_string(),
        "kind": "coincident",
        "items": [circles[0].id.to_string(), circles[1].id.to_string()],
    });
    let serde_json::Value::Object(args) = args else {
        unreachable!()
    };
    let refused = h.with_ctx(|wb, ctx| wb.run_command("sketch.constrain", &args, ctx));
    let refused = refused.unwrap_err().to_string();
    assert!(refused.contains("share a centre"), "{refused}");
}

/// The far piece of a trimmed line stays on its line through one
/// Collinear, the freedom the two pieces have left as it was and nothing
/// redundant, level or not.
#[test]
fn the_far_piece_of_a_trimmed_line_is_held_collinear() {
    for level in [false, true] {
        let mut h = Harness::sketching();
        h.line((1.0, 2.0), (21.0, 3.0));
        h.line((6.0, -3.0), (5.0, 7.0));
        h.line((16.0, -3.0), (15.0, 7.0));
        let target = crate::line_of(&h.sketch());
        if level {
            h.edit_sketch(|s| s.add_constraint(ConstraintKind::Horizontal { element: target.id }));
        }
        h.click(10.75, 2.5, "sketch.trim");
        let sketch = h.sketch();
        let count = |name: &str| {
            sketch
                .constraints
                .iter()
                .filter(|c| format!("{:?}", c.kind).starts_with(name))
                .count()
        };
        assert_eq!(count("Collinear"), 1, "level {level}");
        assert_eq!(count("PointOnLine"), 0, "level {level}");
        assert_eq!(count("Horizontal"), usize::from(level), "level {level}");
        let args =
            serde_json::json!({ "sketch": h.active_object.expect("a sketch").0.to_string() });
        let serde_json::Value::Object(args) = args else {
            unreachable!()
        };
        let status = h.with_ctx(|wb, ctx| wb.run_command("sketch.status", &args, ctx));
        let status = status.unwrap();
        assert_eq!(status["redundant"], serde_json::json!([]), "{status}");
        // As free as the far piece held parallel (or level) with its start
        // on the first's line: the cutters' 8 less the 2 their drawing held,
        // the first piece's 4 (3 level) and the far one's ends sliding
        // along it.
        let free = if level { 11 } else { 12 };
        assert_eq!(status["dof"], free, "{status}");
    }
}

/// Collinear on two separate lines puts both on one line, and the same
/// pair either way round is not held twice.
#[test]
fn collinear_puts_two_lines_on_one_line() {
    let mut h = Harness::sketching();
    h.line((2.0, 3.0), (12.0, 4.0));
    h.line((20.0, 8.0), (30.0, 10.0));
    pick_two_then(
        &mut h,
        (7.0, 3.5),
        (25.0, 9.0),
        "sketch.constrain.collinear",
    );
    let sketch = h.sketch();
    let lines = crate::all_lines(&sketch);
    let at = |id| sketch.point_position(id).unwrap().to_glam();
    let (s, e) = (at(lines[0].start), at(lines[0].end));
    for p in [lines[1].start, lines[1].end] {
        let off = (e - s).normalize().perp_dot(at(p) - s);
        assert!(off.abs() < 1e-6, "{off} off the first line");
    }
    assert!(crate::holds(&sketch, "Collinear", None));

    let args = serde_json::json!({
        "sketch": h.active_object.expect("a sketch").0.to_string(),
        "kind": "collinear",
        "items": [lines[1].id.to_string(), lines[0].id.to_string()],
    });
    let serde_json::Value::Object(args) = args else {
        unreachable!()
    };
    let refused = h.with_ctx(|wb, ctx| wb.run_command("sketch.constrain", &args, ctx));
    assert!(refused.unwrap_err().to_string().contains("already holds"));
}

/// Coincident on two circles puts their centres on one spot, each keeping
/// its size.
#[test]
fn coincident_on_two_circles_makes_them_concentric() {
    let mut h = Harness::sketching();
    h.click(5.0, 5.0, "sketch.circle");
    h.click(7.0, 5.0, "sketch.circle");
    h.click(14.0, 6.0, "sketch.circle");
    h.click(17.0, 6.0, "sketch.circle");
    pick_two_then(
        &mut h,
        (7.0, 5.0),
        (17.0, 6.0),
        "sketch.constrain.coincident",
    );
    let sketch = h.sketch();
    let circles = crate::all_circles(&sketch);
    let at = |id| sketch.point_position(id).unwrap().to_glam();
    let gap = (at(circles[0].center) - at(circles[1].center)).length();
    assert!(gap < 1e-6, "centres {gap} apart");
    let radii = [circles[0].radius, circles[1].radius];
    assert!(
        (radii[0] - 2.0).abs() < 1e-3 && (radii[1] - 3.0).abs() < 1e-3,
        "{radii:?}"
    );
}
