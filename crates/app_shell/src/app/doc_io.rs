//! Document file I/O: open/save/dialog plumbing and shared helpers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use rfd::FileDialog;

use crate::app::view_fit::{aabb_fit_center_radius, document_imported_aabb};
use crate::log_panel as app_log;
use crate::ui::TreeItemId;
use crate::{Document, PrintCadApp};

/// Derive a user-facing document name from a file name by stripping the
/// known document extensions (case-insensitively), longest match first.
/// Returns the original name when no known extension matches.
pub(crate) fn document_name_from_file_name(file_name: &str) -> &str {
    const SUFFIXES: [&str; 4] = [".prtcad.zst", ".prtcad.gz", ".prtcad", ".json"];
    let lowered = file_name.to_ascii_lowercase();
    for suffix in SUFFIXES {
        if let Some(stripped) = lowered.strip_suffix(suffix) {
            return &file_name[..stripped.len()];
        }
    }
    file_name
}

/// A document read from a file, made ready to build: its features' saved
/// dirty flags are the saving run's, so every bench marks its own features
/// due and they build again here, a feature that failed then failing again
/// with its error, the solid saved standing in until the builds land. It
/// is unedited.
pub(crate) fn opened(
    registry: &core_document::DocumentService,
    mut document: Document,
) -> Document {
    registry.invalidate_all(&mut document);
    document.mark_clean();
    document
}

/// The on-disk recent list; missing or unreadable means empty.
pub(crate) fn load_recent() -> settings::recent::RecentStore {
    settings::SettingsStore::recent_file_path()
        .map(|p| settings::recent::RecentStore::load(&p))
        .unwrap_or_default()
}

/// A parsed document on its way back from the open worker.
pub(crate) struct OpenJob {
    token: u64,
    path: PathBuf,
    result: Result<Document, String>,
}

/// A packed archive on its way back from the save worker.
pub(crate) struct SaveJob {
    path: std::path::PathBuf,
    at_seq: u64,
    result: Result<Vec<u8>, String>,
}

/// How far the worker has got, shared with the UI thread.
#[derive(Default)]
pub(crate) struct SaveProgress {
    done: AtomicU64,
    total: AtomicU64,
}

impl SaveProgress {
    fn set(&self, done: u64, total: u64) {
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    pub(crate) fn read(&self) -> Option<(u64, u64)> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed), total))
    }
}

pub(crate) enum FileDialogKind {
    Open,
    Save,
    SaveAs,
    ImportStep,
    Export(kernel_ogeom::export::ExportFormat),
    RunScript,
    /// Files to go with a chat's next prompt, by chat id.
    Attach(String),
    /// A file a bench made, written where the user says.
    SaveFile(Box<FileToSave>),
    /// Files a bench made, written into the folder the user picks.
    SaveFiles(Vec<(String, Vec<u8>)>),
    /// A file, or several, the user picks for a bench to read.
    BenchOpen(Box<BenchOpen>),
    /// A bench's animation, rendered and written where the user says.
    SaveAnimation(Box<crate::app::animation::Animation>),
    /// A workbench package to install.
    InstallPackage,
    /// A file whose first solid becomes the body's shape.
    ReplaceShape(core_document::BodyId),
    /// A picture for a texture's pattern: the body and which texture.
    TexturePicture(core_document::BodyId, usize),
}

/// A file a bench asked to save: its suggested name, the dialog's filter
/// and its bytes.
pub(crate) struct FileToSave {
    pub name: String,
    pub kind: String,
    pub extension: String,
    pub contents: Vec<u8>,
}

/// What a bench asked to be handed: the request's id, the kinds of file
/// offered, and whether several may be picked.
pub(crate) struct BenchOpen {
    pub bench: core_document::WorkbenchId,
    pub id: String,
    pub kinds: Vec<(String, Vec<String>)>,
    pub multiple: bool,
}

/// The most a bench is handed of the files it asked to open, in all.
const BENCH_OPEN_LIMIT: u64 = 64 * 1024 * 1024;

/// Whether `name` is a plain file name, with no folder in it.
fn plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

pub(crate) struct FileDialogResult {
    kind: FileDialogKind,
    /// What was picked: one path, or several for an attachment.
    paths: Vec<PathBuf>,
}

impl PrintCadApp {
    fn document_has_asset_files(&self) -> bool {
        self.session.document.assets().next().is_some()
    }

