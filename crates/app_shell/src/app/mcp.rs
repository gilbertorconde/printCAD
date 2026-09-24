//! The application's MCP server: the tools an agent calls to read and
//! change the document.
//!
//! It listens on a socket of its own; an agent reaches it through
//! `printcad --mcp` (the relay in `agents::bridge`), and any MCP client can
//! the same way. A connection's thread hands each tool call to the UI
//! thread and waits for the answer: the document is only ever touched
//! there. The tools sit on the command API:
//!
//! - `commands` lists the commands with their arguments;
//! - `call` runs one, `lua` runs a script: both on the script thread, as
//!   the console's lines do, each one undo step;
//! - `log` gives the application's recent messages, `view` a picture of
//!   the scene.
//!
//! A change waits for the user's OK when the chat that asked for it (or,
//! for a client outside any chat, the Preferences) says to ask first.

use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use agents::mcp::{Content, ServerInfo, Tool, ToolAnswer, ToolHost};
use serde_json::{Value, json};

use crate::PrintCadApp;
use crate::app::scripts::{RunKind, command_specs};

/// A tool call from a connection, for the UI thread.
pub(crate) struct ToolRequest {
    /// The chat whose agent asked, when a chat's did.
    pub chat: Option<String>,
    pub tool: String,
    pub args: Value,
    pub reply: Sender<ToolAnswer>,
}

/// A change an agent asked for, waiting for the user's OK.
pub(crate) struct Approval {
    pub chat: Option<String>,
    /// What it does, as a line of script.
    pub summary: String,
    pub request: ToolRequest,
}

/// The server: its socket, and the calls its connections hand over.
pub(crate) struct McpServer {
    pub socket: PathBuf,
    requests: Receiver<ToolRequest>,
}

impl Drop for McpServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Where this process's socket goes.
fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("printcad");
    dir.join(format!("mcp-{}.sock", std::process::id()))
}

/// The socket of a running application, for `printcad --mcp`: the one
/// `PRINTCAD_MCP_SOCKET` names, else the newest that answers.
pub(crate) fn find_socket() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PRINTCAD_MCP_SOCKET") {
        return Some(PathBuf::from(named));
    }
    let dir = socket_path().parent()?.to_path_buf();
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("mcp-") && n.ends_with(".sock"))
        })
        .filter(|p| std::os::unix::net::UnixStream::connect(p).is_ok())
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

