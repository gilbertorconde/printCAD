use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("project-tools-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

// Broken containers add no geometric data; all healthy recipes and saved shapes
// remain those of the generated fixture. Real CLI conversion must keep them.
fn mixed_xml() -> String {
    let (archive, _) = super::fixture("body_features");
    let links = [
        ("MissingRoot", "Absent"),
        ("MissingConsumer", "MissingRoot"),
        ("CycleA", "CycleB"),
        ("CycleB", "CycleA"),
        ("CycleConsumer", "CycleA"),
    ];
    let objects = links
        .iter()
        .map(|(name, _)| format!("<Object name=\"{name}\" type=\"PartDesign::Body\"/>"))
        .collect::<String>();
    let data = links.iter().map(|(name, target)| format!("<Object name=\"{name}\"><Properties><Property name=\"Tip\" type=\"App::PropertyLink\"><Link value=\"{target}\"/></Property></Properties></Object>")).collect::<String>();
    archive
        .document_xml
        .replace("</Objects>", &format!("{objects}</Objects>"))
        .replace("</ObjectData>", &format!("{data}</ObjectData>"))
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../project_archive/tests/data")
        .join(format!("{name}.FCStd"))
}

#[test]
fn inspect_lists_every_object_of_the_archive() {
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .arg("inspect")
        .arg(fixture("body_features"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["objects"].as_array().unwrap().len(), 34);
}

#[test]
fn shape_checks_are_explicitly_refused() {
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .args(["check", "--shapes"])
        .arg(fixture("body_features"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--shapes is unavailable"));
    assert!(output.stdout.is_empty());
}

#[test]
fn generated_corpus_checks_sketches_without_claiming_shape_parity() {
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .arg("check")
        .args([
            fixture("body_features"),
            fixture("placement_frames"),
            fixture("sketch_constraints"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["files"], 3);
    assert_eq!(report["summary"]["solids_imported"], 12);
    assert!(report["summary"]["sketches_checked"].as_u64().unwrap() > 0);
    assert_eq!(report["summary"]["failures"], 0);
    assert_eq!(report["summary"]["shape_comparison"], "not_performed");
}

#[test]
fn reader_budget_refusals_reach_the_cli_without_a_partial_output_file() {
    use super::resources::{archive, dependency_xml, nested_xml, underdeclare};
    let scratch = Scratch::new();
    let mut oversized = archive(&[("Document.xml", "")]);
    let limit = project_archive::archive::ArchiveLimits::default().xml.0;
    underdeclare(
        &mut oversized,
        "Document.xml",
        (limit + 1).try_into().unwrap(),
    );
    for (bytes, reason) in [
        (oversized, "Document.xml bytes"),
        (
            archive(&[("Document.xml", &nested_xml(20_000))]),
            "Document.xml element depth",
        ),
        (
            archive(&[("Document.xml", &dependency_xml(10_000, ""))]),
            "dependency depth exceeds",
        ),
    ] {
        let input = scratch.0.join("limited.FCStd");
        let output_path = scratch.0.join("limited.prtcad");
        std::fs::write(&input, bytes).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
            .arg("convert")
            .arg(&input)
            .arg(&output_path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(reason), "{error}");
        assert!(!output_path.exists());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn precise_sketch_numbers_survive_cli_conversion_and_check() {
    use super::numbers::{POINT, sketch_xml};
    let scratch = Scratch::new();
    for value in [100.000003, 1e8 + 1.0] {
        let geometry = POINT.replace("X=\"1\"", &format!("X=\"{value}\""));
        let xml = sketch_xml(&geometry, "", [value, -value, value, 0.0, 0.0, 0.0, 1.0]);
        let input = scratch.0.join("precise.FCStd");
        let destination = scratch.0.join("precise.prtcad");
        std::fs::write(&input, super::resources::archive(&[("Document.xml", &xml)])).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
            .arg("convert")
            .arg(&input)
            .arg(&destination)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let id: core_document::FeatureId =
            serde_json::from_value(report["objects"]["Profile"]["Sketch"].clone()).unwrap();
        let loaded = core_document::Document::load_from_file(&destination).unwrap();
        let feature: wb_sketch::SketchFeature =
            serde_json::from_value(loaded.get_feature_data(id).unwrap().clone()).unwrap();
        assert_eq!(feature.sketch.plane.origin, [value, -value, value]);
        assert!(feature.sketch.geometry.iter().any(
            |g| matches!(g, wb_sketch::sketch::GeometryElement::Point(p) if p.position.x == value)
        ));
        let checked = Command::new(env!("CARGO_BIN_EXE_project-tool"))
            .arg("check")
            .arg(&input)
            .output()
            .unwrap();
        assert!(checked.status.success());
        let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
        assert_eq!(
            report["files"][0]["sketches"]["Profile"]["measurement"]["rounding_mm"],
            0.0
        );
    }
}

#[test]
fn invalid_solver_semantics_have_a_named_cli_error_and_keep_other_measurements() {
    let scratch = Scratch::new();
    let geometry = format!(
        "{}{}<Geometry type=\"Part::GeomCircle\"><Circle CenterX=\"0\" CenterY=\"0\" Radius=\"10\" NormalZ=\"1\"/><Construction value=\"0\"/></Geometry>",
        super::numbers::POINT,
        super::numbers::POINT
    );
    // Symmetry about a line cannot name a circle as that line.
    let constraint = "<Constrain Type=\"14\" Value=\"0\" First=\"0\" FirstPos=\"1\" Second=\"1\" SecondPos=\"1\" Third=\"2\" ThirdPos=\"0\" Name=\"invalid symmetry\"/>";
    let xml =
        super::numbers::sketch_xml(&geometry, constraint, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    let input = scratch.0.join("invalid.FCStd");
    std::fs::write(&input, super::resources::archive(&[("Document.xml", &xml)])).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .arg("check")
        .arg(&input)
        .arg(fixture("sketch_constraints"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["files"], 2);
    assert_eq!(report["summary"]["sketches_checked"], 2);
    assert_eq!(report["summary"]["failures"], 1);
    assert_eq!(report["summary"]["error_categories"]["stationarity"], 1);
    assert!(report["summary"]["error_categories"].get("panic").is_none());
    let invalid = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file"].as_str() == input.to_str())
        .unwrap();
    assert_eq!(
        invalid["sketches"]["Profile"]["error_category"],
        "stationarity"
    );
    assert!(
        invalid["sketches"]["Profile"]["error"]
            .as_str()
            .unwrap()
            .contains("InvalidRelation")
    );
    assert_eq!(
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["sketches"]["Sketch"]["holds"] == true)
            .count(),
        1
    );
}

#[test]
fn mixed_document_converts_and_checks_healthy_solids_with_explicit_dependency_losses() {
    let scratch = Scratch::new();
    let input = scratch.0.join("mixed.FCStd");
    let destination = scratch.0.join("mixed.prtcad");
    std::fs::write(
        &input,
        super::resources::archive(&[("Document.xml", &mixed_xml())]),
    )
    .unwrap();
    for command in ["convert", "check"] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_project-tool"));
        process.arg(command).arg(&input);
        if command == "convert" {
            process.arg(&destination);
        }
        let output = process.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if command == "convert" {
            let loaded = core_document::Document::load_from_file(&destination).unwrap();
            assert_eq!(loaded.bodies().len(), 9);
            for name in ["Pad", "Disc", "PartBox", "Cut"] {
                let body: core_document::BodyId =
                    serde_json::from_value(report["objects"][name]["Solid"].clone()).unwrap();
                assert!(loaded.imported_brep_blob(body).is_some(), "{name}");
            }
        }
        let file = if command == "check" {
            assert_eq!(report["summary"]["files"], 1);
            assert_eq!(report["summary"]["solids_imported"], 9);
            assert_eq!(report["summary"]["failures"], 0);
            &report["files"][0]
        } else {
            &report
        };
        for name in [
            "MissingRoot",
            "MissingConsumer",
            "CycleA",
            "CycleB",
            "CycleConsumer",
        ] {
            let findings: Vec<_> = file["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["object"] == name)
                .collect();
            assert_eq!(findings.len(), 1, "{name}: {findings:?}");
            assert_eq!(findings[0]["category"], "dependency");
        }
    }
}

#[test]
fn check_distinguishes_archive_parse_conversion_rebuild_and_stationarity_failures() {
    let scratch = Scratch::new();
    // A primitive with zero length reaches the actual kernel's validation.
    let bad_box = r#"<Document><Objects><Object name="Box" type="Part::Box"/></Objects><ObjectData><Object name="Box"><Properties>
        <Property name="Placement" type="App::PropertyPlacement"><PropertyPlacement Px="0" Py="0" Pz="0" Q0="0" Q1="0" Q2="0" Q3="1"/></Property>
        <Property name="Length" type="App::PropertyLength"><Float value="0"/></Property>
        <Property name="Width" type="App::PropertyLength"><Float value="2"/></Property>
        <Property name="Height" type="App::PropertyLength"><Float value="3"/></Property>
        </Properties></Object></ObjectData></Document>"#;
    let constraint = "<Constrain Type=\"7\" Value=\"12\" First=\"0\" FirstPos=\"1\" Second=\"-2000\" SecondPos=\"0\" Third=\"-2000\" ThirdPos=\"0\" Name=\"X\"/>";
    let identity = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    for (name, bytes) in [
        ("archive", b"invalid zip".to_vec()),
        (
            "parse",
            super::resources::archive(&[("Document.xml", "<Document>")]),
        ),
        (
            "conversion",
            super::resources::archive(&[(
                "Document.xml",
                &super::numbers::sketch_xml(super::numbers::POINT, "", [0.0; 7]),
            )]),
        ),
        (
            "rebuild",
            super::resources::archive(&[("Document.xml", bad_box)]),
        ),
        (
            "stationarity",
            super::resources::archive(&[(
                "Document.xml",
                &super::numbers::sketch_xml(super::numbers::POINT, constraint, identity),
            )]),
        ),
    ] {
        std::fs::write(scratch.0.join(format!("{name}.FCStd")), bytes).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .arg("check")
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["files"], 5);
    for category in ["archive", "parse", "conversion", "rebuild", "stationarity"] {
        assert!(
            report["summary"]["error_categories"][category]
                .as_u64()
                .unwrap()
                > 0,
            "{report}"
        );
    }
    let destination = scratch.0.join("rebuild.prtcad");
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .arg("convert")
        .arg(scratch.0.join("rebuild.FCStd"))
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!destination.exists());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["build_failures"]["Box"]["category"], "rebuild");
    assert!(
        report["build_failures"]["Box"]["reason"]
            .as_str()
            .unwrap()
            .contains("kernel")
    );
}
