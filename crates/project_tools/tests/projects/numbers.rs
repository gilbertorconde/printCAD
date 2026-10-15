use project_archive::archive::FcstdError;
use project_archive::document_xml::FcDocument;
use project_tools::document_import::import_document;
use project_tools::error::ImportError;

pub(super) fn sketch_xml(geometry: &str, constraint: &str, pose: [f64; 7]) -> String {
    let attributes = ["Px", "Py", "Pz", "Q0", "Q1", "Q2", "Q3"]
        .into_iter()
        .zip(pose)
        .map(|(name, value)| format!("{name}=\"{value}\""))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<Document><Objects><Object name=\"Profile\" type=\"Sketcher::SketchObject\"/></Objects><ObjectData><Object name=\"Profile\"><Properties><Property name=\"Placement\" type=\"Placement\"><PropertyPlacement {attributes}/></Property><Property name=\"Geometry\" type=\"GeometryList\"><GeometryList>{geometry}</GeometryList></Property><Property name=\"Constraints\" type=\"ConstraintList\"><ConstraintList>{constraint}</ConstraintList></Property></Properties></Object></ObjectData></Document>"
    )
}

const IDENTITY: [f64; 7] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
pub(super) const POINT: &str = "<Geometry type=\"Part::GeomPoint\"><GeomPoint X=\"1\" Y=\"2\"/><Construction value=\"0\"/></Geometry>";

fn refusal(xml: &str, name: &str, field: &str, why: &str) {
    let source = FcDocument::parse(xml).unwrap();
    let error = import_document(&source, "numbers")
        .err()
        .expect("invalid source must not return an output document");
    let ImportError::Source(FcstdError::Format { object, reason }) = error else {
        panic!("format error expected: {error}")
    };
    assert_eq!(object, name);
    assert!(reason.contains(field) && reason.contains(why), "{reason}");
}

#[test]
fn sketch_numbers_survive_import_and_native_roundtrip_without_narrowing() {
    for value in [
        100.000003,
        1e8 + 1.0,
        f64::from(f32::MAX) * (1.0 + 1e-6),
        -f64::from(f32::MAX) * (1.0 + 1e-6),
        1e308,
    ] {
        let geometry = format!(
            "<Geometry type=\"Part::GeomPoint\"><GeomPoint X=\"{value}\" Y=\"2\"/><Construction value=\"0\"/></Geometry><Geometry type=\"Part::GeomCircle\"><Circle CenterX=\"0\" CenterY=\"0\" Radius=\"{}\" NormalZ=\"1\"/><Construction value=\"0\"/></Geometry>",
            value.abs()
        );
        let constraint = format!(
            "<Constrain Type=\"7\" Value=\"{value}\" First=\"0\" FirstPos=\"1\" Second=\"-2000\" SecondPos=\"0\" Third=\"-2000\" ThirdPos=\"0\" Name=\"X\"/>"
        );
        let mut pose = IDENTITY;
        pose[..3].fill(value);
        let source = FcDocument::parse(&sketch_xml(&geometry, &constraint, pose)).unwrap();
        let mut project = import_document(&source, "precision").unwrap();
        let imported = &project.sketches["Profile"];
        assert_eq!(imported.sketch.plane.origin, [value; 3]);
        assert_eq!(
            imported
                .sketch
                .point_position(imported.source_points[0].0)
                .unwrap()
                .x,
            value
        );
        assert!(
            matches!(imported.sketch.get_geometry(imported.source_radii[0].0), Some(wb_sketch::sketch::GeometryElement::Circle(c)) if c.radius == value.abs())
        );
        assert!(
            matches!(imported.sketch.constraints[0].kind, wb_sketch::sketch::ConstraintKind::DistanceX { value: stored, .. } if stored == value)
        );
        let bytes = project
            .document
            .save_to_bytes(core_document::Compression::None)
            .unwrap();
        let loaded = core_document::Document::load_from_bytes(bytes).unwrap();
        let project_tools::document_import::ImportedObject::Sketch(id) = project.objects["Profile"]
        else {
            unreachable!()
        };
        assert_eq!(
            loaded.get_feature_data(id).unwrap(),
            project.document.get_feature_data(id).unwrap()
        );
        assert!(project.build_failures.is_empty());
    }
}

