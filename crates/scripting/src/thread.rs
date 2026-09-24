//! Scripts on a thread of their own, so the window stays live while one
//! runs.
//!
//! The thread owns the Lua engine, so the console's globals last between
//! runs. A command a script calls travels to whoever holds the
//! [`ScriptThread`] as an [`Event::Call`] with a reply channel, and the
//! script waits for the answer: every command still runs where the
//! document lives. Printed lines arrive as they are printed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use core_document::{CommandArgs, CommandError, CommandResult, CommandSpec};

use crate::{Host, RunOutput, STOPPED, ScriptEngine};

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
    /// How a run of this job is named: the line itself, or the script's
    /// name.
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

struct Submission {
    job: Job,
    commands: Vec<CommandSpec>,
}

/// The holder's end of the script thread.
pub struct ScriptThread {
    jobs: Sender<Submission>,
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    /// Jobs submitted and not yet finished.
    pending: usize,
}

impl ScriptThread {
    /// Start the thread. `wake` is called whenever an event is sent, for a
    /// holder that sleeps between events.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (jobs_tx, jobs_rx) = channel::<Submission>();
        let (events_tx, events_rx) = channel::<Event>();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let wake = Arc::new(wake);
        std::thread::Builder::new()
            .name("printcad-scripts".to_string())
            .spawn(move || run_jobs(jobs_rx, events_tx, thread_stop, wake))
            .expect("the script thread starts");
        Self {
            jobs: jobs_tx,
            events: events_rx,
            stop,
            pending: 0,
        }
    }

    /// Queue `job`, with the commands it may call (for `help`). Jobs run one
    /// after another.
    pub fn submit(&mut self, job: Job, commands: Vec<CommandSpec>) {
        if self.jobs.send(Submission { job, commands }).is_ok() {
            self.pending += 1;
        }
    }

    /// Whether a job is running or waiting to.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// Stop the running job at its next instruction or command. Queued jobs
    /// still run.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// The next event, waiting at most `wait` for one.
    pub fn next_event(&mut self, wait: Duration) -> Option<Event> {
        let event = if wait.is_zero() {
            self.events.try_recv().ok()
        } else {
            self.events.recv_timeout(wait).ok()
        };
        if matches!(event, Some(Event::Finished { .. })) {
            self.pending = self.pending.saturating_sub(1);
        }
        event
    }
}

fn run_jobs(
    jobs: Receiver<Submission>,
    events: Sender<Event>,
    stop: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut engine = ScriptEngine::new();
    // The holder can stop a run, so nothing is cut off by time.
    engine.set_time_limit(Duration::MAX);
    engine.stop_on(stop.clone());
    {
        let (events, wake) = (events.clone(), wake.clone());
        engine.on_print(move |line| {
            let _ = events.send(Event::Printed(line.to_string()));
            wake();
        });
    }
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    while let Ok(Submission { job, commands }) = jobs.recv() {
        stop.store(false, Ordering::Relaxed);
        let label = job.label();
        send(Event::Started {
            label: label.clone(),
        });
        let mut host = Relay {
            events: events.clone(),
            wake: wake.clone(),
            stop: stop.clone(),
            commands,
        };
        let output = match &job {
            Job::Line(line) => engine.eval_line(line, &mut host),
            Job::Script { source, name } => engine.run_script(source, name, &mut host),
            Job::Command { id, args } => match host.call(id, args.clone()) {
                Ok(answer) => RunOutput {
                    value: Some(
                        serde_json::to_string_pretty(&answer)
                            .unwrap_or_else(|_| answer.to_string()),
                    ),
                    ..RunOutput::default()
                },
                Err(err) => RunOutput {
                    error: Some(format!("{id}: {err}")),
                    ..RunOutput::default()
                },
            },
        };
        send(Event::Finished { label, output });
    }
}

/// The script's side of a command call: ask, and wait for the answer.
struct Relay {
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    stop: Arc<AtomicBool>,
    commands: Vec<CommandSpec>,
}

impl Host for Relay {
    fn commands(&self) -> Vec<CommandSpec> {
        self.commands.clone()
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        if self.stop.load(Ordering::Relaxed) {
            return Err(CommandError::failed(STOPPED));
        }
        let (reply, answer) = channel();
        let asked = self.events.send(Event::Call {
            id: id.to_string(),
            args,
            reply,
        });
        if asked.is_err() {
            return Err(CommandError::failed("the application is closing"));
        }
        (self.wake)();
        answer
            .recv()
            .unwrap_or_else(|_| Err(CommandError::failed("the application is closing")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Answer the thread's calls until the job finishes: `part.pad` gives
    /// an id, anything else is unknown. What was printed, and the output.
    fn serve(thread: &mut ScriptThread) -> (Vec<String>, RunOutput) {
        let mut printed = Vec::new();
        loop {
            match thread.next_event(Duration::from_secs(5)).expect("an event") {
                Event::Started { .. } => {}
                Event::Printed(line) => printed.push(line),
                Event::Call { id, reply, .. } => {
                    let _ = reply.send(match id.as_str() {
                        "part.pad" => Ok(json!("pad-1")),
                        _ => Err(CommandError::Unknown(id)),
                    });
                }
                Event::Finished { output, .. } => return (printed, output),
            }
        }
    }

    #[test]
    fn a_script_on_the_thread_calls_commands_through_its_holder() {
        let mut thread = ScriptThread::spawn(|| {});
        thread.submit(
            Job::Script {
                source: "print(pc.part.pad{length = 2})".into(),
                name: "t.lua".into(),
            },
            Vec::new(),
        );
        assert!(thread.busy());
        let (printed, output) = serve(&mut thread);
        assert_eq!(printed, ["pad-1"]);
        assert_eq!(output.error, None);
        assert!(!thread.busy());
        // The engine is the same from run to run: globals last.
        thread.submit(Job::Line("x = 41".into()), Vec::new());
        serve(&mut thread);
        thread.submit(Job::Line("x + 1".into()), Vec::new());
        assert_eq!(serve(&mut thread).1.value.as_deref(), Some("42"));
    }

    #[test]
    fn a_single_command_answers_json() {
        let mut thread = ScriptThread::spawn(|| {});
        let args = json!({"length": 2}).as_object().unwrap().clone();
        thread.submit(
            Job::Command {
                id: "part.pad".into(),
                args,
            },
            Vec::new(),
        );
        assert_eq!(serve(&mut thread).1.value.as_deref(), Some("\"pad-1\""));
        thread.submit(
            Job::Command {
                id: "no.such".into(),
                args: Default::default(),
            },
            Vec::new(),
        );
        assert!(serve(&mut thread).1.error.unwrap().contains("no command"));
    }

    #[test]
    fn a_running_script_stops_when_asked_and_the_next_one_runs() {
        let mut thread = ScriptThread::spawn(|| {});
        thread.submit(Job::Line("while true do end".into()), Vec::new());
        thread.submit(Job::Line("1 + 1".into()), Vec::new());
        match thread.next_event(Duration::from_secs(5)) {
            Some(Event::Started { .. }) => {}
            other => panic!("{other:?}"),
        }
        thread.stop();
        let (_, stopped) = serve(&mut thread);
        assert!(stopped.error.unwrap().contains(STOPPED));
        assert_eq!(serve(&mut thread).1.value.as_deref(), Some("2"));
    }
}
