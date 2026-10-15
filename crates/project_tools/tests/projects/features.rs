use core_document::Document;
use project_archive::document_xml::FcDocument;
use project_tools::document_import::{ImportedObject, convert_into};
use serde_json::{Value, json};

fn property(name: &str, inner: &str) -> String {
    format!("<Property name=\"{name}\" type=\"t\">{inner}</Property>")
}

fn number(name: &str, value: f64) -> String {
    property(
        name,
        &format!("<Float value=\"{value}\"/><Integer value=\"{value}\"/>"),
    )
}

fn link(name: &str, target: &str, sub: &str) -> String {
    let inner = format!(
        "<Link value=\"{target}\"/><LinkSub value=\"{target}\"><Sub value=\"{sub}\"/></LinkSub>"
    );
    property(name, &inner)
}

fn sketch(name: &str, z: f64) -> (String, &'static str, String) {
    circle(name, z, 0.0, 5.0)
}

/// A sketch at height `z` of one circle at (`x`, 0).
fn circle(name: &str, z: f64, x: f64, radius: f64) -> (String, &'static str, String) {
    let circle = format!(
        "<Geometry type=\"Part::GeomCircle\"><Circle CenterX=\"{x}\" CenterY=\"0\" Radius=\"{radius}\" NormalZ=\"1\"/><Construction value=\"0\"/></Geometry>"
    );
    let props = [
        property(
            "Placement",
            &format!(
                "<PropertyPlacement Px=\"0\" Py=\"0\" Pz=\"{z}\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"1\"/>"
            ),
        ),
        property(
            "Geometry",
            &format!("<GeometryList>{circle}</GeometryList>"),
        ),
        property("Constraints", "<ConstraintList/>"),
    ];
    (name.into(), "Sketcher::SketchObject", props.concat())
}

/// A saved shape of one flat face at z = 3 bounded by one edge, from
/// (0, 0, 3) to (4, 0, 3): `Face1` and `Edge1`.
const SAVED: &str = "Locations 0
Curve2ds 0
Curves 1
1 0 0 3 1 0 0
Polygon3D 0
PolygonOnTriangulations 0
Surfaces 1
1 0 0 3 0 0 1 1 0 0 0 1 0
Triangulations 0

TShapes 5
Ve
1e-07
0 0 3
0 0

0101101
*
Ve
1e-07
4 0 3
0 0

0101101
*
Ed
 1e-07 1 1 0
1  1 0 0 4
0

0101000
+5 0 -4 0 *
Wi

0101000
+3 0 *
Fa
0  1e-07 1 0

0111000
+2 0 *

+1 0
";

/// The Design feature an object of the given type and properties becomes,
/// built on a box (whose saved shape is `SAVED`) from a circle sketch, and
/// the findings naming it.
fn converted(kind: &str, props: &[String]) -> (Option<Value>, Vec<String>) {
    let placement = property(
        "Placement",
        "<PropertyPlacement Px=\"0\" Py=\"0\" Pz=\"0\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"1\"/>",
    );
    let saved = property("Shape", "<Part file=\"Base.brp\"/>");
    let objects = [
        sketch("S", 0.0),
        sketch("T", 10.0),
        (
            "Plane".into(),
            "App::Plane",
            property("Role", "<String value=\"XY_Plane\"/>"),
        ),
        (
            "Base".into(),
            "Part::Box",
            [
                placement,
                saved,
                number("Length", 9.0),
                number("Width", 9.0),
                number("Height", 9.0),
            ]
            .concat(),
        ),
        (
            "F".into(),
            kind,
            [link("BaseFeature", "Base", ""), props.concat()].concat(),
        ),
    ];
    let mut document = Document::new("features");
    let source = FcDocument::parse(&xml(&objects)).unwrap();
    let shapes = [("Base.brp".to_string(), SAVED.to_string())].into();
    let out = convert_into(&source, &shapes, &mut document).unwrap();
    let findings = out.findings.iter().filter(|f| f.object == "F");
    let findings = findings.map(|f| f.reason.clone()).collect();
    let feature = out.objects.get("F").map(|&object| {
        let ImportedObject::Solid { body, .. } = object else {
            panic!("a solid")
        };
        let tip = document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .unwrap()
            .tip
            .unwrap();
        document.get_feature_data(tip).unwrap().clone()
    });
    (feature, findings)
}

fn xml(objects: &[(String, &str, String)]) -> String {
    let mut xml = String::from("<Document><Objects>");
    for (name, kind, _) in objects {
        xml += &format!("<Object name=\"{name}\" type=\"{kind}\"/>");
    }
    xml += "</Objects><ObjectData>";
    for (name, _, props) in objects {
        xml += &format!("<Object name=\"{name}\"><Properties>{props}</Properties></Object>");
    }
    xml + "</ObjectData></Document>"
}

/// Every field of `expected` is in `actual`, as it is there.
fn has(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            e.iter().all(|(k, v)| a.get(k).is_some_and(|a| has(a, v)))
        }
        _ => actual == expected,
    }
}

