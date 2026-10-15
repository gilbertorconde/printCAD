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
    let circle = "<Geometry type=\"Part::GeomCircle\"><Circle CenterX=\"0\" CenterY=\"0\" Radius=\"5\" NormalZ=\"1\"/><Construction value=\"0\"/></Geometry>";
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
    let mut xml = String::from("<Document><Objects>");
    for (name, kind, _) in &objects {
        xml += &format!("<Object name=\"{name}\" type=\"{kind}\"/>");
    }
    xml += "</Objects><ObjectData>";
    for (name, _, props) in &objects {
        xml += &format!("<Object name=\"{name}\"><Properties>{props}</Properties></Object>");
    }
    xml += "</ObjectData></Document>";
    let mut document = Document::new("features");
    let source = FcDocument::parse(&xml).unwrap();
    let shapes = [("Base.brp".to_string(), SAVED.to_string())].into();
    let out = convert_into(&source, &shapes, &mut document).unwrap();
    let findings = out.findings.iter().filter(|f| f.object == "F");
    let findings = findings.map(|f| f.reason.clone()).collect();
    let feature = out.objects.get("F").map(|&object| {
        let ImportedObject::Solid(body) = object else {
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
    // A mirror of chosen features has nothing here to repeat.
    let props = [
        link("MirrorPlane", "Plane", ""),
        property("Originals", "<LinkList><Link value=\"S\"/></LinkList>"),
    ];
    let (feature, findings) = converted("PartDesign::Mirrored", &props);
    assert!(feature.is_none() && findings.iter().any(|f| f.contains("chosen features")));
}
