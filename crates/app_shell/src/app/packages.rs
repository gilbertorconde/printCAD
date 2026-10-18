//! Workbench packages: loaded at start as the user allowed; installed from
//! a file or a GitHub release, updated, removed, turned on and off, and
//! given or refused what they may reach, all while the app runs. Whatever
//! is slow (the network, compiling a component) runs on a thread of its own
//! and reports through `PackageNews`; the registry changes on the UI
//! thread, between frames.

use std::sync::mpsc;

use core_document::{Workbench, WorkbenchId};
use settings::{PackageGrant, PackageSettings};
use workbenches::{Capabilities, Package, PackageState, PackageStatus};

use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::ui::ActiveWorkbench;

/// A package whose files are in place, and its workbench made ready to
/// register: `None` when the user turned the package off.
pub(crate) struct Ready {
    /// What was done, for the notice: "Installed", "Updated", "Loaded".
    pub(crate) done: &'static str,
    pub(crate) package: Package,
    pub(crate) bench: Option<Result<Box<dyn Workbench>, String>>,
}

/// What a package thread found.
pub(crate) enum PackageNews {
    Ready(Box<Ready>),
    Failed(String),
    Checked(Vec<(String, Result<Option<String>, String>)>),
    /// What the app's own releases hold; `quiet` for the look at start.
    AppRelease {
        found: crate::app::updates::ReleaseCheck,
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
pub(crate) struct StoreState {
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
pub(crate) struct StoreView {
    /// Each store by its index's address, in the user's order.
    pub stores: Vec<(String, StoreState)>,
    /// Installed packages already warned of as taken off a list.
    warned: std::collections::HashSet<String>,
}

/// A store's name: what its index calls it, else its address's host.
pub(crate) fn store_name(url: &str, index: Option<&workbenches::StoreIndex>) -> String {
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

    /// Follow the user's list: a store added is kept from a run before
    /// where it can be, one removed goes.
    fn follow(&mut self, urls: &[String]) {
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
fn kept_store(url: &str) -> Option<workbenches::StoreIndex> {
    let text = std::fs::read_to_string(settings::store_cache(url)?).ok()?;
    workbenches::read_store(&text).ok()
}

/// Keep store `url`'s index for the next run, and for browsing offline.
fn keep_store(url: &str, index: &workbenches::StoreIndex) {
    let Some(path) = settings::store_cache(url) else {
        return;
    };
    let written = serde_json::to_string(index)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, text).map_err(|e| e.to_string())
        });
    if let Err(e) = written {
        app_log::warn(format!("Could not keep the workbench store's list: {e}"));
    }
}

/// The package threads' line back, and how many are out.
pub(crate) struct PackageWork {
    pub(crate) tx: mpsc::Sender<PackageNews>,
    rx: mpsc::Receiver<PackageNews>,
    pub(crate) pending: usize,
}

impl Default for PackageWork {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, pending: 0 }
    }
}

impl PackageWork {
    /// A package thread is out: the frames keep coming until it reports.
    pub(crate) fn busy(&self) -> bool {
        self.pending > 0
    }
}

/// What the user allowed a package, as the loader reads it.
pub(crate) fn capabilities(grant: PackageGrant) -> Capabilities {
    Capabilities {
        save_dialog: grant.save_dialog,
        helper: grant.helper,
        network: grant.network,
    }
}

