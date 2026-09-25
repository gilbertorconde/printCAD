//! Real packages through the host: the Gear example and a package that
//! misbehaves on request, both built from `sdk/` for wasm32-wasip2.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use core_document::{Document, DocumentService, FeatureInfo, WorkbenchId, WorkbenchRuntimeContext};
use serde_json::{Value, json};
use wb_wasm::{Capabilities, Package};

/// The SDK workspace's release build for wasm32-wasip2, built once.
fn guests() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sdk");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = std::process::Command::new(cargo)
            .current_dir(&sdk)
            .args([
                "build",
                "--release",
                "--target",
                "wasm32-wasip2",
                "-p",
                "gear",
                "-p",
                "rogue",
            ])
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .status()
            .expect("run cargo for the SDK");
        assert!(
            status.success(),
            "the SDK's packages build for wasm32-wasip2"
        );
        sdk.join("target/wasm32-wasip2/release")
    })
}

/// A fresh folder under the system's temporary one.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "printcad-wasm-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Package folder `source` (under `sdk/`) with its component, packed,
/// installed under a fresh root.
fn installed(source: &str, component: &str, name: &str) -> Package {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sdk");
    let work = scratch(name);
    let folder = work.join("src");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::copy(
        sdk.join(source).join("bench.toml"),
        folder.join("bench.toml"),
    )
    .unwrap();
    std::fs::copy(guests().join(component), folder.join("bench.wasm")).unwrap();
    let icons = sdk.join(source).join("icons");
    if icons.is_dir() {
        std::fs::create_dir_all(folder.join("icons")).unwrap();
        for entry in std::fs::read_dir(icons).unwrap().flatten() {
            std::fs::copy(entry.path(), folder.join("icons").join(entry.file_name())).unwrap();
        }
    }
    let archive = work.join("package.pcbench");
    wb_wasm::pack(&folder, &archive).expect("packs");
    wb_wasm::install(&archive, &work.join("installed")).expect("installs")
}

fn registry_with(package: &Package, granted: Capabilities) -> DocumentService {
    let bench = wb_wasm::load(package, &granted).expect("loads");
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(bench))
        .expect("registers");
    registry
}

fn run(
    registry: &mut DocumentService,
    document: &mut Document,
    bench: &str,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    let mut ctx =
        WorkbenchRuntimeContext::new(document, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
    let args = args.as_object().cloned().unwrap_or_default();
    registry
        .workbench_mut(&WorkbenchId::new(bench))
        .unwrap()
        .run_command(command, &args, &mut ctx)
        .map_err(|e| e.to_string())
}

fn volume(ops: &[kernel_api::SolidOp]) -> f64 {
    let mut kernel = kernel_ogeom::OgeomKernel::new();
    let built = kernel
        .execute_solid_chain(ops, &kernel_api::TessellationSettings::default())
        .expect("the plan builds");
    kernel
        .physical_properties(&built.brep_blob)
        .expect("measures")
        .volume_mm3
        .expect("a closed solid")
}

#[test]
fn a_gear_package_installs_registers_and_builds_a_parametric_gear() {
    let package = installed("examples/gear", "gear.wasm", "gear");
    assert_eq!(package.manifest.id, "example.gear");
    let mut registry = registry_with(
        &package,
        Capabilities {
            save_dialog: true,
            ..Default::default()
        },
    );
    let id = WorkbenchId::new("example.gear");

    let tools = registry.tools_for(&id).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].id, "example.gear.new");
    assert_eq!(tools[0].icon, Some("example.gear/gear"));
    assert!(
        ui_kit::icon::exists("example.gear/gear"),
        "the package's icon is drawable"
    );
    assert!(registry.command("example.gear.make").is_some());

    let mut document = Document::new("gears");
    let made = run(
        &mut registry,
        &mut document,
        "example.gear",
        "example.gear.make",
        json!({"teeth": 20, "module": 2.0, "thickness": 8.0, "bore": 5.0}),
    )
    .expect("makes a gear");
    let feature = core_document::FeatureId(made["feature"].as_str().unwrap().parse().unwrap());
    let node = document.get_feature_meta(feature).unwrap().clone();
    assert_eq!(node.workbench_id.as_str(), "example.gear.gear");
    assert_eq!(node.made_by.as_deref(), Some("example.gear 0.1.0"));

    let info = registry.feature_info(&node).unwrap();
    assert_eq!(info.kind_label, "Spur gear, 20 teeth");
    assert_eq!(info.icon, "example.gear/gear");
    assert_eq!(registry.parameters(&node).len(), 4);

    let jobs = registry.rebuild_jobs(&mut document);
    assert_eq!(jobs.len(), 1, "one body to build");
    let plan = jobs.into_iter().next().unwrap().plan.expect("a plan");
    assert_eq!(plan.op_features, vec![feature]);
    let v20 = volume(&plan.ops);
    // Between the root circle and the tip circle, less the bore.
    let (root, tip, bore) = (20.0 - 2.5, 22.0, 2.5f64);
    let pi = std::f64::consts::PI;
    assert!(v20 > pi * (root * root - bore * bore) * 8.0, "{v20}");
    assert!(v20 < pi * (tip * tip - bore * bore) * 8.0, "{v20}");
    assert!(
        registry.rebuild_jobs(&mut document).is_empty(),
        "the plan settled the dirty flags"
    );

    // A formula drives it like any built-in feature.
    document
        .set_feature_formula(feature, "/teeth", Some("20 + 4".into()))
        .unwrap();
    let jobs = registry.rebuild_jobs(&mut document);
    assert_eq!(jobs.len(), 1, "the formula marked it for rebuilding");
    let v24 = volume(&jobs.into_iter().next().unwrap().plan.unwrap().ops);
    assert!(v24 > v20 * 1.2, "more teeth, a bigger gear: {v20} → {v24}");
}

