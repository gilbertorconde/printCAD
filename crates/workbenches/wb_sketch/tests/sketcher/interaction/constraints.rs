//! Constraint tools on a picked selection: the rules that read the order
//! of the picks take it as picked, whatever ids the elements have, and a
//! relation the sketch holds already is not added again.

use super::Harness;
use core_document::{KeyCode, Workbench};
use uuid::Uuid;
use wb_sketch::sketch::{ConstraintKind, GeometryElement, Point, Vec2D};

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
        let level = |c: &&wb_sketch::sketch::Constraint| {
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
