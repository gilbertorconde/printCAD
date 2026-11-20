//! Export in the running app: File › Export opens a dialog on a draft of
//! the options (format, which bodies, the mesh tolerance, a file per
//! body), a confirmed draft asks for a file name (or a folder, for a file
//! per body), and the kernel writes the files on a thread of its own so a
//! large document never holds the window. The result lands in the log.

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

use doc_export::*;
use kernel_ogeom::export::ExportFormat;

use crate::PrintCadApp;

/// An export asked for while the tab's solids were rebuilding: where it
/// goes, what it writes and what opens it.
pub(crate) struct WaitingExport {
    path: PathBuf,
    draft: ExportDraft,
    open_with: Option<String>,
    /// The tab's build cycle when it was asked for.
    after: u64,
    /// When it was asked for.
    asked: std::time::Instant,
}

/// How long an export waits for the history after the feature an open
/// task edits before it writes what a save would write.
const TASK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

impl Default for WaitingExport {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            draft: ExportDraft::default(),
            open_with: None,
            after: 0,
            asked: std::time::Instant::now(),
        }
    }
}

impl WaitingExport {
    /// Whether it is written at `now`: once the builds since it was asked
    /// are in, or, while a task's preview holds the history after its
    /// feature back, once it has waited [`TASK_WAIT`].
    fn ready(&self, builds: &crate::app::session_state::Builds, now: std::time::Instant) -> bool {
        builds.built_since(self.after)
            || (builds.preview_feature.is_some() && now.duration_since(self.asked) >= TASK_WAIT)
    }
}

impl PrintCadApp {
    /// Open the export dialog on the options used last.
    pub(crate) fn open_export_dialog(&mut self) {
        self.session.file.export_pending = Some(self.files.last_export.clone());
    }

    /// The walkthrough's second step: once the example has a solid, the
    /// export dialog opens over it, set for the whole document.
    pub(crate) fn open_export_when_ready(&mut self) {
        if !self.session.file.export_when_ready {
            return;
        }
        let draft = ExportDraft {
            selected_only: false,
            ..self.files.last_export.clone()
        };
        if !self.export_bodies(&draft).is_empty() {
            self.session.file.export_when_ready = false;
            self.session.file.export_pending = Some(draft);
        }
    }