    /// Save to the tab's file or prompt for a path. Returns false if cancelled or save fails.
    pub(crate) fn save_document_interactive(&mut self) -> bool {
        // A read-only file is not written in place: it asks for a name.
        let in_place = if self.current_file_read_only() {
            None
        } else {
            self.session.file.path.clone()
        };
        let path = if let Some(p) = in_place {
            p
        } else {
            let mut dialog = FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]);
            if let Some(recent_dir) = self.files.recent.last_dir.clone() {
                dialog = dialog.set_directory(recent_dir);
            }
            match dialog.set_file_name("untitled.prtcad").save_file() {
                Some(p) => p,
                None => return false,
            }
        };
        match self.save_document_at(&path) {
            Ok(()) => true,
            Err(err) => {
                app_log::error(format!("Save failed: {err:#}"));
                false
            }
        }
    }

    /// A blank document on screen: the active tab when nothing has
    /// happened in it, a new tab beside it otherwise.
    pub(crate) fn reset_to_new_document(&mut self) {
        self.ensure_fresh_tab();
        self.session.screen = crate::ui::Screen::Workspace;
        app_log::info("New document");
    }

    /// Front the recent list with `path` and write it out.
    pub(crate) fn touch_recent(&mut self, path: &Path) {
        self.files.recent.touch(path);
        self.save_recent();
    }

    /// Remember only the directory of `path`, for files that are not
    /// documents (STEP imports).
    pub(crate) fn remember_recent_dir(&mut self, path: &Path) {
        self.files.recent.remember_dir(path);
        self.save_recent();
    }

    pub(crate) fn remove_recent(&mut self, path: &Path) {
        self.files.recent.remove(path);
        self.save_recent();
    }

    fn save_recent(&self) {
        if let Ok(recent_path) = settings::SettingsStore::recent_file_path()
            && let Err(err) = self.files.recent.save(&recent_path)
        {
            app_log::error(format!("Failed to write the recent list: {err}"));
        }
    }

    /// The server hands over opaque bytes; the client owns the parsing.
    fn parse_document_bytes(path: &Path, bytes: Vec<u8>) -> Result<Document> {
        let document = match path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ext) if ext == "json" => {
                serde_json::from_slice(&bytes).with_context(|| "Failed to parse document JSON")?
            }
            _ => Document::load_from_bytes(bytes)
                .with_context(|| format!("Failed to parse .prtcad document {}", path.display()))?,
        };
        Ok(document)
    }

    /// Replace the in-memory document after a successful load (UI thread).
    fn apply_opened_document(&mut self, path: PathBuf, document: Document) {
        // The outgoing document's held ops go with it, not with the new one.
        self.session.send_outbox();
        self.session.document = opened(&self.registry, document);
        self.session.file.path = Some(path.clone());
        // The chats kept with the file come back, resting until shown.
        self.restore_chats();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        self.session
            .document
            .set_name(document_name_from_file_name(file_name));
        self.session.active_document_object = None;
        self.session.active_body_id = None;
        self.session.selection.tree = Some(TreeItemId::DocumentRoot);
        self.session.selection.body = None;

        self.session.document.mark_clean();
        self.touch_recent(&path);
        self.session.screen = crate::ui::Screen::Workspace;
        // Reframe on the imported meshes' bounds, as a STEP import does, so the
        // scene bounds and the automatic near/far planes agree with the view;
        // keeping the old eye and target would clip the model at the margins.
        // Framed now, and again once the workspace's viewport is laid out
        // and the bodies have their geometry. A file opened before comes
        // back in the view it was left in instead.
        let restored = self.restore_kept_view(&path);
        self.session.view.fit_on_layout = !restored && !self.session.document.bodies().is_empty();
        if !restored {
            if document_imported_aabb(&self.session.document).is_some() {
                self.frame_scene();
            } else {
                self.session.view.camera.clear_scene_zoom_constraint();
                self.session
                    .view
                    .camera
                    .clamp_focal_to_settings(&self.user_settings.camera);
            }
        }
        self.session.journal.reset(&mut self.session.document);
        // A fresh baseline: what the server logged before describes another
        // state than this client's. (set_name above records an op into the
        // new document; it flows normally on the next drain.)
        self.session
            .connection
            .server
            .send(core_document::server::ClientMessage::Rebase);
        app_log::info(format!("Opened document from {}", path.display()));
        self.note_opened_file(&path);
        self.report_missing_packages();
    }

    /// Move the server connection to `socket`: the document's own daemon
    /// (one socket per file) or the session's untitled one.
    /// The old connection is flushed first so no queued write is abandoned;
    /// on failure the old connection stays and the move is only logged: a
    /// working degraded connection beats a broken fresh one.
    pub(crate) fn switch_server_to(&mut self, socket: std::path::PathBuf) {
        if self.session.connection.socket == socket
            && self.session.connection.server.status().connected
        {
            return;
        }
        match crate::app::server::connect(&socket) {
            Ok(server) => {
                self.session.send_outbox();
                self.session.connection.server.flush();
                self.session.connection.server = server;
                self.session.connection.socket = socket;
                // The old connection's peers are not this one's.
                self.session.connection.peer_presence.clear();
                self.session.connection.last_sent_presence = None;
                app_log::info(format!(
                    "Document server: {}",
                    self.session.connection.server.name()
                ));
            }
            Err(err) => {
                app_log::warn(format!(
                    "Could not reach document daemon at {}: {err}; keeping {}",
                    socket.display(),
                    self.session.connection.server.name()
                ));
            }
        }
    }

    /// Gentle self-healing: when the connection is degraded, retry the
    /// expected socket every few seconds. Also upgrades a DirectFiles
    /// fallback to a real daemon once one can be spawned.
    pub(crate) fn maybe_reconnect_server(&mut self) {
        if self.session.connection.server.status().connected {
            return;
        }
        // Nobody is on the other end; stale tints would lie.
        self.session.connection.peer_presence.clear();
        let due = self
            .session
            .connection
            .last_reconnect
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5));
        if !due {
            return;
        }
        self.session.connection.last_reconnect = Some(std::time::Instant::now());
        match crate::app::server::connect(&self.session.connection.socket.clone()) {
            Ok(server) => {
                self.session.connection.server = server;
                app_log::info("Document server reconnected");
            }
            Err(err) => {
                tracing::debug!("server reconnect attempt failed: {err}");
            }
        }
    }

    /// Import a file picked or dropped: an STL after asking its units
    /// (unless Preferences says not to ask), another mesh or a file a
    /// workbench reads at once, a STEP or IGES file through the import
    /// settings, which its meshing needs.
    pub(crate) fn import_picked(&mut self, path: PathBuf) {
        if crate::app::scans::has_no_units(&path) {
            if self.user_settings.files.ask_mesh_units {
                self.session.file.mesh_units_pending = Some(path);
            } else {
                self.imports
                    .units
                    .insert(path.clone(), self.user_settings.files.mesh_units);
                self.import_step_at(&path, self.imports.last_detail.clone());
            }
        } else if kernel_ogeom::is_mesh_file(&path)
            || self.registry.file_import_for(&path).is_some()
        {
            self.import_step_at(&path, self.imports.last_detail.clone());
        } else {
            self.session.file.step_import_pending = Some((path, self.imports.last_detail.clone()));
        }
    }

    /// The answer to what units a waiting STL is in: it imports in them,
    /// and they become the answer offered next (or every STL's, when the
    /// question is turned off). Whether the settings changed.
    pub(crate) fn answer_mesh_units(
        &mut self,
        answer: Option<(kernel_api::LengthUnit, bool)>,
    ) -> bool {
        let Some(path) = self.session.file.mesh_units_pending.take() else {
            return false;
        };
        let Some((unit, quiet)) = answer else {
            return false;
        };
        let files = &mut self.user_settings.files;
        let changed = files.mesh_units != unit || (quiet && files.ask_mesh_units);
        files.mesh_units = unit;
        files.ask_mesh_units &= !quiet;
        self.imports.units.insert(path.clone(), unit);
        self.import_step_at(&path, self.imports.last_detail.clone());
        changed
    }

    /// Files dropped on the window: a printCAD document opens in a tab of
    /// its own, a model file imports as File › Import would take it.
    pub(crate) fn open_dropped(&mut self, paths: Vec<PathBuf>) {
        const MODELS: &[&str] = &[
            "step", "stp", "iges", "igs", "stl", "obj", "3mf", "ply", "glb", "gltf", "wrl", "vrml",
        ];
        for path in paths {
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            if extension == "prtcad" {
                self.open_document_at(path);
            } else if MODELS.contains(&extension.as_str())
                || self.registry.file_import_for(&path).is_some()
            {
                self.import_picked(path);
            } else {
                app_log::warn(format!(
                    "printCAD neither opens nor imports `{}`",
                    path.display()
                ));
            }
        }
    }

    pub(crate) fn open_document_at(&mut self, path: PathBuf) {
        // A file already open is that tab; otherwise it gets a blank one.
        if let Some(index) = self.tab_index_of_file(&path) {
            self.switch_tab(index);
            self.session.screen = crate::ui::Screen::Workspace;
            return;
        }
        self.ensure_fresh_tab();
        self.request_document_open(path);
    }

    /// Ask the server for a document's bytes. The load epoch rides as the
    /// request token so a response landing after File > New is ignored.
    fn request_document_open(&mut self, path: PathBuf) {
        app_log::info(format!("Opening `{}`...", path.display()));
        // The document's own daemon owns its file. Ask it, not the session
        // daemon.
        self.switch_server_to(crate::app::server::socket_for(&path));
        self.session
            .connection
            .server
            .send(core_document::server::ClientMessage::OpenDocument {
                path,
                token: self.session.connection.load_epoch,
            });
    }

    /// Apply everything the server answered since last frame: opened
    /// documents (parsed here: the server serves bytes, never meaning) and
    /// save completions, whose `at_seq` decides whether the document is
    /// truly clean or was edited mid-save.
    pub(crate) fn drain_server_messages(&mut self) {
        use core_document::server::ServerMessage;
        self.maybe_reconnect_server();
        for message in self.session.connection.server.poll() {
            match message {
                ServerMessage::HelloOk { .. } => {}
                ServerMessage::Peers { peers } => {
                    app_log::info(if peers == 0 {
                        "Editing alone".to_string()
                    } else {
                        format!(
                            "{peers} other editor{} on this document",
                            if peers == 1 { "" } else { "s" }
                        )
                    });
                }
                ServerMessage::Ops { actor, ops } => {
                    if self.session.connection.server.status().opens_in_flight > 0 {
                        // An open will replace the document; these ops
                        // describe the incoming one. Apply them after it.
                        self.session.connection.held_remote_ops.push((actor, ops));
                    } else {
                        self.apply_remote_ops(actor, ops);
                    }
                }
                ServerMessage::PresencePeer { actor, state } => {
                    self.session.connection.peer_presence.insert(actor, state);
                }
                ServerMessage::PresenceGone { actor } => {
                    self.session.connection.peer_presence.remove(&actor);
                }
                ServerMessage::Opened { token, path, bytes } => {
                    if token != self.session.connection.load_epoch {
                        continue;
                    }
                    self.start_document_parse(token, path, bytes);
                }
                ServerMessage::OpenFailed { token, path, error } => {
                    self.session.connection.held_remote_ops.clear();
                    if token != self.session.connection.load_epoch {
                        continue;
                    }
                    app_log::error(format!(
                        "Failed to open document {}: {error}",
                        path.display()
                    ));
                }
                ServerMessage::SaveCompleted { path, at_seq } => {
                    // Only call the document clean if nothing was edited
                    // while the write was in flight; otherwise those edits
                    // would be silently marked as saved.
                    if at_seq == self.session.document.mutation_seq() {
                        self.session.document.mark_clean();
                        // Saved, the tab needs no copy to come back from.
                        self.autosave.saved.remove(&self.session.tab);
                        crate::app::recovery::forget(self.session.tab);
                    }
                    self.session.file.path = Some(path.clone());
                    self.note_saved_file();
                    // Its chats are kept with the file it is saved as.
                    self.persist_chats();
                    self.touch_recent(&path);
                    app_log::info(format!("Saved document to {}", path.display()));
                }
                ServerMessage::SaveFailed { path, error } => {
                    app_log::error(format!("Failed to save {}: {error}", path.display()));
                }
            }
        }
    }

    pub(crate) fn save_document_at(&mut self, path: &Path) -> Result<()> {
        if crate::platform::read_only(path) {
            return Err(anyhow::anyhow!(
                "{} is read-only: save the document under another name",
                path.display()
            ));
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let lowered = file_name.to_ascii_lowercase();
        self.session
            .document
            .set_name(document_name_from_file_name(file_name));

        let ext_lower = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if matches!(ext_lower.as_deref(), Some("json")) && self.document_has_asset_files() {
            return Err(anyhow::anyhow!(
                "this document has embedded assets (an imported STEP or IGES file, say), which \
                 JSON cannot hold; save it as .prtcad"
            ));
        }

        // For legacy .json files, keep writing plain JSON.
        // For everything else, use the .prtcad tar-based container with optional compression.
        match path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ext) if ext == "json" => {
                let file = std::fs::File::create(path).with_context(|| {
                    format!("Failed to create document file {}", path.display())
                })?;
                serde_json::to_writer_pretty(file, &self.session.document)
                    .with_context(|| "Failed to serialize document")?;
            }
            _ => {
                let compression = if lowered.ends_with(".prtcad.gz") || lowered.ends_with(".gz") {
                    core_document::Compression::Gzip
                } else if lowered.ends_with(".prtcad.zst") || lowered.ends_with(".zst") {
                    core_document::Compression::Zstd
                } else {
                    core_document::Compression::None
                };

                // A `.prtcad` carries every snapshot blob and the source
                // file, so packing one takes seconds on a large import,
                // long enough that it cannot happen on the UI thread. The
                // clone is cheap (payloads sit behind Arcs), a worker packs
                // the archive, and the bytes go to the server when it
                // finishes; the server owns the write itself.
                if self.session.file.path.as_deref() != Some(path) {
                    // Save As gives the document a new identity, and a new
                    // daemon to own it.
                    self.switch_server_to(crate::app::server::socket_for(path));
                }
                self.start_document_save(path, compression);
                return Ok(());
            }
        }

        self.session.file.path = Some(path.to_path_buf());
        self.note_saved_file();
        // Its chats are kept with the file it is saved as.
        self.persist_chats();
        self.touch_recent(path);
        self.session.document.mark_clean();
        app_log::info(format!("Saved document to {}", path.display()));
        Ok(())
    }

    /// Apply a peer's relayed edits. The ops are resolved effects, replayed
    /// through the same `apply_op` path the determinism tests pin, and the
    /// daemon never echoes this client's own ops, so everything here is
    /// foreign. Marking dirty is apply-side policy: it is what makes this replica
    /// re-derive the geometry the peer's edit invalidated.
    fn apply_remote_ops(&mut self, _actor: uuid::Uuid, ops: Vec<core_document::op::DocumentOp>) {
        use core_document::op::DocumentOp as Op;
        if ops.is_empty() {
            return;
        }
        for op in &ops {
            self.session.document.apply_remote_op(op);
            if let Op::ImportModel {
                asset,
                bytes,
                detail,
                bodies,
                ..
            } = op
            {
                // The op created the bodies; their geometry is derived state,
                // computed again from the carried bytes. The kernel import
                // is deterministic, so meshes land on the peer's
                // pre-allocated body ids by import order.
                // The kernel picks its reader by extension: stage the bytes
                // under the one the asset was imported with.
                let extension = std::path::Path::new(&asset.path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("step")
                    .to_ascii_lowercase();
                let temp = std::env::temp_dir()
                    .join(format!("printcad_remote_{}.{extension}", asset.id.simple()));
                match std::fs::write(&temp, bytes.as_slice()) {
                    Ok(()) => {
                        self.session.connection.remote_import_routes.insert(
                            temp.clone(),
                            crate::RemoteImportRoute {
                                body_ids: bodies.iter().map(|b| b.id).collect(),
                                asset_id: asset.id,
                            },
                        );
                        self.imports.owner.insert(temp.clone(), self.session.tab);
                        self.kernel_worker.request_step_import(temp, detail.clone());
                    }
                    Err(err) => {
                        app_log::error(format!("Remote import: could not stage bytes: {err}"));
                    }
                }
            }
        }
        // Per-user undo survives foreign edits: the journal holds only this
        // user's gestures, and their inverses touch only what those touched.
        app_log::info(format!("{} remote edit(s) applied", ops.len()));
    }

    /// Tell the server what this user has selected, only when it changed.
    /// The display name is the login name.
    pub(crate) fn publish_presence(&mut self) {
        // Centimetre quantization: enough to follow a hand, coarse enough
        // that breathing on the mouse does not broadcast.
        let cursor_world = self.session.hover.world_pos.map(|p| {
            [
                (p[0] * 0.1).round() * 10.0,
                (p[1] * 0.1).round() * 10.0,
                (p[2] * 0.1).round() * 10.0,
            ]
        });
        let state = core_document::server::PresenceState {
            display_name: std::env::var("USER").unwrap_or_else(|_| "editor".to_string()),
            selected_body: self.session.selection.body,
            cursor_world,
        };
        if self.session.connection.last_sent_presence.as_ref() == Some(&state) {
            return;
        }
        self.session
            .connection
            .server
            .send(core_document::server::ClientMessage::Presence(
                state.clone(),
            ));
        self.session.connection.last_sent_presence = Some(state);
    }

    /// Parse opened bytes on a worker. Unpacking an archive costs what
    /// packing one does, so it does not belong on the UI thread either.
    fn start_document_parse(&mut self, token: u64, path: PathBuf, bytes: Vec<u8>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = crate::platform::spawn("printcad-document-open", move || {
            let parsed = Self::parse_document_bytes(&path, bytes);
            let _ = tx.send(OpenJob {
                token,
                path,
                result: parsed.map_err(|err| format!("{err:#}")),
            });
        });
        match spawned {
            Ok(_) => self.session.file.open_rx = Some(rx),
            Err(err) => app_log::error(format!("Failed to start the open: {err}")),
        }
    }

    /// Take the parsed document once the worker has it.
    pub(crate) fn drain_document_opens(&mut self) {
        let Some(rx) = self.session.file.open_rx.as_ref() else {
            return;
        };
        let job = match rx.try_recv() {
            Ok(job) => job,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.session.file.open_rx = None;
                app_log::error("The open worker stopped without parsing the document");
                return;
            }
        };
        self.session.file.open_rx = None;

        // Another open may have started while this one was parsing; the
        // token says whether this document is still the one being waited on.
        if job.token != self.session.connection.load_epoch {
            return;
        }
        match job.result {
            Ok(document) => {
                self.apply_opened_document(job.path, document);
                for (actor, ops) in std::mem::take(&mut self.session.connection.held_remote_ops) {
                    self.apply_remote_ops(actor, ops);
                }
            }
            Err(err) => app_log::error(format!(
                "Failed to open document {}: {err}",
                job.path.display()
            )),
        }
    }

    /// Pack the archive on a worker and hand the bytes to the server when it
    /// is done.
    fn start_document_save(&mut self, path: &Path, compression: core_document::Compression) {
        // The snapshot embodies every op so far; the server clears its
        // unsaved tail when the save lands, so they go to it first.
        self.session.send_outbox();
        let mut document = self.session.document.clone();
        // A body showing a feature's preview is saved whole.
        for (body, preview) in &self.session.builds.previews {
            crate::app::recompute::store_built_solid(&mut document, *body, preview.full.clone());
        }
        let at_seq = self.session.document.mutation_seq();
        let path = path.to_path_buf();
        let preview = self.thumbnail_shapes();
        let (forward, up) = self.session.view.camera.view_basis();
        let progress = Arc::new(SaveProgress::default());
        let worker_progress = Arc::clone(&progress);
        let (tx, rx) = std::sync::mpsc::channel();

        let spawned = crate::platform::spawn("printcad-document-save", move || {
            document.set_thumbnail(doc_render::thumbnail::render(&preview, forward, up));
            let packed = document.save_to_bytes_watched(compression, &move |done, total| {
                worker_progress.set(done, total);
            });
            let _ = tx.send(SaveJob {
                path,
                at_seq,
                result: packed.map_err(|err| err.to_string()),
            });
        });
        match spawned {
            Ok(_) => {
                app_log::info(format!("Saving `{}`…", self.session.document.name()));
                self.session.file.save_rx = Some(rx);
                self.session.file.save_progress = Some(progress);
            }
            Err(err) => app_log::error(format!("Failed to start the save: {err}")),
        }
    }

    /// The visible bodies in the colours they show in, for the preview a
    /// save carries.
    pub(crate) fn thumbnail_shapes(&self) -> Vec<doc_render::thumbnail::Shape> {
        let document = &self.session.document;
        document
            .imported_geometries()
            .filter(|(id, _)| document.imported_body_saved_visible(**id))
            .map(|(id, geometry)| {
                doc_render::thumbnail::preview_shape(document, *id, Arc::clone(&geometry.mesh))
            })
            .collect()
    }

    /// Take the packed archive once the worker has it.
    pub(crate) fn drain_document_saves(&mut self) {
        let Some(rx) = self.session.file.save_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(job) => {
                self.session.file.save_rx = None;
                self.session.file.save_progress = None;
                self.send_packed_document(job);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.session.file.save_rx = None;
                self.session.file.save_progress = None;
                app_log::error("The save worker stopped without packing the document");
            }
        }
    }

    fn send_packed_document(&mut self, job: SaveJob) {
        match job.result {
            Ok(bytes) => self.session.connection.server.send(
                core_document::server::ClientMessage::SaveDocument {
                    path: job.path,
                    bytes,
                    at_seq: job.at_seq,
                },
            ),
            Err(err) => app_log::error(format!("Failed to serialize document: {err}")),
        }
    }

    /// Every tab's saves, before the process ends (CLAUDE.md invariant).
    pub(crate) fn wait_for_all_document_saves(&mut self) {
        self.for_each_tab(|app| app.wait_for_document_saves());
        // On the way out, after the user answered for every tab: no copy is
        // left to come back from.
        for job in self.autosave.jobs.drain(..) {
            job.join();
        }
        for slot in &self.tabs {
            crate::app::recovery::forget(slot.tab);
        }
    }

    /// How long between autosaves; `None` when off. `PRINTCAD_AUTOSAVE_SECS`
    /// shortens it, for trying it out.
    fn autosave_every(&self) -> Option<std::time::Duration> {
        let secs = std::env::var("PRINTCAD_AUTOSAVE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::from(self.user_settings.autosave_minutes) * 60);
        (secs > 0).then(|| std::time::Duration::from_secs(secs))
    }

    /// When the next autosave is due: while a tab has edits its copy lacks,
    /// or a saved tab's copy is still to be taken away. The loop wakes for
    /// it though nothing else moves.
    pub(crate) fn next_autosave(&self) -> Option<std::time::Instant> {
        let every = self.autosave_every()?;
        let stale = self.sessions().any(|session| {
            let kept = self.autosave.saved.get(&session.tab);
            if session.document.metadata().dirty() {
                kept != Some(&session.document.mutation_seq())
            } else {
                kept.is_some()
            }
        });
        stale.then(|| self.autosave.at + every)
    }

    /// Autosave: every few minutes, a copy of each tab's document edited
    /// since its last copy, packed and written on a thread of its own. A
    /// tab that is saved has its copy taken away.
    pub(crate) fn drive_autosave(&mut self) {
        self.autosave.jobs.retain(|job| !job.is_finished());
        if !self
            .next_autosave()
            .is_some_and(|at| at <= std::time::Instant::now())
        {
            return;
        }
        self.autosave.at = std::time::Instant::now();
        // The fields apart, as `self.autosave.saved` changes on the way.
        let sessions = std::iter::once(&self.session)
            .chain(self.tabs.iter().filter_map(|slot| slot.parked.as_ref()));
        let mut wanted = Vec::new();
        for session in sessions {
            let tab = session.tab;
            if !session.document.metadata().dirty() {
                if self.autosave.saved.remove(&tab).is_some() {
                    crate::app::recovery::forget(tab);
                }
                continue;
            }
            let seq = session.document.mutation_seq();
            if self.autosave.saved.get(&tab) == Some(&seq) {
                continue;
            }
            wanted.push((
                tab,
                seq,
                session.document.clone(),
                session.file.path.clone(),
            ));
        }
        for (tab, seq, mut document, file) in wanted {
            self.autosave.saved.insert(tab, seq);
            let name = document.name().to_string();
            let spawned = crate::platform::spawn("printcad-autosave", move || {
                let kept = document
                    .save_to_bytes(core_document::Compression::Zstd)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        crate::app::recovery::keep(tab, &name, file.as_deref(), &bytes)
                            .map_err(|e| e.to_string())
                    });
                if let Err(why) = kept {
                    tracing::warn!("Autosave of `{name}` failed: {why}");
                }
            });
            if let Ok(job) = spawned {
                self.autosave.jobs.push(job);
            }
        }
    }

    /// Bring back a copy a crash left, as an untitled document in a tab
    /// of its own: saving it says where it goes.
    pub(crate) fn recover_document(&mut self, copy: PathBuf) {
        let Some(entry) = self
            .autosave
            .recoverable
            .iter()
            .find(|r| r.copy == copy)
            .cloned()
        else {
            return;
        };
        let document = crate::platform::read(&copy)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Self::parse_document_bytes(&copy, bytes));
        let document = match document {
            Ok(document) => document,
            Err(err) => {
                app_log::error(format!("Could not recover `{}`: {err:#}", entry.name));
                return;
            }
        };
        self.reset_to_new_document();
        self.session.send_outbox();
        self.session.document = opened(&self.registry, document);
        self.session
            .document
            .set_name(format!("{} (recovered)", entry.name));
        self.session.selection.tree = Some(TreeItemId::DocumentRoot);
        if let Some((mn, mx)) = document_imported_aabb(&self.session.document) {
            let (center, radius) = aabb_fit_center_radius(mn, mx);
            self.session.view.camera.reset_to_fit(
                center,
                radius,
                Some((mn, mx)),
                &self.user_settings.camera,
            );
        }
        self.session.journal.reset(&mut self.session.document);
        self.session
            .connection
            .server
            .send(core_document::server::ClientMessage::Rebase);
        self.session.document.mark_dirty();
        self.discard_recovery(copy);
        match &entry.file {
            Some(file) => app_log::info(format!(
                "Recovered `{}`; it was {}: Save as there to keep it",
                entry.name,
                file.display()
            )),
            None => app_log::info(format!("Recovered `{}`", entry.name)),
        }
    }

    /// Let a copy a crash left go.
    pub(crate) fn discard_recovery(&mut self, copy: PathBuf) {
        crate::app::recovery::remove(&copy);
        self.autosave.recoverable.retain(|r| r.copy != copy);
    }

    /// Block until the server has durably handled every queued write.
    ///
    /// Only worth doing on the way out: the process exiting would abandon a
    /// write in flight and could leave a truncated document behind. Every
    /// exit path must call this (CLAUDE.md invariant).
    pub(crate) fn wait_for_document_saves(&mut self) {
        // A gesture's held ops leave with the rest.
        self.session.send_outbox();
        // A packing worker has bytes nobody has sent yet; abandoning it would
        // lose the save outright.
        if let Some(rx) = self.session.file.save_rx.take() {
            app_log::info("Finishing document save before exit…");
            self.session.file.save_progress = None;
            if let Ok(job) = rx.recv() {
                self.send_packed_document(job);
            }
        } else if self.session.connection.server.status().busy() {
            app_log::info("Finishing document save before exit…");
        }
        self.session.connection.server.flush();
    }

    /// Drain a finished file-dialog thread's result, if any.
    pub(crate) fn poll_file_dialog(&mut self) {
        let Some(rx) = &self.files.dialog_rx else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        let path = result.paths.first().cloned();
        match result.kind {
            FileDialogKind::Open => {
                if let Some(path) = path {
                    self.request_document_open(path);
                }
            }
            FileDialogKind::Save | FileDialogKind::SaveAs => {
                if let Some(path) = path {
                    self.save_document_asking(path);
                }
            }
            FileDialogKind::ImportStep => {
                // A mesh file has no shapes to mesh, and a file a workbench
                // imports is the workbench's to read, so the meshing options
                // the import dialog asks for mean nothing to either. A
                // printCAD file's bodies come in linked to it.
                if let Some(path) = path {
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("prtcad"))
                    {
                        self.insert_linked(path);
                    } else {
                        self.import_picked(path);
                    }
                }
            }
            FileDialogKind::Export(_) => {
                if let Some(path) = path {
                    self.start_export(path);
                }
            }
            FileDialogKind::RunScript => {
                if let Some(path) = path {
                    self.scripting.to_run.push(path);
                }
            }
            FileDialogKind::Attach(chat) => self.attach_files(&chat, result.paths),
            FileDialogKind::SaveFile(file) => {
                if let Some(path) = path {
                    match std::fs::write(&path, &file.contents) {
                        Ok(()) => app_log::info(format!("Saved {}", path.display())),
                        Err(err) => {
                            app_log::error(format!("Could not save {}: {err}", path.display()))
                        }
                    }
                }
            }
            FileDialogKind::SaveAnimation(animation) => {
                if let Some(path) = path {
                    crate::app::animation::write_in_background(*animation, path);
                }
            }
            FileDialogKind::SaveFiles(files) => {
                if let Some(folder) = path {
                    write_into(&folder, &files);
                }
            }
            FileDialogKind::BenchOpen(open) => {
                let files = read_for_bench(&result.paths);
                self.hand_files_to_bench(&open.bench, &open.id, files);
            }
            FileDialogKind::InstallPackage => {
                if let Some(path) = path {
                    self.install_package_from(&path);
                }
            }
            FileDialogKind::ReplaceShape(body) => {
                if let Some(path) = path {
                    self.replace_shape_from(body, &path);
                }
            }
            FileDialogKind::TexturePicture(body, index) => {
                if let Some(path) = path {
                    self.use_texture_picture(body, index, &path);
                }
            }
        }
        self.files.dialog_rx = None;
    }

    pub(crate) fn start_file_dialog(&mut self, kind: FileDialogKind) {
        use std::sync::mpsc;
        if self.files.dialog_rx.is_some() {
            return;
        }
        // A read-only file is not written in place: Save becomes Save As.
        let kind = match kind {
            FileDialogKind::Save if self.current_file_read_only() => {
                app_log::warn(format!(
                    "`{}` is read-only: save it under another name",
                    self.session.document.name()
                ));
                FileDialogKind::SaveAs
            }
            kind => kind,
        };

        let (tx, rx) = mpsc::channel::<FileDialogResult>();
        self.files.dialog_rx = Some(rx);

        let current_path = self.session.file.path.clone();
        let stem = current_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        let recent_dir = self.files.recent.last_dir.clone();
        // What the workbenches import beside the host's own formats.
        let bench_imports: Vec<(String, Vec<String>)> = self
            .registry
            .file_imports()
            .into_iter()
            .map(|(_, import)| (import.label.clone(), import.extensions.clone()))
            .collect();

        std::thread::spawn(move || {
            let mut dialog = match kind {
                FileDialogKind::ImportStep => {
                    let mut any = vec![
                        "step", "stp", "iges", "igs", "stl", "obj", "3mf", "ply", "glb", "gltf",
                        "wrl", "vrml", "prtcad",
                    ];
                    any.extend(
                        bench_imports
                            .iter()
                            .flat_map(|(_, e)| e.iter().map(String::as_str)),
                    );
                    let mut dialog = rfd::FileDialog::new();
                    dialog = picking(dialog, "Any file printCAD imports", &any);
                    dialog = picking(dialog, "STEP file", &["step", "stp"]);
                    dialog = picking(dialog, "IGES file", &["iges", "igs"]);
                    dialog = picking(
                        dialog,
                        "Mesh (STL, OBJ, 3MF, PLY, glTF, VRML)",
                        &["stl", "obj", "3mf", "ply", "glb", "gltf", "wrl", "vrml"],
                    );
                    for (label, extensions) in &bench_imports {
                        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
                        dialog = picking(dialog, label, &extensions);
                    }
                    dialog
                }
                FileDialogKind::RunScript => {
                    let dialog = picking(rfd::FileDialog::new(), "Lua script", &["lua"]);
                    match settings::scripts_dir().filter(|d| d.is_dir()) {
                        Some(dir) => dialog.set_directory(dir),
                        None => dialog,
                    }
                }
                FileDialogKind::Attach(_) => rfd::FileDialog::new().set_title("Attach files"),
                FileDialogKind::SaveFile(ref file) => rfd::FileDialog::new()
                    .add_filter(file.kind.as_str(), &[file.extension.as_str()])
                    .set_file_name(file.name.as_str()),
                FileDialogKind::SaveFiles(ref files) => {
                    rfd::FileDialog::new().set_title(format!("A folder for {} files", files.len()))
                }
                FileDialogKind::BenchOpen(ref open) => {
                    let mut dialog = rfd::FileDialog::new();
                    for (label, extensions) in &open.kinds {
                        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
                        dialog = picking(dialog, label, &extensions);
                    }
                    dialog
                }
                FileDialogKind::SaveAnimation(ref animation) => {
                    // Videos where ffmpeg can make them; the name's ending
                    // picks the kind, so the default name has none.
                    let (title, label, extensions) = crate::app::animation::save_dialog(
                        crate::app::animation::ffmpeg().is_some(),
                    );
                    rfd::FileDialog::new()
                        .set_title(title)
                        .add_filter(label, &extensions)
                        .set_file_name(animation.name.as_str())
                }
                FileDialogKind::ReplaceShape(_) => picking(
                    rfd::FileDialog::new()
                        .set_title("Replace the shape with the first solid of a file"),
                    "STEP, IGES or mesh file",
                    &[
                        "step", "stp", "iges", "igs", "stl", "obj", "3mf", "ply", "glb", "gltf",
                        "wrl", "vrml",
                    ],
                ),
                FileDialogKind::TexturePicture(..) => picking(
                    rfd::FileDialog::new()
                        .set_title("A picture for the texture: grey levels, white high"),
                    "PNG or JPEG picture",
                    &["png", "jpg", "jpeg"],
                ),
                FileDialogKind::InstallPackage => picking(
                    rfd::FileDialog::new().set_title("Install a workbench package"),
                    "Workbench package",
                    &[workbenches::ARCHIVE_EXTENSION],
                ),
                FileDialogKind::Open => picking(
                    rfd::FileDialog::new(),
                    "printCAD Document",
                    &["prtcad", "json"],
                ),
                FileDialogKind::Export(format) => rfd::FileDialog::new()
                    .add_filter(format!("{} file", format.label()), &[format.extension()])
                    .set_file_name(format!("{stem}.{}", format.extension())),
                _ => rfd::FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]),
            };

            if let Some(recent_dir) = recent_dir
                && !matches!(
                    kind,
                    FileDialogKind::RunScript | FileDialogKind::InstallPackage
                )
            {
                dialog = dialog.set_directory(recent_dir);
            }

            let paths = match kind {
                FileDialogKind::Attach(_) => dialog.pick_files().unwrap_or_default(),
                FileDialogKind::BenchOpen(ref open) if open.multiple => {
                    dialog.pick_files().unwrap_or_default()
                }
                FileDialogKind::SaveFiles(_) => Vec::from_iter(dialog.pick_folder()),
                _ => Vec::from_iter(match kind {
                    FileDialogKind::Open => dialog.pick_file(),
                    FileDialogKind::ImportStep => dialog.pick_file(),
                    FileDialogKind::Save => {
                        if let Some(existing) = current_path {
                            Some(existing)
                        } else {
                            dialog.set_file_name("untitled.prtcad").save_file()
                        }
                    }
                    FileDialogKind::SaveAs => dialog.set_file_name("untitled.prtcad").save_file(),
                    FileDialogKind::Export(_)
                    | FileDialogKind::SaveFile(_)
                    | FileDialogKind::SaveAnimation(_) => dialog.save_file(),
                    FileDialogKind::RunScript
                    | FileDialogKind::InstallPackage
                    | FileDialogKind::ReplaceShape(_)
                    | FileDialogKind::TexturePicture(..)
                    | FileDialogKind::BenchOpen(_) => dialog.pick_file(),
                    FileDialogKind::Attach(_) | FileDialogKind::SaveFiles(_) => None,
                }),
            };

            let _ = tx.send(FileDialogResult { kind, paths });
        });
    }
}

