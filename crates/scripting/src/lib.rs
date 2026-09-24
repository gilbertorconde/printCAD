//! Lua scripts run against the application's commands.
//!
//! A script reaches every command through the `pc` table: `pc.part.pad{
//! sketch = s, length = 20 }` calls the command `part.pad` with those named
//! arguments and answers what it answers. Lua tables and the commands' JSON
//! values convert both ways; ids are strings. A command that fails raises a
//! Lua error, which `pcall` catches.
//!
//! The engine knows no command itself: a [`Host`] lists them and runs
//! them, so the same scripts run against the application or a test.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

mod record;
mod thread;
pub use record::Recorder;
pub use thread::{Event, Job, ScriptThread};

use core_document::{CommandArgs, CommandError, CommandResult, CommandSpec};
use mlua::serde::SerializeOptions;
use mlua::{HookTriggers, Lua, LuaSerdeExt, MultiValue, VmState};

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
    pub error: Option<String>,
}

/// How long a run may take before it is stopped, unless the caller sets
/// another limit.
const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(10);

/// What a run stopped from outside says.
pub const STOPPED: &str = "stopped";

/// Lua code every run starts with: the `pc` namespace, `print` into the
/// run's output, `show` and `help`.
const PRELUDE: &str = include_str!("prelude.lua");

/// Where printed lines go as they are printed, besides the run's output.
type PrintSink = Rc<RefCell<Option<Box<dyn Fn(&str)>>>>;

pub struct ScriptEngine {
    lua: Lua,
    printed: Rc<RefCell<Vec<String>>>,
    on_print: PrintSink,
    started: Rc<Cell<Instant>>,
    time_limit: Rc<Cell<Duration>>,
    stop: Rc<RefCell<Option<Arc<AtomicBool>>>>,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEngine {
    pub fn new() -> Self {
        let lua = Lua::new();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let started = Rc::new(Cell::new(Instant::now()));
        let time_limit = Rc::new(Cell::new(DEFAULT_TIME_LIMIT));

        let sink = printed.clone();
        let on_print: PrintSink = Rc::new(RefCell::new(None));
        let stream = on_print.clone();
        let print = lua
            .create_function(move |_, line: String| {
                if let Some(f) = stream.borrow().as_ref() {
                    f(&line);
                }
                sink.borrow_mut().push(line);
                Ok(())
            })
            .expect("print function");
        lua.globals()
            .set("__pc_print", print)
            .expect("print global");

        let (since, limit) = (started.clone(), time_limit.clone());
        let stop: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));
        let stopped = stop.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(10_000),
            move |_, _| {
                if stopped
                    .borrow()
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Relaxed))
                {
                    Err(mlua::Error::runtime(STOPPED))
                } else if since.get().elapsed() > limit.get() {
                    Err(mlua::Error::runtime(format!(
                        "stopped after {} s",
                        limit.get().as_secs_f32()
                    )))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .expect("time limit hook");

        lua.load(PRELUDE)
            .set_name("prelude")
            .exec()
            .expect("the prelude runs");
        Self {
            lua,
            printed,
            on_print,
            started,
            time_limit,
            stop,
        }
    }

    /// Hand every printed line to `f` as it is printed, as well as keeping
    /// it for the run's output.
    pub fn on_print(&mut self, f: impl Fn(&str) + 'static) {
        *self.on_print.borrow_mut() = Some(Box::new(f));
    }

    /// Stop the running script once `flag` is set, at its next instruction
    /// check.
    pub fn stop_on(&mut self, flag: Arc<AtomicBool>) {
        *self.stop.borrow_mut() = Some(flag);
    }

    /// Give scripts `arg`, the list of words they were run with, as Lua's
    /// own interpreter does.
    pub fn set_args(&mut self, args: &[String]) {
        if let Ok(table) = self.lua.create_sequence_from(args.iter().cloned()) {
            let _ = self.lua.globals().set("arg", table);
        }
    }

    /// Stop any run that takes longer than `limit`.
    pub fn set_time_limit(&mut self, limit: Duration) {
        self.time_limit.set(limit);
    }

    /// Run one console line. An expression answers its value; anything
    /// else runs as a statement. Globals stay set for the next line.
    pub fn eval_line(&mut self, line: &str, host: &mut dyn Host) -> RunOutput {
        let expression = format!("return {line}");
        if self.lua.load(&expression).into_function().is_ok() {
            self.run(&expression, "console", host, true)
        } else {
            self.run(line, "console", host, false)
        }
    }

    /// Run a whole script; `name` names it in error messages.
    pub fn run_script(&mut self, source: &str, name: &str, host: &mut dyn Host) -> RunOutput {
        self.run(source, name, host, false)
    }

    fn run(&mut self, source: &str, name: &str, host: &mut dyn Host, show: bool) -> RunOutput {
        self.printed.borrow_mut().clear();
        self.started.set(Instant::now());
        let lua = &self.lua;
        let result = lua.scope(|scope| {
            let call =
                scope.create_function_mut(|lua, (id, args): (String, Option<mlua::Value>)| {
                    let args = to_args(lua, args)?;
                    let answer = if id == "app.commands" {
                        Ok(list_commands(host.commands(), &args))
                    } else {
                        host.call(&id, args)
                    };
                    match answer {
                        Ok(value) => lua.to_value_with(
                            &value,
                            SerializeOptions::new()
                                .serialize_none_to_null(false)
                                .serialize_unit_to_null(false),
                        ),
                        Err(error) => Err(mlua::Error::external(ScriptCommandError(format!(
                            "{id}: {error}"
                        )))),
                    }
                })?;
            lua.globals().set("__pc_call", call)?;
            let values: MultiValue = lua.load(source).set_name(name).eval()?;
            if !show || values.iter().all(|v| v.is_nil()) {
                return Ok(None);
            }
            let show: mlua::Function = lua.globals().get("show")?;
            let mut parts = Vec::new();
            for value in values {
                parts.push(show.call::<String>(value)?);
            }
            Ok(Some(parts.join("\t")))
        });
        let _ = self.lua.globals().set("__pc_call", mlua::Nil);
        let printed = std::mem::take(&mut *self.printed.borrow_mut());
        match result {
            Ok(value) => RunOutput {
                printed,
                value,
                error: None,
            },
            Err(error) => RunOutput {
                printed,
                value: None,
                error: Some(error_text(&error)),
            },
        }
    }
}

