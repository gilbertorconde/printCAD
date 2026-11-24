//! Running a script without a window: `printcad --script build.lua`, for
//! batch exports, checks and tests.
//!
//! The script sees the workbenches' commands and the document's own
//! (reading it, adding and naming bodies, visibility, deleting, rebuilding
//! and measuring), and `file.open`, `file.import`, `file.save_as` and `file.export`
//! with a path, an import read and landed on this thread as the app lands
//! it (`step_import::land_import`). What
//! needs a window (the selection, the view, tools) answers that it is not
//! available. Rebuilds run on this thread, so a script can build, measure
//! and export in one go.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService,
    ParamKind, WorkbenchRuntimeContext,
};
use kernel_api::TessellationSettings;
use serde_json::Value;

use crate::app::scripts::{doc_commands, document_command, tree_item};

/// What the command line asked for.
#[derive(Debug, Default, PartialEq)]
pub struct Invocation {
    pub script: PathBuf,
    /// A document to start from, instead of an empty one.
    pub open: Option<PathBuf>,
    /// Where to save the document when the script is done.
    pub save: Option<PathBuf>,
    /// The words after `--`, the script's `arg`.
    pub args: Vec<String>,
}

pub const USAGE: &str =
    "printcad --script <file.lua> [--open <document>] [--save <document>] [-- <arg>...]";

/// The headless run the command line asks for, if it asks for one.
pub fn parse(words: &[String]) -> Result<Option<Invocation>, String> {
    let mut run = Invocation::default();
    let mut script = None;
    let mut words = words.iter();
    while let Some(word) = words.next() {
        let mut value = || {
            words
                .next()
                .cloned()
                .ok_or_else(|| format!("{word} needs a value\n{USAGE}"))
        };
        match word.as_str() {
            "--script" => script = Some(PathBuf::from(value()?)),
            "--open" => run.open = Some(PathBuf::from(value()?)),
            "--save" => run.save = Some(PathBuf::from(value()?)),
            "--" => {
                run.args = words.cloned().collect();
                break;
            }
            other if script.is_some() || run.open.is_some() || run.save.is_some() => {
                return Err(format!("unknown option {other}\n{USAGE}"));
            }
            _ => {}
        }
    }
    match script {
        Some(script) => {
            run.script = script;
            Ok(Some(run))
        }
        None if run.open.is_some() || run.save.is_some() => {
            Err(format!("--open and --save need --script\n{USAGE}"))
        }
        None => Ok(None),
    }
}

/// Run `invocation`: its output on stdout, its error on stderr. Whether the
/// script finished.
pub fn run(invocation: &Invocation, registry: DocumentService) -> Result<bool> {
    let source = std::fs::read_to_string(&invocation.script)
        .with_context(|| format!("could not read {}", invocation.script.display()))?;
    let document = match &invocation.open {
        Some(path) => crate::app::doc_io::opened(
            &registry,
            Document::load_from_file(path)
                .with_context(|| format!("could not open {}", path.display()))?,
        ),
        None => Document::new("Untitled"),
    };
    let mut host = Headless {
        registry,
        document,
        file: invocation.open.clone(),
        journal: Some(core_document::history::OpJournal::new(
            crate::app::session::JOURNAL_LIMIT,
        )),
    };
    host.rebuild();
    let mut engine = scripting::ScriptEngine::new();
    engine.set_time_limit(std::time::Duration::from_secs(3600));
    let name = invocation.script.display().to_string();
    if let Err(why) = engine.set_args(&invocation.args) {
        eprintln!("{name}: its arguments could not be set: {why}");
        return Ok(false);
    }
    let out = engine.run_script_shown(&source, &name, &mut host);
    for line in &out.printed {
        println!("{line}");
    }
    // What the script returns is its answer, shown as the console shows
    // it; text (`return show(t)`) as it is.
    match (&out.returned, &out.value) {
        (Some(Value::String(text)), _) => println!("{text}"),
        (_, Some(value)) => println!("{value}"),
        _ => {}
    }
    if let Some(error) = &out.error {
        eprintln!("{name}: {error}");
        return Ok(false);
    }
    if let Some(path) = &invocation.save {
        host.rebuild();
        host.save(path)
            .with_context(|| format!("could not save {}", path.display()))?;
    }
    Ok(true)
}

/// Run `source` from an empty document with `registry`'s workbenches, the
/// way `printcad --script` runs a file but in the engine an agent's script
/// gets (no `io` or `os`), and hand the registry back: the document the
/// script left, or the error that stopped it.
#[cfg(test)]
pub(crate) fn run_in_empty_document(
    registry: &mut DocumentService,
    source: &str,
    name: &str,
) -> Result<Document, String> {
    let (document, out) = run_in_document(registry, Document::new("Untitled"), source, name);
    out.map(|_| document)
}

/// Run `source` on `document` as `run_in_empty_document` does: the
/// document it leaves, and what it returns or the error that stopped it.
/// It keeps no history: edits not yet closed in `document` are left for
/// the history that holds them.
#[cfg(test)]
pub(crate) fn run_in_document(
    registry: &mut DocumentService,
    document: Document,
    source: &str,
    name: &str,
) -> (Document, Result<Option<Value>, String>) {
    let (document, _, answer) = run_sandboxed(registry, document, source, name, None);
    (document, answer)
}

/// Run `source` on `document` as `run_in_document` does, as one run of
/// a history of its own: that history too, where undo takes the whole
/// run back, unless the run crossed an import that no undo crosses.
#[cfg(test)]
pub(crate) fn run_journaled(
    registry: &mut DocumentService,
    mut document: Document,
    source: &str,
    name: &str,
) -> (
    Document,
    Option<core_document::history::OpJournal>,
    Result<Option<Value>, String>,
) {
    let mut journal = core_document::history::OpJournal::new(crate::app::session::JOURNAL_LIMIT);
    journal.begin_run(&mut document);
    let (mut document, journal, answer) =
        run_sandboxed(registry, document, source, name, Some(journal));
    let journal = journal.filter(|j| !j.run_crossed_barrier()).map(|mut j| {
        j.end_run(&mut document);
        j
    });
    (document, journal, answer)
}

#[cfg(test)]
fn run_sandboxed(
    registry: &mut DocumentService,
    document: Document,
    source: &str,
    name: &str,
    journal: Option<core_document::history::OpJournal>,
) -> (
    Document,
    Option<core_document::history::OpJournal>,
    Result<Option<Value>, String>,
) {
    let mut host = Headless {
        registry: std::mem::take(registry),
        document,
        file: None,
        journal,
    };
    let out = scripting::ScriptEngine::sandboxed().run_script(source, name, &mut host);
    *registry = host.registry;
    let answer = match out.error {
        Some(error) => Err(error),
        None => Ok(out.returned),
    };
    (host.document, host.journal, answer)
}

/// The document and the workbenches, with no window.
struct Headless {
    registry: DocumentService,
    document: Document,
    file: Option<PathBuf>,
    /// The run's history, each call a step as a run's calls are in the
    /// window; none where another history holds the document's edits.
    journal: Option<core_document::history::OpJournal>,
}

