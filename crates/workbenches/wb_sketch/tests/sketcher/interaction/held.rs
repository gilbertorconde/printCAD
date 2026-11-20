//! Shapes drawn through clicks a snap put on a curve or a point stay
//! through them, and centred on one there: moving what was snapped to
//! carries the shape along. The holds say nothing twice.

use super::Harness;
use sketch_model::shape::distance_to_element;
use sketch_model::sketch::{
    Conic, ConicKind, ConstraintKind, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D,
};

/// Each tool's clicks on a sketch holding a level line along y = 10, an
/// upright one along x = 20 and a point at (0, -6); the clicks on them
/// are held there and the shape through them, until those three move,
/// with nothing redundant or conflicting. An arc's end (and a parabola's
/// start) lands on its curve: on a line, where the curve crosses it.
#[test]
fn a_shape_through_snapped_clicks_follows_what_they_snapped_to() {
    let cases: [(&str, &[(f64, f64)]); 13] = [
        ("sketch.circle", &[(-3.0, 2.0), (1.0, 10.0)]),
        ("sketch.circle3", &[(-8.0, 10.0), (20.0, 3.0), (0.0, -6.0)]),
        ("sketch.arc3", &[(-8.0, 10.0), (20.0, 3.0), (0.0, -6.0)]),
        ("sketch.arc3", &[(-8.0, 10.0), (0.0, -6.0), (20.0, 3.0)]),
        (
            "sketch.ellipse3",
            &[(-10.0, 10.0), (20.0, -4.0), (0.0, -6.0)],
        ),
        ("sketch.ellipse", &[(-6.0, 10.0), (4.0, 4.0), (-3.0, 13.0)]),
        ("sketch.slot", &[(-6.0, 10.0), (-6.0, -2.0)]),
        (
            "sketch.ellipse_arc",
            &[(-6.0, 10.0), (4.0, 4.0), (-3.0, 13.0), (-14.0, 12.0)],
        ),
        (
            "sketch.ellipse_arc",
            &[(-6.0, 10.0), (4.0, 4.0), (-1.0, 10.0), (-11.0, 10.0)],
        ),
        (
            "sketch.parabola",
            &[(-6.0, 10.0), (-6.0, 4.0), (-16.0, 3.0), (4.0, 3.0)],
        ),
        (
            "sketch.parabola",
            &[(-6.0, 10.0), (-6.0, 4.0), (-16.0, 3.0), (20.0, -18.1667)],
        ),
        (
            "sketch.hyperbola",
            &[(-6.0, 10.0), (-6.0, 4.0), (20.0, -3.0), (-16.0, 3.0)],
        ),
        (
            "sketch.arc_slot",
            &[(-6.0, 10.0), (20.0, 3.0), (-6.0, 40.0)],
        ),
    ];
    for (tool, clicks) in cases {
        let mut h = Harness::sketching();
        h.wb.options.auto_update = false;
        h.wb.tool_params_mut().slot_width = 4.0;
        let (lines, mark) = h.edit_sketch(|s| {
            let mut point =
                |x, y| s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
            let ends = [
                point(-30.0, 10.0),
                point(30.0, 10.0),
                point(20.0, -30.0),
                point(20.0, 30.0),
            ];
            let mark = point(0.0, -6.0);
            let level = s.add_geometry(GeometryElement::Line(Line::new(ends[0], ends[1])));
            let upright = s.add_geometry(GeometryElement::Line(Line::new(ends[2], ends[3])));
            (
                [(level, ends[0], ends[1]), (upright, ends[2], ends[3])],
                mark,
            )
        });
        let before: Vec<_> = h
            .sketch()
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        for &(x, y) in clicks {
            h.full_click(x, y, tool);
        }
        let mut sketch = h.sketch();
        let case = format!("{tool} {clicks:?}");
        // The point each snapped click stands for.
        let held: Vec<_> = clicks
            .iter()
            .filter_map(|&(x, y)| {
                sketch.geometry.iter().find_map(|g| match g {
                    GeometryElement::Point(p)
                        if (p.position - Vec2D::new(x, y)).to_glam().length() < 0.05
                            && (y == 10.0 || x == 20.0 || p.id == mark) =>
                    {
                        Some(p.id)
                    }
                    _ => None,
                })
            })
            .collect();
        let snapped = clicks
            .iter()
            .filter(|&&(x, y)| y == 10.0 || x == 20.0 || (x, y) == (0.0, -6.0))
            .count();
        assert_eq!(held.len(), snapped, "{case}: a point at each snapped click");
        let curve = sketch
            .geometry
            .iter()
            .find(|g| {
                !before.contains(&g.id())
                    && match g {
                        GeometryElement::Circle(_)
                        | GeometryElement::Ellipse(_)
                        | GeometryElement::Conic(_) => true,
                        GeometryElement::Arc(a) => {
                            tool != "sketch.slot" || held.contains(&a.center)
                        }
                        _ => false,
                    }
            })
            .map(GeometryElement::id)
            .unwrap_or_else(|| panic!("{case}: no shape"));

        // The new curves' own points, and their centres.
        let new = || sketch.geometry.iter().filter(|g| !before.contains(&g.id()));
        let own: Vec<_> = new().flat_map(Sketch::curve_point_ids).collect();
        let centres: Vec<_> = new()
            .filter(|g| !matches!(g, GeometryElement::Line(_) | GeometryElement::BSpline(_)))
            .filter_map(|g| Sketch::curve_point_ids(g).first().copied())
            .collect();
        let status = core_document::testing::call_checked(
            &mut wb_sketch::SketchWorkbench::default(),
            &mut h.doc,
            "sketch.status",
            serde_json::json!({ "sketch": h.active_object.unwrap().0.to_string() }),
        )
        .unwrap();
        for list in ["redundant", "conflicting"] {
            assert_eq!(status[list], serde_json::json!([]), "{case}: {list}");
        }

        // Each held point no new curve's own is on the shape once: none
        // was dropped as saying again what the others do.
        for &id in held.iter().filter(|id| !own.contains(id)) {
            let holds = sketch
                .constraints
                .iter()
                .filter(|c| match c.kind {
                    ConstraintKind::PointOnCircle { point, circle: on }
                    | ConstraintKind::PointOnEllipse { point, ellipse: on } => {
                        point == id && on == curve
                    }
                    _ => false,
                })
                .count();
            assert_eq!(holds, 1, "{case}: a held point on the shape");
        }

        // The lines and the point move; the shape follows.
        let moved = [
            (lines[0].1, -30.0, 11.5),
            (lines[0].2, 30.0, 11.5),
            (lines[1].1, 21.5, -30.0),
            (lines[1].2, 21.5, 30.0),
            (mark, 0.5, -6.5),
        ];
        for (point, x, y) in moved {
            sketch.add_constraint(ConstraintKind::FixedPoint {
                point,
                position: Vec2D::new(x, y),
            });
        }
        assert!(
            matches!(
                sketch_model::operations::solve(&mut sketch),
                Ok(sketch_model::operations::SolveOutcome::Converged { .. })
            ),
            "{case}: solves"
        );
        let on = |id, element| {
            let at = sketch.point_position(id).unwrap();
            let element = sketch.get_geometry(element).unwrap();
            distance_to_element(&sketch, element, at).unwrap()
        };
        for &id in &held {
            let near_line = lines.iter().map(|l| on(id, l.0)).fold(f64::MAX, f64::min);
            assert!(
                near_line < 1e-6 || id == mark,
                "{case}: off its line by {near_line}"
            );
            if !centres.contains(&id) {
                let off = on(id, curve);
                assert!(off < 1e-6, "{case}: the shape left a held point by {off}");
            }
        }
    }
}

