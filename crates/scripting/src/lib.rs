//! Lua scripts run against the application's commands.
//!
//! A script reaches every command through the `pc` table: `pc.design.pad{
//! sketch = s, length = 20 }` calls the command `design.pad` with those named
//! arguments and answers what it answers. Lua tables and the commands' JSON
//! values convert both ways; ids are strings. A command that fails raises a
//! Lua error, which `pcall` catches.
//!
//! The engine knows no command itself: a [`Host`] lists them and runs
//! them, so the same scripts run against the application or a test.

mod engine;
mod job;
mod record;
mod thread;
pub use engine::ScriptEngine;
pub use job::{Event, Job};
pub use record::Recorder;
pub use thread::ScriptThread;

use core_document::{CommandArgs, CommandError, CommandResult, CommandSpec};

/// What runs the commands a script calls.
pub trait Host {
    /// Every command there is, for `help`.
    fn commands(&self) -> Vec<CommandSpec>;
    /// Run one command.
    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult;
}

/// What a run printed, the value it came to and the error that stopped it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunOutput {
    pub printed: Vec<String>,
    /// The value of a console line that is an expression, shown as text.
    pub value: Option<String>,
    /// What the run's chunk returned, as JSON: tables become objects or
    /// lists, several values a list. `None` when it returned nothing or
    /// nil.
    pub returned: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// What a run stopped from outside says.
pub const STOPPED: &str = "stopped";

/// `app.commands`: every command whose id starts with `prefix`.
pub(crate) fn list_commands(commands: Vec<CommandSpec>, args: &CommandArgs) -> serde_json::Value {
    let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
    serde_json::Value::Array(
        commands
            .iter()
            .filter(|c| c.id.starts_with(prefix))
            .map(CommandSpec::to_json)
            .collect(),
    )
}

/// How many commands `help` lists for a word.
const HELP_HITS: usize = 10;

/// The commands that match `args.query` best, best first, as `help`
/// lists them.
pub(crate) fn search_commands(commands: Vec<CommandSpec>, args: &CommandArgs) -> serde_json::Value {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let catalog = agents::discovery::Catalog::new(commands.iter().map(command_entry).collect());
    let found = catalog.search(&[query], HELP_HITS);
    serde_json::Value::Array(
        found[0]
            .hits
            .iter()
            .filter_map(|h| commands.iter().find(|c| c.id == h.entry.id))
            .map(CommandSpec::to_json)
            .collect(),
    )
}

/// A command as the discovery catalog reads it: what `search` ranks and
/// `describe` shows, for the MCP server and `help` alike.
pub fn command_entry(spec: &CommandSpec) -> agents::discovery::Entry {
    use agents::discovery::{Entry, Param};
    use core_document::AgentAccess;
    let mut entry = Entry::command(&spec.id, &spec.summary);
    entry.params = spec
        .params
        .iter()
        .map(|p| {
            let optional = if p.required { "" } else { ", optional" };
            let doc = if p.doc.is_empty() {
                String::new()
            } else {
                format!(": {}", p.doc)
            };
            Param {
                name: p.name.clone(),
                about: format!("({}{optional}){doc}", p.kind.name()),
            }
        })
        .collect();
    if spec.returns != "nothing" {
        entry.returns = spec.returns.clone();
    }
    if let Some(extra) = &spec.extra_args {
        entry.other_args = extra.clone();
    }
    if spec.read_only {
        entry
            .details
            .push("Only reads: an agent runs it without asking.".into());
    }
    match &spec.agent {
        AgentAccess::AsAllowed => {}
        AgentAccess::AlwaysAsk => entry.details.push(
            "Waits for the user's OK every time: run it with `call` on its own, \
             not from a script."
                .into(),
        ),
        AgentAccess::Never(why) => entry
            .details
            .push(format!("An agent never runs it: {why}.")),
    }
    entry.notes = spec.notes.clone();
    entry.examples = spec
        .examples
        .iter()
        .map(|e| agents::discovery::Example {
            title: e.title.clone(),
            script: e.script.clone(),
        })
        .collect();
    entry.see_also = spec.see_also.clone();
    entry
}