/// The document commands a window-less run answers.
const DOC_COMMANDS: &[&str] = &[
    "doc.parameters",
    "doc.set_formula",
    "doc.set_value",
    "var.new",
    "var.set",
    "var.remove",
    "var.rename",
    "var.list",
    "var.eval",
    "config.list",
    "config.new",
    "config.remove",
    "config.rename",
    "config.add_variable",
    "config.remove_variable",
    "config.set",
    "config.leave_out",
    "config.activate",
    "doc.info",
    "doc.bodies",
    "doc.features",
    "doc.feature",
    "doc.new_body",
    "doc.coupon",
    "doc.rename",
    "doc.set_visible",
    "doc.delete",
    "doc.suppress",
    "doc.move",
    "doc.set_tip",
    "doc.rebuild",
    "doc.undo",
    "doc.redo",
    "doc.faces",
    "doc.edges",
    "doc.set_body",
    "doc.set_face_color",
    "doc.set_textures",
    "doc.note_add",
    "doc.note_set",
    "doc.note_remove",
    "doc.notes",
    "doc.notes_open",
    "doc.linked_copy",
    "doc.array",
    "doc.set_array",
    "doc.move_after",
    "doc.recompute",
    "doc.measure",
    "doc.print_layout",
    "doc.picture",
    "doc.sections",
    "doc.save_section",
    "doc.remove_section",
    "file.temp_path",
];

fn file_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(
            "file.import",
            "Read a STEP, IGES or mesh file, or a file a workbench reads, into the document",
        )
        .param("path", ParamKind::String, "The file to read")
        .optional(
            "scan",
            ParamKind::Bool,
            crate::app::scripts::SCAN_IMPORT_DOC,
        )
        .optional(
            "units",
            ParamKind::String,
            crate::app::scripts::UNITS_IMPORT_DOC,
        ),
        CommandSpec::new(
            "file.open",
            "Open a printCAD document in place of the run's document",
        )
        .param("path", ParamKind::String, "The document to open")
        .optional(
            "discard",
            ParamKind::Bool,
            crate::app::scripts::OPEN_DISCARD_DOC,
        ),
        CommandSpec::new("file.save_as", "Save the document").param(
            "path",
            ParamKind::String,
            "Where to save it",
        ),
        crate::app::scripts::export_options(
            CommandSpec::new("file.export", "Write bodies as STEP, STL or 3MF").param(
                "path",
                ParamKind::String,
                "Where to write",
            ),
        ),
    ]
}

impl scripting::Host for Headless {
    fn commands(&self) -> Vec<CommandSpec> {
        let mut out: Vec<CommandSpec> = doc_commands()
            .into_iter()
            .filter(|c| DOC_COMMANDS.contains(&c.id.as_str()))
            .collect();
        out.extend(file_commands());
        out.extend(self.registry.commands().into_iter().map(|(_, c)| c.clone()));
        out
    }

    /// Each command, then the formulas settled: what a value moved has
    /// followed before the next command reads it.
    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        if let Some(journal) = &mut self.journal {
            journal.begin_call(&mut self.document);
        }
        let answer = self.call_one(id, args);
        self.settle_formulas();
        if let Some(journal) = &mut self.journal {
            journal.note_call(&mut self.document, core_document::renamed::command(id));
        }
        answer
    }
}

impl Headless {
    /// The spec of command `id`, found as in [`scripting::Host::commands`]
    /// without making the whole list.
    fn spec(&self, id: &str) -> Option<std::borrow::Cow<'_, CommandSpec>> {
        let windowless = DOC_COMMANDS.contains(&id).then(doc_commands);
        windowless
            .into_iter()
            .flatten()
            .chain(file_commands())
            .find(|c| c.id == id)
            .map(std::borrow::Cow::Owned)
            .or_else(|| {
                self.registry
                    .command(id)
                    .map(|(_, spec)| std::borrow::Cow::Borrowed(spec))
            })
    }

    fn settle_formulas(&mut self) {
        self.registry.evaluate(&mut self.document);
        let moved = self.document.take_moved_values();
        if moved.is_empty() {
            return;
        }
        for bench in self.registry.ids().to_vec() {
            if let Ok(wb) = self.registry.workbench_mut(&bench) {
                wb.values_moved(&mut context(&mut self.document), &moved);
            }
        }
    }

    fn call_one(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        // A script may use a command's former name.
        let id = &*core_document::renamed::command(id);
        self.spec(id)
            .ok_or_else(|| match doc_commands().iter().any(|c| c.id == id) {
                true => CommandError::failed("not available without a window"),
                false => CommandError::unknown_among(
                    id,
                    scripting::Host::commands(self)
                        .iter()
                        .map(|c| c.id.as_str()),
                ),
            })?
            .check(&args)?;
        if let Some(answer) = document_command(
            id,
            &args,
            &mut self.document,
            &self.registry,
            self.file.as_deref(),
        ) {
            return answer;
        }
        let a = Args(&args);
        match id {
            "doc.rebuild" => Ok(Value::Array(self.rebuild())),
            "doc.undo" | "doc.redo" => {
                let journal = self
                    .journal
                    .as_mut()
                    .ok_or_else(|| CommandError::failed("this run keeps no history"))?;
                let label = crate::app::scripts::step_history(
                    journal,
                    &mut self.document,
                    id == "doc.redo",
                )?;
                if label.is_some() {
                    self.registry.invalidate_all(&mut self.document);
                }
                Ok(label.map_or(Value::Null, Value::String))
            }
            "doc.set_visible" => {
                let visible = a.opt_bool("visible")?.unwrap_or(true);
                let item = tree_item(&self.document, a.id("id")?)?;
                crate::app::scripts::set_item_visible(&mut self.document, item, visible)?;
                Ok(Value::Null)
            }
            "doc.delete" => {
                let item = tree_item(&self.document, a.id("id")?)?;
                crate::app::scripts::user_note_guard(&self.document, item, &a, "delete")?;
                crate::app::scripts::delete_item(
                    &mut self.document,
                    &mut self.registry,
                    item,
                    crate::app::workbench_host::WbCtxParams::windowless(),
                )?;
                Ok(Value::Null)
            }
            "doc.picture" => {
                self.rebuild();
                let document = &self.document;
                crate::app::scripts::picture_command(&args, |request| {
                    doc_render::proof::picture(
                        document,
                        request,
                        None,
                        axes::AxisSystem::default(),
                        &crate::app::scripts::picture_ink(),
                    )
                })
            }
            "file.import" => {
                let path = PathBuf::from(a.string("path")?);
                let unit = crate::app::scripts::import_units(
                    &a,
                    &path,
                    kernel_api::LengthUnit::Millimetre,
                )?;
                self.import(&path, a.opt_bool("scan")? == Some(true), unit)?;
                Ok(Value::Null)
            }
            "file.save_as" => {
                let path = PathBuf::from(a.string("path")?);
                self.rebuild();
                self.save(&path)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                self.file = Some(path);
                Ok(Value::Null)
            }
            "file.open" => {
                let path = PathBuf::from(a.string("path")?);
                if self.document.metadata().dirty() && a.opt_bool("discard")? != Some(true) {
                    return Err(CommandError::failed(
                        "the document has edits not saved: save them first \
                         (file.save_as), or pass discard = true to lose them",
                    ));
                }
                let opened = Document::load_from_file(&path)
                    .map_err(|e| CommandError::bad("path", format!("could not be opened: {e}")))?;
                self.document = crate::app::doc_io::opened(&self.registry, opened);
                self.file = Some(path);
                // The history before belongs to the document that is gone.
                if self.journal.is_some() {
                    self.journal = Some(core_document::history::OpJournal::new(
                        crate::app::session::JOURNAL_LIMIT,
                    ));
                }
                self.rebuild();
                Ok(Value::Null)
            }
            "doc.print_layout" => {
                self.rebuild();
                crate::app::scripts::print_layout_command(
                    &args,
                    &self.document,
                    &self.registry,
                    &settings::PrintingSettings::default(),
                )
            }
            "file.export" => {
                let path = PathBuf::from(a.string("path")?);
                let draft = crate::app::scripts::export_draft(&a, &path)?;
                let bodies = crate::app::scripts::body_list(args.get("bodies"))?;
                self.rebuild();
                let layout = draft.layout.then(|| {
                    use doc_render::print_layout::{LayoutSettings, lay_out, parts_to_print};
                    let parts = parts_to_print(&self.document, &self.registry);
                    let parts = match &bodies {
                        Some(bodies) => doc_export::parts_of(parts, bodies),
                        None => parts,
                    };
                    lay_out(
                        &self.document,
                        &parts,
                        &LayoutSettings::of(&settings::PrintingSettings::default()),
                    )
                });
                let folder = draft.per_body.then(|| path.clone());
                let files = doc_export::export_document(
                    (&self.document, &self.registry),
                    path,
                    &draft,
                    bodies,
                    layout.as_ref(),
                    None,
                )
                .map_err(CommandError::failed)?;
                Ok(doc_export::export_answer(folder.as_deref(), &files))
            }
            _ => {
                let (bench, _) = self
                    .registry
                    .command(id)
                    .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
                let bought_kinds = self.registry.bought_kinds();
                let wb = self
                    .registry
                    .workbench_mut(&bench)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                let mut ctx = context(&mut self.document);
                ctx.bought_kinds = bought_kinds;
                wb.run_command(id, &args, &mut ctx)
            }
        }
    }
}

