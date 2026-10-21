//! What the console's runner, the Lua thread, takes and tells.

use std::sync::mpsc::Sender;

use core_document::{CommandArgs, CommandResult};

use crate::RunOutput;

/// What to run.
#[derive(Debug, Clone)]
pub enum Job {
    /// A console line: an expression answers its value.
    Line(String),
    /// A whole script; `name` names it in messages.
    Script { source: String, name: String },
    /// One command: the output's value is its answer as JSON.
    Command {
        id: String,
        args: core_document::CommandArgs,
    },
    /// Commands in order, stopping at the first that fails: the output's
    /// value is the list of their answers (`run_batch`).
    Batch(Vec<(String, core_document::CommandArgs)>),
}

impl Job {
    /// How a run of this job is named: the line itself, the script's name,
    /// or the command's id.
    pub fn label(&self) -> String {
        match self {
            Job::Line(line) => line.clone(),
            Job::Script { name, .. } => name.clone(),
            Job::Command { id, .. } => id.clone(),
            Job::Batch(calls) => format!("a batch of {} calls", calls.len()),
        }
    }
}

/// Run `calls` in order on `host`, stopping at the first that fails. An
/// argument that is exactly `"$N"` (also inside lists and objects) takes
/// the answer of call N, counted from 1, so a call can name what an
/// earlier one made. The output's value is the list of answers.
pub fn run_batch(host: &mut dyn crate::Host, calls: &[(String, CommandArgs)]) -> RunOutput {
    use serde_json::Value;
    fn fill(value: &Value, answers: &[Value]) -> Result<Value, String> {
        Ok(match value {
            Value::String(s) => match s.strip_prefix('$').map(str::parse::<usize>) {
                Some(Ok(n)) => answers
                    .get(n.wrapping_sub(1))
                    .cloned()
                    .ok_or_else(|| format!("`{s}` names no call before this one"))?,
                _ => value.clone(),
            },
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| fill(v, answers))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), fill(v, answers)?)))
                    .collect::<Result<_, String>>()?,
            ),
            other => other.clone(),
        })
    }
    let mut answers = Vec::new();
    for (n, (id, args)) in calls.iter().enumerate() {
        let args = match fill(&Value::Object(args.clone()), &answers) {
            Ok(Value::Object(args)) => args,
            Ok(_) => unreachable!("an object fills to an object"),
            Err(why) => {
                return RunOutput {
                    error: Some(format!("call {} ({id}): {why}", n + 1)),
                    ..RunOutput::default()
                };
            }
        };
        match host.call(id, args) {
            Ok(answer) => answers.push(answer),
            Err(err) => {
                return RunOutput {
                    error: Some(format!("call {} ({id}): {err}", n + 1)),
                    ..RunOutput::default()
                };
            }
        }
    }
    let answers = Value::Array(answers);
    RunOutput {
        value: Some(serde_json::to_string_pretty(&answers).unwrap_or_default()),
        returned: Some(answers),
        ..RunOutput::default()
    }
}

/// What the thread tells its holder.
#[derive(Debug)]
pub enum Event {
    /// A job began.
    Started { label: String },
    /// A line the script printed.
    Printed(String),
    /// Run a command and send its answer back on `reply`.
    Call {
        id: String,
        args: CommandArgs,
        reply: Sender<CommandResult>,
    },
    /// A job ended; `output.printed` repeats what came as `Printed`.
    Finished { label: String, output: RunOutput },
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{CommandError, CommandSpec};
    use serde_json::json;

    /// `make` answers a new id, `use` answers its arguments, `fail` fails;
    /// every call is kept.
    #[derive(Default)]
    struct Fake {
        called: Vec<String>,
    }

    impl crate::Host for Fake {
        fn commands(&self) -> Vec<CommandSpec> {
            Vec::new()
        }

        fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
            self.called.push(id.to_string());
            match id {
                "make" => Ok(json!(format!("id-{}", self.called.len()))),
                "use" => Ok(serde_json::Value::Object(args)),
                _ => Err(CommandError::failed("no")),
            }
        }
    }

    fn args(value: serde_json::Value) -> CommandArgs {
        value.as_object().unwrap().clone()
    }

    /// A batch answers every call's result in order, a `$N` argument
    /// taking call N's; it stops at the first failure, naming it, and a
    /// `$N` past the calls before it is refused.
    #[test]
    fn a_batch_runs_in_order_feeds_answers_on_and_stops_at_a_failure() {
        let mut host = Fake::default();
        let out = run_batch(
            &mut host,
            &[
                ("make".into(), args(json!({}))),
                ("use".into(), args(json!({"of": "$1", "all": ["$1", "$x"]}))),
            ],
        );
        assert_eq!(out.error, None);
        assert_eq!(
            out.returned,
            Some(json!(["id-1", {"of": "id-1", "all": ["id-1", "$x"]}]))
        );

        let mut host = Fake::default();
        let out = run_batch(
            &mut host,
            &[
                ("make".into(), args(json!({}))),
                ("fail".into(), args(json!({}))),
                ("make".into(), args(json!({}))),
            ],
        );
        assert_eq!(out.error.as_deref(), Some("call 2 (fail): no"));
        assert_eq!(
            host.called,
            ["make", "fail"],
            "nothing after the failure runs"
        );

        let out = run_batch(
            &mut Fake::default(),
            &[("use".into(), args(json!({"a": "$2"})))],
        );
        assert!(
            out.error
                .unwrap()
                .contains("`$2` names no call before this one")
        );
    }
}
