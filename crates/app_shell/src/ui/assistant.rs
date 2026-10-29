//! The assistant panel on the right: chats with AI agents, one tab each.
//!
//! The app keeps the chats; this draws them and answers with commands.
//! Above the chat, the changes agents asked for that wait for the user's
//! OK. In the chat: the conversation (the agent's message, its thinking
//! folded away, the tools it called, its plan, its requests for
//! permission), and a box to write in: Enter sends, Shift+Enter starts a
//! new line, Up in an empty box brings back the last message sent. The
//! agent's messages are markdown; each message copies from a button shown
//! on hover, and the header (or Alt+Up and Alt+Down) jumps between the
//! user's own messages.

use egui::RichText;
use ui_kit::markdown;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Card, Tab, icon_button, primary_button, secondary_button, small_secondary_button, tab_plus,
    toggle,
};
use ui_kit::{mono, sans, sans_medium};

use super::UiCommand;
use crate::app::chats::{Chat, ChatEntry, ChatStatus};
use crate::app::mcp::Approval;
use agents::acp::{OptionValue, SessionOption};

/// The panel's own state: whether it shows, the chat on screen and what
/// is being written in each chat.
#[derive(Debug, Default)]
pub struct AssistantState {
    pub open: bool,
    /// The chat on screen, by id.
    active: Option<String>,
    /// Chats known last frame: a new one comes to the front.
    known: usize,
    drafts: std::collections::HashMap<String, String>,
    /// The rules of the document on screen, while they are being edited.
    rules_draft: Option<String>,
    /// The chat being renamed, by id, the name typed so far, and whether
    /// the field has had the keyboard yet.
    renaming: Option<(String, String, bool)>,
    /// Where each chat's conversation was scrolled to, last frame.
    scroll: std::collections::HashMap<String, ChatScroll>,
}

/// A conversation's scroll as last drawn: where the user's messages sit in
/// it, how far down it is, and a jump asked for.
#[derive(Debug, Default)]
struct ChatScroll {
    /// Each user message's entry and its top, from the top of the
    /// conversation.
    user_tops: Vec<(usize, f32)>,
    offset: f32,
    at_bottom: bool,
    jump: Option<Jump>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Jump {
    Previous,
    Next,
    Latest,
}

impl ChatScroll {
    /// The entry a jump lands on: the user's message above (or below) the
    /// top of the view, or the conversation's end.
    fn target(&self, jump: Jump, last: usize) -> Option<usize> {
        // A message whose top is at the view's top already counts as seen.
        const SLACK: f32 = 4.0;
        match jump {
            Jump::Previous => self
                .user_tops
                .iter()
                .rev()
                .find(|(_, top)| *top < self.offset - SLACK)
                .map(|(i, _)| *i),
            Jump::Next => self
                .user_tops
                .iter()
                .find(|(_, top)| *top > self.offset + SLACK)
                .map(|(i, _)| *i),
            Jump::Latest => Some(last),
        }
    }
}

impl AssistantState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

/// What the panel reads each frame.
pub struct AssistantInputs<'a> {
    pub chats: &'a [Chat],
    pub approvals: &'a [Approval],
    /// The agents of the Preferences, by name.
    pub agents: Vec<String>,
    /// The name of the document on screen, and the rules its agents keep
    /// to.
    pub document_name: &'a str,
    pub document_rules: &'a str,
    /// The paste key went down this frame.
    pub paste_pressed: bool,
}

/// What the panel asked for beyond commands.
#[derive(Debug, Default)]
pub struct AssistantResult {
    /// Open Preferences on the AI agents page.
    pub open_agent_settings: bool,
}