fn context(document: &mut Document) -> WorkbenchRuntimeContext<'_> {
    let mut ctx = WorkbenchRuntimeContext::new(document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    ctx
}

/// Reads a shape the document takes from a file as the kernel reads files:
/// a copy of the asset under its own name, removed once read.
struct ShapeFiles;

impl build_driver::ShapeReader for ShapeFiles {
    fn read(
        &mut self,
        kernel: &mut kernel_ogeom::OgeomKernel,
        document: &Document,
        asset: uuid::Uuid,
        detail: &TessellationSettings,
    ) -> Option<kernel_api::MeshSolidResult> {
        let (reference, bytes) = document.asset_with_bytes(asset)?;
        let name = Path::new(&reference.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let shape = app_platform::scratch_file(&name, &bytes)
            .map_err(|e| e.to_string())
            .and_then(|path| {
                let shape = kernel.read_solid(&path, detail).map_err(|e| e.to_string());
                let _ = std::fs::remove_file(path);
                shape
            });
        shape
            .inspect_err(|error| eprintln!("a shape could not be read from {name}: {error}"))
            .ok()
    }
}

impl Headless {
    /// Build every solid that changed, here and now, until nothing is left
    /// to build. The failures, as `{feature, name, error}`.
    fn rebuild(&mut self) -> Vec<Value> {
        let built = build_driver::build_all(
            &mut self.document,
            &self.registry,
            &mut kernel_ogeom::OgeomKernel::new(),
            &TessellationSettings::default(),
            &mut ShapeFiles,
        );
        for note in &built.notes {
            eprintln!("{note}");
        }
        crate::app::scripts::rebuild_failures(&self.document)
    }

    /// Read `path` into the document as the app's import does: a file
    /// kind a workbench reads goes to its command; anything else the
    /// kernel reads here, on this thread, and lands as the app lands it.
    fn import(
        &mut self,
        path: &Path,
        scan: bool,
        unit: Option<kernel_api::LengthUnit>,
    ) -> Result<(), CommandError> {
        if let Some((_, import)) = self.registry.file_import_for(path) {
            let command = import.command.clone();
            let mut args = CommandArgs::new();
            args.insert("path".into(), path.display().to_string().into());
            return self.call_one(&command, args).map(drop);
        }
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("prtcad"))
        {
            return Err(CommandError::failed(
                "a printCAD file comes in as a linked part, which needs the app",
            ));
        }
        let detail = kernel_api::TessellationSettings::default();
        let mut model = kernel_ogeom::OgeomKernel::new()
            .import_step(path, &detail)
            .map_err(|e| CommandError::failed(e.to_string()))?;
        let unit = unit.unwrap_or(kernel_api::LengthUnit::Millimetre);
        crate::scans::scale_to_millimetres(&mut model, unit);
        let bytes = std::fs::read(path).map_err(|e| CommandError::failed(e.to_string()))?;
        let landed =
            crate::app::step_import::land_import(&mut self.document, path, model, bytes, detail);
        if landed.bodies.is_empty() {
            return Err(CommandError::failed("the file held no geometry"));
        }
        if scan {
            crate::scans::mark_scans(&mut self.document, &landed.bodies, unit);
        }
        Ok(())
    }

