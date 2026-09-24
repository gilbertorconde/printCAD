//! A chat with an agent over the Agent Client Protocol.
//!
//! The agent is a program of its own, started with its command, and speaks
//! JSON-RPC on its stdin and stdout. A chat initializes it, opens a
//! session (handing it the MCP servers it may use and the folder it works
//! in), sends prompts and shows what the agent streams back: its message,
//! its thoughts, the tools it calls, its plan. The agent may ask to be
//! allowed something; the chat puts the question to the user and answers.
//!
//! [`AgentChat`] runs all of that on a thread of its own and reports it as
//! [`ChatEvent`]s; it never blocks its holder.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::rpc::{Connection, Incoming, RpcError};

/// The protocol version this client speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// A program to run, with its arguments and environment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// An MCP server the agent may start for the session.
#[derive(Debug, Clone, PartialEq)]
pub struct McpServer {
    pub name: String,
    pub program: Program,
}

/// What the holder asks of the chat.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatCommand {
    /// Send a prompt; one sent while the agent is busy waits its turn.
    Prompt(String),
    /// Ask the agent to stop the turn it is on.
    Cancel,
    /// Answer the agent's request for permission `request` with the option
    /// chosen, or `None` to refuse outright.
    Permission {
        request: Value,
        option: Option<String>,
    },
    /// Change session option `id` to `value`: a choice's value for a
    /// select, a boolean for a toggle.
    SetOption { id: String, value: Value },
}

/// A setting of the session the agent offers: its permission mode, its
/// model, how hard it thinks, or any other it names.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionOption {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `mode`, `model`, `thought_level`, or another the agent names.
    pub category: String,
    pub value: OptionValue,
    /// How the agent takes a change.
    pub via: OptionVia,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OptionValue {
    Select {
        current: String,
        choices: Vec<OptionChoice>,
    },
    Toggle(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionChoice {
    pub value: String,
    pub name: String,
    pub description: String,
}

/// The request that changes an option: the general one, or the older
/// requests for the permission mode and the model some agents still take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptionVia {
    Config,
    Mode,
    Model,
}

impl SessionOption {
    /// The name of the value it has.
    pub fn current_name(&self) -> String {
        match &self.value {
            OptionValue::Select { current, choices } => choices
                .iter()
                .find(|c| &c.value == current)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| current.clone()),
            OptionValue::Toggle(on) => if *on { "on" } else { "off" }.to_string(),
        }
    }

    /// Its value as the agent takes it.
    pub fn current(&self) -> Value {
        match &self.value {
            OptionValue::Select { current, .. } => Value::String(current.clone()),
            OptionValue::Toggle(on) => Value::Bool(*on),
        }
    }

    /// Whether `value` is one it can take.
    pub fn accepts(&self, value: &Value) -> bool {
        match (&self.value, value) {
            (OptionValue::Select { choices, .. }, Value::String(v)) => {
                choices.iter().any(|c| &c.value == v)
            }
            (OptionValue::Toggle(_), Value::Bool(_)) => true,
            _ => false,
        }
    }

    fn set(&mut self, value: &Value) {
        match (&mut self.value, value) {
            (OptionValue::Select { current, .. }, Value::String(v)) => *current = v.clone(),
            (OptionValue::Toggle(on), Value::Bool(v)) => *on = *v,
            _ => {}
        }
    }
}

/// One of the choices a permission request offers.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

/// A step of the agent's plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanEntry {
    pub content: String,
    /// `pending`, `in_progress` or `completed`.
    pub status: String,
}

/// What the chat reports.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// The session is open; prompts may go.
    Ready,
    /// The session's options, all of them, whenever one changed.
    Options(Vec<SessionOption>),
    /// The chat could not start, or stopped, and why.
    Failed(String),
    /// A piece of the agent's message, or of its thinking.
    Text {
        thought: bool,
        text: String,
    },
    /// The agent started a tool call.
    ToolCall {
        id: String,
        title: String,
        kind: String,
        status: String,
    },
    /// A tool call moved on: its title, status or output changed.
    ToolCallUpdate {
        id: String,
        title: Option<String>,
        status: Option<String>,
        output: Option<String>,
    },
    Plan(Vec<PlanEntry>),
    /// The agent asks to be allowed something.
    Permission {
        request: Value,
        title: String,
        options: Vec<PermissionOption>,
    },
    /// The agent finished its turn, and why (`end_turn`, `cancelled`,
    /// `max_tokens`, `refusal` ...).
    TurnEnded {
        stop_reason: String,
    },
    /// A prompt failed.
    Error(String),
    /// A line the agent wrote on its stderr.
    Stderr(String),
    /// The agent's program ended.
    Exited,
}