    /// Ask where to write, once the dialog is confirmed.
    pub(crate) fn confirm_export(&mut self) {
        let Some(draft) = self.session.file.export_pending.take() else {
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
        self.files.last_export = draft.clone();
        self.start_file_dialog(if draft.per_body {
            crate::app::doc_io::FileDialogKind::ExportFolder
        } else {
            crate::app::doc_io::FileDialogKind::Export(draft.format)
        });
    }

    /// Write the confirmed export to `path` on a thread of its own.
    pub(crate) fn start_export(&mut self, path: PathBuf) {
        let draft = self.files.last_export.clone();
        let configured = self
            .session
            .document
            .configurations()
            .is_some_and(|(_, t)| !t.rows.is_empty());
        if draft.every_configuration && configured && !draft.per_body {
            self.export_every_configuration(&path, &draft);
            return;
        }
        self.write_export(path, draft, None);
    }

    /// One file per configuration beside `path`, each named after it
    /// (`bracket-Large.3mf`): every configuration in turn is put in
    /// effect, built and written, and the one in effect before comes back.
    /// It runs as a script, which waits for each build, and is one undo
    /// step.
    fn export_every_configuration(&mut self, path: &Path, draft: &ExportDraft) {
        let base = path.with_extension("");
        let bodies = if draft.selected_only && !draft.layout {
            let selected = self.bodies_to_export_selected();
            if selected.is_empty() {
                app_log::warn("Nothing to export: select a body with geometry first");
                return;
            }
            Some(selected)
        } else {
            None
        };
        let source =
            every_configuration_script(&base.display().to_string(), draft, bodies.as_deref());
        self.submit_script(
            scripting::Job::Script {
                name: "Export every configuration".to_string(),
                source,
            },
            crate::app::scripts::RunKind::File,
        );
    }

    /// Hand every visible body, or with `selected` the selected ones, to
    /// the slicer: written to a file of the slicer's format in this run's
    /// private folder, then opened with the slicer command from
    /// Preferences. Laid out for printing when `laid_out`, or when
    /// Preferences say so: the selected bodies' parts alone, with
    /// `selected`.
    pub(crate) fn send_to_slicer(&mut self, laid_out: bool, selected: bool) {
        let printing = &self.user_settings.printing;
        let layout = laid_out || printing.slicer_layout;
        let draft = ExportDraft {
            format: match printing.slicer_format {
                settings::SlicerFormat::ThreeMf => ExportFormat::ThreeMf,
                settings::SlicerFormat::Stl => ExportFormat::Stl,
            },
            selected_only: selected,
            detail: self.files.last_export.detail.clone(),
            every_configuration: false,
            // What goes to the slicer is what gets printed.
            textures: true,
            layout,
            per_body: false,
        };
        if self.export_bodies(&draft).is_empty() {
            app_log::warn(if selected {
                "Nothing to send: select a body with geometry first"
            } else {
                "Nothing to send: no visible body has geometry"
            });
            return;
        }
        if self.files.export_rx.is_some() {
            app_log::warn("An export is still being written");
            return;
        }
        let folder = match self.files.slicer_folder.clone().filter(|f| f.is_dir()) {
            Some(folder) => folder,
            None => {
                match app_platform::make_private_folder(&app_platform::private_base(), "slicer") {
                    Ok(folder) => {
                        self.files.slicer_folder = Some(folder.clone());
                        folder
                    }
                    Err(err) => {
                        app_log::error(format!("Could not make a folder for the slicer: {err}"));
                        return;
                    }
                }
            }
        };
        let stem = self
            .session
            .file
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_stem_of(self.session.document.name()));
        let path = match app_platform::reserve_file(&folder, &stem, draft.format.extension()) {
            Ok(path) => path,
            Err(err) => {
                app_log::error(format!(
                    "Could not make a file for the slicer in {}: {err}",
                    folder.display()
                ));
                return;
            }
        };
        let command = printing.slicer_command.clone();
        self.write_export(path, draft, Some(command));
    }

    /// Write `draft` to `path` on a thread of its own, then open it with
    /// `open_with` when given. It waits for the tab's next build cycle,
    /// which sends what an edit of the same frame marked dirty, and for
    /// the builds out to land, so the file holds the shapes the history
    /// makes rather than the ones from before an edit.
    fn write_export(&mut self, path: PathBuf, draft: ExportDraft, open_with: Option<String>) {
        if self.files.export_rx.is_some() || self.session.file.export_waiting.is_some() {
            app_log::warn("An export is still being written");
            return;
        }
        if !self.session.builds.quiet() {
            app_log::info("Waiting for the rebuild before writing the export");
        }
        self.session.file.export_waiting = Some(WaitingExport {
            path,
            draft,
            open_with,
            after: self.session.builds.cycles,
            asked: std::time::Instant::now(),
        });
    }

    /// Write the export that waits for the tab's rebuild, once its solids
    /// are built and no other export is being written.
    pub(crate) fn drive_waiting_export(&mut self) {
        let ready = self
            .session
            .file
            .export_waiting
            .as_ref()
            .is_some_and(|w| w.ready(&self.session.builds, std::time::Instant::now()));
        if !ready || self.files.export_rx.is_some() {
            return;
        }
        if let Some(waiting) = self.session.file.export_waiting.take() {
            self.write_export_now(waiting.path, waiting.draft, waiting.open_with);
        }
    }

    /// End the tab's waiting export as its tab or the app goes (`why`):
    /// written now when its rebuild is done or a task's preview holds it
    /// back (what a save writes), the export in flight finished first,
    /// else said in the log not to be written.
    pub(crate) fn settle_waiting_export(&mut self, why: &str) {
        let Some(waiting) = self.session.file.export_waiting.take() else {
            return;
        };
        if waiting.ready(&self.session.builds, waiting.asked + TASK_WAIT) {
            self.finish_export();
            self.write_export_now(waiting.path, waiting.draft, None);
        } else {
            app_log::warn(format!(
                "The export to {} was not written: {why} before the rebuild it waited for",
                waiting.path.display()
            ));
        }
    }

