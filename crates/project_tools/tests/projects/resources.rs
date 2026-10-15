use std::io::{Cursor, Write};

use project_archive::archive::{ArchiveLimits, Bytes, FcstdArchive, FcstdError};
use project_archive::document_xml::{FcDocument, MAX_XML_DEPTH, MAX_XML_NODES};
use project_archive::traversal::{MAX_TRAVERSAL_DEPTH, parent_placement};
use project_tools::document_import::import_document;

pub(super) fn archive(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in entries {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn limit_error(error: FcstdError, resource: &str, limit: u64, observed: u64) {
    assert_eq!(
        error,
        FcstdError::Limit {
            resource: resource.into(),
            limit,
            observed
        }
    );
}

// The central directory's uncompressed-size field is independent of deflate data.
// A forged small size must not let the decompressor exceed the actual byte budget.
pub(super) fn underdeclare(bytes: &mut [u8], name: &str, size: u32) {
    let offset = bytes
        .windows(4)
        .enumerate()
        .find_map(|(offset, magic)| {
            if magic != b"PK\x01\x02" {
                return None;
            }
            let length =
                u16::from_le_bytes(bytes[offset + 28..offset + 30].try_into().unwrap()) as usize;
            (bytes.get(offset + 46..offset + 46 + length) == Some(name.as_bytes()))
                .then_some(offset)
        })
        .unwrap();
    bytes[offset + 24..offset + 28].copy_from_slice(&size.to_le_bytes());
}

#[test]
fn declared_xml_and_shape_budgets_are_inclusive() {
    for (name, resource) in [
        ("Document.xml", "Document.xml bytes"),
        ("shape.brp", "entry shape.brp bytes"),
    ] {
        let bytes = if name == "Document.xml" {
            archive(&[(name, "12345678")])
        } else {
            archive(&[("Document.xml", ""), (name, "12345678")])
        };
        for (budget, succeeds) in [(9, true), (8, true), (7, false)] {
            let mut limits = ArchiveLimits::default();
            if name == "Document.xml" {
                limits.xml = Bytes(budget);
            } else {
                limits.entry = Bytes(budget);
            }
            let result = FcstdArchive::from_bytes_with_limits(&bytes, limits);
            if succeeds {
                assert!(result.is_ok());
            } else {
                limit_error(result.unwrap_err(), resource, 7, 8);
            }
        }
    }
}

#[test]
fn declared_total_includes_ignored_entries() {
    let bytes = archive(&[
        ("Document.xml", "1234"),
        ("shape.brp", "5678"),
        ("ignored.bin", "90"),
    ]);
    for (budget, succeeds) in [(11, true), (10, true), (9, false)] {
        let limits = ArchiveLimits {
            total: Bytes(budget),
            ..Default::default()
        };
        let result = FcstdArchive::from_bytes_with_limits(&bytes, limits);
        if succeeds {
            assert!(result.is_ok());
        } else {
            limit_error(result.unwrap_err(), "total declared bytes", 9, 10);
        }
    }
}

#[test]
fn actual_xml_and_shape_budgets_ignore_forged_sizes() {
    for (name, resource) in [
        ("Document.xml", "Document.xml bytes"),
        ("shape.brp", "entry shape.brp bytes"),
    ] {
        for actual in [7, 8, 9, 100_000] {
            let text = "a".repeat(actual);
            let mut bytes = if name == "Document.xml" {
                archive(&[(name, &text)])
            } else {
                archive(&[("Document.xml", ""), (name, &text)])
            };
            underdeclare(&mut bytes, name, 1);
            let mut limits = ArchiveLimits::default();
            if name == "Document.xml" {
                limits.xml = Bytes(8);
            } else {
                limits.entry = Bytes(8);
            }
            let result = FcstdArchive::from_bytes_with_limits(&bytes, limits);
            if actual <= 8 {
                let decoded = result.unwrap();
                assert_eq!(
                    if name == "Document.xml" {
                        &decoded.document_xml
                    } else {
                        decoded.shape(name).unwrap()
                    },
                    text
                );
            } else {
                // Reading stops on the first extra byte, even for highly compressed data.
                limit_error(result.unwrap_err(), resource, 8, 9);
            }
        }
    }
}

#[test]
fn actual_total_is_shared_across_xml_and_shapes() {
    for length in [3, 4, 5, 100_000] {
        let text = "b".repeat(length);
        let mut bytes = archive(&[("Document.xml", "1234"), ("shape.brp", &text)]);
        underdeclare(&mut bytes, "shape.brp", 1);
        let limits = ArchiveLimits {
            total: Bytes(8),
            ..Default::default()
        };
        let result = FcstdArchive::from_bytes_with_limits(&bytes, limits);
        if length <= 4 {
            assert!(result.is_ok());
        } else {
            limit_error(result.unwrap_err(), "total decompressed bytes", 8, 9);
        }
    }
}

#[test]
fn container_and_entry_count_budgets_apply_to_file_and_memory_inputs() {
    let bytes = archive(&[("Document.xml", ""), ("ignored.bin", "")]);
    let path = std::env::temp_dir().join(format!("project-budget-{}.FCStd", uuid::Uuid::new_v4()));
    std::fs::write(&path, &bytes).unwrap();
    for budget in [
        bytes.len() as u64 + 1,
        bytes.len() as u64,
        bytes.len() as u64 - 1,
    ] {
        let limits = ArchiveLimits {
            container: Bytes(budget),
            ..Default::default()
        };
        for result in [
            FcstdArchive::from_bytes_with_limits(&bytes, limits),
            FcstdArchive::open_with_limits(&path, limits),
        ] {
            if budget >= bytes.len() as u64 {
                assert!(result.is_ok());
            } else {
                limit_error(
                    result.unwrap_err(),
                    "container bytes",
                    budget,
                    bytes.len() as u64,
                );
            }
        }
    }
    std::fs::remove_file(path).unwrap();
    for budget in [3, 2, 1] {
        let result = FcstdArchive::from_bytes_with_limits(
            &bytes,
            ArchiveLimits {
                entries: budget,
                ..Default::default()
            },
        );
        if budget >= 2 {
            assert!(result.is_ok());
        } else {
            limit_error(result.unwrap_err(), "archive entries", 1, 2);
        }
    }
}

#[test]
fn default_budgets_reject_forged_large_entry_metadata_before_reading() {
    for (name, size, resource, limit) in [
        (
            "Document.xml",
            ArchiveLimits::default().xml.0 + 1,
            "Document.xml bytes",
            ArchiveLimits::default().xml.0,
        ),
        (
            "ignored.bin",
            ArchiveLimits::default().entry.0 + 1,
            "entry ignored.bin bytes",
            ArchiveLimits::default().entry.0,
        ),
    ] {
        let mut bytes = archive(&[("Document.xml", ""), ("ignored.bin", "")]);
        underdeclare(&mut bytes, name, size.try_into().unwrap());
        limit_error(
            FcstdArchive::from_bytes(&bytes).unwrap_err(),
            resource,
            limit,
            size,
        );
    }
}

pub(super) fn nested_xml(depth: usize) -> String {
    format!(
        "<Document><Objects/><ObjectData/>{}{}</Document>",
        "<x>".repeat(depth - 1),
        "</x>".repeat(depth - 1)
    )
}

#[test]
fn xml_depth_is_checked_before_copying_or_dropping_owned_trees() {
    for depth in [MAX_XML_DEPTH - 1, MAX_XML_DEPTH, MAX_XML_DEPTH + 1, 20_000] {
        let bytes = archive(&[("Document.xml", &nested_xml(depth))]);
        let decoded = FcstdArchive::from_bytes(&bytes).unwrap();
        let result = FcDocument::parse(&decoded.document_xml);
        if depth <= MAX_XML_DEPTH {
            assert!(result.is_ok());
        } else {
            limit_error(
                result.unwrap_err(),
                "Document.xml element depth",
                MAX_XML_DEPTH as u64,
                MAX_XML_DEPTH as u64 + 1,
            );
        }
    }
}

#[test]
fn xml_preflight_handles_markup_inside_comments_cdata_attributes_and_instructions() {
    let markup = "<x>".repeat(MAX_XML_DEPTH * 2);
    let xml = format!(
        "<?probe {markup} ?><Document><!-- {markup} --><Objects/><ObjectData/><Value attr=\"&lt;x&gt;\"><![CDATA[{markup}]]></Value></Document>"
    );
    assert!(FcDocument::parse(&xml).is_ok());
    // The deepest tree is a property value, exercising recursive copy, clone and drop.
    let xml = format!(
        "<Document><Objects><Object name=\"O\" type=\"Unknown\"/></Objects><ObjectData><Object name=\"O\"><Properties><Property name=\"P\" type=\"Unknown\">{}<x/>{}</Property></Properties></Object></ObjectData></Document>",
        "<x>".repeat(MAX_XML_DEPTH - 6),
        "</x>".repeat(MAX_XML_DEPTH - 6)
    );
    let source = FcDocument::parse(&xml).unwrap();
    assert_eq!(source.clone(), source);
    let too_deep = xml.replace("<x/>", "<x><x/></x>");
    limit_error(
        FcDocument::parse(&too_deep).unwrap_err(),
        "Document.xml element depth",
        MAX_XML_DEPTH as u64,
        MAX_XML_DEPTH as u64 + 1,
    );
    assert!(FcDocument::parse("<!DOCTYPE Document [<!ENTITY nested '<x/>'>]><Document><Objects/><ObjectData/>&nested;</Document>").unwrap_err().to_string().contains("DTD"));
    assert!(FcDocument::parse("<Document><Objects/></wrong>").is_err());
}

#[test]
fn direct_xml_input_has_byte_and_node_budgets() {
    let oversized = " ".repeat(ArchiveLimits::default().xml.0 as usize + 1);
    limit_error(
        FcDocument::parse(&oversized).unwrap_err(),
        "Document.xml bytes",
        ArchiveLimits::default().xml.0,
        oversized.len() as u64,
    );
    // Document, its root and the two required sections also count as parser nodes.
    let xml = format!(
        "<Document><Objects/><ObjectData/>{}</Document>",
        "<x/>".repeat(MAX_XML_NODES as usize)
    );
    let error = FcDocument::parse(&xml).unwrap_err();
    assert!(matches!(error, FcstdError::Xml(_)));
    assert!(error.to_string().contains("nodes limit"), "{error}");
    // Valid XML with very few nodes must not trigger oversized parser reservations.
    for punctuation in ['<', '='] {
        let xml = format!(
            "<Document><Objects/><ObjectData/><!-- {} --></Document>",
            punctuation.to_string().repeat(2_000_000)
        );
        limit_error(
            FcDocument::parse(&xml).unwrap_err(),
            "Document.xml allocation hints",
            2_000_000,
            2_000_005,
        );
    }
}

pub(super) fn dependency_xml(count: usize, tail: &str) -> String {
    let mut objects = String::new();
    let mut data = String::new();
    for index in 0..count {
        let target = if index + 1 == count {
            tail.into()
        } else {
            format!("N{}", index + 1)
        };
        objects.push_str(&format!(
            "<Object name=\"N{index}\" type=\"PartDesign::Body\"/>"
        ));
        data.push_str(&format!("<Object name=\"N{index}\"><Properties><Property name=\"Tip\" type=\"App::PropertyLink\"><Link value=\"{target}\"/></Property></Properties></Object>"));
    }
    format!("<Document><Objects>{objects}</Objects><ObjectData>{data}</ObjectData></Document>")
}

#[test]
fn dependency_walk_has_an_inclusive_pending_depth_budget() {
    for count in [
        MAX_TRAVERSAL_DEPTH - 1,
        MAX_TRAVERSAL_DEPTH,
        MAX_TRAVERSAL_DEPTH + 1,
        10_000,
    ] {
        let bytes = archive(&[("Document.xml", &dependency_xml(count, ""))]);
        let decoded = FcstdArchive::from_bytes(&bytes).unwrap();
        let source = FcDocument::parse(&decoded.document_xml).unwrap();
        let result = import_document(&source, "depth");
        if count <= MAX_TRAVERSAL_DEPTH {
            let project = result.expect("chain at budget must import");
            assert_eq!(project.findings.len(), count);
            assert!(project.objects.is_empty());
        } else {
            let error = result
                .err()
                .expect("over-budget chain must fail")
                .to_string();
            assert!(error.contains("dependency depth exceeds 128"), "{error}");
            assert!(error.contains("N0 -> N1 -> N2"), "{error}");
            assert!(error.ends_with("N128"), "{error}");
        }
    }
}

#[test]
fn dependency_cycles_and_missing_targets_name_the_chain() {
    for (tail, reason, chain) in [
        ("N0", "dependency cycle", "N0 -> N1 -> N0"),
        ("Missing", "linked object missing", "N0 -> N1 -> Missing"),
    ] {
        let source = FcDocument::parse(&dependency_xml(2, tail)).unwrap();
        let project = import_document(&source, "invalid").unwrap();
        assert!(project.objects.is_empty());
        assert_eq!(project.findings.len(), 2);
        for finding in project.findings {
            assert_eq!(finding.category, "dependency");
            assert!(finding.reason.contains(reason), "{finding:?}");
            assert!(finding.reason.contains(chain), "{finding:?}");
        }
    }
}

#[test]
fn cached_rejections_do_not_expand_diagnostics_along_a_long_shallow_walk() {
    // Leaf-first resolution uses at most two pending objects. The originating
    // missing link must stay visible without copying every consumer into it.
    let mut source = FcDocument::parse(&dependency_xml(10_000, "Missing")).unwrap();
    source.objects.reverse();
    let project = import_document(&source, "cached").unwrap();
    assert!(project.objects.is_empty());
    assert_eq!(project.findings.len(), 10_000);
    for finding in &project.findings {
        assert_eq!(finding.category, "dependency");
        assert!(finding.reason.contains("Missing"));
        assert!(finding.reason.len() < 150, "{} bytes", finding.reason.len());
    }
}

fn parent_xml(count: usize, tail: &str) -> String {
    let mut objects = String::new();
    let mut data = String::new();
    for index in 0..count {
        let child = if index + 1 == count {
            tail.into()
        } else {
            format!("N{}", index + 1)
        };
        objects.push_str(&format!("<Object name=\"N{index}\" type=\"App::Part\"/>"));
        data.push_str(&format!("<Object name=\"N{index}\"><Properties><Property name=\"Group\" type=\"App::PropertyLinkList\"><LinkList><Link value=\"{child}\"/></LinkList></Property><Property name=\"Placement\" type=\"App::PropertyPlacement\"><PropertyPlacement Px=\"1\" Py=\"0\" Pz=\"0\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"1\"/></Property></Properties></Object>"));
    }
    format!("<Document><Objects>{objects}</Objects><ObjectData>{data}</ObjectData></Document>")
}

#[test]
fn parent_walk_has_the_same_depth_budget_and_cycle_context() {
    for count in [
        MAX_TRAVERSAL_DEPTH - 2,
        MAX_TRAVERSAL_DEPTH - 1,
        MAX_TRAVERSAL_DEPTH,
        10_000,
    ] {
        let source = FcDocument::parse(&parent_xml(count, "Leaf")).unwrap();
        let result = parent_placement(&source, "Leaf");
        if count < MAX_TRAVERSAL_DEPTH {
            assert_eq!(
                result.unwrap().position,
                project_archive::coordinates::Point3::new(count as f64, 0.0, 0.0)
            );
        } else {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("placement depth exceeds 128"), "{error}");
            assert!(error.contains("Leaf ->"), "{error}");
        }
    }
    let source = FcDocument::parse(&parent_xml(2, "N0")).unwrap();
    let error = parent_placement(&source, "N0").unwrap_err().to_string();
    assert!(error.contains("placement cycle: N0 -> N1 -> N0"), "{error}");
}
