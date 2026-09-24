//! JSON-RPC 2.0 over a byte stream, one message a line: the framing both
//! the Agent Client Protocol and MCP over stdio use.
//!
//! A [`Connection`] sends requests and waits for their answers, sends
//! notifications and answers requests from the other side. A reader thread
//! sorts what arrives: answers go to whoever is waiting for them, requests
//! and notifications to [`Incoming`].

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// A JSON-RPC error answer.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;

    fn of(value: &Value) -> Self {
        Self {
            code: value
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or(Self::INTERNAL),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string(),
        }
    }
}

/// What the other side sent that is not an answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A request, to be answered with [`Connection::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// The stream ended.
    Closed,
}

type Waiting = Arc<Mutex<HashMap<u64, Sender<Result<Value, RpcError>>>>>;

/// One side of a JSON-RPC connection.
#[derive(Clone)]
pub struct Connection {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    next_id: Arc<AtomicU64>,
    waiting: Waiting,
}

impl Connection {
    /// Speak over `reader` and `writer`; what arrives that is not an answer
    /// goes to the receiver returned.
    pub fn new(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> (Self, Receiver<Incoming>) {
        let (incoming_tx, incoming_rx) = channel();
        let waiting: Waiting = Arc::default();
        let connection = Self {
            writer: Arc::new(Mutex::new(Box::new(writer))),
            next_id: Arc::new(AtomicU64::new(1)),
            waiting: waiting.clone(),
        };
        std::thread::Builder::new()
            .name("printcad-rpc-reader".to_string())
            .spawn(move || read_loop(BufReader::new(reader), waiting, incoming_tx))
            .expect("the reader thread starts");
        (connection, incoming_rx)
    }

    /// Send a request; its answer arrives on the receiver returned.
    pub fn request(&self, method: &str, params: Value) -> Receiver<Result<Value, RpcError>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        if let Ok(mut waiting) = self.waiting.lock() {
            waiting.insert(id, tx.clone());
        }
        let sent = self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        if let Err(err) = sent {
            let _ = tx.send(Err(RpcError::new(RpcError::INTERNAL, err.to_string())));
        }
        rx
    }

    pub fn notify(&self, method: &str, params: Value) -> std::io::Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// Answer the other side's request `id`.
    pub fn respond(&self, id: Value, answer: Result<Value, RpcError>) -> std::io::Result<()> {
        self.send(&match answer {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": err.code, "message": err.message},
            }),
        })
    }

    fn send(&self, message: &Value) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("the connection is broken"))?;
        writer.write_all(&line)?;
        writer.flush()
    }
}

fn read_loop(mut reader: impl BufRead, waiting: Waiting, incoming: Sender<Incoming>) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let id = message.get("id").cloned().filter(|v| !v.is_null());
        match (message.get("method").and_then(Value::as_str), id) {
            (Some(method), Some(id)) => {
                let _ = incoming.send(Incoming::Request {
                    id,
                    method: method.to_string(),
                    params: message.get("params").cloned().unwrap_or(Value::Null),
                });
            }
            (Some(method), None) => {
                let _ = incoming.send(Incoming::Notification {
                    method: method.to_string(),
                    params: message.get("params").cloned().unwrap_or(Value::Null),
                });
            }
            (None, Some(id)) => {
                let answer = match message.get("error") {
                    Some(error) => Err(RpcError::of(error)),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let waiter = id
                    .as_u64()
                    .and_then(|id| waiting.lock().ok().and_then(|mut w| w.remove(&id)));
                if let Some(waiter) = waiter {
                    let _ = waiter.send(answer);
                }
            }
            (None, None) => {}
        }
    }
    // Nobody will answer what is still waiting.
    if let Ok(mut waiting) = waiting.lock() {
        for (_, waiter) in waiting.drain() {
            let _ = waiter.send(Err(RpcError::new(
                RpcError::INTERNAL,
                "the connection closed",
            )));
        }
    }
    let _ = incoming.send(Incoming::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    type Side = (Connection, Receiver<Incoming>);

    /// Two connected sides, and the second side's socket, to close it.
    fn pair() -> (Side, Side, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        let handle = b.try_clone().unwrap();
        (
            Connection::new(a.try_clone().unwrap(), a),
            Connection::new(b.try_clone().unwrap(), b),
            handle,
        )
    }

    #[test]
    fn a_request_is_answered_and_a_notification_arrives() {
        let ((client, _), (server, server_in), _) = pair();
        let answer = client.request("add", json!({"a": 2, "b": 3}));
        let Incoming::Request { id, method, params } =
            server_in.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("a request")
        };
        assert_eq!(method, "add");
        let sum = params["a"].as_i64().unwrap() + params["b"].as_i64().unwrap();
        server.respond(id, Ok(json!(sum))).unwrap();
        assert_eq!(
            answer.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(json!(5))
        );

        client.notify("hello", json!({"x": 1})).unwrap();
        assert_eq!(
            server_in.recv_timeout(Duration::from_secs(2)).unwrap(),
            Incoming::Notification {
                method: "hello".into(),
                params: json!({"x": 1})
            }
        );
    }

    #[test]
    fn an_error_answer_and_a_closed_stream_reach_whoever_waits() {
        let ((client, _), (server, server_in), socket) = pair();
        let answer = client.request("nothing", Value::Null);
        let Incoming::Request { id, .. } = server_in.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!()
        };
        server
            .respond(
                id,
                Err(RpcError::new(RpcError::METHOD_NOT_FOUND, "no such method")),
            )
            .unwrap();
        assert_eq!(
            answer
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap_err()
                .code,
            RpcError::METHOD_NOT_FOUND
        );
        let pending = client.request("never", Value::Null);
        socket.shutdown(std::net::Shutdown::Both).unwrap();
        assert!(
            pending
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_err(),
            "a request left waiting when the stream ends is answered with an error"
        );
    }
}
