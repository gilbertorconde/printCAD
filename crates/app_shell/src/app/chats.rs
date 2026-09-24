//! Chats with AI agents: each one an agent's program, started over the
//! Agent Client Protocol, with this application's MCP server on its list
//! so it can reach the document.
//!
//! The app keeps every chat's state; what the agent streams arrives on
//! the chat's thread and is folded into its entries here, once a frame.
//! The assistant panel draws them and answers with commands.

use std::path::PathBuf;

use agents::acp::{
    AgentChat, Attachment, ChatCommand, ChatEvent, McpServer, PermissionOption, PlanEntry, Program,
    SessionOption,
};
use serde_json::Value;

use crate::PrintCadApp;

/// Where a chat stands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChatStatus {
    /// The agent is starting and opening its session.
    Starting,
    /// Waiting for a prompt.
    Ready,
    /// The agent is on a turn.
    Busy,
    /// The chat could not start, or its agent stopped, and why.
    Failed(String),
}

/// One thing shown in a chat.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChatEntry {
    /// What the user sent, and the names of what went with it.
    User {
        text: String,
        attachments: Vec<String>,
    },
    /// The agent's message, or its thinking.
    Agent {
        text: String,
        thought: bool,
    },
    Tool {
        id: String,
        title: String,
        kind: String,
        status: String,
        output: Option<String>,
    },
    Plan(Vec<PlanEntry>),
    /// The agent asks to be allowed something; `answer` is the choice once
    /// made.
    Permission {
        request: Value,
        title: String,
        options: Vec<PermissionOption>,
        answer: Option<String>,
    },
    /// A word from the application: an error, a stopped turn.
    Note(String),
}

pub(crate) struct Chat {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub status: ChatStatus,
    /// A change the agent asks for waits for the user's OK.
    pub ask: bool,
    pub entries: Vec<ChatEntry>,
    /// The session options the agent offers, as they stand.
    pub options: Vec<SessionOption>,
    /// What goes with the next prompt.
    pub attachments: Vec<Attachment>,
    /// The agent's remembered choices have been read.
    chose: bool,
    /// Remembered choices not yet put to the agent, in the order it lists
    /// its options: one may only become possible once another lands (an
    /// effort level the chosen model has).
    choices_left: Vec<(String, Value)>,
    /// The last lines the agent wrote on stderr, to say why it failed.
    stderr: Vec<String>,
    /// Prompts sent: the first carries a word on where the agent is.
    prompts: usize,
    session: AgentChat,
}