#[test]
fn a_bench_that_overruns_traps_or_overgrows_costs_the_app_nothing() {
    let package = installed("tests/rogue", "rogue.wasm", "rogue-limits");
    let mut registry = registry_with(&package, Capabilities::default());
    assert!(
        registry
            .tools_for(&WorkbenchId::new("test.rogue"))
            .unwrap()
            .is_empty(),
        "a tool whose id is not the package's is left out"
    );
    let mut document = Document::new("rogue");
    let mut call = |command: &str| {
        run(
            &mut registry,
            &mut document,
            "test.rogue",
            command,
            json!({}),
        )
    };
    assert_eq!(call("test.rogue.count"), Ok(json!(1)));

    let started = Instant::now();
    assert!(
        call("test.rogue.spin").is_err(),
        "an endless loop is stopped"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "within its budget"
    );
    assert_eq!(
        call("test.rogue.count"),
        Ok(json!(1)),
        "a fresh instance answers"
    );

    assert!(call("test.rogue.panic").is_err());
    assert_eq!(call("test.rogue.count"), Ok(json!(1)));

    assert!(
        call("test.rogue.grow").is_err(),
        "memory stops at the package's cap"
    );
    assert!(
        call("test.rogue.count").is_err(),
        "three failures turn the bench off"
    );
}

#[test]
fn a_bench_reaches_its_own_folder_and_nothing_else() {
    let package = installed("tests/rogue", "rogue.wasm", "rogue-files");
    let mut registry = registry_with(&package, Capabilities::default());
    let mut document = Document::new("rogue");
    let answer = run(
        &mut registry,
        &mut document,
        "test.rogue",
        "test.rogue.files",
        json!({}),
    )
    .expect("files");
    assert_eq!(answer["back"], "kept");
    assert_eq!(
        answer["outside"], false,
        "the file system beyond its folder is not there"
    );
    assert_eq!(
        std::fs::read_to_string(package.data_dir().join("note.txt")).unwrap(),
        "kept"
    );
}

#[test]
fn a_bench_changes_only_its_own_kinds() {
    let package = installed("tests/rogue", "rogue.wasm", "rogue-kinds");
    let mut registry = registry_with(&package, Capabilities::default());
    let mut document = Document::new("rogue");
    let id = run(
        &mut registry,
        &mut document,
        "test.rogue",
        "test.rogue.add",
        json!({}),
    )
    .expect("its own kind");
    assert!(id.as_str().is_some());
    let refused = run(
        &mut registry,
        &mut document,
        "test.rogue",
        "test.rogue.outside",
        json!({}),
    )
    .unwrap_err();
    assert!(refused.contains("does not own"), "{refused}");
    assert_eq!(document.feature_tree().all_nodes().count(), 1);
}

