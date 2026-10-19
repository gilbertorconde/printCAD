use glam::DVec2;
use project_archive::coordinates::Point2;
use project_archive::sketch_decode::{FcGeometry, FcShape, FcSketch};
use project_tools::parity::check_stationary;
use project_tools::sketch_import::import_sketch;
use wb_sketch::sketch::{ConstraintKind, GeometryElement};

fn source(shape: FcShape) -> FcSketch {
    FcSketch {
        geometry: vec![FcGeometry {
            shape,
            construction: false,
            internal: 0,
        }],
        constraints: Vec::new(),
        external: Vec::new(),
    }
}

#[test]
fn stationarity_checks_solver_motion_and_import_rounding_separately() {
    let mut imported = import_sketch(
        &source(FcShape::Line {
            start: Point2::ZERO,
            end: Point2::new(10.0, 0.0),
        }),
        "line",
    )
    .unwrap();
    let line = imported
        .sketch
        .geometry
        .iter()
        .find_map(|g| {
            if let GeometryElement::Line(l) = g {
                Some(l.id)
            } else {
                None
            }
        })
        .unwrap();
    imported
        .sketch
        .add_constraint(ConstraintKind::Length { line, length: 10.0 });
    assert!(check_stationary(&imported).unwrap().holds());
    imported.sketch.constraints[0].kind = ConstraintKind::Length { line, length: 12.0 };
    let moved = check_stationary(&imported).unwrap();
    assert!(!moved.holds());
    assert!(moved.solver_shift_mm > 0.5);
    assert_eq!(moved.rounding_mm, 0.0);
    let precise = import_sketch(
        &source(FcShape::Point(Point2::new(1e8 + 1.0, 0.0))),
        "precise",
    )
    .unwrap();
    let check = check_stationary(&precise).unwrap();
    assert!(check.holds());
    assert_eq!(check.rounding_mm, 0.0);
    assert_eq!(check.solver_shift_mm, 0.0);
}

#[test]
fn arcs_preserve_major_and_wrapped_counterclockwise_spans() {
    for sweep in [180.0_f64, 270.0, 359.999, 450.0, 720.0] {
        for start in [-450.0_f64, -1e-6, 0.0, 360.0, 720.0] {
            let at = |a: f64| DVec2::new(a.to_radians().cos(), a.to_radians().sin()) * 10.0;
            let imported = import_sketch(
                &source(FcShape::Arc {
                    center: Point2::ZERO,
                    radius: 10.0,
                    start: Point2::from_array(at(start).to_array()),
                    end: Point2::from_array(at(start + sweep).to_array()),
                }),
                "arc",
            )
            .unwrap();
            if (sweep % 360.0).abs() > 1e-9 {
                let check = check_stationary(&imported).unwrap();
                assert!(check.holds(), "start={start}, sweep={sweep}, {check:?}");
                assert_eq!(check.radii, 1);
            }
        }
    }
}