pub fn draw_assistant(
    ui: &mut egui::Ui,
    state: &mut AssistantState,
    inputs: AssistantInputs<'_>,
    commands: &mut Vec<UiCommand>,
) -> AssistantResult {
    let mut result = AssistantResult::default();
    if !state.open {
        return result;
    }
    let AssistantInputs {
        chats,
        approvals,
        agents,
        document_name,
        document_rules,
        paste_pressed,
    } = inputs;
    // A chat opened since last frame takes the screen; one closed gives it
    // back to the last.
    if chats.len() > state.known {
        state.active = chats.last().map(|c| c.id.clone());
    }
    state.known = chats.len();
    if !state
        .active
        .as_ref()
        .is_some_and(|id| chats.iter().any(|c| &c.id == id))
    {
        state.active = chats.last().map(|c| c.id.clone());
    }

    // The viewport keeps its room whatever the window's size: the panel
    // never takes more than this, and its content never widens it.
    let room = (ui.available_width() - MIN_VIEWPORT_WIDTH).max(MIN_PANEL_WIDTH);
    egui::Panel::right("assistant_panel")
        .resizable(true)
        .default_size(380.0f32.min(room))
        .size_range(MIN_PANEL_WIDTH..=MAX_PANEL_WIDTH.min(room))
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ui, |ui| {
            // Nothing paints past the panel, whatever its content asks.
            ui.set_clip_rect(ui.max_rect());
            let panel = ui.max_rect();
            if let Some(chat) = state.active.clone() {
                take_dropped_files(ui, panel, chat, commands);
            }
            tab_strip(ui, state, chats, &agents, commands, &mut result);
            rename_row(ui, state, chats, commands);
            rules_editor(ui, state, document_name, document_rules, commands);
            ui.separator();
            for (index, approval) in approvals.iter().enumerate() {
                approval_card(ui, index, approval, chats, commands);
            }
            let Some(chat) = state
                .active
                .as_ref()
                .and_then(|id| chats.iter().find(|c| &c.id == id))
            else {
                empty(ui, &agents, commands, &mut result);
                return;
            };
            // A chat from an earlier visit wakes once it is on screen.
            if chat.status == ChatStatus::Resting {
                commands.push(UiCommand::WakeChat(chat.id.clone()));
            }
            let scroll = state.scroll.entry(chat.id.clone()).or_default();
            chat_header(ui, chat, scroll, commands);
            // Alt+Up and Alt+Down jump while the panel is in use.
            if ui.rect_contains_pointer(panel)
                || ui.memory(|m| m.focused().is_some_and(|f| f == input_id(chat)))
            {
                ui.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp) {
                        scroll.jump = Some(Jump::Previous);
                    }
                    if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown) {
                        scroll.jump = Some(Jump::Next);
                    }
                });
            }
            ui.add_space(SPACE_1);
            let draft = state.drafts.entry(chat.id.clone()).or_default();
            // The box to write in keeps the bottom; the conversation
            // scrolls in what is left above it.
            egui::Panel::bottom(egui::Id::new(("assistant_composer", &chat.id)))
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    top: 6,
                    ..Default::default()
                }))
                .show(ui, |ui| {
                    queued_list(ui, chat, draft, commands);
                    input(ui, chat, draft, paste_pressed, commands);
                });
            let scroll = state.scroll.entry(chat.id.clone()).or_default();
            let land_on = scroll
                .jump
                .take()
                .and_then(|jump| scroll.target(jump, chat.entries.len().saturating_sub(1)));
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let out = egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = SPACE_2;
                            let origin = ui.min_rect().top();
                            let mut user_tops = Vec::new();
                            for (index, entry) in chat.entries.iter().enumerate() {
                                let rect = ui
                                    .scope(|ui| draw_entry(ui, chat, index, entry, commands))
                                    .response
                                    .rect;
                                if matches!(entry, ChatEntry::User { .. }) {
                                    user_tops.push((index, rect.top() - origin));
                                }
                                if land_on == Some(index) {
                                    ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                                }
                            }
                            user_tops
                        });
                    scroll.user_tops = out.inner;
                    scroll.offset = out.state.offset.y;
                    scroll.at_bottom =
                        out.state.offset.y + out.inner_rect.height() >= out.content_size.y - 4.0;
                });
        });
    result
}

const CHAT_TAB_MIN: f32 = 80.0;
const CHAT_TAB_MAX: f32 = 160.0;
const MIN_PANEL_WIDTH: f32 = 320.0;
const MAX_PANEL_WIDTH: f32 = 720.0;
/// What the panel always leaves the viewport.
const MIN_VIEWPORT_WIDTH: f32 = 360.0;
/// The most of a tool's output a chat shows.
const OUTPUT_SHOWN: usize = 4000;

