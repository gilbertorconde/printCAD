//! Export: File › Export opens a dialog on a draft of the options (format,
//! which bodies, the mesh tolerance, a file per body), a confirmed draft
//! asks for a file name (or a folder, for a file per body), and the kernel
//! writes the files on a thread of its own so a large document never holds
//! the window. The result lands in the log.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::channel;

use kernel_api::{LinearDeflectionMode, TessellationSettings, TriMesh};
use kernel_ogeom::export::{
    ExportBody, ExportColors, ExportFormat, ExportNote, ExportPmi, export_with_annotations,
};

use crate::PrintCadApp;

/// The export dialog's options.
#[derive(Debug, Clone)]
pub(crate) struct ExportDraft {
    pub format: ExportFormat,
    /// Only the selected body, rather than every visible one.
    pub selected_only: bool,
    /// The mesh formats' tolerance.
    pub detail: TessellationSettings,
    /// One file per configuration, each named after it.
    pub every_configuration: bool,
    /// Bodies' surface textures pressed into the mesh formats.
    pub textures: bool,
    /// The print layout rather than the bodies where they sit: each part
    /// flat on the bed, as many as the parts list prints.
    pub layout: bool,
    /// One file per body (per copy, laid out), each named after it, in a
    /// folder, rather than one file of them all.
    pub per_body: bool,
}

impl Default for ExportDraft {
    fn default() -> Self {
        Self {
            format: ExportFormat::ThreeMf,
            selected_only: false,
            every_configuration: false,
            textures: true,
            layout: false,
            per_body: false,
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
    body: core_document::BodyId,
    name: String,
    brep: Option<Arc<Vec<u8>>>,
    /// Where the body sits, for its kernel shape; `None` when it has not
    /// moved.
    transform: Option<[[f64; 4]; 4]>,
    mesh: Arc<TriMesh>,
    /// The body's surface textures, to press into its mesh.
    pressing: Option<crate::app::textures::Pressing>,
    /// The body's shown notes, in its own frame; their `body` is set as
    /// the export takes them.
    notes: Vec<ExportNote>,
    /// The colours it shows in.
    colors: Option<ExportColors>,
}

/// The colours `body` shows in: its own (the one it was given, else the
/// default look) and each face's given or imported colour over it.
fn body_colors(document: &core_document::Document, body: core_document::BodyId) -> ExportColors {
    let entry = document.body(body);
    let own = entry.and_then(|b| b.display).unwrap_or_default().color;
    let mut faces: Vec<(u32, [f32; 3])> = document
        .derived()
        .imported_brep_face_colors(body)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, *c))
        .collect();
    if let (Some(entry), Some((mesh, _))) = (entry, document.derived().local_geometry(body)) {
        let count = mesh.faces.iter().max().map_or(0, |m| m + 1);
        for index in 0..count {
            if let Some(c) = entry.face_colors.iter().find(|c| c.is_face(&mesh, index)) {
                faces.retain(|(i, _)| *i != index);
                faces.push((index, c.color));
            }
        }
    }
    faces.sort_by_key(|(i, _)| *i);
    ExportColors { body: own, faces }
}

/// The notes a body's export carries, in the body's frame: those shown,
/// each as its label reads, on its face as the body is built now and
/// naming that face, then a callout of each cosmetic thread, its
/// designation at its bore's mouth, naming the bore.
fn body_notes(document: &core_document::Document, body: core_document::BodyId) -> Vec<ExportNote> {
    let local = document.derived().local_geometry(body);
    let mesh = local.as_ref().map(|(mesh, _)| mesh.as_ref());
    let notes = document
        .body_notes(body)
        .iter()
        .filter(|note| !note.hidden)
        .map(|note| {
            let placed = note.placed(mesh);
            let (at, label) = (placed.at, placed.label);
            ExportNote {
                body: 0,
                text: document.note_text(note),
                at,
                label,
                faces: note.face.map(|_| at.map(f64::from)).into_iter().collect(),
            }
        });
    let threads = document.derived().body_threads(body).map(|thread| {
        let (at, label) = thread.leader();
        ExportNote {
            body: 0,
            text: thread.label(),
            at,
            label,
            faces: vec![thread.on_wall()],
        }
    });
    notes.chain(threads).collect()
}

/// The notes of the owned bodies at `indices`, as the kernel's export
/// takes them: each on its body's place among them.
fn export_notes(bodies: &[OwnedBody], indices: &[usize]) -> Vec<ExportNote> {
    indices
        .iter()
        .enumerate()
        .flat_map(|(at, &i)| {
            bodies[i].notes.iter().map(move |note| ExportNote {
                body: at,
                ..note.clone()
            })
        })
        .collect()
}

/// Gather the benches' world-space annotations and take them into each
/// source body's frame, before the export's own placement is applied.
fn export_pmi(
    document: &core_document::Document,
    registry: &core_document::DocumentService,
    bodies: &[OwnedBody],
    format: ExportFormat,
) -> Vec<ExportPmi> {
    if format.is_mesh() {
        return Vec::new();
    }
    bodies
        .iter()
        .enumerate()
        .filter_map(|(index, body)| {
            body.brep.as_ref()?;
            let mut annotations = registry.semantic_pmi(document, body.body);
            if annotations.is_empty() {
                return None;
            }
            let placement = document.body_placement(body.body);
            let rotation = placement.quat().as_dquat().inverse();
            let translation = placement.offset().as_dvec3();
            annotations
                .map_points(|p| (rotation * (glam::DVec3::from_array(p) - translation)).to_array());
            Some(ExportPmi {
                body: index,
                annotations,
            })
        })
        .collect()
}

