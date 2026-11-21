use super::cli::Scratch;
use project_tools::corpus::report::CorpusReport;
use project_tools::corpus::{ArchiveStatus, DivergenceKind, Located, Verdict};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../project_archive/tests/data")
}

fn fixture_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_stem().is_some_and(|s| s != "drill_point"))
        .collect();
    paths.sort();
    paths
}

/// `name` with the extension the fixtures carry.
fn archive_name(name: &str) -> String {
    let extension = fixture_paths()[0].extension().unwrap().to_owned();
    format!("{name}.{}", extension.to_string_lossy())
}

/// The archive at `path` with `edit` applied to its document.
fn edited(path: &Path, edit: impl Fn(String) -> String) -> Vec<u8> {
    let mut source = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..source.len() {
        let mut entry = source.by_index(index).unwrap();
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        let name = entry.name().to_owned();
        if name == "Document.xml" {
            text = edit(text);
        }
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn run_corpus(folder: &Path, report: &Path, more: &[&str]) -> (Output, CorpusReport) {
    let output = Command::new(env!("CARGO_BIN_EXE_project-tool"))
        .arg("corpus")
        .arg(folder)
        .arg("--report")
        .arg(report)
        .args(more)
        .output()
        .unwrap();
    let text = std::fs::read_to_string(report).unwrap_or_else(|e| {
        panic!("{e}: {}", String::from_utf8_lossy(&output.stderr));
    });
    (output, serde_json::from_str(&text).unwrap())
}

// The generated fixtures, one archive whose first pad was lengthened after
// its shapes were saved, and one that is no archive at all: the broken one
// fails alone, the edited one is caught at the feature that changed, and
// the report reads back as the summary printed.
#[test]
fn a_corpus_run_reports_every_archive_and_where_a_body_first_strays() {
    let scratch = Scratch::new();
    let folder = scratch.0.join("corpus");
    std::fs::create_dir(&folder).unwrap();
    for path in fixture_paths() {
        std::fs::copy(&path, folder.join(path.file_name().unwrap())).unwrap();
    }
    std::fs::write(folder.join(archive_name("broken")), b"not an archive").unwrap();
    let body_features = fixtures().join(archive_name("body_features"));
    let longer = edited(&body_features, |xml| {
        let pad = xml.find("<Object name=\"Pad\"").unwrap();
        let length = pad + xml[pad..].find("<Property name=\"Length\"").unwrap();
        let value = length + xml[length..].find("10.0000000000000000").unwrap();
        format!("{}12{}", &xml[..value], &xml[value + 2..])
    });
    std::fs::write(folder.join(archive_name("longer_pad")), longer).unwrap();

    let report_path = scratch.0.join("report.json");
    let (output, report) = run_corpus(&folder, &report_path, &[]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success(), "{stdout}");
    assert_eq!(report.archives.len(), 5);
    assert_eq!(
        stdout.lines().rev().nth(1),
        Some(report.summary.line().as_str())
    );

    let archive = |stem: &str| {
        report
            .archives
            .iter()
            .find(|a| Path::new(&a.file).file_stem().unwrap() == stem)
            .unwrap()
    };
    let broken = archive("broken");
    assert_eq!(broken.status, ArchiveStatus::Failed);
    assert!(broken.error.as_deref().unwrap().contains("archive"));
    assert_eq!(report.summary.failed, 1);

    // Bodies kept as saved solids prove nothing, so an archive holding them
    // is never complete; neither is one whose sketches lost attachments.
    let frames = archive("placement_frames");
    assert_eq!(frames.status, ArchiveStatus::Partial);
    let verdicts: Vec<_> = frames.bodies.iter().map(|b| b.verdict).collect();
    assert_eq!(
        verdicts,
        [
            Verdict::Matches,
            Verdict::SavedShape,
            Verdict::Matches,
            Verdict::Matches,
            Verdict::SavedShape
        ]
    );
    assert!(
        frames
            .unsupported
            .iter()
            .all(|f| f.category == "saved shape")
    );
    assert!(
        frames
            .lost_parametric
            .iter()
            .any(|f| f.category == "attachment")
    );

    let body = |stem: &str, name: &str| {
        archive(stem)
            .bodies
            .iter()
            .find(|b| b.name == name)
            .unwrap()
            .clone()
    };
    for name in ["ShiftedBody", "Cut", "PartCylinder"] {
        let body = body("body_features", name);
        assert_eq!(body.verdict, Verdict::Matches, "{body:?}");
        let volume = |m: Option<project_tools::corpus::measure::Measures>| m.unwrap().volume_mm3;
        let (saved, built) = (volume(body.saved).unwrap(), volume(body.built).unwrap());
        assert!((saved - built).abs() <= 1e-9 * saved, "{saved} {built}");
    }
    // The main body's saved solid, its hole's bore a trimmed cylinder the
    // kernel measures approximately, matches the rebuilt one through the
    // hole step, its centre of mass included.
    let main = body("body_features", "Body");
    assert_eq!(main.verdict, Verdict::Matches, "{main:?}");
    let lengthened = body("longer_pad", "Body");
    assert_eq!(lengthened.verdict, Verdict::Differs);
    let divergence = lengthened.first_divergence.unwrap();
    assert_eq!(divergence.feature, "Pad");
    assert_eq!(divergence.kind, DivergenceKind::Differs);
    assert_eq!(divergence.located_by, Located::IntermediateShapes);
    let volume = divergence
        .differences
        .iter()
        .find(|d| d.measure == "volume")
        .unwrap();
    assert!(
        (volume.built[0] / volume.saved[0] - 1.2).abs() < 1e-3,
        "{volume:?}"
    );

    // The same run against itself regresses nothing; against a baseline in
    // which the lengthened body matched, it names that body and fails.
    let mut baseline = report.clone();
    let rerun = scratch.0.join("rerun.json");
    let (output, _) = run_corpus(
        &folder,
        &rerun,
        &["--baseline", report_path.to_str().unwrap()],
    );
    assert!(output.status.success());
    let index = baseline
        .archives
        .iter()
        .position(|a| a.file.ends_with(&archive_name("longer_pad")))
        .unwrap();
    for body in &mut baseline.archives[index].bodies {
        body.verdict = Verdict::Matches;
    }
    let baseline_path = scratch.0.join("baseline.json");
    std::fs::write(&baseline_path, serde_json::to_string(&baseline).unwrap()).unwrap();
    let (output, rerun) = run_corpus(
        &folder,
        &rerun,
        &["--baseline", baseline_path.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        rerun
            .regressions
            .iter()
            .all(|r| r.starts_with(&archive_name("longer_pad"))),
        "{:?}",
        rerun.regressions
    );
    assert!(
        rerun
            .regressions
            .iter()
            .any(|r| r.contains("Body matched before"))
    );
}

#[test]
fn an_archive_past_the_time_limit_is_stopped_and_reported() {
    let scratch = Scratch::new();
    let report_path = scratch.0.join("report.json");
    let (output, report) = run_corpus(
        &fixtures().join(archive_name("body_features")),
        &report_path,
        &["--time-limit", "0"],
    );
    assert!(output.status.success());
    assert_eq!(report.archives[0].status, ArchiveStatus::TimedOut);
    assert_eq!(report.summary.timed_out, 1);
}

#[cfg(feature = "heavy-tests")]
mod heavy {
    use super::*;

    /// The corpus run over a folder of real project archives kept outside
    /// the repository, named by `PRINTCAD_PROJECT_CORPUS`; with
    /// `PRINTCAD_PROJECT_CORPUS_BASELINE` naming an earlier run's report,
    /// any body that matched there and no longer does fails it. The report
    /// goes to `PRINTCAD_PROJECT_CORPUS_REPORT`, else the temporary folder.
    #[test]
    #[ignore = "local corpus: set PRINTCAD_PROJECT_CORPUS to a folder of project archives"]
    fn the_local_project_corpus_has_no_regressions() {
        let folder = std::env::var_os("PRINTCAD_PROJECT_CORPUS")
            .expect("PRINTCAD_PROJECT_CORPUS names a folder of project archives");
        let report_path = std::env::var_os("PRINTCAD_PROJECT_CORPUS_REPORT").map_or_else(
            || std::env::temp_dir().join("project-corpus-report.json"),
            PathBuf::from,
        );
        let mut more = Vec::new();
        if let Ok(baseline) = std::env::var("PRINTCAD_PROJECT_CORPUS_BASELINE") {
            more = vec!["--baseline".to_owned(), baseline];
        }
        let more: Vec<&str> = more.iter().map(String::as_str).collect();
        let (output, report) = run_corpus(Path::new(&folder), &report_path, &more);
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(!report.archives.is_empty());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
