//! The local document server and its client.
//!
//! Three pieces, all speaking the protocol defined in
//! `core_document::server`:
//!
//! - [`framing`] — length-prefixed JSON frames over any `Read`/`Write`.
//! - [`daemon`] — the `printcad-serverd` logic: one document, one unix
//!   socket, one client; stores opaque snapshot bytes and an op log. It
//!   never deserializes a `Document`, so an old daemon serves new clients.
//! - [`DaemonClient`] / [`DirectFiles`] — the two `DocumentServer`
//!   implementations the app chooses between: the socket client (spawning
//!   the daemon on demand), and a direct-file fallback with the same
//!   observable behavior for when no daemon can run.

pub mod client;
pub mod daemon;
pub mod direct;
pub mod framing;

pub use client::DaemonClient;
pub use direct::DirectFiles;

use std::path::PathBuf;

/// Where a document's daemon listens. One socket per document, keyed by a
/// hash of its canonical path, under the user's runtime dir — so any client
/// that knows the document knows its server.
pub fn socket_path_for(document: &std::path::Path) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let canonical = document
        .canonicalize()
        .unwrap_or_else(|_| document.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    let key = hasher.finish();
    runtime_dir().join(format!("{key:016x}.sock"))
}

/// A socket for a session that has no file yet (a fresh "Untitled"). Keyed
/// by process id: private to this app instance until the first save gives
/// the document a real identity.
pub fn socket_path_for_untitled() -> PathBuf {
    runtime_dir().join(format!("untitled-{}.sock", std::process::id()))
}

pub(crate) fn runtime_dir_for_logs() -> PathBuf {
    runtime_dir()
}

fn runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("printcad")
}