/// A running chat with an agent.
pub struct AgentChat {
    commands: Sender<ChatCommand>,
    events: Receiver<ChatEvent>,
}

impl AgentChat {
    /// Start `agent` and open a session in `cwd` with `mcp` servers. `wake`
    /// is called whenever an event is ready.
    pub fn start(
        agent: &Program,
        cwd: PathBuf,
        mcp: Vec<McpServer>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (commands_tx, commands_rx) = channel();
        let (events_tx, events_rx) = channel();
        let agent = agent.clone();
        std::thread::Builder::new()
            .name("printcad-agent".to_string())
            .spawn(move || {
                let events = Events {
                    tx: events_tx,
                    wake,
                };
                match spawn(&agent) {
                    Ok((mut child, stdout, stdin, stderr)) => {
                        let stderr_events = events.clone();
                        std::thread::spawn(move || {
                            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                                stderr_events.send(ChatEvent::Stderr(line));
                            }
                        });
                        run(stdout, stdin, cwd, mcp, commands_rx, events);
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    Err(why) => events.send(ChatEvent::Failed(why)),
                }
            })
            .expect("the agent thread starts");
        Self {
            commands: commands_tx,
            events: events_rx,
        }
    }

    /// The same, over streams already connected to an agent (for tests).
    pub fn over(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        cwd: PathBuf,
        mcp: Vec<McpServer>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (commands_tx, commands_rx) = channel();
        let (events_tx, events_rx) = channel();
        std::thread::spawn(move || {
            run(
                reader,
                writer,
                cwd,
                mcp,
                commands_rx,
                Events {
                    tx: events_tx,
                    wake,
                },
            )
        });
        Self {
            commands: commands_tx,
            events: events_rx,
        }
    }

    pub fn send(&self, command: ChatCommand) {
        let _ = self.commands.send(command);
    }

    /// The next event, if one is ready.
    pub fn try_event(&self) -> Option<ChatEvent> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting at most `wait` for one.
    pub fn next_event(&self, wait: Duration) -> Option<ChatEvent> {
        self.events.recv_timeout(wait).ok()
    }
}