/// Files dropped on the panel go with the chat's next prompt; while
/// files hover over it, it says so.
fn take_dropped_files(
    ui: &mut egui::Ui,
    panel: egui::Rect,
    chat: String,
    commands: &mut Vec<UiCommand>,
) {
    let (hovering, dropped, pointer) = ui.ctx().input(|i| {
        (
            !i.raw.hovered_files.is_empty(),
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>(),
            i.pointer.latest_pos(),
        )
    });
    // Some systems give no pointer position during a drag from outside.
    let here = pointer.is_none_or(|p| panel.contains(p));
    if hovering && here {
        ui.painter().rect_stroke(
            panel.shrink(2.0),
            RADIUS_MD,
            egui::Stroke::new(2.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    if !dropped.is_empty() && here {
        // Taken here, they are not the window's to open.
        ui.ctx().input_mut(|i| i.raw.dropped_files.clear());
        commands.push(UiCommand::AttachPaths {
            chat,
            paths: dropped,
        });
    }
}

fn tab_strip(
    ui: &mut egui::Ui,
    state: &mut AssistantState,
    chats: &[Chat],
    agents: &[String],
    commands: &mut Vec<UiCommand>,
    result: &mut AssistantResult,
) {
    // The same tabs as the open documents'.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let available = ui.available_width() - 32.0;
        let width = (available / chats.len().max(1) as f32).clamp(CHAT_TAB_MIN, CHAT_TAB_MAX);
        for chat in chats {
            let on = state.active.as_deref() == Some(chat.id.as_str());
            let tab = Tab::new(&chat.title, on)
                .width(width)
                .closable("Close the chat and its agent")
                .show(ui);
            if tab.closed {
                commands.push(UiCommand::CloseChat(chat.id.clone()));
            } else if tab.selected {
                state.active = Some(chat.id.clone());
            }
            if tab.response.double_clicked() {
                state.renaming = Some((chat.id.clone(), chat.title.clone(), false));
            }
            tab.response.context_menu(|ui| {
                ui.set_min_width(180.0);
                if ui
                    .button(RichText::new("Rename…").font(sans(FONT_SM)))
                    .clicked()
                {
                    state.renaming = Some((chat.id.clone(), chat.title.clone(), false));
                    ui.close();
                }
                let can_ask = !chat.titling
                    && !matches!(chat.status, ChatStatus::Resting | ChatStatus::Failed(_))
                    && chat
                        .entries
                        .iter()
                        .any(|e| matches!(e, ChatEntry::User { .. } | ChatEntry::Agent { .. }));
                if ui
                    .add_enabled(
                        can_ask,
                        egui::Button::new(
                            RichText::new("New title from the agent").font(sans(FONT_SM)),
                        ),
                    )
                    .on_hover_text("The agent names the conversation so far, apart from it")
                    .clicked()
                {
                    commands.push(UiCommand::RetitleChat(chat.id.clone()));
                    ui.close();
                }
                if ui
                    .button(RichText::new("Close chat").font(sans(FONT_SM)))
                    .clicked()
                {
                    commands.push(UiCommand::CloseChat(chat.id.clone()));
                    ui.close();
                }
            });
            let hover = match chat.titling {
                true => format!("{} · naming the chat…", chat.agent),
                false => format!("{} · double click to rename", chat.agent),
            };
            tab.response.on_hover_text(hover);
        }
        let plus = tab_plus(ui, TAB_BAR - 8.0).on_hover_text("New chat");
        let rules = ui
            .add(egui::Button::new(RichText::new("Rules").font(sans(FONT_XS))).small())
            .on_hover_text("What agents keep to in this document");
        if rules.clicked() {
            state.rules_draft = match state.rules_draft {
                Some(_) => None,
                None => Some(String::new()),
            };
        }
        egui::Popup::menu(&plus).show(|ui| {
            ui.set_min_width(180.0);
            new_chat_items(ui, agents, commands, result);
        });
    });
}

/// The name of the chat being renamed, typed under the tabs: Enter keeps
/// it, Escape leaves the name as it was.
fn rename_row(
    ui: &mut egui::Ui,
    state: &mut AssistantState,
    chats: &[Chat],
    commands: &mut Vec<UiCommand>,
) {
    let Some((chat, draft, focused)) = state.renaming.as_mut() else {
        return;
    };
    if !chats.iter().any(|c| &c.id == chat) {
        state.renaming = None;
        return;
    }
    let id = egui::Id::new(("rename_chat", chat.as_str()));
    let mut done = None;
    let line = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        line,
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if ui
                .button(RichText::new("Cancel").font(sans(FONT_XS)))
                .clicked()
            {
                done = Some(false);
            }
            if ui
                .button(RichText::new("Rename").font(sans(FONT_XS)))
                .clicked()
            {
                done = Some(true);
            }
            let edit = ui.add(
                egui::TextEdit::singleline(draft)
                    .id(id)
                    .font(sans(FONT_SM))
                    .hint_text("The chat's name")
                    .desired_width(ui.available_width()),
            );
            if !*focused {
                edit.request_focus();
                *focused = true;
            }
            // The field lets go of Enter and Escape as it takes them: they
            // are spent here, not on the task panel's OK and Cancel.
            if edit.lost_focus() {
                ui.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                        done = Some(true);
                    } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                        done = Some(false);
                    }
                });
            }
        },
    );
    match done {
        Some(true) => {
            commands.push(UiCommand::RenameChat {
                chat: chat.clone(),
                title: draft.clone(),
            });
            state.renaming = None;
        }
        Some(false) => state.renaming = None,
        None => {}
    }
}

/// The rules of the document on screen, edited in place: they go with it
/// when it is saved, beside the ones in Preferences for every document.
fn rules_editor(
    ui: &mut egui::Ui,
    state: &mut AssistantState,
    document_name: &str,
    document_rules: &str,
    commands: &mut Vec<UiCommand>,
) {
    let Some(draft) = state.rules_draft.as_mut() else {
        return;
    };
    // Opened: start from what the document holds.
    if draft.is_empty() && !document_rules.is_empty() {
        draft.push_str(document_rules);
    }
    let mut close = false;
    Card::new().padding(8.0).show(ui, |ui| {
        ui.label(
            RichText::new(format!("Rules for {document_name}"))
                .font(sans_medium(FONT_SM))
                .color(TEXT1),
        );
        ui.label(
            RichText::new(
                "Agents working on this document keep to these, after the rules in \
                 Preferences › AI agents. They are saved with the document.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        ui.add(
            egui::TextEdit::multiline(draft)
                .hint_text("The lid stays 2 mm thick.\nDo not change the mounting holes.")
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .font(mono(FONT_SM)),
        );
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("Save").font(sans(FONT_SM)))
                .clicked()
            {
                commands.push(UiCommand::SetDocumentAgentRules(draft.trim().to_string()));
                close = true;
            }
            if ui
                .button(RichText::new("Cancel").font(sans(FONT_SM)))
                .clicked()
            {
                close = true;
            }
        });
    });
    if close {
        state.rules_draft = None;
    }
}

fn new_chat_items(
    ui: &mut egui::Ui,
    agents: &[String],
    commands: &mut Vec<UiCommand>,
    result: &mut AssistantResult,
) {
    for (index, name) in agents.iter().enumerate() {
        if ui.button(RichText::new(name).font(sans(FONT_SM))).clicked() {
            commands.push(UiCommand::NewChat(index));
            ui.close();
        }
    }
    if !agents.is_empty() {
        ui.separator();
    }
    if ui
        .button(RichText::new("Set up agents…").font(sans(FONT_SM)))
        .clicked()
    {
        result.open_agent_settings = true;
        ui.close();
    }
}

fn empty(
    ui: &mut egui::Ui,
    agents: &[String],
    commands: &mut Vec<UiCommand>,
    result: &mut AssistantResult,
) {
    ui.add_space(SPACE_3);
    ui.label(
        RichText::new(
            "Chat with an AI agent that can read and change the document: it uses the \
             same commands scripts do, and every change it makes can be undone.",
        )
        .font(sans(FONT_SM))
        .color(TEXT2),
    );
    ui.add_space(SPACE_2);
    if agents.is_empty() {
        ui.label(
            RichText::new("No agent is set up yet.")
                .font(sans(FONT_SM))
                .color(TEXT3),
        );
        if primary_button(ui, "Set up an agent").clicked() {
            result.open_agent_settings = true;
        }
        return;
    }
    for (index, name) in agents.iter().enumerate() {
        if secondary_button(ui, &format!("Chat with {name}")).clicked() {
            commands.push(UiCommand::NewChat(index));
        }
    }
}

