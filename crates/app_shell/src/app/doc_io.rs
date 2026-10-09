//! Document file I/O: open/save/dialog plumbing and shared helpers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;

use crate::app::frame::{aabb_fit_center_radius, document_imported_aabb};
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

/// The on-disk recent list; missing or unreadable means empty.
pub(crate) fn load_recent() -> settings::recent::RecentStore {
    if let Some(path) = page_recent_path() {
        let mut recent = crate::platform::read(&path)
            .map(|bytes| settings::recent::RecentStore::from_json(&String::from_utf8_lossy(&bytes)))
            .unwrap_or_default();
        // The list is every document the page keeps.
        recent
            .files
            .retain(|entry| crate::platform::exists(&entry.path));
        if let Some(dir) = page_documents() {
            for path in crate::platform::list(&dir) {
                if !recent.files.iter().any(|entry| entry.path == path) {
                    let size = crate::platform::file_size(&path);
                    recent.files.push(settings::recent::RecentEntry {
                        path,
                        last_opened_ms: 0,
                        size_bytes: size,
                    });
                }
            }
        }
        return recent;
    }
    settings::SettingsStore::recent_file_path()
        .map(|p| settings::recent::RecentStore::load(&p))
        .unwrap_or_default()
}

/// Where a page keeps its recent list, beside the documents it keeps.
fn page_recent_path() -> Option<PathBuf> {
    crate::platform::kept_dir().map(|kept| kept.join("recent.json"))
}

