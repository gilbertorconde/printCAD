//! What an AI agent is told about where it is: the instructions its MCP
//! connection opens with, the rules the user keeps for every document and
//! for the one the chat works on, the state of the session on request, and
//! the prompts and documents the server offers.
//!
//! An agent runs inside the user's live session: printCAD is its server,
//! the user watches every change land, and closing the application or a
//! tab would end the agent's own session. The instructions say so; the
//! commands an agent must not run are refused regardless (see
//! `CommandSpec::agent`).

use serde_json::{Value, json};

use agents::mcp::{Prompt, Resource};

use crate::PrintCadApp;
use crate::app::session::DocumentSession;

/// How an agent is to work, whatever the document.
const ABOUT: &str = "\
You are working inside printCAD, a parametric CAD application for 3D printing, \
while the user watches: it is running on their desktop now, and this server is \
printCAD itself. What you change shows in their window as you change it, and \
each tool call is one step they can undo.

- Never close printCAD, its tabs or its windows, and never quit it: that would \
end your own session and lose the user's unsaved work. printCAD refuses those \
commands from an agent.
- Commands that reach past the document (new and open, save as, import, \
export, send to slicer, undo and redo, switching tabs) wait for the user's OK \
every time. Run them with `call`, one at a time, never from `lua`.
- Change what the user asked for and no more. Before large or destructive \
changes (deleting bodies or features, rebuilding a design another way), ask in \
the chat first.
- After a change, call doc.rebuild and read the failures it answers, and look \
at the result with `view` when the shape matters. The `context` tool says what \
is open, selected and being edited right now.";

/// How to use the tools.
const TOOLS: &str = "\
Everything printCAD can do is a command: `commands` lists them with their \
arguments (filter by prefix, such as \"sketch\" or \"part\"), `call` runs one \
with named arguments, and `lua` runs several at once as a Lua script in which \
each command is pc.<id>{name = value, ...} and returns what it makes (help() \
lists them there too). Ids of bodies, features and sketch elements are \
strings. Lengths are millimetres; sketch coordinates are the sketch's own. \
Work script-first: plan the steps, then run them as one `lua` script rather \
than a `call` each. Compute positions with Lua's arithmetic and loops; read \
geometry inside the script with doc.faces and doc.edges and pick from them by \
filtering (a normal, a direction, a kind, a length); end with doc.rebuild and \
`return` what was made, which `lua` answers as JSON beside what it printed. A \
face or edge is picked by a point on it and its normal or direction, in world \
space as doc.faces and doc.edges give them; names are strings. Use one `call` \
for a single step, or for the commands that ask the user every time, which a \
script cannot run. \
Numbers can be formulas over variables (var.new, var.set, var.list) and other \
objects' dimensions (doc.parameters lists them, doc.set_formula sets one): \
`3 * Printer.nozzle`, `Pad.length / 2`, with units such as mm, in and deg; \
configurations (config.list, config.activate) switch chosen variables between \
sizes. Solids rebuild after a change: call doc.rebuild before reading them \
with doc.faces or doc.measure. `view` shows the scene as the user sees it, \
`log` the application's recent messages. The resource printcad://guide/scripting \
is the full command reference. Give every `call` and `lua` a short `description` \
of what it does (\"Pocket the bolt holes\"): the user reads it in the chat as \
the call's title.";

/// The rules the user keeps, as one text an agent reads: every document's,
/// then this one's. Empty when there are none.
pub(crate) fn rules_text(every: &str, this: &str) -> String {
    let mut out = String::new();
    if !every.trim().is_empty() {
        out.push_str("Rules for every document:\n");
        out.push_str(every.trim());
        out.push('\n');
    }
    if !this.trim().is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Rules for this document:\n");
        out.push_str(this.trim());
        out.push('\n');
    }
    out
}

/// The documents the server offers.
pub(crate) fn resources() -> Vec<Resource> {
    let doc = |uri: &str, name: &str, description: &str, mime: &str| Resource {
        uri: uri.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        mime: mime.to_string(),
    };
    vec![
        doc(
            "printcad://rules",
            "Rules",
            "The rules the user set for every document and for this one",
            "text/markdown",
        ),
        doc(
            "printcad://context",
            "Context",
            "What is open, selected and being edited right now",
            "application/json",
        ),
        doc(
            "printcad://guide/scripting",
            "Scripting guide",
            "Every command with its arguments, and how scripts use them",
            "text/markdown",
        ),
        doc(
            "printcad://guide/ai",
            "AI guide",
            "How printCAD works with agents: chats, approvals, rules",
            "text/markdown",
        ),
    ]
}