/// Load every installed package the user has not turned off, after the
/// built-in benches.
pub(crate) fn register(
    registry: &mut core_document::DocumentService,
    settings: &PackageSettings,
) -> Vec<PackageStatus> {
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
fn prepare(
    package: &Package,
    settings: &PackageSettings,
) -> Option<Result<Box<dyn Workbench>, String>> {
    let id = &package.manifest.id;
    settings
        .enabled(id)
        .then(|| workbenches::prepare_package(package, &capabilities(settings.grant(id))))
}

fn root() -> Option<std::path::PathBuf> {
    let root = settings::workbenches_dir();
    if root.is_none() {
        app_log::error("There is no folder to install workbench packages in");
    }
    root
}

impl PrintCadApp {
    /// Run `work` on a thread of its own, its news read by
    /// [`Self::drain_package_news`].
    pub(crate) fn package_thread(&mut self, work: impl FnOnce() -> PackageNews + Send + 'static) {
        let tx = self.package_work.tx.clone();
        self.package_work.pending += 1;
        let started = crate::platform::spawn("printcad-packages", move || {
            let _ = tx.send(work());
        });
        if let Err(e) = started {
            self.package_work.pending -= 1;
            app_log::error(format!("Could not start the package work: {e}"));
        }
    }

    /// Put a package's files in place with `place` and make its workbench
    /// ready, away from the window.
    fn install_with(
        &mut self,
        done: &'static str,
        place: impl FnOnce(&std::path::Path) -> Result<Package, String> + Send + 'static,
    ) {
        let Some(root) = root() else {
            return;
        };
        let settings = self.user_settings.packages.clone();
        self.package_thread(move || match place(&root) {
            Ok(package) => {
                let bench = prepare(&package, &settings);
                PackageNews::Ready(Box::new(Ready {
                    done,
                    package,
                    bench,
                }))
            }
            Err(e) => PackageNews::Failed(e),
        });
    }

    /// Install the package archive at `path`.
    pub(crate) fn install_package_from(&mut self, path: &std::path::Path) {
        {
            let path = path.to_path_buf();
            self.install_with("Installed", move |root| {
                workbenches::install_package(&path, root)
                    .map_err(|e| format!("Could not install {}: {e}", path.display()))
            });
        }
    }

    /// Install what a GitHub repository or release address publishes.
    pub(crate) fn install_package_from_github(&mut self, text: String) {
        {
            app_log::info(format!(
                "Fetching the workbench package from {}",
                text.trim()
            ));
            self.install_with("Installed", move |root| {
                workbenches::install_from_github(&text, root)
                    .map_err(|e| format!("Could not install the package: {e}"))
            });
        }
    }

    /// Update package `id` to its repository's latest release.
    pub(crate) fn update_package(&mut self, id: String) {
        self.install_with("Updated", move |root| {
            workbenches::update_package(root, &id)
                .map_err(|e| format!("Could not update {id}: {e}"))
        });
    }

    /// The stores the user keeps, by their indexes' addresses.
    pub(crate) fn store_urls(&self) -> Vec<String> {
        // Dev/bench hook: `PRINTCAD_STORE_INDEX` stands in for the list
        // (addresses `;`-separated), such as registries served on this
        // machine.
        if let Ok(urls) = std::env::var("PRINTCAD_STORE_INDEX") {
            return urls.split(';').map(str::to_string).collect();
        }
        self.user_settings
            .packages
            .stores
            .iter()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .collect()
    }

    /// Fetch every store's index, showing the one kept from the last run
    /// meanwhile; `quiet` keeps failures out of the log.
    pub(crate) fn look_at_stores(&mut self, quiet: bool) {
        let urls = self.store_urls();
        self.store.follow(&urls);
        let mut fetch = Vec::new();
        for (url, state) in &mut self.store.stores {
            state.looked = true;
            if !state.looking {
                state.looking = true;
                fetch.push(url.clone());
            }
        }
        for url in fetch {
            self.package_thread(move || PackageNews::Store {
                found: workbenches::fetch_store(&url),
                url,
                quiet,
            });
        }
    }

    fn take_store(
        &mut self,
        url: String,
        found: Result<workbenches::StoreIndex, String>,
        quiet: bool,
    ) {
        let Some((_, state)) = self.store.stores.iter_mut().find(|(u, _)| *u == url) else {
            // Removed from the list while it was fetched.
            return;
        };
        state.looking = false;
        match found {
            Ok(index) => {
                keep_store(&url, &index);
                state.index = Some(index);
                state.error = None;
                // Whoever has a package a store took off its list hears
                // why, once a run.
                for status in &self.packages {
                    let Some(why) = self.store.lists(status).and_then(|l| l.removed.clone()) else {
                        continue;
                    };
                    if self.store.warned.insert(status.id.clone()) {
                        app_log::warn(format!(
                            "{} was taken off the workbench store's list: {why}",
                            status.name
                        ));
                    }
                }
            }
            Err(e) => {
                if !quiet {
                    app_log::warn(format!(
                        "Could not read the workbench store {}: {e}",
                        store_name(&url, state.index.as_ref())
                    ));
                }
                state.error = Some(e);
            }
        }
    }

    /// Install the package store `store` lists as `id`.
    pub(crate) fn install_listed(&mut self, store: String, id: String) {
        let Some(listing) = self.store.listing(&store, &id).cloned() else {
            app_log::error(format!("The workbench store lists no package {id}"));
            return;
        };
        app_log::info(format!(
            "Fetching {} from the workbench store",
            listing.name
        ));
        self.install_with("Installed", move |root| {
            workbenches::install_listed(&listing, root)
                .map_err(|e| format!("Could not install {}: {e}", listing.name))
        });
    }

    /// Look for newer releases of the packages installed from GitHub.
    pub(crate) fn check_package_updates(&mut self) {
        let Some(root) = root() else {
            return;
        };
        if !self.packages.iter().any(|p| p.source.is_some()) {
            return;
        }
        self.package_thread(move || PackageNews::Checked(workbenches::check_updates(&root)));
    }

    /// Take what the package threads found, once a frame.
    pub(crate) fn drain_package_news(&mut self) {
        while let Ok(news) = self.package_work.rx.try_recv() {
            self.package_work.pending = self.package_work.pending.saturating_sub(1);
            match news {
                PackageNews::Ready(ready) => self.take_ready(*ready),
                PackageNews::Failed(e) => app_log::error(e),
                PackageNews::Checked(found) => self.take_checked(found),
                PackageNews::AppRelease { found, quiet } => self.take_app_release(found, quiet),
                PackageNews::Store { url, found, quiet } => self.take_store(url, found, quiet),
            }
        }
    }

    fn take_checked(&mut self, found: Vec<(String, Result<Option<String>, String>)>) {
        for (id, result) in found {
            let status = self.packages.iter_mut().find(|p| p.id == id);
            match (result, status) {
                (Ok(tag), Some(status)) => {
                    if let Some(tag) = &tag
                        && status.update.as_ref() != Some(tag)
                    {
                        app_log::success(format!(
                            "{} {tag} is out: update it in Preferences › Workbench packages",
                            status.name
                        ));
                    }
                    status.update = tag;
                }
                (Ok(_), None) => {}
                (Err(e), _) => app_log::warn(format!("No update check for {id}: {e}")),
            }
        }
    }

    /// A package's files are in place and its workbench ready: it takes
    /// the place of the version running, if one is.
    fn take_ready(&mut self, ready: Ready) {
        let Ready {
            done,
            package,
            bench,
        } = ready;
        let id = WorkbenchId::new(package.manifest.id.clone());
        let mut status = PackageStatus::of(&package, PackageState::Loaded);
        match bench {
            None => {
                self.unload_bench(&id);
                status.state = PackageState::Disabled;
                app_log::success(format!(
                    "{done} {} {}; it is turned off in Preferences › Workbench packages",
                    status.name, status.version
                ));
            }
            Some(Err(e)) => {
                // The version running, if any, keeps running.
                app_log::error(format!(
                    "{} {} did not load: {e}",
                    status.name, status.version
                ));
                status.state = PackageState::Failed(e);
            }
            Some(Ok(bench)) => {
                self.unload_bench(&id);
                match self.load_bench(bench) {
                    Ok(label) => app_log::success(format!(
                        "{done} {} {}: {label} is in the workbench list",
                        status.name, status.version
                    )),
                    Err(e) => {
                        app_log::error(format!(
                            "{} {} did not load: {e}",
                            status.name, status.version
                        ));
                        status.state = PackageState::Failed(e);
                    }
                }
            }
        }
        match self.packages.iter_mut().find(|p| p.id == status.id) {
            Some(slot) => *slot = status,
            None => self.packages.push(status),
        }
    }

    /// Register a prepared workbench with the settings it had, and rebuild
    /// the features it owns in every open document with it; its label.
    fn load_bench(&mut self, mut bench: Box<dyn Workbench>) -> Result<String, String> {
        let descriptor = bench.descriptor();
        if let Some(settings) = self.user_settings.workbenches.get(descriptor.id.as_str()) {
            bench.apply_settings_json(settings);
        }
        workbenches::register_prepared(&mut self.registry, bench)?;
        if let Ok(bench) = self.registry.workbench(&descriptor.id) {
            bench.invalidate_all(&mut self.session.document);
            for slot in &mut self.tabs {
                if let Some(parked) = slot.parked.as_mut() {
                    bench.invalidate_all(&mut parked.document);
                }
            }
        }
        self.command_ids.clear();
        self.redraw_needed = true;
        Ok(descriptor.label)
    }

    /// Take workbench `id` out of the running app, keeping its settings:
    /// every tab on it moves to the landing workbench, and the editing
    /// state the tabs kept for it goes.
    fn unload_bench(&mut self, id: &WorkbenchId) {
        let Ok(bench) = self.registry.workbench(id) else {
            return;
        };
        if let Some(settings) = bench.settings_json() {
            self.user_settings
                .workbenches
                .insert(id.as_str().to_owned(), settings);
        }
        let landing = self
            .registry
            .ids()
            .iter()
            .find(|b| *b != id && !self.registry.is_modal(b))
            .cloned();
        if self.session.active_workbench.0 == *id
            && let Some(landing) = landing.clone()
        {
            self.switch_workbench_for_flow(landing);
        }
        let away = |active: &mut ActiveWorkbench, back: &mut Option<ActiveWorkbench>| {
            if back.as_ref().is_some_and(|b| b.0 == *id) {
                *back = None;
            }
            if active.0 == *id
                && let Some(landing) = &landing
            {
                *active = ActiveWorkbench(landing.clone());
            }
        };
        away(
            &mut self.session.active_workbench,
            &mut self.session.return_workbench,
        );
        self.session.bench_states.remove(id.as_str());
        for slot in &mut self.tabs {
            if let Some(parked) = slot.parked.as_mut() {
                away(&mut parked.active_workbench, &mut parked.return_workbench);
                parked.bench_states.remove(id.as_str());
            }
        }
        workbenches::unregister_package(&mut self.registry, id);
        self.command_ids.clear();
        self.redraw_needed = true;
    }

    /// Say which packages the document on screen needs and does not have
    /// running: not installed (and where to get it), or turned off.
    pub(crate) fn report_missing_packages(&mut self) {
        let mut needed: Vec<(String, Option<String>)> = Vec::new();
        for (_, node) in self.session.document.feature_tree().all_nodes() {
            if self.registry.feature_info(node).is_some() {
                continue;
            }
            if let Some((id, repo)) = core_document::FeatureInfo::needed_package(node) {
                match needed.iter_mut().find(|(n, _)| *n == id) {
                    Some((_, known)) if known.is_none() => *known = repo,
                    Some(_) => {}
                    None => needed.push((id, repo)),
                }
            }
        }
        needed.sort();
        for (id, repo) in needed {
            let installed = self.packages.iter().find(|p| p.id == id);
            let message = match (installed, repo) {
                (Some(status), _) if status.state == PackageState::Disabled => format!(
                    "This document uses {}, which is turned off in Preferences › Workbench packages",
                    status.name
                ),
                (Some(status), _) => format!(
                    "This document uses {}, which did not load: see Preferences › Workbench packages",
                    status.name
                ),
                (None, Some(repo)) => format!(
                    "This document uses the workbench package {id} (github.com/{repo}), which is \
                     not installed: its features' menu in the tree installs it"
                ),
                (None, None) => {
                    format!("This document uses the workbench package {id}, which is not installed")
                }
            };
            app_log::warn(message);
        }
    }

    /// Remove the installed package `id`, unloading its workbench.
    pub(crate) fn remove_package(&mut self, id: &str) {
        let removed = {
            let Some(root) = root() else {
                return;
            };
            self.unload_bench(&WorkbenchId::new(id));
            workbenches::uninstall_package(&root, id)
        };
        match removed {
            Ok(()) => {
                let name = self
                    .packages
                    .iter()
                    .find(|p| p.id == id)
                    .map_or(id.to_string(), |p| p.name.clone());
                self.packages.retain(|p| p.id != id);
                self.user_settings.workbenches.remove(id);
                app_log::success(format!("Removed {name}"));
            }
            Err(e) => app_log::error(format!("Could not remove {id}: {e}")),
        }
    }

    /// Preferences changed which packages load or what they may reach:
    /// unload what was turned off, and load again what was turned on or
    /// given different grants.
    pub(crate) fn packages_changed(&mut self, before: &PackageSettings) {
        let now = self.user_settings.packages.clone();
        if before.stores != now.stores {
            self.look_at_stores(false);
        }
        let packages: Vec<(String, std::path::PathBuf)> = self
            .packages
            .iter()
            .map(|p| (p.id.clone(), p.dir.clone()))
            .collect();
        for (id, dir) in packages {
            let (was, is) = (before.enabled(&id), now.enabled(&id));
            let regranted = before.grant(&id) != now.grant(&id);
            if was && !is {
                self.unload_bench(&WorkbenchId::new(id.clone()));
                if let Some(status) = self.packages.iter_mut().find(|p| p.id == id) {
                    status.state = PackageState::Disabled;
                    app_log::success(format!("Turned {} off", status.name));
                }
            } else if is && (!was || regranted) {
                self.install_with("Loaded", move |_| workbenches::Package::read(&dir));
            }
        }
    }
}
