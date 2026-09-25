//! Workbench packages: loaded at start as the user allowed; installed from
//! a file or a GitHub release, updated and removed from Preferences
//! (taking effect at the next start). Whatever reaches the network runs on
//! a thread of its own and reports through `PackageNews`.

use std::sync::mpsc;

use settings::{PackageGrant, PackageSettings};
use workbenches::{Capabilities, Package, PackageState, PackageStatus};

use crate::PrintCadApp;
use crate::log_panel as app_log;

/// What a package thread found.
pub(crate) enum PackageNews {
    Installed(Result<Package, String>),
    Checked(Vec<(String, Result<Option<String>, String>)>),
    Updated(String, Result<Package, String>),
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
            _ => {}
        }
    }
    statuses
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
        std::thread::Builder::new()
            .name("printcad-packages".into())
            .spawn(move || {
                let _ = tx.send(work());
            })
            .map(|_| ())
            .unwrap_or_else(|e| {
                self.package_work.pending -= 1;
                app_log::error(format!("Could not start the package work: {e}"));
            });
    }

    /// Install the package archive at `path`; it loads at the next start.
    pub(crate) fn install_package_from(&mut self, path: &std::path::Path) {
        let Some(root) = root() else {
            return;
        };
        let installed = workbenches::install_package(path, &root);
        self.take_installed(installed.map_err(|e| format!("{}: {e}", path.display())));
    }

    /// Install what a GitHub repository or release address publishes.
    pub(crate) fn install_package_from_github(&mut self, text: String) {
        let Some(root) = root() else {
            return;
        };
        app_log::info(format!(
            "Fetching the workbench package from {}",
            text.trim()
        ));
        self.package_thread(move || {
            PackageNews::Installed(workbenches::install_from_github(&text, &root))
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

    /// Update package `id` to its repository's latest release.
    pub(crate) fn update_package(&mut self, id: String) {
        let Some(root) = root() else {
            return;
        };
        self.package_thread(move || {
            let updated = workbenches::update_package(&root, &id);
            PackageNews::Updated(id, updated)
        });
    }

    /// Take what the package threads found, once a frame.
    pub(crate) fn drain_package_news(&mut self) {
        while let Ok(news) = self.package_work.rx.try_recv() {
            self.package_work.pending = self.package_work.pending.saturating_sub(1);
            match news {
                PackageNews::Installed(installed) => self.take_installed(installed),
                PackageNews::Checked(found) => {
                    for (id, result) in found {
                        match result {
                            Ok(Some(tag)) => {
                                if let Some(status) = self.packages.iter_mut().find(|p| p.id == id)
                                {
                                    app_log::info(format!(
                                        "{} {tag} is out (Preferences › Workbench packages)",
                                        status.name
                                    ));
                                    status.update = Some(tag);
                                }
                            }
                            Ok(None) => {
                                if let Some(status) = self.packages.iter_mut().find(|p| p.id == id)
                                {
                                    status.update = None;
                                }
                            }
                            Err(e) => app_log::warn(format!("No update check for {id}: {e}")),
                        }
                    }
                }
                PackageNews::Updated(id, updated) => match updated {
                    Ok(package) => self.take_installed(Ok(package)),
                    Err(e) => app_log::error(format!("Could not update {id}: {e}")),
                },
            }
        }
    }

    fn take_installed(&mut self, installed: Result<Package, String>) {
        match installed {
            Ok(package) => {
                let status = PackageStatus::of(&package, PackageState::Installed);
                app_log::info(format!(
                    "Installed {} {}; it loads when printCAD starts again",
                    status.name, status.version
                ));
                self.packages.retain(|p| p.id != status.id);
                self.packages.push(status);
            }
            Err(e) => app_log::error(format!("Could not install the package: {e}")),
        }
    }

    /// Remove the installed package `id`; a loaded one stays until the
    /// next start.
    pub(crate) fn remove_package(&mut self, id: &str) {
        let Some(root) = root() else {
            return;
        };
        match workbenches::uninstall_package(&root, id) {
            Ok(()) => {
                app_log::info(format!("Removed the workbench package {id}"));
                if let Some(status) = self.packages.iter_mut().find(|p| p.id == id) {
                    status.state = PackageState::Removed;
                    status.update = None;
                }
            }
            Err(e) => app_log::error(format!("Could not remove {id}: {e}")),
        }
    }
}