#[test]
fn imported_aliases_preserve_construction_and_only_drawn_closed_profiles() {
    use project_archive::sketch_decode::{FcConstraint, FcConstraintType, FcGeoIndex, FcPointPos};
    let mut input = FcSketch {
        geometry: Vec::new(),
        constraints: Vec::new(),
        external: Vec::new(),
    };
    for (offset, construction) in [(0.0, false), (20.0, true)] {
        let points = [
            Point2::new(offset, 0.0),
            Point2::new(offset + 10.0, 0.0),
            Point2::new(offset + 5.0, 8.0),
        ];
        for (start, end) in [(0, 1), (1, 2), (2, 0)] {
            input.geometry.push(FcGeometry {
                shape: FcShape::Line {
                    start: points[start],
                    end: points[end],
                },
                construction,
                internal: 0,
            });
        }
    }
    input.geometry.push(FcGeometry {
        shape: FcShape::Point(Point2::ZERO),
        construction: true,
        internal: 0,
    });
    input.geometry.push(FcGeometry {
        shape: FcShape::Point(Point2::ZERO),
        construction: false,
        internal: 0,
    });
    let coincident = |first, first_pos, second, second_pos, active, driving| FcConstraint {
        kind: FcConstraintType::Coincident,
        value: 0.0,
        first: FcGeoIndex(first),
        first_pos,
        second: FcGeoIndex(second),
        second_pos,
        third: FcGeoIndex::NONE,
        third_pos: FcPointPos::Edge,
        active,
        driving,
        label_distance: 0.0,
        name: String::new(),
    };
    for offset in [0, 3] {
        for index in 0..3 {
            input.constraints.push(coincident(
                offset + index,
                FcPointPos::End,
                offset + (index + 1) % 3,
                FcPointPos::Start,
                true,
                true,
            ));
        }
    }
    input.constraints.push(coincident(
        6,
        FcPointPos::Start,
        2,
        FcPointPos::End,
        true,
        true,
    ));
    input.constraints.push(coincident(
        7,
        FcPointPos::Start,
        0,
        FcPointPos::Start,
        false,
        true,
    ));
    input.constraints.push(coincident(
        7,
        FcPointPos::Start,
        0,
        FcPointPos::Start,
        true,
        false,
    ));
    let imported = import_sketch(&input, "construction").unwrap();
    assert!(imported.findings.is_empty(), "{:?}", imported.findings);
    assert!(check_stationary(&imported).unwrap().holds());
    let lines: Vec<_> = imported
        .sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 6);
    for (index, line) in lines.iter().enumerate() {
        assert_eq!(imported.sketch.is_construction(line.id), index >= 3);
        assert_eq!(line.end, lines[index / 3 * 3 + (index + 1) % 3].start);
    }
    assert!(!imported.sketch.is_construction(lines[0].start));
    assert!(
        !imported
            .sketch
            .is_construction(imported.source_points.last().unwrap().0)
    );
    assert_ne!(
        lines[0].start,
        imported.source_points.last().unwrap().0,
        "inactive and reference coincidences do not merge points"
    );
    assert_eq!(
        imported.source_points[12].0, lines[0].start,
        "source accounting follows the transitive alias"
    );
    assert!(
        imported
            .sketch
            .construction
            .iter()
            .all(|&id| imported.sketch.get_geometry(id).is_some())
    );
    let restored: wb_sketch::sketch::Sketch =
        serde_json::from_value(serde_json::to_value(&imported.sketch).unwrap()).unwrap();
    let wires = wb_sketch::profile::extract_wires(&restored).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 3);
    for segment in &wires[0].segments {
        let kernel_api::ProfileSegment::Line { start, end } = segment else {
            panic!("triangle uses lines")
        };
        for p in [start, end] {
            assert!(p[0] >= 0.0 && p[0] <= 10.0 && p[1] >= 0.0 && p[1] <= 8.0);
        }
    }
}

#[test]
fn projected_and_spline_curves_close_a_profile_and_guides_keep_their_points() {
    use project_archive::sketch_decode::{FcConstraint, FcConstraintType, FcExternal};
    use project_archive::sketch_decode::{FcGeoIndex, FcPointPos as P};
    let curve = |shape, construction| FcGeometry {
        shape,
        construction,
        internal: 0,
    };
    let line = |a: [f64; 2], b: [f64; 2]| FcShape::Line {
        start: Point2::from_array(a),
        end: Point2::from_array(b),
    };
    let spline = FcShape::BSpline {
        poles: [[0.0, 10.0], [-3.0, 5.0], [0.0, 0.0]]
            .map(Point2::from_array)
            .to_vec(),
        weights: vec![1.0; 3],
        knots: vec![(0.0, 3), (1.0, 3)],
        degree: 2,
        periodic: false,
    };
    let guide = FcShape::Circle {
        center: Point2::new(5.0, 5.0),
        radius: 1.0,
    };
    let coincident = |first, first_pos, second, second_pos| FcConstraint {
        kind: FcConstraintType::Coincident,
        value: 0.0,
        first: FcGeoIndex(first),
        first_pos,
        second: FcGeoIndex(second),
        second_pos,
        third: FcGeoIndex::NONE,
        third_pos: P::Edge,
        active: true,
        driving: true,
        label_distance: 0.0,
        name: String::new(),
    };
    let input = FcSketch {
        geometry: vec![
            curve(line([0.0, 0.0], [10.0, 0.0]), false),
            curve(spline, false),
            curve(FcShape::Point(Point2::new(5.0, 5.0)), false),
        ],
        // Two edges the profile takes from elsewhere, and a guide.
        external: [
            (line([10.0, 0.0], [10.0, 10.0]), true),
            (line([10.0, 10.0], [0.0, 10.0]), true),
            (guide, false),
        ]
        .map(|(shape, defining)| FcExternal {
            geometry: curve(shape, true),
            defining,
        })
        .to_vec(),
        constraints: vec![
            coincident(0, P::End, -3, P::Start),
            coincident(-3, P::End, -4, P::Start),
            coincident(-4, P::End, 1, P::Start),
            coincident(1, P::End, 0, P::Start),
            coincident(2, P::Start, -5, P::Mid),
        ],
    };
    let imported = import_sketch(&input, "projected").unwrap();
    assert!(check_stationary(&imported).unwrap().holds());
    let sketch = &imported.sketch;
    let blocked = sketch.constraints.iter();
    let blocked = blocked.filter(|c| matches!(c.kind, ConstraintKind::Block { .. }));
    assert_eq!(blocked.count(), 3, "projected curves stay where they were");
    let wires = wb_sketch::profile::extract_wires(sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
    // The point on the guide's centre stays a point of its own.
    let point = imported
        .source_points
        .iter()
        .find(|(_, p)| *p == DVec2::new(5.0, 5.0));
    let centre = sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Circle(c) => Some(c.center),
        _ => None,
    });
    assert_ne!(point.unwrap().0, centre.unwrap());
}

