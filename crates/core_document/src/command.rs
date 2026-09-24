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
    /// Unique across the application, dotted like `part.pad`.
    pub id: String,
    /// One line on what it does.
    pub summary: String,
    pub params: Vec<ParamSpec>,
    /// What it answers, in words.
    pub returns: String,
    /// It takes named arguments beyond `params` (such as any field of a
    /// feature), described here.
    pub extra_args: Option<String>,
}

impl CommandSpec {
    pub fn new(id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            params: Vec::new(),
            returns: "nothing".to_string(),
            extra_args: None,
        }
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
                    return Err(CommandError::bad(&param.name, "is required"));
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
            return Err(CommandError::bad(
                unknown,
                "is not an argument of this command",
            ));
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
        });
        if let Some(extra) = &self.extra_args {
            out["extra_args"] = Value::String(extra.clone());
        }
        out
    }
}

/// A command the user ran through the UI rather than a script: what it
/// was called with and what it answered. A recording is a list of these.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub id: String,
    pub args: CommandArgs,
    pub result: Value,
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

impl CommandError {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: Value) -> CommandArgs {
        value.as_object().unwrap().clone()
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
        let open = spec.extra_args("Any field");
        assert!(open.check(&args(json!({"length": 1, "colour": 2}))).is_ok());
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
