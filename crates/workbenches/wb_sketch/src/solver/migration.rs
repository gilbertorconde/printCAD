//! Portable application-boundary cases with stable identities. The saved
//! answers pin outcomes, compilation, diagnosis and post-processing;
//! geometric assertions also check the meaning of selected answers.

use std::time::Instant;

use serde_json::{Value, json};
use sketch_solver::solve::{inf_norm, var_scale};

use super::*;
use crate::sketch::{
    Arc, BSpline, Circle, Conic, ConicKind, Constraint, Ellipse, ExternalReference, ExternalSource,
    Line, Point, TextBlock,
};

fn point(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
    let id = Uuid::from_u128(sketch.geometry.len() as u128 + 1);
    sketch.add_geometry(GeometryElement::Point(Point {
        id,
        position: Vec2D::new(x, y),
    }))
}

fn line(sketch: &mut Sketch, start: Uuid, end: Uuid) -> Uuid {
    let id = Uuid::from_u128(sketch.geometry.len() as u128 + 1);
    sketch.add_geometry(GeometryElement::Line(Line { id, start, end }))
}

fn relation(sketch: &mut Sketch, kind: ConstraintKind) {
    let mut constraint = Constraint::new(kind);
    constraint.id = Uuid::from_u128(1000 + sketch.constraints.len() as u128);
    sketch.constraints.push(constraint);
}

fn fixed(sketch: &mut Sketch, point: Uuid, x: f32, y: f32) {
    relation(
        sketch,
        ConstraintKind::FixedPoint {
            point,
            position: Vec2D::new(x, y),
        },
    );
}

fn circle(sketch: &mut Sketch, center: Uuid, radius: f32) -> Uuid {
    let id = Uuid::from_u128(sketch.geometry.len() as u128 + 1);
    sketch.add_geometry(GeometryElement::Circle(Circle { id, center, radius }))
}