/// Each kind of constraint the source states in a form of its own comes in
/// as the native one that says the same: a distance from a curve as a gap,
/// an angle at a corner or of one element, a spline's end tangent to an
/// arc, and dimensions to the circles marking a spline's control points
/// as dimensions to the control points. The saved sketch satisfies them
/// all, and the markers' own ties to the spline are no loss.
#[test]
fn constraints_stated_against_curves_corners_and_control_points_come_in_whole() {
    use project_archive::sketch_decode::{
        FcConstraint, FcConstraintType as T, FcGeoIndex, FcPointPos as P,
    };
    use std::f64::consts::{FRAC_PI_2, PI};
    let drawn = |shape| FcGeometry {
        shape,
        construction: false,
        internal: 0,
    };
    let at = |x, y| Point2::new(x, y);
    let poles = [at(30.0, 10.0), at(25.0, 5.0), at(25.0, 0.0)];
    let mut input = FcSketch {
        geometry: vec![
            drawn(FcShape::Line {
                start: at(0.0, 0.0),
                end: at(10.0, 0.0),
            }),
            drawn(FcShape::Point(at(3.0, 4.0))),
            drawn(FcShape::Circle {
                center: at(0.0, 10.0),
                radius: 2.0,
            }),
            drawn(FcShape::Line {
                start: at(10.0, 0.0),
                end: at(10.0, 5.0),
            }),
            drawn(FcShape::Arc {
                center: at(20.0, 0.0),
                radius: 5.0,
                start: at(25.0, 0.0),
                end: at(20.0, 5.0),
            }),
            drawn(FcShape::BSpline {
                poles: poles.to_vec(),
                weights: vec![1.0; 3],
                knots: vec![(0.0, 3), (1.0, 3)],
                degree: 2,
                periodic: false,
            }),
        ],
        constraints: Vec::new(),
        external: Vec::new(),
    };
    for pole in poles {
        input.geometry.push(FcGeometry {
            shape: FcShape::Circle {
                center: pole,
                radius: 1.0,
            },
            construction: true,
            internal: 9,
        });
    }
    let constraint = |kind, value, (first, first_pos), (second, second_pos)| FcConstraint {
        kind,
        value,
        first: FcGeoIndex(first),
        first_pos,
        second: FcGeoIndex(second),
        second_pos,
        third: FcGeoIndex::NONE,
        third_pos: P::Edge,
        active: true,
        driving: true,
        label_distance: 0.0,
        name: String::new(),
    };
    let none = (FcGeoIndex::NONE.0, P::Edge);
    input.constraints = vec![
        constraint(T::Distance, 4.0, (1, P::Start), (0, P::Edge)),
        constraint(T::Distance, 8.0, (2, P::Edge), (0, P::Edge)),
        // Away from the first line's end, round to the second's start.
        constraint(T::Angle, -FRAC_PI_2, (0, P::End), (3, P::Start)),
        constraint(T::Angle, FRAC_PI_2, (4, P::Edge), none),
        constraint(T::Tangent, 0.0, (5, P::End), (4, P::Start)),
        constraint(T::DistanceX, -25.0, (7, P::Mid), (-1, P::Start)),
        constraint(T::InternalAlignment, 0.0, (6, P::Mid), (5, P::Edge)),
        // The pole circles' radii are the spline's weights, all one value.
        constraint(T::Weight, 1.0, (6, P::Edge), none),
        constraint(T::Equal, 0.0, (6, P::Edge), (8, P::Edge)),
    ];
    let imported = import_sketch(&input, "kinds").unwrap();
    assert!(
        !imported.findings.iter().any(|f| f.category == "constraint"),
        "{:?}",
        imported.findings
    );
    let kinds: Vec<_> = imported
        .sketch
        .constraints
        .iter()
        .map(|c| &c.kind)
        .collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, ConstraintKind::Gap { .. }))
            .count(),
        2
    );
    assert!(kinds.iter().any(|k| matches!(k, ConstraintKind::Angle { angle_rad, .. } if (angle_rad - PI / 2.0).abs() < 1e-12)));
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, ConstraintKind::ArcAngle { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, ConstraintKind::TangentCurves { .. }))
    );
    let pole = imported
        .source_points
        .iter()
        .find(|(_, p)| *p == DVec2::new(25.0, 5.0))
        .unwrap()
        .0;
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, ConstraintKind::DistanceX { a, .. } if *a == pole))
    );
    let check = check_stationary(&imported).unwrap();
    assert!(check.holds(), "{check:?}");
}
