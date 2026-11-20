//! The AI agents page: the agents chats can talk to, how each starts, and
//! whether their changes wait for an OK.

use super::*;

/// Agents that speak the Agent Client Protocol, known to work with it.
const AGENT_PRESETS: &[(&str, &str, &[&str])] = &[
    ("Claude", "claude-agent-acp", &[]),
    ("Gemini CLI", "gemini", &["--experimental-acp"]),
];

/// The agents chats can talk to, and whether their changes wait for an OK.
pub(super) fn ai_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    let ai = &mut state.draft.ai;
    pref_group(
        ui,
        "Changes",
        vec![
            PrefRow::toggle(
                "Ask before an agent changes the document",
                &mut ai.ask_before_changes,
            )
            .hint("New chats start this way; each chat has its own switch"),
        ],
        filter,
    );
    pref_group(
        ui,
        "Notifications",
        vec![
            PrefRow::toggle("Tell me when an agent is done or needs me", &mut ai.notify)
                .hint("A desktop notification, while printCAD is in the background"),
        ],
        filter,
    );
    let rules = &mut ai.rules;
    pref_block(ui, &["Rules for every document", "agents"], filter, |ui| {
        ui.add_space(SPACE_2);
        ui.label(
            RichText::new("Rules for every document")
                .font(sans_semibold(FONT_SM))
                .color(TEXT1),
        );
        ui.label(
            RichText::new(
                "What every agent keeps to, in every document: units, wall thicknesses, how to \
             name things, what never to change. A document can add its own in the Assistant \
             panel. Chats already open take a change with their next message.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        ui.add_space(SPACE_1);
        ui.add(
        egui::TextEdit::multiline(rules)
            .hint_text(
                "Keep every wall at least 1.2 mm thick.\nName features after what they are for.",
            )
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .font(mono(FONT_SM)),
    );
    });
    let folders = &mut ai.folders;
    pref_block(ui, &["Folders agents may reach"], filter, |ui| {
        ui.add_space(SPACE_2);
        ui.label(
            RichText::new("Folders agents may reach")
                .font(sans_semibold(FONT_SM))
                .color(TEXT1),
        );
        ui.label(
            RichText::new(
                "A file an agent's commands read or write (an import, an export, a picture) must \
             be in an open document's folder, the system's temporary folder, or one of these, \
             one per line, and never a hidden file or folder.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        ui.add_space(SPACE_1);
        let mut text = folders.join("\n");
        if ui
            .add(
                egui::TextEdit::multiline(&mut text)
                    .hint_text("/home/me/prints")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .font(mono(FONT_SM)),
            )
            .changed()
        {
            *folders = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
        }
    });
    let agents = &mut ai.agents;
    let words = [
        "Agents",
        "ai",
        "command",
        "chat",
        "Agent Client Protocol",
        "MCP",
    ];
    pref_block(ui, &words, filter, |ui| {
        ui.add_space(SPACE_2);
        ui.label(
            RichText::new("Agents")
                .font(sans_semibold(FONT_SM))
                .color(TEXT1),
        );
        ui.label(
            RichText::new(
                "Programs that speak the Agent Client Protocol. A chat starts one and gives it \
             printCAD's tools, which reach the document through `printcad --mcp`; any MCP \
             client can use that command too.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        ui.add_space(SPACE_1);
        let mut remove = None;
        for (i, agent) in agents.iter_mut().enumerate() {
            Card::new().padding(10.0).show(ui, |ui| {
                egui::Grid::new(("agent_row", i))
                    .num_columns(2)
                    .spacing([SPACE_3, SPACE_2])
                    .show(ui, |ui| {
                        let field = |ui: &mut Ui, label: &str, text: &mut String, hint: &str| {
                            ui.label(RichText::new(label).font(sans(FONT_SM)).color(TEXT2));
                            let changed = ui
                                .add(
                                    egui::TextEdit::singleline(text)
                                        .hint_text(hint)
                                        .desired_width(300.0)
                                        .font(mono(FONT_SM)),
                                )
                                .changed();
                            ui.end_row();
                            changed
                        };
                        field(ui, "Name", &mut agent.name, "What the chat shows");
                        field(ui, "Command", &mut agent.command, "The program to start");
                        let mut args = join_words(&agent.args);
                        if field(ui, "Arguments", &mut args, "Words after the command") {
                            agent.args = split_words(&args);
                        }
                        let mut env = join_words(
                            &agent
                                .env
                                .iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect::<Vec<_>>(),
                        );
                        if field(ui, "Environment", &mut env, "NAME=value ...") {
                            agent.env = split_words(&env)
                                .into_iter()
                                .filter_map(|pair| {
                                    let (k, v) = pair.split_once('=')?;
                                    Some((k.to_string(), v.to_string()))
                                })
                                .collect();
                        }
                    });
                ui.add_space(SPACE_1);
                if ui_kit::widgets::small_secondary_button(ui, "Remove").clicked() {
                    remove = Some(i);
                }
            });
            ui.add_space(SPACE_2);
        }
        if let Some(i) = remove {
            agents.remove(i);
        }
        ui.horizontal_wrapped(|ui| {
            for (name, command, args) in AGENT_PRESETS {
                if ui_kit::widgets::small_secondary_button(ui, &format!("Add {name}")).clicked() {
                    agents.push(settings::AgentSettings {
                        name: name.to_string(),
                        command: command.to_string(),
                        args: args.iter().map(|a| a.to_string()).collect(),
                        ..Default::default()
                    });
                }
            }
            if ui_kit::widgets::small_secondary_button(ui, "Add another").clicked() {
                agents.push(settings::AgentSettings {
                    name: "Agent".to_string(),
                    ..Default::default()
                });
            }
        });
    });
}

/// Words joined with spaces, a word with a space in it quoted.
fn join_words(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if w.contains(char::is_whitespace) || w.is_empty() {
                format!("\"{}\"", w.replace('"', "\\\""))
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words split at spaces, a quoted run kept as one word.
fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut escaped, mut any) = (false, false, false);
    for c in text.chars() {
        match c {
            _ if escaped => {
                word.push(c);
                escaped = false;
            }
            '\\' if quoted => escaped = true,
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                any = false;
            }
            c => word.push(c),
        }
    }
    if any || !word.is_empty() {
        words.push(word);
    }
    words
}
