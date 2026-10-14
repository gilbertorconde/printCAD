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
//! - `search` and `describe` find commands and guides by the words of a
//!   task, and give their whole entries (`app/discovery.rs`);
//! - `call` runs one, `lua` runs a script: both on the script thread, as
//!   the console's lines do, each one undo step, or none when it fails
//!   and its changes are taken back;
//! - `log` gives the application's recent messages, `view` a picture of
//!   the scene.
//!
//! A change waits for the user's OK when the chat that asked for it (or,
//! for a client outside any chat, the Preferences) says to ask first.

#[cfg(not(target_arch = "wasm32"))]
use local_ipc::Listener as UnixListener;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::channel;
use std::sync::mpsc::{Receiver, Sender};

#[cfg(not(target_arch = "wasm32"))]
use agents::mcp::Tool;
use agents::mcp::{Content, ToolAnswer};
#[cfg(not(target_arch = "wasm32"))]
use agents::mcp::{Prompt, Resource, ServerInfo, ToolHost};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

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
#[cfg(not(target_arch = "wasm32"))]
fn socket_path() -> PathBuf {
    local_ipc::runtime_dir().join(format!("mcp-{}.sock", std::process::id()))
}

/// The socket of a running application, for `printcad --mcp`: the one
/// `PRINTCAD_MCP_SOCKET` names, else the newest that answers.
#[cfg(not(target_arch = "wasm32"))]
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
        .filter(|p| local_ipc::Stream::connect(p).is_ok())
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

