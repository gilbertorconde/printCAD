//! Workbench packages: loaded at start as the user allowed, installed and
//! removed from Preferences (taking effect at the next start).

use settings::{PackageGrant, PackageSettings};
use workbenches::{Capabilities, PackageState, PackageStatus};

use crate::PrintCadApp;
use crate::log_panel as app_log;

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

impl PrintCadApp {
    /// Install the package archive at `path`; it loads at the next start.
    pub(crate) fn install_package_from(&mut self, path: &std::path::Path) {
        let Some(root) = settings::workbenches_dir() else {
            app_log::error("There is no folder to install workbench packages in");
            return;
        };
        match workbenches::install_package(path, &root) {
            Ok(manifest) => {
                app_log::info(format!(
                    "Installed {} {}; it loads when printCAD starts again",
                    manifest.name, manifest.version
                ));
                self.packages.retain(|p| p.id != manifest.id);
                self.packages.push(PackageStatus {
                    dir: root.join(&manifest.id),
                    id: manifest.id,
                    name: manifest.name,
                    version: manifest.version,
                    description: manifest.description,
                    requested: manifest.capabilities,
                    state: PackageState::Installed,
                });
            }
            Err(e) => app_log::error(format!("Could not install {}: {e}", path.display())),
        }
    }

    /// Remove the installed package `id`; a loaded one stays until the
    /// next start.
    pub(crate) fn remove_package(&mut self, id: &str) {
        let Some(root) = settings::workbenches_dir() else {
            return;
        };
        match workbenches::uninstall_package(&root, id) {
            Ok(()) => {
                app_log::info(format!("Removed the workbench package {id}"));
                if let Some(status) = self.packages.iter_mut().find(|p| p.id == id) {
                    status.state = PackageState::Removed;
                }
            }
            Err(e) => app_log::error(format!("Could not remove {id}: {e}")),
        }
    }
}