/// Where the chat's agent works: the document's folder, else home.
fn working_folder(file: Option<&std::path::Path>) -> PathBuf {
    file.and_then(|f| f.parent().map(std::path::Path::to_path_buf))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// What the first prompt of a chat starts with.
const PREAMBLE: &str = "You are working in printCAD, a parametric CAD application for \
3D printing, on the document the user has open. Its MCP server `printcad` has the tools \
to read and change it: start with `commands` to see what can be done.";

impl PrintCadApp {
    /// Start a chat with agent number `agent` of the Preferences.
    pub(crate) fn new_chat(&mut self, agent: usize) {
        let Some(config) = self.user_settings.ai.agents.get(agent).cloned() else {
            return;
        };
        let id = uuid::Uuid::new_v4().to_string();
        let mut mcp = Vec::new();
        if let (Some(server), Ok(exe)) = (&self.mcp, std::env::current_exe()) {
            mcp.push(McpServer {
                name: "printcad".to_string(),
                program: Program {
                    command: exe.display().to_string(),
                    args: vec![
                        "--mcp".into(),
                        "--socket".into(),
                        server.socket.display().to_string(),
                        "--chat".into(),
                        id.clone(),
                    ],
                    env: Vec::new(),
                },
            });
        }
        let session = AgentChat::start(
            &Program {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env.clone(),
            },
            working_folder(self.session.current_file.as_deref()),
            mcp,
            self.waker.clone(),
        );
        let number = self.chats_made + 1;
        self.chats_made = number;
        self.chats.push(Chat {
            id,
            title: format!("Chat {number}"),
            agent: config.name.clone(),
            status: ChatStatus::Starting,
            ask: self.user_settings.ai.ask_before_changes,
            entries: Vec::new(),
            options: Vec::new(),
            attachments: Vec::new(),
            chose: false,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 0,
            session,
        });
    }

    pub(crate) fn chat_asks(&self, id: &str) -> Option<bool> {
        self.chats.iter().find(|c| c.id == id).map(|c| c.ask)
    }

    fn chat_mut(&mut self, id: &str) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|c| c.id == id)
    }

    pub(crate) fn send_to_chat(&mut self, id: &str, text: String) {
        let Some(chat) = self.chat_mut(id) else {
            return;
        };
        if (text.trim().is_empty() && chat.attachments.is_empty())
            || matches!(chat.status, ChatStatus::Failed(_))
        {
            return;
        }
        chat.choices_left.clear();
        let attachments = std::mem::take(&mut chat.attachments);
        chat.entries.push(ChatEntry::User {
            text: text.clone(),
            attachments: attachments.iter().map(Attachment::name).collect(),
        });
        let prompt = if chat.prompts == 0 {
            format!("{PREAMBLE}\n\n{text}")
        } else {
            text
        };
        chat.prompts += 1;
        chat.session.send(ChatCommand::Prompt {
            text: prompt,
            attachments,
        });
        if chat.status == ChatStatus::Ready {
            chat.status = ChatStatus::Busy;
        }
    }

    /// Add files to what goes with the chat's next prompt.
    pub(crate) fn attach_files(&mut self, id: &str, paths: Vec<PathBuf>) {
        if let Some(chat) = self.chat_mut(id) {
            for path in paths {
                if !chat
                    .attachments
                    .iter()
                    .any(|a| matches!(a, Attachment::File(p) if *p == path))
                {
                    chat.attachments.push(Attachment::File(path));
                }
            }
        }
    }

    /// Add a picture of the scene as the user sees it.
    pub(crate) fn attach_view(&mut self, id: &str) {
        let Some(png) = self.view_png(1280, 960) else {
            crate::app_log::warn("Nothing is visible to attach a picture of");
            return;
        };
        if let Some(chat) = self.chat_mut(id) {
            let taken = chat
                .attachments
                .iter()
                .filter(|a| matches!(a, Attachment::Image { .. }))
                .count();
            let name = match taken {
                0 => "view.png".to_string(),
                n => format!("view-{}.png", n + 1),
            };
            chat.attachments.push(Attachment::Image {
                name,
                mime: "image/png".to_string(),
                data: png,
            });
        }
    }

    pub(crate) fn detach(&mut self, id: &str, index: usize) {
        if let Some(chat) = self.chat_mut(id)
            && index < chat.attachments.len()
        {
            chat.attachments.remove(index);
        }
    }

    /// Ask the chat's agent to stop its turn; its held changes are refused.
    pub(crate) fn cancel_chat(&mut self, id: &str) {
        if let Some(chat) = self.chat_mut(id) {
            chat.session.send(ChatCommand::Cancel);
        }
        let held: Vec<usize> = (0..self.approvals.len())
            .rev()
            .filter(|i| self.approvals[*i].chat.as_deref() == Some(id))
            .collect();
        for index in held {
            self.settle_approval(index, false);
        }
    }

    /// Close a chat: its agent's program ends with it.
    pub(crate) fn close_chat(&mut self, id: &str) {
        self.cancel_chat(id);
        self.chats.retain(|c| c.id != id);
    }

    pub(crate) fn answer_permission(&mut self, id: &str, entry: usize, option: Option<String>) {
        let Some(chat) = self.chat_mut(id) else {
            return;
        };
        if let Some(ChatEntry::Permission {
            request, answer, ..
        }) = chat.entries.get_mut(entry)
            && answer.is_none()
        {
            *answer = Some(option.clone().unwrap_or_else(|| "refused".to_string()));
            chat.session.send(ChatCommand::Permission {
                request: request.clone(),
                option,
            });
        }
    }

    /// Set whether the chat's changes wait for an OK; turned off, the ones
    /// waiting go ahead.
    pub(crate) fn set_chat_asks(&mut self, id: &str, ask: bool) {
        if let Some(chat) = self.chat_mut(id) {
            chat.ask = ask;
        }
        if !ask {
            let held: Vec<usize> = (0..self.approvals.len())
                .rev()
                .filter(|i| self.approvals[*i].chat.as_deref() == Some(id))
                .collect();
            for index in held.into_iter().rev() {
                // Run oldest first; indices shift as each one goes.
                let at = index.min(self.approvals.len().saturating_sub(1));
                self.settle_approval(at, true);
            }
        }
    }

    /// Change a session option of the chat's agent, and remember the
    /// choice for the agent's next chats.
    pub(crate) fn set_chat_option(&mut self, id: &str, option: String, value: Value) {
        let Some(chat) = self.chat_mut(id) else {
            return;
        };
        let agent = chat.agent.clone();
        chat.choices_left.clear();
        chat.session.send(ChatCommand::SetOption {
            id: option.clone(),
            value: value.clone(),
        });
        if let Some(config) = self
            .user_settings
            .ai
            .agents
            .iter_mut()
            .find(|a| a.name == agent)
        {
            config.choices.insert(option, value);
            if let Err(err) = self.settings_store.save(&self.user_settings) {
                crate::app_log::warn(format!("Could not save the agent's choice: {err}"));
            }
        }
    }

    /// Fold what every chat's agent sent since the last frame into the
    /// chat.
    pub(crate) fn drive_chats(&mut self) {
        let mut attention = false;
        for chat in &mut self.chats {
            while let Some(event) = chat.session.try_event() {
                attention |= matches!(event, ChatEvent::Permission { .. });
                apply(chat, event);
            }
            if !chat.chose && !chat.options.is_empty() {
                chat.chose = true;
                if let Some(config) = self
                    .user_settings
                    .ai
                    .agents
                    .iter()
                    .find(|a| a.name == chat.agent)
                {
                    chat.choices_left = chat
                        .options
                        .iter()
                        .filter_map(|o| Some((o.id.clone(), config.choices.get(&o.id)?.clone())))
                        .collect();
                }
            }
            put_choices(chat);
        }
        if attention {
            self.assistant_attention = true;
        }
    }
}

