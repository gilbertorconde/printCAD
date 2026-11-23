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
    /// How a client shows it to people.
    pub title: String,
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
    /// The tool as `tools/list` lists it. Every tool acts on the one
    /// application and nothing beyond it (`openWorldHint` false); one that
    /// changes things may change what is there (`destructiveHint`).
    fn to_json(&self) -> Value {
        let mut tool = json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {
                "title": self.title,
                "readOnlyHint": self.read_only,
                "destructiveHint": !self.read_only,
                "idempotentHint": self.read_only,
                "openWorldHint": false,
            },
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

/// A document a client may read (`resources/read`), and show or attach
/// for the model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime: String,
}

/// A prompt a client offers its user, often as a slash command.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Prompt {
    pub name: String,
    pub title: String,
    pub description: String,
    /// Its arguments: name, what it is, whether it is required.
    pub arguments: Vec<(String, String, bool)>,
}

/// What knows the tools and runs them, and what else the server offers.
pub trait ToolHost {
    /// Every tool there is.
    fn tools(&self) -> Vec<Tool>;
    /// Run tool `name` with `args` (a JSON object).
    fn call(&mut self, name: &str, args: Value) -> ToolAnswer;
    /// The instructions for this client, when they depend on who it is;
    /// else [`ServerInfo::instructions`].
    fn instructions(&mut self) -> Option<String> {
        None
    }
    /// The documents a client may read.
    fn resources(&self) -> Vec<Resource> {
        Vec::new()
    }
    /// The text of resource `uri`, if there is one.
    fn read_resource(&mut self, _uri: &str) -> Option<String> {
        None
    }
    /// The prompts a client may offer.
    fn prompts(&self) -> Vec<Prompt> {
        Vec::new()
    }
    /// The text prompt `name` asks with `args`, if there is one.
    fn prompt(&mut self, _name: &str, _args: &Value) -> Option<String> {
        None
    }
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
            let mut capabilities = json!({"tools": {"listChanged": false}});
            if !host.resources().is_empty() {
                capabilities["resources"] = json!({"listChanged": false});
            }
            if !host.prompts().is_empty() {
                capabilities["prompts"] = json!({"listChanged": false});
            }
            let instructions = host
                .instructions()
                .unwrap_or_else(|| info.instructions.clone());
            Ok(json!({
                "protocolVersion": version,
                "capabilities": capabilities,
                "serverInfo": {"name": info.name, "version": info.version},
                "instructions": instructions,
            }))
        }
        "ping" => Ok(json!({})),
        "resources/list" => Ok(json!({
            "resources": host.resources().iter().map(|r| json!({
                "uri": r.uri,
                "name": r.name,
                "description": r.description,
                "mimeType": r.mime,
            })).collect::<Vec<_>>(),
        })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).ok_or_else(|| {
                RpcError::new(RpcError::INVALID_PARAMS, "a read names its resource's uri")
            })?;
            let mime = host
                .resources()
                .into_iter()
                .find(|r| r.uri == uri)
                .map(|r| r.mime)
                .ok_or_else(|| {
                    RpcError::new(RpcError::INVALID_PARAMS, format!("no resource `{uri}`"))
                })?;
            let text = host.read_resource(uri).ok_or_else(|| {
                RpcError::new(RpcError::INVALID_PARAMS, format!("no resource `{uri}`"))
            })?;
            Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
        }
        "prompts/list" => Ok(json!({
            "prompts": host.prompts().iter().map(|p| json!({
                "name": p.name,
                "title": p.title,
                "description": p.description,
                "arguments": p.arguments.iter().map(|(name, about, required)| json!({
                    "name": name,
                    "description": about,
                    "required": required,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })),
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
                RpcError::new(RpcError::INVALID_PARAMS, "a prompt is asked for by name")
            })?;
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let prompt = host.prompts().into_iter().find(|p| p.name == name);
            let text = prompt
                .as_ref()
                .and_then(|_| host.prompt(name, &args))
                .ok_or_else(|| {
                    RpcError::new(RpcError::INVALID_PARAMS, format!("no prompt `{name}`"))
                })?;
            Ok(json!({
                "description": prompt.map(|p| p.description).unwrap_or_default(),
                "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
            }))
        }
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
    use local_ipc::Stream as UnixStream;
    use std::time::Duration;

    struct Adder;

    impl ToolHost for Adder {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool {
                name: "add".into(),
                title: "Add".into(),
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

    /// Answers a call with its arguments as JSON text.
    struct Echo;

    impl ToolHost for Echo {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool {
                name: "echo".into(),
                title: "Echo".into(),
                description: "Echo".into(),
                input_schema: json!({"type": "object"}),
                read_only: true,
                always_load: false,
            }]
        }
        fn call(&mut self, _: &str, args: Value) -> ToolAnswer {
            ToolAnswer::text(args.to_string())
        }
    }

    /// A whole number in a call's arguments reaches the tool exactly, past
    /// 2^53 and past 2^63; one 64 bits cannot hold is refused, never run
    /// rounded to the nearest double.
    #[test]
    fn a_call_keeps_whole_numbers_exact_and_refuses_one_past_64_bits() {
        use std::io::{BufRead, BufReader, Write};
        let (mut ours, theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(theirs.try_clone().unwrap(), theirs, &info(), &mut Echo));
        let mut lines = BufReader::new(ours.try_clone().unwrap()).lines();
        let mut ask = |id: u32, n: &str| {
            writeln!(
                ours,
                r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"echo","arguments":{{"faces":[{n}]}}}}}}"#
            )
            .unwrap();
            serde_json::from_str::<Value>(&lines.next().unwrap().unwrap()).unwrap()
        };
        for n in [
            "2151931432261558259",
            "11208812177732762000",
            "-9223372036854775808",
        ] {
            let answer = ask(1, n);
            let text = answer["result"]["content"][0]["text"].as_str().unwrap();
            assert_eq!(text, format!(r#"{{"faces":[{n}]}}"#));
        }
        let refused = ask(2, "18446744073709551616");
        assert_eq!(refused["id"], 2);
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("18446744073709551616"), "{message}");
        assert!(crate::rpc::inexact_integer(r#"{"a":"99999999999999999999","b":1e30}"#).is_none());
    }

    /// A host with a document, a prompt and its own instructions.
    struct Library;

    impl ToolHost for Library {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool {
                name: "look".into(),
                title: "Look".into(),
                description: "Look".into(),
                input_schema: json!({"type": "object"}),
                read_only: true,
                always_load: false,
            }]
        }
        fn call(&mut self, _: &str, _: Value) -> ToolAnswer {
            ToolAnswer::text("seen")
        }
        fn instructions(&mut self) -> Option<String> {
            Some("You are inside the library.".into())
        }
        fn resources(&self) -> Vec<Resource> {
            vec![Resource {
                uri: "lib://rules".into(),
                name: "Rules".into(),
                description: "House rules".into(),
                mime: "text/markdown".into(),
            }]
        }
        fn read_resource(&mut self, uri: &str) -> Option<String> {
            (uri == "lib://rules").then(|| "Whisper.".to_string())
        }
        fn prompts(&self) -> Vec<Prompt> {
            vec![Prompt {
                name: "greet".into(),
                title: "Greet".into(),
                description: "Say hello".into(),
                arguments: vec![("who".into(), "Whom".into(), true)],
            }]
        }
        fn prompt(&mut self, _: &str, args: &Value) -> Option<String> {
            Some(format!("Hello, {}", args["who"].as_str().unwrap_or("you")))
        }
    }

    #[test]
    fn a_host_gives_its_own_instructions_documents_and_prompts() {
        let info = ServerInfo {
            name: "lib".into(),
            version: "1".into(),
            instructions: "fallback".into(),
        };
        let mut host = Library;
        let init = answer("initialize", &json!({}), &info, &mut host).unwrap();
        assert_eq!(init["instructions"], "You are inside the library.");
        assert!(init["capabilities"]["resources"].is_object());
        assert!(init["capabilities"]["prompts"].is_object());

        let tools = answer("tools/list", &json!({}), &info, &mut host).unwrap();
        let annotations = &tools["tools"][0]["annotations"];
        assert_eq!(annotations["readOnlyHint"], true);
        assert_eq!(annotations["destructiveHint"], false);
        assert_eq!(annotations["openWorldHint"], false);
        assert_eq!(tools["tools"][0]["title"], "Look");

        let listed = answer("resources/list", &json!({}), &info, &mut host).unwrap();
        assert_eq!(listed["resources"][0]["uri"], "lib://rules");
        let read = answer(
            "resources/read",
            &json!({"uri": "lib://rules"}),
            &info,
            &mut host,
        )
        .unwrap();
        assert_eq!(read["contents"][0]["text"], "Whisper.");
        assert_eq!(read["contents"][0]["mimeType"], "text/markdown");
        assert!(
            answer(
                "resources/read",
                &json!({"uri": "lib://none"}),
                &info,
                &mut host
            )
            .is_err()
        );

        let prompts = answer("prompts/list", &json!({}), &info, &mut host).unwrap();
        assert_eq!(prompts["prompts"][0]["arguments"][0]["required"], true);
        let got = answer(
            "prompts/get",
            &json!({"name": "greet", "arguments": {"who": "Ada"}}),
            &info,
            &mut host,
        )
        .unwrap();
        assert_eq!(got["messages"][0]["content"]["text"], "Hello, Ada");
        assert!(answer("prompts/get", &json!({"name": "shout"}), &info, &mut host).is_err());
    }

    #[test]
    fn a_host_without_them_keeps_the_fixed_instructions_and_offers_nothing_more() {
        let info = ServerInfo {
            name: "t".into(),
            version: "1".into(),
            instructions: "fixed".into(),
        };
        let init = answer("initialize", &json!({}), &info, &mut Adder).unwrap();
        assert_eq!(init["instructions"], "fixed");
        assert!(init["capabilities"].get("resources").is_none());
        assert!(init["capabilities"].get("prompts").is_none());
    }
}
