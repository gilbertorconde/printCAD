//! The application as a Model Context Protocol server: the tools an agent
//! calls to read and change the document.
//!
//! The protocol runs over any stream; [`serve`] answers `initialize`,
//! `tools/list` and `tools/call` for a [`ToolHost`] that knows the tools.
//! What the tools do is the host's business: this module knows none.

use std::io::{Read, Write};

use serde_json::{Value, json};

use crate::rpc::{Connection, Incoming, RpcError};

/// The protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// A tool: its name, what it does, and its arguments as a JSON Schema.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// It only reads: a client may run it without asking, and alongside
    /// others (`readOnlyHint`).
    pub read_only: bool,
    /// A client that defers tools until it searches for them loads this
    /// one from the start (`anthropic/alwaysLoad`, which Claude Code
    /// reads).
    pub always_load: bool,
}

impl Tool {
    /// The tool as `tools/list` lists it.
    fn to_json(&self) -> Value {
        let mut tool = json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {"readOnlyHint": self.read_only},
        });
        if self.always_load {
            tool["_meta"] = json!({"anthropic/alwaysLoad": true});
        }
        tool
    }
}

/// One piece of what a tool answers.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(String),
    /// An image, base64-encoded, and its type (`image/png`).
    Image {
        data: String,
        mime: String,
    },
}

/// What a tool call answers.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolAnswer {
    pub content: Vec<Content>,
    /// The tool ran but failed; the content says why.
    pub is_error: bool,
}

impl ToolAnswer {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            is_error: true,
        }
    }
}

/// What knows the tools and runs them.
pub trait ToolHost {
    /// Every tool there is.
    fn tools(&self) -> Vec<Tool>;
    /// Run tool `name` with `args` (a JSON object).
    fn call(&mut self, name: &str, args: Value) -> ToolAnswer;
}

/// About the server, for `initialize`.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// What a client should know to use the tools well.
    pub instructions: String,
}

/// Answer one request.
pub fn answer(
    method: &str,
    params: &Value,
    info: &ServerInfo,
    host: &mut dyn ToolHost,
) -> Result<Value, RpcError> {
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| PROTOCOL_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": info.name, "version": info.version},
                "instructions": info.instructions,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": host.tools().iter().map(Tool::to_json).collect::<Vec<_>>(),
        })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
                RpcError::new(RpcError::INVALID_PARAMS, "a tool call names its tool")
            })?;
            if !host.tools().iter().any(|t| t.name == name) {
                return Err(RpcError::new(
                    RpcError::INVALID_PARAMS,
                    format!("no tool `{name}`"),
                ));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let answer = host.call(name, args);
            Ok(json!({
                "content": answer.content.iter().map(|c| match c {
                    Content::Text(text) => json!({"type": "text", "text": text}),
                    Content::Image { data, mime } => {
                        json!({"type": "image", "data": data, "mimeType": mime})
                    }
                }).collect::<Vec<_>>(),
                "isError": answer.is_error,
            }))
        }
        other => Err(RpcError::new(
            RpcError::METHOD_NOT_FOUND,
            format!("this server has no method {other}"),
        )),
    }
}

/// Serve one client over `reader` and `writer` until it goes.
pub fn serve(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    info: &ServerInfo,
    host: &mut dyn ToolHost,
) {
    let (connection, incoming) = Connection::new(reader, writer);
    for message in incoming {
        match message {
            Incoming::Request { id, method, params } => {
                let reply = answer(&method, &params, info, host);
                if connection.respond(id, reply).is_err() {
                    return;
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Barrier(mark) => mark.pass(),
            Incoming::Closed => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    struct Adder;

    impl ToolHost for Adder {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool {
                name: "add".into(),
                description: "Add two numbers".into(),
                input_schema: json!({"type": "object"}),
                read_only: true,
                always_load: true,
            }]
        }

        fn call(&mut self, _: &str, args: Value) -> ToolAnswer {
            match (args["a"].as_f64(), args["b"].as_f64()) {
                (Some(a), Some(b)) => ToolAnswer::text((a + b).to_string()),
                _ => ToolAnswer::error("a and b are numbers"),
            }
        }
    }

    fn info() -> ServerInfo {
        ServerInfo {
            name: "test".into(),
            version: "0".into(),
            instructions: "Add things".into(),
        }
    }

    #[test]
    fn a_client_initializes_lists_the_tools_and_calls_one() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(theirs.try_clone().unwrap(), theirs, &info(), &mut Adder));
        let (client, _) = Connection::new(ours.try_clone().unwrap(), ours);
        let wait = |rx: std::sync::mpsc::Receiver<Result<Value, RpcError>>| {
            rx.recv_timeout(Duration::from_secs(2)).unwrap()
        };
        let init = wait(client.request(
            "initialize",
            json!({"protocolVersion": "2024-11-05", "capabilities": {}}),
        ))
        .unwrap();
        assert_eq!(init["protocolVersion"], "2024-11-05");
        assert_eq!(init["serverInfo"]["name"], "test");
        let listed = wait(client.request("tools/list", json!({}))).unwrap();
        assert_eq!(listed["tools"][0]["name"], "add");
        assert_eq!(listed["tools"][0]["annotations"]["readOnlyHint"], true);
        assert_eq!(listed["tools"][0]["_meta"]["anthropic/alwaysLoad"], true);
        let sum = wait(client.request(
            "tools/call",
            json!({"name": "add", "arguments": {"a": 2, "b": 3.5}}),
        ))
        .unwrap();
        assert_eq!(sum["content"][0]["text"], "5.5");
        assert_eq!(sum["isError"], false);
        let bad =
            wait(client.request("tools/call", json!({"name": "add", "arguments": {}}))).unwrap();
        assert_eq!(bad["isError"], true);
        assert!(wait(client.request("tools/call", json!({"name": "nope"}))).is_err());
        let newest = answer(
            "initialize",
            &json!({"protocolVersion": "1999-01-01"}),
            &info(),
            &mut Adder,
        )
        .unwrap();
        assert_eq!(newest["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }
}