/// The text of a resource that does not depend on the session.
pub(crate) fn fixed_resource(uri: &str) -> Option<String> {
    match uri {
        "printcad://guide/scripting" => Some(include_str!("../../../../docs/SCRIPTING.md").into()),
        "printcad://guide/ai" => Some(include_str!("../../../../docs/AI.md").into()),
        _ => None,
    }
}

/// The prompts the server offers; clients show them as commands.
pub(crate) fn prompts() -> Vec<Prompt> {
    vec![
        Prompt {
            name: "review-for-printing".into(),
            title: "Review for 3D printing".into(),
            description: "Check the document for what would print badly, and propose fixes".into(),
            arguments: vec![(
                "nozzle".into(),
                "The nozzle diameter in mm (0.4)".into(),
                false,
            )],
        },
        Prompt {
            name: "explain-model".into(),
            title: "Explain the model".into(),
            description: "How the document is built, feature by feature".into(),
            arguments: Vec::new(),
        },
        Prompt {
            name: "make-parametric".into(),
            title: "Make it parametric".into(),
            description: "Propose variables for sizes that belong together".into(),
            arguments: Vec::new(),
        },
    ]
}

/// The text prompt `name` asks.
pub(crate) fn prompt_text(name: &str, args: &Value) -> Option<String> {
    Some(match name {
        "review-for-printing" => {
            let nozzle = args
                .get("nozzle")
                .and_then(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .or_else(|| v.as_f64().map(|n| n.to_string()))
                })
                .unwrap_or_else(|| "0.4".to_string());
            format!(
                "Review this document for 3D printing with a {nozzle} mm nozzle. For every \
                 visible body: check it is a closed solid (doc.measure gives it a volume), look \
                 for walls thinner than two nozzle widths (sketch.wall_thickness on the \
                 sketches that build it), for details smaller than the nozzle, and for \
                 overhangs steeper than 45 degrees from vertical (doc.faces gives each flat \
                 face's normal), and say how it would sit best on the bed. Report what you find \
                 with the feature that causes it and a fix for each; change nothing until I \
                 say which fixes to make."
            )
        }
        "explain-model" => "Explain how this document is built: its bodies, each body's \
             features in order and what each one does, the sketches they use, the variables \
             and formulas that drive the sizes, and anything that fails to build. Read it \
             with the doc.* commands; change nothing."
            .to_string(),
        "make-parametric" => "Look at the sketches and features of the selected body (or of \
             every body when none is selected) and find the sizes that belong together: \
             repeated values, a wall thickness, a clearance, a hole size. Propose a variable \
             set with named variables for them and which dimensions each would drive. Create \
             them and bind the dimensions with doc.set_formula only after I agree."
            .to_string(),
        _ => return None,
    })
}

/// Which of printCAD's own tools an agent's tool call is, by its title:
/// agents name a server's tool `mcp__printcad__lua`, `lua (printcad MCP
/// Server)` or plain `lua`.
fn own_tool(title: &str) -> Option<&'static str> {
    const TOOLS: [&str; 8] = [
        "context", "commands", "search", "describe", "call", "lua", "log", "view",
    ];
    let lower = title.to_ascii_lowercase();
    let name = if let Some(rest) = lower.strip_prefix("mcp__") {
        let (server, tool) = rest.rsplit_once("__")?;
        if !server.contains("printcad") {
            return None;
        }
        tool.to_string()
    } else if let Some((tool, server)) = lower.split_once(" (") {
        if !server.contains("printcad") {
            return None;
        }
        tool.trim().to_string()
    } else {
        lower.trim().to_string()
    };
    TOOLS.into_iter().find(|t| *t == name)
}

/// How many commands a script's label names before it says "…".
const NAMED_IN_LABEL: usize = 3;