/// The centre-point arc's end clicked on a line (an ellipse, a parabola)
/// near where the arc's circle crosses it lands on the crossing, named by
/// the cue, and is held on the line. Clicked on the line away from the
/// circle, the cue names no snap and the end lands on the circle, free of
/// the line.
#[test]
fn an_arc_s_end_lands_where_its_circle_meets_what_it_snapped_to() {
    let centre = Vec2D::new(5.0, -3.0);
    let on_ellipse = Vec2D::new(5.0 + (500f64 / 3.0).sqrt(), -3.0 + (175f64 / 3.0).sqrt());
    let on_level = Vec2D::new(5.0 + 56f64.sqrt(), 10.0);
    let on_parabola = Vec2D::new(14.0, 9.0);
    for (x, y, crossing, held, drawn) in [
        (on_level.x + 0.2, 10.0, on_level, true, "line"),
        (20.0, 10.0, on_level, false, "line"),
        (
            on_ellipse.x + 0.1,
            on_ellipse.y + 0.1,
            on_ellipse,
            true,
            "ellipse",
        ),
        (14.1, 9.1, on_parabola, true, "parabola"),
    ] {
        let mut h = Harness::sketching();
        let curve = h.edit_sketch(|s| {
            let mut at =
                |x, y| s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
            let drawn = match drawn {
                "line" => GeometryElement::Line(Line::new(at(-30.0, 10.0), at(30.0, 10.0))),
                "ellipse" => GeometryElement::Ellipse(Ellipse::new(
                    at(centre.x, centre.y),
                    Vec2D::new(20.0, 0.0),
                    0.5,
                )),
                _ => GeometryElement::Conic(Conic::new(
                    ConicKind::Parabola,
                    at(5.0, 0.0),
                    Vec2D::new(0.0, 2.25),
                    0.0,
                    at(-7.0, 16.0),
                    at(17.0, 16.0),
                )),
            };
            s.add_geometry(drawn)
        });
        h.click(centre.x, centre.y, "sketch.arc");
        h.click(20.0, -3.0, "sketch.arc");
        h.mouse_move(x, y, "sketch.arc");
        let names = ["Endpoint", "Intersection", "On curve", "Midpoint"];
        let cue = h
            .labels()
            .into_iter()
            .find(|l| names.contains(&l.text.as_str()));
        h.click(x, y, "sketch.arc");
        let sketch = h.sketch();
        let arc = crate::arc_of(&sketch);
        let end = sketch.point_position(arc.end).unwrap();
        let on_it = sketch.constraints.iter().any(|c| match c.kind {
            ConstraintKind::PointOnLine { point, line: on }
            | ConstraintKind::PointOnEllipse { point, ellipse: on }
            | ConstraintKind::PointOnCurve { point, curve: on } => point == arc.end && on == curve,
            _ => false,
        });
        let case = format!("end at ({x}, {y})");
        assert_eq!(
            cue.map(|l| l.text),
            held.then(|| "Intersection".into()),
            "{case}"
        );
        assert_eq!(on_it, held, "{case}");
        assert_eq!((end - crossing).to_glam().length() < 1e-3, held, "{case}");
        let off = ((end - centre).to_glam().length() - 15.0).abs();
        assert!(off < 1e-3, "{case}: off the circle by {off}");
    }
}

