//! Workbench packages: loaded at start as the user allowed, prepared and
//! checked on a thread of its own, which reports through `PackageNews`;
//! and what the app knows of the workbench stores.

use std::sync::mpsc;

use core_document::Workbench;
use settings::{PackageGrant, PackageSettings};
use workbenches::{Capabilities, Package, PackageState, PackageStatus};

/// A package whose files are in place, and its workbench made ready to
/// register: `None` when the user turned the package off.
pub struct Ready {
    /// What was done, for the notice: "Installed", "Updated", "Loaded".
    pub done: &'static str,
    pub package: Package,
    pub bench: Option<Result<Box<dyn Workbench>, String>>,
}

/// What a package thread found.
pub enum PackageNews {
    Ready(Box<Ready>),
    Failed(String),
    Checked(Vec<(String, Result<Option<String>, String>)>),
    /// What the app's own releases hold; `quiet` for the look at start.
    AppRelease {
        found: crate::ReleaseCheck,
        quiet: bool,
    },
    /// The index of the workbench store at `url`; `quiet` for the look
    /// at start.
    Store {
        url: String,
        found: Result<workbenches::StoreIndex, String>,
        quiet: bool,
    },
}

/// One workbench store as the app knows it.
#[derive(Default)]
pub struct StoreState {
    /// Its index as last fetched, or as kept from a run before.
    pub index: Option<workbenches::StoreIndex>,
    /// A fetch is out.
    pub looking: bool,
    /// Why the last fetch failed.
    pub error: Option<String>,
    /// A fetch was asked for in this run.
    pub looked: bool,
}

/// What the app knows of the workbench stores the user keeps.
#[derive(Default)]
pub struct StoreView {
    /// Each store by its index's address, in the user's order.
    pub stores: Vec<(String, StoreState)>,
    /// Installed packages already warned of as taken off a list.
    warned: std::collections::HashSet<String>,
}

/// A store's name: what its index calls it, else its address's host.
pub fn store_name(url: &str, index: Option<&workbenches::StoreIndex>) -> String {
    index
        .and_then(|i| i.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            url.split("://")
                .nth(1)
                .unwrap_or(url)
                .split('/')
                .next()
                .unwrap_or(url)
                .to_string()
        })
}

impl StoreView {
    /// Every listing of every store, each with its store's address.
    pub fn listings(&self) -> impl Iterator<Item = (&str, &workbenches::Listing)> {
        self.stores.iter().flat_map(|(url, state)| {
            state
                .index
                .iter()
                .flat_map(|i| i.packages.iter())
                .map(move |l| (url.as_str(), l))
        })
    }

    /// Store `url`'s listing of package `id`.
    pub fn listing(&self, url: &str, id: &str) -> Option<&workbenches::Listing> {
        self.listings()
            .find(|(u, l)| *u == url && l.id == id)
            .map(|(_, l)| l)
    }

    /// The listing of an installed package: the same id from the same
    /// repository, in any store.
    pub fn lists(&self, status: &PackageStatus) -> Option<&workbenches::Listing> {
        let source = status.source.as_ref()?;
        self.listings()
            .map(|(_, l)| l)
            .find(|l| l.id == status.id && source.repo.eq_ignore_ascii_case(&l.repository))
    }

    /// Whether a store has not been fetched in this run.
    pub fn unlooked(&self) -> bool {
        self.stores.iter().any(|(_, s)| !s.looked)
    }

    /// Whether package `id` is still to be warned of as taken off a
    /// list: true once, then false.
    pub fn warn_once(&mut self, id: &str) -> bool {
        self.warned.insert(id.to_string())
    }

    /// Follow the user's list: a store added is kept from a run before
    /// where it can be, one removed goes.
    pub fn follow(&mut self, urls: &[String]) {
        let mut before = std::mem::take(&mut self.stores);
        for url in urls {
            let state = match before.iter().position(|(u, _)| u == url) {
                Some(at) => before.remove(at).1,
                None => StoreState {
                    index: kept_store(url),
                    ..StoreState::default()
                },
            };
            self.stores.push((url.clone(), state));
        }
    }
}