/// Where a page keeps the documents saved in it.
pub(crate) fn page_documents() -> Option<PathBuf> {
    crate::platform::kept_dir().map(|kept| kept.join("documents"))
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

pub(crate) struct FileDialogResult {
    kind: FileDialogKind,
    /// What was picked: one path, or several for an attachment.
    paths: Vec<PathBuf>,
}

impl PrintCadApp {
    fn document_has_asset_files(&self) -> bool {
        self.session.document.assets().next().is_some()
    }

    /// Save to [`Self::current_file`] or prompt for a path. Returns false if cancelled or save fails.
    pub(crate) fn save_document_interactive(&mut self) -> bool {
        let path = if let Some(ref p) = self.session.current_file {
            p.clone()
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut dialog =
                    FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]);
                if let Some(recent_dir) = self.recent.last_dir.clone() {
                    dialog = dialog.set_directory(recent_dir);
                }
                match dialog.set_file_name("untitled.prtcad").save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
            // A page keeps the document under a name it asks for.
            #[cfg(target_arch = "wasm32")]
            {
                let stem = self.session.document.name().to_string();
                let name = crate::platform::ask_name("Save the document in this browser as", &stem);
                match (page_documents(), name) {
                    (Some(dir), Some(name)) => {
                        let name = name
                            .strip_suffix(".prtcad")
                            .unwrap_or(&name)
                            .replace(['/', '\\'], "-");
                        dir.join(format!("{name}.prtcad"))
                    }
                    _ => return false,
                }
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
        if crate::platform::ON_PAGE {
            let size = crate::platform::file_size(path);
            let now = web_time::SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64);
            self.recent.touch_at(path, now, size);
        } else {
            self.recent.touch(path);
        }
        self.save_recent();
    }

    /// Remember only the directory of `path`, for files that are not
    /// documents (STEP imports).
    pub(crate) fn remember_recent_dir(&mut self, path: &Path) {
        self.recent.remember_dir(path);
        self.save_recent();
    }

    pub(crate) fn remove_recent(&mut self, path: &Path) {
        self.recent.remove(path);
        self.save_recent();
    }

    fn save_recent(&self) {
        if let Some(path) = page_recent_path() {
            let written = serde_json::to_vec_pretty(&self.recent)
                .map_err(std::io::Error::other)
                .and_then(|json| crate::platform::write(&path, &json));
            if let Err(err) = written {
                app_log::error(format!("Failed to keep the recent list: {err}"));
            }
            return;
        }
        if let Ok(recent_path) = settings::SettingsStore::recent_file_path()
            && let Err(err) = self.recent.save(&recent_path)
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
        self.session.document = document;
        self.session.current_file = Some(path.clone());
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
        self.session.tree_selection = Some(TreeItemId::DocumentRoot);
        self.session.selected_body = None;

        self.session.document.mark_clean();
        self.touch_recent(&path);
        self.session.screen = crate::ui::Screen::Workspace;
        // Reframe on the imported meshes' bounds, as a STEP import does, so the
        // scene bounds and the automatic near/far planes agree with the view;
        // keeping the old eye and target would clip the model at the margins.
        // Framed now, and again once the workspace's viewport is laid out
        // and the bodies have their geometry.
        self.session.fit_on_layout = !self.session.document.bodies().is_empty();
        if document_imported_aabb(&self.session.document).is_some() {
            self.frame_scene();
        } else {
            self.session.camera.clear_scene_zoom_constraint();
            self.session
                .camera
                .clamp_focal_to_settings(&self.user_settings.camera);
        }
        self.session.journal.reset(&mut self.session.document);
        // A fresh baseline: what the server logged before describes another
        // state than this client's. (set_name above records an op into the
        // new document; it flows normally on the next drain.)
        self.session
            .server
            .send(core_document::server::ClientMessage::Rebase);
        app_log::info(format!("Opened document from {}", path.display()));
        self.report_missing_packages();
    }

    /// Move the server connection to `socket`: the document's own daemon
    /// (one socket per file) or the session's untitled one.
    /// The old connection is flushed first so no queued write is abandoned;
    /// on failure the old connection stays and the move is only logged: a
    /// working degraded connection beats a broken fresh one.
    pub(crate) fn switch_server_to(&mut self, socket: std::path::PathBuf) {
        if self.session.server_socket == socket && self.session.server.status().connected {
            return;
        }
        match crate::app::server::connect(&socket) {
            Ok(server) => {
                self.session.server.flush();
                self.session.server = server;
                self.session.server_socket = socket;
                // The old connection's peers are not this one's.
                self.session.peer_presence.clear();
                self.session.last_sent_presence = None;
                app_log::info(format!("Document server: {}", self.session.server.name()));
            }
            Err(err) => {
                app_log::warn(format!(
                    "Could not reach document daemon at {}: {err}; keeping {}",
                    socket.display(),
                    self.session.server.name()
                ));
            }
        }
    }

    /// Gentle self-healing: when the connection is degraded, retry the
    /// expected socket every few seconds. Also upgrades a DirectFiles
    /// fallback to a real daemon once one can be spawned.
    pub(crate) fn maybe_reconnect_server(&mut self) {
        if self.session.server.status().connected {
            return;
        }
        // Nobody is on the other end; stale tints would lie.
        self.session.peer_presence.clear();
        let due = self
            .session
            .last_server_reconnect
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5));
        if !due {
            return;
        }
        self.session.last_server_reconnect = Some(web_time::Instant::now());
        match crate::app::server::connect(&self.session.server_socket.clone()) {
            Ok(server) => {
                self.session.server = server;
                app_log::info("Document server reconnected");
            }
            Err(err) => {
                tracing::debug!("server reconnect attempt failed: {err}");
            }
        }
    }

    /// Import a file picked or dropped: a mesh, or a file a workbench
    /// reads, at once; a STEP or IGES file through the import settings,
    /// which its meshing needs.
    pub(crate) fn import_picked(&mut self, path: PathBuf) {
        if kernel_ogeom::is_mesh_file(&path) || self.registry.file_import_for(&path).is_some() {
            self.import_step_at(&path, self.last_step_import_detail.clone());
        } else {
            self.session.step_import_pending = Some((path, self.last_step_import_detail.clone()));
        }
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
            .server
            .send(core_document::server::ClientMessage::OpenDocument {
                path,
                token: self.session.document_load_epoch,
            });
    }

    /// Apply everything the server answered since last frame: opened
    /// documents (parsed here: the server serves bytes, never meaning) and
    /// save completions, whose `at_seq` decides whether the document is
    /// truly clean or was edited mid-save.
    pub(crate) fn drain_server_messages(&mut self) {
        use core_document::server::ServerMessage;
        self.maybe_reconnect_server();
        for message in self.session.server.poll() {
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
                    if self.session.server.status().opens_in_flight > 0 {
                        // An open will replace the document; these ops
                        // describe the incoming one. Apply them after it.
                        self.session.held_remote_ops.push((actor, ops));
                    } else {
                        self.apply_remote_ops(actor, ops);
                    }
                }
                ServerMessage::PresencePeer { actor, state } => {
                    self.session.peer_presence.insert(actor, state);
                }
                ServerMessage::PresenceGone { actor } => {
                    self.session.peer_presence.remove(&actor);
                }
                ServerMessage::Opened { token, path, bytes } => {
                    if token != self.session.document_load_epoch {
                        continue;
                    }
                    self.start_document_parse(token, path, bytes);
                }
                ServerMessage::OpenFailed { token, path, error } => {
                    self.session.held_remote_ops.clear();
                    if token != self.session.document_load_epoch {
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
                        self.autosaved.remove(&self.session.tab);
                        crate::app::recovery::forget(self.session.tab);
                    }
                    self.session.current_file = Some(path.clone());
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
                // A page keeps its documents in memory as well as in the
                // browser's storage, so it keeps them packed small.
                let compression = if crate::platform::ON_PAGE {
                    core_document::Compression::Zstd
                } else if lowered.ends_with(".prtcad.gz") || lowered.ends_with(".gz") {
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
                if self.session.current_file.as_deref() != Some(path) {
                    // Save As gives the document a new identity, and a new
                    // daemon to own it.
                    self.switch_server_to(crate::app::server::socket_for(path));
                }
                self.start_document_save(path, compression);
                return Ok(());
            }
        }

        self.session.current_file = Some(path.to_path_buf());
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
                        self.session.remote_import_routes.insert(
                            temp.clone(),
                            crate::RemoteImportRoute {
                                body_ids: bodies.iter().map(|b| b.id).collect(),
                                asset_id: asset.id,
                            },
                        );
                        self.import_owner.insert(temp.clone(), self.session.tab);
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
        let cursor_world = self.session.hovered_world_pos.map(|p| {
            [
                (p[0] * 0.1).round() * 10.0,
                (p[1] * 0.1).round() * 10.0,
                (p[2] * 0.1).round() * 10.0,
            ]
        });
        let state = core_document::server::PresenceState {
            display_name: std::env::var("USER").unwrap_or_else(|_| "editor".to_string()),
            selected_body: self.session.selected_body,
            cursor_world,
        };
        if self.session.last_sent_presence.as_ref() == Some(&state) {
            return;
        }
        self.session
            .server
            .send(core_document::server::ClientMessage::Presence(
                state.clone(),
            ));
        self.session.last_sent_presence = Some(state);
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
            Ok(_) => self.session.document_open_rx = Some(rx),
            Err(err) => app_log::error(format!("Failed to start the open: {err}")),
        }
    }

    /// Take the parsed document once the worker has it.
    pub(crate) fn drain_document_opens(&mut self) {
        let Some(rx) = self.session.document_open_rx.as_ref() else {
            return;
        };
        let job = match rx.try_recv() {
            Ok(job) => job,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.session.document_open_rx = None;
                app_log::error("The open worker stopped without parsing the document");
                return;
            }
        };
        self.session.document_open_rx = None;

        // Another open may have started while this one was parsing; the
        // token says whether this document is still the one being waited on.
        if job.token != self.session.document_load_epoch {
            return;
        }
        match job.result {
            Ok(document) => {
                self.apply_opened_document(job.path, document);
                for (actor, ops) in std::mem::take(&mut self.session.held_remote_ops) {
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
        let mut document = self.session.document.clone();
        // A body showing a feature's preview is saved whole.
        for (body, preview) in &self.session.previews {
            crate::app::recompute::store_built_solid(&mut document, *body, preview.full.clone());
        }
        let at_seq = self.session.document.mutation_seq();
        let path = path.to_path_buf();
        let preview = self.thumbnail_shapes();
        let (forward, up) = self.session.camera.view_basis();
        let progress = Arc::new(SaveProgress::default());
        let worker_progress = Arc::clone(&progress);
        let (tx, rx) = std::sync::mpsc::channel();

        let spawned = crate::platform::spawn("printcad-document-save", move || {
            document.set_thumbnail(crate::thumbnail::render(&preview, forward, up));
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
                self.session.document_save_rx = Some(rx);
                self.session.save_progress = Some(progress);
            }
            Err(err) => app_log::error(format!("Failed to start the save: {err}")),
        }
    }

    /// The visible bodies in the colours they show in, for the preview a
    /// save carries.
    pub(crate) fn thumbnail_shapes(&self) -> Vec<crate::thumbnail::Shape> {
        let document = &self.session.document;
        document
            .imported_geometries()
            .filter(|(id, _)| document.imported_body_effective_visible(**id))
            .map(|(id, geometry)| preview_shape(document, *id, Arc::clone(&geometry.mesh)))
            .collect()
    }

    /// Take the packed archive once the worker has it.
    pub(crate) fn drain_document_saves(&mut self) {
        let Some(rx) = self.session.document_save_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(job) => {
                self.session.document_save_rx = None;
                self.session.save_progress = None;
                self.send_packed_document(job);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.session.document_save_rx = None;
                self.session.save_progress = None;
                app_log::error("The save worker stopped without packing the document");
            }
        }
    }

    fn send_packed_document(&mut self, job: SaveJob) {
        match job.result {
            Ok(bytes) => {
                self.session
                    .server
                    .send(core_document::server::ClientMessage::SaveDocument {
                        path: job.path,
                        bytes,
                        at_seq: job.at_seq,
                    })
            }
            Err(err) => app_log::error(format!("Failed to serialize document: {err}")),
        }
    }

    /// Every tab's saves, before the process ends (CLAUDE.md invariant).
    pub(crate) fn wait_for_all_document_saves(&mut self) {
        self.for_each_tab(|app| app.wait_for_document_saves());
        // On the way out, after the user answered for every tab: no copy is
        // left to come back from.
        for job in self.autosave_jobs.drain(..) {
            job.join();
        }
        crate::app::recovery::forget(self.session.tab);
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
    pub(crate) fn next_autosave(&self) -> Option<web_time::Instant> {
        let every = self.autosave_every()?;
        let stale = std::iter::once(&self.session)
            .chain(self.tabs.iter().filter_map(|slot| slot.parked.as_ref()))
            .any(|session| {
                let kept = self.autosaved.get(&session.tab);
                if session.document.metadata().dirty() {
                    kept != Some(&session.document.mutation_seq())
                } else {
                    kept.is_some()
                }
            });
        stale.then(|| self.autosaved_at + every)
    }

    /// Autosave: every few minutes, a copy of each tab's document edited
    /// since its last copy, packed and written on a thread of its own. A
    /// tab that is saved has its copy taken away.
    pub(crate) fn drive_autosave(&mut self) {
        self.autosave_jobs.retain(|job| !job.is_finished());
        if !self
            .next_autosave()
            .is_some_and(|at| at <= web_time::Instant::now())
        {
            return;
        }
        self.autosaved_at = web_time::Instant::now();
        let sessions = std::iter::once(&self.session)
            .chain(self.tabs.iter().filter_map(|slot| slot.parked.as_ref()));
        let mut wanted = Vec::new();
        for session in sessions {
            let tab = session.tab;
            if !session.document.metadata().dirty() {
                if self.autosaved.remove(&tab).is_some() {
                    crate::app::recovery::forget(tab);
                }
                continue;
            }
            let seq = session.document.mutation_seq();
            if self.autosaved.get(&tab) == Some(&seq) {
                continue;
            }
            wanted.push((
                tab,
                seq,
                session.document.clone(),
                session.current_file.clone(),
            ));
        }
        for (tab, seq, mut document, file) in wanted {
            self.autosaved.insert(tab, seq);
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
                self.autosave_jobs.push(job);
            }
        }
    }

    /// Hand the document on screen to the browser as a `.prtcad` download,
    /// named after its file or its name.
    pub(crate) fn download_document(&mut self) {
        let name = self
            .session
            .current_file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.session.document.name().to_string());
        // What a save writes: the preview's whole solid and a picture.
        let mut document = self.session.document.clone();
        for (body, preview) in &self.session.previews {
            crate::app::recompute::store_built_solid(&mut document, *body, preview.full.clone());
        }
        let (forward, up) = self.session.camera.view_basis();
        document.set_thumbnail(crate::thumbnail::render(
            &self.thumbnail_shapes(),
            forward,
            up,
        ));
        let bytes = document.save_to_bytes(core_document::Compression::Zstd);
        let downloaded = bytes.map_err(|e| e.to_string()).and_then(|bytes| {
            crate::platform::write(Path::new(&format!("{name}.prtcad")), &bytes)
                .map_err(|e| e.to_string())
        });
        match downloaded {
            Ok(()) => app_log::info(format!("Downloading {name}.prtcad")),
            Err(err) => app_log::error(format!("Could not download the document: {err}")),
        }
    }

    /// Bring back a copy a crash left, as an untitled document in a tab
    /// of its own: saving it says where it goes.
    pub(crate) fn recover_document(&mut self, copy: PathBuf) {
        let Some(entry) = self.recoverable.iter().find(|r| r.copy == copy).cloned() else {
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
        self.session.document = document;
        self.session
            .document
            .set_name(format!("{} (recovered)", entry.name));
        self.session.tree_selection = Some(TreeItemId::DocumentRoot);
        if let Some((mn, mx)) = document_imported_aabb(&self.session.document) {
            let (center, radius) = aabb_fit_center_radius(mn, mx);
            self.session.camera.reset_to_fit(
                center,
                radius,
                Some((mn, mx)),
                &self.user_settings.camera,
            );
        }
        self.session.journal.reset(&mut self.session.document);
        self.session
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
        self.recoverable.retain(|r| r.copy != copy);
    }

    /// Block until the server has durably handled every queued write.
    ///
    /// Only worth doing on the way out: the process exiting would abandon a
    /// write in flight and could leave a truncated document behind. Every
    /// exit path must call this (CLAUDE.md invariant).
    pub(crate) fn wait_for_document_saves(&mut self) {
        // A packing worker has bytes nobody has sent yet; abandoning it would
        // lose the save outright.
        if let Some(rx) = self.session.document_save_rx.take() {
            app_log::info("Finishing document save before exit…");
            self.session.save_progress = None;
            if let Ok(job) = rx.recv() {
                self.send_packed_document(job);
            }
        } else if self.session.server.status().busy() {
            app_log::info("Finishing document save before exit…");
        }
        self.session.server.flush();
    }

    /// Drain a finished file-dialog thread's result, if any.
    pub(crate) fn poll_file_dialog(&mut self) {
        let Some(rx) = &self.file_dialog_rx else {
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
                if let Some(path) = path
                    && let Err(err) = self.save_document_at(&path)
                {
                    app_log::error(format!("Failed to save document: {err}"));
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
                    self.scripts_to_run.push(path);
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
        self.file_dialog_rx = None;
    }

    pub(crate) fn start_file_dialog(&mut self, kind: FileDialogKind) {
        use std::sync::mpsc;
        if self.file_dialog_rx.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel::<FileDialogResult>();
        self.file_dialog_rx = Some(rx);

        let current_path = self.session.current_file.clone();
        let stem = current_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        let recent_dir = self.recent.last_dir.clone();
        // What the workbenches import beside the host's own formats.
        let bench_imports: Vec<(String, Vec<String>)> = self
            .registry
            .file_imports()
            .into_iter()
            .map(|(_, import)| (import.label.clone(), import.extensions.clone()))
            .collect();

        // A browser page: a save names the download, and a pick goes
        // through the page's own picker, the same kinds of file offered.
        #[cfg(target_arch = "wasm32")]
        {
            let _ = recent_dir;
            let named = |name: String| vec![std::path::Path::new("/browser").join(name)];
            // A document is kept by the page, under a name asked for once;
            // a picked file is a copy, kept the same way when saved.
            let keep = |stem: &str| -> Vec<PathBuf> {
                let Some(dir) = page_documents() else {
                    return Vec::new();
                };
                crate::platform::ask_name("Save the document in this browser as", stem)
                    .map(|name| {
                        let name: String = name
                            .chars()
                            .map(|c| if matches!(c, '/' | '\\') { '-' } else { c })
                            .collect();
                        let name = name.strip_suffix(".prtcad").unwrap_or(&name).to_string();
                        vec![dir.join(format!("{name}.prtcad"))]
                    })
                    .unwrap_or_default()
            };
            let paths = match &kind {
                FileDialogKind::Save => match current_path {
                    Some(path) if crate::platform::web::is_kept(&path) => vec![path],
                    _ => keep(&stem),
                },
                FileDialogKind::SaveAs => keep(&stem),
                FileDialogKind::Export(format) => named(format!("{stem}.{}", format.extension())),
                FileDialogKind::SaveFile(file) => {
                    let name = if file.name.ends_with(&format!(".{}", file.extension)) {
                        file.name.clone()
                    } else {
                        format!("{}.{}", file.name, file.extension)
                    };
                    app_log::info(format!("Downloading the {} as {name}", file.kind));
                    named(name)
                }
                FileDialogKind::SaveAnimation(animation) => {
                    named(format!("{}.png", animation.name))
                }
                _ => {
                    let offered: Vec<String> = match &kind {
                        // A printCAD file's parts come in linked to it,
                        // read again by its path, which a page cannot.
                        FileDialogKind::ImportStep => {
                            let mut any: Vec<String> = [
                                "step", "stp", "iges", "igs", "stl", "obj", "3mf", "ply", "glb",
                                "gltf", "wrl", "vrml",
                            ]
                            .map(String::from)
                            .to_vec();
                            any.extend(bench_imports.into_iter().flat_map(|(_, e)| e));
                            any
                        }
                        FileDialogKind::ReplaceShape(_) => [
                            "step", "stp", "iges", "igs", "stl", "obj", "3mf", "ply", "glb",
                            "gltf", "wrl", "vrml",
                        ]
                        .map(String::from)
                        .to_vec(),
                        FileDialogKind::TexturePicture(..) => {
                            ["png", "jpg", "jpeg"].map(String::from).to_vec()
                        }
                        FileDialogKind::RunScript => vec!["lua".into()],
                        FileDialogKind::InstallPackage => {
                            vec![workbenches::ARCHIVE_EXTENSION.into()]
                        }
                        FileDialogKind::Attach(_) => Vec::new(),
                        _ => vec!["prtcad".into(), "json".into()],
                    };
                    let accept = offered
                        .iter()
                        .map(|e| format!(".{e}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let many = matches!(kind, FileDialogKind::Attach(_));
                    crate::platform::web::pick(&accept, many, move |paths| {
                        let _ = tx.send(FileDialogResult { kind, paths });
                    });
                    return;
                }
            };
            let _ = tx.send(FileDialogResult { kind, paths });
        }

        #[cfg(not(target_arch = "wasm32"))]
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
                    let mut dialog = rfd::FileDialog::new()
                        .add_filter("Any file printCAD imports", &any)
                        .add_filter("STEP file", &["step", "stp"])
                        .add_filter("IGES file", &["iges", "igs"])
                        .add_filter(
                            "Mesh (STL, OBJ, 3MF, PLY, glTF, VRML)",
                            &["stl", "obj", "3mf", "ply", "glb", "gltf", "wrl", "vrml"],
                        );
                    for (label, extensions) in &bench_imports {
                        dialog = dialog.add_filter(label.as_str(), extensions);
                    }
                    dialog
                }
                FileDialogKind::RunScript => {
                    let dialog = rfd::FileDialog::new().add_filter("Lua script", &["lua"]);
                    match settings::scripts_dir().filter(|d| d.is_dir()) {
                        Some(dir) => dialog.set_directory(dir),
                        None => dialog,
                    }
                }
                FileDialogKind::Attach(_) => rfd::FileDialog::new().set_title("Attach files"),
                FileDialogKind::SaveFile(ref file) => rfd::FileDialog::new()
                    .add_filter(file.kind.as_str(), &[file.extension.as_str()])
                    .set_file_name(file.name.as_str()),
                FileDialogKind::SaveAnimation(ref animation) => rfd::FileDialog::new()
                    .add_filter("Animated PNG", &["png"])
                    .add_filter("GIF", &["gif"])
                    .add_filter(
                        "A folder of PNG frames",
                        &[crate::app::animation::FRAMES_EXTENSION],
                    )
                    .set_file_name(format!("{}.png", animation.name)),
                FileDialogKind::ReplaceShape(_) => rfd::FileDialog::new()
                    .set_title("Replace the shape with the first solid of a file")
                    .add_filter(
                        "STEP, IGES or mesh file",
                        &[
                            "step", "stp", "iges", "igs", "stl", "obj", "3mf", "ply", "glb",
                            "gltf", "wrl", "vrml",
                        ],
                    ),
                FileDialogKind::TexturePicture(..) => rfd::FileDialog::new()
                    .set_title("A picture for the texture: grey levels, white high")
                    .add_filter("PNG or JPEG picture", &["png", "jpg", "jpeg"]),
                FileDialogKind::InstallPackage => rfd::FileDialog::new()
                    .set_title("Install a workbench package")
                    .add_filter("Workbench package", &[workbenches::ARCHIVE_EXTENSION]),
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
                    | FileDialogKind::TexturePicture(..) => dialog.pick_file(),
                    FileDialogKind::Attach(_) => None,
                }),
            };

            let _ = tx.send(FileDialogResult { kind, paths });
        });
    }
}