fn cases() -> Vec<(Sketch, Vec<Uuid>)> {
    let mut cases = Vec::new();
    let mut add = |mut sketch: Sketch, held: Vec<Uuid>| {
        sketch.id = Uuid::from_u128(900);
        cases.push((sketch, held));
    };
    add(Sketch::new("empty"), vec![]);
    for name in [
        "ordinary",
        "held_success",
        "held_fallback",
        "conflict",
        "redundant",
        "filtered",
    ] {
        let mut sketch = Sketch::new(name);
        let a = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, 5.0, 5.0);
        let l = line(&mut sketch, a, b);
        relation(
            &mut sketch,
            ConstraintKind::Length {
                line: l,
                length: 10.0,
            },
        );
        if name != "held_success" {
            fixed(&mut sketch, a, 0.0, 0.0);
            relation(&mut sketch, ConstraintKind::Horizontal { element: l });
        }
        if name == "held_fallback" {
            let GeometryElement::Point(p) = sketch.get_geometry_mut(b).unwrap() else {
                unreachable!()
            };
            p.position = Vec2D::new(40.0, 0.0);
        }
        if name == "conflict" || name == "redundant" {
            relation(
                &mut sketch,
                ConstraintKind::Length {
                    line: l,
                    length: if name == "conflict" { 12.0 } else { 10.0 },
                },
            );
        }
        if name == "filtered" {
            sketch.constraints[0].driving = false;
            sketch.constraints[1].active = false;
            relation(
                &mut sketch,
                ConstraintKind::Length {
                    line: Uuid::from_u128(500),
                    length: 3.0,
                },
            );
        }
        let held = if name.starts_with("held_") {
            vec![b]
        } else {
            vec![]
        };
        add(sketch, held);
    }
    for count in [60, 61] {
        let mut sketch = Sketch::new(format!("diagnosis_{count}"));
        let p = point(&mut sketch, 0.0, 0.0);
        for _ in 0..count {
            fixed(&mut sketch, p, 0.0, 0.0);
        }
        add(sketch, vec![]);
    }
    let mut sketch = Sketch::new("references");
    let p = point(&mut sketch, 4.0, 5.0);
    relation(
        &mut sketch,
        ConstraintKind::Coincident {
            point1: p,
            point2: ORIGIN_ID,
        },
    );
    relation(
        &mut sketch,
        ConstraintKind::PointOnLine {
            point: p,
            line: X_AXIS_ID,
        },
    );
    add(sketch, vec![]);

    let mut sketch = Sketch::new("external_fully_held");
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 10.0, 0.0);
    let l = line(&mut sketch, a, b);
    sketch.external.insert(
        l,
        ExternalSource::of_reference(ExternalReference::Datum {
            datum: Uuid::from_u128(700),
        }),
    );
    relation(
        &mut sketch,
        ConstraintKind::Length {
            line: l,
            length: 10.0,
        },
    );
    add(sketch, vec![]);

    for (name, distance) in [("tangent_external", 12.0), ("tangent_internal", 3.0)] {
        let mut sketch = Sketch::new(name);
        let a = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, distance, 0.0);
        let ca = circle(&mut sketch, a, 5.0);
        let cb = circle(&mut sketch, b, 2.0);
        fixed(&mut sketch, a, 0.0, 0.0);
        relation(
            &mut sketch,
            ConstraintKind::Radius {
                circle: ca,
                radius: 5.0,
            },
        );
        relation(
            &mut sketch,
            ConstraintKind::Radius {
                circle: cb,
                radius: 2.0,
            },
        );
        relation(
            &mut sketch,
            ConstraintKind::Tangent {
                line_or_circle1: ca,
                item2: cb,
            },
        );
        add(sketch, vec![]);
    }
    for name in ["arc_unconstrained", "arc_radius"] {
        let mut sketch = Sketch::new(name);
        let c = point(&mut sketch, 0.0, 0.0);
        let s = point(&mut sketch, 5.0, 0.0);
        let e = point(&mut sketch, 0.0, 5.0);
        let id = Uuid::from_u128(4);
        sketch.add_geometry(GeometryElement::Arc(Arc {
            id,
            center: c,
            start: s,
            end: e,
            radius: 5.0,
        }));
        if name == "arc_radius" {
            fixed(&mut sketch, c, 0.0, 0.0);
            relation(
                &mut sketch,
                ConstraintKind::Radius {
                    circle: id,
                    radius: 8.0,
                },
            );
        }
        add(sketch, vec![]);
    }
    for name in ["ellipse_swap", "ellipse_focus_probe", "ellipse_held_focus"] {
        let mut sketch = Sketch::new(name);
        let c = point(&mut sketch, 0.0, 0.0);
        let f = point(&mut sketch, 4.0, 0.0);
        let e = Uuid::from_u128(3);
        sketch.add_geometry(GeometryElement::Ellipse(Ellipse {
            id: e,
            center: c,
            major: Vec2D::new(5.0, 0.0),
            ratio: 0.6,
            arc: None,
        }));
        let a = point(&mut sketch, -5.0, 0.0);
        let b = point(&mut sketch, 5.0, 0.0);
        let axis = line(&mut sketch, a, b);
        fixed(&mut sketch, c, 0.0, 0.0);
        relation(
            &mut sketch,
            ConstraintKind::InternalAlignment {
                element: axis,
                curve: e,
                role: InternalRole::MajorAxis,
            },
        );
        relation(
            &mut sketch,
            ConstraintKind::InternalAlignment {
                element: f,
                curve: e,
                role: InternalRole::Focus1,
            },
        );
        relation(
            &mut sketch,
            ConstraintKind::EllipseRadius {
                ellipse: e,
                major: true,
                radius: 5.0,
            },
        );
        relation(
            &mut sketch,
            ConstraintKind::EllipseRadius {
                ellipse: e,
                major: false,
                radius: if name == "ellipse_swap" { 7.0 } else { 3.0 },
            },
        );
        if name == "ellipse_focus_probe" {
            fixed(&mut sketch, f, 4.0, 0.0);
        }
        add(
            sketch,
            if name == "ellipse_held_focus" {
                vec![f]
            } else {
                vec![]
            },
        );
    }
    for (name, periodic) in [
        ("rational_spline", false),
        ("periodic_spline", true),
        ("fit_spline", false),
    ] {
        let mut sketch = Sketch::new(name);
        let a = point(&mut sketch, 0.0, 0.0);
        let b = point(&mut sketch, 5.0, 10.0);
        let c = point(&mut sketch, 10.0, 0.0);
        let mut spline = BSpline::new(vec![a, b, c], periodic);
        spline.id = Uuid::from_u128(4);
        spline.degree = 2;
        if name == "rational_spline" {
            spline.weights = vec![1.0, 0.7, 1.0];
        }
        if name == "fit_spline" {
            let f0 = point(&mut sketch, 0.0, 0.0);
            let f1 = point(&mut sketch, 5.0, 5.0);
            let f2 = point(&mut sketch, 10.0, 0.0);
            spline.id = Uuid::from_u128(7);
            spline.fit_points = vec![f0, f1, f2];
            spline.fit_params = vec![0.0, 0.5, 1.0];
            fixed(&mut sketch, f1, 5.0, 7.0);
        } else {
            fixed(&mut sketch, a, 0.0, 0.0);
            fixed(&mut sketch, b, 5.0, 10.0);
            fixed(&mut sketch, c, 10.0, 0.0);
            let p = point(&mut sketch, 5.0, 6.0);
            spline.id = Uuid::from_u128(5);
            relation(
                &mut sketch,
                ConstraintKind::PointOnCurve {
                    point: p,
                    curve: spline.id,
                },
            );
        }
        sketch.add_geometry(GeometryElement::BSpline(spline));
        add(sketch, vec![]);
    }
    for (name, kind, axis, minor, initial) in [
        ("parabola", ConicKind::Parabola, 2.0, 0.0, [1.0, 3.0]),
        ("hyperbola", ConicKind::Hyperbola, 3.0, 2.0, [4.0, 1.0]),
    ] {
        let mut sketch = Sketch::new(name);
        let c = point(&mut sketch, 0.0, 0.0);
        let a = point(&mut sketch, initial[0], initial[1]);
        let b = point(&mut sketch, initial[0], -initial[1]);
        sketch.add_geometry(GeometryElement::Conic(Conic {
            id: Uuid::from_u128(4),
            kind,
            center: c,
            axis: Vec2D::new(axis, 0.0),
            minor,
            start: a,
            end: b,
        }));
        fixed(&mut sketch, c, 0.0, 0.0);
        add(sketch, vec![]);
    }
    let mut sketch = Sketch::new("text_follow");
    let anchor = point(&mut sketch, 0.0, 0.0);
    let a = point(&mut sketch, 1.0, 2.0);
    let b = point(&mut sketch, 3.0, 2.0);
    let l = line(&mut sketch, a, b);
    sketch.texts.push(TextBlock {
        id: Uuid::from_u128(800),
        text: "I".into(),
        font: crate::text::DEFAULT_FONT.into(),
        size: 10.0,
        spacing: 0.0,
        angle: 0.0,
        anchor,
        elements: vec![a, b, l],
        placed: Vec2D::new(0.0, 0.0),
    });
    fixed(&mut sketch, anchor, 7.0, 9.0);
    add(sketch, vec![]);
    cases
}