fn approval_card(
    ui: &mut egui::Ui,
    index: usize,
    approval: &Approval,
    chats: &[Chat],
    commands: &mut Vec<UiCommand>,
) {
    let asker = approval
        .chat
        .as_ref()
        .and_then(|id| chats.iter().find(|c| &c.id == id))
        .map(|c| format!("{} ({})", c.title, c.agent))
        .unwrap_or_else(|| "An MCP client".to_string());
    Card::new().padding(10.0).show(ui, |ui| {
        ui.label(
            RichText::new(format!("{asker} asks to make a change"))
                .font(sans_medium(FONT_SM))
                .color(WARNING),
        );
        // What the agent says the change is for, above what it runs.
        if let Some(description) = approval
            .request
            .args
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            ui.label(RichText::new(description).font(sans(FONT_SM)).color(TEXT1));
        }
        egui::ScrollArea::vertical()
            .id_salt(("approval", index))
            .max_height(140.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&approval.summary)
                            .font(mono(FONT_XS))
                            .color(TEXT1),
                    )
                    .selectable(true)
                    .wrap(),
                );
            });
        ui.horizontal(|ui| {
            if primary_button(ui, "Allow").clicked() {
                commands.push(UiCommand::SettleApproval { index, allow: true });
            }
            if secondary_button(ui, "Deny").clicked() {
                commands.push(UiCommand::SettleApproval {
                    index,
                    allow: false,
                });
            }
            if let Some(chat) = &approval.chat
                && small_secondary_button(ui, "Allow all in this chat")
                    .on_hover_text("Turn off \"Ask before changes\" for this chat")
                    .clicked()
            {
                commands.push(UiCommand::SetChatAsk {
                    chat: chat.clone(),
                    ask: false,
                });
            }
        });
    });
    ui.add_space(SPACE_2);
}

fn chat_header(
    ui: &mut egui::Ui,
    chat: &Chat,
    scroll: &mut ChatScroll,
    commands: &mut Vec<UiCommand>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&chat.agent)
                .font(sans_medium(FONT_SM))
                .color(TEXT1),
        );
        let (text, color) = match &chat.status {
            ChatStatus::Resting => ("earlier chat", TEXT3),
            ChatStatus::Starting => ("starting", TEXT3),
            ChatStatus::Ready => ("ready", SUCCESS),
            ChatStatus::Busy => ("working", ACCENT),
            ChatStatus::Failed(_) => ("stopped", DANGER),
        };
        ui.label(RichText::new(text).font(sans(FONT_XS)).color(color));
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let mut ask = chat.ask;
                if ui
                    .checkbox(
                        &mut ask,
                        RichText::new("Ask before changes").font(sans(FONT_XS)),
                    )
                    .on_hover_text("Hold each change the agent makes for your OK")
                    .changed()
                {
                    commands.push(UiCommand::SetChatAsk {
                        chat: chat.id.clone(),
                        ask,
                    });
                }
                if !scroll.at_bottom
                    && icon_button(ui, "chevrons-down", "Jump to the latest").clicked()
                {
                    scroll.jump = Some(Jump::Latest);
                }
                if scroll.target(Jump::Next, 0).is_some()
                    && icon_button(ui, "chevron-down", "Your next message (Alt+Down)").clicked()
                {
                    scroll.jump = Some(Jump::Next);
                }
                if scroll.target(Jump::Previous, 0).is_some()
                    && icon_button(ui, "chevron-up", "Your previous message (Alt+Up)").clicked()
                {
                    scroll.jump = Some(Jump::Previous);
                }
            },
        );
    });
    if let ChatStatus::Failed(why) = &chat.status {
        ui.add(
            egui::Label::new(RichText::new(why).font(mono(FONT_XS)).color(DANGER))
                .selectable(true)
                .wrap(),
        );
    }
}

