//! A tool call an agent's MCP connection hands the UI thread, and a
//! change waiting for the user's OK.

use std::sync::mpsc::Sender;

use agents::mcp::ToolAnswer;
use serde_json::Value;

/// A tool call from a connection, for the UI thread.
pub struct ToolRequest {
    /// The chat whose agent asked, when a chat's did.
    pub chat: Option<String>,
    pub tool: String,
    pub args: Value,
    pub reply: Sender<ToolAnswer>,
}

/// A change an agent asked for, waiting for the user's OK.
pub struct Approval {
    pub chat: Option<String>,
    /// What it does, as a line of script.
    pub summary: String,
    pub request: ToolRequest,
}