fn compiled(sys: &System) -> Value {
    compiled_core(sys)
}

fn compiled_core(sys: &sketch_solver::compile::System) -> Value {
    use sketch_solver::problem::PointReference;
    let mut derived: Vec<_> = sys
        .derived_foci
        .iter()
        .map(|id| match id {
            PointReference::Point(id) => Uuid::from_u128(id.0),
            PointReference::Origin => ORIGIN_ID,
        })
        .collect();
    derived.sort();
    json!({"values": sys.vars, "free": sys.free, "equations": sys.residual_len,
        "spec_dimensions": sys.specs.iter().map(sketch_solver::residual::ResidualSpec::dim).collect::<Vec<_>>(), "derived_foci": derived})
}

#[test]
fn semantic_replay_corpus_matches_ported_solver() {
    use sketch_solver::replay::{Fixture, Regression, SCHEMA_VERSION, run};
    let replay: Vec<_> = cases()
        .into_iter()
        .map(|(input, held)| {
            let fixture = Fixture {
                schema_version: SCHEMA_VERSION,
                problem: adapter::prepare(&input, &held),
            };
            let expected = run(&fixture);
            Regression {
                name: input.name,
                fixture,
                expected,
            }
        })
        .collect();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../sketch_solver/fixtures/migration.json");
    if std::env::var_os("PRINTCAD_CAPTURE_SOLVER_REPLAY_CORPUS").is_some() {
        assert!(
            !path.exists(),
            "the saved semantic corpus must not be overwritten"
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&replay).unwrap() + "\n").unwrap();
    }
    let mut saved: Vec<Regression> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in &mut saved {
        case.fixture.problem.external.sort();
        case.fixture.problem.application_held_points.sort();
    }
    equivalent(
        &serde_json::to_value(saved).unwrap(),
        &serde_json::to_value(replay).unwrap(),
        "semantic_replay",
    );
}

