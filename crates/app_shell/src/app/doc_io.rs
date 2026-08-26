//! Document file I/O: open/save/dialog plumbing and shared helpers.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use glam::Vec3;
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

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

/// Directory the last document was opened from / saved to, used to seed
/// file dialogs. Best-effort: any failure just means no seeding.
pub(crate) fn read_recent_dir() -> Option<PathBuf> {
    let recent_path = settings::SettingsStore::recent_file_path().ok()?;
    let file = std::fs::File::open(recent_path).ok()?;
    let saved_dir: String = serde_json::from_reader(file).ok()?;
    Some(PathBuf::from(saved_dir))
}

/// Persist the parent directory of `path` for future dialog seeding.
/// Best-effort: failures are ignored.
pub(crate) fn write_recent_dir(path: &Path) {
    if let Ok(recent_path) = settings::SettingsStore::recent_file_path() {
        if let Some(dir) = path.parent() {
            if let Ok(file) = std::fs::File::create(&recent_path) {
                let mut s = dir.to_string_lossy().to_string();
                if !s.ends_with(std::path::MAIN_SEPARATOR) {
                    s.push(std::path::MAIN_SEPARATOR);
                }
                let _ = serde_json::to_writer(file, &s);
            }
        }
    }
}

/// Button labels for the unsaved-changes dialog. GTK and Zenity backends report
/// `MessageDialogResult::Custom(label)` for `YesNoCancelCustom`, not Yes/No/Cancel.
const UNSAVED_CHANGES_SAVE: &str = "Save";
const UNSAVED_CHANGES_DISCARD: &str = "Discard";
const UNSAVED_CHANGES_CANCEL: &str = "Cancel";

pub(crate) enum FileDialogKind {
    Open,
    Save,
    SaveAs,
    ImportStep,
}

pub(crate) struct FileDialogResult {
    kind: FileDialogKind,
    path: Option<PathBuf>,
}

impl PrintCadApp {
    fn document_has_asset_files(&self) -> bool {
        self.document.assets().next().is_some()
    }