/// Write each file into `folder`, leaving out a name with a folder in it.
fn write_into(folder: &std::path::Path, files: &[(String, Vec<u8>)]) {
    for (name, contents) in files {
        if !plain_file_name(name) {
            app_log::warn(format!("Left out `{name}`: not a plain file name"));
            continue;
        }
        let path = folder.join(name);
        match std::fs::write(&path, contents) {
            Ok(()) => app_log::info(format!("Saved {}", path.display())),
            Err(err) => app_log::error(format!("Could not save {}: {err}", path.display())),
        }
    }
}

/// The files at `paths`, each its name and bytes, up to
/// [`BENCH_OPEN_LIMIT`] in all; one that does not read, or would pass the
/// limit, is left out with a log line.
fn read_for_bench(paths: &[PathBuf]) -> Vec<(String, Vec<u8>)> {
    let mut left = BENCH_OPEN_LIMIT;
    let mut files = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size > left {
            app_log::warn(format!(
                "Left out {}: the files picked pass {} MiB",
                path.display(),
                BENCH_OPEN_LIMIT / (1024 * 1024)
            ));
            continue;
        }
        match std::fs::read(path) {
            Ok(contents) => {
                left -= contents.len() as u64;
                files.push((name, contents));
            }
            Err(err) => app_log::error(format!("Could not read {}: {err}", path.display())),
        }
    }
    files
}

