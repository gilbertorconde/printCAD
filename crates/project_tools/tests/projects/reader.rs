use project_archive::archive::{FcstdArchive, FcstdError};
use project_archive::document_xml::FcDocument;
use project_archive::sketch_decode::{FcShape, FcSketch};

#[test]
fn generated_archives_expose_objects_and_saved_shapes() {
    let (archive, source) = super::fixture("body_features");
    assert_eq!(source.objects.len(), 34);
    assert!(source.object("Pad").unwrap().has("Profile"));
    let file = source.object("Pad").unwrap().shape_file("Shape").unwrap();
    assert!(archive.shape(file).unwrap().contains("CASCADE Topology"));
    assert!(archive.shape("missing.brp").is_err());
}

#[test]
fn malformed_external_data_is_rejected_with_context() {
    assert!(matches!(
        FcstdArchive::from_bytes(b"bad"),
        Err(FcstdError::Zip(_))
    ));
    assert!(FcDocument::parse("<bad/>").is_err());
    let (_, source) = super::fixture("sketch_constraints");
    let xml = source.object("Sketch").unwrap();
    let mut object = xml.clone();
    object.properties.remove("Geometry");
    assert!(
        FcSketch::decode(&object)
            .unwrap_err()
            .to_string()
            .contains("Sketch")
    );
}

#[test]
fn geometry_indices_are_list_order_and_construction_flags_survive() {
    let (_, source) = super::fixture("sketch_constraints");
    let sketch = FcSketch::decode(source.object("Sketch").unwrap()).unwrap();
    assert!(sketch.geometry.iter().any(|g| g.construction));
    assert!(
        sketch
            .geometry
            .iter()
            .any(|g| matches!(g.shape, FcShape::Arc { .. }))
    );
    assert!(sketch.constraints.len() > 20);
}

#[test]
fn missing_document_entry_is_a_named_error() {
    use std::io::Write;
    let mut bytes = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut bytes);
    zip.start_file("other.xml", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"<Document/>").unwrap();
    zip.finish().unwrap();
    assert!(matches!(
        FcstdArchive::from_bytes(bytes.get_ref()),
        Err(FcstdError::MissingEntry("Document.xml"))
    ));
}

#[test]
fn negative_normal_arc_endpoints_follow_the_canonical_point_positions() {
    let (_, source) = super::fixture("sketch_constraints");
    let original = source.object("Sketch").unwrap();
    for (normal, expected_start, expected_end) in [
        ("1", [10.0, 0.0], [0.0, 10.0]),
        ("-1", [0.0, 10.0], [-10.0, 0.0]),
    ] {
        let mut object = original.clone();
        let list = &mut object.properties.get_mut("Geometry").unwrap().value[0];
        let geometry = list
            .children
            .iter_mut()
            .find(|g| g.attr("type") == Some("Part::GeomArcOfCircle"))
            .unwrap();
        let arc = geometry
            .children
            .iter_mut()
            .find(|c| c.tag == "ArcOfCircle")
            .unwrap();
        for (name, value) in [
            ("NormalZ", normal),
            ("CenterX", "0"),
            ("CenterY", "0"),
            ("Radius", "10"),
            ("AngleXU", "0"),
            ("StartAngle", "0"),
            ("EndAngle", "1.5707963267948966"),
        ] {
            arc.attrs.iter_mut().find(|a| a.0 == name).unwrap().1 = value.into();
        }
        let decoded = FcSketch::decode(&object).unwrap();
        let (start, end) = decoded
            .geometry
            .iter()
            .find_map(|g| {
                if let FcShape::Arc { start, end, .. } = g.shape {
                    Some((start, end))
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            glam::DVec2::from_array(start.to_array())
                .distance(glam::DVec2::from_array(expected_start))
                < 1e-12
        );
        assert!(
            glam::DVec2::from_array(end.to_array()).distance(glam::DVec2::from_array(expected_end))
                < 1e-12
        );
    }
}

#[test]
fn a_saved_cone_reads_with_its_whole_slant() {
    // A 2.5 mm drill 7.5 mm deep with a 118° point, its cone on a
    // left-handed frame, as the source saves it.
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../project_archive/tests/data/drill_point.brp"),
    )
    .unwrap();
    let restated = project_archive::restate::restate(
        &text,
        project_archive::document_xml::Placement::IDENTITY,
    )
    .unwrap();
    let mut kernel = kernel_ogeom::OgeomKernel::new();
    let read = kernel.read_brep(&restated, &Default::default()).unwrap();
    let volume = kernel
        .physical_properties(&read.brep_blob)
        .unwrap()
        .volume_mm3
        .unwrap();
    let r = 1.25_f64;
    let point = r / 59_f64.to_radians().tan();
    let want = std::f64::consts::PI * r * r * (7.5 + point / 3.0);
    assert!((volume - want).abs() < 1e-6, "{volume} against {want}");
}
