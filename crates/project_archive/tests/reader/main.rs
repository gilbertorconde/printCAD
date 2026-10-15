use std::io::{Cursor, Write};

use project_archive::archive::{ArchiveLimits, Bytes, FcstdArchive, FcstdError};
use project_archive::coordinates::{Point2, Point3};
use project_archive::document_xml::FcDocument;
use project_archive::sketch_decode::{FcConstraintType, FcShape, FcSketch};
use project_archive::traversal::{ParentMap, TraversalPath};

fn archive(xml: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("Document.xml", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn source_geometry_and_constraints_decode_without_an_application_model() {
    let xml = r#"<Document><Objects><Object name="Sketch" type="Sketcher::SketchObject"/></Objects><ObjectData><Object name="Sketch"><Properties>
        <Property name="Geometry" type="GeometryList"><GeometryList>
        <Geometry type="Part::GeomLineSegment"><LineSegment StartX="1.25" StartY="-3" EndX="8.5" EndY="2"/><Construction value="0"/></Geometry>
        <Geometry type="Part::GeomCircle"><Circle CenterX="2" CenterY="4" Radius="3" NormalZ="1"/><Construction value="1"/></Geometry>
        </GeometryList></Property>
        <Property name="Constraints" type="ConstraintList"><ConstraintList><Constrain Type="18" Value="6" First="1" FirstPos="0" Second="-2000" SecondPos="0" Third="-2000" ThirdPos="0" Name="Diameter"/></ConstraintList></Property>
        </Properties></Object></ObjectData></Document>"#;
    let bytes = archive(xml);
    let file = FcstdArchive::from_bytes(&bytes).unwrap();
    let document = FcDocument::parse(&file.document_xml).unwrap();
    let sketch = FcSketch::decode(document.object("Sketch").unwrap()).unwrap();
    assert_eq!(sketch.geometry.len(), 2);
    assert_eq!(
        sketch.geometry[0].shape,
        FcShape::Line {
            start: Point2::new(1.25, -3.0),
            end: Point2::new(8.5, 2.0)
        }
    );
    assert_eq!(
        sketch.geometry[1].shape,
        FcShape::Circle {
            center: Point2::new(2.0, 4.0),
            radius: 3.0
        }
    );
    assert!(sketch.geometry[1].construction);
    assert_eq!(sketch.constraints.len(), 1);
    assert_eq!(sketch.constraints[0].kind, FcConstraintType::Diameter);
    assert_eq!(sketch.constraints[0].value, 6.0);
    // A reader-level budget is observable without a converter or a kernel.
    let error = FcstdArchive::from_bytes_with_limits(
        &bytes,
        ArchiveLimits {
            xml: Bytes(xml.len() as u64 - 1),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, FcstdError::Limit { .. }));
}

fn placed_groups() -> FcDocument {
    FcDocument::parse(r#"<Document><Objects><Object name="Outer" type="App::Part"/><Object name="Inner" type="App::Part"/><Object name="Child" type="Unknown"/></Objects><ObjectData>
        <Object name="Outer"><Properties><Property name="Group" type="LinkList"><LinkList><Link value="Inner"/></LinkList></Property><Property name="Placement" type="Placement"><PropertyPlacement Px="10" Py="0" Pz="3" Q0="0" Q1="0" Q2="0.7071067811865476" Q3="0.7071067811865476"/></Property></Properties></Object>
        <Object name="Inner"><Properties><Property name="Group" type="LinkList"><LinkList><Link value="Child"/></LinkList></Property><Property name="Placement" type="Placement"><PropertyPlacement Px="0" Py="5" Pz="0" Q0="0" Q1="0" Q2="0" Q3="1"/></Property></Properties></Object>
        </ObjectData></Document>"#).unwrap()
}

#[test]
fn nested_parent_frames_apply_rotation_before_translation() {
    let document = placed_groups();
    let frame = ParentMap::build(&document)
        .unwrap()
        .placement(&document, "Child")
        .unwrap();
    let expected = Point3::new(5.0, 0.0, 3.0);
    for (actual, expected) in frame
        .position
        .to_array()
        .into_iter()
        .zip(expected.to_array())
    {
        assert!((actual - expected).abs() < 1e-12);
    }
    let x = frame.rotation.transform_vector([1.0, 0.0, 0.0]);
    assert!(x[0].abs() < 1e-12 && (x[1] - 1.0).abs() < 1e-12 && x[2].abs() < 1e-12);
}

#[test]
fn ambiguous_group_ownership_is_an_external_data_error() {
    let mut source = placed_groups();
    let outer = &mut source.objects[0];
    outer.properties.get_mut("Group").unwrap().value[0].children[0].attrs[0].1 = "Child".into();
    let error = ParentMap::build(&source)
        .err()
        .expect("two owners must fail");
    assert!(
        error
            .to_string()
            .contains("Child has multiple parent groups")
    );
}

#[test]
fn a_suspended_walk_can_resume_and_reports_cycles_with_the_chain() {
    let mut path = TraversalPath::new("dependency");
    path.enter("Result").unwrap();
    path.enter("Tool").unwrap();
    path.leave();
    assert_eq!(path.current(), Some("Result"));
    path.enter("Profile").unwrap();
    let error = path.enter("Result").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("dependency cycle: Result -> Profile -> Result")
    );
    path.leave();
    path.leave();
    assert_eq!(path.current(), None);
}

#[test]
fn placement_turns_preserve_axes_across_whole_turns_and_boundaries() {
    let mut cases = 0;
    for (degrees, expected_x) in [
        (180.0_f64, [-1.0, 0.0, 0.0]),
        (270.0, [0.0, -1.0, 0.0]),
        (360.0, [1.0, 0.0, 0.0]),
        (450.0, [0.0, 1.0, 0.0]),
        (720.0, [1.0, 0.0, 0.0]),
        (-180.0, [-1.0, 0.0, 0.0]),
        (-270.0, [0.0, 1.0, 0.0]),
        (-360.0, [1.0, 0.0, 0.0]),
        (-450.0, [0.0, -1.0, 0.0]),
        (-720.0, [1.0, 0.0, 0.0]),
    ] {
        for delta in [-1e-9_f64, 0.0, 1e-9] {
            let half_angle = (degrees.to_radians() + delta) * 0.5;
            let xml = format!(
                "<Document><Objects><Object name=\"Pose\" type=\"Unknown\"/></Objects><ObjectData><Object name=\"Pose\"><Properties><Property name=\"Placement\" type=\"Placement\"><PropertyPlacement Px=\"2\" Py=\"-3\" Pz=\"4\" Q0=\"0\" Q1=\"0\" Q2=\"{}\" Q3=\"{}\"/></Property></Properties></Object></ObjectData></Document>",
                half_angle.sin(),
                half_angle.cos()
            );
            let source = FcDocument::parse(&xml).unwrap();
            let pose = source
                .object("Pose")
                .unwrap()
                .placement("Placement")
                .unwrap();
            assert_eq!(pose.position, Point3::new(2.0, -3.0, 4.0));
            let actual_x = pose.rotation.transform_vector([1.0, 0.0, 0.0]);
            // The tangent to a unit circle at X is its 90-degree CCW rotation.
            // At this epsilon the omitted second-order term is below f64 rounding.
            let expected = [
                expected_x[0] - delta * expected_x[1],
                expected_x[1] + delta * expected_x[0],
                0.0,
            ];
            for (actual, expected) in actual_x.into_iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-12);
            }
            for (actual, expected) in pose
                .rotation
                .transform_vector([0.0, 0.0, 1.0])
                .into_iter()
                .zip([0.0, 0.0, 1.0])
            {
                assert!((actual - expected).abs() < 1e-12);
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 30);
}

fn object_xml(properties: &str) -> String {
    format!(
        "<Document><Objects><Object name=\"Profile\" type=\"Sketcher::SketchObject\"/></Objects><ObjectData><Object name=\"Profile\"><Properties>{properties}</Properties></Object></ObjectData></Document>"
    )
}

fn format_error(error: FcstdError, object: &str, field: &str, why: &str) {
    let FcstdError::Format {
        object: actual,
        reason,
    } = error
    else {
        panic!("expected format error, got {error}");
    };
    assert_eq!(actual, object);
    assert!(reason.contains(field) && reason.contains(why), "{reason}");
}

#[test]
fn nonfinite_property_numbers_are_rejected_with_object_and_attribute() {
    for field in ["Px", "Py", "Pz", "Q0", "Q1", "Q2", "Q3"] {
        for text in ["NaN", "inf", "-inf", "1e309", "-1e309"] {
            let attributes = ["Px", "Py", "Pz", "Q0", "Q1", "Q2", "Q3"]
                .map(|attr| {
                    format!(
                        "{attr}=\"{}\"",
                        if attr == field {
                            text
                        } else if attr == "Q3" {
                            "1"
                        } else {
                            "0"
                        }
                    )
                })
                .join(" ");
            let xml = object_xml(&format!(
                "<Property name=\"Placement\" type=\"Placement\"><PropertyPlacement {attributes}/></Property>"
            ));
            let file = FcstdArchive::from_bytes(&archive(&xml)).unwrap();
            let source = FcDocument::parse(&file.document_xml).unwrap();
            format_error(
                source
                    .object("Profile")
                    .unwrap()
                    .placement("Placement")
                    .unwrap_err(),
                "Profile",
                &format!("Placement.{field}"),
                "not finite",
            );
        }
    }
}

#[test]
fn zero_nonunit_and_overflowing_quaternion_norms_fail_before_normalization() {
    for (w, why) in [
        ("0", "zero quaternion"),
        ("1e-300", "zero quaternion"),
        ("2", "not unit"),
        ("1e308", "norm is not finite"),
    ] {
        let source = FcDocument::parse(&object_xml(&format!("<Property name=\"Placement\" type=\"Placement\"><PropertyPlacement Px=\"0\" Py=\"0\" Pz=\"0\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"{w}\"/></Property>"))).unwrap();
        format_error(
            source
                .object("Profile")
                .unwrap()
                .placement("Placement")
                .unwrap_err(),
            "Profile",
            "Placement",
            why,
        );
    }
    // Only serialization roundoff is normalized, not arbitrary input scaling.
    for (w, accepted) in [
        (1.0 - 0.5e-9, true),
        (1.0 + 0.5e-9, true),
        (1.0 - 2e-9, false),
        (1.0 + 2e-9, false),
    ] {
        let source = FcDocument::parse(&object_xml(&format!("<Property name=\"Placement\" type=\"Placement\"><PropertyPlacement Px=\"0\" Py=\"0\" Pz=\"0\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"{w}\"/></Property>"))).unwrap();
        assert_eq!(
            source
                .object("Profile")
                .unwrap()
                .placement("Placement")
                .is_ok(),
            accepted
        );
    }
}

fn sketch_xml(geometry: &str) -> String {
    object_xml(&format!(
        "<Property name=\"Geometry\" type=\"GeometryList\"><GeometryList>{geometry}</GeometryList></Property><Property name=\"Constraints\" type=\"ConstraintList\"><ConstraintList/></Property>"
    ))
}

#[test]
fn absent_arc_rotation_selects_the_reference_frame_for_both_normals() {
    let mut cases = 0;
    for (degrees, direction) in [
        (180.0_f64, [-1.0, 0.0]),
        (270.0, [0.0, -1.0]),
        (360.0, [1.0, 0.0]),
        (450.0, [0.0, 1.0]),
        (720.0, [1.0, 0.0]),
        (-180.0, [-1.0, 0.0]),
        (-270.0, [0.0, 1.0]),
        (-360.0, [1.0, 0.0]),
        (-450.0, [0.0, -1.0]),
        (-720.0, [1.0, 0.0]),
    ] {
        for delta in [-1e-7_f64, 0.0, 1e-7] {
            // Rotate known cardinal vectors; the oracle does not reconstruct
            // the decoder's absolute angle or its normal-dependent phase.
            let (sin, cos) = delta.sin_cos();
            let radial = [
                direction[0] * cos - direction[1] * sin,
                direction[1] * cos + direction[0] * sin,
            ];
            let quarter = [-radial[1], radial[0]];
            let begin = degrees.to_radians() + delta;
            for radius in [0.1_f64, 10.0, 1000.0] {
                for normal in [-1.0, 1.0] {
                    let expected = if normal > 0.0 {
                        [radial, quarter]
                    } else {
                        [[-quarter[0], quarter[1]], [-radial[0], radial[1]]]
                    };
                    for rotation in ["", " AngleXU=\"0\""] {
                        let xml = sketch_xml(&format!(
                            "<Geometry type=\"Part::GeomArcOfCircle\"><ArcOfCircle CenterX=\"2\" CenterY=\"-3\" Radius=\"{radius}\" NormalZ=\"{normal}\" StartAngle=\"{begin}\" EndAngle=\"{}\"{rotation}/><Construction value=\"0\"/></Geometry>",
                            begin + std::f64::consts::FRAC_PI_2
                        ));
                        let source = FcDocument::parse(&xml).unwrap();
                        let sketch = FcSketch::decode(source.object("Profile").unwrap()).unwrap();
                        let FcShape::Arc {
                            center,
                            radius: actual_radius,
                            start,
                            end,
                        } = sketch.geometry[0].shape
                        else {
                            panic!("expected arc")
                        };
                        assert_eq!(center, Point2::new(2.0, -3.0));
                        assert_eq!(actual_radius, radius);
                        for (actual, unit) in [start, end].into_iter().zip(expected) {
                            assert!(
                                (actual.x - (2.0 + radius * unit[0])).abs()
                                    < 1e-11 * radius.max(1.0)
                            );
                            assert!(
                                (actual.y - (-3.0 + radius * unit[1])).abs()
                                    < 1e-11 * radius.max(1.0)
                            );
                        }
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 180);
}

#[test]
fn optional_arc_rotation_does_not_hide_invalid_or_missing_fields() {
    let attrs =
        "CenterX=\"0\" CenterY=\"0\" Radius=\"1\" NormalZ=\"1\" StartAngle=\"0\" EndAngle=\"1\"";
    for rotation in ["", "broken", "NaN", "inf", "-inf", "1e309"] {
        let source = FcDocument::parse(&sketch_xml(&format!(
            "<Geometry type=\"Part::GeomArcOfCircle\"><ArcOfCircle {attrs} AngleXU=\"{rotation}\"/><Construction value=\"0\"/></Geometry>"
        ))).unwrap();
        let error = FcSketch::decode(source.object("Profile").unwrap()).unwrap_err();
        assert!(error.to_string().contains("AngleXU"), "{error}");
    }
    for missing in [
        "Radius=\"1\"",
        "NormalZ=\"1\"",
        "StartAngle=\"0\"",
        "EndAngle=\"1\"",
    ] {
        let source = FcDocument::parse(&sketch_xml(&format!(
            "<Geometry type=\"Part::GeomArcOfCircle\"><ArcOfCircle {}/><Construction value=\"0\"/></Geometry>",
            attrs.replace(missing, "")
        ))).unwrap();
        let error = FcSketch::decode(source.object("Profile").unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(missing.split('=').next().unwrap()),
            "{error}"
        );
    }
}

#[test]
fn sketch_coordinates_include_optional_spatial_components_and_computed_endpoints() {
    for (kind, tag, attrs, field) in [
        (
            "GeomLineSegment",
            "LineSegment",
            "StartX=\"0\" StartY=\"0\" EndX=\"1\" EndY=\"2\"",
            "StartZ",
        ),
        ("GeomPoint", "GeomPoint", "X=\"1\" Y=\"2\"", "Z"),
        (
            "GeomCircle",
            "Circle",
            "CenterX=\"0\" CenterY=\"0\" Radius=\"1\" NormalZ=\"1\"",
            "NormalX",
        ),
        (
            "GeomArcOfCircle",
            "ArcOfCircle",
            "CenterX=\"0\" CenterY=\"0\" Radius=\"1\" NormalZ=\"1\" AngleXU=\"0\" StartAngle=\"0\" EndAngle=\"1\"",
            "NormalY",
        ),
    ] {
        for (value, why) in [
            ("NaN", "not finite"),
            ("inf", "not finite"),
            ("-inf", "not finite"),
            ("1", "outside the local XY plane"),
        ] {
            let xml = sketch_xml(&format!(
                "<Geometry type=\"Part::{kind}\"><{tag} {attrs} {field}=\"{value}\"/><Construction value=\"0\"/></Geometry>"
            ));
            let source = FcDocument::parse(&xml).unwrap();
            format_error(
                FcSketch::decode(source.object("Profile").unwrap()).unwrap_err(),
                "Profile",
                field,
                why,
            );
        }
    }
    for (attrs, field) in [
        (
            "CenterX=\"1e308\" CenterY=\"0\" Radius=\"1e308\" AngleXU=\"0\" StartAngle=\"0\" EndAngle=\"1\"",
            "endpoint.x",
        ),
        (
            "CenterX=\"0\" CenterY=\"0\" Radius=\"1\" AngleXU=\"1e308\" StartAngle=\"1e308\" EndAngle=\"0\"",
            "computed angle",
        ),
    ] {
        let source = FcDocument::parse(&sketch_xml(&format!("<Geometry type=\"Part::GeomArcOfCircle\"><ArcOfCircle {attrs} NormalZ=\"1\"/><Construction value=\"0\"/></Geometry>"))).unwrap();
        format_error(
            FcSketch::decode(source.object("Profile").unwrap()).unwrap_err(),
            "Profile",
            field,
            "not finite",
        );
    }
}

#[test]
fn parent_composition_cannot_overflow_finite_source_translations() {
    let mut source = placed_groups();
    for object in &mut source.objects[..2] {
        let pose = &mut object.properties.get_mut("Placement").unwrap().value[0];
        for (field, value) in [("Px", "1e308"), ("Q2", "0"), ("Q3", "1")] {
            pose.attrs.iter_mut().find(|a| a.0 == field).unwrap().1 = value.into();
        }
    }
    format_error(
        ParentMap::build(&source)
            .unwrap()
            .placement(&source, "Child")
            .unwrap_err(),
        "Child",
        "composed Placement.position.x",
        "not finite",
    );
}
