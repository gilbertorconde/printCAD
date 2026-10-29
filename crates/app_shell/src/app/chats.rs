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
    /// A chat from an earlier visit to the document: its agent starts,
    /// and its conversation comes back, once the chat is shown.
    Resting,
    /// The agent is starting and opening its session.
    Starting,
    /// Waiting for a prompt.
    Ready,
    /// The agent is on a turn.
    Busy,
    /// The chat could not start, or its agent stopped, and why.
    Failed(String),
}

/// Where a chat's title came from, which says what may replace it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Naming {
    /// "Chat 3": the agent is asked for one after its first answer.
    Numbered,
    /// The agent named its session on its own.
    Agent,
    /// The agent's answer when asked.
    Asked,
    /// The user's: only the user, or asking again, replaces it.
    User,
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
        /// The arguments, as the agent sent them.
        input: Option<serde_json::Value>,
        /// What the call does, in words, for one of printCAD's own tools
        /// (`agent_context::tool_label`); the agent's title otherwise.
        label: Option<String>,
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
    pub naming: Naming,
    /// A title is being asked for.
    pub titling: bool,
    /// The user asked for it: it replaces even their own.
    titling_asked: bool,
    /// The title after the first answer has been asked for, whatever came
    /// of it.
    titled_once: bool,
    pub agent: String,
    pub status: ChatStatus,
    /// A change the agent asks for waits for the user's OK.
    pub ask: bool,
    pub entries: Vec<ChatEntry>,
    /// The session options the agent offers, as they stand.
    pub options: Vec<SessionOption>,
    /// What goes with the next prompt.
    pub attachments: Vec<Attachment>,
    /// Messages written while the agent was on a turn, in the order they
    /// go: each goes when the turn before it ends.
    pub queued: Vec<Queued>,
    /// Stop holds what is queued until the user sends again or lets it go.
    pub held: bool,
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
    /// The user's rules as the agent was last told them: a change goes
    /// with the next prompt.
    rules_sent: String,
    /// The tab whose document the chat works on.
    pub tab: uuid::Uuid,
    /// The agent's session, kept with the document's file so the chat can
    /// continue when the file is opened again.
    pub session_id: Option<String>,
    /// The folder the agent was first started in: an agent keeps its
    /// sessions by the folder it ran in, so a resumed session must start
    /// there again, wherever the document has since been saved.
    pub folder: PathBuf,
    /// The running agent; `None` while the chat rests.
    session: Option<AgentChat>,
}

