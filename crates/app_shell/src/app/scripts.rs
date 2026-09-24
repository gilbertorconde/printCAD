//! Scripts and the console: the Lua engine run against every command the
//! application and the workbenches offer.
//!
//! The application's own commands are the document's queries and edits
//! (`doc.*`) and every command a key can run (`file.save`, `view.front`,
//! ...). A workbench's command runs in the workbench through
//! `Workbench::run_command`, with the same context its tools get, so the
//! host never needs to know what the command does. Every call's arguments
//! are checked against its spec before it runs.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
};
use serde_json::{Value, json};
use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::app::workbench_host::HookSite;
use crate::console::{self, LineKind};
use crate::ui::TreeItemId;
use crate::ui::keymap::{self, HostOutcome, HostState};

/// The application's own commands beyond the keyboard's.
pub(crate) fn doc_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("doc.info", "The document's name, file and unit")
            .returns("{name, file, unit, modified}")
            .read_only(),
        CommandSpec::new("doc.bodies", "List the bodies")
            .returns("a list of {id, name, visible, features}")
            .read_only(),
        CommandSpec::new("doc.features", "List the features in build order")
            .optional("body", ParamKind::Id, "Only this body's")
            .returns("a list of {id, name, kind, body, visible, suppressed, error}")
            .read_only(),
        CommandSpec::new("doc.feature", "A feature with its fields")
            .param("id", ParamKind::Id, "")
            .returns("{id, name, kind, body, visible, fields}")
            .read_only(),
        CommandSpec::new("doc.selection", "What is selected")
            .returns("{item, body, feature}, each an id or nil")
            .read_only(),
        CommandSpec::new(
            "doc.select",
            "Select a body or a feature, as a click on its row",
        )
        .param("id", ParamKind::Id, ""),
        CommandSpec::new("doc.new_body", "Make an empty body")
            .optional("name", ParamKind::String, "")
            .returns("the body's id"),
        CommandSpec::new("doc.rename", "Rename a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("name", ParamKind::String, ""),
        CommandSpec::new("doc.set_visible", "Show or hide a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("visible", ParamKind::Bool, ""),
        CommandSpec::new("doc.delete", "Delete a body or a feature").param("id", ParamKind::Id, ""),
        CommandSpec::new(
            "doc.repair",
            "Repair the shapes the kernel's checker calls broken",
        )
        .param("bodies", ParamKind::List, "The bodies")
        .returns("nothing; pc.doc.rebuild() waits for the repair"),
        CommandSpec::new("doc.convert_to_solid", "Turn mesh bodies into solids")
            .param("bodies", ParamKind::List, "The mesh bodies")
            .returns("nothing; pc.doc.rebuild() waits for the conversion"),
        CommandSpec::new(
            "doc.suppress",
            "Leave a feature out of its body's solid, or back in",
        )
        .param("id", ParamKind::Id, "The feature")
        .optional("suppressed", ParamKind::Bool, "true (the default) or false"),
        CommandSpec::new("doc.move", "Move a feature one step in its body's history")
            .param("id", ParamKind::Id, "The feature")
            .param("up", ParamKind::Bool, "true: earlier, false: later")
            .returns(
                "whether it moved: not at the end of the history, nor past a feature it needs",
            ),
        CommandSpec::new(
            "doc.set_tip",
            "Build a body only up to a feature, or all of it again",
        )
        .param("id", ParamKind::Id, "A feature of the body")
        .optional(
            "clear",
            ParamKind::Bool,
            "true: build the whole history again",
        ),
        CommandSpec::new(
            "doc.rebuild",
            "Rebuild every solid that changed, repair or convert what was asked, and wait",
        )
        .optional("timeout", ParamKind::Number, "Seconds to wait at most (60)")
        .returns("a list of {feature, error} for every feature that failed")
        .read_only(),
        CommandSpec::new("doc.faces", "The faces of a body's solid, where it sits")
            .param("body", ParamKind::Id, "")
            .returns(
                "a list of {index, kind, point, area, normal?, axis?, radius?}: \
                 point lies on the face, normal is a flat face's outward one, \
                 axis a turned face's {point, direction}",
            )
            .read_only(),
        CommandSpec::new(
            "doc.measure",
            "A body's volume, surface area, centre and bounds",
        )
        .param("body", ParamKind::Id, "")
        .returns("{volume, area, centre, min, max, approximate}")
        .read_only(),
        CommandSpec::new(
            "doc.parameters",
            "A feature's numbers that formulas set and read",
        )
        .param("id", ParamKind::Id, "The feature")
        .returns(
            "a list of {name, key, label, kind, value, text, formula, error}: name is what \
             formulas call it (nil when they cannot), value in mm or degrees",
        )
        .read_only(),
        CommandSpec::new(
            "doc.set_formula",
            "Set one of a feature's numbers by a formula, or take the formula away",
        )
        .param("id", ParamKind::Id, "The feature")
        .param(
            "parameter",
            ParamKind::String,
            "Its name or key, as doc.parameters lists them",
        )
        .optional(
            "formula",
            ParamKind::String,
            "Such as \"Printer.wall * 2\"; nil takes it away",
        )
        .returns("{value, text, error}: what it comes to"),
        CommandSpec::new("var.new", "Make a variable set")
            .param(
                "name",
                ParamKind::String,
                "What formulas call it: Printer.nozzle",
            )
            .returns("the set's id"),
        CommandSpec::new("var.set", "Set a variable to a formula, adding it when new")
            .param("set", ParamKind::String, "The set, by name or id")
            .param("name", ParamKind::String, "")
            .param(
                "formula",
                ParamKind::String,
                "Such as \"0.4 mm\" or \"3 * Printer.nozzle\"",
            )
            .optional("comment", ParamKind::String, "")
            .returns("{value, text, error}: what it comes to"),
        CommandSpec::new("var.remove", "Take a variable out of its set")
            .param("set", ParamKind::String, "The set, by name or id")
            .param("name", ParamKind::String, ""),
        CommandSpec::new(
            "var.rename",
            "Rename a variable, and every formula that reads it",
        )
        .param("set", ParamKind::String, "The set, by name or id")
        .param("name", ParamKind::String, "")
        .param("to", ParamKind::String, ""),
        CommandSpec::new(
            "var.list",
            "The variable sets and what each variable comes to",
        )
        .optional("set", ParamKind::String, "Only this set, by name or id")
        .returns(
            "a list of {id, name, variables}, each variable {name, formula, value, text, \
                 kind, error, comment}",
        )
        .read_only(),
        CommandSpec::new("var.eval", "What a formula comes to in this document")
            .param("formula", ParamKind::String, "")
            .returns("{value, kind, text}: value in mm or degrees")
            .read_only(),
    ]
}