/// A command's refusal, carried through Lua's error unchanged.
#[derive(Debug)]
struct ScriptCommandError(String);

impl std::fmt::Display for ScriptCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScriptCommandError {}

/// The message of an error, without the traceback Lua appends.
fn error_text(error: &mlua::Error) -> String {
    let text = match error {
        mlua::Error::CallbackError { cause, .. } => return error_text(cause),
        mlua::Error::ExternalError(inner) => inner.to_string(),
        other => other.to_string(),
    };
    text.split("\nstack traceback:")
        .next()
        .unwrap_or("")
        .to_string()
}

/// A Lua argument table as named arguments: nothing, or a table of names.
fn to_args(lua: &Lua, args: Option<mlua::Value>) -> mlua::Result<CommandArgs> {
    let Some(args) = args else {
        return Ok(CommandArgs::new());
    };
    if args.is_nil() {
        return Ok(CommandArgs::new());
    }
    match lua.from_value::<serde_json::Value>(args)? {
        serde_json::Value::Object(map) => Ok(map),
        serde_json::Value::Array(list) if list.is_empty() => Ok(CommandArgs::new()),
        _ => Err(mlua::Error::runtime(
            "a command takes a table of named arguments, like {length = 10}",
        )),
    }
}

/// `app.commands`: every command whose id starts with `prefix`.
fn list_commands(commands: Vec<CommandSpec>, args: &CommandArgs) -> serde_json::Value {
    let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
    serde_json::Value::Array(
        commands
            .iter()
            .filter(|c| c.id.starts_with(prefix))
            .map(CommandSpec::to_json)
            .collect(),
    )
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

    /// A host with two commands that records what it was asked.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<(String, CommandArgs)>,
    }

    impl Host for Recorder {
        fn commands(&self) -> Vec<CommandSpec> {
            vec![
                CommandSpec::new("part.pad", "Pad a sketch")
                    .param("sketch", ParamKind::String, "")
                    .optional("length", ParamKind::Number, ""),
                CommandSpec::new("doc.bodies", "List the bodies"),
            ]
        }

        fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
            self.calls.push((id.to_string(), args.clone()));
            match id {
                "part.pad" => match args.get("length").and_then(|v| v.as_f64()) {
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
            r#"local id = pc.part.pad{sketch = "s-1", length = 20}
               print("made", id)"#,
            "test",
            &mut host,
        );
        assert_eq!(out.error, None);
        assert_eq!(out.printed, ["made\tpad-1"]);
        assert_eq!(host.calls[0].0, "part.pad");
        assert_eq!(host.calls[0].1["length"], json!(20));
        assert_eq!(host.calls[0].1["sketch"], json!("s-1"));
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
    fn a_refused_command_stops_the_script_unless_caught() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.run_script(
            r#"pc.part.pad{sketch = "s", length = -1}
               print("not reached")"#,
            "test",
            &mut host,
        );
        assert_eq!(
            out.error.as_deref(),
            Some("part.pad: argument `length` must be positive")
        );
        assert!(out.printed.is_empty());
        let caught = engine.eval_line(
            r#"select(2, pcall(pc.part.pad, {sketch = "s", length = 0}))"#,
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
        engine.set_args(&["out.stl".to_string(), "20".to_string()]);
        let out = engine.eval_line(
            "arg[1] .. ':' .. tonumber(arg[2]) * 2",
            &mut Recorder::default(),
        );
        assert_eq!(out.value.as_deref(), Some("\"out.stl:40\""));
    }

    #[test]
    fn help_lists_the_commands() {
        let mut engine = ScriptEngine::new();
        let mut host = Recorder::default();
        let out = engine.eval_line("help('part')", &mut host);
        assert_eq!(out.printed, ["pc.part.pad{sketch, length?}  Pad a sketch"]);
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
