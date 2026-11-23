//! A chat with an AI agent: what it holds (entries, queue, options,
//! attachments and mentions) and how what the agent streams is folded
//! into it (`apply`).

use std::path::PathBuf;

use agents::acp::{
    AgentChat, Attachment, ChatCommand, ChatEvent, PermissionOption, PlanEntry, SessionOption,
};
use serde_json::Value;

use crate::chat_store::SavedChat;
use crate::mentions::{self, Mention};

/// Where a chat stands.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatStatus {
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
pub enum Naming {
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
pub enum ChatEntry {
    /// What the user sent, the names of what went with it and the
    /// mentions its text holds.
    User {
        text: String,
        attachments: Vec<String>,
        mentions: Vec<Mention>,
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

pub struct Chat {
    pub id: String,
    pub title: String,
    pub naming: Naming,
    /// A title is being asked for.
    pub titling: bool,
    /// The user asked for it: it replaces even their own.
    pub titling_asked: bool,
    /// The title after the first answer has been asked for, whatever came
    /// of it.
    pub titled_once: bool,
    pub agent: String,
    pub status: ChatStatus,
    /// A change the agent asks for waits for the user's OK.
    pub ask: bool,
    pub entries: Vec<ChatEntry>,
    /// The session options the agent offers, as they stand.
    pub options: Vec<SessionOption>,
    /// What goes with the next prompt.
    pub attachments: Vec<Attachment>,
    /// The mentions picked into the message being written; those whose
    /// tokens its text still holds go with it.
    pub mentions: Vec<Mention>,
    /// Messages written while the agent was on a turn, in the order they
    /// go: each goes when the turn before it ends.
    pub queued: Vec<Queued>,
    /// Stop holds what is queued until the user sends again or lets it go.
    pub held: bool,
    /// The turn on now was stopped to send a queued message at once: its
    /// end says so, and what is queued goes on.
    pub steering: bool,
    /// The agent's remembered choices have been read.
    pub chose: bool,
    /// Remembered choices not yet put to the agent, in the order it lists
    /// its options: one may only become possible once another lands (an
    /// effort level the chosen model has).
    pub choices_left: Vec<(String, Value)>,
    /// The last lines the agent wrote on stderr, to say why it failed.
    pub stderr: Vec<String>,
    /// Prompts sent: the first carries a word on where the agent is.
    pub prompts: usize,
    /// The user's rules as the agent was last told them: a change goes
    /// with the next prompt.
    pub rules_sent: String,
    /// The tab whose document the chat works on.
    pub tab: uuid::Uuid,
    /// The agent's session, kept with the document's file so the chat can
    /// continue when the file is opened again.
    pub session_id: Option<String>,
    /// The folder the agent was first started in: an agent keeps its
    /// sessions by the folder it ran in, so a resumed session must start
    /// there again, wherever the document has since been saved.
    pub folder: PathBuf,
    /// When the chat was last opened or sent a message, in milliseconds
    /// since the epoch: the history lists the newest first.
    pub used: u64,
    /// The running agent; `None` while the chat rests.
    pub session: Option<AgentChat>,
}

/// A closed chat the assistant's history offers to reopen.
#[derive(Debug, Clone, PartialEq)]
pub struct PastChat {
    /// The store's key of the file it was kept with; `None` for a chat of
    /// a document not saved, which is kept for the run only.
    pub file: Option<String>,
    /// The tab of the document not saved it was closed on.
    pub tab: Option<uuid::Uuid>,
    pub chat: SavedChat,
    /// Kept with the document on screen.
    pub here: bool,
}

/// A message waiting for the agent's turn to end.
#[derive(Debug, Clone)]
pub struct Queued {
    /// Names it for the panel's edit and remove, whatever goes before it.
    pub id: u64,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub mentions: Vec<Mention>,
    /// Open in the panel's editor: it does not go while it is, and the
    /// queue waits for it when it is next.
    pub editing: bool,
}

impl Chat {
    /// A chat kept with a document, resting on `tab` until it is shown;
    /// a chat kept without its folder resumes in `folder`.
    pub fn resting(kept: SavedChat, tab: uuid::Uuid, ask: bool, folder: PathBuf) -> Self {
        Chat {
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
            ask,
            entries: Vec::new(),
            options: Vec::new(),
            attachments: Vec::new(),
            mentions: Vec::new(),
            queued: Vec::new(),
            held: false,
            steering: false,
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
            folder: kept.folder.unwrap_or(folder),
            used: kept.used.unwrap_or(0),
            session: None,
        }
    }

    /// What the store keeps of the chat, once its agent named a session.
    pub fn saved(&self) -> Option<SavedChat> {
        Some(SavedChat {
            agent: self.agent.clone(),
            session: self.session_id.clone()?,
            title: self.title.clone(),
            named: self.naming == Naming::User,
            folder: Some(self.folder.clone()),
            used: Some(self.used),
        })
    }

    pub fn send(&self, command: ChatCommand) {
        if let Some(session) = &self.session {
            session.send(command);
        }
    }

    /// Queue `text` with what is attached; sending again lets anything
    /// Stop held go too. A stopped chat takes nothing.
    pub fn enqueue(&mut self, text: String) {
        if (text.trim().is_empty() && self.attachments.is_empty())
            || matches!(self.status, ChatStatus::Failed(_))
        {
            return;
        }
        let attachments = std::mem::take(&mut self.attachments);
        let mentions = mentions::in_text(&std::mem::take(&mut self.mentions), &text);
        let id = self.queued.iter().map(|q| q.id + 1).max().unwrap_or(0);
        self.queued.push(Queued {
            id,
            text,
            attachments,
            mentions,
            editing: false,
        });
        self.held = false;
    }

    fn queued_at(&self, id: u64) -> Option<usize> {
        self.queued.iter().position(|q| q.id == id)
    }

    /// Take queued message `id` out of the queue.
    pub fn unqueue(&mut self, id: u64) {
        let Some(at) = self.queued_at(id) else {
            return;
        };
        self.queued.remove(at);
        if self.queued.is_empty() {
            self.held = false;
        }
    }

    /// Move queued message `id` to place `to` of the queue (the last
    /// place when past it).
    pub fn move_queued(&mut self, id: u64, to: usize) {
        let Some(at) = self.queued_at(id) else {
            return;
        };
        let taken = self.queued.remove(at);
        let to = to.min(self.queued.len());
        self.queued.insert(to, taken);
    }

    /// Open queued message `id` in the editor, or close it unchanged.
    pub fn editing_queued(&mut self, id: u64, editing: bool) {
        if let Some(at) = self.queued_at(id) {
            self.queued[at].editing = editing;
        }
    }

    /// Give queued message `id` the edited `text`, without the attachments
    /// at `removed`; the mentions its text still holds stay. It keeps its
    /// place, and goes out of the queue when nothing is left of it.
    pub fn edit_queued(&mut self, id: u64, text: String, removed: &[usize]) {
        let Some(at) = self.queued_at(id) else {
            return;
        };
        let queued = &mut self.queued[at];
        let mut index = 0;
        queued.attachments.retain(|_| {
            index += 1;
            !removed.contains(&(index - 1))
        });
        queued.mentions = mentions::in_text(&queued.mentions, &text);
        queued.text = text;
        queued.editing = false;
        if queued.text.trim().is_empty() && queued.attachments.is_empty() {
            self.unqueue(id);
        }
    }

    /// Send queued message `id` next, ahead of the rest: a turn on now is
    /// stopped for it, and the rest of the queue goes on after it in its
    /// order. It goes at once when the agent waits (`send_next`).
    pub fn steer(&mut self, id: u64) {
        let Some(at) = self.queued_at(id) else {
            return;
        };
        let mut taken = self.queued.remove(at);
        taken.editing = false;
        self.queued.insert(0, taken);
        self.held = false;
        if self.status == ChatStatus::Busy {
            self.send(ChatCommand::Cancel);
            self.steering = true;
        }
    }

    /// The next queued message waits for its edit.
    pub fn waits_on_edit(&self) -> bool {
        self.queued.first().is_some_and(|q| q.editing)
    }

    /// Ask the agent for a title for the conversation so far; `asked` when
    /// the user did, so it replaces their own. Nothing is asked while one
    /// is on its way, or of a conversation with no words in it yet.
    pub fn ask_title(&mut self, asked: bool) {
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
    pub fn take_title(&mut self, text: String, asked: bool) {
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
    pub fn stop(&mut self) {
        self.send(ChatCommand::Cancel);
        self.held = !self.queued.is_empty();
        self.steering = false;
    }

    /// Send the first queued message when the agent waits for one and it
    /// is not being edited, with the word on where it is on the first and
    /// `rules` when they changed.
    pub fn send_next(&mut self, rules: String) {
        if self.status != ChatStatus::Ready
            || self.held
            || self.queued.is_empty()
            || self.waits_on_edit()
        {
            return;
        }
        let Queued {
            text,
            mut attachments,
            mentions,
            ..
        } = self.queued.remove(0);
        self.choices_left.clear();
        self.entries.push(ChatEntry::User {
            text: text.clone(),
            attachments: attachments.iter().map(Attachment::name).collect(),
            mentions: mentions.clone(),
        });
        attachments.extend(mentions.into_iter().map(|m| Attachment::Text {
            name: m.token,
            text: m.block,
        }));
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
pub fn conversation_text(entries: &[ChatEntry]) -> String {
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
pub fn working_folder(file: Option<&std::path::Path>) -> PathBuf {
    file.and_then(|f| f.parent().map(std::path::Path::to_path_buf))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// What the first prompt of a chat starts with.
pub const PREAMBLE: &str = "You are working inside printCAD, a parametric CAD application for \
3D printing, running on the user's desktop while they watch, on the document of the tab \
this chat belongs to. Its MCP server `printcad` has the tools to read and change it: \
`context` says what is open, `commands` what can be done. Never close printCAD or its \
tabs: that would end this session.";

#[cfg(any(test, feature = "testing"))]
impl Chat {
    /// A chat with these entries whose agent never answers.
    pub fn for_test(id: &str, status: ChatStatus, entries: Vec<ChatEntry>) -> Self {
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
            mentions: Vec::new(),
            queued: Vec::new(),
            held: false,
            steering: false,
            chose: true,
            choices_left: Vec::new(),
            stderr: Vec::new(),
            prompts: 1,
            rules_sent: String::new(),
            tab: uuid::Uuid::nil(),
            session_id: None,
            folder: PathBuf::new(),
            used: 0,
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
pub fn put_choices(chat: &mut Chat) {
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
/// stopped it or steered past it), a question from the agent, an agent that stopped.
pub fn notice(chat: &Chat, event: &ChatEvent) -> Option<(String, String)> {
    let title = &chat.title;
    let stopped = |why: &str| Some((format!("{title}: the agent stopped"), why.to_string()));
    match event {
        ChatEvent::TurnEnded { stop_reason } if chat.status == ChatStatus::Busy => {
            match stop_reason.as_str() {
                "end_turn" if chat.queued.is_empty() || chat.held || chat.waits_on_edit() => {
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
pub fn apply(chat: &mut Chat, event: ChatEvent) {
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
                    mentions: Vec::new(),
                }),
            }
            // The mentions' blocks come back with it: chips again.
            if let Some(ChatEntry::User { text, mentions, .. }) = chat.entries.last_mut() {
                mentions.extend(mentions::take_blocks(text));
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
            let steered = std::mem::take(&mut chat.steering);
            match stop_reason.as_str() {
                "end_turn" => {}
                "cancelled" if steered => chat
                    .entries
                    .push(ChatEntry::Note("Stopped to steer".into())),
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
    use crate::chat_store;

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

    fn order(c: &Chat) -> Vec<&str> {
        c.queued.iter().map(|q| q.text.as_str()).collect()
    }

    /// A queued message moves to any place of the queue, is edited where
    /// it stands, or is dropped with what was attached to it.
    #[test]
    fn a_queued_message_moves_is_edited_in_place_or_is_dropped() {
        let mut c = chat();
        c.attachments = vec![
            Attachment::File("/tmp/a.png".into()),
            Attachment::File("/tmp/b.png".into()),
        ];
        c.enqueue("first".into());
        c.enqueue("second".into());
        c.enqueue("third".into());
        let (first, second, third) = (c.queued[0].id, c.queued[1].id, c.queued[2].id);

        c.move_queued(third, 0);
        assert_eq!(order(&c), ["third", "first", "second"]);
        c.move_queued(third, 9);
        assert_eq!(order(&c), ["first", "second", "third"], "past the end");

        c.edit_queued(first, "first, edited".into(), &[0]);
        assert_eq!(order(&c), ["first, edited", "second", "third"]);
        assert_eq!(
            c.queued[0].attachments,
            [Attachment::File("/tmp/b.png".into())],
            "only the one taken off goes"
        );
        c.edit_queued(second, " ".into(), &[]);
        assert_eq!(order(&c), ["first, edited", "third"], "nothing left of it");
        c.unqueue(first);
        assert_eq!(order(&c), ["third"]);
        assert!(c.attachments.is_empty(), "a dropped one's files go with it");
    }

    /// A message being edited does not go: when it is next, the queue
    /// waits for it, and the user is told the turn ended.
    #[test]
    fn the_queue_waits_for_the_message_being_edited() {
        let mut c = chat();
        c.enqueue("taller".into());
        c.enqueue("and red".into());
        let first = c.queued[0].id;
        c.editing_queued(first, true);
        let ended = ChatEvent::TurnEnded {
            stop_reason: "end_turn".into(),
        };
        assert!(notice(&c, &ended).is_some(), "it waits on the user");
        end_turn(&mut c, "end_turn");
        c.send_next(String::new());
        assert!(sent(&c).is_empty(), "waits on the edit");

        c.edit_queued(first, "much taller".into(), &[]);
        c.send_next(String::new());
        assert_eq!(sent(&c), ["much taller"]);
    }

    /// Steering sends a queued message next: a turn on now stops for it,
    /// unheld and untold, the stop shown as a steer, and the rest goes on
    /// after it in its order.
    #[test]
    fn a_steered_message_stops_the_turn_and_goes_ahead_of_the_rest() {
        let mut c = chat();
        for text in ["first", "second", "third"] {
            c.enqueue(text.into());
        }
        c.stop();
        let third = c.queued[2].id;
        c.editing_queued(third, true);
        c.steer(third);
        assert!(!c.held, "a steer lets a held queue go");
        assert!(c.steering);
        assert!(!c.queued[0].editing, "its edit is closed");
        assert_eq!(order(&c), ["third", "first", "second"]);

        let cancelled = ChatEvent::TurnEnded {
            stop_reason: "cancelled".into(),
        };
        assert_eq!(notice(&c, &cancelled), None);
        end_turn(&mut c, "cancelled");
        assert_eq!(
            c.entries.last(),
            Some(&ChatEntry::Note("Stopped to steer".into()))
        );
        assert!(!c.steering);
        c.send_next(String::new());
        assert_eq!(sent(&c), ["third"]);
        end_turn(&mut c, "end_turn");
        c.send_next(String::new());
        assert_eq!(sent(&c), ["third", "first"], "the rest goes on");

        // With the agent waiting, it just goes next; nothing is stopped.
        end_turn(&mut c, "end_turn");
        c.enqueue("fourth".into());
        let fourth = c.queued[1].id;
        c.steer(fourth);
        assert!(!c.steering);
        c.send_next(String::new());
        assert_eq!(sent(&c), ["third", "first", "fourth"]);
        assert_eq!(order(&c), ["second"]);
    }

    /// A mention goes with the message whose text holds its token, stays
    /// with a queued message whose edit keeps its token, and comes back
    /// out of a replayed conversation.
    #[test]
    fn mentions_go_with_their_message_and_come_back() {
        let block = |token: &str| Mention {
            token: token.into(),
            kind: "selection".into(),
            block: format!(
                "<printcad:mention>{{\"token\":\"{token}\",\"kind\":\"selection\"}}</printcad:mention>"
            ),
        };
        let mut c = chat();
        c.mentions = vec![block("@selection"), block("@selection-2")];
        c.enqueue("thicken @selection and @selection-2".into());
        assert!(c.mentions.is_empty());
        let id = c.queued[0].id;
        c.edit_queued(id, "thicken @selection by 2".into(), &[]);
        assert_eq!(
            c.queued[0].mentions,
            [block("@selection")],
            "the one taken out of the text goes"
        );
        end_turn(&mut c, "end_turn");
        c.send_next(String::new());
        let Some(ChatEntry::User { mentions, .. }) = c.entries.last() else {
            panic!("sent")
        };
        assert_eq!(mentions, &[block("@selection")]);

        c.entries.clear();
        let whole = block("@selection").block;
        for piece in ["thicken @selection", &whole[..9], &whole[9..]] {
            apply(&mut c, ChatEvent::UserText(piece.to_string()));
        }
        assert_eq!(sent(&c), ["thicken @selection"]);
        let Some(ChatEntry::User { mentions, .. }) = c.entries.last() else {
            panic!("replayed")
        };
        assert_eq!(mentions, &[block("@selection")]);
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

    /// A closed chat goes to the history and, reopened from it, rests on
    /// the tab on screen in the same session, which waking resumes.
    #[test]
    fn a_closed_chat_reopens_in_its_session() {
        let dir = std::env::temp_dir().join(format!("printcad-reopen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("chats.json");
        let mut c = chat();
        c.session_id = Some("s1".into());
        c.title = "Bracket".into();
        c.naming = Naming::User;
        c.folder = dir.clone();
        c.used = 7;
        chat_store::close_in(&store, "/x/a.prtcad", c.saved().unwrap());
        let past = chat_store::history_in(&store).remove(0);
        let tab = uuid::Uuid::new_v4();
        let again = Chat::resting(past.chat, tab, true, PathBuf::from("/elsewhere"));
        assert_eq!(again.session_id.as_deref(), Some("s1"));
        assert_eq!(
            (again.title.as_str(), again.naming),
            ("Bracket", Naming::User)
        );
        assert_eq!(
            (again.status, again.tab, again.folder),
            (ChatStatus::Resting, tab, dir.clone())
        );
        std::fs::remove_dir_all(&dir).unwrap();
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
                mentions: Vec::new(),
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
            mentions: Vec::new(),
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