impl McpServer {
    /// Listen for clients; `wake` is called when a tool call is waiting.
    #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
fn serve_client(
    stream: local_ipc::Stream,
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

#[cfg(not(target_arch = "wasm32"))]
fn server_info() -> ServerInfo {
    ServerInfo {
        name: "printCAD".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        instructions: "printCAD, parametric CAD for 3D printing, running on the user's desktop."
            .to_string(),
    }
}

/// Requests a connection makes of the UI thread on its own account.
const ASK_INSTRUCTIONS: &str = "(instructions)";
const ASK_RULES: &str = "(rules)";
/// How long a connection waits for the UI thread to answer an `ask`.
#[cfg(not(target_arch = "wasm32"))]
const ASK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// A connection's side: each call goes to the UI thread, and waits.
#[cfg(not(target_arch = "wasm32"))]
struct Relay {
    chat: Option<String>,
    tx: Sender<ToolRequest>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Relay {
    /// Something only the UI thread knows (the rules, the context, the
    /// instructions), asked as a request no client can name: `tools/call`
    /// only reaches the tools `tools()` lists. A UI thread that does not
    /// answer within `ASK_WAIT` gets `None`, so a client's `initialize`
    /// never waits on a busy window: the fixed instructions stand in.
    fn ask(&mut self, what: &str) -> Option<String> {
        let answer = self.send(what, json!({}))?.recv_timeout(ASK_WAIT).ok()?;
        (!answer.is_error).then(|| {
            answer
                .content
                .into_iter()
                .filter_map(|c| match c {
                    Content::Text(text) => Some(text),
                    Content::Image { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// Hands a request to the UI thread and wakes it; `None` once the
    /// application is closing.
    fn send(&mut self, name: &str, args: Value) -> Option<Receiver<ToolAnswer>> {
        let (reply, answer) = channel();
        let request = ToolRequest {
            chat: self.chat.clone(),
            tool: name.to_string(),
            args,
            reply,
        };
        self.tx.send(request).ok()?;
        (self.wake)();
        Some(answer)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ToolHost for Relay {
    fn tools(&self) -> Vec<Tool> {
        tools()
    }

    fn instructions(&mut self) -> Option<String> {
        self.ask(ASK_INSTRUCTIONS)
    }

    fn resources(&self) -> Vec<Resource> {
        crate::app::agent_context::resources()
    }

    fn read_resource(&mut self, uri: &str) -> Option<String> {
        match uri {
            "printcad://rules" => self.ask(ASK_RULES),
            "printcad://context" => self.ask("context"),
            _ => crate::app::agent_context::fixed_resource(uri),
        }
    }

    fn prompts(&self) -> Vec<Prompt> {
        crate::app::agent_context::prompts()
    }

    fn prompt(&mut self, name: &str, args: &Value) -> Option<String> {
        crate::app::agent_context::prompt_text(name, args)
    }

    fn call(&mut self, name: &str, args: Value) -> ToolAnswer {
        self.send(name, args)
            .and_then(|answer| answer.recv().ok())
            .unwrap_or_else(|| ToolAnswer::error("printCAD is closing"))
    }
}

/// What a call's `description` argument is for.
#[cfg(not(target_arch = "wasm32"))]
const DESCRIBE: &str = "A few words on what this does, such as \"Pocket the bolt holes\": \
                        the user sees it as the call's title in the chat";

/// The tools, every one loaded by the agent from the start: they are few,
/// and an agent that has to search for them first loses turns doing it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "context".into(),
            title: "What is open".into(),
            description: "What printCAD has open right now: the document your chat works on \
                          (its file, unsaved changes), the workbench, what is selected and \
                          being edited, the bodies, the features that fail to build, and the \
                          user's rules."
                .into(),
            input_schema: json!({"type": "object", "properties": {}}),
            read_only: true,
            always_load: true,
        },
        Tool {
            name: "commands".into(),
            title: "List commands".into(),
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
            name: "search".into(),
            title: "Search commands and guides".into(),
            description: "Find the commands and guides for a task by its words (\"round the \
                          edges\", \"shell a box\"), CAD synonyms included: for each query, \
                          the best matches, one line each."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "queries": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "What to look for; each query answers a ranked group of its own"
                    },
                    "limit": {"type": "integer", "description": "How many matches a query (8)"}
                },
                "required": ["queries"]
            }),
            read_only: true,
            always_load: true,
        },
        Tool {
            name: "describe".into(),
            title: "Describe commands".into(),
            description: "The whole entry of each command or guide named: arguments, what it \
                          answers, notes, examples, related commands. A key is an id \
                          (\"design.fillet\"), a bare name (\"fillet\" lists what it could \
                          mean), a prefix (\"surface\") or a near spelling."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "keys": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Command ids, names or guide ids"
                    }
                },
                "required": ["keys"]
            }),
            read_only: true,
            always_load: true,
        },
        Tool {
            name: "call".into(),
            title: "Run a command".into(),
            description: "Run one command with named arguments, as `commands` lists them, \
                          and answer its result as JSON."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The command's id, such as \"design.pad\""},
                    "args": {"type": "object", "description": "Its named arguments"},
                    "description": {"type": "string", "description": DESCRIBE}
                },
                "required": ["command"]
            }),
            read_only: false,
            always_load: true,
        },
        Tool {
            name: "lua".into(),
            title: "Run a Lua script".into(),
            description: "Run a Lua script: pc.<command id>{name = value} calls a command \
                          and returns its result. Answers {returned, printed} as JSON: what \
                          the script returns (tables as objects or lists) and its print() \
                          output; {error, printed} when an error stops it. The whole script \
                          is one undo step; a script an error stops changes nothing."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": {"type": "string"},
                    "description": {"type": "string", "description": DESCRIBE}
                },
                "required": ["source"]
            }),
            read_only: false,
            always_load: true,
        },
        Tool {
            name: "log".into(),
            title: "Read the log".into(),
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
            title: "Look at the view".into(),
            description: "A picture of the bodies as a PNG, drawn the same for the same \
                          arguments whatever the user's camera (except view \"current\", \
                          the default): a standard view or any direction, framed to what \
                          is drawn, with outlines, painted faces and edges, markers, a \
                          section, see-through bodies and the box's sizes."
                .into(),
            input_schema: crate::proof::schema(),
            read_only: true,
            always_load: true,
        },
    ]
}

