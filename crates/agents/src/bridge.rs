//! `printcad --mcp`: the MCP server an agent starts over stdio, relaying to
//! the running application.
//!
//! The application listens on a socket of its own; an agent's session is
//! handed a command that runs this relay with that socket's path. The
//! relay's first line to the socket says which chat started it
//! ([`Header`]), so the application knows whose tool calls these are; then
//! it copies stdin to the socket and the socket to stdout until either
//! ends.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use serde_json::{Value, json};

/// The key of the relay's first line.
const HEADER_KEY: &str = "printcad_relay";

/// What the relay says of itself before the protocol starts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Header {
    /// The chat that started it, when a chat did.
    pub chat: Option<String>,
}

/// Relay stdin and stdout to the application at `socket`.
pub fn relay(socket: &Path, header: &Header) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!(
                "printCAD is not running with its MCP socket at {} ({e})",
                socket.display()
            ),
        )
    })?;
    let mut first = serde_json::to_vec(&json!({ HEADER_KEY: {"chat": header.chat} }))?;
    first.push(b'\n');
    stream.write_all(&first)?;
    let mut from_app = stream.try_clone()?;
    let to_client = std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        let _ = std::io::copy(&mut from_app, &mut stdout);
    });
    let mut stdin = std::io::stdin();
    let _ = std::io::copy(&mut stdin, &mut stream);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = to_client.join();
    Ok(())
}

/// The application's side: the relay's header, if the stream starts with
/// one, and a reader for the protocol that follows (a client that
/// connects without the relay starts straight into it).
pub fn accept(stream: UnixStream) -> std::io::Result<(Header, impl Read + Send + 'static)> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let header = serde_json::from_str::<Value>(first.trim())
        .ok()
        .and_then(|v| v.get(HEADER_KEY).cloned());
    Ok(match header {
        Some(h) => (
            Header {
                chat: h.get("chat").and_then(Value::as_str).map(str::to_string),
            },
            Box::new(std::io::Cursor::new(Vec::new()).chain(reader)) as Box<dyn Read + Send>,
        ),
        None => (
            Header::default(),
            Box::new(std::io::Cursor::new(first.into_bytes()).chain(reader)),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_s_header_names_the_chat_and_a_bare_client_keeps_its_first_line() {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(b"{\"printcad_relay\":{\"chat\":\"c7\"}}\n{\"x\":1}\n")
            .unwrap();
        drop(a);
        let (header, mut rest) = accept(b).unwrap();
        assert_eq!(header.chat.as_deref(), Some("c7"));
        let mut text = String::new();
        rest.read_to_string(&mut text).unwrap();
        assert_eq!(text, "{\"x\":1}\n");

        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(b"{\"jsonrpc\":\"2.0\"}\n").unwrap();
        drop(a);
        let (header, mut rest) = accept(b).unwrap();
        assert_eq!(header, Header::default());
        let mut text = String::new();
        rest.read_to_string(&mut text).unwrap();
        assert_eq!(text, "{\"jsonrpc\":\"2.0\"}\n");
    }
}