impl OwnedBody {
    /// What a mesh file writes of the body's faces: its textures pressed
    /// in at the export's detail.
    fn finish(&self) -> Option<Finish<'_>> {
        let pressing = self.pressing.as_ref()?;
        Some(Box::new(move |mesh: TriMesh| {
            pressing.apply(&mesh, surface_texture::Detail::EXPORT)
        }))
    }
}

/// What a mesh file writes of one body's faces once meshed.
type Finish<'a> = Box<dyn Fn(TriMesh) -> TriMesh + Sync + 'a>;

/// Borrow the owned bodies at `indices` as the kernel's export takes
/// them, their finishes kept alive beside them.
fn borrowed<'a>(
    bodies: &'a [OwnedBody],
    finishes: &'a [Option<Finish<'a>>],
    indices: &[usize],
    scale: Option<[f64; 3]>,
    rounds: Option<[f64; 2]>,
) -> Vec<ExportBody<'a>> {
    indices
        .iter()
        .map(|&i| {
            let b = &bodies[i];
            ExportBody {
                name: b.name.clone(),
                brep: b.brep.as_deref().map(Vec::as_slice),
                transform: b.transform,
                mesh: &b.mesh,
                finish: finishes[i]
                    .as_deref()
                    .map(|f| f as &(dyn Fn(TriMesh) -> TriMesh + Sync)),
                scale,
                rounds,
                colors: b.colors.clone(),
            }
        })
        .collect()
}

/// The files an export writes: each its path and the owned bodies in it,
/// by index. One file of them all, or with `per_body` one per body in the
/// folder `path` names, each named after its body.
fn export_files(
    path: &Path,
    bodies: &[OwnedBody],
    format: ExportFormat,
    per_body: bool,
) -> Vec<(PathBuf, Vec<usize>)> {
    if per_body {
        let names: Vec<&str> = bodies.iter().map(|b| b.name.as_str()).collect();
        per_body_paths(path, &names, format)
            .into_iter()
            .enumerate()
            .map(|(i, path)| (path, vec![i]))
            .collect()
    } else {
        vec![(with_extension(path, format), (0..bodies.len()).collect())]
    }
}

/// A file in `folder` for each of `names`, named after it as a file name
/// may be (`file_stem_of`), a number added to a name another took first,
/// whatever its case.
pub(crate) fn per_body_paths(folder: &Path, names: &[&str], format: ExportFormat) -> Vec<PathBuf> {
    let mut taken = std::collections::HashSet::new();
    names
        .iter()
        .map(|name| {
            let stem = file_stem_of(name);
            let mut candidate = stem.clone();
            let mut n = 1;
            while !taken.insert(candidate.to_lowercase()) {
                n += 1;
                candidate = format!("{stem}-{n}");
            }
            folder.join(format!("{candidate}.{}", format.extension()))
        })
        .collect()
}

/// Write each of `files` from `bodies` as `draft` says: its bodies, their
/// notes and their annotations. Each file's outcome, in order; a file
/// written goes in whole or not at all.
fn write_files(
    bodies: &[OwnedBody],
    semantic: &[ExportPmi],
    files: Vec<(PathBuf, Vec<usize>)>,
    draft: &ExportDraft,
    (scale, rounds): (Option<[f64; 3]>, Option<[f64; 2]>),
) -> Vec<(PathBuf, Result<kernel_ogeom::export::Exported, String>)> {
    let finishes: Vec<_> = bodies.iter().map(OwnedBody::finish).collect();
    files
        .into_iter()
        .map(|(path, indices)| {
            let borrowed = borrowed(bodies, &finishes, &indices, scale, rounds);
            let notes = export_notes(bodies, &indices);
            let semantic: Vec<ExportPmi> = semantic
                .iter()
                .filter_map(|p| {
                    let at = indices.iter().position(|i| *i == p.body)?;
                    Some(ExportPmi {
                        body: at,
                        annotations: p.annotations.clone(),
                    })
                })
                .collect();
            let result =
                export_with_annotations(&borrowed, &notes, &semantic, draft.format, &draft.detail)
                    .map_err(|e| e.to_string())
                    .and_then(|exported| {
                        app_platform::write(&path, &exported.bytes)
                            .map(|()| exported)
                            .map_err(|e| format!("could not write the file: {e}"))
                    });
            (path, result)
        })
        .collect()
}

/// What the log says of an export thread that ended without an outcome.
const EXPORT_STOPPED: &str = "The export stopped without writing the file";

/// A finished export, for the log.
pub(crate) struct ExportOutcome {
    /// Each file and what writing it came to.
    files: Vec<(PathBuf, Result<kernel_ogeom::export::Exported, String>)>,
    /// The slicer command to open the file with once written, when the
    /// export was a send to the slicer.
    open_with: Option<String>,
}

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

