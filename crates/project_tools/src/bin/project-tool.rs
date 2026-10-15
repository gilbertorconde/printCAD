use project_archive::archive::FcstdArchive;
use project_archive::document_xml::FcDocument;
use project_tools::document_import::{import_document, panic_message};
use project_tools::error::{ErrorCategory, Failure};
use project_tools::parity::check_stationary;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let usage = "usage: project-tool inspect <file.FCStd> | convert <file.FCStd> <out.prtcad> | check <file-or-directory>...";
    let Some(command) = args.first().and_then(|s| s.to_str()) else {
        return Err(usage.into());
    };
    match command {
        "inspect" if args.len() == 2 => {
            let (archive, document) = read(Path::new(&args[1])).map_err(|e| e.to_string())?;
            print_json(
                &json!({ "version": document.program_version, "entries": archive.entries, "shapes": archive.shapes.keys().collect::<Vec<_>>(), "objects": document.objects.iter().map(|o| json!({ "name": o.name, "type": o.type_name, "properties": o.properties.keys().collect::<Vec<_>>() })).collect::<Vec<_>>() }),
            )?;
        }
        "convert" if args.len() == 3 => {
            let path = Path::new(&args[1]);
            let (_, source) = read(path).map_err(|e| e.to_string())?;
            let mut project = import_document(
                &source,
                &path.file_stem().unwrap_or_default().to_string_lossy(),
            )
            .map_err(|e| Failure::from(e).to_string())?;
            if !project.build_failures.is_empty() {
                print_json(
                    &json!({ "findings": project.findings, "build_failures": project.build_failures }),
                )?;
                return Err("conversion has rebuild failures; output was not written".into());
            }
            project
                .document
                .save_to_file(Path::new(&args[2]), core_document::Compression::None)
                .map_err(|e| e.to_string())?;
            print_json(
                &json!({ "objects": project.objects, "findings": project.findings, "output": args[2].to_string_lossy() }),
            )?;
        }
        "check" if args.len() >= 2 => {
            if args.iter().skip(1).any(|a| a == "--shapes") {
                return Err("--shapes is unavailable; check validates sketch stationarity, shape comparison requires an external tool".into());
            }
            let mut paths = Vec::new();
            for path in args.iter().skip(1) {
                collect(Path::new(path), &mut paths)?;
            }
            paths.sort();
            paths.dedup();
            if paths.is_empty() {
                return Err("no .FCStd files found".into());
            }
            let mut reports = Vec::new();
            let mut failures = 0;
            let mut categories: BTreeMap<String, usize> = BTreeMap::new();
            let mut error_categories: BTreeMap<ErrorCategory, usize> = BTreeMap::new();
            let mut sketch_count = 0;
            let mut solid_count = 0;
            for path in paths {
                let result = std::panic::catch_unwind(|| check_file(&path));
                let result = result.unwrap_or_else(|panic| {
                    Err(Failure::new(ErrorCategory::Panic, panic_message(&panic)))
                });
                match result {
                    Ok((report, count)) => {
                        failures += count;
                        sketch_count += report["sketches"].as_object().expect("sketch map").len();
                        solid_count +=
                            report["solid_count"].as_u64().expect("solid count") as usize;
                        for finding in report["findings"].as_array().expect("finding list") {
                            *categories
                                .entry(finding["category"].as_str().expect("category").into())
                                .or_default() += 1;
                        }
                        for (category, count) in report["error_categories"]
                            .as_object()
                            .expect("error categories")
                        {
                            let category = serde_json::from_value(json!(category))
                                .expect("serialized error category");
                            *error_categories.entry(category).or_default() +=
                                count.as_u64().expect("error count") as usize;
                        }
                        reports.push(report);
                    }
                    Err(error) => {
                        failures += 1;
                        *error_categories.entry(error.category).or_default() += 1;
                        reports.push(json!({ "file": path, "error": error.reason, "error_category": error.category }));
                    }
                }
            }
            print_json(
                &json!({ "summary": { "files": reports.len(), "sketches_checked": sketch_count, "solids_imported": solid_count, "shape_comparison": "not_performed", "failures": failures, "loss_categories": categories, "error_categories": error_categories }, "files": reports }),
            )?;
            if failures > 0 {
                return Err(format!("{failures} parity/read/rebuild failure(s)"));
            }
        }
        _ => return Err(usage.into()),
    }
    Ok(())
}

fn read(path: &Path) -> Result<(FcstdArchive, FcDocument), Failure> {
    let archive = FcstdArchive::open(path)
        .map_err(|e| Failure::new(ErrorCategory::Archive, format!("{}: {e}", path.display())))?;
    let document = FcDocument::parse(&archive.document_xml)
        .map_err(|e| Failure::new(ErrorCategory::Parse, format!("{}: {e}", path.display())))?;
    Ok((archive, document))
}

fn check_file(path: &Path) -> Result<(Value, usize), Failure> {
    let (_, source) = read(path)?;
    let project = import_document(
        &source,
        &path.file_stem().unwrap_or_default().to_string_lossy(),
    )
    .map_err(Failure::from)?;
    let mut failures = project.build_failures.len();
    let mut sketches = BTreeMap::new();
    for (name, sketch) in &project.sketches {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check_stationary(sketch)));
        let value = match result {
            Ok(Ok(check)) => {
                if !check.holds() {
                    failures += 1;
                }
                json!({ "holds": check.holds(), "measurement": check, "import_complete": sketch.findings.is_empty() })
            }
            Ok(Err(error)) => {
                failures += 1;
                json!({ "error": error.reason, "error_category": error.category })
            }
            Err(panic) => {
                failures += 1;
                json!({ "error": panic_message(&panic) })
            }
        };
        sketches.insert(name, value);
    }
    let mut solid_count = 0;
    for (name, imported) in &project.objects {
        let project_tools::document_import::ImportedObject::Solid(body) = imported else {
            continue;
        };
        // Group aliases point to the same result; count the actual solid object once.
        if source.object(name).expect("source exists").type_name == "PartDesign::Body" {
            continue;
        }
        solid_count += 1;
        if !project.build_failures.contains_key(name)
            && project.document.imported_brep_blob(*body).is_none()
        {
            return Err(Failure::new(
                ErrorCategory::Rebuild,
                format!("{name}: rebuilt body has no snapshot"),
            ));
        }
    }
    Ok((
        json!({ "file": path, "objects": project.objects.len(), "solid_count": solid_count, "sketches": sketches, "build_failures": project.build_failures, "findings": project.findings, "error_categories": { "rebuild": project.build_failures.len(), "stationarity": failures - project.build_failures.len() } }),
        failures,
    ))
}

fn collect(path: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
            collect(&entry.map_err(|e| e.to_string())?.path(), paths)?;
        }
    } else if metadata.is_file()
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fcstd"))
    {
        paths.push(path.into());
    }
    Ok(())
}

fn print_json(value: &Value) -> Result<(), String> {
    use std::io::Write;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, value).map_err(|e| e.to_string())?;
    writeln!(output).map_err(|e| e.to_string())
}