#[derive(Clone)]
struct Events {
    tx: Sender<ChatEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Events {
    fn send(&self, event: ChatEvent) {
        let _ = self.tx.send(event);
        (self.wake)();
    }
}

type Pipes = (
    Child,
    std::process::ChildStdout,
    std::process::ChildStdin,
    std::process::ChildStderr,
);

fn spawn(agent: &Program) -> Result<Pipes, String> {
    let mut child = Command::new(&agent.command)
        .args(&agent.args)
        .envs(agent.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", agent.command))?;
    let (Some(stdout), Some(stdin), Some(stderr)) =
        (child.stdout.take(), child.stdin.take(), child.stderr.take())
    else {
        let _ = child.kill();
        return Err("the agent's pipes did not open".to_string());
    };
    Ok((child, stdout, stdin, stderr))
}

/// How long the agent has to answer `initialize` and `session/new`: a
/// program fetched on first use can take a while.
const START_TIMEOUT: Duration = Duration::from_secs(120);

fn run(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    cwd: PathBuf,
    mcp: Vec<McpServer>,
    commands: Receiver<ChatCommand>,
    events: Events,
) {
    let (connection, incoming) = Connection::new(reader, writer);
    let options: Options = Default::default();
    {
        let (connection, events, options) = (connection.clone(), events.clone(), options.clone());
        std::thread::spawn(move || serve_agent(connection, incoming, events, options));
    }
    let session = match open_session(&connection, &cwd, &mcp) {
        Ok((session, offered)) => {
            options.lock().unwrap().list = offered;
            session
        }
        Err(why) => {
            events.send(ChatEvent::Failed(why));
            return;
        }
    };
    events.send(ChatEvent::Ready);
    events.send(ChatEvent::Options(options.lock().unwrap().list.clone()));
    let mut changing: Vec<Change> = Vec::new();

    let mut turn: Option<Receiver<Result<Value, RpcError>>> = None;
    let mut queued: std::collections::VecDeque<String> = Default::default();
    loop {
        if turn.is_none()
            && let Some(text) = queued.pop_front()
        {
            turn = Some(connection.request(
                "session/prompt",
                json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
            ));
        }
        if let Some(pending) = &turn {
            match pending.try_recv() {
                Ok(Ok(answer)) => {
                    let stop_reason = answer
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn")
                        .to_string();
                    events.send(ChatEvent::TurnEnded { stop_reason });
                    turn = None;
                }
                Ok(Err(err)) => {
                    events.send(ChatEvent::Error(err.to_string()));
                    events.send(ChatEvent::TurnEnded {
                        stop_reason: "error".to_string(),
                    });
                    turn = None;
                }
                Err(_) => {}
            }
        }
        changing.retain(|change| match change.answer.try_recv() {
            Ok(Ok(answer)) => {
                // The general request answers with every option and its value.
                let mut shared = options.lock().unwrap();
                if let Some(list) = answer.get("configOptions")
                    && shared.pushed == change.pushed
                {
                    shared.list = config_options(list);
                    events.send(ChatEvent::Options(shared.list.clone()));
                }
                false
            }
            Ok(Err(err)) => {
                let mut shared = options.lock().unwrap();
                if shared.pushed == change.pushed {
                    shared.list = change.before.clone();
                    events.send(ChatEvent::Options(shared.list.clone()));
                }
                events.send(ChatEvent::Error(format!(
                    "the agent kept its setting: {}",
                    err.message
                )));
                false
            }
            Err(_) => true,
        });
        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(ChatCommand::Prompt(text)) => queued.push_back(text),
            Ok(ChatCommand::Cancel) => {
                queued.clear();
                let _ = connection.notify("session/cancel", json!({"sessionId": session}));
            }
            Ok(ChatCommand::Permission { request, option }) => {
                let outcome = match option {
                    Some(id) => json!({"outcome": "selected", "optionId": id}),
                    None => json!({"outcome": "cancelled"}),
                };
                let _ = connection.respond(request, Ok(json!({ "outcome": outcome })));
            }
            Ok(ChatCommand::SetOption { id, value }) => {
                let mut shared = options.lock().unwrap();
                let before = shared.list.clone();
                let pushed = shared.pushed;
                let Some(option) = shared.list.iter_mut().find(|o| o.id == id) else {
                    continue;
                };
                if !option.accepts(&value) || option.current() == value {
                    continue;
                }
                let (method, params) = match option.via {
                    OptionVia::Config => (
                        "session/set_config_option",
                        json!({"sessionId": session, "configId": id, "value": value}),
                    ),
                    OptionVia::Mode => (
                        "session/set_mode",
                        json!({"sessionId": session, "modeId": value}),
                    ),
                    OptionVia::Model => (
                        "session/set_model",
                        json!({"sessionId": session, "modelId": value}),
                    ),
                };
                // Shown at once; put back if the agent refuses.
                option.set(&value);
                events.send(ChatEvent::Options(shared.list.clone()));
                changing.push(Change {
                    answer: connection.request(method, params),
                    before,
                    pushed,
                });
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The holder let go of the chat: it ends, and the agent with it.
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// An option change on its way to the agent.
struct Change {
    answer: Receiver<Result<Value, RpcError>>,
    /// The options before it, put back if the agent refuses.
    before: Vec<SessionOption>,
    /// The agent's updates heard when it went out.
    pushed: u64,
}

fn open_session(
    connection: &Connection,
    cwd: &std::path::Path,
    mcp: &[McpServer],
) -> Result<(String, Vec<SessionOption>), String> {
    let wait =
        |rx: Receiver<Result<Value, RpcError>>, what: &str| match rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(format!("{what}: {}", err.message)),
            Err(_) => Err(format!("{what}: the agent did not answer")),
        };
    let init = wait(
        connection.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false,
                },
            }),
        ),
        "the agent did not start",
    )?;
    let version = init
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .unwrap_or(PROTOCOL_VERSION);
    if version != PROTOCOL_VERSION {
        return Err(format!(
            "the agent speaks protocol version {version}; this application speaks {PROTOCOL_VERSION}"
        ));
    }
    let servers: Vec<Value> = mcp
        .iter()
        .map(|server| {
            json!({
                "name": server.name,
                "command": server.program.command,
                "args": server.program.args,
                "env": server.program.env.iter()
                    .map(|(name, value)| json!({"name": name, "value": value}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let session = wait(
        connection.request(
            "session/new",
            json!({"cwd": cwd.display().to_string(), "mcpServers": servers}),
        ),
        "the agent did not open a session",
    )?;
    let id = session
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "the agent opened a session without an id".to_string())?;
    Ok((id, session_options(&session)))
}

type Options = Arc<Mutex<Shared>>;

/// The session's options, shared by the loop that changes them and the
/// thread that hears the agent's updates.
#[derive(Default)]
struct Shared {
    list: Vec<SessionOption>,
    /// Updates the agent sent unasked: an answer to a change that one has
    /// overtaken is older than what it sent, and is not applied.
    pushed: u64,
}

/// The options a new session offers: its `configOptions`, or else the
/// older `modes` and `models`.
fn session_options(session: &Value) -> Vec<SessionOption> {
    if let Some(list) = session.get("configOptions") {
        return config_options(list);
    }
    let mut out = Vec::new();
    if let Some(modes) = session.get("modes") {
        out.push(SessionOption {
            id: "mode".to_string(),
            name: "Mode".to_string(),
            description: "Session mode".to_string(),
            category: "mode".to_string(),
            value: OptionValue::Select {
                current: text(modes, "currentModeId"),
                choices: listed(modes, "availableModes", "id"),
            },
            via: OptionVia::Mode,
        });
    }
    if let Some(models) = session.get("models") {
        out.push(SessionOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: "AI model to use".to_string(),
            category: "model".to_string(),
            value: OptionValue::Select {
                current: text(models, "currentModelId"),
                choices: listed(models, "availableModels", "modelId"),
            },
            via: OptionVia::Model,
        });
    }
    out
}

fn listed(value: &Value, key: &str, id: &str) -> Vec<OptionChoice> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| OptionChoice {
                    value: text(item, id),
                    name: text(item, "name"),
                    description: text(item, "description"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A `configOptions` list: selects (their choices flat, or in groups,
/// flattened) and booleans; kinds this client does not know are left out.
fn config_options(list: &Value) -> Vec<SessionOption> {
    fn choices(items: &[Value], out: &mut Vec<OptionChoice>) {
        for item in items {
            match item.get("options").and_then(Value::as_array) {
                Some(group) => choices(group, out),
                None => out.push(OptionChoice {
                    value: text(item, "value"),
                    name: text(item, "name"),
                    description: text(item, "description"),
                }),
            }
        }
    }
    let Some(items) = list.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let current = item.get("currentValue");
            let value = match item.get("type").and_then(Value::as_str)? {
                "select" => {
                    let mut all = Vec::new();
                    choices(item.get("options")?.as_array()?, &mut all);
                    OptionValue::Select {
                        current: current?.as_str()?.to_string(),
                        choices: all,
                    }
                }
                "boolean" | "toggle" => OptionValue::Toggle(current?.as_bool()?),
                _ => return None,
            };
            Some(SessionOption {
                id: text(item, "id"),
                name: text(item, "name"),
                description: text(item, "description"),
                category: text(item, "category"),
                value,
                via: OptionVia::Config,
            })
        })
        .collect()
}

/// What the agent sends unasked: its updates, and its requests, which are
/// for permission or else refused (this client offers no files or
/// terminals of its own).
fn serve_agent(
    connection: Connection,
    incoming: Receiver<Incoming>,
    events: Events,
    options: Options,
) {
    for message in incoming {
        match message {
            Incoming::Notification { method, params } if method == "session/update" => {
                let update = params.get("update").unwrap_or(&Value::Null);
                if let Some(now) = options_update(update, &options) {
                    events.send(ChatEvent::Options(now));
                } else if let Some(event) = update_event(update) {
                    events.send(event);
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } if method == "session/request_permission" => {
                let call = params.get("toolCall").unwrap_or(&Value::Null);
                let title = call
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("The agent asks to go on")
                    .to_string();
                let options = params
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .map(|o| PermissionOption {
                                id: text(o, "optionId"),
                                name: text(o, "name"),
                                kind: text(o, "kind"),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                events.send(ChatEvent::Permission {
                    request: id,
                    title,
                    options,
                });
            }
            Incoming::Request { id, method, .. } => {
                let _ = connection.respond(
                    id,
                    Err(RpcError::new(
                        RpcError::METHOD_NOT_FOUND,
                        format!("this client does not offer {method}"),
                    )),
                );
            }
            Incoming::Closed => {
                events.send(ChatEvent::Exited);
                return;
            }
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The text in a content block, or in a list of tool-call contents.
fn content_text(content: &Value) -> Option<String> {
    match content {
        Value::Object(block) => match block.get("type").and_then(Value::as_str) {
            Some("text") => block
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string),
            Some("content") => block.get("content").and_then(content_text),
            Some("diff") => block
                .get("path")
                .and_then(Value::as_str)
                .map(|p| format!("(changes to {p})")),
            _ => None,
        },
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(content_text).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

/// The options after an update that changes them: the agent sends every
/// option, or the older word that its mode changed.
fn options_update(update: &Value, options: &Options) -> Option<Vec<SessionOption>> {
    let mut shared = options.lock().unwrap();
    match update.get("sessionUpdate").and_then(Value::as_str)? {
        "config_option_update" => shared.list = config_options(update.get("configOptions")?),
        "current_mode_update" => {
            let mode = Value::String(text(update, "currentModeId"));
            shared
                .list
                .iter_mut()
                .find(|o| o.category == "mode")?
                .set(&mode);
        }
        _ => return None,
    }
    shared.pushed += 1;
    Some(shared.list.clone())
}

/// The chat event a session update comes to, if it is one the chat shows.
fn update_event(update: &Value) -> Option<ChatEvent> {
    let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
    let optional = |key: &str| update.get(key).and_then(Value::as_str).map(str::to_string);
    Some(match kind {
        "agent_message_chunk" | "agent_thought_chunk" => ChatEvent::Text {
            thought: kind == "agent_thought_chunk",
            text: content_text(update.get("content")?)?,
        },
        "tool_call" => ChatEvent::ToolCall {
            id: text(update, "toolCallId"),
            title: text(update, "title"),
            kind: optional("kind").unwrap_or_else(|| "other".to_string()),
            status: optional("status").unwrap_or_else(|| "pending".to_string()),
        },
        "tool_call_update" => ChatEvent::ToolCallUpdate {
            id: text(update, "toolCallId"),
            title: optional("title"),
            status: optional("status"),
            output: update.get("content").and_then(content_text),
        },
        "plan" => ChatEvent::Plan(
            update
                .get("entries")
                .and_then(Value::as_array)?
                .iter()
                .map(|e| PlanEntry {
                    content: text(e, "content"),
                    status: text(e, "status"),
                })
                .collect(),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    /// A made-up agent on the other end of `stream`: it opens a session,
    /// and each prompt gets a thought, a tool call that asks permission,
    /// the tool's result and an answer quoting the prompt.
    fn fake_agent(stream: UnixStream) {
        let (connection, incoming) = Connection::new(stream.try_clone().unwrap(), stream);
        std::thread::spawn(move || {
            let mut permission: Option<Receiver<Result<Value, RpcError>>> = None;
            let mut prompt: Option<(Value, String)> = None;
            loop {
                if let Some(rx) = &permission
                    && let Ok(answer) = rx.recv_timeout(Duration::from_millis(10))
                {
                    let chosen = answer.unwrap()["outcome"]["optionId"]
                        .as_str()
                        .unwrap_or("none")
                        .to_string();
                    let update = |u: Value| {
                        connection
                            .notify("session/update", json!({"sessionId": "s1", "update": u}))
                            .unwrap()
                    };
                    update(json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "t1",
                        "status": "completed",
                        "content": [{"type": "content", "content": {"type": "text", "text": format!("chose {chosen}")}}],
                    }));
                    let (id, said) = prompt.take().unwrap();
                    update(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": format!("You said: {said}")},
                    }));
                    connection
                        .respond(id, Ok(json!({"stopReason": "end_turn"})))
                        .unwrap();
                    permission = None;
                }
                let Ok(message) = incoming.recv_timeout(Duration::from_millis(10)) else {
                    continue;
                };
                match message {
                    Incoming::Request { id, method, params } => match method.as_str() {
                        "initialize" => connection
                            .respond(
                                id,
                                Ok(json!({"protocolVersion": 1, "agentCapabilities": {}})),
                            )
                            .unwrap(),
                        "session/new" => {
                            assert_eq!(params["mcpServers"][0]["name"], "printcad");
                            connection
                                .respond(id, Ok(json!({"sessionId": "s1"})))
                                .unwrap()
                        }
                        "session/prompt" => {
                            let said = params["prompt"][0]["text"].as_str().unwrap().to_string();
                            let update = |u: Value| {
                                connection
                                    .notify(
                                        "session/update",
                                        json!({"sessionId": "s1", "update": u}),
                                    )
                                    .unwrap()
                            };
                            update(json!({
                                "sessionUpdate": "agent_thought_chunk",
                                "content": {"type": "text", "text": "thinking"},
                            }));
                            update(json!({
                                "sessionUpdate": "tool_call",
                                "toolCallId": "t1",
                                "title": "Pad the sketch",
                                "kind": "edit",
                                "status": "pending",
                            }));
                            permission = Some(connection.request(
                                "session/request_permission",
                                json!({
                                    "sessionId": "s1",
                                    "toolCall": {"toolCallId": "t1", "title": "Pad the sketch"},
                                    "options": [
                                        {"optionId": "yes", "name": "Allow", "kind": "allow_once"},
                                        {"optionId": "no", "name": "Reject", "kind": "reject_once"},
                                    ],
                                }),
                            ));
                            prompt = Some((id, said));
                        }
                        _ => connection
                            .respond(id, Err(RpcError::new(RpcError::METHOD_NOT_FOUND, "no")))
                            .unwrap(),
                    },
                    Incoming::Notification { .. } => {}
                    Incoming::Closed => return,
                }
            }
        });
    }

    fn next(chat: &AgentChat) -> ChatEvent {
        chat.next_event(Duration::from_secs(5)).expect("an event")
    }

    #[test]
    fn a_chat_opens_a_session_streams_a_turn_and_answers_a_permission_request() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        fake_agent(theirs);
        let mcp = vec![McpServer {
            name: "printcad".into(),
            program: Program {
                command: "printcad".into(),
                args: vec!["--mcp".into()],
                env: Vec::new(),
            },
        }];
        let chat = AgentChat::over(
            ours.try_clone().unwrap(),
            ours,
            PathBuf::from("/tmp"),
            mcp,
            Arc::new(|| {}),
        );
        assert_eq!(next(&chat), ChatEvent::Ready);
        assert_eq!(next(&chat), ChatEvent::Options(Vec::new()));
        chat.send(ChatCommand::Prompt("make a box".into()));
        assert_eq!(
            next(&chat),
            ChatEvent::Text {
                thought: true,
                text: "thinking".into()
            }
        );
        assert!(
            matches!(next(&chat), ChatEvent::ToolCall { ref title, .. } if title == "Pad the sketch")
        );
        let ChatEvent::Permission {
            request, options, ..
        } = next(&chat)
        else {
            panic!("a permission request")
        };
        assert_eq!(options[0].kind, "allow_once");
        chat.send(ChatCommand::Permission {
            request,
            option: Some(options[0].id.clone()),
        });
        assert_eq!(
            next(&chat),
            ChatEvent::ToolCallUpdate {
                id: "t1".into(),
                title: None,
                status: Some("completed".into()),
                output: Some("chose yes".into()),
            }
        );
        assert_eq!(
            next(&chat),
            ChatEvent::Text {
                thought: false,
                text: "You said: make a box".into()
            }
        );
        assert_eq!(
            next(&chat),
            ChatEvent::TurnEnded {
                stop_reason: "end_turn".into()
            }
        );
    }

    /// An agent that answers each request with `answer(method, params)`
    /// and sends `after(method)`'s session updates once it has.
    fn scripted_agent(
        stream: UnixStream,
        answer: impl Fn(&str, &Value) -> Result<Value, RpcError> + Send + 'static,
        after: impl Fn(&str) -> Vec<Value> + Send + 'static,
    ) {
        let (connection, incoming) = Connection::new(stream.try_clone().unwrap(), stream);
        std::thread::spawn(move || {
            for message in incoming {
                if let Incoming::Request { id, method, params } = message {
                    connection.respond(id, answer(&method, &params)).unwrap();
                    for update in after(&method) {
                        connection
                            .notify(
                                "session/update",
                                json!({"sessionId": "s1", "update": update}),
                            )
                            .unwrap();
                    }
                }
            }
        });
    }

    fn open(
        answer: impl Fn(&str, &Value) -> Result<Value, RpcError> + Send + 'static,
        after: impl Fn(&str) -> Vec<Value> + Send + 'static,
    ) -> (AgentChat, Vec<SessionOption>) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        scripted_agent(theirs, answer, after);
        let chat = AgentChat::over(
            ours.try_clone().unwrap(),
            ours,
            PathBuf::from("/tmp"),
            Vec::new(),
            Arc::new(|| {}),
        );
        assert_eq!(next(&chat), ChatEvent::Ready);
        let ChatEvent::Options(options) = next(&chat) else {
            panic!("the options")
        };
        (chat, options)
    }

    fn options_of(event: ChatEvent) -> Vec<SessionOption> {
        match event {
            ChatEvent::Options(options) => options,
            other => panic!("options, not {other:?}"),
        }
    }

    fn current(options: &[SessionOption], id: &str) -> Value {
        options.iter().find(|o| o.id == id).unwrap().current()
    }

    fn config(mode: &str, fast: bool) -> Value {
        json!([
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select",
             "currentValue": mode, "options": [
                {"value": "default", "name": "Manual"},
                {"value": "plan", "name": "Plan"}]},
            {"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": "small", "options": [
                {"group": "fast", "name": "Fast", "options": [{"value": "small", "name": "Small"}]},
                {"group": "big", "name": "Big", "options": [{"value": "large", "name": "Large"}]}]},
            {"id": "fast", "name": "Fast mode", "type": "boolean", "currentValue": fast},
            {"id": "odd", "name": "Something new", "type": "slider", "currentValue": 3},
        ])
    }

    #[test]
    fn session_options_are_offered_changed_and_put_back_when_the_agent_refuses() {
        let (chat, options) = open(
            |method, params| match method {
                "initialize" => Ok(json!({"protocolVersion": 1})),
                "session/new" => {
                    Ok(json!({"sessionId": "s1", "configOptions": config("default", false)}))
                }
                "session/set_config_option" if params["configId"] == "mode" => {
                    Ok(json!({"configOptions": config(params["value"].as_str().unwrap(), false)}))
                }
                _ => Err(RpcError::new(RpcError::METHOD_NOT_FOUND, "not that one")),
            },
            // Each time an option is set, the agent sets the mode back itself.
            |method| match method {
                "session/set_config_option" => vec![json!({
                    "sessionUpdate": "config_option_update",
                    "configOptions": config("default", true),
                })],
                _ => Vec::new(),
            },
        );
        assert_eq!(options.len(), 3, "the unknown kind is left out");
        let OptionValue::Select { choices, .. } = &options[1].value else {
            panic!()
        };
        assert_eq!(
            choices.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Small", "Large"],
            "groups flatten"
        );
        assert_eq!(options[2].value, OptionValue::Toggle(false));

        // A choice the option does not have never goes out.
        chat.send(ChatCommand::SetOption {
            id: "mode".into(),
            value: json!("turbo"),
        });
        chat.send(ChatCommand::SetOption {
            id: "mode".into(),
            value: json!("plan"),
        });
        assert_eq!(
            current(&options_of(next(&chat)), "mode"),
            "plan",
            "shown at once"
        );
        // The answer and the agent's own change after it arrive in either
        // order; the change is what stays.
        let mut last = options_of(next(&chat));
        while let Some(ChatEvent::Options(more)) = chat.next_event(Duration::from_millis(200)) {
            last = more;
        }
        assert_eq!(current(&last, "mode"), "default", "the agent's own change");
        assert_eq!(current(&last, "fast"), true);

        chat.send(ChatCommand::SetOption {
            id: "model".into(),
            value: json!("large"),
        });
        assert_eq!(current(&options_of(next(&chat)), "model"), "large");
        assert_eq!(
            current(&options_of(next(&chat)), "model"),
            "small",
            "put back"
        );
        assert!(matches!(next(&chat), ChatEvent::Error(why) if why.contains("not that one")));
    }

    #[test]
    fn older_agents_offer_modes_and_models_and_take_their_own_requests() {
        let (chat, options) = open(
            |method, params| match method {
                "initialize" => Ok(json!({"protocolVersion": 1})),
                "session/new" => Ok(json!({
                    "sessionId": "s1",
                    "modes": {"currentModeId": "ask", "availableModes": [
                        {"id": "ask", "name": "Ask"}, {"id": "code", "name": "Code"}]},
                    "models": {"currentModelId": "m1", "availableModels": [
                        {"modelId": "m1", "name": "One"}, {"modelId": "m2", "name": "Two"}]},
                })),
                "session/set_mode" if params["modeId"] == "code" => Ok(json!({})),
                "session/set_model" if params["modelId"] == "m2" => Ok(json!({})),
                _ => Err(RpcError::new(RpcError::METHOD_NOT_FOUND, "no")),
            },
            |method| match method {
                "session/set_model" => vec![json!({
                    "sessionUpdate": "current_mode_update",
                    "currentModeId": "ask",
                })],
                _ => Vec::new(),
            },
        );
        assert_eq!(
            options
                .iter()
                .map(|o| (o.id.as_str(), o.via))
                .collect::<Vec<_>>(),
            [("mode", OptionVia::Mode), ("model", OptionVia::Model)]
        );
        assert_eq!(options[0].current_name(), "Ask");
        chat.send(ChatCommand::SetOption {
            id: "mode".into(),
            value: json!("code"),
        });
        assert_eq!(current(&options_of(next(&chat)), "mode"), "code");
        chat.send(ChatCommand::SetOption {
            id: "model".into(),
            value: json!("m2"),
        });
        assert_eq!(current(&options_of(next(&chat)), "model"), "m2");
        let after = options_of(next(&chat));
        assert_eq!(current(&after, "mode"), "ask", "the mode update lands");
        assert_eq!(current(&after, "model"), "m2", "and the model stays");
        assert!(
            chat.next_event(Duration::from_millis(200)).is_none(),
            "no refusal"
        );
    }

    #[test]
    fn a_program_that_is_not_there_says_so() {
        let chat = AgentChat::start(
            &Program {
                command: "printcad-no-such-agent".into(),
                ..Program::default()
            },
            PathBuf::from("/tmp"),
            Vec::new(),
            Arc::new(|| {}),
        );
        let ChatEvent::Failed(why) = next(&chat) else {
            panic!()
        };
        assert!(why.contains("could not start"), "{why}");
    }

    #[test]
    fn a_program_that_does_not_speak_the_protocol_fails_to_start() {
        // It writes a line that is not JSON and ends: the initialize
        // request is never answered.
        let chat = AgentChat::start(
            &Program {
                command: "sh".into(),
                args: vec!["-c".into(), "echo not json; exit 3".into()],
                env: Vec::new(),
            },
            PathBuf::from("/tmp"),
            Vec::new(),
            Arc::new(|| {}),
        );
        loop {
            match next(&chat) {
                ChatEvent::Failed(why) => {
                    assert!(why.contains("did not start"), "{why}");
                    break;
                }
                ChatEvent::Exited | ChatEvent::Stderr(_) => {}
                other => panic!("{other:?}"),
            }
        }
    }
}