    fn save(&mut self, path: &Path) -> Result<()> {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            self.document
                .name_after_file(file_session::document_name_from_file_name(name));
        }
        let folder = |file: &Path| {
            std::path::absolute(file)
                .ok()?
                .parent()
                .map(Path::to_path_buf)
        };
        if let Some(to) = folder(path) {
            let from = self.file.as_deref().and_then(folder);
            self.document.place_links_for_file(from.as_deref(), &to);
        }
        self.document
            .save_to_file(path, core_document::Compression::None)?;
        self.document.mark_clean();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command the window-less host answers, given hostile values for
    /// each of its parameters in turn (the wrong kind, nothing, empty,
    /// negative, a fraction for a count, an id naming nothing), answers
    /// with an error or a result and never panics. Huge counts are left
    /// out: they ask for long work rather than wrong input.
    #[test]
    fn every_command_survives_hostile_arguments() {
        // Each call starts from a document holding one empty body, so what
        // the calls before it made does not slow it down.
        let mut empty = Document::new("Hostile");
        let body = Value::from(empty.create_body(Some("Body".into())).0.to_string());
        let mut host = Headless {
            registry: registry(),
            document: empty.clone(),
            file: None,
            journal: None,
        };
        let hostile = |kind: ParamKind, path: bool| -> Vec<Value> {
            let mut values = vec![
                Value::Null,
                Value::Bool(true),
                serde_json::json!([]),
                serde_json::json!({"x": [1, {"y": null}]}),
                serde_json::json!([null, "", -1]),
            ];
            // A path's string would be written to; it gets only the wrong
            // kinds and a name no system takes.
            if path {
                values.push("\u{0}".into());
                return values;
            }
            values.extend([
                Value::from(""),
                Value::from("\u{0}not an id"),
                Value::from("x".repeat(5000)),
                Value::from(uuid::Uuid::new_v4().to_string()),
                Value::from(-1),
                Value::from(0),
                Value::from(-1e300),
            ]);
            if kind == ParamKind::Integer {
                values.push(Value::from(2.5));
            }
            values
        };
        // Each parameter a command requires, filled as it asks, so the
        // hostile one is what the command meets.
        let plain = |kind: ParamKind, path: bool| -> Value {
            match kind {
                _ if path => Value::from("\u{0}"),
                ParamKind::Number => Value::from(1.0),
                ParamKind::Integer => Value::from(1),
                ParamKind::Bool => Value::Bool(true),
                ParamKind::Id => body.clone(),
                ParamKind::List => serde_json::json!([]),
                ParamKind::String | ParamKind::Any => Value::from("x"),
            }
        };
        let mut panicked = Vec::new();
        for spec in scripting::Host::commands(&host) {
            for target in &spec.params {
                for value in hostile(target.kind, target.name == "path") {
                    let mut args = CommandArgs::new();
                    for p in spec.params.iter().filter(|p| p.required) {
                        args.insert(p.name.clone(), plain(p.kind, p.name == "path"));
                    }
                    args.insert(target.name.clone(), value.clone());
                    host.document = empty.clone();
                    let call = std::panic::AssertUnwindSafe(|| {
                        let _ = scripting::Host::call(&mut host, &spec.id, args);
                    });
                    if std::panic::catch_unwind(call).is_err() {
                        panicked.push(format!("{} {}={value}", spec.id, target.name));
                    }
                }
            }
        }
        assert!(panicked.is_empty(), "panicked on: {panicked:#?}");
    }

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    fn registry() -> DocumentService {
        let mut registry = DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        registry
    }

    /// A scratch folder of the test's own.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("printcad-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs the script file as `printcad --script` does, saving where
    /// `save` says; whether its checks held.
    fn run_file(script: PathBuf, save: Option<PathBuf>, args: Vec<String>) -> bool {
        let invocation = Invocation {
            script,
            open: None,
            save,
            args,
        };
        run(&invocation, registry()).unwrap()
    }

