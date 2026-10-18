//! The document server a tab talks to: a daemon per document
//! (`printcad-serverd`), or the app's own file access when none can start.

use std::path::{Path, PathBuf};

use core_document::server::DocumentServer;
use uuid::Uuid;

/// Where an untitled tab's server listens.
pub(crate) fn socket_for_untitled(tab: Uuid) -> PathBuf {
    doc_server::socket_path_for_untitled(tab)
}

/// Where the server of the document at `path` listens.
pub(crate) fn socket_for(path: &Path) -> PathBuf {
    doc_server::socket_path_for(path)
}

/// The server at `socket`, started when none is there yet.
pub(crate) fn connect(socket: &Path) -> Result<Box<dyn DocumentServer>, String> {
    doc_server::DaemonClient::spawn_or_connect(socket)
        .map(|client| Box::new(client) as Box<dyn DocumentServer>)
        .map_err(|err| err.to_string())
}

/// The app's own file access, for when no server starts.
pub(crate) fn fallback() -> Box<dyn DocumentServer> {
    Box::new(doc_server::DirectFiles::new())
}