#[test]
fn compilation_reports_explain_filtering_and_partial_offsets() {
    use super::report::{Disposition, compilation_reports};
    let mut sketch = Sketch::new("reports");
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 4.0, 0.0);
    let first = line(&mut sketch, a, b);
    let c = point(&mut sketch, 0.0, 2.0);
    let d = point(&mut sketch, 4.0, 2.0);
    let second = line(&mut sketch, c, d);
    relation(
        &mut sketch,
        ConstraintKind::Length {
            line: first,
            length: 4.0,
        },
    );
    sketch.constraints[0].active = false;
    relation(
        &mut sketch,
        ConstraintKind::Length {
            line: first,
            length: 4.0,
        },
    );
    sketch.constraints[1].driving = false;
    let missing = Uuid::from_u128(999);
    relation(
        &mut sketch,
        ConstraintKind::Length {
            line: missing,
            length: 4.0,
        },
    );
    relation(
        &mut sketch,
        ConstraintKind::Offset {
            pairs: vec![[first, second], [first, missing], [first, a]],
            distance: 2.0,
        },
    );
    relation(&mut sketch, ConstraintKind::Horizontal { element: first });
    sketch.constraints[4].driving = false;
    relation(
        &mut sketch,
        ConstraintKind::Pitch {
            points: vec![a],
            columns: 1,
            distance: 1.0,
            across: false,
            direction: None,
        },
    );
    let reports = compilation_reports(&sketch);
    assert_eq!(reports.len(), 6);
    assert_eq!(reports[0].disposition, Disposition::Inactive);
    assert_eq!(reports[1].disposition, Disposition::ReferenceDimension);
    assert_eq!(reports[2].disposition, Disposition::NoEquations);
    assert!(matches!(
        reports[2].input_error,
        Some(sketch_solver::input::InputError::MissingReference(_))
    ));
    assert_eq!(reports[3].disposition, Disposition::Compiled);
    assert_eq!(reports[3].equations.len(), 2);
    assert_eq!(reports[3].skipped_members, vec![1, 2]);
    assert!(reports[3].input_error.is_some());
    assert_eq!(reports[4].disposition, Disposition::Compiled);
    assert_eq!(reports[5].disposition, Disposition::NoEquations);
    assert!(reports[5].input_error.is_none());
}

/// This match makes a new application relation require an inventory entry.
fn relation_name(kind: &ConstraintKind) -> &'static str {
    match kind {
        ConstraintKind::FixedPoint { .. } => "FixedPoint",
        ConstraintKind::Coincident { .. } => "Coincident",
        ConstraintKind::Parallel { .. } => "Parallel",
        ConstraintKind::Perpendicular { .. } => "Perpendicular",
        ConstraintKind::EqualLength { .. } => "EqualLength",
        ConstraintKind::Length { .. } => "Length",
        ConstraintKind::EqualRadius { .. } => "EqualRadius",
        ConstraintKind::Radius { .. } => "Radius",
        ConstraintKind::Diameter { .. } => "Diameter",
        ConstraintKind::PointOnLine { .. } => "PointOnLine",
        ConstraintKind::PointOnCircle { .. } => "PointOnCircle",
        ConstraintKind::PointOnEllipse { .. } => "PointOnEllipse",
        ConstraintKind::Horizontal { .. } => "Horizontal",
        ConstraintKind::Vertical { .. } => "Vertical",
        ConstraintKind::HorizontalPoints { .. } => "HorizontalPoints",
        ConstraintKind::VerticalPoints { .. } => "VerticalPoints",
        ConstraintKind::Block { .. } => "Block",
        ConstraintKind::Distance { .. } => "Distance",
        ConstraintKind::DistanceX { .. } => "DistanceX",
        ConstraintKind::DistanceY { .. } => "DistanceY",
        ConstraintKind::Angle { .. } => "Angle",
        ConstraintKind::AngleToAxis { .. } => "AngleToAxis",
        ConstraintKind::Tangent { .. } => "Tangent",
        ConstraintKind::Symmetric { .. } => "Symmetric",
        ConstraintKind::SymmetricAboutPoint { .. } => "SymmetricAboutPoint",
        ConstraintKind::Midpoint { .. } => "Midpoint",
        ConstraintKind::ArcLength { .. } => "ArcLength",
        ConstraintKind::Gap { .. } => "Gap",
        ConstraintKind::AngleAtPoint { .. } => "AngleAtPoint",
        ConstraintKind::EllipseRadius { .. } => "EllipseRadius",
        ConstraintKind::CurveLength { .. } => "CurveLength",
        ConstraintKind::PointOnCurve { .. } => "PointOnCurve",
        ConstraintKind::TangentCurves { .. } => "TangentCurves",
        ConstraintKind::PerpendicularCurves { .. } => "PerpendicularCurves",
        ConstraintKind::EqualEllipse { .. } => "EqualEllipse",
        ConstraintKind::Offset { .. } => "Offset",
        ConstraintKind::Pitch { .. } => "Pitch",
        ConstraintKind::PolarPitch { .. } => "PolarPitch",
        ConstraintKind::ArcAngle { .. } => "ArcAngle",
        ConstraintKind::AngleThreePoints { .. } => "AngleThreePoints",
        ConstraintKind::Refraction { .. } => "Refraction",
        ConstraintKind::InternalAlignment { .. } => "InternalAlignment",
    }
}