impl McpServer {
    /// Listen for clients; `wake` is called when a tool call is waiting.
    pub(crate) fn start(wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Self> {
        let socket = socket_path();
        if let Some(dir) = socket.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("printcad-mcp".to_string())
            .spawn(move || {
                for stream in listener.incoming().map_while(Result::ok) {
                    let (tx, wake) = (tx.clone(), wake.clone());
                    std::thread::spawn(move || serve_client(stream, tx, wake));
                }
            })?;
        Ok(Self {
            socket,
            requests: rx,
        })
    }

    fn next(&self) -> Option<ToolRequest> {
        self.requests.try_recv().ok()
    }
}

fn serve_client(
    stream: std::os::unix::net::UnixStream,
    tx: Sender<ToolRequest>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let Ok((header, reader)) = agents::bridge::accept(stream) else {
        return;
    };
    let mut host = Relay {
        chat: header.chat,
        tx,
        wake,
    };
    agents::mcp::serve(reader, writer, &server_info(), &mut host);
}

fn server_info() -> ServerInfo {
    ServerInfo {
        name: "printCAD".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        instructions: INSTRUCTIONS.to_string(),
    }
}

/// What an agent is told about the tools when it connects.
const INSTRUCTIONS: &str = "\
printCAD is parametric CAD for 3D printing. Everything it can do is a command: \
`commands` lists them with their arguments (filter by prefix, such as \"sketch\" or \
\"part\"), `call` runs one with named arguments, and `lua` runs several at once as a \
Lua script in which each command is pc.<id>{name = value, ...} and returns what it \
makes (help() lists them there too). Ids of bodies, features and sketch elements \
are strings. Lengths are millimetres; sketch coordinates are the sketch's own. \
Numbers can be formulas over variables (var.new, var.set, var.list) and other \
objects' dimensions (doc.parameters lists them, doc.set_formula sets one): \
`3 * Printer.nozzle`, `Pad.length / 2`, with units such as mm, in and deg. \
Solids rebuild after a change: call doc.rebuild before reading them with doc.faces \
or doc.measure. `view` shows the scene as the user sees it, `log` the application's \
recent messages. The user may be asked to allow each change, and can undo any of \
them.";

/// A connection's side: each call goes to the UI thread, and waits.
struct Relay {
    chat: Option<String>,
    tx: Sender<ToolRequest>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl ToolHost for Relay {
    fn tools(&self) -> Vec<Tool> {
        tools()
    }

    fn call(&mut self, name: &str, args: Value) -> ToolAnswer {
        let (reply, answer) = channel();
        let request = ToolRequest {
            chat: self.chat.clone(),
            tool: name.to_string(),
            args,
            reply,
        };
        if self.tx.send(request).is_err() {
            return ToolAnswer::error("printCAD is closing");
        }
        (self.wake)();
        answer
            .recv()
            .unwrap_or_else(|_| ToolAnswer::error("printCAD is closing"))
    }
}

/// The tools, every one loaded by the agent from the start: they are few,
/// and an agent that has to search for them first loses turns doing it.
pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "commands".into(),
            description: "List printCAD's commands: each one's id, what it does, its \
                          arguments and what it answers."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prefix": {
                        "type": "string",
                        "description": "Only commands whose id starts with this, such as \"sketch.\""
                    }
                }
            }),
            read_only: true,
            always_load: true,
        },
        Tool {
            name: "call".into(),
            description: "Run one command with named arguments, as `commands` lists them, \
                          and answer its result as JSON."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The command's id, such as \"part.pad\""},
                    "args": {"type": "object", "description": "Its named arguments"}
                },
                "required": ["command"]
            }),
            read_only: false,
            always_load: true,
        },
        Tool {
            name: "lua".into(),
            description: "Run a Lua script: pc.<command id>{name = value} calls a command \
                          and returns its result; print() output and the error that stops \
                          it come back. The whole script is one undo step."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"source": {"type": "string"}},
                "required": ["source"]
            }),
            read_only: false,
            always_load: true,
        },
        Tool {
            name: "log".into(),
            description: "The application's most recent log messages.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"lines": {"type": "integer", "description": "How many (50)"}}
            }),
            read_only: true,
            always_load: true,
        },
        Tool {
            name: "view".into(),
            description: "A picture of the visible bodies from the direction the view \
                          looks, as a PNG."
                .into(),
            input_schema: json!({"type": "object", "properties": {}}),
            read_only: true,
            always_load: true,
        },
    ]
}

impl PrintCadApp {
    /// Open the MCP server's socket. Without it chats still talk, but their
    /// agents cannot reach the document; the log says why.
    pub(crate) fn start_agent_server(&mut self) {
        match McpServer::start(self.waker.clone()) {
            Ok(server) => {
                tracing::info!(target: "printcad.agents", "MCP server at {}", server.socket.display());
                self.mcp = Some(server);
            }
            Err(err) => {
                crate::app_log::warn(format!("Agents cannot reach the document: {err}"));
            }
        }
    }

    /// Whether a change the agent of chat `chat` asks for waits for the
    /// user's OK: the chat's own switch, else the Preferences'.
    pub(crate) fn asks_before_changes(&self, chat: Option<&str>) -> bool {
        chat.and_then(|id| self.chat_asks(id))
            .unwrap_or(self.user_settings.ai.ask_before_changes)
    }

    /// Take the tool calls waiting: answer the ones that only read, run
    /// the others or hold them for the user's OK.
    pub(crate) fn drive_agent_tools(&mut self) {
        let Some(server) = &self.mcp else {
            return;
        };
        let mut waiting = Vec::new();
        while let Some(request) = server.next() {
            waiting.push(request);
        }
        for request in waiting {
            self.tool_request(request);
        }
    }

