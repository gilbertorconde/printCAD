//! The Lua engine: the `pc` namespace over a [`Host`]'s commands.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use core_document::CommandArgs;
use mlua::serde::SerializeOptions;
use mlua::{HookTriggers, Lua, LuaOptions, LuaSerdeExt, MultiValue, StdLib, VmState};

use crate::{Host, RunOutput, STOPPED, list_commands, search_commands};

/// How long a run may take before it is stopped, unless the caller sets
/// another limit.
const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(10);

/// How much memory an engine's Lua state may hold, unless the caller sets
/// another limit.
pub const DEFAULT_MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// Base functions a sandboxed engine removes: they read files (`dofile`,
/// `loadfile`) or compile whatever chunk a script hands them, binary ones
/// included (`load`). The console and the runner compile what they run
/// from the Rust side, so no script needs them.
const SANDBOX_REMOVED: [&str; 3] = ["dofile", "loadfile", "load"];

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
    memory_limit: usize,
    stop: Rc<RefCell<Option<Arc<AtomicBool>>>>,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEngine {
    /// An engine with every library Lua counts as safe, `io` and `os`
    /// included: for scripts the user runs.
    pub fn new() -> Self {
        Self::with(Lua::new())
    }

    /// An engine for scripts the user has not read, such as an agent's:
    /// the base functions without those that load code or files, and the
    /// `table`, `string`, `math`, `utf8` and `coroutine` libraries; no
    /// `io`, `os`, `package` or `debug`. A script reaches files only
    /// through commands, which check what they touch.
    pub fn sandboxed() -> Self {
        let libraries =
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
        let lua =
            Lua::new_with(libraries, LuaOptions::default()).expect("the sandboxed libraries load");
        for name in SANDBOX_REMOVED {
            lua.globals()
                .raw_set(name, mlua::Nil)
                .expect("a base function is removed");
        }
        Self::with(lua)
    }

    fn with(lua: Lua) -> Self {
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
        // `array(...)`: a table marked as a list, so it converts to a list
        // even when it is empty.
        let array = lua
            .create_function(|lua, values: mlua::Variadic<mlua::Value>| {
                let list = lua.create_sequence_from(values)?;
                list.set_metatable(Some(lua.array_metatable()))?;
                Ok(list)
            })
            .expect("array function");
        lua.globals().set("array", array).expect("array global");
        // `__pc_json(value)`: a value as JSON text, as a run returns it.
        let to_json = lua
            .create_function(|lua, value: mlua::Value| Ok(lua_json(lua, &value, 0).to_string()))
            .expect("json function");
        lua.globals()
            .set("__pc_json", to_json)
            .expect("json global");

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
                        "stopped after {} s, the longest a script may run here",
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
        lua.set_memory_limit(DEFAULT_MEMORY_LIMIT)
            .expect("a memory limit is set");
        Self {
            lua,
            printed,
            on_print,
            started,
            time_limit,
            memory_limit: DEFAULT_MEMORY_LIMIT,
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
    pub fn set_args(&mut self, args: &[String]) -> Result<(), String> {
        let table = self
            .lua
            .create_sequence_from(args.iter().cloned())
            .map_err(|e| e.to_string())?;
        self.lua
            .globals()
            .set("arg", table)
            .map_err(|e| e.to_string())
    }

    /// Stop any run whose Lua runs longer than `limit`. Time spent waiting
    /// for a command's answer does not count.
    pub fn set_time_limit(&mut self, limit: Duration) {
        self.time_limit.set(limit);
    }

    /// Let the Lua state hold at most `bytes`; a run that needs more stops
    /// with an error saying so.
    pub fn set_memory_limit(&mut self, bytes: usize) -> Result<(), String> {
        self.lua
            .set_memory_limit(bytes)
            .map_err(|e| e.to_string())?;
        self.memory_limit = bytes;
        Ok(())
    }

    /// Run one console line. An expression answers its value; anything
    /// else runs as a statement, answering what it `return`s. Globals stay
    /// set for the next line. The expression is handed to a function
    /// rather than returned as it is, so a call in it is no tail call and
    /// an error it raises keeps the line it was raised on.
    pub fn eval_line(&mut self, line: &str, host: &mut dyn Host) -> RunOutput {
        let expression = format!("return (function(...) return ... end)({line}\n)");
        if self.lua.load(&expression).into_function().is_ok() {
            self.run(&expression, "console", host, true)
        } else {
            self.run(line, "console", host, true)
        }
    }

    /// Run a whole script; `name` names it in error messages.
    pub fn run_script(&mut self, source: &str, name: &str, host: &mut dyn Host) -> RunOutput {
        self.run(source, name, host, false)
    }

    /// Run a whole script, what it returns answered as `value` too, shown
    /// as text the way a console line's value is.
    pub fn run_script_shown(&mut self, source: &str, name: &str, host: &mut dyn Host) -> RunOutput {
        self.run(source, name, host, true)
    }

    fn run(&mut self, source: &str, name: &str, host: &mut dyn Host, show: bool) -> RunOutput {
        self.printed.borrow_mut().clear();
        self.started.set(Instant::now());
        let lua = &self.lua;
        let started = &self.started;
        let result = lua.scope(|scope| {
            let call =
                scope.create_function_mut(|lua, (id, args): (String, Option<mlua::Value>)| {
                    let args = to_args(lua, args)?;
                    let asked = Instant::now();
                    let answer = if id == "app.commands" {
                        Ok(list_commands(host.commands(), &args))
                    } else if id == "app.search" {
                        Ok(search_commands(host.commands(), &args))
                    } else {
                        host.call(&id, args)
                    };
                    // The time limit is on the script's own running, not
                    // on the commands it waits for.
                    started.set(started.get() + asked.elapsed());
                    match answer {
                        Ok(mut value) => {
                            wide_integers_as_text(&mut value);
                            lua.to_value_with(
                                &value,
                                SerializeOptions::new()
                                    .serialize_none_to_null(false)
                                    .serialize_unit_to_null(false),
                            )
                        }
                        Err(error) => Err(mlua::Error::external(ScriptCommandError(format!(
                            "{id}: {error}"
                        )))),
                    }
                })?;
            lua.globals().set("__pc_call", call)?;
            let values: MultiValue = lua.load(source).set_name(name).eval()?;
            if values.iter().all(|v| v.is_nil()) {
                return Ok((None, None));
            }
            let returned = Some(returned_json(lua, &values));
            if !show {
                return Ok((None, returned));
            }
            let show: mlua::Function = lua.globals().get("show")?;
            let mut parts = Vec::new();
            for value in values {
                parts.push(show.call::<String>(value)?);
            }
            Ok((Some(parts.join("\t")), returned))
        });
        let _ = self.lua.globals().set("__pc_call", mlua::Nil);
        let printed = std::mem::take(&mut *self.printed.borrow_mut());
        match result {
            Ok((value, returned)) => RunOutput {
                printed,
                value,
                returned,
                error: None,
            },
            Err(error) if out_of_memory(&error) => RunOutput {
                printed,
                error: Some(format!(
                    "out of memory: a script may use at most {} MB",
                    self.memory_limit / (1024 * 1024)
                )),
                ..RunOutput::default()
            },
            Err(error) => RunOutput {
                printed,
                error: Some(error_text(&error)),
                ..RunOutput::default()
            },
        }
    }
}

/// Whether `error` is the Lua state running past its memory limit.
fn out_of_memory(error: &mlua::Error) -> bool {
    match error {
        mlua::Error::MemoryError(_) => true,
        mlua::Error::CallbackError { cause, .. } => out_of_memory(cause),
        _ => false,
    }
}

/// The values a chunk returned as JSON: one value as itself, several as a
/// list. Ids are strings in Lua and stay strings.
fn returned_json(lua: &Lua, values: &MultiValue) -> serde_json::Value {
    match values.len() {
        1 => lua_json(lua, &values[0], 0),
        _ => serde_json::Value::Array(values.iter().map(|v| lua_json(lua, v, 0)).collect()),
    }
}

/// How deep a returned table may nest before the rest is written as text.
const RETURN_DEPTH: usize = 32;

/// A Lua value as JSON. A table whose keys are 1 to n (or one made with
/// `array`) is a list, any other an object, its keys written as text.
/// What JSON cannot hold (a function, a number that is not finite, a table
/// nested past [`RETURN_DEPTH`], as one that holds itself is) is written
/// as Lua's `tostring` of it, or null for a number.
fn lua_json(lua: &Lua, value: &mlua::Value, depth: usize) -> serde_json::Value {
    use serde_json::Value as J;
    match value {
        mlua::Value::Nil => J::Null,
        mlua::Value::LightUserData(u) if u.0.is_null() => J::Null,
        mlua::Value::Boolean(b) => J::Bool(*b),
        mlua::Value::Integer(i) => J::from(*i),
        mlua::Value::Number(n) => serde_json::Number::from_f64(*n).map_or(J::Null, J::Number),
        mlua::Value::String(s) => J::String(s.to_string_lossy()),
        mlua::Value::Table(table) if depth < RETURN_DEPTH => {
            let len = table.raw_len();
            let pairs: Vec<(mlua::Value, mlua::Value)> =
                table.pairs().filter_map(Result::ok).collect();
            let listed = table
                .metatable()
                .is_some_and(|m| m == lua.array_metatable());
            if listed || (len > 0 && pairs.len() == len) {
                J::Array(
                    (1..=len)
                        .map(|i| {
                            lua_json(
                                lua,
                                &table.raw_get(i).unwrap_or(mlua::Value::Nil),
                                depth + 1,
                            )
                        })
                        .collect(),
                )
            } else if pairs.is_empty() {
                J::Array(Vec::new())
            } else {
                J::Object(
                    pairs
                        .iter()
                        .map(|(k, v)| {
                            let key = match k {
                                mlua::Value::String(s) => s.to_string_lossy(),
                                other => other.to_string().unwrap_or_default(),
                            };
                            (key, lua_json(lua, v, depth + 1))
                        })
                        .collect(),
                )
            }
        }
        other => J::String(other.to_string().unwrap_or_default()),
    }
}

/// `value` with every whole number past what a Lua integer holds (above
/// 2^63 - 1) written as a string of its digits: as a number it would
/// become the nearest float, its last digits lost. Commands that take
/// such numbers (face names) take the string back.
fn wide_integers_as_text(value: &mut serde_json::Value) {
    use serde_json::Value as J;
    match value {
        J::Number(n) if n.is_u64() && n.as_i64().is_none() => {
            *value = J::String(n.to_string());
        }
        J::Array(items) => items.iter_mut().for_each(wide_integers_as_text),
        J::Object(map) => map.values_mut().for_each(wide_integers_as_text),
        _ => {}
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

/// The message of an error, without the traceback Lua appends or the
/// `runtime error: ` its display starts with.
fn error_text(error: &mlua::Error) -> String {
    let text = match error {
        mlua::Error::CallbackError { cause, .. } => return error_text(cause),
        mlua::Error::ExternalError(inner) => inner.to_string(),
        mlua::Error::RuntimeError(message) => message.clone(),
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
        serde_json::Value::Object(mut map) => {
            map.values_mut().for_each(empty_tables_as_lists);
            Ok(map)
        }
        serde_json::Value::Array(list) if list.is_empty() => Ok(CommandArgs::new()),
        _ => Err(mlua::Error::runtime(
            "a command takes a table of named arguments, like {length = 10}",
        )),
    }
}

/// An empty Lua table cannot say whether it is a list or a table of
/// names; inside a command's arguments it is taken as the empty list, the
/// one a script means when it writes `{}` for a list of items. A command
/// reading a table of names takes an empty list as none given.
fn empty_tables_as_lists(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) if map.is_empty() => {
            *value = serde_json::Value::Array(Vec::new());
        }
        serde_json::Value::Object(map) => map.values_mut().for_each(empty_tables_as_lists),
        serde_json::Value::Array(items) => items.iter_mut().for_each(empty_tables_as_lists),
        _ => {}
    }
}