    fn write_export_now(&mut self, path: PathBuf, draft: ExportDraft, open_with: Option<String>) {
        let bodies = self.export_bodies(&draft);
        let files = export_files(&path, &bodies, draft.format, draft.per_body);
        let path = if draft.per_body {
            path
        } else {
            with_extension(&path, draft.format)
        };
        self.warn_stopped_at_task(&path, &bodies);
        let semantic = export_pmi(
            &self.session.document,
            &self.registry,
            &bodies,
            draft.format,
        );
        let (tx, rx) = channel();
        self.files.export_rx = Some(rx);
        app_log::info(format!(
            "Exporting {} {} to {}",
            bodies.len(),
            if bodies.len() == 1 { "body" } else { "bodies" },
            path.display()
        ));
        // A mesh for printing makes up for the printer's shrinkage.
        let printing = &self.user_settings.printing;
        let compensation = printing
            .compensation
            .as_ref()
            .filter(|_| printing.apply_compensation && draft.format.is_mesh());
        let scale = compensation.map(|c| c.scale);
        let rounds = compensation
            .map(|c| [c.holes, c.bosses])
            .filter(|r| r.iter().any(|d| *d != 0.0));
        if let Some([x, y, z]) = scale {
            app_log::info(format!("Compensating shrinkage: ×{x:.4} ×{y:.4} ×{z:.4}"));
        }
        if let Some([holes, bosses]) = rounds {
            app_log::info(format!(
                "Compensating round faces: holes {holes:+.3} mm, bosses {bosses:+.3} mm in radius"
            ));
        }
        let started = app_platform::spawn("printcad-export", move || {
            let files = write_files(&bodies, &semantic, files, &draft, (scale, rounds));
            let _ = tx.send(ExportOutcome { files, open_with });
        });
        if let Err(err) = started {
            self.files.export_rx = None;
            app_log::error(format!("Could not start the export: {err}"));
        }
    }

    /// Say which of the bodies written to `path` stop at the feature an
    /// open task edits, the history after it not built yet.
    fn warn_stopped_at_task(&self, path: &Path, bodies: &[OwnedBody]) {
        let stopped = self.session.builds.stopped_at_task();
        let feature = self
            .session
            .builds
            .preview_feature
            .and_then(|f| self.session.document.get_feature_meta(f))
            .map_or_else(|| "the feature".to_string(), |n| format!("`{}`", n.name));
        for body in stopped
            .iter()
            .filter_map(|id| bodies.iter().find(|b| b.body == *id))
        {
            app_log::warn(format!(
                "The export to {} holds `{}` only up to {feature}, which an open task edits: \
                 the history after it was not built. Close the task and export again for the \
                 whole body",
                path.display(),
                body.name,
            ));
        }
    }

    /// Log a finished export, if one has finished. An animation written
    /// stops keeping frames coming once its thread ends.
    pub(crate) fn poll_export(&mut self) {
        self.files.animations.retain(|job| !job.is_finished());
        match app_platform::take_answer(&mut self.files.export_rx) {
            Some(Ok(outcome)) => report_export(outcome, true),
            Some(Err(_)) => app_log::error(EXPORT_STOPPED),
            None => {}
        }
    }

    /// Wait for an export and the animations being written, before the
    /// process ends: one cut off would leave no file. Nothing opens in the
    /// slicer on the way out.
    pub(crate) fn finish_export(&mut self) {
        let animations = std::mem::take(&mut self.files.animations);
        if animations.iter().any(|job| !job.is_finished()) {
            app_log::info("Finishing the animation before exit…");
        }
        for job in animations {
            job.join();
        }
        let Some(rx) = self.files.export_rx.take() else {
            return;
        };
        app_log::info("Finishing the export before exit…");
        match rx.recv() {
            Ok(outcome) => report_export(outcome, false),
            Err(_) => app_log::error(EXPORT_STOPPED),
        }
    }