#[test]
fn a_job_runs_away_from_the_window_and_reports_when_the_bench_is_active() {
    let package = installed("tests/rogue", "rogue.wasm", "rogue-job");
    let mut registry = registry_with(&package, Capabilities::default());
    let mut document = Document::new("rogue");
    let job = run(
        &mut registry,
        &mut document,
        "test.rogue",
        "test.rogue.job",
        json!({"steps": 300}),
    )
    .expect("starts");
    assert_eq!(job, json!(1));
    let bench = registry
        .workbench_mut(&WorkbenchId::new("test.rogue"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while bench.busy() {
        assert!(Instant::now() < deadline, "the job ends");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut ctx =
        WorkbenchRuntimeContext::new(&mut document, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
    bench.on_activate(&mut ctx);
    bench.on_frame(0.016, &mut ctx);
    let logs: Vec<String> = ctx.drain_logs().into_iter().map(|l| l.message).collect();
    assert!(
        logs.iter().any(|l| l.starts_with("job 1: Ok(")),
        "the bench is told how its job ended: {logs:?}"
    );
}

/// Run the rogue's helper job and answer how it ended, as the bench was
/// told.
fn helper_job(granted: Capabilities, name: &str) -> String {
    let package = installed("tests/rogue", "rogue.wasm", name);
    let helpers = package.dir.join("helpers").join(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    std::fs::create_dir_all(&helpers).unwrap();
    let script = helpers.join("upper");
    std::fs::write(&script, "#!/bin/sh\ntr a-z A-Z\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut registry = registry_with(&package, granted);
    let mut document = Document::new("rogue");
    run(
        &mut registry,
        &mut document,
        "test.rogue",
        "test.rogue.helper",
        json!({}),
    )
    .unwrap();
    let bench = registry
        .workbench_mut(&WorkbenchId::new("test.rogue"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while bench.busy() {
        assert!(Instant::now() < deadline, "the job ends");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut ctx =
        WorkbenchRuntimeContext::new(&mut document, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
    bench.on_activate(&mut ctx);
    bench.on_frame(0.016, &mut ctx);
    ctx.drain_logs()
        .into_iter()
        .map(|l| l.message)
        .find(|m| m.starts_with("job "))
        .expect("the bench heard of its job")
}

#[cfg(unix)]
#[test]
fn a_helper_runs_only_when_the_user_allowed_it() {
    let allowed = helper_job(
        Capabilities {
            helper: true,
            ..Default::default()
        },
        "rogue-helper-on",
    );
    assert_eq!(allowed, "job 1: Ok(\"HELLO\")");
    let refused = helper_job(Capabilities::default(), "rogue-helper-off");
    assert!(refused.contains("not been allowed"), "{refused}");
}

#[test]
fn a_feature_whose_package_is_missing_names_it_and_keeps_its_data() {
    let package = installed("tests/rogue", "rogue.wasm", "rogue-missing");
    let mut registry = registry_with(&package, Capabilities::default());
    let mut document = Document::new("rogue");
    run(
        &mut registry,
        &mut document,
        "test.rogue",
        "test.rogue.add",
        json!({}),
    )
    .unwrap();
    let node = document
        .feature_tree()
        .all_nodes()
        .next()
        .unwrap()
        .1
        .clone();

    let without = DocumentService::default();
    assert!(without.feature_info(&node).is_none());
    assert_eq!(
        FeatureInfo::unowned(&node).family_label,
        "Needs test.rogue 0.1.0"
    );
    assert!(
        FeatureInfo::missing_package(&node)
            .unwrap()
            .contains("not installed")
    );
}

#[test]
fn reinstalling_keeps_the_data_folder_and_uninstalling_removes_it() {
    let package = installed("tests/rogue", "rogue.wasm", "rogue-reinstall");
    let root = package.dir.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(package.data_dir()).unwrap();
    std::fs::write(package.data_dir().join("keep.txt"), "mine").unwrap();
    let archive = root.parent().unwrap().join("package.pcbench");
    let again = wb_wasm::install(&archive, &root).expect("reinstalls");
    assert_eq!(
        std::fs::read_to_string(again.data_dir().join("keep.txt")).unwrap(),
        "mine"
    );
    assert_eq!(wb_wasm::discover(&root).len(), 1);
    wb_wasm::uninstall(&root, "test.rogue").unwrap();
    assert!(wb_wasm::discover(&root).is_empty());
}