    fn tool_request(&mut self, request: ToolRequest) {
        let args = &request.args;
        let answer = match request.tool.as_str() {
            "commands" => {
                let prefix = args.get("prefix").and_then(Value::as_str).unwrap_or("");
                let listed: Vec<Value> = command_specs(&self.registry)
                    .iter()
                    .filter(|c| c.id.starts_with(prefix))
                    .map(|c| c.to_json())
                    .collect();
                ToolAnswer::text(serde_json::to_string_pretty(&listed).unwrap_or_default())
            }
            "log" => {
                let lines = args.get("lines").and_then(Value::as_u64).unwrap_or(50) as usize;
                let entries = crate::log_panel::entries();
                let from = entries.len().saturating_sub(lines);
                ToolAnswer::text(
                    entries[from..]
                        .iter()
                        .map(|e| format!("{}: {}", e.level, e.message))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            "view" => self.view_picture(),
            "call" | "lua" => {
                let summary = match self.change_summary(&request) {
                    Ok(summary) => summary,
                    Err(why) => {
                        let _ = request.reply.send(ToolAnswer::error(why));
                        return;
                    }
                };
                let asks = self.asks_before_changes(request.chat.as_deref());
                if asks && !self.only_reads(&request) {
                    self.approvals.push(Approval {
                        chat: request.chat.clone(),
                        summary,
                        request,
                    });
                    self.assistant_attention = true;
                    self.redraw_needed = true;
                } else {
                    self.run_tool(request);
                }
                return;
            }
            other => ToolAnswer::error(format!("no tool `{other}`")),
        };
        let _ = request.reply.send(answer);
    }

    /// What a `call` or `lua` does, as a line (or lines) of script.
    fn change_summary(&self, request: &ToolRequest) -> Result<String, String> {
        match request.tool.as_str() {
            "call" => {
                let id = request
                    .args
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or("`call` names its command")?;
                let mut recorder = scripting::Recorder::default();
                recorder.push(&core_document::Recorded {
                    id: id.to_string(),
                    args: request
                        .args
                        .get("args")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                    result: Value::Null,
                });
                Ok(recorder.script("").trim().to_string())
            }
            _ => request
                .args
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| "`lua` takes its script as `source`".to_string()),
        }
    }

    /// Whether a call only reads, so it needs no OK.
    fn only_reads(&self, request: &ToolRequest) -> bool {
        request.tool == "call"
            && request
                .args
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|id| {
                    command_specs(&self.registry)
                        .iter()
                        .any(|c| c.id == id && c.read_only)
                })
    }

    /// Run a `call` or `lua` on the script thread; its end answers it.
    pub(crate) fn run_tool(&mut self, request: ToolRequest) {
        let job = match request.tool.as_str() {
            "call" => scripting::Job::Command {
                id: request
                    .args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                args: request
                    .args
                    .get("args")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            },
            _ => scripting::Job::Script {
                source: request
                    .args
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: "an agent's script".to_string(),
            },
        };
        self.submit_script(
            job,
            RunKind::Agent {
                reply: request.reply,
            },
        );
    }

    /// Settle a held change: run it, or tell the agent the user said no.
    pub(crate) fn settle_approval(&mut self, index: usize, allow: bool) {
        if index >= self.approvals.len() {
            return;
        }
        let approval = self.approvals.remove(index);
        if allow {
            self.run_tool(approval.request);
        } else {
            let _ = approval
                .request
                .reply
                .send(ToolAnswer::error("The user did not allow this change."));
        }
    }

    /// The scene as the user sees it, as a picture an agent can look at.
    /// The scene from the current view, as a PNG `width` by `height`.
    pub(crate) fn view_png(&self, width: u32, height: u32) -> Option<Vec<u8>> {
        let shapes = self.thumbnail_shapes();
        let (forward, up) = self.session.camera.view_basis();
        crate::thumbnail::render_at(&shapes, forward, up, width, height)
    }

    fn view_picture(&self) -> ToolAnswer {
        use base64::Engine as _;
        match self.view_png(800, 600) {
            Some(png) => ToolAnswer {
                content: vec![Content::Image {
                    data: base64::engine::general_purpose::STANDARD.encode(png),
                    mime: "image/png".to_string(),
                }],
                is_error: false,
            },
            None => ToolAnswer::text("Nothing is visible to draw."),
        }
    }
}

/// `printcad --mcp [--chat <id>]`: relay stdio to the running
/// application. What the command line asked, if it asked for this.
pub(crate) fn relay_from_args(words: &[String]) -> Option<std::io::Result<()>> {
    if words.first().map(String::as_str) != Some("--mcp") {
        return None;
    }
    let mut chat = None;
    let mut socket = None;
    let mut rest = words[1..].iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--chat" => chat = rest.next().cloned(),
            "--socket" => socket = rest.next().map(PathBuf::from),
            _ => {}
        }
    }
    let Some(socket) = socket.or_else(find_socket) else {
        return Some(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "printCAD is not running (no MCP socket found)",
        )));
    };
    Some(agents::bridge::relay(
        &socket,
        &agents::bridge::Header { chat },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_object_schema_and_a_description() {
        for tool in tools() {
            assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
            assert!(!tool.description.is_empty());
            assert!(tool.always_load, "{} loads from the start", tool.name);
        }
        let changes: Vec<String> = tools()
            .into_iter()
            .filter(|t| !t.read_only)
            .map(|t| t.name)
            .collect();
        assert_eq!(
            changes,
            ["call", "lua"],
            "only these may change the document"
        );
        assert!(!INSTRUCTIONS.contains('\u{2014}'));
    }
}