/// The index of store `url` as the last run kept it.
pub fn kept_store(url: &str) -> Option<workbenches::StoreIndex> {
    let value: serde_json::Value = settings::json_file::load(&settings::store_cache(url)?)
        .map_err(|err| app_log::warn(format!("Could not read the workbench store: {err}")))
        .ok()?;
    workbenches::read_store(&value.to_string()).ok()
}

/// Keep store `url`'s index for the next run, and for browsing offline.
pub fn keep_store(url: &str, index: &workbenches::StoreIndex) {
    let Some(path) = settings::store_cache(url) else {
        return;
    };
    let written = settings::json_file::keep(&path, &Some(index.clone()));
    if let Err(e) = written {
        app_log::warn(format!("Could not keep the workbench store's list: {e}"));
    }
}

/// The package threads' line back, and how many are out.
pub struct PackageWork {
    tx: mpsc::Sender<PackageNews>,
    rx: mpsc::Receiver<PackageNews>,
    pending: usize,
}

impl Default for PackageWork {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, pending: 0 }
    }
}

impl PackageWork {
    /// A package thread is out: the frames keep coming until it reports.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// Run `work` on a thread of its own, its news read by [`Self::take`].
    pub fn spawn(&mut self, work: impl FnOnce() -> PackageNews + Send + 'static) {
        let tx = self.tx.clone();
        self.pending += 1;
        let started = app_platform::spawn("printcad-packages", move || {
            let _ = tx.send(work());
        });
        if let Err(e) = started {
            self.pending -= 1;
            app_log::error(format!("Could not start the package work: {e}"));
        }
    }

    /// What a package thread has sent since the last call, if anything.
    pub fn take(&mut self) -> Option<PackageNews> {
        let news = self.rx.try_recv().ok()?;
        self.pending = self.pending.saturating_sub(1);
        Some(news)
    }
}

/// What the user allowed a package, as the loader reads it.
pub fn capabilities(grant: PackageGrant) -> Capabilities {
    Capabilities {
        save_dialog: grant.save_dialog,
        open_dialog: grant.open_dialog,
        helper: grant.helper,
        network: grant.network,
    }
}

/// Load every installed package the user has not turned off, after the
/// built-in benches.
pub fn register(
    registry: &mut core_document::DocumentService,
    settings: &PackageSettings,
) -> Vec<PackageStatus> {
    if let Some(compiled) = settings::compiled_packages_dir() {
        workbenches::set_compiled_cache(compiled);
    }
    let Some(root) = settings::workbenches_dir() else {
        return Vec::new();
    };
    let statuses = workbenches::register_packages(
        registry,
        &root,
        |id| settings.enabled(id),
        |id| capabilities(settings.grant(id)),
    );
    for status in &statuses {
        match &status.state {
            PackageState::Loaded => app_log::info(format!(
                "Loaded the workbench package {} {}",
                status.name, status.version
            )),
            PackageState::Failed(reason) => app_log::warn(format!(
                "The workbench package {} did not load: {reason}",
                status.id
            )),
            PackageState::Disabled => {}
        }
    }
    statuses
}

/// `package`'s workbench, made ready as `settings` allow; `None` when
/// they turn it off. Slow: it compiles the component.
pub fn prepare(
    package: &Package,
    settings: &PackageSettings,
) -> Option<Result<Box<dyn Workbench>, String>> {
    let id = &package.manifest.id;
    settings
        .enabled(id)
        .then(|| workbenches::prepare_package(package, &capabilities(settings.grant(id))))
}

/// The folder packages are installed in, logged as an error when there
/// is none.
pub fn root() -> Option<std::path::PathBuf> {
    let root = settings::workbenches_dir();
    if root.is_none() {
        app_log::error("There is no folder to install workbench packages in");
    }
    root
}
