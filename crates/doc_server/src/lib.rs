//! The local document server and its client.
//!
//! Three pieces, all speaking the protocol defined in
//! `core_document::server`:
//!
//! - [`framing`]: length-prefixed JSON frames over any `Read`/`Write`.
//! - [`daemon`]: the `printcad-serverd` logic: one document, one unix
//!   socket, any number of clients; stores opaque snapshot bytes and an op
//!   log. It never deserializes a `Document`, so an old daemon serves new
//!   clients.
//! - [`DaemonClient`] / [`DirectFiles`]: the two `DocumentServer`
//!   implementations the app chooses between: the socket client (spawning
//!   the daemon on demand), and a direct-file fallback with the same
//!   observable behavior for when no daemon can run.

pub mod client;
pub mod daemon;
pub mod direct;
mod file_io;
pub mod framing;

pub use client::DaemonClient;
pub use direct::DirectFiles;

use std::path::PathBuf;

/// Where a document's daemon listens. One socket per document, keyed by a
/// hash of its canonical path, under the user's runtime dir, so any client
/// that knows the document knows its server.
pub fn socket_path_for(document: &std::path::Path) -> PathBuf {
    let canonical = document
        .canonicalize()
        .unwrap_or_else(|_| document.to_path_buf());
    let key = path_key(&canonical);
    runtime_dir().join(format!("{key:016x}.sock"))
}

/// A socket for a tab that has no file yet (a fresh "Untitled"). Keyed by
/// process id and the tab's id: private to that tab until the first save
/// gives the document a real identity, and never shared by two untitled
/// tabs of one app.
pub fn socket_path_for_untitled(tab: uuid::Uuid) -> PathBuf {
    let mut bytes = Vec::from(std::process::id().to_le_bytes());
    bytes.extend_from_slice(tab.as_bytes());
    let key = local_ipc::stable_key(bytes);
    runtime_dir().join(format!("untitled-{key:016x}.sock"))
}

/// Hash the platform's path units without lossy text conversion or Rust's Hash encoding.
fn path_key(path: &std::path::Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        local_ipc::stable_key(path.as_os_str().as_bytes())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let bytes: Vec<_> = path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect();
        local_ipc::stable_key(bytes)
    }
}

pub(crate) fn runtime_dir_for_logs() -> PathBuf {
    runtime_dir()
}

fn runtime_dir() -> PathBuf {
    local_ipc::runtime_dir()
}
