//! Scripts and the console: the Lua engine run against every command the
//! application and the workbenches offer.
//!
//! The application's own commands are the document's queries and edits
//! (`doc.*`) and every command a key can run (`file.save`, `view.front`,
//! ...). A workbench's command runs in the workbench through
//! `Workbench::run_command`, with the same context its tools get, so the
//! host never needs to know what the command does. Every call's arguments
//! are checked against its spec before it runs.

mod catalog;
pub(crate) use catalog::{
    SCAN_IMPORT_DOC, UNITS_IMPORT_DOC, command_specs, doc_commands, export_options, import_units,
    reference,
};
use catalog::{all_commands, app_commands, key_commands};
mod document;
pub(crate) use document::{
    body_list, document_command, drop_place, export_format, features_in_order, kind_of, move_after,
    move_in_history, print_layout_command, rebuild_failures, set_body_tip, set_tip, suppress,
    suppress_all, suppress_call, surface_json, tree_item, user_note_guard,
};
#[cfg(test)]
use document::{faces_of, set_body};
mod recording;
use recording::item_id;
pub(crate) use recording::{body_edit_call, recorded_of};
mod reports;
pub(crate) use reports::picture_command;

use core_document::{Args, CommandArgs, CommandError, CommandResult};
use serde_json::{Value, json};
use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::app::workbench_host::HookSite;
use crate::console::{self, LineKind};
use crate::ui::TreeItemId;
use crate::ui::keymap::{self, HostOutcome, HostState};