/// What a quantity is called in a command's answer.
fn kind_name(dim: core_document::expr::Dim) -> String {
    use core_document::expr::Dim;
    match dim {
        Dim::LENGTH => "length".into(),
        Dim::ANGLE => "angle".into(),
        Dim::NUMBER => "number".into(),
        Dim::AREA => "area".into(),
        Dim::VOLUME => "volume".into(),
        other => other.unit_text("mm"),
    }
}

/// A slot's value as a command answers it.
fn slot_json(
    result: &Result<core_document::expr::Quantity, String>,
    unit: core_document::Unit,
) -> Value {
    match result {
        Ok(q) => json!({
            "value": q.value,
            "kind": kind_name(q.dim),
            "text": q.display(unit, 4),
            "error": Value::Null,
        }),
        Err(why) => json!({"value": Value::Null, "text": Value::Null, "error": why}),
    }
}

/// A variable set named or given by id.
fn set_arg(document: &core_document::Document, a: &Args) -> Result<FeatureId, CommandError> {
    let given = a.string("set")?;
    let by_id = Uuid::parse_str(given).ok().map(FeatureId).filter(|id| {
        document
            .get_feature_meta(*id)
            .is_some_and(|n| n.workbench_id.as_str() == core_document::VARIABLES_KIND)
    });
    by_id
        .or_else(|| {
            document.object_named(given).filter(|id| {
                document
                    .get_feature_meta(*id)
                    .is_some_and(|n| n.workbench_id.as_str() == core_document::VARIABLES_KIND)
            })
        })
        .ok_or_else(|| CommandError::failed(format!("no variable set is called {given}")))
}

/// What slot `key` of feature `id` comes to now.
fn slot_answer(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    id: FeatureId,
    key: &str,
) -> Value {
    registry.evaluate(document);
    let unit = document.display_unit();
    document
        .evaluated_slots(id)
        .iter()
        .find(|s| s.key == key)
        .map(|s| slot_json(&s.result, unit))
        .unwrap_or(Value::Null)
}

/// The commands a key can run that make sense without a key: all but
/// those that open a window of the interface's own.
fn key_commands() -> impl Iterator<Item = (CommandSpec, keymap::HostAction)> {
    use keymap::HostAction::*;
    keymap::host_actions()
        .filter(|(_, _, action)| {
            !matches!(
                action,
                Palette
                    | Preferences
                    | Console
                    | Assistant
                    | RunScript
                    | Record
                    | Delete
                    | ToggleVisibility
                    | PivotAtCursor
            )
        })
        .map(|(id, label, action)| {
            let spec = with_file_args(CommandSpec::new(id, label), action);
            // The view's commands move the camera, not the document.
            let spec = if id.starts_with("view.") {
                spec.read_only()
            } else {
                spec
            };
            (spec, action)
        })
}