    /// `file.import` reads a STEP file into the document as the app does,
    /// so a recording or a crash report that imports replays here too;
    /// a printCAD file, a linked part, waits for the app. A mesh imported
    /// as a scan is marked so and can be set back, an STL read in inches
    /// says so, and a solid refuses to be one.
    #[test]
    fn a_script_imports_a_step_file() {
        let step = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kernel_ogeom/tests/data/box_native.step")
            .display()
            .to_string();
        let dir = std::env::temp_dir().join(format!("printcad_scan_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mesh = dir.join("tetra.stl");
        std::fs::write(&mesh, TETRAHEDRON_STL).unwrap();
        let mesh = mesh.display().to_string();
        let source = format!(
            r#"
            pc.file.import{{path = [[{step}]]}}
            local bodies = pc.doc.bodies()
            assert(#bodies == 1, #bodies)
            assert(math.abs(pc.doc.measure{{body = bodies[1].id}}.volume - 24000) < 1e-6)
            assert(not pcall(pc.file.import, {{path = "part.prtcad"}}))
            assert(not pcall(pc.doc.set_body, {{body = bodies[1].id, scan = true}}))
            pc.file.import{{path = [[{mesh}]], scan = true}}
            local scan = pc.doc.bodies()[2]
            assert(scan.scan and not bodies[1].scan)
            assert(scan.scan.units == "mm")
            pc.doc.set_body{{body = scan.id, scan = false}}
            assert(not pc.doc.bodies()[2].scan)
            pc.file.import{{path = [[{mesh}]], scan = true, units = "inch"}}
            assert(pc.doc.bodies()[3].scan.units == "inch")
            assert(not pcall(pc.file.import, {{path = [[{mesh}]], units = "furlong"}}))
            "#
        );
        let mut registry = registry();
        let ran = crate::headless::run_in_empty_document(&mut registry, &source, "import");
        let _ = std::fs::remove_dir_all(&dir);
        ran.unwrap();
    }

    /// A closed tetrahedron, outward faces, as ASCII STL.
    const TETRAHEDRON_STL: &str = "solid t
facet normal 0 0 -1
outer loop
vertex 0 0 0
vertex 0 10 0
vertex 10 0 0
endloop
endfacet
facet normal 0 -1 0
outer loop
vertex 0 0 0
vertex 10 0 0
vertex 0 0 10
endloop
endfacet
facet normal -1 0 0
outer loop
vertex 0 0 0
vertex 0 0 10
vertex 0 10 0
endloop
endfacet
facet normal 1 1 1
outer loop
vertex 10 0 0
vertex 0 10 0
vertex 0 0 10
endloop
endfacet
endsolid t
";

    #[test]
    fn the_command_line_names_the_script_and_what_to_do_around_it() {
        assert_eq!(parse(&words("")), Ok(None));
        assert_eq!(parse(&words("some.prtcad")), Ok(None), "a plain launch");
        let run = parse(&words(
            "--script b.lua --open in.prtcad --save out.prtcad -- 20 x",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(run.script, PathBuf::from("b.lua"));
        assert_eq!(run.open, Some(PathBuf::from("in.prtcad")));
        assert_eq!(run.save, Some(PathBuf::from("out.prtcad")));
        assert_eq!(run.args, ["20", "x"]);
        assert!(parse(&words("--save out.prtcad")).is_err());
        assert!(parse(&words("--script")).is_err());
    }

    /// Deleting a step rebuilds its body from what is left, and the last
    /// step's going leaves the body no shape: for surface steps as for
    /// Design features.
    #[test]
    fn deleting_a_step_rebuilds_its_body_or_clears_it() {
        let script = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.polyline{sketch = s, points = {{1, 0}, {11, 0}}}
            local walls = pc.surface.extrude{sketches = {s}, length = 5}
            local body = pc.doc.feature{id = walls}.body
            local mirror = pc.surface.mirror{body = body, plane = "YZ"}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.area - 100) < 1e-6)
            pc.doc.delete{id = mirror}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.area - 50) < 1e-6, "the mirror is gone")
            pc.doc.delete{id = walls}
            assert(#pc.doc.rebuild() == 0)
            assert(not pcall(pc.doc.measure, {body = body}), "nothing builds the body")

            local p = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = p, x = 0, y = 0, width = 10, height = 10}
            local pad = pc.design.pad{sketch = p, length = 2}
            local block = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            pc.doc.delete{id = pad}
            assert(#pc.doc.rebuild() == 0)
            assert(not pcall(pc.doc.measure, {body = block}), "the pad's solid went with it")
            "#;
        run_in_empty_document(&mut registry(), script, "delete").unwrap();
    }

    /// A feature a package that is not installed made keeps its data: a
    /// script without a window cannot delete it, as the tree cannot.
    #[test]
    fn deleting_a_missing_packages_feature_is_refused() {
        let mut document = Document::new("Needs a package");
        let body = document.create_body(Some("Body".into()));
        let feature = document.add_feature_of_kind(
            core_document::WorkbenchId::new("acme.widget"),
            "Widget".into(),
            Some(body),
            Vec::new(),
            serde_json::json!({}),
            core_document::FeatureOrigin::new("acme.widget 1.0.0", None),
        );
        let script = format!(r#"pc.doc.delete{{id = "{}"}}"#, feature.0);
        let (document, out) = run_in_document(&mut registry(), document, &script, "delete");
        let why = out.unwrap_err();
        assert!(why.contains("not installed"), "{why}");
        assert!(
            document.get_feature_meta(feature).is_some(),
            "the data waits"
        );
    }

    #[test]
    fn a_script_builds_measures_exports_and_saves_without_a_window() {
        let dir = scratch("headless");
        let script = dir.join("block.lua");
        std::fs::write(
            &script,
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.design.pad{sketch = s, length = tonumber(arg[1])}
            assert(#pc.doc.rebuild() == 0, "the pad builds")
            local body = pc.doc.bodies()[1].id
            local m = pc.doc.measure{body = body}
            print(string.format("%.1f", m.volume))
            pc.file.export{path = arg[2]}
            local shot = pc.doc.picture{path = arg[3], view = "iso", size = {400, 300},
                highlight = {{body = body, faces = {0}}}, annotate = true,
                markers = {{point = {0, 0, 5}, label = "corner"}}}
            assert(shot.width == 400 and shot.height == 300)
            "#,
        )
        .unwrap();
        let stl = dir.join("block.stl");
        let saved = dir.join("block.prtcad");
        let picture = dir.join("shots/block.png");
        let args = vec![
            "5".into(),
            stl.display().to_string(),
            picture.display().to_string(),
        ];
        assert!(run_file(script, Some(saved.clone()), args));
        assert!(
            std::fs::metadata(&stl).unwrap().len() > 84,
            "an STL with triangles"
        );
        let png = tiny_skia::Pixmap::load_png(&picture).unwrap();
        assert_eq!((png.width(), png.height()), (400, 300));
        let reopened = Document::load_from_file(&saved).unwrap();
        assert_eq!(reopened.bodies().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    /// `doc.edges` lists a box's twelve lines and a cylinder's circles, and
    /// an edge taken from it is what a fillet takes.
    #[test]
    fn a_script_finds_edges_and_fillets_one_it_picked() {
        let script = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local round = pc.doc.new_body{name = "Round"}
            local c = pc.sketch.new{body = round, plane = "XY"}
            pc.sketch.circle{sketch = c, x = 50, y = 0, radius = 4}
            pc.design.pad{sketch = c, length = 6}
            assert(#pc.doc.rebuild() == 0, "both build")

            local edges = pc.doc.edges{body = body}
            assert(#edges == 12, "a box has 12 edges, not " .. #edges)
            local count = {}
            for _, e in ipairs(edges) do
              assert(e.kind == "line", "every edge of a box is a line")
              assert(#e.faces == 2, "between two faces")
              local l = math.floor(e.length + 0.5)
              count[l] = (count[l] or 0) + 1
            end
            assert(count[20] == 4 and count[10] == 4 and count[5] == 4, "lengths")

            local circles = 0
            for _, e in ipairs(pc.doc.edges{body = round}) do
              if e.kind == "circle" then
                circles = circles + 1
                assert(math.abs(e.radius - 4) < 1e-3, "radius " .. e.radius)
                assert(math.abs(e.centre[1] - 50) < 1e-3, "centre")
              end
            end
            assert(circles >= 2, "a cylinder's rims are circles")

            -- The upright edge at x = 20, y = 10, between two side faces.
            local faces = {}
            for _, f in ipairs(pc.doc.faces{body = body}) do faces[f.index] = f end
            local pick
            for _, e in ipairs(edges) do
              if math.abs(e.direction[3]) > 0.99 and e.point[1] > 19 and e.point[2] > 9 then
                pick = e
              end
            end
            for _, i in ipairs(pick.faces) do
              assert(math.abs(faces[i].normal[3]) < 1e-3, "a side face")
            end
            local before = pc.doc.measure{body = body}.volume
            pc.design.fillet{body = body, radius = 2, edges = {Edges = {
              {point = pick.point, direction = pick.direction, faces = pick.names}}}}
            assert(#pc.doc.rebuild() == 0, "the fillet builds")
            local taken = before - pc.doc.measure{body = body}.volume
            local wanted = (4 - math.pi) * 5
            assert(math.abs(taken - wanted) < 0.05, "one edge rounded: " .. taken)
            "#;
        run_in_empty_document(&mut registry(), script, "edges").unwrap();
    }

    /// A fillet read with `doc.feature` gives its edges' face names as
    /// strings, whatever their size (a name at or above 2^63 is past what
    /// a Lua integer holds), and its edges written back unchanged keep
    /// every name: the
    /// fillet builds with each edge found by its names, no warning. A
    /// float where a name goes is refused, naming the field.
    #[test]
    fn a_fillet_read_and_written_back_keeps_its_face_names() {
        let script = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0, "the pad builds")
            local picks = {}
            for _, e in ipairs(pc.doc.edges{body = body}) do
              if math.abs(e.direction[3]) > 0.99 then
                picks[#picks + 1] = {point = e.point, direction = e.direction, faces = e.names}
              end
            end
            assert(#picks == 4, "four upright edges")
            local fillet = pc.design.fillet{body = body, radius = 1, edges = {Edges = picks}}
            assert(#pc.doc.rebuild() == 0, "the fillet builds")

            local read = pc.doc.feature{id = fillet}
            local edges = read.fields.Fillet.edges
            for i, p in ipairs(edges.Edges) do
              for j, n in ipairs(p.faces) do
                assert(type(n) == "string", "a name reads as a string: " .. tostring(n))
                assert(n == picks[i].faces[j], n .. " read, " .. picks[i].faces[j] .. " picked")
              end
            end

            pc.design.set{feature = fillet, edges = edges}
            assert(#pc.doc.rebuild() == 0, "the fillet builds again")
            local after = pc.doc.feature{id = fillet}
            assert(after.error == nil, tostring(after.error))
            assert(after.warning == nil, tostring(after.warning))
            for i, p in ipairs(after.fields.Fillet.edges.Edges) do
              assert(p.faces[1] == picks[i].faces[1] and p.faces[2] == picks[i].faces[2])
            end

            local p = picks[1]
            local ok, why = pcall(pc.design.set, {feature = fillet, edges = {Edges = {
              {point = p.point, direction = p.direction, faces = {1.1208812177732762e19, 5}}}}})
            assert(not ok, "a float is no name")
            why = tostring(why)
            assert(why:find("`faces`", 1, true) and why:find("floating-point", 1, true), why)
            "#;
        run_in_empty_document(&mut registry(), script, "names").unwrap();
    }

    /// A pick stored before a later pad merges its faces with others is
    /// listed under the merged faces: `doc.faces` and `doc.edges` give
    /// every name a face carries (`name` and `also`), so each name the pick
    /// kept is found on the edge it still resolves to, and a fillet on it
    /// builds unwarned. Names follow random feature ids, so only equality
    /// between reads is asserted, never which name is least.
    #[test]
    fn faces_and_edges_list_every_name_a_merged_face_carries() {
        let script = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0, "the block builds")
            local function front_top(edges)
              for _, e in ipairs(edges) do
                if math.abs(e.direction[1]) > 0.99 and math.abs(e.point[2]) < 1e-3
                    and math.abs(e.point[3] - 5) < 1e-3 then
                  return e
                end
              end
            end
            local picked = front_top(pc.doc.edges{body = body})
            local kept = {picked.names[1], picked.names[2]}

            local beside = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = beside, x = 20, y = 0, width = 10, height = 10}
            pc.design.pad{sketch = beside, length = 5, refine = true}
            assert(#pc.doc.rebuild() == 0, "the second block fuses on")

            local function carries(name, first, others)
              if first == name then return true end
              for _, n in ipairs(others or {}) do
                if n == name then return true end
              end
              return false
            end
            local edge = front_top(pc.doc.edges{body = body})
            assert(math.abs(edge.length - 30) < 1e-3, "one merged edge: " .. edge.length)
            assert(edge.also and #edge.also[1] > 0 and #edge.also[2] > 0,
              "each merged face carries both blocks' names")
            for _, name in ipairs(kept) do
              local found = false
              for i = 1, 2 do
                local also = edge.also and edge.also[i] or {}
                found = found or carries(name, edge.names[i], also)
              end
              assert(found, "doc.edges lists " .. name .. " on the merged edge")
              local on = 0
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if carries(name, f.name, f.also) then
                  on = on + 1
                  assert(f.index == edge.faces[1] or f.index == edge.faces[2],
                    "on one of the edge's faces")
                end
              end
              assert(on == 1, "doc.faces lists " .. name .. " on one face, not " .. on)
            end

            local fillet = pc.design.fillet{body = body, radius = 1, edges = {Edges = {
              {point = picked.point, direction = picked.direction, faces = kept}}}}
            assert(#pc.doc.rebuild() == 0, "the fillet builds")
            local read = pc.doc.feature{id = fillet}
            assert(read.warning == nil, tostring(read.warning))
            "#;
        run_in_empty_document(&mut registry(), script, "merged").unwrap();
    }

    /// A document read back builds its features again rather than take
    /// the solid it saved as built: a fillet too big for its edge fails
    /// again, the body is the pad before it, and the document is unedited.
    #[test]
    fn a_reopened_document_finds_its_failing_feature_again() {
        let mut registry = registry();
        let made = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            pc.design.fillet{body = body, radius = 50, mode = "Radius",
              edges = {Edges = {{point = {20, 5, 5}, direction = {0, 1, 0}}}}}
            assert(#pc.doc.rebuild() == 1, "the fillet fails")
            "#;
        let mut document = run_in_empty_document(&mut registry, made, "made").unwrap();
        let bytes = document
            .save_to_bytes(core_document::Compression::None)
            .unwrap();
        let reopened =
            crate::app::doc_io::opened(&registry, Document::load_from_bytes(bytes).unwrap());
        let check = r#"
            local failed = pc.doc.rebuild()
            assert(#failed == 1 and failed[1].name == "Fillet", "failures: " .. #failed)
            local m = pc.doc.measure{body = pc.doc.bodies()[1].id}
            assert(math.abs(m.volume - 1000) < 1e-6, "the pad: " .. m.volume)
            "#;
        let (document, out) = run_in_document(&mut registry, reopened, check, "reopened");
        out.unwrap();
        assert!(!document.metadata().dirty(), "building it again is no edit");
    }

    /// Evaluating after each edit, which takes what the last evaluation
    /// settled where the input is the same, comes to what evaluating the
    /// document afresh does: over a seeded run of edits to variables,
    /// formulas, a configuration, sketch dimensions and a datum a sketch
    /// stands on, every feature's values and formula results agree, and
    /// every built feature whose values moved is marked for rebuilding.
    #[test]
    fn evaluating_after_each_edit_agrees_with_evaluating_afresh() {
        let mut registry = registry();
        let made = r#"
            pc.var.new{name = "Size"}
            pc.var.set{set = "Size", name = "w", formula = "30 mm"}
            pc.var.set{set = "Size", name = "t", formula = "Size.w / 10"}
            local body = pc.doc.new_body{name = "Part"}
            local s = pc.sketch.new{body = body, plane = "XY", name = "Base"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
            local wide = pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 10}[1]
            local deep = pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[2]}, value = 10}[1]
            pc.doc.set_formula{id = s, parameter = wide, formula = "Size.w"}
            local pad = pc.design.pad{sketch = s}
            pc.doc.set_formula{id = pad, parameter = "length", formula = "Size.t"}
            local pad_name = "`" .. pc.doc.feature{id = pad}.name .. "`"
            local datum = pc.design.datum{kind = "plane", body = body}
            pc.doc.set_formula{id = datum, parameter = "offset_z", formula = pad_name .. ".length"}
            local top = pc.sketch.new{body = body, on = datum, name = "Top"}
            pc.sketch.circle{sketch = top, x = 5, y = 5, radius = 2}
            local pad2 = pc.design.pad{sketch = top, length = 2}
            pc.doc.set_formula{id = pad2, parameter = "length", formula = pad_name .. ".length * 2"}
            pc.config.add_variable{variable = "Size.w"}
            pc.config.new{name = "Small"}
            pc.config.set{name = "Small", variable = "Size.w", value = "20 mm"}
            return {s = s, deep = deep, pad = pad, datum = datum, pad2 = pad2, name = pad_name}
            "#;
        let (mut document, ids) = run_in_document(&mut registry, Document::new("t"), made, "made");
        let ids = ids.unwrap().unwrap();
        let id = |key: &str| ids[key].as_str().unwrap().to_string();
        let (s, deep, pad, datum, pad2, name) = (
            id("s"),
            id("deep"),
            id("pad"),
            id("datum"),
            id("pad2"),
            id("name"),
        );
        let state = |doc: &Document| {
            let mut out: Vec<_> = doc
                .feature_tree()
                .all_nodes()
                .map(|(id, _)| {
                    let results: Vec<_> = doc
                        .evaluated_slots(*id)
                        .iter()
                        .map(|slot| slot.result.clone())
                        .collect();
                    (*id, doc.feature_values(*id).cloned(), results)
                })
                .collect();
            out.sort_by_key(|(id, ..)| *id);
            out
        };
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for step in 0..40 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let n = 1.0 + (seed % 37) as f64;
            let edit = match seed % 7 {
                0 => format!(
                    r#"pc.var.set{{set = "Size", name = "w", formula = "{} mm"}}"#,
                    10.0 + n
                ),
                1 => format!(
                    r#"pc.doc.set_value{{id = "{pad}", parameter = "length", value = {n}}}"#
                ),
                2 => format!(
                    r#"pc.doc.set_formula{{id = "{pad}", parameter = "length", formula = "Size.t"}}"#
                ),
                3 => {
                    format!(r#"pc.doc.set_value{{id = "{s}", parameter = "{deep}", value = {n}}}"#)
                }
                4 => format!(
                    r#"pc.config.activate{{name = {}}}"#,
                    ["nil", "\"Small\""][step % 2]
                ),
                5 => format!(
                    r#"pc.doc.set_formula{{id = "{pad2}", parameter = "length", formula = "{name}.length + {n} mm"}}"#
                ),
                _ => format!(
                    r#"pc.doc.set_value{{id = "{datum}", parameter = "offset_z", value = {n}}}"#
                ),
            };
            let ids: Vec<_> = document
                .feature_tree()
                .all_nodes()
                .map(|(id, _)| *id)
                .collect();
            for id in &ids {
                document.clear_feature_dirty(*id);
            }
            let before = state(&document);
            let out;
            (document, out) = run_in_document(&mut registry, document, &edit, "edit");
            out.unwrap_or_else(|why| panic!("{edit}: {why}"));
            let after = state(&document);
            let bytes = document
                .clone()
                .save_to_bytes(core_document::Compression::None)
                .unwrap();
            let mut fresh =
                crate::app::doc_io::opened(&registry, Document::load_from_bytes(bytes).unwrap());
            registry.evaluate(&mut fresh);
            assert_eq!(after, state(&fresh), "step {step}: {edit}");
            for ((id, was, _), (_, now, _)) in before.iter().zip(&after) {
                let node = document.get_feature_meta(*id).unwrap();
                let built = node.body.is_some();
                assert!(
                    was == now || !built || node.dirty,
                    "step {step}: {edit} moved {}",
                    node.name
                );
            }
        }
    }

    #[test]
    fn variables_drive_a_pad_and_their_formulas_are_saved() {
        let dir = scratch("vars");
        let script = dir.join("vars.lua");
        std::fs::write(
            &script,
            r#"
            pc.var.new{name = "Printer"}
            pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
            local wall = pc.var.set{set = "Printer", name = "wall", formula = "3 * Printer.nozzle"}
            assert(math.abs(wall.value - 1.2) < 1e-9, wall.text)
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            pc.doc.set_formula{id = pad, parameter = "length", formula = "Printer.wall * 10"}
            local height = function()
              assert(#pc.doc.rebuild() == 0, "it builds")
              local m = pc.doc.measure{body = body}
              return m.max[3] - m.min[3]
            end
            assert(math.abs(height() - 12) < 1e-3, "12 mm")
            pc.var.set{set = "Printer", name = "nozzle", formula = "0.6 mm"}
            assert(math.abs(height() - 18) < 1e-3, "18 mm")
            local bad = pc.var.set{set = "Printer", name = "bad", formula = "Printer.wall + 30 deg"}
            assert(bad.error:find("a length and an angle"), bad.error)
            pc.var.remove{set = "Printer", name = "bad"}
            pc.var.rename{set = "Printer", name = "nozzle", to = "bore"}
            assert(pc.var.eval{formula = "Pad.length / Printer.bore"}.value == 30)
            "#,
        )
        .unwrap();
        let saved = dir.join("vars.prtcad");
        assert!(
            run_file(script, Some(saved.clone()), Vec::new()),
            "the script's checks hold"
        );
        let reopened = Document::load_from_file(&saved).unwrap();
        let sets = reopened.variable_sets();
        assert_eq!(sets[0].1, "Printer");
        assert_eq!(sets[0].2.variables[1].formula, "3 * Printer.bore");
        let pad = reopened.object_named("Pad").expect("the pad");
        assert_eq!(
            reopened.feature_formula(pad, "/Pad/length"),
            Some("Printer.wall * 10")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A project archive comes in through the command File › Import runs
    /// for it: a body for each source body and each chain of objects, its
    /// features in order, the fillet's edge found on the saved shape; the
    /// body whose history the kernel cannot build shows the shape its file
    /// saved, and keeps showing it after an edit until it builds.
    #[test]
    fn a_project_archive_imports_as_sketches_features_and_bodies_that_build() {
        let mut registry = registry();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../project_archive/tests/data/body_features.FCStd");
        let (_, import) = registry.file_import_for(&path).expect("an importer");
        assert_eq!(import.label, "FreeCAD file");
        let script = format!(
            r#"
            local path = [[{}]]
            local result = pc.project.import{{path = path}}
            assert(#result.bodies == 4, "a body per source body or chain: " .. #result.bodies)
            assert(#result.sketches == 5, "five sketches: " .. #result.sketches)
            local failed = pc.doc.rebuild()
            assert(#failed == 0, "every body builds: " .. #failed)

            local bodies = {{}}
            for _, b in ipairs(pc.doc.bodies()) do bodies[b.name] = b end
            local names = {{}}
            for _, id in ipairs(bodies.Body.features) do
              local f = pc.doc.feature{{id = id}}
              if f.kind ~= "Sketch" then names[#names + 1] = f.name end
            end
            assert(table.concat(names, " ") == "Pad Hole Boss Notch Recess Fillet AfterFillet",
              table.concat(names, " "))
            local pad = pc.doc.feature{{id = bodies.Body.features[2]}}
            assert(pad.kind == "Pad" and pad.fields.Pad.length == 10, "the pad's length")
            local shown = pc.doc.measure{{body = bodies.Body.id}}
            -- Built from its features, within a part in a thousand of the
            -- shape the file saved.
            assert(math.abs(shown.volume / 11335.327 - 1) < 1e-3, "the saved shape: " .. shown.volume)
            local cut = pc.doc.measure{{body = bodies.Cut.id}}
            assert(math.abs(cut.volume - 717.257) < 1e-2 and math.abs(cut.min[2] - 60) < 1e-6)
            pc.doc.set_formula{{id = pad.id, parameter = "length", formula = "20 mm"}}
            assert(#pc.doc.rebuild() == 0)
            shown = pc.doc.measure{{body = bodies.Body.id}}
            assert(math.abs(shown.volume - 22704.680) < 1e-2, "the longer pad: " .. shown.volume)
            "#,
            path.display(),
        );
        run_in_empty_document(&mut registry, &script, "project").unwrap();
    }

    /// A formula that fails is a failure of its feature: `doc.rebuild`
    /// names it, and the feature does not build from the number it held.
    #[test]
    fn a_failing_formula_fails_its_feature_s_build() {
        let script = r#"
            pc.var.new{name = "Dims"}
            pc.var.set{set = "Dims", name = "thickness", formula = "6"}
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0, "it builds")
            pc.doc.set_formula{id = pad, parameter = "length", formula = "Dims.thickness"}
            local failed = pc.doc.rebuild()
            assert(#failed == 1, "one failure, not " .. #failed)
            assert(failed[1].feature == pad, "the pad fails")
            assert(failed[1].error:find("formula"), failed[1].error)
            assert(not pcall(pc.doc.measure, {body = body}), "no solid from the old length")
            pc.var.set{set = "Dims", name = "thickness", formula = "6 mm"}
            assert(#pc.doc.rebuild() == 0, "it builds again")
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.max[3] - m.min[3] - 6) < 1e-3, "6 mm")
        "#;
        run_in_empty_document(&mut registry(), script, "formula").unwrap();
    }

    #[test]
    fn the_export_of_every_configuration_writes_one_file_each_and_puts_back_the_active_one() {
        let dir = scratch("every");
        let script = dir.join("every.lua");
        let setup = r#"
            pc.var.new{name = "Size"}
            pc.var.set{set = "Size", name = "height", formula = "5 mm"}
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 1}
            pc.doc.set_formula{id = pad, parameter = "length", formula = "Size.height"}
            pc.config.new{name = "Small"}
            pc.config.new{name = "Large two"}
            pc.config.add_variable{variable = "Size.height"}
            pc.config.set{name = "Small", variable = "Size.height", value = "3 mm"}
            pc.config.set{name = "Large two", variable = "Size.height", value = "20 mm"}
            pc.config.activate{name = "Small"}
        "#;
        // The NURBS kind of STEP, which the file's extension cannot say.
        let draft = doc_export::ExportDraft {
            format: kernel_ogeom::export::ExportFormat::StepNurbs,
            ..Default::default()
        };
        let export = doc_export::every_configuration_script(
            &dir.join("part").display().to_string(),
            &draft,
            None,
        );
        let check = r#"
            assert(pc.config.list().active == "Small", "the active one is back")
        "#;
        std::fs::write(&script, format!("{setup}\n{export}\n{check}")).unwrap();
        assert!(run_file(script, None, Vec::new()), "the script runs");
        for name in ["part-Small.step", "part-Large two.step"] {
            let text = std::fs::read_to_string(dir.join(name)).unwrap();
            assert!(
                text.contains("B_SPLINE_SURFACE") && !text.contains("PLANE("),
                "{name} is all splines"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The app builds a document as a script does: its kernel worker,
    /// with its threads, kept solids and chain states, and the landing of
    /// its answers come to the same meshes and the same feature errors as
    /// the build on the script's thread, for a body that builds, one that
    /// stops at a failing fillet and one built against another body.
    #[test]
    fn the_app_s_kernel_worker_builds_what_a_script_builds() {
        use build_driver::{Next, clear_unbuilt, record_failure, store_answers, store_solid};
        use kernel_worker::{KernelResponse, KernelWorker};
        let mut registry = registry();
        let made = r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            pc.design.fillet{body = body, radius = 50, mode = "Radius",
              edges = {Edges = {{point = {20, 5, 5}, direction = {0, 1, 0}}}}}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
            pc.sketch.rect{sketch = top, x = 2, y = 2, width = 5, height = 5}
            pc.design.pad{sketch = top, length = 3}
            local tool = pc.doc.new_body{name = "Tool"}
            local t = pc.sketch.new{body = tool, plane = "XY"}
            pc.sketch.circle{sketch = t, x = 10, y = 5, radius = 3}
            pc.design.pad{sketch = t, length = 20}
            local other = pc.doc.new_body{name = "Other"}
            local o = pc.sketch.new{body = other, plane = "XY"}
            pc.sketch.rect{sketch = o, x = 0, y = 0, width = 30, height = 30}
            pc.design.pad{sketch = o, length = 2}
            pc.design.boolean{body = other, tool_body = tool, kind = "Cut"}
            assert(#pc.doc.rebuild() == 2, "the fillet fails, the pad after it is left out")
            "#;
        let made = run_in_empty_document(&mut registry, made, "made").unwrap();
        // Nothing built: every body's history to build again.
        let unbuilt = |registry: &DocumentService| {
            let mut document = made.clone();
            for body in document.bodies().iter().map(|b| b.id).collect::<Vec<_>>() {
                document.derived_mut().remove_imported_geometry(body);
                document.derived_mut().clear_body_feature_errors(body);
            }
            registry.invalidate_all(&mut document);
            document
        };
        let scripted = unbuilt(&registry);
        let (scripted, out) = run_in_document(&mut registry, scripted, "pc.doc.rebuild()", "s");
        out.unwrap();

        let mut app = unbuilt(&registry);
        let mut worker = KernelWorker::spawn();
        let detail = TessellationSettings::default();
        for _ in 0..32 {
            let jobs = registry.rebuild_jobs(&mut app);
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                let begun = build_driver::begin(&mut app, job);
                if let Next::Build(plan) = begun.next {
                    worker.request_build_solid(
                        begun.body.0,
                        plan.ops,
                        plan.op_features,
                        detail.clone(),
                        None,
                        plan.probes,
                        plan.projections,
                    );
                }
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while worker.in_flight() > 0 && std::time::Instant::now() < deadline {
                for answer in worker.drain() {
                    match answer {
                        KernelResponse::SolidBuilt {
                            body_id,
                            result,
                            probes,
                            projections,
                            op_features,
                            failed,
                            ..
                        } => {
                            let body = core_document::BodyId(body_id);
                            store_answers(&mut app, &op_features, &probes, &projections, &result);
                            store_solid(&mut app, body, *result);
                            if let Some(failed) = failed {
                                record_failure(&mut app, &failed);
                            }
                        }
                        KernelResponse::SolidFailed {
                            body_id,
                            failure,
                            nothing_built,
                        } => {
                            record_failure(&mut app, &failure);
                            if nothing_built {
                                clear_unbuilt(&mut app, core_document::BodyId(body_id));
                            }
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        let shown = |document: &Document| {
            let bodies: Vec<_> = document
                .bodies()
                .iter()
                .map(|b| {
                    let mesh = document
                        .derived()
                        .imported_geometry(b.id)
                        .map(|g| g.mesh.clone());
                    let mesh = mesh.map(|m| (m.positions.clone(), m.indices.clone()));
                    (b.name.clone(), mesh)
                })
                .collect();
            let mut errors: Vec<_> = document
                .feature_tree()
                .all_nodes()
                .map(|(id, n)| {
                    let built = document.derived();
                    let said = |s: Option<&str>| s.map(str::to_string);
                    (
                        n.name.clone(),
                        said(built.error(*id)),
                        said(built.warning(*id)),
                    )
                })
                .collect();
            errors.sort();
            (bodies, errors)
        };
        let (bodies, errors) = shown(&scripted);
        assert!(
            bodies.iter().all(|(_, mesh)| mesh.is_some()),
            "every body built"
        );
        assert_eq!(errors.iter().filter(|(_, e, _)| e.is_some()).count(), 2);
        assert_eq!((bodies, errors), shown(&app));
    }
}