/// A typed sweep puts the centre-point arc's end on its circle: a point
/// there is reused, one near it but off the circle is left alone.
#[test]
fn a_typed_arc_end_reuses_only_a_point_on_its_circle() {
    for (x, y, reused) in [(0.0, 10.0, true), (0.15, 10.1, false)] {
        let mut h = Harness::sketching();
        // The start on a point, so the radius is exactly 10.
        let near = h.edit_sketch(|s| {
            let mut at =
                |x, y| s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
            at(10.0, 0.0);
            at(x, y)
        });
        h.click(0.0, 0.0, "sketch.arc");
        h.click(10.0, 0.0, "sketch.arc");
        h.mouse_move(3.0, 9.0, "sketch.arc");
        h.type_text("90", Some("sketch.arc"));
        h.key(core_document::KeyCode::Enter, Some("sketch.arc"));
        let sketch = h.sketch();
        let arc = crate::arc_of(&sketch);
        assert_eq!(arc.end == near, reused, "point at ({x}, {y})");
        let [end, start] = [arc.end, arc.start].map(|p| sketch.point_position(p).unwrap());
        let off = (end.to_glam().length() - start.to_glam().length()).abs();
        assert!(off < 1e-6, "({x}, {y}): end off the circle by {off}");
    }
}

/// A centre-point arc's end, clicked or typed, reuses the point within
/// reach that lies on its circle even when a nearer one lies off it.
#[test]
fn an_arc_end_reuses_a_point_on_its_circle_past_a_nearer_one_off_it() {
    let at = |deg: f64, r: f64| {
        let (sin, cos) = deg.to_radians().sin_cos();
        Vec2D::new(r * cos, r * sin)
    };
    for typed in [false, true] {
        let mut h = Harness::sketching();
        let on = h.edit_sketch(|s| {
            let mut put = |p| s.add_geometry(GeometryElement::Point(Point::new(p)));
            put(at(0.0, 10.0));
            put(at(60.0, 10.01));
            put(at(60.25, 10.0))
        });
        h.click(0.0, 0.0, "sketch.arc");
        h.click(10.0, 0.0, "sketch.arc");
        let end = at(60.0, 10.0);
        if typed {
            h.mouse_move(end.x, end.y, "sketch.arc");
            h.type_text("60", Some("sketch.arc"));
            h.key(core_document::KeyCode::Enter, Some("sketch.arc"));
        } else {
            h.click(end.x, end.y, "sketch.arc");
        }
        assert_eq!(crate::arc_of(&h.sketch()).end, on, "typed: {typed}");
    }
}

/// A circle's rim clicked onto a text's outline keeps no point there: the
/// outlines are made again from the text each time the sketch is read,
/// which drops any constraint on them, so the circle is its centre and
/// itself, and a reload keeps every constraint.
#[test]
fn a_click_on_a_text_outline_is_not_held_on_it() {
    let mut h = Harness::sketching();
    let spec = sketch_model::text::TextSpec {
        text: "I".into(),
        ..Default::default()
    };
    h.edit_sketch(|s| sketch_model::text::add(s, Vec2D::new(0.0, 0.0), &spec).unwrap());
    let sketch = h.sketch();
    let longest = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) if sketch.derived.contains(&l.id) => {
                let a = sketch.point_position(l.start)?.to_glam();
                Some((a, sketch.point_position(l.end)?.to_glam()))
            }
            _ => None,
        })
        .max_by(|x, y| x.0.distance(x.1).total_cmp(&y.0.distance(y.1)));
    let (a, b) = longest.expect("an outline line");
    let at = a.lerp(b, 0.3);
    let made = core_document::testing::call_checked(
        &mut wb_sketch::SketchWorkbench::default(),
        &mut h.doc,
        "sketch.draw",
        serde_json::json!({
            "sketch": h.active_object.unwrap().0.to_string(),
            "tool": "circle",
            "points": [[-10.0, at.y], [at.x, at.y]],
            "params": {"auto_constraints": true},
        }),
    )
    .unwrap();
    assert_eq!(made["elements"].as_array().unwrap().len(), 2, "{made}");
    let sketch = h.sketch();
    let read: Sketch = serde_json::from_value(serde_json::to_value(&sketch).unwrap()).unwrap();
    let ids = |s: &Sketch| s.constraints.iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(&read), ids(&sketch));
}