#[test]
fn ends_sides_holes_lofts_and_mirrors_become_their_design_features() {
    let enums = "<Integer value=\"2\"/><CustomEnumList><Enum value=\"None\"/><Enum value=\"Counterbore\"/><Enum value=\"ISO 4762\"/></CustomEnumList>";
    let cases = [
        // Both sides, the first to the last face: a newer file's sides.
        (
            "PartDesign::Pad",
            vec![
                link("Profile", "S", ""),
                property("Reversed", "<Bool value=\"false\"/>"),
                number("SideType", 1.0),
                number("Type", 1.0),
                number("Type2", 0.0),
                number("Length", 3.0),
                number("Length2", 4.0),
            ],
            json!({ "Pad": { "mode": "ToLast", "mode2": "Dimension", "length2": 4.0 } }),
        ),
        // Up to a face of the saved shape, a pad's offset moving it back.
        (
            "PartDesign::Pad",
            vec![
                link("Profile", "S", ""),
                property("Reversed", "<Bool value=\"false\"/>"),
                number("Type", 3.0),
                link("UpToFace", "Base", "Face1"),
                number("Offset", 2.0),
            ],
            json!({ "Pad": { "mode": "UpToFace", "up_to_offset": -2.0, "up_to_face": { "point": [2.0, 0.0, 3.0], "normal": [0.0, 0.0, 1.0] } } }),
        ),
        (
            "PartDesign::Fillet",
            vec![link("Base", "Base", "Edge1"), number("Radius", 1.0)],
            json!({ "Fillet": { "radius": 1.0, "edges": { "Edges": [{ "point": [2.0, 0.0, 3.0], "direction": [1.0, 0.0, 0.0] }] } } }),
        ),
        (
            "PartDesign::Chamfer",
            vec![
                property("UseAllEdges", "<Bool value=\"true\"/>"),
                number("ChamferType", 1.0),
                number("Size", 1.0),
                number("Size2", 2.0),
            ],
            json!({ "Chamfer": { "mode": "TwoDistances", "size2": 2.0, "edges": "All" } }),
        ),
        // An older file's two lengths.
        (
            "PartDesign::Pocket",
            vec![
                link("Profile", "S", ""),
                property("Reversed", "<Bool value=\"false\"/>"),
                number("Type", 4.0),
                number("Length", 2.0),
                number("Length2", 3.0),
            ],
            json!({ "Pocket": { "mode": "TwoLengths", "depth": 2.0, "depth2": 3.0 } }),
        ),
        (
            "PartDesign::Hole",
            vec![
                link("Profile", "S", ""),
                number("Diameter", 3.0),
                number("Depth", 5.0),
                number("DepthType", 0.0),
                number("DrillPoint", 1.0),
                number("DrillPointAngle", 118.0),
                property("HoleCutType", enums),
                number("HoleCutDiameter", 6.0),
                number("HoleCutDepth", 3.0),
                number("HoleCutCountersinkAngle", 90.0),
                property("Threaded", "<Bool value=\"true\"/>"),
            ],
            json!({ "Hole": { "diameter": 3.0, "depth": 5.0, "cut": { "Counterbore": { "diameter": 6.0, "depth": 3.0 } }, "drill_point": { "Angled": { "angle_deg": 118.0 } } } }),
        ),
        (
            "PartDesign::AdditiveLoft",
            vec![
                link("Profile", "S", ""),
                property(
                    "Sections",
                    "<LinkSubList><Link obj=\"T\" sub=\"\"/></LinkSubList>",
                ),
            ],
            json!({ "Loft": { "subtractive": false, "ruled": false } }),
        ),
        (
            "PartDesign::Mirrored",
            vec![
                link("MirrorPlane", "Plane", ""),
                property("Originals", "<LinkList/>"),
            ],
            json!({ "Mirrored": { "plane": "XY", "originals": [] } }),
        ),
    ];
    for (kind, props, expected) in cases {
        let (feature, findings) = converted(kind, &props);
        let feature = feature.unwrap_or_else(|| panic!("{kind}: {findings:?}"));
        assert!(has(&feature, &expected), "{kind}: {feature}");
        assert_eq!(
            findings.iter().any(|f| f.contains("thread")),
            kind.ends_with("Hole")
        );
    }
}

fn list(name: &str, items: &[&str]) -> String {
    let links: String = items
        .iter()
        .map(|i| format!("<Link value=\"{i}\"/>"))
        .collect();
    property(name, &format!("<LinkList>{links}</LinkList>"))
}