/// A file command takes the file by name, to run without its dialog.
fn with_file_args(spec: CommandSpec, action: keymap::HostAction) -> CommandSpec {
    use keymap::HostAction::*;
    let path = |spec: CommandSpec, doc: &str| spec.optional("path", ParamKind::String, doc);
    match action {
        Open => path(spec, "The document to open; the dialog when left out")
            .returns("nothing; the document opens in its tab after the script"),
        SaveAs => path(spec, "Where to save; the dialog when left out"),
        Import => path(
            spec,
            "The STEP, IGES, STL, OBJ or 3MF file; the dialog when left out",
        )
        .returns("nothing; pc.doc.rebuild() waits for the import"),
        Export => path(spec, "Where to write; the dialog when left out")
            .optional(
                "format",
                ParamKind::String,
                "step, stl or 3mf; from the path's extension when left out",
            )
            .optional(
                "bodies",
                ParamKind::List,
                "The bodies to write; every visible one when left out",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "The mesh formats' distance to the true surface, mm (0.01)",
            )
            .returns("{path, written, skipped, triangles}"),
        _ => spec,
    }
}

/// The application's commands that are not a key's.
fn app_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("app.workbenches", "The workbenches, in the order they load")
            .returns("a list of {id, label, active}")
            .read_only(),
        CommandSpec::new("app.workbench", "Switch to a workbench").param(
            "id",
            ParamKind::String,
            "As app.workbenches lists it",
        ),
        CommandSpec::new("app.tool", "Start a toolbar tool, as a click on it does").param(
            "id",
            ParamKind::String,
            "The tool's id, as Preferences › Keyboard lists it",
        ),
        CommandSpec::new("app.tools", "The tools of a workbench")
            .optional(
                "workbench",
                ParamKind::String,
                "The active one when left out",
            )
            .returns("a list of {id, label, keys}")
            .read_only(),
    ]
}

/// The commands a script can call, by the application and then the
/// workbenches in registration order.
fn all_commands(app: &PrintCadApp) -> Vec<CommandSpec> {
    command_specs(&app.registry)
}

/// Every command a script in the application can call: the application's,
/// then each workbench's in registration order.
pub(crate) fn command_specs(registry: &core_document::DocumentService) -> Vec<CommandSpec> {
    let mut out = doc_commands();
    out.extend(app_commands());
    out.extend(key_commands().map(|(spec, _)| spec));
    out.extend(registry.commands().into_iter().map(|(_, c)| c.clone()));
    out
}

/// The command reference `docs/SCRIPTING.md` carries: every command, by
/// the name before its first dot.
#[cfg(test)]
pub(crate) fn reference(commands: &[CommandSpec]) -> String {
    let mut groups: Vec<&str> = Vec::new();
    for c in commands {
        let group = c.id.split('.').next().unwrap_or("");
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    let mut out = String::new();
    for group in groups {
        out.push_str(&format!("\n### {group}\n"));
        for c in commands
            .iter()
            .filter(|c| c.id.split('.').next() == Some(group))
        {
            out.push_str(&format!(
                "\n`pc.{}`: {}.\n",
                c.id,
                c.summary.trim_end_matches('.')
            ));
            let mut lines: Vec<String> = c
                .params
                .iter()
                .map(|p| {
                    let optional = if p.required { "" } else { ", optional" };
                    let doc = if p.doc.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", p.doc)
                    };
                    format!("- `{}` ({}{optional}){doc}", p.name, p.kind.name())
                })
                .collect();
            if let Some(extra) = &c.extra_args {
                lines.push(format!("- Other arguments: {extra}"));
            }
            if c.returns != "nothing" {
                lines.push(format!("- Returns {}", c.returns));
            }
            if !lines.is_empty() {
                out.push('\n');
                out.push_str(&lines.join("\n"));
                out.push('\n');
            }
        }
    }
    out
}

impl PrintCadApp {
    /// Run command `id` for a script, its arguments checked against its
    /// spec: the application's own here, a workbench's in the workbench.
    fn execute_command(
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
            return self.run_key_command(action, event_loop);
        }
        let Some((bench, spec)) = self.registry.command(id) else {
            return Err(CommandError::Unknown(id.to_string()));
        };
        spec.check(&args)?;
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

    pub(crate) fn submit_script(&mut self, job: scripting::Job, kind: RunKind) {
        let commands = command_specs(&self.registry);
        self.script_runs.push_back(ScriptRun {
            tab: self.session.tab,
            kind,
            label: match &job {
                scripting::Job::Line(_) => "a console line".to_string(),
                scripting::Job::Script { name, .. } => name.clone(),
                scripting::Job::Command { id, .. } => id.clone(),
            },
        });
        self.script_thread.submit(job, commands);
        self.redraw_needed = true;
    }