impl PrintCadApp {
    /// Run command `id` for a script, its arguments checked against its
    /// spec: the application's own here, a workbench's in the workbench.
    pub(crate) fn execute_command(
        &mut self,
        id: &str,
        args: CommandArgs,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        if let Some(spec) = doc_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.run_doc_command(id, &args, event_loop);
        }
        if let Some(spec) = app_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.run_app_command(id, &args);
        }
        if let Some((spec, action)) = key_commands().find(|(c, _)| c.id == id) {
            spec.check(&args)?;
            if Args(&args).has("path") {
                return self.run_file_command(action, &args);
            }
            if let (keymap::HostAction::SendToSlicer, Some(layout)) =
                (action, Args(&args).opt_bool("layout")?)
            {
                let was = self.user_settings.printing.slicer_layout;
                self.user_settings.printing.slicer_layout = layout;
                self.send_to_slicer(false);
                self.user_settings.printing.slicer_layout = was;
                return Ok(Value::Null);
            }
            return self.run_key_command(action, event_loop);
        }
        let Some((bench, spec)) = self.registry.command(id) else {
            let known = command_specs(&self.registry);
            return Err(CommandError::unknown_among(
                id,
                known.iter().map(|c| c.id.as_str()),
            ));
        };
        spec.check(&args)?;
        // A bench reads features as their formulas leave them: worked out
        // for the calls before this one, as a headless run has them.
        self.registry.evaluate(&mut self.session.document);
        let params = self.interaction_ctx_params();
        let (result, outcome) = self
            .with_workbench_ctx(&bench, params, |wb, ctx| wb.run_command(id, &args, ctx))
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        self.apply_hook_outcome(outcome, HookSite::Interaction);
        result
    }

    /// Run one line typed in the console on the script thread.
    pub(crate) fn run_console_line(&mut self, line: &str) {
        console::push(LineKind::Input, line);
        self.submit_script(scripting::Job::Line(line.to_string()), RunKind::Console);
    }

    /// Run one command the palette called, its call and answer shown in
    /// the console, which opens for them.
    pub(crate) fn run_palette_command(&mut self, id: String, args: CommandArgs) {
        console::push(
            LineKind::Input,
            format!("{id} {}", serde_json::Value::Object(args.clone())),
        );
        self.console_attention = true;
        self.submit_script(scripting::Job::Command { id, args }, RunKind::Console);
    }

    pub(crate) fn submit_script(&mut self, job: scripting::Job, kind: RunKind) {
        let tab = self.session.tab;
        self.submit_script_in(job, kind, tab);
    }

    /// Run `job` on the document of `tab`, whichever tab is on screen.
    pub(crate) fn submit_script_in(&mut self, job: scripting::Job, kind: RunKind, tab: uuid::Uuid) {
        let commands = command_specs(&self.registry);
        self.script_runs.push_back(ScriptRun {
            calls: Vec::new(),
            tab,
            kind,
            label: match &job {
                scripting::Job::Line(_) => "a console line".to_string(),
                scripting::Job::Script { name, .. } => name.clone(),
                scripting::Job::Command { id, .. } => id.clone(),
                scripting::Job::Batch(calls) => format!("a batch of {} calls", calls.len()),
            },
        });
        self.script_thread.submit(job, commands);
        self.redraw_needed = true;
    }

    fn run_app_command(&mut self, id: &str, args: &CommandArgs) -> CommandResult {
        let a = Args(args);
        match id {
            "scan.mark" => self.run_scan_mark(&a),
            "view.section" => self.view_section(args),
            "app.workbenches" => Ok(Value::Array(
                self.registry
                    .ids()
                    .iter()
                    .map(|bench| {
                        json!({
                            "id": bench.as_str(),
                            "label": self.registry.descriptor(bench).map(|d| d.label.clone()),
                            "active": *bench == self.session.active_workbench.0,
                        })
                    })
                    .collect(),
            )),
            "app.print_compensation" => {
                let a = core_document::Args(args);
                let printing = &mut self.user_settings.printing;
                let given = ["x", "y", "z", "holes", "bosses"].iter().any(|k| a.has(k));
                if given {
                    let offset = |k: &str| -> Result<f64, CommandError> {
                        let v = a.opt_number(k)?.unwrap_or(0.0);
                        if v.abs() <= 1.0 {
                            Ok(v)
                        } else {
                            Err(CommandError::bad(k, "is an offset within ±1 mm"))
                        }
                    };
                    let factor = |k: &str| -> Result<f64, CommandError> {
                        let v = a.opt_number(k)?.unwrap_or(1.0);
                        if (0.5..=2.0).contains(&v) {
                            Ok(v)
                        } else {
                            Err(CommandError::bad(k, "is a factor between 0.5 and 2"))
                        }
                    };
                    printing.compensation = Some(settings::PrintCompensation {
                        scale: [factor("x")?, factor("y")?, factor("z")?],
                        holes: offset("holes")?,
                        bosses: offset("bosses")?,
                        from: a.opt_string("from")?.unwrap_or_default().to_string(),
                    });
                    printing.apply_compensation = a.opt_bool("on")?.unwrap_or(true);
                } else if let Some(on) = a.opt_bool("on")? {
                    printing.apply_compensation = on && printing.compensation.is_some();
                }
                let answer = match &self.user_settings.printing.compensation {
                    Some(c) => json!({
                        "x": c.scale[0], "y": c.scale[1], "z": c.scale[2],
                        "holes": c.holes, "bosses": c.bosses, "from": c.from, "on": self.user_settings.printing.apply_compensation,
                    }),
                    None => json!({"on": false}),
                };
                if (given || a.has("on"))
                    && let Err(err) = self.settings_store.save(&self.user_settings)
                {
                    crate::log_panel::warn(format!("Failed to save settings: {err}"));
                }
                Ok(answer)
            }
            "app.workbench" => {
                let bench = self.bench_arg(a.string("id")?)?;
                self.switch_workbench_for_flow(bench);
                Ok(Value::Null)
            }
            "app.tools" => {
                let bench = match a.opt_string("workbench")? {
                    Some(id) => self.bench_arg(id)?,
                    None => self.session.active_workbench.0.clone(),
                };
                let tools = self.registry.tools_for(&bench).unwrap_or_default();
                Ok(Value::Array(
                    tools
                        .iter()
                        .filter(|t| t.planned.is_none())
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "label": t.label,
                                "keys": t.shortcuts.iter().map(ToString::to_string).collect::<Vec<_>>(),
                            })
                        })
                        .collect(),
                ))
            }
            "app.tool" => {
                let id = a.string("id")?;
                let bench = self
                    .registry
                    .ids()
                    .iter()
                    .find(|bench| {
                        self.registry.tools_for(bench).is_ok_and(|tools| {
                            tools.iter().any(|t| t.id == id && t.planned.is_none())
                        })
                    })
                    .cloned()
                    .ok_or_else(|| CommandError::bad("id", "is not a tool"))?;
                if bench != self.session.active_workbench.0 {
                    self.switch_workbench_for_flow(bench.clone());
                }
                let tools = self.registry.tools_for(&bench).unwrap_or_default().to_vec();
                if let Some(tool) = tools.iter().find(|t| t.id == id) {
                    crate::ui::toolbar::activate_tool(
                        &mut self.session.active_tool,
                        &tools,
                        tool,
                        id,
                    );
                }
                Ok(Value::Null)
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }

    fn bench_arg(&self, id: &str) -> Result<core_document::WorkbenchId, CommandError> {
        self.registry
            .ids()
            .iter()
            .find(|b| b.as_str() == id)
            .cloned()
            .ok_or_else(|| {
                CommandError::bad("id", "is not a workbench; app.workbenches lists them")
            })
    }

    /// A file command given its file: no dialog.
    fn run_file_command(
        &mut self,
        action: keymap::HostAction,
        args: &CommandArgs,
    ) -> CommandResult {
        use keymap::HostAction::*;
        let a = Args(args);
        let path = std::path::PathBuf::from(a.string("path")?);
        match action {
            Open => {
                self.open_document_at(path);
                Ok(Value::Null)
            }
            SaveAs => {
                self.save_document_at(&path)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                Ok(Value::Null)
            }
            Import => {
                if !path.is_file() {
                    return Err(CommandError::bad("path", "is not a file"));
                }
                if a.opt_bool("scan")? == Some(true) {
                    self.scan_imports.insert(path.clone());
                }
                if let Some(unit) = import_units(&a, &path, self.user_settings.files.mesh_units)? {
                    self.import_units.insert(path.clone(), unit);
                }
                let detail = self.last_step_import_detail.clone();
                self.import_step_at(&path, detail);
                Ok(Value::Null)
            }
            Export => {
                let format = export_format(a.opt_string("format")?, &path)?;
                let bodies = body_list(args.get("bodies"))?;
                let tolerance = a.opt_number("tolerance")?.map(|t| t as f32);
                let layout =
                    (a.opt_bool("layout")? == Some(true)).then(|| self.current_print_layout());
                let printing = &self.user_settings.printing;
                let compensation = printing
                    .compensation
                    .as_ref()
                    .filter(|_| printing.apply_compensation);
                let (path, exported) = crate::app::export::export_document(
                    (&self.session.document, &self.registry),
                    path,
                    format,
                    bodies,
                    tolerance,
                    layout.as_ref(),
                    compensation,
                )
                .map_err(CommandError::failed)?;
                Ok(json!({
                    "path": path.display().to_string(),
                    "written": exported.written,
                    "skipped": exported.skipped,
                    "triangles": exported.triangles,
                }))
            }
            _ => Err(CommandError::failed("this command takes no file")),
        }
    }

    /// Run a script file on the script thread. What it prints and the error
    /// that stops it open the console.
    pub(crate) fn run_script_file(&mut self, path: &std::path::Path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let read = crate::platform::read(path).and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
        });
        match read {
            Ok(source) => {
                console::push(LineKind::Input, format!("run {name}"));
                self.submit_script(scripting::Job::Script { source, name }, RunKind::File);
            }
            Err(err) => {
                console::push(LineKind::Error, format!("{name}: {err}"));
                self.console_attention = true;
            }
        }
    }

    /// Stop the running script: at its next instruction, or at once when
    /// it waits on a rebuild.
    pub(crate) fn stop_script(&mut self) {
        self.script_thread.stop();
        if let Some(wait) = self.script_rebuild.take() {
            let _ = wait
                .reply
                .send(Err(CommandError::failed(scripting::STOPPED)));
        }
    }

    /// Answer the script thread: run the commands it asks for, show what it
    /// prints, and close a run's undo step when it ends. Commands keep
    /// coming within a few milliseconds a frame, so a script runs at speed
    /// while the window keeps drawing.
    pub(crate) fn drive_scripts(&mut self, event_loop: &ActiveEventLoop) {
        const BUDGET: std::time::Duration = std::time::Duration::from_millis(8);
        let started = std::time::Instant::now();
        loop {
            if self.script_rebuild.is_some() {
                self.answer_rebuild();
                if self.script_rebuild.is_some() {
                    return;
                }
            }
            let wait = if self.script_thread.busy() && started.elapsed() < BUDGET {
                std::time::Duration::from_millis(1)
            } else {
                std::time::Duration::ZERO
            };
            let Some(event) = self.script_thread.next_event(wait) else {
                return;
            };
            self.script_event(event, event_loop);
            if started.elapsed() > BUDGET {
                self.redraw_needed = true;
                return;
            }
        }
    }

    fn script_event(&mut self, event: scripting::Event, event_loop: &ActiveEventLoop) {
        use scripting::Event;
        let kind = self.script_runs.front().map(|r| r.kind.clone());
        let from_file = matches!(kind, Some(RunKind::File));
        let agent = matches!(kind, Some(RunKind::Agent { .. }));
        match event {
            Event::Started { label } => {
                let step = match kind {
                    Some(RunKind::File) => format!("Run {label}"),
                    Some(RunKind::Agent { .. }) => format!("Agent: {label}"),
                    _ => "Console".to_string(),
                };
                self.in_script_tab(|app| {
                    app.close_gesture();
                    app.session.journal.label_next(step);
                    app.session.journal.hold(true);
                });
            }
            Event::Printed(line) => {
                // An agent's script answers the agent: its output goes back
                // with it when it ends.
                if agent {
                    return;
                }
                console::push(LineKind::Printed, line);
                if from_file {
                    self.console_attention = true;
                }
            }
            Event::Call {
                id,
                mut args,
                reply,
            } => {
                // A script or recording may use a command's former name.
                let id = core_document::renamed::command(&id).into_owned();
                // An agent's notes are the agent's, whatever it says.
                if agent
                    && matches!(
                        id.as_str(),
                        "doc.note_add" | "doc.note_set" | "doc.delete" | "doc.rename"
                    )
                {
                    args.insert("author".into(), json!("agent"));
                }
                if let Some(RunKind::Agent {
                    single, elsewhere, ..
                }) = &kind
                    && let Err(refused) = self
                        .agent_may_run(&id, *single)
                        .and_then(|()| self.agent_path_check(&args))
                        .and_then(|()| match *elsewhere && !self.command_reads_only(&id) {
                            true => Err(crate::app::mcp::changes_elsewhere(&id)),
                            false => Ok(()),
                        })
                {
                    let _ = reply.send(Err(CommandError::failed(refused)));
                    return;
                }
                if id == "doc.rebuild" {
                    self.start_rebuild(args, reply);
                    return;
                }
                let kept = args.clone();
                let answer = self
                    .in_script_tab(|app| app.execute_command(&id, args, event_loop))
                    .unwrap_or_else(|| {
                        Err(CommandError::failed("the script's document was closed"))
                    });
                if let (Ok(result), Some(run)) = (&answer, self.script_runs.front_mut()) {
                    run.calls.push(core_document::Recorded {
                        id,
                        args: kept,
                        result: result.clone(),
                    });
                }
                let _ = reply.send(answer);
            }
            Event::Finished { label, mut output } => {
                let failed = output.error.is_some();
                let rolled = self.in_script_tab(|app| {
                    app.session.journal.hold(false);
                    // A run that stops with an error leaves nothing behind.
                    let rolled = failed.then(|| {
                        let rolled = app.session.journal.roll_back(&mut app.session.document);
                        if rolled.0 > 0 {
                            app.after_history_jump();
                        }
                        rolled
                    });
                    app.close_gesture();
                    rolled
                });
                if let (Some(error), Some(Some((undone, barrier)))) = (&mut output.error, rolled)
                    && undone > 0
                {
                    error.push_str(&rolled_back_note(barrier));
                }
                if let Some(run) = self.script_runs.pop_front()
                    && !failed
                {
                    crate::app::crash::note(run.tab, &run.calls);
                }
                if let Some(RunKind::Agent { reply, single, .. }) = kind {
                    let answer = if single {
                        agent_answer(output)
                    } else {
                        script_answer(output)
                    };
                    let _ = reply.send(answer);
                    self.redraw_needed = true;
                    return;
                }
                match (output.value, &output.returned) {
                    (Some(value), _) => console::push(LineKind::Value, value),
                    (None, Some(returned)) => console::push(LineKind::Value, short_json(returned)),
                    (None, None) => {}
                }
                match output.error {
                    Some(error) => {
                        console::push(LineKind::Error, error.clone());
                        if from_file {
                            self.console_attention = true;
                            crate::app_log::warn(format!("Script {label} stopped: {error}"));
                        }
                    }
                    None if from_file => crate::app_log::info(format!("Ran {label}")),
                    None => {}
                }
                self.redraw_needed = true;
            }
        }
    }

    /// Run `f` with the tab the running script started in on screen, for
    /// as long as `f` takes. `None` when that tab has closed.
    fn in_script_tab<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let tab = self.script_runs.front().map(|r| r.tab)?;
        if tab == self.session.tab {
            return Some(f(self));
        }
        let index = self.tab_index_of(tab)?;
        Some(self.with_tab(index, f))
    }

    /// `doc.rebuild`: send every changed body to the kernel now, and answer
    /// once the kernel has nothing left to do.
    fn start_rebuild(&mut self, args: CommandArgs, reply: std::sync::mpsc::Sender<CommandResult>) {
        let spec = doc_commands().into_iter().find(|c| c.id == "doc.rebuild");
        if let Some(Err(err)) = spec.map(|s| s.check(&args)) {
            let _ = reply.send(Err(err));
            return;
        }
        let timeout = Args(&args)
            .opt_number("timeout")
            .ok()
            .flatten()
            .unwrap_or(60.0);
        self.in_script_tab(|app| {
            app.drive_part_recompute();
            app.drive_shape_repairs();
            app.drive_shape_refinements();
            app.drive_shape_replacements();
            app.drive_mesh_solids();
            app.drive_mesh_histories();
            app.drive_mirrored_copies();
            app.drive_links();
        });
        self.script_rebuild = Some(RebuildWait {
            reply,
            deadline: std::time::Instant::now()
                + std::time::Duration::from_secs_f64(timeout.max(0.0)),
        });
    }

    /// Answer the waiting `doc.rebuild` once the kernel is idle: the
    /// features that failed.
    fn answer_rebuild(&mut self) {
        let Some(deadline) = self.script_rebuild.as_ref().map(|wait| wait.deadline) else {
            return;
        };
        let mesh_pending = self
            .in_script_tab(|app| {
                app.drive_mesh_histories();
                app.session.mesh_histories.pending()
            })
            .unwrap_or(false);
        let answer = if self.kernel_worker.in_flight() == 0 && !mesh_pending {
            let errors = self
                .in_script_tab(|app| rebuild_failures(&app.session.document))
                .unwrap_or_default();
            Ok(Value::Array(errors))
        } else if std::time::Instant::now() > deadline {
            Err(CommandError::failed(
                "the kernel was still working at the timeout",
            ))
        } else {
            return;
        };
        if let Some(wait) = self.script_rebuild.take() {
            let _ = wait.reply.send(answer);
        }
    }

    /// Keep `calls` in the recording, when one is on and no script is
    /// running (a script's own calls are already a script).
    pub(crate) fn record_calls(&mut self, calls: Vec<core_document::Recorded>) {
        if !self.script_runs.is_empty() {
            return;
        }
        crate::app::crash::note(self.session.tab, &calls);
        if let Some(recorder) = self.recording.as_mut() {
            for call in &calls {
                recorder.push(call);
            }
        }
    }

    /// Start a recording, or stop the one on and save it as a new script
    /// in the scripts folder, where the Scripts menu lists it.
    pub(crate) fn toggle_recording(&mut self) {
        let Some(recorder) = self.recording.take() else {
            self.recording = Some(scripting::Recorder::default());
            crate::app_log::info("Recording: what you do now is written as a script when you stop");
            return;
        };
        if recorder.is_empty() {
            crate::app_log::info("Recording stopped; nothing was recorded");
            return;
        }
        let Some(dir) = settings::scripts_dir() else {
            crate::app_log::warn("The system names no configuration folder for scripts");
            return;
        };
        let path = crate::script_library::fresh_name(&dir, "recording");
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().replace('_', " "))
            .unwrap_or_default();
        let text = recorder.script(&format!("A recording ({name})"));
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text)) {
            Ok(()) => {
                self.script_library_read = None;
                console::push(
                    LineKind::Printed,
                    format!("Recording saved as {}", path.display()),
                );
                crate::app_log::info(format!("Recording saved as {}", path.display()));
            }
            Err(err) => {
                crate::app_log::error(format!("Could not write {}: {err}", path.display()));
            }
        }
    }

    /// Every command's id, for the console's completion.
    pub(crate) fn script_command_ids(&self) -> Vec<String> {
        all_commands(self).into_iter().map(|c| c.id).collect()
    }

    /// Read the scripts folder again, every couple of seconds while frames
    /// run, so a script saved in an editor shows up without a restart.
    pub(crate) fn refresh_script_library(&mut self) {
        let due = self
            .script_library_read
            .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(2));
        if !due {
            return;
        }
        self.script_library_read = Some(std::time::Instant::now());
        self.script_library = settings::scripts_dir()
            .map(|dir| crate::script_library::scan(&dir))
            .unwrap_or_default();
    }

    /// Make a new script in the scripts folder, from `runs` (what the
    /// console ran) or else the template, and open it in the system's
    /// editor.
    pub(crate) fn new_script(&mut self, runs: Option<Vec<String>>) {
        let Some(dir) = settings::scripts_dir() else {
            crate::app_log::warn("The system names no configuration folder for scripts");
            return;
        };
        let path = crate::script_library::fresh_path(&dir);
        let text = match runs {
            Some(runs) => crate::script_library::from_runs(&runs),
            None => crate::script_library::TEMPLATE.to_string(),
        };
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text));
        match written {
            Ok(()) => {
                crate::app_log::info(format!("New script {}", path.display()));
                self.script_library_read = None;
                self.edit_script(Some(path));
            }
            Err(err) => crate::app_log::error(format!("Could not write {}: {err}", path.display())),
        }
    }

    /// Open `path` in the system's editor, or the scripts folder in its
    /// file manager.
    pub(crate) fn edit_script(&mut self, path: Option<std::path::PathBuf>) {
        let target = match path {
            Some(path) => path,
            None => {
                let Some(dir) = settings::scripts_dir() else {
                    return;
                };
                if let Err(err) = std::fs::create_dir_all(&dir) {
                    crate::app_log::error(format!("Could not make {}: {err}", dir.display()));
                    return;
                }
                dir
            }
        };
        if let Err(err) = local_ipc::open_with_system(&target) {
            crate::app_log::error(format!("Could not open {}: {err}", target.display()));
        }
    }

    fn run_key_command(
        &mut self,
        action: keymap::HostAction,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        let state = HostState {
            active_tab: Some(self.session.tab),
            section_on: self.session.section.is_some(),
            document: &self.session.document,
            tree_selection: self.session.tree_selection,
            editing: false,
        };
        match keymap::host_outcome(action, &state) {
            HostOutcome::Command(command) => {
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            _ => Err(CommandError::failed("this command is not available here")),
        }
    }

    fn run_doc_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        // Given no plane, the view's plane is kept.
        if id == "doc.save_section" && !Args(args).has("axis") && !Args(args).has("normal") {
            return self.save_view_section(args);
        }
        if let Some(answer) = document_command(
            id,
            args,
            &mut self.session.document,
            &self.registry,
            self.session.current_file.as_deref(),
        ) {
            return answer;
        }
        let a = Args(args);
        match id {
            "doc.picture" => picture_command(args, |request| self.picture(request)),
            "doc.report_pdf" => self.report_pdf(args),
            "doc.print_layout" => print_layout_command(
                args,
                &self.session.document,
                &self.registry,
                &self.user_settings.printing,
            ),
            "doc.selection" => Ok(json!({
                "item": self.session.tree_selection.and_then(item_id).map(|u| u.to_string()),
                "body": self.session.active_body_id.map(|b| b.0.to_string()),
                "feature": self.session.active_document_object.map(|f| f.0.to_string()),
            })),
            "doc.select" => {
                let item = tree_item(&self.session.document, a.id("id")?)?;
                self.apply_tree_selection(item);
                Ok(Value::Null)
            }
            "doc.set_visible" => {
                let visible = a.opt_bool("visible")?.unwrap_or(true);
                let command = match tree_item(&self.session.document, a.id("id")?)? {
                    TreeItemId::Body(body) => {
                        crate::ui::UiCommand::SetBodyVisible { body, visible }
                    }
                    TreeItemId::Feature(feature) => crate::ui::UiCommand::TreeFeature {
                        feature,
                        command: crate::ui::TreeFeatureCommand::SetVisible(visible),
                    },
                    TreeItemId::ImportedObject(node) => {
                        crate::ui::UiCommand::SetImportedVisibility { node, visible }
                    }
                    TreeItemId::Component(component) => {
                        let commands = self
                            .session
                            .document
                            .component_bodies(component)
                            .into_iter()
                            .map(|body| crate::ui::UiCommand::SetBodyVisible { body, visible })
                            .collect();
                        self.apply_ui_commands(commands, event_loop);
                        return Ok(Value::Null);
                    }
                    TreeItemId::Array(array) => {
                        let commands = self
                            .session
                            .document
                            .copy_array(array)
                            .map(|a| a.copy_bodies().collect::<Vec<_>>())
                            .unwrap_or_default()
                            .into_iter()
                            .map(|body| crate::ui::UiCommand::SetBodyVisible { body, visible })
                            .collect();
                        self.apply_ui_commands(commands, event_loop);
                        return Ok(Value::Null);
                    }
                    TreeItemId::Note(note) => {
                        crate::ui::UiCommand::SetNoteVisible { note, visible }
                    }
                    TreeItemId::DocumentRoot | TreeItemId::Notes(_) => {
                        return Err(CommandError::bad("id", "cannot be hidden"));
                    }
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.repair" | "doc.convert_to_solid" | "doc.refine" => {
                let bodies = body_list(args.get("bodies"))?.unwrap_or_default();
                let command = match id {
                    "doc.repair" => crate::ui::UiCommand::RepairShapes(bodies),
                    "doc.refine" => crate::ui::UiCommand::RefineShapes(bodies),
                    _ => crate::ui::UiCommand::ConvertToSolid(bodies),
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.delete" => {
                let item = tree_item(&self.session.document, a.id("id")?)?;
                user_note_guard(&self.session.document, item, &a, "delete")?;
                self.apply_ui_commands(
                    vec![crate::ui::UiCommand::DeleteTreeItem(item)],
                    event_loop,
                );
                Ok(Value::Null)
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

/// A script run submitted to the script thread: the tab it runs against
/// and who asked for it.
#[derive(Debug, Clone)]
pub(crate) struct ScriptRun {
    pub tab: Uuid,
    pub kind: RunKind,
    /// The script's name, the console line, or the command.
    pub label: String,
    /// The calls it made that answered, for its tab's crash trail once
    /// it finishes (a run that fails is taken back, and so are they).
    pub calls: Vec<core_document::Recorded>,
}

/// Who asked for a script run, and where its output goes.
#[derive(Debug, Clone)]
pub(crate) enum RunKind {
    /// A console line: its output shows there.
    Console,
    /// A script file: its output shows in the console, which opens for it.
    File,
    /// An agent's tool call: its output is the answer. `single` for a
    /// `call`, one command the user's approval covered; a `lua` script's
    /// commands are checked one by one as it calls them. `elsewhere` when
    /// it runs in another open file than its chat's, where every command
    /// must only read.
    Agent {
        reply: std::sync::mpsc::Sender<agents::mcp::ToolAnswer>,
        single: bool,
        elsewhere: bool,
    },
}

/// What a failed run's error adds once its changes are taken back;
/// `barrier` when an import in it could not be.
fn rolled_back_note(barrier: bool) -> String {
    if barrier {
        " (its changes after its last import were undone; the import stays)".into()
    } else {
        " (its changes were undone)".into()
    }
}

/// What an agent's run answers: what it printed and came to, or why it
/// stopped.
fn agent_answer(output: scripting::RunOutput) -> agents::mcp::ToolAnswer {
    let mut text = output.printed.join("\n");
    let mut add = |part: &str| {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(part);
    };
    match (&output.error, &output.value) {
        (Some(error), _) => {
            add(error);
            agents::mcp::ToolAnswer::error(text)
        }
        (None, Some(value)) => {
            add(value);
            agents::mcp::ToolAnswer::text(text)
        }
        (None, None) => {
            if text.is_empty() {
                text = "done".to_string();
            }
            agents::mcp::ToolAnswer::text(text)
        }
    }
}

/// What an agent's `lua` answers: `{returned, printed}` as JSON, the
/// script's return value and its printed lines as one text, or
/// `{error, printed}` as an error when something stopped it.
fn script_answer(output: scripting::RunOutput) -> agents::mcp::ToolAnswer {
    let printed = output.printed.join("\n");
    let text = |value: Value| serde_json::to_string_pretty(&value).unwrap_or_default();
    match output.error {
        Some(error) => {
            agents::mcp::ToolAnswer::error(text(json!({"error": error, "printed": printed})))
        }
        None => agents::mcp::ToolAnswer::text(text(json!({
            "returned": output.returned.unwrap_or(Value::Null),
            "printed": printed,
        }))),
    }
}

/// A script's return value as one console line: compact JSON, cut short
/// past a few hundred characters.
fn short_json(value: &Value) -> String {
    const MOST: usize = 300;
    let text = value.to_string();
    match text.char_indices().nth(MOST) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

/// A `doc.rebuild` waiting on the kernel.
pub(crate) struct RebuildWait {
    reply: std::sync::mpsc::Sender<CommandResult>,
    deadline: std::time::Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::datum_points as points;
    use core_document::{BodyId, FeatureId};

    /// One of the document's own commands, its arguments checked first.
    fn run(
        doc: &mut core_document::Document,
        registry: &core_document::DocumentService,
        id: &str,
        args: Value,
    ) -> CommandResult {
        let Value::Object(args) = args else {
            panic!("arguments are an object");
        };
        doc_commands()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
            .check(&args)
            .unwrap();
        document_command(id, &args, doc, registry, None).unwrap()
    }

    #[test]
    fn faces_are_listed_with_a_point_on_each_and_what_surface_it_is() {
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 10.0, 0.0],
                [0.0, 10.0, 0.0],
                [0.0, 0.0, 5.0],
                [1.0, 0.0, 5.0],
                [0.0, 1.0, 5.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6],
            faces: vec![0, 0, 1],
            face_surfaces: vec![
                kernel_api::FaceSurface::Plane {
                    origin: [0.0; 3],
                    normal: [0.0, 0.0, -1.0],
                },
                kernel_api::FaceSurface::Cylinder {
                    origin: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 3.0,
                },
            ],
            ..Default::default()
        };
        let faces = faces_of(&mesh);
        let faces = faces.as_array().unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0]["kind"], "plane");
        assert_eq!(faces[0]["normal"], json!([0.0, 0.0, -1.0]));
        assert!((faces[0]["area"].as_f64().unwrap() - 100.0).abs() < 1e-3);
        assert_eq!(faces[0]["point"][2], json!(0.0));
        assert_eq!(faces[1]["kind"], "cylinder");
        assert_eq!(faces[1]["axis"]["direction"], json!([0.0, 0.0, 1.0]));
        assert_eq!(faces[1]["radius"], json!(3.0));
    }

    /// An agent's script answers what it returned beside what it printed,
    /// or what stopped it.
    #[test]
    fn a_script_answers_the_agent_its_return_value_and_output() {
        let answer = |output: scripting::RunOutput| {
            let answer = script_answer(output);
            let agents::mcp::Content::Text(text) = &answer.content[0] else {
                panic!("text");
            };
            (
                answer.is_error,
                serde_json::from_str::<Value>(text).unwrap(),
            )
        };
        let (failed, body) = answer(scripting::RunOutput {
            printed: vec!["one".into(), "two".into()],
            returned: Some(json!({"pad": "f-1", "volume": 1000.0})),
            ..Default::default()
        });
        assert!(!failed);
        assert_eq!(
            body,
            json!({"returned": {"pad": "f-1", "volume": 1000.0}, "printed": "one\ntwo"})
        );
        let (_, body) = answer(scripting::RunOutput::default());
        assert_eq!(body, json!({"returned": null, "printed": ""}));
        let (failed, body) = answer(scripting::RunOutput {
            error: Some("design.pad: no sketch".into()),
            ..Default::default()
        });
        assert!(failed);
        assert_eq!(body["error"], "design.pad: no sketch");
    }

    /// Where the reference sits in `docs/SCRIPTING.md`.
    const BEGIN: &str = "<!-- commands: generated from the registered commands -->";
    const END: &str = "<!-- /commands -->";

    #[test]
    fn the_scripting_guide_lists_every_command_as_registered() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let generated = reference(&command_specs(&registry));
        assert!(!generated.contains('\u{2014}'), "the docs use no long dash");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SCRIPTING.md");
        let text = std::fs::read_to_string(&path).unwrap();
        let (Some(start), Some(end)) = (text.find(BEGIN), text.find(END)) else {
            panic!("docs/SCRIPTING.md has no command reference markers");
        };
        let current = &text[start + BEGIN.len()..end];
        let wanted = format!("\n{}\n", generated.trim());
        if current != wanted {
            if std::env::var_os("PRINTCAD_WRITE_DOCS").is_some() {
                let updated = format!("{}{BEGIN}{wanted}{}", &text[..start], &text[end..]);
                std::fs::write(&path, updated).unwrap();
            } else {
                panic!(
                    "docs/SCRIPTING.md's command reference is out of date; \
                     PRINTCAD_WRITE_DOCS=1 cargo test -p app_shell scripting_guide rewrites it"
                );
            }
        }
    }

    /// Every design a release saved in `tests/corpus/<version>/` (each
    /// recipe's blocks, run then), rebuilt from its features by this one,
    /// measures as that release measured it in the `.json` beside it. The
    /// expected numbers are the saving release's and never follow a later
    /// one's output: a change that moves them changes what old files mean.
    /// `PRINTCAD_WRITE_CORPUS=1` saves this version's designs.
    #[test]
    fn every_corpus_design_rebuilds_as_its_release_measured_it() {
        const MEASURE: &str = r#"
            pc.doc.rebuild(); local m = {}
            for _, b in ipairs(pc.doc.bodies()) do
                local ok, got = pcall(pc.doc.measure, {body = b.id})
                if ok then
                    m[b.name] = {volume = got.volume, area = got.area, centre = got.centre,
                                 min = got.min, max = got.max}
                end
            end
            return m
        "#;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        if std::env::var_os("PRINTCAD_WRITE_CORPUS").is_some() {
            let here = root.join(env!("CARGO_PKG_VERSION"));
            std::fs::create_dir_all(&here).unwrap();
            let recipes = root.join("../../../../docs/recipes");
            for file in std::fs::read_dir(&recipes).unwrap().flatten() {
                let stem = file
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let text = std::fs::read_to_string(file.path()).unwrap();
                for (n, block) in lua_blocks(&text).iter().enumerate() {
                    let script = format!("{block}\n{MEASURE}");
                    let (mut document, out) = crate::headless::run_in_document(
                        &mut registry,
                        core_document::Document::new("Corpus"),
                        &script,
                        &stem,
                    );
                    let path = here.join(format!("{stem}-{}.prtcad", n + 1));
                    document
                        .save_to_file(&path, core_document::Compression::Zstd)
                        .unwrap();
                    let expected = serde_json::to_string_pretty(&out.unwrap()).unwrap();
                    std::fs::write(path.with_extension("json"), expected + "\n").unwrap();
                }
            }
        }
        let (mut checked, mut wrong) = (0, Vec::new());
        for release in std::fs::read_dir(&root).expect("tests/corpus").flatten() {
            for file in std::fs::read_dir(release.path()).unwrap().flatten() {
                let path = file.path();
                if path.extension().is_none_or(|e| e != "prtcad") {
                    continue;
                }
                let name = path.strip_prefix(&root).unwrap().display().to_string();
                let expected: serde_json::Value = serde_json::from_str(
                    &std::fs::read_to_string(path.with_extension("json")).unwrap(),
                )
                .unwrap();
                let document = crate::app::doc_io::opened(
                    &registry,
                    core_document::Document::load_from_file(&path).unwrap(),
                );
                let (_, got) =
                    crate::headless::run_in_document(&mut registry, document, MEASURE, &name);
                match got {
                    Ok(Some(got)) => {
                        if let Some(at) = differs(&expected, &got, String::new()) {
                            wrong.push(format!("{name}: {at}"));
                        }
                    }
                    other => wrong.push(format!("{name}: {other:?}")),
                }
                checked += 1;
            }
        }
        assert!(checked > 0, "the corpus holds designs");
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Where two measures part: a number off by more than a ten-thousandth
    /// of its size (or of a millimetre), or anything else unequal.
    fn differs(want: &Value, got: &Value, at: String) -> Option<String> {
        match (want, got) {
            (Value::Number(w), Value::Number(g)) => {
                let (w, g) = (w.as_f64().unwrap(), g.as_f64().unwrap());
                ((w - g).abs() > 1e-4 * w.abs().max(1.0)).then(|| format!("{at}: {w} became {g}"))
            }
            (Value::Object(w), Value::Object(g)) if w.len() == g.len() => {
                w.iter().find_map(|(k, v)| {
                    g.get(k).map_or(Some(format!("{at}.{k}: gone")), |gv| {
                        differs(v, gv, format!("{at}.{k}"))
                    })
                })
            }
            (Value::Array(w), Value::Array(g)) if w.len() == g.len() => w
                .iter()
                .zip(g)
                .enumerate()
                .find_map(|(i, (w, g))| differs(w, g, format!("{at}[{i}]"))),
            _ => (want != got).then(|| format!("{at}: {want} became {got}")),
        }
    }

    /// What a known failure's block starts with (its fence reads
    /// ```` ```lua known-failure: <why> ````).
    const KNOWN_FAILURE: &str = "-- known-failure";

    /// The ```lua blocks of a markdown text, a known failure's marked.
    fn lua_blocks(text: &str) -> Vec<String> {
        let mut blocks = Vec::new();
        let mut open: Option<String> = None;
        for line in text.lines() {
            let fence = line.trim_start();
            match open.as_mut() {
                None if fence == "```lua" => open = Some(String::new()),
                None if fence.starts_with("```lua known-failure") => {
                    open = Some(format!("{KNOWN_FAILURE}\n"));
                }
                Some(_) if line.trim_start() == "```" => blocks.extend(open.take()),
                Some(block) => {
                    block.push_str(line);
                    block.push('\n');
                }
                None => {}
            }
        }
        blocks
    }

    #[test]
    fn markdown_lua_blocks_are_found_whole() {
        let text =
            "Text\n```lua\nlocal a = 1\nassert(a)\n```\n```\nnot lua\n```\n```lua\nb()\n```\n";
        assert_eq!(lua_blocks(text), ["local a = 1\nassert(a)\n", "b()\n"]);
    }

    /// Every example of every command, and every Lua block of every recipe
    /// under `docs/recipes/`, runs from an empty document as `printcad
    /// --script` runs a file, and its asserts hold. Undone as one step,
    /// what it did leaves the document as empty as it found it: every edit
    /// it made has an inverse that takes it back exactly. A script that
    /// imports (a barrier no undo crosses) is left out of that.
    #[test]
    fn every_command_example_and_recipe_runs() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let mut runs: Vec<(String, String, String)> = command_specs(&registry)
            .into_iter()
            .flat_map(|c| {
                c.examples
                    .into_iter()
                    .map(move |e| (c.id.clone(), e.title, e.script))
            })
            .collect();
        let recipes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/recipes");
        let mut files: Vec<_> = std::fs::read_dir(&recipes)
            .expect("docs/recipes")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "docs/recipes has recipes");
        let docs = recipes.parent().unwrap();
        files.push(docs.join("REFERENCE_PARTS.md"));
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let name = format!("docs/{}", file.strip_prefix(docs).unwrap().display());
            assert!(
                !text.contains('\u{2014}'),
                "{name}: the docs use no long dash"
            );
            let blocks = lua_blocks(&text);
            assert!(!blocks.is_empty(), "{name} has a lua block");
            for (n, block) in blocks.into_iter().enumerate() {
                runs.push((name.clone(), format!("block {}", n + 1), block));
            }
        }
        // The scripts run on every core: each worker takes the next one and
        // runs it from an empty document in the worker's own registry. Files
        // a script writes go to `os.tmpname()`, so no two collide.
        let next = std::sync::atomic::AtomicUsize::new(0);
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
        let mut failed: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        let mut registry = core_document::DocumentService::default();
                        workbenches::register_all_workbenches(&mut registry).unwrap();
                        let mut failed = Vec::new();
                        while let Some((owner, title, script)) =
                            runs.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                        {
                            // A known failure fails until it is fixed, and
                            // then asks for its mark to come off.
                            let known = script.starts_with(KNOWN_FAILURE);
                            match crate::headless::run_in_empty_document(
                                &mut registry,
                                script,
                                title,
                            ) {
                                Err(_) if known => {}
                                Ok(_) if known => failed.push(format!(
                                    "{owner}, \"{title}\": passes now; take off its known-failure mark"
                                )),
                                Err(error) => failed.push(format!("{owner}, \"{title}\": {error}")),
                                Ok(document) => {
                                    if let Some(left) = undone_leaves(document) {
                                        failed.push(format!(
                                            "{owner}, \"{title}\": undone, it leaves {left}"
                                        ));
                                    }
                                }
                            }
                        }
                        failed
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        failed.sort();
        assert!(
            failed.is_empty(),
            "examples that fail:\n{}",
            failed.join("\n")
        );
    }

    /// What a script's edits, undone as one step, leave behind that an
    /// empty document does not hold: the first path where they differ.
    /// `None` when the undo takes everything back, or when an import made
    /// the edits a barrier undo does not cross.
    fn undone_leaves(mut document: core_document::Document) -> Option<String> {
        let mut journal = core_document::history::OpJournal::new(1);
        journal.note(&mut document);
        journal.undo(&mut document)?;
        let left = document.replicated_projection();
        // An empty document of its own: the same id, nothing else in it.
        let mut empty = core_document::Document::new("Untitled").replicated_projection();
        empty["metadata"]["id"] = left["metadata"]["id"].clone();
        first_difference(&empty, &left, String::new())
    }

    fn first_difference(a: &Value, b: &Value, at: String) -> Option<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => x.keys().chain(y.keys()).find_map(|k| {
                let none = Value::Null;
                first_difference(
                    x.get(k).unwrap_or(&none),
                    y.get(k).unwrap_or(&none),
                    format!("{at}/{k}"),
                )
            }),
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
                .iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| first_difference(p, q, format!("{at}/{i}"))),
            _ if a == b => None,
            _ => Some(format!("{at}: {b} (empty: {a})")),
        }
    }

    #[test]
    fn host_ui_edits_record_as_the_document_s_commands() {
        use crate::ui::{TreeFeatureCommand, UiCommand};
        let body = BodyId(Uuid::new_v4());
        let feature = FeatureId(Uuid::new_v4());
        let rename = recorded_of(&UiCommand::RenameTreeItem {
            item: TreeItemId::Body(body),
            name: "Frame".into(),
        })
        .unwrap();
        assert_eq!(rename.id, "doc.rename");
        assert_eq!(rename.args["name"], json!("Frame"));
        let hide = recorded_of(&UiCommand::TreeFeature {
            feature,
            command: TreeFeatureCommand::SetVisible(false),
        })
        .unwrap();
        assert_eq!(
            (hide.id.as_str(), &hide.args["visible"]),
            ("doc.set_visible", &json!(false))
        );
        let delete = recorded_of(&UiCommand::DeleteTreeItem(TreeItemId::Feature(feature))).unwrap();
        assert_eq!(delete.args["id"], json!(feature.0.to_string()));
        assert!(recorded_of(&UiCommand::FitView).is_none());
        let mut more = Vec::new();
        for command in [
            TreeFeatureCommand::Suppress(true),
            TreeFeatureCommand::MoveUp,
            TreeFeatureCommand::ClearTip,
        ] {
            more.push(recorded_of(&UiCommand::TreeFeature { feature, command }).unwrap());
        }
        more.push(recorded_of(&UiCommand::RepairShapes(vec![body])).unwrap());
        more.push(recorded_of(&UiCommand::ConvertToSolid(vec![body])).unwrap());
        more.push(recorded_of(&UiCommand::RefineShapes(vec![body])).unwrap());
        assert_eq!(
            more.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            [
                "doc.suppress",
                "doc.move",
                "doc.set_tip",
                "doc.repair",
                "doc.convert_to_solid",
                "doc.refine"
            ]
        );
        // Every call a recording can hold is a command a script can call.
        for call in [rename, hide, delete].into_iter().chain(more) {
            assert!(doc_commands().iter().any(|c| c.id == call.id));
            let spec = doc_commands()
                .into_iter()
                .find(|c| c.id == call.id)
                .unwrap();
            spec.check(&call.args).unwrap();
        }
    }

    #[test]
    fn suppressing_a_tree_selection_is_one_undo_step_recorded_per_feature() {
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let features = points(&mut doc, body, &["a", "b", "c"]);
        let mut journal = core_document::history::OpJournal::new(16);
        journal.note(&mut doc);
        let suppressed = |doc: &core_document::Document| {
            features
                .iter()
                .map(|f| doc.get_feature_meta(*f).unwrap().suppressed)
                .collect::<Vec<_>>()
        };

        let chosen = [features[0], features[2]];
        suppress_all(&mut doc, &chosen, true);
        journal.label_next("Suppress features");
        journal.note(&mut doc);
        assert_eq!(suppressed(&doc), [true, false, true]);
        assert!(journal.undo(&mut doc).is_some());
        assert_eq!(
            suppressed(&doc),
            [false, false, false],
            "one undo puts both back"
        );
        assert!(journal.redo(&mut doc).is_some());
        assert_eq!(suppressed(&doc), [true, false, true]);

        // A recording replays it as the document's own command, per feature.
        let call = suppress_call(chosen[1], true);
        let spec = doc_commands()
            .into_iter()
            .find(|c| c.id == call.id)
            .unwrap();
        spec.check(&call.args).unwrap();
        assert_eq!(call.args["id"], json!(chosen[1].0.to_string()));
    }

    /// The first feature can go no earlier, and the move says so.
    #[test]
    fn moving_the_first_feature_up_is_refused() {
        let registry = core_document::DocumentService::default();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let b = points(&mut doc, body, &["a", "b"])[1];
        let up = |id: FeatureId| json!({"id": id.0.to_string(), "up": true});
        assert_eq!(
            run(&mut doc, &registry, "doc.move", up(b)).unwrap(),
            json!(true)
        );
        let stuck = run(&mut doc, &registry, "doc.move", up(b)).unwrap_err();
        assert!(stuck.to_string().contains("already first"), "{stuck}");
        let order: Vec<String> = features_in_order(&doc, Some(body))
            .iter()
            .map(|n| n.name.clone())
            .collect();
        assert_eq!(order, ["b", "a"]);
    }

    #[test]
    fn a_body_menu_s_edits_are_commands() {
        let registry = core_document::DocumentService::default();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let mesh = kernel_api::TriMesh {
            positions: vec![[0.0; 3], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0], [4.0, 4.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 1, 3, 2],
            faces: vec![0, 1],
            face_names: vec![11, 12],
            ..Default::default()
        };
        doc.set_imported_geometry(body, crate::test_support::geometry(mesh));
        let run = |doc: &mut core_document::Document, id: &str, args: Value| {
            run(doc, &registry, id, args)
        };
        let id = body.0.to_string();
        run(
            &mut doc,
            "doc.set_body",
            json!({"body": id, "frozen": true, "selectable": false, "opacity": 0.5,
                   "material": {"name": "PETG", "density": 1.27},
                   "translation": [1, 2, 3]}),
        )
        .unwrap();
        let entry = doc.bodies()[0].clone();
        assert!(entry.frozen && entry.unselectable);
        assert_eq!(entry.display.unwrap().opacity, 0.5);
        assert_eq!(entry.material.unwrap().name, "PETG");
        assert_eq!(entry.placement.translation, [1.0, 2.0, 3.0]);
        run(
            &mut doc,
            "doc.set_face_color",
            json!({"body": id, "face": 1, "color": {"r": 1, "g": 0, "b": 0}}),
        )
        .unwrap();
        assert_eq!(doc.bodies()[0].face_colors[0].name, 12);
        assert!(
            run(
                &mut doc,
                "doc.set_face_color",
                json!({"body": id, "face": 5})
            )
            .is_err(),
            "a face the body lacks"
        );
        run(
            &mut doc,
            "doc.set_body",
            json!({"body": id, "face_colors": []}),
        )
        .unwrap();
        assert!(doc.bodies()[0].face_colors.is_empty());
        let copy = run(&mut doc, "doc.linked_copy", json!({"body": id})).unwrap();
        let copy = BodyId(uuid::Uuid::parse_str(copy.as_str().unwrap()).unwrap());
        assert_eq!(
            doc.bodies().iter().find(|b| b.id == copy).unwrap().copy_of,
            Some(body)
        );
        assert!(
            doc.body_placement(copy).translation[0] > 4.0,
            "beside the original"
        );
        // What the menus record is a call a script can make.
        for edit in [
            crate::ui::BodyEdit::Frozen(false),
            crate::ui::BodyEdit::Selectable(true),
            crate::ui::BodyEdit::Recompute,
        ] {
            let (id, args) = body_edit_call(body, &edit);
            run(&mut doc, id, args).unwrap();
        }
        assert!(!doc.bodies()[0].frozen);
    }

    #[test]
    fn features_move_after_another() {
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let ids = points(&mut doc, body, &["a", "b", "c", "d"]);
        let order = |doc: &core_document::Document| -> Vec<String> {
            features_in_order(doc, Some(body))
                .iter()
                .map(|n| n.name.clone())
                .collect()
        };
        move_after(&mut doc, ids[0], Some(ids[2])).unwrap();
        assert_eq!(order(&doc), ["b", "c", "a", "d"]);
        move_after(&mut doc, ids[3], Some(ids[1])).unwrap();
        assert_eq!(order(&doc), ["b", "d", "c", "a"]);
        move_after(&mut doc, ids[0], None).unwrap();
        assert_eq!(order(&doc), ["a", "b", "d", "c"]);

        // A drop above a row goes before it, below it after it.
        let place = |doc: &core_document::Document, target, before| {
            drop_place(doc, target, before).unwrap()
        };
        assert_eq!(
            place(&doc, ids[0], true),
            None,
            "above the first: the start"
        );
        assert_eq!(place(&doc, ids[3], true), Some(ids[1]), "above d: after b");
        assert_eq!(place(&doc, ids[3], false), Some(ids[3]));
        let after = place(&doc, ids[1], true);
        move_after(&mut doc, ids[2], after).unwrap();
        assert_eq!(order(&doc), ["a", "c", "b", "d"], "c dropped above b");
    }

    #[test]
    fn the_application_s_command_ids_are_unique() {
        let mut ids: Vec<String> = doc_commands().into_iter().map(|c| c.id).collect();
        ids.extend(app_commands().into_iter().map(|c| c.id));
        ids.extend(key_commands().map(|(c, _)| c.id));
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
