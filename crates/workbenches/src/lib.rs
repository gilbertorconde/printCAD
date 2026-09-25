use core_document::{DocumentResult, DocumentService, Workbench};
use wb_assembly::AssemblyWorkbench;
use wb_part::PartDesignWorkbench;
use wb_sketch::SketchWorkbench;

// Use the core_document macro to define a helper that registers all built-in
// workbenches and records their descriptors for the UI.
core_document::define_workbenches!(SketchWorkbench, PartDesignWorkbench, AssemblyWorkbench);

pub use core_document::registration::REGISTERED_WORKBENCHES;
pub use wb_wasm::{Capabilities, Package, package::ARCHIVE_EXTENSION};

/// How an installed workbench package fared when the app started.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub dir: std::path::PathBuf,
    /// What it asks to reach.
    pub requested: Capabilities,
    pub state: PackageState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PackageState {
    Loaded,
    /// Turned off in Preferences.
    Disabled,
    /// It did not load; why.
    Failed(String),
    /// Installed since the app started: it loads at the next start.
    Installed,
    /// Removed since the app started: it is gone at the next start.
    Removed,
}

/// Register every package installed under `root` that `enabled` allows,
/// each allowed `granted(id)` of what it asks for, after the built-in
/// benches. One that does not load is reported and left out.
pub fn register_packages(
    registry: &mut DocumentService,
    root: &std::path::Path,
    enabled: impl Fn(&str) -> bool,
    granted: impl Fn(&str) -> Capabilities,
) -> Vec<PackageStatus> {
    let mut statuses = Vec::new();
    for found in wb_wasm::discover(root) {
        let package = match found {
            Ok(package) => package,
            Err((dir, reason)) => {
                let id = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                statuses.push(PackageStatus {
                    name: id.clone(),
                    id,
                    version: String::new(),
                    description: String::new(),
                    dir,
                    requested: Capabilities::default(),
                    state: PackageState::Failed(reason),
                });
                continue;
            }
        };
        let manifest = &package.manifest;
        let mut status = PackageStatus {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            dir: package.dir.clone(),
            requested: manifest.capabilities.clone(),
            state: PackageState::Loaded,
        };
        if !enabled(&manifest.id) {
            status.state = PackageState::Disabled;
            statuses.push(status);
            continue;
        }
        let loaded = wb_wasm::load(&package, &granted(&manifest.id)).and_then(|bench| {
            let descriptor = bench.descriptor();
            registry
                .register_workbench(Box::new(bench))
                .map_err(|e| e.to_string())?;
            let mut known = REGISTERED_WORKBENCHES.lock().unwrap();
            known.push(descriptor);
            known.sort_by(|a, b| a.label.cmp(&b.label));
            Ok(())
        });
        if let Err(reason) = loaded {
            tracing::warn!(target: "printcad.bench", package = %manifest.id, "not loaded: {reason}");
            status.state = PackageState::Failed(reason);
        }
        statuses.push(status);
    }
    statuses
}

/// Install the package archive at `archive` under `root`; its manifest.
/// It loads the next time the app starts.
pub fn install_package(
    archive: &std::path::Path,
    root: &std::path::Path,
) -> Result<wb_wasm::Manifest, String> {
    wb_wasm::install(archive, root).map(|p| p.manifest)
}

/// Remove the installed package `id`.
pub fn uninstall_package(root: &std::path::Path, id: &str) -> Result<(), String> {
    wb_wasm::uninstall(root, id)
}