fn draw_entry(
    ui: &mut egui::Ui,
    chat: &Chat,
    index: usize,
    entry: &ChatEntry,
    commands: &mut Vec<UiCommand>,
) {
    match entry {
        ChatEntry::User { text, attachments } => {
            let rect = egui::Frame::new()
                .fill(BG3)
                .corner_radius(RADIUS_MD as u8)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !text.is_empty() {
                        ui.add(
                            egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT1))
                                .selectable(true)
                                .wrap(),
                        );
                    }
                    if !attachments.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for name in attachments {
                                chip(ui, name);
                            }
                        });
                    }
                })
                .response
                .rect;
            copy_on_hover(ui, rect, text, BG3);
        }
        ChatEntry::Agent {
            text,
            thought: false,
        } => {
            let rect = ui
                .scope(|ui| {
                    markdown::show(
                        ui,
                        egui::Id::new(("chat_message", &chat.id, index)),
                        text,
                        markdown::Style::default(),
                    )
                })
                .response
                .rect;
            copy_on_hover(ui, rect, text, BG1);
        }
        ChatEntry::Agent {
            text,
            thought: true,
        } => {
            egui::CollapsingHeader::new(RichText::new("Thinking").font(sans(FONT_XS)).color(TEXT3))
                .id_salt((&chat.id, index))
                .show(ui, |ui| {
                    markdown::show(
                        ui,
                        egui::Id::new(("chat_thought", &chat.id, index)),
                        text,
                        markdown::Style {
                            size: FONT_XS,
                            color: TEXT3,
                        },
                    );
                });
        }
        ChatEntry::Tool {
            title,
            status,
            output,
            label,
            ..
        } => {
            let (mark, color) = match status.as_str() {
                "completed" => ("✓", SUCCESS),
                "failed" => ("×", DANGER),
                _ => ("…", TEXT3),
            };
            // One of printCAD's own tools says what it does, in words;
            // another's title can be a whole command line. Either is cut
            // to the panel's width; the hover shows the agent's title.
            let header = match label {
                Some(label) => egui::Label::new(
                    RichText::new(format!("{mark} {label}"))
                        .font(sans(FONT_XS))
                        .color(color),
                ),
                None => egui::Label::new(
                    RichText::new(format!("{mark} {title}"))
                        .font(mono(FONT_XS))
                        .color(color),
                ),
            }
            .truncate();
            match output {
                Some(output) => {
                    let id = ui.make_persistent_id((&chat.id, index));
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        id,
                        false,
                    )
                    .show_header(ui, |ui| ui.add(header).on_hover_text(title))
                    .body(|ui| {
                        let shown = match output.char_indices().nth(OUTPUT_SHOWN) {
                            Some((cut, _)) => format!("{}\n…", &output[..cut]),
                            None => output.clone(),
                        };
                        ui.add(
                            egui::Label::new(RichText::new(shown).font(mono(FONT_XS)).color(TEXT2))
                                .selectable(true)
                                .wrap(),
                        );
                    });
                }
                None => {
                    ui.add(header).on_hover_text(title);
                }
            }
        }
        ChatEntry::Plan(entries) => {
            Card::new().padding(8.0).show(ui, |ui| {
                ui.label(
                    RichText::new("Plan")
                        .font(sans_medium(FONT_XS))
                        .color(TEXT2),
                );
                for step in entries {
                    let (mark, color) = match step.status.as_str() {
                        "completed" => ("✓", SUCCESS),
                        "in_progress" => ("•", ACCENT),
                        _ => ("–", TEXT3),
                    };
                    ui.label(
                        RichText::new(format!("{mark} {}", step.content))
                            .font(sans(FONT_XS))
                            .color(color),
                    );
                }
            });
        }
        ChatEntry::Permission {
            title,
            options,
            answer,
            ..
        } => {
            Card::new().padding(10.0).show(ui, |ui| {
                ui.label(
                    RichText::new(format!("The agent asks: {title}"))
                        .font(sans_medium(FONT_SM))
                        .color(WARNING),
                );
                match answer {
                    Some(chosen) => {
                        let name = options
                            .iter()
                            .find(|o| &o.id == chosen)
                            .map(|o| o.name.as_str())
                            .unwrap_or("Refused");
                        ui.label(RichText::new(name).font(sans(FONT_XS)).color(TEXT3));
                    }
                    None => {
                        ui.horizontal_wrapped(|ui| {
                            for option in options {
                                let clicked = if option.kind.starts_with("allow") {
                                    primary_button(ui, &option.name).clicked()
                                } else {
                                    secondary_button(ui, &option.name).clicked()
                                };
                                if clicked {
                                    commands.push(UiCommand::AnswerPermission {
                                        chat: chat.id.clone(),
                                        entry: index,
                                        option: Some(option.id.clone()),
                                    });
                                }
                            }
                        });
                    }
                }
            });
        }
        ChatEntry::Note(text) => {
            ui.label(
                RichText::new(text)
                    .font(sans(FONT_XS))
                    .color(TEXT3)
                    .italics(),
            );
        }
    }
}

/// A copy button at the top right of a message, while the pointer is over
/// it; `fill` matches what it sits on so the text under it does not show
/// through.
fn copy_on_hover(ui: &mut egui::Ui, rect: egui::Rect, text: &str, fill: egui::Color32) {
    if text.is_empty() || !ui.rect_contains_pointer(rect) {
        return;
    }
    let size = egui::vec2(20.0, 20.0);
    let top = rect.top().max(ui.clip_rect().top());
    let spot = egui::Rect::from_min_size(egui::pos2(rect.right() - size.x, top), size);
    ui.painter().rect_filled(spot, RADIUS_SM as u8, fill);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(spot));
    if icon_button(&mut child, "copy", "Copy the message").clicked() {
        ui.ctx().copy_text(text.to_string());
    }
}

fn input_id(chat: &Chat) -> egui::Id {
    egui::Id::new(("assistant_input", &chat.id))
}