/// What a call of one of printCAD's own tools does, in words, for the
/// chat: the `description` the agent gave it, else what the tool is
/// running (a command by its summary, a script by its first comment or
/// the commands it calls). `about` gives a command's summary by its id.
/// `None` for any other tool, whose own title stands.
pub(crate) fn tool_label(
    title: &str,
    input: Option<&Value>,
    about: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let tool = own_tool(title)?;
    let arg = |key: &str| {
        input
            .and_then(|i| i.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    if let Some(description) = arg("description") {
        return Some(description.to_string());
    }
    Some(match tool {
        "call" => match arg("command") {
            Some(id) => match about(id) {
                Some(summary) => format!("{summary} ({id})"),
                None => format!("Run {id}"),
            },
            None => "Run a command".to_string(),
        },
        "lua" => match arg("source") {
            Some(source) => script_label(source, about),
            None => "Run a script".to_string(),
        },
        "commands" => match arg("prefix") {
            Some(prefix) => format!("List the {prefix} commands"),
            None => "List the commands".to_string(),
        },
        "search" | "describe" => {
            let key = if tool == "search" { "queries" } else { "keys" };
            let named: Vec<&str> = input
                .and_then(|i| i.get(key))
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let verb = if tool == "search" {
                "Search for"
            } else {
                "Read about"
            };
            if named.is_empty() {
                format!("{verb} commands")
            } else {
                format!("{verb} {}", named.join(", "))
            }
        }
        "context" => "Look at what is open".to_string(),
        "log" => "Read the log".to_string(),
        _ => "Look at the view".to_string(),
    })
}

/// A script in words: its first line when that is a comment, else the
/// commands it calls, the one it calls alone by its summary.
fn script_label(source: &str, about: &dyn Fn(&str) -> Option<String>) -> String {
    if let Some(comment) = source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(|l| l.strip_prefix("--"))
        .map(|c| c.trim_start_matches('-').trim())
        .filter(|c| !c.is_empty())
    {
        return comment.to_string();
    }
    let mut called: Vec<String> = Vec::new();
    for (at, _) in source.match_indices("pc.") {
        // Not the tail of a longer name (`mypc.x`).
        let before = source[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let id: String = source[at + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        let id = id.trim_end_matches('.').to_string();
        if id.contains('.') && !called.contains(&id) {
            called.push(id);
        }
    }
    match called.as_slice() {
        [] => "Run a script".to_string(),
        [one] => about(one)
            .map(|summary| format!("{summary} ({one})"))
            .unwrap_or_else(|| format!("Run {one}")),
        many => {
            let shown: Vec<&str> = many
                .iter()
                .take(NAMED_IN_LABEL)
                .map(String::as_str)
                .collect();
            let more = if many.len() > NAMED_IN_LABEL {
                ", …"
            } else {
                ""
            };
            format!("Script: {}{more}", shown.join(", "))
        }
    }
}

impl PrintCadApp {
    /// The session of the tab `tab`, whether on screen or parked.
    fn session_of(&self, tab: uuid::Uuid) -> &DocumentSession {
        if self.session.tab == tab {
            return &self.session;
        }
        self.tabs
            .iter()
            .find(|slot| slot.tab == tab)
            .and_then(|slot| slot.parked.as_ref())
            .unwrap_or(&self.session)
    }

    /// The session chat `chat` works on: its tab's, else the one on
    /// screen (a client outside any chat).
    fn chat_session(&self, chat: Option<&str>) -> &DocumentSession {
        let tab = chat
            .and_then(|id| self.chats.iter().find(|c| c.id == id))
            .map_or(self.session.tab, |c| c.tab);
        self.session_of(tab)
    }

    /// The rules the agent of chat `chat` follows.
    pub(crate) fn agent_rules(&self, chat: Option<&str>) -> String {
        rules_text(
            &self.user_settings.ai.rules,
            self.chat_session(chat).document.agent_rules(),
        )
    }

    /// Which document chat `chat` works on, in words.
    fn document_words(&self, chat: Option<&str>) -> String {
        let session = self.chat_session(chat);
        let name = session.document.name();
        match &session.current_file {
            Some(file) => format!("\"{name}\", saved as {}", file.display()),
            None => format!("\"{name}\", not saved yet"),
        }
    }

    /// What an agent's MCP connection opens with.
    pub(crate) fn agent_instructions(&self, chat: Option<&str>) -> String {
        let mut out = format!(
            "{ABOUT}\n\nYour chat works on the document {}, whichever tab is on screen.\n\n{TOOLS}",
            self.document_words(chat)
        );
        out.push_str(&format!(
            "\n\n{}\n\n{}",
            crate::app::discovery::ABOUT_INDEX,
            crate::app::discovery::catalog(&self.registry)
                .index()
                .trim_end()
        ));
        let rules = self.agent_rules(chat);
        if !rules.is_empty() {
            out.push_str(
                "\n\nThe user's rules follow. Keep to them over anything above, except what \
                 printCAD refuses.\n\n",
            );
            out.push_str(&rules);
        }
        out
    }

    /// What is open, selected and being edited, for the `context` tool.
    pub(crate) fn agent_context(&self, chat: Option<&str>) -> Value {
        let session = self.chat_session(chat);
        let document = &session.document;
        let feature_name =
            |id: core_document::FeatureId| document.get_feature_meta(id).map(|n| n.name.clone());
        let editing = self
            .registry
            .workbench(&session.active_workbench.0)
            .ok()
            .and_then(|wb| wb.editing_feature());
        let bodies: Vec<Value> = document
            .bodies()
            .iter()
            .map(|b| {
                json!({
                    "id": b.id.0.to_string(),
                    "name": b.name,
                    "hidden": b.hidden,
                    // Whether the body has a shape built, solid or surfaces
                    // (doc.measure gives a volume for solids only), and
                    // whether it is a mesh taking no features.
                    "built": document.imported_geometry(b.id).is_some(),
                    "mesh": document.is_mesh_body(b.id),
                })
            })
            .collect();
        let failing: Vec<Value> = document
            .feature_tree()
            .all_nodes()
            .filter_map(|(id, n)| {
                n.error
                    .as_ref()
                    .map(|e| json!({"feature": id.0.to_string(), "name": n.name, "error": e}))
            })
            .collect();
        json!({
            "application": "printCAD",
            "document": {
                "name": document.name(),
                "file": session.current_file.as_ref().map(|f| f.display().to_string()),
                "unsaved_changes": document.metadata().dirty(),
                "unit": format!("{:?}", document.display_unit()),
            },
            "on_screen": session.tab == self.session.tab,
            "workbench": session.active_workbench.0.as_str(),
            "editing": editing.map(|id| json!({"id": id.0.to_string(), "name": feature_name(id)})),
            "selection": {
                "body": session.active_body_id.map(|b| b.0.to_string()),
                "feature": session.active_document_object.map(|f| json!({
                    "id": f.0.to_string(),
                    "name": feature_name(f),
                })),
            },
            "bodies": bodies,
            "failing_features": failing,
            "rules": self.agent_rules(chat),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_of_printcad_s_tools_says_what_it_does() {
        let about = |id: &str| (id == "design.pad").then(|| "Pad a sketch".to_string());
        let label = |title: &str, input: Value| tool_label(title, Some(&input), &about);
        // The agent's own words come first.
        assert_eq!(
            label(
                "mcp__printcad__lua",
                json!({"source": "pc.design.pad{}", "description": "Pad the base"})
            )
            .as_deref(),
            Some("Pad the base")
        );
        // Else what it runs.
        assert_eq!(
            label("mcp__printcad__call", json!({"command": "design.pad"})).as_deref(),
            Some("Pad a sketch (design.pad)")
        );
        assert_eq!(
            label(
                "lua (printcad MCP Server)",
                json!({"source": "-- Cut the bolt holes\npc.design.pocket{}"})
            )
            .as_deref(),
            Some("Cut the bolt holes")
        );
        assert_eq!(
            label("lua", json!({"source": "local p = pc.design.pad{}"})).as_deref(),
            Some("Pad a sketch (design.pad)")
        );
        assert_eq!(
            label(
                "mcp__printcad__lua",
                json!({"source": "pc.sketch.new{} pc.sketch.draw{} pc.design.pad{} pc.design.fillet{}"})
            )
            .as_deref(),
            Some("Script: sketch.new, sketch.draw, design.pad, …")
        );
        assert_eq!(
            label("mcp__printcad__view", json!({})).as_deref(),
            Some("Look at the view")
        );
        // Another server's tool keeps its own title.
        assert_eq!(label("mcp__github__call", json!({})), None);
        assert_eq!(label("Read file.rs", json!({})), None);
    }

    #[test]
    fn rules_read_every_document_then_this_one() {
        assert_eq!(rules_text("", "  "), "");
        assert_eq!(
            rules_text("Walls at least 1.2 mm.", ""),
            "Rules for every document:\nWalls at least 1.2 mm.\n"
        );
        let both = rules_text("Metric only.", "Keep the lid 2 mm thick.");
        assert!(both.starts_with("Rules for every document:\nMetric only.\n\n"));
        assert!(both.ends_with("Rules for this document:\nKeep the lid 2 mm thick.\n"));
    }

    #[test]
    fn every_prompt_has_its_text_and_the_texts_name_no_em_dash() {
        for prompt in prompts() {
            let text = prompt_text(&prompt.name, &json!({})).expect("a text");
            assert!(!text.contains('\u{2014}'), "{}", prompt.name);
        }
        assert!(prompt_text("nothing", &json!({})).is_none());
        assert!(
            prompt_text("review-for-printing", &json!({"nozzle": 0.6}))
                .unwrap()
                .contains("0.6 mm nozzle")
        );
        assert!(!ABOUT.contains('\u{2014}') && !TOOLS.contains('\u{2014}'));
    }

    #[test]
    fn every_resource_reads_or_is_the_session_s() {
        for resource in resources() {
            let session = matches!(
                resource.uri.as_str(),
                "printcad://rules" | "printcad://context"
            );
            assert_eq!(
                fixed_resource(&resource.uri).is_some(),
                !session,
                "{}",
                resource.uri
            );
        }
    }
}