/// A source body's features make one body, in their order and to its tip,
/// its patterns repeating the features they chose there; a body started
/// from a solid with no counterpart starts from that solid's saved shape.
#[test]
fn a_body_is_one_history_its_patterns_repeat_chosen_features_and_a_frozen_solid_is_kept() {
    let extrude = |profile: &str, length: f64, base: &str| {
        [
            link("Profile", profile, ""),
            link("BaseFeature", base, ""),
            property("Reversed", "<Bool value=\"false\"/>"),
            number("Type", 0.0),
            number("Length", length),
        ]
        .concat()
    };
    let at = |z: f64| {
        property(
            "Placement",
            &format!(
                "<PropertyPlacement Px=\"0\" Py=\"0\" Pz=\"{z}\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"1\"/>"
            ),
        )
    };
    let (archive, _) = super::fixture("placement_frames");
    let frozen = archive.shape("A.Shape.brp").unwrap().to_string();
    let group = ["S", "Pad", "T", "Hole", "Row", "Ring"];
    let objects = [
        (
            "Body".into(),
            "PartDesign::Body",
            [at(0.0), list("Group", &group), link("Tip", "Ring", "")].concat(),
        ),
        sketch("S", 0.0),
        ("Pad".into(), "PartDesign::Pad", extrude("S", 4.0, "")),
        circle("T", 4.0, 2.0, 0.5),
        (
            "Hole".into(),
            "PartDesign::Pocket",
            extrude("T", 1.0, "Pad"),
        ),
        (
            "Row".into(),
            "PartDesign::LinearPattern",
            [
                list("Originals", &["Hole"]),
                link("BaseFeature", "Hole", ""),
                link("Direction", "S", "V_Axis"),
                number("Length", 2.0),
                number("Occurrences", 2.0),
            ]
            .concat(),
        ),
        (
            "Ring".into(),
            "PartDesign::MultiTransform",
            [
                list("Originals", &["Hole"]),
                link("BaseFeature", "Row", ""),
                list("Transformations", &["Spin"]),
            ]
            .concat(),
        ),
        (
            "Spin".into(),
            "PartDesign::PolarPattern",
            [
                link("Axis", "S", "N_Axis"),
                number("Angle", 360.0),
                number("Occurrences", 4.0),
            ]
            .concat(),
        ),
        (
            "Frozen".into(),
            "Part::Feature",
            property("Shape", "<Part file=\"Frozen.brp\"/>"),
        ),
        (
            "Based".into(),
            "PartDesign::Body",
            [
                at(0.0),
                list("Group", &["Start", "U", "Top"]),
                link("Tip", "Top", ""),
            ]
            .concat(),
        ),
        (
            "Start".into(),
            "PartDesign::FeatureBase",
            link("BaseFeature", "Frozen", ""),
        ),
        circle("U", 10.0, 5.0, 2.0),
        ("Top".into(), "PartDesign::Pad", extrude("U", 3.0, "Start")),
    ];
    let source = FcDocument::parse(&xml(&objects)).unwrap();
    let shapes = [("Frozen.brp".to_string(), frozen)].into();
    let project =
        project_tools::document_import::import_document(&source, &shapes, "body").unwrap();
    assert!(
        project.build_failures.is_empty(),
        "{:?}",
        project.build_failures
    );
    let body = |name: &str| project.bodies.iter().find(|b| b.name == name).unwrap();
    let main = body("Body");
    assert_eq!(main.features, ["Pad", "Hole", "Row", "Ring"]);
    let feature = |name: &str| match project.objects[name] {
        ImportedObject::Solid {
            feature: Some(f), ..
        } => f,
        _ => panic!("{name}"),
    };
    let data = |name: &str| {
        project
            .document
            .get_feature_data(feature(name))
            .unwrap()
            .clone()
    };
    let tip = project
        .document
        .bodies()
        .iter()
        .find(|b| b.id == main.body)
        .unwrap()
        .tip;
    assert_eq!(tip, Some(feature("Ring")));
    let hole = serde_json::to_value(feature("Hole")).unwrap();
    assert!(has(
        &data("Row"),
        &json!({ "LinearPattern": { "originals": [hole], "length": 2.0, "axis": { "Sketch": { "axis": "Vertical" } } } })
    ));
    let ring = &data("Ring")["MultiTransform"];
    assert_eq!(ring["originals"], json!([hole]));
    let spin =
        json!({ "angle_deg": 360.0, "occurrences": 4, "axis": { "Sketch": { "axis": "Normal" } } });
    assert!(has(&ring["steps"][0]["Polar"], &spin), "{ring}");
    // The disc less five bores: the original, its copy along the row, and
    // three more round the ring.
    let volume = |name: &str| {
        let blob = project
            .document
            .imported_brep_blob(body(name).body)
            .unwrap();
        let mut kernel = kernel_ogeom::OgeomKernel::new();
        kernel
            .physical_properties(blob)
            .unwrap()
            .volume_mm3
            .unwrap()
    };
    let pi = std::f64::consts::PI;
    assert!((volume("Body") - (100.0 - 5.0 * 0.25) * pi).abs() < 1e-3);
    // The frozen solid comes in as its saved shape, the body based on it
    // as a clone of that and the pad on it.
    assert_eq!(
        body("Frozen").kind,
        project_tools::document_import::BodyKind::SavedShape
    );
    assert_eq!(body("Based").features, ["Start", "Top"]);
    assert!((volume("Based") - (1000.0 + 12.0 * pi)).abs() < 1e-3);
}