/// The messages waiting for the agent's turn to end, above the box to
/// write in: each can be taken back into the box to edit, or dropped.
/// After Stop they wait for the user to let them go.
fn queued_list(ui: &mut egui::Ui, chat: &Chat, draft: &mut String, commands: &mut Vec<UiCommand>) {
    if chat.queued.is_empty() {
        return;
    }
    if chat.held {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Held after Stop")
                    .font(sans(FONT_XS))
                    .color(TEXT3),
            );
            if small_secondary_button(ui, "Send them")
                .on_hover_text("Send the queued messages, one per turn")
                .clicked()
            {
                commands.push(UiCommand::ResumeChatQueue(chat.id.clone()));
            }
        });
    }
    for queued in &chat.queued {
        egui::Frame::new()
            .fill(BG2)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(RADIUS_MD as u8)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Queued")
                            .font(sans_medium(FONT_XS))
                            .color(TEXT3),
                    );
                    if !queued.attachments.is_empty() {
                        let n = queued.attachments.len();
                        ui.label(
                            RichText::new(match n {
                                1 => "1 attachment".to_string(),
                                n => format!("{n} attachments"),
                            })
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close = match ui_kit::icon::image(ui.ctx(), "close", 12.0, TEXT2) {
                            Some(image) => egui::Button::image(image),
                            None => egui::Button::new(RichText::new("×").font(sans(FONT_XS))),
                        };
                        if ui
                            .add(close.frame(false))
                            .on_hover_text("Remove it from the queue")
                            .clicked()
                        {
                            commands.push(UiCommand::UnqueueChat {
                                chat: chat.id.clone(),
                                queued: queued.id,
                                keep: false,
                            });
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Edit").font(sans(FONT_XS)))
                                    .frame(false),
                            )
                            .on_hover_text("Take it back into the box; Enter queues it again")
                            .clicked()
                        {
                            *draft = match draft.trim().is_empty() {
                                true => queued.text.clone(),
                                false => format!("{}\n\n{draft}", queued.text),
                            };
                            ui.memory_mut(|m| {
                                m.request_focus(egui::Id::new(("assistant_input", &chat.id)))
                            });
                            commands.push(UiCommand::UnqueueChat {
                                chat: chat.id.clone(),
                                queued: queued.id,
                                keep: true,
                            });
                        }
                    });
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(queued_preview(&queued.text))
                            .font(sans(FONT_SM))
                            .color(TEXT2),
                    )
                    .wrap(),
                );
            });
        ui.add_space(SPACE_1);
    }
}

/// A queued message as its card shows it: the first few lines.
fn queued_preview(text: &str) -> String {
    const LINES: usize = 3;
    let lines: Vec<&str> = text.trim().lines().collect();
    match lines.len() > LINES {
        true => format!("{}…", lines[..LINES].join("\n")),
        false => lines.join("\n"),
    }
}

/// The box to write in, and under it the bar with the agent's session
/// options (permission mode, model, effort ...), its working spinner and
/// Send, or Stop while it works.
fn input(
    ui: &mut egui::Ui,
    chat: &Chat,
    draft: &mut String,
    paste_pressed: bool,
    commands: &mut Vec<UiCommand>,
) {
    // A resting chat takes a message too: sending wakes it.
    let open = matches!(
        chat.status,
        ChatStatus::Ready | ChatStatus::Busy | ChatStatus::Resting
    );
    let busy = chat.status == ChatStatus::Busy;
    let sendable = !draft.trim().is_empty() || !chat.attachments.is_empty();
    let id = input_id(chat);
    let mut send = false;
    // Up in an empty box brings back the last message sent, to edit.
    if open && draft.is_empty() && ui.memory(|m| m.has_focus(id)) {
        let last = chat.entries.iter().rev().find_map(|e| match e {
            ChatEntry::User { text, .. } if !text.is_empty() => Some(text),
            _ => None,
        });
        if let Some(last) = last
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp))
        {
            draft.clone_from(last);
            if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(draft.chars().count());
                edit.cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                edit.store(ui.ctx(), id);
            }
        }
    }
    if open && ui.memory(|m| m.has_focus(id)) {
        let mut pasted_files = Vec::new();
        let mut pasted_text = false;
        send = ui.input_mut(|i| {
            let mut hit = false;
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.is_none() => {
                    hit = true;
                    false
                }
                // Files copied in a file manager paste as their paths.
                egui::Event::Paste(text) => {
                    pasted_text = true;
                    match pasted_paths(text) {
                        Some(paths) => {
                            pasted_files.extend(paths);
                            false
                        }
                        None => true,
                    }
                }
                _ => true,
            });
            hit
        });
        if !pasted_files.is_empty() {
            commands.push(UiCommand::AttachPaths {
                chat: chat.id.clone(),
                paths: pasted_files,
            });
        }
        // A clipboard holding a picture and no text pastes nothing as
        // text: the picture goes with the message.
        if paste_pressed && !pasted_text {
            commands.push(UiCommand::AttachClipboard(chat.id.clone()));
        }
    }
    egui::Frame::new()
        .fill(BG2)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS_MD as u8)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                if !chat.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for (index, attachment) in chat.attachments.iter().enumerate() {
                            if chip(ui, &attachment.name()).clicked() {
                                commands.push(UiCommand::Detach {
                                    chat: chat.id.clone(),
                                    index,
                                });
                            }
                        }
                    });
                }
                let hint = match chat.status {
                    ChatStatus::Starting => "The agent is starting…",
                    ChatStatus::Resting => "Continue this chat (Enter sends)",
                    ChatStatus::Failed(_) => "The chat has stopped",
                    ChatStatus::Busy => {
                        "Write the next message (Enter queues it for when the agent is done)"
                    }
                    _ => "Ask the agent (Enter sends, Shift+Enter for a new line)",
                };
                ui.add_enabled(
                    open,
                    egui::TextEdit::multiline(draft)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .font(sans(FONT_SM))
                        .desired_rows(2)
                        .desired_width(ui.available_width())
                        .hint_text(hint),
                );
                ui.horizontal(|ui| {
                    attach_menu(ui, chat, open, commands);
                    // Send (or Stop) keeps the right end; the options take
                    // the rest, on more lines when it is narrow.
                    let line = ui.spacing().interact_size.y;
                    let end = 52.0;
                    let rest = (ui.available_width() - end).max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(rest, line),
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                        |ui| {
                            if busy {
                                ui.add(egui::Spinner::new().size(12.0).color(TEXT3));
                            }
                            for option in ordered(&chat.options) {
                                option_control(ui, chat, option, commands);
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), line),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if busy {
                                if stop_button(ui).on_hover_text("Stop the agent").clicked() {
                                    commands.push(UiCommand::CancelChat(chat.id.clone()));
                                }
                            } else if ui
                                .add_enabled(
                                    open && sendable,
                                    egui::Button::new(RichText::new("Send").font(sans(FONT_XS))),
                                )
                                .clicked()
                            {
                                send = true;
                            }
                        },
                    );
                });
            });
        });
    if send && sendable {
        commands.push(UiCommand::SendChat {
            chat: chat.id.clone(),
            text: std::mem::take(draft),
        });
    }
}