#[test]
fn every_relation_has_an_equation_inventory_entry() {
    let mut sketch = Sketch::new("inventory");
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 5.0, 0.0);
    let c = point(&mut sketch, 0.0, 5.0);
    let la = line(&mut sketch, a, b);
    let lb = line(&mut sketch, a, c);
    let ca = circle(&mut sketch, a, 5.0);
    let cb = circle(&mut sketch, b, 2.0);
    let ellipse = Uuid::from_u128(8);
    sketch.add_geometry(GeometryElement::Ellipse(Ellipse {
        id: ellipse,
        center: a,
        major: Vec2D::new(5.0, 0.0),
        ratio: 0.6,
        arc: None,
    }));
    let arc = Uuid::from_u128(9);
    sketch.add_geometry(GeometryElement::Arc(Arc {
        id: arc,
        center: a,
        start: b,
        end: c,
        radius: 5.0,
    }));
    let base_rows = build_system_with(&sketch, None, &[], true).residual_len;
    let relations = vec![
        (
            ConstraintKind::FixedPoint {
                point: a,
                position: Vec2D::new(0.0, 0.0),
            },
            2,
        ),
        (
            ConstraintKind::Coincident {
                point1: a,
                point2: b,
            },
            2,
        ),
        (
            ConstraintKind::Parallel {
                line1: la,
                line2: lb,
            },
            1,
        ),
        (
            ConstraintKind::Perpendicular {
                line1: la,
                line2: lb,
            },
            1,
        ),
        (
            ConstraintKind::EqualLength {
                line1: la,
                line2: lb,
            },
            1,
        ),
        (
            ConstraintKind::Length {
                line: la,
                length: 5.0,
            },
            1,
        ),
        (
            ConstraintKind::EqualRadius {
                circle1: ca,
                circle2: cb,
            },
            1,
        ),
        (
            ConstraintKind::Radius {
                circle: ca,
                radius: 5.0,
            },
            1,
        ),
        (
            ConstraintKind::Diameter {
                circle: ca,
                diameter: 10.0,
            },
            1,
        ),
        (ConstraintKind::PointOnLine { point: a, line: la }, 1),
        (
            ConstraintKind::PointOnCircle {
                point: b,
                circle: ca,
            },
            1,
        ),
        (ConstraintKind::PointOnEllipse { point: b, ellipse }, 1),
        (ConstraintKind::Horizontal { element: la }, 1),
        (ConstraintKind::Vertical { element: lb }, 1),
        (
            ConstraintKind::HorizontalPoints {
                point1: a,
                point2: b,
            },
            1,
        ),
        (
            ConstraintKind::VerticalPoints {
                point1: a,
                point2: c,
            },
            1,
        ),
        (ConstraintKind::Block { element: ca }, 3),
        (
            ConstraintKind::Distance {
                point1: a,
                point2: b,
                distance: 5.0,
            },
            1,
        ),
        (
            ConstraintKind::DistanceX {
                a,
                b: Some(b),
                value: -5.0,
            },
            1,
        ),
        (
            ConstraintKind::DistanceY {
                a,
                b: None,
                value: 5.0,
            },
            1,
        ),
        (
            ConstraintKind::Angle {
                line1: la,
                line2: lb,
                angle_rad: 1.0,
            },
            1,
        ),
        (
            ConstraintKind::AngleToAxis {
                line: la,
                axis: AxisDirection::Horizontal,
                angle_rad: 0.0,
            },
            1,
        ),
        (
            ConstraintKind::Tangent {
                line_or_circle1: la,
                item2: cb,
            },
            1,
        ),
        (
            ConstraintKind::Symmetric {
                point1: a,
                point2: b,
                line: lb,
            },
            2,
        ),
        (
            ConstraintKind::SymmetricAboutPoint {
                point1: a,
                point2: b,
                center: c,
            },
            2,
        ),
        (ConstraintKind::Midpoint { point: c, line: la }, 2),
        (ConstraintKind::ArcLength { arc, length: 4.0 }, 1),
        (
            ConstraintKind::Gap {
                item1: ca,
                item2: cb,
                distance: 2.0,
            },
            1,
        ),
        (
            ConstraintKind::AngleAtPoint {
                curve1: la,
                curve2: ca,
                point: b,
                angle_rad: 1.0,
            },
            1,
        ),
        (
            ConstraintKind::EllipseRadius {
                ellipse,
                major: true,
                radius: 5.0,
            },
            1,
        ),
        (
            ConstraintKind::CurveLength {
                curve: la,
                length: 5.0,
            },
            1,
        ),
        (
            ConstraintKind::PointOnCurve {
                point: c,
                curve: la,
            },
            2,
        ),
        (
            ConstraintKind::TangentCurves {
                curve1: la,
                curve2: ca,
            },
            3,
        ),
        (
            ConstraintKind::PerpendicularCurves {
                curve1: la,
                curve2: ca,
            },
            3,
        ),
        (
            ConstraintKind::EqualEllipse {
                ellipse1: ellipse,
                ellipse2: ellipse,
            },
            2,
        ),
        (
            ConstraintKind::Offset {
                pairs: vec![[la, lb], [ca, cb]],
                distance: 2.0,
            },
            3,
        ),
        (
            ConstraintKind::Pitch {
                points: vec![a, b, c],
                columns: 3,
                distance: 5.0,
                across: false,
                direction: None,
            },
            3,
        ),
        (
            ConstraintKind::PolarPitch {
                center: a,
                points: vec![b, c],
                angle_rad: 1.0,
            },
            2,
        ),
        (
            ConstraintKind::ArcAngle {
                arc,
                angle_rad: 1.0,
            },
            1,
        ),
        (
            ConstraintKind::AngleThreePoints {
                point1: b,
                vertex: a,
                point2: c,
                angle_rad: 1.0,
            },
            1,
        ),
        (
            ConstraintKind::Refraction {
                ray1: la,
                ray2: lb,
                interface: ca,
                point: b,
                ratio: 1.5,
            },
            1,
        ),
        (
            ConstraintKind::InternalAlignment {
                element: la,
                curve: ellipse,
                role: InternalRole::MajorAxis,
            },
            2,
        ),
    ];
    let inventory = include_str!("../../../../../docs/rfcs/0003-solver-equations.md");
    let mut names = std::collections::HashSet::new();
    for (kind, rows) in relations {
        let name = relation_name(&kind);
        assert!(names.insert(name));
        assert!(inventory.contains(&format!("| {name} |")), "{name}");
        let mut probe = sketch.clone();
        relation(&mut probe, kind);
        let sys = build_system_with(&probe, None, &[], true);
        assert_eq!(sys.residual_len - base_rows, rows, "{name}");
        let problem = adapter::prepare(&probe, &[]);
        let core = sketch_solver::compile::compile_system(&problem, None, true);
        equivalent(&compiled(&sys), &compiled_core(&core), name);
    }
    assert_eq!(names.len(), 42);
}

