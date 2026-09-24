//! The assistant panel on the right: chats with AI agents, one tab each.
//!
//! The app keeps the chats; this draws them and answers with commands.
//! Above the chat, the changes agents asked for that wait for the user's
//! OK. In the chat: the conversation (the agent's message, its thinking
//! folded away, the tools it called, its plan, its requests for
//! permission), and a box to write in: Enter sends, Shift+Enter starts a
//! new line.

use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, primary_button, secondary_button, small_secondary_button};
use ui_kit::{mono, sans, sans_medium, sans_semibold};

use super::UiCommand;
use crate::app::chats::{Chat, ChatEntry, ChatStatus};
use crate::app::mcp::Approval;

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

    egui::Panel::right("assistant_panel")
        .resizable(true)
        .default_size(380.0)
        .size_range(300.0..=720.0)
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ui, |ui| {
            tab_strip(ui, state, chats, &agents, commands, &mut result);
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
            chat_header(ui, chat, commands);
            ui.add_space(SPACE_1);
            let draft = state.drafts.entry(chat.id.clone()).or_default();
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                input(ui, chat, draft, commands);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = SPACE_2;
                            for (index, entry) in chat.entries.iter().enumerate() {
                                draw_entry(ui, chat, index, entry, commands);
                            }
                            if chat.status == ChatStatus::Busy {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
                                    ui.label(
                                        RichText::new("Working").font(sans(FONT_XS)).color(TEXT3),
                                    );
                                });
                            }
                        });
                    });
            });
        });
    result
}

fn tab_strip(
    ui: &mut egui::Ui,
    state: &mut AssistantState,
    chats: &[Chat],
    agents: &[String],
    commands: &mut Vec<UiCommand>,
    result: &mut AssistantResult,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Assistant")
                .font(sans_semibold(FONT_SM))
                .color(TEXT1),
        );
        ui.add_space(SPACE_2);
        for chat in chats {
            let on = state.active.as_deref() == Some(chat.id.as_str());
            let response = ui
                .selectable_label(on, RichText::new(&chat.title).font(sans(FONT_SM)))
                .on_hover_text(&chat.agent);
            if response.clicked() {
                state.active = Some(chat.id.clone());
            }
        }
        ui.menu_button(RichText::new("+ New chat").font(sans(FONT_SM)), |ui| {
            new_chat_items(ui, agents, commands, result);
        });
    });
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

fn chat_header(ui: &mut egui::Ui, chat: &Chat, commands: &mut Vec<UiCommand>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&chat.agent)
                .font(sans_medium(FONT_SM))
                .color(TEXT1),
        );
        let (text, color) = match &chat.status {
            ChatStatus::Starting => ("starting", TEXT3),
            ChatStatus::Ready => ("ready", SUCCESS),
            ChatStatus::Busy => ("working", ACCENT),
            ChatStatus::Failed(_) => ("stopped", DANGER),
        };
        ui.label(RichText::new(text).font(sans(FONT_XS)).color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if small_secondary_button(ui, "Close")
                .on_hover_text("End the chat and its agent")
                .clicked()
            {
                commands.push(UiCommand::CloseChat(chat.id.clone()));
            }
            if chat.status == ChatStatus::Busy && small_secondary_button(ui, "Stop").clicked() {
                commands.push(UiCommand::CancelChat(chat.id.clone()));
            }
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
        });
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
        ChatEntry::User(text) => {
            egui::Frame::new()
                .fill(BG3)
                .corner_radius(RADIUS_MD as u8)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT1))
                            .selectable(true)
                            .wrap(),
                    );
                });
        }
        ChatEntry::Agent {
            text,
            thought: false,
        } => {
            ui.add(
                egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT1))
                    .selectable(true)
                    .wrap(),
            );
        }
        ChatEntry::Agent {
            text,
            thought: true,
        } => {
            egui::CollapsingHeader::new(RichText::new("Thinking").font(sans(FONT_XS)).color(TEXT3))
                .id_salt((&chat.id, index))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).font(sans(FONT_XS)).color(TEXT3))
                            .selectable(true)
                            .wrap(),
                    );
                });
        }
        ChatEntry::Tool {
            title,
            status,
            output,
            ..
        } => {
            let (mark, color) = match status.as_str() {
                "completed" => ("✓", SUCCESS),
                "failed" => ("✕", DANGER),
                _ => ("…", TEXT3),
            };
            let header = RichText::new(format!("{mark} {title}"))
                .font(mono(FONT_XS))
                .color(color);
            match output {
                Some(output) => {
                    egui::CollapsingHeader::new(header)
                        .id_salt((&chat.id, index))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(output).font(mono(FONT_XS)).color(TEXT2),
                                )
                                .selectable(true)
                                .wrap(),
                            );
                        });
                }
                None => {
                    ui.label(header);
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
                        "in_progress" => ("●", ACCENT),
                        _ => ("○", TEXT3),
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

fn input(ui: &mut egui::Ui, chat: &Chat, draft: &mut String, commands: &mut Vec<UiCommand>) {
    let open = matches!(chat.status, ChatStatus::Ready | ChatStatus::Busy);
    let id = egui::Id::new(("assistant_input", &chat.id));
    let mut send = false;
    if open && ui.memory(|m| m.has_focus(id)) {
        send = ui.input_mut(|i| {
            let mut hit = false;
            i.events.retain(|e| {
                let bare_enter = matches!(
                    e,
                    egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. }
                        if modifiers.is_none()
                );
                hit |= bare_enter;
                !bare_enter
            });
            hit
        });
    }
    ui.horizontal(|ui| {
        let hint = match chat.status {
            ChatStatus::Starting => "The agent is starting…",
            ChatStatus::Failed(_) => "The chat has stopped",
            _ => "Ask the agent (Enter sends, Shift+Enter for a new line)",
        };
        ui.add_enabled(
            open,
            egui::TextEdit::multiline(draft)
                .id(id)
                .font(sans(FONT_SM))
                .desired_rows(2)
                .desired_width(ui.available_width() - 60.0)
                .hint_text(hint),
        );
        if ui
            .add_enabled(open && !draft.trim().is_empty(), egui::Button::new("Send"))
            .clicked()
        {
            send = true;
        }
    });
    if send && !draft.trim().is_empty() {
        commands.push(UiCommand::SendChat {
            chat: chat.id.clone(),
            text: std::mem::take(draft),
        });
    }
}