/// A body's mesh as a preview draws it: in its display colour, or its
/// faces' own colours when it has them and no display colour.
pub(crate) fn preview_shape(
    document: &core_document::Document,
    body: core_document::BodyId,
    mesh: Arc<kernel_api::TriMesh>,
) -> crate::thumbnail::Shape {
    let entry = document.bodies().iter().find(|b| b.id == body);
    let display = entry.and_then(|b| b.display);
    let face_colors = entry.map(|b| b.face_colors.as_slice()).unwrap_or(&[]);
    let vertex_colours =
        display.is_none() && mesh.colors.len() == mesh.positions.len() && !mesh.colors.is_empty();
    let color = match display {
        Some(display) => display.color,
        None if vertex_colours => [1.0; 3],
        None => core_document::BodyDisplay::default().color,
    };
    // A mesh without its faces named takes no face colours.
    if face_colors.is_empty() || mesh.faces.len() * 3 != mesh.indices.len() {
        return crate::thumbnail::Shape {
            mesh,
            color,
            vertex_colours,
        };
    }
    // Faces coloured on their own, over the body's colour, as the view
    // draws them.
    let coloured = if display.is_some() {
        let mut plain = (*mesh).clone();
        plain.colors.clear();
        core_document::mesh_with_face_colors(&plain, face_colors, color)
    } else {
        core_document::mesh_with_face_colors(&mesh, face_colors, color)
    };
    crate::thumbnail::Shape {
        mesh: Arc::new(coloured),
        color: [1.0; 3],
        vertex_colours: true,
    }
}

#[cfg(test)]
mod tests {
    use super::document_name_from_file_name;

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
}