/// A command that a host does not have.
pub fn unknown(id: &str) -> CommandResult {
    Err(CommandError::Unknown(id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::ParamKind;
    use serde_json::json;
    use std::time::Duration;

    /// A host with two commands that records what it was asked.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<(String, CommandArgs)>,
    }

    impl Host for Recorder {
        fn commands(&self) -> Vec<CommandSpec> {
            vec![
                CommandSpec::new("design.pad", "Pad a sketch")
                    .param("sketch", ParamKind::String, "")
                    .optional("length", ParamKind::Number, "")
                    .optional("items", ParamKind::List, ""),
                CommandSpec::new("doc.bodies", "List the bodies"),
            ]
        }

        fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
            self.calls.push((id.to_string(), args.clone()));
            match id {
                "design.pad" => match args.get("length").and_then(|v| v.as_f64()) {
                    Some(l) if l <= 0.0 => Err(CommandError::bad("length", "must be positive")),
                    _ => Ok(json!("pad-1")),
                },
                "doc.bodies" => Ok(json!([{"name": "Body", "id": "b-1", "parent": null}])),
                _ => unknown(id),
            }
        }
    }

    #[test]
    fn a_script_calls_commands_through_the_pc_table() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.run_script(
            r#"local id = pc.design.pad{sketch = "s-1", length = 20}
               print("made", id)"#,
            "test",
            &mut host,
        );
        assert_eq!(out.error, None);
        assert_eq!(out.printed, ["made\tpad-1"]);
        assert_eq!(host.calls[0].0, "design.pad");
        assert_eq!(host.calls[0].1["length"], json!(20));
        assert_eq!(host.calls[0].1["sketch"], json!("s-1"));
    }

    /// `{}` in a script's arguments is the empty list it is written for.
    #[test]
    fn an_empty_table_is_an_empty_list() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.run_script(
            r#"pc.design.pad{sketch = "s-1", items = {}}"#,
            "test",
            &mut host,
        );
        assert_eq!(out.error, None);
        assert_eq!(host.calls[0].1["items"], json!([]));
    }

    #[test]
    fn a_console_expression_shows_its_value_and_globals_persist() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        assert_eq!(engine.eval_line("x = 2", &mut host).value, None);
        assert_eq!(
            engine.eval_line("x * 21", &mut host).value.as_deref(),
            Some("42")
        );
        let bodies = engine.eval_line("pc.doc.bodies()", &mut host);
        assert_eq!(bodies.error, None);
        let shown = bodies.value.unwrap();
        assert!(shown.contains("name = \"Body\""), "{shown}");
        assert!(!shown.contains("parent"), "null becomes nil: {shown}");
    }

    #[test]
    fn a_script_run_shown_answers_what_it_returns_as_text() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.run_script_shown("return {a = 1}", "t", &mut host);
        assert_eq!(out.value.as_deref(), Some("{\n  a = 1\n}"));
        // A script that returns nothing answers nothing, and `show` alone
        // prints nothing.
        let out = engine.run_script_shown("local t = {a = 1}\nshow(t)", "t", &mut host);
        assert_eq!((out.value, out.printed.len()), (None, 0));
    }

    #[test]
    fn a_refused_command_stops_the_script_unless_caught() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.run_script(
            r#"pc.design.pad{sketch = "s", length = -1}
               print("not reached")"#,
            "test",
            &mut host,
        );
        assert_eq!(
            out.error.as_deref(),
            Some("design.pad: argument `length` must be positive")
        );
        assert!(out.printed.is_empty());
        let caught = engine.eval_line(
            r#"select(2, pcall(pc.design.pad, {sketch = "s", length = 0}))"#,
            &mut host,
        );
        assert!(
            caught.value.unwrap().contains("must be positive"),
            "pcall catches it"
        );
        let unknown = engine.eval_line("pc.nothing.here()", &mut host);
        assert_eq!(
            unknown.error.as_deref(),
            Some("nothing.here: no command `nothing.here`")
        );
    }

    #[test]
    fn a_script_reads_the_words_it_was_run_with() {
        let mut engine = ScriptEngine::new();
        engine
            .set_args(&["out.stl".to_string(), "20".to_string()])
            .unwrap();
        let out = engine.eval_line(
            "arg[1] .. ':' .. tonumber(arg[2]) * 2",
            &mut Recorder::default(),
        );
        assert_eq!(out.value.as_deref(), Some("\"out.stl:40\""));
    }

    #[test]
    fn an_empty_array_reaches_a_command_as_a_list() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        engine.eval_line(
            "pc.design.pad{sketch = \"s\", items = array(), more = array(1, 2)}",
            &mut host,
        );
        assert_eq!(host.calls[0].1["items"], json!([]));
        assert_eq!(host.calls[0].1["more"], json!([1, 2]));
    }

    #[test]
    fn help_lists_the_commands() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.eval_line("help('design')", &mut host);
        assert_eq!(
            out.printed,
            ["pc.design.pad{sketch, length?, items?}  Pad a sketch"]
        );
    }

    #[test]
    fn a_script_returns_its_value_as_json() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let mut returned = |source: &str| {
            let out = engine.run_script(source, "test", &mut host);
            assert_eq!(out.error, None, "{source}");
            assert_eq!(out.value, None, "a script shows nothing itself");
            out.returned
        };
        assert_eq!(
            returned(
                r#"local id = pc.design.pad{sketch = "s-1"}
                   return {id = id, sizes = {10, 2.5}, ok = true}"#
            ),
            Some(json!({"id": "pad-1", "sizes": [10, 2.5], "ok": true}))
        );
        assert_eq!(returned("return 6 * 7"), Some(json!(42)));
        assert_eq!(returned("return 'pad-1'"), Some(json!("pad-1")));
        assert_eq!(returned("return 1, 'two'"), Some(json!([1, "two"])));
        assert_eq!(returned("local x = 1"), None);
        assert_eq!(returned("return nil"), None);
        assert_eq!(returned("return {}"), Some(json!([])));
        assert_eq!(
            returned("return {1, 2, x = 3}"),
            Some(json!({"1": 1, "2": 2, "x": 3})),
            "a table with names keeps them"
        );
        // What JSON cannot hold is named rather than lost.
        let function = returned("return print").unwrap();
        assert!(function.as_str().unwrap().starts_with("function"));
        assert_eq!(returned("return 0/0"), Some(json!(null)));
        let looped = returned("local t = {}; t.t = t; return t").unwrap();
        assert!(looped.to_string().contains("table: "), "a loop ends");
    }

    #[test]
    fn a_console_statement_shows_what_it_returns() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.eval_line("local x = 20; return x + 1", &mut host);
        assert_eq!(out.value.as_deref(), Some("21"));
        assert_eq!(out.returned, Some(json!(21)));
    }

    #[test]
    fn help_with_a_word_searches_the_commands() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.eval_line("help('extrude a profile')", &mut host);
        assert_eq!(
            out.printed,
            ["pc.design.pad{sketch, length?, items?}  Pad a sketch"]
        );
        let out = engine.eval_line("help('xyzzy')", &mut host);
        assert_eq!(out.printed, ["Nothing matches \"xyzzy\"."]);
    }

    #[test]
    fn a_runaway_script_is_stopped() {
        let mut engine = ScriptEngine::new();
        engine.set_time_limit(Duration::from_millis(50));
        let out = engine.run_script("while true do end", "loop", &mut Recorder::default());
        assert!(out.error.unwrap().contains("stopped after"));
        // The engine still works afterwards.
        let out = engine.eval_line("1 + 1", &mut Recorder::default());
        assert_eq!(out.value.as_deref(), Some("2"));
    }
}
