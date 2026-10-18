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
}

impl Job {
    /// How a run of this job is named: the line itself, the script's name,
    /// or the command's id.
    pub fn label(&self) -> String {
        match self {
            Job::Line(line) => line.clone(),
            Job::Script { name, .. } => name.clone(),
            Job::Command { id, .. } => id.clone(),
        }
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
