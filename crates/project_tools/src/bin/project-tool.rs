use kernel_ogeom::OgeomKernel;
use project_archive::archive::FcstdArchive;
use project_archive::coordinates::{Point3, Rotation};
use project_archive::document_xml::FcDocument;
use project_archive::document_xml::Placement;
use project_tools::document_import::{BodyKind, import_document, panic_message};
use project_tools::error::{ErrorCategory, Failure};
use project_tools::parity::{check_stationary, compare_shape};
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
            let (archive, source) = read(path).map_err(|e| e.to_string())?;
            let mut project = import_document(
                &source,
                &archive.shapes,
                &path.file_stem().unwrap_or_default().to_string_lossy(),
            )
            .map_err(|e| Failure::from(e).to_string())?;
            // A body that does not build and has no saved shape to show
            // would be missing from the document.
            if project
                .build_failures
                .keys()
                .any(|name| !project.stood_in.contains(name))
            {
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
                &json!({ "objects": project.objects, "bodies": project.bodies, "findings": project.findings, "output": args[2].to_string_lossy() }),
            )?;
        }
        "check" if args.len() >= 2 => {
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
            let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
            for path in paths {
                let result = std::panic::catch_unwind(|| check_file(&path));
                let result = result.unwrap_or_else(|panic| {
                    Err(Failure::new(ErrorCategory::Panic, panic_message(&panic)))
                });
                match result {
                    Ok((report, count)) => {
                        failures += count;
                        sketch_count += report["sketches"].as_object().expect("sketch map").len();
                        for body in report["bodies"].as_array().expect("body list") {
                            *totals.entry("bodies").or_default() += 1;
                            let kind = match body["kind"].as_str().expect("a kind") {
                                "features" => "built_from_features",
                                "saved_shape" => "saved_shapes",
                                _ => "linked_copies",
                            };
                            *totals.entry(kind).or_default() += 1;
                            *totals.entry("features").or_default() +=
                                body["features"].as_u64().unwrap_or(0) as usize;
                            if body["stood_in"] == true {
                                *totals.entry("stood_in").or_default() += 1;
                            }
                            let matched = if body["bounds_match"] == true {
                                "bounds_matched"
                            } else {
                                "bounds_differing"
                            };
                            *totals.entry(matched).or_default() += 1;
                        }
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
            let mut summary = json!({ "files": reports.len(), "sketches_checked": sketch_count, "failures": failures, "loss_categories": categories, "error_categories": error_categories });
            for key in [
                "bodies",
                "built_from_features",
                "saved_shapes",
                "linked_copies",
                "features",
                "stood_in",
                "bounds_matched",
                "bounds_differing",
            ] {
                summary[key] = totals.get(key).copied().unwrap_or(0).into();
            }
            print_json(&json!({ "summary": summary, "files": reports }))?;
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
    let (archive, source) = read(path)?;
    let project = import_document(
        &source,
        &archive.shapes,
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
    let parents = project_archive::traversal::ParentMap::build(&source)
        .map_err(|e| Failure::new(ErrorCategory::Conversion, e.to_string()))?;
    let mut bodies = Vec::new();
    let mut kernel = OgeomKernel::new();
    for body in &project.bodies {
        let failure = project.build_failures.get(&body.shape_of);
        let mut entry = json!({ "name": body.name, "kind": body.kind, "features": body.features.len(), "shape_of": body.shape_of, "visible": body.visible, "stood_in": project.stood_in.contains(&body.shape_of), "bounds_match": false });
        if let Some(failure) = failure {
            entry["failure"] = json!(failure.reason);
        }
        // The saved shape is the oracle a body is held to: its tip's,
        // moved as a linked copy is.
        let saved = source
            .object(&body.shape_of)
            .and_then(|object| object.shape_file("Shape").ok())
            .and_then(|file| archive.shape(file).ok());
        let geometry = project.document.imported_geometry(body.body);
        if let (Some(geometry), Some(saved)) = (geometry, saved) {
            let frame = parents
                .placement(&source, &body.shape_of)
                .map_err(|e| Failure::new(ErrorCategory::Conversion, e.to_string()))?;
            let frame = match body.kind {
                BodyKind::LinkedCopy => {
                    let moved = project.document.body_placement(body.body);
                    let [x, y, z, w] = moved.rotation.map(f64::from);
                    Placement {
                        position: Point3::from_array(moved.translation.map(f64::from)),
                        rotation: Rotation([x, y, z, w]),
                    }
                    .compose(frame, &body.shape_of)
                    .map_err(|e| Failure::new(ErrorCategory::Conversion, e.to_string()))?
                }
                _ => frame,
            };
            let built_faces = geometry.mesh.face_surfaces.len();
            let bounds = geometry.bounds_mm;
            match std::panic::catch_unwind(|| compare_shape(saved, frame, bounds, built_faces)) {
                Ok(Ok(parity)) => {
                    entry["bounds_match"] = parity.bounds_match().into();
                    entry["deviation_mm"] = json!(parity.deviation_mm());
                    entry["faces_match"] = (parity.saved_faces == parity.built_faces).into();
                    entry["measurement"] = json!(parity);
                }
                Ok(Err(error)) => entry["error"] = json!(error.reason),
                Err(panic) => entry["error"] = json!(panic_message(&panic)),
            }
            // Volumes, the built solid's and the saved one's.
            let volume = |blob: &[u8], kernel: &mut OgeomKernel| {
                kernel
                    .physical_properties(blob)
                    .ok()
                    .and_then(|p| p.volume_mm3)
            };
            if let Some(blob) = project.document.imported_brep_blob(body.body) {
                entry["volume_mm3"] = json!(volume(blob, &mut kernel));
            }
            let read = project_archive::restate::restate(saved, frame)
                .ok()
                .and_then(|text| kernel.read_brep(&text, &Default::default()).ok());
            if let Some(read) = read {
                entry["saved_volume_mm3"] = json!(volume(&read.brep_blob, &mut kernel));
            }
        }
        bodies.push(entry);
    }
    Ok((
        json!({ "file": path, "objects": project.objects.len(), "bodies": bodies, "sketches": sketches, "build_failures": project.build_failures, "findings": project.findings, "error_categories": { "rebuild": project.build_failures.len(), "stationarity": failures - project.build_failures.len() } }),
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
