//! AI agents and the application: the two protocols between them.
//!
//! - [`acp`]: the application as a client of an agent that speaks the Agent
//!   Client Protocol. The agent runs as a program of its own; a chat sends
//!   it prompts and shows what it streams back, and answers its requests
//!   for permission.
//! - [`mcp`]: the application as a Model Context Protocol server, the tools
//!   an agent (or any MCP client) calls to read and change the document.
//! - [`bridge`]: `printcad --mcp`, which an agent starts as an MCP server
//!   over stdio and which relays to the running application's socket.
//!
//! Both protocols are JSON-RPC 2.0, one message a line ([`rpc`]).

pub mod acp;
pub mod bridge;
pub mod mcp;
pub mod rpc;