fn outcome(value: SolveOutcome) -> Value {
    match value {
        SolveOutcome::Converged { iterations } => json!({"converged": iterations}),
        SolveOutcome::NotConverged { residual } => json!({"not_converged": residual}),
        SolveOutcome::NothingToSolve => json!("nothing_to_solve"),
    }
}

fn answer(input: &Sketch, held: &[Uuid]) -> Value {
    let ordinary = build_system(input);
    let held_sys = build_system_holding(input, None, held);
    let mut attempt = input.clone();
    let held_outcome = solve_system(&mut attempt, held_sys);
    let mut sketch = input.clone();
    let result = solve_holding(&mut sketch, held);
    let diagnosis = diagnose(&sketch);
    json!({"input": input, "held": held, "ordinary_compile": compiled(&ordinary),
        "freedom_compile": compiled(&build_system_with(input, None, &[], true)),
        "held_compile": compiled(&build_system_holding(input, None, held)),
        "held_attempt": outcome(held_outcome), "outcome": outcome(result),
        "stored": sketch, "dof": dof_estimate(&sketch),
        "diagnosis": {"dof": diagnosis.dof, "analyzed": diagnosis.analyzed,
            "redundant": diagnosis.redundant, "conflicting": diagnosis.conflicting}})
}

/// Stored f32 geometry may differ by one rounding unit on another platform.
/// Integer metadata, IDs, enum tags, ordering and topology compare exactly;
/// floating values use these fixed tolerances at every migration step.
fn equivalent(expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            let tolerance = 1e-6 * a.abs().max(1.0);
            assert!(
                (a - b).abs() <= tolerance,
                "{path}: {a} != {b}, tolerance {tolerance}"
            );
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                equivalent(a, b, &format!("{path}/{i}"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{path}"
            );
            for (key, a) in a {
                equivalent(a, &b[key], &format!("{path}/{key}"));
            }
        }
        _ => assert_eq!(expected, actual, "{path}"),
    }
}

