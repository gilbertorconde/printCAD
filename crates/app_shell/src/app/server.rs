//! The document server a tab talks to: on a desktop, a daemon per document
//! (`printcad-serverd`) or the app's own file access when none can start;
//! on a browser page, which can start no process, [`BrowserFiles`].

use std::path::{Path, PathBuf};

use core_document::server::DocumentServer;
use uuid::Uuid;

/// Where an untitled tab's server listens.
pub(crate) fn socket_for_untitled(tab: Uuid) -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    return doc_server::socket_path_for_untitled(tab);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = tab;
        PathBuf::from("/browser/untitled")
    }
}

/// Where the server of the document at `path` listens.
pub(crate) fn socket_for(path: &Path) -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    return doc_server::socket_path_for(path);
    #[cfg(target_arch = "wasm32")]
    path.to_path_buf()
}

/// The server at `socket`, started when none is there yet.
pub(crate) fn connect(socket: &Path) -> Result<Box<dyn DocumentServer>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    return doc_server::DaemonClient::spawn_or_connect(socket)
        .map(|client| Box::new(client) as Box<dyn DocumentServer>)
        .map_err(|err| err.to_string());
    #[cfg(target_arch = "wasm32")]
    {
        let _ = socket;
        Ok(Box::new(BrowserFiles::default()))
    }
}

/// The app's own file access, for when no server starts.
pub(crate) fn fallback() -> Box<dyn DocumentServer> {
    #[cfg(not(target_arch = "wasm32"))]
    return Box::new(doc_server::DirectFiles::new());
    #[cfg(target_arch = "wasm32")]
    Box::new(BrowserFiles::default())
}

/// A browser page's files: a document opens from the files its user
/// picked, and saves as a download. No peers and no op log: the page is
/// the only one editing.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct BrowserFiles {
    replies: std::collections::VecDeque<core_document::server::ServerMessage>,
}

#[cfg(target_arch = "wasm32")]
impl DocumentServer for BrowserFiles {
    fn name(&self) -> &str {
        "browser files"
    }

    fn standalone(&self) -> bool {
        true
    }

    fn send(&mut self, msg: core_document::server::ClientMessage) {
        use core_document::server::{ClientMessage, ServerMessage};
        match msg {
            ClientMessage::Hello { .. }
            | ClientMessage::Rebase
            | ClientMessage::Presence(_)
            | ClientMessage::Ops(_) => {}
            ClientMessage::SaveDocument {
                path,
                bytes,
                at_seq,
            } => self
                .replies
                .push_back(match crate::platform::write_owned(&path, bytes) {
                    Ok(()) => ServerMessage::SaveCompleted { path, at_seq },
                    Err(err) => ServerMessage::SaveFailed {
                        path,
                        error: err.to_string(),
                    },
                }),
            ClientMessage::OpenDocument { path, token } => {
                self.replies.push_back(match crate::platform::read(&path) {
                    Ok(bytes) => ServerMessage::Opened { token, path, bytes },
                    Err(err) => ServerMessage::OpenFailed {
                        token,
                        path,
                        error: err.to_string(),
                    },
                })
            }
        }
    }

    fn poll(&mut self) -> Vec<core_document::server::ServerMessage> {
        self.replies.drain(..).collect()
    }

    fn status(&self) -> core_document::server::ServerStatus {
        core_document::server::ServerStatus {
            connected: true,
            ..Default::default()
        }
    }

    fn flush(&mut self) {}
}