    /// If the document is dirty, prompt Save / Discard / Cancel. Returns false when
    /// the user cancels or save fails.
    pub(crate) fn confirm_discard_or_save(&mut self) -> bool {
        if !self.document.metadata().dirty() {
            return true;
        }
        let res = MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(
                "Save changes before continuing? Save writes the file, Discard loses edits, Cancel stays here.",
            )
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNoCancelCustom(
                UNSAVED_CHANGES_SAVE.into(),
                UNSAVED_CHANGES_DISCARD.into(),
                UNSAVED_CHANGES_CANCEL.into(),
            ))
            .show();
        match res {
            MessageDialogResult::Cancel => false,
            MessageDialogResult::No => true,
            MessageDialogResult::Yes => self.save_document_interactive(),
            MessageDialogResult::Custom(s) if s == UNSAVED_CHANGES_SAVE => {
                self.save_document_interactive()
            }
            MessageDialogResult::Custom(s) if s == UNSAVED_CHANGES_DISCARD => true,
            MessageDialogResult::Custom(s) if s == UNSAVED_CHANGES_CANCEL => false,
            _ => false,
        }
    }

    /// Save to [`Self::current_file`] or prompt for a path. Returns false if cancelled or save fails.
    fn save_document_interactive(&mut self) -> bool {
        let path = if let Some(ref p) = self.current_file {
            p.clone()
        } else {
            let mut dialog = FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]);
            if let Some(recent_dir) = read_recent_dir() {
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

    /// Replace the document with a blank one and reset navigation/selection.
    pub(crate) fn reset_to_new_document(&mut self) {
        while !self.kernel_worker.drain().is_empty() {}

        self.document_load_epoch = self.document_load_epoch.wrapping_add(1);
        self.step_import_pending = None;

        let wb_id = self.active_workbench.0.clone();
        self.call_workbench_deactivate(&wb_id);

        self.document = Document::new("Untitled");
        self.current_file = None;
        self.active_document_object = None;
        self.active_body_id = None;
        self.tree_selection = Some(TreeItemId::DocumentRoot);
        self.selected_body = None;
        self.hovered_body = None;

        let wb_id = self.active_workbench.0.clone();
        self.call_workbench_activate(&wb_id);

        self.camera
            .reset_to_fit(Vec3::ZERO, 50.0, None, &self.user_settings.camera);
        self.undo.reset(&self.document);
        // The old document's history no longer describes this client.
        let _ = self.document.take_pending_ops();
        self.server
            .send(core_document::server::ClientMessage::Rebase);
        self.switch_server_to(doc_server::socket_path_for_untitled());
        app_log::info("New document");
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
        self.document = document;
        self.current_file = Some(path.clone());
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        self.document
            .set_name(document_name_from_file_name(file_name));
        self.active_document_object = None;
        self.active_body_id = None;
        self.tree_selection = Some(TreeItemId::DocumentRoot);
        self.selected_body = None;

        self.document.mark_clean();
        write_recent_dir(&path);
        // Match STEP import: reframe imported mesh bounds so scene AABB and auto
        // near/far use the same view as STEP apply (opening only updated zoom
        // limits before, which left stale eye/target → marginal clipping until the
        // user toggled projection or hit Fit View).
        if let Some((mn, mx)) = document_imported_aabb(&self.document) {
            let (center, radius) = aabb_fit_center_radius(mn, mx);
            self.camera
                .reset_to_fit(center, radius, Some((mn, mx)), &self.user_settings.camera);
        } else {
            self.camera.clear_scene_zoom_constraint();
            self.camera
                .clamp_focal_to_settings(&self.user_settings.camera);
        }
        self.undo.reset(&self.document);
        // A fresh baseline: whatever the server logged before no longer
        // describes this client's state. (set_name above records an op into
        // the new document; it flows normally on the next drain.)
        self.server
            .send(core_document::server::ClientMessage::Rebase);
        app_log::info(format!("Opened document from {}", path.display()));
    }

    /// Move the server connection to `socket` — the document's own daemon
    /// (one socket per file) or the session's untitled one.
    /// The old connection is flushed first so no queued write is abandoned;
    /// on failure the old connection stays and the move is only logged: a
    /// working degraded connection beats a broken fresh one.
    pub(crate) fn switch_server_to(&mut self, socket: std::path::PathBuf) {
        if self.server_socket == socket && self.server.status().connected {
            return;
        }
        match doc_server::DaemonClient::spawn_or_connect(&socket) {
            Ok(client) => {
                self.server.flush();
                self.server = Box::new(client);
                self.server_socket = socket;
                app_log::info(format!("Document server: {}", self.server.name()));
            }
            Err(err) => {
                app_log::warn(format!(
                    "Could not reach document daemon at {}: {err}; keeping {}",
                    socket.display(),
                    self.server.name()
                ));
            }
        }
    }

    /// Gentle self-healing: when the connection is degraded, retry the
    /// expected socket every few seconds. Also upgrades a DirectFiles
    /// fallback to a real daemon once one can be spawned.
    pub(crate) fn maybe_reconnect_server(&mut self) {
        if self.server.status().connected {
            return;
        }
        let due = self
            .last_server_reconnect
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5));
        if !due {
            return;
        }
        self.last_server_reconnect = Some(std::time::Instant::now());
        match doc_server::DaemonClient::spawn_or_connect(&self.server_socket.clone()) {
            Ok(client) => {
                self.server = Box::new(client);
                app_log::info("Document server reconnected");
            }
            Err(err) => {
                tracing::debug!("server reconnect attempt failed: {err}");
            }
        }
    }

    /// Ask the server for a document's bytes. The load epoch rides as the
    /// request token so a response landing after File > New is ignored.
    /// Public face of [`Self::request_document_open`] for startup hooks.
    pub(crate) fn open_document_at(&mut self, path: PathBuf) {
        self.request_document_open(path);
    }

    fn request_document_open(&mut self, path: PathBuf) {
        app_log::info(format!("Opening `{}`...", path.display()));
        // The document's own daemon owns its file (and, later, its other
        // clients). Ask it, not the session daemon.
        self.switch_server_to(doc_server::socket_path_for(&path));
        self.server
            .send(core_document::server::ClientMessage::OpenDocument {
                path,
                token: self.document_load_epoch,
            });
    }

    /// Apply everything the server answered since last frame: opened
    /// documents (parsed here — the server serves bytes, never meaning) and
    /// save completions, whose `at_seq` decides whether the document is
    /// truly clean or was edited mid-save.
    pub(crate) fn drain_server_messages(&mut self) {
        use core_document::server::ServerMessage;
        self.maybe_reconnect_server();
        for message in self.server.poll() {
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
                    self.apply_remote_ops(actor, ops);
                }
                ServerMessage::Opened { token, path, bytes } => {
                    if token != self.document_load_epoch {
                        continue;
                    }
                    match Self::parse_document_bytes(&path, bytes) {
                        Ok(document) => self.apply_opened_document(path, document),
                        Err(err) => {
                            app_log::error(format!(
                                "Failed to open document {}: {err:#}",
                                path.display()
                            ));
                        }
                    }
                }
                ServerMessage::OpenFailed { token, path, error } => {
                    if token != self.document_load_epoch {
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
                    if at_seq == self.document.mutation_seq() {
                        self.document.mark_clean();
                    }
                    self.current_file = Some(path.clone());
                    write_recent_dir(&path);
                    app_log::info(format!("Saved document to {}", path.display()));
                }
                ServerMessage::SaveFailed { path, error } => {
                    app_log::error(format!("Failed to save {}: {error}", path.display()));
                }
            }
        }
    }

    pub(crate) fn save_document_at(&mut self, path: &Path) -> Result<()> {
        // Derive a user-facing document name from the file name (strip known extensions).
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let lowered = file_name.to_ascii_lowercase();
        self.document
            .set_name(document_name_from_file_name(file_name));

        let ext_lower = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if matches!(ext_lower.as_deref(), Some("json")) && self.document_has_asset_files() {
            let _ = MessageDialog::new()
                .set_title("Cannot save as JSON")
                .set_description(
                    "This document has embedded assets (e.g. imported STEP). JSON export does not include those bytes. Save as .prtcad instead.",
                )
                .set_level(MessageLevel::Warning)
                .set_buttons(MessageButtons::Ok)
                .show();
            return Err(anyhow::anyhow!(
                "JSON format cannot store embedded assets; save as .prtcad"
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
                serde_json::to_writer_pretty(file, &self.document)
                    .with_context(|| "Failed to serialize document")?;
            }
            _ => {
                // Choose compression based on the full file name suffix.
                let compression = if lowered.ends_with(".prtcad.gz") || lowered.ends_with(".gz") {
                    core_document::Compression::Gzip
                } else if lowered.ends_with(".prtcad.zst") || lowered.ends_with(".zst") {
                    core_document::Compression::Zstd
                } else {
                    core_document::Compression::None
                };

                // A `.prtcad` carries every snapshot blob and the source
                // file, so writing it takes seconds on a large import. The
                // serialization is cheap (payloads sit behind Arcs in the
                // clone); the server owns the actual write, and the rest of
                // the bookkeeping happens in `drain_server_messages` when
                // its completion lands.
                if self.current_file.as_deref() != Some(path) {
                    // Save As gives the document a new identity — and a new
                    // daemon to own it.
                    self.switch_server_to(doc_server::socket_path_for(path));
                }
                let at_seq = self.document.mutation_seq();
                let bytes = self
                    .document
                    .clone()
                    .save_to_bytes(compression)
                    .with_context(|| "Failed to serialize document")?;
                app_log::info(format!("Saving `{}`...", path.display()));
                self.server
                    .send(core_document::server::ClientMessage::SaveDocument {
                        path: path.to_path_buf(),
                        bytes,
                        at_seq,
                    });
                return Ok(());
            }
        }

        self.current_file = Some(path.to_path_buf());
        write_recent_dir(path);
        self.document.mark_clean();
        app_log::info(format!("Saved document to {}", path.display()));
        Ok(())
    }

    /// Apply a peer's relayed edits. The ops are resolved effects — the
    /// same `apply_op` replay path the determinism tests pin — and the
    /// daemon never echoes our own ops, so everything here is foreign.
    /// Marking dirty is apply-side policy: it is what makes THIS replica
    /// re-derive the geometry the peer's edit invalidated.
    fn apply_remote_ops(&mut self, _actor: uuid::Uuid, ops: Vec<core_document::op::DocumentOp>) {
        use core_document::op::DocumentOp as Op;
        if ops.is_empty() {
            return;
        }
        for op in &ops {
            self.document.apply_remote_op(op);
            if let Op::ImportModel {
                asset,
                bytes,
                detail,
                bodies,
                ..
            } = op
            {
                // The op created the bodies; the geometry is derived state
                // we re-compute from the carried bytes. The kernel import
                // is deterministic, so meshes land on the peer's
                // pre-allocated body ids by import order.
                let temp = std::env::temp_dir()
                    .join(format!("printcad_remote_{}.step", asset.id.simple()));
                match std::fs::write(&temp, bytes.as_slice()) {
                    Ok(()) => {
                        self.remote_import_routes.insert(
                            temp.clone(),
                            crate::RemoteImportRoute {
                                body_ids: bodies.iter().map(|b| b.id).collect(),
                                asset_id: asset.id,
                            },
                        );
                        self.kernel_worker.request_step_import(temp, detail.clone());
                    }
                    Err(err) => {
                        app_log::error(format!("Remote import: could not stage bytes: {err}"));
                    }
                }
            }
        }
        // Marking replayed edits dirty above; mark_feature_dirty bumped the
        // seq too. Snapshot undo cannot distinguish a peer's edits from
        // ours, and undoing a peer's work would be wrong — drop local undo
        // history when foreign edits land (op-based per-user undo is the
        // proper fix, tracked).
        self.undo.reset(&self.document);
        app_log::info(format!("{} remote edit(s) applied", ops.len()));
    }

    /// Block until the server has durably handled every queued write.
    ///
    /// Only worth doing on the way out: the process exiting would abandon a
    /// write in flight and could leave a truncated document behind. Every
    /// exit path must call this (CLAUDE.md invariant).
    pub(crate) fn wait_for_document_saves(&mut self) {
        if self.server.status().busy() {
            app_log::info("Finishing document save before exit…");
        }
        self.server.flush();
    }

    /// Drain a finished file-dialog thread's result, if any.
    pub(crate) fn poll_file_dialog(&mut self) {
        let Some(rx) = &self.file_dialog_rx else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        match result.kind {
            FileDialogKind::Open => {
                if let Some(path) = result.path {
                    self.request_document_open(path);
                }
            }
            FileDialogKind::Save | FileDialogKind::SaveAs => {
                if let Some(path) = result.path {
                    if let Err(err) = self.save_document_at(&path) {
                        app_log::error(format!("Failed to save document: {err}"));
                    }
                }
            }
            FileDialogKind::ImportStep => {
                if let Some(path) = result.path {
                    self.step_import_pending = Some((path, self.last_step_import_detail.clone()));
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

        let current_path = self.current_file.clone();

        std::thread::spawn(move || {
            let mut dialog = match kind {
                FileDialogKind::ImportStep => {
                    rfd::FileDialog::new().add_filter("STEP file", &["step", "stp"])
                }
                _ => rfd::FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]),
            };

            if let Some(recent_dir) = read_recent_dir() {
                dialog = dialog.set_directory(recent_dir);
            }

            let path = match kind {
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
            };

            let _ = tx.send(FileDialogResult { kind, path });
        });
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