#[test]
fn captured_application_corpus() {
    let mut actual = serde_json::Map::new();
    for (sketch, held) in cases() {
        actual.insert(sketch.name.clone(), answer(&sketch, &held));
    }
    let actual = Value::Object(actual);
    if std::env::var_os("PRINTCAD_CAPTURE_SOLVER_CORPUS").is_some() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/solver/migration.json");
        std::fs::write(path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: Value = serde_json::from_str(include_str!("migration.json")).unwrap();
    assert_eq!(expected.as_object().unwrap().len(), 24);
    equivalent(&expected, &actual, "corpus");
}

#[test]
fn corpus_checks_geometric_meaning_and_held_fallback() {
    for (mut sketch, held) in cases() {
        let name = sketch.name.clone();
        let result = solve_holding(&mut sketch, &held);
        if name == "conflict" {
            assert!(matches!(result, SolveOutcome::NotConverged { .. }));
            assert!(sketch.unsolved);
        } else if name == "held_success" || name == "held_fallback" || name == "ordinary" {
            assert!(
                matches!(result, SolveOutcome::Converged { .. }),
                "{name}: {result:?}"
            );
            let a = sketch.point_position(Uuid::from_u128(1)).unwrap();
            let b = sketch.point_position(Uuid::from_u128(2)).unwrap();
            assert!(((b - a).to_glam().length() - 10.0).abs() < 1e-4);
            if name == "held_success" {
                assert_eq!(b, Vec2D::new(5.0, 5.0));
            }
            if name == "held_fallback" {
                assert!(b.x < 11.0);
                assert!(a.to_glam().length() < 1e-5);
            }
        } else if name == "text_follow" {
            assert!(matches!(result, SolveOutcome::Converged { .. }));
            assert_eq!(
                sketch.point_position(Uuid::from_u128(2)).unwrap(),
                Vec2D::new(8.0, 11.0)
            );
        } else if name.starts_with("tangent_") {
            assert!(matches!(result, SolveOutcome::Converged { .. }));
            let a = sketch.point_position(Uuid::from_u128(1)).unwrap();
            let b = sketch.point_position(Uuid::from_u128(2)).unwrap();
            let expected = if name == "tangent_internal" { 3.0 } else { 7.0 };
            assert!(((b - a).to_glam().length() - expected).abs() < 1e-4);
        }
    }
}

#[test]
fn iteration_exposes_failed_attempts_without_application_writeback() {
    let cases = cases();
    for (input, held) in &cases {
        let before = serde_json::to_value(input).unwrap();
        let system = build_system_with(input, None, held, false);
        let result = iterate(&system, input.solver);
        assert_eq!(
            serde_json::to_value(input).unwrap(),
            before,
            "{}",
            input.name
        );
        assert!(
            result.values.iter().all(|value| value.is_finite()),
            "{}",
            input.name
        );
        assert_eq!(
            result.residual,
            inf_norm(&eval_residuals(&system, &result.values))
        );
        assert_eq!(
            result.threshold,
            input.solver.tolerance * var_scale(&result.values)
        );
        match result.outcome {
            SolveOutcome::Converged { iterations } => {
                assert_eq!(result.iterations, iterations);
                assert!(result.residual < result.threshold);
            }
            SolveOutcome::NotConverged { residual } => {
                assert_eq!(result.residual, residual);
                assert!(result.iterations > 0);
                assert!(result.residual >= result.threshold);
            }
            SolveOutcome::NothingToSolve => assert_eq!(result.iterations, 0),
        }
        if input.name == "held_fallback" {
            assert!(matches!(result.outcome, SolveOutcome::NotConverged { .. }));
            // The failed numerical attempt itself keeps the held point;
            // application retry is responsible for releasing it later.
            let at = system.point_vars[&adapter::point(held[0])];
            assert_eq!(result.values[at], 40.0);
            assert_eq!(
                input.point_position(held[0]).unwrap(),
                Vec2D::new(40.0, 0.0)
            );
        }
    }
}

#[test]
fn numerical_residual_and_stored_geometry_are_measured_separately() {
    let (mut input, held) = cases()
        .into_iter()
        .find(|(s, _)| s.name == "ordinary")
        .unwrap();
    let system = build_system_with(&input, None, &held, false);
    let result = iterate(&system, input.solver);
    assert!(matches!(result.outcome, SolveOutcome::Converged { .. }));
    assert!(result.residual < result.threshold);
    apply(&mut input, &system, &result);
    let a = input.point_position(Uuid::from_u128(1)).unwrap().to_glam();
    let b = input.point_position(Uuid::from_u128(2)).unwrap().to_glam();
    assert!(a.length() < 1e-5);
    assert!((b.y - a.y).abs() < 1e-5);
    assert!(((b - a).length() - 10.0).abs() < 1e-4);
    assert!(input.is_fully_constrained);
    assert!(!input.unsolved);
}

#[test]
fn corpus_comparison_rejects_geometry_and_metadata_mutations() {
    let (input, held) = cases()
        .into_iter()
        .find(|(s, _)| s.name == "ordinary")
        .unwrap();
    let expected = answer(&input, &held);
    for pointer in [
        "/stored/geometry/1/Point/position/x",
        "/dof",
        "/outcome/converged",
    ] {
        let mut changed = expected.clone();
        *changed.pointer_mut(pointer).unwrap() = json!(999);
        assert!(
            std::panic::catch_unwind(|| equivalent(&expected, &changed, "mutation")).is_err(),
            "{pointer}"
        );
    }
}

#[test]
#[ignore = "manual measurement: compile, solve including application write-back, and diagnosis"]
fn measure_migration_baseline() {
    let inputs = cases();
    let repetitions = 50;
    for phase in ["compile", "iterate", "solve_apply", "diagnose"] {
        // Preparation is outside the iteration timer. The other phases
        // keep their whole-corpus application-boundary measurement.
        let prepared: Vec<_> = inputs
            .iter()
            .map(|(input, held)| build_system_with(input, None, held, false))
            .collect();
        let start = Instant::now();
        for _ in 0..repetitions {
            for ((input, held), system) in inputs.iter().zip(&prepared) {
                match phase {
                    "compile" => {
                        std::hint::black_box(build_system_holding(input, None, held));
                    }
                    "solve_apply" => {
                        std::hint::black_box(solve_holding(&mut input.clone(), held));
                    }
                    "iterate" => {
                        std::hint::black_box(iterate(system, input.solver));
                    }
                    "diagnose" => {
                        std::hint::black_box(diagnose(input));
                    }
                    _ => unreachable!(),
                }
            }
        }
        eprintln!(
            "solver baseline {phase}: {} cases x {repetitions}, {:.3} us/corpus",
            inputs.len(),
            start.elapsed().as_secs_f64() * 1e6 / f64::from(repetitions)
        );
    }
}
