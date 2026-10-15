use core_document::{DocumentResult, DocumentService, Workbench};
use wb_assembly::AssemblyWorkbench;
use wb_design::DesignWorkbench;
use wb_sketch::SketchWorkbench;
use wb_surface::SurfaceWorkbench;

mod listed {
    use super::*;
    // Defines `register_all_workbenches`: registers the built-in workbenches
    // and records their descriptors for the UI.
    core_document::define_workbenches!(
        SketchWorkbench,
        DesignWorkbench,
        SurfaceWorkbench,
        AssemblyWorkbench
    );
}

/// Register the built-in workbenches: those the UI lists, then the ones
/// that only register imports and commands, which it does not.
pub fn register_all_workbenches(registry: &mut DocumentService) -> DocumentResult<()> {
    listed::register_all_workbenches(registry)?;
    registry.register_workbench(Box::new(project_tools::ProjectImportBench))
}

pub use core_document::registration::REGISTERED_WORKBENCHES;

/// What fetches packages and store indexes: HTTPS on a desktop.
#[cfg(not(target_arch = "wasm32"))]
fn network() -> wb_wasm::remote::Http {
    wb_wasm::remote::Http::default()
}

/// A browser page fetches with the page's own requests, which are not
/// synchronous: what installs through [`wb_wasm::remote::Fetch`] is the
/// desktop's, and the page has its own paths ([`prepare_package_later`],
/// [`listed_package`]).
#[cfg(target_arch = "wasm32")]
fn network() -> PageFetch {
    PageFetch
}

#[cfg(target_arch = "wasm32")]
const PAGE_FETCH: &str = "a page installs packages from a file or a workbench store";

#[cfg(target_arch = "wasm32")]
struct PageFetch;

#[cfg(target_arch = "wasm32")]
impl wb_wasm::remote::Fetch for PageFetch {
    fn json(&self, _url: &str) -> Result<serde_json::Value, String> {
        Err(PAGE_FETCH.into())
    }

    fn bytes(&self, _url: &str, _limit: u64) -> Result<Vec<u8>, String> {
        Err(PAGE_FETCH.into())
    }
}

/// Start `package` as a workbench on a browser page, allowing it `granted`
/// of what it asks for, and hand `done` the workbench for
/// [`register_prepared`] once the page has it running.
#[cfg(target_arch = "wasm32")]
pub fn prepare_package_later(
    package: Package,
    granted: Capabilities,
    done: impl FnOnce(Result<Box<dyn Workbench>, String>) + 'static,
) {
    wb_wasm::set_profile_source(wb_sketch::profile::closed_profile);
    wb_wasm::load(package, granted, move |bench| {
        done(bench.map(|bench| Box::new(bench) as Box<dyn Workbench>))
    });
}

pub use wb_wasm::remote::Source;
#[cfg(target_arch = "wasm32")]
pub use wb_wasm::store::listed_package;
pub use wb_wasm::store::{Index as StoreIndex, Listing};
pub use wb_wasm::{Capabilities, Package, package::ARCHIVE_EXTENSION};

/// How an installed workbench package stands: loaded, turned off, or
/// failed and why.
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
    /// The GitHub release it was installed from, when it was.
    pub source: Option<Source>,
    /// A newer release's tag, once a check found one.
    pub update: Option<String>,
}

impl PackageStatus {
    /// An installed package in `state`.
    pub fn of(package: &Package, state: PackageState) -> Self {
        let manifest = &package.manifest;
        Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            dir: package.dir.clone(),
            requested: manifest.capabilities.clone(),
            state,
            source: wb_wasm::remote::source_of(package),
            update: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PackageState {
    Loaded,
    /// Turned off in Preferences.
    Disabled,
    /// It did not load; why.
    Failed(String),
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
                    source: None,
                    update: None,
                });
                continue;
            }
        };
        let manifest = &package.manifest;
        let mut status = PackageStatus::of(&package, PackageState::Loaded);
        if !enabled(&manifest.id) {
            status.state = PackageState::Disabled;
            statuses.push(status);
            continue;
        }
        let loaded = prepare_package(&package, &granted(&manifest.id))
            .and_then(|bench| register_prepared(registry, bench));
        if let Err(reason) = loaded {
            tracing::warn!(target: "printcad.bench", package = %manifest.id, "not loaded: {reason}");
            status.state = PackageState::Failed(reason);
        }
        statuses.push(status);
    }
    statuses
}

