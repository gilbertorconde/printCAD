//! Chats with AI agents in the running app: each one an agent's program,
//! started over the Agent Client Protocol, with this application's MCP
//! server on its list so it can reach the document.
//!
//! The app keeps every chat's state (`assistant::Chat`); what the agent
//! streams arrives on the chat's thread and is folded into its entries
//! once a frame. The assistant panel draws them and answers with
//! commands.

use std::path::PathBuf;

use agents::acp::{AgentChat, Attachment, ChatCommand, ChatEvent, McpServer, Program};
use assistant::chat_store::{self, SavedChat};
use assistant::mentions::{self, MentionOption};
use assistant::{
    Chat, ChatEntry, ChatStatus, Naming, PastChat, apply, notice, put_choices, working_folder,
};
use serde_json::Value;

use crate::PrintCadApp;

impl PrintCadApp {
    /// The `PRINTCAD_BENCH_CHAT` hook's chat: no agent behind it, waiting,
    /// `message` sent with the mentions it names, the assistant open.
    pub(crate) fn bench_chat(&mut self, message: String) {
        let tab = self.session.tab;
        let options = self.mention_options(tab);
        let mut mentions = Vec::new();
        for option in &options {
            if mentions::find_tokens(&message, &[option.token.as_str()]).is_empty() {
                continue;
            }
            mentions.extend(self.mention_block(option.source, &option.token));
        }
        self.agents.chats.push(Chat {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Chat 1".into(),
            naming: Naming::User,
            titling: false,
            titling_asked: false,
            titled_once: true,
            agent: "Bench".into(),
            status: ChatStatus::Ready,
            ask: true,
            entries: Vec::new(),
            options: Vec::new(),
            attachments: Vec::new(),
            mentions,
            queued: Vec::new(),
            held: false,
            chose: true,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 1,
            rules_sent: String::new(),
            tab,
            session_id: None,
            folder: PathBuf::new(),
            used: chat_store::now_ms(),
            session: None,
        });
        if let Some(chat) = self.agents.chats.last_mut() {
            chat.enqueue(message);
            chat.send_next(String::new());
            chat.status = ChatStatus::Ready;
        }
        self.agents.assistant_attention = true;
    }