/// `path` with the format's extension, unless it already names one the
/// format answers to.
pub(crate) fn with_extension(path: &Path, format: ExportFormat) -> PathBuf {
    if ExportFormat::of_path(path).map(ExportFormat::extension) == Some(format.extension()) {
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

/// The parts among `parts` that have a body of `bodies`.
pub(crate) fn parts_of(
    parts: Vec<core_document::PrintPart>,
    bodies: &[core_document::BodyId],
) -> Vec<core_document::PrintPart> {
    parts
        .into_iter()
        .filter(|p| p.bodies.iter().any(|b| bodies.contains(b)))
        .collect()
}

/// Log how an export went, and open it in the slicer it was for when
/// `open` and it was written.
fn report_export(outcome: ExportOutcome, open: bool) {
    let ExportOutcome { files, open_with } = outcome;
    let single = files.len() == 1;
    for (path, result) in files {
        let exported = match result {
            Ok(exported) => exported,
            Err(err) => {
                app_log::error(format!("Export to {} failed: {err}", path.display()));
                continue;
            }
        };
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
        if !exported.annotations_left_out.is_empty() {
            app_log::warn(format!(
                "Annotations left out, as they lie on no face of their body: {}",
                exported.annotations_left_out.join(", ")
            ));
        }
        if let Some(command) = open_with.as_deref().filter(|_| open && single) {
            match open_in_slicer(command, &path) {
                Ok(program) => app_log::info(format!("Opened it with {program}")),
                Err(err) => app_log::error(format!(
                    "Could not open it in the slicer: {err}. Set the slicer \
                     in Preferences › 3D printing."
                )),
            }
        }
    }
}

/// The copies of a print layout, each its body's shape where the layout
/// puts it, named as the layout names them.
fn layout_bodies(
    document: &core_document::Document,
    layout: &doc_render::print_layout::Layout,
    textured: bool,
) -> Vec<OwnedBody> {
    let mut pictures = crate::app::textures::Pictures::new();
    layout
        .pieces
        .iter()
        .filter_map(|piece| {
            let pressing = textured
                .then(|| crate::app::textures::Pressing::of(document, piece.body, &mut pictures))
                .flatten();
            let brep = document.derived().imported_brep_blob_arc(piece.body);
            let (local, _) = document.derived().local_geometry(piece.body)?;
            if local.indices.is_empty() {
                return None;
            }
            Some(match brep {
                Some(brep) => OwnedBody {
                    body: piece.body,
                    name: piece.name.clone(),
                    brep: Some(brep),
                    transform: Some(piece.placement.rows()),
                    mesh: local,
                    pressing,
                    notes: body_notes(document, piece.body),
                    colors: Some(body_colors(document, piece.body)),
                },
                // A mesh body has only its triangles: pressed in its own
                // frame, then moved to the bed.
                None => {
                    let pressed = match &pressing {
                        Some(pressing) => pressing.apply(&local, surface_texture::Detail::EXPORT),
                        None => (*local).clone(),
                    };
                    OwnedBody {
                        body: piece.body,
                        name: piece.name.clone(),
                        brep: None,
                        transform: None,
                        mesh: Arc::new(piece.placement.mesh(&pressed)),
                        pressing: None,
                        notes: Vec::new(),
                        colors: Some(body_colors(document, piece.body)),
                    }
                }
            })
        })
        .collect()
}

/// The script that writes every configuration: `base-<name>.<ext>` each,
/// as the export `draft` says, of `bodies` or of every body an export of
/// everything takes when `None`.
pub(crate) fn every_configuration_script(
    base: &str,
    draft: &ExportDraft,
    bodies: Option<&[uuid::Uuid]>,
) -> String {
    let mut args = crate::app::scripts::export_args(draft);
    if let Some(bodies) = bodies {
        let ids = bodies.iter().map(|b| b.to_string().into()).collect();
        args.insert("bodies".into(), serde_json::Value::Array(ids));
    }
    let options = lua_table(&args);
    let ext = draft.format.extension();
    format!(
        r#"-- Export every configuration
local base, ext, options = {base:?}, {ext:?}, {options}
local table = pc.config.list()
local was = table.active
for _, row in ipairs(table.rows) do
  pc.config.activate{{name = row.name}}
  pc.doc.rebuild()
  local file = base .. "-" .. row.name:gsub("[^%w%-_. ]", "-") .. "." .. ext
  options.path = file
  pc.file.export(options)
  print("Wrote " .. file)
end
pc.config.activate{{name = was}}
"#
    )
}

/// A command's arguments as a Lua table: strings, numbers, booleans and
/// lists of them.
fn lua_table(args: &core_document::CommandArgs) -> String {
    fn lua(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(s) => format!("{s:?}"),
            serde_json::Value::Array(items) => {
                let items: Vec<String> = items.iter().map(lua).collect();
                format!("{{{}}}", items.join(", "))
            }
            other => other.to_string(),
        }
    }
    let fields: Vec<String> = args
        .iter()
        .map(|(k, v)| format!("{k} = {}", lua(v)))
        .collect();
    format!("{{{}}}", fields.join(", "))
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
        words.push(local_ipc::SYSTEM_OPENER.to_string());
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

/// The slicers this run started that are still running, and when the
/// latest was handed its file.
static SLICERS: std::sync::Mutex<(usize, Option<std::time::Instant>)> =
    std::sync::Mutex::new((0, None));

/// How long a slicer handed a file is given to read it. A slicer started
/// through the system's opener outlives the process this run watches, so
/// the time is what keeps its file until it has loaded.
const SLICER_READS_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether the files handed to slicers may go: none of the slicers
/// started is still running (one may load its file again), and the
/// latest was handed its file long enough ago to have read it.
fn slicer_files_may_go(
    running: usize,
    handed: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    running == 0 && handed.is_none_or(|at| now.duration_since(at) >= SLICER_READS_WITHIN)
}

/// Start the slicer on `file` beside the app, and say which program ran.
fn open_in_slicer(command: &str, file: &Path) -> std::io::Result<String> {
    let (program, args) = slicer_invocation(command, file);
    let mut child = local_ipc::background(&mut std::process::Command::new(&program))
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    {
        let mut slicers = SLICERS.lock().unwrap_or_else(|e| e.into_inner());
        slicers.0 += 1;
        slicers.1 = Some(std::time::Instant::now());
    }
    // Collected when it exits, so it never lingers as a finished process.
    std::thread::spawn(move || {
        let _ = child.wait();
        let mut slicers = SLICERS.lock().unwrap_or_else(|e| e.into_inner());
        slicers.0 = slicers.0.saturating_sub(1);
    });
    Ok(program)
}

/// The bodies an export of `draft` writes: the copies of `layout` when
/// given (laid out of the parts `listed` alone, when listed), else those
/// `listed`, else every visible body with geometry that
/// no bench calls bought; their textures with them when the draft presses
/// them into a mesh format. The dialog, the slicer and a script's export
/// all take their bodies here.
fn owned_bodies(
    (document, registry): (&core_document::Document, &core_document::DocumentService),
    draft: &ExportDraft,
    listed: Option<&[core_document::BodyId]>,
    layout: Option<&doc_render::print_layout::Layout>,
) -> Vec<OwnedBody> {
    let textured = draft.textures && draft.format.is_mesh();
    if let Some(layout) = layout {
        return layout_bodies(document, layout, textured);
    }
    let not_made = match listed {
        Some(_) => Vec::new(),
        None => registry.not_printed(document),
    };
    bodies_to_export(
        document,
        |id| match listed {
            Some(list) => list.contains(&id),
            None => document.imported_body_effective_visible(id) && !not_made.contains(&id),
        },
        textured,
    )
}

/// Write what `draft` asks for to `path` now, on this thread: `bodies` of
/// `document` (every one an export of everything takes when `None`), or
/// the copies of `layout` when given (those of `bodies`' parts alone, when
/// both are). A mesh format makes up for the printer as `compensation`
/// says, when given. With `draft.per_body`, `path` is the folder each
/// body's file goes in. What a script's export does: each file written,
/// or the first that failed.
pub(crate) fn export_document(
    (document, registry): (&core_document::Document, &core_document::DocumentService),
    path: PathBuf,
    draft: &ExportDraft,
    bodies: Option<Vec<core_document::BodyId>>,
    layout: Option<&doc_render::print_layout::Layout>,
    compensation: Option<&settings::PrintCompensation>,
) -> Result<Vec<(PathBuf, kernel_ogeom::export::Exported)>, String> {
    let format = draft.format;
    let owned = owned_bodies((document, registry), draft, bodies.as_deref(), layout);
    if owned.is_empty() {
        return Err("there is nothing to export".to_string());
    }
    let compensation = compensation.filter(|_| format.is_mesh());
    let scale = compensation.map(|c| c.scale);
    let rounds = compensation
        .map(|c| [c.holes, c.bosses])
        .filter(|r| r.iter().any(|d| *d != 0.0));
    let files = export_files(&path, &owned, format, draft.per_body);
    let semantic = export_pmi(document, registry, &owned, format);
    write_files(&owned, &semantic, files, draft, (scale, rounds))
        .into_iter()
        .map(|(path, result)| {
            result
                .map(|exported| (path.clone(), exported))
                .map_err(|e| {
                    if draft.per_body {
                        format!("{}: {e}", path.display())
                    } else {
                        e
                    }
                })
        })
        .collect()
}

/// What a script's export answers: where it wrote (the folder, for a file
/// per body, with each file), and how many bodies, left out and triangles.
pub(crate) fn export_answer(
    folder: Option<&Path>,
    files: &[(PathBuf, kernel_ogeom::export::Exported)],
) -> serde_json::Value {
    let path = folder
        .map(Path::to_path_buf)
        .or_else(|| files.first().map(|(p, _)| p.clone()))
        .unwrap_or_default();
    let mut answer = serde_json::json!({
        "path": path.display().to_string(),
        "written": files.iter().map(|(_, e)| e.written).sum::<usize>(),
        "skipped": files.iter().flat_map(|(_, e)| e.skipped.clone()).collect::<Vec<_>>(),
        "triangles": files.iter().map(|(_, e)| e.triangles).sum::<usize>(),
    });
    if folder.is_some() {
        answer["files"] = files
            .iter()
            .map(|(p, _)| serde_json::json!(p.display().to_string()))
            .collect();
    }
    answer
}

/// The bodies of `document` that `take` accepts and that have geometry,
/// in tree order, their surface textures with them when `textured`.
fn bodies_to_export(
    document: &core_document::Document,
    take: impl Fn(core_document::BodyId) -> bool,
    textured: bool,
) -> Vec<OwnedBody> {
    let mut pictures = crate::app::textures::Pictures::new();
    let mut out: Vec<(usize, OwnedBody)> = document
        .derived()
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
                    body: *id,
                    name,
                    brep: document.derived().imported_brep_blob_arc(*id),
                    transform: {
                        let placement = document.body_placement(*id);
                        (!placement.is_identity()).then(|| placement.rows())
                    },
                    mesh: Arc::clone(&geometry.mesh),
                    pressing: textured
                        .then(|| crate::app::textures::Pressing::of(document, *id, &mut pictures))
                        .flatten(),
                    notes: body_notes(document, *id),
                    colors: Some(body_colors(document, *id)),
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

    /// A 10 mm box built by the kernel, its top face textured.
    fn textured_box(textured: bool) -> core_document::Document {
        use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
        let built = kernel_ogeom::OgeomKernel::new()
            .execute_solid_chain(kernel_ogeom::ChainRequest::new(
                &[SolidOp::Primitive {
                    kind: PrimitiveKind::Box {
                        length: 10.0,
                        width: 10.0,
                        height: 10.0,
                    },
                    placement: Placement::default(),
                    op: BooleanOp::NewSolid,
                }],
                &TessellationSettings::default(),
            ))
            .unwrap();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let mesh = Arc::new(built.mesh.clone());
        let top = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.faces)
            .find(|(t, _)| {
                t.iter()
                    .all(|i| (mesh.positions[*i as usize][2] - 10.0).abs() < 1e-4)
            })
            .map(|(_, f)| *f)
            .unwrap();
        doc.derived_mut()
            .set_imported_geometry(body, crate::test_support::geometry(Arc::clone(&mesh)));
        doc.derived_mut()
            .set_imported_brep_data(body, built.brep_blob, Vec::new());
        if textured {
            doc.set_body_textures(
                body,
                vec![core_document::FaceTexture {
                    texture: surface_texture::Texture {
                        tile_mm: 2.0,
                        depth_mm: 0.5,
                        ..Default::default()
                    },
                    faces: vec![core_document::FaceKey::of(&mesh, top)],
                }],
            );
        }
        doc
    }

    /// What exporting `doc` as `format` writes, the file taken away again.
    fn export(
        doc: &core_document::Document,
        format: ExportFormat,
        layout: Option<&doc_render::print_layout::Layout>,
    ) -> kernel_ogeom::export::Exported {
        let path = std::env::temp_dir().join(format!("export-{}", uuid::Uuid::new_v4()));
        let mut files = export_document(
            (doc, &core_document::DocumentService::default()),
            path,
            &ExportDraft {
                format,
                ..Default::default()
            },
            None,
            layout,
            None,
        )
        .unwrap();
        let (path, exported) = files.remove(0);
        let _ = std::fs::remove_file(path);
        exported
    }

    /// A binary STL's triangle count and the corners of its bounds.
    fn stl_bounds(bytes: &[u8]) -> (usize, [f32; 3], [f32; 3]) {
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for t in 0..count {
            for v in 0..3 {
                let at = 84 + t * 50 + 12 + v * 12;
                for k in 0..3 {
                    let c =
                        f32::from_le_bytes(bytes[at + k * 4..at + k * 4 + 4].try_into().unwrap());
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
        }
        (count, lo, hi)
    }

    /// An STL of a textured box carries the texture: many more triangles,
    /// and the top raised by up to the texture's depth.
    #[test]
    fn a_mesh_export_presses_the_texture_in() {
        let top_of = |bytes: &[u8]| stl_bounds(bytes).2[2];
        let plain = export(&textured_box(false), ExportFormat::Stl, None);
        let textured = export(&textured_box(true), ExportFormat::Stl, None);
        assert!(
            textured.triangles > plain.triangles * 20,
            "{} vs {}",
            textured.triangles,
            plain.triangles
        );
        let (plain_top, textured_top) = (top_of(&plain.bytes), top_of(&textured.bytes));
        assert!((plain_top - 10.0).abs() < 1e-3, "{plain_top}");
        assert!(
            textured_top > 10.3 && textured_top <= 10.5 + 1e-3,
            "{textured_top}"
        );
    }

    /// An export of the print layout writes every copy where the layout
    /// puts it: three boxes side by side on the bed, the model untouched.
    #[test]
    fn a_layout_export_writes_each_copy_on_the_bed() {
        use doc_render::print_layout::{LayoutSettings, lay_out};
        let mut doc = textured_box(false);
        let body = doc.bodies()[0].id;
        doc.set_body_placement(
            body,
            core_document::BodyPlacement::new(
                glam::Quat::IDENTITY,
                glam::Vec3::new(500.0, 0.0, 40.0),
            ),
        );
        let parts = [core_document::PrintPart {
            name: "Box".into(),
            bodies: vec![body],
            count: 3,
        }];
        let settings = LayoutSettings {
            bed_mm: [100.0, 100.0, 100.0],
            origin_center: false,
            gap_mm: 4.0,
        };
        let layout = lay_out(&doc, &parts, &settings);
        let exported = export(&doc, ExportFormat::Stl, Some(&layout));
        assert_eq!(exported.written, 3);
        let (_, lo, hi) = stl_bounds(&exported.bytes);
        // Three 10 mm boxes 4 mm apart, 2 mm in from the bed's corner.
        assert!(
            (lo[0] - 2.0).abs() < 1e-3 && (hi[0] - 40.0).abs() < 1e-3,
            "{lo:?} {hi:?}"
        );
        assert!(lo[2].abs() < 1e-3 && (hi[2] - 10.0).abs() < 1e-3);
        assert!(
            doc.body_placement(body).offset().x > 499.0,
            "the body stays put"
        );
    }

    /// A binary STL's triangles, each its three corners.
    fn stl_triangles(bytes: &[u8]) -> Vec<[[f32; 3]; 3]> {
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        let float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        (0..count)
            .map(|t| {
                let at = 84 + t * 50 + 12;
                std::array::from_fn(|v| std::array::from_fn(|k| float(at + v * 12 + k * 4)))
            })
            .collect()
    }

    /// The corners of the triangles of `bytes` whose first corner's x
    /// lies in `range`, each moved by `map`.
    fn corners_in(
        bytes: &[u8],
        range: std::ops::Range<f32>,
        map: impl Fn([f32; 3]) -> [f32; 3],
    ) -> Vec<[f32; 3]> {
        stl_triangles(bytes)
            .into_iter()
            .filter(|t| range.contains(&t[0][0]))
            .flatten()
            .map(map)
            .collect()
    }

    /// Whether two lists of corners are the same mesh, whatever the
    /// triangles' order and winding: as many corners, each of `a` within a
    /// micrometre of one of `b`.
    fn same_corners(a: &[[f32; 3]], b: &[[f32; 3]]) -> bool {
        let cell = |p: [f32; 3]| p.map(|c| (c * 1000.0).round() as i64);
        let cells: std::collections::HashSet<[i64; 3]> = b.iter().map(|p| cell(*p)).collect();
        a.len() == b.len()
            && a.iter().all(|p| {
                let [x, y, z] = cell(*p);
                (-1..=1).any(|dx| {
                    (-1..=1).any(|dy| (-1..=1).any(|dz| cells.contains(&[x + dx, y + dy, z + dz])))
                })
            })
    }

    /// A textured box's array of copies exports textured: every copy as
    /// the box itself, through a plain export and the print layout alike,
    /// whichever of the part's bodies the layout prints.
    #[test]
    fn an_array_of_a_textured_body_exports_every_copy_textured() {
        use doc_render::print_layout::{LayoutSettings, lay_out};
        let single = export(&textured_box(true), ExportFormat::Stl, None);
        let mut doc = textured_box(true);
        let body = doc.bodies()[0].id;
        doc.create_copy_array(
            body,
            core_document::ArrayPattern::Linear {
                direction: [1.0, 0.0, 0.0],
                spacing: 20.0,
            },
            3,
        )
        .unwrap();
        let copies = doc.copies_of(body);
        assert_eq!(copies.len(), 2);

        let plain = export(&doc, ExportFormat::Stl, None);
        assert_eq!(plain.written, 3);
        assert_eq!(
            plain.triangles,
            single.triangles * 3,
            "each copy is textured"
        );
        let source = corners_in(&single.bytes, -1.0..11.0, |p| p);
        for n in 1..=2 {
            let x = 20.0 * n as f32;
            let copy = corners_in(&plain.bytes, x - 1.0..x + 11.0, |p| [p[0] - x, p[1], p[2]]);
            assert!(
                same_corners(&copy, &source),
                "copy {n} is the box, texture and all"
            );
        }

        // The layout's part lists a copy first: it prints that body's shape.
        let parts = [core_document::PrintPart {
            name: "Box".into(),
            bodies: vec![copies[0], body, copies[1]],
            count: 3,
        }];
        let settings = LayoutSettings {
            bed_mm: [100.0, 100.0, 100.0],
            origin_center: false,
            gap_mm: 4.0,
        };
        let layout = lay_out(&doc, &parts, &settings);
        assert_eq!(layout.pieces[0].body, copies[0]);
        let laid = export(&doc, ExportFormat::Stl, Some(&layout));
        assert_eq!(laid.written, 3);
        assert_eq!(laid.triangles, plain.triangles, "the layout agrees");
    }

    /// A mirrored copy of a textured box exports the mirror image of the
    /// box's texture, not the pattern pressed afresh where the copy lies.
    #[test]
    fn a_mirrored_copy_exports_its_source_texture_mirrored() {
        use kernel_api::KernelQueries;
        let mut doc = textured_box(true);
        let body = doc.bodies()[0].id;
        // A texture whose pattern is not symmetric about the plane: a turn
        // of 30° on the knurl.
        let mut textures = doc.bodies()[0].textures.clone();
        textures[0].texture.rotation_deg = 30.0;
        doc.set_body_textures(body, textures);
        let plane = core_document::MirrorPlane {
            point: [17.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        let copy = doc.create_mirrored_copy(body, plane, None).unwrap();
        for (id, from, local) in doc.copies_awaiting_shape() {
            let blob = kernel_ogeom::QUERIES
                .mirror(
                    &from,
                    local.point.map(f64::from),
                    local.normal.map(f64::from),
                )
                .unwrap();
            assert!(doc.derived_mut().set_mirrored_shape(id, &from, blob));
        }
        assert!(doc.derived().imported_brep_blob_arc(copy).is_some());

        let exported = export(&doc, ExportFormat::Stl, None);
        assert_eq!(exported.written, 2);
        let source = corners_in(&exported.bytes, -1.0..11.0, |p| p);
        let mirrored = corners_in(&exported.bytes, 23.0..35.0, |p| [34.0 - p[0], p[1], p[2]]);
        assert!(source.len() > 1000, "the box is textured");
        assert!(
            same_corners(&mirrored, &source),
            "the copy is the box mirrored, texture and all"
        );
    }

    /// STEP carries drawn thread callouts and the benches' semantic PMI;
    /// both refer to the placed body's faces.
    #[test]
    fn a_step_export_writes_thread_callouts_and_semantic_pmi() {
        let mut doc = textured_box(false);
        let body = doc.bodies()[0].id;
        doc.set_body_placement(
            body,
            core_document::BodyPlacement::new(
                glam::Quat::IDENTITY,
                glam::Vec3::new(100.0, 0.0, 0.0),
            ),
        );
        doc.derived_mut()
            .set_cosmetic_threads(vec![core_document::ThreadCallout {
                feature: core_document::FeatureId::new(),
                body,
                designation: "M6".into(),
                mouth: [5.0, 5.0, 10.0],
                axis: [0.0, 0.0, -1.0],
                radius: 2.5,
            }]);
        struct Bench;
        impl core_document::Workbench for Bench {
            fn descriptor(&self) -> core_document::WorkbenchDescriptor {
                core_document::WorkbenchDescriptor::new("test.pmi", "PMI", "")
            }
            fn configure(&self, _: &mut core_document::WorkbenchContext) {}
        }

        impl core_document::BenchFeatures for Bench {}

        impl core_document::BenchTools for Bench {}

        impl core_document::BenchPanels for Bench {}

        impl core_document::BenchBuild for Bench {}

        impl core_document::BenchParameters for Bench {}

        impl core_document::BenchOutput for Bench {
            fn semantic_pmi(
                &self,
                _: &core_document::Document,
                _: core_document::BodyId,
            ) -> core_document::SemanticPmi {
                core_document::SemanticPmi {
                    tolerances: vec![core_document::PmiTolerance {
                        kind: "flatness".into(),
                        magnitude: 0.05,
                        at: vec![[105.0, 5.0, 10.0]],
                        ..Default::default()
                    }],
                    ..Default::default()
                }
            }
        }

        impl core_document::BenchCommands for Bench {}
        let mut registry = core_document::DocumentService::default();
        registry.register_workbench(Box::new(Bench)).unwrap();
        let path = std::env::temp_dir().join(format!("export-pmi-{}.step", uuid::Uuid::new_v4()));
        let (_, exported) = export_document(
            (&doc, &registry),
            path.clone(),
            &ExportDraft {
                format: ExportFormat::Step,
                ..Default::default()
            },
            None,
            None,
            None,
        )
        .unwrap()
        .remove(0);
        std::fs::remove_file(path).unwrap();
        let text = String::from_utf8(exported.bytes).unwrap();
        assert!(text.contains("FLATNESS_TOLERANCE("));
        assert!(text.contains("DRAUGHTING_CALLOUT('M6 thread'"), "{text}");
        let (at, _) = doc.derived().cosmetic_threads()[0].leader();
        let rim = format!("({:?},", f64::from(at[0]) + 100.0);
        let list = text
            .lines()
            .find(|l| l.contains("COORDINATES_LIST('M6 thread'"))
            .expect("the leader is written");
        assert!(list.contains(&rim), "{list} starts at {rim}");
    }

    /// Every configuration is written by a script; its export reads back
    /// every option the dialog was confirmed with, whatever the file's
    /// extension says.
    #[test]
    fn the_every_configuration_script_carries_the_dialogs_options() {
        let mut draft = ExportDraft {
            format: ExportFormat::StepNurbs,
            textures: false,
            layout: true,
            ..Default::default()
        };
        draft.detail.chord_tolerance = 0.05;
        draft.detail.angular_tolerance_deg = 12.0;
        let args = crate::app::scripts::export_args(&draft);
        let back =
            crate::app::scripts::export_draft(&core_document::Args(&args), Path::new("a.step"))
                .unwrap();
        assert_eq!(back.format, ExportFormat::StepNurbs);
        assert!(!back.textures && back.layout);
        assert_eq!(back.detail.chord_tolerance, 0.05);
        assert_eq!(back.detail.angular_tolerance_deg, 12.0);
        let script = every_configuration_script("/t/a", &draft, None);
        assert!(script.contains(&lua_table(&args)), "{script}");
        assert!(script.contains("format = \"step_nurbs\""), "{script}");
    }

    /// A script's export of everything takes the bodies the dialog's does,
    /// leaving out what a bench says is bought, and presses textures in
    /// only when asked to.
    #[test]
    fn a_script_export_leaves_out_bought_bodies_and_unticked_textures() {
        struct Bought;
        impl core_document::Workbench for Bought {
            fn descriptor(&self) -> core_document::WorkbenchDescriptor {
                core_document::WorkbenchDescriptor::new("test.bought", "Bought", "")
            }
            fn configure(&self, _: &mut core_document::WorkbenchContext) {}
        }

        impl core_document::BenchFeatures for Bought {}

        impl core_document::BenchTools for Bought {}

        impl core_document::BenchPanels for Bought {}

        impl core_document::BenchBuild for Bought {}

        impl core_document::BenchParameters for Bought {}

        impl core_document::BenchOutput for Bought {
            fn not_printed(
                &self,
                document: &core_document::Document,
                _: &[core_document::WorkbenchId],
            ) -> Vec<core_document::BodyId> {
                document.bodies().iter().map(|b| b.id).collect()
            }
        }

        impl core_document::BenchCommands for Bought {}
        let doc = textured_box(true);
        let body = doc.bodies()[0].id;
        let mut registry = core_document::DocumentService::default();
        registry.register_workbench(Box::new(Bought)).unwrap();
        let stl = |textures: bool| ExportDraft {
            format: ExportFormat::Stl,
            textures,
            ..Default::default()
        };
        let path = std::env::temp_dir().join(format!("export-{}.stl", uuid::Uuid::new_v4()));
        let everything = export_document(
            (&doc, &registry),
            path.clone(),
            &stl(true),
            None,
            None,
            None,
        );
        assert_eq!(
            everything.err().as_deref(),
            Some("there is nothing to export")
        );

        let untextured = export_document(
            (&doc, &registry),
            path.clone(),
            &stl(false),
            Some(vec![body]),
            None,
            None,
        )
        .unwrap()
        .remove(0)
        .1;
        let plain = export(&textured_box(false), ExportFormat::Stl, None);
        let _ = std::fs::remove_file(&path);
        assert_eq!(untextured.triangles, plain.triangles);
    }

    #[test]
    fn the_slicer_command_takes_the_file_where_it_says_or_last() {
        let file = Path::new("/tmp/printcad/slicer/part.3mf");
        let (program, args) = slicer_invocation("", file);
        assert_eq!(program, local_ipc::SYSTEM_OPENER);
        assert_eq!(args, ["/tmp/printcad/slicer/part.3mf"]);
        let (program, args) = slicer_invocation("\"/opt/My Slicer/run\" --single", file);
        assert_eq!(program, "/opt/My Slicer/run");
        assert_eq!(args, ["--single", "/tmp/printcad/slicer/part.3mf"]);
        let (_, args) = slicer_invocation("slice --load={file} --go", file);
        assert_eq!(args, ["--load=/tmp/printcad/slicer/part.3mf", "--go"]);

        // Its files go at exit only once no slicer may still be reading one.
        let now = std::time::Instant::now();
        let long_ago = now.checked_sub(SLICER_READS_WITHIN * 2);
        assert!(slicer_files_may_go(0, None, now));
        assert!(slicer_files_may_go(0, long_ago, now));
        assert!(!slicer_files_may_go(0, Some(now), now), "just handed over");
        assert!(!slicer_files_may_go(1, long_ago, now), "still running");
    }

    /// A file per body is named after its body as a file name may be, a
    /// number added to a name taken already, whatever its case.
    #[test]
    fn a_file_per_body_is_named_after_its_body() {
        let folder = Path::new("/t/out");
        let names = ["Bracket", "bracket", "Lid/2", "Lid 2", "  "];
        let paths = per_body_paths(folder, &names, ExportFormat::ThreeMf);
        let files: Vec<_> = paths
            .iter()
            .map(|p| p.strip_prefix(folder).unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            files,
            [
                "Bracket.3mf",
                "bracket-2.3mf",
                "Lid-2.3mf",
                "Lid-2-2.3mf",
                "model.3mf"
            ]
        );
    }

    /// A file per body writes each body alone into the folder, every one
    /// whole, and the answer names each file.
    #[test]
    fn an_export_of_a_file_per_body_writes_each_alone() {
        let mut doc = core_document::Document::new("t");
        let tetra = |x: f32| TriMesh {
            positions: vec![
                [x, 0.0, 0.0],
                [x + 10.0, 0.0, 0.0],
                [x, 10.0, 0.0],
                [x, 0.0, 10.0],
            ],
            indices: vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
            ..Default::default()
        };
        for (name, x) in [("Left", 0.0), ("Right", 50.0)] {
            let body = doc.create_body(Some(name.into()));
            doc.derived_mut()
                .set_imported_geometry(body, crate::test_support::geometry(tetra(x)));
        }
        let folder = std::env::temp_dir().join(format!("export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let draft = ExportDraft {
            format: ExportFormat::Stl,
            per_body: true,
            ..Default::default()
        };
        let files = export_document(
            (&doc, &core_document::DocumentService::default()),
            folder.clone(),
            &draft,
            None,
            None,
            None,
        )
        .unwrap();
        let answer = export_answer(Some(&folder), &files);
        let written: Vec<(String, usize, f32)> = files
            .iter()
            .map(|(path, exported)| {
                let bytes = std::fs::read(path).unwrap();
                let (count, lo, _) = stl_bounds(&bytes);
                assert_eq!(count, exported.triangles);
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name, exported.written, lo[0])
            })
            .collect();
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(
            written,
            [("Left.stl".into(), 1, 0.0), ("Right.stl".into(), 1, 50.0)]
        );
        assert_eq!(answer["written"], 2);
        assert_eq!(answer["files"].as_array().map(Vec::len), Some(2));
        assert_eq!(answer["path"], folder.display().to_string());
    }

    /// The selection's parts are those with a selected body among theirs,
    /// a copy's selection taking its source's part.
    #[test]
    fn the_selection_keeps_the_parts_it_touches() {
        let id = || core_document::BodyId(uuid::Uuid::new_v4());
        let (bracket, copy, lid) = (id(), id(), id());
        let part = |name: &str, bodies: Vec<core_document::BodyId>| core_document::PrintPart {
            name: name.into(),
            bodies,
            count: 1,
        };
        let parts = vec![part("Bracket", vec![bracket, copy]), part("Lid", vec![lid])];
        let kept = parts_of(parts, &[copy]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "Bracket");
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