impl PrintCadApp {
    /// Hand `bench` the files picked for its open request `id`: none when
    /// the dialog was cancelled or could not open.
    pub(crate) fn hand_files_to_bench(
        &mut self,
        bench: &core_document::WorkbenchId,
        id: &str,
        files: Vec<(String, Vec<u8>)>,
    ) {
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(bench, params, |wb, ctx| wb.files_opened(ctx, id, files))
        {
            self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
        }
    }
}

/// `dialog` offering the files whose names end in one of `extensions`,
/// in any case: `part.STEP` as well as `part.step`. A file chooser on Linux
/// matches a pattern case by case, so each letter is given both ways there;
/// elsewhere the system's chooser ignores case already.
fn picking(dialog: rfd::FileDialog, name: &str, extensions: &[&str]) -> rfd::FileDialog {
    let extensions: Vec<String> = extensions.iter().map(|e| any_case(e)).collect();
    dialog.add_filter(name, &extensions)
}

/// `extension` as a pattern that matches it in any case where the file
/// chooser's patterns are case sensitive: `[sS][tT][eE][pP]`.
fn any_case(extension: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        return extension.to_string();
    }
    extension
        .chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else {
                c.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{document_name_from_file_name, write_into};

    /// Files a bench saves into a folder stay in it: a name with a folder
    /// in it is left out.
    #[test]
    fn files_written_into_a_folder_stay_in_it() {
        let root = std::env::temp_dir().join(format!("printcad-into-{}", std::process::id()));
        let folder = root.join("posts");
        std::fs::create_dir_all(&folder).unwrap();
        let file = |name: &str| (name.to_string(), b"G0".to_vec());
        write_into(
            &folder,
            &[
                file("1001.nc"),
                file("../escaped.nc"),
                file(".."),
                file("a/b.nc"),
            ],
        );
        let mut names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["1001.nc"]);
        assert!(!root.join("escaped.nc").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn strips_known_extensions() {
        assert_eq!(document_name_from_file_name("part.prtcad"), "part");
        assert_eq!(document_name_from_file_name("part.prtcad.gz"), "part");
        assert_eq!(document_name_from_file_name("part.prtcad.zst"), "part");
        assert_eq!(document_name_from_file_name("part.json"), "part");
    }

    #[test]
    fn strips_case_insensitively_but_preserves_name_case() {
        assert_eq!(document_name_from_file_name("Bracket.PRTCAD"), "Bracket");
        assert_eq!(
            document_name_from_file_name("Bracket.PrtCad.ZST"),
            "Bracket"
        );
    }

    #[test]
    fn leaves_unknown_extensions_alone() {
        assert_eq!(document_name_from_file_name("part.step"), "part.step");
        assert_eq!(document_name_from_file_name("Untitled"), "Untitled");
        assert_eq!(document_name_from_file_name(""), "");
    }

    #[test]
    fn keeps_inner_dots() {
        assert_eq!(document_name_from_file_name("v1.2.prtcad"), "v1.2");
        // `.gz` alone is not a document extension.
        assert_eq!(document_name_from_file_name("part.gz"), "part.gz");
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn an_opened_file_is_offered_whatever_the_case_of_its_extension() {
        assert_eq!(
            crate::app::doc_io::any_case("fcstd"),
            "[fF][cC][sS][tT][dD]"
        );
        assert_eq!(crate::app::doc_io::any_case("3mf"), "3[mM][fF]");
    }
}
