//! Commands: everything the application and the workbenches can be asked
//! to do by name, with typed arguments, for scripts and other callers that
//! are not a click.
//!
//! A workbench registers the commands it answers with
//! [`crate::WorkbenchContext::register_command`] and runs them in
//! [`crate::Workbench::run_command`]; the application registers its own the
//! same way. Arguments and results are JSON values, so any caller that
//! speaks JSON (a script, an agent) reaches every command the same way.
//! A command changes the document through its ordinary mutators, so each
//! change is an operation like any other: undone, saved and replicated.

mod prose;

pub use prose::CommandProse;

use crate::feature::BodyId;
use serde_json::{Map, Value};
use uuid::Uuid;

/// Named arguments to a command.
pub type CommandArgs = Map<String, Value>;

/// What a command answers: a value, or why it did not run.
pub type CommandResult = Result<Value, CommandError>;

/// The kind of value an argument takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Number,
    Integer,
    Bool,
    String,
    /// The id of a body or a feature, as a string.
    Id,
    List,
    /// Any value; the command reads it itself.
    Any,
}

impl ParamKind {
    pub fn name(self) -> &'static str {
        match self {
            ParamKind::Number => "number",
            ParamKind::Integer => "integer",
            ParamKind::Bool => "boolean",
            ParamKind::String => "string",
            ParamKind::Id => "id",
            ParamKind::List => "list",
            ParamKind::Any => "any",
        }
    }

    fn accepts(self, value: &Value) -> bool {
        match self {
            ParamKind::Number => value.is_number(),
            ParamKind::Integer => value.is_i64() || value.is_u64(),
            ParamKind::Bool => value.is_boolean(),
            ParamKind::String => value.is_string(),
            ParamKind::Id => value.as_str().is_some_and(|s| Uuid::parse_str(s).is_ok()),
            ParamKind::List => value.is_array(),
            ParamKind::Any => true,
        }
    }
}

/// One named argument of a command.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    pub kind: ParamKind,
    pub required: bool,
    pub doc: String,
}

/// A command: its id, what it does, what it takes and what it answers.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandSpec {
    /// Unique across the application, dotted like `design.pad`.
    pub id: String,
    /// One line on what it does.
    pub summary: String,
    pub params: Vec<ParamSpec>,
    /// What it answers, in words.
    pub returns: String,
    /// It takes named arguments beyond `params` (such as any field of a
    /// feature), described here.
    pub extra_args: Option<String>,
    /// It only reads: it changes neither the document nor the files, so an
    /// agent may run it without asking.
    pub read_only: bool,
    /// What an AI agent may do with it, whatever the user allowed.
    pub agent: AgentAccess,
    /// What a caller must know beyond the arguments: what it refuses, does
    /// silently, or is easily mistaken for; one sentence or two each.
    pub notes: Vec<String>,
    /// Working uses, each a Lua script run from an empty document; the
    /// test suite runs every one.
    pub examples: Vec<Example>,
    /// Other commands that do the related thing, by id.
    pub see_also: Vec<String>,
}

/// A working use of a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    /// What it shows, one line.
    pub title: String,
    /// A Lua script, run as a script file is, from an empty document; it
    /// ends in `assert`s on what it made.
    pub script: String,
}

/// What an AI agent may do with a command: the agent runs inside the
/// user's session, so some commands would pull the ground from under it or
/// reach past the document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AgentAccess {
    /// As the user's approval setting says.
    #[default]
    AsAllowed,
    /// Only once the user allows it, even where changes run without
    /// asking: it reaches past the document (files, other programs, the
    /// undo history).
    AlwaysAsk,
    /// Never: the reason, which the agent is told.
    Never(String),
}

