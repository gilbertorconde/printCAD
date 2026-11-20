//! Workbench packages in the running app: installed from a file or a
//! GitHub release, updated, removed, turned on and off, and given or
//! refused what they may reach, all while the app runs. Whatever is slow
//! (the network, compiling a component) runs on a package thread
//! (`package_store::PackageWork`); the registry changes on the UI thread,
//! between frames.

use core_document::{Workbench, WorkbenchId};
use package_store::{PackageNews, Ready, keep_store, prepare, root, store_name};
use settings::PackageSettings;
use workbenches::{Package, PackageState, PackageStatus};

use crate::PrintCadApp;
use crate::contract::ActiveWorkbench;

impl PrintCadApp {
    /// Run `work` on a thread of its own, its news read by
    /// [`Self::drain_package_news`].
    pub(crate) fn package_thread(&mut self, work: impl FnOnce() -> PackageNews + Send + 'static) {
        self.packages.work.spawn(work);
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
        self.packages.store.follow(&urls);
        let mut fetch = Vec::new();
        for (url, state) in &mut self.packages.store.stores {
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
        let Some((_, state)) = self
            .packages
            .store
            .stores
            .iter_mut()
            .find(|(u, _)| *u == url)
        else {
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
                for status in &self.packages.installed {
                    let Some(why) = self
                        .packages
                        .store
                        .lists(status)
                        .and_then(|l| l.removed.clone())
                    else {
                        continue;
                    };
                    if self.packages.store.warn_once(&status.id) {
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
        let Some(listing) = self.packages.store.listing(&store, &id).cloned() else {
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
        if !self.packages.installed.iter().any(|p| p.source.is_some()) {
            return;
        }
        self.package_thread(move || PackageNews::Checked(workbenches::check_updates(&root)));
    }

    /// Take what the package threads found, once a frame.
    pub(crate) fn drain_package_news(&mut self) {
        while let Some(news) = self.packages.work.take() {
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
            let status = self.packages.installed.iter_mut().find(|p| p.id == id);
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
        match self
            .packages
            .installed
            .iter_mut()
            .find(|p| p.id == status.id)
        {
            Some(slot) => *slot = status,
            None => self.packages.installed.push(status),
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
            for session in crate::app::tabs::sessions_mut(&mut self.session, &mut self.tabs) {
                bench.invalidate_all(&mut session.document);
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
        for session in crate::app::tabs::sessions_mut(&mut self.session, &mut self.tabs) {
            away(&mut session.active_workbench, &mut session.return_workbench);
            session.bench_states.remove(id.as_str());
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
            let installed = self.packages.installed.iter().find(|p| p.id == id);
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
                    .installed
                    .iter()
                    .find(|p| p.id == id)
                    .map_or(id.to_string(), |p| p.name.clone());
                self.packages.installed.retain(|p| p.id != id);
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
            .installed
            .iter()
            .map(|p| (p.id.clone(), p.dir.clone()))
            .collect();
        for (id, dir) in packages {
            let (was, is) = (before.enabled(&id), now.enabled(&id));
            let regranted = before.grant(&id) != now.grant(&id);
            if was && !is {
                self.unload_bench(&WorkbenchId::new(id.clone()));
                if let Some(status) = self.packages.installed.iter_mut().find(|p| p.id == id) {
                    status.state = PackageState::Disabled;
                    app_log::success(format!("Turned {} off", status.name));
                }
            } else if is && (!was || regranted) {
                self.install_with("Loaded", move |_| workbenches::Package::read(&dir));
            }
        }
    }
}