/// The files a paste names, when every line of it is a `file://` URI or
/// the absolute path of a file that exists; `None` for any other text.
fn pasted_paths(text: &str) -> Option<Vec<std::path::PathBuf>> {
    let mut paths = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let path = match line.strip_prefix("file://") {
            Some(uri) => std::path::PathBuf::from(percent_decoded(uri)?),
            None => std::path::PathBuf::from(line),
        };
        if !path.is_absolute() || !path.is_file() {
            return None;
        }
        paths.push(path);
    }
    (!paths.is_empty()).then_some(paths)
}

fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The "+" menu: files, or a picture of the view, to go with the next
/// prompt.
fn attach_menu(ui: &mut egui::Ui, chat: &Chat, open: bool, commands: &mut Vec<UiCommand>) {
    let plus = match ui_kit::icon::image(ui.ctx(), "plus", 14.0, TEXT2) {
        Some(icon) => egui::containers::menu::MenuButton::new(icon),
        None => egui::containers::menu::MenuButton::new(RichText::new("+").font(sans(FONT_SM))),
    };
    ui.add_enabled_ui(open, |ui| {
        plus.ui(ui, |ui| {
            ui.set_min_width(200.0);
            if ui
                .button(RichText::new("Files…").font(sans(FONT_SM)))
                .on_hover_text("Pictures go as pictures, small text files with their text, other files as a path the agent can open")
                .clicked()
            {
                commands.push(UiCommand::AttachFiles(chat.id.clone()));
                ui.close();
            }
            if ui
                .button(RichText::new("Picture of the view").font(sans(FONT_SM)))
                .clicked()
            {
                commands.push(UiCommand::AttachView(chat.id.clone()));
                ui.close();
            }
            if ui
                .button(RichText::new("Picture from the clipboard").font(sans(FONT_SM)))
                .on_hover_text("Ctrl+V in the message box does the same")
                .clicked()
            {
                commands.push(UiCommand::AttachClipboard(chat.id.clone()));
                ui.close();
            }
        })
        .0
        .on_hover_text(
            "Attach files or a picture of the view or the clipboard (or drop files on the panel)",
        );
    });
}

/// An attachment's name in a small rounded box; with a pointer on it,
/// clicking takes it off.
fn chip(ui: &mut egui::Ui, name: &str) -> egui::Response {
    egui::Frame::new()
        .fill(BG4)
        .corner_radius(RADIUS_SM as u8)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(name).font(sans(FONT_XS)).color(TEXT1));
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_text("Click to take it off")
}

/// The options in the order the bar shows them: the permission mode, the
/// model, the effort, then the rest as the agent lists them.
fn ordered(options: &[SessionOption]) -> Vec<&SessionOption> {
    let rank = |o: &SessionOption| match o.category.as_str() {
        "mode" => 0,
        "model" => 1,
        "thought_level" => 2,
        _ => 3,
    };
    let mut out: Vec<&SessionOption> = options.iter().collect();
    out.sort_by_key(|o| rank(o));
    out
}

/// One session option: a dropdown of its choices, or a switch.
fn option_control(
    ui: &mut egui::Ui,
    chat: &Chat,
    option: &SessionOption,
    commands: &mut Vec<UiCommand>,
) {
    let mut set = |value: serde_json::Value| {
        commands.push(UiCommand::SetChatOption {
            chat: chat.id.clone(),
            id: option.id.clone(),
            value,
        })
    };
    let hover = if option.description.is_empty() {
        option.name.clone()
    } else {
        format!("{}: {}", option.name, option.description)
    };
    match &option.value {
        OptionValue::Toggle(on) => {
            let mut on = *on;
            if toggle(ui, &mut on).on_hover_text(&hover).changed() {
                set(serde_json::Value::Bool(on));
            }
            ui.label(RichText::new(&option.name).font(sans(FONT_XS)).color(TEXT2));
        }
        OptionValue::Select { current, choices } => {
            let label = RichText::new(option.current_name())
                .font(sans(FONT_XS))
                .color(TEXT2);
            let button = match ui_kit::icon::image(ui.ctx(), "chevron-down", 12.0, TEXT3) {
                Some(chevron) => egui::containers::menu::MenuButton::new((label, chevron)),
                None => egui::containers::menu::MenuButton::new(label),
            };
            let response = button
                .config(
                    egui::containers::menu::MenuConfig::new()
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClick),
                )
                .ui(ui, |ui| {
                    ui.set_min_width(220.0);
                    ui.label(
                        RichText::new(&option.name)
                            .font(sans_medium(FONT_XS))
                            .color(TEXT3),
                    );
                    for choice in choices {
                        let on = &choice.value == current;
                        let text = RichText::new(&choice.name).font(sans(FONT_SM));
                        let mut row = ui.selectable_label(on, text);
                        if !choice.description.is_empty() {
                            row = row.on_hover_text(&choice.description);
                        }
                        if row.clicked() && !on {
                            set(serde_json::Value::String(choice.value.clone()));
                        }
                    }
                })
                .0;
            response.on_hover_text(hover);
        }
    }
}

