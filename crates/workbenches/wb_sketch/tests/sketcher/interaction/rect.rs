//! The rectangle tools through the viewport: the preview draws the
//! rectangle the click then lands, a refusal included, and a corner an
//! edit took away ends the gesture with nothing half made.

use super::Harness;
use crate::all_lines;
use wb_sketch::sketch::{GeometryElement, Point, Vec2D};

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
