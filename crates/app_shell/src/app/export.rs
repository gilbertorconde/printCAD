//! Export: File › Export opens a dialog on a draft of the options (format,
//! which bodies, the mesh tolerance), a confirmed draft asks for a file
//! name, and the kernel writes the file on a thread of its own so a large
//! document never holds the window. The result lands in the log.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use kernel_api::{LinearDeflectionMode, TessellationSettings, TriMesh};
use kernel_ogeom::export::{ExportBody, ExportFormat, export};

use crate::PrintCadApp;
use crate::app_log;

/// The export dialog's options.
#[derive(Debug, Clone)]
pub(crate) struct ExportDraft {
    pub format: ExportFormat,
    /// Only the selected body, rather than every visible one.
    pub selected_only: bool,
    /// The mesh formats' tolerance.
    pub detail: TessellationSettings,
}

impl Default for ExportDraft {
    fn default() -> Self {
        Self {
            format: ExportFormat::ThreeMf,
            selected_only: false,
            // A print resolves far finer than a screen: a hundredth of a
            // millimetre off the true surface and a few degrees per facet.
            detail: TessellationSettings {
                linear_deflection_mode: LinearDeflectionMode::AbsoluteMm,
                chord_tolerance: 0.01,
                angular_tolerance_deg: 5.0,
                ..TessellationSettings::default()
            },
        }
    }
}

/// One body's share of an export, owned so the writer thread can hold it.
struct OwnedBody {
    name: String,
    brep: Option<Arc<Vec<u8>>>,
    mesh: Arc<TriMesh>,
}

/// A finished export, for the log.
pub(crate) struct ExportOutcome {
    path: PathBuf,
    result: Result<kernel_ogeom::export::Exported, String>,
}

/// `path` with the format's extension, unless it already names one the
/// format answers to.
pub(crate) fn with_extension(path: &Path, format: ExportFormat) -> PathBuf {
    if ExportFormat::of_path(path) == Some(format) {
        path.to_path_buf()
    } else {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(format.extension());
        PathBuf::from(name)
    }
}

impl PrintCadApp {
    /// Open the export dialog on the options used last.
    pub(crate) fn open_export_dialog(&mut self) {
        self.session.export_pending = Some(self.last_export.clone());
    }

    /// The walkthrough's second step: once the example has a solid, the
    /// export dialog opens over it, set for the whole document.
    pub(crate) fn open_export_when_ready(&mut self) {
        if !self.session.export_when_ready {
            return;
        }
        let draft = ExportDraft {
            selected_only: false,
            ..self.last_export.clone()
        };
        if !self.export_bodies(&draft).is_empty() {
            self.session.export_when_ready = false;
            self.session.export_pending = Some(draft);
        }
    }

    /// Ask where to write, once the dialog is confirmed.
    pub(crate) fn confirm_export(&mut self) {
        let Some(draft) = self.session.export_pending.take() else {
            return;
        };
        if self.export_bodies(&draft).is_empty() {
            app_log::warn(if draft.selected_only {
                "Nothing to export: select a body with geometry first"
            } else {
                "Nothing to export: no visible body has geometry"
            });
            return;
        }
        self.last_export = draft.clone();
        self.start_file_dialog(crate::app::doc_io::FileDialogKind::Export(draft.format));
    }

    /// Write the confirmed export to `path` on a thread of its own.
    pub(crate) fn start_export(&mut self, path: PathBuf) {
        if self.export_rx.is_some() {
            app_log::warn("An export is still being written");
            return;
        }
        let draft = self.last_export.clone();
        let path = with_extension(&path, draft.format);
        let bodies = self.export_bodies(&draft);
        let (tx, rx) = channel();
        self.export_rx = Some(rx);
        app_log::info(format!(
            "Exporting {} {} to {}",
            bodies.len(),
            if bodies.len() == 1 { "body" } else { "bodies" },
            path.display()
        ));
        std::thread::spawn(move || {
            let borrowed: Vec<ExportBody<'_>> = bodies
                .iter()
                .map(|b| ExportBody {
                    name: b.name.clone(),
                    brep: b.brep.as_deref().map(Vec::as_slice),
                    mesh: &b.mesh,
                })
                .collect();
            let result = export(&borrowed, draft.format, &draft.detail)
                .map_err(|e| e.to_string())
                .and_then(|exported| {
                    std::fs::write(&path, &exported.bytes)
                        .map(|()| exported)
                        .map_err(|e| format!("could not write the file: {e}"))
                });
            let _ = tx.send(ExportOutcome { path, result });
        });
    }

    /// Log a finished export, if one has finished.
    pub(crate) fn poll_export(&mut self) {
        let Some(outcome) = self
            .export_rx
            .as_ref()
            .and_then(|rx: &Receiver<ExportOutcome>| rx.try_recv().ok())
        else {
            return;
        };
        self.export_rx = None;
        let ExportOutcome { path, result } = outcome;
        match result {
            Ok(exported) => {
                let mut line = format!(
                    "Exported {} to {}",
                    match exported.written {
                        1 => "1 body".to_string(),
                        n => format!("{n} bodies"),
                    },
                    path.display()
                );
                if exported.triangles > 0 {
                    line.push_str(&format!(" ({} triangles)", exported.triangles));
                }
                app_log::info(line);
                if !exported.skipped.is_empty() {
                    app_log::warn(format!(
                        "Left out, as meshes have no exact shape to write: {}",
                        exported.skipped.join(", ")
                    ));
                }
            }
            Err(err) => app_log::error(format!("Export to {} failed: {err}", path.display())),
        }
    }

    /// The bodies the draft asks for: the selected one, or every visible
    /// body with geometry, in the document's body order.
    fn export_bodies(&self, draft: &ExportDraft) -> Vec<OwnedBody> {
        let document = &self.session.document;
        let selected = self.session.selected_body;
        let mut out: Vec<(usize, OwnedBody)> = document
            .imported_geometries()
            .filter(|(id, geometry)| {
                !geometry.mesh.indices.is_empty()
                    && if draft.selected_only {
                        selected == Some(id.0)
                    } else {
                        document.imported_body_effective_visible(**id)
                    }
            })
            .map(|(id, geometry)| {
                let order = document
                    .bodies()
                    .iter()
                    .position(|b| b.id == *id)
                    .unwrap_or(usize::MAX);
                let name = document
                    .bodies()
                    .get(order)
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| format!("Body {}", &id.0.to_string()[..8]));
                (
                    order,
                    OwnedBody {
                        name,
                        brep: document.imported_brep_blob_arc(*id),
                        mesh: Arc::clone(&geometry.mesh),
                    },
                )
            })
            .collect();
        out.sort_by_key(|(order, body)| (*order, body.name.clone()));
        out.into_iter().map(|(_, body)| body).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_takes_the_format_s_extension_unless_it_has_it() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            with_extension(&p("/t/a.3mf"), ExportFormat::ThreeMf),
            p("/t/a.3mf")
        );
        assert_eq!(
            with_extension(&p("/t/a.STP"), ExportFormat::Step),
            p("/t/a.STP")
        );
        assert_eq!(with_extension(&p("/t/a"), ExportFormat::Stl), p("/t/a.stl"));
        assert_eq!(
            with_extension(&p("/t/a.stl"), ExportFormat::ThreeMf),
            p("/t/a.stl.3mf")
        );
    }
}