#[cfg(test)]
impl Chat {
    /// A chat with these entries whose agent never answers.
    pub(crate) fn for_test(id: &str, status: ChatStatus, entries: Vec<ChatEntry>) -> Self {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        Chat {
            id: id.into(),
            title: "Chat 1".into(),
            agent: "Test".into(),
            status,
            ask: true,
            entries,
            options: Vec::new(),
            attachments: Vec::new(),
            chose: true,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 1,
            session: AgentChat::over(
                ours.try_clone().unwrap(),
                ours,
                PathBuf::from("/"),
                Vec::new(),
                std::sync::Arc::new(|| {}),
            ),
        }
    }
}

/// Put the remembered choices the agent's options can take now to it;
/// the rest wait for an update that makes them possible.
fn put_choices(chat: &mut Chat) {
    let options = &chat.options;
    let session = &chat.session;
    chat.choices_left.retain(|(id, value)| {
        let Some(option) = options.iter().find(|o| &o.id == id) else {
            return true;
        };
        if option.current() == *value {
            return false;
        }
        if !option.accepts(value) {
            return true;
        }
        session.send(ChatCommand::SetOption {
            id: id.clone(),
            value: value.clone(),
        });
        false
    });
}

/// Fold one event into a chat.
fn apply(chat: &mut Chat, event: ChatEvent) {
    match event {
        ChatEvent::Ready => chat.status = ChatStatus::Ready,
        ChatEvent::Options(options) => chat.options = options,
        ChatEvent::Failed(why) => {
            let tail = chat.stderr.join("\n");
            chat.status = ChatStatus::Failed(if tail.is_empty() {
                why
            } else {
                format!("{why}\n{tail}")
            });
        }
        ChatEvent::Text { thought, text } => match chat.entries.last_mut() {
            Some(ChatEntry::Agent {
                text: last,
                thought: was,
            }) if *was == thought => last.push_str(&text),
            _ => chat.entries.push(ChatEntry::Agent { text, thought }),
        },
        ChatEvent::ToolCall {
            id,
            title,
            kind,
            status,
        } => chat.entries.push(ChatEntry::Tool {
            id,
            title,
            kind,
            status,
            output: None,
        }),
        ChatEvent::ToolCallUpdate {
            id,
            title,
            status,
            output,
        } => {
            let found = chat.entries.iter_mut().rev().find_map(|e| match e {
                ChatEntry::Tool {
                    id: tool,
                    title: t,
                    status: s,
                    output: o,
                    ..
                } if *tool == id => Some((t, s, o)),
                _ => None,
            });
            if let Some((t, s, o)) = found {
                if let Some(title) = title {
                    *t = title;
                }
                if let Some(status) = status {
                    *s = status;
                }
                if output.is_some() {
                    *o = output;
                }
            }
        }
        ChatEvent::Plan(entries) => match chat.entries.last_mut() {
            Some(ChatEntry::Plan(last)) => *last = entries,
            _ => chat.entries.push(ChatEntry::Plan(entries)),
        },
        ChatEvent::Permission {
            request,
            title,
            options,
        } => chat.entries.push(ChatEntry::Permission {
            request,
            title,
            options,
            answer: None,
        }),
        ChatEvent::TurnEnded { stop_reason } => {
            if chat.status == ChatStatus::Busy {
                chat.status = ChatStatus::Ready;
            }
            match stop_reason.as_str() {
                "end_turn" => {}
                "cancelled" => chat.entries.push(ChatEntry::Note("Stopped".into())),
                "max_tokens" => chat.entries.push(ChatEntry::Note(
                    "The agent ran out of room for its answer".into(),
                )),
                "refusal" => chat
                    .entries
                    .push(ChatEntry::Note("The agent declined to go on".into())),
                other => chat
                    .entries
                    .push(ChatEntry::Note(format!("The turn ended: {other}"))),
            }
        }
        ChatEvent::Error(why) => chat.entries.push(ChatEntry::Note(why)),
        ChatEvent::Stderr(line) => {
            chat.stderr.push(line);
            if chat.stderr.len() > 12 {
                chat.stderr.remove(0);
            }
        }
        ChatEvent::Exited => {
            if !matches!(chat.status, ChatStatus::Failed(_)) {
                let tail = chat.stderr.join("\n");
                chat.status = ChatStatus::Failed(if tail.is_empty() {
                    "The agent's program ended".to_string()
                } else {
                    format!("The agent's program ended:\n{tail}")
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat() -> Chat {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        Chat {
            id: "c".into(),
            title: "Chat 1".into(),
            agent: "Test".into(),
            status: ChatStatus::Busy,
            ask: true,
            entries: Vec::new(),
            options: Vec::new(),
            attachments: Vec::new(),
            chose: false,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 1,
            session: AgentChat::over(
                ours.try_clone().unwrap(),
                ours,
                PathBuf::from("/"),
                Vec::new(),
                std::sync::Arc::new(|| {}),
            ),
        }
    }

    fn select(id: &str, category: &str, current: &str, values: &[&str]) -> SessionOption {
        SessionOption {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            category: category.into(),
            value: agents::acp::OptionValue::Select {
                current: current.into(),
                choices: values
                    .iter()
                    .map(|v| agents::acp::OptionChoice {
                        value: v.to_string(),
                        name: v.to_string(),
                        description: String::new(),
                    })
                    .collect(),
            },
            via: agents::acp::OptionVia::Config,
        }
    }

    #[test]
    fn remembered_choices_go_in_order_and_wait_until_the_agent_can_take_them() {
        let mut c = chat();
        apply(
            &mut c,
            ChatEvent::Options(vec![
                select("mode", "mode", "ask", &["ask", "plan"]),
                select("model", "model", "small", &["small", "large"]),
                select("effort", "thought_level", "low", &["low"]),
            ]),
        );
        c.choices_left = vec![
            ("mode".into(), Value::from("ask")),
            ("model".into(), Value::from("large")),
            ("effort".into(), Value::from("max")),
        ];
        put_choices(&mut c);
        // The mode is as remembered and the model goes out; the small
        // model has no "max", so the effort waits.
        assert_eq!(c.choices_left, [("effort".to_string(), Value::from("max"))]);
        apply(
            &mut c,
            ChatEvent::Options(vec![
                select("mode", "mode", "ask", &["ask", "plan"]),
                select("model", "model", "large", &["small", "large"]),
                select("effort", "thought_level", "low", &["low", "max"]),
            ]),
        );
        put_choices(&mut c);
        assert!(c.choices_left.is_empty(), "the large model takes it");
    }

    #[test]
    fn streamed_pieces_join_and_a_tool_call_moves_on_in_place() {
        let mut c = chat();
        for piece in ["Pad", "ding it"] {
            apply(
                &mut c,
                ChatEvent::Text {
                    thought: false,
                    text: piece.into(),
                },
            );
        }
        apply(
            &mut c,
            ChatEvent::ToolCall {
                id: "t".into(),
                title: "call".into(),
                kind: "edit".into(),
                status: "pending".into(),
            },
        );
        apply(
            &mut c,
            ChatEvent::ToolCallUpdate {
                id: "t".into(),
                title: Some("Pad".into()),
                status: Some("completed".into()),
                output: Some("pad1".into()),
            },
        );
        apply(
            &mut c,
            ChatEvent::TurnEnded {
                stop_reason: "cancelled".into(),
            },
        );
        assert_eq!(
            c.entries,
            [
                ChatEntry::Agent {
                    text: "Padding it".into(),
                    thought: false
                },
                ChatEntry::Tool {
                    id: "t".into(),
                    title: "Pad".into(),
                    kind: "edit".into(),
                    status: "completed".into(),
                    output: Some("pad1".into()),
                },
                ChatEntry::Note("Stopped".into()),
            ]
        );
        assert_eq!(c.status, ChatStatus::Ready);
    }

    #[test]
    fn a_program_that_ends_says_what_it_last_wrote() {
        let mut c = chat();
        apply(&mut c, ChatEvent::Stderr("not signed in".into()));
        apply(&mut c, ChatEvent::Exited);
        let ChatStatus::Failed(why) = &c.status else {
            panic!()
        };
        assert!(why.contains("not signed in"));
    }
}