    fn run_app_command(&mut self, id: &str, args: &CommandArgs) -> CommandResult {
        let a = Args(args);
        match id {
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
                let detail = self.last_step_import_detail.clone();
                self.import_step_at(&path, detail);
                Ok(Value::Null)
            }
            Export => {
                let format = export_format(a.opt_string("format")?, &path)?;
                let bodies = body_list(args.get("bodies"))?;
                let tolerance = a.opt_number("tolerance")?.map(|t| t as f32);
                let (path, exported) = crate::app::export::export_document(
                    &self.session.document,
                    path,
                    format,
                    bodies,
                    tolerance,
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
        match std::fs::read_to_string(path) {
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
            Event::Call { id, args, reply } => {
                if id == "doc.rebuild" {
                    self.start_rebuild(args, reply);
                    return;
                }
                let answer = self
                    .in_script_tab(|app| app.execute_command(&id, args, event_loop))
                    .unwrap_or_else(|| {
                        Err(CommandError::failed("the script's document was closed"))
                    });
                let _ = reply.send(answer);
            }
            Event::Finished { label, output } => {
                self.in_script_tab(|app| {
                    app.session.journal.hold(false);
                    app.close_gesture();
                });
                self.script_runs.pop_front();
                if let Some(RunKind::Agent { reply }) = kind {
                    let _ = reply.send(agent_answer(output));
                    self.redraw_needed = true;
                    return;
                }
                if let Some(value) = output.value {
                    console::push(LineKind::Value, value);
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
            app.drive_mesh_solids();
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
        let Some(wait) = &self.script_rebuild else {
            return;
        };
        let answer = if self.kernel_worker.in_flight() == 0 {
            let errors = self
                .in_script_tab(|app| {
                    features_in_order(&app.session.document, None)
                        .into_iter()
                        .filter_map(|n| {
                            n.error.as_ref().map(|e| {
                                json!({"feature": n.id.0.to_string(), "name": n.name, "error": e})
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(Value::Array(errors))
        } else if std::time::Instant::now() > wait.deadline {
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
        if let Err(err) = std::process::Command::new("xdg-open").arg(&target).spawn() {
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
                    TreeItemId::DocumentRoot => {
                        return Err(CommandError::bad("id", "cannot be hidden"));
                    }
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.repair" | "doc.convert_to_solid" => {
                let bodies = body_list(args.get("bodies"))?.unwrap_or_default();
                let command = if id == "doc.repair" {
                    crate::ui::UiCommand::RepairShapes(bodies)
                } else {
                    crate::ui::UiCommand::ConvertToSolid(bodies)
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.delete" => {
                let item = tree_item(&self.session.document, a.id("id")?)?;
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

/// What a host UI command does, as the command a recording says it with:
/// renaming, showing or hiding and deleting tree rows. The benches record
/// their own.
pub(crate) fn recorded_of(command: &crate::ui::UiCommand) -> Option<core_document::Recorded> {
    use crate::ui::{TreeFeatureCommand, UiCommand};
    let call = |id: &str, args: Value| core_document::Recorded {
        id: id.to_string(),
        args: match args {
            Value::Object(map) => map,
            _ => CommandArgs::new(),
        },
        result: Value::Null,
    };
    match command {
        UiCommand::RenameTreeItem { item, name } => {
            let id = item_id(*item)?;
            Some(call(
                "doc.rename",
                json!({"id": id.to_string(), "name": name}),
            ))
        }
        UiCommand::SetBodyVisible { body, visible } => Some(call(
            "doc.set_visible",
            json!({"id": body.0.to_string(), "visible": visible}),
        )),
        UiCommand::SetImportedVisibility { node, visible } => Some(call(
            "doc.set_visible",
            json!({"id": node.to_string(), "visible": visible}),
        )),
        UiCommand::TreeFeature {
            feature,
            command: TreeFeatureCommand::SetVisible(visible),
        } => Some(call(
            "doc.set_visible",
            json!({"id": feature.0.to_string(), "visible": visible}),
        )),
        UiCommand::DeleteTreeItem(item) => {
            let id = item_id(*item)?;
            Some(call("doc.delete", json!({"id": id.to_string()})))
        }
        UiCommand::RepairShapes(bodies) | UiCommand::ConvertToSolid(bodies) => {
            let id = if matches!(command, UiCommand::RepairShapes(_)) {
                "doc.repair"
            } else {
                "doc.convert_to_solid"
            };
            let bodies: Vec<String> = bodies.iter().map(|b| b.0.to_string()).collect();
            Some(call(id, json!({"bodies": bodies})))
        }
        UiCommand::TreeFeature { feature, command } => {
            let id = feature.0.to_string();
            match command {
                TreeFeatureCommand::Suppress(on) => {
                    Some(call("doc.suppress", json!({"id": id, "suppressed": on})))
                }
                TreeFeatureCommand::Delete => Some(call("doc.delete", json!({"id": id}))),
                TreeFeatureCommand::MoveUp => Some(call("doc.move", json!({"id": id, "up": true}))),
                TreeFeatureCommand::MoveDown => {
                    Some(call("doc.move", json!({"id": id, "up": false})))
                }
                TreeFeatureCommand::SetTip => Some(call("doc.set_tip", json!({"id": id}))),
                TreeFeatureCommand::ClearTip => {
                    Some(call("doc.set_tip", json!({"id": id, "clear": true})))
                }
                TreeFeatureCommand::SetVisible(_) => None,
            }
        }
        _ => None,
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
}

/// Who asked for a script run, and where its output goes.
#[derive(Debug, Clone)]
pub(crate) enum RunKind {
    /// A console line: its output shows there.
    Console,
    /// A script file: its output shows in the console, which opens for it.
    File,
    /// An agent's tool call: its output is the answer.
    Agent {
        reply: std::sync::mpsc::Sender<agents::mcp::ToolAnswer>,
    },
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

/// A `doc.rebuild` waiting on the kernel.
pub(crate) struct RebuildWait {
    reply: std::sync::mpsc::Sender<CommandResult>,
    deadline: std::time::Instant,
}

/// The commands every host of a document answers the same way, with or
/// without a window: reading it, naming and adding bodies, and measuring.
/// `None` for any other command.
pub(crate) fn document_command(
    id: &str,
    args: &CommandArgs,
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    file: Option<&std::path::Path>,
) -> Option<CommandResult> {
    let a = Args(args);
    let answer = (|| match id {
        "doc.info" => Ok(json!({
            "name": document.name(),
            "file": file.map(|p| p.display().to_string()),
            "unit": document.display_unit().short_label(),
            "modified": document.metadata().dirty(),
        })),
        "doc.bodies" => Ok(Value::Array(
            document
                .bodies()
                .iter()
                .map(|b| {
                    json!({
                        "id": b.id.0.to_string(),
                        "name": b.name,
                        "visible": !b.hidden,
                        "features": features_in_order(document, Some(b.id))
                            .iter()
                            .map(|n| n.id.0.to_string())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )),
        "doc.features" => {
            let body = a.opt_id("body")?.map(BodyId);
            Ok(Value::Array(
                features_in_order(document, None)
                    .into_iter()
                    .filter(|n| body.is_none() || n.body == body)
                    .map(|n| {
                        json!({
                            "id": n.id.0.to_string(),
                            "name": n.name,
                            "kind": kind_of(registry, n),
                            "body": n.body.map(|b| b.0.to_string()),
                            "visible": n.visible,
                            "suppressed": n.suppressed,
                            "error": n.error,
                        })
                    })
                    .collect(),
            ))
        }
        "doc.feature" => {
            let node = document
                .get_feature_meta(FeatureId(a.id("id")?))
                .ok_or_else(|| CommandError::bad("id", "is not a feature of this document"))?;
            Ok(json!({
                "id": node.id.0.to_string(),
                "name": node.name,
                "kind": kind_of(registry, node),
                "body": node.body.map(|b| b.0.to_string()),
                "visible": node.visible,
                "fields": node.data,
            }))
        }
        "doc.suppress" => {
            let feature = feature_arg(document, &a)?;
            suppress(document, feature, a.opt_bool("suppressed")?.unwrap_or(true));
            Ok(Value::Null)
        }
        "doc.move" => {
            let feature = feature_arg(document, &a)?;
            let up = a.opt_bool("up")?.unwrap_or(true);
            Ok(json!(move_in_history(document, feature, up)))
        }
        "doc.set_tip" => {
            let feature = feature_arg(document, &a)?;
            let tip = (!a.opt_bool("clear")?.unwrap_or(false)).then_some(feature);
            set_tip(document, registry, feature, tip).map_err(CommandError::failed)?;
            Ok(Value::Null)
        }
        "doc.new_body" => {
            let body = document.create_body(None);
            if let Some(name) = a.opt_string("name")? {
                document.rename_body(body, name);
            }
            Ok(json!(body.0.to_string()))
        }
        "doc.rename" => {
            let name = a.string("name")?.to_string();
            match tree_item(document, a.id("id")?)? {
                TreeItemId::Body(body) => document.rename_body(body, name),
                TreeItemId::Feature(feature) => document.rename_feature(feature, name),
                _ => return Err(CommandError::bad("id", "cannot be renamed")),
            }
            Ok(Value::Null)
        }
        "doc.faces" => {
            let body = body_arg(document, &a)?;
            let geometry = document
                .imported_geometry(body)
                .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            Ok(faces_of(&geometry.mesh))
        }
        "doc.measure" => {
            let body = body_arg(document, &a)?;
            let (min, max) = document
                .imported_geometry(body)
                .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
                .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            let blob = document.imported_brep_blob_arc(body).ok_or_else(|| {
                CommandError::failed("the body is a mesh; it has no solid to measure")
            })?;
            let props = kernel_ogeom::OgeomKernel::new()
                .physical_properties(&blob)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let c = props.centre_mm.map(|v| v as f32);
            let centre = document.body_placement(body).point(c);
            Ok(json!({
                "volume": props.volume_mm3,
                "area": props.area_mm2,
                "centre": centre,
                "min": min,
                "max": max,
                "approximate": props.approximate,
            }))
        }
        "doc.parameters" => {
            let feature = FeatureId(a.id("id")?);
            let node = document
                .get_feature_meta(feature)
                .ok_or_else(|| CommandError::failed("no such feature"))?
                .clone();
            registry.evaluate(document);
            let unit = document.display_unit();
            let slots = document.evaluated_slots(feature);
            Ok(Value::Array(
                registry
                    .parameters(&node)
                    .into_iter()
                    .map(|p| {
                        let mut row = slots
                            .iter()
                            .find(|s| s.key == p.key)
                            .map(|s| slot_json(&s.result, unit))
                            .unwrap_or_else(|| json!({}));
                        row["name"] = json!(p.name);
                        row["key"] = json!(p.key);
                        row["label"] = json!(p.label);
                        row["kind"] = json!(kind_name(p.dim));
                        row["formula"] = json!(node.formulas.get(&p.key));
                        row
                    })
                    .collect(),
            ))
        }
        "doc.set_formula" => {
            let feature = FeatureId(a.id("id")?);
            let node = document
                .get_feature_meta(feature)
                .ok_or_else(|| CommandError::failed("no such feature"))?
                .clone();
            let wanted = a.string("parameter")?;
            let params = registry.parameters(&node);
            let parameter = params
                .iter()
                .find(|p| p.name.as_deref() == Some(wanted) || p.key == wanted)
                .ok_or_else(|| {
                    let names: Vec<String> = params
                        .iter()
                        .map(|p| p.name.clone().unwrap_or_else(|| p.key.clone()))
                        .collect();
                    CommandError::failed(format!(
                        "{} has no number {wanted}; it has {}",
                        node.name,
                        names.join(", ")
                    ))
                })?;
            let formula = a.opt_string("formula")?.map(str::to_string);
            if let Some(text) = &formula {
                core_document::expr::check_syntax(text)
                    .map_err(|e| CommandError::failed(e.message))?;
            }
            document
                .set_feature_formula(feature, parameter.key.clone(), formula)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(slot_answer(document, registry, feature, &parameter.key))
        }
        "var.new" => document
            .add_variable_set(a.string("name")?)
            .map(|id| json!(id.0.to_string()))
            .map_err(CommandError::failed),
        "var.set" => {
            let set = set_arg(document, &a)?;
            let name = a.string("name")?.to_string();
            document
                .set_variable(set, &name, a.string("formula")?, a.opt_string("comment")?)
                .map_err(CommandError::failed)?;
            Ok(slot_answer(document, registry, set, &name))
        }
        "var.remove" => {
            let set = set_arg(document, &a)?;
            document
                .remove_variable(set, a.string("name")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.rename" => {
            let set = set_arg(document, &a)?;
            document
                .rename_variable(set, a.string("name")?, a.string("to")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.list" => {
            let only = match a.opt_string("set")? {
                Some(_) => Some(set_arg(document, &a)?),
                None => None,
            };
            registry.evaluate(document);
            let unit = document.display_unit();
            Ok(Value::Array(
                document
                    .variable_sets()
                    .into_iter()
                    .filter(|(id, ..)| only.is_none_or(|o| o == *id))
                    .map(|(id, name, set)| {
                        let slots = document.evaluated_slots(id);
                        let variables: Vec<Value> = set
                            .variables
                            .iter()
                            .map(|v| {
                                let mut row = slots
                                    .iter()
                                    .find(|s| s.key == v.name)
                                    .map(|s| slot_json(&s.result, unit))
                                    .unwrap_or_else(|| json!({}));
                                row["name"] = json!(v.name);
                                row["formula"] = json!(v.formula);
                                row["comment"] = json!(v.comment);
                                row
                            })
                            .collect();
                        json!({"id": id.0.to_string(), "name": name, "variables": variables})
                    })
                    .collect(),
            ))
        }
        "var.eval" => {
            let q = registry
                .evaluate_formula(document, a.string("formula")?, None)
                .map_err(CommandError::failed)?;
            Ok(json!({
                "value": q.value,
                "kind": kind_name(q.dim),
                "text": q.display(document.display_unit(), 4),
            }))
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    })();
    match answer {
        Err(CommandError::Unknown(_)) => None,
        answer => Some(answer),
    }
}

fn feature_arg(document: &core_document::Document, a: &Args) -> Result<FeatureId, CommandError> {
    let feature = FeatureId(a.id("id")?);
    if document.get_feature_meta(feature).is_some() {
        Ok(feature)
    } else {
        Err(CommandError::bad("id", "is not a feature of this document"))
    }
}

/// Leave `feature` out of its body's solid, or put it back.
pub(crate) fn suppress(document: &mut core_document::Document, feature: FeatureId, on: bool) {
    document.set_feature_suppressed(feature, on);
    document.mark_feature_dirty(feature);
}

/// Move `feature` a step in its body's history; whether it could.
pub(crate) fn move_in_history(
    document: &mut core_document::Document,
    feature: FeatureId,
    up: bool,
) -> bool {
    document.move_feature_in_history(feature, up)
}

/// Build the body of `of` only up to `tip`, or all of it when `None`.
pub(crate) fn set_tip(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    of: FeatureId,
    tip: Option<FeatureId>,
) -> Result<(), &'static str> {
    let body = document
        .get_feature_meta(of)
        .and_then(|n| n.body)
        .ok_or("the feature belongs to no body")?;
    document.set_body_tip(body, tip);
    // The chain changes shape: rebuild from the first feature.
    registry.invalidate_body(document, body);
    Ok(())
}

fn body_arg(document: &core_document::Document, a: &Args) -> Result<BodyId, CommandError> {
    let body = BodyId(a.id("body")?);
    if document.bodies().iter().any(|b| b.id == body) {
        Ok(body)
    } else {
        Err(CommandError::bad("body", "is not a body of this document"))
    }
}

/// What kind of feature `node` is, as its workbench names it.
fn kind_of(registry: &core_document::DocumentService, node: &core_document::FeatureNode) -> String {
    registry
        .feature_info(node)
        .map(|info| info.kind_label)
        .unwrap_or_else(|| node.workbench_id.as_str().to_string())
}

/// The tree row an id names: a body, a feature or an imported part.
pub(crate) fn tree_item(
    document: &core_document::Document,
    id: Uuid,
) -> Result<TreeItemId, CommandError> {
    if document.bodies().iter().any(|b| b.id.0 == id) {
        Ok(TreeItemId::Body(BodyId(id)))
    } else if document.get_feature_meta(FeatureId(id)).is_some() {
        Ok(TreeItemId::Feature(FeatureId(id)))
    } else if document.imported_object(id).is_some() {
        Ok(TreeItemId::ImportedObject(id))
    } else {
        Err(CommandError::bad("id", "is not in this document"))
    }
}

/// The format an export names, else the one its path's extension says.
pub(crate) fn export_format(
    name: Option<&str>,
    path: &std::path::Path,
) -> Result<kernel_ogeom::export::ExportFormat, CommandError> {
    use kernel_ogeom::export::ExportFormat;
    match name {
        Some(name) => match name.to_ascii_lowercase().as_str() {
            "step" | "stp" => Ok(ExportFormat::Step),
            "stl" => Ok(ExportFormat::Stl),
            "3mf" => Ok(ExportFormat::ThreeMf),
            _ => Err(CommandError::bad("format", "must be step, stl or 3mf")),
        },
        None => ExportFormat::of_path(path).ok_or_else(|| {
            CommandError::bad(
                "format",
                "is needed when the path has no .step, .stl or .3mf",
            )
        }),
    }
}

/// A list of body ids, when one is given.
pub(crate) fn body_list(value: Option<&Value>) -> Result<Option<Vec<BodyId>>, CommandError> {
    match value {
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(BodyId)
                    .ok_or_else(|| CommandError::bad("bodies", "must be a list of body ids"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Ok(None),
    }
}

/// The features of `body` (or all of them) in build order.
fn features_in_order(
    document: &core_document::Document,
    body: Option<BodyId>,
) -> Vec<&core_document::FeatureNode> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n)
        .filter(|n| body.is_none() || n.body == body)
        .collect();
    nodes.sort_by_key(|n| n.seq);
    nodes
}

/// Each face of a mesh: what surface it is, a point on it, its area, and a
/// flat face's normal or a turned face's axis.
fn faces_of(mesh: &kernel_api::TriMesh) -> Value {
    let count = mesh.face_surfaces.len().max(
        mesh.faces
            .iter()
            .map(|f| *f as usize + 1)
            .max()
            .unwrap_or(0),
    );
    // Per face: area, the area-weighted centre, and the triangle centre
    // nearest it (a point that is on the face even when it curves).
    let mut area = vec![0.0f32; count];
    let mut centre = vec![[0.0f32; 3]; count];
    let tri = |t: usize| {
        let p = |i: usize| glam::Vec3::from(mesh.positions[mesh.indices[3 * t + i] as usize]);
        (p(0), p(1), p(2))
    };
    for (t, face) in mesh.faces.iter().enumerate() {
        let (a, b, c) = tri(t);
        let w = (b - a).cross(c - a).length() / 2.0;
        let f = *face as usize;
        area[f] += w;
        let m = (a + b + c) / 3.0;
        for i in 0..3 {
            centre[f][i] += m[i] * w;
        }
    }
    let mut on_face = vec![None::<(f32, glam::Vec3)>; count];
    for (t, face) in mesh.faces.iter().enumerate() {
        let f = *face as usize;
        if area[f] <= 0.0 {
            continue;
        }
        let target = glam::Vec3::from(centre[f]) / area[f];
        let (a, b, c) = tri(t);
        let m = (a + b + c) / 3.0;
        let d = m.distance(target);
        if on_face[f].is_none_or(|(best, _)| d < best) {
            on_face[f] = Some((d, m));
        }
    }
    let list = (0..count)
        .filter_map(|f| {
            let (_, point) = on_face[f]?;
            let surface = mesh.face_surfaces.get(f).copied().unwrap_or_default();
            let kind = match surface {
                kernel_api::FaceSurface::Plane { .. } => "plane",
                kernel_api::FaceSurface::Cylinder { .. } => "cylinder",
                kernel_api::FaceSurface::Cone { .. } => "cone",
                kernel_api::FaceSurface::Sphere { .. } => "sphere",
                kernel_api::FaceSurface::Torus { .. } => "torus",
                kernel_api::FaceSurface::Other => "other",
            };
            let mut out = json!({
                "index": f,
                "kind": kind,
                "point": point.to_array(),
                "area": area[f],
            });
            match surface {
                kernel_api::FaceSurface::Plane { normal, .. } => out["normal"] = json!(normal),
                kernel_api::FaceSurface::Cylinder { radius, .. }
                | kernel_api::FaceSurface::Sphere { radius, .. } => out["radius"] = json!(radius),
                _ => {}
            }
            if let Some((p, d)) = surface.axis() {
                out["axis"] = json!({"point": p, "direction": d});
            }
            Some(out)
        })
        .collect();
    Value::Array(list)
}

fn item_id(item: TreeItemId) -> Option<Uuid> {
    match item {
        TreeItemId::Body(b) => Some(b.0),
        TreeItemId::Feature(f) => Some(f.0),
        TreeItemId::ImportedObject(n) => Some(n),
        TreeItemId::DocumentRoot => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            more.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            [
                "doc.suppress",
                "doc.move",
                "doc.set_tip",
                "doc.repair",
                "doc.convert_to_solid"
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
    fn the_tree_s_history_edits_work_on_the_document_as_commands() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let datum = |doc: &mut core_document::Document, name: &str| {
            doc.add_feature_in_body(
                core_document::DatumFeature {
                    shape: core_document::DatumShape::Point,
                    attachment: core_document::DatumAttachment::BasePlane(
                        core_document::BasePlane::XY,
                    ),
                    offset: Default::default(),
                },
                name.to_string(),
                Some(body),
            )
            .unwrap()
        };
        let (a, b) = (datum(&mut doc, "a"), datum(&mut doc, "b"));
        let run = |doc: &mut core_document::Document, id: &str, args: Value| {
            let args = match args {
                Value::Object(map) => map,
                _ => CommandArgs::new(),
            };
            document_command(id, &args, doc, &registry, None).unwrap()
        };
        run(&mut doc, "doc.suppress", json!({"id": a.0.to_string()})).unwrap();
        assert!(doc.get_feature_meta(a).unwrap().suppressed);
        let moved = run(
            &mut doc,
            "doc.move",
            json!({"id": b.0.to_string(), "up": true}),
        )
        .unwrap();
        assert_eq!(moved, json!(true));
        let order: Vec<String> = features_in_order(&doc, Some(body))
            .iter()
            .map(|n| n.name.clone())
            .collect();
        assert_eq!(order, ["b", "a"]);
        let stuck = run(
            &mut doc,
            "doc.move",
            json!({"id": b.0.to_string(), "up": true}),
        )
        .unwrap();
        assert_eq!(stuck, json!(false), "the first can go no earlier");
        run(&mut doc, "doc.set_tip", json!({"id": b.0.to_string()})).unwrap();
        assert_eq!(doc.bodies()[0].tip, Some(b));
        run(
            &mut doc,
            "doc.set_tip",
            json!({"id": b.0.to_string(), "clear": true}),
        )
        .unwrap();
        assert_eq!(doc.bodies()[0].tip, None);
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
