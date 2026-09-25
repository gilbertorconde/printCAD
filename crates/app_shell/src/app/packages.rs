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
    done: &'static str,
    package: Package,
    bench: Option<Result<Box<dyn Workbench>, String>>,
}

/// What a package thread found.
pub(crate) enum PackageNews {
    Ready(Box<Ready>),
    Failed(String),
    Checked(Vec<(String, Result<Option<String>, String>)>),
}

/// The package threads' line back, and how many are out.
pub(crate) struct PackageWork {
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
    fn package_thread(&mut self, work: impl FnOnce() -> PackageNews + Send + 'static) {
        let tx = self.package_work.tx.clone();
        self.package_work.pending += 1;
        let started = std::thread::Builder::new()
            .name("printcad-packages".into())
            .spawn(move || {
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
        let path = path.to_path_buf();
        self.install_with("Installed", move |root| {
            workbenches::install_package(&path, root)
                .map_err(|e| format!("Could not install {}: {e}", path.display()))
        });
    }

    /// Install what a GitHub repository or release address publishes.
    pub(crate) fn install_package_from_github(&mut self, text: String) {
        app_log::info(format!(
            "Fetching the workbench package from {}",
            text.trim()
        ));
        self.install_with("Installed", move |root| {
            workbenches::install_from_github(&text, root)
                .map_err(|e| format!("Could not install the package: {e}"))
        });
    }

    /// Update package `id` to its repository's latest release.
    pub(crate) fn update_package(&mut self, id: String) {
        self.install_with("Updated", move |root| {
            workbenches::update_package(root, &id)
                .map_err(|e| format!("Could not update {id}: {e}"))
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

    /// Remove the installed package `id`, unloading its workbench.
    pub(crate) fn remove_package(&mut self, id: &str) {
        let Some(root) = root() else {
            return;
        };
        self.unload_bench(&WorkbenchId::new(id));
        match workbenches::uninstall_package(&root, id) {
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