/// `value` with every empty list in it an empty table instead. A script's
/// `{}` reaches a command as an empty list, since it cannot say which it
/// is; a command reading a table of names where it found an empty list
/// reads it again this way (`params = {}`, `{Angled = {}}`).
pub fn empty_lists_as_tables(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) if items.is_empty() => {
            *value = serde_json::Value::Object(Default::default());
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(empty_lists_as_tables),
        serde_json::Value::Object(map) => map.values_mut().for_each(empty_lists_as_tables),
        _ => {}
    }
}

impl CommandSpec {
    pub fn new(id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            params: Vec::new(),
            returns: "nothing".to_string(),
            extra_args: None,
            read_only: false,
            agent: AgentAccess::AsAllowed,
            notes: Vec::new(),
            examples: Vec::new(),
            see_also: Vec::new(),
        }
    }

    /// Something a caller must know beyond the arguments.
    pub fn note(mut self, note: &str) -> Self {
        self.notes.push(note.to_string());
        self
    }

    /// A working use: `script` is Lua, run from an empty document, ending
    /// in `assert`s on what it made. The indentation its lines share and
    /// the blank lines around it are taken off.
    pub fn example(mut self, title: &str, script: &str) -> Self {
        self.examples.push(Example {
            title: title.to_string(),
            script: dedent(script),
        });
        self
    }

    /// Another command that does the related thing.
    pub fn see_also(mut self, id: &str) -> Self {
        self.see_also.push(id.to_string());
        self
    }

    /// Say that it only reads.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// An agent runs it only once the user allows it, every time.
    pub fn agent_always_asks(mut self) -> Self {
        self.agent = AgentAccess::AlwaysAsk;
        self
    }

    /// An agent never runs it, for `reason`.
    pub fn agent_never(mut self, reason: &str) -> Self {
        self.agent = AgentAccess::Never(reason.to_string());
        self
    }

    /// A required argument.
    pub fn param(mut self, name: &str, kind: ParamKind, doc: &str) -> Self {
        self.params.push(ParamSpec {
            name: name.to_string(),
            kind,
            required: true,
            doc: doc.to_string(),
        });
        self
    }

    /// An argument that may be left out.
    pub fn optional(mut self, name: &str, kind: ParamKind, doc: &str) -> Self {
        self.params.push(ParamSpec {
            name: name.to_string(),
            kind,
            required: false,
            doc: doc.to_string(),
        });
        self
    }

    pub fn returns(mut self, doc: &str) -> Self {
        self.returns = doc.to_string();
        self
    }

    /// Accept named arguments beyond the declared ones.
    pub fn extra_args(mut self, doc: &str) -> Self {
        self.extra_args = Some(doc.to_string());
        self
    }

    /// Check `args` against the declared arguments: every required one
    /// present, each of the right kind, and no unknown names unless the
    /// command takes extra ones.
    pub fn check(&self, args: &CommandArgs) -> Result<(), CommandError> {
        for param in &self.params {
            match args.get(&param.name) {
                None | Some(Value::Null) if param.required => {
                    // An argument given under a slip of this one's name.
                    let slip = args
                        .keys()
                        .filter(|k| !self.params.iter().any(|p| &p.name == *k))
                        .find(|k| nearest(k, [param.name.as_str()]).is_some());
                    return Err(CommandError::bad(
                        &param.name,
                        match slip {
                            Some(slip) => format!("is required (`{slip}` was given)"),
                            None => "is required".to_string(),
                        },
                    ));
                }
                Some(value) if !value.is_null() && !param.kind.accepts(value) => {
                    return Err(CommandError::bad(
                        &param.name,
                        format!("must be a {}", param.kind.name()),
                    ));
                }
                _ => {}
            }
        }
        if self.extra_args.is_none()
            && let Some(unknown) = args
                .keys()
                .find(|k| !self.params.iter().any(|p| &p.name == *k))
        {
            let message = match nearest(unknown, self.params.iter().map(|p| p.name.as_str())) {
                Some(near) => format!("is not an argument of this command; did you mean `{near}`?"),
                None => "is not an argument of this command".to_string(),
            };
            return Err(CommandError::bad(unknown, message));
        }
        Ok(())
    }

    /// The command as a JSON object: what a caller lists or shows as help.
    pub fn to_json(&self) -> Value {
        let params: Vec<Value> = self
            .params
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "kind": p.kind.name(),
                    "required": p.required,
                    "doc": p.doc,
                })
            })
            .collect();
        let mut out = serde_json::json!({
            "id": self.id,
            "summary": self.summary,
            "params": params,
            "returns": self.returns,
            "read_only": self.read_only,
        });
        match &self.agent {
            AgentAccess::AsAllowed => {}
            AgentAccess::AlwaysAsk => out["agent"] = Value::String("always asks".into()),
            AgentAccess::Never(why) => out["agent"] = Value::String(format!("never: {why}")),
        }
        if let Some(extra) = &self.extra_args {
            out["extra_args"] = Value::String(extra.clone());
        }
        if !self.notes.is_empty() {
            out["notes"] = serde_json::json!(self.notes);
        }
        if !self.examples.is_empty() {
            out["examples"] = self
                .examples
                .iter()
                .map(|e| serde_json::json!({"title": e.title, "script": e.script}))
                .collect();
        }
        if !self.see_also.is_empty() {
            out["see_also"] = serde_json::json!(self.see_also);
        }
        out
    }
}