/// A message waiting for the agent's turn to end.
#[derive(Debug, Clone)]
pub(crate) struct Queued {
    /// Names it for the panel's edit and remove, whatever goes before it.
    pub id: u64,
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl Chat {
    fn send(&self, command: ChatCommand) {
        if let Some(session) = &self.session {
            session.send(command);
        }
    }

    /// Queue `text` with what is attached; sending again lets anything
    /// Stop held go too. A stopped chat takes nothing.
    fn enqueue(&mut self, text: String) {
        if (text.trim().is_empty() && self.attachments.is_empty())
            || matches!(self.status, ChatStatus::Failed(_))
        {
            return;
        }
        let attachments = std::mem::take(&mut self.attachments);
        let id = self.queued.iter().map(|q| q.id + 1).max().unwrap_or(0);
        self.queued.push(Queued {
            id,
            text,
            attachments,
        });
        self.held = false;
    }

    /// Take queued message `id` out, its attachments back with the next
    /// prompt when `keep`.
    fn unqueue(&mut self, id: u64, keep: bool) {
        let Some(at) = self.queued.iter().position(|q| q.id == id) else {
            return;
        };
        let taken = self.queued.remove(at);
        if keep {
            for attachment in taken.attachments {
                if !self.attachments.contains(&attachment) {
                    self.attachments.push(attachment);
                }
            }
        }
        if self.queued.is_empty() {
            self.held = false;
        }
    }

    /// Ask the agent for a title for the conversation so far; `asked` when
    /// the user did, so it replaces their own. Nothing is asked while one
    /// is on its way, or of a conversation with no words in it yet.
    fn ask_title(&mut self, asked: bool) {
        let conversation = conversation_text(&self.entries);
        if self.titling || self.session.is_none() || conversation.is_empty() {
            return;
        }
        self.send(ChatCommand::SuggestTitle { conversation });
        self.titling = true;
        self.titling_asked = asked;
        self.titled_once = true;
    }

    /// Take a title the agent gave, unless it may not replace the one the
    /// chat has.
    fn take_title(&mut self, text: String, asked: bool) {
        if asked {
            self.titling = false;
        }
        let may = match asked {
            true => self.titling_asked || self.naming != Naming::User,
            false => matches!(self.naming, Naming::Numbered | Naming::Agent),
        };
        if !text.is_empty() && may {
            self.title = text;
            self.naming = if asked { Naming::Asked } else { Naming::Agent };
        }
    }

    /// Stop the agent's turn, holding what is queued.
    fn stop(&mut self) {
        self.send(ChatCommand::Cancel);
        self.held = !self.queued.is_empty();
    }

    /// Send the first queued message when the agent waits for one, with
    /// the word on where it is on the first and `rules` when they changed.
    fn send_next(&mut self, rules: String) {
        if self.status != ChatStatus::Ready || self.held || self.queued.is_empty() {
            return;
        }
        let Queued {
            text, attachments, ..
        } = self.queued.remove(0);
        self.choices_left.clear();
        self.entries.push(ChatEntry::User {
            text: text.clone(),
            attachments: attachments.iter().map(Attachment::name).collect(),
        });
        let prompt = if self.prompts == 0 {
            match rules.is_empty() {
                true => format!("{PREAMBLE}\n\n{text}"),
                false => format!("{PREAMBLE}\n\nThe user's rules; keep to them:\n{rules}\n{text}"),
            }
        } else if rules != self.rules_sent {
            format!(
                "(The user's rules for printCAD changed; keep to these from now on:\n{rules})\n\n{text}"
            )
        } else {
            text
        };
        self.rules_sent = rules;
        self.prompts += 1;
        self.send(ChatCommand::Prompt {
            text: prompt,
            attachments,
        });
        self.status = ChatStatus::Busy;
    }
}

/// The longest message the title is asked with, in characters, and the
/// longest conversation: a longer one keeps its start and its end.
const TITLE_MESSAGE: usize = 800;
const TITLE_HEAD: usize = 1500;
const TITLE_TAIL: usize = 4500;

/// The words of a conversation, the user's and the agent's, for the agent
/// to name it by.
fn conversation_text(entries: &[ChatEntry]) -> String {
    let cut = |text: &str, most: usize| -> String {
        let text = text.trim();
        match text.chars().count() > most {
            true => format!("{}…", text.chars().take(most).collect::<String>()),
            false => text.to_string(),
        }
    };
    let said: Vec<String> = entries
        .iter()
        .filter_map(|e| match e {
            ChatEntry::User { text, .. } if !text.trim().is_empty() => {
                Some(format!("User: {}", cut(text, TITLE_MESSAGE)))
            }
            ChatEntry::Agent {
                text,
                thought: false,
            } if !text.trim().is_empty() => {
                Some(format!("Assistant: {}", cut(text, TITLE_MESSAGE)))
            }
            _ => None,
        })
        .collect();
    let all = said.join("\n\n");
    let count = all.chars().count();
    if count <= TITLE_HEAD + TITLE_TAIL {
        return all;
    }
    let head: String = all.chars().take(TITLE_HEAD).collect();
    let tail: String = all.chars().skip(count - TITLE_TAIL).collect();
    format!("{head}\n…\n{tail}")
}

/// Where the chat's agent works: the document's folder, else home.
fn working_folder(file: Option<&std::path::Path>) -> PathBuf {
    file.and_then(|f| f.parent().map(std::path::Path::to_path_buf))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// What the first prompt of a chat starts with.
const PREAMBLE: &str = "You are working inside printCAD, a parametric CAD application for \
3D printing, running on the user's desktop while they watch, on the document of the tab \
this chat belongs to. Its MCP server `printcad` has the tools to read and change it: \
`context` says what is open, `commands` what can be done. Never close printCAD or its \
tabs: that would end this session.";

impl PrintCadApp {
    /// Start a chat with agent number `agent` of the Preferences, on the
    /// document on screen.
    pub(crate) fn new_chat(&mut self, agent: usize) {
        let Some(config) = self.user_settings.ai.agents.get(agent).cloned() else {
            return;
        };
        let id = uuid::Uuid::new_v4().to_string();
        let folder = working_folder(self.session.current_file.as_deref());
        let session = self.start_agent(&config, &id, None, &folder);
        let number = self.chats_made + 1;
        self.chats_made = number;
        self.chats.push(Chat {
            id,
            title: format!("Chat {number}"),
            naming: Naming::Numbered,
            titling: false,
            titling_asked: false,
            titled_once: false,
            agent: config.name.clone(),
            status: ChatStatus::Starting,
            ask: self.user_settings.ai.ask_before_changes,
            entries: Vec::new(),
            options: Vec::new(),
            attachments: Vec::new(),
            queued: Vec::new(),
            held: false,
            chose: false,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 0,
            rules_sent: String::new(),
            tab: self.session.tab,
            session_id: None,
            folder,
            session: Some(session),
        });
    }

    /// Start `config`'s program for chat `id`, with this application's MCP
    /// server, in `folder`; in session `resume` when there is one.
    fn start_agent(
        &self,
        config: &settings::AgentSettings,
        id: &str,
        resume: Option<String>,
        folder: &std::path::Path,
    ) -> AgentChat {
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
                        id.to_string(),
                    ],
                    env: Vec::new(),
                },
            });
        }
        AgentChat::start(
            &Program {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env.clone(),
            },
            folder.to_path_buf(),
            mcp,
            resume,
            self.waker.clone(),
        )
    }

    /// Bring back the chats kept with the file of the document on screen,
    /// resting until each is shown.
    pub(crate) fn restore_chats(&mut self) {
        let Some(file) = self.session.current_file.clone() else {
            return;
        };
        let tab = self.session.tab;
        let saved = crate::app::chat_store::chats_of(&file);
        let mut restored = false;
        for kept in saved {
            if self
                .chats
                .iter()
                .any(|c| c.session_id.as_deref() == Some(kept.session.as_str()))
            {
                continue;
            }
            self.chats.push(Chat {
                id: uuid::Uuid::new_v4().to_string(),
                title: kept.title,
                naming: if kept.named {
                    Naming::User
                } else {
                    Naming::Asked
                },
                titling: false,
                titling_asked: false,
                titled_once: true,
                agent: kept.agent,
                status: ChatStatus::Resting,
                ask: self.user_settings.ai.ask_before_changes,
                entries: Vec::new(),
                options: Vec::new(),
                attachments: Vec::new(),
                queued: Vec::new(),
                held: false,
                chose: false,
                choices_left: Vec::new(),
                stderr: Vec::new(),
                // The conversation already began: no word on where it is,
                // but the rules go again with the next prompt, in case they
                // changed since.
                prompts: 1,
                rules_sent: String::new(),
                tab,
                session_id: Some(kept.session),
                folder: kept
                    .folder
                    .unwrap_or_else(|| working_folder(Some(file.as_path()))),
                session: None,
            });
            restored = true;
        }
        if restored {
            self.assistant_attention = true;
        }
    }

    /// Start a resting chat's agent in its earlier session; its
    /// conversation comes back as the agent replays it.
    pub(crate) fn wake_chat(&mut self, id: &str) {
        let Some(chat) = self.chats.iter().find(|c| c.id == id) else {
            return;
        };
        if chat.status != ChatStatus::Resting {
            return;
        }
        let (agent, resume, folder) = (
            chat.agent.clone(),
            chat.session_id.clone(),
            chat.folder.clone(),
        );
        let config = self
            .user_settings
            .ai
            .agents
            .iter()
            .find(|a| a.name == agent)
            .cloned();
        let session = config.map(|config| self.start_agent(&config, id, resume, &folder));
        let Some(chat) = self.chat_mut(id) else {
            return;
        };
        match session {
            Some(session) => {
                chat.session = Some(session);
                chat.status = ChatStatus::Starting;
            }
            None => {
                chat.status = ChatStatus::Failed(format!(
                    "No agent called {agent} is set up any more: add it again in Preferences › AI agents"
                ))
            }
        }
    }

    /// Keep each document's chats with its file, for the next time it is
    /// opened: the chats of every tab that has a file and a session.
    pub(crate) fn persist_chats(&self) {
        let mut files: Vec<(uuid::Uuid, PathBuf)> = Vec::new();
        if let Some(file) = &self.session.current_file {
            files.push((self.session.tab, file.clone()));
        }
        for slot in &self.tabs {
            if let Some(parked) = &slot.parked
                && let Some(file) = &parked.current_file
            {
                files.push((parked.tab, file.clone()));
            }
        }
        for (tab, file) in files {
            let kept: Vec<crate::app::chat_store::SavedChat> = self
                .chats
                .iter()
                .filter(|c| c.tab == tab)
                .filter_map(|c| {
                    Some(crate::app::chat_store::SavedChat {
                        agent: c.agent.clone(),
                        session: c.session_id.clone()?,
                        title: c.title.clone(),
                        named: c.naming == Naming::User,
                        folder: Some(c.folder.clone()),
                    })
                })
                .collect();
            crate::app::chat_store::keep(&file, kept);
        }
    }

    /// Name the chat as the user typed it; an empty name changes nothing.
    pub(crate) fn rename_chat(&mut self, id: &str, title: &str) {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(chat) = self.chat_mut(id) else {
            return;
        };
        if title.is_empty() {
            return;
        }
        chat.title = title;
        chat.naming = Naming::User;
        self.persist_chats();
    }

    /// Ask the chat's agent for a new title, which replaces any.
    pub(crate) fn retitle_chat(&mut self, id: &str) {
        if let Some(chat) = self.chat_mut(id) {
            chat.ask_title(true);
        }
    }

    pub(crate) fn chat_asks(&self, id: &str) -> Option<bool> {
        self.chats.iter().find(|c| c.id == id).map(|c| c.ask)
    }

    fn chat_mut(&mut self, id: &str) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|c| c.id == id)
    }

    /// Queue a message, with what is attached, and send it at once when
    /// the agent is waiting for one.
    pub(crate) fn send_to_chat(&mut self, id: &str, text: String) {
        self.wake_chat(id);
        if let Some(chat) = self.chat_mut(id) {
            chat.enqueue(text);
        }
        self.send_queued(id);
    }

    /// Take a queued message back; its attachments go with the next prompt
    /// again when it is being edited (`keep`).
    pub(crate) fn unqueue_chat(&mut self, id: &str, queued: u64, keep: bool) {
        if let Some(chat) = self.chat_mut(id) {
            chat.unqueue(queued, keep);
        }
    }

    /// Let what Stop held go on.
    pub(crate) fn resume_chat_queue(&mut self, id: &str) {
        if let Some(chat) = self.chat_mut(id) {
            chat.held = false;
        }
        self.send_queued(id);
    }

    /// Send the chat's next queued message, when its agent waits for one:
    /// the user's rules go with it as they stand when it goes.
    fn send_queued(&mut self, id: &str) {
        let rules = self.agent_rules(Some(id));
        if let Some(chat) = self.chat_mut(id) {
            chat.send_next(rules);
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
        self.attach_picture(id, "view", png);
    }

    /// Add the picture on the clipboard; a clipboard without one adds
    /// nothing.
    pub(crate) fn attach_clipboard(&mut self, id: &str) {
        let picture = arboard::Clipboard::new().and_then(|mut c| c.get_image());
        let png = match picture {
            Ok(image) => {
                crate::app::animation::png_of(image.width as u32, image.height as u32, &image.bytes)
            }
            Err(arboard::Error::ContentNotAvailable) => return,
            Err(err) => Err(err.to_string()),
        };
        match png {
            Ok(png) => self.attach_picture(id, "pasted", png),
            Err(err) => {
                crate::app_log::warn(format!("Could not read the clipboard's picture: {err}"))
            }
        }
    }

    /// Add a PNG named `<stem>.png`, numbered after the first.
    fn attach_picture(&mut self, id: &str, stem: &str, png: Vec<u8>) {
        if let Some(chat) = self.chat_mut(id) {
            let taken = chat
                .attachments
                .iter()
                .filter(|a| matches!(a, Attachment::Image { name, .. } if name.starts_with(stem)))
                .count();
            let name = match taken {
                0 => format!("{stem}.png"),
                n => format!("{stem}-{}.png", n + 1),
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
            chat.stop();
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
        self.persist_chats();
    }

    pub(crate) fn answer_permission(&mut self, id: &str, entry: usize, option: Option<String>) {
        let Some(chat) = self.chat_mut(id) else {
            return;
        };
        let request = match chat.entries.get_mut(entry) {
            Some(ChatEntry::Permission {
                request, answer, ..
            }) if answer.is_none() => {
                *answer = Some(option.clone().unwrap_or_else(|| "refused".to_string()));
                request.clone()
            }
            _ => return,
        };
        chat.send(ChatCommand::Permission { request, option });
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
        chat.send(ChatCommand::SetOption {
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
        let mut learned = false;
        let mut retitled = false;
        let mut notices = Vec::new();
        let mut specs: Option<Vec<core_document::CommandSpec>> = None;
        for chat in &mut self.chats {
            while let Some(event) = chat.session.as_ref().and_then(AgentChat::try_event) {
                attention |= matches!(event, ChatEvent::Permission { .. });
                learned |= matches!(event, ChatEvent::Session { .. });
                let touched = match &event {
                    ChatEvent::ToolCall { id, .. } | ChatEvent::ToolCallUpdate { id, .. } => {
                        Some(id.clone())
                    }
                    _ => None,
                };
                retitled |= matches!(event, ChatEvent::Title { .. });
                notices.extend(notice(chat, &event));
                apply(chat, event);
                // A call of printCAD's own tools says what it does.
                if let Some(touched) = touched
                    && let Some(ChatEntry::Tool {
                        title,
                        input,
                        label,
                        ..
                    }) = chat
                        .entries
                        .iter_mut()
                        .rev()
                        .find(|e| matches!(e, ChatEntry::Tool { id, .. } if *id == touched))
                {
                    let specs = specs
                        .get_or_insert_with(|| crate::app::scripts::command_specs(&self.registry));
                    let about =
                        |id: &str| specs.iter().find(|s| s.id == id).map(|s| s.summary.clone());
                    *label = crate::app::agent_context::tool_label(title, input.as_ref(), &about);
                }
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
            // The agent's first answer names the chat.
            if chat.naming == Naming::Numbered
                && !chat.titled_once
                && chat.status == ChatStatus::Ready
                && chat
                    .entries
                    .iter()
                    .any(|e| matches!(e, ChatEntry::Agent { thought: false, .. }))
            {
                chat.ask_title(false);
            }
        }
        // A turn that ended lets the next queued message go.
        let waiting: Vec<String> = self
            .chats
            .iter()
            .filter(|c| c.status == ChatStatus::Ready && !c.held && !c.queued.is_empty())
            .map(|c| c.id.clone())
            .collect();
        for id in waiting {
            self.send_queued(&id);
        }
        // A chat that learned its session is kept with its document.
        for (summary, body) in notices {
            self.notify(summary, &body);
        }
        if learned || retitled {
            self.persist_chats();
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
        let (ours, _theirs) = local_ipc::Stream::pair().unwrap();
        Chat {
            id: id.into(),
            title: "Chat 1".into(),
            naming: Naming::Numbered,
            titling: false,
            titling_asked: false,
            titled_once: false,
            agent: "Test".into(),
            status,
            ask: true,
            entries,
            options: Vec::new(),
            attachments: Vec::new(),
            queued: Vec::new(),
            held: false,
            chose: true,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 1,
            rules_sent: String::new(),
            tab: uuid::Uuid::nil(),
            session_id: None,
            folder: PathBuf::new(),
            session: Some(AgentChat::over(
                ours.try_clone().unwrap(),
                ours,
                PathBuf::from("/"),
                Vec::new(),
                None,
                std::sync::Arc::new(|| {}),
            )),
        }
    }
}

/// Put the remembered choices the agent's options can take now to it;
/// the rest wait for an update that makes them possible.
fn put_choices(chat: &mut Chat) {
    let options = &chat.options;
    let session = &chat.session;
    let Some(session) = session else {
        return;
    };
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

/// What the user is told of `event`, while away, as a summary and a body:
/// a turn that ended (unless a queued message goes next, or the user
/// stopped it), a question from the agent, an agent that stopped.
fn notice(chat: &Chat, event: &ChatEvent) -> Option<(String, String)> {
    let title = &chat.title;
    let stopped = |why: &str| Some((format!("{title}: the agent stopped"), why.to_string()));
    match event {
        ChatEvent::TurnEnded { stop_reason } if chat.status == ChatStatus::Busy => {
            match stop_reason.as_str() {
                "end_turn" if chat.queued.is_empty() || chat.held => {
                    let said = chat.entries.iter().rev().find_map(|e| match e {
                        ChatEntry::Agent {
                            text,
                            thought: false,
                        } => Some(text.as_str()),
                        _ => None,
                    });
                    Some((
                        format!("{title}: the agent is done"),
                        said.unwrap_or("Its turn ended").to_string(),
                    ))
                }
                "end_turn" | "cancelled" => None,
                "max_tokens" => stopped("It ran out of room for its answer"),
                "refusal" => stopped("It declined to go on"),
                other => stopped(&format!("The turn ended: {other}")),
            }
        }
        ChatEvent::Permission { title: asks, .. } => {
            Some((format!("{title}: the agent asks"), asks.clone()))
        }
        ChatEvent::Failed(why) => stopped(why.lines().next().unwrap_or_default()),
        ChatEvent::Exited if !matches!(chat.status, ChatStatus::Failed(_)) => {
            stopped("Its program ended")
        }
        _ => None,
    }
}

/// Fold one event into a chat.
fn apply(chat: &mut Chat, event: ChatEvent) {
    match event {
        ChatEvent::Ready => chat.status = ChatStatus::Ready,
        ChatEvent::Options(options) => chat.options = options,
        ChatEvent::Session { id, .. } => chat.session_id = Some(id),
        ChatEvent::UserText(text) => {
            // A replayed first prompt without the word on where it is.
            let text = text
                .strip_prefix(PREAMBLE)
                .map(|rest| rest.trim_start().to_string())
                .unwrap_or(text);
            match chat.entries.last_mut() {
                Some(ChatEntry::User { text: last, .. }) => last.push_str(&text),
                _ => chat.entries.push(ChatEntry::User {
                    text,
                    attachments: Vec::new(),
                }),
            }
        }
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
            input,
        } => chat.entries.push(ChatEntry::Tool {
            id,
            title,
            kind,
            status,
            output: None,
            input,
            label: None,
        }),
        ChatEvent::ToolCallUpdate {
            id,
            title,
            status,
            input,
            output,
        } => {
            let found = chat.entries.iter_mut().rev().find_map(|e| match e {
                ChatEntry::Tool {
                    id: tool,
                    title: t,
                    status: s,
                    output: o,
                    input: i,
                    ..
                } if *tool == id => Some((t, s, o, i)),
                _ => None,
            });
            if let Some((t, s, o, i)) = found {
                if let Some(title) = title {
                    *t = title;
                }
                if let Some(status) = status {
                    *s = status;
                }
                if input.is_some() {
                    *i = input;
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
        ChatEvent::Title { text, asked } => {
            chat.take_title(text, asked);
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

    /// A chat on its agent's turn, no choices put to it yet.
    fn chat() -> Chat {
        Chat {
            chose: false,
            ..Chat::for_test("c", ChatStatus::Busy, Vec::new())
        }
    }

    fn end_turn(c: &mut Chat, stop_reason: &str) {
        apply(
            c,
            ChatEvent::TurnEnded {
                stop_reason: stop_reason.into(),
            },
        );
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

    fn sent(c: &Chat) -> Vec<String> {
        c.entries
            .iter()
            .filter_map(|e| match e {
                ChatEntry::User { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Written while the agent works, a message waits, not shown as sent,
    /// and goes when the turn ends, one per turn, in order.
    #[test]
    fn messages_written_during_a_turn_wait_for_it_to_end() {
        let mut c = chat();
        c.enqueue("taller".into());
        c.enqueue("and red".into());
        c.send_next(String::new());
        assert!(sent(&c).is_empty(), "the agent is still on its turn");
        assert_eq!(c.queued.len(), 2);

        end_turn(&mut c, "end_turn");
        c.send_next(String::new());
        assert_eq!(sent(&c), ["taller"]);
        assert_eq!(c.status, ChatStatus::Busy);
        c.send_next(String::new());
        assert_eq!(sent(&c), ["taller"], "one per turn");

        end_turn(&mut c, "end_turn");
        c.send_next(String::new());
        assert_eq!(sent(&c), ["taller", "and red"]);
        assert!(c.queued.is_empty());
    }

    /// A queued message comes back to be edited with what was attached to
    /// it, or is dropped; the rest keep their order.
    #[test]
    fn a_queued_message_can_be_taken_back_or_dropped() {
        let mut c = chat();
        c.attachments.push(Attachment::File("/tmp/a.png".into()));
        c.enqueue("first".into());
        c.enqueue("second".into());
        c.enqueue("third".into());
        let (first, second) = (c.queued[0].id, c.queued[1].id);

        c.unqueue(first, true);
        assert_eq!(c.attachments, [Attachment::File("/tmp/a.png".into())]);
        c.unqueue(second, false);
        assert!(c.attachments.len() == 1, "a dropped one's files go with it");
        let left: Vec<&str> = c.queued.iter().map(|q| q.text.as_str()).collect();
        assert_eq!(left, ["third"]);
        // Taken back and queued again, it goes after what was waiting.
        c.enqueue("first, edited".into());
        let order: Vec<&str> = c.queued.iter().map(|q| q.text.as_str()).collect();
        assert_eq!(order, ["third", "first, edited"]);
        assert!(c.queued[1].attachments.len() == 1);
    }

    /// Stop holds what is queued: the end of the stopped turn sends
    /// nothing until the user sends again.
    #[test]
    fn stop_holds_the_queue_until_the_user_sends() {
        let mut c = chat();
        c.enqueue("taller".into());
        c.stop();
        assert!(c.held);
        end_turn(&mut c, "cancelled");
        c.send_next(String::new());
        assert!(sent(&c).is_empty(), "held");

        c.enqueue("wider".into());
        c.send_next(String::new());
        assert_eq!(
            sent(&c),
            ["taller"],
            "sending again lets the queue go, in order"
        );
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
    fn a_replayed_conversation_comes_back_as_it_was_typed() {
        let mut c = chat();
        c.entries.clear();
        for event in [
            ChatEvent::UserText(format!("{PREAMBLE}\n\nmake a box")),
            ChatEvent::Text {
                thought: false,
                text: "Done.".into(),
            },
            ChatEvent::UserText("taller".into()),
            ChatEvent::Session {
                id: "s1".into(),
                resumed: true,
            },
        ] {
            apply(&mut c, event);
        }
        assert_eq!(
            sent(&c),
            ["make a box", "taller"],
            "without the preamble, in order"
        );
        assert_eq!(c.session_id.as_deref(), Some("s1"));
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
                input: None,
                id: "t".into(),
                title: "call".into(),
                kind: "edit".into(),
                status: "pending".into(),
            },
        );
        apply(
            &mut c,
            ChatEvent::ToolCallUpdate {
                input: None,
                id: "t".into(),
                title: Some("Pad".into()),
                status: Some("completed".into()),
                output: Some("pad1".into()),
            },
        );
        end_turn(&mut c, "cancelled");
        assert_eq!(
            c.entries,
            [
                ChatEntry::Agent {
                    text: "Padding it".into(),
                    thought: false
                },
                ChatEntry::Tool {
                    input: None,
                    label: None,
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

    #[test]
    fn a_title_replaces_the_chats_as_its_naming_allows() {
        let title = |text: &str, asked: bool| ChatEvent::Title {
            text: text.into(),
            asked,
        };
        // After the first answer the agent is asked once; its own name for
        // the session counts only until then.
        let mut c = chat();
        c.status = ChatStatus::Ready;
        c.entries = vec![
            ChatEntry::User {
                text: "make a box".into(),
                attachments: Vec::new(),
            },
            ChatEntry::Agent {
                text: "Done.".into(),
                thought: false,
            },
        ];
        c.ask_title(false);
        assert!(c.titling && c.titled_once);
        apply(&mut c, title("Its own name", false));
        assert_eq!(
            (c.title.as_str(), c.naming),
            ("Its own name", Naming::Agent)
        );
        apply(&mut c, title("Box", true));
        assert_eq!((c.title.as_str(), c.naming), ("Box", Naming::Asked));
        assert!(!c.titling);
        apply(&mut c, title("Later name", false));
        assert_eq!(c.title, "Box");
        // The user's name stays until they ask the agent for a new one; an
        // answer the agent could not give changes nothing.
        c.title = "Mine".into();
        c.naming = Naming::User;
        for (event, kept) in [
            (title("Its own name", false), "Mine"),
            (title("", true), "Mine"),
        ] {
            apply(&mut c, event);
            assert_eq!(c.title, kept);
        }
        c.ask_title(true);
        apply(&mut c, title("Box with a lid", true));
        assert_eq!(
            (c.title.as_str(), c.naming),
            ("Box with a lid", Naming::Asked)
        );
    }

    #[test]
    fn a_long_conversation_is_named_from_its_start_and_its_end() {
        let said = |text: String| ChatEntry::User {
            text,
            attachments: Vec::new(),
        };
        let entries: Vec<ChatEntry> = (0..40)
            .map(|i| said(format!("{i} {}", "x".repeat(400))))
            .collect();
        let text = conversation_text(&entries);
        assert!(text.chars().count() <= TITLE_HEAD + TITLE_TAIL + 3);
        assert!(text.starts_with("User: 0 ") && text.contains("User: 39 "));
        let thought = ChatEntry::Agent {
            text: "hmm".into(),
            thought: true,
        };
        assert_eq!(conversation_text(&[thought]), "");
    }

    #[test]
    fn the_user_is_told_of_an_ended_turn_a_question_and_a_stop() {
        let ended = |why: &str| ChatEvent::TurnEnded {
            stop_reason: why.into(),
        };
        let mut c = chat();
        c.entries.push(ChatEntry::Agent {
            text: "Padded it.".into(),
            thought: false,
        });
        let told = |c: &Chat, e: ChatEvent| notice(c, &e).map(|(s, b)| format!("{s} / {b}"));
        assert_eq!(
            told(&c, ended("end_turn")).unwrap(),
            "Chat 1: the agent is done / Padded it."
        );
        assert_eq!(told(&c, ended("cancelled")), None);
        assert!(told(&c, ended("refusal")).unwrap().contains("declined"));
        assert!(told(&c, ChatEvent::Exited).unwrap().contains("stopped"));
        // A queued message goes next: the turn after it tells.
        c.enqueue("taller".into());
        assert_eq!(told(&c, ended("end_turn")), None);
        c.status = ChatStatus::Ready;
        assert_eq!(told(&c, ended("end_turn")), None, "not the agent's turn");
    }
}
