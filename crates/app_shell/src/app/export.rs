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
    /// Where the body sits, for its kernel shape; `None` when it has not
    /// moved.
    transform: Option<[[f64; 4]; 4]>,
    mesh: Arc<TriMesh>,
}

/// A finished export, for the log.
pub(crate) struct ExportOutcome {
    path: PathBuf,
    result: Result<kernel_ogeom::export::Exported, String>,
    /// The slicer command to open the file with once written, when the
    /// export was a send to the slicer.
    open_with: Option<String>,
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
        let draft = self.last_export.clone();
        self.write_export(path, draft, None);
    }

    /// Hand every visible body to the slicer: written to a file of the
    /// slicer's format in the temporary folder, then opened with the
    /// slicer command from Preferences.
    pub(crate) fn send_to_slicer(&mut self) {
        let printing = &self.user_settings.printing;
        let draft = ExportDraft {
            format: match printing.slicer_format {
                settings::SlicerFormat::ThreeMf => ExportFormat::ThreeMf,
                settings::SlicerFormat::Stl => ExportFormat::Stl,
            },
            selected_only: false,
            detail: self.last_export.detail.clone(),
        };
        if self.export_bodies(&draft).is_empty() {
            app_log::warn("Nothing to send: no visible body has geometry");
            return;
        }
        let folder = std::env::temp_dir().join("printcad").join("slicer");
        if let Err(err) = std::fs::create_dir_all(&folder) {
            app_log::error(format!("Could not make {}: {err}", folder.display()));
            return;
        }
        let stem = self
            .session
            .current_file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_stem_of(self.session.document.name()));
        let path = folder.join(format!("{stem}.{}", draft.format.extension()));
        let command = printing.slicer_command.clone();
        self.write_export(path, draft, Some(command));
    }

    /// Write `draft` to `path` on a thread of its own, then open it with
    /// `open_with` when given.
    fn write_export(&mut self, path: PathBuf, draft: ExportDraft, open_with: Option<String>) {
        if self.export_rx.is_some() {
            app_log::warn("An export is still being written");
            return;
        }
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
                    transform: b.transform,
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
            let _ = tx.send(ExportOutcome {
                path,
                result,
                open_with,
            });
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
        let ExportOutcome {
            path,
            result,
            open_with,
        } = outcome;
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
                if let Some(command) = open_with {
                    match open_in_slicer(&command, &path) {
                        Ok(program) => app_log::info(format!("Opened it with {program}")),
                        Err(err) => app_log::error(format!(
                            "Could not open it in the slicer: {err}. Set the slicer \
                             in Preferences › 3D printing."
                        )),
                    }
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
        bodies_to_export(document, |id| {
            if draft.selected_only {
                selected == Some(id.0)
            } else {
                document.imported_body_effective_visible(id)
            }
        })
    }
}

/// A document name as a file name: letters, digits and a few marks kept,
/// anything else a dash.
fn file_stem_of(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if stem.trim_matches('-').is_empty() {
        "model".to_string()
    } else {
        stem
    }
}

/// A command line split into words: spaces separate, double quotes hold a
/// word with spaces together.
fn command_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The program and arguments that open `file` with `command`: `{file}`
/// stands for the path, which otherwise goes last; an empty command hands
/// the file to the system's application for its type.
fn slicer_invocation(command: &str, file: &Path) -> (String, Vec<String>) {
    let mut words = command_words(command);
    if words.is_empty() {
        words.push("xdg-open".to_string());
    }
    let file = file.display().to_string();
    let program = words.remove(0);
    let named = words.iter().any(|w| w.contains("{file}"));
    let mut args: Vec<String> = words
        .into_iter()
        .map(|w| w.replace("{file}", &file))
        .collect();
    if !named {
        args.push(file);
    }
    (program, args)
}

/// Start the slicer on `file` beside the app, and say which program ran.
fn open_in_slicer(command: &str, file: &Path) -> std::io::Result<String> {
    let (program, args) = slicer_invocation(command, file);
    let mut child = std::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    // Collected when it exits, so it never lingers as a finished process.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(program)
}

/// Write `bodies` of `document` (every visible one when `None`) to `path`
/// now, on this thread: what a script's export does.
pub(crate) fn export_document(
    document: &core_document::Document,
    path: PathBuf,
    format: ExportFormat,
    bodies: Option<Vec<core_document::BodyId>>,
    tolerance: Option<f32>,
) -> Result<(PathBuf, kernel_ogeom::export::Exported), String> {
    let owned = bodies_to_export(document, |id| match &bodies {
        Some(list) => list.contains(&id),
        None => document.imported_body_effective_visible(id),
    });
    if owned.is_empty() {
        return Err("there is nothing to export".to_string());
    }
    let mut detail = ExportDraft::default().detail;
    if let Some(tolerance) = tolerance {
        detail.chord_tolerance = tolerance;
    }
    let borrowed: Vec<ExportBody<'_>> = owned
        .iter()
        .map(|b| ExportBody {
            name: b.name.clone(),
            brep: b.brep.as_deref().map(Vec::as_slice),
            transform: b.transform,
            mesh: &b.mesh,
        })
        .collect();
    let path = with_extension(&path, format);
    let exported = export(&borrowed, format, &detail).map_err(|e| e.to_string())?;
    std::fs::write(&path, &exported.bytes).map_err(|e| format!("could not write the file: {e}"))?;
    Ok((path, exported))
}

/// The bodies of `document` that `take` accepts and that have geometry,
/// in tree order.
fn bodies_to_export(
    document: &core_document::Document,
    take: impl Fn(core_document::BodyId) -> bool,
) -> Vec<OwnedBody> {
    let mut out: Vec<(usize, OwnedBody)> = document
        .imported_geometries()
        .filter(|(id, geometry)| !geometry.mesh.indices.is_empty() && take(**id))
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
                    transform: {
                        let placement = document.body_placement(*id);
                        (!placement.is_identity()).then(|| placement.rows())
                    },
                    mesh: Arc::clone(&geometry.mesh),
                },
            )
        })
        .collect();
    out.sort_by_key(|(order, body)| (*order, body.name.clone()));
    out.into_iter().map(|(_, body)| body).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slicer_command_takes_the_file_where_it_says_or_last() {
        let file = Path::new("/tmp/printcad/slicer/part.3mf");
        let (program, args) = slicer_invocation("", file);
        assert_eq!(program, "xdg-open");
        assert_eq!(args, ["/tmp/printcad/slicer/part.3mf"]);
        let (program, args) = slicer_invocation("\"/opt/My Slicer/run\" --single", file);
        assert_eq!(program, "/opt/My Slicer/run");
        assert_eq!(args, ["--single", "/tmp/printcad/slicer/part.3mf"]);
        let (_, args) = slicer_invocation("slice --load={file} --go", file);
        assert_eq!(args, ["--load=/tmp/printcad/slicer/part.3mf", "--go"]);
    }

    #[test]
    fn a_document_name_becomes_a_safe_file_name() {
        assert_eq!(file_stem_of("Bracket v2"), "Bracket-v2");
        assert_eq!(file_stem_of("a/b"), "a-b");
        assert_eq!(file_stem_of("   "), "model");
    }

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