/// `text` without the blank lines around it and the indentation its other
/// lines share.
fn dedent(text: &str) -> String {
    let lines: Vec<&str> = text.trim_matches('\n').lines().collect();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out: Vec<&str> = lines
        .iter()
        .map(|l| l.get(indent..).unwrap_or("").trim_end())
        .collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n") + "\n"
}

/// A command the user ran through the UI rather than a script: what it
/// was called with and what it answered. A recording is a list of these.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub id: String,
    pub args: CommandArgs,
    pub result: Value,
}

/// A JSON object as named arguments; anything else as none.
pub fn object(value: Value) -> CommandArgs {
    match value {
        Value::Object(map) => map,
        _ => CommandArgs::new(),
    }
}

/// What runs a command: its checked arguments and the context a bench's
/// tools get.
pub type CommandFn =
    fn(&CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult;

/// What runs a command that several specs share: told which one it is.
pub type CommandFnById =
    fn(&str, &CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult;

/// What runs a command that needs the bench's own state `B`.
pub type BenchFn<B> =
    fn(&mut B, &CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult;

/// What runs a command that needs the bench's own state, shared by
/// several specs: told which one it is.
pub type BenchFnById<B> =
    fn(&mut B, &str, &CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult;

type Free = Box<
    dyn Fn(&CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult
        + Send
        + Sync,
>;

type OnBench<B> = Box<
    dyn Fn(&mut B, &CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult
        + Send
        + Sync,
>;

/// What runs one command: on the context alone, or on the bench's state
/// too.
enum Handler<B> {
    Free(Free),
    OnBench(OnBench<B>),
}

/// A bench's commands, each spec beside the function that runs it, so a
/// command's id is written once, in its spec: registration, which ids
/// the bench answers and dispatch all read it from here.
///
/// `B` is the bench state a handler may take (`&mut B`), handed in by
/// [`run_on`](Self::run_on): the bench itself for a bench whose commands
/// read its settings or caches, `()` for one whose commands need only the
/// context. Code that holds the context but not the bench (a panel, a
/// tool) runs the commands needing no bench state through
/// [`run`](Self::run). A family adds its commands with a `table(t)`
/// function of its own; a bench builds one table from its families and
/// keeps it in a `OnceLock`. A step several commands share (load a
/// feature, edit it, store it) is a function making the handler, added
/// through [`add_free`](Self::add_free) or [`add_with`](Self::add_with).
pub struct CommandTable<B = ()> {
    entries: Vec<(CommandSpec, Handler<B>)>,
}

impl<B> Default for CommandTable<B> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<B: 'static> CommandTable<B> {
    /// Add `spec`, run by `run`.
    pub fn add(&mut self, spec: CommandSpec, run: CommandFn) {
        self.add_free(spec, run);
    }

    /// Add `spec`, run by `run` told the command's id: for specs made in
    /// a loop over a table of their own.
    pub fn add_by_id(&mut self, spec: CommandSpec, run: CommandFnById) {
        let id = spec.id.clone();
        self.add_free(spec, move |args, ctx| run(&id, args, ctx));
    }

    /// Add `spec`, run by `run`, a handler needing no bench state made by
    /// a wrapper that does what several commands share around their own
    /// step.
    pub fn add_free(
        &mut self,
        spec: CommandSpec,
        run: impl Fn(&CommandArgs, &mut crate::runtime::WorkbenchRuntimeContext) -> CommandResult
        + Send
        + Sync
        + 'static,
    ) {
        self.entries.push((spec, Handler::Free(Box::new(run))));
    }

    /// Add `spec`, run by `run` on the bench's state.
    pub fn add_on(&mut self, spec: CommandSpec, run: BenchFn<B>) {
        self.add_with(spec, run);
    }

    /// Add `spec`, run by `run` on the bench's state told the command's
    /// id.
    pub fn add_on_by_id(&mut self, spec: CommandSpec, run: BenchFnById<B>) {
        let id = spec.id.clone();
        self.add_with(spec, move |bench, args, ctx| run(bench, &id, args, ctx));
    }

    /// Add `spec`, run by `run` on the bench's state: a handler made by a
    /// wrapper.
    pub fn add_with(
        &mut self,
        spec: CommandSpec,
        run: impl Fn(
            &mut B,
            &CommandArgs,
            &mut crate::runtime::WorkbenchRuntimeContext,
        ) -> CommandResult
        + Send
        + Sync
        + 'static,
    ) {
        self.entries.push((spec, Handler::OnBench(Box::new(run))));
    }

    /// Add every command of `family`, a table of its own kept for code
    /// that asks whether an id is one of that family; each runs through
    /// it, needing no bench state.
    pub fn include(&mut self, family: &'static CommandTable) {
        for spec in family.specs() {
            let id = spec.id.clone();
            self.add_free(spec.clone(), move |args, ctx| family.run(&id, args, ctx));
        }
    }

    /// Add `prose`'s notes, related commands and examples to the specs
    /// it names.
    pub fn explain(&mut self, prose: &CommandProse) {
        for (spec, _) in &mut self.entries {
            prose.add_to(spec);
        }
    }

    /// The specs in the order they are held.
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.entries.iter().map(|(spec, _)| spec)
    }

    /// Order the commands as `order` lists them, any it does not list
    /// first in the order they were added.
    pub fn sort_by_order(&mut self, order: &[&str]) {
        self.entries.sort_by_key(|(spec, _)| {
            order
                .iter()
                .position(|id| *id == spec.id)
                .map_or(0, |i| i + 1)
        });
    }

    /// Register every spec with `context`, in the order held.
    pub fn register(&self, context: &mut crate::workbench::WorkbenchContext) {
        for spec in self.specs() {
            context.register_command(spec.clone());
        }
    }

    /// Whether `id` is one of these commands.
    pub fn owns(&self, id: &str) -> bool {
        self.entries.iter().any(|(spec, _)| spec.id == id)
    }

    /// Run command `id` on `bench`, refused as unknown when it is not one
    /// of these.
    pub fn run_on(
        &self,
        bench: &mut B,
        id: &str,
        args: &CommandArgs,
        ctx: &mut crate::runtime::WorkbenchRuntimeContext,
    ) -> CommandResult {
        match self.entries.iter().find(|(spec, _)| spec.id == id) {
            Some((_, Handler::Free(run))) => run(args, ctx),
            Some((_, Handler::OnBench(run))) => run(bench, args, ctx),
            None => Err(CommandError::Unknown(id.to_string())),
        }
    }

    /// Run command `id` without the bench's state: refused as unknown
    /// when it is not one of these, and when it needs that state.
    pub fn run(
        &self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut crate::runtime::WorkbenchRuntimeContext,
    ) -> CommandResult {
        match self.entries.iter().find(|(spec, _)| spec.id == id) {
            Some((_, Handler::Free(run))) => run(args, ctx),
            Some((_, Handler::OnBench(_))) => Err(CommandError::failed(format!(
                "`{id}` runs only on its bench"
            ))),
            None => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

/// Why a command did not run.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("no command `{0}`")]
    Unknown(String),
    #[error("argument `{name}` {message}")]
    BadArgument { name: String, message: String },
    #[error("{0}")]
    Failed(String),
}

/// The one of `names` a mistyped `name` most likely meant: within two
/// edits of it (fewer for a short name), or, for a name without a dot, the
/// command whose last part it is (`pad` for `design.pad`).
pub fn nearest<'a>(name: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let reach = (name.chars().count() / 3).clamp(1, 2);
    let mut best: Option<(usize, &str)> = None;
    for candidate in names {
        let last_part = !name.contains('.')
            && candidate
                .rsplit_once('.')
                .is_some_and(|(_, last)| last == name);
        let distance = if last_part {
            0
        } else {
            edit_distance(name, candidate)
        };
        if distance <= reach && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// Levenshtein distance, by characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

impl CommandError {
    /// No command `id` among `ids`: naming the one it most likely meant.
    pub fn unknown_among<'a>(id: &str, ids: impl IntoIterator<Item = &'a str>) -> Self {
        match nearest(id, ids) {
            Some(near) => Self::Failed(format!("no command `{id}`; did you mean `{near}`?")),
            None => Self::Unknown(id.to_string()),
        }
    }

    pub fn bad(name: &str, message: impl Into<String>) -> Self {
        Self::BadArgument {
            name: name.to_string(),
            message: message.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

/// The numbers `value` holds under `keys`: an object with those keys
/// (`{x, y, z}`) or a list holding them in order (`[x, y, z]`), each a
/// finite number. `name` is the argument the error names.
pub fn numbers_of<const N: usize>(
    value: Option<&Value>,
    name: &str,
    keys: [&str; N],
) -> Result<[f64; N], CommandError> {
    let bad = || {
        let keys = keys.join(", ");
        CommandError::bad(name, format!("must be {{{keys}}} or [{keys}]"))
    };
    let value = value.ok_or_else(bad)?;
    let mut out = [0.0; N];
    for (i, key) in keys.iter().enumerate() {
        out[i] = value
            .get(*key)
            .or_else(|| value.get(i))
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .ok_or_else(bad)?;
    }
    Ok(out)
}

/// A point or vector: `{x, y, z}` or `[x, y, z]`.
pub fn vec3_of(value: Option<&Value>, name: &str) -> Result<[f64; 3], CommandError> {
    numbers_of(value, name, ["x", "y", "z"])
}

/// A point in a plane: `{x, y}` or `[x, y]`.
pub fn vec2_of(value: Option<&Value>, name: &str) -> Result<[f64; 2], CommandError> {
    numbers_of(value, name, ["x", "y"])
}

/// What a body argument naming no body of the document is told.
pub const NOT_A_BODY: &str = "is not a body of this document";

/// Typed reads of a command's arguments, for the command's own code.
pub struct Args<'a>(pub &'a CommandArgs);

impl Args<'_> {
    pub fn has(&self, name: &str) -> bool {
        self.0.get(name).is_some_and(|v| !v.is_null())
    }

    pub fn number(&self, name: &str) -> Result<f64, CommandError> {
        self.opt_number(name)?
            .ok_or_else(|| CommandError::bad(name, "is required"))
    }

    pub fn opt_number(&self, name: &str) -> Result<Option<f64>, CommandError> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .map(Some)
                .ok_or_else(|| CommandError::bad(name, "must be a number")),
        }
    }

    pub fn string(&self, name: &str) -> Result<&str, CommandError> {
        self.opt_string(name)?
            .ok_or_else(|| CommandError::bad(name, "is required"))
    }

    pub fn opt_string(&self, name: &str) -> Result<Option<&str>, CommandError> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_str()
                .map(Some)
                .ok_or_else(|| CommandError::bad(name, "must be a string")),
        }
    }

    pub fn opt_bool(&self, name: &str) -> Result<Option<bool>, CommandError> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_bool()
                .map(Some)
                .ok_or_else(|| CommandError::bad(name, "must be a boolean")),
        }
    }

    pub fn id(&self, name: &str) -> Result<Uuid, CommandError> {
        self.opt_id(name)?
            .ok_or_else(|| CommandError::bad(name, "is required"))
    }

    pub fn opt_id(&self, name: &str) -> Result<Option<Uuid>, CommandError> {
        match self.opt_string(name)? {
            None => Ok(None),
            Some(s) => Uuid::parse_str(s)
                .map(Some)
                .map_err(|_| CommandError::bad(name, "must be an id")),
        }
    }

    /// The point or vector `name` holds: `{x, y, z}` or `[x, y, z]`.
    pub fn vec3(&self, name: &str) -> Result<[f64; 3], CommandError> {
        self.opt_vec3(name)?
            .ok_or_else(|| CommandError::bad(name, "is required"))
    }

    /// The point or vector `name` holds, when given.
    pub fn opt_vec3(&self, name: &str) -> Result<Option<[f64; 3]>, CommandError> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            value => vec3_of(value, name).map(Some),
        }
    }

    /// The point in a plane `name` holds: `{x, y}` or `[x, y]`.
    pub fn vec2(&self, name: &str) -> Result<[f64; 2], CommandError> {
        match self.0.get(name) {
            None | Some(Value::Null) => Err(CommandError::bad(name, "is required")),
            value => vec2_of(value, name),
        }
    }

    /// The body `name` names, which must be one of `document`'s.
    pub fn body(&self, document: &crate::Document, name: &str) -> Result<BodyId, CommandError> {
        self.opt_body(document, name)?
            .ok_or_else(|| CommandError::bad(name, "is required"))
    }

    /// The body `name` names, when given, which must be one of `document`'s.
    pub fn opt_body(
        &self,
        document: &crate::Document,
        name: &str,
    ) -> Result<Option<BodyId>, CommandError> {
        match self.opt_id(name)?.map(BodyId) {
            Some(body) if document.body(body).is_none() => Err(CommandError::bad(name, NOT_A_BODY)),
            body => Ok(body),
        }
    }

    /// The body `name` names by its own id or by the id of a feature in it.
    pub fn body_or_feature_body(
        &self,
        document: &crate::Document,
        name: &str,
    ) -> Result<BodyId, CommandError> {
        let id = self.id(name)?;
        if document.body(BodyId(id)).is_some() {
            return Ok(BodyId(id));
        }
        document
            .get_feature_meta(crate::FeatureId(id))
            .and_then(|n| n.body)
            .ok_or_else(|| CommandError::bad(name, "is neither a body nor a feature in one"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: Value) -> CommandArgs {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_point_reads_as_a_list_or_by_name() {
        let bad = || Err(CommandError::bad("at", "must be {x, y, z} or [x, y, z]"));
        let table: [(Value, Result<[f64; 3], CommandError>); 7] = [
            (json!([1, 2.5, -3]), Ok([1.0, 2.5, -3.0])),
            (json!({"x": 1, "y": 2, "z": 3}), Ok([1.0, 2.0, 3.0])),
            (json!({"x": 1, "y": 2, "z": 3, "w": 9}), Ok([1.0, 2.0, 3.0])),
            (json!([1, 2]), bad()),
            (json!({"x": 1, "y": 2}), bad()),
            (json!([1, "2", 3]), bad()),
            (json!("1, 2, 3"), bad()),
        ];
        for (value, want) in table {
            let given = args(json!({ "at": value }));
            assert_eq!(Args(&given).vec3("at"), want, "{value}");
        }
        let none = args(json!({"at": null}));
        assert_eq!(Args(&none).opt_vec3("at"), Ok(None));
        assert_eq!(
            Args(&none).vec3("at"),
            Err(CommandError::bad("at", "is required"))
        );
        let flat = args(json!({"at": {"x": 4, "y": 5}}));
        assert_eq!(Args(&flat).vec2("at"), Ok([4.0, 5.0]));
    }

    #[test]
    fn a_command_checks_its_arguments() {
        let spec = CommandSpec::new("t.cmd", "Test")
            .param("length", ParamKind::Number, "How long")
            .optional("name", ParamKind::String, "What to call it");
        assert!(spec.check(&args(json!({"length": 3}))).is_ok());
        assert!(spec.check(&args(json!({"length": 3, "name": "a"}))).is_ok());
        assert_eq!(
            spec.check(&args(json!({}))),
            Err(CommandError::bad("length", "is required"))
        );
        assert_eq!(
            spec.check(&args(json!({"length": "long"}))),
            Err(CommandError::bad("length", "must be a number"))
        );
        assert!(
            spec.check(&args(json!({"length": 1, "colour": 2})))
                .is_err()
        );
        // A slip names the argument meant.
        assert_eq!(
            spec.check(&args(json!({"lenght": 1}))),
            Err(CommandError::bad(
                "length",
                "is required (`lenght` was given)"
            ))
        );
        assert_eq!(
            spec.check(&args(json!({"length": 1, "nme": "a"}))),
            Err(CommandError::bad(
                "nme",
                "is not an argument of this command; did you mean `name`?"
            ))
        );
        let open = spec.extra_args("Any field");
        assert!(open.check(&args(json!({"length": 1, "colour": 2}))).is_ok());
    }

    /// An unknown command names the one most likely meant: a slip of a
    /// letter or two, or its last part alone; nothing for a stranger.
    #[test]
    fn an_unknown_command_names_the_one_meant() {
        let ids = ["design.pad", "design.pocket", "doc.bodies"];
        let near = |name| nearest(name, ids);
        assert_eq!(near("design.pda"), Some("design.pad"));
        assert_eq!(near("pocket"), Some("design.pocket"));
        assert_eq!(near("doc.bodys"), Some("doc.bodies"));
        assert_eq!(near("sketch.line"), None);
        assert_eq!(
            CommandError::unknown_among("design.pda", ids).to_string(),
            "no command `design.pda`; did you mean `design.pad`?"
        );
        assert_eq!(
            CommandError::unknown_among("nothing.here", ids),
            CommandError::Unknown("nothing.here".into())
        );
    }

    #[test]
    fn an_example_keeps_its_script_without_the_source_s_indentation() {
        let spec = CommandSpec::new("t.cmd", "Test").example(
            "Two lines",
            "
            local a = 1
            if a then
              assert(a == 1)
            end
            ",
        );
        assert_eq!(
            spec.examples[0].script,
            "local a = 1\nif a then\n  assert(a == 1)\nend\n"
        );
    }

    #[test]
    fn an_id_argument_must_parse() {
        let spec = CommandSpec::new("t.cmd", "Test").param("body", ParamKind::Id, "");
        let id = Uuid::new_v4().to_string();
        assert!(spec.check(&args(json!({ "body": id }))).is_ok());
        assert!(spec.check(&args(json!({"body": "nope"}))).is_err());
        let a = args(json!({ "body": id }));
        assert_eq!(Args(&a).id("body").unwrap().to_string(), id);
    }
}
