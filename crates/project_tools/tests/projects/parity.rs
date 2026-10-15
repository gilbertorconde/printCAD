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
        external_count: 0,
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
        external_count: 0,
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