    /// Start a chat with agent number `agent` of the Preferences, on the
    /// document on screen.
    pub(crate) fn new_chat(&mut self, agent: usize) {
        let Some(config) = self.user_settings.ai.agents.get(agent).cloned() else {
            return;
        };
        let id = uuid::Uuid::new_v4().to_string();
        let folder = working_folder(self.session.file.path.as_deref());
        let session = self.start_agent(&config, &id, None, &folder);
        let number = self.agents.chats_made + 1;
        self.agents.chats_made = number;
        self.agents.chats.push(Chat {
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
            mentions: Vec::new(),
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
            used: chat_store::now_ms(),
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
        if let (Some(server), Ok(exe)) = (&self.agents.mcp, std::env::current_exe()) {
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
            self.agents.waker.clone(),
        )
    }

    /// Bring back the chats kept with the file of the document on screen,
    /// resting until each is shown.
    pub(crate) fn restore_chats(&mut self) {
        let Some(file) = self.session.file.path.clone() else {
            return;
        };
        let tab = self.session.tab;
        let saved = chat_store::chats_of(&file);
        let mut restored = false;
        for kept in saved {
            if self
                .agents
                .chats
                .iter()
                .any(|c| c.session_id.as_deref() == Some(kept.session.as_str()))
            {
                continue;
            }
            let folder = working_folder(Some(file.as_path()));
            let ask = self.user_settings.ai.ask_before_changes;
            self.agents
                .chats
                .push(Chat::resting(kept, tab, ask, folder));
            restored = true;
        }
        if restored {
            self.agents.assistant_attention = true;
        }
    }

    /// Start a resting chat's agent in its earlier session; its
    /// conversation comes back as the agent replays it.
    pub(crate) fn wake_chat(&mut self, id: &str) {
        let Some(chat) = self.agents.chats.iter().find(|c| c.id == id) else {
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
    /// opened: the chats of every tab that has a file and a session. A
    /// document saved for the first time takes the chats closed on it
    /// before into its history.
    pub(crate) fn persist_chats(&mut self) {
        let files: Vec<(uuid::Uuid, PathBuf)> = self
            .sessions()
            .filter_map(|s| Some((s.tab, s.file.path.clone()?)))
            .collect();
        for (tab, file) in files {
            let (closed, rest) = std::mem::take(&mut self.agents.closed_unsaved)
                .into_iter()
                .partition(|(on, _)| *on == tab);
            self.agents.closed_unsaved = rest;
            for (_, chat) in closed {
                chat_store::close(&file, chat);
            }
            let kept: Vec<SavedChat> = self
                .agents
                .chats
                .iter()
                .filter(|c| c.tab == tab)
                .filter_map(Chat::saved)
                .collect();
            chat_store::keep(&file, kept);
        }
    }

    /// The tab `tab` closes: the chats of a document not saved stay in
    /// the history for the rest of the run; a file's stay kept with it.
    pub(crate) fn let_go_of_tab_chats(&mut self, tab: uuid::Uuid, saved: bool) {
        let chats = std::mem::take(&mut self.agents.chats);
        let (going, staying): (Vec<Chat>, Vec<Chat>) =
            chats.into_iter().partition(|c| c.tab == tab);
        self.agents.chats = staying;
        if !saved {
            self.agents
                .closed_unsaved
                .extend(going.iter().filter_map(|c| Some((tab, c.saved()?))));
        }
        self.refresh_chat_history();
    }

    /// Read the closed chats again for the assistant's history: every
    /// file's from the store and those of documents not saved, newest
    /// first.
    pub(crate) fn refresh_chat_history(&mut self) {
        let mut past: Vec<PastChat> = chat_store::history()
            .into_iter()
            .map(|c| PastChat {
                file: Some(c.file),
                tab: None,
                chat: c.chat,
                here: false,
            })
            .chain(
                self.agents
                    .closed_unsaved
                    .iter()
                    .map(|(tab, chat)| PastChat {
                        file: None,
                        tab: Some(*tab),
                        chat: chat.clone(),
                        here: false,
                    }),
            )
            .collect();
        past.sort_by_key(|a| std::cmp::Reverse(a.chat.used));
        self.agents.history = past;
    }

    /// The history with the chats kept with the document on screen marked.
    pub(crate) fn chat_history(&self) -> Vec<PastChat> {
        if self.agents.history.is_empty() {
            return Vec::new();
        }
        let here = self.session.file.path.as_deref().map(chat_store::key);
        let tab = self.session.tab;
        self.agents
            .history
            .iter()
            .map(|p| PastChat {
                here: match &p.file {
                    Some(file) => here.as_deref() == Some(file.as_str()),
                    None => p.tab == Some(tab),
                },
                ..p.clone()
            })
            .collect()
    }

    /// Reopen a closed chat as a tab of the assistant, on the document on
    /// screen whatever file it was kept with, and kept with this one from
    /// now on. It rests until shown, then resumes its session.
    pub(crate) fn reopen_chat(&mut self, file: Option<String>, session: &str) {
        let found = match &file {
            Some(file) => self
                .agents
                .history
                .iter()
                .find(|p| p.file.as_ref() == Some(file) && p.chat.session == session)
                .map(|p| p.chat.clone()),
            None => self
                .agents
                .closed_unsaved
                .iter()
                .find(|(_, c)| c.session == session)
                .map(|(_, c)| c.clone()),
        };
        let Some(mut kept) = found else {
            return;
        };
        self.forget_chat(file, session);
        if self
            .agents
            .chats
            .iter()
            .any(|c| c.session_id.as_deref() == Some(session))
        {
            return;
        }
        kept.used = Some(chat_store::now_ms());
        let folder = working_folder(self.session.file.path.as_deref());
        let ask = self.user_settings.ai.ask_before_changes;
        let tab = self.session.tab;
        self.agents
            .chats
            .push(Chat::resting(kept, tab, ask, folder));
        self.persist_chats();
    }

    /// Take a closed chat out of the history and the store for good.
    pub(crate) fn forget_chat(&mut self, file: Option<String>, session: &str) {
        match &file {
            Some(file) => chat_store::forget(file, session),
            None => self
                .agents
                .closed_unsaved
                .retain(|(_, c)| c.session != session),
        }
        self.refresh_chat_history();
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
        self.agents.chats.iter().find(|c| c.id == id).map(|c| c.ask)
    }

    fn chat_mut(&mut self, id: &str) -> Option<&mut Chat> {
        self.agents.chats.iter_mut().find(|c| c.id == id)
    }

    /// Queue a message, with what is attached, and send it at once when
    /// the agent is waiting for one.
    pub(crate) fn send_to_chat(&mut self, id: &str, text: String) {
        self.wake_chat(id);
        if let Some(chat) = self.chat_mut(id) {
            chat.enqueue(text);
            chat.used = chat_store::now_ms();
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
            app_log::warn("Nothing is visible to attach a picture of");
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
            Err(err) => app_log::warn(format!("Could not read the clipboard's picture: {err}")),
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

    /// Take a mention picked in the `@` list into what goes with the
    /// chat's next prompt, as what it names stands now.
    pub(crate) fn add_mention(&mut self, id: &str, option: &MentionOption) {
        let Some(mention) = self.mention_block(option.source, &option.token) else {
            return;
        };
        if let Some(chat) = self.chat_mut(id) {
            chat.mentions.retain(|m| m.token != mention.token);
            chat.mentions.push(mention);
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
        let held: Vec<usize> = (0..self.agents.approvals.len())
            .rev()
            .filter(|i| self.agents.approvals[*i].chat.as_deref() == Some(id))
            .collect();
        for index in held {
            self.settle_approval(index, false);
        }
    }

    /// Close a chat: its agent's program ends with it, and the chat goes
    /// to the history, from which it can be reopened.
    /// Move a chat to index `to` of the strip, kept in that order with
    /// its document.
    pub(crate) fn move_chat(&mut self, id: &str, to: usize) {
        let Some(from) = self.agents.chats.iter().position(|c| c.id == id) else {
            return;
        };
        let chat = self.agents.chats.remove(from);
        let to = to.min(self.agents.chats.len());
        self.agents.chats.insert(to, chat);
        self.persist_chats();
    }

    pub(crate) fn close_chat(&mut self, id: &str) {
        self.cancel_chat(id);
        let Some(index) = self.agents.chats.iter().position(|c| c.id == id) else {
            return;
        };
        let chat = self.agents.chats.remove(index);
        if let Some(saved) = chat.saved() {
            let file = self
                .sessions()
                .find(|s| s.tab == chat.tab)
                .and_then(|s| s.file.path.clone());
            match file {
                Some(file) => chat_store::close(&file, saved),
                None => self.agents.closed_unsaved.push((chat.tab, saved)),
            }
        }
        self.persist_chats();
        self.refresh_chat_history();
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
            let held: Vec<usize> = (0..self.agents.approvals.len())
                .rev()
                .filter(|i| self.agents.approvals[*i].chat.as_deref() == Some(id))
                .collect();
            for index in held.into_iter().rev() {
                // Run oldest first; indices shift as each one goes.
                let at = index.min(self.agents.approvals.len().saturating_sub(1));
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
                app_log::warn(format!("Could not save the agent's choice: {err}"));
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
        for chat in &mut self.agents.chats {
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
            .agents
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
            self.agents.assistant_attention = true;
        }
    }
}