impl PrintCadApp {
    /// Open the MCP server's socket. Without it chats still talk, but their
    /// agents cannot reach the document; the log says why.
    #[cfg(not(target_arch = "wasm32"))]
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
            ASK_INSTRUCTIONS => ToolAnswer::text(self.agent_instructions(request.chat.as_deref())),
            ASK_RULES => ToolAnswer::text(self.agent_rules(request.chat.as_deref())),
            "context" => ToolAnswer::text(
                serde_json::to_string_pretty(&self.agent_context(request.chat.as_deref()))
                    .unwrap_or_default(),
            ),
            "commands" => {
                let prefix = args.get("prefix").and_then(Value::as_str).unwrap_or("");
                let listed: Vec<Value> = command_specs(&self.registry)
                    .iter()
                    .filter(|c| c.id.starts_with(prefix))
                    .map(|c| c.to_json())
                    .collect();
                ToolAnswer::text(serde_json::to_string_pretty(&listed).unwrap_or_default())
            }
            "search" => crate::app::discovery::search(&self.registry, args)
                .map_or_else(ToolAnswer::error, ToolAnswer::text),
            "describe" => crate::app::discovery::describe(&self.registry, args)
                .map_or_else(ToolAnswer::error, ToolAnswer::text),
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
            "view" => self.view_picture(args),
            "call" | "lua" => {
                let named = (request.tool == "call")
                    .then(|| request.args.get("command").and_then(Value::as_str))
                    .flatten();
                if let Some(id) = named
                    && let Err(refused) = self.agent_may_run(id, true)
                {
                    let _ = request.reply.send(ToolAnswer::error(refused));
                    return;
                }
                let summary = match self.change_summary(&request) {
                    Ok(summary) => summary,
                    Err(why) => {
                        let _ = request.reply.send(ToolAnswer::error(why));
                        return;
                    }
                };
                let access = named.map(|id| self.agent_access(id)).unwrap_or_default();
                if waits_for_ok(
                    &access,
                    self.asks_before_changes(request.chat.as_deref()),
                    self.only_reads(&request),
                ) {
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

    /// What an agent may do with command `id`.
    pub(crate) fn agent_access(&self, id: &str) -> core_document::AgentAccess {
        let id = core_document::renamed::command(id);
        command_specs(&self.registry)
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.agent)
            .unwrap_or_default()
    }

    /// Whether an agent may run command `id` now: never one the command
    /// forbids, and one that always asks only as a `call` of its own
    /// (`single`), which the user's approval covers, not from inside a
    /// script. Refused, the reason the agent is told.
    pub(crate) fn agent_may_run(&self, id: &str, single: bool) -> Result<(), String> {
        agent_check(&self.agent_access(id), id, single)
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
                if !command_specs(&self.registry).iter().any(|c| c.id == id) {
                    return Err(format!(
                        "there is no command `{id}`; the `commands` tool lists them"
                    ));
                }
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
                    let id = core_document::renamed::command(id);
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
        // A chat works on its own document, whichever is on screen.
        let tab = request
            .chat
            .as_ref()
            .and_then(|id| self.chats.iter().find(|c| &c.id == id))
            .map_or(self.session.tab, |c| c.tab);
        let single = request.tool == "call";
        self.submit_script_in(
            job,
            RunKind::Agent {
                reply: request.reply,
                single,
            },
            tab,
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

    /// The scene as the user sees it from the current view, as a PNG
    /// `width` by `height` an agent can look at.
    pub(crate) fn view_png(&self, width: u32, height: u32) -> Option<Vec<u8>> {
        let shapes = self.thumbnail_shapes();
        let (forward, up) = self.session.camera.view_basis();
        crate::thumbnail::render_at(&shapes, forward, up, width, height)
    }

    fn view_picture(&self, args: &Value) -> ToolAnswer {
        use base64::Engine as _;
        let none = serde_json::Map::new();
        let drawn = crate::proof::parse(args.as_object().unwrap_or(&none))
            .and_then(|request| self.picture(&request));
        match drawn {
            Ok(png) => ToolAnswer {
                content: vec![Content::Image {
                    data: base64::engine::general_purpose::STANDARD.encode(png),
                    mime: "image/png".to_string(),
                }],
                is_error: false,
            },
            Err(why) => ToolAnswer::error(why),
        }
    }

    /// The picture `request` asks for of the active document, `current`
    /// looking the way the user's view looks.
    pub(crate) fn picture(&self, request: &crate::proof::Request) -> Result<Vec<u8>, String> {
        let camera = &self.session.camera;
        crate::proof::picture(
            &self.session.document,
            request,
            Some(camera.view_basis()),
            camera.axis_system(),
        )
    }
}

/// Whether an agent may run command `id` of access `access`, as a `call`
/// of its own (`single`) or from a script; refused, the reason it is told.
pub(crate) fn agent_check(
    access: &core_document::AgentAccess,
    id: &str,
    single: bool,
) -> Result<(), String> {
    match access {
        core_document::AgentAccess::Never(why) => {
            Err(format!("printCAD does not let an agent run `{id}`: {why}."))
        }
        core_document::AgentAccess::AlwaysAsk if !single => Err(format!(
            "`{id}` needs the user's OK every time: run it on its own with the `call` tool, \
             not from a script."
        )),
        _ => Ok(()),
    }
}

/// Whether an agent's `call` of a command of access `access` (the default
/// for `lua`) waits for the user's OK: always for one that always asks,
/// else when the chat asks before changes (`asks`) and it does more than
/// read (`only_reads`).
pub(crate) fn waits_for_ok(
    access: &core_document::AgentAccess,
    asks: bool,
    only_reads: bool,
) -> bool {
    *access == core_document::AgentAccess::AlwaysAsk || (asks && !only_reads)
}

/// `printcad --mcp [--chat <id>]`: relay stdio to the running
/// application. What the command line asked, if it asked for this.
#[cfg(not(target_arch = "wasm32"))]
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

    /// An agent never ends its own session, and what reaches past the
    /// document waits for the user every time, never run from a script.
    #[test]
    fn an_agent_never_quits_and_asks_before_reaching_past_the_document() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let specs = command_specs(&registry);
        let access = |id: &str| {
            specs
                .iter()
                .find(|c| c.id == id)
                .unwrap_or_else(|| panic!("no command {id}"))
                .agent
                .clone()
        };
        for never in ["app.quit", "tab.close", "doc.set_agent_rules"] {
            let refused = agent_check(&access(never), never, true).unwrap_err();
            assert!(refused.contains("does not let an agent"), "{refused}");
        }
        for asks in [
            "file.new",
            "file.open",
            "file.save_as",
            "file.import",
            "file.export",
            "file.send_to_slicer",
            "doc.replace_shape",
            "sketch.image",
            "sketch.import_dxf",
            "doc.picture",
            "edit.undo",
            "edit.redo",
            "tab.new",
            "tab.next",
        ] {
            assert_eq!(
                access(asks),
                core_document::AgentAccess::AlwaysAsk,
                "{asks}"
            );
            assert!(
                agent_check(&access(asks), asks, true).is_ok(),
                "{asks} as a call"
            );
            assert!(
                agent_check(&access(asks), asks, false).is_err(),
                "{asks} in a script"
            );
            let reads = specs.iter().any(|c| c.id == asks && c.read_only);
            assert!(
                waits_for_ok(&access(asks), false, reads),
                "{asks} waits even where changes run without asking"
            );
        }
        // Looking at the model writes nothing: the `view` tool answers
        // without asking, while a `doc.picture` writing its file waits.
        assert!(tools().iter().any(|t| t.name == "view" && t.read_only));
        assert!(!waits_for_ok(&access("doc.bodies"), true, true));
        for free in ["doc.bodies", "file.save", "view.top", "doc.agent_rules"] {
            assert!(agent_check(&access(free), free, false).is_ok(), "{free}");
        }
        // What an agent may not do is in the command list it reads.
        let quit = specs.iter().find(|c| c.id == "app.quit").unwrap().to_json();
        assert!(quit["agent"].as_str().unwrap().starts_with("never: "));
    }

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
        for tool in tools() {
            assert!(!tool.title.is_empty(), "{} has a title", tool.name);
            assert!(!tool.name.starts_with('('), "{}", tool.name);
        }
    }
}