/// Compile and start `package` as a workbench, allowing it `granted` of
/// what it asks for, ready for [`register_prepared`]. The slow part of
/// loading a package: call it away from the window.
pub fn prepare_package(
    package: &Package,
    granted: &Capabilities,
) -> Result<Box<dyn Workbench>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // A package reads a sketch's closed loops as a feature's profile.
        wb_wasm::set_profile_source(wb_sketch::profile::closed_profile);
        wb_wasm::load(package, granted).map(|bench| Box::new(bench) as Box<dyn Workbench>)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (package, granted);
        Err("a page starts packages with prepare_package_later".into())
    }
}

/// Add a prepared package's workbench to the registry and the lists the
/// toolbar and menus read.
pub fn register_prepared(
    registry: &mut DocumentService,
    bench: Box<dyn Workbench>,
) -> Result<(), String> {
    let descriptor = bench.descriptor();
    registry
        .register_workbench(bench)
        .map_err(|e| e.to_string())?;
    let mut known = REGISTERED_WORKBENCHES.lock().unwrap();
    known.push(descriptor);
    known.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(())
}

/// Take a package's workbench out of the registry and those lists; the
/// workbench, when it was there.
pub fn unregister_package(
    registry: &mut DocumentService,
    id: &core_document::WorkbenchId,
) -> Option<Box<dyn Workbench>> {
    let bench = registry.unregister_workbench(id)?;
    REGISTERED_WORKBENCHES
        .lock()
        .unwrap()
        .retain(|d| &d.id != id);
    Some(bench)
}

/// Install the package archive at `archive` under `root`.
pub fn install_package(
    archive: &std::path::Path,
    root: &std::path::Path,
) -> Result<Package, String> {
    wb_wasm::install(archive, root)
}

/// Install the package a GitHub repository or release address publishes.
/// It reaches the network; call it away from the window.
pub fn install_from_github(text: &str, root: &std::path::Path) -> Result<Package, String> {
    wb_wasm::remote::install_from_github(&network(), text, root)
}

/// The workbench store's index at `url`. It reaches the network.
pub fn fetch_store(url: &str) -> Result<StoreIndex, String> {
    wb_wasm::store::fetch_index(&network(), url)
}

/// An index the app kept, from its text.
pub fn read_store(text: &str) -> Result<StoreIndex, String> {
    wb_wasm::store::read_index_text(text)
}

/// Install a package listed in the store, checked against the index. It
/// reaches the network.
pub fn install_listed(listing: &Listing, root: &std::path::Path) -> Result<Package, String> {
    wb_wasm::store::install_listed(&network(), listing, root)
}

/// Whether release `latest` is newer than the one installed (`tag`, with
/// the manifest's `version`).
pub fn is_newer(latest: &str, tag: &str, version: &str) -> bool {
    wb_wasm::remote::newer(latest, tag, version)
}

/// The latest published release of `repo` (`owner/repo`), and whether it
/// is newer than `version`. It reaches the network.
pub fn latest_release(
    repo: &str,
    version: &str,
) -> Result<(wb_wasm::remote::Latest, bool), String> {
    let latest = wb_wasm::remote::latest(&network(), repo)?;
    let newer = wb_wasm::remote::newer(&latest.tag, "", version);
    Ok((latest, newer))
}

/// Every package installed from GitHub under `root`, each with the newer
/// release's tag when there is one. It reaches the network.
pub fn check_updates(root: &std::path::Path) -> Vec<(String, Result<Option<String>, String>)> {
    let http = network();
    wb_wasm::discover(root)
        .into_iter()
        .flatten()
        .filter(|p| wb_wasm::remote::source_of(p).is_some())
        .map(|p| {
            let found = wb_wasm::remote::check(&http, &p).map(|r| r.map(|r| r.tag));
            (p.manifest.id, found)
        })
        .collect()
}

/// Update the installed package `id` to its repository's latest release,
/// keeping its data. It reaches the network.
pub fn update_package(root: &std::path::Path, id: &str) -> Result<Package, String> {
    let package = Package::read(&root.join(id))?;
    let http = network();
    match wb_wasm::remote::check(&http, &package)? {
        Some(release) => wb_wasm::remote::update(&http, &package, &release),
        None => Err(format!("{} is up to date", package.manifest.name)),
    }
}

/// Remove the installed package `id`.
pub fn uninstall_package(root: &std::path::Path, id: &str) -> Result<(), String> {
    wb_wasm::uninstall(root, id)
}