    /// Remove this run's private folders on the way out, after the export
    /// in flight is written: the scratch folder, and the slicer's once no
    /// slicer may still be reading a file in it. One left for a slicer
    /// stays for the system to clear with the runtime or temporary folder.
    pub(crate) fn remove_private_folders(&mut self) {
        app_platform::remove_scratch_folder();
        let Some(folder) = self.files.slicer_folder.take() else {
            return;
        };
        let (running, handed) = *SLICERS.lock().unwrap_or_else(|e| e.into_inner());
        if slicer_files_may_go(running, handed, std::time::Instant::now()) {
            let _ = std::fs::remove_dir_all(&folder);
        } else {
            tracing::info!(
                target: "printcad.export",
                "left {} for the slicer reading it",
                folder.display()
            );
        }
    }

    /// The bodies the draft asks for: the selected ones, or every visible
    /// body with geometry that is made; or the print layout's copies.
    fn export_bodies(&self, draft: &ExportDraft) -> Vec<OwnedBody> {
        let selected: Option<Vec<core_document::BodyId>> = draft.selected_only.then(|| {
            self.bodies_to_export_selected()
                .into_iter()
                .map(core_document::BodyId)
                .collect()
        });
        let layout = draft
            .layout
            .then(|| self.print_layout_of(selected.as_deref()));
        owned_bodies(
            (&self.whole_solids(), &self.registry),
            draft,
            selected.as_deref(),
            layout.as_ref(),
        )
    }

    /// The bodies an export of the selection writes: the selected body
    /// and any others a box took with it.
    pub(crate) fn bodies_to_export_selected(&self) -> Vec<uuid::Uuid> {
        self.session
            .selection
            .earlier_bodies
            .iter()
            .copied()
            .chain(self.session.selection.body)
            .collect()
    }

    /// The print layout as it stands, of every part or of the parts of
    /// `only`'s bodies: the one shown, when it is up to date, else made
    /// now.
    pub(crate) fn print_layout_of(
        &self,
        only: Option<&[core_document::BodyId]>,
    ) -> doc_render::print_layout::Layout {
        use doc_render::print_layout::{LayoutSettings, lay_out, layout_key, parts_to_print};
        let document = &self.session.document;
        let parts = parts_to_print(document, &self.registry);
        let parts = match only {
            Some(bodies) => parts_of(parts, bodies),
            None => parts,
        };
        let settings = LayoutSettings::of(&self.user_settings.printing);
        let key = layout_key(document, &parts, &settings);
        match &self.session.tools.print_layout {
            Some(shown) if shown.key == key => shown.layout.clone(),
            _ => lay_out(document, &parts, &settings),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// While a task's preview holds a body's history back, an export
    /// waits a bounded time and then writes what a save writes, naming
    /// the bodies that stop at the task's feature; without a task it
    /// waits for the builds however long they take.
    #[test]
    fn an_export_held_back_by_a_task_preview_waits_a_bounded_time() {
        let mut builds = crate::app::session_state::Builds::default();
        let body = core_document::BodyId(uuid::Uuid::new_v4());
        let preview = |complete| crate::app::session::BodyPreview {
            full: Default::default(),
            complete,
            tool: Default::default(),
            id: uuid::Uuid::new_v4(),
            revision: 0,
        };
        builds.cycles = 1;
        builds.previews.insert(body, preview(false));
        let other = uuid::Uuid::new_v4();
        builds.out.sent(other, None);
        let asked = std::time::Instant::now();
        let waiting = WaitingExport {
            asked,
            ..WaitingExport::default()
        };
        let later = asked + TASK_WAIT;
        assert!(!waiting.ready(&builds, later), "no task: it waits");
        builds.preview_feature = Some(core_document::FeatureId(uuid::Uuid::new_v4()));
        assert!(!waiting.ready(&builds, asked));
        assert!(waiting.ready(&builds, later));
        assert_eq!(builds.stopped_at_task(), vec![body]);

        builds.out.landed(other);
        builds.previews.insert(body, preview(true));
        assert!(waiting.ready(&builds, asked), "built: written at once");
        assert!(builds.stopped_at_task().is_empty());
    }
}
