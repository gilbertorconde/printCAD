//! Shapes drawn through clicks a snap put on a curve or a point stay
//! through them, and centred on one there: moving what was snapped to
//! carries the shape along. The holds say nothing twice.

use super::Harness;
use wb_sketch::sketch::{ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D};
use wb_sketch::snap::distance_to_element;

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
                wb_sketch::operations::solve(&mut sketch),
                Ok(wb_sketch::operations::SolveOutcome::Converged { .. })
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

/// The centre-point arc's end clicked on a line near where the arc's
/// circle crosses it lands on the crossing, named by the cue, and is held
/// on the line. Clicked on the line away from the circle, the cue names no
/// snap and the end lands on the circle, free of the line.
#[test]
fn an_arc_s_end_lands_where_its_circle_meets_what_it_snapped_to() {
    let (centre, crossing) = (Vec2D::new(5.0, -3.0), Vec2D::new(5.0 + 56f64.sqrt(), 10.0));
    for (x, held) in [(crossing.x + 0.2, true), (20.0, false)] {
        let mut h = Harness::sketching();
        h.line((-30.0, 10.0), (30.0, 10.0));
        h.click(centre.x, centre.y, "sketch.arc");
        h.click(20.0, -3.0, "sketch.arc");
        h.mouse_move(x, 10.0, "sketch.arc");
        let names = ["Endpoint", "Intersection", "On curve", "Midpoint"];
        let cue = h
            .labels()
            .into_iter()
            .find(|l| names.contains(&l.text.as_str()));
        h.click(x, 10.0, "sketch.arc");
        let sketch = h.sketch();
        let (arc, line) = (crate::arc_of(&sketch), crate::line_of(&sketch).id);
        let end = sketch.point_position(arc.end).unwrap();
        let on_line = sketch.constraints.iter().any(|c| {
            matches!(c.kind, ConstraintKind::PointOnLine { point, line: l }
                if point == arc.end && l == line)
        });
        let case = format!("end at x = {x}");
        assert_eq!(
            cue.map(|l| l.text),
            held.then(|| "Intersection".into()),
            "{case}"
        );
        assert_eq!(on_line, held, "{case}");
        assert_eq!((end - crossing).to_glam().length() < 1e-3, held, "{case}");
        let off = ((end - centre).to_glam().length() - 15.0).abs();
        assert!(off < 1e-3, "{case}: off the circle by {off}");
    }
}
