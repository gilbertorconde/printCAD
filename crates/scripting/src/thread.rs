//! Scripts on a thread of their own, so the window stays live while one
//! runs.
//!
//! The thread owns the Lua engine, so the console's globals last between
//! runs. An agent's script runs in a fresh sandboxed engine of its own
//! instead ([`ScriptEngine::sandboxed`]), under a time limit: it neither
//! sees nor leaves the console's globals. A command a script calls
//! travels to whoever holds the [`ScriptThread`] as an [`Event::Call`]
//! with a reply channel, and the script waits for the answer: every
//! command still runs where the document lives. Printed lines arrive as
//! they are printed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use core_document::{CommandArgs, CommandError, CommandResult, CommandSpec};

use crate::{Event, Host, Job, RunOutput, STOPPED, ScriptEngine};

/// How long an agent's script may run, counting only its own Lua and
/// not the commands it waits for; the user's runs have Stop instead.
pub const AGENT_TIME_LIMIT: Duration = Duration::from_secs(30);

struct Submission {
    job: Job,
    commands: Vec<CommandSpec>,
    /// For an agent's job, the time limit its sandboxed script runs under.
    agent: Option<Duration>,
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
        self.send(Submission {
            job,
            commands,
            agent: None,
        });
    }

    /// Queue an agent's `job`, in turn with the others: a script runs in
    /// a fresh sandboxed engine and stops once its Lua has run for
    /// `time_limit` ([`AGENT_TIME_LIMIT`] in the application).
    pub fn submit_for_agent(&mut self, job: Job, commands: Vec<CommandSpec>, time_limit: Duration) {
        self.send(Submission {
            job,
            commands,
            agent: Some(time_limit),
        });
    }

    fn send(&mut self, submission: Submission) {
        if self.jobs.send(submission).is_ok() {
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
    let prepare = |mut engine: ScriptEngine, time_limit: Duration| {
        engine.set_time_limit(time_limit);
        engine.stop_on(stop.clone());
        let (events, wake) = (events.clone(), wake.clone());
        engine.on_print(move |line| {
            let _ = events.send(Event::Printed(line.to_string()));
            wake();
        });
        engine
    };
    // The holder can stop a run, so nothing the user runs is cut off by
    // time.
    let mut engine = prepare(ScriptEngine::new(), Duration::MAX);
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    while let Ok(Submission {
        job,
        commands,
        agent,
    }) = jobs.recv()
    {
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
        let mut sandboxed = None;
        let engine = match (&job, agent) {
            (Job::Line(_) | Job::Script { .. }, Some(limit)) => {
                sandboxed.insert(prepare(ScriptEngine::sandboxed(), limit))
            }
            _ => &mut engine,
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
                    returned: Some(answer),
                    ..RunOutput::default()
                },
                Err(err) => RunOutput {
                    error: Some(format!("{id}: {err}")),
                    ..RunOutput::default()
                },
            },
            Job::Batch(calls) => crate::job::run_batch(&mut host, calls),
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

    /// Answer the thread's calls until the job finishes: `design.pad` gives
    /// an id, anything else is unknown. What was printed, and the output.
    fn serve(thread: &mut ScriptThread) -> (Vec<String>, RunOutput) {
        let mut printed = Vec::new();
        loop {
            match thread.next_event(Duration::from_secs(5)).expect("an event") {
                Event::Started { .. } => {}
                Event::Printed(line) => printed.push(line),
                Event::Call { id, reply, .. } => {
                    let _ = reply.send(match id.as_str() {
                        "design.pad" => Ok(json!("pad-1")),
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
                source: "print(pc.design.pad{length = 2})".into(),
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
                id: "design.pad".into(),
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

    /// An agent's script runs sandboxed in an engine of its own: no `io`,
    /// none of the console's globals, none left behind for it; and it
    /// stops at its time limit while the user's runs have none.
    #[test]
    fn an_agent_script_is_sandboxed_and_times_out() {
        let mut thread = ScriptThread::spawn(|| {});
        thread.submit(Job::Line("x = 41".into()), Vec::new());
        serve(&mut thread);
        let agent = |source: &str| Job::Script {
            source: source.into(),
            name: "agent.lua".into(),
        };
        let limit = Duration::from_millis(50);
        thread.submit_for_agent(
            agent("y = 1 return {io == nil, os == nil, x == nil}"),
            Vec::new(),
            limit,
        );
        assert_eq!(
            serve(&mut thread).1.returned,
            Some(json!([true, true, true]))
        );
        thread.submit_for_agent(agent("while true do end"), Vec::new(), limit);
        let timed_out = serve(&mut thread).1.error.unwrap();
        assert!(timed_out.contains("stopped after 0.05 s"), "{timed_out}");
        thread.submit(Job::Line("x + 1, y == nil".into()), Vec::new());
        assert_eq!(serve(&mut thread).1.value.as_deref(), Some("42\ttrue"));
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