#[test]
fn extrusion_range_refusals_keep_the_solid_object_and_property() {
    let (_, mut source) = super::fixture("body_features");
    let pad = source.objects.iter_mut().find(|o| o.name == "Pad").unwrap();
    pad.properties.get_mut("Length").unwrap().value[0]
        .attrs
        .iter_mut()
        .find(|a| a.0 == "value")
        .unwrap()
        .1 = "1e308".into();
    let error = import_document(&source, "length")
        .err()
        .expect("out of range extrusion");
    let ImportError::Source(FcstdError::Format { object, reason }) = error else {
        panic!("format error expected")
    };
    assert_eq!(object, "Pad");
    assert!(
        reason.contains("Length") && reason.contains("outside f32 range"),
        "{reason}"
    );
}

#[test]
fn nonfinite_sketch_values_are_errors_rather_than_unsupported_geometry() {
    for text in ["NaN", "inf", "-inf", "1e309", "-1e309"] {
        for (geometry, field) in [
            (
                format!(
                    "<Geometry type=\"Part::GeomPoint\"><GeomPoint X=\"{text}\" Y=\"2\"/><Construction value=\"0\"/></Geometry>"
                ),
                "GeomPoint.X",
            ),
            (
                format!(
                    "<Geometry type=\"Part::GeomCircle\"><Circle CenterX=\"0\" CenterY=\"0\" Radius=\"{text}\" NormalZ=\"1\"/><Construction value=\"0\"/></Geometry>"
                ),
                "GeomCircle.Radius",
            ),
        ] {
            refusal(
                &sketch_xml(&geometry, "", IDENTITY),
                "Profile",
                field,
                "not finite",
            );
        }
        let constraint = format!(
            "<Constrain Type=\"7\" Value=\"{text}\" First=\"0\" FirstPos=\"1\" Second=\"-2000\" SecondPos=\"0\" Third=\"-2000\" ThirdPos=\"0\" Name=\"X\"/>"
        );
        refusal(
            &sketch_xml(POINT, &constraint, IDENTITY),
            "Profile",
            "Constrain.Value",
            "not finite",
        );
    }
}

#[test]
fn stored_planes_preserve_rotation_meaning_and_serialization() {
    let mut cases = 0;
    for (degrees, expected_y) in [
        (180.0_f64, [0.0, -1.0, 0.0]),
        (270.0, [0.0, 0.0, -1.0]),
        (360.0, [0.0, 1.0, 0.0]),
        (450.0, [0.0, 0.0, 1.0]),
        (720.0, [0.0, 1.0, 0.0]),
        (-180.0, [0.0, -1.0, 0.0]),
        (-270.0, [0.0, 0.0, 1.0]),
        (-360.0, [0.0, 1.0, 0.0]),
        (-450.0, [0.0, 0.0, -1.0]),
        (-720.0, [0.0, 1.0, 0.0]),
    ] {
        for delta in [-1e-5_f64, 0.0, 1e-5] {
            let half = (degrees.to_radians() + delta) * 0.5;
            let source = FcDocument::parse(&sketch_xml(
                POINT,
                "",
                [2.0, -3.0, 4.0, half.sin(), 0.0, 0.0, half.cos()],
            ))
            .unwrap();
            let mut project = import_document(&source, "turn").unwrap();
            let plane = project.sketches["Profile"].sketch.plane;
            assert_eq!(plane.origin, [2.0, -3.0, 4.0]);
            for (actual, expected) in plane.x_axis.into_iter().zip([1.0, 0.0, 0.0]) {
                assert!((actual - expected).abs() < 1e-12);
            }
            let y = [
                0.0,
                expected_y[1] - delta * expected_y[2],
                expected_y[2] + delta * expected_y[1],
            ];
            let normal = [0.0, -y[2], y[1]];
            for (actual, expected) in plane
                .y_axis
                .into_iter()
                .zip(y)
                .chain(plane.normal.into_iter().zip(normal))
            {
                assert!((actual - expected).abs() < 1e-9);
            }
            let bytes = project
                .document
                .save_to_bytes(core_document::Compression::None)
                .unwrap();
            let loaded = core_document::Document::load_from_bytes(bytes).unwrap();
            let project_tools::document_import::ImportedObject::Sketch(id) =
                project.objects["Profile"]
            else {
                panic!("sketch expected")
            };
            assert_eq!(
                loaded.get_feature_data(id).unwrap(),
                project.document.get_feature_data(id).unwrap()
            );
            cases += 1;
        }
    }
    assert_eq!(cases, 30);
}