/// A small square in the danger colour: stops the agent's turn.
fn stop_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    let fill = if response.hovered() {
        DANGER
    } else {
        DANGER.gamma_multiply(0.85)
    };
    ui.painter().rect_filled(rect.shrink(5.0), 2.0, fill);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A select option of a chat's agent, with these choices.
    fn select(id: &str, category: &str, choices: &[&str]) -> agents::acp::SessionOption {
        agents::acp::SessionOption {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            category: category.into(),
            value: agents::acp::OptionValue::Select {
                current: choices[0].into(),
                choices: choices
                    .iter()
                    .map(|c| agents::acp::OptionChoice {
                        value: c.to_string(),
                        name: c.to_string(),
                        description: String::new(),
                    })
                    .collect(),
            },
            via: agents::acp::OptionVia::Config,
        }
    }

    /// Lay the panel out in a window `width` wide, with a busy chat whose
    /// tool title is one very long line: the input's rect, what is left for
    /// the viewport, and everything the panel painted, before clipping.
    fn lay_out(width: f32) -> (egui::Rect, egui::Rect, egui::Rect) {
        let long = format!(
            "jq -r '.[0].text' {}",
            "/home/someone/a/long/path".repeat(12)
        );
        let mut chats = [Chat::for_test(
            "c",
            ChatStatus::Busy,
            vec![
                ChatEntry::User {
                    text: "make a box".into(),
                    attachments: Vec::new(),
                },
                ChatEntry::Tool {
                    input: None,
                    label: None,
                    id: "t".into(),
                    title: long.clone(),
                    kind: "execute".into(),
                    status: "pending".into(),
                    output: Some(long),
                },
            ],
        )];
        // An agent's options as wide as a real one's.
        chats[0].options = vec![
            select("mode", "mode", &["Bypass permissions", "Manual"]),
            select("model", "model", &["Opus 5.5 (1M context)", "Sonnet 5"]),
            select("effort", "thought_level", &["Default", "Max"]),
            agents::acp::SessionOption {
                id: "fast".into(),
                name: "Fast mode".into(),
                description: String::new(),
                category: "other".into(),
                value: agents::acp::OptionValue::Toggle(false),
                via: agents::acp::OptionVia::Config,
            },
        ];
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let mut state = AssistantState {
            open: true,
            ..Default::default()
        };
        let mut viewport = egui::Rect::NOTHING;
        let mut painted = egui::Rect::NOTHING;
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 860.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                draw_assistant(
                    ui,
                    &mut state,
                    AssistantInputs {
                        chats: &chats,
                        approvals: &[],
                        agents: vec!["Test".into()],
                        document_name: "Test",
                        document_rules: "",
                        paste_pressed: false,
                    },
                    &mut Vec::new(),
                );
                viewport = ui.available_rect_before_wrap();
            });
            output.textures_delta.clear();
            painted = output
                .shapes
                .iter()
                .map(|s| s.shape.visual_bounding_rect())
                .filter(|r| r.is_finite() && r.is_positive())
                .fold(egui::Rect::NOTHING, |all, r| all.union(r));
        }
        let input = ctx
            .read_response(egui::Id::new(("assistant_input", "c")))
            .expect("the input is drawn")
            .rect;
        (input, viewport, painted)
    }

    #[test]
    fn a_long_line_neither_widens_the_panel_nor_moves_the_input_off_the_bottom() {
        for width in [780.0, 1600.0] {
            let (input, viewport, painted) = lay_out(width);
            assert!(
                painted.left() >= viewport.right() - 1.0 && painted.right() <= width + 1.0,
                "at {width}: everything the panel paints is in it: {painted:?}, {viewport:?}"
            );
            assert!(
                viewport.width() >= MIN_VIEWPORT_WIDTH - 1.0,
                "at {width}: the viewport keeps its room: {viewport:?}"
            );
            assert!(
                width - viewport.width() <= 380.0 + 1.0,
                "at {width}: the panel stays at its width: {viewport:?}"
            );
            assert!(
                input.left() >= viewport.right() && input.right() <= width,
                "at {width}: the panel's content stays in the panel: {input:?}, {viewport:?}"
            );
            assert!(
                input.bottom() > 860.0 - 110.0 && input.height() < 120.0,
                "at {width}: the input sits at the bottom, a few lines tall: {input:?}"
            );
        }
    }

    #[test]
    fn a_paste_of_copied_files_names_them_and_other_text_stays_text() {
        let dir = std::env::temp_dir().join(format!("printcad-paste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a b.step");
        let b = dir.join("notes.txt");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "x").unwrap();
        let uri = format!("file://{}", a.display()).replace(' ', "%20");
        assert_eq!(
            pasted_paths(&format!("{uri}\r\n{}\n", b.display())),
            Some(vec![a.clone(), b.clone()])
        );
        assert_eq!(pasted_paths("make the wall 2 mm"), None);
        assert_eq!(
            pasted_paths(&format!("{}\nand some words", b.display())),
            None
        );
        assert_eq!(
            pasted_paths(&dir.join("gone.txt").display().to_string()),
            None
        );
        assert_eq!(pasted_paths("notes.txt"), None, "relative paths are text");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
